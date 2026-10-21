//! Project-authored, fixed spectral Reference Exposure conversion (CPU f64).
use crate::{params::RuntimeParams, profile::Profile};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spektrafilm_math::{
    colorspace,
    image::ImageBuf,
    precision::{from_f64, to_f64},
    spectral::N_WAVELENGTHS,
};
use std::path::Path;

pub const VERSION: &str = "spektrafilm_digital_reference_v1";
pub const RECEIVER_ASSET: &str = "kodak_portra_endura";
pub const BASIS_ASSET: &str = "kodak_portra_400";
pub const REFERENCE_LOG_EXPOSURE: f64 = 0.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceProvenance {
    pub version: String,
    pub illumination: String,
    pub receiver_asset: String,
    pub receiver_content_identity: String,
    pub dye_basis_asset: String,
    pub dye_basis_content_identity: String,
    pub reference_log_exposure: f64,
    pub middle_gray: f64,
    pub response: String,
    pub working_color_space: String,
    pub asset_license: String,
    pub transform_license: String,
    pub clear_log_readings: [f64; 3],
    pub reference_coordinates: [f64; 3],
    pub monochrome: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DigitalReference {
    log_weights: Vec<[f64; 3]>,
    dyes: Vec<[f64; 3]>,
    base: Vec<f64>,
    inverse: [[f64; 3]; 3],
    clear: [f64; 3],
    anchor: [f64; 3],
    monochrome: bool,
    pub provenance: ReferenceProvenance,
}

fn load_asset(data_dir: &Path, stock: &str) -> Result<(Profile, String), String> {
    let path = data_dir.join("profiles").join(format!("{stock}.json"));
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("Digital Reference asset {}: {e}", path.display()))?;
    let identity = format!("sha256-{:x}", Sha256::digest(&bytes));
    let profile = crate::profile::load_profile_bytes(&bytes, &path.display().to_string())
        .map_err(|e| format!("Digital Reference asset {stock}: {e}"))?;
    Ok((profile, identity))
}

fn transform(matrix: [[f64; 3]; 3], value: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|c| (0..3).map(|k| matrix[c][k] * value[k]).sum())
}

