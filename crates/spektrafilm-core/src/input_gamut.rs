//! Input gamut compression — faithful port of upstream
//! `spektrafilm/utils/gamut_compression.py` (input side).
//!
//! The compression is baked into the tc_lut once at build time via
//! remap-resample: `new_lut[xy] = old_lut[compress(xy)]`. The per-pixel
//! runtime path (RGB → CIE xy → tc → bilinear LUT sample) therefore stays
//! compression-agnostic. Both upstream algorithms are ported: the production
//! `"xy"` default (ACES-RGC-style radial compression toward the visible
//! spectral locus) and the inspection-only `"oklch"` variant
//! (CSS-Color-4-style chroma reduction in OkLab). The upstream `active`
//! flag gates the whole remap; `false` passes the LUT through unchanged.

use std::sync::{Arc, LazyLock};

use rayon::prelude::*;
use spektrafilm_math::spectral::{self, CMF_X_F64, CMF_Y_F64, CMF_Z_F64, TcLut};

use crate::gamut_compression::{oklab_to_xyz, reinhard_knee, xyz_to_oklab};
use crate::params::InputGamutCompressParams;

/// CIE 1931 2° visible spectral locus as a closed xy polygon, sampled at 5 nm
/// from 380 to 700 nm (65 vertices + the first repeated). Mirrors upstream
/// `spectral_locus_xy()`; the CMFs are the same colour-science values.
pub(crate) fn spectral_locus_xy() -> Vec<[f64; 2]> {
    // Wavelengths 380..=700 step 5 → indices 0..65 of the 380..780 grid.
    const N: usize = 65;
    let mut poly = Vec::with_capacity(N + 1);
    for i in 0..N {
        let (x, y, z) = (CMF_X_F64[i], CMF_Y_F64[i], CMF_Z_F64[i]);
        let total = (x + y + z).max(1e-12);
        poly.push([x / total, y / total]);
    }
    poly.push(poly[0]);
    poly
}

/// Distance from `origin` along unit `direction` to the first intersection
/// with the closed polygon, via parametric segment intersection. Returns
/// `f64::INFINITY` for rays that miss (should not happen for an interior
/// origin and the visible locus). Mirrors upstream `_ray_polygon_distance`.
fn ray_polygon_distance(origin: [f64; 2], direction: [f64; 2], polygon: &[[f64; 2]]) -> f64 {
    let mut t_min = f64::INFINITY;
    let (dx, dy) = (direction[0], direction[1]);
    for seg in polygon.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
        let denom = dx * ey - dy * ex;
        if denom.abs() <= 1e-12 {
            continue;
        }
        let (ox, oy) = (origin[0] - a[0], origin[1] - a[1]);
        // origin + t·direction = a + s·edge
        let t = (-ox * ey + oy * ex) / denom;
        let s = (-ox * dy + oy * dx) / denom;
        if t > 1e-9 && (0.0..=1.0).contains(&s) && t < t_min {
            t_min = t;
        }
    }
    t_min
}

/// ACES-RGC-style radial compression of a single CIE xy toward the spectral
/// locus, around `white_xy`. Hue (dominant wavelength) is preserved by
/// construction. Mirrors upstream `compress_xy_radial`.
fn compress_xy_radial(
    xy: [f64; 2],
    white_xy: [f64; 2],
    knee: (f64, f64, f64),
    locus: &[[f64; 2]],
) -> [f64; 2] {
    let delta = [xy[0] - white_xy[0], xy[1] - white_xy[1]];
    let dist = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
    if dist < 1e-9 {
        return xy;
    }
    let safe_dist = dist.max(1e-12);
    let direction = [delta[0] / safe_dist, delta[1] / safe_dist];
    let boundary = ray_polygon_distance(white_xy, direction, locus);
    let d_norm = dist / boundary.max(1e-12);
    let (t, l, p) = knee;
    let d_compressed = reinhard_knee(d_norm, t, l, p);
    let scaled = d_compressed * boundary;
    [
        white_xy[0] + direction[0] * scaled,
        white_xy[1] + direction[1] * scaled,
    ]
}

