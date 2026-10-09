//! Optical diffusion filters: discrete PSFs and reflected FFT convolution.

use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{Scalar, from_f64};

// ===========================================================================
// Lens diffusion filter (Black Pro Mist family).
//
// Faithful port of `spektrafilm/model/diffusion.py::apply_diffusion_filter_um`
// (v0.3.2). The PSF is a per-channel sum of 2D isotropic exponentials in
// three groups {core, halo, bloom}; the halo is colour-tinted by an
// energy-conserving "warmth" redistribution across its sub-components. The
// effect is the energy-conserving convex combination
//     E_out = (1 - p_s) * E_in + p_s * (K_s * E_in)
// with p_s the deflected-photon fraction from strength+family. Applied via
// FFT convolution (`fft_conv::convolve2d_reflect`) to reproduce Python's
// reflect-pad + fftconvolve('same') exactly.
// ===========================================================================

use spektrafilm_math::fft_conv::convolve2d_reflect;

#[derive(Clone, Copy)]
struct GroupCfg {
    lambda_um: f64,
    spread: f64,
    n_components: usize,
    /// Power-law tail exponent; only meaningful for the bloom group.
    alpha: f64,
}

#[derive(Clone, Copy)]
struct FamilyShape {
    core: GroupCfg,
    halo: GroupCfg,
    bloom: GroupCfg,
    w_c: f64,
    w_h: f64,
    w_b: f64,
    halo_warmth_base: f64,
    /// Per-family scaling on the shared strength→scatter saturation table.
    total_gain: f64,
}

/// Borrowed diffusion-filter parameters (mirrors core's `DiffusionFilterParams`
/// without the dependency-inverting type coupling). All values f64.
pub struct DiffusionFilter<'a> {
    pub family: &'a str,
    pub strength: f64,
    pub spatial_scale: f64,
    pub halo_warmth: f64,
    pub core_intensity: f64,
    pub core_size: f64,
    pub halo_intensity: f64,
    pub halo_size: f64,
    pub bloom_intensity: f64,
    pub bloom_size: f64,
}

/// Per-channel warmth axis: warmth>0 pushes warm light (R, slight G) to the
/// outer halo and cool (B) to the inner. Matches `_HALO_CHANNEL_WARMTH_AXIS`.
const HALO_CHANNEL_WARMTH_AXIS: [f64; 3] = [1.30, 0.15, -1.45];

/// Strength→deflected-fraction table (commercial filter stops), log2-interpolated.
const STRENGTH_BREAKPOINTS: [f64; 5] = [0.125, 0.25, 0.5, 1.0, 2.0];
const STRENGTH_TOTAL_FRACTION: [f64; 5] = [0.10, 0.20, 0.35, 0.55, 0.75];

