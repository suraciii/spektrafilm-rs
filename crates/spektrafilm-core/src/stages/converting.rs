//! Inverse scan stage for the experimental convert-film workflow.
//!
//! This is the CPU counterpart of the upstream bounded Gauss-Newton solver:
//! scanned RGB is mapped back to film CMY density using the same spectral scan
//! model used by `scanning.rs`.

use rayon::prelude::*;
use spektrafilm_math::{
    colorspace,
    image::ImageBuf,
    precision::{from_f64, to_f64},
    spectral,
};

use crate::{params::RuntimeParams, profile::Profile, spectral_service::select_illuminant_f64};

const LN10: f64 = std::f64::consts::LN_10;

fn parse_calibration(text: &str) -> Result<[[f64; 3]; 3], String> {
    let mut values = Vec::new();
    for part in text
        .split(|c: char| c.is_ascii_whitespace() || matches!(c, ',' | ';' | '[' | ']' | '(' | ')'))
        .filter(|part| !part.is_empty())
    {
        let value = part
            .parse::<f64>()
            .map_err(|_| format!("convert calibration contains a non-numeric token {part:?}"))?;
        if !value.is_finite() {
            return Err("convert calibration must contain only finite numbers".into());
        }
        values.push(value);
    }
    if values.len() != 9 {
        return Err(format!(
            "convert calibration must contain exactly 9 numbers, got {}",
            values.len()
        ));
    }
    Ok([
        [values[0], values[1], values[2]],
        [values[3], values[4], values[5]],
        [values[6], values[7], values[8]],
    ])
}

