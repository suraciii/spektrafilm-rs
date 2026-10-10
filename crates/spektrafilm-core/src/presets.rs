//! Portable look presets: strict look-owned schema and transactional resolution.
use crate::params::{
    DiffusionFilterParams, EnlargerParams, FilmRenderingParams, PrintRenderingParams,
    RuntimeParams, ScannerParams,
};
use crate::params_builder::apply_film_specifics;
use crate::profile::{self, Profile};
use crate::suggest::{MAX_SUGGESTIONS, closest, error_suffix};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const LOOK_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRef {
    pub stock: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub content_identity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookProvenance {
    pub implementation_version: String,
    pub model_version: String,
}

/// Parameters owned by a look. Every group is explicit so later stock-default
/// changes cannot silently rewrite a saved preset.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LookParameters {
    pub enlarger: EnlargerParams,
    pub scanner: ScannerParams,
    pub film_render: FilmRenderingParams,
    pub print_render: PrintRenderingParams,
    pub camera_diffusion_filter: DiffusionFilterParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookPreset {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub film_profile: ProfileRef,
    pub print_profile: ProfileRef,
    pub parameters: LookParameters,
    pub provenance: LookProvenance,
}

#[derive(Debug, Clone)]
pub struct ResolvedLookPreset {
    pub film_name: String,
    pub print_name: String,
    pub default_route: String,
    pub params: RuntimeParams,
    pub film: Profile,
    pub print: Profile,
}

impl LookPreset {
    pub fn from_json_str(json: &str) -> Result<Self, String> {
        Self::from_carrier(json, false)
    }

    pub fn to_json(&self) -> Result<String, String> {
        validate_document(self)?;
        validate_look_values(&self.parameters)?;
        serde_json::to_string_pretty(self).map_err(|e| format!("serializing look preset: {e}"))
    }

    pub fn from_toml_str(toml: &str) -> Result<Self, String> {
        Self::from_carrier(toml, true)
    }

    fn from_carrier(text: &str, is_toml: bool) -> Result<Self, String> {
        let value = crate::params::sources::parse_carrier(text, is_toml)?;
        let preset: Self =
            serde_json::from_value(value).map_err(|e| format!("invalid look preset: {e}"))?;
        validate_document(&preset)?;
        validate_look_values(&preset.parameters)?;
        Ok(preset)
    }

    pub fn to_toml(&self) -> Result<String, String> {
        validate_document(self)?;
        validate_look_values(&self.parameters)?;
        toml::to_string_pretty(self).map_err(|e| format!("serializing look preset TOML: {e}"))
    }

    /// Resolve without mutating the caller's active runtime context.
    pub fn resolve(
        &self,
        data_dir: &Path,
        current_params: &RuntimeParams,
    ) -> Result<ResolvedLookPreset, String> {
        validate_document(self)?;
        validate_look_values(&self.parameters)?;
        let film_identity = profile_content_identity_if_requested(data_dir, &self.film_profile)?;
        let print_identity = profile_content_identity_if_requested(data_dir, &self.print_profile)?;
        let film = profile::load_profile_by_name(data_dir, &self.film_profile.stock)
            .map_err(|e| format!("loading film profile {:?}: {e}", self.film_profile.stock))?;
        let print = profile::load_profile_by_name(data_dir, &self.print_profile.stock)
            .map_err(|e| format!("loading print profile {:?}: {e}", self.print_profile.stock))?;
        validate_profile_ref("film", &self.film_profile, &film, film_identity.as_deref())?;
        validate_profile_ref(
            "print",
            &self.print_profile,
            &print,
            print_identity.as_deref(),
        )?;

        // Stock initialization is independent of preview/debug constraints.
        // The complete look replaces its owned controls, preserving all context.
        let mut params = current_params.clone();
        apply_film_specifics(&mut params, &film);
        params.enlarger = self.parameters.enlarger.clone();
        params.neutral_print_filters_protected = [true; 3];
        params.scanner = self.parameters.scanner.clone();
        params.film_render = self.parameters.film_render.clone();
        params.print_render = self.parameters.print_render.clone();
        params.camera.diffusion_filter = self.parameters.camera_diffusion_filter.clone();
        let default_route = workflow_default(&film);
        params.workflow.route = default_route.clone();
        params.io.scan_film = default_route == "input > film > scan";
        params.validate()?;
        Ok(ResolvedLookPreset {
            film_name: self.film_profile.stock.clone(),
            print_name: self.print_profile.stock.clone(),
            default_route,
            params,
            film,
            print,
        })
    }

    pub fn capture(
        name: impl Into<String>,
        id: impl Into<String>,
        film: impl Into<String>,
        print: impl Into<String>,
        params: &RuntimeParams,
        provenance: LookProvenance,
    ) -> Self {
        Self {
            schema_version: LOOK_SCHEMA_VERSION,
            id: id.into(),
            name: name.into(),
            author: None,
            description: None,
            film_profile: ProfileRef {
                stock: film.into(),
                version: None,
                content_identity: None,
            },
            print_profile: ProfileRef {
                stock: print.into(),
                version: None,
                content_identity: None,
            },
            parameters: LookParameters {
                enlarger: params.enlarger.clone(),
                scanner: params.scanner.clone(),
                film_render: params.film_render.clone(),
                print_render: params.print_render.clone(),
                camera_diffusion_filter: params.camera.diffusion_filter.clone(),
            },
            provenance,
        }
    }
}

fn validate_look_values(look: &LookParameters) -> Result<(), String> {
    let mut params = RuntimeParams::default();
    params.enlarger = look.enlarger.clone();
    params.scanner = look.scanner.clone();
    params.film_render = look.film_render.clone();
    params.print_render = look.print_render.clone();
    params.camera.diffusion_filter = look.camera_diffusion_filter.clone();
    crate::params::sources::validate_finite(&params)?;
    params.validate()
}

/// Extension-selected files or exact immutable built-in IDs, never GUI names.
pub fn load_selector(selector: &str, _data_dir: &Path) -> Result<LookPreset, String> {
    if selector.ends_with(".toml") || selector.ends_with(".json") {
        let text = std::fs::read_to_string(selector)
            .map_err(|e| format!("reading preset file {selector:?}: {e}"))?;
        return if selector.ends_with(".toml") {
            LookPreset::from_toml_str(&text)
        } else {
            LookPreset::from_json_str(&text)
        }
        .map_err(|e| format!("preset file {selector:?}: {e}"));
    }
    if selector.contains('/') || selector.contains('\\') {
        return Err(format!(
            "invalid preset ID {selector:?}: path separators require a .toml or .json file"
        ));
    }
    let mut presets = builtin_look_presets();
    if let Some(index) = presets.iter().position(|preset| preset.id == selector) {
        return Ok(presets.remove(index));
    }
    Err(format!(
        "unknown built-in preset ID {selector:?}; {}",
        error_suffix(
            &closest(
                selector,
                presets.into_iter().map(|preset| preset.id),
                MAX_SUGGESTIONS
            ),
            "use spektrafilm preset list to list built-in IDs"
        )
    ))
}

pub fn suggest_preset_ids(input: &str) -> Vec<String> {
    closest(
        input,
        builtin_look_presets().into_iter().map(|preset| preset.id),
        MAX_SUGGESTIONS,
    )
}
fn validate_document(preset: &LookPreset) -> Result<(), String> {
    if preset.schema_version != LOOK_SCHEMA_VERSION {
        return Err(format!(
            "unsupported look preset schema version {}",
            preset.schema_version
        ));
    }
    if preset.id.trim().is_empty() || preset.name.trim().is_empty() {
        return Err("look preset id and name must not be empty".into());
    }
    if preset.provenance.implementation_version.trim().is_empty()
        || preset.provenance.model_version.trim().is_empty()
    {
        return Err("look preset provenance versions must not be empty".into());
    }
    validate_stock_id("film", &preset.film_profile.stock)?;
    validate_content_identity("film", &preset.film_profile)?;
    validate_stock_id("print", &preset.print_profile.stock)?;
    validate_content_identity("print", &preset.print_profile)?;
    Ok(())
}

fn validate_stock_id(kind: &str, stock: &str) -> Result<(), String> {
    if stock.is_empty()
        || !stock
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!("invalid {kind} profile stock identifier {stock:?}"));
    }
    Ok(())
}