fn family_shape(family: &str) -> Option<FamilyShape> {
    let s = match family {
        "glimmerglass" => FamilyShape {
            core: GroupCfg {
                lambda_um: 10.0,
                spread: 1.5,
                n_components: 2,
                alpha: 0.0,
            },
            halo: GroupCfg {
                lambda_um: 50.0,
                spread: 2.0,
                n_components: 3,
                alpha: 0.0,
            },
            bloom: GroupCfg {
                lambda_um: 260.0,
                spread: 2.5,
                n_components: 4,
                alpha: 3.2,
            },
            w_c: 0.60,
            w_h: 0.30,
            w_b: 0.10,
            halo_warmth_base: 0.0,
            total_gain: 0.65,
        },
        "black_pro_mist" => FamilyShape {
            core: GroupCfg {
                lambda_um: 16.0,
                spread: 1.5,
                n_components: 2,
                alpha: 0.0,
            },
            halo: GroupCfg {
                lambda_um: 95.0,
                spread: 2.0,
                n_components: 3,
                alpha: 0.0,
            },
            bloom: GroupCfg {
                lambda_um: 380.0,
                spread: 2.5,
                n_components: 4,
                alpha: 3.5,
            },
            w_c: 0.40,
            w_h: 0.47,
            w_b: 0.13,
            halo_warmth_base: 0.65,
            total_gain: 0.75,
        },
        "pro_mist" => FamilyShape {
            core: GroupCfg {
                lambda_um: 14.0,
                spread: 1.5,
                n_components: 2,
                alpha: 0.0,
            },
            halo: GroupCfg {
                lambda_um: 150.0,
                spread: 2.0,
                n_components: 3,
                alpha: 0.0,
            },
            bloom: GroupCfg {
                lambda_um: 650.0,
                spread: 2.5,
                n_components: 4,
                alpha: 2.9,
            },
            w_c: 0.28,
            w_h: 0.42,
            w_b: 0.30,
            halo_warmth_base: 0.40,
            total_gain: 1.05,
        },
        "cinebloom" => FamilyShape {
            core: GroupCfg {
                lambda_um: 20.0,
                spread: 1.5,
                n_components: 2,
                alpha: 0.0,
            },
            halo: GroupCfg {
                lambda_um: 200.0,
                spread: 2.0,
                n_components: 3,
                alpha: 0.0,
            },
            bloom: GroupCfg {
                lambda_um: 1000.0,
                spread: 2.5,
                n_components: 4,
                alpha: 2.5,
            },
            w_c: 0.22,
            w_h: 0.30,
            w_b: 0.48,
            halo_warmth_base: 0.85,
            total_gain: 1.00,
        },
        _ => return None,
    };
    Some(s)
}

/// The four valid diffusion-filter families (Python
/// `_DIFFUSION_FILTER_SHAPES` keys / `DIFFUSION_FILTER_FAMILIES`).
pub const VALID_FAMILIES: [&str; 4] = ["glimmerglass", "black_pro_mist", "pro_mist", "cinebloom"];

/// Reject an unknown diffusion-filter family with an actionable error.
/// Python 0.3.4 raises
/// `ValueError(f"Unknown diffusion filter family: {family!r}; "
///            f"available: {list(_DIFFUSION_FILTER_SHAPES)}")`
/// from `apply_diffusion_filter_um`; this mirrors it for the Result-based
/// Rust API so a malformed family can never silently pass through as a
/// no-op.
pub fn validate_family(family: &str) -> Result<(), String> {
    if VALID_FAMILIES.contains(&family) {
        Ok(())
    } else {
        Err(format!(
            "Unknown diffusion filter family: '{family}'; available: {VALID_FAMILIES:?}"
        ))
    }
}

/// Apply per-group intensity (weight) and size (lambda) multipliers, then
/// renormalise the group weights to sum to 1. Mirrors `_resolve_family_cfg`.
fn resolve_family_cfg(mut s: FamilyShape, df: &DiffusionFilter) -> FamilyShape {
    let (ci, hi, bi) = (df.core_intensity, df.halo_intensity, df.bloom_intensity);
    let (cs, hs, bs) = (df.core_size, df.halo_size, df.bloom_size);
    if ci == 1.0 && hi == 1.0 && bi == 1.0 && cs == 1.0 && hs == 1.0 && bs == 1.0 {
        return s;
    }
    let w_c = s.w_c * ci.max(0.0);
    let w_h = s.w_h * hi.max(0.0);
    let w_b = s.w_b * bi.max(0.0);
    let total = w_c + w_h + w_b;
    if total <= 0.0 {
        return s;
    }
    s.core.lambda_um *= cs.max(1e-6);
    s.halo.lambda_um *= hs.max(1e-6);
    s.bloom.lambda_um *= bs.max(1e-6);
    s.w_c = w_c / total;
    s.w_h = w_h / total;
    s.w_b = w_b / total;
    s
}

/// Largest lambda in the (resolved) bloom progression, image-plane μm.
fn bloom_max_lambda_um(cfg: &FamilyShape) -> f64 {
    cfg.bloom.lambda_um * cfg.bloom.spread
}

