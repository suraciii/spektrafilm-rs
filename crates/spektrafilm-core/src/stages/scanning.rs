/// Scanning stage: convert print/film density back to RGB.
///
/// Dispatches the spectral integration to the GPU backend when available.
use rayon::prelude::*;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::pchip3d::{pchip_interp, prepare_pchip_3d};
use spektrafilm_math::precision::{Scalar, from_f64};
use spektrafilm_math::spectral;

use crate::params::RuntimeParams;
use crate::profile::Profile;
use crate::spectral_service::select_illuminant_f64;

/// Build a `steps × steps² × 3` ImageBuf holding the LUT-input cmy grid.
/// Same layout as the enlarger LUT helper in `printing.rs`.
fn build_lut_grid(steps: usize, data_min: [f64; 3], data_max: [f64; 3]) -> ImageBuf {
    let mut grid = ImageBuf::new(steps as u32, (steps * steps) as u32);
    let step_inv = (steps - 1) as f64;
    for i in 0..steps {
        let x_r = data_min[0] + (data_max[0] - data_min[0]) * (i as f64) / step_inv;
        for j in 0..steps {
            let x_g = data_min[1] + (data_max[1] - data_min[1]) * (j as f64) / step_inv;
            for k in 0..steps {
                let x_b = data_min[2] + (data_max[2] - data_min[2]) * (k as f64) / step_inv;
                let row = i * steps + j;
                let base = (row * steps + k) * 3;
                grid.data[base] = from_f64(x_r);
                grid.data[base + 1] = from_f64(x_g);
                grid.data[base + 2] = from_f64(x_b);
            }
        }
    }
    grid
}

/// Scanner LUT bounds. When scan_film=true: `-grain.density_min` to
/// `nanmax(film.density_curves)`. Else: `nanmin..nanmax` of
/// `print.density_curves`. Mirrors Python `ScanningStage._density_to_rgb`.
fn scanner_lut_bounds(profile: &Profile, params: &RuntimeParams) -> ([f64; 3], [f64; 3]) {
    let curves = profile.density_curves_f64();
    let mut dmin_curves = [f64::INFINITY; 3];
    let mut dmax_curves = [f64::NEG_INFINITY; 3];
    for row in &curves {
        for c in 0..3 {
            if !row[c].is_nan() {
                if row[c] < dmin_curves[c] {
                    dmin_curves[c] = row[c];
                }
                if row[c] > dmax_curves[c] {
                    dmax_curves[c] = row[c];
                }
            }
        }
    }
    let (data_min, data_max) = if params.io.scan_film {
        let gmin = params.film_render.grain.density_min;
        ([-gmin[0], -gmin[1], -gmin[2]], dmax_curves)
    } else {
        (dmin_curves, dmax_curves)
    };
    let mut dm = data_min;
    let mut dx = data_max;
    for c in 0..3 {
        if !dm[c].is_finite() {
            dm[c] = 0.0;
        }
        if !dx[c].is_finite() {
            dx[c] = 1.0;
        }
        if dx[c] <= dm[c] {
            dx[c] = dm[c] + 1.0;
        }
    }
    (dm, dx)
}

