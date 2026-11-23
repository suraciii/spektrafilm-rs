//! Pipeline stage-topology execution.

use super::*;

impl Pipeline {
    /// Topological tap order of the active chain — the Rust spelling of
    /// upstream `_build_topology`. `io.scan_film` short-circuits the print
    /// stages exactly like the Python node list.
    pub(super) fn tap_order(&self) -> &'static [Tap] {
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
                    self.host_tc_lut()?,
                    self.mallett_core(),
                    &select_illuminant(self.front_illuminant()),
                    pixel_size_um,
                    ae_ev,
                )?;
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
                )?;
                print_stage_timing(print_timings, "filming_develop", t);
                record_stage_timing(stage_timings, "filming_develop", t);
                tracing::info!("pipeline: filming complete");
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &out);
                Ok(out)
            }
            Tap::CmyFilm => {
                if self.params.io.scan_film {
                    let t = Instant::now();
                    let out = crate::stages::scanning::scan_with_options(
                        &image,
                        &self.film,
                        &self.conversion_tones,
                        &self.params,
                        backend,
                        &self.output_gamut,
                        self.params.workflow.route_enum()
                            != crate::WorkflowRoute::ConvertFilmScanMinusBase,
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
                        self.print_illuminant_slice(),
                        self.print_exposure_factor(),
                        self.preflash_raw(),
                        pixel_size_um,
                    )?;
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
        // A selected GPU must fail rather than silently run a stage on the
        // CPU: reject an image the device cannot bind before dispatch,
        // whether the adapter was selected automatically or explicitly.
        if backend.is_gpu()
            && let Some(reason) = backend.image_binding_limit_reason(&image)
        {
            return Err(format!(
                "selected GPU exceeds the device resource limits ({reason}); retry with an \
                 explicit CPU request"
            ));
        }
        let forms_film = matches!(inject, Tap::RgbIn | Tap::RgbPre | Tap::LogEFilm);
        let _ = forms_film;
        let (mut cur, mut pixel_size_um, ae_ev) = if inject == Tap::RgbIn && cp > ip {
            crate::stages::grain_dye::validate_supported(&self.film, &self.params)?;
            let (image, ae_ev) = self.expose_full_input(image, backend)?;
            let (working, pitch) = self.rescale_working(&image, backend)?;
            let working = match working {
                std::borrow::Cow::Owned(working) => working,
                std::borrow::Cow::Borrowed(_) => image,
            };
            (working, pitch, ae_ev)
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
        let mut i = ip;
        if inject == Tap::RgbIn && cp > ip {
            i += 1;
        }
        let mut output_crop = None;
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
            if order[i] == Tap::LogEFilm && crate::stages::grain_dye::active(&self.params) {
                let t = Instant::now();
                let spatial_context = ((collect == Tap::RgbOut || material_geometry.is_some())
                    && crate::stages::grain_dye::scanner_spatial_context(&self.params))
                    || ((collect != Tap::CmyFilm || material_geometry.is_some())
                        && crate::stages::grain_dye::print_spatial_context(&self.params))
                    || (material_geometry.is_some() && collect == Tap::CmyFilm);
                let plan = crate::stages::grain_dye::readout_plan(
                    cur.width,
                    cur.height,
                    &self.params,
                    pixel_size_um,
                    spatial_context,
                )?;
                output_crop = plan.deferred_crop;
                let geometry = plan.geometry;
                let grid = plan.grid;
                // The existing scalar optical controls use the actual pitch
                // on the full source's long axis. Rounding may make the other
                // axis differ; the field itself retains both exact pitches.
                let long_axis = usize::from(cur.height > cur.width);
                let requested_pitch_um = geometry.output_pitch_um[long_axis];
                cur = crate::stages::grain_dye::readout(
                    &cur,
                    &self.film,
                    &self.params,
                    pixel_size_um,
                    grid,
                    backend,
                )?;
                pixel_size_um = requested_pitch_um;
                print_stage_timing(stage_timings_enabled(), "grain_dye_readout", t);
                record_stage_timing(&mut stage_timings, "grain_dye_readout", t);
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &cur);
            }
            if order[i] == Tap::LogEFilm && crate::stages::grain_fotufilm::active(&self.params) {
                let t = Instant::now();
                let spatial_context = ((collect == Tap::RgbOut || material_geometry.is_some())
                    && crate::stages::grain_dye::scanner_spatial_context(&self.params))
                    || ((collect != Tap::CmyFilm || material_geometry.is_some())
                        && crate::stages::grain_dye::print_spatial_context(&self.params))
                    || (material_geometry.is_some() && collect == Tap::CmyFilm);
                let plan = crate::stages::grain_dye::readout_plan(
                    cur.width,
                    cur.height,
                    &self.params,
                    pixel_size_um,
                    spatial_context,
                )?;
                output_crop = plan.deferred_crop;
                let geometry = plan.geometry;
                let grid = plan.grid;
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
                pixel_size_um = geometry.output_pitch_um[long_axis];
                record_stage_timing(&mut stage_timings, "grain_fotufilm_readout", t);
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &cur);
            }
            // The value at the next tap has settled (post-grain, post-crop
            // decision). `rgb_out` is recorded by the caller as the delivered
            // pre-saving output, including post-processing.
            i += 1;
        }
        if let Some(geometry) = material_geometry.as_mut() {
            geometry.pixel_size_um = pixel_size_um;
            geometry.pending_crop = output_crop.map(|r| [r.x, r.y, r.width, r.height]);
            return Ok(cur);
        }
        match output_crop {
            Some(rect) => crate::stages::grain_dye::crop_output_on_backend(cur, rect, backend),
            None => Ok(cur),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_film_diagnostic_taps_preserve_exposure_and_density_boundaries() {
        let data = crate::test_support::data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
        let image = ImageBuf::from_data(
            2,
            2,
            [
                0.24, 0.37, 0.51, 0.66, 0.42, 0.18, 0.3, 0.5, 0.6, 0.75, 0.6, 0.4,
            ]
            .into_iter()
            .map(from_f64)
            .collect(),
        );
        for route in [
            "input > convert-film > scan",
            "input > convert-film > scan-minus-base",
            "input > convert-film > print > scan",
        ] {
            let mut params = RuntimeParams::default();
            params.workflow.route = route.into();
            params.io.scan_film = !route.contains("print");
            params.camera.auto_exposure = false;
            params.camera.lens_blur_um = 0.0;
            params.camera.diffusion_filter.active = false;
            params.film_render.halation.active = false;
            params.film_render.grain.active = false;
            params.film_render.dir_couplers.active = false;
            let pipeline =
                Pipeline::new_with_spectral(film.clone(), print.clone(), params, &data).unwrap();
            let pitch = pipeline.params.camera.film_format_mm as f64 * 1000.0 / 2.0;
            let exposure = crate::stages::filming::expose(
                &image,
                &pipeline.film,
                &pipeline.params,
                &backend,
                pipeline.host_tc_lut().unwrap(),
                pipeline.mallett_core(),
                &select_illuminant(pipeline.front_illuminant()),
                pitch,
                0.0,
            )
            .unwrap();
            let density = crate::stages::filming::develop(
                &exposure,
                &pipeline.film,
                &pipeline.params,
                &backend,
                pitch,
            )
            .unwrap();
            for inject in [Tap::RgbIn, Tap::RgbPre] {
                for (collect, expected) in [(Tap::LogEFilm, &exposure), (Tap::CmyFilm, &density)] {
                    let actual = pipeline
                        .process_with_taps_timed(
                            image.clone(),
                            &backend,
                            Some(inject),
                            Some(collect),
                            None,
                        )
                        .unwrap();
                    assert_eq!(
                        actual.data, expected.data,
                        "{route}: {inject:?} -> {collect:?}"
                    );
                }
            }
        }
    }
}
