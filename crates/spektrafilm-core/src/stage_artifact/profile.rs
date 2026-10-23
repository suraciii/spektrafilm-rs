//! Embedded material dependencies, including the resolved base-density column
//! omitted by the library profile's external wire representation.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::profile::Profile;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedProfile {
    pub profile: Profile,
    #[serde(with = "nullable_spectrum")]
    pub resolved_base_density: Vec<f64>,
    pub sha256: String,
    pub source_sha256: String,
}

impl EmbeddedProfile {
    pub fn new(profile: Profile) -> Result<Self> {
        let source_sha256 = Self::source_identity(&profile)?;
        Self::build(profile, source_sha256)
    }

    pub fn with_source(profile: Profile, source: &Profile) -> Result<Self> {
        Self::build(profile, Self::source_identity(source)?)
    }

    fn build(profile: Profile, source_sha256: String) -> Result<Self> {
        let mut embedded = Self {
            resolved_base_density: profile.data.base_density.clone(),
            profile,
            sha256: String::new(),
            source_sha256,
        };
        embedded.sha256 = embedded.identity()?;
        embedded.validate()?;
        Ok(embedded)
    }

    pub fn source_identity(profile: &Profile) -> Result<String> {
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(profile)?)))
    }

    pub fn matches_source(&self, profile: &Profile) -> Result<bool> {
        Ok(Self::source_identity(profile)? == self.source_sha256)
    }

    fn identity(&self) -> Result<String> {
        let mut digest = Sha256::new();
        digest.update(serde_json::to_vec(&self.profile)?);
        digest.update(serde_json::to_vec(&self.resolved_base_density)?);
        digest.update(self.source_sha256.as_bytes());
        Ok(format!("{:x}", digest.finalize()))
    }

    pub(super) fn validate(&self) -> Result<()> {
        super::payload::validate_digest(&self.sha256)?;
        super::payload::validate_digest(&self.source_sha256)?;
        ensure!(self.identity()? == self.sha256, "embedded profile SHA-256 mismatch");
        validate_embedded_profile(&self.profile)?;
        let data = &self.profile.data;
        ensure!(self.resolved_base_density.len() == data.wavelengths.len(), "embedded profile is missing resolved base density");
        for (name, values) in [
            ("wavelengths", data.wavelengths.as_slice()),
            ("log_exposure", data.log_exposure.as_slice()),
            ("development_time", data.development_time.as_slice()),
            ("adaptation_window", data.hanatos2025_adaptation_window_params.as_slice()),
        ] {
            ensure!(values.iter().all(|v| v.is_finite()), "embedded profile {name} contains non-finite values");
        }
        ensure!(self.profile.info.log_sensitivity_density_over_min.is_finite(), "embedded profile sensitivity density is non-finite");
        for matrix in [&data.log_sensitivity, &data.density_curves, &data.hanatos2025_adaptation_surface_params] {
            ensure!(matrix.iter().flatten().all(|v| v.is_finite()), "embedded profile required matrix contains non-finite values");
        }
        ensure!(data.density_curves_layers.iter().flatten().flatten().all(|v| v.is_finite()), "embedded profile layer curves contain non-finite values");
        if !data.density_curves_layers.is_empty() {
            let channels = data.density_curves.first().map_or(0, Vec::len);
            let layers = data.density_curves_layers[0].len();
            ensure!(layers > 0 && data.density_curves_layers.len() == data.log_exposure.len()
                && data.density_curves_layers.iter().all(|rows| rows.len() == layers
                    && rows.iter().all(|row| row.len() == channels)), "embedded material layer-curve shape is inconsistent");
        }
        if let Some(model) = &data.density_curves_model {
            let rows = model.centers.len();
            let layers = model.centers.first().map_or(0, Vec::len);
            ensure!(rows > 0 && layers > 0
                && [&model.centers, &model.amplitudes, &model.sigmas].iter().all(|matrix| matrix.len() == rows && matrix.iter().all(|row| row.len() == layers))
                && model.alphas.as_ref().is_none_or(|matrix| matrix.len() == rows && matrix.iter().all(|row| row.len() == layers)), "embedded material curve-model shape is inconsistent");
            for matrix in [&model.centers, &model.amplitudes, &model.sigmas] {
                ensure!(matrix.iter().flatten().all(|v| v.is_finite()), "embedded profile curve model contains non-finite values");
            }
            ensure!(model.alphas.as_ref().is_none_or(|m| m.iter().flatten().all(|v| v.is_finite())), "embedded profile curve model alphas are non-finite");
        }
        // Spectral no-measurement values are optional profile data, not
        // non-finite simulation state. Preserve null/NaN; reject infinities.
        ensure!(data.channel_density.iter().flatten().chain(data.midscale_neutral_density.iter())
            .chain(data.base_density_rows.iter().flatten()).chain(self.resolved_base_density.iter())
            .all(|v| !v.is_infinite()), "embedded profile spectral measurement contains infinity");
        Ok(())
    }

    pub(super) fn restore(&mut self) {
        self.profile.data.base_density.clone_from(&self.resolved_base_density);
    }
}

