//! Completed material boundaries use the same topology nodes as image rendering.
use super::*;
use crate::stage_artifact::{
    EmbeddedProfile, InterpretationContext, MaterialKind, MaterialState, StageGeometry, StageHeader,
};

impl Pipeline {
    pub fn process_stage(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        seed: u64,
    ) -> Result<MaterialState, String> {
        if backend.explicit_gpu_request() {
            return Err("explicit GPU request cannot execute the complete material-boundary per-stage computation on WGPU; use an explicit CPU request".into());
        }
        let kind = self
            .params
            .workflow
            .route_enum()
            .artifact_output()
            .ok_or("image stage execution requires a route ending at film or print")?;
        if self.params.workflow.route_enum().artifact_input().is_some() {
            return Err("completed material input requires continue_stage".into());
        }
        crate::stage_execution::validate_stage_model(&self.params, kind)?;
        let source = if self.params.output_mix.amount < 1.0 {
            Some(self.prepare_output_mix_source(&image)?)
        } else {
            None
        };
        let mut geometry = StageGeometry {
            source_width: image.width,
            source_height: image.height,
            pixel_size_um: 1.0,
            pending_crop: None,
            print_spatial_completed: kind == MaterialKind::Print
                && self.params.enlarger.diffusion_filter.active,
        };
        let collect = if kind == MaterialKind::Film {
            Tap::CmyFilm
        } else {
            Tap::CmyPrint
        };
        let seeded;
        let pipeline = if self.params.random_seed == seed {
            self
        } else {
            seeded = {
                let mut pipeline = self.clone();
                pipeline.params.random_seed = seed;
                pipeline
            };
            &seeded
        };
        let metering = crate::stages::StageObservation::cpu(
            backend,
            "metering",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        let metered_ev = pipeline.meter_autoexposure(&image);
        let image = pipeline.apply_autoexposure(image, metered_ev);
        drop(metering);
        let (working, pitch) = pipeline.rescale_working(&image, backend)?;
        let working = match working {
            std::borrow::Cow::Owned(working) => working,
            std::borrow::Cow::Borrowed(_) => image,
        };
        let density = pipeline.run_from(
            working,
            Tap::RgbPre,
            collect,
            backend,
            Some((pitch, 0.0)),
            None,
            None,
            Some(&mut geometry),
        )?;
        let print = if self.params.workflow.route_enum().prints() {
            Some(
                EmbeddedProfile::with_source(self.print.clone(), &self.original_print)
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let original_print = if print.is_some() {
            Some(EmbeddedProfile::new((*self.original_print).clone()).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let context = InterpretationContext {
            exposure: crate::stage_artifact::StageExposure {
                auto_enabled: self.params.camera.auto_exposure,
                method: self.params.camera.auto_exposure_method.clone(),
                metered_ev,
            },
            camera_compensation_ev: self.params.camera.exposure_compensation_ev as f64,
            print_exposure_factor: self.print_exposure_factor,
            print_illuminant: self.print_illuminant.clone(),
            preflash_raw: self.preflash_raw,
            midgray_density: self.midgray_density.clone(),
            compensated_midgray_density: self.compensated_midgray_density.clone(),
            prepared_source_color_space: source
                .as_ref()
                .map(|_| self.params.io.output_color_space.clone()),
        };
        let film = EmbeddedProfile::with_source(self.film.clone(), &self.original_film)
            .map_err(|e| e.to_string())?;
        let mut baseline = self.params.clone();
        baseline.random_seed = seed;
        let mut header = StageHeader::new(
            kind,
            baseline,
            film,
            print,
            original_print,
            geometry,
            context,
            seed,
        );
        header.provenance.backend = backend.name().to_string();
        header.provenance.producing_route = self.params.workflow.route_enum().as_str().to_string();
        Ok(MaterialState {
            header,
            density,
            prepared_source: source,
        })
    }

    pub(crate) fn continue_material(
        state: MaterialState,
        mut params: RuntimeParams,
        selected_print: Option<Profile>,
        data_dir: &Path,
        backend: &dyn ComputeBackend,
        seed: u64,
    ) -> Result<crate::stage_execution::StageResult, String> {
        use crate::stage_execution::{StageResult, validate_continuation};
        if backend.explicit_gpu_request() {
            return Err("explicit GPU request cannot execute the complete material continuation per-stage computation on WGPU; use an explicit CPU request".into());
        }
        validate_continuation(&state.header, &params)?;
        params.random_seed = seed;
        params.io.scan_film = params.workflow.route_enum().scans_film();
        let MaterialState {
            mut header,
            mut density,
            mut prepared_source,
        } = state;
        let film = header.film.profile.clone();
        let printing = params.workflow.route_enum().prints();
        let unchanged_print = selected_print.is_none()
            && header.print.is_some()
            && serde_json::to_value(&params.enlarger).map_err(|e| e.to_string())?
                == serde_json::to_value(&header.baseline.enlarger).map_err(|e| e.to_string())?
            && serde_json::to_value(&params.print_render).map_err(|e| e.to_string())?
                == serde_json::to_value(&header.baseline.print_render)
                    .map_err(|e| e.to_string())?;
        let unchanged_print = unchanged_print
            && params.settings.print_exposure_ev_tracking
                == header.baseline.settings.print_exposure_ev_tracking
            && params.settings.neutral_print_filters_from_database
                == header.baseline.settings.neutral_print_filters_from_database;
        let mut print = header
            .print
            .as_ref()
            .map(|p| p.profile.clone())
            .unwrap_or_else(|| film.clone());
        let mut original_print = header.original_print.clone();
        let mut factor = header.context.print_exposure_factor;
        let mut illuminant = header.context.print_illuminant.clone();
        let mut preflash = header.context.preflash_raw;
        if printing && !unchanged_print {
            let source = selected_print
                .or_else(|| original_print.as_ref().map(|p| p.profile.clone()))
                .ok_or("film > print requires a printing material; select --paper")?;
            let prepared = crate::pipeline_calibration::prepare_continued_print(
                &film,
                source.clone(),
                &mut params,
                &header.context,
                data_dir,
            )?;
            print = prepared.0;
            factor = prepared.1;
            illuminant = prepared.2;
            preflash = prepared.3;
            original_print = Some(EmbeddedProfile::new(source).map_err(|e| e.to_string())?);
        }
        if let Some([x, y, width, height]) = header.geometry.pending_crop {
            let glare = &params.print_render.glare;
            let scan_spatial = params.scanner.lens_blur > 0.0
                || params.scanner.unsharp_mask.iter().all(|&v| v > 0.0)
                || (!params.io.scan_film && glare.active && glare.percent > 0.0);
            let print_spatial = printing && params.enlarger.diffusion_filter.active;
            if !scan_spatial && !print_spatial && !header.geometry.print_spatial_completed {
                density = crate::stages::grain_v3::crop_output(
                    density,
                    crate::stages::grain_v3::SourceRect {
                        x,
                        y,
                        width,
                        height,
                    },
                );
                header.geometry.pending_crop = None;
            }
        }
        let output_gamut = crate::gamut_compression::OutputGamutCompress::build(
            &params.io.output_gamut_compress,
            &params.io.output_color_space,
        )?;
        let conversion_tones = crate::stages::scanning::style::resolve(
            &params.scanner.conversion_style,
            Some(data_dir),
        )?;
        let pipeline = Self {
            original_film: Arc::new(film.clone()),
            original_print: Arc::new(print.clone()),
            fotufilm_prepared: crate::stages::grain_fotufilm::Prepared::new(
                &film,
                params.film_render.development_time,
            ),
            front_illuminant: film.info.reference_illuminant.clone(),
            film,
            print,
            params,
            tc_lut: None,
            mallett_core: None,
            print_exposure_factor: factor,
            print_illuminant: illuminant,
            preflash_raw: preflash,
            midgray_density: header.context.midgray_density.clone(),
            compensated_midgray_density: header.context.compensated_midgray_density.clone(),
            conversion_tones,
            output_gamut,
            data_dir: Some(data_dir.to_owned()),
            cancellation: None,
        };
        let inject = if header.state == MaterialKind::Film {
            Tap::CmyFilm
        } else {
            Tap::CmyPrint
        };
        let material_output = pipeline.params.workflow.route_enum().artifact_output();
        let collect = if material_output.is_some() {
            Tap::CmyPrint
        } else {
            Tap::RgbOut
        };
        let mut output = pipeline.run_from(
            density,
            inject,
            collect,
            backend,
            Some((header.geometry.pixel_size_um, 0.0)),
            None,
            None,
            None,
        )?;
        if let Some(kind) = material_output {
            header.lineage.push(crate::stage_artifact::StageLineage {
                seed: header.seed,
                provenance: header.provenance.clone(),
                effective_parameters: header.baseline.clone(),
            });
            header.geometry.print_spatial_completed =
                pipeline.params.enlarger.diffusion_filter.active;
            header.provenance.inherited_operations = vec!["film".into()];
            header.state = kind;
            header.boundary = kind.boundary().into();
            header.print = Some(
                EmbeddedProfile::with_source(
                    pipeline.print.clone(),
                    &original_print
                        .as_ref()
                        .ok_or("missing original print material")?
                        .profile,
                )
                .map_err(|e| e.to_string())?,
            );
            header.original_print = original_print;
            header.baseline = pipeline.params.clone();
            header.seed = seed;
            header.preparation = header.baseline.render_preparation_state();
            header.density = None;
            header.prepared_source = None;
            header.context.print_exposure_factor = factor;
            header.context.print_illuminant = pipeline.print_illuminant.clone();
            header.context.preflash_raw = preflash;
            header.provenance.backend = backend.name().to_string();
            header.provenance.producing_route =
                pipeline.params.workflow.route_enum().as_str().into();
            return Ok(StageResult::Material(MaterialState {
                header,
                density: output,
                prepared_source,
            }));
        }
        if let Some([x, y, width, height]) = header.geometry.pending_crop {
            output = crate::stages::grain_v3::crop_output(
                output,
                crate::stages::grain_v3::SourceRect {
                    x,
                    y,
                    width,
                    height,
                },
            );
        }
        if pipeline.params.output_mix.amount < 1.0 {
            let source = prepared_source.as_mut().ok_or("output_mix.amount below 1 requires a retained prepared-source branch; regenerate the artifact with source mixing enabled")?;
            let space = header
                .context
                .prepared_source_color_space
                .as_deref()
                .ok_or("prepared source has no color declaration")?;
            if space != pipeline.params.io.output_color_space {
                use spektrafilm_math::colorspace::{conversion_matrix, convert_rgb, resolve};
                let from = resolve(space)?;
                let to = resolve(&pipeline.params.io.output_color_space)?;
                let matrix = conversion_matrix(from, to);
                source.data.par_chunks_exact_mut(3).for_each(|p| {
                    let rgb = convert_rgb(
                        [to_f64(p[0]), to_f64(p[1]), to_f64(p[2])],
                        from,
                        false,
                        to,
                        false,
                        &matrix,
                    );
                    for c in 0..3 {
                        p[c] = from_f64(rgb[c]);
                    }
                });
            }
        }
        Ok(StageResult::Image(pipeline.compose_output_mix(
            output,
            prepared_source,
            backend,
            None,
        )?))
    }
}
