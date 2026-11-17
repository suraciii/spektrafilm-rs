//! Delivered film-coordinate Dye grain law.
//!
//! The law is a coordinate-addressed value-noise field whose relative
//! amplitude follows one density layer's developed record density, plus a
//! conditional Poisson event count. For record `c` and density layer `l`:
//!
//! ```text
//! u_c,l      = sqrt(rho) * u_shared + sqrt(1 - rho) * u_own,c,l
//! e_c,l(x)   = 1 + white * strength[c] * taper(t_c,l(x)) * u_c,l(x)
//! lambda_c,l = (t_c,l + fog/layers) * e_c,l * A / q
//! N_c,l      ~ Poisson(lambda_c,l)
//! D_c        = sum_l (q * N_c,l / A - fog/layers)
//! ```
//!
//! `u` is zero-mean and unit-variance in the ensemble, and the layer targets
//! sum to the developed target, so `E[D] = t`.
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
    /// Shared fraction across records, `[0, 1]`. The public color grain
    /// maps to `rho = 1 - color_grain`; monochrome callers hold it at `0.8`.
    pub rho: f64,
    /// Relative density modulation at strength 1.
    pub white: f64,
    /// Single-event mass in `D * um^2`.
    pub q: f64,
    /// Base (fog) density for this record.
    pub fog: f64,
    /// Relative field correlation length. The delivered reference is `1.0`.
    pub length_scale: f64,
    /// Relative cell of the microcontrast component, or zero to disable it.
    pub fine_scale: f64,
    /// Variance fraction carried by the microcontrast component.
    pub fine_weight: f64,
    /// Zero-based density-layer index. It only selects the independent
    /// record stream; the density target is already that layer's share.
    pub layer: u32,
    /// Per-channel density peak of the active profile layer. The taper is
    /// evaluated against this sampled peak and reaches its asymptotic zero at
    /// `dmax / uniformity`.
    pub dmax: [f64; 3],
    pub seed: u64,
    /// Replace every draw by its conditional mean (diagnostic only).
    pub expectation: bool,
    /// One effective record carried in three equal lanes.
    pub monochrome: bool,
}

impl DyeLawParams {
    /// The engine's reference Dye grain law with the default color coupling.
    /// Every parameter is part of the engine, not of the caller's environment:
    /// the model is the engine's grain, so there is nothing to switch on and
    /// nothing to fall back to.
    pub fn delivered() -> Self {
        Self {
            strength: law::STRENGTH,
            // Public color grain defaults to 0.75; `rho = 1 - color_grain`.
            rho: 0.25,
            white: law::WHITE,
            q: law::EVENT_AREA_UM2,
            fog: law::FOG,
            length_scale: 1.0,
            fine_scale: 0.0,
            fine_weight: 0.0,
            layer: 0,
            dmax: [law::DMAX; 3],
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
        if !self.fog.is_finite() || self.fog < 0.0 {
            return Err("film grain Dye: fog must be finite and non-negative".into());
        }
        if !self.length_scale.is_finite() || self.length_scale <= 0.0 {
            return Err("film grain Dye: length_scale must be finite and positive".into());
        }
        if !self.fine_weight.is_finite() || !(0.0..1.0).contains(&self.fine_weight) {
            return Err("film grain Dye: fine_weight must be finite and in [0, 1)".into());
        }
        if self.fine_weight > 0.0 && (!self.fine_scale.is_finite() || self.fine_scale <= 0.0) {
            return Err("film grain Dye: fine_scale must be finite and positive".into());
        }
        if self
            .dmax
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err("film grain Dye: dmax must be finite and positive".into());
        }
        Ok(())
    }
}

/// One zero-mean, unit-variance value-noise field on the source lattice.
/// `stream` is the shared field (`0`) or one record's own field.
#[allow(clippy::too_many_arguments)]
fn correlated_field(
    w: usize,
    h: usize,
    pitch_um: f64,
    seed: u64,
    stream: u32,
    length_scale: f64,
    fine_scale: f64,
    fine_weight: f64,
) -> Vec<Scalar> {
    super::value::field_with_fine(
        w,
        h,
        pitch_um,
        length_scale,
        seed,
        stream,
        fine_scale,
        fine_weight,
    )
}

