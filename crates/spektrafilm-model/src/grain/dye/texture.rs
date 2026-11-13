//! Delivered film-coordinate Dye grain law.
//!
//! The law is a coordinate-addressed value-noise field whose relative
//! amplitude follows the developed record density, plus a conditional
//! Poisson event count. For record `c`:
//!
//! ```text
//! u_c      = sqrt(rho) * u_shared + sqrt(1 - rho) * u_own,c
//! e_c(x)   = 1 + white * strength[c] * taper(t_c(x)) * u_c(x)
//! lambda_c = (t_c + fog) * e_c * A / q
//! N_c      ~ Poisson(lambda_c)
//! D_c      = q * N_c / A - fog
//! ```
//!
//! `u` is zero-mean and unit-variance in the ensemble, so `E[D] = t`.
//! `expectation` replaces the material modulation and every count by its
//! conditional mean and returns the target. The field is realized on the
//! source film lattice and read through output footprints; a crop or a
//! rescale samples the same film. `specs/film-grain/dye/design.md` owns the
//! contract.

use rayon::prelude::*;
use spektrafilm_backend::dye_params::law;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{Scalar, ZERO, from_f64, to_f64};
use spektrafilm_math::stats::{normal01_coordinate, poisson_exact_coordinate};

/// Above this conditional count rate both executors draw the same normal
/// deviation instead of the exact Poisson count. The distributions agree to
/// the declared precision; the branch keeps a tiny mapped event area from
/// overflowing the rate or losing the mean.
pub const LARGE_RATE_THRESHOLD: f64 = 1e9;

/// Where the readout samples the film. A window is a set of film coordinates,
/// so a crop, a preview scale, or a full-frame read all address the same
/// lattice.
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

/// Resolved Dye look parameters.
#[derive(Clone, Debug)]
pub struct DyeLawParams {
    /// Relative per-record strength (`[1, 1, 1]` is neutral).
    pub strength: [f64; 3],
    /// Shared fraction across records, `[0, 1]`.
    pub rho: f64,
    /// Relative density modulation at strength 1.
    pub white: f64,
    /// Single-event mass in `D * um^2`.
    pub q: f64,
    /// Relative field correlation length. The delivered reference is `1.0`.
    pub length_scale: f64,
    pub seed: u64,
    /// Replace every draw by its conditional mean (diagnostic only).
    pub expectation: bool,
    /// One effective record carried in three equal lanes.
    pub monochrome: bool,
}

impl DyeLawParams {
    /// The delivered Dye grain law. Every parameter is part of the engine,
    /// not of the caller's environment: the model is the engine's grain, so
    /// there is nothing to switch on and nothing to fall back to.
    pub fn delivered() -> Self {
        Self {
            strength: law::STRENGTH,
            rho: law::RHO,
            white: law::WHITE,
            q: law::EVENT_AREA_UM2,
            length_scale: 1.0,
            seed: 7,
            expectation: false,
            monochrome: false,
        }
    }

    /// Domain checks; an invalid value fails instead of being repaired.
    pub fn validate(&self) -> Result<(), String> {
        if self
            .strength
            .iter()
            .any(|s| !s.is_finite() || *s < 0.0 || *s > 4.0)
        {
            return Err("film grain Dye: strength must be finite and in [0, 4]".into());
        }
        if !self.white.is_finite() || self.white < 0.0 || self.white > 1.5 {
            return Err("film grain Dye: white must be finite and in [0, 1.5]".into());
        }
        if !self.rho.is_finite() || !(0.0..=1.0).contains(&self.rho) {
            return Err("film grain Dye: rho must be finite and in [0, 1]".into());
        }
        if !self.q.is_finite() || self.q <= 0.0 {
            return Err("film grain Dye: q must be finite and positive".into());
        }
        if !self.length_scale.is_finite() || self.length_scale <= 0.0 {
            return Err("film grain Dye: length_scale must be finite and positive".into());
        }
        Ok(())
    }
}

