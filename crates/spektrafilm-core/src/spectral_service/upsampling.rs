//! Spectral contraction, sensitivity adaptation and exposure normalization.
use super::illuminant::select_illuminant;
use super::registry::{SpectraCube, SpectraLut, load_lut, lut_descriptor};
use rayon::prelude::*;
use spektrafilm_math::spectral::{self, N_WAVELENGTHS, TcLut};
use std::path::Path;
// Pull in BLAS for the spectra→tc_lut contraction (matches Python's
// opt_einsum + numpy summation pattern bit-for-bit on macOS Accelerate).
#[cfg(not(target_os = "windows"))]
#[allow(unused_imports)]
use blas_src as _;

fn dgemm_no_transpose(a: &[f64], b: &[f64], c: &mut [f64], m: usize, n: usize, k: usize) {
    assert_eq!(a.len(), m * k, "dgemm_no_transpose: a.len() != m*k");
    assert_eq!(b.len(), k * n, "dgemm_no_transpose: b.len() != k*n");
    assert_eq!(c.len(), m * n, "dgemm_no_transpose: c.len() != m*n");

    #[cfg(not(target_os = "windows"))]
    unsafe {
        cblas::dgemm(
            cblas::Layout::RowMajor,
            cblas::Transpose::None,
            cblas::Transpose::None,
            m as i32,
            n as i32,
            k as i32,
            1.0,
            a,
            k as i32,
            b,
            n as i32,
            0.0,
            c,
            n as i32,
        );
    }

    #[cfg(target_os = "windows")]
    {
        for row in 0..m {
            for col in 0..n {
                let mut sum = 0.0f64;
                for kk in 0..k {
                    sum += a[row * k + kk] * b[kk * n + col];
                }
                c[row * n + col] = sum;
            }
        }
    }
}
/// Spectral Gaussian blur of the spectra cube along the wavelength axis.
///
/// Port of the `spectral_gaussian_blur` step of Python
/// `compute_hanatos2025_tc_lut`:
/// `scipy.ndimage.gaussian_filter(spectra_lut, (0, 0, sigma))`.
///
/// scipy semantics replicated exactly:
/// - 1-D correlation along the wavelength (last) axis only — the tc-grid
///   axes get sigma 0 and are untouched.
/// - `truncate = 4.0` → kernel radius `lw = int(4.0 * sigma + 0.5)`.
/// - The kernel is normalized over its full truncated support
///   (`phi_x / phi_x.sum()`), not renormalized per sample window.
/// - `mode='reflect'` boundary: the edge sample repeats,
///   `(d c b a | a b c d | d c b a)`, i.e. index `-1 → 0`, `n → n-1`.
pub fn gaussian_blur_wavelength(cube: &SpectraCube, sigma: f64) -> SpectraCube {
    let lw = (4.0 * sigma + 0.5) as i64;
    let n = cube.n_wavelengths;
    if sigma <= 0.0 || lw == 0 || n == 0 {
        return cube.clone();
    }

    // `scipy.ndimage._gaussian_kernel1d` (order 0):
    //   phi_x = exp(-0.5 / sigma^2 * x^2); phi_x /= phi_x.sum()
    let sigma2 = sigma * sigma;
    let mut kernel = vec![0.0f64; (2 * lw + 1) as usize];
    for (k, w) in kernel.iter_mut().enumerate() {
        let x = k as f64 - lw as f64;
        *w = (-0.5 / sigma2 * x * x).exp();
    }
    let kernel_sum = pairwise_sum_f64(&kernel);
    for w in kernel.iter_mut() {
        *w /= kernel_sum;
    }

    let n_i64 = n as i64;
    let mut data = vec![0.0f64; cube.data.len()];
    let src = cube.data.as_slice();
    // Each tc cell's spectrum is an independent 81-sample row; cells are
    // independent, so blur them in parallel.
    data.par_chunks_exact_mut(n)
        .enumerate()
        .for_each(|(cell, out)| {
            let row = &src[cell * n..cell * n + n];
            for i in 0..n {
                let mut acc = 0.0f64;
                for k in -lw..=lw {
                    // scipy's symmetric ('reflect') extension — it keeps
                    // reflecting for radii larger than the line: index -1 → 0,
                    // n → n-1, ..., period 2n.
                    let j = (i as i64 + k).rem_euclid(2 * n_i64);
                    let j = if j >= n_i64 { 2 * n_i64 - 1 - j } else { j };
                    acc += kernel[(k + lw) as usize] * row[j as usize];
                }
                out[i] = acc;
            }
        });

    SpectraCube {
        size: cube.size,
        n_wavelengths: cube.n_wavelengths,
        data,
    }
}

