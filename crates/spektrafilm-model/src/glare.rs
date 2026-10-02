// Viewing glare simulation.
// Adds a small fraction of randomized blurred illuminant to simulate surface reflections.

use spektrafilm_math::gaussian::gaussian_blur_channel;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::numpy_rng::GaussRng;
use spektrafilm_math::precision::{Scalar, from_f32, from_f64};

/// Generate the per-pixel glare_amount field (lognormal sampled, then blurred, then /100).
///
/// Port of Python `compute_random_glare_amount` (model/glare.py:19). The lognormal
/// parameters are inverted from linear-space mean and std:
///   σ = sqrt( ln(1 + (s²/m²)) )
///   μ = ln(m) - σ²/2
///
/// Uses NumPy's legacy normal sampler with an explicit reproducible seed. The
/// upstream JIT kernel uses unseeded thread-local streams, so its texture is
/// comparable statistically rather than pixel-for-pixel. Spatial blur uses the
/// same scalar Gaussian filter, and the result is divided by 100.
pub fn compute_random_glare_amount(
    width: u32,
    height: u32,
    percent: f32,
    roughness: f32,
    blur: f32,
    seed: u64,
) -> Vec<Scalar> {
    let n_pixels = (width as usize) * (height as usize);
    if percent <= 0.0 {
        return vec![Scalar::default(); n_pixels];
    }
    let m = percent as f64;
    let s = roughness as f64 * m;
    let sigma2 = (1.0 + (s * s) / (m * m)).ln();
    let sigma = sigma2.sqrt();
    let mu = m.ln() - sigma2 / 2.0;
    let mut rng = GaussRng::new(seed as u32);

    // The upstream fast_lognormal kernel skips normal draws below this
    // threshold, including zero roughness.
    let mut glare: Vec<Scalar> = (0..n_pixels)
        .map(|_| {
            let value = if sigma < 1e-6 {
                mu.exp()
            } else {
                (mu + sigma * rng.gauss()).exp()
            };
            from_f64(value)
        })
        .collect();

    if blur > 0.0 {
        glare = gaussian_blur_channel(&glare, width, height, blur);
    }

    // Divide by 100 (CC-style fraction).
    let inv100 = from_f64(1.0 / 100.0);
    for v in &mut glare {
        *v *= inv100;
    }
    glare
}

/// Add glare in any 3-channel image space — XYZ or RGB.
///
/// `space_per_pixel_offset` is the per-color constant offset vector. For XYZ-space glare
/// this is `illuminant_xyz`. For RGB-space glare it's `M * illuminant_xyz` where M is the
/// CAT+matrix that would convert XYZ → output RGB. By linearity:
///   `M * (xyz + g * illu) = M*xyz + g * (M*illu)`
/// so applying glare in either space gives the same result.
///
/// `glare_amount` must be `width*height` Scalar values from `compute_random_glare_amount`.
pub fn add_glare_with_amount(
    image: &mut ImageBuf,
    glare_amount: &[Scalar],
    space_offset: [Scalar; 3],
) {
    debug_assert_eq!(glare_amount.len(), image.pixel_count());
    for (i, px) in image.pixels_mut().enumerate() {
        let g = glare_amount[i];
        px[0] += g * space_offset[0];
        px[1] += g * space_offset[1];
        px[2] += g * space_offset[2];
    }
}

/// Add viewing glare to an XYZ image.
///
/// Port of Python `add_glare`. Generates a random glare pattern
/// using lognormal distribution, blurs it, and adds it as
/// a fraction of the illuminant.
pub fn add_glare(
    xyz: &ImageBuf,
    illuminant_xyz: [f32; 3],
    percent: f32,
    roughness: f32,
    blur: f32,
) -> ImageBuf {
    if percent <= 0.0 {
        return xyz.clone();
    }

    let glare_amount = compute_random_glare_amount(
        xyz.width,
        xyz.height,
        percent,
        roughness,
        blur,
        0,
    );
    let illum = illuminant_xyz.map(from_f32);
    let mut result = xyz.clone();
    add_glare_with_amount(&mut result, &glare_amount, illum);

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_glare_matches_numpy_normal_stream() {
        // Upstream fast_lognormal.py_func after np.random.seed(0), with
        // the same float32 public parameters promoted before multiplication.
        let expected = [
            0.00205799574328467,
            0.0010805333829014246,
            0.0014201538343492563,
            0.0025779203241936516,
        ];
        let actual = compute_random_glare_amount(2, 2, 0.1, 0.5, 0.0, 0);
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - from_f64(expected)).abs() < from_f64(1e-9));
        }
    }

    #[test]
    fn subthreshold_glare_is_independent_of_seed() {
        for roughness in [0.0, 1e-7] {
            let first = compute_random_glare_amount(2, 2, 0.1, roughness, 0.0, 0);
            let second = compute_random_glare_amount(2, 2, 0.1, roughness, 0.0, 42);
            assert_eq!(first, second);
            for value in first {
                assert!((value - from_f64(0.001)).abs() < from_f64(1e-9));
            }
        }
    }
}