// Profile's ordinary library decoder intentionally accepts additional fields.
// Generated stage persistence does not: enforce its exact nested wire schema
// here without changing the unrelated external profile language.
pub(super) fn validate_profile_fields(value: &serde_json::Value) -> Result<()> {
    fn object<'a>(value: &'a serde_json::Value, allowed: &[&str]) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
        let map = value.as_object().context("embedded profile record must be an object")?;
        for key in map.keys() { ensure!(allowed.contains(&key.as_str()), "unknown embedded profile field {key:?}"); }
        for key in allowed { ensure!(map.contains_key(*key), "missing embedded profile field {key:?}"); }
        Ok(map)
    }
    let profile = object(value, &["metadata", "info", "data"])?;
    object(&profile["metadata"], &["version", "copyright", "created", "license", "citation", "datasource"])?;
    object(&profile["info"], &["stock", "name", "type", "support", "stage", "use", "antihalation", "target_print", "channel_model", "densitometer", "log_sensitivity_density_over_min", "reference_illuminant"])?;
    let data = object(&profile["data"], &["wavelengths", "log_sensitivity", "hanatos2025_adaptation_window_params", "hanatos2025_adaptation_surface_params", "channel_density", "base_density", "midscale_neutral_density", "log_exposure", "density_curves", "density_curves_layers", "density_curves_model", "development_time"])?;
    if !data["density_curves_model"].is_null() {
        object(&data["density_curves_model"], &["model_type", "centers", "amplitudes", "sigmas", "alphas"])?;
    }
    // Nulls are meaningful only in named optional spectral measurement fields;
    // the external decoder would otherwise silently coerce them to zeros.
    for key in ["wavelengths", "log_sensitivity", "hanatos2025_adaptation_window_params", "hanatos2025_adaptation_surface_params", "log_exposure", "density_curves", "density_curves_layers", "development_time"] {
        reject_null(&data[key], key)?;
    }
    Ok(())
}

fn reject_null(value: &serde_json::Value, path: &str) -> Result<()> {
    ensure!(!value.is_null(), "missing required finite embedded profile state {path}");
    if let Some(values) = value.as_array() { for value in values { reject_null(value, path)?; } }
    Ok(())
}

pub(super) mod nullable_spectrum {
    use serde::{Deserialize, Serialize};

    pub fn serialize<S: serde::Serializer>(values: &[f64], serializer: S) -> Result<S::Ok, S::Error> {
        values.iter().map(|v| v.is_finite().then_some(*v)).collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<f64>, D::Error> {
        Ok(Vec::<Option<f64>>::deserialize(deserializer)?.into_iter().map(|v| v.unwrap_or(f64::NAN)).collect())
    }
}

fn validate_embedded_profile(profile: &Profile) -> Result<()> {
    let data = &profile.data;
    if profile.is_bw() && data.log_sensitivity.first().is_some_and(|row| row.len() == 3) {
        // resolve_for_render broadcasts the owning monochrome record to
        // execution lanes, retaining channel_model=bw. The library validator
        // instead owns unprepared development-time families.
        ensure!(["negative", "positive"].contains(&profile.info.film_type.as_str())
            && ["film", "paper"].contains(&profile.info.support.as_str())
            && ["filming", "printing"].contains(&profile.info.stage.as_str())
            && ["still", "cine"].contains(&profile.info.usage.as_str())
            && ["strong", "weak", "no"].contains(&profile.info.antihalation.as_str())
            && ["D50", "D55", "D65", "T", "TH-KG3", "K75P"].contains(&profile.info.reference_illuminant.as_str()), "invalid resolved monochrome material metadata");
        ensure!(!data.wavelengths.is_empty() && !data.log_exposure.is_empty(), "resolved monochrome material is missing spectral/curve axes");
        ensure!(data.development_time.len() <= 1, "resolved monochrome material retains an unresolved development-time family");
        ensure!(data.density_curves.len() == data.log_exposure.len()
            && data.density_curves.iter().all(|row| row.len() == 3 && row[0] == row[1] && row[0] == row[2]), "resolved monochrome density curves are not replicated record lanes");
        ensure!(data.log_sensitivity.len() == data.wavelengths.len()
            && data.log_sensitivity.iter().all(|row| row.len() == 3 && row[0] == row[1] && row[0] == row[2]), "resolved monochrome sensitivity is not replicated record lanes");
        ensure!(data.channel_density.len() == data.wavelengths.len()
            && data.channel_density.iter().all(|row| row.len() == 3 && row[1] == 0.0 && row[2] == 0.0), "resolved monochrome spectral density has invalid channel semantics");
        ensure!(data.base_density_rows.len() == data.wavelengths.len() && data.base_density_rows.iter().all(|row| row.len() == 1), "resolved monochrome material base density shape is invalid");
        ensure!(data.midscale_neutral_density.is_empty() || data.midscale_neutral_density.len() == data.wavelengths.len(), "resolved monochrome neutral density shape is invalid");
    } else {
        crate::profile::validate_profile(profile).context("invalid embedded material profile")?;
    }
    Ok(())
}
