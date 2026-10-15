use crate::common::Result;
use rustfft::{FftPlanner, num_complex::Complex};
use serde_json::Value;

use super::err;

const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];
pub fn luminance(p: [f64; 3]) -> f64 {
    p[0] * LUMA[0] + p[1] * LUMA[1] + p[2] * LUMA[2]
}
pub fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}
pub fn quantile(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let index = (sorted.len() - 1) as f64 * q;
    let low = index.floor() as usize;
    let high = index.ceil() as usize;
    let fraction = index - low as f64;
    let difference = sorted[high] - sorted[low];
    Some(if fraction >= 0.5 {
        sorted[high] - difference * (1.0 - fraction)
    } else {
        sorted[low] + difference * fraction
    })
}
pub fn sort(v: &mut [f64]) {
    v.sort_by(f64::total_cmp);
}
pub fn median(v: &mut [f64]) -> Option<f64> {
    sort(v);
    quantile(v, 0.5)
}
pub fn selected(v: &[f64], mask: &[bool]) -> Vec<f64> {
    v.iter()
        .zip(mask)
        .filter_map(|(x, m)| m.then_some(*x))
        .collect()
}
pub fn count(mask: &[bool]) -> usize {
    mask.iter().filter(|m| **m).count()
}
pub fn number(settings: &Value, key: &str) -> Result<f64> {
    settings
        .get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| err("invalid_protocol", format!("{key} must be finite numeric")))
}
// Python's str distinguishes an integer scale from a floating scale (3 vs 3.0).
pub fn spelling(value: &Value) -> String {
    value.to_string()
}
pub fn reflect(i: isize, length: usize) -> usize {
    let period = 2 * length as isize;
    let i = i.rem_euclid(period);
    if i >= length as isize {
        (period - 1 - i) as usize
    } else {
        i as usize
    }
}
pub fn gaussian(v: &[f64], width: usize, height: usize, sigma: f64) -> Vec<f64> {
    if sigma <= 1e-15 {
        return v.to_vec();
    }
    let radius = (4.0 * sigma + 0.5).floor() as isize;
    let mut kernel: Vec<f64> = (-radius..=radius)
        .map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp())
        .collect();
    let total = kernel.iter().sum::<f64>();
    for k in &mut kernel {
        *k /= total;
    }
    let mut temporary = vec![0.0; v.len()];
    let mut result = vec![0.0; v.len()];
    // scipy.ndimage processes axes in order: vertical then horizontal.
    for y in 0..height {
        for x in 0..width {
            let mut value = v[y * width + x] * kernel[radius as usize];
            for j in 1..=radius {
                value += (v[reflect(y as isize - j, height) * width + x]
                    + v[reflect(y as isize + j, height) * width + x])
                    * kernel[(radius + j) as usize];
            }
            temporary[y * width + x] = value;
        }
    }
    for y in 0..height {
        for x in 0..width {
            let mut value = temporary[y * width + x] * kernel[radius as usize];
            for j in 1..=radius {
                value += (temporary[y * width + reflect(x as isize - j, width)]
                    + temporary[y * width + reflect(x as isize + j, width)])
                    * kernel[(radius + j) as usize];
            }
            result[y * width + x] = value;
        }
    }
    result
}

pub fn periodic_hann(index: usize, length: usize) -> f64 {
    if length == 1 {
        1.0
    } else {
        0.5 - 0.5 * (2.0 * std::f64::consts::PI * index as f64 / length as f64).cos()
    }
}
pub fn fft2(
    values: &mut [Complex<f64>],
    width: usize,
    height: usize,
    inverse: bool,
    planner: &mut FftPlanner<f64>,
) {
    let row = if inverse {
        planner.plan_fft_inverse(width)
    } else {
        planner.plan_fft_forward(width)
    };
    let column = if inverse {
        planner.plan_fft_inverse(height)
    } else {
        planner.plan_fft_forward(height)
    };
    let mut scratch = vec![
        Complex::default();
        row.get_inplace_scratch_len()
            .max(column.get_inplace_scratch_len())
    ];
    for line in values.chunks_exact_mut(width) {
        row.process_with_scratch(line, &mut scratch);
    }
    let mut line = vec![Complex::default(); height];
    for x in 0..width {
        for y in 0..height {
            line[y] = values[y * width + x];
        }
        column.process_with_scratch(&mut line, &mut scratch);
        for y in 0..height {
            values[y * width + x] = line[y];
        }
    }
}
pub fn frequency(index: usize, length: usize) -> f64 {
    // np.fft.fftfreq multiplies the integer lattice by one precomputed reciprocal.
    // Division per coordinate changes rounding at radial annulus boundaries.
    (index as isize - (length / 2) as isize) as f64 * (1.0 / length as f64)
}