/// Build a 17³ log_xyz LUT (matching Python's `cmy_to_log_xyz`),
/// PCHIP-interpolate per pixel, then apply `10^log_xyz → CAT →
/// xyz_to_rgb` to get RGB. Glare is added after this returns in RGB
/// space — algebraically equivalent to Python's add_glare-in-XYZ
/// because the matrices are linear (see comment in `scan()` below).
///
/// This LUTs the *same intermediate quantity* Python LUTs
/// (`cmy_to_log_xyz`), not the post-`10^x`/post-matrix RGB output —
/// keeps the PCHIP interpolating the smoother log10-domain surface
/// for bit-identical parity with Python's LUT path.
#[allow(clippy::too_many_arguments)]
fn scan_spectral_via_lut(
    density_cmy: &ImageBuf,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    illuminant: &[f64],
    normalization: f64,
    cat: &[[f64; 3]; 3],
    xyz_to_rgb: &[[f64; 3]; 3],
    _backend: &dyn ComputeBackend,
    data_min: [f64; 3],
    data_max: [f64; 3],
    steps: usize,
    color_ref: &crate::color_reference::ColorReference,
) -> ImageBuf {
    let grid = build_lut_grid(steps, data_min, data_max);
    // log_xyz LUT — same function Python LUTs.
    let lut_log_xyz = spektrafilm_gpu::cpu_backend::scan_log_xyz_cpu(
        &grid,
        channel_density,
        base_density,
        illuminant,
        normalization,
    );
    let prepared = prepare_pchip_3d(lut_log_xyz, steps);
    let scale = (steps - 1) as f64;
    let inv = [
        scale / (data_max[0] - data_min[0]),
        scale / (data_max[1] - data_min[1]),
        scale / (data_max[2] - data_min[2]),
    ];
    let mut out = density_cmy.clone();
    out.data
        .par_chunks_exact_mut(3)
        .zip(density_cmy.data.par_chunks_exact(3))
        .for_each(|(dst, src)| {
            let r = (src[0] as f64 - data_min[0]) * inv[0];
            let g = (src[1] as f64 - data_min[1]) * inv[1];
            let b = (src[2] as f64 - data_min[2]) * inv[2];
            let log_xyz = pchip_interp(&prepared, r, g, b);
            // Post-LUT: 10^log_xyz → CAT → xyz_to_rgb, mirroring Python's
            // `_density_to_rgb` AFTER the LUT call. bw_correction and
            // glare run after this returns (RGB space).
            let mut xyz = [
                10.0f64.powf(log_xyz[0]),
                10.0f64.powf(log_xyz[1]),
                10.0f64.powf(log_xyz[2]),
            ];
            // B&W/slide luminance remap on the pre-CAT scan XYZ (Python's
            // black_white_xyz_correction). Scalar scale from the Y channel;
            // no-op when color_ref has no remap.
            let scale = color_ref.xyz_scale(xyz[1]);
            if scale != 1.0 {
                xyz[0] *= scale;
                xyz[1] *= scale;
                xyz[2] *= scale;
            }
            let xa = cat[0][0] * xyz[0] + cat[0][1] * xyz[1] + cat[0][2] * xyz[2];
            let ya = cat[1][0] * xyz[0] + cat[1][1] * xyz[1] + cat[1][2] * xyz[2];
            let za = cat[2][0] * xyz[0] + cat[2][1] * xyz[1] + cat[2][2] * xyz[2];
            dst[0] =
                from_f64(xyz_to_rgb[0][0] * xa + xyz_to_rgb[0][1] * ya + xyz_to_rgb[0][2] * za);
            dst[1] =
                from_f64(xyz_to_rgb[1][0] * xa + xyz_to_rgb[1][1] * ya + xyz_to_rgb[1][2] * za);
            dst[2] =
                from_f64(xyz_to_rgb[2][0] * xa + xyz_to_rgb[2][1] * ya + xyz_to_rgb[2][2] * za);
        });
    out
}

