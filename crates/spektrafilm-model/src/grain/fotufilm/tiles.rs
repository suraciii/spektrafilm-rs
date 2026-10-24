use super::*;
use spektrafilm_math::precision::{from_f64, to_f64};
use std::sync::{Arc, LazyLock, Mutex};
const N: usize = 256;
const SAMPLE: usize = 768;
const SEED: u64 = 0xA11C40575EED0001;
const GOLDEN: u32 = 0x9E3779B9;
#[derive(Debug, Clone)]
pub struct TileLevel {
    pub sums: Vec<f32>,
    pub mean_light: f64,
}
#[derive(Debug, Clone)]
pub struct PitchTables {
    pub mean_density: [Vec<f64>; 3],
    pub blend_correlation: [Vec<f64>; 3],
}
#[derive(Debug)]
pub struct Bank {
    pub levels: [Vec<TileLevel>; 3],
    pub d_min: [f64; 3],
    pub d_max: [f64; 3],
    pub monochrome: bool,
    diagnostics: Diagnostics,
    pitches: Mutex<Vec<([u64; 2], Arc<PitchTables>)>>,
}
pub(super) fn pcg(v: u32) -> u32 {
    let s = v.wrapping_mul(747796405).wrapping_add(2891336453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277803737);
    (w >> 22) ^ w
}
fn key(seed: u32, stream: u32, x: usize, y: usize) -> u32 {
    pcg(x as u32 ^ pcg(y as u32 ^ pcg(seed ^ stream.wrapping_mul(GOLDEN))))
}
fn uniform(k: u32, i: u32) -> f32 {
    ((pcg(k ^ i.wrapping_mul(GOLDEN)) >> 8) as f32 + 0.5) * (1.0 / 16777216.0)
}
/// The Poisson count thresholds of a coating cell, in the source's f32 law.
///
/// `light` draws one uniform per cell and counts the thresholds below it, so
/// the device receives the same 48 values the host counts against.
pub(super) fn thresholds(mean: f64) -> [f32; 48] {
    let mut term = (-mean).exp();
    let mut sum = term;
    std::array::from_fn(|k| {
        if k > 0 {
            term *= mean / k as f64;
            sum += term;
        }
        sum.min(1.0) as f32
    })
}
/// The pinned formation seed, as `light` derives it.
pub(super) fn formation_seed() -> u32 {
    pcg(SEED as u32) ^ (SEED >> 32) as u32
}
/// The 17 covariance-window sharing weights of the sigma48 measurement.
///
/// The table is computed once on first use and read unchanged afterwards, so
/// the device and the host anchor measurement share one law.
pub(super) fn sharing_table() -> [f64; 17] {
    static TABLE: LazyLock<[f64; 17]> = LazyLock::new(|| {
        std::array::from_fn(|lag| {
            let mut total = 0.0;
            for i in 0..2048 {
                let c = (i as f64 + 0.5) / 16.0;
                let d = c + lag as f64;
                let (mut s, mut a, mut b) = (0.0, 0.0, 0.0);
                for block in -1..=3 {
                    let wc = weight(c, block);
                    let wd = weight(d, block);
                    s += wc * wd;
                    a += wc * wc;
                    b += wd * wd;
                }
                total += s / (a * b).sqrt();
            }
            total / 2048.0
        })
    });
    *TABLE
}
fn wrap(i: isize, n: usize) -> usize {
    i.rem_euclid(n as isize) as usize
}
fn kernel(s: f32) -> Vec<f32> {
    let radius = (3.5 * s).ceil().max(1.0) as isize;
    let mut k: Vec<f32> = (-radius..=radius)
        .map(|i| (-(i * i) as f32 / (2.0 * s * s)).exp())
        .collect();
    let sum = k.iter().sum::<f32>();
    for x in &mut k {
        *x /= sum;
    }
    k
}
fn blur(
    source: &[f32],
    n: usize,
    sigma: f32,
    cancel: Cancellation<'_>,
) -> Result<Vec<f32>, String> {
    let k = kernel(sigma);
    let radius = (k.len() / 2) as isize;
    let mut across = vec![0.0; n * n];
    let mut out = vec![0.0; n * n];
    for y in 0..n {
        if y % 32 == 0 {
            (cancel)()?;
        }
        for x in 0..n {
            let mut v = 0.0;
            for (t, w) in k.iter().enumerate() {
                v += w * source[y * n + wrap(x as isize + t as isize - radius, n)];
            }
            across[y * n + x] = v;
        }
    }
    for y in 0..n {
        if y % 32 == 0 {
            (cancel)()?;
        }
        for x in 0..n {
            let mut v = 0.0;
            for (t, w) in k.iter().enumerate() {
                v += w * across[wrap(y as isize + t as isize - radius, n) * n + x];
            }
            out[y * n + x] = v;
        }
    }
    Ok(out)
}
fn cloud_term(
    deposit: &[f32],
    sigma: f32,
    weight: f32,
    cancel: Cancellation<'_>,
) -> Result<Vec<f32>, String> {
    let mut factor = (sigma / 1.6).floor().max(1.0) as usize;
    while factor > 1 && !SAMPLE.is_multiple_of(factor) {
        factor -= 1;
    }
    let scale = weight * 2.0 * std::f32::consts::PI * sigma * sigma;
    if factor == 1 {
        let mut b = blur(deposit, SAMPLE, sigma, cancel)?;
        for x in &mut b {
            *x *= scale;
        }
        return Ok(b);
    }
    let m = SAMPLE / factor;
    let f = factor as f32;
    let tent: Vec<f32> = (0..3 * factor)
        .map(|t| (1.0 - ((t as f32 + 0.5) / f - 1.5).abs()).max(0.0))
        .collect();
    let mut rows = vec![0.0; m * SAMPLE];
    let mut cells = vec![0.0; m * m];
    for y in 0..SAMPLE {
        for x in 0..m {
            let mut s = 0.0;
            for (t, w) in tent.iter().enumerate() {
                s += w * deposit[y * SAMPLE
                    + wrap((factor * x) as isize - factor as isize + t as isize, SAMPLE)];
            }
            rows[y * m + x] = s;
        }
    }
    for y in 0..m {
        for x in 0..m {
            let mut s = 0.0;
            for (t, w) in tent.iter().enumerate() {
                s += w * rows
                    [wrap((factor * y) as isize - factor as isize + t as isize, SAMPLE) * m + x];
            }
            cells[y * m + x] = s;
        }
    }
    let blurred = blur(&cells, m, (sigma * sigma - f * f / 3.0).sqrt() / f, cancel)?;
    let mut out = vec![0.0; SAMPLE * SAMPLE];
    for y in 0..SAMPLE {
        let v = (y as f32 + 0.5) / f - 0.5;
        let iy = v.floor() as isize;
        let ay = v - iy as f32;
        for x in 0..SAMPLE {
            let u = (x as f32 + 0.5) / f - 0.5;
            let ix = u.floor() as isize;
            let ax = u - ix as f32;
            let a = (blurred[wrap(iy, m) * m + wrap(ix, m)] * (1.0 - ax)
                + blurred[wrap(iy, m) * m + wrap(ix + 1, m)] * ax)
                * (1.0 - ay);
            let b = (blurred[wrap(iy + 1, m) * m + wrap(ix, m)] * (1.0 - ax)
                + blurred[wrap(iy + 1, m) * m + wrap(ix + 1, m)] * ax)
                * ay;
            out[y * SAMPLE + x] = scale / (f * f) * (a + b);
        }
    }
    Ok(out)
}
fn forming(layer: &Sublayer, record: &population::Record, gross: f64) -> f32 {
    let t =
        ((gross - record.d_min) / (record.d_max - record.d_min).max(1e-6)).clamp(0.0, 1.0) * 128.0;
    let i = (t as usize).min(127);
    let f = t - i as f64;
    (layer.forming[i] * (1.0 - f) + layer.forming[i + 1] * f) as f32
}
fn light(
    record: &population::Record,
    r: usize,
    gross: f64,
    cancel: Cancellation<'_>,
) -> Result<Vec<f32>, String> {
    let seed = pcg(SEED as u32) ^ (SEED >> 32) as u32;
    let mut log_light = vec![0.0_f32; SAMPLE * SAMPLE];
    for (b, layer) in record.sublayers.iter().enumerate() {
        (cancel)()?;
        let cells = (0.256 / layer.cell_mm).round().max(1.0) as usize;
        let cell_mm = 0.256 / cells as f64;
        let cell = SAMPLE as f32 / cells as f32;
        let ts =
            thresholds((layer.crystals_per_mm2 as f32 * cell_mm as f32 * cell_mm as f32) as f64);
        let fraction = forming(layer, record, gross);
        let mut deposits = vec![0.0_f32; SAMPLE * SAMPLE];
        let mut uncovered = if layer.silver {
            vec![0.0_f32; SAMPLE * SAMPLE]
        } else {
            Vec::new()
        };
        let sigma = layer.sigma_mm as f32 / (0.001 / 3.0);
        let resolved = ((sigma - 0.5) / 1.5).clamp(0.0, 1.0);
        for cy in 0..cells {
            if cy % 16 == 0 {
                (cancel)()?;
            }
            for cx in 0..cells {
                let count_key = key(seed, (r * 8 + b * 2 + 1) as u32, cx, cy);
                let u = uniform(count_key, 0);
                let count = ts.iter().filter(|t| **t < u).count();
                let k = key(seed, (r * 8 + b * 2) as u32, cx, cy);
                for c in 0..count {
                    let i = (c * 7) as u32;
                    let px = (cx as f32 + uniform(k, i)) * cell;
                    let py = (cy as f32 + uniform(k, i + 1)) * cell;
                    let develop = uniform(k, i + 2);
                    let mut mark = 0.0_f32;
                    for j in 0..4 {
                        mark -= uniform(k, i + 3 + j).max(1e-7).ln();
                    }
                    mark /= 4.0;
                    if develop >= fraction {
                        continue;
                    }
                    let peak = layer.edge as f32 * mark;
                    if !layer.silver {
                        let u = px - 0.5;
                        let v = py - 0.5;
                        let ix = u.floor() as isize;
                        let iy = v.floor() as isize;
                        let ax = u - ix as f32;
                        let ay = v - iy as f32;
                        for (dy, wy) in [(0, 1.0 - ay), (1, ay)] {
                            for (dx, wx) in [(0, 1.0 - ax), (1, ax)] {
                                deposits[wrap(iy + dy, SAMPLE) * SAMPLE + wrap(ix + dx, SAMPLE)] +=
                                    peak * wx * wy;
                            }
                        }
                    } else {
                        if resolved > 0.0 {
                            let reach = 4.2 * sigma;
                            let x0 = (px - reach).floor() as isize;
                            let x1 = (px + reach).ceil() as isize;
                            let y0 = (py - reach).floor() as isize;
                            let y1 = (py + reach).ceil() as isize;
                            for y in y0..=y1 {
                                let dy = y as f32 + 0.5 - py;
                                let row =
                                    peak * resolved * (-dy * dy / (2.0 * sigma * sigma)).exp();
                                if row < 1e-5 {
                                    continue;
                                }
                                for x in x0..=x1 {
                                    let dx = x as f32 + 0.5 - px;
                                    deposits[wrap(y, SAMPLE) * SAMPLE + wrap(x, SAMPLE)] +=
                                        row * (-dx * dx / (2.0 * sigma * sigma)).exp();
                                }
                            }
                        }
                        if resolved < 1.0 {
                            let side = sigma
                                * (population::occupied_area(peak as f64, true) as f32).sqrt();
                            let left = px - side / 2.0;
                            let right = px + side / 2.0;
                            let top = py - side / 2.0;
                            let bottom = py + side / 2.0;
                            for y in top.floor() as isize..=bottom.floor() as isize {
                                let oy = bottom.min(y as f32 + 1.0) - top.max(y as f32);
                                if oy <= 0.0 {
                                    continue;
                                }
                                for x in left.floor() as isize..=right.floor() as isize {
                                    let ox = right.min(x as f32 + 1.0) - left.max(x as f32);
                                    if ox > 0.0 {
                                        uncovered[wrap(y, SAMPLE) * SAMPLE + wrap(x, SAMPLE)] +=
                                            (1.0 - (1.0 - resolved) * ox * oy).max(1e-6).ln();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let demand = if layer.silver {
            deposits
        } else {
            let mut sum = vec![0.0_f32; SAMPLE * SAMPLE];
            for (s, w) in [
                (0.1162, 0.1096),
                (0.3089, 0.2167),
                (0.7469, 0.4030),
                (1.732, 0.2510),
            ] {
                let term = cloud_term(&deposits, s * sigma, w, cancel)?;
                for (i, t) in sum.iter_mut().zip(term) {
                    *i += t;
                }
            }
            sum
        };
        let capacity = layer.capacity as f32;
        let full = (-std::f32::consts::LN_10 * capacity).exp();
        for i in 0..log_light.len() {
            let d = capacity * (1.0 - (-demand[i]).exp());
            if layer.silver {
                let u = uncovered[i].exp();
                let passed = u * (-std::f32::consts::LN_10 * d).exp() + (1.0 - u) * full;
                log_light[i] += passed.max(1e-12).ln();
            } else {
                log_light[i] -= std::f32::consts::LN_10 * d;
            }
        }
    }
    let mut out = vec![0.0; N * N];
    for y in 0..N {
        for x in 0..N {
            let mut t = 0.0;
            for dy in 0..3 {
                for dx in 0..3 {
                    t += log_light[(3 * y + dy) * SAMPLE + 3 * x + dx].exp();
                }
            }
            out[y * N + x] = t / 9.0;
        }
    }
    Ok(out)
}
impl TileLevel {
    pub fn from_light(light: &[f32]) -> Result<Self, String> {
        if light.len() != N * N || light.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err("Fotufilm tile requires 256x256 positive finite transmittance".into());
        }
        let mean = light.iter().map(|x| *x as f64).sum::<f64>() / (N * N) as f64;
        let mut sums = vec![0.0; (N + 1) * (N + 1)];
        let mut columns = vec![0.0; N];
        for y in 0..N {
            let mut run = 0.0;
            for x in 0..N {
                columns[x] += light[y * N + x] as f64 - mean;
                run += columns[x];
                sums[(y + 1) * (N + 1) + x + 1] = run as f32;
            }
        }
        Ok(Self {
            sums,
            mean_light: mean as f32 as f64,
        })
    }
    fn inside(&self, x: f64, y: f64) -> f64 {
        let ix = (x.floor() as usize).min(N - 1);
        let iy = (y.floor() as usize).min(N - 1);
        let a = x - ix as f64;
        let b = y - iy as f64;
        let s00 = self.sums[iy * (N + 1) + ix] as f64;
        let s10 = self.sums[iy * (N + 1) + ix + 1] as f64;
        let s01 = self.sums[(iy + 1) * (N + 1) + ix] as f64;
        let s11 = self.sums[(iy + 1) * (N + 1) + ix + 1] as f64;
        s00 + (s10 - s00) * a + (s01 - s00) * b + (s11 - s10 - s01 + s00) * a * b
    }
    fn periodic_sum(&self, x: f64, y: f64) -> f64 {
        let n = N as f64;
        let qx = (x / n).floor();
        let qy = (y / n).floor();
        let rx = x - qx * n;
        let ry = y - qy * n;
        self.inside(rx, ry)
            + qx * self.inside(n, ry)
            + qy * self.inside(rx, n)
            + qx * qy * self.inside(n, n)
    }
    /// Complete periodic texel-area light integral; arbitrary finite rectangles, no margin clip.
    pub fn rectangle_light(&self, x0: f64, x1: f64, y0: f64, y1: f64) -> f64 {
        let area = (x1 - x0) * (y1 - y0);
        let s = self.periodic_sum(x1, y1) - self.periodic_sum(x0, y1) - self.periodic_sum(x1, y0)
            + self.periodic_sum(x0, y0);
        self.mean_light + s / area
    }
}
fn weight(c: f64, b: i64) -> f64 {
    let start = b as f64 * 128.0;
    ((c - start + 16.0) / 32.0).clamp(0.0, 1.0) * ((start + 144.0 - c) / 32.0).clamp(0.0, 1.0)
}
fn sharing(lag: usize) -> f64 {
    sharing_table()[lag]
}
// The source stores ln(10) rounded to ten digits (`FilmGrain.swift:715`);
// keeping the same digits preserves the ported arithmetic.
#[allow(clippy::approx_constant)]
fn sigma48(light: &[f32], cancel: Cancellation<'_>) -> Result<f64, String> {
    let mean = light.iter().map(|x| *x as f64).sum::<f64>() / (N * N) as f64;
    let d: Vec<f64> = light.iter().map(|x| *x as f64 - mean).collect();
    let mut power = 0.0;
    for dy in -16_isize..=16 {
        (cancel)()?;
        for dx in -16_isize..=16 {
            let mut c = 0.0;
            for y in 0..N {
                let other = wrap(y as isize + dy, N) * N;
                for x in 0..N {
                    c += d[y * N + x] * d[other + wrap(x as isize + dx, N)];
                }
            }
            power += c / (N * N) as f64 * sharing(dx.unsigned_abs()) * sharing(dy.unsigned_abs());
        }
    }
    Ok((power.max(0.0) / (std::f64::consts::PI * 24.0 * 24.0)).sqrt() / (mean * 2.302585093))
}
pub(super) fn anchor(
    p: &mut Population,
    gross: [f64; 3],
    cancel: Cancellation<'_>,
) -> Result<(), String> {
    let active: Vec<usize> = if p.monochrome { vec![1] } else { vec![0, 1, 2] };
    let mut factors = [1.0_f64; 3];
    for r in active {
        let target = p.diagnostics.anchor_target[r];
        if target <= 0.0 || p.records[r].sublayers.is_empty() {
            continue;
        }
        let mut slope = 0.5;
        let mut last: Option<(f64, f64)> = None;
        for pass in 0..8 {
            (cancel)()?;
            let measured = sigma48(&light(&p.records[r], r, gross[r], cancel)?, cancel)?;
            if !measured.is_finite() || measured <= 0.0 {
                return Err(format!(
                    "Fotufilm sigma48 anchor record {r} has no positive measured variance"
                ));
            }
            let miss = (target / measured).ln();
            p.diagnostics.anchor_passes[r] = pass + 1;
            if miss.abs() < 0.02 {
                break;
            }
            if let Some((factor, old_sigma)) = last
                && (factors[r] / factor).ln().abs() > 1e-3
            {
                slope = ((measured / old_sigma).ln() / (factors[r] / factor).ln()).clamp(0.25, 1.5);
            }
            last = Some((factors[r], measured));
            let step = (miss / slope).exp();
            if !step.is_finite() || step <= 0.0 {
                return Err(format!(
                    "Fotufilm sigma48 anchor record {r} cannot converge"
                ));
            }
            for b in &mut p.records[r].sublayers {
                b.crystals_per_mm2 /= step;
                b.dye_per_cloud *= step;
                b.shape();
            }
            factors[r] *= step;
        }
        let final_sigma = sigma48(&light(&p.records[r], r, gross[r], cancel)?, cancel)?;
        let error = (target / final_sigma).ln();
        p.diagnostics.anchor_measured[r] = final_sigma;
        p.diagnostics.anchor_log_error[r] = error;
        if !error.is_finite() || error.abs() >= 0.02 {
            return Err(format!(
                "Fotufilm sigma48 anchor record {r} failed after eight passes: target {target:0.8}, measured {final_sigma:0.8}, log error {error:0.6}"
            ));
        }
    }
    if p.monochrome {
        for r in [0, 2] {
            for b in &mut p.records[r].sublayers {
                b.crystals_per_mm2 /= factors[1];
                b.dye_per_cloud *= factors[1];
                b.shape();
            }
            p.diagnostics.anchor_measured[r] = p.diagnostics.anchor_measured[1];
            p.diagnostics.anchor_log_error[r] = p.diagnostics.anchor_log_error[1];
            p.diagnostics.anchor_passes[r] = p.diagnostics.anchor_passes[1];
        }
    }
    for r in 0..3 {
        p.diagnostics.records[r].bins = p.records[r].sublayers.clone();
    }
    Ok(())
}
#[allow(clippy::needless_range_loop)] // three parallel record arrays
pub(super) fn build_bank(p: &Population, cancel: Cancellation<'_>) -> Result<Bank, String> {
    let mut levels: [Vec<TileLevel>; 3] = std::array::from_fn(|_| Vec::new());
    for r in 0..3 {
        if p.monochrome && r != 1 {
            continue;
        }
        if p.records[r].sublayers.is_empty() {
            continue;
        }
        for k in 0..17 {
            (cancel)()?;
            let record = &p.records[r];
            let gross = record.d_min + (record.d_max - record.d_min) * k as f64 / 16.0;
            levels[r].push(TileLevel::from_light(&light(record, r, gross, cancel)?)?);
        }
    }
    Ok(Bank {
        levels,
        d_min: std::array::from_fn(|r| p.records[r].d_min),
        d_max: std::array::from_fn(|r| p.records[r].d_max),
        monochrome: p.monochrome,
        diagnostics: p.diagnostics(),
        pitches: Mutex::new(Vec::new()),
    })
}
fn block_light(level: &TileLevel, rect: [f64; 4], bx: i64, by: i64, seed: u32, r: usize) -> f64 {
    let h = pcg(bx as u32 ^ pcg(by as u32 ^ pcg(seed ^ ((200 + r) as u32).wrapping_mul(GOLDEN))));
    let ox = (h % 256) as f64 + 64.0;
    let oy = ((h >> 8) % 256) as f64 + 64.0;
    let turn = (h >> 16) & 7;
    let (mut x0, mut x1, mut y0, mut y1) = (
        rect[0] - bx as f64 * 128.0,
        rect[1] - bx as f64 * 128.0,
        rect[2] - by as f64 * 128.0,
        rect[3] - by as f64 * 128.0,
    );
    if turn & 1 != 0 {
        (x0, x1) = (128.0 - x1, 128.0 - x0);
    }
    if turn & 2 != 0 {
        (y0, y1) = (128.0 - y1, 128.0 - y0);
    }
    if turn & 4 != 0 {
        (x0, x1, y0, y1) = (y0, y1, x0, x1);
    }
    level.rectangle_light(x0 + ox, x1 + ox, y0 + oy, y1 + oy) as f32 as f64
}
impl Bank {
    pub fn diagnostics(&self) -> Diagnostics {
        self.diagnostics.clone()
    }
    #[allow(clippy::needless_range_loop)] // level and field arrays are indexed together
    pub fn pitch_tables(
        &self,
        footprint_texels: [f64; 2],
        cancel: Cancellation<'_>,
    ) -> Result<Arc<PitchTables>, String> {
        if footprint_texels.iter().any(|x| !x.is_finite() || *x <= 0.0) {
            return Err("Fotufilm footprint must be positive finite on both axes".into());
        }
        let key = footprint_texels.map(f64::to_bits);
        if let Some((_, tables)) = self
            .pitches
            .lock()
            .map_err(|_| "Fotufilm pitch cache poisoned")?
            .iter()
            .find(|(k, _)| *k == key)
        {
            return Ok(tables.clone());
        }
        let mut tables = PitchTables {
            mean_density: std::array::from_fn(|_| Vec::new()),
            blend_correlation: std::array::from_fn(|_| Vec::new()),
        };
        for r in 0..3 {
            if self.levels[r].is_empty() {
                continue;
            }
            let mut fields = vec![vec![0.0_f64; 128 * 128]; 17];
            for k in 0..17 {
                (cancel)()?;
                for y in 0..128 {
                    (cancel)()?;
                    for x in 0..128 {
                        let px = (x * 7919 % 4096) as f64 * footprint_texels[0];
                        let py = (y * 104729 % 4096) as f64 * footprint_texels[1];
                        let rect = [px, px + footprint_texels[0], py, py + footprint_texels[1]];
                        let bx = ((px + footprint_texels[0] / 2.0) / 128.0).floor() as i64;
                        let by = ((py + footprint_texels[1] / 2.0) / 128.0).floor() as i64;
                        fields[k][y * 128 + x] =
                            -block_light(&self.levels[r][k], rect, bx, by, 0x5A3F1E1D, r)
                                .max(1e-9)
                                .log10();
                    }
                }
            }
            let means: Vec<f64> = fields
                .iter()
                .map(|f| f.iter().sum::<f64>() / f.len() as f64)
                .collect();
            tables.blend_correlation[r] = (0..16)
                .map(|k| {
                    let (mut aa, mut bb, mut ab) = (0.0, 0.0, 0.0);
                    for i in 0..128 * 128 {
                        let a = fields[k][i] - means[k];
                        let b = fields[k + 1][i] - means[k + 1];
                        aa += a * a;
                        bb += b * b;
                        ab += a * b;
                    }
                    if aa > 0.0 && bb > 0.0 {
                        (ab / (aa * bb).sqrt()).clamp(-1.0, 1.0)
                    } else {
                        1.0
                    }
                })
                .collect();
            tables.mean_density[r] = means;
        }
        let tables = Arc::new(tables);
        let mut cache = self
            .pitches
            .lock()
            .map_err(|_| "Fotufilm pitch cache poisoned")?;
        cache.retain(|(k, _)| *k != key);
        cache.push((key, tables.clone()));
        if cache.len() > 8 {
            cache.remove(0);
        }
        Ok(tables)
    }
    fn residual(
        &self,
        r: usize,
        gross: f64,
        center: [f64; 2],
        footprint: [f64; 2],
        seed: u32,
        tables: &PitchTables,
    ) -> f64 {
        if self.levels[r].is_empty() {
            return 0.0;
        }
        let t = ((gross - self.d_min[r]) / (self.d_max[r] - self.d_min[r]).max(1e-6))
            .clamp(0.0, 1.0)
            * 16.0;
        let k = (t as usize).min(15);
        let w = t - k as f64;
        let kept =
            ((1.0 - w).powi(2) + w * w + 2.0 * w * (1.0 - w) * tables.blend_correlation[r][k])
                .max(1e-4)
                .sqrt();
        let rect = [
            center[0] - footprint[0] / 2.0,
            center[0] + footprint[0] / 2.0,
            center[1] - footprint[1] / 2.0,
            center[1] + footprint[1] / 2.0,
        ];
        let (mut grain, mut norm) = (0.0, 0.0);
        for by in ((center[1] - 16.0) / 128.0).floor() as i64
            ..=((center[1] + 16.0) / 128.0).floor() as i64
        {
            for bx in ((center[0] - 16.0) / 128.0).floor() as i64
                ..=((center[0] + 16.0) / 128.0).floor() as i64
            {
                let weight = weight(center[0], bx) * weight(center[1], by);
                if weight <= 0.0 {
                    continue;
                }
                let a = -block_light(&self.levels[r][k], rect, bx, by, seed, r)
                    .max(1e-9)
                    .log10()
                    - tables.mean_density[r][k];
                let b = -block_light(&self.levels[r][k + 1], rect, bx, by, seed, r)
                    .max(1e-9)
                    .log10()
                    - tables.mean_density[r][k + 1];
                grain += weight * (a + (b - a) * w) / kept;
                norm += weight * weight;
            }
        }
        grain / norm.max(1e-12).sqrt()
    }
    #[allow(clippy::needless_range_loop)] // the per-record loop writes disjoint channels
    pub(super) fn apply_image(
        &self,
        input: &ImageBuf,
        g: Geometry,
        look: Look,
        seed: u64,
        cancel: Cancellation<'_>,
    ) -> Result<ImageBuf, String> {
        (cancel)()?;
        if input.data.len() != input.pixel_count() * 3
            || input.data.iter().any(|x| !to_f64(*x).is_finite())
        {
            return Err("Fotufilm requires finite developed CMY image".into());
        }
        if g.source_pitch_mm
            .iter()
            .chain(&g.output_pitch_mm)
            .chain(&g.footprint_mm)
            .any(|x| !x.is_finite() || *x <= 0.0)
            || g.origin_mm.iter().any(|x| !x.is_finite())
        {
            return Err("Fotufilm requires finite physical full-frame geometry and positive pitches/footprints".into());
        }
        if [look.amount, look.size, look.color, look.softness]
            .iter()
            .chain(&look.layers)
            .any(|x| !x.is_finite())
            || look.amount < 0.0
            || look.layers.iter().any(|x| *x < 0.0)
        {
            return Err("invalid Fotufilm look controls".into());
        }
        if look.amount == 0.0 {
            return Ok(input.clone());
        }
        let size = look.size.clamp(0.5, 4.0);
        let softness = look.softness.clamp(0.5, 4.0);
        let pitch = g.output_pitch_mm.map(|x| x / 0.001 / size);
        let origin = g.origin_mm.map(|x| x / 0.001 / size);
        let footprint = g.footprint_mm.map(|x| x / 0.001 / size * softness);
        let tables = self.pitch_tables(footprint, cancel)?;
        let own = look.color.clamp(0.0, 1.0);
        let shared = (3.0 - 2.0 * own * own).sqrt() - own;
        let mut out = input.clone();
        for y in 0..input.height {
            (cancel)()?;
            for x in 0..input.width {
                let i = (y as usize * input.width as usize + x as usize) * 3;
                let center = [
                    origin[0] + (x as f64 + 0.5) * pitch[0],
                    origin[1] + (y as f64 + 0.5) * pitch[1],
                ];
                let mut grain = [0.0; 3];
                for r in 0..3 {
                    if self.monochrome && r != 1 {
                        continue;
                    }
                    grain[r] = self.residual(
                        r,
                        to_f64(input.data[i + r]),
                        center,
                        footprint,
                        seed as u32,
                        &tables,
                    ) * look.amount
                        * look.layers[r]
                        / size;
                }
                let mean = grain.iter().sum::<f64>() / 3.0;
                for r in 0..3 {
                    let d = if self.monochrome {
                        grain[1]
                    } else {
                        own * grain[r] + shared * mean
                    };
                    out.data[i + r] = from_f64(to_f64(input.data[i + r]) + d);
                }
            }
        }
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn periodic_aperture_constant_and_whole_period() {
        let constant = TileLevel::from_light(&vec![0.37; N * N]).unwrap();
        for rect in [
            [-900.25, 1307.75, -0.4, 1024.6],
            [255.5, 257.25, 255.75, 259.1],
        ] {
            assert!(
                (constant.rectangle_light(rect[0], rect[1], rect[2], rect[3]) - 0.37).abs() < 1e-7
            );
        }
        let light: Vec<f32> = (0..N * N).map(|i| 0.1 + (i % 37) as f32 * 0.02).collect();
        let tile = TileLevel::from_light(&light).unwrap();
        let mean = light.iter().map(|x| *x as f64).sum::<f64>() / (N * N) as f64;
        assert!(
            (tile.rectangle_light(-17.25, -17.25 + 768.0, 43.5, 43.5 + 512.0) - mean).abs() < 1e-7
        );
    }
    #[test]
    fn periodic_aperture_matches_direct_area() {
        let light: Vec<f32> = (0..N * N).map(|i| 0.15 + (i % 19) as f32 * 0.025).collect();
        let tile = TileLevel::from_light(&light).unwrap();
        let (x0, x1, y0, y1) = (-2.3_f64, 261.7_f64, 254.4_f64, 258.2_f64);
        let mut sum = 0.0_f64;
        for y in y0.floor() as isize..y1.ceil() as isize {
            for x in x0.floor() as isize..x1.ceil() as isize {
                let area = (x1.min(x as f64 + 1.0) - x0.max(x as f64))
                    * (y1.min(y as f64 + 1.0) - y0.max(y as f64));
                sum += area * light[wrap(y, N) * N + wrap(x, N)] as f64;
            }
        }
        assert!(
            (tile.rectangle_light(x0, x1, y0, y1) - sum / ((x1 - x0) * (y1 - y0))).abs() < 1e-6
        );
    }
    fn constant_bank() -> Bank {
        Bank {
            levels: std::array::from_fn(|_| {
                (0..17)
                    .map(|_| TileLevel::from_light(&vec![0.5; N * N]).unwrap())
                    .collect()
            }),
            d_min: [0.; 3],
            d_max: [2.; 3],
            monochrome: false,
            diagnostics: Diagnostics {
                provenance: PROVENANCE,
                records: std::array::from_fn(|_| RecordDiagnostics::default()),
                anchor_target: [0.; 3],
                anchor_measured: [0.; 3],
                anchor_log_error: [0.; 3],
                anchor_passes: [0; 3],
            },
            pitches: Mutex::new(Vec::new()),
        }
    }
    #[test]
    fn zero_strength_preserves_outside_density() {
        let bank = constant_bank();
        let image = ImageBuf::from_data(1, 1, vec![from_f64(-0.2), from_f64(3.), from_f64(0.7)]);
        let g = Geometry {
            source_pitch_mm: [0.01; 2],
            origin_mm: [0.; 2],
            output_pitch_mm: [0.01; 2],
            footprint_mm: [0.01; 2],
        };
        let out = bank
            .apply(
                &image,
                g,
                Look {
                    amount: 0.,
                    ..Look::default()
                },
                7,
                &|| Ok(()),
            )
            .unwrap();
        assert_eq!(out.data, image.data);
    }
}
