//! Exposure Grain: a reconstructed post-scan encoded-RGB value-noise appearance.
//!
//! The frozen scalar reference is the reconstructed Exposure X7 luma path. The
//! device and host share the same binary32 tables and control coefficients from
//! [`spektrafilm_math::grain::exposure`].
use rayon::prelude::*;
use spektrafilm_math::grain::exposure::{ExposureTables, exposure_coefficients};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f32, to_f32};

pub use spektrafilm_math::grain::exposure::exposure_seed;

/// Stable serialized film-format choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposureFilmFormat {
    ThirtyFive,
    Medium,
    Large,
}

impl ExposureFilmFormat {
    pub const fn short_edge_mm(self) -> f32 {
        match self {
            Self::ThirtyFive => 24.0,
            Self::Medium => 56.0,
            Self::Large => 101.6,
        }
    }
}

/// The host-side control set used by the CPU and GPU executors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExposureGrainParams {
    pub strength: f32,
    pub size: f32,
    pub auto_size: bool,
    pub format: ExposureFilmFormat,
    pub absolute_size_px: f32,
    pub roughness: f32,
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub push: f32,
    /// The reference seed domain. Use [`exposure_seed`] to reduce the shared
    /// 64-bit runtime seed without introducing a second phase input.
    pub seed: u32,
    pub render_scale: f32,
}

impl Default for ExposureGrainParams {
    fn default() -> Self {
        Self {
            strength: 100.0,
            size: 1.0,
            auto_size: true,
            format: ExposureFilmFormat::ThirtyFive,
            absolute_size_px: 1.5,
            roughness: 50.0,
            shadows: 50.0,
            midtones: 70.0,
            highlights: 30.0,
            push: 0.0,
            seed: 0,
            render_scale: 1.0,
        }
    }
}

#[inline]
fn smoothstep(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    (3.0 - 2.0 * value) * value * value
}

#[inline]
fn zone_weights(luma: f32, midtone: f32, highlight: f32) -> [f32; 3] {
    let lo = if luma < 0.0 {
        smoothstep((luma - -0.5) / 0.5)
    } else {
        1.0
    };
    let middle = smoothstep(luma / midtone);
    let high = smoothstep((luma - midtone) / (highlight - midtone));
    let last = smoothstep((luma - 1.0) / 0.5);
    [lo - middle, middle - high, high - last]
}

#[inline]
fn shaped(value: f32, roughness_effective: f32) -> f32 {
    if (roughness_effective - 0.5).abs() < 1e-5 {
        return value;
    }
    let t = (1.0 - 0.5) * roughness_effective / (roughness_effective - 0.5);
    let a = t * t - t;
    let b = t;
    let magnitude = if 1.0 - value.abs() - b == 0.0 {
        1.0
    } else {
        (a / (1.0 - value.abs() - b) + b).clamp(0.0, 1.0)
    };
    magnitude.copysign(value)
}

