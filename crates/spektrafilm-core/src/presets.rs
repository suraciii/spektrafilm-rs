//! Portable look and section documents. Library operations are shared by both frontends.
mod library;
mod sections;
use crate::params::sources::{ParameterEdits, parse_carrier};
use crate::params::{
    DiffusionFilterParams, EnlargerParams, FilmRenderingParams, OutputMixParams,
    PrintRenderingParams, RuntimeParams, ScannerParams, SettingsParams,
};
use crate::params_builder::apply_film_specifics;
use crate::profile::{self, Profile};
pub use library::*;
pub use sections::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const PRESET_SCHEMA_VERSION: u32 = 3;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRef {
    pub stock: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_identity: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub implementation_version: String,
    pub model_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scanner: Option<Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookParameters {
    /// Absent in legacy presets, which preserve the current virtual exposure.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_camera"
    )]
    pub camera: Option<LookCameraParams>,
    pub enlarger: EnlargerParams,
    pub scanner: ScannerParams,
    pub film_render: FilmRenderingParams,
    pub print_render: PrintRenderingParams,
    pub camera_diffusion_filter: DiffusionFilterParams,
    pub output_mix: OutputMixParams,
    /// Spectral-reconstruction switches that shape the rendered look. This is
    /// deliberately a subset of `SettingsParams`: preview limits, performance
    /// toggles, and the working wavelength grid are not look content.
    #[serde(default)]
    pub settings: LookSettingsParams,
}
/// Virtual film exposure and metering, separate from RAW and input correction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookCameraParams {
    pub exposure_compensation_ev: f32,
    pub auto_exposure: bool,
    pub auto_exposure_method: String,
}
fn deserialize_present_camera<'de, D>(deserializer: D) -> Result<Option<LookCameraParams>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    LookCameraParams::deserialize(deserializer).map(Some)
}
impl LookCameraParams {
    fn capture(params: &RuntimeParams) -> Self {
        Self {
            exposure_compensation_ev: params.camera.exposure_compensation_ev,
            auto_exposure: params.camera.auto_exposure,
            auto_exposure_method: params.camera.auto_exposure_method.clone(),
        }
    }
    fn apply_to(&self, params: &mut RuntimeParams) {
        params.camera.exposure_compensation_ev = self.exposure_compensation_ev;
        params.camera.auto_exposure = self.auto_exposure;
        params
            .camera
            .auto_exposure_method
            .clone_from(&self.auto_exposure_method);
    }
    fn matches(&self, params: &RuntimeParams) -> bool {
        self.exposure_compensation_ev == params.camera.exposure_compensation_ev
            && self.auto_exposure == params.camera.auto_exposure
            && self.auto_exposure_method == params.camera.auto_exposure_method
    }
}
/// Look-owned processing settings (subset of `SettingsParams`, same defaults).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookSettingsParams {
    #[serde(default = "default_true")]
    pub apply_hanatos2025_adaptation_window: bool,
    #[serde(default)]
    pub apply_hanatos2025_adaptation_surface: bool,
    #[serde(default)]
    pub spectral_gaussian_blur: f32,
    #[serde(default = "default_true")]
    pub use_cat16: bool,
}
impl Default for LookSettingsParams {
    fn default() -> Self {
        Self {
            apply_hanatos2025_adaptation_window: true,
            apply_hanatos2025_adaptation_surface: false,
            spectral_gaussian_blur: 0.0,
            use_cat16: true,
        }
    }
}
fn default_true() -> bool {
    true
}
// The look variant owns the full look, and preset collections are small.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum PresetParameters {
    Look(LookParameters),
    Section(Value),
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Preset {
    pub schema_version: u32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub film_profile: Option<ProfileRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub print_profile: Option<ProfileRef>,
    pub parameters: PresetParameters,
    pub provenance: Provenance,
}
impl<'de> Deserialize<'de> for Preset {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_value(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
#[derive(Debug, Clone)]
pub struct ResolvedPreset {
    pub film_name: String,
    pub print_name: String,
    pub default_route: String,
    pub params: RuntimeParams,
    pub film: Profile,
    pub print: Profile,
}
impl LookParameters {
    pub fn capture(params: &RuntimeParams) -> Self {
        Self {
            camera: Some(LookCameraParams::capture(params)),
            enlarger: params.enlarger.clone(),
            scanner: params.scanner.clone(),
            film_render: params.film_render.clone(),
            print_render: params.print_render.clone(),
            camera_diffusion_filter: params.camera.diffusion_filter.clone(),
            output_mix: params.output_mix.clone(),
            settings: LookSettingsParams {
                apply_hanatos2025_adaptation_window: params
                    .settings
                    .apply_hanatos2025_adaptation_window,
                apply_hanatos2025_adaptation_surface: params
                    .settings
                    .apply_hanatos2025_adaptation_surface,
                spectral_gaussian_blur: params.settings.spectral_gaussian_blur,
                use_cat16: params.settings.use_cat16,
            },
        }
    }

    /// Replace every look-owned parameter of `params` with this look.
    ///
    /// The look owns virtual exposure when present, enlarger, scanner, film
    /// and print rendering, camera diffusion, and look processing settings.
    /// Every other parameter section keeps its value.
    pub fn apply_to(&self, params: &mut RuntimeParams) {
        if let Some(camera) = &self.camera {
            camera.apply_to(params);
        }
        params.enlarger = self.enlarger.clone();
        params.scanner = self.scanner.clone();
        params.film_render = self.film_render.clone();
        params.print_render = self.print_render.clone();
        params.camera.diffusion_filter = self.camera_diffusion_filter.clone();
        params.output_mix = self.output_mix.clone();
        self.settings.apply_to(&mut params.settings);
    }
    /// Compare only carried controls; legacy presets do not own exposure.
    pub fn matches(&self, params: &RuntimeParams) -> bool {
        self.camera
            .as_ref()
            .is_none_or(|camera| camera.matches(params))
            && self.enlarger == params.enlarger
            && self.scanner == params.scanner
            && self.film_render == params.film_render
            && self.print_render == params.print_render
            && self.camera_diffusion_filter == params.camera.diffusion_filter
            && self.output_mix == params.output_mix
            && self.settings.apply_hanatos2025_adaptation_window
                == params.settings.apply_hanatos2025_adaptation_window
            && self.settings.apply_hanatos2025_adaptation_surface
                == params.settings.apply_hanatos2025_adaptation_surface
            && self.settings.spectral_gaussian_blur == params.settings.spectral_gaussian_blur
            && self.settings.use_cat16 == params.settings.use_cat16
    }
}
impl LookSettingsParams {
    fn apply_to(&self, settings: &mut SettingsParams) {
        settings.apply_hanatos2025_adaptation_window = self.apply_hanatos2025_adaptation_window;
        settings.apply_hanatos2025_adaptation_surface = self.apply_hanatos2025_adaptation_surface;
        settings.spectral_gaussian_blur = self.spectral_gaussian_blur;
        settings.use_cat16 = self.use_cat16;
    }
}
impl Preset {
    pub fn look_parameters(&self) -> Option<&LookParameters> {
        if let PresetParameters::Look(look) = &self.parameters {
            Some(look)
        } else {
            None
        }
    }
    pub fn look_parameters_mut(&mut self) -> Option<&mut LookParameters> {
        if let PresetParameters::Look(look) = &mut self.parameters {
            Some(look)
        } else {
            None
        }
    }
    pub fn from_json_str(text: &str) -> Result<Self, String> {
        Self::from_value(parse_carrier(text, false)?)
    }
    pub fn from_toml_str(text: &str) -> Result<Self, String> {
        Self::from_value(parse_carrier(text, true)?)
    }
    fn from_value(mut value: Value) -> Result<Self, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_version: u32,
            name: String,
            author: Option<String>,
            description: Option<String>,
            film_profile: Option<ProfileRef>,
            print_profile: Option<ProfileRef>,
            parameters: Value,
            provenance: Provenance,
        }
        if matches!(value["schema_version"].as_u64(), Some(1 | 2)) {
            let version = value["schema_version"].as_u64().unwrap();
            let object = value.as_object_mut().ok_or("preset must be an object")?;
            if version == 1 {
                object.remove("id");
            }
            if object
                .get("parameters")
                .and_then(|p| p.get("output_mix"))
                .is_some()
            {
                return Err(
                    "legacy preset parameters.output_mix is unknown; use schema_version 3".into(),
                );
            }
            if object
                .get("parameters")
                .and_then(|p| p.get("camera"))
                .is_some()
            {
                return Err(
                    "legacy preset parameters.camera is unknown; use schema_version 3".into(),
                );
            }
            if object.get("film_profile").is_some_and(|v| !v.is_null())
                || object.get("print_profile").is_some_and(|v| !v.is_null())
            {
                let parameters = object
                    .get_mut("parameters")
                    .and_then(Value::as_object_mut)
                    .ok_or("preset parameters must be an object")?;
                parameters.insert(
                    "output_mix".into(),
                    serde_json::to_value(OutputMixParams::default()).map_err(|e| e.to_string())?,
                );
            }
            object.insert("schema_version".into(), PRESET_SCHEMA_VERSION.into());
        }
        let wire: Wire =
            serde_json::from_value(value).map_err(|e| format!("invalid preset: {e}"))?;
        let parameters = if wire.film_profile.is_some() || wire.print_profile.is_some() {
            PresetParameters::Look(
                serde_json::from_value(wire.parameters)
                    .map_err(|e| format!("invalid look: {e}"))?,
            )
        } else {
            PresetParameters::Section(wire.parameters)
        };
        let preset = Self {
            schema_version: wire.schema_version,
            name: wire.name,
            author: wire.author,
            description: wire.description,
            film_profile: wire.film_profile,
            print_profile: wire.print_profile,
            parameters,
            provenance: wire.provenance,
        };
        preset.validate()?;
        Ok(preset)
    }
    pub fn kind(&self) -> Result<PresetKind, String> {
        match &self.parameters {
            PresetParameters::Look(_) if self.film_profile.is_some() && self.print_profile.is_some()=>Ok(PresetKind::Look),
            PresetParameters::Section(payload) if self.film_profile.is_none() && self.print_profile.is_none()=>Ok(PresetKind::Section(section_from_payload(payload)?)),
            _=>Err("a preset must carry a complete look with both profiles or exactly one section without profiles".into()),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PRESET_SCHEMA_VERSION {
            return Err(format!(
                "unsupported preset schema version {}",
                self.schema_version
            ));
        }
        validate_name(&self.name)?;
        if self.provenance.implementation_version.trim().is_empty()
            || self.provenance.model_version.trim().is_empty()
        {
            return Err("preset provenance versions must not be empty".into());
        }
        self.kind()?;
        for (kind, reference) in [("film", &self.film_profile), ("print", &self.print_profile)] {
            if let Some(reference) = reference {
                validate_stock_id(kind, &reference.stock)?;
                validate_content_identity(kind, reference)?;
            }
        }
        // Presets do not carry workflow routing. Validate scanner-owned values
        // in their supported film-scan context; applying checks the actual route.
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        match &self.parameters {
            PresetParameters::Look(look) => look.apply_to(&mut params),
            PresetParameters::Section(payload) => parameter_edits(payload)?.apply(&mut params)?,
        }
        if let Some(scanner) = &self.provenance.scanner {
            let expected = serde_json::to_value(
                crate::scan_receiver::ScannerProvenance::from_params(&params),
            )
            .map_err(|e| e.to_string())?;
            if scanner != &expected {
                return Err(
                    "scanner provenance does not match the typed scanner parameters".into(),
                );
            }
        }
        crate::params::sources::validate_finite(&params)?;
        params.validate()
    }
    pub fn to_json(&self) -> Result<String, String> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }
    pub fn to_toml(&self) -> Result<String, String> {
        self.validate()?;
        if let PresetParameters::Section(payload) = &self.parameters {
            fn null_paths(prefix: &str, value: &Value, out: &mut Vec<String>) {
                if let Some(map) = value.as_object() {
                    for (key, value) in map {
                        let path = if prefix.is_empty() {
                            key.clone()
                        } else {
                            format!("{prefix}.{key}")
                        };
                        null_paths(&path, value, out);
                    }
                } else if value.is_null() {
                    out.push(prefix.to_owned());
                }
            }
            let mut paths = Vec::new();
            null_paths("", payload, &mut paths);
            if !paths.is_empty() {
                return Err(format!(
                    "TOML cannot represent explicit null at {}; use --format json or a .json extension",
                    paths.join(", ")
                ));
            }
        }
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }
    pub fn resolve(
        &self,
        data_dir: &Path,
        current: &RuntimeParams,
    ) -> Result<ResolvedPreset, String> {
        self.validate()?;
        let (Some(film_ref), Some(print_ref), PresetParameters::Look(look)) =
            (&self.film_profile, &self.print_profile, &self.parameters)
        else {
            return Err("section presets use apply_section, not look resolution".into());
        };
        let film_identity = profile_content_identity_if_requested(data_dir, film_ref)?;
        let print_identity = profile_content_identity_if_requested(data_dir, print_ref)?;
        let film =
            profile::load_profile_by_name(data_dir, &film_ref.stock).map_err(|e| e.to_string())?;
        let print =
            profile::load_profile_by_name(data_dir, &print_ref.stock).map_err(|e| e.to_string())?;
        validate_profile_ref("film", film_ref, &film, film_identity.as_deref())?;
        validate_profile_ref("print", print_ref, &print, print_identity.as_deref())?;
        let mut params = current.clone();
        apply_film_specifics(&mut params, &film);
        look.apply_to(&mut params);
        params.neutral_print_filters_protected = [true; 3];
        let default_route = workflow_default(&film);
        params.workflow.route = default_route.clone();
        params.io.scan_film = film.is_positive();
        if params.scanner.positive_interpretation
            == crate::params::PositiveInterpretation::DigitalReference
            && (film.is_positive() || params.workflow.route != "input > film > scan")
        {
            params.scanner.positive_interpretation =
                crate::params::PositiveInterpretation::default();
            params.scanner.receiver_profile = crate::params::ReceiverProfile::default();
            params.scanner.reference_exposure_ev = 0.0;
            params.scanner.reference_balance_ev = [0.0; 3];
        }
        if film.is_positive()
            || !matches!(
                params.workflow.route.as_str(),
                "input > film > scan" | "input > convert-film > scan"
            )
        {
            params.scanner.scan_output = "direct_scan".into();
            params.scanner.positive_interpretation =
                crate::params::PositiveInterpretation::default();
            params.scanner.auto_setup = crate::params::ScannerAutoSetup::default();
            params.scanner.finish = crate::params::ScannerFinish::default();
            if params.scanner.receiver_profile == crate::params::ReceiverProfile::DigitalReference {
                params.scanner.receiver_profile = crate::params::ReceiverProfile::default();
                params.scanner.reference_exposure_ev = 0.0;
                params.scanner.reference_balance_ev = [0.0; 3];
            }
        }
        if !matches!(
            params.workflow.route.as_str(),
            "input > film > scan" | "input > convert-film > scan"
        ) {
            params.scanner.receiver_profile = crate::params::ReceiverProfile::default();
        }
        crate::pipeline_calibration::validate_scan_output_route(&params)?;
        params.validate()?;
        Ok(ResolvedPreset {
            film_name: film_ref.stock.clone(),
            print_name: print_ref.stock.clone(),
            default_route,
            params,
            film,
            print,
        })
    }
    pub fn capture(
        name: impl Into<String>,
        film: impl Into<String>,
        print: impl Into<String>,
        params: &RuntimeParams,
        mut provenance: Provenance,
    ) -> Self {
        provenance.scanner = Some(
            serde_json::to_value(crate::scan_receiver::ScannerProvenance::from_params(params))
                .expect("built-in provenance is serializable"),
        );
        Self {
            schema_version: PRESET_SCHEMA_VERSION,
            name: name.into(),
            author: None,
            description: None,
            film_profile: Some(ProfileRef {
                stock: film.into(),
                version: None,
                content_identity: None,
            }),
            print_profile: Some(ProfileRef {
                stock: print.into(),
                version: None,
                content_identity: None,
            }),
            parameters: PresetParameters::Look(LookParameters::capture(params)),
            provenance,
        }
    }
    pub fn capture_section(
        name: impl Into<String>,
        section: Section,
        params: &RuntimeParams,
        mut provenance: Provenance,
    ) -> Result<Self, String> {
        let payload = extract_section(section, params)?;
        let mut captured = RuntimeParams::default();
        parameter_edits(&payload)?.apply(&mut captured)?;
        provenance.scanner = Some(
            serde_json::to_value(crate::scan_receiver::ScannerProvenance::from_params(
                &captured,
            ))
            .map_err(|e| e.to_string())?,
        );
        let preset = Self {
            schema_version: PRESET_SCHEMA_VERSION,
            name: name.into(),
            author: None,
            description: None,
            film_profile: None,
            print_profile: None,
            parameters: PresetParameters::Section(payload),
            provenance,
        };
        preset.validate()?;
        Ok(preset)
    }
    /// Returns a candidate validated against the current film and scan configuration.
    /// An unrelated edit may preserve an already-invalid scan configuration.
    pub fn apply_section(
        &self,
        current: &RuntimeParams,
        film: &Profile,
    ) -> Result<RuntimeParams, String> {
        let params = self.overlay_section(current)?;
        params.validate()?;
        let scan_changed = params.scanner.scan_output != current.scanner.scan_output
            || params.scanner.white_correction != current.scanner.white_correction
            || params.scanner.black_correction != current.scanner.black_correction
            || params.film_render.grain.active != current.film_render.grain.active
            || params.film_render.grain.engine != current.film_render.grain.engine;
        if scan_changed || crate::pipeline_calibration::validate_scan_output(current, film).is_ok()
        {
            crate::pipeline_calibration::validate_scan_output(&params, film)?;
        }
        Ok(params)
    }