/// Deflected-photon fraction p_s for a strength + family. Mirrors `_strength_to_scatter`.
fn strength_to_scatter(strength: f64, gain: f64) -> f64 {
    if strength <= 0.0 {
        return 0.0;
    }
    let log_strength = strength.max(1e-6).log2();
    let log_breaks: Vec<f64> = STRENGTH_BREAKPOINTS.iter().map(|b| b.log2()).collect();
    let base_total = interp(log_strength, &log_breaks, &STRENGTH_TOTAL_FRACTION);
    (base_total * gain).clamp(0.0, 0.99)
}

/// numpy.interp: clamped piecewise-linear interpolation on a sorted table.
fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    if x <= xs[0] {
        return ys[0];
    }
    let last = xs.len() - 1;
    if x >= xs[last] {
        return ys[last];
    }
    for i in 1..xs.len() {
        if x <= xs[i] {
            let t = (x - xs[i - 1]) / (xs[i] - xs[i - 1]);
            return ys[i - 1] + t * (ys[i] - ys[i - 1]);
        }
    }
    ys[last]
}

/// `n` evenly spaced points in `[a, b]` inclusive (numpy.linspace).
fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    if n == 1 {
        return vec![a];
    }
    (0..n)
        .map(|i| a + (b - a) * (i as f64) / ((n - 1) as f64))
        .collect()
}

/// Expand a {core|halo|bloom} group into (sub-component lambdas_um, weights
/// summing to 1). Mirrors `_expand_group`.
fn expand_group(g: &GroupCfg, is_bloom: bool) -> (Vec<f64>, Vec<f64>) {
    let n = g.n_components.max(1);
    if n == 1 || g.spread <= 1.0 {
        return (vec![g.lambda_um], vec![1.0]);
    }
    let log_lo = (g.lambda_um / g.spread).ln();
    let log_hi = (g.lambda_um * g.spread).ln();
    let lambdas: Vec<f64> = linspace(log_lo, log_hi, n)
        .iter()
        .map(|x| x.exp())
        .collect();
    let mut weights: Vec<f64> = if is_bloom {
        lambdas.iter().map(|&l| l.powf(2.0 - g.alpha)).collect()
    } else {
        vec![1.0; n]
    };
    let s: f64 = weights.iter().sum();
    for w in &mut weights {
        *w /= s;
    }
    (lambdas, weights)
}

/// Energy-conserving per-channel halo weight redistribution. Returns one
/// weight vector per channel (R, G, B). Mirrors `_halo_channel_weights`.
fn halo_channel_weights(weights: &[f64], warmth: f64) -> [Vec<f64>; 3] {
    let n = weights.len();
    if n < 2 {
        return [weights.to_vec(), weights.to_vec(), weights.to_vec()];
    }
    let warmth = warmth.clamp(-1.5, 1.5);
    let mut g = linspace(-1.0, 1.0, n);
    let wsum: f64 = weights.iter().sum();
    let g_mean: f64 = g.iter().zip(weights).map(|(gi, wi)| gi * wi).sum::<f64>() / wsum;
    for gi in &mut g {
        *gi -= g_mean;
    }
    let target_total = wsum;
    let mut out = [vec![0.0; n], vec![0.0; n], vec![0.0; n]];
    for c in 0..3 {
        let raw: Vec<f64> = (0..n)
            .map(|k| (weights[k] * (1.0 + warmth * HALO_CHANNEL_WARMTH_AXIS[c] * g[k])).max(0.0))
            .collect();
        let s: f64 = raw.iter().sum();
        if s > 0.0 {
            for k in 0..n {
                out[c][k] = raw[k] * (target_total / s);
            }
        } else {
            out[c].copy_from_slice(weights);
        }
    }
    out
}

