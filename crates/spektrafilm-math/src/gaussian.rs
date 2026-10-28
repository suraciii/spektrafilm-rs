use crate::image::ImageBuf;
use crate::precision::{Scalar, ZERO, from_f64, to_f64};
use rayon::prelude::*;

/// Separable Gaussian blur on an ImageBuf.
///
/// Uses reflect-boundary FIR below sigma 3, sample-replicating
/// Young-van Vliet IIR below sigma 8, and reflected three-box filtering above.
/// Each channel is processed independently with rayon.
/// Single-channel form for callers with per-channel sigma values.
pub fn gaussian_blur_channel(
    data: &[Scalar],
    w: u32,
    h: u32,
    sigma: impl Into<f64>,
) -> Vec<Scalar> {
    let wu = w as usize;
    let hu = h as usize;
    assert_eq!(data.len(), wu * hu);
    let sigma = sigma.into();
    if sigma <= 0.0 {
        return data.to_vec();
    }
    if wu == 0 || hu == 0 {
        return data.to_vec();
    }
    let mut chan = data.to_vec();
    let mut tmp = vec![ZERO; wu * hu];
    let horizontal_first = sigma >= 3.0;
    blur_1d_parallel(&chan, &mut tmp, wu, hu, sigma, horizontal_first);
    blur_1d_parallel(&tmp, &mut chan, wu, hu, sigma, !horizontal_first);
    chan
}

/// Amplitude and σ/decay ratio of each Gaussian in the 3-component
/// approximation of the exponential PSF — Python's
/// `_EXPONENTIAL_GAUSSIAN_FITS[3]`. Shared by the CPU reference and the
/// GPU kernel builder so the fitted tail cannot drift apart.
pub const EXPONENTIAL_GAUSSIAN_FITS: [(f64, f64); 3] =
    [(0.1633, 0.5360), (0.6496, 1.5236), (0.1870, 2.7684)];

/// Sum-of-3-Gaussians approximation of an isotropic 2D exponential PSF
/// `exp(-r / decay) / (2π · decay²)`, using the Python fit amplitudes and
/// σ/λ ratios. Used for the scatter-tail component of halation.
pub fn exponential_filter_channel(
    data: &[Scalar],
    w: u32,
    h: u32,
    decay_constant: impl Into<f64>,
) -> Vec<Scalar> {
    // Python `_EXPONENTIAL_GAUSSIAN_FITS[3]` - amplitude, σ / decay.
    const FIT: [(f64, f64); 3] = EXPONENTIAL_GAUSSIAN_FITS;
    let n = data.len();
    let mut result = vec![ZERO; n];
    let decay_f64 = decay_constant.into();
    for &(amp, ratio) in &FIT {
        let sigma_k = ratio * decay_f64;
        let component = gaussian_blur_channel(data, w, h, sigma_k);
        let amp_s = from_f64(amp);
        for (r, &v) in result.iter_mut().zip(component.iter()) {
            *r += amp_s * v;
        }
    }
    result
}

pub fn gaussian_blur(img: &ImageBuf, sigma: impl Into<f64>) -> ImageBuf {
    let sigma = sigma.into();
    if sigma <= 0.0 {
        return img.clone();
    }

    let w = img.width as usize;
    let h = img.height as usize;
    if w == 0 || h == 0 {
        return img.clone();
    }

    // Process each channel separately for cache-friendly access
    let mut channels: Vec<Vec<Scalar>> = (0..3).map(|c| img.extract_channel(c)).collect();

    channels.par_iter_mut().for_each(|chan| {
        let mut tmp = vec![ZERO; w * h];
        // Python FIR is vertical then horizontal; IIR uses the reverse order.
        let horizontal_first = sigma >= 3.0;
        blur_1d_parallel(chan, &mut tmp, w, h, sigma, horizontal_first);
        blur_1d_parallel(&tmp, chan, w, h, sigma, !horizontal_first);
    });

    let mut out = ImageBuf::new(img.width, img.height);
    for (c, channel) in channels.iter().enumerate() {
        out.write_channel(c, channel);
    }
    out
}