fn validate_content_identity(kind: &str, reference: &ProfileRef) -> Result<(), String> {
    let Some(identity) = reference.content_identity.as_deref() else {
        return Ok(());
    };
    let bytes = identity.as_bytes();
    let valid = bytes.len() == 71
        && bytes.starts_with(b"sha256-")
        && bytes[7..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
    if !valid {
        return Err(format!(
            "invalid {kind} profile content identity {identity:?}; expected sha256- followed by 64 lowercase hexadecimal characters"
        ));
    }
    Ok(())
}
fn profile_content_identity_if_requested(
    data_dir: &Path,
    reference: &ProfileRef,
) -> Result<Option<String>, String> {
    let Some(expected) = &reference.content_identity else {
        return Ok(None);
    };
    let path = data_dir
        .join("profiles")
        .join(format!("{}.json", reference.stock));
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("reading profile content for {:?}: {e}", reference.stock))?;
    let actual = format!("sha256-{:x}", Sha256::digest(bytes));
    if actual != *expected {
        return Err(format!(
            "profile {:?} content identity mismatch: expected {expected:?}, got {actual:?}",
            reference.stock
        ));
    }
    Ok(Some(actual))
}

fn validate_profile_ref(
    kind: &str,
    reference: &ProfileRef,
    profile: &Profile,
    content_identity: Option<&str>,
) -> Result<(), String> {
    if profile.info.stock.as_deref() != Some(reference.stock.as_str()) {
        return Err(format!(
            "{kind} profile {:?} does not declare the requested stock identifier",
            reference.stock
        ));
    }
    let valid = if kind == "film" {
        profile.is_film() && profile.is_filming()
    } else {
        profile.is_paper() && profile.is_printing()
    };
    if !valid {
        return Err(format!(
            "{kind} profile {:?} has incompatible support/stage",
            reference.stock
        ));
    }
    if let Some(version) = &reference.version {
        if &profile.metadata.version != version {
            return Err(format!(
                "{kind} profile {:?} version mismatch: expected {version:?}, got {:?}",
                reference.stock, profile.metadata.version
            ));
        }
    }
    if let Some(expected) = content_identity {
        if Some(expected) != reference.content_identity.as_deref() {
            return Err(format!(
                "{kind} profile {:?} content identity was not verified",
                reference.stock
            ));
        }
    }
    Ok(())
}

