/// Scanning stage: convert print/film density back to RGB.
///
/// Dispatches the spectral integration to the GPU backend when available.
use rayon::prelude::*;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::pchip3d::{pchip_interp, prepare_pchip_3d};
use spektrafilm_math::precision::{Scalar, from_f64, to_f64};

use super::build_lut_grid;
use crate::params::RuntimeParams;
use crate::profile::Profile;

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

fn scan_reference_rgb(
    density_cmy: [f64; 3],
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    illuminant: &[f64],
    normalization: f64,
    adapt: &[[f64; 3]; 3],
    xyz_to_rgb: &[[f64; 3]; 3],
) -> [f64; 3] {
    let sample = ImageBuf::from_data(1, 1, density_cmy.into_iter().map(from_f64).collect());
    let captured = if color_ref.has_remap() || params.settings.use_scanner_lut {
        let (data_min, data_max) = scanner_lut_bounds(profile, params);
        scan_spectral_via_lut(
            &sample,
            channel_density,
            base_density,
            illuminant,
            normalization,
            adapt,
            xyz_to_rgb,
            backend,
            data_min,
            data_max,
            params.settings.lut_resolution as usize,
            color_ref,
        )
    } else {
        backend.scan_spectral(
            &sample,
            channel_density,
            base_density,
            illuminant,
            normalization,
            adapt,
            xyz_to_rgb,
        )
    };
    [
        to_f64(captured.data[0]),
        to_f64(captured.data[1]),
        to_f64(captured.data[2]),
    ]
}

fn positive_scan_requested(params: &RuntimeParams) -> bool {
    params.scanner.scan_output == "positive_scan"
}

