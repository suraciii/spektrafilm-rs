//! One device-owned prepared acquisition, independent of operator and output tone.
//!
//! The retained density owner is deliberately part of the entry: working scan
//! buffers are mutable private copies, and an unowned address can be reused.
//! Owning the original payload makes pointer equality an immutable-frame key.
use std::sync::{Arc, Mutex};

use spektrafilm_backend::{DeviceImagePayload, ScannerDensityGpuParams};

use super::{WgpuBackend, scan::PreparedScan};

#[derive(Default)]
pub(super) struct ScanCache {
    entry: Mutex<Option<Entry>>,
}

struct Entry {
    source: Arc<dyn DeviceImagePayload>,
    acquisition: Acquisition,
    prepared: Arc<PreparedScan>,
}

struct Acquisition {
    width: u32,
    height: u32,
    binding_limit: u64,
    density: Vec<f64>,
    exposure: Vec<f64>,
    offsets: [u32; 3],
    counts: [u32; 3],
    lens_blur: f32,
    standard_setup: bool,
    aim_density: f64,
    gamma: f64,
}

impl Acquisition {
    fn new(p: &ScannerDensityGpuParams, width: u32, height: u32, binding_limit: u64) -> Self {
        Self {
            width,
            height,
            binding_limit,
            density: p.density.clone(),
            exposure: p.exposure.clone(),
            offsets: p.offsets,
            counts: p.counts,
            lens_blur: p.lens_blur,
            standard_setup: p.standard_setup,
            aim_density: p.setup_aim_density,
            gamma: p.setup_gamma,
        }
    }

    fn matches(
        &self,
        p: &ScannerDensityGpuParams,
        width: u32,
        height: u32,
        binding_limit: u64,
    ) -> bool {
        self.width == width
            && self.height == height
            && self.binding_limit == binding_limit
            && self.density == p.density
            && self.exposure == p.exposure
            && self.offsets == p.offsets
            && self.counts == p.counts
            && self.lens_blur == p.lens_blur
            && self.standard_setup == p.standard_setup
            && self.aim_density == p.setup_aim_density
            && self.gamma == p.setup_gamma
    }
}

impl WgpuBackend {
    pub(super) fn cached_prepared_scan(
        &self,
        source: &Arc<dyn DeviceImagePayload>,
        p: &ScannerDensityGpuParams,
        width: u32,
        height: u32,
    ) -> Option<Arc<PreparedScan>> {
        if source.backend_id() != self.backend_id || self.check_cancelled().is_err() {
            return None;
        }
        let binding_limit = super::bounded::storage_binding_limit(self).ok()?;
        let cache = self.scan_cache.entry.lock().unwrap();
        cache
            .as_ref()
            .filter(|entry| {
                Arc::ptr_eq(&entry.source, source)
                    && entry.acquisition.matches(p, width, height, binding_limit)
            })
            .map(|entry| Arc::clone(&entry.prepared))
    }