/// 1D blur pass — either horizontal (along rows) or vertical (along columns).
fn blur_1d_parallel(
    src: &[Scalar],
    dst: &mut [Scalar],
    w: usize,
    h: usize,
    sigma: f64,
    horizontal: bool,
) {
    if sigma < 3.0 {
        fir_blur_1d(src, dst, w, h, from_f64(sigma), horizontal);
    } else if sigma < 8.0 {
        iir_blur_1d(src, dst, w, h, from_f64(sigma), horizontal);
    } else {
        box_blur_1d(src, dst, w, h, sigma, horizontal);
    }
}

/// Choose an odd box width minimizing squared error against the normalized,
/// three-sigma truncated Gaussian. Ties favor the smaller width.
pub fn gaussian_box_width(sigma: f64) -> usize {
    let center = ((4.0 * sigma * sigma + 1.0).sqrt() + 0.5).floor() as usize;
    let radius = (3.0 * sigma + 0.5) as usize;
    let gaussian: Vec<f64> = (0..=radius)
        .map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp())
        .collect();
    let norm = gaussian[0] + 2.0 * gaussian[1..].iter().sum::<f64>();
    let triangle = |n: isize| -> f64 {
        if n < 0 {
            0.0
        } else {
            (n as f64 + 1.0) * (n as f64 + 2.0) * 0.5
        }
    };
    let mut best = (f64::INFINITY, 1);
    for width in center.saturating_sub(12).max(1)..=center.saturating_add(12) {
        if width % 2 == 0 {
            continue;
        }
        let half = width / 2;
        let extent = radius.max(3 * half);
        let denom = (width as f64).powi(3);
        let mut error = 0.0;
        for i in 0..=extent {
            let x = (i + 3 * half) as isize;
            let w = width as isize;
            let box_value = (triangle(x) - 3.0 * triangle(x - w) + 3.0 * triangle(x - 2 * w)
                - triangle(x - 3 * w))
                / denom;
            let reference = if i <= radius { gaussian[i] / norm } else { 0.0 };
            let delta = box_value - reference;
            error += delta * delta * if i == 0 { 1.0 } else { 2.0 };
        }
        if error < best.0 {
            best = (error, width);
        }
    }
    best.1
}

/// Three unit-sum boxes, with reflection applied independently on each pass.
fn box_blur_1d(
    src: &[Scalar],
    dst: &mut [Scalar],
    w: usize,
    h: usize,
    sigma: f64,
    horizontal: bool,
) {
    let width = gaussian_box_width(sigma);
    let radius = (width / 2) as isize;
    let mut first = vec![ZERO; src.len()];
    let mut second = vec![ZERO; src.len()];
    let (lines, length, stride) = if horizontal { (h, w, 1) } else { (w, h, w) };
    let pass = |input: &[Scalar], output: &mut [Scalar]| {
        for line in 0..lines {
            let start = if horizontal { line * w } else { line };
            let mut sum = 0.0;
            for offset in -radius..=radius {
                sum += to_f64(input[start + reflect_index(offset, length) * stride]);
            }
            for index in 0..length {
                output[start + index * stride] = from_f64(sum / width as f64);
                if index + 1 < length {
                    sum -= to_f64(
                        input[start + reflect_index(index as isize - radius, length) * stride],
                    );
                    sum += to_f64(
                        input[start + reflect_index(index as isize + radius + 1, length) * stride],
                    );
                }
            }
        }
    };
    pass(src, &mut first);
    pass(&first, &mut second);
    pass(&second, dst);
}

