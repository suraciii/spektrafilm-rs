//! Independent transmission-first measurements for the Dye grain contract.
//!
//! This module measures fixed CMY record-density realizations outside the Dye
//! formation path. The caller declares the film spectra, receiver response,
//! aperture samples, wavelength quadrature, seed population, and calibration
//! provenance. The oracle integrates transmission over the aperture and
//! wavelength before converting the receiver capture to optical density.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Density convention used for the receiver result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DyeDensityConvention {
    /// `D = -log10(capture / reference)`.
    OpticalDensity,
}

/// Evidence state attached to a measurement or calibration record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DyeCalibrationStatus {
    Measured,
    Estimated,
    Proxy,
    Uncalibrated,
    Rejected,
}

/// One named effective algorithm input retained with a measurement record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DyeEffectiveParameter {
    pub name: String,
    pub value: f64,
}

/// A receiver's wavelength-dependent response for three output channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DyeReceiverSpectrum {
    pub name: String,
    pub source: String,
    pub version: String,
    pub calibration_status: DyeCalibrationStatus,
    /// Receiver response multiplied by any declared wavelength-independent
    /// detector scale. Each row corresponds to one protocol wavelength.
    pub response: Vec<[f64; 3]>,
    /// Positive clear-reference captures in the same units as `response`.
    pub reference_levels: [f64; 3],
}

impl DyeReceiverSpectrum {
    /// Construct a receiver spectrum after validating its physical response
    /// domain. Wavelength quadrature belongs to the measurement protocol.
    pub fn new(
        name: impl Into<String>,
        source: impl Into<String>,
        version: impl Into<String>,
        calibration_status: DyeCalibrationStatus,
        response: Vec<[f64; 3]>,
        reference_levels: [f64; 3],
    ) -> Result<Self, String> {
        let receiver = Self {
            name: name.into(),
            source: source.into(),
            version: version.into(),
            calibration_status,
            response,
            reference_levels,
        };
        receiver.validate()?;
        Ok(receiver)
    }

    fn validate(&self) -> Result<(), String> {
        require_text("receiver name", &self.name)?;
        require_text("receiver source", &self.source)?;
        require_text("receiver version", &self.version)?;
        if self.response.is_empty() {
            return Err("Dye measurement receiver response must not be empty".into());
        }
        for (wavelength, row) in self.response.iter().enumerate() {
            for (channel, &value) in row.iter().enumerate() {
                if !value.is_finite() || value < 0.0 {
                    return Err(format!(
                        "Dye measurement receiver response at wavelength index {wavelength}, channel {channel} must be finite and nonnegative"
                    ));
                }
            }
        }
        for (channel, &value) in self.reference_levels.iter().enumerate() {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!(
                    "Dye measurement receiver reference level {channel} must be finite and positive"
                ));
            }
            if self.response.iter().all(|row| row[channel] == 0.0) {
                return Err(format!(
                    "Dye measurement receiver channel {channel} has no response"
                ));
            }
        }
        Ok(())
    }
}

/// A normalized set of spatial samples in one output aperture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DyeApertureKernel {
    /// Sample positions relative to the measured output footprint, in µm.
    pub offsets_um: Vec<[f64; 2]>,
    /// Normalized nonnegative sample weights.
    pub weights: Vec<f64>,
}

impl DyeApertureKernel {
    /// Normalize and validate an explicitly declared aperture kernel.
    pub fn new(offsets_um: Vec<[f64; 2]>, weights: Vec<f64>) -> Result<Self, String> {
        if offsets_um.is_empty() || offsets_um.len() != weights.len() {
            return Err(
                "Dye measurement aperture must have equally sized nonempty offsets and weights"
                    .into(),
            );
        }
        for (index, offset) in offsets_um.iter().enumerate() {
            if offset.iter().any(|value| !value.is_finite()) {
                return Err(format!(
                    "Dye measurement aperture offset {index} must be finite"
                ));
            }
        }
        if weights
            .iter()
            .any(|&weight| !weight.is_finite() || weight < 0.0)
        {
            return Err("Dye measurement aperture weights must be finite and nonnegative".into());
        }
        let total: f64 = weights.iter().sum();
        if !total.is_finite() || total <= 0.0 {
            return Err("Dye measurement aperture weights must have a positive finite sum".into());
        }
        Ok(Self {
            offsets_um,
            weights: weights.into_iter().map(|weight| weight / total).collect(),
        })
    }