    /// Call only after reconstruction, setup and their bounded status check
    /// succeeded. A cancelled operation cannot publish partially prepared data.
    pub(super) fn publish_prepared_scan(
        &self,
        source: Arc<dyn DeviceImagePayload>,
        p: &ScannerDensityGpuParams,
        width: u32,
        height: u32,
        prepared: Arc<PreparedScan>,
    ) -> Result<(), String> {
        self.check_cancelled()?;
        if source.backend_id() != self.backend_id {
            return Err("prepared scanner acquisition belongs to a different device".into());
        }
        let binding_limit = super::bounded::storage_binding_limit(self)?;
        let acquisition = Acquisition::new(p, width, height, binding_limit);
        let mut cache = self.scan_cache.entry.lock().unwrap();
        self.check_cancelled()?;
        // Existing in-flight renders keep their own Arc; eviction only drops
        // this cache's reference, never resources still used by a submission.
        *cache = Some(Entry {
            source,
            acquisition,
            prepared,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::film_chain::device_image::BufferPayload;
    use super::*;
    use spektrafilm_backend::{DeviceImage, SampleDomain};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct RestoreLimit(Option<std::ffi::OsString>);
    impl Drop for RestoreLimit {
        fn drop(&mut self) {
            unsafe {
                match &self.0 {
                    Some(value) => std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", value),
                    None => std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"),
                }
            }
        }
    }

    fn params() -> ScannerDensityGpuParams {
        ScannerDensityGpuParams {
            density: vec![-2.0, 4.0, -2.0, 4.0, -2.0, 4.0],
            exposure: vec![-2.0, 4.0, -2.0, 4.0, -2.0, 4.0],
            offsets: [0, 2, 4],
            counts: [2; 3],
            contrast: [0.9, 1.1, 1.2],
            dmax: [1.6, 1.8, 2.0],
            brightness_ev: [-0.4, 0.1, 0.2],
            denominator: [1.6f64, 1.8, 2.0].map(|d| -(-std::f64::consts::LN_10 * d).exp_m1()),
            camera_ev: 0.0,
            lens_blur: 0.0,
            standard_setup: true,
            setup_aim_density: 0.8,
            setup_gamma: 1.0,
            operator_pivot: -0.2,
            operator_contrast_delta: 0.15,
            operator_exposure_ev: -0.3,
            operator_color: [0.1, -0.2, 0.1],
            tone_to_output: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    fn source(gpu: &WgpuBackend, values: &[f32]) -> DeviceImage {
        let pages = gpu
            .allocate_image_pages(7, 6, "scan_cache_fixture")
            .unwrap();
        assert_eq!(values.len(), 7 * 6 * 3);
        for page in &pages {
            let first = page.first_pixel as usize * 3;
            let end = first + page.n_pixels as usize * 3;
            gpu.queue
                .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&values[first..end]));
        }
        gpu.retain_image_pages(
            pages,
            7,
            6,
            SampleDomain::Density,
            &spektrafilm_math::colorspace::SRGB,
            Some(6.0),
        )
    }

    fn run(
        gpu: &WgpuBackend,
        p: &ScannerDensityGpuParams,
        values: &[f32],
        key: Option<&Arc<dyn DeviceImagePayload>>,
        expected_setup: bool,
    ) -> Result<Vec<f32>, String> {
        // Every render receives new mutable scratch, not the retained owner.
        let scratch = source(gpu, values);
        let payload = scratch
            .payload_ref()?
            .as_any()
            .downcast_ref::<BufferPayload>()
            .unwrap();
        let output = gpu.allocate_image_pages(7, 6, "scan_cache_output")?;
        let state = gpu.build_scan_density_state(p, &payload.pages, &output, 7, 6, key)?;
        assert_eq!(state.has_setup(), expected_setup);
        state.submit_optics(gpu)?;
        state.submit_setup(gpu)?;
        state.submit_inverse(gpu)?;
        state.check(gpu, p, key, 7, 6)?;
        let mut result = Vec::new();
        for page in &output {
            result.extend(
                gpu.pipe
                    .readback_buffer_as::<f32>(&page.buffer, u64::from(page.n_pixels) * 12),
            );
        }
        Ok(result)
    }

    fn interpolate(value: f64, x: &[f64], y: &[f64]) -> f64 {
        let hi = x.partition_point(|&v| v < value).clamp(1, x.len() - 1);
        y[hi - 1] + (value - x[hi - 1]) / (x[hi] - x[hi - 1]) * (y[hi] - y[hi - 1])
    }

    // Independent scalar reference: raster-order statistics, sorted percentile,
    // calibrated inverse and positive tone. No GPU setup or exposures are read.
    fn reference(p: &ScannerDensityGpuParams, values: &[f32]) -> Vec<f32> {
        let axes = |c: usize| {
            let start = p.offsets[c] as usize;
            let end = start + p.counts[c] as usize;
            (&p.density[start..end], &p.exposure[start..end])
        };
        let mut neutral = Vec::new();
        let mut sums = [0.0; 3];
        for pixel in values.as_chunks::<3>().0 {
            let d = [
                f64::from(pixel[0]),
                f64::from(pixel[1]),
                f64::from(pixel[2]),
            ];
            let x: [f64; 3] = std::array::from_fn(|c| {
                let (d_axis, x_axis) = axes(c);
                interpolate(d[c], d_axis, x_axis)
            });
            let mean_x = (x[0] + x[1] + x[2]) / 3.0;
            let residual: [f64; 3] = std::array::from_fn(|c| {
                let (d_axis, x_axis) = axes(c);
                d[c] - interpolate(mean_x, x_axis, d_axis)
            });
            let n = (d[0] + d[1] + d[2]) / 3.0;
            neutral.push(n);
            sums[0] += n;
            sums[1] += (residual[0] - residual[1]) / 2.0f64.sqrt();
            sums[2] += (residual[0] + residual[1] - 2.0 * residual[2]) / 6.0f64.sqrt();
        }
        neutral.sort_by(f64::total_cmp);
        let position = (neutral.len() - 1) as f64 * 0.05;
        let lo = position.floor() as usize;
        let n5 = neutral[lo]
            + position.fract() * (neutral[(lo + 1).min(neutral.len() - 1)] - neutral[lo]);
        let min = neutral[0];
        let max = *neutral.last().unwrap();
        let w = if max - n5 >= 1.8 {
            [0.30, 0.35, 0.29]
        } else {
            [0.33, 0.30, 0.35]
        };
        let count = neutral.len() as f64;
        let en = (w[0] * (sums[0] / count) + w[1] * max + w[2] * min) / (w[0] + w[1] + w[2]);
        let dx = 0.75 * (p.setup_aim_density - en) / p.setup_gamma;
        let u = sums[1] / count;
        let v = sums[2] / count;
        let weight = (1.0 - 40.0 * (u * u + v * v)).max(0.2);
        let u = -0.9 * weight * u / p.setup_gamma;
        let v = -0.9 * weight * v / p.setup_gamma;
        let offset = [
            dx + u / 2.0f64.sqrt() + v / 6.0f64.sqrt(),
            dx - u / 2.0f64.sqrt() + v / 6.0f64.sqrt(),
            dx - 2.0 * v / 6.0f64.sqrt(),
        ];
        values
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|pixel| {
                let x: [f64; 3] = std::array::from_fn(|c| {
                    let (d_axis, x_axis) = axes(c);
                    interpolate(f64::from(pixel[c]), d_axis, x_axis) + offset[c]
                });
                let stops = x.map(|x| (x - p.operator_pivot) / std::f64::consts::LOG10_2);
                let common = p.operator_contrast_delta * ((stops[0] + stops[1] + stops[2]) / 3.0)
                    + p.operator_exposure_ev;
                let rgb: [f64; 3] = std::array::from_fn(|c| {
                    let x = p.operator_pivot
                        + std::f64::consts::LOG10_2 * (stops[c] + common + p.operator_color[c]);
                    let z = p.contrast[c] * (x / std::f64::consts::LOG10_2 + p.brightness_ev[c]);
                    let t = 1.0 / (1.0 + (-z).exp());
                    ((-std::f64::consts::LN_10 * p.dmax[c] * (1.0 - t)).exp_m1() + p.denominator[c])
                        / p.denominator[c]
                });
                p.tone_to_output
                    .map(|row| ((row[0] * rgb[0] + row[1] * rgb[1]) + row[2] * rgb[2]) as f32)
            })
            .collect()
    }

    fn assert_close(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (i, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() < 3e-6,
                "sample {i}: {actual} != {expected}"
            );
        }
    }

    #[test]
    fn prepared_acquisition_transitions_preserve_independent_numerical_results() {
        let gpu = crate::test_support::explicit_adapter();
        let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "192");
        }
        for (maximum, colored) in [(1.9, false), (2.0, false), (2.0, true)] {
            let mut p = params();
            if colored {
                p.density = vec![-1.5, 4.5, -1.9, 2.9, -1.8, 5.4];
            }
            let values: Vec<f32> = (0..42)
                .flat_map(|i| {
                    let n = match i {
                        0 => 0.0,
                        1 => 0.05,
                        2 => 0.1,
                        41 => maximum,
                        _ => 0.3 + (i - 3) as f32 * 0.03,
                    };
                    [
                        n,
                        n + if colored { 0.03 } else { 0.0 },
                        n - if colored { 0.01 } else { 0.0 },
                    ]
                })
                .collect();
            let owner = source(&gpu, &values);
            let token = owner.payload_ref().unwrap();
            let first = run(&gpu, &p, &values, Some(token), true).unwrap();
            assert_close(&first, &reference(&p, &values));
            let prepared = gpu.cached_prepared_scan(token, &p, 7, 6).unwrap();
            assert!(
                prepared.exposure.len() > 1,
                "binary64 exposure must cross physical pages"
            );
            p.operator_exposure_ev = 0.6;
            p.operator_color = [-0.1, 0.3, -0.2];
            p.contrast = [1.2, 0.8, 1.4];
            p.camera_ev = 20.0; // Standard setup ignores camera compensation.
            p.tone_to_output = [[0.9, 0.1, 0.0], [0.0, 0.9, 0.1], [0.1, 0.0, 0.9]];
            // A hit must not read/reconstruct scratch, even if it is invalid.
            let invalid_scratch = [f32::NAN; 126];
            let changed = run(&gpu, &p, &invalid_scratch, Some(token), false).unwrap();
            assert_close(&changed, &reference(&p, &values));
            assert_close(&changed, &run(&gpu, &p, &values, None, true).unwrap());
            assert!(
                first
                    .iter()
                    .zip(&changed)
                    .any(|(a, b)| (a - b).abs() > 0.01)
            );
            let derived = gpu.derived_explicit();
            assert!(Arc::ptr_eq(
                &prepared,
                &derived.cached_prepared_scan(token, &p, 7, 6).unwrap()
            ));
            let (batch, _batch_guard) = gpu.observed_batch("scan_cache_derivation");
            assert!(Arc::ptr_eq(
                &prepared,
                &batch.cached_prepared_scan(token, &p, 7, 6).unwrap()
            ));
            assert!(
                gpu.cached_prepared_scan(token, &p, 6, 7).is_none(),
                "raster geometry is part of acquisition identity"
            );
            let second_values: Vec<_> = values
                .iter()
                .enumerate()
                .map(|(i, &v)| v + if i % 3 == 0 { 0.25 } else { -0.05 })
                .collect();
            let second_owner = source(&gpu, &second_values);
            let second_token = second_owner.payload_ref().unwrap();
            let second = run(&gpu, &p, &second_values, Some(second_token), true).unwrap();
            assert_close(&second, &reference(&p, &second_values));
            assert!(
                second
                    .iter()
                    .zip(&changed)
                    .any(|(a, b)| (a - b).abs() > 0.001)
            );
            assert!(
                gpu.cached_prepared_scan(token, &p, 7, 6).is_none(),
                "one-entry cache must evict the first frame"
            );
            assert_eq!(
                prepared
                    .exposure
                    .iter()
                    .map(|page| page.n_pixels)
                    .sum::<u32>(),
                42,
                "in-flight owner survives eviction"
            );
            p.exposure[1] += 0.2;
            let axes = run(&gpu, &p, &second_values, Some(second_token), true).unwrap();
            assert_close(&axes, &reference(&p, &second_values));
            assert_close(&axes, &run(&gpu, &p, &second_values, None, true).unwrap());
            assert!(axes.iter().zip(&second).any(|(a, b)| (a - b).abs() > 1e-4));
            p.lens_blur = 0.35;
            let optics = run(&gpu, &p, &second_values, Some(second_token), true).unwrap();
            assert_close(&optics, &run(&gpu, &p, &second_values, None, true).unwrap());
            assert!(optics.iter().zip(&axes).any(|(a, b)| (a - b).abs() > 1e-5));
            assert_close(
                &run(&gpu, &p, &invalid_scratch, Some(second_token), false).unwrap(),
                &optics,
            );
            unsafe {
                std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "384");
            }
            let repartitioned = run(&gpu, &p, &second_values, Some(second_token), true).unwrap();
            assert_close(&repartitioned, &optics);
            unsafe {
                std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "192");
            }
        }
    }

    #[test]
    fn cancelled_or_failed_preparation_never_publishes_a_cache_entry() {
        let gpu = crate::test_support::explicit_adapter();
        let p = params();
        let values = [0.4f32; 126];
        let owner = source(&gpu, &values);
        let token = owner.payload_ref().unwrap();
        let scratch = source(&gpu, &values);
        let payload = scratch
            .payload_ref()
            .unwrap()
            .as_any()
            .downcast_ref::<BufferPayload>()
            .unwrap();
        let output = gpu
            .allocate_image_pages(7, 6, "scan_cache_cancelled_output")
            .unwrap();
        let state = gpu
            .build_scan_density_state(&p, &payload.pages, &output, 7, 6, Some(token))
            .unwrap();
        state.submit_optics(&gpu).unwrap();
        state.submit_setup(&gpu).unwrap();
        state.submit_inverse(&gpu).unwrap();
        let flag = Arc::new(AtomicBool::new(true));
        let mut cancelled = gpu.derived_explicit();
        cancelled.pipe = gpu.pipe.with_cancellation(Some(flag.clone()));
        cancelled.cancellation = Some(flag.clone());
        assert!(state.check(&cancelled, &p, Some(token), 7, 6).is_err());
        flag.store(false, Ordering::Relaxed);
        assert!(gpu.cached_prepared_scan(token, &p, 7, 6).is_none());
        assert!(Arc::ptr_eq(&gpu.scan_cache, &cancelled.scan_cache));
        let mut nonfinite = values;
        nonfinite[125] = f32::INFINITY;
        assert!(run(&gpu, &p, &nonfinite, Some(token), true).is_err());
        assert!(gpu.cached_prepared_scan(token, &p, 7, 6).is_none());
        let recovered = run(&gpu, &p, &values, Some(token), true).unwrap();
        assert_close(&recovered, &reference(&p, &values));
        let mut invalid_operator = params();
        invalid_operator.operator_exposure_ev = f64::INFINITY;
        assert!(run(&gpu, &invalid_operator, &values, Some(token), false).is_err());
        assert_close(
            &run(&gpu, &p, &values, Some(token), false).unwrap(),
            &recovered,
        );
    }
}