/// Apply the frozen scalar path to native encoded RGB.
pub fn apply_cpu(input: &ImageBuf, mut params: ExposureGrainParams) -> ImageBuf {
    if input.pixel_count() == 0 || params.strength <= 0.0 {
        return input.clone();
    }
    params.seed = params.seed.min(exposure_seed(u64::MAX));
    let coefficients = exposure_coefficients(
        params.auto_size,
        params.size,
        params.format.short_edge_mm(),
        params.absolute_size_px,
        params.strength,
        params.roughness,
        params.shadows,
        params.midtones,
        params.highlights,
        params.push,
        params.render_scale,
        input.width,
        input.height,
    );
    let tables = ExposureTables::new(params.seed, coefficients.size_low, coefficients.size_high);
    let mut output = input.clone();
    output
        .data
        .par_chunks_exact_mut(3)
        .enumerate()
        .for_each(|(index, pixel)| {
            let width = input.width as usize;
            let x = (index % width) as f32;
            let y = (index / width) as f32;
            let rgb = [to_f32(pixel[0]), to_f32(pixel[1]), to_f32(pixel[2])];
            let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
            let [shadow, middle, highlight] = zone_weights(luma, 0.424_672_4, 0.849_344_8);
            let displacement = shadow * coefficients.shadow_amplitude
                + middle * coefficients.midtone_amplitude
                + highlight * coefficients.highlight_amplitude;
            let grain = shaped(tables.noise(x, y), coefficients.roughness_effective);
            let output_luma =
                (luma + displacement * coefficients.frequency_gain * grain).clamp(0.0, 1.0);
            let delta = output_luma - luma;
            for channel in 0..3 {
                pixel[channel] = from_f32((rgb[channel] + delta).clamp(0.0, 1.0));
            }
        });
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_weights_partition_display_luma() {
        for luma in [0.0, 0.2, 0.424_672_4, 0.6, 0.849_344_8, 1.0] {
            let sum: f32 = zone_weights(luma, 0.424_672_4, 0.849_344_8).iter().sum();
            assert!((sum - 1.0).abs() < 2e-3);
        }
    }

    #[test]
    fn roughness_identity_and_zero_denominator_guard() {
        assert_eq!(shaped(0.37, 0.5), 0.37);
        assert_eq!(shaped(1.0, 0.0), 1.0);
        assert_eq!(shaped(-1.0, 0.0), -1.0);
    }

    #[test]
    fn strength_zero_and_empty_input_are_exact_bypasses() {
        let image = ImageBuf::from_data(2, 2, vec![from_f32(0.4); 12]);
        let params = ExposureGrainParams {
            strength: 0.0,
            ..Default::default()
        };
        assert_eq!(apply_cpu(&image, params).data, image.data);
        let empty = ImageBuf::from_data(0, 0, Vec::new());
        assert_eq!(
            apply_cpu(&empty, ExposureGrainParams::default()).data,
            empty.data
        );
    }

    #[test]
    fn frozen_reference_vector_matches() {
        let expected = [
            0.022_249_42,
            0.122_249_42,
            0.222_249_42,
            0.498_152_08,
            0.498_152_08,
            0.548_152_08,
            0.599_189_3,
            0.499_189_3,
            0.499_189_3,
            0.845_625_3,
            0.645_625_3,
            0.595_625_3,
            0.231_514_09,
            0.361_514_09,
            0.511_514_1,
            0.525_190_1,
            0.555_190_1,
            0.655_190_1,
            0.601_370_2,
            0.531_370_2,
            0.581_370_2,
            0.835_610_2,
            0.665_610_2,
            0.665_610_2,
            0.333_935_15,
            0.493_935_15,
            0.693_935_15,
            0.549_323_4,
            0.609_323_4,
            0.759_323_4,
            0.586_061_5,
            0.546_061_5,
            0.646_061_5,
            0.821_086,
            0.681_086,
            0.731_086,
        ];
        let image = ImageBuf::from_data(
            4,
            3,
            (0..3)
                .flat_map(|y| {
                    (0..4).flat_map(move |x| {
                        [
                            from_f32(0.1 + x as f32 * 0.2 + y as f32 * 0.02),
                            from_f32(0.2 + x as f32 * 0.1 + y as f32 * 0.05),
                            from_f32(0.3 + x as f32 * 0.05 + y as f32 * 0.1),
                        ]
                    })
                })
                .collect(),
        );
        let params = ExposureGrainParams {
            auto_size: false,
            absolute_size_px: 2.0,
            seed: 7,
            ..Default::default()
        };
        let actual = apply_cpu(&image, params);
        for (index, (&actual, &expected)) in actual.data.iter().zip(&expected).enumerate() {
            assert!(
                (to_f32(actual) - expected).abs() < 2e-5,
                "sample {index}: {} != {expected}",
                to_f32(actual)
            );
        }
    }

    #[test]
    fn deterministic_rendering_changes_with_seed_and_keeps_channels_in_range() {
        let image = ImageBuf::from_data(
            8,
            6,
            (0..8 * 6 * 3)
                .map(|i| from_f32(0.05 + (i % 31) as f32 / 40.0))
                .collect(),
        );
        let mut params = ExposureGrainParams {
            seed: 17,
            ..Default::default()
        };
        let a = apply_cpu(&image, params);
        let b = apply_cpu(&image, params);
        assert_eq!(a.data, b.data);
        params.seed = 18;
        let c = apply_cpu(&image, params);
        assert_ne!(a.data, c.data);
        for value in a.data.iter().chain(c.data.iter()) {
            assert!(to_f32(*value).is_finite());
            assert!((0.0..=1.0).contains(&to_f32(*value)));
        }
    }
}