fn solve3(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> [f64; 3] {
    for column in 0..3 {
        let mut pivot = column;
        for row in (column + 1)..3 {
            if a[row][column].abs() > a[pivot][column].abs() {
                pivot = row;
            }
        }
        if a[pivot][column].abs() < 1e-14 {
            return [0.0; 3];
        }
        if pivot != column {
            a.swap(column, pivot);
            b.swap(column, pivot);
        }
        for row in (column + 1)..3 {
            let factor = a[row][column] / a[column][column];
            for k in column..3 {
                a[row][k] -= factor * a[column][k];
            }
            b[row] -= factor * b[column];
        }
    }
    let mut x = [0.0; 3];
    for row in (0..3).rev() {
        let tail = (row + 1..3).map(|k| a[row][k] * x[k]).sum::<f64>();
        x[row] = (b[row] - tail) / a[row][row];
    }
    x
}

struct Converter {
    dye: Vec<[f64; 3]>,
    base: Vec<f64>,
    illuminant: Vec<f64>,
    cmfs: Vec<[f64; 3]>,
    normalization: f64,
    xyz_to_rgb: [[f64; 3]; 3],
    cmy_max: [f64; 3],
    calibration: [[f64; 3]; 3],
    gain: f64,
    seed_log_rgb: [f64; 3],
    seed_inverse: nalgebra::Matrix3<f64>,
}

impl Converter {
    fn new(profile: &Profile, params: &RuntimeParams) -> Result<Self, String> {
        let illuminant =
            select_illuminant_f64(&params.film_render.convert.scan_illuminant).into_owned();
        let n = illuminant
            .len()
            .min(profile.data.channel_density.len())
            .min(spectral::N_WAVELENGTHS);
        let dye: Vec<[f64; 3]> = profile.data.channel_density[..n]
            .iter()
            .map(|row| {
                [
                    row.get(0).copied().unwrap_or(0.0),
                    row.get(1).copied().unwrap_or(0.0),
                    row.get(2).copied().unwrap_or(0.0),
                ]
            })
            .collect();
        let base: Vec<f64> =
            profile.data.base_density[..n.min(profile.data.base_density.len())].to_vec();
        let mut cmfs = Vec::with_capacity(n);
        for i in 0..n {
            cmfs.push([
                spectral::CMF_X_F64[i],
                spectral::CMF_Y_F64[i],
                spectral::CMF_Z_F64[i],
            ]);
        }
        let normalization = (0..n).map(|i| illuminant[i] * cmfs[i][1]).sum::<f64>();
        let mut illum_xyz = [0.0; 3];
        for i in 0..n {
            for k in 0..3 {
                illum_xyz[k] += illuminant[i] * cmfs[i][k];
            }
        }
        for value in &mut illum_xyz {
            *value /= normalization;
        }
        let sum = illum_xyz.iter().sum::<f64>();
        let scan_white = [
            illum_xyz[0] / sum / (illum_xyz[1] / sum),
            1.0,
            illum_xyz[2] / sum / (illum_xyz[1] / sum),
        ];
        let input_space = colorspace::resolve(&params.io.input_color_space)
            .expect("validated input colour space");
        let cat =
            colorspace::chromatic_adaptation_matrix_f64(scan_white, input_space.whitepoint_xyz());
        let mut xyz_to_rgb = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                for k in 0..3 {
                    xyz_to_rgb[i][j] += input_space.matrix_xyz_to_rgb[i][k] * cat[k][j];
                }
            }
        }
        let mut cmy_max = [1.0; 3];
        for channel in 0..3 {
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for row in &profile.data.density_curves {
                if let Some(value) = row.get(channel).copied().filter(|v| v.is_finite()) {
                    lo = lo.min(value);
                    hi = hi.max(value);
                }
            }
            if lo.is_finite() && hi.is_finite() {
                cmy_max[channel] = (hi - lo).max(1e-6);
            }
        }
        let mut converter = Self {
            dye,
            base,
            illuminant,
            cmfs,
            normalization,
            xyz_to_rgb,
            cmy_max,
            calibration: parse_calibration(&params.film_render.convert.calibration)?,
            gain: 2.0f64.powf(params.film_render.convert.exposure_compensation_ev),
            seed_log_rgb: [0.0; 3],
            seed_inverse: nalgebra::Matrix3::zeros(),
        };
        let (rgb, jacobian) = converter.forward_jacobian(cmy_max.map(|v| v * 0.5));
        converter.seed_log_rgb = rgb.map(|v| v.max(1e-12).log10());
        let gradient = nalgebra::Matrix3::from_fn(|row, column| {
            -jacobian[row][column] / (rgb[row].max(1e-12) * LN10)
        });
        let svd = gradient.svd(true, true);
        let cutoff = svd.singular_values.max() * 1e-15;
        converter.seed_inverse = svd
            .pseudo_inverse(cutoff)
            .map_err(|error| format!("Cannot initialize film conversion seed: {error}"))?;
        Ok(converter)
    }

    fn forward_jacobian(&self, cmy: [f64; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
        let mut xyz = [0.0; 3];
        let mut dxyz = [[0.0; 3]; 3];
        for i in 0..self.dye.len() {
            let base = self.base.get(i).copied().unwrap_or(0.0);
            let density =
                base + cmy[0] * self.dye[i][0] + cmy[1] * self.dye[i][1] + cmy[2] * self.dye[i][2];
            let light = self.illuminant[i] * 10.0f64.powf(-density);
            if !light.is_finite() {
                continue;
            }
            for k in 0..3 {
                xyz[k] += light * self.cmfs[i][k];
                for channel in 0..3 {
                    dxyz[channel][k] -= LN10 * light * self.dye[i][channel] * self.cmfs[i][k];
                }
            }
        }
        for k in 0..3 {
            xyz[k] /= self.normalization;
            for channel in 0..3 {
                dxyz[channel][k] /= self.normalization;
            }
        }
        let mut rgb = [0.0; 3];
        let mut jac = [[0.0; 3]; 3];
        for i in 0..3 {
            for k in 0..3 {
                rgb[i] += self.xyz_to_rgb[i][k] * xyz[k];
                for channel in 0..3 {
                    jac[i][channel] += self.xyz_to_rgb[i][k] * dxyz[channel][k];
                }
            }
        }
        (rgb, jac)
    }

    fn invert(&self, source: [f64; 3]) -> [f64; 3] {
        let mut target = [0.0; 3];
        for j in 0..3 {
            target[j] = (source[0] * self.calibration[0][j]
                + source[1] * self.calibration[1][j]
                + source[2] * self.calibration[2][j])
                * self.gain;
        }
        let log_delta =
            nalgebra::Vector3::from_fn(|i, _| self.seed_log_rgb[i] - target[i].max(1e-12).log10());
        let offset = self.seed_inverse * log_delta;
        let mut cmy = std::array::from_fn(|i| {
            (self.cmy_max[i] * 0.5 + offset[i]).clamp(0.0, self.cmy_max[i])
        });
        for _ in 0..8 {
            let (rgb, jac) = self.forward_jacobian(cmy);
            let residual = [rgb[0] - target[0], rgb[1] - target[1], rgb[2] - target[2]];
            if residual.iter().map(|v| v.abs()).fold(0.0, f64::max) < 1e-6 {
                break;
            }
            let mut jtj = [[0.0; 3]; 3];
            let mut jtr = [0.0; 3];
            for row in 0..3 {
                for col in 0..3 {
                    jtj[col][row] = (0..3).map(|k| jac[k][col] * jac[k][row]).sum();
                    jtr[col] += jac[row][col] * residual[row];
                }
            }
            for i in 0..3 {
                jtj[i][i] += 1e-7;
            }
            let delta = solve3(jtj, jtr);
            for i in 0..3 {
                cmy[i] = (cmy[i] - delta[i]).clamp(0.0, self.cmy_max[i]);
            }
        }
        cmy
    }
}

