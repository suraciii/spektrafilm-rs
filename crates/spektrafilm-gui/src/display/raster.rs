use egui::{Pos2, Rect};
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
    fn radius(self) -> i32 {
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
}
impl DisplayRaster {
    pub fn new(size: [usize; 2], rgb: Vec<[f32; 3]>) -> Result<Self, String> {
        if size.contains(&0) || size[0].checked_mul(size[1]) != Some(rgb.len()) {
            return Err("Invalid viewer raster dimensions".into());
        }
        Ok(Self {
            size,
            rgb: rgb.into(),
        })
    }
    pub fn from_rgba(size: [usize; 2], rgba: &[u8]) -> Result<Self, String> {
        if size[0].checked_mul(size[1]).and_then(|n| n.checked_mul(4)) != Some(rgba.len()) {
            return Err("Invalid viewer RGBA dimensions".into());
        }
        Self::new(
            size,
            rgba.chunks_exact(4)
                .map(|p| {
                    [
                        p[0] as f32 / 255.0,
                        p[1] as f32 / 255.0,
                        p[2] as f32 / 255.0,
                    ]
                })
                .collect(),
        )
    }
    pub fn from_float(image: &ImageBuf) -> Result<Self, String> {
        Self::new(
            [image.width as usize, image.height as usize],
            image
                .data
                .chunks_exact(3)
                .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
                .collect(),
        )
    }
    pub fn sample(&self, x: f32, y: f32, mode: Interpolation) -> [f32; 3] {
        sample_f32(x, y, mode, |x, y| {
            self.rgb[y.clamp(0, self.size[1] as i32 - 1) as usize * self.size[0]
                + x.clamp(0, self.size[0] as i32 - 1) as usize]
        })
    }
}

fn sample_f32<F>(x: f32, y: f32, mode: Interpolation, mut pixel: F) -> [f32; 3]
where
    F: FnMut(i32, i32) -> [f32; 3],
{
    if mode == Interpolation::Nearest {
        return pixel((x + 0.5).floor() as i32, (y + 0.5).floor() as i32);
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

pub(super) fn sample_image_with(
    image: &ImageBuf,
    p: Pos2,
    rect: Rect,
    mode: Interpolation,
    transform: impl Fn([f32; 3]) -> [f32; 3],
) -> [f32; 3] {
    let uv = (p - rect.min) / rect.size();
    let x = uv.x * image.width as f32 - 0.5;
    let y = uv.y * image.height as f32 - 0.5;
    sample_f32(x, y, mode, |x, y| {
        transform(
            image
                .get(
                    x.clamp(0, image.width as i32 - 1) as u32,
                    y.clamp(0, image.height as i32 - 1) as u32,
                )
                .map(|v| v as f32),
        )
    })
}

pub(super) fn capped_raster(image: &ImageBuf, max_edge: usize) -> Result<DisplayRaster, String> {
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
    let mut rgb = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        let sy = ((y as f64 + 0.5) * h as f64 / size[1] as f64 - 0.5).clamp(0.0, (h - 1) as f64);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(h - 1);
        let fy = sy - y0 as f64;
        for x in 0..size[0] {
            let sx =
                ((x as f64 + 0.5) * w as f64 / size[0] as f64 - 0.5).clamp(0.0, (w - 1) as f64);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(w - 1);
            let fx = sx - x0 as f64;
            rgb.push(std::array::from_fn(|c| {
                let a = image.data[(y0 * w + x0) * 3 + c] as f64;
                let b = image.data[(y0 * w + x1) * 3 + c] as f64;
                let d = image.data[(y1 * w + x0) * 3 + c] as f64;
                let e = image.data[(y1 * w + x1) * 3 + c] as f64;
                ((a * (1.0 - fx) + b * fx) * (1.0 - fy) + (d * (1.0 - fx) + e * fx) * fy) as f32
            }));
        }
    }
    DisplayRaster::new(size, rgb)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_constructors_preserve_dimensions_and_rgb_values() {
        let rgba = DisplayRaster::from_rgba([1, 1], &[64, 128, 255, 7]).unwrap();
        assert_eq!(rgba.size, [1, 1]);
        assert_eq!(rgba.rgb[0], [64.0 / 255.0, 128.0 / 255.0, 1.0]);
        let image = ImageBuf::from_data(1, 1, vec![0.25 as _, 0.5 as _, 0.75 as _]);
        let raster = DisplayRaster::from_float(&image).unwrap();
        assert_eq!(raster.size, [1, 1]);
        assert_eq!(raster.rgb[0], [0.25, 0.5, 0.75]);
    }
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
    fn image_sampling_transforms_each_tap_before_interpolation() {
        let image = ImageBuf::from_data(
            2,
            1,
            vec![
                0.25 as _, 0.25 as _, 0.25 as _, 0.75 as _, 0.75 as _, 0.75 as _,
            ],
        );
        let rect = Rect::from_min_size(Pos2::ZERO, egui::Vec2::new(2.0, 1.0));
        let out = sample_image_with(
            &image,
            Pos2::new(1.0, 0.5),
            rect,
            Interpolation::Linear,
            |p| p.map(|v| v * v),
        );
        assert_eq!(out, [0.3125; 3]);
    }
}
