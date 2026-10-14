//! Nine-layer Polaroid development animation and the pinned virtual paper
//! back: watermark lattice, rotation and seed-zero paper glare.
use super::raster::DisplayRaster;
use rayon::prelude::*;
/// Prepared nine-layer chemical development, using the pinned Python formulas.
pub struct PolaroidState {
    size: [usize; 2],
    layers: Vec<[[f32; 3]; 9]>,
}
impl PolaroidState {
    pub fn new(image: &DisplayRaster) -> Self {
        let divisor = if image.rgb.iter().flatten().copied().fold(0.0, f32::max) > 1.0 {
            255.0
        } else {
            1.0
        };
        let density: Vec<[f32; 3]> = image
            .rgb
            .par_iter()
            .map(|p| p.map(|v| (-(v / divisor).clamp(1e-4, 1.0).powf(2.2).ln()).clamp(0.0, 6.0)))
            .collect();
        let (dmax, cmax, nmax) = density
            .par_iter()
            .fold(
                || ([0.0_f32; 3], [0.0_f32; 3], 0.0_f32),
                |(mut dmax, mut cmax, mut nmax), d| {
                    let n = d.iter().copied().fold(f32::INFINITY, f32::min);
                    nmax = nmax.max(n);
                    for c in 0..3 {
                        dmax[c] = dmax[c].max(d[c]);
                        cmax[c] = cmax[c].max(d[c] - n);
                    }
                    (dmax, cmax, nmax)
                },
            )
            .reduce(
                || ([0.0_f32; 3], [0.0_f32; 3], 0.0_f32),
                |(dmax, cmax, nmax), (dmax_other, cmax_other, nmax_other)| {
                    (
                        std::array::from_fn(|c| dmax[c].max(dmax_other[c])),
                        std::array::from_fn(|c| cmax[c].max(cmax_other[c])),
                        nmax.max(nmax_other),
                    )
                },
            );
        let layers = density
            .par_iter()
            .map(|d| {
                let n = d.iter().copied().fold(f32::INFINITY, f32::min);
                let ch = d.map(|v| v - n);
                let dl = std::array::from_fn::<_, 3, _>(|c| d[c] / (dmax[c] + 1e-4));
                let cl = std::array::from_fn::<_, 3, _>(|c| ch[c] / (cmax[c] + 1e-4));
                let shadow = (d[0] * 0.34 + d[1] * 0.39 + d[2] * 0.27).clamp(0.0, 1.75) / 1.75;
                let h = 1.0 - shadow;
                let strength = cl.iter().copied().fold(0.0, f32::max);
                let warmth = ((ch[1] + 1.25 * ch[2] - 0.85 * ch[0])
                    / (ch.iter().sum::<f32>() + 1e-4))
                    .clamp(0.0, 1.0);
                let silver = (0.24 + 0.46 * shadow + 0.30 * n / (nmax + 1e-4)).clamp(0.0, 1.0);
                let mut l = [[0.0; 3]; 9];
                l[0] = [n; 3];
                l[1][2] = ch[2];
                l[2][1] = ch[1];
                l[3][0] = ch[0];
                for c in 0..3 {
                    l[4][c] = (0.30 + 0.70 * h) * [0.04, 0.08, 0.15][c];
                    l[5][c] = (silver * [0.18, 0.19, 0.20][c]
                        + cl[(c + 1) % 3] * [0.03, 0.02, 0.04][c])
                        .clamp(0.0, 0.28);
                    l[6][c] = (h * [0.10, 0.06, 0.02][c]
                        + (1.0 - dl[(c + 2) % 3]) * [0.13, 0.07, 0.18][c]
                        + cl[(c + 1) % 3] * [0.04, 0.02, 0.06][c])
                        * (0.40 + 0.60 * h);
                    l[7][c] =
                        (0.24 + 0.42 * h + 0.34 * warmth.max(strength)) * [0.02, 0.08, 0.19][c];
                    l[8][c] = h * [0.03, 0.06, 0.11][c];
                }
                l
            })
            .collect();
        Self {
            size: image.size,
            layers,
        }
    }
    pub fn frame(&self, t: f32) -> DisplayRaster {
        let k = layer_coefficients(t);
        let rgb: Vec<[f32; 3]> = self
            .layers
            .par_iter()
            .map(|l| {
                std::array::from_fn(|c| {
                    let d = (0..9).map(|i| k[i] * l[i][c]).sum::<f32>().clamp(0.0, 8.0);
                    (-d / 2.2).exp()
                })
            })
            .collect();
        DisplayRaster {
            size: self.size,
            rgb: rgb.into(),
        }
    }
}
pub fn layer_coefficients(t: f32) -> [f32; 9] {
    let t = t.clamp(0.0, 1.0);
    let sigmoid = |t: f32, c: f32, s: f32| {
        let expit = |x: f32| 1.0 / (1.0 + (-x.clamp(-60.0, 60.0)).exp());
        let start = expit(-c * s);
        ((expit((t - c) * s) - start) / (expit((1.0 - c) * s) - start + 1e-4)).clamp(0.0, 1.0)
    };
    let rise = |c, s, p| {
        let r = sigmoid(t, c, s);
        r + (1.0 - r) * t.powf(p)
    };
    let pulse = |a, b, c, d, p| sigmoid(t, a, b) * (1.0 - sigmoid(t, c, d)) * (1.0 - t).powf(p);
    let body = rise(0.19, 7.4, 3.8);
    [
        body * rise(0.12, 8.0, 2.8),
        body * rise(0.22, 11.0, 3.0),
        body * rise(0.31, 10.2, 3.4),
        body * rise(0.52, 9.4, 5.4),
        1.08 * (1.0 - t).powf(0.46),
        pulse(0.05, 32.0, 0.44, 8.0, 0.24),
        1.22 * pulse(0.10, 54.0, 0.28, 28.0, 0.18),
        0.96 * pulse(0.12, 20.0, 0.80, 6.5, 0.34),
        0.68 * pulse(0.03, 24.0, 0.98, 4.0, 0.08),
    ]
}

