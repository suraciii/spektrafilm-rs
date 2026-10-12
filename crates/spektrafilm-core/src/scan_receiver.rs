//! Scanner acquisition response, separate from film development and input conversion.
use crate::{
    chain_prep::ScanColorContext,
    params::{ReceiverProfile, RuntimeParams},
};
use rayon::prelude::*;
use serde::Serialize;
use spektrafilm_math::{
    colorspace,
    image::ImageBuf,
    precision::{from_f64, to_f64},
    spectral,
};

/// Complete scanner provenance retained by recipes and GUI state.
/// Receiver and finish metadata are derived from built-in implementations so
/// callers cannot claim a measured device profile by supplying strings.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ScannerProvenance {
    pub receiver: ReceiverProvenance,
    pub finish: &'static str,
    pub finish_source: &'static str,
    pub finish_version: &'static str,
    pub finish_license: &'static str,
}

impl ScannerProvenance {
    pub fn from_params(params: &RuntimeParams) -> Self {
        let receiver = provenance(params.scanner.receiver_profile);
        let (finish, finish_source, finish_version, finish_license) = match params.scanner.finish {
            crate::params::ScannerFinish::Neutral => (
                "neutral",
                "SpektraFilm built-in neutral scanner finish",
                "neutral_v1",
                "SpektraFilm",
            ),
            crate::params::ScannerFinish::LabScan => (
                "lab_scan",
                "FotufilmCore LabScanFinish.swift",
                "fotufilm_lab_scan_finish_fd235219",
                "Apache-2.0",
            ),
        };
        Self {
            receiver,
            finish,
            finish_source,
            finish_version,
            finish_license,
        }
    }
}

/// Versioned acquisition provenance. Lab scan bands are an approximation,
/// not a measured or calibrated scanner device profile.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ReceiverProvenance {
    pub profile: ReceiverProfile,
    pub source: &'static str,
    pub version: &'static str,
    pub calibration_status: &'static str,
    pub reference_levels: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band_centers_nm: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band_sigma_nm: Option<[f64; 3]>,
}

pub fn provenance(profile: ReceiverProfile) -> ReceiverProvenance {
    match profile {
        ReceiverProfile::LegacyColorimetric => ReceiverProvenance {
            profile,
            source: "SpektraFilm historical CIE 1931 spectral-to-output path",
            version: "legacy_colorimetric_v1",
            calibration_status: "legacy_colorimetric",
            reference_levels: [1.0; 3],
            band_centers_nm: None,
            band_sigma_nm: None,
        },
        ReceiverProfile::LabScanApprox => ReceiverProvenance {
            profile,
            source: "Fotufilm research snapshot 20261012, Sources/FotufilmCore/PrintPaperTables.swift:296-305 (labScanSensitivity)",
            version: "fotufilm_lab_scan_approx_v1",
            calibration_status: "approximate_uncalibrated",
            reference_levels: [1.0; 3],
            band_centers_nm: Some([630.0, 545.0, 465.0]),
            band_sigma_nm: Some([8.5, 14.0, 9.5]),
        },
    }
}

/// Prepared narrow-band capture weights. Bands follow the pinned Fotufilm
/// Gaussian LED approximation (630/545/465 nm, sigma 8.5/14/9.5 nm), evaluated
/// on our 380..780 nm / 5 nm grid. Each illuminated band sums to one, so a
/// zero-density, zero-base sample reads [1,1,1]. The three detector readings
/// are assigned to linear sRGB and transformed to the requested output space;
/// this display assignment is approximate and carries no measured device claim.
#[derive(Debug, Clone)]
pub(crate) struct ScanReceiver {
    weights: Vec<[f64; 3]>,
    to_output: [[f64; 3]; 3],
    output_to_y: [f64; 3],
}