/// Normalized layer variance envelope used by the material grain model.
///
/// `p = t / peak` is the developed fraction of the layer's sampled density
/// peak. The variance shape is `p * (1 - uniformity * p)`, so it is zero at
/// clear base and zero only at the asymptotic saturation
/// `peak / uniformity`, above the sampled profile peak. The correlated field
/// therefore keeps a positive residual at the sampled peak instead of
/// removing the material texture there.
fn density_taper(t: f64, dmax: f64, taper_peak: f64) -> f64 {
    let p_dev = (t / dmax).clamp(0.0, 1.0 / law::UNIFORMITY);
    let shape = (p_dev * (1.0 - law::UNIFORMITY * p_dev)).max(0.0).sqrt();
    shape / taper_peak
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
    let layer_stream = p.layer * 4;
    let own = correlated_field(
        w,
        h,
        pitch_um,
        p.seed,
        if p.monochrome {
            1 + layer_stream
        } else {
            c as u32 + 1 + layer_stream
        },
        p.length_scale,
        p.fine_scale,
        p.fine_weight,
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
            let taper = density_taper(t, p.dmax[c], taper_peak);
            if p.expectation || q <= 0.0 {
                *o = from_f64(t);
                return Ok(());
            }
            let wi = a * to_f64(shared_white[i]) + b * to_f64(own[i]);
            let e = (1.0 + sigma_white * taper * wi).max(0.02);
            let expected = (t.max(0.0) + p.fog) * e;
            let lambda = expected * area / q;
            let stream = if p.monochrome {
                100 + layer_stream
            } else {
                100 + c as u32 + layer_stream
            };
            let x = (i % w) as u32;
            let y = (i / w) as u32;
            if lambda > LARGE_RATE_THRESHOLD {
                // The exact sampler is indistinguishable from the normal law at this
                // rate. Evaluate the deviation directly so a small mapped event area
                // cannot overflow the rate and lose the mean.
                let variance = (q * expected / area).max(0.0);
                let z = normal01_coordinate(p.seed as u32, stream, x, y);
                *o = from_f64(expected - p.fog + variance.sqrt() * z);
            } else {
                // One count from the draw the device kernel also uses: the
                // uniform is addressed by global cell coordinate and sample
                // ordinal, so one seed selects one realization on either
                // executor. A sequential stream cannot be replayed on a device
                // whose invocations run in an order it does not control.
                let count = poisson_exact_coordinate(lambda, p.seed as u32, stream, x, y) as f64;
                *o = from_f64(q * count / area - p.fog);
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
    let shared_white = correlated_field(
        w,
        h,
        pitch_um,
        params.seed,
        0,
        params.length_scale,
        params.fine_scale,
        params.fine_weight,
    );
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
    fn density_taper_vanishes_at_clear_base_and_asymptotic_saturation() {
        let peak = law::taper_peak();
        assert_eq!(density_taper(0.0, law::DMAX, peak), 0.0);
        assert!(
            density_taper(law::DMAX / law::UNIFORMITY, law::DMAX, peak) < 1e-6,
            "asymptotic saturation should vanish"
        );
        assert!(density_taper(law::DMAX * 0.5, law::DMAX, peak) > 0.0);
    }

    #[test]
    fn density_taper_keeps_a_positive_residual_at_the_sampled_peak() {
        let peak = law::taper_peak();
        let dmax = 2.25;
        let at_peak = density_taper(dmax, dmax, peak);
        assert!(at_peak > 0.0, "sampled peak amplitude {at_peak}");
        assert!(
            (at_peak - 2.0 * (law::UNIFORMITY * (1.0 - law::UNIFORMITY)).sqrt()).abs() < 1e-12,
            "sampled peak should retain the material variance residual"
        );
        assert!(
            density_taper(dmax / law::UNIFORMITY, dmax, peak) < 1e-6,
            "asymptotic saturation should vanish"
        );
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
    fn color_grain_reduces_cross_record_correlation_without_moving_the_mean() {
        let src = flat(96, 0.7);
        let geom = ReadoutGeometry::whole(96, 96, 3.6676);
        let corr = |out: &ImageBuf| {
            let n = out.width as usize * out.height as usize;
            let (mut sa, mut sb, mut sab) = (0.0, 0.0, 0.0);
            let (mut ma, mut mb) = (0.0, 0.0);
            for i in 0..n {
                ma += to_f64(out.data[i * 3]);
                mb += to_f64(out.data[i * 3 + 1]);
            }
            ma /= n as f64;
            mb /= n as f64;
            for i in 0..n {
                let a = to_f64(out.data[i * 3]) - ma;
                let b = to_f64(out.data[i * 3 + 1]) - mb;
                sa += a * a;
                sb += b * b;
                sab += a * b;
            }
            (sab / (sa * sb).sqrt()).abs()
        };
        let mut correlations = Vec::new();
        for color_grain in [0.0, 0.5, 1.0] {
            let mut p = params([1.0, 1.0, 1.0], false);
            p.rho = 1.0 - color_grain;
            let out = apply(&src, 3.6676, &geom, &p).unwrap();
            let mean = out.data.iter().map(|&v| to_f64(v)).sum::<f64>() / out.data.len() as f64;
            assert!(
                (mean - 0.7).abs() / 0.7 < 0.01,
                "mean drifted at {color_grain}"
            );
            correlations.push(corr(&out));
        }
        assert!(
            correlations[0] > correlations[1] && correlations[1] > correlations[2],
            "cross-record correlation must fall with color grain: {correlations:?}"
        );
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