/// Σ_k weight_k · exp(-r/λ_k) / (2π λ_k²) over a radius grid.
fn exp_sum(r: &[f64], lambdas_px: &[f64], weights: &[f64]) -> Vec<f64> {
    let mut total = vec![0.0f64; r.len()];
    for (&wk, &lk0) in weights.iter().zip(lambdas_px) {
        let lk = lk0.max(1e-6);
        let denom = 2.0 * std::f64::consts::PI * lk * lk;
        for (t, &ri) in total.iter_mut().zip(r) {
            *t += wk * (-ri / lk).exp() / denom;
        }
    }
    total
}

/// Core / per-channel halo / bloom radial contributions (family weights
/// folded in). Mirrors `_radial_components`. `halo_warmth` is the effective
/// warmth (family base already added by the caller).
fn radial_components(
    r: &[f64],
    cfg: &FamilyShape,
    spatial_scale: f64,
    pixel_size_um: f64,
    halo_warmth: f64,
) -> (Vec<f64>, [Vec<f64>; 3], Vec<f64>) {
    let spatial_scale = spatial_scale.max(1e-6);
    let (core_l, core_w) = expand_group(&cfg.core, false);
    let (halo_l, halo_w) = expand_group(&cfg.halo, false);
    let (bloom_l, bloom_w) = expand_group(&cfg.bloom, true);
    let halo_per_ch = halo_channel_weights(&halo_w, halo_warmth);

    let to_px = |ls: &[f64]| -> Vec<f64> {
        ls.iter()
            .map(|&l| l * spatial_scale / pixel_size_um)
            .collect()
    };
    let core_px = to_px(&core_l);
    let halo_px = to_px(&halo_l);
    let bloom_px = to_px(&bloom_l);

    let mut core = exp_sum(r, &core_px, &core_w);
    for v in &mut core {
        *v *= cfg.w_c;
    }
    let mut bloom = exp_sum(r, &bloom_px, &bloom_w);
    for v in &mut bloom {
        *v *= cfg.w_b;
    }
    let halo = std::array::from_fn(|c| {
        let mut h = exp_sum(r, &halo_px, &halo_per_ch[c]);
        for v in &mut h {
            *v *= cfg.w_h;
        }
        h
    });
    (core, halo, bloom)
}

/// Per-channel 2D PSF (k×k each), sum-normalised per channel. Mirrors
/// `diffusion_filter_psf`; `halo_warmth` is the user knob (family base added here).
fn diffusion_filter_psf(
    k: usize,
    cfg: &FamilyShape,
    spatial_scale: f64,
    pixel_size_um: f64,
    halo_warmth: f64,
) -> [Vec<f64>; 3] {
    let center = (k / 2) as f64;
    let mut r = vec![0.0f64; k * k];
    for y in 0..k {
        for x in 0..k {
            let dx = x as f64 - center;
            let dy = y as f64 - center;
            r[y * k + x] = (dx * dx + dy * dy).sqrt();
        }
    }
    let effective_warmth = cfg.halo_warmth_base + halo_warmth;
    let (core, halo, bloom) =
        radial_components(&r, cfg, spatial_scale, pixel_size_um, effective_warmth);

    std::array::from_fn(|c| {
        let mut psf: Vec<f64> = (0..k * k)
            .map(|p| core[p] + halo[c][p] + bloom[p])
            .collect();
        let s: f64 = psf.iter().sum();
        if s > 0.0 {
            for v in &mut psf {
                *v /= s;
            }
        }
        psf
    })
}

