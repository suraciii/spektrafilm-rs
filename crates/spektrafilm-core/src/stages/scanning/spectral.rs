use rayon::prelude::*;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::pchip3d::{pchip_interp, prepare_pchip_3d};
use spektrafilm_math::precision::from_f64;

use super::super::build_lut_grid;
use crate::params::RuntimeParams;
use crate::profile::Profile;

/// Scanner LUT bounds. When scan_film=true: `-grain.density_min` to
/// `nanmax(film.density_curves)`. Else: `nanmin..nanmax` of
/// `print.density_curves`.
pub(crate) fn scanner_lut_bounds(
    profile: &Profile,
    params: &RuntimeParams,
) -> ([f64; 3], [f64; 3]) {
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

struct PreparedScannerLut {
    pchip: spektrafilm_math::pchip3d::PreparedPchip3d,
    minimum: [f64; 3],
    inverse_step: [f64; 3],
}

fn prepare_scanner_lut(
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    color: &crate::chain_prep::ScanColorContext,
    minimum: [f64; 3],
    maximum: [f64; 3],
    steps: usize,
) -> PreparedScannerLut {
    let grid = build_lut_grid(steps, minimum, maximum);
    // log_xyz LUT over the clear-film-normalized capture: the normalization
    // is the clear-film Y reading of the scan context, so the interpolated
    // surface is the same quantity the direct kernel produces.
    let log_xyz = spektrafilm_backend::cpu_backend::scan_log_xyz_cpu(
        &grid,
        channel_density,
        base_density,
        &color.illuminant,
        color.normalization,
    );
    let pchip = prepare_pchip_3d(log_xyz, steps);
    let scale = (steps - 1) as f64;
    PreparedScannerLut {
        pchip,
        minimum,
        inverse_step: std::array::from_fn(|c| scale / (maximum[c] - minimum[c])),
    }
}

/// Build a 17³ log_xyz LUT, PCHIP-interpolate per pixel, then apply
/// `10^log_xyz → CAT → xyz_to_rgb` to get RGB. Glare is added after this
/// returns in RGB space — algebraically equivalent to an add in XYZ because
/// the matrices are linear.
#[allow(clippy::too_many_arguments)]
fn scan_spectral_via_lut(
    density_cmy: &ImageBuf,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    color: &crate::chain_prep::ScanColorContext,
    backend: &dyn ComputeBackend,
    data_min: [f64; 3],
    data_max: [f64; 3],
    steps: usize,
) -> Result<ImageBuf, String> {
    let device_params = spektrafilm_backend::ScannerSpectralLutGpuParams {
        channel_density,
        base_density,
        illuminant: &color.illuminant,
        normalization: color.normalization,
        cat: &color.adapt,
        xyz_to_rgb: &color.base_xyz_to_rgb,
        minimum: data_min,
        maximum: data_max,
        size: steps as u32,
    };
    match backend.scanner_spectral_lut(density_cmy, &device_params)? {
        Some(image) => return Ok(image),
        None if backend.is_gpu() => {
            return Err("scanner_spectral_lut: selected GPU has no device implementation".into());
        }
        None => {}
    }
    let _lut = super::super::StageObservation::cpu(
        backend,
        "scanner_lut",
        spektrafilm_backend::telemetry::CpuReason::SpectralLut,
    );
    let prepared = prepare_scanner_lut(
        channel_density,
        base_density,
        color,
        data_min,
        data_max,
        steps,
    );
    let cat = &color.adapt;
    let xyz_to_rgb = &color.base_xyz_to_rgb;
    let mut out = density_cmy.clone();
    out.data
        .par_chunks_exact_mut(3)
        .zip(density_cmy.data.par_chunks_exact(3))
        .for_each(|(dst, src)| {
            let r = (src[0] as f64 - prepared.minimum[0]) * prepared.inverse_step[0];
            let g = (src[1] as f64 - prepared.minimum[1]) * prepared.inverse_step[1];
            let b = (src[2] as f64 - prepared.minimum[2]) * prepared.inverse_step[2];
            let log_xyz = pchip_interp(&prepared.pchip, r, g, b);
            let xyz = [
                10.0f64.powf(log_xyz[0]),
                10.0f64.powf(log_xyz[1]),
                10.0f64.powf(log_xyz[2]),
            ];
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
    Ok(out)
}

/// Spectral integration of one density image under the scan colour context:
/// the neutral scan model's colorimetric readout, normalized by the clear-film
/// reading and adapted from the clear-film white to the output white.
#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_spectral_rgb(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    color: &crate::chain_prep::ScanColorContext,
) -> Result<ImageBuf, String> {
    if params.settings.use_scanner_lut {
        let (data_min, data_max) = scanner_lut_bounds(profile, params);
        scan_spectral_via_lut(
            density_cmy,
            channel_density,
            base_density,
            color,
            backend,
            data_min,
            data_max,
            params.settings.lut_resolution as usize,
        )
    } else {
        backend.scan_spectral(
            density_cmy,
            channel_density,
            base_density,
            &color.illuminant,
            color.normalization,
            &color.adapt,
            &color.base_xyz_to_rgb,
        )
    }
}