/// Compute the TC LUT for a given film stock.
///
/// Port of Python `compute_hanatos2025_tc_lut` (no-adaptation path):
///   tc_lut[i][j][c] = sum_wl( spectra_lut[i][j][wl] * sensitivity[wl][c] )
///
/// The result maps tc coordinates → per-channel film raw exposure,
/// normalized so that the reference illuminant midgray produces balanced
/// exposure on the green channel (matching Python's `raw / raw_midgray[1]`).
pub fn compute_tc_lut(spectra_cube: &SpectraCube, sensitivity: &[[f64; 3]]) -> TcLut {
    let size = spectra_cube.size;
    let n_wl = spectra_cube.n_wavelengths.min(sensitivity.len());
    let channels = 3;

    let mut data = vec![0.0f64; size * size * channels];

    for i in 0..size {
        for j in 0..size {
            let spectrum = spectra_cube.spectrum(i, j);
            let mut raw = [0.0f64; 3];
            for wl in 0..n_wl {
                for c in 0..3 {
                    raw[c] += spectrum[wl] * sensitivity[wl][c];
                }
            }
            let base = (i * size + j) * channels;
            data[base] = raw[0];
            data[base + 1] = raw[1];
            data[base + 2] = raw[2];
        }
    }

    TcLut {
        size,
        channels,
        data,
    }
}

/// Build a TC→film-raw LUT from an effective-reflectance surface by relighting
/// each reflectance with the film/reference illuminant first:
///
/// `raw[i,j,c] = sum_wl reflectance[i,j,wl] * illuminant[wl] * sensitivity[wl,c]`
pub fn compute_reflectance_tc_lut(
    reflectance_lut: &SpectraLut,
    sensitivity: &[[f64; 3]],
    illuminant: &[f64],
) -> TcLut {
    let size = reflectance_lut.size;
    let n_wl = reflectance_lut
        .n_wavelengths
        .min(sensitivity.len())
        .min(illuminant.len());
    let channels = 3;
    let mut data = vec![0.0f64; size * size * channels];

    for i in 0..size {
        for j in 0..size {
            let spectrum = reflectance_lut.spectrum(i, j);
            let mut raw = [0.0f64; 3];
            for wl in 0..n_wl {
                let lit = spectrum[wl] as f64 * illuminant[wl];
                for c in 0..3 {
                    raw[c] += lit * sensitivity[wl][c];
                }
            }
            let base = (i * size + j) * channels;
            data[base] = raw[0];
            data[base + 1] = raw[1];
            data[base + 2] = raw[2];
        }
    }

    TcLut {
        size,
        channels,
        data,
    }
}
/// Descriptor-driven dispatcher used by the pipeline. Reflectance LUTs are
/// relit under the film reference illuminant and normalized at the descriptor's
/// scene white on every channel; irradiance LUTs use the Hanatos path.
pub fn compute_registered_tc_lut(
    data_dir: &Path,
    identifier: &str,
    sensitivity: &[[f64; 3]],
    reference_illuminant: &[f64],
    adaptation: Option<&Hanatos2025Adaptation<'_>>,
) -> Result<TcLut, String> {
    let descriptor = lut_descriptor(data_dir, identifier)?;
    let spectra = load_lut(data_dir, identifier)?;
    match descriptor.kind.as_str() {
        "reflectance" => {
            let mut lut = compute_reflectance_tc_lut(&spectra, sensitivity, reference_illuminant);
            let scene = descriptor
                .scene_illuminant
                .as_deref()
                .ok_or_else(|| "reflectance descriptor missing scene_illuminant".to_string())?;
            let scene_xy = spectral::illuminant_to_xy(&select_illuminant(scene));
            let (tx, ty) = spectral::xy_to_tc(scene_xy.0, scene_xy.1);
            let neutral = spektrafilm_math::lut::bicubic_2d(
                &spectra.data,
                spectra.size,
                spectra.size,
                spectra.n_wavelengths,
                tx as f32 * (spectra.size - 1) as f32,
                ty as f32 * (spectra.size - 1) as f32,
            );
            let n_wl = neutral
                .len()
                .min(sensitivity.len())
                .min(reference_illuminant.len());
            let mut response = [0.0f64; 3];
            for wl in 0..n_wl {
                for c in 0..3 {
                    response[c] +=
                        neutral[wl] as f64 * reference_illuminant[wl] * sensitivity[wl][c];
                }
            }
            for c in 0..lut.channels {
                let n = response[c];
                if n.abs() > 1e-15 {
                    for cell in lut.data.chunks_exact_mut(lut.channels) {
                        cell[c] /= n;
                    }
                }
            }
            Ok(lut)
        }
        "irradiance" => {
            let adaptation = adaptation
                .ok_or_else(|| "hanatos2025 requires sensitivity adaptation".to_string())?;
            compute_hanatos2025_tc_lut(&spectra, sensitivity, adaptation)
        }
        other => Err(format!("unsupported spectral LUT kind {other:?}")),
    }
}