/// Apply a lens diffusion-filter PSF to an RGB image. Port of Python's
/// `apply_diffusion_filter_um`. Returns the input unchanged when the filter
/// is effectively a no-op (strength/spatial_scale ≤ 0 or p_s ≤ 0); an
/// unknown family is an `Err` — Python 0.3.4 raises `ValueError` there
/// instead of silently passing the image through. `pixel_size_um` is the
/// image-plane sampling pitch.
pub fn apply_diffusion_filter_um(
    image: &ImageBuf,
    df: &DiffusionFilter,
    pixel_size_um: f64,
) -> Result<ImageBuf, String> {
    if df.strength <= 0.0 || df.spatial_scale <= 0.0 {
        return Ok(image.clone());
    }
    validate_family(df.family)?;
    // `validate_family` only passes for the four known family keys.
    let base = family_shape(df.family).expect("validated family");
    let p_s = strength_to_scatter(df.strength, base.total_gain);
    if p_s <= 0.0 {
        return Ok(image.clone());
    }
    let cfg = resolve_family_cfg(base, df);

    let w = image.width as usize;
    let h = image.height as usize;

    // Kernel radius: 8·λ_max captures 99.95% of a 2D exponential's energy.
    let bloom_max_px = bloom_max_lambda_um(&cfg) * df.spatial_scale / pixel_size_um;
    let mut radius = (8.0 * bloom_max_px).max(5.0).ceil() as usize;
    let cap = (h.min(w) / 2).saturating_sub(1).max(1);
    radius = radius.min(cap);
    let k = 2 * radius + 1;

    let psf = diffusion_filter_psf(k, &cfg, df.spatial_scale, pixel_size_um, df.halo_warmth);

    let mut out = ImageBuf::new(image.width, image.height);
    for c in 0..3 {
        let chan_s = image.extract_channel(c);
        let chan: Vec<f64> = chan_s.iter().map(|&v| v as f64).collect();
        let blurred = convolve2d_reflect(&chan, h, w, &psf[c], k);
        let mixed: Vec<Scalar> = (0..h * w)
            .map(|i| from_f64((1.0 - p_s) * chan[i] + p_s * blurred[i]))
            .collect();
        out.write_channel(c, &mixed);
    }
    Ok(out)
}

/// Apply diffusion during backend-driven previews using the faithful CPU PSF.
///
/// The former downsampled sum-of-Gaussians approximation changed the source
/// sampling, finite PSF normalisation, and boundary handling. Until a backend
/// implements the same discrete PSF and reflect convolution, previews use
/// `apply_diffusion_filter_um`, matching the CPU export path.
pub fn apply_diffusion_filter_blur(
    image: &ImageBuf,
    df: &DiffusionFilter,
    pixel_size_um: f64,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    tracing::debug!(
        backend = backend.name(),
        family = df.family,
        "diffusion filter: using faithful CPU PSF fallback"
    );
    apply_diffusion_filter_um(image, df, pixel_size_um)
}

#[cfg(test)]
mod family_tests {
    use super::*;

