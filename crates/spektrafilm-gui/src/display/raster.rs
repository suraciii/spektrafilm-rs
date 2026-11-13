use egui::{Pos2, Rect};
use spektrafilm_backend::ComputeBackend;
#[cfg(test)]
use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_math::image::ImageBuf;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    Nearest,
    Linear,
    Cubic,
    Spline16,
    #[default]
    Spline36,
    Lanczos,
    Blackman,
}
impl Interpolation {
    pub fn parse(s: &str) -> Self {
        match s {
            "nearest" => Self::Nearest,
            "linear" => Self::Linear,
            "cubic" => Self::Cubic,
            "spline16" => Self::Spline16,
            "lanczos" => Self::Lanczos,
            "blackman" => Self::Blackman,
            _ => Self::Spline36,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Linear => "linear",
            Self::Cubic => "cubic",
            Self::Spline16 => "spline16",
            Self::Spline36 => "spline36",
            Self::Lanczos => "lanczos",
            Self::Blackman => "blackman",
        }
    }
    /// The device discriminant for this mode. The viewer's device kernels
    /// select the kernel weights and the radius from this value, so the
    /// numbering is part of the device ABI: it follows `INTERPOLATIONS` order,
    /// and a device kernel that reordered them would change what the viewer
    /// shows.
    pub fn discriminant(self) -> u32 {
        match self {
            Self::Nearest => 0,
            Self::Linear => 1,
            Self::Cubic => 2,
            Self::Spline16 => 3,
            Self::Spline36 => 4,
            Self::Lanczos => 5,
            Self::Blackman => 6,
        }
    }
    pub(crate) fn radius(self) -> i32 {
        match self {
            Self::Nearest | Self::Linear => 1,
            Self::Cubic | Self::Spline16 => 2,
            Self::Spline36 => 3,
            _ => 4,
        }
    }
    pub fn weight(self, x: f32) -> f32 {
        let x = x.abs();
        if x >= self.radius() as f32 {
            return 0.0;
        }
        match self {
            Self::Nearest => {
                if x < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Linear => 1.0 - x,
            Self::Cubic => {
                if x < 1.0 {
                    (4.0 - 6.0 * x * x + 3.0 * x * x * x) / 6.0
                } else {
                    (2.0 - x).powi(3) / 6.0
                }
            }
            Self::Spline16 => {
                if x < 1.0 {
                    ((x - 1.8) * x - 0.2) * x + 1.0
                } else {
                    let y = x - 1.0;
                    ((-y / 3.0 + 0.8) * y - 7.0 / 15.0) * y
                }
            }
            Self::Spline36 => {
                if x < 1.0 {
                    ((13.0 / 11.0 * x - 453.0 / 209.0) * x - 3.0 / 209.0) * x + 1.0
                } else if x < 2.0 {
                    let y = x - 1.0;
                    ((-6.0 / 11.0 * y + 270.0 / 209.0) * y - 156.0 / 209.0) * y
                } else {
                    let y = x - 2.0;
                    ((y / 11.0 - 45.0 / 209.0) * y + 26.0 / 209.0) * y
                }
            }
            Self::Lanczos | Self::Blackman => {
                if x < 1e-7 {
                    return 1.0;
                }
                let p = std::f32::consts::PI * x;
                let q = p / 4.0;
                let window = if self == Self::Lanczos {
                    q.sin() / q
                } else {
                    0.42 + 0.5 * q.cos() + 0.08 * (2.0 * q).cos()
                };
                p.sin() / p * window
            }
        }
    }
}

#[derive(Clone)]
pub struct DisplayRaster {
    pub size: [usize; 2],
    pub rgb: Arc<[[f32; 3]]>,
    /// Device-only disposable pixels. `rgb` is empty for this variant; only
    /// the native presentation boundary may materialize its packed bytes.
    pub(crate) retained: Option<spektrafilm_backend::DeviceImage>,
}
impl DisplayRaster {
    pub fn new(size: [usize; 2], rgb: Vec<[f32; 3]>) -> Result<Self, String> {
        if size.contains(&0) || size[0].checked_mul(size[1]) != Some(rgb.len()) {
            return Err("Invalid viewer raster dimensions".into());
        }
        Ok(Self {
            size,
            rgb: rgb.into(),
            retained: None,
        })
    }
    pub(crate) fn device(image: spektrafilm_backend::DeviceImage) -> Self {
        Self {
            size: [image.width() as usize, image.height() as usize],
            rgb: Arc::from([]),
            retained: Some(image),
        }
    }
    pub fn sample(&self, x: f32, y: f32, mode: Interpolation) -> [f32; 3] {
        assert!(
            self.retained.is_none(),
            "device viewer raster must be sampled on its backend"
        );
        sample_f32(x, y, mode, |x, y| {
            self.rgb[y.clamp(0, self.size[1] as i32 - 1) as usize * self.size[0]
                + x.clamp(0, self.size[0] as i32 - 1) as usize]
        })
    }
}

fn floor_snap(x: f32) -> i32 {
    let base = x.floor();
    let frac = x - base;
    // Devices may round the shifted coordinate one ulp below an exact
    // integer. Snap such near-boundary values up so the host and device
    // select the same cell.
    let tolerance = 2.0 * f32::EPSILON * x.abs().max(1.0);
    if frac >= 1.0 - tolerance {
        (base + 1.0) as i32
    } else {
        base as i32
    }
}

pub(super) fn sample_f32<F>(x: f32, y: f32, mode: Interpolation, mut pixel: F) -> [f32; 3]
where
    F: FnMut(i32, i32) -> [f32; 3],
{
    if mode == Interpolation::Nearest {
        return pixel(floor_snap(x + 0.5), floor_snap(y + 0.5));
    }
    let r = mode.radius();
    let bx = x.floor() as i32;
    let by = y.floor() as i32;
    let mut wx = [0.0; 8];
    let mut wy = [0.0; 8];
    for i in 0..2 * r {
        wx[i as usize] = mode.weight(x - (bx + i - r + 1) as f32);
        wy[i as usize] = mode.weight(y - (by + i - r + 1) as f32);
    }
    let mut out = [0.0; 3];
    for j in 0..2 * r {
        for i in 0..2 * r {
            let w = wx[i as usize] * wy[j as usize];
            let p = pixel(bx + i - r + 1, by + j - r + 1);
            for c in 0..3 {
                out[c] += w * p[c];
            }
        }
    }
    out
}

/// Average source cells covered by a presentation pixel. Coordinates use the
/// same pixel centers as interpolation; callers clamp fetched edge samples.
pub(super) fn sample_area_f32(
    x: f32,
    y: f32,
    footprint: [f32; 2],
    mut pixel: impl FnMut(i32, i32) -> [f32; 3],
) -> [f32; 3] {
    let [width, height] = footprint.map(|side| side.max(1.0));
    let left = x + 0.5 - width * 0.5;
    let top = y + 0.5 - height * 0.5;
    let right = left + width;
    let bottom = top + height;
    let mut out = [0.0f64; 3];
    for sy in top.floor() as i32..bottom.ceil() as i32 {
        let wy = (bottom.min(sy as f32 + 1.0) - top.max(sy as f32)) / height;
        for sx in left.floor() as i32..right.ceil() as i32 {
            let wx = (right.min(sx as f32 + 1.0) - left.max(sx as f32)) / width;
            let value = pixel(sx, sy);
            let weight = f64::from(wx) * f64::from(wy);
            for c in 0..3 {
                out[c] += weight * f64::from(value[c]);
            }
        }
    }
    out.map(|value| value as f32)
}

pub(super) fn sample_footprint_f32(
    x: f32,
    y: f32,
    mode: Interpolation,
    footprint: [f32; 2],
    pixel: impl FnMut(i32, i32) -> [f32; 3],
) -> [f32; 3] {
    if mode != Interpolation::Nearest && footprint.iter().any(|&side| side > 1.0) {
        sample_area_f32(x, y, footprint, pixel)
    } else {
        sample_f32(x, y, mode, pixel)
    }
}

pub(super) fn sample_rect(
    raster: &DisplayRaster,
    p: Pos2,
    rect: Rect,
    mode: Interpolation,
) -> [f32; 3] {
    let uv = (p - rect.min) / rect.size();
    raster.sample(
        uv.x * raster.size[0] as f32 - 0.5,
        uv.y * raster.size[1] as f32 - 0.5,
        mode,
    )
}

#[cfg(test)]
pub(super) fn capped_raster(image: &ImageBuf, max_edge: usize) -> Result<DisplayRaster, String> {
    capped_raster_with(&CpuBackend, image, max_edge)
}

/// The viewer's capped raster with a device route: the destination size is
/// resolved here in f64 so the viewer owns the geometry, and source-cell area
/// averaging runs on the compute device when the backend provides a
/// device resampler. The source image is read-only, so this never changes a
/// retained or saved sample.
pub(super) fn capped_raster_with(
    backend: &dyn ComputeBackend,
    image: &ImageBuf,
    max_edge: usize,
) -> Result<DisplayRaster, String> {
    if image.width == 0 || image.height == 0 {
        return Err("Cannot view an empty image".into());
    }
    let max_edge = max_edge.max(1);
    let w = image.width as usize;
    let h = image.height as usize;
    let factor = (max_edge as f64 / w.max(h) as f64).min(1.0);
    let size = [
        ((w as f64 * factor).round() as usize).max(1),
        ((h as f64 * factor).round() as usize).max(1),
    ];
    let device = backend
        .viewer_resample(
            image,
            &spektrafilm_backend::ViewerResampleGpuParams {
                width: image.width,
                height: image.height,
                out_width: size[0] as u32,
                out_height: size[1] as u32,
            },
        )
        .map_err(|error| format!("viewer resample: {error}"))?;
    let rgb = match device {
        Some(rgb) => rgb,
        None => host_capped_raster_pixels(image, &size, w, h),
    };
    DisplayRaster::new(size, rgb)
}

/// The host reference reduction, kept as the fallback for a backend without a
/// device resampler and as the comparison target for the device test.
fn host_capped_raster_pixels(
    image: &ImageBuf,
    size: &[usize; 2],
    w: usize,
    h: usize,
) -> Vec<[f32; 3]> {
    let footprint = [w as f32 / size[0] as f32, h as f32 / size[1] as f32];
    let mut rgb = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        let sy = (y as f32 + 0.5) * footprint[1] - 0.5;
        for x in 0..size[0] {
            let sx = (x as f32 + 0.5) * footprint[0] - 0.5;
            rgb.push(sample_area_f32(sx, sy, footprint, |x, y| {
                image
                    .get(
                        x.clamp(0, w as i32 - 1) as u32,
                        y.clamp(0, h as i32 - 1) as u32,
                    )
                    .map(spektrafilm_math::precision::to_f32)
            }));
        }
    }
    rgb
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_kernel_weights_and_boundary_sampling_are_stable() {
        let modes = [
            (Interpolation::Nearest, 1.0),
            (Interpolation::Linear, 0.75),
            (Interpolation::Cubic, 0.6119791667),
            (Interpolation::Spline16, 0.853125),
            (Interpolation::Spline36, 0.8794108852),
            (Interpolation::Lanczos, 0.8945424536),
            (Interpolation::Blackman, 0.8861840535),
        ];
        for (mode, expected) in modes {
            assert!((mode.weight(0.25) - expected).abs() < 1e-6);
        }
        let raster =
            DisplayRaster::new([2, 2], vec![[0.0; 3], [1.0; 3], [0.5; 3], [0.25; 3]]).unwrap();
        assert_eq!(raster.sample(0.49, 0.0, Interpolation::Nearest), [0.0; 3]);
        assert_eq!(raster.sample(0.5, 0.0, Interpolation::Nearest), [1.0; 3]);
        assert_eq!(
            raster.sample(0.25, 0.25, Interpolation::Linear),
            [0.296875; 3]
        );
        assert_eq!(raster.sample(1.5, 1.5, Interpolation::Linear), [0.25; 3]);
        let samples = [
            (Interpolation::Nearest, [0.0, 0.5, 0.25]),
            (Interpolation::Linear, [0.0, 0.390625, 0.25]),
            (
                Interpolation::Cubic,
                [0.0307074636, 0.3878919780, 0.2702907622],
            ),
            (
                Interpolation::Spline16,
                [-0.1195312589, 0.3957519829, 0.1679687798],
            ),
            (
                Interpolation::Spline36,
                [-0.1601995230, 0.3963389993, 0.1391425580],
            ),
            (
                Interpolation::Lanczos,
                [-0.1969356239, 0.3968215585, 0.1140105054],
            ),
            (
                Interpolation::Blackman,
                [-0.1576106548, 0.3975127935, 0.1413212866],
            ),
        ];
        for (mode, expected) in samples {
            for ((x, y), expected) in [(-0.5, -0.5), (0.25, 0.75), (1.5, 1.5)]
                .into_iter()
                .zip(expected)
            {
                for actual in raster.sample(x, y, mode) {
                    assert!((actual - expected).abs() < 1e-6);
                }
            }
        }
    }
    #[test]
    fn area_reduction_covers_fractional_cells_and_repeats_the_edge() {
        let sample = |x: i32, _: i32| [if x <= 0 { 1.0 } else { 0.0 }; 3];
        for (x, footprint, expected) in [
            (0.25, [2.5, 1.0], 0.6),
            (0.5, [2.0, 1.0], 0.5),
            (-0.25, [1.5, 1.0], 1.0),
        ] {
            for value in sample_area_f32(x, 0.0, footprint, sample) {
                assert!((value - expected).abs() < 1e-6);
            }
        }
        let image = ImageBuf::from_data(
            99,
            3,
            (0..99 * 3)
                .flat_map(|i| [(i % 3 == 2) as u8 as _; 3])
                .collect(),
        );
        let reduced = capped_raster(&image, 33).unwrap();
        for value in reduced.rgb.iter().flatten() {
            assert!((value - 1.0 / 3.0).abs() < 1e-6);
        }
    }