/// Contract the spectra cube against the sensitivity through the erf4 spectral
/// bandpass window, normalizing against the reference illuminant.
fn contract_with_window(
    spectra_cube: &SpectraCube,
    sensitivity: &[[f64; 3]],
    window_params: &[f64],
    reference_xy: (f64, f64),
) -> TcLut {
    let n_wl = spectra_cube
        .n_wavelengths
        .min(sensitivity.len())
        .min(N_WAVELENGTHS);
    let window = eval_erf4_bandpass(window_params);
    let (tx, ty) = spectral::xy_to_tc(reference_xy.0, reference_xy.1);
    let neutral = spektrafilm_math::lut::bicubic_2d_f64(
        &spectra_cube.data,
        spectra_cube.size,
        spectra_cube.size,
        spectra_cube.n_wavelengths,
        tx * (spectra_cube.size - 1) as f64,
        ty * (spectra_cube.size - 1) as f64,
    );
    let mut num_per_wl = [
        Vec::<f64>::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
    ];
    let mut den_per_wl = [
        Vec::<f64>::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
    ];
    for wl in 0..n_wl {
        for c in 0..3 {
            let si = sensitivity[wl][c] * neutral[wl];
            num_per_wl[c].push(si * window[wl][c]);
            den_per_wl[c].push(si);
        }
    }
    let mut window_normalized = window.clone();
    for c in 0..3 {
        let num_c = pairwise_sum_f64(&num_per_wl[c]);
        let den_c = pairwise_sum_f64(&den_per_wl[c]);
        if num_c > 1e-10 && den_c > 1e-10 {
            let normalization_c = num_c / den_c;
            for wl in 0..n_wl {
                window_normalized[wl][c] = window[wl][c] / normalization_c;
            }
        }
    }

    // Compute raw LUT via BLAS dgemm to match Python's
    // `opt_einsum.contract('ijl,lm->ijm', spectra, sens*window)`.
    // numpy/opt_einsum routes this through GEMM with pairwise
    // accumulation, so a hand-rolled left-to-right loop is off by
    // 1-2 ULP per cell. Going through dgemm matches bit-for-bit.
    let size = spectra_cube.size;
    let channels = 3;
    let n_pix = size * size;
    let mut spec_f64 = vec![0.0f64; n_pix * n_wl];
    for i in 0..n_pix {
        let src = &spectra_cube.data
            [i * spectra_cube.n_wavelengths..i * spectra_cube.n_wavelengths + n_wl];
        let dst = &mut spec_f64[i * n_wl..(i + 1) * n_wl];
        dst.copy_from_slice(src);
    }
    // Build sens*window as (n_wl × 3) row-major.
    let mut sw_flat = vec![0.0f64; n_wl * 3];
    for wl in 0..n_wl {
        for c in 0..3 {
            sw_flat[wl * 3 + c] = sensitivity[wl][c] * window_normalized[wl][c];
        }
    }
    let mut data = vec![0.0f64; n_pix * channels];
    dgemm_no_transpose(&spec_f64, &sw_flat, &mut data, n_pix, channels, n_wl);

    TcLut {
        size,
        channels,
        data,
    }
}

