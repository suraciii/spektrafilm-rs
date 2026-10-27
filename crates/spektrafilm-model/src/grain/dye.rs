//! Clump-based record grain (research, v4 candidate).
//!
//! Law, per record `c`:
//!
//! ```text
//! delta_c(x) = sigma_c * [ sqrt(rho) u_shared(x) + sqrt(1-rho) u_c(x) ]
//! sigma_c    = base * strength[c]
//! e_c(x)     = exp(delta_c(x) - sigma_c^2 / 2)          E[e_c] = 1 pointwise
//! N_c(x)    ~ Poisson(t_c(x) e_c(x) A / q)
//! D_c(x)     = q N_c(x) / A                            E[D_c] = t_c
//! ```
//!
//! `u_shared` and `u_c` are zero-mean, unit-variance, multi-octave Gaussian fields
//! (separable box-filter approximation), addressed by global pixel coordinates so a
//! crop does not reseed them. The target density is never convolved, so the image
//! mean is preserved exactly and fine detail is not blurred.
//!
//! Numerical scope: full-field, source grid == readout grid (1:1). Banded/tiled
//! execution is not implemented here.

use rayon::prelude::*;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f64;
use spektrafilm_math::gaussian;
use spektrafilm_math::stats::{FastStatsRng, poisson_exact};

/// Octave centres (µm) and weights; weights are normalised by the caller.
const OCTAVES: [(f64, f64); 3] = [(4.0, 0.35), (2.0, 1.0), (1.2, 0.9)];

/// Octave list, overridable for scale studies: `len_um,weight;len_um,weight;...`
fn octaves() -> Vec<(f64, f64)> {
    // The envelope field belongs to the legacy clump branch; the delivered model runs
    // the texture path, so this mix only has to stay well-formed.
    OCTAVES.to_vec()
}

/// Texture-field octaves (µm, weight) used when the correlated field is enabled.
const TEXTURE_OCTAVES: [(f64, f64); 3] = [(4.5, 1.0), (9.0, 1.0), (18.0, 0.5)];
/// Band limit of the texture field, in micrometres.
const TEXTURE_HIGHPASS_UM: f64 = 12.0;

/// strength_r,strength_g,strength_b,rho,base,q,white — the delivered Dye defaults.
const DEFAULT_V4: &str = "1.0,1.0,1.18,0.92,0.073,0.0015,0.035";

fn texture_octaves() -> Vec<(f64, f64)> {
    TEXTURE_OCTAVES.to_vec()
}

/// Off by default: `SPEKTRAFILM_GRAIN_V4_TEXTURE=1` swaps the per-pixel field for a
/// correlated one, so the realised texture comes from aggregate scale rather than
/// from per-pixel draws.

/// Build a zero-mean, unit-variance multi-octave field. Octave length is the
/// Gaussian sigma in micrometres; weights are normalised here.
fn correlated_field(w: usize, h: usize, pitch_um: f64, seed: u64, tag: u64) -> Vec<f32> {
    let octs = texture_octaves();
    let norm = octs.iter().map(|(_, v)| v * v).sum::<f64>().sqrt();
    let mut field = vec![0.0f32; w * h];
    for (o, (length_um, weight)) in octs.iter().enumerate() {
        let v0 = noise_field(w, h, seed, tag + o as u64);
        // A real Gaussian, because the box approximation cannot resolve sigma < 1 px
        // and that is exactly the range the aggregate texture lives in.
        let mut v = gaussian::gaussian_blur_channel(&v0, w as u32, h as u32, (length_um / pitch_um / 2.0) as f32);
        standardize(&mut v);
        let k = (weight / norm) as f32;
        field.par_iter_mut().zip(v.par_iter()).for_each(|(a, b)| *a += k * b);
    }
    // Band-limit: subtract the field's own large-scale component. Aggregate grain
    // should not move the local level over tens of microns, because that reads as a
    // dark blotch rather than grain. `SPEKTRAFILM_GRAIN_V4_TEXTURE_HP` is that cut in
    // micrometres (0 disables it).
    let hp = texture_highpass_um();
    if hp > 0.0 {
        let big = gaussian::gaussian_blur_channel(&field, w as u32, h as u32, (hp / pitch_um) as f32);
        field.par_iter_mut().zip(big.par_iter()).for_each(|(a, b)| *a -= *b);
    }
    standardize(&mut field);
    field
}