pub fn scan_with_options(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
    scan_illuminant: Option<&str>,
    include_base: bool,
) -> ImageBuf {
    // Python parity — channel_density / base_density are f64 in the JSON profile.
    let channel_density: Vec<[f64; 3]> = profile
        .data
        .channel_density
        .iter()
        .map(|row| {
            [
                row.get(0).copied().unwrap_or(0.0),
                row.get(1).copied().unwrap_or(0.0),
                row.get(2).copied().unwrap_or(0.0),
            ]
        })
        .collect();
    let base_density: Vec<f64> = if include_base {
        profile.data.base_density.clone()
    } else {
        vec![0.0; profile.data.base_density.len()]
    };

    let illuminant = scan_illuminant
        .map(select_illuminant_f64)
        .unwrap_or_else(|| select_illuminant_f64(&profile.info.viewing_illuminant));
    let n_wl = illuminant
        .len()
        .min(channel_density.len())
        .min(spectral::N_WAVELENGTHS);

    // Compute normalization in f64 — Python parity (81-element sum).
    // Use f64 CMF_Y for the same precision reason.
    let normalization: f64 = (0..n_wl)
        .map(|i| illuminant[i] * spectral::CMF_Y_F64[i])
        .sum();
    // Build XYZ→RGB matrix with chromatic adaptation from viewing illuminant to output colorspace white.
    // Python's `colour.XYZ_to_RGB` applies CAT and the XYZ→RGB matrix as
    // TWO sequential matrix multiplies per pixel; pre-combining them
    // into one (M_rgb @ M_cat) loses ~1 ULP per output channel and
    // accumulates to ~5e-6 scan-stage drift. We keep them split for
    // CPU bit-parity; the GPU path can collapse them for performance.
    //
    // Compute viewing_white at runtime (matches Python's
    // `XYZ_to_xy(integrate(illu, CMFs)/norm) → xy_to_xyZ`). The
    // pre-computed `illuminant_xyz_f64` constant is 3 ULPs off the
    // runtime value because Python's `contract(...)/norm` rounds Y
    // slightly off 1.0, which xy_to_xyY then corrects — replaying that
    // chain is bit-exact.
    let mut illu_xyz_runtime = [0.0f64; 3];
    let illu_y_sum: f64 = (0..n_wl)
        .map(|i| illuminant[i] * spectral::CMF_Y_F64[i])
        .sum();
    for i in 0..n_wl {
        illu_xyz_runtime[0] += illuminant[i] * spectral::CMF_X_F64[i];
        illu_xyz_runtime[1] += illuminant[i] * spectral::CMF_Y_F64[i];
        illu_xyz_runtime[2] += illuminant[i] * spectral::CMF_Z_F64[i];
    }
    let _ = illu_y_sum;
    for c in 0..3 {
        illu_xyz_runtime[c] /= normalization;
    }
    // XYZ_to_xy then xy_to_xyz roundtrip (renormalizes Y to exactly 1).
    let sum_xyz = illu_xyz_runtime[0] + illu_xyz_runtime[1] + illu_xyz_runtime[2];
    let vx = illu_xyz_runtime[0] / sum_xyz;
    let vy = illu_xyz_runtime[1] / sum_xyz;
    let viewing_white = [vx / vy, 1.0f64, (1.0 - vx - vy) / vy];
    let output_space = colorspace::resolve(&params.io.output_color_space)
        .expect("output color space must be validated before scanning");
    let adapt = colorspace::chromatic_adaptation_matrix_f64(
        viewing_white,
        output_space.whitepoint_xyz(),
    );
    let base_xyz_to_rgb = output_space.matrix_xyz_to_rgb;

    // Dispatch spectral integration to backend (GPU or CPU). The backend
    // applies the two matrices in sequence — we pass them separately.
    //
    // `use_scanner_lut` path: evaluate `scan_spectral` on a 17³ CMY
    // grid and PCHIP-interpolate per pixel. Glare is added in RGB
    // space after this returns (the existing post-step at the bottom
    // of this function), so swapping to the LUT path is glare-safe.
    // The faithful B&W/slide luminance remap (color_ref.has_remap) applies
    // per-pixel on the scan XYZ inside scan_spectral_via_lut — so route
    // through the LUT path when it's active (the LUT result matches the
    // direct spectral path to sub-LSB, and only this path exposes the
    // pre-CAT XYZ hook). Otherwise honour use_scanner_lut.
    let mut rgb = if color_ref.has_remap() || params.settings.use_scanner_lut {
        let (data_min, data_max) = scanner_lut_bounds(profile, params);
        scan_spectral_via_lut(
            density_cmy,
            &channel_density,
            &base_density,
            &illuminant,
            normalization,
            &adapt,
            &base_xyz_to_rgb,
            backend,
            data_min,
            data_max,
            params.settings.lut_resolution as usize,
            color_ref,
        )
    } else {
        backend.scan_spectral(
            density_cmy,
            &channel_density,
            &base_density,
            &illuminant,
            normalization,
            &adapt,
            &base_xyz_to_rgb,
        )
    };

    // White/black correction is fully handled by color_ref's XYZ luminance
    // remap (applied inside the scan paths above). For the combos where no
    // remap is built we apply nothing: scan_film+negative because upstream
    // Python explicitly skips it (`black_white_xyz_correction`: "do not
    // correct negative film scans"), and a positive print paper because
    // upstream crashes there (its printing exposure correction returns None)
    // — unsupported, and unrepresentable with the shipped profiles anyway.

    // Viewing glare (Python: `add_glare(xyz, illuminant_xyz, glare)` between scan_spectral
    // and chromatic adapt + matrix). Since CAT+matrix is linear, we equivalently add
    // glare in RGB space by pre-multiplying the illuminant XYZ through the same matrix.
    //   Python: rgb = M @ (xyz + g*illu) = M@xyz + g*(M@illu)
    //   Rust:   rgb = M @ xyz; then rgb += g * (M@illu)
    //
    // Python 0.3.4 `ScanningStage._density_to_rgb` sets `glare = None` on the
    // `io.scan_film` path — direct-film scans get no viewing glare, and
    // `film_render.glare` is never read anywhere upstream. Only the print
    // path consumes `print_render.glare` (scan_film=false).
    let glare = (!params.io.scan_film)
        .then(|| &params.print_render.glare)
        .filter(|g| g.active && g.percent > 0.0);
    if let Some(glare) = glare {
        // Illuminant XYZ (Y=1) from the SPD (matches Python `contract('k,kl->l', illu, CMFs)/norm`).
        // Use the unnormalized integration here — the scaling cancels because we apply M next.
        let mut illu_xyz = [0.0f64; 3];
        for i in 0..n_wl {
            illu_xyz[0] += illuminant[i] * spectral::CMF_X_F64[i];
            illu_xyz[1] += illuminant[i] * spectral::CMF_Y_F64[i];
            illu_xyz[2] += illuminant[i] * spectral::CMF_Z_F64[i];
        }
        for c in 0..3 {
            illu_xyz[c] /= normalization;
        }
        // Two-step: M_base @ M_cat @ illu_xyz — same order as the
        // per-pixel scan path so the glare offset stays consistent.
        let xyz_adapt = [
            adapt[0][0] * illu_xyz[0] + adapt[0][1] * illu_xyz[1] + adapt[0][2] * illu_xyz[2],
            adapt[1][0] * illu_xyz[0] + adapt[1][1] * illu_xyz[1] + adapt[1][2] * illu_xyz[2],
            adapt[2][0] * illu_xyz[0] + adapt[2][1] * illu_xyz[1] + adapt[2][2] * illu_xyz[2],
        ];
        let glare_rgb_offset: [Scalar; 3] = [
            from_f64(
                base_xyz_to_rgb[0][0] * xyz_adapt[0]
                    + base_xyz_to_rgb[0][1] * xyz_adapt[1]
                    + base_xyz_to_rgb[0][2] * xyz_adapt[2],
            ),
            from_f64(
                base_xyz_to_rgb[1][0] * xyz_adapt[0]
                    + base_xyz_to_rgb[1][1] * xyz_adapt[1]
                    + base_xyz_to_rgb[1][2] * xyz_adapt[2],
            ),
            from_f64(
                base_xyz_to_rgb[2][0] * xyz_adapt[0]
                    + base_xyz_to_rgb[2][1] * xyz_adapt[1]
                    + base_xyz_to_rgb[2][2] * xyz_adapt[2],
            ),
        ];
        let glare_amount = spektrafilm_model::glare::compute_random_glare_amount(
            rgb.width,
            rgb.height,
            glare.percent,
            glare.roughness,
            glare.blur,
            0,
        );
        spektrafilm_model::glare::add_glare_with_amount(&mut rgb, &glare_amount, glare_rgb_offset);
    }

    // Output gamut compression — mirrors Python's `compress_rgb` after
    // XYZ→RGB and glare, before blur/unsharp. No-op when inactive.
    if gamut.is_active() {
        rgb.data.par_chunks_exact_mut(3).for_each(|px| {
            let out = gamut.compress([px[0] as f64, px[1] as f64, px[2] as f64]);
            px[0] = from_f64(out[0]);
            px[1] = from_f64(out[1]);
            px[2] = from_f64(out[2]);
        });
    }

    // Lens blur
    if params.scanner.lens_blur > 0.0 {
        rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur);
    }

    // Unsharp mask
    let [usm_sigma, usm_amount] = params.scanner.unsharp_mask;
    if usm_sigma > 0.0 && usm_amount > 0.0 {
        rgb =
            spektrafilm_model::diffusion::apply_unsharp_mask(&rgb, usm_sigma, usm_amount, backend);
    }

    // Match colour.RGB_to_RGB(cs, cs): apply the stored same-space matrix
    // roundtrip and destination CCTF. Preserve values outside [0, 1] for
    // formats and later consumers that support extended range.
    if params.io.output_cctf_encoding {
        rgb.data.par_chunks_exact_mut(3).for_each(|px| {
            let encoded = colorspace::encode_rgb(
                [px[0] as f64, px[1] as f64, px[2] as f64],
                output_space,
            );
            px[0] = from_f64(encoded[0]);
            px[1] = from_f64(encoded[1]);
            px[2] = from_f64(encoded[2]);
        });
    }

    rgb
}

pub fn scan(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
) -> ImageBuf {
    scan_with_options(density_cmy, profile, params, backend, color_ref, gamut, None, true)
}

pub fn process(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
) -> ImageBuf {
    scan(density_cmy, profile, params, backend, color_ref, gamut)
}