    fn df(family: &str, strength: f64) -> DiffusionFilter<'_> {
        DiffusionFilter {
            family,
            strength,
            spatial_scale: 1.0,
            halo_warmth: 0.0,
            core_intensity: 1.0,
            core_size: 1.0,
            halo_intensity: 1.0,
            halo_size: 1.0,
            bloom_intensity: 1.0,
            bloom_size: 1.0,
        }
    }

    fn ramp(n: usize) -> ImageBuf {
        // A 32×32 ramp with a bright corner block — exercises the halo/bloom
        // tails as well as the core.
        let mut data = Vec::with_capacity(n * n * 3);
        for y in 0..n {
            for x in 0..n {
                let v = (x.max(y) as f64 / (n as f64 - 1.0)).min(1.0);
                let hot = if x >= n - 8 && y < 8 { 1.0 } else { v * 0.3 };
                data.push(from_f64(hot));
                data.push(from_f64(hot * 0.9));
                data.push(from_f64(hot * 0.8));
            }
        }
        ImageBuf::from_data(n as u32, n as u32, data)
    }

    #[test]
    fn validate_family_matches_python_families() {
        for f in VALID_FAMILIES {
            assert!(validate_family(f).is_ok(), "{f} must be valid");
        }
        for bad in ["black_promist", "", "Black Pro Mist", "golden_glow"] {
            let err = validate_family(bad).unwrap_err();
            assert!(
                err.starts_with("Unknown diffusion filter family: '"),
                "message should mirror Python's ValueError, got: {err}"
            );
            assert!(
                err.contains("glimmerglass"),
                "must list the available families"
            );
        }
    }

    #[test]
    fn unknown_family_errors_instead_of_passing_through() {
        // Python 0.3.4 raises ValueError for an effective filter with an
        // unknown family — the image must NOT come back unchanged.
        let img = ramp(32);
        let e = apply_diffusion_filter_um(&img, &df("black_promist", 0.5), 10.0).unwrap_err();
        assert!(e.contains("Unknown diffusion filter family"));
        let e = apply_diffusion_filter_blur(
            &img,
            &df("black_promist", 0.5),
            10.0,
            &spektrafilm_gpu::cpu_backend::CpuBackend,
        )
        .unwrap_err();
        assert!(e.contains("Unknown diffusion filter family"));
    }

    #[test]
    fn ineffective_filter_never_reaches_the_family_check() {
        // Python returns the image before the family lookup when
        // strength/spatial_scale ≤ 0 — a bogus name must not error there.
        let img = ramp(32);
        let mut f = df("black_promist", 0.0);
        let out = apply_diffusion_filter_um(&img, &f, 10.0).unwrap();
        assert_eq!(out.data.len(), img.data.len());
        f.strength = 0.5;
        f.spatial_scale = 0.0;
        let out = apply_diffusion_filter_um(&img, &f, 10.0).unwrap();
        assert_eq!(out.data.len(), img.data.len());
    }

    #[test]
    fn all_four_families_scatter_and_stay_distinct() {
        let img = ramp(32);
        let mut outputs = Vec::new();
        for family in VALID_FAMILIES {
            let out = apply_diffusion_filter_um(&img, &df(family, 1.0), 10.0)
                .unwrap_or_else(|e| panic!("{family}: {e}"));
            // The filter must actually scatter: at least some pixel moves.
            let max_shift = out
                .data
                .iter()
                .zip(img.data.iter())
                .map(|(a, b)| {
                    (spektrafilm_math::precision::to_f64(*a)
                        - spektrafilm_math::precision::to_f64(*b))
                    .abs()
                })
                .fold(0.0f64, f64::max);
            assert!(
                max_shift > 1e-3,
                "{family} did nothing (max shift {max_shift})"
            );
            outputs.push((family, out));
        }
        for (i, (fa, oa)) in outputs.iter().enumerate() {
            for (fb, ob) in outputs.iter().skip(i + 1) {
                let max_diff = oa
                    .data
                    .iter()
                    .zip(ob.data.iter())
                    .map(|(a, b)| {
                        (spektrafilm_math::precision::to_f64(*a)
                            - spektrafilm_math::precision::to_f64(*b))
                        .abs()
                    })
                    .fold(0.0f64, f64::max);
                assert!(
                    max_diff > 1e-4,
                    "families {fa} and {fb} produced the same output"
                );
            }
        }
    }

    #[test]
    fn uniform_image_is_energy_conserving() {
        // (1−p_s)·c + p_s·(K_s∗c) = c for a PSF that sums to one per
        // channel — the model's energy-conservation invariant (Python
        // `_strength_to_scatter` docstring). Small spatial_scale keeps the
        // kernel inside the radius cap so truncation loss is negligible.
        let n = 48usize;
        let img = ImageBuf::from_data(n as u32, n as u32, vec![from_f64(0.37); n * n * 3]);
        for family in VALID_FAMILIES {
            let mut f = df(family, 1.0);
            f.spatial_scale = 0.02;
            let out = apply_diffusion_filter_um(&img, &f, 10.0).unwrap();
            let max_diff = out
                .data
                .iter()
                .map(|v| (spektrafilm_math::precision::to_f64(*v) - 0.37).abs())
                .fold(0.0f64, f64::max);
            assert!(
                max_diff < 2e-3,
                "{family} broke energy conservation on a uniform field ({max_diff})"
            );
        }
    }
}