impl ScanReceiver {
    pub(crate) fn resolve(color: &ScanColorContext, output: &str) -> Self {
        let centers = [630.0, 545.0, 465.0];
        let sigma = [8.5, 14.0, 9.5];
        let mut weights: Vec<[f64; 3]> = (0..color.n_wl)
            .map(|i| {
                let wavelength =
                    spectral::WAVELENGTH_MIN as f64 + i as f64 * spectral::WAVELENGTH_STEP as f64;
                std::array::from_fn(|c| {
                    let distance = (wavelength - centers[c]) / sigma[c];
                    color.illuminant[i] * (-0.5 * distance * distance).exp()
                })
            })
            .collect();
        let sum: [f64; 3] = std::array::from_fn(|c| weights.iter().map(|row| row[c]).sum());
        for row in &mut weights {
            for c in 0..3 {
                row[c] /= sum[c];
            }
        }
        let destination = colorspace::resolve(output).expect("validated output color space");
        let to_output = if destination.name == colorspace::SRGB.name {
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        } else {
            colorspace::conversion_matrix(&colorspace::SRGB, destination)
        };
        // Invert the actual acquisition matrix, rather than an independently
        // rounded output-space inverse or the historical scanner CAT.
        let [a, b, c] = to_output;
        let cofactors = [
            [
                b[1] * c[2] - b[2] * c[1],
                b[2] * c[0] - b[0] * c[2],
                b[0] * c[1] - b[1] * c[0],
            ],
            [
                a[2] * c[1] - a[1] * c[2],
                a[0] * c[2] - a[2] * c[0],
                a[1] * c[0] - a[0] * c[1],
            ],
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ],
        ];
        let determinant = a[0] * cofactors[0][0] + a[1] * cofactors[0][1] + a[2] * cofactors[0][2];
        let nominal_y = colorspace::SRGB.matrix_rgb_to_xyz[1];
        let output_to_y = std::array::from_fn(|i| {
            (0..3)
                .map(|k| nominal_y[k] * cofactors[i][k] / determinant)
                .sum()
        });
        Self {
            weights,
            to_output,
            output_to_y,
        }
    }

    pub(crate) fn luminance_weights(&self) -> [f64; 3] {
        self.output_to_y
    }

    /// Prepare slide scan correction from this receiver's own zero and maximum
    /// dye captures, leaving filming exposure calibration unchanged.
    pub(crate) fn correction(
        &self,
        profile: &crate::profile::Profile,
        params: &crate::params::RuntimeParams,
        original: &crate::color_reference::ColorReference,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
    ) -> crate::color_reference::ColorReference {
        if !original.has_remap() {
            return *original;
        }
        let mut maximum = [f64::NEG_INFINITY; 3];
        for row in profile.density_curves_f64() {
            for c in 0..3 {
                if row[c].is_finite() {
                    maximum[c] = maximum[c].max(row[c]);
                }
            }
        }
        let luminance = |rgb: [f64; 3]| (0..3).map(|c| self.output_to_y[c] * rgb[c]).sum();
        original.with_scan_references(
            luminance(self.capture_f64([0.0; 3], channel_density, base_density)),
            luminance(self.capture_f64(maximum, channel_density, base_density)),
            params,
        )
    }

    pub(crate) fn capture_f64(
        &self,
        density: [f64; 3],
        channel_density: &[[f64; 3]],
        base_density: &[f64],
    ) -> [f64; 3] {
        let mut capture = [0.0; 3];
        let has_base = base_density.len() >= self.weights.len();
        for (i, weights) in self.weights.iter().enumerate() {
            let dye = channel_density[i];
            let d = density[0] * dye[0]
                + density[1] * dye[1]
                + density[2] * dye[2]
                + if has_base { base_density[i] } else { 0.0 };
            let light = if d.is_nan() { 0.0 } else { 10.0f64.powf(-d) };
            for c in 0..3 {
                capture[c] += light * weights[c];
            }
        }
        std::array::from_fn(|c| (0..3).map(|k| self.to_output[c][k] * capture[k]).sum())
    }

    pub(crate) fn capture(
        &self,
        image: &ImageBuf,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
    ) -> ImageBuf {
        let mut output = image.clone();
        output.data.par_chunks_exact_mut(3).for_each(|px| {
            let rgb = self.capture_f64(
                [to_f64(px[0]), to_f64(px[1]), to_f64(px[2])],
                channel_density,
                base_density,
            );
            for c in 0..3 {
                px[c] = from_f64(rgb[c]);
            }
        });
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receiver_luminance_uses_actual_output_transform() {
        let color = ScanColorContext::build(
            vec![1.0; spectral::N_WAVELENGTHS],
            spectral::N_WAVELENGTHS,
            "ProPhoto RGB",
        );
        let receiver = ScanReceiver::resolve(&color, "ProPhoto RGB");
        let detector = [0.2, 0.6, 0.9];
        let output: [f64; 3] =
            std::array::from_fn(|c| (0..3).map(|k| receiver.to_output[c][k] * detector[k]).sum());
        let extracted: f64 = (0..3)
            .map(|c| receiver.luminance_weights()[c] * output[c])
            .sum();
        let expected: f64 = (0..3)
            .map(|c| colorspace::SRGB.matrix_rgb_to_xyz[1][c] * detector[c])
            .sum();
        assert!((extracted - expected).abs() < 1e-12);
    }

    #[test]
    fn clear_and_neutral_densities_preserve_reference_levels() {
        let color = ScanColorContext::build(
            vec![1.0; spectral::N_WAVELENGTHS],
            spectral::N_WAVELENGTHS,
            "sRGB",
        );
        let receiver = ScanReceiver::resolve(&color, "sRGB");
        let dye = vec![[1.0, 0.0, 0.0]; spectral::N_WAVELENGTHS];
        let base = vec![0.0; spectral::N_WAVELENGTHS];
        for (density, expected) in [(0.0, 1.0), (1.0, 0.1), (2.0, 0.01)] {
            for value in receiver.capture_f64([density, 0.0, 0.0], &dye, &base) {
                assert!((value - expected).abs() < 1e-12);
            }
        }
        let acquisition = provenance(ReceiverProfile::LabScanApprox);
        assert_eq!(acquisition.calibration_status, "approximate_uncalibrated");
        assert_eq!(acquisition.band_centers_nm, Some([630.0, 545.0, 465.0]));
    }
}