/// One zero-mean, unit-variance value-noise field on the source lattice.
/// `stream` is the shared field (`0`) or one record's own field.
fn correlated_field(
    w: usize,
    h: usize,
    pitch_um: f64,
    seed: u64,
    stream: u32,
    length_scale: f64,
) -> Vec<Scalar> {
    super::value::field(w, h, pitch_um, length_scale, seed, stream)
}

/// Realize one channel of the delivered law on a 1:1 field.
#[allow(clippy::too_many_arguments)]
fn channel(
    target: &[Scalar],
    out: &mut [Scalar],
    w: usize,
    h: usize,
    pitch_um: f64,
    p: &DyeLawParams,
    c: usize,
    shared_white: &[Scalar],
) -> Result<(), String> {
    let record_strength = if p.monochrome { 1.0 } else { p.strength[c] };
    let sigma_white = p.white * record_strength;
    let own = correlated_field(
        w,
        h,
        pitch_um,
        p.seed,
        if p.monochrome { 1 } else { c as u32 + 1 },
        p.length_scale,
    );
    let a = p.rho.sqrt();
    let b = (1.0 - p.rho).sqrt();
    let area = pitch_um * pitch_um;
    let q = p.q;
    let taper_peak = law::taper_peak();
    out.par_iter_mut()
        .enumerate()
        .try_for_each(|(i, o)| -> Result<(), String> {
            let t = to_f64(target[i * 3 + c]);
            let p_dev = (t / law::DMAX).clamp(0.0, 1.0);
            let sat = 1.0 - p_dev * law::UNIFORMITY;
            let shape = (sat * p_dev * (1.0 - p_dev)).max(0.0).sqrt();
            let taper = (shape / taper_peak).max(law::TAPER_FLOOR);
            if p.expectation || q <= 0.0 {
                *o = from_f64(t);
                return Ok(());
            }
            let wi = a * to_f64(shared_white[i]) + b * to_f64(own[i]);
            let e = (1.0 + sigma_white * taper * wi).max(0.02);
            let expected = (t.max(0.0) + law::FOG) * e;
            let lambda = expected * area / q;
            let stream = if p.monochrome { 100 } else { 100 + c as u32 };
            let x = (i % w) as u32;
            let y = (i / w) as u32;
            if lambda > LARGE_RATE_THRESHOLD {
                // The exact sampler is indistinguishable from the normal law at this
                // rate. Evaluate the deviation directly so a small mapped event area
                // cannot overflow the rate and lose the mean.
                let variance = (q * expected / area).max(0.0);
                let z = normal01_coordinate(p.seed as u32, stream, x, y);
                *o = from_f64(expected - law::FOG + variance.sqrt() * z);
            } else {
                // One count from the draw the device kernel also uses: the
                // uniform is addressed by global cell coordinate and sample
                // ordinal, so one seed selects one realization on either
                // executor. A sequential stream cannot be replayed on a device
                // whose invocations run in an order it does not control.
                let count = poisson_exact_coordinate(lambda, p.seed as u32, stream, x, y) as f64;
                *o = from_f64(q * count / area - law::FOG);
            }
            Ok(())
        })
}