/// High-pass cut for the texture field, in micrometres. Constant: this is a property
/// of the model, not a knob.
fn texture_highpass_um() -> f64 {
    TEXTURE_HIGHPASS_UM
}


/// Resolved clump parameters.
/// Where the readout samples the film. A window is a set of film coordinates, so a
/// crop, a preview scale, or a full-frame read all address the same lattice.
#[derive(Clone, Copy, Debug)]
pub struct ReadoutGeometry {
    /// Film coordinate of the first readout cell's corner, in micrometres.
    pub origin_um: [f64; 2],
    /// Readout pitch, in micrometres.
    pub pitch_um: [f64; 2],
    pub width: u32,
    pub height: u32,
}

impl ReadoutGeometry {
    /// The readout that covers the whole source raster at its own pitch.
    pub fn whole(width: u32, height: u32, pitch_um: f64) -> Self {
        Self {
            origin_um: [0.0, 0.0],
            pitch_um: [pitch_um, pitch_um],
            width,
            height,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClumpParams {
    /// Relative per-record strength (`[1, 1, 1]` is neutral).
    pub strength: [f64; 3],
    /// Shared fraction across records, `[0, 1]`.
    pub rho: f64,
    /// Per-record clump-scale multiplier.
    pub scales: [f64; 3],
    /// Relative density modulation at strength 1.
    pub base: f64,
    /// Per-pixel (unblurred) modulation amplitude. Keeps the texture sharp; the
    /// octave field supplies only the slowly varying envelope.
    pub white: f64,
    /// Single-event mass in `D * um^2`.
    pub q: f64,
    pub seed: u64,
    /// Replace every count by its conditional mean (diagnostic only).
    pub expectation: bool,
    /// Record density where the granularity taper peaks.
    pub taper_centre: f64,
    /// Record density at which the density scale saturates (profile Dmax).
    pub dmax: f64,
    /// Development-probability uniformity (Spektra `uniformity`).
    pub uniformity: f64,
    /// Peak of the derived shape, used to normalise the taper to 1.
    pub taper_peak: f64,
    /// Residual granularity fraction at full development.
    pub taper_floor: f64,
    /// Base (fog) density that keeps the event rate alive in the deep shadows.
    pub fog: f64,
    /// Exponent of the local-density amplitude law, sigma_D proportional to t^amp_exp.
    pub amp_exp: f64,
    /// Band limit derived from the SIZE_UM knob; 0 keeps the environment value.
    pub knob_highpass_um: f64,
    /// Density-noise floor, in density units: granularity that survives at Dmin
    /// (fog crystals plus the base's own irregularity). Added on the same field as
    /// the density-proportional term, so the mean stays exactly t.
    pub fog_grain: f64,
    /// Level (multiplicative) envelope instead of amplitude modulation. Level modulation is
    /// the only way to create a visible texture band, and it is also what can produce
    /// large-scale dark patches; the octave list decides which of the two dominates.
    pub level_mode: bool,
    /// Shared field, installed by [`apply`] before the records are realised.
    pub shared_white: std::sync::Arc<Vec<f32>>,
    /// Correlated field instead of per-pixel noise (texture from aggregate scale).
    pub texture: bool,
}

impl ClumpParams {
    /// Read the research parameters from the environment. Unset means "not active".
    /// The delivered Dye grain law. Every parameter is part of the engine, not of the
    /// caller's environment: the model is the engine's grain, so there is nothing to
    /// switch on and nothing to fall back to.
    pub fn delivered() -> Self {
        let parts: Vec<f64> = DEFAULT_V4.split(',').map(|v| v.trim().parse::<f64>().unwrap()).collect();
        let strength = [parts[0], parts[1], parts[2]];
        let mut params = Self {
            strength,
            rho: parts[3],
            scales: [1.0, 1.0, 1.4],
            base: parts[4],
            white: parts[6],
            q: parts[5],
            seed: 7,
            expectation: false,
            shared_white: std::sync::Arc::new(Vec::new()),
            texture: true,
            dmax: 1.2,
            uniformity: 0.97,
            taper_peak: 0.0,
            knob_highpass_um: 0.0,
            fog_grain: 0.0,
            amp_exp: 1.0,
            fog: 0.15,
            taper_floor: 0.18,
            taper_centre: 1.0,
            level_mode: false,
        };
        params.taper_peak = params.shape_peak();
        params
    }

    /// Peak of the derived granularity shape `sqrt(sat p (1-p))` over p in [0, 1].
    fn shape_peak(&self) -> f64 {
        let mut peak = 0.0_f64;
        for i in 1..512 {
            let p = i as f64 / 512.0;
            let sat = 1.0 - p * self.uniformity;
            let v = (sat * p * (1.0 - p)).max(0.0).sqrt();
            if v > peak {
                peak = v;
            }
        }
        peak.max(1e-6)
    }

    /// Domain checks; an invalid value fails instead of being repaired.
    pub fn validate(&self) -> Result<(), String> {
        if self.strength.iter().any(|s| !s.is_finite() || *s < 0.0 || *s > 4.0) {
            return Err("film grain V4: strength must be finite and in [0, 4]".into());
        }
        if !self.white.is_finite() || self.white < 0.0 || self.white > 1.5 {
            return Err("film grain V4: white must be finite and in [0, 1.5]".into());
        }
        if !self.rho.is_finite() || !(0.0..=1.0).contains(&self.rho) {
            return Err("film grain V4: rho must be finite and in [0, 1]".into());
        }
        if !self.base.is_finite() || self.base < 0.0 || self.base > 0.6 {
            return Err("film grain V4: base must be finite and in [0, 0.6]".into());
        }
        if !self.q.is_finite() || self.q <= 0.0 {
            return Err("film grain V4: q must be finite and positive".into());
        }
        if self.scales.iter().any(|s| !s.is_finite() || *s <= 0.0) {
            return Err("film grain V4: scales must be finite and positive".into());
        }
        Ok(())
    }
}


/// Box blur: horizontal `src -> tmp`, then vertical `tmp -> src` (edge replicated).
fn box_blur(src: &mut [f32], tmp: &mut [f32], w: usize, h: usize, half: usize) {
    if half == 0 {
        return;
    }
    let n = (2 * half + 1) as f32;
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let base = y * w;
        for x in 0..w {
            let mut acc = 0.0f32;
            for k in 0..=2 * half {
                let xx = (x + k).saturating_sub(half).min(w - 1);
                acc += src[base + xx];
            }
            row[x] = acc / n;
        }
    });
    src.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let mut acc = 0.0f32;
            for k in 0..=2 * half {
                let yy = (y + k).saturating_sub(half).min(h - 1);
                acc += tmp[yy * w + x];
            }
            row[x] = acc / n;
        }
    });
}

