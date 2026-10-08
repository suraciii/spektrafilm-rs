//! Film and paper profiles: JSON models, persistence, render preparation, and validation.
//!
//! The private modules separate responsibilities while this facade preserves
//! the public profile API.

mod io;
mod model;
mod render;
mod validation;

pub use io::{
    list_profiles, load_profile, load_profile_by_name, save_processed_profile, save_profile,
};
pub use model::{DensityCurvesModel, Profile, ProfileData, ProfileInfo, ProfileMetadata};
pub use render::{development_time_index, resolve_for_render};
pub(crate) use validation::validate_profile;

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("loading profile {0}: {1}")]
    Io(String, std::io::Error),
    #[error("parsing profile {0}: {1}")]
    Parse(String, serde_json::Error),
    #[error("serializing profile {0}: {1}")]
    Serialize(String, serde_json::Error),
    #[error("invalid profile: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn data_profile(name: &str) -> Option<Profile> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("data/profiles/{name}.json"));
        if !path.exists() {
            eprintln!("Skipping test — profile not found at {}", path.display());
            return None;
        }
        Some(load_profile(&path).unwrap())
    }

    /// B&W profiles carry whole-field nulls, single-channel arrays, and a
    /// development-time family — all of which the loader must accept.
    #[test]
    fn loads_bw_development_time_family() {
        let Some(p) = data_profile("kodak_doublex") else {
            return;
        };
        assert!(p.is_bw());
        assert_eq!(p.data.development_time, vec![4.0, 5.0, 6.5, 9.0, 12.0]);
        assert!(p.data.log_sensitivity.iter().all(|r| r.len() == 1));
        assert!(p.data.density_curves.iter().all(|r| r.len() == 5));
        assert!(p.data.base_density_rows.iter().all(|r| r.len() == 5));
        // load_profile resolves the default (floor-middle) base column.
        assert_eq!(p.data.base_density.len(), 81);
        // Whole-field nulls parse as empty.
        assert!(p.data.midscale_neutral_density.is_empty());
        assert!(p.data.hanatos2025_adaptation_window_params.is_empty());
    }

    #[test]
    fn rejects_invalid_metadata_and_shape() {
        let Some(mut profile) = data_profile("kodak_portra_400") else {
            return;
        };
        profile.info.stage = "unknown".into();
        assert!(validate_profile(&profile).is_err());

        let mut profile = data_profile("kodak_portra_400").unwrap();
        profile.data.channel_density[0].pop();
        assert!(validate_profile(&profile).is_err());
    }

    /// Resolving collapses the family to the requested (nearest) time and
    /// broadcasts the single channel to the 3-channel engine layout with the
    /// `[dye, 0, 0]` spectral encoding.
    #[test]
    fn resolve_bw_selects_time_and_broadcasts() {
        let Some(p) = data_profile("kodak_doublex") else {
            return;
        };
        // 10.0 is nearest to 9.0 (index 3).
        let want_curve: Vec<f64> = p.data.density_curves.iter().map(|r| r[3]).collect();
        let r = resolve_for_render(p, Some(10.0));
        assert_eq!(r.data.development_time, vec![9.0]);
        assert!(
            r.data
                .log_sensitivity
                .iter()
                .all(|row| row.len() == 3 && row[0] == row[1] && row[1] == row[2])
        );
        for (row, want) in r.data.density_curves.iter().zip(&want_curve) {
            assert_eq!(row.len(), 3);
            assert_eq!(row[0], *want);
            assert_eq!(row[0], row[1]);
            assert_eq!(row[1], row[2]);
        }
        // Spectral dye encoding: [dye, 0, 0] so every integration computes
        // the upstream single-channel density · dye_spectrum.
        assert!(
            r.data
                .channel_density
                .iter()
                .all(|row| row.len() == 3 && row[1] == 0.0 && row[2] == 0.0)
        );
        // Layer curves: the chosen development-time column, replicated
        // across the 3 engine channels ([k][sublayer][channel]).
        assert!(r.data.density_curves_layers.iter().all(|row| {
            row.iter()
                .all(|l| l.len() == 3 && l[0] == l[1] && l[1] == l[2])
        }));
        let layers = r.density_curves_layers_f64();
        assert_eq!(layers.len(), 256);
        let v = r.data.density_curves_layers[0][0][0];
        assert_eq!(layers[0][0], [v, v, v]);
        let model = r.data.density_curves_model.as_ref().unwrap();
        assert_eq!(model.n_channels(), 3);
        assert_eq!(model.centers[0], model.centers[1]);
    }

    /// Colour profiles expose their layer tensor in the Python
    /// `[k, sublayer, channel]` layout through the f64 accessor.
    #[test]
    fn density_curves_layers_f64_maps_colour_layout() {
        let Some(p) = data_profile("kodak_portra_400") else {
            return;
        };
        let layers = p.density_curves_layers_f64();
        assert_eq!(layers.len(), p.data.density_curves_layers.len());
        assert_eq!(layers.len(), 256);
        for k in [0usize, 100, 255] {
            for sl in 0..3 {
                for ch in 0..3 {
                    assert_eq!(layers[k][sl][ch], p.data.density_curves_layers[k][sl][ch]);
                }
            }
        }
    }

    /// Colour profiles pass through `resolve_for_render` untouched.
    #[test]
    fn resolve_is_identity_for_colour() {
        let Some(p) = data_profile("kodak_gold_200") else {
            return;
        };
        let before = p.data.density_curves.clone();
        let r = resolve_for_render(p, Some(5.0));
        assert!(!r.is_bw());
        assert_eq!(r.data.density_curves, before);
    }

    #[test]
    fn test_load_kodak_portra_400() {
        // This test requires the data directory to be present
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data/profiles/kodak_portra_400.json");
        if !path.exists() {
            eprintln!("Skipping test — profile not found at {}", path.display());
            return;
        }
        let profile = load_profile(&path).unwrap();
        assert_eq!(profile.info.stock.as_deref(), Some("kodak_portra_400"));
        assert_eq!(profile.info.film_type, "negative");
        assert_eq!(profile.info.support, "film");
        assert_eq!(profile.data.wavelengths.len(), 81);
        assert_eq!(profile.data.log_exposure.len(), 256);
        assert_eq!(profile.data.density_curves.len(), 256);
    }

    #[test]
    fn save_profile_preserves_nulls_and_does_not_mutate_source() {
        let Some(mut profile) = data_profile("kodak_portra_400") else {
            return;
        };
        let original_stock = profile.info.stock.clone();
        profile.data.midscale_neutral_density[0] = f64::NAN;
        let root =
            std::env::temp_dir().join(format!("spektrafilm-profile-save-{}", std::process::id()));
        let path = save_profile(&root, &profile, "_copy").unwrap();
        assert_eq!(profile.info.stock, original_stock);
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["info"]["stock"], "kodak_portra_400_copy");
        assert!(json["data"]["midscale_neutral_density"][0].is_null());
        let loaded = load_profile(&path).unwrap();
        assert!(loaded.data.midscale_neutral_density[0].is_nan());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fitted_models_replace_stale_sampled_curves_and_clear_single_layer_cache() {
        let directory =
            std::env::temp_dir().join(format!("spektrafilm-profile-model-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for model_type in ["norm_cdfs", "sept_norm_cdfs"] {
            let path = directory.join(format!("{model_type}.json"));
            let value = serde_json::json!({
                "metadata":{},"info":{},"data":{
                    "wavelengths":[380.0],
                    "log_sensitivity":[[0.0,0.0,0.0]],
                    "channel_density":[[0.0,0.0,0.0]],
                    "base_density":[[0.0]],
                    "log_exposure":[-4.0,0.0,4.0],
                    "density_curves":[[0.2,0.2,0.2],[0.8,0.8,0.8],[1.8,1.8,1.8]],
                    "density_curves_layers":[[[0.2,0.2,0.2]]],
                    "density_curves_model":{"model_type":model_type,"centers":[[0.0],[0.0],[0.0]],"amplitudes":[[2.0],[2.0],[2.0]],"sigmas":[[1.0],[1.0],[1.0]],"alphas":[[0.6],[0.6],[0.6]]}
                }
            });
            std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            let profile = load_profile(&path).unwrap();
            assert_eq!(profile.data.density_curves[1], vec![1.0; 3]);
            assert!(profile.data.density_curves[0].iter().all(|v| *v < 0.001));
            assert!(profile.data.density_curves[2].iter().all(|v| *v > 1.999));
            assert!(profile.data.density_curves_layers.is_empty());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
