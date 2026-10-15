//! Pipeline render entry points.
use crate::WorkflowRoute;

use super::*;

impl Pipeline {
    /// Process an image with explicit pipeline taps — the Rust spelling of
    /// upstream `SimulationPipeline.process(image, inject, collect)`.
    ///
    /// Omitted endpoints use persistent `params.taps`, then `rgb_in` /
    /// `rgb_out`. Injecting at `log_e_film` or later bypasses the camera
    /// (and film) stages upstream of the injection point; collecting at
    /// `cmy_film` returns film densities instead of final RGB. Invalid or
    /// unreachable pairs fail with the upstream error text.
    ///
    /// Nondefault endpoints use per-stage boundaries. The normal end-to-end
    /// pair retains the fused GPU-resident path and full-input preparation.
    pub fn process_with_taps(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps_timed(image, backend, inject, collect, None, None)
    }

    /// Process while collecting per-node wall-clock timings.
    pub fn process_with_timings(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        timings: &mut BTreeMap<String, f64>,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps_timed(image, backend, None, None, Some(timings), None)
    }

    pub fn process_with_taps_timed(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
        mut timings: Option<&mut BTreeMap<String, f64>>,
        mut capture: Option<&mut crate::stage_evidence::StageCapture>,
    ) -> Result<ImageBuf, String> {
        if capture.is_some() {
            crate::stage_evidence::ensure_supported_backend(backend)?;
            crate::stage_evidence::ensure_supported_params(&self.params)?;
        }
        let inject = match inject {
            Some(tap) => tap,
            None => self
                .params
                .taps
                .inject
                .as_deref()
                .map(Tap::parse)
                .transpose()?
                .unwrap_or(Tap::RgbIn),
        };
        if self.params.scanner.auto_setup == crate::params::ScannerAutoSetup::PerFrame
            && self.scanner_setup.is_none()
        {
            if inject != Tap::RgbIn {
                return Err("per_frame scanner setup requires the full rgb_in input".into());
            }
            let setup = self.prepare_scanner_setup(&image)?;
            let mut prepared = self.clone();
            prepared.scanner_setup = Some(setup);
            return prepared.process_with_taps_timed(
                image,
                backend,
                Some(inject),
                collect,
                timings,
                capture,
            );
        }
        // Negative-to-positive interpretation is part of the resident
        // decision: the capture methods run as the assembled scanner readout
        // fence and the density reading and Digital Reference replace the
        // acquisition as the assembled acquisition fence (pipe
        // architecture, capability class). `resident_decision` is the sole
        // authority, so no whole-route demotion happens here.
        let collect = match collect {
            Some(tap) => tap,
            None => self
                .params
                .taps
                .collect
                .as_deref()
                .map(Tap::parse)
                .transpose()?
                .unwrap_or(Tap::RgbOut),
        };
        if collect == Tap::RgbOut && self.params.output_mix.amount < 1.0 && inject != Tap::RgbIn {
            return Err(
                "output_mix.amount below 1 requires rgb_in injection for final rgb_out collection"
                    .into(),
            );
        }
        if crate::stages::grain_v3::active(&self.params) {
            let cell_spectral = crate::stages::grain_v3::validate_cell_spectral(&self.params)?;
            let aperture_factor = crate::stages::grain_v3::validate_aperture(&self.params)?;
            if cell_spectral {
                crate::stages::grain_v3::validate_supported(&self.film, &self.params)?;
            }
            Self::validate_grain_v3_taps(
                cell_spectral,
                aperture_factor,
                inject,
                collect,
                self.params.io.scan_film,
            )?;
        }
        if let Some(context) = backend.observation_context()
            && (inject != Tap::RgbIn || collect != Tap::RgbOut)
        {
            context.decline_resident(
                spektrafilm_gpu::telemetry::ResidentDeclineReason::DiagnosticTapRoute,
            );
            context.set_path(if backend.is_gpu() {
                spektrafilm_gpu::telemetry::ExecutionPath::PerStage
            } else {
                spektrafilm_gpu::telemetry::ExecutionPath::Cpu
            });
        }
        if let Some(capture) = capture.as_deref_mut() {
            capture.begin(
                &self.params,
                self.print_exposure_factor,
                inject,
                collect,
                &image,
            );
        }
        if inject == Tap::RgbIn && collect == Tap::RgbOut {
            let source = if self.params.output_mix.amount < 1.0 {
                Some(self.prepare_output_mix_source(&image)?)
            } else {
                None
            };
            let result = self.process_full(
                image,
                backend,
                timings.as_deref_mut(),
                capture.as_deref_mut(),
            )?;
            let result = self.compose_output_mix(result, source, backend, timings)?;
            if let Some(capture) = capture.as_deref_mut() {
                capture.record_boundary(
                    crate::stage_evidence::StageId::RgbOut,
                    &result,
                    crate::stage_evidence::CoordinateSpace::Output,
                );
            }
            return Ok(result);
        }
        tracing::info!(
            inject = inject.name(),
            collect = collect.name(),
            "pipeline: start"
        );
        let t = Instant::now();
        let color_ref = self.color_reference(backend);
        record_stage_timing(&mut timings, "color_reference", t);
        let result = self.run_from(
            image,
            inject,
            collect,
            backend,
            &color_ref,
            None,
            timings,
            capture.as_deref_mut(),
        )?;
        if collect == Tap::RgbOut
            && let Some(capture) = capture
        {
            capture.record_boundary(
                crate::stage_evidence::StageId::RgbOut,
                &result,
                crate::stage_evidence::CoordinateSpace::Output,
            );
        }
        Ok(result)
    }

