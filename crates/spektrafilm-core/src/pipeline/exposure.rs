//! Pipeline auto-exposure metering.

use super::*;

impl Pipeline {
    /// Meter the auto-exposure EV on the given (full, pre-geometry)
    /// image when auto-exposure is enabled.
    ///
    /// The reference meters the full-frame preview's **film raw**, not its
    /// display luminance: the preview runs through the same sensitivity path
    /// as the real exposure, then the raw channel mean is brought to `1.0`
    /// by the metering pattern. The CPU reference runs this bounded preview
    /// path on the host; a device meter reconstructs the preview's raw field
    /// with the prepared front-pass table before reducing it.
    pub(super) fn meter_autoexposure(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<f64, String> {
        if !self.params.camera.auto_exposure {
            return Ok(0.0);
        }
        if !backend.is_gpu() {
            let _meter = crate::stages::StageObservation::cpu(
                backend,
                "metering",
                spektrafilm_backend::telemetry::CpuReason::BackendDefault,
            );
            let cpu = spektrafilm_backend::cpu_backend::CpuBackend;
            return crate::stages::filming::meter_raw_autoexposure_ev(
                image,
                &self.params,
                &cpu,
                self.host_tc_lut()?,
                self.mallett_core(),
                &select_illuminant(self.front_illuminant()),
            );
        }
        let params = self.device_meter_params(image.width, image.height);
        let device = backend.meter_autoexposure(image, &params)?;
        if let Some(ev) = device {
            tracing::info!(
                target: "spektrafilm_core::stages::filming",
                ev,
                raw = params.raw_tc_lut.is_some() || self.mallett_core().is_some(),
                "autoexposure_device"
            );
            return Ok(ev);
        }
        // Name the resource limit when available; a declined device
        // meter never authorizes the host reference after GPU selection.
        Err(match backend.image_binding_limit_reason(image) {
            Some(reason) => format!(
                "selected GPU cannot execute full-source auto-exposure metering: device resource limits: {reason}"
            ),
            None => "selected GPU cannot execute full-source auto-exposure metering".into(),
        })
    }

    /// Build the device metering request. A device front pass can meter film
    /// raw on the preview: Hanatos uses its prepared TC table, while Mallett's
    /// raw channel mean is one linear combination of RGB.
    pub(super) fn device_meter_params(
        &self,
        width: u32,
        height: u32,
    ) -> spektrafilm_backend::MeterAutoexposureParams {
        let space = spektrafilm_math::colorspace::resolve(&self.params.io.input_color_space)
            .expect("validated input color space");
        let mut params = spektrafilm_backend::MeterAutoexposureParams {
            width,
            height,
            max_size: crate::stages::filming::MAX_SIZE as u32,
            y_row: space.matrix_rgb_to_xyz[1],
            cctf_kind: self
                .params
                .io
                .input_cctf_decoding
                .then(|| spektrafilm_backend::cctf_kind(space.cctf)),
            method: self.params.camera.auto_exposure_method.clone(),
            mid_grey: crate::stages::filming::MID_GREY,
            raw_tc_lut: None,
        };
        if let Some(core) = self.mallett_core() {
            let m = crate::mallett::film_matrix(core, &self.params.io.input_color_space);
            params.y_row = [
                (m[0][0] + m[1][0] + m[2][0]) / 3.0,
                (m[0][1] + m[1][1] + m[2][1]) / 3.0,
                (m[0][2] + m[1][2] + m[2][2]) / 3.0,
            ];
            params.mid_grey = 1.0;
        } else if let (Some(table), Some(lut)) = (self.prepared_tc_lut(), self.tc_lut()) {
            let ref_illuminant = select_illuminant(self.front_illuminant());
            params.raw_tc_lut = Some(spektrafilm_backend::MeterRawTcLut {
                table: table.clone(),
                lut_size: lut.size as u32,
                rgb_to_xyz_adapted: spektrafilm_math::spectral::build_rgb_to_adapted_xyz(
                    &self.params.io.input_color_space,
                    &ref_illuminant,
                    self.params.settings.use_cat16,
                ),
            });
            params.mid_grey = 1.0;
        }
        params
    }

    /// Meter the full input and return it with the metered EV. The exposure
    /// multiply runs later on film raw, after the working image has been
    /// converted by the spectral front end.
    pub(super) fn expose_full_input(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        capture: Option<&mut crate::stage_evidence::StageCapture>,
    ) -> Result<(ImageBuf, f64), String> {
        if !self.params.workflow.route_enum().exposes_film() {
            return Ok((image, 0.0));
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
        drop(metering);
        Ok((image, ae_ev))
    }
}