    fn validate(&self) -> Result<(), String> {
        if self.offsets_um.is_empty() || self.offsets_um.len() != self.weights.len() {
            return Err("Dye measurement aperture shape is invalid".into());
        }
        if self
            .offsets_um
            .iter()
            .any(|offset| offset.iter().any(|value| !value.is_finite()))
            || self
                .weights
                .iter()
                .any(|&weight| !weight.is_finite() || weight < 0.0)
        {
            return Err("Dye measurement aperture contains a non-finite or negative value".into());
        }
        let total: f64 = self.weights.iter().sum();
        if !total.is_finite() || (total - 1.0).abs() > 1e-12 {
            return Err(format!(
                "Dye measurement aperture weights must be normalized (sum {total})"
            ));
        }
        Ok(())
    }
}

/// A retained link to a measurement output or acceptance artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DyeArtifactReference {
    /// Relative artifact path or stable artifact URI.
    pub path: String,
    /// Artifact role, such as `measurement_output` or `acceptance_record`.
    pub kind: String,
    /// Human-readable purpose of the retained artifact.
    pub description: String,
}

/// A fixed optical protocol and all data needed to reproduce it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DyeMeasurementProtocol {
    pub profile_id: String,
    pub profile_provenance: String,
    pub density_convention: DyeDensityConvention,
    pub wavelengths_nm: Vec<f64>,
    /// Positive quadrature weights for the declared wavelength samples.
    pub wavelength_weights: Vec<f64>,
    /// Base-plus-fog density at each wavelength.
    pub base_density: Vec<f64>,
    /// CMY dye-record density at each wavelength and record channel.
    pub dye_density: Vec<[f64; 3]>,
    pub receiver: DyeReceiverSpectrum,
    pub aperture: DyeApertureKernel,
    pub output_footprint_um: [f64; 2],
    pub viewing_condition: String,
    pub seed_population: Vec<u64>,
    pub algorithm_revision: String,
    pub precision: String,
    pub executor: String,
    pub effective_parameters: Vec<DyeEffectiveParameter>,
    pub calibration_status: DyeCalibrationStatus,
    /// Links to measurement outputs and acceptance artifacts retained with the record.
    #[serde(default)]
    pub artifact_references: Vec<DyeArtifactReference>,

    /// Optional declared reference density used to report tone and color bias.
    pub reference_density: Option<[f64; 3]>,
}

/// One fixed CMY realization sampled at the aperture positions.
#[derive(Debug, Clone, PartialEq)]
pub struct DyeRealization {
    pub samples: Vec<[f64; 3]>,
}

impl DyeRealization {
    pub fn new(samples: Vec<[f64; 3]>) -> Self {
        Self { samples }
    }
}

/// Summary of one measurement population.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DyeMeasurementRecord {
    /// Artifact links copied from the declared protocol for retained evidence.
    pub artifact_references: Vec<DyeArtifactReference>,

    pub protocol: DyeMeasurementProtocol,
    pub realization_count: usize,
    pub mean_density: [f64; 3],
    /// Population standard deviation of the receiver density readings.
    pub rms_density: [f64; 3],
    pub covariance: [[f64; 3]; 3],
    /// Standard error of the mean for each receiver density channel.
    pub uncertainty_density: [f64; 3],
    /// Mean channel bias relative to `reference_density`, when declared.
    pub tone_bias: Option<f64>,
    /// Per-channel residual bias after removing `tone_bias`, when declared.
    pub color_bias: Option<[f64; 3]>,
}

/// Independent transmission-first Dye measurement oracle.
#[derive(Debug, Clone)]
pub struct DyeOpticalOracle {
    protocol: DyeMeasurementProtocol,
}

impl DyeOpticalOracle {
    /// Validate and retain a complete measurement protocol.
    pub fn new(protocol: DyeMeasurementProtocol) -> Result<Self, String> {
        validate_protocol(&protocol)?;
        Ok(Self { protocol })
    }

    pub fn protocol(&self) -> &DyeMeasurementProtocol {
        &self.protocol
    }