// Original pinned virtual_paper_watermark.png embedded so the viewer requires no
// Python installation or runtime asset path.
fn logo_alpha() -> (usize, usize, Vec<f32>) {
    let image = image::load_from_memory(include_bytes!("../../assets/virtual_paper_watermark.png"))
        .expect("embedded watermark PNG")
        .to_rgba8();
    (
        image.width() as usize,
        image.height() as usize,
        image.pixels().map(|p| p[3] as f32 / 255.0).collect(),
    )
}
// NumPy default_rng(0): PCG64 with the pinned SeedSequence state. NumPy consumes
// low then high u32 words for float32, and discards their low eight bits.
struct PaperRng {
    state: u128,
    pending: Option<u32>,
}
impl PaperRng {
    fn new() -> Self {
        Self {
            state: 35399562948360463058890781895381311971,
            pending: None,
        }
    }
    fn next(&mut self) -> f32 {
        let word = if let Some(word) = self.pending.take() {
            word
        } else {
            self.state = self
                .state
                .wrapping_mul(47026247687942121848144207491837523525)
                .wrapping_add(87136372517582989555478159403783844777);
            let hi = (self.state >> 64) as u64;
            let lo = self.state as u64;
            let value = (hi ^ lo).rotate_right((self.state >> 122) as u32);
            self.pending = Some((value >> 32) as u32);
            value as u32
        };
        (word >> 8) as f32 * (1.0 / 16777216.0)
    }
}
fn alpha_sample(alpha: &[f32], width: usize, height: usize, x: f32, y: f32) -> f32 {
    if x < 0.0 || y < 0.0 || x > (width - 1) as f32 || y > (height - 1) as f32 {
        return 0.0;
    }
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let a = x - x0 as f32;
    let b = y - y0 as f32;
    (alpha[y0 * width + x0] * (1.0 - a) + alpha[y0 * width + x1] * a) * (1.0 - b)
        + (alpha[y1 * width + x0] * (1.0 - a) + alpha[y1 * width + x1] * a) * b
}
/// Pinned 72° rhombic watermark lattice, 16° rotation, seed-zero paper glare.
pub fn virtual_paper_back(size: [usize; 2]) -> DisplayRaster {
    let (sw, sh, mut alpha) = logo_alpha();
    let long = size[0].max(size[1]) as f32;
    let scale = 0.22 * long / sw.max(sh) as f32;
    let sigma = (0.5 / scale - 0.5).max(0.0);
    if sigma > 0.0 {
        let radius = (4.0 * sigma + 0.5).floor() as i32;
        let mut kernel: Vec<f32> = (-radius..=radius)
            .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
            .collect();
        let sum = kernel.iter().sum::<f32>();
        for w in &mut kernel {
            *w /= sum;
        }
        let mut temp = vec![0.0; alpha.len()];
        for y in 0..sh {
            for x in 0..sw {
                temp[y * sw + x] = (-radius..=radius)
                    .map(|i| {
                        alpha[y * sw + (x as i32 + i).clamp(0, sw as i32 - 1) as usize]
                            * kernel[(i + radius) as usize]
                    })
                    .sum();
            }
        }
        for y in 0..sh {
            for x in 0..sw {
                alpha[y * sw + x] = (-radius..=radius)
                    .map(|i| {
                        temp[(y as i32 + i).clamp(0, sh as i32 - 1) as usize * sw + x]
                            * kernel[(i + radius) as usize]
                    })
                    .sum();
            }
        }
    }
    let tw = (sw as f32 * scale).round().max(1.0) as usize;
    let th = (sh as f32 * scale).round().max(1.0) as usize;
    let mut tile = vec![0.0; tw * th];
    for y in 0..th {
        for x in 0..tw {
            tile[y * tw + x] = alpha_sample(
                &alpha,
                sw,
                sh,
                x as f32 * (sw - 1) as f32 / (tw - 1).max(1) as f32,
                y as f32 * (sh - 1) as f32 / (th - 1).max(1) as f32,
            );
        }
    }
    let angle = 16_f32.to_radians();
    let (sin, cos) = angle.sin_cos();
    let rw = (cos.abs() * tw as f32 + sin.abs() * th as f32 + 0.5).floor() as usize;
    let rh = (sin.abs() * tw as f32 + cos.abs() * th as f32 + 0.5).floor() as usize;
    let mut rotated = vec![0.0; rw * rh];
    let mut xmin = rw;
    let mut xmax = 0;
    let mut ymin = rh;
    let mut ymax = 0;
    for y in 0..rh {
        for x in 0..rw {
            let dx = x as f32 - (rw - 1) as f32 / 2.0;
            let dy = y as f32 - (rh - 1) as f32 / 2.0;
            let a = alpha_sample(
                &tile,
                tw,
                th,
                cos * dx - sin * dy + (tw - 1) as f32 / 2.0,
                sin * dx + cos * dy + (th - 1) as f32 / 2.0,
            );
            rotated[y * rw + x] = a;
            if a > 1e-4 {
                xmin = xmin.min(x);
                xmax = xmax.max(x);
                ymin = ymin.min(y);
                ymax = ymax.max(y);
            }
        }
    }
    let stamp_w = xmax - xmin + 1;
    let stamp_h = ymax - ymin + 1;
    let radius = 0.5 * ((stamp_w * stamp_w + stamp_h * stamp_h) as f32).sqrt();
    let half = 36_f32.to_radians();
    let distance = 0.25 * long;
    let basis = [
        [
            distance * (half.cos() * cos + half.sin() * sin),
            distance * (-half.cos() * sin + half.sin() * cos),
        ],
        [
            distance * (half.cos() * cos - half.sin() * sin),
            distance * (-half.cos() * sin - half.sin() * cos),
        ],
    ];
    let limit =
        (((size[0] as f32).hypot(size[1] as f32) + 2.0 * radius) / distance).ceil() as i32 + 2;
    let mut centers = Vec::new();
    for i in -limit..=limit {
        for j in -limit..=limit {
            let x = size[0] as f32 * 0.5 + i as f32 * basis[0][0] + j as f32 * basis[1][0];
            let y = size[1] as f32 * 0.5 + i as f32 * basis[0][1] + j as f32 * basis[1][1];
            if x >= -radius
                && x <= size[0] as f32 + radius
                && y >= -radius
                && y <= size[1] as f32 + radius
            {
                centers.push((x, y));
            }
        }
    }
    centers.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut trans = vec![1.0; size[0] * size[1]];
    for (cx, cy) in centers {
        let x0 = (cx - 0.5 * stamp_w as f32).round_ties_even() as i32;
        let y0 = (cy - 0.5 * stamp_h as f32).round_ties_even() as i32;
        for y in 0..stamp_h {
            let dy = y0 + y as i32;
            if dy < 0 || dy >= size[1] as i32 {
                continue;
            }
            for x in 0..stamp_w {
                let dx = x0 + x as i32;
                if dx >= 0 && dx < size[0] as i32 {
                    trans[dy as usize * size[0] + dx as usize] *=
                        1.0 - rotated[(y + ymin) * rw + x + xmin];
                }
            }
        }
    }
    let mut rng = PaperRng::new();
    let a: Vec<f32> = (0..trans.len()).map(|_| rng.next()).collect();
    let rgb = trans
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let g = 0.88 * a[i] + 0.12 * rng.next().powf(10.0);
            let paper = 0.30 + 0.70 * t;
            let variation = 1.0 + (2.0 * g - 1.0) * paper * 0.07 * 0.25;
            let lift = g.powi(6) * paper * 0.42 * 0.25;
            let base = (215.0 / 255.0 + t * 27.0 / 255.0) * variation;
            let value = (base + (1.0 - base) * lift).clamp(0.0, 1.0);
            let value = (value * 255.0).round_ties_even() / 255.0;
            [value; 3]
        })
        .collect();
    DisplayRaster { size, rgb }
}

