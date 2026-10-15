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
    pub(super) fn meter_autoexposure(&self, image: &ImageBuf) -> f64 {
        if !self.params.camera.auto_exposure {
            return 0.0;
        }
        crate::stages::filming::meter_autoexposure_ev(image, &self.params)
    }

    /// Metering plus the exposure multiply, under the `metering` observation
    /// scope. Both owned-image entry points apply the metered EV immediately,
    /// so the EV itself stays internal.
<<<<<<< HEAD
    pub(super) fn expose_full_input(&self, image: ImageBuf, backend: &dyn ComputeBackend) -> ImageBuf {
        if self
            .params
            .workflow
            .route
            .starts_with("input > convert-film")
        {
=======
    pub(super) fn expose_full_input(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> ImageBuf {
        if self.params.workflow.route_enum().is_convert_film() {
>>>>>>> origin/main-0.3.4
            return image;
        }
        let metering = crate::stages::StageObservation::cpu(
            backend,
            "metering",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        let ae_ev = self.meter_autoexposure(&image);
        let image = self.apply_autoexposure(image, ae_ev);
        drop(metering);
        image
    }
}
