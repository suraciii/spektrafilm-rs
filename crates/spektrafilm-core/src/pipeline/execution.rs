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
    /// Every selected-GPU span uses retained boundaries; only the requested
    /// result is materialized. CPU-selected spans use the reference stages.
    pub fn process_with_taps(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps_timed(image, backend, inject, collect, None, None)
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
        let route = self.params.workflow.route_enum();
        if route.artifact_input().is_some() || route.artifact_output().is_some() {
            return Err("material routes require the stage execution API, not image processing or diagnostic taps".into());
        }
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

        // Negative-to-positive interpretation is part of the resident
        // decision: the density reading runs as the assembled scanner readout
        // fence and replaces the acquisition as the assembled acquisition
        // fence (pipe architecture, capability class). `resident_decision`
        // is the sole authority, so no whole-route demotion happens here.
        if collect == Tap::RgbOut && self.params.output_mix.amount < 1.0 && inject != Tap::RgbIn {
            return Err(
                "output_mix.amount below 1 requires rgb_in injection for final rgb_out collection"
                    .into(),
            );
        }
        if crate::stages::grain_v3::active(&self.params) {
            crate::stages::grain_v3::validate_supported(&self.film, &self.params)?;
        }
        if backend.is_gpu() {
            // Diagnostic endpoints are not permission to enter the CPU
            // reference topology. The retained entry validates and executes
            // only the selected span, including full-source metering when
            // injection starts at rgb_in.
            let retained = self
                .process_retained_with_taps(
                    &image,
                    RetainedRender::FULL,
                    backend,
                    Some(inject),
                    Some(collect),
                )?
                .ok_or_else(|| "selected GPU declined the retained pipeline route".to_owned())?;
            return backend.materialize_retained(&retained);
        }
        if let Some(context) = backend.observation_context()
            && (inject != Tap::RgbIn || collect != Tap::RgbOut)
        {
            context.decline_resident(
                spektrafilm_backend::telemetry::ResidentDeclineReason::DiagnosticTapRoute,
            );
            context.set_path(spektrafilm_backend::telemetry::ExecutionPath::Cpu);
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
            let result = {
                let source = if self.params.output_mix.amount < 1.0 {
                    Some(self.prepare_output_mix_source(&image, backend)?)
                } else {
                    None
                };
                let result = self.process_full(
                    image,
                    backend,
                    timings.as_deref_mut(),
                    capture.as_deref_mut(),
                )?;
                self.compose_output_mix(result, source, backend, timings)?
            };
            if let Some(capture) = capture.as_deref_mut() {
                capture.record_boundary(
                    crate::stage_evidence::StageId::RgbOut,
                    &result,
                    crate::stage_evidence::CoordinateSpace::Output,
                );
                // The terminal CMYK ink style appearance already ran inside
                // the scanned route's post-scan finishing; the record names
                // that post-rgb_out terminal boundary with the executed
                // parameters, never a spectral stage.
                capture.record_terminal_cmyk_params(&self.params);
            }
            return Ok(result);
        }
        tracing::info!(
            inject = inject.name(),
            collect = collect.name(),
            "pipeline: start"
        );
        let result = self.run_from(
            image,
            inject,
            collect,
            backend,
            None,
            timings,
            capture.as_deref_mut(),
            None,
        )?;
        if collect == Tap::RgbOut
            && let Some(capture) = capture
        {
            capture.record_boundary(
                crate::stage_evidence::StageId::RgbOut,
                &result,
                crate::stage_evidence::CoordinateSpace::Output,
            );
            // The scanned route applied the terminal CMYK ink style
            // appearance inside its post-scan finishing before collection;
            // the record names that post-rgb_out terminal boundary with the
            // executed parameters, never a spectral stage.
            capture.record_terminal_cmyk_params(&self.params);
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

    pub(super) fn process_full(
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
            use spektrafilm_math::colorspace::{conversion_matrix, convert_rgb, resolve};
            let source = resolve(&self.params.io.input_color_space)?;
            let destination = resolve(&self.params.io.output_color_space)?;
            let matrix = conversion_matrix(source, destination);
            // This conversion is the whole numerical content of the input
            // route, so a device that can convert does so; the host path stays
            // the reference for a backend without a device conversion.
            let device = backend
                .convert_colorspace(
                    &image,
                    &spektrafilm_backend::ConvertColorspaceGpuParams {
                        src_cctf: spektrafilm_backend::cctf_kind(source.cctf),
                        dst_cctf: spektrafilm_backend::cctf_kind(destination.cctf),
                        decode: self.params.io.input_cctf_decoding,
                        encode: self.params.io.output_cctf_encoding,
                        matrix: std::array::from_fn(|i| {
                            std::array::from_fn(|j| matrix[i][j] as f32)
                        }),
                    },
                )
                .map_err(|error| format!("input route conversion: {error}"))?;
            let mut output = match device {
                Some(converted) => converted,
                None if backend.is_gpu() => {
                    return Err("selected GPU cannot execute input route conversion".into());
                }
                None => {
                    let _observation = crate::stages::StageObservation::cpu(
                        backend,
                        "post_scan",
                        spektrafilm_backend::telemetry::CpuReason::BackendDefault,
                    );
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
                    output
                }
            };
            if let Some(context) = backend.observation_context() {
                context.set_path(if backend.is_gpu() {
                    spektrafilm_backend::telemetry::ExecutionPath::GpuResident
                } else {
                    spektrafilm_backend::telemetry::ExecutionPath::Cpu
                });
                context.set_working_dimensions(output.width, output.height);
            }
            // The input route applies the shared terminal RGB CMYK ink style
            // appearance after output color management and before publication.
            // The appearance is terminal RGB finishing, not a spectral stage:
            // the caller records it as an independent post-rgb_out record.
            crate::cmyk::apply(
                &mut output,
                &self.params.cmyk,
                destination,
                self.params.io.output_cctf_encoding,
            );
            return Ok(output);
        }
        let image = self.expose_full_input(image, backend, capture.as_deref_mut())?;
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
        if self.params.workflow.route_enum().is_convert_film() {
            if backend.is_gpu()
                && let Some(output) =
                    self.try_gpu_resident(&working, backend, pixel_size_um, 0.0)?
            {
                return Ok(output);
            }
            if backend.is_gpu() {
                return Err("selected GPU cannot run the convert-film inverse scan on this device; retry with an explicit CPU request".into());
            }
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
                spektrafilm_backend::telemetry::ExecutionPath::PerStage
            } else {
                spektrafilm_backend::telemetry::ExecutionPath::Cpu
            });
            context.decline_resident(
                spektrafilm_backend::telemetry::ResidentDeclineReason::WorkflowRoute,
            );
        }
        let route = self.params.workflow.route_enum();
        let print_after_convert = route == WorkflowRoute::ConvertFilmPrintScan;
        let scan_minus_base = route == WorkflowRoute::ConvertFilmScanMinusBase;
        let mut converting = crate::stages::StageObservation::cpu(
            backend,
            "converting",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        let converted = crate::stages::converting::process(&image, &self.film, &self.params);
        converting.set_complete(converted.is_ok());
        let film_density = converted?;
        drop(converting);
        if let Some(capture) = capture.as_deref_mut() {
            capture.record_boundary(StageId::CmyFilm, &film_density, CoordinateSpace::Working);
        }
        if print_after_convert {
            let printing = crate::stages::StageObservation::new(backend, "printing");
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
                pixel_size_um,
            )?;
            let developed = crate::stages::printing::develop(
                &print_density,
                &self.print,
                &self.params,
                printing_backend,
            );
            let print_density = developed?;
            drop(printing);
            if let Some(capture) = capture.as_deref_mut() {
                capture.record_boundary(
                    StageId::LogEPrint,
                    &print_density,
                    CoordinateSpace::Working,
                );
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
                &self.conversion_tones,
                &self.params,
                backend,
                &self.output_gamut,
            )?)
        } else {
            if let Some(capture) = capture {
                capture.record_skip(StageId::LogEPrint, "no_print_stage_on_route");
                capture.record_skip(StageId::CmyPrint, "no_print_stage_on_route");
            }
            let mut scan_params = self.params.clone();
            scan_params.io.scan_film = true;
            let scanning = crate::stages::StageObservation::new(backend, "scanning");
            let backend = scanning.backend(backend);
            Ok(crate::stages::scanning::scan_with_options(
                &film_density,
                &self.film,
                &self.conversion_tones,
                &scan_params,
                backend,
                &self.output_gamut,
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
        timings: Option<&mut BTreeMap<String, f64>>,
        capture: Option<&mut crate::stage_evidence::StageCapture>,
    ) -> Result<ImageBuf, String> {
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(image.width, image.height);
        }

        // GPU fast path: dispatch the whole filming→printing→scanning chain
        // (or filming→scanning when scan_film) as a single GPU command
        // buffer — one upload + one readback total. WGSL runs the
        // extended resident chain, including camera/scanner lens blur and
        // highlight boost. Unsupported effects select the faithful per-stage
        // path; backends without resident support return `None`. Active Dye
        // selects a per-stage path through `resident_decision`.
        if (self.tc_lut.is_some() || self.mallett_core.is_some())
            && let Some(out) = self.try_gpu_resident(&image, backend, pixel_size_um, ae_ev)?
        {
            tracing::info!("pipeline: gpu-resident fast path complete");
            // Post-scan finishing (output transfer + cmyk) already ran
            // as pipe-tail passes; the readback is final (pipe
            // architecture, S4).
            return Ok(out);
        }
        if let Some(context) = backend.observation_context() {
            context.set_path(if backend.is_gpu() {
                spektrafilm_backend::telemetry::ExecutionPath::PerStage
            } else {
                spektrafilm_backend::telemetry::ExecutionPath::Cpu
            });
        }

        // Per-stage path — the same topology `process_with_taps` walks, from
        // the rescaled working image (rgb_pre) to rgb_out.
        self.run_from(
            image,
            Tap::RgbPre,
            Tap::RgbOut,
            backend,
            Some((pixel_size_um, ae_ev)),
            timings,
            capture,
            None,
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
    /// `Ok(None)` means the initially selected CPU backend cannot run a
    /// resident chain. Every selected GPU span, including diagnostic taps,
    /// returns its requested materialization or an error, never host fallback.
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
        if backend.is_gpu() {
            let retained = self
                .process_retained_with_taps(
                    image,
                    RetainedRender::FULL,
                    backend,
                    Some(inject),
                    Some(collect),
                )?
                .ok_or_else(|| "selected GPU declined the retained pipeline route".to_owned())?;
            return backend.materialize_retained(&retained).map(Some);
        }
        if inject != Tap::RgbIn || collect != Tap::RgbOut {
            if let Some(context) = backend.observation_context() {
                context.decline_resident(
                    spektrafilm_backend::telemetry::ResidentDeclineReason::DiagnosticTapRoute,
                );
            }
            return Ok(None);
        }
        if self.tc_lut.is_none() && self.mallett_core.is_none() {
            if let Some(context) = backend.observation_context() {
                context.decline_resident(
                    spektrafilm_backend::telemetry::ResidentDeclineReason::MissingResidentFrontPass,
                );
            }
            return Ok(None);
        }
        tracing::info!(
            backend = backend.name(),
            "pipeline: borrowed resident start"
        );
        // Selected GPU processing uses retained preparation and composition;
        // only an initially selected CPU can reach the host reference below.
        self.ensure_gpu_coverage(backend)?;
        let source = if self.params.output_mix.amount < 1.0 {
            Some(self.prepare_output_mix_source(image, backend)?)
        } else {
            None
        };
        // The device meter records its own GPU executor observation, so this
        // scope only exists to bound the host reference meter's stage.
        let metering = crate::stages::StageObservation::new(backend, "metering");
        let ae_ev = self.meter_autoexposure(image, backend)?;
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
        let out = match self.try_gpu_resident(&working, backend, pixel_size_um, 0.0)? {
            Some(out) => out,
            None => return Ok(None),
        };
        // Post-scan finishing already ran as pipe-tail passes on the
        // resident chain (pipe architecture, S4); the readback is final.
        Ok(Some(self.compose_output_mix(out, source, backend, None)?))
    }
}