/// Hanatos2025 sensitivity-adaptation controls that shape the filming TC LUT.
///
/// Mirrors the LUT-relevant fields of Python's
/// `Hanatos2025SensitivityAdaptation` (profiles/io.py): the erf4 bandpass
/// window parameters, the poly4 log-exposure-correction surface parameters,
/// the spectral Gaussian blur sigma and the reference illuminant (SPD for
/// normalization, xy for the surface center).
pub struct Hanatos2025Adaptation<'a> {
    /// (c_uv, sigma_uv, c_ir, sigma_ir) — erf4 bandpass parameters.
    pub window_params: &'a [f64],
    /// 3 channels × 15 polynomial coefficients (c0 unused) — poly4 surface.
    pub surface_params: &'a [Vec<f64>],
    /// Gaussian blur sigma in nm applied to the spectra LUT (0 = off).
    pub spectral_gaussian_blur: f64,
    /// Reference illuminant SPD (f64) for the window normalization.
    pub reference_illuminant: &'a [f64],
    /// Reference illuminant xy chromaticity (surface evaluation center).
    pub reference_illuminant_xy: (f64, f64),
    pub apply_window: bool,
    pub apply_surface: bool,
}

/// Compute the filming TC LUT with Hanatos2025 sensitivity adaptation.
///
/// Port of Python `compute_hanatos2025_tc_lut(sensitivity,
/// hanatos2025_adaptation)` (utils/spectral_upsampling.py), minus the
/// input-gamut bake which the pipeline applies afterwards as before:
///
/// 1. `spectral_gaussian_blur > 0` → Gaussian-blur the spectra cube along
///    the wavelength axis (`scipy.ndimage.gaussian_filter` semantics).
/// 2. `apply_window` → contract through the erf4 bandpass window,
///    normalized against the reference illuminant so white balance is
///    preserved; otherwise contract the sensitivity directly.
/// 3. `apply_surface` → multiply by `2 ** surface`, the poly4
///    log-exposure-correction surface centered on the illuminant.
///
/// Both window and surface preserve white balance by construction.
pub fn compute_hanatos2025_tc_lut(
    spectra_lut: &SpectraLut,
    sensitivity: &[[f64; 3]],
    adaptation: &Hanatos2025Adaptation,
) -> Result<TcLut, String> {
    let apply_window = adaptation.apply_window;
    if apply_window && adaptation.window_params.len() != 4 {
        return Err(format!(
            "hanatos2025 adaptation window requires exactly 4 erf4 parameters \
             (c_uv, sigma_uv, c_ir, sigma_ir), got {}",
            adaptation.window_params.len()
        ));
    }
    if adaptation.apply_surface {
        let rows = adaptation.surface_params.len();
        if rows != 3 || adaptation.surface_params.iter().any(|r| r.len() != 15) {
            return Err(format!(
                "hanatos2025 adaptation surface requires 3 channels × 15 poly4 \
                 coefficients, got {rows} rows of lengths {:?}",
                adaptation
                    .surface_params
                    .iter()
                    .map(|r| r.len())
                    .collect::<Vec<_>>()
            ));
        }
    }

    let mut cube = spectra_lut.to_f64_cube();
    if adaptation.spectral_gaussian_blur > 0.0 {
        cube = gaussian_blur_wavelength(&cube, adaptation.spectral_gaussian_blur);
    }

    let mut tc_lut = if apply_window {
        contract_with_window(
            &cube,
            sensitivity,
            adaptation.window_params,
            adaptation.reference_illuminant_xy,
        )
    } else {
        compute_tc_lut(&cube, sensitivity)
    };

    if adaptation.apply_surface {
        apply_log_exposure_surface(
            &mut tc_lut,
            adaptation.surface_params,
            adaptation.reference_illuminant_xy,
        );
    }

    Ok(tc_lut)
}

