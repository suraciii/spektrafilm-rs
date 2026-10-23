//! Independent transmission-first measurements for the Dye grain contract.
//!
//! This module measures fixed CMY record-density realizations outside the Dye
//! formation path. The caller declares the film spectra, receiver response,
//! aperture samples, wavelength quadrature, seed population, and calibration
//! provenance. The oracle integrates transmission over the aperture and
//! wavelength before converting the receiver capture to optical density.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const MEASUREMENT_REVISION_PREFIX: &str = "dye-measurement-v2:";

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
    /// Caller revision; new population records prepend `dye-measurement-v2:`
    /// unless already present. The oracle retains the declared value unchanged.
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

/// One fixed CMY realization sampled at the declared aperture positions.
///
/// The geometry is retained with the samples so a population cannot be
/// relabeled with a different aperture or output footprint after sampling.
#[derive(Debug, Clone, PartialEq)]
pub struct DyeRealization {
    /// Seed that generated this realization.
    pub seed: u64,
    /// Positions corresponding one-to-one with `samples`, in µm.
    pub offsets_um: Vec<[f64; 2]>,
    /// Output footprint represented by these samples, in µm.
    pub footprint_um: [f64; 2],
    /// CMY density values at the declared aperture positions.
    pub samples: Vec<[f64; 3]>,
}

impl DyeRealization {
    pub fn new(
        seed: u64,
        offsets_um: Vec<[f64; 2]>,
        footprint_um: [f64; 2],
        samples: Vec<[f64; 3]>,
    ) -> Self {
        Self {
            seed,
            offsets_um,
            footprint_um,
            samples,
        }
    }
}

fn validate_realization_geometry(
    realization: &DyeRealization,
    protocol: &DyeMeasurementProtocol,
) -> Result<(), String> {
    if realization.offsets_um != protocol.aperture.offsets_um {
        return Err(
            "Dye measurement realization aperture positions do not match the protocol".into(),
        );
    }
    if realization.footprint_um != protocol.output_footprint_um {
        return Err(
            "Dye measurement realization output footprint does not match the protocol".into(),
        );
    }
    Ok(())
}

