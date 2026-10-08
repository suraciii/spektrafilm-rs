//! Profile metadata and stored-array shape validation.

use super::{Profile, ProfileError};

pub(crate) fn validate_profile(profile: &Profile) -> Result<(), ProfileError> {
    let info = &profile.info;
    let data = &profile.data;
    for (field, value, allowed) in [
        (
            "type",
            info.film_type.as_str(),
            &["negative", "positive"][..],
        ),
        ("support", info.support.as_str(), &["film", "paper"][..]),
        ("stage", info.stage.as_str(), &["filming", "printing"][..]),
        ("use", info.usage.as_str(), &["still", "cine"][..]),
        (
            "antihalation",
            info.antihalation.as_str(),
            &["strong", "weak", "no"][..],
        ),
        (
            "channel_model",
            info.channel_model.as_str(),
            &["color", "bw"][..],
        ),
    ] {
        if !allowed.contains(&value) {
            return Err(ProfileError::Validation(format!(
                "unsupported {field} value {value:?}"
            )));
        }
    }
    for (field, value) in [
        ("reference_illuminant", info.reference_illuminant.as_str()),
        ("viewing_illuminant", info.viewing_illuminant.as_str()),
    ] {
        if !matches!(value, "D50" | "D55" | "D65" | "T" | "TH-KG3" | "K75P") {
            return Err(ProfileError::Validation(format!(
                "unsupported {field} value {value:?}"
            )));
        }
    }
    if data.wavelengths.is_empty() {
        return Err(ProfileError::Validation("wavelengths is empty".into()));
    }
    if data.log_exposure.is_empty() {
        return Err(ProfileError::Validation("log_exposure is empty".into()));
    }
    if data.density_curves.len() != data.log_exposure.len() {
        return Err(ProfileError::Validation(
            "density_curves length must match log_exposure length".into(),
        ));
    }
    let channels = if profile.is_bw() { 1 } else { 3 };
    if data.log_sensitivity.len() != data.wavelengths.len()
        || data.log_sensitivity.iter().any(|row| row.len() != channels)
    {
        return Err(ProfileError::Validation(format!(
            "log_sensitivity must be {}×{}",
            data.wavelengths.len(),
            channels
        )));
    }
    if data.channel_density.len() != data.wavelengths.len()
        || data.channel_density.iter().any(|row| row.len() != channels)
    {
        return Err(ProfileError::Validation(format!(
            "channel_density must be {}×{}",
            data.wavelengths.len(),
            channels
        )));
    }
    if data.base_density_rows.len() != data.wavelengths.len()
        || data.base_density_rows.iter().any(|row| row.is_empty())
    {
        return Err(ProfileError::Validation(
            "base_density must have one non-empty row per wavelength".into(),
        ));
    }
    if !data.midscale_neutral_density.is_empty()
        && data.midscale_neutral_density.len() != data.wavelengths.len()
    {
        return Err(ProfileError::Validation(
            "midscale_neutral_density length must match wavelengths".into(),
        ));
    }
    if profile.is_bw() {
        // B&W: one sensitivity/dye channel; density-curve columns index the
        // development-time family (validated against its length when present).
        if data
            .density_curves
            .iter()
            .any(|r| r.len() != data.development_time.len().max(1))
        {
            return Err(ProfileError::Validation(
                "bw profile density_curves columns must match development_time length".into(),
            ));
        }
        let n_times = data.development_time.len().max(1);
        if data
            .base_density_rows
            .iter()
            .any(|r| r.len() != 1 && r.len() != n_times)
        {
            return Err(ProfileError::Validation(
                "bw profile base_density columns must be 1 or match development_time".into(),
            ));
        }
    } else if data.density_curves.iter().any(|r| r.len() != 3) {
        return Err(ProfileError::Validation(
            "color profile density_curves must have 3 channels".into(),
        ));
    }
    Ok(())
}
