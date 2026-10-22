use crate::image::ImageBuf;
use crate::precision::{Scalar, from_f64};
/// Fast 1D linear interpolation with binary search.
///
/// Port of Python `fast_interp.py`. Supports both common x-axis (same for all channels)
/// and per-channel x-axes.
use rayon::prelude::*;

/// Interpolate a single value from sorted arrays.
#[inline]
pub fn interp_1d(x: &[f32], y: &[f32], xq: f32) -> f32 {
    debug_assert_eq!(x.len(), y.len());
    debug_assert!(!x.is_empty());

    let n = x.len();
    if n == 1 {
        return y[0];
    }
    if xq <= x[0] {
        return y[0];
    }
    if xq >= x[n - 1] {
        return y[n - 1];
    }

    let i = match x.binary_search_by(|v| v.partial_cmp(&xq).unwrap()) {
        Ok(i) => return y[i],
        Err(i) => i - 1,
    };

    let t = (xq - x[i]) / (x[i + 1] - x[i]);
    y[i] + t * (y[i + 1] - y[i])
}

/// Fast image interpolation: for each pixel (HxWx3), look up each channel
/// independently in a common x-axis → y-values table.
///
/// `x_axis`: sorted 1D array of length K.
/// `y_vals`: [K][3] array of y-values per channel.
pub fn fast_interp_image_f64(img: &ImageBuf, x_axis: &[f64], y_vals: &[[f64; 3]]) -> ImageBuf {
    let k = x_axis.len();
    assert_eq!(y_vals.len(), k);

    // Promote to Scalar once (no-op in f64 mode, narrowing in f32 mode).
    let xa: Vec<Scalar> = x_axis.iter().map(|&v| from_f64(v)).collect();
    let ya: Vec<[Scalar; 3]> = y_vals
        .iter()
        .map(|r| [from_f64(r[0]), from_f64(r[1]), from_f64(r[2])])
        .collect();
    let inv_dx: Vec<Scalar> = (0..k - 1)
        .map(|i| {
            let dx = xa[i + 1] - xa[i];
            if dx != 0.0 { 1.0 / dx } else { 0.0 }
        })
        .collect();

    let mut out = img.clone();
    out.par_pixels_mut().for_each(|px| {
        for c in 0..3 {
            let x = px[c];
            if x <= xa[0] {
                px[c] = ya[0][c];
            } else if x >= xa[k - 1] {
                px[c] = ya[k - 1][c];
            } else {
                // Match numpy's searchsorted(side='right') - 1: include equal values on the left side.
                let idx = xa.partition_point(|&v| v <= x);
                let low = if idx > 0 { idx - 1 } else { 0 };
                let t = (x - xa[low]) * inv_dx[low];
                px[c] = ya[low][c] + t * (ya[low + 1][c] - ya[low][c]);
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interp_1d_exact() {
        // An exact node hit takes the `binary_search` Ok branch and must
        // return the node's value without interpolation.
        let x = vec![0.0, 1.0, 2.0, 3.0];
        let y = vec![0.0, 10.0, 20.0, 30.0];
        assert_eq!(interp_1d(&x, &y, 1.0), 10.0);
        assert_eq!(interp_1d(&x, &y, 2.0), 20.0);
    }

    #[test]
    fn test_interp_1d_midpoint() {
        let x = vec![0.0, 1.0, 2.0];
        let y = vec![0.0, 10.0, 20.0];
        let v = interp_1d(&x, &y, 0.5);
        assert!((v - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_interp_1d_clamp() {
        let x = vec![0.0, 1.0];
        let y = vec![5.0, 15.0];
        assert_eq!(interp_1d(&x, &y, -1.0), 5.0);
        assert_eq!(interp_1d(&x, &y, 2.0), 15.0);
    }
}
