//! Independent f32-buffer oracle for the Gaussian filter.
//!
//! FIR and IIR are ported from `fast_gaussian_filter.py` at upstream commit
//! 28bf883. The large-sigma model is three reflected unit-sum boxes per axis.
//! No production GPU coefficient, kernel, reflection, or paging helpers are
//! used here, so these comparisons can detect shared implementation mistakes.

pub(super) fn blur_channel(input: &[f32], width: u32, height: u32, sigma: f64) -> Vec<f32> {
    let (width, height) = (width as usize, height as usize);
    if sigma <= 0.0 || width == 0 || height == 0 {
        return input.to_vec();
    }
    if sigma < 3.0 {
        fir(input, width, height, sigma)
    } else if sigma < 8.0 {
        iir(input, width, height, sigma)
    } else {
        boxes(input, width, height, sigma)
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

// Independently construct candidate box kernels by convolution, then choose
// the one closest to the normalized truncated Gaussian in squared error.
fn box_width(sigma: f64) -> usize {
    let center = ((4.0 * sigma * sigma + 1.0).sqrt() + 0.5).floor() as usize;
    let radius = (3.0 * sigma + 0.5) as usize;
    let gaussian: Vec<f64> = (0..=radius)
        .map(|i| (-0.5 * (i as f64 / sigma).powi(2)).exp())
        .collect();
    let norm = gaussian[0] + 2.0 * gaussian[1..].iter().sum::<f64>();
    let mut best = (f64::INFINITY, 1);
    for width in center.saturating_sub(12).max(1)..=center.saturating_add(12) {
        if width % 2 == 0 {
            continue;
        }
        let mut kernel = vec![1.0];
        for _ in 0..3 {
            let mut next = vec![0.0; kernel.len() + width - 1];
            for (i, &value) in kernel.iter().enumerate() {
                for tap in &mut next[i..i + width] {
                    *tap += value / width as f64;
                }
            }
            kernel = next;
        }
        let middle = kernel.len() / 2;
        let extent = radius.max(middle);
        let error = (0..=extent)
            .map(|i| {
                let actual = kernel.get(middle + i).copied().unwrap_or(0.0);
                let expected = gaussian.get(i).copied().unwrap_or(0.0) / norm;
                let d = actual - expected;
                d * d * if i == 0 { 1.0 } else { 2.0 }
            })
            .sum::<f64>();
        if error < best.0 {
            best = (error, width);
        }
    }
    best.1
}

fn boxes(input: &[f32], width: usize, height: usize, sigma: f64) -> Vec<f32> {
    let box_width = box_width(sigma);
    let radius = (box_width / 2) as isize;
    let mut source = input.to_vec();
    let mut destination = vec![0.0; input.len()];
    for horizontal in [true, false] {
        for _ in 0..3 {
            for y in 0..height {
                for x in 0..width {
                    let mut sum = 0.0f64;
                    for offset in -radius..=radius {
                        let (sx, sy) = if horizontal {
                            (reflected(x as isize + offset, width), y)
                        } else {
                            (x, reflected(y as isize + offset, height))
                        };
                        sum += f64::from(source[sy * width + sx]);
                    }
                    destination[y * width + x] = (sum / box_width as f64) as f32;
                }
            }
            std::mem::swap(&mut source, &mut destination);
        }
    }
    source
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