    /// Read one aperture realization.
    pub fn read_realization(&self, realization: &DyeRealization) -> Result<[f64; 3], String> {
        if realization.samples.len() != self.protocol.aperture.weights.len() {
            return Err(format!(
                "Dye measurement realization has {} aperture samples; expected {}",
                realization.samples.len(),
                self.protocol.aperture.weights.len()
            ));
        }
        let mut capture = [0.0; 3];
        for (sample, &aperture_weight) in realization
            .samples
            .iter()
            .zip(&self.protocol.aperture.weights)
        {
            let receiver = receiver_capture_validated(sample, &self.protocol)?;
            for channel in 0..3 {
                capture[channel] += aperture_weight * receiver[channel];
            }
        }
        let mut density = [0.0; 3];
        for channel in 0..3 {
            let ratio = capture[channel] / self.protocol.receiver.reference_levels[channel];
            if !ratio.is_finite() || ratio <= 0.0 {
                return Err(format!(
                    "Dye measurement receiver capture {channel} is not positive and finite"
                ));
            }
            density[channel] = -ratio.log10();
        }
        Ok(density)
    }

    /// Measure the declared seed population without reading or normalizing a
    /// current image. Each realization must correspond to one declared seed.
    pub fn measure(&self, realizations: &[DyeRealization]) -> Result<DyeMeasurementRecord, String> {
        if realizations.is_empty() {
            return Err("Dye measurement realization population must not be empty".into());
        }
        if self.protocol.seed_population.len() != realizations.len() {
            return Err(format!(
                "Dye measurement seed population has {} entries for {} realizations",
                self.protocol.seed_population.len(),
                realizations.len()
            ));
        }
        let mut unique = HashSet::with_capacity(self.protocol.seed_population.len());
        for &seed in &self.protocol.seed_population {
            if !unique.insert(seed) {
                return Err(format!(
                    "Dye measurement seed population repeats seed {seed}"
                ));
            }
        }
        let readings: Vec<[f64; 3]> = realizations
            .iter()
            .map(|realization| self.read_realization(realization))
            .collect::<Result<_, _>>()?;
        let count = readings.len() as f64;
        let mean = std::array::from_fn(|channel| {
            readings.iter().map(|reading| reading[channel]).sum::<f64>() / count
        });
        let mut covariance = [[0.0; 3]; 3];
        for reading in &readings {
            for a in 0..3 {
                for b in 0..3 {
                    covariance[a][b] += (reading[a] - mean[a]) * (reading[b] - mean[b]);
                }
            }
        }
        for row in &mut covariance {
            for value in row {
                *value /= count;
            }
        }
        let rms = std::array::from_fn(|channel| covariance[channel][channel].sqrt());
        let uncertainty = rms.map(|value| value / count.sqrt());
        let (tone_bias, color_bias) = self
            .protocol
            .reference_density
            .map(|reference| {
                let delta = std::array::from_fn(|channel| mean[channel] - reference[channel]);
                let tone = delta.iter().sum::<f64>() / 3.0;
                (Some(tone), Some(delta.map(|value| value - tone)))
            })
            .unwrap_or((None, None));
        Ok(DyeMeasurementRecord {
            protocol: self.protocol.clone(),
            realization_count: readings.len(),
            mean_density: mean,
            rms_density: rms,
            covariance,
            uncertainty_density: uncertainty,
            tone_bias,
            color_bias,
            artifact_references: self.protocol.artifact_references.clone(),
        })
    }
}