#[cfg(test)]
mod preview_parity_tests {
    use super::*;

    #[test]
    fn preview_cpu_fallback_preserves_pinned_python_hotspot_diffusion() {
        // A bright corner exercises the discrete core and reflected halo/bloom
        // boundaries. Compare with Python 0.3.4 FFT samples from commit
        // 3bb2c2d2801ff68b92019cf1dbcbb133d60832bc, retaining the 0.005 budget.
        let n = 48usize;
        let mut data = Vec::with_capacity(n * n * 3);
        for y in 0..n {
            for x in 0..n {
                let v = (x.max(y) as f64 / (n as f64 - 1.0)).min(1.0);
                let hot = if x >= n - 12 && y < 12 { 1.0 } else { v * 0.3 };
                data.push(from_f64(hot));
                data.push(from_f64(hot * 0.9));
                data.push(from_f64(hot * 0.8));
            }
        }
        let img = ImageBuf::from_data(n as u32, n as u32, data);
        let df = DiffusionFilter {
            family: "black_pro_mist",
            strength: 1.0,
            spatial_scale: 0.05,
            halo_warmth: 0.0,
            core_intensity: 1.0,
            core_size: 1.0,
            halo_intensity: 1.0,
            halo_size: 1.0,
            bloom_intensity: 1.0,
            bloom_size: 1.0,
        };
        let preview =
            apply_diffusion_filter_blur(&img, &df, 10.0, &spektrafilm_gpu::cpu_backend::CpuBackend)
                .unwrap();
        // (x, y, RGB): dark/hot corners, both sides of the hotspot boundary,
        // its inner corner (the former approximation's maximum-error pixel),
        // the surrounding ramp, and the opposite image edges.
        let expected = [
            (
                0,
                0,
                [
                    0.0001656748356651663,
                    0.00011847992803590178,
                    0.00006887024631659705,
                ],
            ),
            (
                47,
                0,
                [0.9999611456128636, 0.8999656676998947, 0.7999702399825238],
            ),
            (
                35,
                0,
                [
                    0.22733680364241327,
                    0.20389738797708928,
                    0.18040232965255765,
                ],
            ),
            (
                36,
                0,
                [0.9959659137605512, 0.897091112168781, 0.7982732190282742],
            ),
            (
                35,
                11,
                [
                    0.22599846102267726,
                    0.20297636539409727,
                    0.17992097812422198,
                ],
            ),
            (
                36,
                11,
                [0.9933886879364295, 0.8951923565049538, 0.7970861067471279],
            ),
            (
                36,
                12,
                [
                    0.23237456842784404,
                    0.20871619741761965,
                    0.18502464005050112,
                ],
            ),
            (
                47,
                12,
                [0.303522512644374, 0.27253741571669504, 0.24150242300582822],
            ),
            (
                24,
                24,
                [
                    0.15327507540634389,
                    0.13793224324917236,
                    0.12258820284520613,
                ],
            ),
            (
                0,
                47,
                [0.2998862907538783, 0.2699192696864028, 0.23995395227045013],
            ),
            (
                47,
                47,
                [0.29993825634342175, 0.2699570193008415, 0.2399767747872168],
            ),
        ];
        let mut max_diff = 0.0f64;
        for (x, y, rgb) in expected {
            for (c, reference) in rgb.into_iter().enumerate() {
                let actual = spektrafilm_math::precision::to_f64(preview.data[(y * n + x) * 3 + c]);
                max_diff = max_diff.max((actual - reference).abs());
            }
        }
        assert!(
            max_diff < 5e-3,
            "preview CPU diffusion fallback differs from pinned Python FFT samples ({max_diff})"
        );
    }
}
