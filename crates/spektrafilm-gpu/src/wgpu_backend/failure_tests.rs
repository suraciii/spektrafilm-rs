//! A selected automatic WGPU backend must fail closed, even without telemetry.
//!
//! These tests inject a per-instance device resource failure at the actual
//! dispatch/preparation boundary. No CPU reference image is computed: success
//! (including an optional device hook declining with `Ok(None)`) is a failure.

use super::*;
use spektrafilm_backend::{EnlargerLutGpuParams, OpticalDiffusionGpuParams};
use spektrafilm_math::precision::from_f64;

fn source() -> ImageBuf {
    ImageBuf::from_data(
        3,
        2,
        (0..18).map(|i| from_f64(0.05 + i as f64 * 0.04)).collect(),
    )
}

fn rejects_without_image<T>(
    dispatch_name: &'static str,
    error_name: &str,
    operation: impl Fn(&dyn ComputeBackend, &ImageBuf) -> Result<T, String>,
) {
    // Holds the shared serial device guard through all observed derivatives.
    let backend = crate::test_support::adapter();
    assert!(backend.is_gpu());
    assert!(!backend.explicit_gpu_request());
    let image = source();

    for mode in [CollectionMode::Off, CollectionMode::Summary] {
        let context = ObservationContext::new(mode);
        let stage = context.scope(
            "injected_device_failure",
            ObservationKind::Stage,
            Purpose::Image,
        );
        let observed = backend
            .with_observation_context(stage.context().clone())
            .expect("WGPU must preserve its selected device when rebinding telemetry");
        assert!(observed.is_gpu());
        assert!(!observed.explicit_gpu_request());
        assert_eq!(
            observed.observation_context().unwrap().enabled(),
            mode != CollectionMode::Off
        );

        backend.pipe.test_fail_operation(dispatch_name);
        let error = match operation(observed.as_ref(), &image) {
            Err(error) => error,
            Ok(_) => panic!(
                "{dispatch_name} returned an image or CPU-fallback permission after device failure ({mode:?})"
            ),
        };
        assert!(
            error.contains(error_name),
            "{dispatch_name} lost its operation name: {error}"
        );
        // This counter belongs to the pipe, not the telemetry collector. It
        // proves the Off route attempted the device operation instead of
        // simply declining the hook before reaching the injected failure.
        assert_eq!(
            backend.pipe.test_failure_attempts(),
            1,
            "{dispatch_name} did not attempt its device operation ({mode:?})"
        );
        drop(observed);
        drop(stage);

        if mode != CollectionMode::Off {
            let snapshot = context.snapshot();
            assert!(
                snapshot
                    .observations
                    .iter()
                    .any(|item| item.name == "injected_device_failure")
            );
            assert!(
                snapshot.observations.iter().all(|item| {
                    !matches!(item.executor, Some(Executor::Cpu | Executor::Mixed))
                        && item.cpu_reason.is_none()
                }),
                "{dispatch_name} recorded CPU execution after device failure: {:?}",
                snapshot.observations
            );
            assert_eq!(
                snapshot.totals.readback_count.unwrap_or(0),
                0,
                "{dispatch_name} materialized output after failure"
            );
        }
    }
}

#[test]
fn selected_auto_gaussian_preparation_failure_returns_no_single_or_batch_image() {
    rejects_without_image("gaussian_blur", "gaussian_blur", |backend, image| {
        backend.gaussian_blur(image, 1.25)
    });
    rejects_without_image(
        "gaussian_blur_multi",
        "gaussian_blur_multi",
        |backend, image| backend.gaussian_blur_multi(image, &[0.75, 3.5]),
    );
}