pub fn workflow_default(film: &Profile) -> String {
    if film.is_positive() {
        "input > film > scan".into()
    } else {
        "input > film > print > scan".into()
    }
}

fn classic_parameters(film: &str) -> LookParameters {
    let mut params = RuntimeParams::default();
    let grain = &mut params.film_render.grain;
    grain.rms_granularity = match film {
        "kodak_gold_200" => [5.0; 3],
        "kodak_portra_400" => [4.5; 3],
        "kodak_ektar_100" => [4.0; 3],
        _ => [8.0; 3],
    };
    grain.density_min = [0.03; 3];
    grain.uniformity = if film == "kodak_portra_400" {
        [0.97, 0.99, 0.97]
    } else {
        [0.97; 3]
    };
    grain.particle_scale_sublayers = [1.0, 0.4, 0.25];

    let couplers = &mut params.film_render.dir_couplers;
    match film {
        "fujifilm_velvia_100" => {
            couplers.gamma_samelayer_rgb = [0.108, 0.072, 0.054];
            couplers.gamma_interlayer_r_to_gb = [0.0707, 0.2367];
            couplers.gamma_interlayer_g_to_rb = [0.0119, 0.0380];
            couplers.gamma_interlayer_b_to_rg = [0.0111, 0.1413];
        }
        "fujifilm_provia_100f" => {
            couplers.gamma_samelayer_rgb = [0.2786, 0.1257, 0.3480];
            couplers.gamma_interlayer_r_to_gb = [0.1448, 0.2003];
            couplers.gamma_interlayer_g_to_rb = [0.0516, 0.1250];
            couplers.gamma_interlayer_b_to_rg = [0.0518, 0.3089];
        }
        _ => {
            couplers.gamma_samelayer_rgb = [0.5159, 0.5934, 0.2829];
            couplers.gamma_interlayer_r_to_gb = [0.4032, 0.2488];
            couplers.gamma_interlayer_g_to_rb = [0.2227, 0.4340];
            couplers.gamma_interlayer_b_to_rg = [0.1829, 0.1799];
        }
    }

    let halation = &mut params.film_render.halation;
    halation.halation_first_sigma_um = [65.0; 3];
    halation.halation_strength = if film == "kodak_gold_200" {
        [0.08, 0.02, 0.0]
    } else {
        [0.015, 0.005, 0.0]
    };
    params.enlarger.c_filter_neutral = 0.0;
    (
        params.enlarger.m_filter_neutral,
        params.enlarger.y_filter_neutral,
    ) = match film {
        "kodak_gold_200" => (58.4153, 55.5983),
        "kodak_portra_400" => (51.8405, 51.2543),
        "kodak_ektar_100" => (73.6153, 59.8118),
        _ => (50.0, 50.0),
    };
    LookParameters {
        enlarger: params.enlarger,
        scanner: params.scanner,
        film_render: params.film_render,
        print_render: params.print_render,
        camera_diffusion_filter: params.camera.diffusion_filter,
    }
}