fn box_blur_approx(field: &mut [f32], tmp: &mut [f32], w: usize, h: usize, sigma_px: f64) {
    if sigma_px <= 0.35 {
        return;
    }
    let half = (((12.0 * sigma_px * sigma_px / 3.0 + 1.0).sqrt() - 1.0) / 2.0)
        .round()
        .max(1.0) as usize;
    for _ in 0..3 {
        box_blur(field, tmp, w, h, half);
    }
}

/// Zero-mean unit-variance field addressed by global pixel coordinates.
fn noise_field(w: usize, h: usize, seed: u64, tag: u64) -> Vec<f32> {
    let mut v = vec![0.0f32; w * h];
    v.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, cell) in row.iter_mut().enumerate() {
            let index = ((y as u64) << 32) | x as u64;
            let mut rng = FastStatsRng::stream(seed, tag, index);
            let z = rng.randn();
            *cell = z as f32;
        }
    });
    v
}

fn standardize(v: &mut [f32]) {
    let mean = v.par_iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
    let var = v
        .par_iter()
        .map(|&x| {
            let d = x as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / v.len() as f64;
    let sd = var.sqrt().max(1e-12);
    let (scale, shift) = ((1.0 / sd) as f32, mean as f32);
    v.par_iter_mut().for_each(|x| *x = (*x - shift) * scale);
}

/// Realize one channel of the clump law on a 1:1 field.
fn channel(
    target: &[f32],
    out: &mut [f32],
    w: usize,
    h: usize,
    pitch_um: f64,
    p: &ClumpParams,
    c: usize,
    shared: &[Vec<f32>],
) -> Result<(), String> {
    let mut tmp = vec![0.0f32; w * h];
    let mut delta = vec![0.0f32; w * h];
    let octs = octaves();
    let norm = octs.iter().map(|(_, v)| v * v).sum::<f64>().sqrt();
    // Relative modulation is tapered where the record density is already high: the
    // emulsion cannot keep the same relative fluctuation once the record saturates,
    // and an untapered multiplicative field reads as large light/dark clouds there.
    let sigma_base = p.base * p.strength[c];
    // Per-pixel component: unblurred, so the realised texture stays as sharp as the
    // readout grid allows. The octave field above only shapes the envelope.
    let sigma_white = p.white * p.strength[c];
    // The per-pixel component must be shared across records like the envelope: one
    // grain event affects the three layers together. Independent per-record white noise
    // would destroy the measured inter-record correlation (~0.9 in real scans).
    let white_independent = if p.texture {
        correlated_field(w, h, pitch_um, p.seed, 600 + (c as u64) * 8)
    } else {
        let mut v = noise_field(w, h, p.seed, 600 + c as u64);
        standardize(&mut v);
        v
    };
    for (o, (length_um, weight)) in octs.iter().enumerate() {
        let mut independent = noise_field(w, h, p.seed, 200 + (c as u64) * 8 + o as u64);
        box_blur_approx(&mut independent, &mut tmp, w, h, length_um * p.scales[c] / pitch_um);
        standardize(&mut independent);
        let us = &shared[o];
        let a = p.rho.sqrt() as f32;
        let b = (1.0 - p.rho).sqrt() as f32;
        let k = (weight / norm) as f32;
        delta.par_iter_mut().enumerate().for_each(|(i, d)| {
            *d += k * (a * us[i] + b * independent[i]);
        });
    }
    // Diagnostic expectation mode is the *unconditional* mean of the law: the material
    // modulation is replaced by its mean (e == 1) as well as each count by its mean.
    // The conditional mean of one material realization would be t * e, which is not the
    // grain-off image and is therefore not the invariant this mode exists to check.
    if p.expectation {
        delta.iter_mut().for_each(|d| *d = 0.0);
    }
    let area = pitch_um * pitch_um;
    let q = p.q;
    // Clumped counting ("grain reconstructs the image"): the density is built from a
    // Poisson process of clumps, each carrying `1 + Poisson(kbar-1)` grains, and each
    // clump is spread over `clump_um`. The mean is exactly t (smoothing preserves it),
    // so no multiplicative envelope is needed and no neighbourhood is darkened; the
    // texture comes from the clump *extent*, not from a level modulation.
    if let Some((kbar, clump_um)) = clump_params() {
        let qc = q * kbar;
        let mut mass = vec![0.0f32; w * h];
        mass.par_iter_mut().enumerate().for_each(|(i, m)| {
            let tv = target[i * 3 + c] as f64;
            let index = (((i / w) as u64) << 32) | (i % w) as u64;
            let mut rng = FastStatsRng::stream(p.seed, 800 + c as u64, index);
            let lambda = (tv.max(0.0) * area / qc).min(1e12);
            let count = if p.expectation { lambda } else { poisson_exact(&mut rng, lambda).map(|v| v as f64).unwrap_or(0.0) };
            let k = if p.expectation { kbar } else { 1.0 + poisson_exact(&mut rng, (kbar - 1.0).max(0.0)).map(|v| v as f64).unwrap_or(0.0) };
            *m = (count * k) as f32;
        });
        box_blur_approx(&mut mass, &mut tmp, w, h, clump_um / pitch_um);
        out.par_iter_mut().enumerate().for_each(|(i, o)| {
            *o = from_f64(q * mass[i] as f64 / area);
        });
        return Ok(());
    }
    out.par_iter_mut().enumerate().try_for_each(|(i, o)| -> Result<(), String> {
        let t = target[i * 3 + c] as f64;
        // Grain-vs-density response is *derived*, not tuned: for n particles with
        // development probability p, Var = n p (1-p); with the particle budget
        // reduced by `saturation` the standard deviation follows
        // sqrt(saturation * p * (1 - p)). It vanishes at clear base and at Dmax and
        // peaks in the mid densities, which is what the measured curves do.
        let p_dev = (t / p.dmax).clamp(0.0, 1.0);
        let sat = 1.0 - p_dev * p.uniformity;
        let shape = (sat * p_dev * (1.0 - p_dev)).max(0.0).sqrt();
        // A fully developed record still has granularity: the binomial shape goes to
        // zero at p -> 1, which makes deep shadows perfectly clean. A floor keeps the
        // measured residual instead of inventing a grain-free black.
        let taper = (shape / p.taper_peak).max(p.taper_floor);
        let sigma = sigma_base * taper;
        // Amplitude modulation instead of level modulation: the envelope scales the
        // local grain *contrast*, the mean density stays exactly t. A multiplicative
        // level term (what exp() used to do) darkens whole neighbourhoods and reads as
        // large dark patches.
        let e = if p.expectation {
            1.0
        } else {
            let wi = {
                let a = p.rho.sqrt() as f32;
                let b = (1.0 - p.rho).sqrt() as f32;
                a * p.shared_white[i] + b * white_independent[i]
            };
            if p.level_mode {
                let dm = sigma * delta[i] as f64 + sigma_white * wi as f64
                    - 0.5 * (sigma * sigma + sigma_white * sigma_white);
                if dm > 40.0 { 40.0_f64.exp() } else { dm.exp() }
            } else {
                let m = if p.texture {
                    1.0
                } else {
                    (1.0 + sigma * delta[i] as f64).clamp(0.05, 3.0)
                };
                // Absolute density granularity saturates with density (Selwyn): a purely
                // relative amplitude makes the densest (brightest) areas the grainiest,
                // which reads as dirty highlights. Cap the absolute sigma instead.
                // Amplitude exponent: sigma_D = t^amp_exp * taper. alpha = 1 keeps the
                // purely relative form (sigma proportional to local density); alpha = 0
                // makes the density noise absolute, so bright, thin areas keep granularity
                // instead of fading to a clean white.
                let amp = if p.amp_exp == 1.0 {
                    1.0
                } else {
                    let tref = (p.dmax * 0.5).max(1e-6);
                    (t / tref).clamp(1e-4, 1e4).powf(p.amp_exp - 1.0)
                };
                // Fog granularity is bounded by the base density it lives in: the
                // effective floor is sigma_fog * t/(t + 0.05), so it saturates at
                // Dmin instead of blowing up where the record is nearly clear.
                // Fog granularity lives at the base, so it decays over roughly one
                // base density: w = 1 / (1 + (t / 0.1)^2).
                let w = 1.0 / (1.0 + (t / 0.1) * (t / 0.1));
                let rel = sigma_white * taper * amp + p.fog_grain * w / (t.max(0.0) + 0.02);
                (1.0 + rel * wi as f64 * m).max(0.02)
            }
        };
        // Fog baseline: a developed record still carries base density, so the event
        // rate does not vanish at t -> 0. The baseline is removed again after the
        // count, so E[D] = t is unchanged.
        let lambda = ((t.max(0.0) + p.fog) * e * area / q).min(1e12);
        let count = if p.expectation {
            lambda
        } else {
            let index = (((i / w) as u64) << 32) | (i % w) as u64;
            let mut rng = FastStatsRng::stream(p.seed, 400 + c as u64, index);
            poisson_exact(&mut rng, lambda)? as f64
        };
        *o = from_f64(q * count / area - p.fog);
        Ok(())
    })?;
    Ok(())
}

/// Optional clumped-counting parameters: `SPEKTRAFILM_GRAIN_V4_CLUMP=kbar,clump_um`.
fn clump_params() -> Option<(f64, f64)> {
    None
}

/// Apply the clump law to a 1:1 source target.
/// Bilinear read of the developed density at each readout cell centre. The law is
/// defined on the source raster; the readout grid only changes where it is sampled.
fn resample_density(
    src: &[f32],
    sw: usize,
    sh: usize,
    source_pitch_um: f64,
    grid: &ReadoutGeometry,
) -> Vec<f32> {
    // Each readout cell names a film-coordinate window. Finer than the lattice, it
    // interpolates that window's centre; coarser, it integrates the lattice cells the
    // window covers. Integrating rather than point sampling is what makes a cropped
    // readout and a full-frame readout agree where they overlap, and what keeps a
    // render split into stages identical to the same render done in one pass.
    let dw = grid.width as usize;
    let dh = grid.height as usize;
    let mut out = vec![0.0f32; dw * dh * 3];
    for y in 0..dh {
        let fy0 = (grid.origin_um[1] + y as f64 * grid.pitch_um[1]) / source_pitch_um;
        let fy1 = fy0 + grid.pitch_um[1] / source_pitch_um;
        for x in 0..dw {
            let fx0 = (grid.origin_um[0] + x as f64 * grid.pitch_um[0]) / source_pitch_um;
            let fx1 = fx0 + grid.pitch_um[0] / source_pitch_um;
            if fx1 - fx0 >= 1.0 || fy1 - fy0 >= 1.0 {
                let mut acc = [0.0f64; 3];
                let mut weight = 0.0f64;
                let mut sy = fy0.floor() as i64;
                while (sy as f64) < fy1 {
                    let wy = (fy1.min(sy as f64 + 1.0) - fy0.max(sy as f64)).max(0.0);
                    let mut sx = fx0.floor() as i64;
                    while (sx as f64) < fx1 {
                        let wx = (fx1.min(sx as f64 + 1.0) - fx0.max(sx as f64)).max(0.0);
                        let w = wx * wy;
                        if w > 0.0 {
                            let syc = sy.clamp(0, sh as i64 - 1) as usize;
                            let sxc = sx.clamp(0, sw as i64 - 1) as usize;
                            for c in 0..3 {
                                acc[c] += w * src[(syc * sw + sxc) * 3 + c] as f64;
                            }
                            weight += w;
                        }
                        sx += 1;
                    }
                    sy += 1;
                }
                let norm = if weight > 0.0 { weight } else { 1.0 };
                for c in 0..3 {
                    out[(y * dw + x) * 3 + c] = (acc[c] / norm) as f32;
                }
            } else {
                let sx = ((fx0 + fx1) * 0.5 - 0.5).clamp(0.0, (sw - 1) as f64);
                let sy = ((fy0 + fy1) * 0.5 - 0.5).clamp(0.0, (sh - 1) as f64);
                let x0 = sx.floor() as usize;
                let y0 = sy.floor() as usize;
                let x1 = (x0 + 1).min(sw - 1);
                let y1 = (y0 + 1).min(sh - 1);
                let wx = (sx - x0 as f64) as f32;
                let wy = (sy - y0 as f64) as f32;
                for c in 0..3 {
                    let p00 = src[(y0 * sw + x0) * 3 + c];
                    let p10 = src[(y0 * sw + x1) * 3 + c];
                    let p01 = src[(y1 * sw + x0) * 3 + c];
                    let p11 = src[(y1 * sw + x1) * 3 + c];
                    let top = p00 + (p10 - p00) * wx;
                    let bottom = p01 + (p11 - p01) * wx;
                    out[(y * dw + x) * 3 + c] = top + (bottom - top) * wy;
                }
            }
        }
    }
    out
}

pub fn apply(
    target: &ImageBuf,
    pitch_um: f64,
    grid: &ReadoutGeometry,
    params: &ClumpParams,
) -> Result<ImageBuf, String> {
    params.validate()?;
    let (w, h) = (target.width as usize, target.height as usize);
    if !pitch_um.is_finite() || pitch_um <= 0.0 {
        return Err("film grain V4: source pitch must be finite and positive".into());
    }
    // The law is evaluated on the film lattice, i.e. on the source raster, and the
    // readout resamples the developed density afterwards. Realizing the field on the
    // readout grid instead would make the draw depend on how the render was cropped,
    // scaled, or split into stages, which no film does.
    let target_data = &target.data;
    let _m_oct = octaves();
    let octs = octaves();
    let mut shared = Vec::with_capacity(octs.len());
    let mut tmp = vec![0.0f32; w * h];
    for (o, (length_um, _)) in octs.iter().enumerate() {
        let mut v = noise_field(w, h, params.seed, 100 + o as u64);
        box_blur_approx(&mut v, &mut tmp, w, h, length_um / pitch_um);
        standardize(&mut v);
        shared.push(v);
    }
    let shared_white = if params.texture {
        correlated_field(w, h, pitch_um, params.seed, 900)
    } else {
        let mut v = noise_field(w, h, params.seed, 600);
        standardize(&mut v);
        v
    };
    let mut params = params.clone();
    params.shared_white = std::sync::Arc::new(shared_white);
    let params = &params;
    let mut out = ImageBuf::new(w as u32, h as u32);
    for c in 0..3 {
        let mut plane = vec![0.0f32; w * h];
        channel(target_data, &mut plane, w, h, pitch_um, params, c, &shared)?;
        out.data
            .par_chunks_mut(3)
            .zip(plane.par_iter())
            .for_each(|(px, v)| px[c] = *v);
    }
    let whole = (grid.width as usize, grid.height as usize) == (w, h)
        && grid.origin_um == [0.0, 0.0]
        && grid.pitch_um == [pitch_um, pitch_um];
    if whole {
        return Ok(out);
    }
    let mut readout = ImageBuf::new(grid.width, grid.height);
    readout.data = resample_density(&out.data, w, h, pitch_um, grid);
    Ok(readout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(n: u32, d: f32) -> ImageBuf {
        let mut b = ImageBuf::new(n, n);
        b.data.iter_mut().for_each(|v| *v = d);
        b
    }

    fn params(strength: [f64; 3], expectation: bool) -> ClumpParams {
        let mut p = ClumpParams::delivered();
        p.strength = strength;
        p.expectation = expectation;
        p
    }

    #[test]
    fn expectation_mode_reproduces_the_target_exactly() {
        let src = flat(48, 0.7);
        let out = apply(&src, 3.6676, &ReadoutGeometry::whole(48, 48, 3.6676), &params([1.0, 1.0, 1.0], true)).unwrap();
        for (a, b) in src.data.iter().zip(out.data.iter()) {
            assert!((*a - *b).abs() < 1e-6, "{a} vs {b}");
        }
    }

    #[test]
    fn mean_is_preserved_for_any_strength() {
        let src = flat(96, 0.7);
        for strength in [[1.0, 1.0, 1.0], [0.67, 1.0, 1.71], [0.0, 2.0, 0.5]] {
            let out = apply(&src, 3.6676, &ReadoutGeometry::whole(96, 96, 3.6676), &params(strength, false)).unwrap();
            let mean = out.data.iter().map(|&v| v as f64).sum::<f64>() / out.data.len() as f64;
            let rel = (mean - 0.7).abs() / 0.7;
            assert!(rel < 0.01, "strength {strength:?} relative mean error {rel}");
        }
    }

    #[test]
    fn variance_grows_with_strength() {
        let src = flat(96, 0.7);
        let sd = |out: &ImageBuf| {
            let m = out.data.iter().map(|&v| v as f64).sum::<f64>() / out.data.len() as f64;
            (out.data.iter().map(|&v| (v as f64 - m).powi(2)).sum::<f64>() / out.data.len() as f64)
                .sqrt()
        };
        let one = sd(&apply(&src, 3.6676, &ReadoutGeometry::whole(96, 96, 3.6676), &params([1.0, 1.0, 1.0], false)).unwrap());
        let two = sd(&apply(&src, 3.6676, &ReadoutGeometry::whole(96, 96, 3.6676), &params([2.0, 2.0, 2.0], false)).unwrap());
        let ratio = two / one;
        assert!(ratio > 1.4 && ratio < 2.6, "variance ratio {ratio} not near 2");
    }
}