#[cfg(test)]
pub(crate) fn virtual_paper_back_without_watermark(size: [usize; 2]) -> DisplayRaster {
    let (sw, sh, mut alpha) = logo_alpha();
    let long = size[0].max(size[1]) as f32;
    let scale = 0.22 * long / sw.max(sh) as f32;
    let sigma = (0.5 / scale - 0.5).max(0.0);
    if sigma > 0.0 {
        let radius = (4.0 * sigma + 0.5).floor() as i32;
        let mut kernel: Vec<f32> = (-radius..=radius)
            .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
            .collect();
        let sum = kernel.iter().sum::<f32>();
        for w in &mut kernel {
            *w /= sum;
        }
        let mut temp = vec![0.0; alpha.len()];
        for y in 0..sh {
            for x in 0..sw {
                temp[y * sw + x] = (-radius..=radius)
                    .map(|i| {
                        alpha[y * sw + (x as i32 + i).clamp(0, sw as i32 - 1) as usize]
                            * kernel[(i + radius) as usize]
                    })
                    .sum();
            }
        }
        for y in 0..sh {
            for x in 0..sw {
                alpha[y * sw + x] = (-radius..=radius)
                    .map(|i| {
                        temp[(y as i32 + i).clamp(0, sh as i32 - 1) as usize * sw + x]
                            * kernel[(i + radius) as usize]
                    })
                    .sum();
            }
        }
    }
    let tw = (sw as f32 * scale).round().max(1.0) as usize;
    let th = (sh as f32 * scale).round().max(1.0) as usize;
    let mut tile = vec![0.0; tw * th];
    for y in 0..th {
        for x in 0..tw {
            tile[y * tw + x] = alpha_sample(
                &alpha,
                sw,
                sh,
                x as f32 * (sw - 1) as f32 / (tw - 1).max(1) as f32,
                y as f32 * (sh - 1) as f32 / (th - 1).max(1) as f32,
            );
        }
    }
    let angle = 16_f32.to_radians();
    let (sin, cos) = angle.sin_cos();
    let rw = (cos.abs() * tw as f32 + sin.abs() * th as f32 + 0.5).floor() as usize;
    let rh = (sin.abs() * tw as f32 + cos.abs() * th as f32 + 0.5).floor() as usize;
    let mut rotated = vec![0.0; rw * rh];
    let mut xmin = rw;
    let mut xmax = 0;
    let mut ymin = rh;
    let mut ymax = 0;
    for y in 0..rh {
        for x in 0..rw {
            let dx = x as f32 - (rw - 1) as f32 / 2.0;
            let dy = y as f32 - (rh - 1) as f32 / 2.0;
            let a = alpha_sample(
                &tile,
                tw,
                th,
                cos * dx - sin * dy + (tw - 1) as f32 / 2.0,
                sin * dx + cos * dy + (th - 1) as f32 / 2.0,
            );
            rotated[y * rw + x] = a;
            if a > 1e-4 {
                xmin = xmin.min(x);
                xmax = xmax.max(x);
                ymin = ymin.min(y);
                ymax = ymax.max(y);
            }
        }
    }
    let stamp_w = xmax - xmin + 1;
    let stamp_h = ymax - ymin + 1;
    let radius = 0.5 * ((stamp_w * stamp_w + stamp_h * stamp_h) as f32).sqrt();
    let half = 36_f32.to_radians();
    let distance = 0.25 * long;
    let basis = [
        [
            distance * (half.cos() * cos + half.sin() * sin),
            distance * (-half.cos() * sin + half.sin() * cos),
        ],
        [
            distance * (half.cos() * cos - half.sin() * sin),
            distance * (-half.cos() * sin - half.sin() * cos),
        ],
    ];
    let limit =
        (((size[0] as f32).hypot(size[1] as f32) + 2.0 * radius) / distance).ceil() as i32 + 2;
    let mut centers = Vec::new();
    for i in -limit..=limit {
        for j in -limit..=limit {
            let x = size[0] as f32 * 0.5 + i as f32 * basis[0][0] + j as f32 * basis[1][0];
            let y = size[1] as f32 * 0.5 + i as f32 * basis[0][1] + j as f32 * basis[1][1];
            if x >= -radius
                && x <= size[0] as f32 + radius
                && y >= -radius
                && y <= size[1] as f32 + radius
            {
                centers.push((x, y));
            }
        }
    }
    centers.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let trans = vec![1.0; size[0] * size[1]];
    let mut rng = PaperRng::new();
    let a: Vec<f32> = (0..trans.len()).map(|_| rng.next()).collect();
    let rgb = trans
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let g = 0.88 * a[i] + 0.12 * rng.next().powf(10.0);
            let paper = 0.30 + 0.70 * t;
            let variation = 1.0 + (2.0 * g - 1.0) * paper * 0.07 * 0.25;
            let lift = g.powi(6) * paper * 0.42 * 0.25;
            let base = (215.0 / 255.0 + t * 27.0 / 255.0) * variation;
            let value = (base + (1.0 - base) * lift).clamp(0.0, 1.0);
            let value = (value * 255.0).round_ties_even() / 255.0;
            [value; 3]
        })
        .collect();
    DisplayRaster { size, rgb }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the output bits of the nine development frames. The nine-layer
    /// formulas are the pinned Python ones, and the parallel iteration in
    /// `PolaroidState` must not move a single float, so this hash fails on any
    /// numerical drift in the layers, the coefficients, or the frame.
    #[test]
    fn development_frames_keep_the_pinned_formulas() {
        let (w, h) = (301usize, 197usize);
        let rgb: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                let v = (i % 251) as f32 / 255.0;
                [v, (v * 0.83).fract(), 1.0 - v]
            })
            .collect();
        let raster = DisplayRaster::new([w, h], rgb).unwrap();
        let state = PolaroidState::new(&raster);
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for step in 0..=8 {
            let frame = state.frame(step as f32 / 8.0);
            assert_eq!(frame.size, [w, h]);
            for pixel in frame.rgb.iter() {
                for channel in pixel {
                    for byte in channel.to_bits().to_le_bytes() {
                        hash = (hash ^ byte as u64).wrapping_mul(0x1000_0000_01b3);
                    }
                }
            }
        }
        assert_eq!(hash, 0x520f_1920_404c_129f);
    }
}