/// Evaluate the poly4 log-exposure-correction surface on the tc grid.
///
/// Port of Python `eval_poly4_log_exposure_surface` (the 0.3.4 default
/// surface model): a degree-4 polynomial per channel evaluated on the
/// `linspace(0, 1, size)` tc grid, centered (in tc coordinates) on the
/// reference illuminant chromaticity, passed through the bounded
/// Jakob & Hanika 2019 algebraic sigmoid (±`_HANATOS2025_MAX_CORRECTION_STOPS`
/// = 2 stops).
///
/// Returns a flat `[size * size * 3]` grid aligned with the TC LUT layout.
pub fn eval_poly4_log_exposure_surface(
    surface_params: &[Vec<f64>],
    illuminant_xy: (f64, f64),
    surface_size: usize,
) -> Vec<f64> {
    let (cx, cy) = spektrafilm_math::spectral::xy_to_tc(illuminant_xy.0, illuminant_xy.1);
    // np.linspace(0, 1, surface_size): start + i * step, step = 1/(n-1).
    let step = 1.0 / (surface_size as f64 - 1.0);
    let mut data = vec![0.0f64; surface_size * surface_size * 3];
    for i in 0..surface_size {
        let tx = i as f64 * step;
        for j in 0..surface_size {
            let ty = j as f64 * step;
            let base = (i * surface_size + j) * 3;
            for ch in 0..3 {
                let raw = poly2d_deg4(tx, ty, &surface_params[ch], (cx, cy));
                data[base + ch] = hanika_sigmoid(raw, HANATOS2025_MAX_CORRECTION_STOPS);
            }
        }
    }
    data
}

/// Multiply a TC LUT by `2 ** surface` — the `apply_surface` step of Python
/// `compute_hanatos2025_tc_lut` (`raw_lut *= 2**surface`).
pub fn apply_log_exposure_surface(
    tc_lut: &mut TcLut,
    surface_params: &[Vec<f64>],
    illuminant_xy: (f64, f64),
) {
    let surface = eval_poly4_log_exposure_surface(surface_params, illuminant_xy, tc_lut.size);
    for (cell, s) in tc_lut.data.iter_mut().zip(surface) {
        *cell *= 2.0f64.powf(s);
    }
}

/// Bounded correction of the hanatos2025 surface fit. Port of Python
/// `hanika_sigmoid`: `z / sqrt(1 + (z / max_val)^2)`, bounded to
/// ±`max_val` stops.
#[inline]
fn hanika_sigmoid(z: f64, max_val: f64) -> f64 {
    z / (1.0 + (z / max_val) * (z / max_val)).sqrt()
}