    /// Compose independently validated leaves without validating intermediate controls.
    /// The caller must validate the complete candidate before applying it.
    pub(crate) fn overlay_section(&self, current: &RuntimeParams) -> Result<RuntimeParams, String> {
        self.validate()?;
        let PresetParameters::Section(payload) = &self.parameters else {
            return Err("expected a section preset".into());
        };
        let edits = parameter_edits(payload)?;
        let mut params = current.clone();
        edits.apply(&mut params)?;
        for path in edits.paths() {
            params.explicit_section_paths.insert(path.to_owned());
            for (axis, name) in ["c_filter_neutral", "m_filter_neutral", "y_filter_neutral"]
                .iter()
                .enumerate()
            {
                if path == format!("enlarger.{name}") {
                    params.neutral_print_filters_protected[axis] = true;
                }
            }
        }
        Ok(params)
    }
}
fn parameter_edits(payload: &Value) -> Result<ParameterEdits, String> {
    let mut value = payload.clone();
    if let Some(filter) = value
        .as_object_mut()
        .and_then(|m| m.remove("camera_diffusion_filter"))
    {
        value["camera"] = serde_json::json!({"diffusion_filter":filter});
    }
    ParameterEdits::from_value(value)
}
include!(concat!(env!("OUT_DIR"), "/preset_assets.rs"));
pub fn builtin_presets() -> Vec<Preset> {
    let mut presets = BUILTIN_ASSETS
        .iter()
        .map(|(_, text)| Preset::from_toml_str(text).expect("invalid compiled preset asset"))
        .collect::<Vec<_>>();
    presets.sort_by(|a, b| name_order(&a.name, &b.name));
    presets
}
pub fn load_selector(selector: &str, _data_dir: &Path) -> Result<Preset, String> {
    PresetLibrary::default()
        .resolve_selector(selector)
        .map(|selection| selection.preset)
}
pub fn suggest_preset_names(input: &str) -> Vec<String> {
    PresetLibrary::default().suggest(input)
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
    if let Some(version) = &reference.version
        && &profile.metadata.version != version
    {
        return Err(format!(
            "{kind} profile {:?} version mismatch: expected {version:?}, got {:?}",
            reference.stock, profile.metadata.version
        ));
    }
    if let Some(expected) = content_identity
        && Some(expected) != reference.content_identity.as_deref()
    {
        return Err(format!(
            "{kind} profile {:?} content identity was not verified",
            reference.stock
        ));
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

pub(crate) fn explicit_stock_values(params: &RuntimeParams) -> Option<ParameterEdits> {
    if params.explicit_section_paths.is_empty() {
        return None;
    }
    let root = serde_json::to_value(params).expect("runtime parameters serialize");
    let mut payload = serde_json::json!({});
    for path in &params.explicit_section_paths {
        if let Some(value) = root.pointer(&format!("/{}", path.replace('.', "/"))) {
            crate::params::sources::insert(&mut payload, path, value.clone());
        }
    }
    Some(ParameterEdits::from_value(payload).expect("validated explicit paths"))
}
pub(crate) fn restore_explicit_stock_values(
    params: &mut RuntimeParams,
    edits: Option<ParameterEdits>,
) {
    if let Some(edits) = edits {
        edits
            .apply(params)
            .expect("retaining validated explicit controls");
    }
}
#[cfg(test)]
mod camera_tests;
#[cfg(test)]
mod tests;