    #[test]
    fn million_cell_area_preserves_constant_multichannel_brightness() {
        let expected = [0.2, 0.5, 0.9];
        let actual = sample_area_f32(499.5, 499.5, [1000.0, 1000.0], |_, _| expected);
        for (got, want) in actual.into_iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "{got} != {want}");
        }
    }

    #[test]
    fn explicit_native_footprints_keep_selected_kernels() {
        let footprint = [1.0, 1.0];
        let raster =
            DisplayRaster::new([2, 2], vec![[0.0; 3], [1.0; 3], [0.5; 3], [0.25; 3]]).unwrap();
        let pixel =
            |x: i32, y: i32| raster.rgb[y.clamp(0, 1) as usize * 2 + x.clamp(0, 1) as usize];
        for (mode, expected) in [
            (Interpolation::Cubic, 0.3878919780),
            (Interpolation::Spline16, 0.3957519829),
            (Interpolation::Spline36, 0.3963389993),
            (Interpolation::Lanczos, 0.3968215585),
        ] {
            for got in sample_footprint_f32(0.25, 0.75, mode, footprint, pixel) {
                assert!(
                    (got - expected).abs() < 1e-6,
                    "{mode:?}: {got} != {expected}"
                );
            }
        }
        let reduced = [2.0, 2.0];
        for mode in [
            Interpolation::Cubic,
            Interpolation::Spline36,
            Interpolation::Lanczos,
        ] {
            for got in sample_footprint_f32(0.5, 0.5, mode, reduced, pixel) {
                assert!((got - 0.4375).abs() < 1e-6);
            }
        }
    }

    /// Both processors integrate the same fractional source-cell areas. The
    /// host accumulates in binary64, while the device uses binary32.
    #[test]
    fn device_resample_matches_the_host_reference_within_f32() {
        use spektrafilm_math::precision::from_f64;

        // The software Vulkan driver is not safe under concurrent device use
        // in one process; the returned guard serializes every device test in
        // this binary and must outlive the backend.
        let backend = spektrafilm_gpu::test_support::adapter();
        let (w, h) = (37u32, 23u32);
        // A chromatic, out-of-range wedge exercises fractional-cell weights
        // and preserves the display-side headroom before its own clamp.
        let image = ImageBuf::from_data(
            w,
            h,
            (0..w * h)
                .flat_map(|i| {
                    let x = (i % w) as f64 / (w - 1) as f64;
                    let y = (i / w) as f64 / (h - 1) as f64;
                    // Values above 1.0 exercise the display-side headroom the
                    // viewer can carry before its own clamp.
                    let encode = |v: f64| from_f64(1.4 * v - 0.2);
                    [encode(x), encode(y * 0.8 + 0.1), encode(x * y + 0.05)]
                })
                .collect(),
        );
        let max_edge = 8usize;
        let device = capped_raster_with(backend.backend(), &image, max_edge)
            .expect("the device resampler provides the capped raster");
        let host = host_capped_raster_pixels(&image, &device.size, w as usize, h as usize);
        assert_eq!(device.size, [8, 5]);
        let mut worst = 0.0f64;
        for (got, want) in device.rgb.iter().zip(host.iter()) {
            for c in 0..3 {
                worst = worst.max((f64::from(got[c]) - f64::from(want[c])).abs());
            }
        }
        // Allow for the device's binary32 area-weight accumulation.
        assert!(
            worst < 1e-6,
            "the device resample differs from the host reference by {worst:.3e}"
        );
        // The source image is read-only: the resample never rewrites a sample
        // the pipeline retained or saved.
        let untouched: Vec<_> = (0..w * h)
            .flat_map(|i| {
                let x = (i % w) as f64 / (w - 1) as f64;
                let y = (i / w) as f64 / (h - 1) as f64;
                let encode = |v: f64| from_f64(1.4 * v - 0.2);
                [encode(x), encode(y * 0.8 + 0.1), encode(x * y + 0.05)]
            })
            .collect();
        assert_eq!(image.data, untouched);

        // A CPU backend reports no device path and keeps the host reference,
        // which is the fallback a device-less selection must exercise.
        let fallback = capped_raster_with(
            &spektrafilm_backend::cpu_backend::CpuBackend,
            &image,
            max_edge,
        )
        .unwrap();
        assert_eq!(&*fallback.rgb, &host[..]);
    }
}
