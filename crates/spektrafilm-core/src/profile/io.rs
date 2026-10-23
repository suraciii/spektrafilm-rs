//! Profile loading, sampled-curve repair, saving, and directory listing.

use std::path::Path;

use super::render::base_density_column;
use super::{
    Profile, ProfileError, ProfileValidationError, development_time_index, validate_profile,
};

/// Load a profile from a JSON file on disk.
pub fn load_profile(path: &Path) -> Result<Profile, ProfileError> {
    let bytes = std::fs::read(path).map_err(|e| ProfileError::Io(path.display().to_string(), e))?;
    load_profile_bytes(&bytes, &path.display().to_string())
}

/// Parse and resolve exactly the bytes whose content identity a caller retains.
pub fn load_profile_bytes(bytes: &[u8], source: &str) -> Result<Profile, ProfileError> {
    let mut profile: Profile =
        serde_json::from_slice(bytes).map_err(|e| ProfileError::Parse(source.to_owned(), e))?;
    validate_profile(&profile)?;
    // Resolve the default base+fog column so the field is usable straight
    // after load. `resolve_for_render` re-selects it for B&W development-time
    // families when a specific time is requested.
    let idx = development_time_index(&profile.data.development_time, None);
    profile.data.base_density = base_density_column(&profile.data.base_density_rows, idx);
    let curves_stale = true;
    let model = profile.data.density_curves_model.clone();
    if let Some(model) = model.as_ref() {
        let expected_exposures = profile.data.log_exposure.len();
        let expected_layers = model.n_layers();
        let layers_stale = expected_layers > 1
            && (profile.data.density_curves_layers.len() != expected_exposures
                || profile.data.density_curves_layers.iter().any(|row| {
                    row.len() != expected_layers
                        || row.iter().any(|layer| {
                            layer.len() != 3 || layer.iter().any(|value| !value.is_finite())
                        })
                }));
        if curves_stale {
            profile.data.density_curves =
                spektrafilm_model::density_curves::evaluate_density_curves_detailed(
                    &profile.data.log_exposure,
                    &model.model_type,
                    &model.centers,
                    &model.amplitudes,
                    &model.sigmas,
                    model.alphas.as_deref(),
                    profile.is_positive(),
                )
                .map_err(|error| {
                    ProfileError::Validation(ProfileValidationError::DensityModel(error))
                })?;
        }
        if expected_layers > 1 && (curves_stale || layers_stale) {
            profile.data.density_curves_layers =
                spektrafilm_model::density_curves::evaluate_density_curves_layers_detailed(
                    &profile.data.log_exposure,
                    &model.model_type,
                    &model.centers,
                    &model.amplitudes,
                    &model.sigmas,
                    model.alphas.as_deref(),
                    profile.is_positive(),
                )
                .map_err(|error| {
                    ProfileError::Validation(ProfileValidationError::DensityModel(error))
                })?;
        } else if expected_layers <= 1 && curves_stale {
            profile.data.density_curves_layers.clear();
        }
    }
    Ok(profile)
}

/// Load a profile by stock name from a data directory.
pub fn load_profile_by_name(data_dir: &Path, stock: &str) -> Result<Profile, ProfileError> {
    let path = data_dir.join("profiles").join(format!("{stock}.json"));
    load_profile(&path)
}

/// Save a profile under `data_dir/profiles`, preserving `NaN` as JSON `null`.
///
/// The profile is cloned before applying `suffix`; the caller's profile is
/// never mutated. The returned path is the written profile filename.
pub fn save_profile(
    data_dir: &Path,
    profile: &Profile,
    suffix: &str,
) -> Result<std::path::PathBuf, ProfileError> {
    if suffix.contains(['/', '\\']) {
        return Err(ProfileError::Validation(ProfileValidationError::Suffix(
            suffix.to_owned(),
        )));
    }
    let mut saved = profile.clone();
    let stock = saved.info.stock.as_mut().ok_or(ProfileError::Validation(
        ProfileValidationError::MissingStock,
    ))?;
    stock.push_str(suffix);
    let path = data_dir.join("profiles").join(format!("{stock}.json"));
    save_profile_to(&path, &saved)?;
    Ok(path)
}

/// Save a profile to an explicit path, validating it first.
///
/// The parent directory is created when missing, and the profile is validated
/// before anything is written.
pub fn save_profile_to(path: &Path, profile: &Profile) -> Result<(), ProfileError> {
    validate_profile(profile)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| ProfileError::Io(path.display().to_string(), e))?;
    }
    let json = serde_json::to_vec_pretty(profile)
        .map_err(|e| ProfileError::Serialize(path.display().to_string(), e))?;
    std::fs::write(path, json).map_err(|e| ProfileError::Io(path.display().to_string(), e))?;
    Ok(())
}

/// Compatibility spelling for callers that used the upstream split API.
pub fn save_processed_profile(
    data_dir: &Path,
    profile: &Profile,
    suffix: &str,
) -> Result<std::path::PathBuf, ProfileError> {
    save_profile(data_dir, profile, suffix)
}

/// Return bundled profile slugs in deterministic order.
pub fn list_profiles(data_dir: &Path) -> Result<Vec<String>, ProfileError> {
    let root = data_dir.join("profiles");
    let mut names = Vec::new();
    for entry in
        std::fs::read_dir(&root).map_err(|e| ProfileError::Io(root.display().to_string(), e))?
    {
        let entry = entry.map_err(|e| ProfileError::Io(root.display().to_string(), e))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json")
            && let Some(stem) = path.file_stem().and_then(|name| name.to_str())
        {
            names.push(stem.to_owned());
        }
    }
    names.sort();
    Ok(names)
}