pub fn validate_protocol(protocol: &DyeMeasurementProtocol) -> Result<(), String> {
    require_text("Dye measurement profile_id", &protocol.profile_id)?;
    require_text(
        "Dye measurement profile_provenance",
        &protocol.profile_provenance,
    )?;
    require_text(
        "Dye measurement viewing_condition",
        &protocol.viewing_condition,
    )?;
    require_text(
        "Dye measurement algorithm_revision",
        &protocol.algorithm_revision,
    )?;
    require_text("Dye measurement precision", &protocol.precision)?;
    require_text("Dye measurement executor", &protocol.executor)?;
    if protocol.wavelengths_nm.is_empty()
        || protocol.wavelengths_nm.len() != protocol.wavelength_weights.len()
        || protocol.wavelengths_nm.len() != protocol.base_density.len()
        || protocol.wavelengths_nm.len() != protocol.dye_density.len()
        || protocol.wavelengths_nm.len() != protocol.receiver.response.len()
    {
        return Err(
            "Dye measurement wavelength, quadrature, spectrum, base, dye, and receiver arrays must have one nonempty shared length"
                .into(),
        );
    }
    for (index, &wavelength) in protocol.wavelengths_nm.iter().enumerate() {
        if !wavelength.is_finite()
            || (index > 0 && wavelength <= protocol.wavelengths_nm[index - 1])
        {
            return Err(
                "Dye measurement wavelengths must be finite and strictly increasing".into(),
            );
        }
        let weight = protocol.wavelength_weights[index];
        if !weight.is_finite() || weight <= 0.0 {
            return Err(
                "Dye measurement wavelength quadrature weights must be finite and positive".into(),
            );
        }
        if !protocol.base_density[index].is_finite()
            || protocol.dye_density[index]
                .iter()
                .any(|value| !value.is_finite())
        {
            return Err(
                "Dye measurement base and dye spectra must contain only finite values".into(),
            );
        }
    }
    protocol.receiver.validate()?;
    protocol.aperture.validate()?;
    if protocol
        .output_footprint_um
        .iter()
        .any(|&value| !value.is_finite() || value <= 0.0)
    {
        return Err("Dye measurement output footprint must be finite and positive".into());
    }
    if protocol.seed_population.is_empty() {
        return Err("Dye measurement seed population must not be empty".into());
    }
    for parameter in &protocol.effective_parameters {
        require_text("Dye effective parameter name", &parameter.name)?;
        if !parameter.value.is_finite() {
            return Err(format!(
                "Dye effective parameter {} must be finite",
                parameter.name
            ));
        }
    }
    if protocol.artifact_references.is_empty() {
        return Err("Dye measurement artifact references must not be empty".into());
    }
    for artifact in &protocol.artifact_references {
        require_text("Dye measurement artifact path", &artifact.path)?;
        require_text("Dye measurement artifact kind", &artifact.kind)?;
        require_text(
            "Dye measurement artifact description",
            &artifact.description,
        )?;
    }

    if let Some(reference) = protocol.reference_density {
        if reference.iter().any(|&value| !value.is_finite()) {
            return Err("Dye measurement reference density must be finite".into());
        }
    }
    Ok(())
}

/// Convert one total optical density to transmission.
pub fn transmission(total_density: f64) -> Result<f64, String> {
    if !total_density.is_finite() {
        return Err("Dye measurement total density must be finite".into());
    }
    let value = 10.0f64.powf(-total_density);
    if !value.is_finite() || value <= 0.0 {
        return Err("Dye measurement transmission must be positive and finite".into());
    }
    Ok(value)
}

/// Integrate one CMY density sample through the declared spectral receiver.
pub fn receiver_capture(
    density: &[f64; 3],
    protocol: &DyeMeasurementProtocol,
) -> Result<[f64; 3], String> {
    validate_protocol(protocol)?;
    receiver_capture_validated(density, protocol)
}

fn receiver_capture_validated(
    density: &[f64; 3],
    protocol: &DyeMeasurementProtocol,
) -> Result<[f64; 3], String> {
    if density.iter().any(|&value| !value.is_finite()) {
        return Err("Dye measurement realization density must be finite".into());
    }
    let mut output = [0.0; 3];
    for wavelength in 0..protocol.wavelengths_nm.len() {
        let mut total_density = protocol.base_density[wavelength];
        for channel in 0..3 {
            total_density += density[channel] * protocol.dye_density[wavelength][channel];
        }
        let sample_transmission = transmission(total_density)
            .map_err(|error| format!("{error} at wavelength index {wavelength}"))?;
        let wavelength_weight = protocol.wavelength_weights[wavelength];
        for channel in 0..3 {
            output[channel] += wavelength_weight
                * protocol.receiver.response[wavelength][channel]
                * sample_transmission;
        }
    }
    if output
        .iter()
        .any(|&value| !value.is_finite() || value <= 0.0)
    {
        return Err("Dye measurement receiver capture must be positive and finite".into());
    }
    Ok(output)
}