/// FIR Gaussian blur (truncated kernel). Good for small sigma.
fn fir_blur_1d(
    src: &[Scalar],
    dst: &mut [Scalar],
    w: usize,
    h: usize,
    sigma: Scalar,
    horizontal: bool,
) {
    // Python `_gaussian_kernel_1d`: `radius = int(truncate * sigma + 0.5)`
    // with `truncate=3.0`. `int(x)` on a non-negative float truncates
    // toward zero, which for `truncate·sigma + 0.5` is the standard
    // "round half up" rule. `(x + 0.5) as usize` produces the same
    // value as Python's `int(x + 0.5)` for all positive inputs we
    // care about. Rust used to use `.ceil()` here, which biases the
    // radius up by 1 when `truncate·sigma` lands at a half-integer
    // and changes the FIR kernel size — measurable parity drift.
    let radius = (from_f64(3.0) * sigma + from_f64(0.5)) as usize;
    let kernel = make_gaussian_kernel(sigma, radius);

    if horizontal {
        dst.par_chunks_exact_mut(w)
            .enumerate()
            .for_each(|(y, row_dst)| {
                let row_src = &src[y * w..(y + 1) * w];
                // The index supplies reflected sample coordinates.
                #[allow(clippy::needless_range_loop)]
                for x in 0..w {
                    let mut sum = ZERO;
                    for (ki, &kv) in kernel.iter().enumerate() {
                        let sx = reflect_index(x as isize + ki as isize - radius as isize, w);
                        sum += row_src[sx] * kv;
                    }
                    row_dst[x] = sum;
                }
            });
    } else {
        dst.par_chunks_exact_mut(w)
            .enumerate()
            .for_each(|(y, row_dst)| {
                for x in 0..w {
                    let mut sum = ZERO;
                    for (ki, &kv) in kernel.iter().enumerate() {
                        let sy = reflect_index(y as isize + ki as isize - radius as isize, h);
                        sum += src[sy * w + x] * kv;
                    }
                    row_dst[x] = sum;
                }
            });
    }
}

/// Half-sample symmetric reflection, including kernels wider than the image.
fn reflect_index(index: isize, length: usize) -> usize {
    let period = 2 * length as isize;
    let folded = index.rem_euclid(period);
    if folded < length as isize {
        folded as usize
    } else {
        (period - 1 - folded) as usize
    }
}

/// Young-van Vliet recursive IIR Gaussian blur. Good for large sigma.
///
/// Reference: I.T. Young, L.J. van Vliet, "Recursive implementation of the
/// Gaussian filter", Signal Processing 44 (1995) 139-151.
fn iir_blur_1d(
    src: &[Scalar],
    dst: &mut [Scalar],
    w: usize,
    h: usize,
    sigma: Scalar,
    horizontal: bool,
) {
    // Young-van Vliet coefficients stay binary64 in every build. Their DC gain
    // is `a / (1 - b1 - b2 - b3)`, and at large sigma `a` is around 1e-6, so
    // working-precision f32 arithmetic cannot separate `1 - b1 - b2 - b3` from
    // 1.0: an f32 recurrence loses the gain and a constant field drifts (36 %
    // at sigma = 147 px on a 1024-wide frame). The device kernel has always
    // carried f64 coefficients and f64 state, so this also restores parity.
    let sigma = to_f64(sigma);
    let q = if sigma >= 2.5 {
        0.98711 * sigma - 0.96330
    } else {
        3.97156 - 4.14554 * (1.0 - 0.26891 * sigma).sqrt()
    };

    let q2 = q * q;
    let q3 = q2 * q;

    let b0 = 1.57825 + 2.44413 * q + 1.4281 * q2 + 0.422205 * q3;
    let b1_raw = 2.44413 * q + 2.85619 * q2 + 1.26661 * q3;
    let b2_raw = -(1.4281 * q2 + 1.26661 * q3);
    let b3_raw = 0.422205 * q3;
    let a = 1.0 - (b1_raw + b2_raw + b3_raw) / b0;
    let b1 = b1_raw / b0;
    let b2 = b2_raw / b0;
    let b3 = b3_raw / b0;

    if horizontal {
        dst.par_chunks_exact_mut(w)
            .enumerate()
            .for_each(|(y, row_dst)| {
                let row_src = &src[y * w..(y + 1) * w];
                iir_filter_row(row_src, row_dst, a, b1, b2, b3);
            });
    } else {
        // Vertical recursive filtering has a true dependency down each column,
        // so columns are independent work items. Store them column-major first
        // to keep parallel writes contiguous, then transpose back to row-major.
        let mut col_major = vec![ZERO; w * h];
        col_major
            .par_chunks_exact_mut(h)
            .enumerate()
            .for_each(|(x, col_dst)| {
                let col_src: Vec<Scalar> = (0..h).map(|y| src[y * w + x]).collect();
                iir_filter_row(&col_src, col_dst, a, b1, b2, b3);
            });
        dst.par_chunks_exact_mut(w)
            .enumerate()
            .for_each(|(y, row_dst)| {
                for x in 0..w {
                    row_dst[x] = col_major[x * h + y];
                }
            });
    }
}

