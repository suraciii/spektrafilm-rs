//! Pipeline auto-exposure metering and application.

use super::*;

impl Pipeline {
    pub(super) fn apply_autoexposure(&self, mut image: ImageBuf, ev: f64) -> ImageBuf {
        if self.params.camera.auto_exposure {
            let scale = from_f64(2.0f64.powf(ev));
            image.data.par_iter_mut().for_each(|value| *value *= scale);
        }
        image
    }

    /// Meter the auto-exposure EV on the given (full, pre-geometry)
    /// image when auto-exposure is enabled.
    ///
    /// Metering is a full-source reduction. A selected GPU must provide its
    /// device meter; only a CPU backend may run the host reference.
    pub(super) fn meter_autoexposure(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<f64, String> {
        if !self.params.camera.auto_exposure {
            return Ok(0.0);
        }
        let space = spektrafilm_math::colorspace::resolve(&self.params.io.input_color_space)
            .expect("validated input color space");
        let device = backend.meter_autoexposure(
            image,
            &spektrafilm_backend::MeterAutoexposureParams {
                width: image.width,
                height: image.height,
                max_size: crate::stages::filming::MAX_SIZE as u32,
                y_row: space.matrix_rgb_to_xyz[1],
                cctf_kind: self
                    .params
                    .io
                    .input_cctf_decoding
                    .then(|| spektrafilm_backend::cctf_kind(space.cctf)),
                method: self.params.camera.auto_exposure_method.clone(),
                mid_grey: crate::stages::filming::MID_GREY,
            },
        )?;
        if let Some(ev) = device {
            return Ok(ev);
        }
        if backend.is_gpu() {
            // Name the resource limit when available; a declined device
            // meter never authorizes the host reference after GPU selection.
            return Err(match backend.image_binding_limit_reason(image) {
                Some(reason) => format!(
                    "selected GPU cannot execute full-source auto-exposure metering: device resource limits: {reason}"
                ),
                None => "selected GPU cannot execute full-source auto-exposure metering".into(),
            });
        }
        let _meter = crate::stages::StageObservation::cpu(
            backend,
            "metering",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        Ok(crate::stages::filming::meter_autoexposure_ev(
            image,
            &self.params,
        ))
    }

    /// Metering plus the reference exposure multiply under the `metering`
    /// observation scope. Production selected-GPU processing keeps this
    /// multiply in the retained route; this helper serves CPU and diagnostic
    /// stage execution and records the metering actually applied.
    pub(super) fn expose_full_input(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        capture: Option<&mut crate::stage_evidence::StageCapture>,
    ) -> Result<ImageBuf, String> {
        if self.params.workflow.route_enum().is_convert_film() {
            return Ok(image);
        }
        let ae_ev = self.meter_autoexposure(&image, backend)?;
        // The device meter records its own GPU executor observation, so this
        // scope only exists to bound the host reference meter's stage.
        let metering = crate::stages::StageObservation::new(backend, "metering");
        if let Some(capture) = capture {
            capture.record_exposure_source(crate::stage_evidence::ExposureSource {
                auto_enabled: self.params.camera.auto_exposure,
                method: self.params.camera.auto_exposure_method.clone(),
                source_dimensions: [image.width, image.height],
                metered_ev: ae_ev,
                metered_ev_source: if self.params.camera.auto_exposure {
                    "measured"
                } else {
                    "disabled_zero"
                },
            });
        }
        let image = self.apply_autoexposure(image, ae_ev);
        drop(metering);
        Ok(image)
    }
}