/// Bilinear read of the developed density at each readout cell centre. The law
/// is defined on the source raster; the readout grid only changes where it is
/// sampled. A finer grid interpolates that window's centre; a coarser grid
/// integrates the source cells the window covers, so full-frame, cropped, and
/// stage-split reads agree where they overlap.
fn resample_density(
    src: &[Scalar],
    sw: usize,
    sh: usize,
    source_pitch_um: f64,
    grid: &ReadoutGeometry,
) -> Vec<Scalar> {
    let dw = grid.width as usize;
    let dh = grid.height as usize;
    let mut out = vec![ZERO; dw * dh * 3];
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
                                acc[c] += w * to_f64(src[(syc * sw + sxc) * 3 + c]);
                            }
                            weight += w;
                        }
                        sx += 1;
                    }
                    sy += 1;
                }
                let norm = if weight > 0.0 { weight } else { 1.0 };
                for c in 0..3 {
                    out[(y * dw + x) * 3 + c] = from_f64(acc[c] / norm);
                }
            } else {
                let sx = ((fx0 + fx1) * 0.5 - 0.5).clamp(0.0, (sw - 1) as f64);
                let sy = ((fy0 + fy1) * 0.5 - 0.5).clamp(0.0, (sh - 1) as f64);
                let x0 = sx.floor() as usize;
                let y0 = sy.floor() as usize;
                let x1 = (x0 + 1).min(sw - 1);
                let y1 = (y0 + 1).min(sh - 1);
                let wx = from_f64(sx - x0 as f64);
                let wy = from_f64(sy - y0 as f64);
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
    params: &DyeLawParams,
) -> Result<ImageBuf, String> {
    params.validate()?;
    let (w, h) = (target.width as usize, target.height as usize);
    if !pitch_um.is_finite() || pitch_um <= 0.0 {
        return Err("film grain Dye: source pitch must be finite and positive".into());
    }
    // The law is evaluated on the film lattice, i.e. on the source raster, and the
    // readout resamples the developed density afterwards. Realizing the field on the
    // readout grid instead would make the draw depend on how the render was cropped,
    // scaled, or split into stages, which no film does.
    let shared_white = correlated_field(w, h, pitch_um, params.seed, 0, params.length_scale);
    let mut out = ImageBuf::new(w as u32, h as u32);
    for c in 0..3 {
        let mut plane = vec![ZERO; w * h];
        channel(
            &target.data,
            &mut plane,
            w,
            h,
            pitch_um,
            params,
            c,
            &shared_white,
        )?;
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

    fn flat(n: u32, d: Scalar) -> ImageBuf {
        let mut b = ImageBuf::new(n, n);
        b.data.iter_mut().for_each(|v| *v = d);
        b
    }

    fn params(strength: [f64; 3], expectation: bool) -> DyeLawParams {
        let mut p = DyeLawParams::delivered();
        p.strength = strength;
        p.expectation = expectation;
        p
    }

    #[test]
    fn expectation_mode_reproduces_the_target_exactly() {
        let src = flat(48, 0.7);
        let out = apply(
            &src,
            3.6676,
            &ReadoutGeometry::whole(48, 48, 3.6676),
            &params([1.0, 1.0, 1.0], true),
        )
        .unwrap();
        for (a, b) in src.data.iter().zip(out.data.iter()) {
            assert!((*a - *b).abs() < 1e-6, "{a} vs {b}");
        }
    }

    #[test]
    fn mean_is_preserved_for_any_strength() {
        let src = flat(96, 0.7);
        for strength in [[1.0, 1.0, 1.0], [0.67, 1.0, 1.71], [0.0, 2.0, 0.5]] {
            let out = apply(
                &src,
                3.6676,
                &ReadoutGeometry::whole(96, 96, 3.6676),
                &params(strength, false),
            )
            .unwrap();
            let mean = out.data.iter().map(|&v| to_f64(v)).sum::<f64>() / out.data.len() as f64;
            let rel = (mean - 0.7).abs() / 0.7;
            assert!(
                rel < 0.01,
                "strength {strength:?} relative mean error {rel}"
            );
        }
    }

    #[test]
    fn variance_grows_with_strength() {
        let src = flat(96, 0.7);
        let sd = |out: &ImageBuf| {
            let m = out.data.iter().map(|&v| to_f64(v)).sum::<f64>() / out.data.len() as f64;
            (out.data
                .iter()
                .map(|&v| (to_f64(v) - m).powi(2))
                .sum::<f64>()
                / out.data.len() as f64)
                .sqrt()
        };
        let one = sd(&apply(
            &src,
            3.6676,
            &ReadoutGeometry::whole(96, 96, 3.6676),
            &params([1.0, 1.0, 1.0], false),
        )
        .unwrap());
        let two = sd(&apply(
            &src,
            3.6676,
            &ReadoutGeometry::whole(96, 96, 3.6676),
            &params([2.0, 2.0, 2.0], false),
        )
        .unwrap());
        let ratio = two / one;
        assert!(
            ratio > 1.4 && ratio < 2.6,
            "variance ratio {ratio} not near 2"
        );
    }
}