/// Apply Young-van Vliet IIR filter to a 1D signal (forward + backward pass).
fn iir_filter_row(src: &[Scalar], dst: &mut [Scalar], a: f64, b1: f64, b2: f64, b3: f64) {
    let n = src.len();
    if n == 0 {
        return;
    }

    // Python parity (`_iir_horizontal` in `fast_gaussian_filter.py`):
    //
    //   forward:
    //     w1 = w2 = w3 = src[0]              (sample-replication border)
    //     for j in 0..n:
    //         w = a*src[j] + b1*w1 + b2*w2 + b3*w3
    //         dst[j] = w
    //         w3 = w2; w2 = w1; w1 = w
    //   backward:
    //     w1 = w2 = w3 = dst[n-1]            (sample-replication border)
    //     for j in (0..n).rev():
    //         w = a*dst[j] + b1*w1 + b2*w2 + b3*w3
    //         dst[j] = w
    //         w3 = w2; w2 = w1; w1 = w
    //
    // Crucially, the state is initialized to the FIRST sample (not zero),
    // and the iteration covers EVERY index in both directions (not
    // n-3..0). The previous Rust code zeroed the state and skipped the
    // last three positions in the backward pass — which produced a
    // small but visible edge bias vs. Python at every halation σ.
    // The recurrence state and coefficients are binary64 even when samples are
    // f32: the DC gain of this filter is a difference of nearly equal
    // quantities, so a working-precision state loses it at large sigma. Only
    // the stored samples are narrowed, which is what the device kernel does.
    let x0 = to_f64(src[0]);
    let mut w1 = x0;
    let mut w2 = x0;
    let mut w3 = x0;
    for j in 0..n {
        let w = a * to_f64(src[j]) + b1 * w1 + b2 * w2 + b3 * w3;
        dst[j] = from_f64(w);
        w3 = w2;
        w2 = w1;
        w1 = w;
    }
    let xn = to_f64(dst[n - 1]);
    w1 = xn;
    w2 = xn;
    w3 = xn;
    for j in (0..n).rev() {
        let w = a * to_f64(dst[j]) + b1 * w1 + b2 * w2 + b3 * w3;
        dst[j] = from_f64(w);
        w3 = w2;
        w2 = w1;
        w1 = w;
    }
}

