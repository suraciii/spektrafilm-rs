//! Shared optical image operations used by camera and scanner stages.

use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f64;
/// Apply Python's unsharp mask with the faithful CPU Gaussian filter.
/// Keep sigma and amount in f64 before conversion to the image scalar type.
pub fn apply_unsharp_mask(
    image: &ImageBuf,
    sigma: impl Into<f64>,
    amount: impl Into<f64>,
    _backend: &dyn ComputeBackend,
) -> ImageBuf {
    let sigma = sigma.into();
    let amount = amount.into();
    if sigma <= 0.0 || amount <= 0.0 {
        return image.clone();
    }
    let blurred = spektrafilm_math::gaussian::gaussian_blur(image, sigma);
    let amount_s = from_f64(amount);
    let mut result = image.clone();
    for (r, (o, b)) in result
        .data
        .iter_mut()
        .zip(image.data.iter().zip(blurred.data.iter()))
    {
        *r = o + amount_s * (o - b);
    }
    result
}

/// Apply Gaussian blur in physical units (micrometers).
pub fn apply_gaussian_blur_um(
    image: &ImageBuf,
    sigma_um: f32,
    pixel_size_um: f32,
    backend: &dyn ComputeBackend,
) -> ImageBuf {
    let sigma_px = sigma_um / pixel_size_um;
    if sigma_px > 0.0 {
        backend.gaussian_blur(image, sigma_px)
    } else {
        image.clone()
    }
}

/// Highlight-boost tone curve. Port of Python `boost_highlights`
/// (numba_boost_hightlights.py): reconstructs pre-clip highlight irradiance
/// before scatter/halation. Identity below `midgray·2^protect_ev`; above it
/// adds `boost_scale·(exp(a·dx) − a·dx − 1)`, normalised so the brightest
/// pixel gains exactly `2^boost_ev`. `midgray` is fixed at 0.184 (the value
/// the filming stage uses). A no-op when `boost_ev == 0`.
pub fn boost_highlights(
    image: &ImageBuf,
    boost_ev: f64,
    boost_range: f64,
    protect_ev: f64,
) -> ImageBuf {
    use rayon::prelude::*;
    const MIDGRAY: f64 = 0.184;
    if boost_ev == 0.0 {
        return image.clone();
    }
    let max_raw = image
        .data
        .iter()
        .map(|&v| v as f64)
        .fold(f64::NEG_INFINITY, f64::max);
    if max_raw == 0.0 {
        return ImageBuf::new(image.width, image.height);
    }
    let raw_x0 = (MIDGRAY * 2f64.powf(protect_ev)).clamp(0.0, max_raw);
    if raw_x0 == max_raw {
        return image.clone();
    }
    let a = 28f64.powf(1.0 - boost_range);
    let x0 = raw_x0 / max_raw;
    let denom = (a * (1.0 - x0)).exp() - a * (1.0 - x0) - 1.0;
    if denom <= 0.0 {
        return image.clone();
    }
    let k = (2f64.powf(boost_ev) - 1.0) / denom;
    let inv_max_raw = 1.0 / max_raw;
    let boost_scale = k * max_raw;

    let mut out = image.clone();
    out.data.par_iter_mut().for_each(|v| {
        let xv = *v as f64;
        let nv = if xv <= raw_x0 {
            xv
        } else {
            let dx = (xv - raw_x0) * inv_max_raw;
            xv + boost_scale * ((a * dx).exp() - a * dx - 1.0)
        };
        *v = from_f64(nv);
    });
    out
}

