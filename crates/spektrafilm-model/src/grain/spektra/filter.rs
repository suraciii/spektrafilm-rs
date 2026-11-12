//! Spektra's upstream Gaussian arithmetic, independent of the host image dtype.
//!
//! Keep this local: Dye and fotufilm use the shared math/backend filters and
//! must not inherit Spektra's per-operation working-buffer rounding.

pub(super) trait Sample: Copy + Send + Sync {
    fn load(self) -> f64;
    fn store(value: f64) -> Self;
}

impl Sample for f32 {
    fn load(self) -> f64 {
        self as f64
    }
    fn store(value: f64) -> Self {
        value as f32
    }
}

impl Sample for f64 {
    fn load(self) -> f64 {
        self
    }
    fn store(value: f64) -> Self {
        value
    }
}

pub(super) fn blur<T: Sample>(data: &[T], w: usize, h: usize, sigma: f64) -> Vec<T> {
    if sigma <= 0.0 || w == 0 || h == 0 {
        return data.to_vec();
    }
    // Dispatch in f64, before any working-buffer narrowing. In particular,
    // 2.99999999 is FIR even when the image is f32. The pinned upstream
    // reference uses its recursive IIR filter for every sigma >= 3; the old
    // large-sigma box approximation was not the reference law.
    if sigma < 3.0 {
        fir(data, w, h, sigma)
    } else {
        iir(data, w, h, sigma)
    }
}

fn reflect(index: isize, length: usize) -> usize {
    let period = 2 * length as isize;
    let index = index.rem_euclid(period);
    if index >= length as isize {
        (period - 1 - index) as usize
    } else {
        index as usize
    }
}

fn fir<T: Sample>(data: &[T], w: usize, h: usize, sigma: f64) -> Vec<T> {
    let radius = (3.0 * sigma + 0.5) as isize;
    if radius == 0 {
        return data.to_vec();
    }
    let mut kernel: Vec<f64> = (-radius..=radius)
        .map(|x| (-0.5 * (x as f64 / sigma).powi(2)).exp())
        .collect();
    let total: f64 = kernel.iter().sum();
    for value in &mut kernel {
        *value /= total;
    }
    let mut tmp = vec![T::store(0.0); data.len()];
    // Upstream's fused FIR first accumulates vertically into an image-dtype
    // temporary, rounding after every kernel contribution (not just the sum).
    for (tap, &weight) in kernel.iter().enumerate() {
        let offset = tap as isize - radius;
        for y in 0..h {
            let source = reflect(y as isize + offset, h) * w;
            for x in 0..w {
                let index = y * w + x;
                tmp[index] = T::store(tmp[index].load() + data[source + x].load() * weight);
            }
        }
    }
    let mut out = vec![T::store(0.0); data.len()];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0;
            for (tap, &weight) in kernel.iter().enumerate() {
                let source = reflect(x as isize + tap as isize - radius, w);
                sum += tmp[y * w + source].load() * weight;
            }
            out[y * w + x] = T::store(sum);
        }
    }
    out
}

fn iir<T: Sample>(data: &[T], w: usize, h: usize, sigma: f64) -> Vec<T> {
    let q = if sigma >= 2.5 {
        0.98711 * sigma - 0.96330
    } else {
        3.97156 - 4.14554 * (1.0 - 0.26891 * sigma).sqrt()
    };
    let q2 = q * q;
    let q3 = q2 * q;
    let b0 = 1.57825 + 2.44413 * q + 1.4281 * q2 + 0.422205 * q3;
    let b1 = 2.44413 * q + 2.85619 * q2 + 1.26661 * q3;
    let b2 = -(1.4281 * q2 + 1.26661 * q3);
    let b3 = 0.422205 * q3;
    let coefficients = [1.0 - (b1 + b2 + b3) / b0, b1 / b0, b2 / b0, b3 / b0];
    let mut tmp = vec![T::store(0.0); data.len()];
    for y in 0..h {
        filter_line(data, &mut tmp, y * w, 1, w, coefficients);
    }
    let mut out = vec![T::store(0.0); data.len()];
    for x in 0..w {
        filter_line(&tmp, &mut out, x, w, h, coefficients);
    }
    out
}

fn filter_line<T: Sample>(
    src: &[T],
    dst: &mut [T],
    start: usize,
    stride: usize,
    length: usize,
    [a, b1, b2, b3]: [f64; 4],
) {
    // State remains f64 even when each published recurrence value is f32.
    let mut state = [src[start].load(); 3];
    for index in 0..length {
        let offset = start + index * stride;
        let value = a * src[offset].load() + b1 * state[0] + b2 * state[1] + b3 * state[2];
        dst[offset] = T::store(value);
        state = [value, state[0], state[1]];
    }
    state = [dst[start + (length - 1) * stride].load(); 3];
    for index in (0..length).rev() {
        let offset = start + index * stride;
        let value = a * dst[offset].load() + b1 * state[0] + b2 * state[1] + b3 * state[2];
        dst[offset] = T::store(value);
        state = [value, state[0], state[1]];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_sigma_matches_upstream_iir_not_boxes() {
        // Frozen f32 output from `fast_gaussian_filter` at experimental
        // 28bf883 for a 7x5 ramp. The old local box approximation produced
        // visibly different values here; every sigma >= 3 must use IIR.
        let data: Vec<f32> = (0..35)
            .map(|index| (((index * 37 + 11) % 97) as f64 / 97.0) as f32)
            .collect();
        for (sigma, expected) in [
            (
                8.0,
                [
                    0.21965311467647552,
                    0.22280146181583405,
                    0.22518789768218994,
                    0.2268066257238388,
                    0.22774997353553772,
                    0.22818094491958618,
                    0.2283034771680832,
                    0.22065573930740356,
                    0.22379407286643982,
                    0.22617292404174805,
                ],
            ),
            (
                20.0,
                [
                    0.12586252391338348,
                    0.12590548396110535,
                    0.1259356588125229,
                    0.12595488131046295,
                    0.12596552073955536,
                    0.12597021460533142,
                    0.12597151100635529,
                    0.12587271630764008,
                    0.12591567635536194,
                    0.1259458363056183,
                ],
            ),
        ] {
            let actual = blur(&data, 7, 5, sigma);
            for (index, (&got, &want)) in actual.iter().zip(&expected).enumerate() {
                assert!(
                    (f64::from(got) - want).abs() < 1e-6,
                    "sigma {sigma} sample {index}: {got} != {want}"
                );
            }
        }
    }
}