/// Create normalized Gaussian kernel of given radius.
fn make_gaussian_kernel(sigma: Scalar, radius: usize) -> Vec<Scalar> {
    let size = 2 * radius + 1;
    let mut kernel = Vec::with_capacity(size);
    let half = from_f64(-0.5);

    for i in 0..size {
        let x = from_f64(i as f64) - from_f64(radius as f64);
        let scaled = x / sigma;
        kernel.push((half * (scaled * scaled)).exp());
    }

    let sum: Scalar = kernel.iter().sum();
    for v in &mut kernel {
        *v /= sum;
    }
    kernel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gaussian_blur_uniform_image() {
        // Blurring a uniform image should return the same image
        let img = ImageBuf::from_data(8, 8, vec![from_f64(0.42); 8 * 8 * 3]);
        let out = gaussian_blur(&img, 2.0);
        for (a, b) in img.data.iter().zip(out.data.iter()) {
            assert!((a - b).abs() < from_f64(1e-4), "expected {a}, got {b}");
        }
    }

    /// A blur is mass-preserving, so a constant field must survive any sigma.
    /// The recursive filter's DC gain is `a / (1 - b1 - b2 - b3)`, a difference
    /// of nearly equal quantities: with f32 coefficients or f32 state this
    /// drifted by 36 % at sigma = 147 px, which is inside the DIR coupler tail
    /// and halation scatter range at small pixel pitches.
    #[test]
    fn blur_preserves_a_constant_field_at_large_sigma() {
        let (w, h) = (256u32, 256u32);
        let value = 0.75;
        let field = vec![from_f64(value); (w as usize) * (h as usize)];
        for sigma in [3.0_f64, 5.32, 8.0, 28.5, 81.0, 147.0] {
            let out = gaussian_blur_channel(&field, w, h, sigma);
            let mean = out.iter().map(|v| to_f64(*v)).sum::<f64>() / out.len() as f64;
            assert!(
                (mean - value).abs() < 1e-6,
                "sigma {sigma} changed a constant field: {mean} vs {value}"
            );
        }
    }

    #[test]
    fn box_kernel_fidelity_against_truncated_gaussian() {
        // σ / width: L1 / max / RMS over the union of the supports:
        // 8 / 15:   0.03645100 / 0.00168298 / 0.00090100
        // 32 / 61:  0.03129204 / 0.00033285 / 0.00018418
        // 147 / 283: 0.03289820 / 0.00007097 / 0.00004107
        // Upper bounds allow 2% margin for working-buffer rounding.
        for (sigma, width, expected) in [
            (8.0_f64, 15, [0.03645100, 0.00168298, 0.00090100]),
            (32.0, 61, [0.03129204, 0.00033285, 0.00018418]),
            (147.0, 283, [0.03289820, 0.00007097, 0.00004107]),
        ] {
            assert_eq!(gaussian_box_width(sigma), width);
            let radius = (3.0 * sigma + 0.5) as usize;
            let extent = radius.max(3 * (width / 2));
            let length = 2 * extent + 1;
            let mut impulse = vec![ZERO; length];
            impulse[extent] = from_f64(1.0);
            let mut actual = vec![ZERO; length];
            box_blur_1d(&impulse, &mut actual, length, 1, sigma, true);
            let gaussian = make_gaussian_kernel(from_f64(sigma), radius);
            let mut l1 = 0.0_f64;
            let mut max = 0.0_f64;
            let mut squares = 0.0_f64;
            for (i, &value) in actual.iter().enumerate() {
                let distance = i.abs_diff(extent);
                let reference = if distance <= radius {
                    to_f64(gaussian[i + radius - extent])
                } else {
                    0.0
                };
                let error = (to_f64(value) - reference).abs();
                l1 += error;
                max = max.max(error);
                squares += error * error;
            }
            let measured = [l1, max, (squares / length as f64).sqrt()];
            for (value, baseline) in measured.into_iter().zip(expected) {
                assert!(value <= baseline * 1.02, "sigma {sigma}: {measured:?}");
            }
        }
    }

    #[test]
    fn box_cascade_reflects_on_short_axes() {
        let input = [from_f64(0.0), from_f64(1.0), from_f64(0.25)];
        let mut expected = input;
        let radius = 7isize; // sigma 8 gives width 15
        for _ in 0..3 {
            let previous = expected;
            for (i, value) in expected.iter_mut().enumerate() {
                *value = from_f64(
                    (-radius..=radius)
                        .map(|d| to_f64(previous[reflect_index(i as isize + d, 3)]))
                        .sum::<f64>()
                        / 15.0,
                );
            }
        }
        let mut actual = [ZERO; 3];
        box_blur_1d(&input, &mut actual, 3, 1, 8.0, true);
        for (a, b) in actual.into_iter().zip(expected) {
            assert!((to_f64(a) - to_f64(b)).abs() < 1e-6);
        }
    }

    /// The exponential tail is a weighted sum of Gaussians, so it inherits the
    /// same requirement up to the declared fit's amplitude sum.
    #[test]
    fn exponential_tail_preserves_a_constant_field_at_large_decay() {
        let (w, h) = (256u32, 256u32);
        let value = 0.75;
        let field = vec![from_f64(value); (w as usize) * (h as usize)];
        let amp_sum: f64 = EXPONENTIAL_GAUSSIAN_FITS.iter().map(|(a, _)| a).sum();
        for decay in [5.0_f64, 53.2, 147.0] {
            let out = exponential_filter_channel(&field, w, h, decay);
            let mean = out.iter().map(|v| to_f64(*v)).sum::<f64>() / out.len() as f64;
            let expected = value * amp_sum;
            assert!(
                (mean - expected).abs() < 1e-6,
                "decay {decay} changed a constant field: {mean} vs {expected}"
            );
        }
    }

    #[test]
    fn fir_reflects_repeatedly_on_narrow_images() {
        // Pinned Python fast_gaussian_filter([[0, 1]], 1.2): radius 4
        // extends beyond both edges repeatedly, including the singleton axis.
        let out = gaussian_blur_channel(&[ZERO, from_f64(1.0)], 2, 1, 1.2f64);
        let expected = [0.41537666378143734, 0.5846233362185624];
        for (actual, expected) in out.iter().zip(expected) {
            assert!((*actual - from_f64(expected)).abs() < from_f64(1e-6));
        }
    }
}