// ─────────────────────────────────────────────────────────────────────────
// Oklch input compressor + per-locus C_max(L, h) cache — mirrors upstream
// `_OKLCH_CMAX_TABLE_*` / `_build_oklch_c_max_table` / `compress_oklch_chroma`.
// ─────────────────────────────────────────────────────────────────────────

const N_L: usize = 64;
const N_H: usize = 720;
const N_BISECT: usize = 18;
/// Input-side lightness grid: `linspace(0.05, 1.0, 64)` (the output-side
/// table uses 0.02 — do not share constants).
const L_MIN: f64 = 0.05;
const L_MAX: f64 = 1.0;
/// "0.5 covers all realistic chromas" (upstream comment).
const CHROMA_UPPER: f64 = 0.5;

#[inline]
fn h_grid(j: usize) -> f64 {
    // linspace(-pi, pi, N_H, endpoint=False)
    -std::f64::consts::PI + (2.0 * std::f64::consts::PI) * (j as f64) / (N_H as f64)
}

/// Even-odd ray-crossing point-in-polygon test. The spectral locus is a
/// single convex loop, so this matches `matplotlib.path.Path.contains_points`
/// (non-zero winding and even-odd agree on convex loops).
fn point_in_polygon(p: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut inside = false;
    for seg in polygon.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x_int = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if p[0] < x_int {
                inside = !inside;
            }
        }
    }
    inside
}

/// xyY → XYZ at Y = 1 (lightness-preserving reconstruction). Mirrors
/// upstream `_xy_to_xyz_unit_y`.
fn xy_to_xyz_unit_y(xy: [f64; 2]) -> [f64; 3] {
    let safe_y = xy[1].max(1e-12);
    [xy[0] / safe_y, 1.0, (1.0 - xy[0] - xy[1]) / safe_y]
}

/// XYZ → xy with the upstream `fmax(total, 1e-12)` guard. Mirrors
/// `_xyz_to_xy`.
fn xyz_to_xy(xyz: [f64; 3]) -> [f64; 2] {
    let total = (xyz[0] + xyz[1] + xyz[2]).max(1e-12);
    [xyz[0] / total, xyz[1] / total]
}