/// Summary of one measurement population. Interpret legacy uncertainty values
/// using the retained `protocol.algorithm_revision`; deserialization does not
/// relabel or recompute old records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DyeMeasurementRecord {
    /// Artifact links copied from the declared protocol for retained evidence.
    pub artifact_references: Vec<DyeArtifactReference>,

    pub protocol: DyeMeasurementProtocol,
    pub realization_count: usize,
    pub mean_density: [f64; 3],
    /// Population standard deviation of receiver density readings, in D (ddof 0).
    pub rms_density: [f64; 3],
    /// Descriptive receiver-density covariance, in D², divided by population N.
    pub covariance: [[f64; 3]; 3],
    /// For `dye-measurement-v2:` records, estimated standard error of the mean
    /// in D: `rms_density / sqrt(N - 1)`, assuming independent seed realizations.
    /// Unique seeds do not establish independence, and zero estimated SEM does
    /// not imply zero systematic error. Legacy revisions retain their old meaning.
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

    pub fn read_realization(&self, realization: &DyeRealization) -> Result<[f64; 3], String> {
        validate_realization_geometry(realization, &self.protocol)?;
        if realization.samples.len() != self.protocol.aperture.weights.len() {
            return Err(format!(
                "Dye measurement realization has {} aperture samples; expected {}",
                realization.samples.len(),
                self.protocol.aperture.weights.len()
            ));
        }
        let mut capture = [0.0; 3];
        for (sample_index, (sample, &aperture_weight)) in realization
            .samples
            .iter()
            .zip(&self.protocol.aperture.weights)
            .enumerate()
        {
            let receiver = receiver_capture_validated(sample, &self.protocol).map_err(|error| {
                format!(
                    "{error} at aperture sample index {sample_index}, seed {}",
                    realization.seed
                )
            })?;
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

    /// Measure at least two declared seed realizations without reading or
    /// normalizing a current image. Each realization must match one declared
    /// seed, in order. SEM estimation assumes independent seed realizations.
    pub fn measure(&self, realizations: &[DyeRealization]) -> Result<DyeMeasurementRecord, String> {
        if realizations.len() < 2 {
            return Err("Dye measurement population requires at least two realizations".into());
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
        for (index, realization) in realizations.iter().enumerate() {
            if realization.seed != self.protocol.seed_population[index] {
                return Err(format!(
                    "Dye measurement realization {index} has seed {}; expected {}",
                    realization.seed, self.protocol.seed_population[index]
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
        let sem_divisor = (count - 1.0).sqrt();
        let uncertainty = rms.map(|value| value / sem_divisor);
        let (tone_bias, color_bias) = self
            .protocol
            .reference_density
            .map(|reference| {
                let delta = std::array::from_fn(|channel| mean[channel] - reference[channel]);
                let tone = delta.iter().sum::<f64>() / 3.0;
                (Some(tone), Some(delta.map(|value| value - tone)))
            })
            .unwrap_or((None, None));
        let mut protocol = self.protocol.clone();
        if !protocol
            .algorithm_revision
            .starts_with(MEASUREMENT_REVISION_PREFIX)
        {
            protocol
                .algorithm_revision
                .insert_str(0, MEASUREMENT_REVISION_PREFIX);
        }
        Ok(DyeMeasurementRecord {
            protocol,
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

    let invalid_reference = protocol
        .reference_density
        .is_some_and(|reference| reference.iter().any(|&value| !value.is_finite()));
    if invalid_reference {
        return Err("Dye measurement reference density must be finite".into());
    }
    Ok(())
}

/// Convert finite nonnegative absolute total spectral density to transmission.
/// Negative totals are invalid for passive material; they are never clamped.
pub fn transmission(total_density: f64) -> Result<f64, String> {
    if !total_density.is_finite() {
        return Err("Dye measurement absolute total spectral density must be finite".into());
    }
    if total_density < 0.0 {
        return Err(format!(
            "Dye measurement absolute total spectral density must be nonnegative (got {total_density})"
        ));
    }
    let value = 10.0f64.powf(-total_density);
    if !value.is_finite() {
        return Err("Dye measurement transmission must be finite".into());
    }
    if value == 0.0 {
        return Err("Dye measurement transmission underflow at high absolute density".into());
    }
    Ok(value)
}

/// Integrate one CMY density sample through the declared spectral receiver.
/// Only its summed base-plus-CMY spectral density must be nonnegative; an
/// individual CMY coefficient may be negative.
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
    let mut output = [0.0; 3];
    #[allow(clippy::needless_range_loop)]
    for wavelength in 0..protocol.wavelengths_nm.len() {
        let mut total_density = protocol.base_density[wavelength];
        for channel in 0..3 {
            total_density += density[channel] * protocol.dye_density[wavelength][channel];
        }
        let sample_transmission = transmission(total_density).map_err(|error| {
            format!(
                "{error} at wavelength index {wavelength} ({} nm)",
                protocol.wavelengths_nm[wavelength]
            )
        })?;
        let wavelength_weight = protocol.wavelength_weights[wavelength];
        #[allow(clippy::needless_range_loop)]
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

    fn point_protocol(reference_density: Option<[f64; 3]>) -> DyeMeasurementProtocol {
        protocol(
            DyeApertureKernel::new(vec![[0.0, 0.0]], vec![1.0]).unwrap(),
            reference_density,
        )
    }

    fn realization(
        protocol: &DyeMeasurementProtocol,
        seed: u64,
        samples: Vec<[f64; 3]>,
    ) -> DyeRealization {
        DyeRealization::new(
            seed,
            protocol.aperture.offsets_um.clone(),
            protocol.output_footprint_um,
            samples,
        )
    }
    #[test]
    fn constant_field_is_positive_and_transmission_first() {
        let aperture = DyeApertureKernel::new(vec![[0.0, 0.0]], vec![4.0]).unwrap();
        let oracle = DyeOpticalOracle::new(protocol(aperture, None)).unwrap();
        let reading = oracle
            .read_realization(&realization(oracle.protocol(), 1, vec![[0.4, 0.6, 0.0]]))
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
            .read_realization(&realization(
                oracle.protocol(),
                1,
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            ))
            .unwrap();
        let expected_capture = 0.25 * 1.0 + 0.75 * (0.5 * 10.0f64.powf(-1.0) + 0.5);
        assert!((reading[0] + expected_capture.log10()).abs() < 1e-12);
    }

    #[test]
    fn population_statistics_and_bias_use_optical_readings() {
        let oracle = DyeOpticalOracle::new(point_protocol(Some([0.25; 3]))).unwrap();
        // Equal CMY coefficients at both wavelengths give receiver densities
        // [0, 2] in every channel, including the off-diagonal covariance pairs.
        let record = oracle
            .measure(&[
                realization(oracle.protocol(), 1, vec![[0.0; 3]]),
                realization(oracle.protocol(), 2, vec![[2.0; 3]]),
            ])
            .unwrap();
        assert_eq!(record.realization_count, 2);
        for channel in 0..3 {
            assert!((record.mean_density[channel] - 1.0).abs() < 1e-12);
            assert!((record.rms_density[channel] - 1.0).abs() < 1e-12);
            assert!((record.uncertainty_density[channel] - 1.0).abs() < 1e-12);
            for &covariance in &record.covariance[channel] {
                assert!((covariance - 1.0).abs() < 1e-12);
            }
        }
        assert!((record.tone_bias.unwrap() - 0.75).abs() < 1e-12);
        assert!(
            record
                .color_bias
                .unwrap()
                .iter()
                .all(|value| value.abs() < 1e-12)
        );
    }

    #[test]
    fn constant_population_has_zero_descriptive_spread_and_estimated_sem() {
        let mut protocol = point_protocol(None);
        protocol.seed_population = vec![1, 2, 3];
        let oracle = DyeOpticalOracle::new(protocol).unwrap();
        let population: Vec<_> = oracle
            .protocol()
            .seed_population
            .iter()
            .map(|&seed| realization(oracle.protocol(), seed, vec![[2.0; 3]]))
            .collect();
        let record = oracle.measure(&population).unwrap();
        assert!(
            record
                .mean_density
                .iter()
                .all(|value| (value - 2.0).abs() < 1e-12)
        );
        assert_eq!(record.covariance, [[0.0; 3]; 3]);
        assert_eq!(record.rms_density, [0.0; 3]);
        assert_eq!(record.uncertainty_density, [0.0; 3]);
    }

    #[test]
    fn passive_totals_retain_the_existing_transmission_calculation() {
        for total in [0.0, -0.0, 0.4, 2.0, 300.0] {
            assert_eq!(transmission(total).unwrap(), 10.0f64.powf(-total));
        }
        let protocol = point_protocol(None);
        let expected = 0.5 * 10.0f64.powf(-0.4) + 0.5 * 10.0f64.powf(-0.6);
        assert_eq!(
            receiver_capture(&[0.4, 0.6, 0.0], &protocol).unwrap(),
            [expected; 3]
        );
    }

    #[test]
    fn absolute_density_failures_are_distinct_and_located() {
        let aperture =
            DyeApertureKernel::new(vec![[0.0, 0.0], [1.0, 0.0]], vec![1.0, 1.0]).unwrap();
        let oracle = DyeOpticalOracle::new(protocol(aperture, None)).unwrap();
        for (value, reason) in [
            (-0.1, "must be nonnegative"),
            (f64::NAN, "must be finite"),
            (f64::INFINITY, "must be finite"),
            (f64::NEG_INFINITY, "must be finite"),
            (400.0, "transmission underflow"),
        ] {
            let direct = transmission(value).unwrap_err();
            assert!(direct.contains(reason), "{direct}");
            let sample = [0.0, value, 0.0];
            let error = oracle
                .read_realization(&realization(oracle.protocol(), 17, vec![[0.0; 3], sample]))
                .unwrap_err();
            assert!(error.contains(reason), "{error}");
            assert!(error.contains("wavelength index"), "{error}");
            // Nonfinite coefficients also produce NaN when multiplied by a
            // zero spectral coefficient, so their first invalid total is 500 nm.
            let wavelength = if value.is_finite() {
                "1 (600 nm)"
            } else {
                "0 (500 nm)"
            };
            assert!(error.contains(wavelength), "{error}");
            assert!(error.contains("aperture sample index 1"), "{error}");
            assert!(error.contains("seed 17"), "{error}");

            let helper_error = receiver_capture(&sample, oracle.protocol()).unwrap_err();
            assert!(helper_error.contains(reason), "{helper_error}");
            assert!(helper_error.contains(wavelength), "{helper_error}");
            assert!(!helper_error.contains("seed"), "{helper_error}");
        }
    }

    #[test]
    fn passivity_applies_to_the_sum_not_individual_cmy_coefficients() {
        let mut protocol = point_protocol(None);
        protocol.base_density = vec![0.5, 0.5];
        let oracle = DyeOpticalOracle::new(protocol).unwrap();
        let reading = oracle
            .read_realization(&realization(oracle.protocol(), 1, vec![[-0.25, 0.25, 0.0]]))
            .unwrap();
        let expected = -(0.5 * 10.0f64.powf(-0.25) + 0.5 * 10.0f64.powf(-0.75)).log10();
        assert!(reading.iter().all(|value| (value - expected).abs() < 1e-12));
    }

    #[test]
    fn declared_reference_can_produce_negative_record_density() {
        let mut protocol = point_protocol(None);
        protocol.receiver.reference_levels = [0.1; 3];
        let oracle = DyeOpticalOracle::new(protocol).unwrap();
        let population = [
            realization(oracle.protocol(), 1, vec![[0.0; 3]]),
            realization(oracle.protocol(), 2, vec![[0.0; 3]]),
        ];
        assert_eq!(oracle.read_realization(&population[0]).unwrap(), [-1.0; 3]);
        assert_eq!(oracle.measure(&population).unwrap().mean_density, [-1.0; 3]);
    }

    #[test]
    fn single_realization_read_is_valid_but_population_measure_requires_two() {
        let mut protocol = point_protocol(None);
        protocol.seed_population = vec![1];
        let oracle = DyeOpticalOracle::new(protocol).unwrap();
        let sample = realization(oracle.protocol(), 1, vec![[0.0; 3]]);
        assert_eq!(oracle.read_realization(&sample).unwrap(), [0.0; 3]);
        for population in [&[][..], std::slice::from_ref(&sample)] {
            let error = oracle.measure(population).unwrap_err();
            assert!(error.contains("at least two realizations"), "{error}");
        }
        let mut empty_protocol = oracle.protocol().clone();
        empty_protocol.seed_population.clear();
        assert!(
            DyeOpticalOracle::new(empty_protocol)
                .unwrap_err()
                .contains("must not be empty")
        );
    }

    #[test]
    fn realization_geometry_must_match_for_reads_and_population_measurements() {
        let aperture =
            DyeApertureKernel::new(vec![[0.0, 0.0], [1.0, 0.0]], vec![1.0, 1.0]).unwrap();
        let oracle = DyeOpticalOracle::new(protocol(aperture, None)).unwrap();
        let mut wrong_positions = realization(oracle.protocol(), 1, vec![[0.0; 3]; 2]);
        wrong_positions.offsets_um[1] = [2.0, 0.0];
        let mut wrong_footprint = realization(oracle.protocol(), 1, vec![[0.0; 3]; 2]);
        wrong_footprint.footprint_um[0] = 2.0;
        let wrong_sample_count = realization(oracle.protocol(), 1, vec![[0.0; 3]]);
        for (invalid, reason) in [
            (wrong_positions, "aperture positions do not match"),
            (wrong_footprint, "output footprint does not match"),
            (wrong_sample_count, "1 aperture samples; expected 2"),
        ] {
            let error = oracle.read_realization(&invalid).unwrap_err();
            assert!(error.contains(reason), "{error}");
            let population = [
                invalid,
                realization(oracle.protocol(), 2, vec![[0.0; 3]; 2]),
            ];
            let error = oracle.measure(&population).unwrap_err();
            assert!(error.contains(reason), "{error}");
        }
    }

    #[test]
    fn invalid_aperture_and_seed_populations_are_rejected() {
        assert!(DyeApertureKernel::new(vec![[0.0, 0.0]], vec![0.0]).is_err());
        let mut duplicate = point_protocol(None);
        duplicate.seed_population = vec![1, 1];
        let oracle = DyeOpticalOracle::new(duplicate).unwrap();
        let error = oracle
            .measure(&[
                realization(oracle.protocol(), 1, vec![[0.0; 3]]),
                realization(oracle.protocol(), 1, vec![[0.0; 3]]),
            ])
            .unwrap_err();
        assert!(error.contains("repeats seed 1"), "{error}");

        let oracle = DyeOpticalOracle::new(point_protocol(None)).unwrap();
        let error = oracle
            .measure(&[
                realization(oracle.protocol(), 2, vec![[0.0; 3]]),
                realization(oracle.protocol(), 1, vec![[0.0; 3]]),
            ])
            .unwrap_err();
        assert!(
            error.contains("realization 0 has seed 2; expected 1"),
            "{error}"
        );

        let mut wrong_count = oracle.protocol().clone();
        wrong_count.seed_population = vec![1, 2, 3];
        let oracle = DyeOpticalOracle::new(wrong_count).unwrap();
        let error = oracle
            .measure(&[
                realization(oracle.protocol(), 1, vec![[0.0; 3]]),
                realization(oracle.protocol(), 2, vec![[0.0; 3]]),
            ])
            .unwrap_err();
        assert!(error.contains("3 entries for 2 realizations"), "{error}");
    }

    #[test]
    fn legacy_json_keeps_old_sem_while_new_records_stamp_the_corrected_estimator() {
        // A retained two-realization legacy record used rms/sqrt(N), not the
        // independent-seed SEM now estimated from ddof-1 sample variance.
        let legacy_json = r#"{
            "artifact_references": [{"path":"measurements/legacy.json",
                "kind":"measurement_output","description":"Legacy fixture"}],
            "protocol": {
                "profile_id":"test-stock", "profile_provenance":"test fixture",
                "density_convention":"optical_density",
                "wavelengths_nm":[500.0,600.0], "wavelength_weights":[0.5,0.5],
                "base_density":[0.0,0.0],
                "dye_density":[[1.0,0.0,0.0],[0.0,1.0,0.0]],
                "receiver": {"name":"test receiver","source":"test fixture",
                    "version":"1","calibration_status":"uncalibrated",
                    "response":[[1.0,1.0,1.0],[1.0,1.0,1.0]],
                    "reference_levels":[1.0,1.0,1.0]},
                "aperture":{"offsets_um":[[0.0,0.0]],"weights":[1.0]},
                "output_footprint_um":[1.0,1.0], "viewing_condition":"test viewing",
                "seed_population":[1,2], "algorithm_revision":"legacy-generator",
                "precision":"f64", "executor":"test", "effective_parameters":[],
                "calibration_status":"uncalibrated",
                "artifact_references":[{"path":"measurements/legacy.json",
                    "kind":"measurement_output","description":"Legacy fixture"}],
                "reference_density":[0.25,0.25,0.25]
            },
            "realization_count":2, "mean_density":[1.0,1.0,1.0],
            "rms_density":[1.0,1.0,1.0], "covariance":[[1,1,1],[1,1,1],[1,1,1]],
            "uncertainty_density":[0.7071067811865476,0.7071067811865476,0.7071067811865476],
            "tone_bias":0.75, "color_bias":[0.0,0.0,0.0]
        }"#;
        let legacy: DyeMeasurementRecord = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(legacy.protocol.algorithm_revision, "legacy-generator");
        assert_eq!(
            legacy.uncertainty_density,
            [std::f64::consts::FRAC_1_SQRT_2; 3]
        );
        assert_eq!(legacy.covariance, [[1.0; 3]; 3]);

        let oracle = DyeOpticalOracle::new(legacy.protocol.clone()).unwrap();
        let population = [
            realization(oracle.protocol(), 1, vec![[0.0; 3]]),
            realization(oracle.protocol(), 2, vec![[2.0; 3]]),
        ];
        let current = oracle.measure(&population).unwrap();
        assert_eq!(oracle.protocol().algorithm_revision, "legacy-generator");
        assert_eq!(
            current.protocol.algorithm_revision,
            "dye-measurement-v2:legacy-generator"
        );
        assert_eq!(current.mean_density, legacy.mean_density);
        assert_eq!(current.covariance, legacy.covariance);
        assert_eq!(current.rms_density, legacy.rms_density);
        assert_eq!(current.uncertainty_density, [1.0; 3]);
        assert_eq!(
            legacy.uncertainty_density,
            [std::f64::consts::FRAC_1_SQRT_2; 3]
        );

        let reused = DyeOpticalOracle::new(current.protocol.clone())
            .unwrap()
            .measure(&population)
            .unwrap();
        assert_eq!(
            reused.protocol.algorithm_revision,
            current.protocol.algorithm_revision
        );

        let old_value: serde_json::Value = serde_json::from_str(legacy_json).unwrap();
        let new_json = serde_json::to_string(&current).unwrap();
        let new_value: serde_json::Value = serde_json::from_str(&new_json).unwrap();
        for (old, new) in [
            (&old_value, &new_value),
            (&old_value["protocol"], &new_value["protocol"]),
        ] {
            assert_eq!(
                old.as_object().unwrap().keys().collect::<HashSet<_>>(),
                new.as_object().unwrap().keys().collect::<HashSet<_>>()
            );
        }
        // Every field in this fixture is present; null would hide a nonfinite
        // result rather than preserving a meaningful numerical measurement.
        assert!(!new_json.contains("null"));
        assert_eq!(
            serde_json::from_str::<DyeMeasurementRecord>(&new_json).unwrap(),
            current
        );
    }
}