fn interpret_negative_capture(
    mut rgb: ImageBuf,
    clear_rgb: [f64; 3],
    dense_rgb: [f64; 3],
) -> ImageBuf {
    rgb.data.par_chunks_exact_mut(3).for_each(|px| {
        for channel in 0..3 {
            let span = (clear_rgb[channel] - dense_rgb[channel]).max(1e-12);
            px[channel] =
                from_f64(((clear_rgb[channel] - to_f64(px[channel])) / span).clamp(0.0, 1.0));
        }
    });
    rgb
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
    let positive_scan = positive_scan_requested(params);
    let backend = if positive_scan {
        static CPU_BACKEND: spektrafilm_gpu::cpu_backend::CpuBackend =
            spektrafilm_gpu::cpu_backend::CpuBackend;
        &CPU_BACKEND as &dyn ComputeBackend
    } else {
        backend
    };
    let channel_density = crate::chain_prep::channel_density(profile);
    let base_density = if include_base {
        std::borrow::Cow::Borrowed(profile.data.base_density.as_slice())
    } else {
        std::borrow::Cow::Owned(vec![0.0; profile.data.base_density.len()])
    };

    let output_space = colorspace::resolve(&params.io.output_color_space)
        .expect("output color space must be validated before scanning");
    let prepared = crate::chain_prep::PreparedChain::for_scan(profile, params, scan_illuminant);
    let scan_context = &prepared.scan;
    let illuminant = &scan_context.illuminant;
    let normalization = scan_context.normalization;
    let adapt = scan_context.adapt;
    let base_xyz_to_rgb = scan_context.base_xyz_to_rgb;

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
        // Shared scan context keeps the CPU two-step CAT→RGB operation
        // order used by the reference path.
        let glare_rgb_offset_f64 = crate::chain_prep::glare_rgb_offset_f64(&scan_context);
        let glare_rgb_offset: [Scalar; 3] = glare_rgb_offset_f64.map(from_f64);
        let glare_amount = spektrafilm_model::glare::compute_random_glare_amount(
            rgb.width,
            rgb.height,
            glare.percent,
            glare.roughness,
            glare.blur,
            params.random_seed,
        );
        spektrafilm_model::glare::add_glare_with_amount(&mut rgb, &glare_amount, glare_rgb_offset);
    }

    // Positive interpretation is scanner-domain: optical capture and lens
    // blur happen before the calibrated negative endpoint inversion.
    if positive_scan {
        let clear_rgb = scan_reference_rgb(
            [0.0; 3],
            profile,
            params,
            backend,
            color_ref,
            &channel_density,
            base_density.as_ref(),
            &illuminant,
            normalization,
            &adapt,
            &base_xyz_to_rgb,
        );
        let (_, dense_density) = scanner_lut_bounds(profile, params);
        let dense_rgb = scan_reference_rgb(
            dense_density,
            profile,
            params,
            backend,
            color_ref,
            &channel_density,
            base_density.as_ref(),
            &illuminant,
            normalization,
            &adapt,
            &base_xyz_to_rgb,
        );
        if params.scanner.lens_blur > 0.0 {
            rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur);
        }
        rgb = interpret_negative_capture(rgb, clear_rgb, dense_rgb);
    }

    // Output gamut compression — direct scans preserve the historical
    // capture order; positive scans compress only after interpretation.
    if gamut.is_active() {
        rgb.data.par_chunks_exact_mut(3).for_each(|px| {
            let out = gamut.compress([px[0] as f64, px[1] as f64, px[2] as f64]);
            px[0] = from_f64(out[0]);
            px[1] = from_f64(out[1]);
            px[2] = from_f64(out[2]);
        });
    }

    // Direct scans apply optical blur after gamut compression, preserving
    // the established direct-scan behavior.
    if !positive_scan && params.scanner.lens_blur > 0.0 {
        rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur);
    }

    // Unsharp mask
    let [usm_sigma, usm_amount] = params.scanner.unsharp_mask;
    if usm_sigma > 0.0 && usm_amount > 0.0 {
        rgb = spektrafilm_model::optics::apply_unsharp_mask(&rgb, usm_sigma, usm_amount, backend);
    }

    // Grain consumes native display-encoded RGB, with no internal transfer
    // or primaries conversion. The scanned image supplies its output space.
    let grain_v2_active = params.film_render.grain.active
        && params.settings.rgb_to_raw_method != "mallett2019"
        && matches!(
            params.film_render.grain.engine,
            crate::params::grain::GrainEngine::V2
        );
    let magazine_active = params.magazine_print_color.active
        && params.magazine_print_color.strength > 0.0
        && params.workflow.route == "input > film > scan > magazine";
    let encoded_domain = params.io.output_cctf_encoding || grain_v2_active;
    if encoded_domain {
        rgb.data.par_chunks_exact_mut(3).for_each(|px| {
            let encoded =
                colorspace::encode_rgb([px[0] as f64, px[1] as f64, px[2] as f64], output_space);
            px[0] = from_f64(encoded[0]);
            px[1] = from_f64(encoded[1]);
            px[2] = from_f64(encoded[2]);
        });
    }
    if grain_v2_active {
        let mut grain = params.film_render.grain.resolved_grain_v2();
        if params.debug.deactivate_spatial_effects {
            grain.resolution_factor = 100.0;
        }
        grain.seed = params.random_seed as u32;
        let gpu_params = params
            .film_render
            .grain
            .gpu_params(params.random_seed, params.debug.deactivate_spatial_effects);
        rgb = backend
            .grain_v2(&rgb, &gpu_params)
            .unwrap_or_else(|| spektrafilm_model::grain::v2::apply_cpu(&rgb, grain));
    }
    // Magazine color is the final post-scan appearance, after scanner-domain
    // grain. A linear export uses the helper's finite transfer round-trip and
    // remains linear at the API boundary.
    if magazine_active {
        crate::magazine_print_color::apply(
            &mut rgb,
            &params.magazine_print_color,
            output_space,
            encoded_domain,
        );
    }
    // Linear exports retain the same grain realization as encoded exports.
    // Decode only the transfer; the stored same-space matrix ran above once.
    if grain_v2_active && !params.io.output_cctf_encoding {
        rgb.data.par_iter_mut().for_each(|value| {
            *value = from_f64(colorspace::cctf_decode(*value as f64, output_space.cctf));
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
    scan_with_options(
        density_cmy,
        profile,
        params,
        backend,
        color_ref,
        gamut,
        None,
        true,
    )
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
