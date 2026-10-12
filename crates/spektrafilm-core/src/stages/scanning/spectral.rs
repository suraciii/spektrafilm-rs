use rayon::prelude::*;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::pchip3d::{pchip_interp, prepare_pchip_3d};
use spektrafilm_math::precision::{from_f64, to_f64};

use super::super::build_lut_grid;
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
    // log_xyz LUT — same function Python LUTs.
    let log_xyz = spektrafilm_gpu::cpu_backend::scan_log_xyz_cpu(
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

/// Build a 17³ log_xyz LUT (matching Python's `cmy_to_log_xyz`),
/// PCHIP-interpolate per pixel, then apply `10^log_xyz → CAT →
/// xyz_to_rgb` to get RGB. Glare is added after this returns in RGB
/// space — algebraically equivalent to Python's add_glare-in-XYZ
/// because the matrices are linear (see the glare note in
/// `scan_with_options` below).
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
    color: &crate::chain_prep::ScanColorContext,
    _backend: &dyn ComputeBackend,
    data_min: [f64; 3],
    data_max: [f64; 3],
    steps: usize,
    color_ref: &crate::color_reference::ColorReference,
) -> ImageBuf {
    let _lut = super::super::StageObservation::cpu(
        _backend,
        "scanner_lut",
        spektrafilm_gpu::telemetry::CpuReason::SpectralLut,
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

/// Spectral integration of one density image under the scan colour context.
///
/// The faithful B&W/slide luminance remap (`color_ref.has_remap`) only exists
/// on the LUT path, so it forces that path; `settings.use_scanner_lut` selects
/// it otherwise. The bulk scan and the one-pixel reference samples share this
/// switch, so a new path condition cannot desynchronize them.
// Both scan paths share the complete spectral context.
#[allow(clippy::too_many_arguments)]
pub(super) fn scan_spectral_rgb(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    color: &crate::chain_prep::ScanColorContext,
    defer_correction: bool,
) -> ImageBuf {
    if let Some(receiver) = &color.receiver {
        let mut capture = receiver.capture(density_cmy, channel_density, base_density);
        if color_ref.has_remap() && !defer_correction {
            let to_y = receiver.luminance_weights();
            capture.data.par_chunks_exact_mut(3).for_each(|px| {
                let y = (0..3).map(|c| to_y[c] * to_f64(px[c])).sum();
                let scale = color_ref.xyz_scale(y);
                if scale != 1.0 {
                    for value in px {
                        *value = from_f64(to_f64(*value) * scale);
                    }
                }
            });
        }
        return capture;
    }
    if color_ref.has_remap() || params.settings.use_scanner_lut {
        let (data_min, data_max) = scanner_lut_bounds(profile, params);
        let identity = crate::color_reference::ColorReference::identity();
        scan_spectral_via_lut(
            density_cmy,
            channel_density,
            base_density,
            color,
            backend,
            data_min,
            data_max,
            params.settings.lut_resolution as usize,
            if defer_correction {
                &identity
            } else {
                color_ref
            },
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
/// Prepared CPU f64 forward capture for inverse-table calibration.
pub(crate) struct ForwardCapture<'a> {
    channel_density: &'a [[f64; 3]],
    base_density: &'a [f64],
    color: &'a crate::chain_prep::ScanColorContext,
    color_ref: &'a crate::color_reference::ColorReference,
    lut: Option<PreparedScannerLut>,
}

impl<'a> ForwardCapture<'a> {
    pub(crate) fn prepare(
        profile: &Profile,
        params: &RuntimeParams,
        channel_density: &'a [[f64; 3]],
        base_density: &'a [f64],
        color: &'a crate::chain_prep::ScanColorContext,
        color_ref: &'a crate::color_reference::ColorReference,
    ) -> Self {
        let (minimum, maximum) = scanner_lut_bounds(profile, params);
        let steps = params.settings.lut_resolution as usize;
        let lut = if color.receiver.is_none()
            && (params.settings.use_scanner_lut || color_ref.has_remap())
        {
            Some(prepare_scanner_lut(
                channel_density,
                base_density,
                color,
                minimum,
                maximum,
                steps,
            ))
        } else {
            None
        };
        Self {
            channel_density,
            base_density,
            color,
            color_ref,
            lut,
        }
    }

    pub(crate) fn capture(&self, density: [f64; 3]) -> [f64; 3] {
        if let Some(receiver) = &self.color.receiver {
            let mut rgb = receiver.capture_f64(density, self.channel_density, self.base_density);
            if self.color_ref.has_remap() {
                let to_y = receiver.luminance_weights();
                let y = (0..3).map(|c| to_y[c] * rgb[c]).sum();
                let scale = self.color_ref.xyz_scale(y);
                for value in &mut rgb {
                    *value *= scale;
                }
            }
            return rgb;
        }
        let mut xyz = if let Some(lut) = &self.lut {
            let position: [f64; 3] =
                std::array::from_fn(|c| (density[c] - lut.minimum[c]) * lut.inverse_step[c]);
            pchip_interp(&lut.pchip, position[0], position[1], position[2]).map(|v| 10.0f64.powf(v))
        } else {
            let mut xyz = [0.0; 3];
            for i in 0..self.color.n_wl {
                let dye = self.channel_density[i];
                let d = density[0] * dye[0]
                    + density[1] * dye[1]
                    + density[2] * dye[2]
                    + if self.base_density.len() >= self.color.n_wl {
                        self.base_density[i]
                    } else {
                        0.0
                    };
                let light =
                    if d.is_nan() { 0.0 } else { 10.0f64.powf(-d) } * self.color.illuminant[i];
                xyz[0] += light * spektrafilm_math::spectral::CMF_X_F64[i];
                xyz[1] += light * spektrafilm_math::spectral::CMF_Y_F64[i];
                xyz[2] += light * spektrafilm_math::spectral::CMF_Z_F64[i];
            }
            xyz.map(|v| 10.0f64.powf(((v / self.color.normalization).max(0.0) + 1e-10).log10()))
        };
        let scale = self.color_ref.xyz_scale(xyz[1]);
        if scale != 1.0 {
            for value in &mut xyz {
                *value *= scale;
            }
        }
        let adapted: [f64; 3] = std::array::from_fn(|c| {
            self.color.adapt[c][0] * xyz[0]
                + self.color.adapt[c][1] * xyz[1]
                + self.color.adapt[c][2] * xyz[2]
        });
        std::array::from_fn(|c| {
            self.color.base_xyz_to_rgb[c][0] * adapted[0]
                + self.color.base_xyz_to_rgb[c][1] * adapted[1]
                + self.color.base_xyz_to_rgb[c][2] * adapted[2]
        })
    }
}