/// Degree-4 bivariate polynomial. Port of Python `poly2d_deg4`: the c0
/// term is intentionally dropped so `center_tc` maps to zero correction.
#[inline]
fn poly2d_deg4(tx: f64, ty: f64, params: &[f64], center_tc: (f64, f64)) -> f64 {
    // Python unpacks `_, c1, ..., c14 = params` (15 coefficients).
    let c1 = params[1];
    let c2 = params[2];
    let c3 = params[3];
    let c4 = params[4];
    let c5 = params[5];
    let c6 = params[6];
    let c7 = params[7];
    let c8 = params[8];
    let c9 = params[9];
    let c10 = params[10];
    let c11 = params[11];
    let c12 = params[12];
    let c13 = params[13];
    let c14 = params[14];
    let x = tx - center_tc.0;
    let y = ty - center_tc.1;
    let x2 = x * x;
    let y2 = y * y;
    let xy = x * y;
    let x3 = x2 * x;
    let y3 = y2 * y;
    c1 * x
        + c2 * y
        + c3 * x2
        + c4 * y2
        + c5 * xy
        + c6 * x3
        + c7 * y3
        + c8 * (x2 * y)
        + c9 * (x * y2)
        + c10 * (x2 * x2)
        + c11 * (y2 * y2)
        + c12 * (x3 * y)
        + c13 * (x2 * y2)
        + c14 * (x * y3)
}

/// Maximum hanatos2025 surface correction in stops — Python
/// `_HANATOS2025_MAX_CORRECTION_STOPS` (matches the fit).
const HANATOS2025_MAX_CORRECTION_STOPS: f64 = 2.0;

/// Camera UV/IR band-pass filter. Port of Python `compute_band_pass_filter`
/// (model/color_filters.py): two erf edges whose amplitudes are clipped to
/// [0, 1], multiplied together.
///
/// `filter_uv`/`filter_ir`: (amplitude, cutoff_wavelength_nm, edge_width_nm).
/// The IR edge uses a negated width, flipping the erf slope.
pub fn compute_band_pass_filter(filter_uv: [f64; 3], filter_ir: [f64; 3]) -> [f64; N_WAVELENGTHS] {
    let amp_uv = filter_uv[0].clamp(0.0, 1.0);
    let amp_ir = filter_ir[0].clamp(0.0, 1.0);
    let mut band_pass = [0.0f64; N_WAVELENGTHS];
    for i in 0..N_WAVELENGTHS {
        let wl = spectral::WAVELENGTH_MIN as f64 + (i as f64) * spectral::WAVELENGTH_STEP as f64;
        let edge_uv = 1.0 - amp_uv + amp_uv * sigmoid_erf(wl, filter_uv[1], filter_uv[2]);
        let edge_ir = 1.0 - amp_ir + amp_ir * sigmoid_erf(wl, filter_ir[1], -filter_ir[2]);
        band_pass[i] = edge_uv * edge_ir;
    }
    band_pass
}

/// `scipy.special.erf((x - center) / width) * 0.5 + 0.5` — Python
/// `sigmoid_erf` (model/color_filters.py).
#[inline]
fn sigmoid_erf(x: f64, center: f64, width: f64) -> f64 {
    erf((x - center) / width) * 0.5 + 0.5
}

/// Apply the camera UV/IR band-pass filter to the film sensitivity.
///
/// Port of the filtering in Python `FilmingStage._rgb_to_film_raw`
/// (runtime/stages/filming.py), active when either filter amplitude is
/// positive:
///
/// ```python
/// band_pass_filter = compute_band_pass_filter(filter_uv, filter_ir)
/// normalization = (np.sum(sensitivity * band_pass_filter * illuminant[:, None], axis=0)
///                  / np.sum(sensitivity * illuminant[:, None], axis=0))
/// sensitivity *= band_pass_filter[:, None] / normalization
/// ```
///
/// The reference-illuminant normalization preserves white balance: the
/// filtered sensitivity integrates to the same channel ratios under the
/// film's reference illuminant. Python applies this before BOTH upsampler
/// branches (hanatos2025 tc LUT and mallett2019 matrix), so callers must
/// filter the sensitivity before building either front-end.
pub fn apply_camera_uv_ir_band_pass(
    sensitivity: &mut [[f64; 3]],
    filter_uv: [f64; 3],
    filter_ir: [f64; 3],
    illuminant: &[f64],
) {
    let band_pass = compute_band_pass_filter(filter_uv, filter_ir);
    let n_wl = sensitivity.len().min(illuminant.len()).min(N_WAVELENGTHS);

    // numpy reduces the 81-wavelength axis with pairwise summation; the
    // numerator multiplies left-to-right: `(sens * band_pass) * illuminant`.
    let mut num = [
        Vec::<f64>::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
    ];
    let mut den = [
        Vec::<f64>::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
        Vec::with_capacity(n_wl),
    ];
    for wl in 0..n_wl {
        for c in 0..3 {
            let si = (sensitivity[wl][c] * band_pass[wl]) * illuminant[wl];
            num[c].push(si);
            den[c].push(sensitivity[wl][c] * illuminant[wl]);
        }
    }
    let mut normalization = [1.0f64; 3];
    for c in 0..3 {
        normalization[c] = pairwise_sum_f64(&num[c]) / pairwise_sum_f64(&den[c]);
    }

    // Python: `sensitivity *= band_pass_filter[:, None] / normalization`
    for wl in 0..n_wl {
        for c in 0..3 {
            sensitivity[wl][c] *= band_pass[wl] / normalization[c];
        }
    }
}

