//! Pipeline stage-topology execution.

use super::*;

impl Pipeline {
    /// Topological tap order of the active chain — the Rust spelling of
    /// upstream `_build_topology`. `io.scan_film` short-circuits the print
    /// stages exactly like the Python node list.
    fn tap_order(&self) -> &'static [Tap] {
        if self.params.io.scan_film {
            &[
                Tap::RgbIn,
                Tap::RgbPre,
                Tap::LogEFilm,
                Tap::CmyFilm,
                Tap::RgbOut,
            ]
        } else {
            &[
                Tap::RgbIn,
                Tap::RgbPre,
                Tap::LogEFilm,
                Tap::CmyFilm,
                Tap::LogEPrint,
                Tap::CmyPrint,
                Tap::RgbOut,
            ]
        }
    }

    /// One topology node: transform the image at tap `at` into its value at
    /// the next tap in [`Pipeline::tap_order`]. `RgbOut` is terminal.
    #[allow(clippy::too_many_arguments)]
    fn fire_node(
        &self,
        at: Tap,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        pixel_size_um: f64,
        ae_ev: f64,
        stage_timings: &mut Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        let print_timings = stage_timings_enabled();
        let name = match at {
            Tap::RgbPre => "filming_expose",
            Tap::LogEFilm => "filming_develop",
            Tap::CmyFilm if self.params.io.scan_film => "scanning",
            Tap::CmyFilm | Tap::LogEPrint => "printing",
            Tap::CmyPrint => "scanning",
            Tap::RgbIn | Tap::RgbOut => "unreachable_tap",
        };
        let mut observation = crate::stages::StageObservation::cpu(
            backend,
            name,
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        let backend = observation.backend(backend);
        match at {
            Tap::RgbIn => unreachable!("rgb_in preprocessing is handled by run_from"),
            Tap::RgbPre => {
                let t = Instant::now();
                let out = crate::stages::filming::expose(
                    &image,
                    &self.film,
                    &self.params,
                    backend,
                    self.tc_lut.as_ref(),
                    self.mallett_core.as_ref(),
                    &select_illuminant(&self.front_illuminant),
                    pixel_size_um,
                    ae_ev,
                );
                record_stage_timing(stage_timings, "filming_expose", t);
                dump_if_env("SPEKTRAFILM_DUMP_FILM_LOG_RAW", &out);
                Ok(out)
            }
            Tap::LogEFilm => {
                let t = Instant::now();
                let out = crate::stages::filming::develop(
                    &image,
                    &self.film,
                    &self.params,
                    backend,
                    pixel_size_um,
                );
                print_stage_timing(print_timings, "filming_develop", t);
                record_stage_timing(stage_timings, "filming_develop", t);
                tracing::info!("pipeline: filming complete");
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &out);
                Ok(out)
            }
            Tap::CmyFilm => {
                if self.params.io.scan_film {
                    let t = Instant::now();
                    let out = crate::stages::scanning::process(
                        &image,
                        &self.film,
                        &self.conversion_tones,
                        &self.params,
                        backend,
                        &self.output_gamut,
                    )?;
                    record_stage_timing(stage_timings, "scanning", t);
                    tracing::info!("pipeline: scanning complete (film scan)");
                    Ok(out)
                } else {
                    let t = Instant::now();
                    let out = crate::stages::printing::expose_calibrated(
                        &image,
                        &self.film,
                        &self.print,
                        &self.params,
                        backend,
                        &self.print_illuminant,
                        self.print_exposure_factor,
                        self.preflash_raw,
                        pixel_size_um,
                    );
                    record_stage_timing(stage_timings, "printing", t);
                    dump_if_env("SPEKTRAFILM_DUMP_PRINT_LOG_RAW", &out);
                    Ok(out)
                }
            }
            Tap::LogEPrint => {
                let t = Instant::now();
                let result =
                    crate::stages::printing::develop(&image, &self.print, &self.params, backend);
                observation.set_complete(result.is_ok());
                let out = result?;
                print_stage_timing(print_timings, "printing_develop", t);
                record_stage_timing(stage_timings, "printing_develop", t);
                dump_if_env("SPEKTRAFILM_DUMP_PRINT_DENSITY", &out);
                Ok(out)
            }
            Tap::CmyPrint => {
                let t = Instant::now();
                let out = crate::stages::scanning::process(
                    &image,
                    &self.print,
                    &self.conversion_tones,
                    &self.params,
                    backend,
                    &self.output_gamut,
                )?;
                print_stage_timing(print_timings, "scanning", t);
                record_stage_timing(stage_timings, "scanning", t);
                tracing::info!("pipeline: printing complete");
                Ok(out)
            }
            Tap::RgbOut => unreachable!("RgbOut is terminal — run_from stops before it"),
        }
    }

    pub(super) fn validate_grain_v3_taps(
        cell_spectral: bool,
        aperture_factor: Option<u32>,
        inject: Tap,
        collect: Tap,
        scan_film: bool,
    ) -> Result<(), String> {
        if !cell_spectral {
            return Ok(());
        }
        if aperture_factor.is_some() {
            return Err("SPEKTRAFILM_GRAIN_V3_CELL_SPECTRAL conflicts with SPEKTRAFILM_GRAIN_V3_APERTURE_FACTOR".to_owned());
        }
        let supported_collect = collect == Tap::RgbOut
            || (!scan_film && matches!(collect, Tap::LogEPrint | Tap::CmyPrint));
        if !matches!(inject, Tap::RgbIn | Tap::RgbPre | Tap::LogEFilm) || !supported_collect {
            return Err("SPEKTRAFILM_GRAIN_V3_CELL_SPECTRAL requires injection at or before log_e_film and optical collection (print log_e_print/cmy_print/rgb_out, film rgb_out); cmy_film is a density diagnostic".to_owned());
        }
        Ok(())
    }

    /// Run the topology from the value at `inject` to the value at
    /// `collect` (both inclusive bounds in [`Pipeline::tap_order`]).
    /// Mirrors upstream `run_topology`: unreachable pairs error with the
    /// upstream message, `inject == collect` returns the input unchanged.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_from(
        &self,
        image: ImageBuf,
        inject: Tap,
        collect: Tap,
        backend: &dyn ComputeBackend,
        physical_context: Option<(f64, f64)>,
        stage_timings: Option<&mut BTreeMap<String, f64>>,
        mut capture: Option<&mut crate::stage_evidence::StageCapture>,
        mut material_geometry: Option<&mut crate::stage_artifact::StageGeometry>,
    ) -> Result<ImageBuf, String> {
        self.check_cancelled()?;
        if matches!(inject, Tap::RgbIn | Tap::RgbPre | Tap::LogEFilm) {
            crate::stages::grain_fotufilm::validate_supported(&self.film, &self.params)?;
        }
        let order = self.tap_order();
        let Some(ip) = order.iter().position(|t| *t == inject) else {
            return Err(format!(
                "tap {inject:?} is not part of this topology (scan_film = {})",
                self.params.io.scan_film
            ));
        };
        let Some(cp) = order.iter().position(|t| *t == collect) else {
            return Err(format!(
                "tap {collect:?} is not part of this topology (scan_film = {})",
                self.params.io.scan_film
            ));
        };
        if cp < ip {
            return Err(format!(
                "no node path reaches tap {:?} from {:?}",
                collect, inject
            ));
        }
        // An explicit GPU request must fail rather than silently run a
        // stage on the CPU: reject an image the device cannot bind before
        // any stage dispatch. The auto route declines such stages to their
        // CPU implementations inside the backend.
        if backend.is_gpu()
            && backend.explicit_gpu_request()
            && let Some(reason) = backend.image_binding_limit_reason(&image)
        {
            return Err(format!(
                "explicit GPU request exceeds the device resource limits ({reason}); retry with an \
                 explicit CPU request"
            ));
        }
        let forms_film = matches!(inject, Tap::RgbIn | Tap::RgbPre | Tap::LogEFilm);
        let aperture_factor = if forms_film {
            crate::stages::grain_v3::validate_aperture(&self.params)?
        } else {
            None
        };
        let cell_spectral =
            forms_film && crate::stages::grain_v3::validate_cell_spectral(&self.params)?;
        Self::validate_grain_v3_taps(
            cell_spectral,
            aperture_factor,
            inject,
            collect,
            self.params.io.scan_film,
        )?;
        if aperture_factor.is_some()
            && (collect != Tap::RgbOut
                || ip > order.iter().position(|t| *t == Tap::LogEFilm).unwrap())
        {
            return Err("SPEKTRAFILM_GRAIN_V3_APERTURE_FACTOR requires injection at or before log_e_film and collect=rgb_out".to_owned());
        }
        if let Some(capture) = capture.as_deref_mut()
            && self.params.io.scan_film
        {
            // `io.scan_film` short-circuits the print stages.
            use crate::stage_evidence::StageId;
            capture.record_skip(StageId::LogEPrint, "no_print_stage_on_route");
            capture.record_skip(StageId::CmyPrint, "no_print_stage_on_route");
        }
        let (mut cur, mut pixel_size_um, ae_ev) = if inject == Tap::RgbIn && cp > ip {
            crate::stages::grain_v3::validate_supported(&self.film, &self.params)?;
            let image = self.expose_full_input(image, backend, capture.as_deref_mut());
            let (working, pitch) = self.rescale_working(&image, backend)?;
            let working = match working {
                std::borrow::Cow::Owned(working) => working,
                std::borrow::Cow::Borrowed(_) => image,
            };
            // The exposure multiply already ran, so the stage sees EV 0 —
            // matching upstream's prepared-input contract.
            (working, pitch, 0.0)
        } else {
            let (pitch, ae_ev) = physical_context.unwrap_or_else(|| {
                (
                    self.params.camera.film_format_mm as f64 * 1000.0
                        / image.width.max(image.height).max(1) as f64,
                    0.0,
                )
            });
            (image, pitch, ae_ev)
        };
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(cur.width, cur.height);
        }
        let mut stage_timings = stage_timings;
        let mut aperture_output_pitch_um = None;
        let mut i = ip;
        if inject == Tap::RgbIn && cp > ip {
            i += 1;
        }
        let mut output_crop = None;
        use crate::stage_evidence::CoordinateSpace;
        let mut stage_space = CoordinateSpace::Working;
        while i < cp {
            self.check_cancelled()?;
            cur = self.fire_node(
                order[i],
                cur,
                backend,
                pixel_size_um,
                ae_ev,
                &mut stage_timings,
            )?;
            if order[i] == Tap::RgbPre
                && let Some(capture) = capture.as_deref_mut()
            {
                // The filming exposure multiply runs inside the node just
                // fired; the stage evidence reports what was applied.
                capture.record_compensation(self.params.camera.exposure_compensation_ev as f64);
            }
            if order[i] == Tap::LogEFilm && crate::stages::grain_v3::active(&self.params) {
                let t = Instant::now();
                let geometry =
                    crate::stages::grain_v3::geometry(&cur, &self.params, pixel_size_um)?;
                let glare = &self.params.print_render.glare;
                let scan_spatial = (collect == Tap::RgbOut || material_geometry.is_some())
                    && (self.params.scanner.lens_blur > 0.0
                        || self.params.scanner.unsharp_mask.iter().all(|&v| v > 0.0)
                        || (!self.params.io.scan_film && glare.active && glare.percent > 0.0));
                let print_spatial = !self.params.io.scan_film
                    && (collect != Tap::CmyFilm || material_geometry.is_some())
                    && self.params.enlarger.diffusion_filter.active;
                let (grid, readout_space) = if let Some(factor) = aperture_factor {
                    (
                        crate::stages::grain_v3::aperture_grid(&geometry, factor)?,
                        CoordinateSpace::GrainAperture,
                    )
                } else if self.params.io.crop
                    && (scan_spatial || print_spatial || material_geometry.is_some())
                {
                    let (grid, crop) = geometry.full_film_grid(&cur)?;
                    output_crop = Some(crop);
                    (grid, CoordinateSpace::FilmFrame)
                } else {
                    (geometry.grid(), CoordinateSpace::Output)
                };
                // The existing scalar optical controls use the actual pitch
                // on the full source's long axis. Rounding may make the other
                // axis differ; the field itself retains both exact pitches.
                let long_axis = usize::from(cur.height > cur.width);
                let requested_pitch_um = geometry.output_pitch_um[long_axis];
                if cell_spectral {
                    cur = crate::stages::grain_v3::readout_with_response(
                        &cur,
                        &self.film,
                        &self.params,
                        pixel_size_um,
                        grid,
                        backend,
                        |cells, backend| {
                            if self.params.io.scan_film {
                                crate::stages::scanning::capture_linear(
                                    cells,
                                    &self.film,
                                    &self.params,
                                    backend,
                                )
                            } else {
                                crate::stages::printing::expose_linear_calibrated(
                                    cells,
                                    &self.film,
                                    &self.print,
                                    &self.params,
                                    backend,
                                    &self.print_illuminant,
                                    self.print_exposure_factor,
                                    self.preflash_raw,
                                )
                            }
                        },
                    )?;
                    stage_space = readout_space;
                    pixel_size_um = requested_pitch_um;
                    if let Some(capture) = capture.as_deref_mut() {
                        // The optical readout fused development and grain
                        // into one step: no developed-density boundary flows.
                        use crate::stage_evidence::StageId;
                        capture.record_skip(StageId::CmyFilm, "fused_into_grain_readout");
                    }
                    if self.params.io.scan_film {
                        let observation = crate::stages::StageObservation::new(backend, "scanning");
                        cur = crate::stages::scanning::finish_capture(
                            cur,
                            &self.film,
                            &self.params,
                            observation.backend(backend),
                            &self.output_gamut,
                        )?;
                        // The scanner tail finished the route: leave the
                        // topology at the terminal tap.
                        i = order.iter().position(|tap| *tap == Tap::RgbOut).unwrap();
                    } else {
                        cur.data.par_iter_mut().for_each(|value| {
                            *value = from_f64(to_f64(*value).log10());
                        });
                        dump_if_env("SPEKTRAFILM_DUMP_PRINT_LOG_RAW", &cur);
                        if let Some(capture) = capture.as_deref_mut() {
                            capture.record_boundary(
                                crate::stage_evidence::StageId::LogEPrint,
                                &cur,
                                stage_space,
                            );
                        }
                        i = order.iter().position(|tap| *tap == Tap::LogEPrint).unwrap();
                    }
                    print_stage_timing(stage_timings_enabled(), "grain_v3_cell_spectral", t);
                    record_stage_timing(&mut stage_timings, "grain_v3_cell_spectral", t);
                    continue;
                }
                cur = crate::stages::grain_v3::readout(
                    &cur,
                    &self.film,
                    &self.params,
                    pixel_size_um,
                    grid,
                    backend,
                )?;
                stage_space = readout_space;
                pixel_size_um = if let Some(factor) = aperture_factor {
                    aperture_output_pitch_um = Some(requested_pitch_um);
                    requested_pitch_um / factor as f64
                } else {
                    requested_pitch_um
                };
                print_stage_timing(stage_timings_enabled(), "grain_v3_readout", t);
                record_stage_timing(&mut stage_timings, "grain_v3_readout", t);
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &cur);
            }
            if order[i] == Tap::LogEFilm && crate::stages::grain_fotufilm::active(&self.params) {
                let t = Instant::now();
                let geometry =
                    crate::stages::grain_v3::geometry(&cur, &self.params, pixel_size_um)?;
                let glare = &self.params.print_render.glare;
                let scan_spatial = (collect == Tap::RgbOut || material_geometry.is_some())
                    && (self.params.scanner.lens_blur > 0.0
                        || self.params.scanner.unsharp_mask.iter().all(|&v| v > 0.0)
                        || (!self.params.io.scan_film && glare.active && glare.percent > 0.0));
                let print_spatial = !self.params.io.scan_film
                    && (collect != Tap::CmyFilm || material_geometry.is_some())
                    && self.params.enlarger.diffusion_filter.active;
                let (grid, readout_space) = if self.params.io.crop
                    && (scan_spatial || print_spatial || material_geometry.is_some())
                {
                    let (grid, crop) = geometry.full_film_grid(&cur)?;
                    output_crop = Some(crop);
                    (grid, CoordinateSpace::FilmFrame)
                } else {
                    (geometry.grid(), CoordinateSpace::Output)
                };
                let long_axis = usize::from(cur.height > cur.width);
                cur = self.fotufilm_prepared.readout(
                    &cur,
                    &self.film,
                    &self.params,
                    pixel_size_um,
                    grid,
                    backend,
                    &|| self.check_cancelled(),
                )?;
                stage_space = readout_space;
                pixel_size_um = geometry.output_pitch_um[long_axis];
                record_stage_timing(&mut stage_timings, "grain_fotufilm_readout", t);
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &cur);
            }
            if order[i] == Tap::CmyFilm
                && let Some(factor) = aperture_factor
            {
                let t = Instant::now();
                cur = crate::stages::grain_v3::collapse_log_exposure(&cur, factor)?;
                stage_space = CoordinateSpace::Output;
                pixel_size_um = aperture_output_pitch_um.ok_or_else(|| {
                    "film grain Dye aperture: missing requested output pitch".to_owned()
                })?;
                print_stage_timing(stage_timings_enabled(), "grain_v3_aperture", t);
                record_stage_timing(&mut stage_timings, "grain_v3_aperture", t);
                tracing::info!(factor, "film grain Dye: collapsed linear print exposure");
            }
            // The value at the next tap has settled (post-grain, post-crop
            // decision). `rgb_out` is recorded by the caller as the delivered
            // pre-saving output, including post-processing.
            if let Some(next) = order.get(i + 1).copied()
                && next != Tap::RgbOut
                && let Some(id) = crate::stage_evidence::StageId::of_tap(next)
                && let Some(capture) = capture.as_deref_mut()
            {
                capture.record_boundary(id, &cur, stage_space);
            }
            i += 1;
        }
        if let Some(geometry) = material_geometry.as_mut() {
            geometry.pixel_size_um = pixel_size_um;
            geometry.pending_crop = output_crop.map(|r| [r.x, r.y, r.width, r.height]);
            return Ok(cur);
        }
        Ok(match output_crop {
            Some(rect) => crate::stages::grain_v3::crop_output(cur, rect),
            None => cur,
        })
    }
}
