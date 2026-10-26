//! Independent f32-buffer oracle for Spektra's pinned Gaussian implementation.
//!
//! Ported from `src/spektrafilm/utils/fast_gaussian_filter.py` at upstream
//! experimental commit 28bf883: `_fir_2d_fused`, `_yvv_coeffs`,
//! `_iir_horizontal`, and `_iir_vertical`. Do not use production GPU coefficient,
//! kernel, reflection, or paging helpers here: these comparisons must detect
//! shared mistakes in direct and paged execution. The shared math Gaussian is
//! not an oracle for Spektra: its f32 coefficients and recurrence state differ
//! from upstream's f64 arithmetic over f32 working buffers.

pub(super) fn blur_channel(input: &[f32], width: u32, height: u32, sigma: f64) -> Vec<f32> {
    let (width, height) = (width as usize, height as usize);
    if sigma <= 0.0 || width == 0 || height == 0 {
        return input.to_vec();
    }
    if sigma < 3.0 {
        fir(input, width, height, sigma)
    } else {
        iir(input, width, height, sigma)
    }
}

fn reflected(index: isize, length: usize) -> usize {
    let period = 2 * length as isize;
    let folded = index.rem_euclid(period);
    if folded < length as isize {
        folded as usize
    } else {
        (period - 1 - folded) as usize
    }
}

fn fir(input: &[f32], width: usize, height: usize, sigma: f64) -> Vec<f32> {
    let radius = (3.0 * sigma + 0.5) as isize;
    let mut weights: Vec<f64> = (-radius..=radius)
        .map(|offset| (-0.5 * (offset as f64 / sigma).powi(2)).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    for weight in &mut weights {
        *weight /= total;
    }
    let mut vertical = vec![0.0f32; input.len()];
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            // Upstream stores each vertical contribution in image.dtype.
            for (tap, &weight) in weights.iter().enumerate() {
                let row = reflected(y as isize + tap as isize - radius, height);
                vertical[index] = (f64::from(vertical[index])
                    + f64::from(input[row * width + x]) * weight)
                    as f32;
            }
        }
    }
    let mut output = vec![0.0f32; input.len()];
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0;
            for (tap, &weight) in weights.iter().enumerate() {
                let column = reflected(x as isize + tap as isize - radius, width);
                sum += f64::from(vertical[y * width + column]) * weight;
            }
            output[y * width + x] = sum as f32;
        }
    }
    output
}

fn iir(input: &[f32], width: usize, height: usize, sigma: f64) -> Vec<f32> {
    // Dispatch guarantees sigma >= 3, hence _yvv_coeffs' linear q branch.
    let q = 0.98711 * sigma - 0.96330;
    let q2 = q * q;
    let q3 = q2 * q;
    let b0 = 1.57825 + 2.44413 * q + 1.4281 * q2 + 0.422205 * q3;
    let b1 = 2.44413 * q + 2.85619 * q2 + 1.26661 * q3;
    let b2 = -(1.4281 * q2 + 1.26661 * q3);
    let b3 = 0.422205 * q3;
    let gain = 1.0 - (b1 + b2 + b3) / b0;
    let feedback = [b1 / b0, b2 / b0, b3 / b0];
    let mut output = input.to_vec();
    // Horizontal then vertical, each with full true-endpoint sweeps. No
    // knowledge of GPU page boundaries can leak into this reference.
    for horizontal in [true, false] {
        let (lines, length) = if horizontal {
            (height, width)
        } else {
            (width, height)
        };
        for line in 0..lines {
            let at = |coordinate| {
                if horizontal {
                    line * width + coordinate
                } else {
                    coordinate * width + line
                }
            };
            for forward in [true, false] {
                // Backward initialization re-reads the stored f32 endpoint,
                // not the unrounded f64 state from the forward sweep.
                let endpoint = if forward { 0 } else { length - 1 };
                let mut state = [f64::from(output[at(endpoint)]); 3];
                for step in 0..length {
                    let coordinate = if forward { step } else { length - 1 - step };
                    let index = at(coordinate);
                    let value = gain * f64::from(output[index])
                        + feedback[0] * state[0]
                        + feedback[1] * state[1]
                        + feedback[2] * state[2];
                    output[index] = value as f32;
                    // Published samples narrow, recurrence history does not.
                    state = [value, state[0], state[1]];
                }
            }
        }
    }
    output
}