/// Require a nonblank text value in a declared measurement record.
pub fn require_text(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol(
        aperture: DyeApertureKernel,
        reference_density: Option<[f64; 3]>,
    ) -> DyeMeasurementProtocol {
        DyeMeasurementProtocol {
            profile_id: "test-stock".into(),
            profile_provenance: "test fixture".into(),
            density_convention: DyeDensityConvention::OpticalDensity,
            wavelengths_nm: vec![500.0, 600.0],
            wavelength_weights: vec![0.5, 0.5],
            base_density: vec![0.0, 0.0],
            dye_density: vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            receiver: DyeReceiverSpectrum::new(
                "test receiver",
                "test fixture",
                "1",
                DyeCalibrationStatus::Uncalibrated,
                vec![[1.0, 1.0, 1.0], [1.0, 1.0, 1.0]],
                [1.0; 3],
            )
            .unwrap(),
            aperture,
            output_footprint_um: [1.0, 1.0],
            viewing_condition: "test viewing".into(),
            seed_population: vec![1, 2],
            algorithm_revision: "test".into(),
            precision: "f64".into(),
            executor: "test".into(),
            effective_parameters: vec![],
            calibration_status: DyeCalibrationStatus::Uncalibrated,
            artifact_references: vec![DyeArtifactReference {
                path: "measurements/test.json".into(),
                kind: "measurement_output".into(),
                description: "Synthetic oracle measurement".into(),
            }],
            reference_density,
        }
    }

    #[test]
    fn constant_field_is_positive_and_transmission_first() {
        let aperture = DyeApertureKernel::new(vec![[0.0, 0.0]], vec![4.0]).unwrap();
        let oracle = DyeOpticalOracle::new(protocol(aperture, None)).unwrap();
        let reading = oracle
            .read_realization(&DyeRealization::new(vec![[0.4, 0.6, 0.0]]))
            .unwrap();
        // The two spectral transmissions are 10^-0.4 and 10^-0.6. Their
        // receiver average is converted only after the spectral integration.
        let expected = -((0.5 * 10.0f64.powf(-0.4) + 0.5 * 10.0f64.powf(-0.6)).log10());
        assert!((reading[0] - expected).abs() < 1e-12, "{reading:?}");
        assert!(reading.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn aperture_weights_are_normalized_and_average_transmission() {
        let aperture =
            DyeApertureKernel::new(vec![[0.0, 0.0], [1.0, 0.0]], vec![1.0, 3.0]).unwrap();
        assert_eq!(aperture.weights, vec![0.25, 0.75]);
        let oracle = DyeOpticalOracle::new(protocol(aperture, None)).unwrap();
        let reading = oracle
            .read_realization(&DyeRealization::new(vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]))
            .unwrap();
        let expected_capture = 0.25 * 1.0 + 0.75 * (0.5 * 10.0f64.powf(-1.0) + 0.5);
        assert!((reading[0] + expected_capture.log10()).abs() < 1e-12);
    }

    #[test]
    fn population_statistics_and_bias_are_declared() {
        let aperture = DyeApertureKernel::new(vec![[0.0, 0.0]], vec![1.0]).unwrap();
        let oracle = DyeOpticalOracle::new(protocol(aperture, Some([0.25; 3]))).unwrap();
        let record = oracle
            .measure(&[
                DyeRealization::new(vec![[0.0, 0.0, 0.0]]),
                DyeRealization::new(vec![[1.0, 1.0, 1.0]]),
            ])
            .unwrap();
        assert_eq!(record.realization_count, 2);
        assert_eq!(record.artifact_references.len(), 1);
        assert_eq!(record.artifact_references[0].path, "measurements/test.json");
        assert!(record.rms_density.iter().all(|value| *value > 0.0));
        assert!(
            record
                .covariance
                .iter()
                .flatten()
                .all(|value| value.is_finite())
        );
        assert!(record.tone_bias.is_some());
        assert!(record.color_bias.is_some());
        assert!(
            record
                .color_bias
                .unwrap()
                .iter()
                .all(|value| value.abs() < 1e-12)
        );
    }
    #[test]
    fn public_helpers_preserve_the_transmission_first_boundary() {
        assert!((transmission(0.4).unwrap() - 10.0f64.powf(-0.4)).abs() < 1e-12);
        assert!(transmission(f64::NAN).is_err());
        let aperture = DyeApertureKernel::new(vec![[0.0, 0.0]], vec![1.0]).unwrap();
        let protocol = protocol(aperture, None);
        assert!(validate_protocol(&protocol).is_ok());
        assert!(require_text("field", "  ").is_err());
        let capture = receiver_capture(&[0.4, 0.6, 0.0], &protocol).unwrap();
        assert!(
            capture
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
        );
    }

    #[test]
    fn invalid_aperture_and_seed_population_fail_before_measurement() {
        assert!(DyeApertureKernel::new(vec![[0.0, 0.0]], vec![0.0]).is_err());
        let aperture = DyeApertureKernel::new(vec![[0.0, 0.0]], vec![1.0]).unwrap();
        let mut invalid = protocol(aperture, None);
        invalid.seed_population = vec![1];
        let oracle = DyeOpticalOracle::new(invalid).unwrap();
        assert!(
            oracle
                .measure(&[
                    DyeRealization::new(vec![[0.0, 0.0, 0.0]]),
                    DyeRealization::new(vec![[0.0, 0.0, 0.0]]),
                ])
                .is_err()
        );
    }
}