fn builtin(id: &str, name: &str, film: &str, print: &str) -> LookPreset {
    LookPreset {
        schema_version: LOOK_SCHEMA_VERSION,
        id: id.into(),
        name: name.into(),
        author: Some("SpektraFilm".into()),
        description: Some("SpektraFilm classic stock look".into()),
        film_profile: ProfileRef {
            stock: film.into(),
            version: Some("0.3.4".into()),
            content_identity: None,
        },
        print_profile: ProfileRef {
            stock: print.into(),
            version: Some("0.3.4".into()),
            content_identity: None,
        },
        parameters: classic_parameters(film),
        provenance: LookProvenance {
            implementation_version: "0.3.4".into(),
            model_version: "0.3.4".into(),
        },
    }
}

/// Immutable classic presets. Returned values are owned clones and cannot
/// mutate the built-in definitions.
pub fn builtin_look_presets() -> Vec<LookPreset> {
    vec![
        builtin(
            "classic-kodak-gold-200",
            "Kodak Gold 200",
            "kodak_gold_200",
            "kodak_portra_endura",
        ),
        builtin(
            "classic-kodak-portra-400",
            "Kodak Portra 400",
            "kodak_portra_400",
            "kodak_portra_endura",
        ),
        builtin(
            "classic-kodak-ektar-100",
            "Kodak Ektar 100",
            "kodak_ektar_100",
            "kodak_portra_endura",
        ),
        builtin(
            "classic-fujifilm-velvia-100",
            "Fujifilm Velvia 100",
            "fujifilm_velvia_100",
            "fujifilm_crystal_archive_typeii",
        ),
        builtin(
            "classic-fujifilm-provia-100f",
            "Fujifilm Provia 100F",
            "fujifilm_provia_100f",
            "fujifilm_crystal_archive_typeii",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_preset_ids_suggest_builtin_ids_or_discovery() {
        assert_eq!(
            suggest_preset_ids("classic-kodak-gold-20"),
            ["classic-kodak-gold-200"]
        );
        let error = load_selector("classic-kodak-gold-20", Path::new("data")).unwrap_err();
        assert!(error.contains("did you mean classic-kodak-gold-200?"));
        assert!(suggest_preset_ids("unrelated").is_empty());
        assert!(
            load_selector("unrelated", Path::new("data"))
                .unwrap_err()
                .contains("spektrafilm preset list")
        );
    }

    #[test]
    fn toml_roundtrip_preserves_optional_and_precise_controls() {
        let mut preset = builtin_look_presets().remove(0);
        preset.parameters.film_render.grain.v2_amount = None;
        preset.parameters.film_render.chemistry.gamma_factor = 1.2345678901234567;
        let text = preset.to_toml().unwrap();
        assert!(!text.contains("v2_amount"));
        let restored = LookPreset::from_toml_str(&text).unwrap();
        assert_eq!(restored.parameters.film_render.grain.v2_amount, None);
        assert_eq!(
            restored.parameters.film_render.chemistry.gamma_factor,
            preset.parameters.film_render.chemistry.gamma_factor
        );
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(preset).unwrap()
        );
    }

    #[test]
    fn carriers_reject_duplicate_keys_and_f32_overflow() {
        let preset = builtin_look_presets().remove(0);
        let mut value = serde_json::to_value(&preset).unwrap();
        value["parameters"]["enlarger"]["print_exposure"] = serde_json::json!(1e100);
        assert!(LookPreset::from_json_str(&value.to_string()).is_err());
        let duplicate = preset.to_json().unwrap().replacen(
            "\"schema_version\": 1",
            "\"schema_version\": 1, \"schema_version\": 1",
            1,
        );
        assert!(LookPreset::from_json_str(&duplicate).is_err());
        let text = preset
            .to_toml()
            .unwrap()
            .replace("print_exposure = 1.0", "print_exposure = inf");
        assert!(LookPreset::from_toml_str(&text).is_err());
    }

    #[test]
    fn selectors_are_exact_and_missing_files_do_not_fall_back() {
        assert!(load_selector("Kodak Gold 200", Path::new("data")).is_err());
        assert!(load_selector("classic-kodak-gold-200.toml", Path::new("data")).is_err());
        assert!(load_selector("folder/classic-kodak-gold-200", Path::new("data")).is_err());
        assert_eq!(
            load_selector("classic-kodak-gold-200", Path::new("data"))
                .unwrap()
                .id,
            "classic-kodak-gold-200"
        );
    }

    #[test]
    fn version_one_materializes_omitted_look_controls() {
        let json = r#"{"schema_version":1,"id":"x","name":"X","film_profile":{"stock":"f"},"print_profile":{"stock":"p"},"parameters":{"enlarger":{},"scanner":{},"film_render":{},"print_render":{},"camera_diffusion_filter":{}},"provenance":{"implementation_version":"x","model_version":"x"}}"#;
        let preset = LookPreset::from_json_str(json).unwrap();
        assert_eq!(preset.parameters.enlarger.y_filter_neutral, 55.0);
        let materialized = preset.to_toml().unwrap();
        assert!(materialized.contains("y_filter_neutral"));
        assert!(materialized.contains("gamma_factor"));
        let null = json.replace(
            "\"film_render\":{}",
            "\"film_render\":{\"grain\":{\"v2_amount\":null}}",
        );
        let restored = LookPreset::from_json_str(&null).unwrap();
        assert_eq!(restored.parameters.film_render.grain.v2_amount, None);
    }

    fn provenance() -> LookProvenance {
        LookProvenance {
            implementation_version: "test".into(),
            model_version: "test".into(),
        }
    }

    #[test]
    fn route_is_rejected_as_unknown() {
        let json = r#"{"schema_version":1,"id":"x","name":"X","film_profile":{"stock":"f"},"print_profile":{"stock":"p"},"parameters":{"enlarger":{},"scanner":{},"film_render":{},"print_render":{},"camera_diffusion_filter":{}},"provenance":{"implementation_version":"x","model_version":"x"},"route":"input"}"#;
        assert!(LookPreset::from_json_str(json).is_err());
    }

    #[test]
    fn content_identity_requires_canonical_sha256() {
        let mut preset = builtin_look_presets()[0].clone();
        preset.film_profile.content_identity = Some("sha256-not-a-hash".into());
        let json = serde_json::to_string(&preset).unwrap();
        let error = LookPreset::from_json_str(&json).unwrap_err();
        assert!(error.contains("content identity"));
    }

    #[test]
    fn capture_roundtrip_preserves_f64() {
        let mut params = RuntimeParams::default();
        params.film_render.chemistry.gamma_factor = 1.234567890123;
        params.film_render.halation.halation_strength = [0.765432109876; 3];
        let preset = LookPreset::capture(
            "Test",
            "test",
            "kodak_portra_400",
            "kodak_portra_endura",
            &params,
            provenance(),
        );
        let restored = LookPreset::from_json_str(&preset.to_json().unwrap()).unwrap();
        let film = restored.parameters.film_render;
        assert_eq!(
            film.chemistry.gamma_factor,
            params.film_render.chemistry.gamma_factor
        );
        assert_eq!(
            film.halation.halation_strength,
            params.film_render.halation.halation_strength
        );
    }

    #[test]
    fn five_classics_have_stable_ids_explicit_pairs_and_look_values() {
        let presets = builtin_look_presets();
        assert_eq!(presets.len(), 5);
        assert!(presets.iter().all(|p| {
            !p.id.is_empty()
                && p.film_profile.version.is_some()
                && p.print_profile.version.is_some()
                && p.parameters.film_render.grain.rms_granularity != [0.0; 3]
        }));
        assert_eq!(presets[0].film_profile.stock, "kodak_gold_200");
        assert_eq!(
            presets[3].print_profile.stock,
            "fujifilm_crystal_archive_typeii"
        );
        assert_eq!(
            presets[3]
                .parameters
                .film_render
                .dir_couplers
                .gamma_samelayer_rgb,
            [0.108, 0.072, 0.054]
        );
    }

    #[test]
    fn portable_schema_has_no_route() {
        let json = builtin_look_presets()[0].to_json().unwrap();
        assert!(!json.contains("\"route\""));
        assert!(!json.contains("\"workflow\""));
    }
    #[test]
    fn builtins_resolve_against_shipped_profiles() {
        let data_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut context = RuntimeParams::default();
        context.io.crop = true;
        context.random_seed = 1234;
        context.debug.lut_mode = true;
        context.settings.use_enlarger_lut = true;
        for preset in builtin_look_presets() {
            let resolved = preset.resolve(&data_dir, &context).unwrap();
            assert_eq!(resolved.film_name, preset.film_profile.stock);
            assert_eq!(resolved.print_name, preset.print_profile.stock);
            assert_eq!(
                resolved.params.workflow.route,
                if resolved.film.is_positive() {
                    "input > film > scan"
                } else {
                    "input > film > print > scan"
                }
            );
            assert!(resolved.params.io.crop);
            assert!(resolved.params.debug.lut_mode);
            assert!(resolved.params.settings.use_enlarger_lut);
            assert_eq!(resolved.params.random_seed, 1234);
            assert!(resolved.params.settings.neutral_print_filters_from_database);
        }
    }
    #[test]
    fn failed_resolution_is_non_mutating_and_route_is_derived() {
        let data_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let mut context = RuntimeParams::default();
        context.workflow.route = "input".into();
        let before = serde_json::to_value(&context).unwrap();
        let negative = builtin_look_presets()[0]
            .resolve(&data_dir, &context)
            .unwrap();
        assert_eq!(negative.default_route, "input > film > print > scan");
        assert_eq!(negative.params.workflow.route, negative.default_route);
        assert_eq!(
            serde_json::to_value(&context).unwrap(),
            before,
            "resolution must not mutate the active context"
        );

        let mut invalid = builtin_look_presets()[0].clone();
        invalid.film_profile.stock = "../outside".into();
        assert!(invalid.resolve(&data_dir, &context).is_err());
        assert_eq!(serde_json::to_value(&context).unwrap(), before);
    }
}