pub fn process(
    image: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
) -> Result<ImageBuf, String> {
    let converter = Converter::new(profile, params)?;
    let mut output = ImageBuf::new(image.width, image.height);
    output
        .data
        .par_chunks_exact_mut(3)
        .zip(image.data.par_chunks_exact(3))
        .for_each(|(dst, src)| {
            let mut rgb = [to_f64(src[0]), to_f64(src[1]), to_f64(src[2])];
            if params.io.input_cctf_decoding {
                let space = colorspace::resolve(&params.io.input_color_space)
                    .expect("validated input colour space");
                rgb = rgb.map(|v| colorspace::cctf_decode(v, space.cctf));
            }
            let cmy = converter.invert(rgb);
            dst[0] = from_f64(cmy[0]);
            dst[1] = from_f64(cmy[1]);
            dst[2] = from_f64(cmy[2]);
        });
    Ok(output)
}

/// Damped least squares shared by the small calibration problems.
fn fit<const N: usize>(
    mut x: [f64; N],
    residual: impl Fn(&[f64; N]) -> Result<Vec<f64>, String>,
) -> Result<[f64; N], String> {
    let mut r = residual(&x)?;
    let cost = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
    if r.is_empty() || r.iter().any(|v| !v.is_finite()) {
        return Err("Calibration has no finite residuals".into());
    }
    let mut damping = 1e-3;
    for _ in 0..60 {
        let mut jac = Vec::with_capacity(N);
        for column in 0..N {
            let mut trial = x;
            let step = 1e-4 * (1.0 + x[column].abs());
            trial[column] += step;
            let next = residual(&trial)?;
            jac.push(
                next.iter()
                    .zip(&r)
                    .map(|(a, b)| (a - b) / step)
                    .collect::<Vec<_>>(),
            );
        }
        let mut a = [[0.0; N]; N];
        let mut b = [0.0; N];
        for i in 0..N {
            b[i] = -jac[i].iter().zip(&r).map(|(j, r)| j * r).sum::<f64>();
            for j in 0..N {
                a[i][j] = jac[i].iter().zip(&jac[j]).map(|(a, b)| a * b).sum();
            }
            a[i][i] += damping;
        }
        for k in 0..N {
            let pivot = (k..N)
                .max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))
                .unwrap();
            a.swap(k, pivot);
            b.swap(k, pivot);
            if a[k][k].abs() < 1e-18 {
                return Err("Singular calibration fit".into());
            }
            for i in k + 1..N {
                let ratio = a[i][k] / a[k][k];
                for j in k..N {
                    a[i][j] -= ratio * a[k][j];
                }
                b[i] -= ratio * b[k];
            }
        }
        let mut delta = [0.0; N];
        for i in (0..N).rev() {
            delta[i] = (b[i] - (i + 1..N).map(|j| a[i][j] * delta[j]).sum::<f64>()) / a[i][i];
        }
        let trial = std::array::from_fn(|i| x[i] + delta[i]);
        let next = residual(&trial)?;
        if next.iter().all(|v| v.is_finite()) && cost(&next) < cost(&r) {
            let improvement = cost(&r) - cost(&next);
            x = trial;
            r = next;
            damping = (damping * 0.3).max(1e-9);
            if improvement < 1e-12 || delta.iter().all(|v| v.abs() < 1e-7) {
                break;
            }
        } else {
            damping *= 10.0;
            if damping > 1e12 {
                break;
            }
        }
    }
    if x.iter().any(|v| !v.is_finite()) {
        return Err("Non-finite calibration result".into());
    }
    Ok(x)
}

fn linear_pixels(image: &ImageBuf, params: &RuntimeParams) -> Result<Vec<[f64; 3]>, String> {
    let space = colorspace::resolve(&params.io.input_color_space).map_err(|e| e.to_string())?;
    let pixels: Vec<_> = image
        .data
        .chunks_exact(3)
        .map(|p| {
            let rgb = [to_f64(p[0]), to_f64(p[1]), to_f64(p[2])];
            if params.io.input_cctf_decoding {
                rgb.map(|v| colorspace::cctf_decode(v, space.cctf))
            } else {
                rgb
            }
        })
        .filter(|p| p.iter().all(|v| v.is_finite()))
        .collect();
    if pixels.is_empty() {
        return Err("Image contains no finite RGB pixels".into());
    }
    Ok(pixels)
}

