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

/// Narrow one f64 to f32 at the GPU boundary, mapping NaN to `0.0`.
///
/// GPU shaders are f32-only, and Metal's compiler runs fast-math (no-NaN
/// assumption), so NaNs must never reach a shader. Channel data (curves,
/// sensitivities, illuminants) follows the CPU model of zeroing NaNs; callers
/// needing the spectral base-density rule instead use
/// [`sanitize_spectral_inputs`].
#[inline]
pub(crate) fn nan_safe_f32(v: f64) -> f32 {
    if v.is_nan() { 0.0 } else { v as f32 }
}

/// NaN-safe narrowing of a flat f64 slice (e.g. an illuminant) to f32.
#[inline]
pub(crate) fn nan_safe_slice_to_f32(values: &[f64]) -> Vec<f32> {
    values.iter().map(|&v| nan_safe_f32(v)).collect()
}

/// NaN-safe narrowing plus flattening of `[[f64; 3]]` rows (curves,
/// sensitivities) into a row-major `Vec<f32>`.
#[inline]
pub(crate) fn nan_safe_rows_to_f32(rows: &[[f64; 3]]) -> Vec<f32> {
    rows.iter()
        .flat_map(|r| r.iter().map(|&v| nan_safe_f32(v)))
        .collect()
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
    // Wavelength indices align channel and base densities.
    #[allow(clippy::needless_range_loop)]
    for wl in 0..n_wl {
        let [r, g, b] = channel_density[wl];
        let base = base_density.get(wl).copied().unwrap_or(f64::NAN);
        let row_has_nan = r.is_nan() || g.is_nan() || b.is_nan() || base.is_nan();
        cd.extend([nan_safe_f32(r), nan_safe_f32(g), nan_safe_f32(b)]);
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

    #[test]
    fn narrowing_helpers_zero_nans_and_preserve_values() {
        assert_eq!(nan_safe_f32(2.5), 2.5);
        assert!(nan_safe_f32(f64::NAN).is_finite() && nan_safe_f32(f64::NAN) == 0.0);
        // Non-NaN infinities and out-of-range values narrow unchanged, matching
        // a plain `as f32` for every input that is not NaN.
        assert_eq!(nan_safe_f32(f64::INFINITY), f32::INFINITY);
        assert_eq!(nan_safe_f32(1e300), f32::INFINITY);

        assert_eq!(
            nan_safe_slice_to_f32(&[1.0, f64::NAN, -2.5]),
            vec![1.0, 0.0, -2.5]
        );
        assert_eq!(nan_safe_slice_to_f32(&[]), Vec::<f32>::new());

        assert_eq!(
            nan_safe_rows_to_f32(&[[1.0, f64::NAN, 3.0], [f64::NAN, 5.0, f64::NAN]]),
            vec![1.0, 0.0, 3.0, 0.0, 5.0, 0.0]
        );
        assert_eq!(nan_safe_rows_to_f32(&[]), Vec::<f32>::new());
    }
}