impl DigitalReference {
    pub fn prepare(film: &Profile, data_dir: &Path) -> Result<Self, String> {
        let (receiver, receiver_identity) = load_asset(data_dir, RECEIVER_ASSET)?;
        let (basis, basis_identity) = load_asset(data_dir, BASIS_ASSET)?;
        let basis_dyes = crate::chain_prep::channel_density(&basis);
        let dyes = crate::chain_prep::channel_density(film);
        if receiver.data.log_sensitivity.len() != N_WAVELENGTHS
            || basis_dyes.len() != N_WAVELENGTHS
            || dyes.len() != N_WAVELENGTHS
            || film.data.base_density.len() != N_WAVELENGTHS
        {
            return Err("Digital Reference requires complete 380..780 nm / 5 nm receiver, dye and base spectra".into());
        }
        for profile in [&receiver, &basis, film] {
            if profile.data.wavelengths.len() != N_WAVELENGTHS
                || profile
                    .data
                    .wavelengths
                    .iter()
                    .enumerate()
                    .any(|(i, w)| *w != 380.0 + 5.0 * i as f64)
            {
                return Err(
                    "Digital Reference requires the exact bundled 380..780 nm / 5 nm spectral grid"
                        .into(),
                );
            }
        }
        let mut weights = receiver.sensitivity_f64();
        let sums: [f64; 3] = std::array::from_fn(|c| weights.iter().map(|w| w[c]).sum());
        if sums.iter().any(|s| !s.is_finite() || *s <= 0.0)
            || dyes
                .iter()
                .chain(&basis_dyes)
                .any(|row| row.iter().any(|v| v.is_infinite()))
            || film.data.base_density.iter().any(|v| v.is_infinite())
        {
            return Err("Digital Reference requires finite spectral assets with positive receiver integrals".into());
        }
        for row in &mut weights {
            for c in 0..3 {
                row[c] /= sums[c];
            }
        }
        let jacobian: [[f64; 3]; 3] = std::array::from_fn(|c| {
            std::array::from_fn(|k| {
                weights
                    .iter()
                    .zip(&basis_dyes)
                    .map(|(w, d)| {
                        if d.iter().any(|v| v.is_nan()) {
                            0.0
                        } else {
                            w[c] * d[k]
                        }
                    })
                    .sum()
            })
        });
        let flattened: [f64; 9] = std::array::from_fn(|i| jacobian[i / 3][i % 3]);
        let matrix = nalgebra::Matrix3::from_row_slice(&flattened);
        let inverse_matrix = matrix
            .try_inverse()
            .ok_or("Digital Reference dye Jacobian is singular")?;
        let condition = matrix.norm() * inverse_matrix.norm();
        if !condition.is_finite() || condition > 1000.0 {
            return Err(
                "Digital Reference dye Jacobian is ill-conditioned (Frobenius condition > 1000)"
                    .into(),
            );
        }
        let inverse = std::array::from_fn(|c| std::array::from_fn(|k| inverse_matrix[(c, k)]));
        let curves = crate::chain_prep::FilmCurves::prepare(film);
        let axis = curves.log_exposure;
        if axis.len() < 2
            || axis.len() != curves.normalized.len()
            || axis.iter().any(|e| !e.is_finite())
            || axis.windows(2).any(|w| w[0] >= w[1])
            || film
                .data
                .density_curves
                .iter()
                .any(|row| row.len() != 3 || row.iter().any(|v| !v.is_finite()))
            || axis[0] > REFERENCE_LOG_EXPOSURE
            || axis[axis.len() - 1] < REFERENCE_LOG_EXPOSURE
        {
            return Err("Digital Reference requires characteristic curves covering declared log10 exposure 0".into());
        }
        let upper = axis
            .partition_point(|e| *e < REFERENCE_LOG_EXPOSURE)
            .clamp(1, axis.len() - 1);
        let lower = upper - 1;
        let t = (REFERENCE_LOG_EXPOSURE - axis[lower]) / (axis[upper] - axis[lower]);
        let reference = std::array::from_fn(|c| {
            curves.normalized[lower][c]
                + t * (curves.normalized[upper][c] - curves.normalized[lower][c])
        });
        let mut prepared = Self {
            log_weights: weights.iter().map(|row| row.map(f64::log10)).collect(),
            dyes,
            base: film.data.base_density.clone(),
            inverse,
            clear: [0.0; 3],
            anchor: [1.0; 3],
            monochrome: film.is_bw(),
            provenance: ReferenceProvenance {
                version: VERSION.into(),
                illumination: "equal_energy_380_780_5nm".into(),
                receiver_asset: RECEIVER_ASSET.into(),
                receiver_content_identity: receiver_identity,
                dye_basis_asset: BASIS_ASSET.into(),
                dye_basis_content_identity: basis_identity,
                reference_log_exposure: REFERENCE_LOG_EXPOSURE,
                middle_gray: 0.18,
                response: "sign(x)*x^2/(x^2+41/9); clear=0; reference=0.18; limits=+/-1".into(),
                working_color_space: "linear sRGB".into(),
                asset_license: "CC BY-SA 4.0; Andrea Volpato; repository data/license".into(),
                transform_license: "SpektraFilm project license".into(),
                clear_log_readings: [0.0; 3],
                reference_coordinates: [0.0; 3],
                monochrome: film.is_bw(),
            },
        };
        prepared.clear = prepared.capture([0.0; 3]);
        prepared.anchor = prepared.coordinates(prepared.capture(reference));
        if prepared.clear.iter().any(|v| !v.is_finite())
            || prepared.anchor.iter().any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("Digital Reference requires positive finite clear readings and reference coordinates".into());
        }
        prepared.provenance.clear_log_readings = prepared.clear;
        prepared.provenance.reference_coordinates = prepared.anchor;
        Ok(prepared)
    }

    /// Base-10 log-sum-exp avoids multiplying extreme densities by ln(10).
    fn capture(&self, density: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|c| {
            let mut exponents = [f64::NEG_INFINITY; N_WAVELENGTHS];
            for (i, exponent) in exponents.iter_mut().enumerate() {
                if !self.base[i].is_nan() && !self.dyes[i].iter().any(|v| v.is_nan()) {
                    *exponent = self.log_weights[i][c]
                        - (self.base[i]
                            + (0..3).map(|k| density[k] * self.dyes[i][k]).sum::<f64>());
                }
            }
            let maximum = exponents.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            maximum
                + exponents
                    .iter()
                    .map(|e| 10.0f64.powf(e - maximum))
                    .sum::<f64>()
                    .log10()
        })
    }

    fn coordinates(&self, capture: [f64; 3]) -> [f64; 3] {
        let optical = std::array::from_fn(|c| self.clear[c] - capture[c]);
        if self.monochrome {
            [optical.iter().map(|v| v / 3.0).sum::<f64>(); 3]
        } else {
            transform(self.inverse, optical)
        }
    }

    fn tone(x: f64) -> f64 {
        let a = x.abs();
        let ratio = if a > 1.0 {
            1.0 / (1.0 + (41.0 / 9.0) / a / a)
        } else {
            let square = a * a;
            square / (square + 41.0 / 9.0)
        };
        ratio.copysign(x)
    }

    pub fn convert(
        &self,
        density: &ImageBuf,
        params: &RuntimeParams,
        backend: &dyn spektrafilm_backend::ComputeBackend,
    ) -> Result<ImageBuf, String> {
        if density.data.iter().any(|v| !v.is_finite()) {
            return Err("Digital Reference requires finite developed densities".into());
        }
        // Capture is optical, so lens blur precedes positive conversion. Capture values
        // are relative transmissions; logarithmic integration is retained for unblurred extremes.
        let mut output = density.clone();
        if params.scanner.lens_blur > 0.0 {
            output.data.par_chunks_exact_mut(3).try_for_each(|px| -> Result<(), String> {
                let capture = self.capture([to_f64(px[0]),to_f64(px[1]),to_f64(px[2])]);
                if capture.iter().any(|v| !v.is_finite()) { return Err("Digital Reference optical capture is nonfinite".into()); }
                for c in 0..3 { px[c]=from_f64(10.0f64.powf(capture[c]-self.clear[c])); }
                if px.iter().any(|v| !v.is_finite() || *v <= from_f64(0.0)) {
                    return Err("Digital Reference optical capture is not positive and representable for lens blur".into());
                }
                Ok(())
            })?;
            output = backend.gaussian_blur(&output, params.scanner.lens_blur);
            if output
                .data
                .iter()
                .any(|v| !v.is_finite() || *v <= from_f64(0.0))
            {
                return Err(
                    "Digital Reference blurred optical capture is not positive and representable"
                        .into(),
                );
            }
        }
        let destination =
            colorspace::resolve(&params.io.output_color_space).expect("validated output space");
        let to_output = if destination.name == colorspace::SRGB.name {
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        } else {
            colorspace::conversion_matrix(&colorspace::SRGB, destination)
        };
        let gains: [f64; 3] = std::array::from_fn(|c| {
            2.0f64
                .powf(params.scanner.reference_exposure_ev + params.scanner.reference_balance_ev[c])
        });
        output
            .data
            .par_chunks_exact_mut(3)
            .try_for_each(|px| -> Result<(), String> {
                let capture = if params.scanner.lens_blur > 0.0 {
                    std::array::from_fn(|c| self.clear[c] + to_f64(px[c]).log10())
                } else {
                    self.capture([to_f64(px[0]), to_f64(px[1]), to_f64(px[2])])
                };
                let coordinates = self.coordinates(capture);
                if capture.iter().any(|v| !v.is_finite())
                    || coordinates.iter().any(|v| !v.is_finite())
                {
                    return Err(
                        "Digital Reference optical capture or separated coordinate is nonfinite"
                            .into(),
                    );
                }
                let rgb = transform(
                    to_output,
                    std::array::from_fn(|c| Self::tone(coordinates[c] / self.anchor[c]) * gains[c]),
                );
                for c in 0..3 {
                    px[c] = from_f64(rgb[c]);
                }
                Ok(())
            })?;
        if output.data.iter().any(|v| !v.is_finite()) {
            return Err(
                "Digital Reference capture or output is not finite in the native precision".into(),
            );
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{NegativeConversionMethod, ReceiverProfile};
    use crate::test_support::data_dir as data;
    fn fixture() -> (Profile, RuntimeParams, DigitalReference) {
        let film = crate::profile::load_profile_by_name(&data(), "kodak_portra_400").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.receiver_profile = ReceiverProfile::DigitalReference;
        params.scanner.negative_conversion_method = NegativeConversionMethod::DigitalReference;
        params.scanner.unsharp_mask = [0.0; 2];
        params.io.output_color_space = "sRGB".into();
        params.io.output_cctf_encoding = false;
        let reference = DigitalReference::prepare(&film, &data()).unwrap();
        (film, params, reference)
    }
    fn reference_density(film: &Profile) -> [f64; 3] {
        let curves = crate::chain_prep::FilmCurves::prepare(film);
        let upper = curves
            .log_exposure
            .partition_point(|e| *e < 0.0)
            .clamp(1, curves.log_exposure.len() - 1);
        let t = -curves.log_exposure[upper - 1]
            / (curves.log_exposure[upper] - curves.log_exposure[upper - 1]);
        std::array::from_fn(|c| {
            curves.normalized[upper - 1][c]
                + t * (curves.normalized[upper][c] - curves.normalized[upper - 1][c])
        })
    }
    fn pixel(reference: &DigitalReference, params: &RuntimeParams, density: [f64; 3]) -> [f64; 3] {
        let image = ImageBuf::from_data(1, 1, density.into_iter().map(from_f64).collect());
        let result = reference
            .convert(
                &image,
                params,
                &spektrafilm_backend::cpu_backend::CpuBackend,
            )
            .unwrap();
        std::array::from_fn(|c| to_f64(result.data[c]))
    }
    #[test]
    fn reference_clear_gray_and_continuous_finite_extremes() {
        let (film, params, reference) = fixture();
        assert_eq!(pixel(&reference, &params, [0.0; 3]), [0.0; 3]);
        for value in pixel(&reference, &params, reference_density(&film)) {
            assert!((value - 0.18).abs() < 2e-7);
        }
        for x in [-1e150, -100.0, -1.0, -1e-9, 0.0, 1e-9, 1.0, 100.0, 1e150] {
            assert!(DigitalReference::tone(x).is_finite());
            assert!((DigitalReference::tone(x + 1e-10) - DigitalReference::tone(x)).abs() < 1e-8);
        }
        for density in [-100.0, -0.01, 0.0, 0.01, 100.0, 1e6] {
            assert!(
                pixel(&reference, &params, [density; 3])
                    .iter()
                    .all(|v| v.is_finite())
            );
        }
    }
    #[test]
    fn monochrome_extremes_continue_without_blur_and_reject_unrepresentable_blur() {
        let (_, mut params, _) = fixture();
        let film = crate::profile::resolve_for_render(
            crate::profile::load_profile_by_name(&data(), "kodak_doublex").unwrap(),
            None,
        );
        let reference = DigitalReference::prepare(&film, &data()).unwrap();
        #[cfg(feature = "precision-f64")]
        let extreme = 1e308;
        #[cfg(not(feature = "precision-f64"))]
        let extreme = 1e30;
        assert_eq!(pixel(&reference, &params, [extreme; 3]), [1.0; 3]);
        assert_eq!(pixel(&reference, &params, [-extreme; 3]), [-1.0; 3]);
        params.scanner.lens_blur = 1.0;
        let image = ImageBuf::from_data(2, 2, vec![from_f64(1000.0); 12]);
        let error = reference
            .convert(
                &image,
                &params,
                &spektrafilm_backend::cpu_backend::CpuBackend,
            )
            .unwrap_err();
        assert!(error.contains("positive and representable for lens blur"));
    }

    #[test]
    fn common_response_retains_slope_and_spectral_dye_changes() {
        let (film, params, reference) = fixture();
        let neutral = reference_density(&film);
        let mut changed = film.clone();
        let minima: [f64; 3] = std::array::from_fn(|c| {
            changed
                .data
                .density_curves
                .iter()
                .map(|row| row[c])
                .fold(f64::INFINITY, f64::min)
        });
        // Change wedge slope while retaining zero base and the ONE reference reading.
        for row in &mut changed.data.density_curves {
            for c in 0..3 {
                row[c] = minima[c] + neutral[c] * ((row[c] - minima[c]) / neutral[c]).powf(1.5);
            }
        }
        let changed_reference = DigitalReference::prepare(&changed, &data()).unwrap();
        let lower = reference_density(&film).map(|v| v * 0.4);
        let changed_lower = std::array::from_fn(|c| neutral[c] * (lower[c] / neutral[c]).powf(1.5));
        let old = pixel(&reference, &params, lower);
        let new = pixel(&changed_reference, &params, changed_lower);
        assert!(old.iter().zip(new).any(|(a, b)| (a - b).abs() > 0.005));
        let mut dye_film = film.clone();
        for (i, row) in dye_film.data.channel_density.iter_mut().enumerate() {
            row[0] *= 1.0 + 0.3 * (i as f64 / 80.0);
        }
        let dye_reference = DigitalReference::prepare(&dye_film, &data()).unwrap();
        let chromatic = [neutral[0] * 1.5, neutral[1] * 0.6, neutral[2] * 0.9];
        let a = pixel(&reference, &params, chromatic);
        let b = pixel(&dye_reference, &params, chromatic);
        assert!(a.iter().zip(b).any(|(x, y)| (x - y).abs() > 1e-4));
    }
    #[test]
    fn patches_ignore_surrounding_histograms_and_manual_gain_has_declared_units() {
        let (_, mut params, reference) = fixture();
        let patch = [0.3, 0.7, 0.5];
        let image = |background: f64| {
            ImageBuf::from_data(
                3,
                1,
                [
                    background, background, background, patch[0], patch[1], patch[2], background,
                    background, background,
                ]
                .into_iter()
                .map(from_f64)
                .collect(),
            )
        };
        let cpu = spektrafilm_backend::cpu_backend::CpuBackend;
        let a = reference.convert(&image(0.01), &params, &cpu).unwrap();
        let b = reference.convert(&image(3.0), &params, &cpu).unwrap();
        assert_eq!(&a.data[3..6], &b.data[3..6]);
        let normal = pixel(&reference, &params, patch);
        params.scanner.reference_exposure_ev = 1.0;
        let gain = pixel(&reference, &params, patch);
        for c in 0..3 {
            assert!((gain[c] - 2.0 * normal[c]).abs() < 1e-7);
        }
        params.scanner.reference_exposure_ev = 0.0;
        params.scanner.reference_balance_ev = [1.0, -1.0, 0.0];
        let gain = pixel(&reference, &params, patch);
        for c in 0..3 {
            assert!((gain[c] - [2.0, 0.5, 1.0][c] * normal[c]).abs() < 1e-7);
        }
    }
    #[test]
    fn monochrome_is_neutral_and_nonfinite_density_is_rejected() {
        let (_, params, _) = fixture();
        let film = crate::profile::resolve_for_render(
            crate::profile::load_profile_by_name(&data(), "kodak_doublex").unwrap(),
            None,
        );
        let reference = DigitalReference::prepare(&film, &data()).unwrap();
        for density in [0.0, 0.1, 1.0, 3.0] {
            let rgb = pixel(&reference, &params, [density; 3]);
            assert_eq!(rgb[0], rgb[1]);
            assert_eq!(rgb[1], rgb[2]);
        }
        let bad = ImageBuf::from_data(1, 1, vec![from_f64(f64::NAN); 3]);
        assert!(
            reference
                .convert(&bad, &params, &spektrafilm_backend::cpu_backend::CpuBackend)
                .is_err()
        );
    }
    #[test]
    fn incompatible_routes_receivers_setup_points_and_controls_fail() {
        let (_, params, _) = fixture();
        assert!(params.validate().is_ok());
        let mut variants = Vec::new();
        let mut p = params.clone();
        p.workflow.route = "input > convert-film > scan".into();
        variants.push(p);
        let mut p = params.clone();
        p.scanner.receiver_profile = ReceiverProfile::LegacyColorimetric;
        variants.push(p);
        let mut p = params.clone();
        p.scanner.negative_conversion_method = NegativeConversionMethod::Density;
        variants.push(p);
        let mut p = params.clone();
        p.scanner.auto_setup = crate::params::ScannerAutoSetup::Full;
        variants.push(p);
        let mut p = params.clone();
        p.scanner.black_point = [0.1; 3];
        variants.push(p);
        let mut p = params.clone();
        p.scanner.conversion_brightness_ev = 0.0;
        variants.push(p);
        for p in variants {
            assert!(p.validate().is_err());
        }
        let mut plain = RuntimeParams::default();
        plain.scanner.reference_exposure_ev = 1.0;
        assert!(plain.validate().is_err());
    }
}