fn bright_mean(pixels: &[[f64; 3]], percentile: f64) -> [f64; 3] {
    let luminance = |p: &[f64; 3]| p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722;
    let mut levels: Vec<_> = pixels.iter().map(luminance).collect();
    levels.sort_by(f64::total_cmp);
    let position = percentile.clamp(0.0, 100.0) / 100.0 * (levels.len() - 1) as f64;
    let lo = position.floor() as usize;
    let hi = position.ceil() as usize;
    let threshold = levels[lo] + (levels[hi] - levels[lo]) * (position - lo as f64);
    let mut mean = [0.0; 3];
    let mut count = 0;
    for p in pixels.iter().filter(|p| luminance(p) >= threshold) {
        count += 1;
        for i in 0..3 {
            mean[i] += p[i];
        }
    }
    mean.map(|v| v / count as f64)
}

/// Fit clear-film CMY base tuning and the scan exposure, using a native (untuned)
/// profile. The image is decoded in the selected input colour space.
pub fn detect_base(
    image: &ImageBuf,
    native: &Profile,
    params: &RuntimeParams,
) -> Result<(crate::params::FilmBaseParams, f64), String> {
    params.validate()?;
    let target = bright_mean(
        &linear_pixels(image, params)?,
        params.film_render.convert.base_percentile,
    );
    let denom = target.iter().map(|v| v * v).sum::<f64>();
    if denom < 1e-12 {
        return Err("Clear-film sample is black".into());
    }
    let forward = |x: &[f64; 3]| {
        let mut base = params.film_render.base.clone();
        base.active = true;
        base.cyan = x[0];
        base.magenta = x[1];
        base.yellow = x[2];
        let mut profile = native.clone();
        crate::pipeline::apply_base_tuning(&mut profile, &base, None);
        Converter::new(&profile, params)
            .expect("validated conversion parameters")
            .forward_jacobian([0.0; 3])
            .0
    };
    let x = fit(
        [
            params.film_render.base.cyan,
            params.film_render.base.magenta,
            params.film_render.base.yellow,
        ],
        |x| {
            let f = forward(x);
            let gain = (0..3).map(|i| f[i] * target[i]).sum::<f64>() / denom;
            Ok((0..3)
                .map(|i| f[i] - gain * target[i])
                .chain(x.iter().map(|v| 0.02 * (v - 1.0)))
                .collect())
        },
    )?;
    let f = forward(&x);
    let gain = (0..3).map(|i| f[i] * target[i]).sum::<f64>() / denom;
    if !gain.is_finite() || gain <= 0.0 {
        return Err("Clear-film sample cannot be fitted".into());
    }
    let mut base = params.film_render.base.clone();
    base.active = true;
    base.cyan = x[0];
    base.magenta = x[1];
    base.yellow = x[2];
    Ok((base, gain.log2()))
}

/// Fit a device correction against the bounded spectral dye gamut. Pass the
/// resolved, base-tuned film profile from the current pipeline.
pub fn blind_calibration(
    image: &ImageBuf,
    film: &Profile,
    params: &RuntimeParams,
) -> Result<String, String> {
    params.validate()?;
    let pixels = linear_pixels(image, params)?;
    let count = pixels.len().min(1500);
    let samples: Vec<_> = (0..count)
        .map(|i| pixels[i * (pixels.len() - 1) / (count - 1).max(1)].map(|v| v.max(0.0)))
        .collect();
    let base = bright_mean(&samples, 99.0);
    let mut converter = Converter::new(film, params)?;
    let gain = converter.gain;
    converter.gain = 1.0;
    converter.calibration = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let truth = converter.forward_jacobian([0.; 3]).0;
    let eye = [1., 0., 0., 0., 1., 0., 0., 0., 1.];
    let matrix = fit(eye, |x| {
        let correct = |p: [f64; 3]| {
            let mut out = [0.0; 3];
            for j in 0..3 {
                out[j] = (0..3).map(|i| p[i] * x[i * 3 + j]).sum::<f64>() * gain;
            }
            out
        };
        let mut r = Vec::with_capacity(samples.len() * 3 + 12);
        for &p in &samples {
            let corrected = correct(p);
            let cmy = converter.invert(corrected.map(|v| v.max(0.0)));
            let predicted = converter.forward_jacobian(cmy).0;
            for i in 0..3 {
                r.push(corrected[i] - predicted[i]);
            }
        }
        let anchor = correct(base);
        for i in 0..3 {
            r.push(5.0 * (anchor[i] - truth[i]));
        }
        for i in 0..9 {
            r.push(0.03 * (x[i] - eye[i]));
        }
        Ok(r)
    })?;
    Ok(matrix
        .iter()
        .map(|v| format!("{v:.5}"))
        .collect::<Vec<_>>()
        .join(" "))
}