    pub fn process(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps(image, backend, None, None)
    }

    fn process_full(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        mut timings: Option<&mut BTreeMap<String, f64>>,
        mut capture: Option<&mut crate::stage_evidence::StageCapture>,
    ) -> Result<ImageBuf, String> {
        // Active Dye outside its supported condition fails here, before any
        // route work — including the passthrough and convert routes, which
        // never reach a grain stage.
        crate::stages::grain_v3::validate_supported(&self.film, &self.params)?;
        crate::stages::grain_fotufilm::validate_supported(&self.film, &self.params)?;
        self.check_cancelled()?;
        if self.params.workflow.route_enum() == WorkflowRoute::Input {
            if let Some(capture) = capture.as_deref_mut() {
                use crate::stage_evidence::StageId;
                capture.record_skip(StageId::LogEFilm, "no_film_stage_on_route");
                capture.record_skip(StageId::CmyFilm, "no_film_stage_on_route");
                capture.record_skip(StageId::LogEPrint, "no_print_stage_on_route");
                capture.record_skip(StageId::CmyPrint, "no_print_stage_on_route");
            }
            let (working, _) = crate::resizing::crop_and_rescale(
                &image,
                &self.params.io,
                self.params.camera.film_format_mm,
            )?;
            let image = match working {
                std::borrow::Cow::Owned(working) => working,
                std::borrow::Cow::Borrowed(_) => image,
            };
            let _observation = crate::stages::StageObservation::cpu(
                backend,
                "post_scan",
                spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
            );
            if let Some(context) = backend.observation_context() {
                context.set_path(spektrafilm_gpu::telemetry::ExecutionPath::Cpu);
                context.set_working_dimensions(image.width, image.height);
                context.decline_resident(
                    spektrafilm_gpu::telemetry::ResidentDeclineReason::WorkflowRoute,
                );
            }
            use spektrafilm_math::colorspace::{conversion_matrix, convert_rgb, resolve};
            let source = resolve(&self.params.io.input_color_space)?;
            let destination = resolve(&self.params.io.output_color_space)?;
            let matrix = conversion_matrix(source, destination);
            let mut output = image;
            output.data.par_chunks_exact_mut(3).for_each(|pixel| {
                let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
                let rgb = convert_rgb(
                    rgb,
                    source,
                    self.params.io.input_cctf_decoding,
                    destination,
                    self.params.io.output_cctf_encoding,
                    &matrix,
                );
                for channel in 0..3 {
                    pixel[channel] = from_f64(rgb[channel]);
                }
            });
            // Finished RGB accepts the optional magazine finishing exactly like
            // a scanned base workflow: after output color management, before
            // the caller's image writer.
            crate::magazine_print_color::apply(
                &mut output,
                &self.params.magazine_print_color,
                destination,
                self.params.io.output_cctf_encoding,
            );
            return Ok(output);
        }
        let image = self.expose_full_input(image, backend, capture.as_deref_mut());
        let t = Instant::now();
        let (working, pixel_size_um) = self.rescale_working(&image, backend)?;
        print_stage_timing(stage_timings_enabled(), "resize", t);
        record_stage_timing(&mut timings, "resize", t);
        let working = match working {
            std::borrow::Cow::Owned(working) => working,
            std::borrow::Cow::Borrowed(_) => image,
        };
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(working.width, working.height);
        }
        if self
            .params
            .workflow
            .route
            .starts_with("input > convert-film")
        {
            return self.process_convert(working, pixel_size_um, backend, capture);
        }
        self.run(working, pixel_size_um, 0.0, backend, timings, capture)
    }

    fn process_convert(
        &self,
        image: ImageBuf,
        pixel_size_um: f64,
        backend: &dyn ComputeBackend,
        mut capture: Option<&mut crate::stage_evidence::StageCapture>,
    ) -> Result<ImageBuf, String> {
        use crate::stage_evidence::{CoordinateSpace, StageId};
        if let Some(capture) = capture.as_deref_mut() {
            // Conversion maps the receiver capture back to film density: the
            // chain has no film exposure boundary.
            capture.record_skip(StageId::LogEFilm, "no_film_exposure_on_route");
        }
        if let Some(context) = backend.observation_context() {
            context.set_path(if backend.is_gpu() {
                spektrafilm_gpu::telemetry::ExecutionPath::PerStage
            } else {
                spektrafilm_gpu::telemetry::ExecutionPath::Cpu
            });
            context
                .decline_resident(spektrafilm_gpu::telemetry::ResidentDeclineReason::WorkflowRoute);
        }
        let route = self.params.workflow.route_enum();
        let print_after_convert = route == WorkflowRoute::ConvertFilmPrintScan;
        let scan_minus_base = route == WorkflowRoute::ConvertFilmScanMinusBase;
        let mut converting = crate::stages::StageObservation::cpu(
            backend,
            "converting",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        let converted = crate::stages::converting::process(&image, &self.film, &self.params);
        converting.set_complete(converted.is_ok());
        let film_density = converted?;
        drop(converting);
        if let Some(capture) = capture.as_deref_mut() {
            capture.record_boundary(StageId::CmyFilm, &film_density, CoordinateSpace::Working);
        }
        let color_ref = self.color_reference(backend);
        if print_after_convert {
            let mut printing = crate::stages::StageObservation::new(backend, "printing");
            let printing_backend = printing.backend(backend);
            let print_density = crate::stages::printing::expose_calibrated(
                &film_density,
                &self.film,
                &self.print,
                &self.params,
                printing_backend,
                &self.print_illuminant,
                self.print_exposure_factor,
                self.preflash_raw,
                color_ref.printing_exposure_correction,
                pixel_size_um,
            );
            if let Some(capture) = capture.as_deref_mut() {
                capture.record_boundary(
                    StageId::LogEPrint,
                    &print_density,
                    CoordinateSpace::Working,
                );
            }
            let developed = crate::stages::printing::develop(
                &print_density,
                &self.print,
                &self.params,
                printing_backend,
            );
            printing.set_complete(developed.is_ok());
            let print_density = developed?;
            drop(printing);
            if let Some(capture) = capture.as_deref_mut() {
                capture.record_boundary(
                    StageId::CmyPrint,
                    &print_density,
                    CoordinateSpace::Working,
                );
            }
            let scanning = crate::stages::StageObservation::new(backend, "scanning");
            let backend = scanning.backend(backend);
            Ok(crate::stages::scanning::process(
                &print_density,
                &self.print,
                &self.params,
                backend,
                &color_ref,
                &self.output_gamut,
            )?)
        } else {
            if let Some(capture) = capture {
                capture.record_skip(StageId::LogEPrint, "no_print_stage_on_route");
                capture.record_skip(StageId::CmyPrint, "no_print_stage_on_route");
            }
            let mut scan_params = self.scan_params().into_owned();
            scan_params.io.scan_film = true;
            let scanning = crate::stages::StageObservation::new(backend, "scanning");
            let backend = scanning.backend(backend);
            Ok(crate::stages::scanning::scan_with_options(
                &film_density,
                &self.film,
                &scan_params,
                backend,
                &color_ref,
                &self.output_gamut,
                Some(&self.params.film_render.convert.scan_illuminant),
                !scan_minus_base,
            )?)
        }
    }

    /// Shared CPU/GPU chain from the prepared working image (crop +
    /// upscale already applied). `pixel_size_um` is the pitch derived
    /// from the full input; `ae_ev` the EV metered on the full input.
    fn run(
        &self,
        image: ImageBuf,
        pixel_size_um: f64,
        ae_ev: f64,
        backend: &dyn ComputeBackend,
        mut timings: Option<&mut BTreeMap<String, f64>>,
        capture: Option<&mut crate::stage_evidence::StageCapture>,
    ) -> Result<ImageBuf, String> {
        let stage_timings = stage_timings_enabled();
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(image.width, image.height);
        }
        // Scanner B&W/slide exposure correction (no-op unless scanner
        // white/black correction is on for a slide or print scan). Computed
        // up front so both the GPU-resident and per-stage paths share it.
        let t = Instant::now();
        let color_ref = self.color_reference(backend);
        print_stage_timing(stage_timings, "color_reference", t);
        record_stage_timing(&mut timings, "color_reference", t);

        // GPU fast path: dispatch the whole filming→printing→scanning chain
        // (or filming→scanning when scan_film) as a single GPU command
        // buffer — one upload + one readback total. WGSL runs the
        // extended resident chain, including camera/scanner lens blur and
        // highlight boost. Unsupported effects select the faithful per-stage
        // path; backends without resident support return `None`. Active Dye
        // selects a per-stage path through `resident_decision`.
        if (self.tc_lut.is_some() || self.mallett_core.is_some())
            && let Some(out) =
                self.try_gpu_resident(&image, backend, &color_ref, pixel_size_um, ae_ev)?
        {
            tracing::info!("pipeline: gpu-resident fast path complete");
            // Post-scan finishing (output transfer + magazine) already ran
            // as pipe-tail passes; the readback is final (pipe
            // architecture, S4).
            return Ok(out);
        }
        if let Some(context) = backend.observation_context() {
            context.set_path(if backend.is_gpu() {
                spektrafilm_gpu::telemetry::ExecutionPath::PerStage
            } else {
                spektrafilm_gpu::telemetry::ExecutionPath::Cpu
            });
        }

        // Per-stage path — the same topology `process_with_taps` walks, from
        // the rescaled working image (rgb_pre) to rgb_out.
        self.run_from(
            image,
            Tap::RgbPre,
            Tap::RgbOut,
            backend,
            &color_ref,
            Some((pixel_size_um, ae_ev)),
            timings,
            capture,
        )
    }

    /// Run only the GPU-resident path from a borrowed image. This is used by
    /// the live GUI, so a full-resolution preview does not need to
    /// deep-clone the loaded image just to satisfy `process(ImageBuf)`.
    ///
    /// Working geometry (crop + upscale, `crop_and_rescale`) is applied
    /// here from the borrow; with neither active the image stays
    /// zero-copy. Auto-exposure is metered on the full input exactly as
    /// [`Self::process`] does.
    ///
    /// `Ok(None)` means the backend cannot run the resident chain and the
    /// caller should fall back to [`Self::process`]; `Err` carries an
    /// invalid-geometry error that must fail the render.
    pub fn process_resident_borrowed(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<Option<ImageBuf>, String> {
        let inject = self
            .params
            .taps
            .inject
            .as_deref()
            .map(Tap::parse)
            .transpose()?
            .unwrap_or(Tap::RgbIn);
        let collect = self
            .params
            .taps
            .collect
            .as_deref()
            .map(Tap::parse)
            .transpose()?
            .unwrap_or(Tap::RgbOut);
        if collect == Tap::RgbOut && self.params.output_mix.amount < 1.0 && inject != Tap::RgbIn {
            return Err(
                "output_mix.amount below 1 requires rgb_in injection for final rgb_out collection"
                    .into(),
            );
        }
        if inject != Tap::RgbIn || collect != Tap::RgbOut {
            if let Some(context) = backend.observation_context() {
                context.decline_resident(
                    spektrafilm_gpu::telemetry::ResidentDeclineReason::DiagnosticTapRoute,
                );
            }
            return Ok(None);
        }
        if self.tc_lut.is_none() && self.mallett_core.is_none() {
            if let Some(context) = backend.observation_context() {
                context.decline_resident(
                    spektrafilm_gpu::telemetry::ResidentDeclineReason::MissingResidentFrontPass,
                );
            }
            return Ok(None);
        }
        tracing::info!(
            backend = backend.name(),
            "pipeline: borrowed resident start"
        );
        let source = if self.params.output_mix.amount < 1.0 {
            Some(self.prepare_output_mix_source(image)?)
        } else {
            None
        };
        let metering = crate::stages::StageObservation::cpu(
            backend,
            "metering",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        let ae_ev = self.meter_autoexposure(image);
        // Clone only when the exposure multiply actually needs owned pixels;
        // a full-resolution preview must not deep-clone a no-op exposure.
        let exposed;
        let image = if self.params.camera.auto_exposure {
            exposed = self.apply_autoexposure(image.clone(), ae_ev);
            &exposed
        } else {
            image
        };
        drop(metering);
        let (working, pixel_size_um) = self.rescale_working(image, backend)?;
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(working.width, working.height);
        }
        let color_ref = self.color_reference(backend);
        let out = match self.try_gpu_resident(&working, backend, &color_ref, pixel_size_um, 0.0)? {
            Some(out) => out,
            None => return Ok(None),
        };
        // Post-scan finishing already ran as pipe-tail passes on the
        // resident chain (pipe architecture, S4); the readback is final.
        Ok(Some(self.compose_output_mix(out, source, backend, None)?))
    }
}
