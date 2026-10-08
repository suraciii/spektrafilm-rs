use std::borrow::Cow;

use spektrafilm_math::precision::Scalar;

/// Convert scalar image data at the GPU boundary without allocating in f32 mode.
#[inline]
pub(crate) fn scalars_to_f32(v: &[Scalar]) -> Cow<'_, [f32]> {
    #[cfg(feature = "precision-f64")]
    {
        Cow::Owned(v.iter().map(|&s| s as f32).collect())
    }
    #[cfg(not(feature = "precision-f64"))]
    {
        Cow::Borrowed(v)
    }
}

/// Convert f32 GPU readback to the crate scalar type.
#[inline]
pub(crate) fn f32_to_scalars(v: Vec<f32>) -> Vec<Scalar> {
    #[cfg(feature = "precision-f64")]
    {
        v.into_iter().map(|x| x as f64).collect()
    }
    #[cfg(not(feature = "precision-f64"))]
    {
        v
    }
}

/// Sanitize profile NaNs before GPU upload.
///
/// NaN channel densities become zero. A NaN base density, or any NaN in a
/// wavelength row, becomes +1000 base density so `10^-base` removes that
/// wavelength from the spectral integral. This matches the CPU/Python model
/// and avoids relying on shader NaN behavior under GPU fast math.
pub(crate) fn sanitize_spectral_inputs(
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    n_wl: usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut cd = Vec::with_capacity(n_wl * 3);
    let mut bd = Vec::with_capacity(n_wl);
    for wl in 0..n_wl {
        let [r, g, b] = channel_density[wl];
        let base = base_density.get(wl).copied().unwrap_or(f64::NAN);
        let row_has_nan = r.is_nan() || g.is_nan() || b.is_nan() || base.is_nan();
        cd.extend([
            if r.is_nan() { 0.0 } else { r as f32 },
            if g.is_nan() { 0.0 } else { g as f32 },
            if b.is_nan() { 0.0 } else { b as f32 },
        ]);
        bd.push(if row_has_nan { 1000.0 } else { base as f32 });
    }
    (cd, bd)
}

/// Detect the endpoint-derived grid expected by WGPU density shaders.
///
/// The shader's direct-index path assumes every point lies on the grid formed
/// by the endpoints, including small deviations in exposure spacing.
#[inline]
pub(crate) fn is_uniform_grid_endpoint(xs: &[f64]) -> bool {
    if xs.len() < 3 {
        return true;
    }
    let step = (xs[xs.len() - 1] - xs[0]) / (xs.len() as f64 - 1.0);
    let tol = step.abs() * 1e-9 + 1e-12;
    (1..xs.len()).all(|i| {
        let expected = xs[0] + (i as f64) * step;
        (xs[i] - expected).abs() <= tol
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_nan_rows_consistently() {
        let (channel, base) =
            sanitize_spectral_inputs(&[[f64::NAN, 2.0, 3.0], [4.0, 5.0, 6.0]], &[7.0], 2);
        assert_eq!(channel, vec![0.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(base, vec![1000.0, 1000.0]);
    }

    #[test]
    fn detects_wgpu_endpoint_grid() {
        assert!(is_uniform_grid_endpoint(&[0.0, 0.5, 1.0]));
        assert!(!is_uniform_grid_endpoint(&[0.0, 0.50000001, 1.0]));
    }
}