/// Neutralize the actual film → enlarger → print → scanner path, holding C fixed.
pub fn neutralize_filters(
    film: &Profile,
    print: &Profile,
    params: &RuntimeParams,
    data: &std::path::Path,
    backend: &dyn spektrafilm_gpu::ComputeBackend,
) -> Result<(f32, f32), String> {
    let mut p = params.clone();
    p.workflow.route = "input > film > print > scan".into();
    p.io.scan_film = false;
    p.scanner.scan_output = "direct_scan".into();
    p.io.output_cctf_encoding = false;
    p.io.output_gamut_compress.algorithm = "off".into();
    p.settings.use_enlarger_lut = false;
    p.settings.use_scanner_lut = false;
    p.debug.lut_mode = true;
    let residual = |x: &[f64; 2]| {
        let mut trial = p.clone();
        trial.enlarger.m_filter_shift = x[0] as f32;
        trial.enlarger.y_filter_shift = x[1] as f32;
        let pipeline =
            crate::pipeline::Pipeline::new_with_spectral(film.clone(), print.clone(), trial, data)?;
        let output = pipeline.process(
            ImageBuf::from_data(2, 2, vec![from_f64(0.184); 12]),
            backend,
        )?;
        let mut rgb = [0.; 3];
        for pixel in output.data.chunks_exact(3) {
            for i in 0..3 {
                rgb[i] += to_f64(pixel[i]) / 4.0;
            }
        }
        let g = rgb[1].max(1e-6);
        Ok(vec![(rgb[0] - rgb[1]) / g, (rgb[2] - rgb[1]) / g])
    };
    let x = fit(
        [
            p.enlarger.m_filter_shift as f64,
            p.enlarger.y_filter_shift as f64,
        ],
        &residual,
    )?;
    if residual(&x)?.iter().any(|v| v.abs() > 0.01) {
        return Err("Print balance did not converge to neutral RGB".into());
    }
    Ok((x[0] as f32, x[1] as f32))
}

#[cfg(test)]
mod action_tests {
    use super::*;
    #[test]
    fn print_neutralization_is_independent_of_positive_scan_mode() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let film = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.camera.auto_exposure = false;
        params.film_render.grain.active = false;
        params.film_render.halation.active = false;
        params.film_render.dir_couplers.active = false;
        params.print_render.glare.active = false;
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let direct = neutralize_filters(&film, &print, &params, &data, &backend).unwrap();
        params.scanner.scan_output = "positive_scan".into();
        let positive = neutralize_filters(&film, &print, &params, &data, &backend).unwrap();
        assert_eq!(positive, direct);
        assert_eq!(params.workflow.route, "input > film > scan");
        assert_eq!(params.scanner.scan_output, "positive_scan");
    }

    #[test]
    fn bright_sample_ignores_nonfinite_pixels_and_uses_clear_tail() {
        let image = ImageBuf::from_data(
            3,
            1,
            [0.1, 0.2, 0.1, 0.8, 0.7, 0.6, f64::NAN, 0.0, 0.0]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        let mut p = RuntimeParams::default();
        p.io.input_cctf_decoding = false;
        let clear = bright_mean(&linear_pixels(&image, &p).unwrap(), 99.0);
        for (actual, expected) in clear.into_iter().zip([0.8, 0.7, 0.6]) {
            assert!((actual - expected).abs() < 1e-6);
        }
    }
    #[test]
    fn finite_least_squares_recovers_coupled_solution() {
        let x = fit([0.0, 0.0], |x| {
            Ok(vec![x[0] + 2.0 * x[1] - 5.0, 3.0 * x[0] - x[1] - 1.0])
        })
        .unwrap();
        assert!((x[0] - 1.0).abs() < 1e-6);
        assert!((x[1] - 2.0).abs() < 1e-6);
    }
    #[test]
    fn empty_finite_sample_is_an_error() {
        assert!(
            linear_pixels(
                &ImageBuf::from_data(1, 1, vec![from_f64(f64::NAN); 3]),
                &RuntimeParams::default()
            )
            .is_err()
        );
    }
    #[test]
    fn malformed_calibration_is_rejected() {
        assert!(parse_calibration("1 0 nope 0 1 0 0 0 1").is_err());
        assert!(parse_calibration("1 0 0 0 1 0 0 0").is_err());
        assert!(parse_calibration("1 0 0 0 1 0 0 0 1").is_ok());
    }
}
