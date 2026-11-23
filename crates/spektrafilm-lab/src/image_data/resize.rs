use super::err;
use crate::common::Result;

pub(crate) fn lanczos(x: f64) -> f64 {
    if x == 0. {
        1.
    } else if x.abs() < 3. {
        let p = std::f64::consts::PI * x;
        (p.sin() / p) * ((p / 3.).sin() / (p / 3.))
    } else {
        0.
    }
}
pub(crate) fn resize_weights(input: usize, output: usize) -> Vec<(usize, Vec<f64>)> {
    if input == output {
        return (0..input).map(|i| (i, vec![1.])).collect();
    }
    let scale = input as f64 / output as f64;
    let filter_scale = scale.max(1.);
    let support = 3. * filter_scale;
    (0..output)
        .map(|i| {
            let center = (i as f64 + 0.5) * scale;
            let first = ((center - support + 0.5) as isize).max(0) as usize;
            let last = ((center + support + 0.5) as usize).min(input);
            let mut weights: Vec<f64> = (first..last)
                .map(|j| lanczos((j as f64 - center + 0.5) / filter_scale))
                .collect();
            let sum: f64 = weights.iter().sum();
            if sum != 0. {
                for w in &mut weights {
                    *w /= sum
                }
            }
            (first, weights)
        })
        .collect()
}
pub fn resize_display(
    samples: &[[f64; 3]],
    width: usize,
    height: usize,
    max_edge: usize,
) -> Result<(Vec<[f64; 3]>, usize, usize)> {
    if width == 0 || height == 0 || max_edge == 0 || samples.len() != width * height {
        return Err(err("invalid_image_layout", "Resize dimensions mismatch"));
    }
    let scale = (max_edge as f64 / width.max(height) as f64).min(1.);
    let w = (width as f64 * scale).round_ties_even().max(1.) as usize;
    let h = (height as f64 * scale).round_ties_even().max(1.) as usize;
    let wx = resize_weights(width, w);
    let wy = resize_weights(height, h);
    let mut temp = vec![[0f64; 3]; height * w];
    for y in 0..height {
        for (x, (first, weights)) in wx.iter().enumerate() {
            for c in 0..3 {
                let v: f64 = weights
                    .iter()
                    .enumerate()
                    .map(|(k, weight)| {
                        samples[y * width + first + k][c].clamp(0.0, 1.0) as f32 as f64 * weight
                    })
                    .sum();
                temp[y * w + x][c] = v as f32 as f64
            }
        }
    }
    let mut out = vec![[0f64; 3]; w * h];
    for (y, (first, weights)) in wy.iter().enumerate() {
        for x in 0..w {
            for c in 0..3 {
                let v: f64 = weights
                    .iter()
                    .enumerate()
                    .map(|(k, weight)| temp[(first + k) * w + x][c] * weight)
                    .sum();
                out[y * w + x][c] = (v as f32 as f64).clamp(0., 1.)
            }
        }
    }
    Ok((out, w, h))
}