/// Compute the midgray normalization factor for Hanatos2025.
///
/// Port of Python: `raw_midgray = einsum('k,km->m', illuminant * 0.184, sensitivity)`
/// then normalize by `raw_midgray[1]` (green channel).
pub fn compute_midgray_normalization(sensitivity: &[[f64; 3]], illuminant: &[f32]) -> f64 {
    let n_wl = sensitivity.len().min(illuminant.len());
    let mut raw_midgray = [0.0f64; 3];
    for wl in 0..n_wl {
        for c in 0..3 {
            raw_midgray[c] += illuminant[wl] as f64 * 0.184 * sensitivity[wl][c];
        }
    }
    if raw_midgray[1] > 1e-10 {
        1.0 / raw_midgray[1]
    } else {
        1.0
    }
}

/// Public re-export: pairwise sum for f64 spectra/wavelength reductions.
/// See `pairwise_sum_f64` doc below.
pub fn pairwise_sum_f64_pub(xs: &[f64]) -> f64 {
    pairwise_sum_f64(xs)
}

/// Pairwise (recursive-halving) f64 summation. Matches numpy's
/// `np.add.reduce` reduction pattern, which is what `np.sum` uses for
/// 1-D arrays. For 81 elements this is recursive halving; the result
/// differs from a naive left-to-right sum by 1-2 ULPs but matches numpy
/// bit-for-bit.
fn pairwise_sum_f64(xs: &[f64]) -> f64 {
    match xs.len() {
        0 => 0.0,
        1 => xs[0],
        2 => xs[0] + xs[1],
        n => {
            let mid = n / 2;
            pairwise_sum_f64(&xs[..mid]) + pairwise_sum_f64(&xs[mid..])
        }
    }
}

/// Evaluate the erf4 spectral bandpass window.
/// params: (c_uv, sigma_uv, c_ir, sigma_ir)
/// Returns [81][3] window values (same for all 3 channels in erf4 model).
fn eval_erf4_bandpass(params: &[f64]) -> Vec<[f64; 3]> {
    let sqrt2 = std::f64::consts::SQRT_2;
    let c_uv = params[0];
    let sigma_uv = params[1];
    let c_ir = params[2];
    let sigma_ir = params[3];

    let mut window = vec![[0.0f64; 3]; N_WAVELENGTHS];
    for i in 0..N_WAVELENGTHS {
        let wl = spectral::WAVELENGTH_MIN as f64 + (i as f64) * spectral::WAVELENGTH_STEP as f64;
        let edge_uv = 0.5 * (1.0 + erf((wl - c_uv) / (sigma_uv * sqrt2)));
        let edge_ir = 0.5 * (1.0 - erf((wl - c_ir) / (sigma_ir * sqrt2)));
        let w = edge_uv * edge_ir;
        window[i] = [w, w, w];
    }
    window
}

/// f64 erf via libm — matches scipy.special.erf at full f64 precision.
#[inline]
fn erf(x: f64) -> f64 {
    libm::erf(x)
}