/// Bisect the max Oklch chroma at each (L, h) grid node such that the
/// reconstructed chromaticity stays inside the spectral locus. Mirrors
/// upstream `_build_oklch_c_max_table`, computed once per process (the locus
/// is a fixed CIE 1931 singleton, so one table serves every film).
fn build_oklch_c_max_table(locus: &[[f64; 2]]) -> Vec<f64> {
    let mut table = vec![0.0f64; N_L * N_H];
    table
        .par_chunks_exact_mut(N_H)
        .enumerate()
        .for_each(|(i, row)| {
            let l = L_MIN + (L_MAX - L_MIN) * (i as f64) / ((N_L - 1) as f64);
            for (j, out) in row.iter_mut().enumerate() {
                let h = h_grid(j);
                let (cos_h, sin_h) = (h.cos(), h.sin());
                let mut lo = 0.0f64;
                let mut hi = CHROMA_UPPER;
                for _ in 0..N_BISECT {
                    let mid = (lo + hi) * 0.5;
                    let xyz = oklab_to_xyz([l, mid * cos_h, mid * sin_h]);
                    let xy = xyz_to_xy(xyz);
                    if point_in_polygon(xy, locus) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                *out = lo;
            }
        });
    table
}

/// Process-wide input `C_max` table (the locus is a fixed CIE 1931
/// singleton, so one table serves every film).
static OKLCH_CMAX: LazyLock<Arc<Vec<f64>>> =
    LazyLock::new(|| Arc::new(build_oklch_c_max_table(&spectral_locus_xy())));

/// Bilinear `C_max(L, h)` lookup (L clamped to the grid, h wrapped) —
/// mirrors upstream `_c_max_lookup`.
fn c_max_lookup(table: &[f64], l: f64, h: f64) -> f64 {
    let l = l.clamp(L_MIN, L_MAX);
    let h_step = h_grid(1) - h_grid(0);
    let h_idx = (h - h_grid(0)) / h_step;
    let h_floor = h_idx.floor();
    let h_lo = (h_floor as i64).rem_euclid(N_H as i64) as usize;
    let h_hi = (h_lo + 1) % N_H;
    let h_frac = h_idx - h_floor;

    let l_idx = (l - L_MIN) / (L_MAX - L_MIN) * ((N_L - 1) as f64);
    let l_lo = (l_idx.floor() as i64).clamp(0, (N_L - 2) as i64) as usize;
    let l_hi = l_lo + 1;
    let l_frac = l_idx - l_lo as f64;

    let v00 = table[l_lo * N_H + h_lo];
    let v01 = table[l_lo * N_H + h_hi];
    let v10 = table[l_hi * N_H + h_lo];
    let v11 = table[l_hi * N_H + h_hi];
    v00 * (1.0 - l_frac) * (1.0 - h_frac)
        + v01 * (1.0 - l_frac) * h_frac
        + v10 * l_frac * (1.0 - h_frac)
        + v11 * l_frac * h_frac
}

/// CSS-Color-4-style chroma reduction in Oklch: convert xy (at Y = 1) →
/// OkLab → OkLch, compress C only (L and h preserved), reconstruct. Mirrors
/// upstream `compress_oklch_chroma` (`white_xy` is unused there too, kept
/// for API symmetry).
fn compress_oklch_chroma(xy: [f64; 2], knee: (f64, f64, f64), table: &[f64]) -> [f64; 2] {
    let lab = xyz_to_oklab(xy_to_xyz_unit_y(xy));
    let (l, a, b) = (lab[0], lab[1], lab[2]);
    let c = a.hypot(b);
    let h = b.atan2(a);

    let c_max = c_max_lookup(table, l, h);
    let safe_c_max = c_max.max(1e-9);
    let d_norm = c / safe_c_max;
    let (t, lim, p) = knee;
    let d_compressed = reinhard_knee(d_norm, t, lim, p);
    let c_new = d_compressed * safe_c_max;

    let xyz_new = oklab_to_xyz([l, c_new * h.cos(), c_new * h.sin()]);
    xyz_to_xy(xyz_new)
}

/// Input compression algorithm selector. Mirrors upstream
/// `InputGamutCompressSpec.algorithm`.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Algorithm {
    /// Radial compression in CIE 1931 chromaticity toward the spectral
    /// locus (ACES RGC family) — the production default.
    Xy,
    /// Chroma reduction at constant Oklch (L, h).
    Oklch,
}

/// Configured input gamut compressor. `build` resolves and validates the
/// upstream `InputGamutCompressSpec` fields (`active`, `algorithm`, `knee`);
/// `is_active` reports whether a non-identity remap will be applied.
pub struct InputGamutCompress {
    active: bool,
    algorithm: Algorithm,
    knee: (f64, f64, f64),
    locus: Vec<[f64; 2]>,
    c_max: Arc<Vec<f64>>,
}