#[test]
fn telemetry_off_and_summary_attempt_diffusion_and_both_lut_implementations() {
    let psf = std::array::from_fn(|_| vec![1.0 / 9.0; 9]);
    let params = OpticalDiffusionGpuParams {
        psf: &psf,
        radius: 1,
        scatter_fraction: 0.4,
    };
    rejects_without_image(
        "optical_diffusion",
        "optical_diffusion",
        |backend, image| backend.optical_diffusion(image, &params),
    );
    let lut = Lut3D {
        size: 2,
        data: vec![0.5; 24],
    };
    rejects_without_image("lut3d_interp", "lut3d_interp", |backend, image| {
        backend.lut3d_interp(image, &lut)
    });
    let lattice = [0.5; 24];
    let slopes = [0.0; 24];
    let params = EnlargerLutGpuParams {
        lut: &lattice,
        slope_x: &slopes,
        slope_y: &slopes,
        slope_z: &slopes,
        cell_min: &[0.0; 3],
        cell_max: &[1.0; 3],
        size: 2,
        data_min: [0.0; 3],
        data_scale: [1.0; 3],
    };
    rejects_without_image(
        "enlarger_lut_interp",
        "enlarger_lut_interp",
        |backend, image| backend.enlarger_lut_interp(image, &params),
    );
}

#[test]
fn selected_auto_shared_dispatch_failure_returns_no_host_image() {
    rejects_without_image("film_density", "density_curve_interp", |backend, image| {
        backend.density_curve_interp(
            image,
            &[-1.0, 0.0, 1.0],
            &[[0.0; 3], [0.5; 3], [1.0; 3]],
            1.0,
        )
    });
}

#[test]
fn optical_trait_hooks_produce_identical_device_pixels_with_telemetry_off_or_summary() {
    let backend = crate::test_support::adapter();
    assert!(backend.is_gpu());
    assert!(!backend.explicit_gpu_request());
    let image = source();
    let psf = std::array::from_fn(|_| vec![1.0 / 9.0; 9]);
    let diffusion = OpticalDiffusionGpuParams {
        psf: &psf,
        radius: 1,
        scatter_fraction: 0.4,
    };
    let mut lattice = Vec::new();
    for x in 0..2 {
        for y in 0..2 {
            for z in 0..2 {
                lattice.extend([x as f64 * 0.8, y as f64 * 0.7, z as f64 * 0.6]);
            }
        }
    }
    let prepared = spektrafilm_math::pchip3d::prepare_pchip_3d(lattice, 2);
    let lut = EnlargerLutGpuParams {
        lut: &prepared.lut,
        slope_x: &prepared.slope_x,
        slope_y: &prepared.slope_y,
        slope_z: &prepared.slope_z,
        cell_min: &prepared.cell_min,
        cell_max: &prepared.cell_max,
        size: prepared.size as u32,
        data_min: [0.0; 3],
        data_scale: [1.0; 3],
    };
    let mut off_pixels = None;
    for mode in [CollectionMode::Off, CollectionMode::Summary] {
        let context = ObservationContext::new(mode);
        let stage = context.scope(
            "optical_device_success",
            ObservationKind::Stage,
            Purpose::Image,
        );
        let observed = backend
            .with_observation_context(stage.context().clone())
            .unwrap();
        let outputs = [
            observed
                .optical_diffusion(&image, &diffusion)
                .expect("valid device diffusion must execute")
                .expect("telemetry must not disable the diffusion hook"),
            observed
                .enlarger_lut_interp(&image, &lut)
                .expect("valid prepared device LUT must execute")
                .expect("telemetry must not disable the enlarger LUT hook"),
        ];
        for output in &outputs {
            assert_eq!((output.width, output.height), (image.width, image.height));
            assert_eq!(output.data.len(), image.data.len());
            assert!(output.data.iter().all(|value| value.is_finite()));
            assert!(output.data.iter().any(|value| *value > from_f64(0.0)));
        }
        drop(observed);
        drop(stage);
        if mode == CollectionMode::Off {
            off_pixels = Some(outputs);
        } else {
            for (off, summary) in off_pixels.as_ref().unwrap().iter().zip(&outputs) {
                assert_eq!(
                    off.data, summary.data,
                    "telemetry changed device output pixels"
                );
            }
            let snapshot = context.snapshot();
            assert!(snapshot.observations.iter().all(|item| {
                !matches!(item.executor, Some(Executor::Cpu | Executor::Mixed))
                    && item.cpu_reason.is_none()
            }));
            assert!(
                snapshot
                    .observations
                    .iter()
                    .any(|item| item.executor == Some(Executor::Gpu))
            );
            assert_eq!(snapshot.totals.readback_count, Some(2));
            assert!(snapshot.totals.dispatch_count.unwrap_or(0) >= 2);
        }
    }
}