impl InputGamutCompress {
    /// Resolve from the params. `active = false` disables compression (the
    /// remap passes the LUT through unchanged). Any algorithm other than
    /// `"xy"`/`"oklch"`, or an invalid knee, errors — mirroring upstream
    /// `InputGamutCompressSpec.__post_init__` (unsupported values fail
    /// before any artifact is produced).
    pub fn build(params: &InputGamutCompressParams) -> Result<Self, String> {
        let algorithm = match params.algorithm.as_str() {
            "xy" => Algorithm::Xy,
            "oklch" => Algorithm::Oklch,
            other => {
                return Err(format!(
                    "input gamut compression algorithm must be 'xy' or 'oklch', got {other:?}"
                ))
            }
        };
        let [t, l, p] = params.knee;
        if !(0.0..1.0).contains(&t) {
            return Err(format!(
                "input gamut compression knee threshold must be in [0, 1), got {t}"
            ));
        }
        if l <= 0.0 {
            return Err(format!("input gamut compression knee limit must be > 0, got {l}"));
        }
        if p <= 0.0 {
            return Err(format!("input gamut compression knee power must be > 0, got {p}"));
        }
        let locus = spectral_locus_xy();
        let c_max = match (params.active, algorithm) {
            (true, Algorithm::Oklch) => OKLCH_CMAX.clone(),
            _ => Arc::new(Vec::new()),
        };
        Ok(Self {
            active: params.active,
            algorithm,
            knee: (t as f64, l as f64, p as f64),
            locus,
            c_max,
        })
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Compress a single xy chromaticity per the configured algorithm.
    /// Mirrors upstream `compress_xy` (identity when inactive).
    pub(crate) fn compress_xy(&self, xy: [f64; 2], white_xy: [f64; 2]) -> [f64; 2] {
        if !self.active {
            return xy;
        }
        match self.algorithm {
            Algorithm::Xy => compress_xy_radial(xy, white_xy, self.knee, &self.locus),
            Algorithm::Oklch => {
                compress_oklch_chroma(xy, self.knee, &self.c_max)
            }
        }
    }

    /// Bake the compression into a freshly resampled copy of `lut`. For each
    /// LUT cell we decode its tc index to CIE xy, compress, re-encode to tc,
    /// and bilinearly sample the original LUT there (clamp-to-edge boundary,
    /// matching `scipy.ndimage.map_coordinates(order=1, mode="nearest")`).
    /// `ref_xy` is the film reference illuminant chromaticity — the
    /// compression's achromatic axis. Returns `lut` unchanged when inactive.
    pub fn remap(&self, lut: &TcLut, ref_xy: [f64; 2]) -> TcLut {
        if !self.active {
            return lut.clone();
        }
        let size = lut.size;
        let ch = lut.channels;
        let inv = 1.0 / (size as f64 - 1.0);
        let mut data = vec![0.0f64; size * size * ch];
        data.par_chunks_exact_mut(ch)
            .enumerate()
            .for_each(|(cell, out)| {
                let (i, j) = (cell / size, cell % size);
                // Step 1: tc cell → CIE xy.
                let (x, y) = spectral::tc_to_xy(i as f64 * inv, j as f64 * inv);
                // Step 2: compress.
                let cxy = self.compress_xy([x, y], ref_xy);
                // Step 3: compressed xy → tc.
                let (tx, ty) = spectral::xy_to_tc(cxy[0], cxy[1]);
                // Step 4: bilinear sample original LUT at (tx, ty) grid coords.
                let sample =
                    bilinear_sample(lut, tx * (size as f64 - 1.0), ty * (size as f64 - 1.0));
                out[..ch].copy_from_slice(&sample[..ch]);
            });
        TcLut {
            size,
            channels: ch,
            data,
        }
    }
}

/// Bilinear sample of all channels at fractional grid coordinate `(ci, cj)`,
/// clamping fetch indices to the edge (scipy `mode="nearest"`).
fn bilinear_sample(lut: &TcLut, ci: f64, cj: f64) -> [f64; 3] {
    let size = lut.size;
    let ch = lut.channels;
    let max_idx = size as isize - 1;
    let clamp = |v: isize| v.clamp(0, max_idx) as usize;

    let bi = ci.floor();
    let bj = cj.floor();
    let fi = ci - bi;
    let fj = cj - bj;
    let (i0, i1) = (clamp(bi as isize), clamp(bi as isize + 1));
    let (j0, j1) = (clamp(bj as isize), clamp(bj as isize + 1));

    let cell = |i: usize, j: usize, c: usize| lut.data[(i * size + j) * ch + c];
    let mut out = [0.0f64; 3];
    for (c, slot) in out.iter_mut().enumerate().take(ch) {
        let v00 = cell(i0, j0, c);
        let v01 = cell(i0, j1, c);
        let v10 = cell(i1, j0, c);
        let v11 = cell(i1, j1, c);
        *slot = v00 * (1.0 - fi) * (1.0 - fj)
            + v01 * (1.0 - fi) * fj
            + v10 * fi * (1.0 - fj)
            + v11 * fi * fj;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(algorithm: &str, active: bool) -> InputGamutCompressParams {
        InputGamutCompressParams {
            active,
            algorithm: algorithm.into(),
            knee: [0.0, 1.0, 6.0],
        }
    }

    #[test]
    fn disabled_algorithms_preserve_out_of_locus_chromaticities() {
        for algorithm in ["xy", "oklch"] {
            let compressor = InputGamutCompress::build(&params(algorithm, false)).unwrap();
            for xy in [[0.8, 0.2], [0.05, 0.02], [0.15, 0.8]] {
                assert_eq!(compressor.compress_xy(xy, [0.33243, 0.34744]), xy);
            }
        }
    }

    /// Deterministic synthetic LUT, mirrored bit-for-bit in the Python
    /// pinned git-show Python probe: cell (i, j, c) = `((i² + 3j + 7c) % 97) / 97`.
    fn synthetic_lut(size: usize) -> TcLut {
        let mut data = vec![0.0f64; size * size * 3];
        for i in 0..size {
            for j in 0..size {
                for c in 0..3 {
                    let v = ((i * i + j * 3 + c * 7) % 97) as f64 / 97.0;
                    data[(i * size + j) * 3 + c] = v;
                }
            }
        }
        TcLut {
            size,
            channels: 3,
            data,
        }
    }

    /// The baked tc_lut remap matches the upstream remap-resample
    /// (`remap_tc_lut_for_compression` with the `"xy"` algorithm) against the
    /// CIE 1931 2° locus, sampled with scipy `map_coordinates(order=1,
    /// mode="nearest")`.
    #[test]
    fn remap_matches_python_reference() {
        let spec = InputGamutCompress::build(&params("xy", true)).unwrap();
        let lut = synthetic_lut(64);
        let out = spec.remap(&lut, [0.3127, 0.3290]);

        // (i, j) → [r, g, b] from the Python reference.
        let cells = [
            (0usize, 0usize, [0.4137658005, 0.1877218965, 0.2598868450]),
            (10, 20, [0.7521338372, 0.8242987857, 0.8964637341]),
            (32, 32, [0.5463917966, 0.6185567450, 0.6907215587]),
            (50, 5, [0.5807479510, 0.0691153288, 0.1412802772]),
            (63, 63, [0.6891891640, 0.7613541125, 0.8335190609]),
            (5, 60, [0.0995187055, 0.1716836540, 0.2438486024]),
        ];
        for (i, j, expected) in cells {
            let base = (i * 64 + j) * 3;
            for c in 0..3 {
                assert!(
                    (out.data[base + c] - expected[c]).abs() < 1e-7,
                    "cell ({i},{j}) ch {c}: got {}, expected {}",
                    out.data[base + c],
                    expected[c],
                );
            }
        }

        let checksum: f64 = out.data.iter().sum();
        assert!(
            (checksum - 5846.5426164588).abs() < 1e-6,
            "checksum {checksum}",
        );
    }

    /// Fresh pinned 0.3.4 remap probe exercises table lookup, tc mapping,
    /// channel interpolation and nearest-edge handling for Oklch compression.
    #[test]
    fn oklch_remap_matches_pinned_python() {
        let spec = InputGamutCompress::build(&params("oklch", true)).unwrap();
        let out = spec.remap(&synthetic_lut(64), [0.3127, 0.329]);
        let cells = [
            (0usize, 0usize, [0.30973501908479395, 0.38189996753840216, 0.4540649159920104]),
            (10usize, 20usize, [0.390231319454233, 0.4623962679078412, 0.5345612163614495]),
            (32usize, 32usize, [0.546394870475601, 0.6185598189292094, 0.6907152119946107]),
            (50usize, 5usize, [0.1387877654727771, 0.21095271392638537, 0.2831176623799936]),
            (63usize, 63usize, [0.4193442458939582, 0.49150919434756646, 0.5636741428011747]),
            (5usize, 60usize, [0.35586974920999714, 0.42803469766360536, 0.5001996461172137]),
        ];
        for (i, j, want) in cells {
            let base = (i * 64 + j) * 3;
            for c in 0..3 {
                assert!((out.data[base + c] - want[c]).abs() < 1e-6,
                    "cell ({i}, {j}) channel {c}: {} expected {}", out.data[base + c], want[c]);
            }
        }
        let checksum: f64 = out.data.iter().sum();
        assert!((checksum - 6060.726600945642).abs() < 1e-6, "checksum {checksum}");
    }

    /// An inactive compressor (`active = false`) returns the LUT unchanged —
    /// the upstream `spec.active` passthrough in `compress_xy`.
    #[test]
    fn inactive_is_identity() {
        let spec = InputGamutCompress::build(&params("xy", false)).unwrap();
        assert!(!spec.is_active());
        let lut = synthetic_lut(16);
        let out = spec.remap(&lut, [0.3127, 0.3290]);
        assert_eq!(out.data, lut.data);
        // Inactive oklch likewise passes through.
        let spec = InputGamutCompress::build(&params("oklch", false)).unwrap();
        assert!(!spec.is_active());
        let out = spec.remap(&lut, [0.3127, 0.3290]);
        assert_eq!(out.data, lut.data);
    }

    /// Unknown algorithms fail (upstream raises `ValueError` — never a
    /// silent default).
    #[test]
    fn unknown_algorithm_fails() {
        assert!(InputGamutCompress::build(&params("off", true)).is_err());
        assert!(InputGamutCompress::build(&params("aces_rgc", true)).is_err());
        assert!(InputGamutCompress::build(&params("", true)).is_err());
    }

    /// Invalid knees fail, mirroring upstream `__post_init__` validation.
    #[test]
    fn invalid_knee_fails() {
        let mut p = params("xy", true);
        p.knee = [1.0, 1.0, 6.0];
        assert!(InputGamutCompress::build(&p).is_err());
        p.knee = [0.0, 0.0, 6.0];
        assert!(InputGamutCompress::build(&p).is_err());
        p.knee = [0.0, 1.0, -1.0];
        assert!(InputGamutCompress::build(&p).is_err());
    }

    /// `oklch` chroma compression reference rows from upstream
    /// `compress_oklch_chroma` (knee (0, 1, 6)) around the D55 film
    /// reference white — regenerated against pinned Python 0.3.4.
    #[test]
    fn oklch_matches_python_reference() {
        let spec = InputGamutCompress::build(&params("oklch", true)).unwrap();
        // (xy in, xy out) pairs from the Python reference.
        let cases: [([f64; 2], [f64; 2]); 7] = [
            ([0.3127, 0.329], [0.3126999999999998, 0.3290000000000001]),
            ([0.3341, 0.345], [0.33409999360229875, 0.3449999954202616]),
            ([0.7347, 0.2653], [0.6274088393042179, 0.297513482354933]),
            ([0.8, 0.2], [0.5833350407616412, 0.2774177957006853]),
            ([0.15, 0.8], [0.19019187433749238, 0.6955307950940326]),
            ([0.05, 0.02], [0.07959461349111929, 0.2759231240242639]),
            ([0.3324, 0.3474], [0.33239999253168373, 0.3473999932588385]),
        ];
        for (xy, want) in cases {
            let got = spec.compress_xy(xy, [0.3324, 0.3474]);
            for c in 0..2 {
                assert!(
                    (got[c] - want[c]).abs() < 1e-6,
                    "xy {xy:?} ch {c}: got {} want {}",
                    got[c],
                    want[c]
                );
            }
        }
    }

    /// `xy` radial compression reference rows from upstream
    /// `compress_xy_radial` (knee (0, 1, 6)) around the D55 white.
    #[test]
    fn xy_matches_python_reference() {
        let spec = InputGamutCompress::build(&params("xy", true)).unwrap();
        let cases: [([f64; 2], [f64; 2]); 7] = [
            ([0.3127, 0.329], [0.31270000105451345, 0.3290000009849263]),
            ([0.3341, 0.345], [0.33409999999999923, 0.3450000000000011]),
            ([0.7347, 0.2653], [0.6907988180263942, 0.27425920218750444]),
            ([0.8, 0.2], [0.6688464892135599, 0.24134308701865112]),
            ([0.15, 0.8], [0.16812093141693846, 0.7550354519774871]),
            ([0.05, 0.02], [0.11699956781337714, 0.09767584455417735]),
            ([0.3324, 0.3474], [0.3324, 0.3474]),
        ];
        for (xy, want) in cases {
            let got = spec.compress_xy(xy, [0.3324, 0.3474]);
            for c in 0..2 {
                assert!(
                    (got[c] - want[c]).abs() < 1e-9,
                    "xy {xy:?} ch {c}: got {} want {}",
                    got[c],
                    want[c]
                );
            }
        }
    }
}
