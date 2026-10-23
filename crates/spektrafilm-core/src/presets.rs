//! Portable look and section documents. Library operations are shared by both frontends.
mod library;
mod sections;
use crate::params::sources::{ParameterEdits, parse_carrier};
use crate::params::{
    CmykParams, DiffusionFilterParams, EnlargerParams, FilmRenderingParams, OutputMixParams,
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

pub const PRESET_SCHEMA_VERSION: u32 = 6;
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
#[derive(Debug, Clone, PartialEq)]
pub struct LookParameters {
    /// Absent in legacy presets, which preserve the current virtual exposure.
    pub camera: Option<LookCameraParams>,
    pub enlarger: EnlargerParams,
    pub scanner: ScannerParams,
    pub film_render: FilmRenderingParams,
    pub print_render: PrintRenderingParams,
    pub camera_diffusion_filter: DiffusionFilterParams,
    pub output_mix: OutputMixParams,
    /// Canonical CMYK ink style enablement and strength, replacing the retired
    /// `magazine_print_color` spelling at the preset schema 5 boundary.
    pub cmyk: CmykParams,
    /// Spectral-reconstruction switches that shape the rendered look. This is
    /// deliberately a subset of `SettingsParams`: preview limits, performance
    /// toggles, and the working wavelength grid are not look content.
    pub settings: LookSettingsParams,
}
/// Private carrier for the portable look surface. Runtime groups also contain
/// calibration, derived, and compatibility fields that are not preset-owned.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LookParametersWire {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_present_camera"
    )]
    camera: Option<LookCameraParams>,
    enlarger: EnlargerParams,
    scanner: ScannerParams,
    film_render: FilmRenderingParams,
    print_render: PrintRenderingParams,
    camera_diffusion_filter: DiffusionFilterParams,
    output_mix: OutputMixParams,
    #[serde(default)]
    cmyk: CmykParams,
    #[serde(default)]
    settings: LookSettingsParams,
}

const UNOWNED_PRESET_FIELDS: &[(&str, &[&str])] = &[
    (
        "enlarger",
        &[
            "c_filter_neutral",
            "m_filter_neutral",
            "y_filter_neutral",
            "lens_blur",
        ],
    ),
    ("film_render", &["glare"]),
    (
        "film_render.grain",
        &["micro_sublayers", "monochrome", "particle_scale_layers"],
    ),
];

fn remove_unowned_preset_fields(value: &mut Value) {
    for &(group, fields) in UNOWNED_PRESET_FIELDS {
        if let Some(object) = group
            .split('.')
            .try_fold(&mut *value, |current, key| current.get_mut(key))
            .and_then(Value::as_object_mut)
        {
            for field in fields {
                object.remove(*field);
            }
        }
    }
}
fn remove_null_values(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.retain(|_, child| !child.is_null());
            for child in object.values_mut() {
                remove_null_values(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                remove_null_values(child);
            }
        }
        _ => {}
    }
}
pub(crate) fn reject_unowned_preset_fields(value: &Value) -> Result<(), String> {
    for &(group, fields) in UNOWNED_PRESET_FIELDS {
        let object = group
            .split('.')
            .try_fold(value, |current, key| current.get(key))
            .and_then(Value::as_object);
        if let Some(object) = object {
            for field in fields {
                if object.contains_key(*field) {
                    return Err(format!("preset field {group}.{field} is not preset-owned"));
                }
            }
        }
    }
    Ok(())
}

/// Canonical replacements for preset spellings retired before schema 5, so a
/// current preset document naming one names its replacement instead of
/// reporting a bare unknown field.
const RETIRED_PRESET_FIELDS: &[(&str, &str)] = &[
    ("magazine_print_color", "cmyk"),
    ("magazine_print_color.active", "cmyk.active"),
    ("magazine_print_color.strength", "cmyk.strength"),
];

pub(crate) fn reject_retired_preset_fields(value: &Value) -> Result<(), String> {
    for &(retired, replacement) in RETIRED_PRESET_FIELDS {
        let present = match retired.rsplit_once('.') {
            Some((group, leaf)) => value
                .get(group)
                .and_then(Value::as_object)
                .is_some_and(|object| object.contains_key(leaf)),
            None => value.get(retired).is_some(),
        };
        if present {
            return Err(format!(
                "preset field {retired} was retired; use {replacement} instead; a versioned \
                 legacy preset document may migrate it, but current-schema documents must use \
                 the canonical field"
            ));
        }
    }
    Ok(())
}

impl Serialize for LookParameters {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let wire = LookParametersWire {
            camera: self.camera.clone(),
            enlarger: self.enlarger.clone(),
            scanner: self.scanner.clone(),
            film_render: self.film_render.clone(),
            print_render: self.print_render.clone(),
            camera_diffusion_filter: self.camera_diffusion_filter.clone(),
            output_mix: self.output_mix.clone(),
            settings: self.settings.clone(),
            cmyk: self.cmyk.clone(),
        };
        let mut value = serde_json::to_value(wire).map_err(serde::ser::Error::custom)?;
        remove_unowned_preset_fields(&mut value);
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LookParameters {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        reject_unowned_preset_fields(&value).map_err(serde::de::Error::custom)?;
        reject_retired_preset_fields(&value).map_err(serde::de::Error::custom)?;
        let wire = serde_json::from_value::<LookParametersWire>(value)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            camera: wire.camera,
            enlarger: wire.enlarger,
            scanner: wire.scanner,
            film_render: wire.film_render,
            print_render: wire.print_render,
            camera_diffusion_filter: wire.camera_diffusion_filter,
            output_mix: wire.output_mix,
            settings: wire.settings,
            cmyk: wire.cmyk,
        })
    }
}
/// Virtual film exposure and metering, separate from RAW and input correction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookCameraParams {
    pub exposure_compensation_ev: f32,
    pub auto_exposure: bool,
    pub auto_exposure_method: String,
    /// Scene illuminant of the filming front pass. `None` inherits the
    /// selected film profile's reference illuminant; an explicit catalog
    /// source or `BB<kelvin>` blackbody overrides it. Never inferred from
    /// RAW or EXIF metadata, and distinct from the RAW white balance, the
    /// scan illuminant, the enlarger illuminant and the output white point.
    #[serde(default)]
    pub capture_illuminant: Option<String>,
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
            capture_illuminant: params.camera.capture_illuminant.clone(),
        }
    }
    fn apply_to(&self, params: &mut RuntimeParams) {
        params.camera.exposure_compensation_ev = self.exposure_compensation_ev;
        params.camera.auto_exposure = self.auto_exposure;
        params
            .camera
            .auto_exposure_method
            .clone_from(&self.auto_exposure_method);
        params
            .camera
            .capture_illuminant
            .clone_from(&self.capture_illuminant);
    }
    fn matches(&self, params: &RuntimeParams) -> bool {
        self.exposure_compensation_ev == params.camera.exposure_compensation_ev
            && self.auto_exposure == params.camera.auto_exposure
            && self.auto_exposure_method == params.camera.auto_exposure_method
            && self.capture_illuminant == params.camera.capture_illuminant
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
    #[serde(default)]
    pub print_exposure_ev_tracking: crate::params::PrintExposureEvTracking,
}
impl Default for LookSettingsParams {
    fn default() -> Self {
        Self {
            apply_hanatos2025_adaptation_window: true,
            apply_hanatos2025_adaptation_surface: false,
            spectral_gaussian_blur: 0.0,
            use_cat16: true,
            print_exposure_ev_tracking: crate::params::PrintExposureEvTracking::default(),
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
            cmyk: params.cmyk.clone(),
            settings: LookSettingsParams {
                apply_hanatos2025_adaptation_window: params
                    .settings
                    .apply_hanatos2025_adaptation_window,
                apply_hanatos2025_adaptation_surface: params
                    .settings
                    .apply_hanatos2025_adaptation_surface,
                spectral_gaussian_blur: params.settings.spectral_gaussian_blur,
                use_cat16: params.settings.use_cat16,
                print_exposure_ev_tracking: params.settings.print_exposure_ev_tracking,
            },
        }
    }

    /// Replace every look-owned parameter of `params` with this look.
    ///
    /// Runtime groups also contain calibration, derived, and compatibility
    /// fields that are deliberately absent from the preset wire surface. Keep
    /// those values from the active runtime while replacing owned leaves.
    pub fn apply_to(&self, params: &mut RuntimeParams) {
        if let Some(camera) = &self.camera {
            camera.apply_to(params);
        }
        let neutral = [
            params.enlarger.c_filter_neutral,
            params.enlarger.m_filter_neutral,
            params.enlarger.y_filter_neutral,
        ];
        let enlarger_lens_blur = params.enlarger.lens_blur;
        params.enlarger = self.enlarger.clone();
        params.enlarger.c_filter_neutral = neutral[0];
        params.enlarger.m_filter_neutral = neutral[1];
        params.enlarger.y_filter_neutral = neutral[2];
        params.enlarger.lens_blur = enlarger_lens_blur;
        params.scanner = self.scanner.clone();
        let film_glare = params.film_render.glare.clone();
        let grain_internal = (
            params.film_render.grain.micro_sublayers,
            params.film_render.grain.monochrome,
            params.film_render.grain.particle_scale_layers,
        );
        params.film_render = self.film_render.clone();
        params.film_render.glare = film_glare;
        params.film_render.grain.micro_sublayers = grain_internal.0;
        params.film_render.grain.monochrome = grain_internal.1;
        params.film_render.grain.particle_scale_layers = grain_internal.2;
        params.print_render = self.print_render.clone();
        params.camera.diffusion_filter = self.camera_diffusion_filter.clone();
        params.output_mix = self.output_mix.clone();
        params.cmyk = self.cmyk.clone();
        self.settings.apply_to(&mut params.settings);
    }
    /// Compare only carried controls; legacy presets do not own exposure.
    pub fn matches(&self, params: &RuntimeParams) -> bool {
        let mut expected = params.clone();
        self.apply_to(&mut expected);
        self.camera
            .as_ref()
            .is_none_or(|camera| camera.matches(params))
            && expected.enlarger == params.enlarger
            && expected.scanner == params.scanner
            && expected.film_render == params.film_render
            && expected.print_render == params.print_render
            && expected.camera.diffusion_filter == params.camera.diffusion_filter
            && expected.output_mix == params.output_mix
            && expected.cmyk == params.cmyk
            && expected.settings == params.settings
    }
}
impl LookSettingsParams {
    fn apply_to(&self, settings: &mut SettingsParams) {
        settings.apply_hanatos2025_adaptation_window = self.apply_hanatos2025_adaptation_window;
        settings.apply_hanatos2025_adaptation_surface = self.apply_hanatos2025_adaptation_surface;
        settings.spectral_gaussian_blur = self.spectral_gaussian_blur;
        settings.use_cat16 = self.use_cat16;
        settings.print_exposure_ev_tracking = self.print_exposure_ev_tracking;
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
        if value["schema_version"].as_u64() == Some(5)
            && let Some(parameters) = value.get("parameters")
        {
            reject_retired_preset_fields(parameters)?;
        }
        if matches!(value["schema_version"].as_u64(), Some(1..=5)) {
            let version = value["schema_version"].as_u64().unwrap();
            let object = value.as_object_mut().ok_or("preset must be an object")?;
            if version == 1 {
                object.remove("id");
            }
            if version <= 2 {
                if object
                    .get("parameters")
                    .and_then(|p| p.get("output_mix"))
                    .is_some()
                {
                    return Err(
                        "legacy preset parameters.output_mix is unknown; use schema_version 5"
                            .into(),
                    );
                }
                if object
                    .get("parameters")
                    .and_then(|p| p.get("camera"))
                    .is_some()
                {
                    return Err(
                        "legacy preset parameters.camera is unknown; use schema_version 5".into(),
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
                        serde_json::to_value(OutputMixParams::default())
                            .map_err(|e| e.to_string())?,
                    );
                }
            }
            if version <= 3 {
                let settings_allowed = object
                    .get("parameters")
                    .and_then(Value::as_object)
                    .is_some_and(|parameters| parameters.len() > 1);
                let parameters = object
                    .get_mut("parameters")
                    .ok_or("preset parameters must be an object")?;
                crate::params::sources::normalize_legacy_conversion_paths(
                    parameters,
                    settings_allowed,
                )?;
            }
            if version <= 4 {
                // The retired `magazine_print_color` group belongs to every
                // accepted legacy preset version. The pass also applies the
                // output lightness retirement rule, so a retired gamut leaf
                // aborts the whole migration transactionally.
                let parameters = object
                    .get_mut("parameters")
                    .ok_or("preset parameters must be an object")?;
                crate::params::sources::normalize_legacy_complete_params(parameters)?;
            }
            // Version 6 removes the scanner lamp and makes the old built-in
            // spelling an input alias. Validate historical scanner provenance
            // before replacing it; arbitrary metadata is not a migration.
            let lamp = object
                .get("parameters")
                .and_then(|p| p.pointer("/scanner/lamp"))
                .cloned();
            if let Some(scanner) = object
                .get("provenance")
                .and_then(|p| p.get("scanner"))
                .filter(|scanner| !scanner.is_null())
            {
                let current = serde_json::to_value(crate::ScannerProvenance::from_params(
                    &RuntimeParams::default(),
                ))
                .map_err(|e| e.to_string())?;
                let mut legacy = current.clone();
                legacy["model"] = "neutral_scan_v2".into();
                let mut with_lamp = legacy.clone();
                with_lamp["lamp"] = lamp.clone().unwrap_or_else(|| "D55".into());
                if scanner != &legacy && scanner != &with_lamp && scanner != &current {
                    return Err(
                        "legacy scanner provenance does not match the built-in readout model"
                            .into(),
                    );
                }
                object["provenance"]["scanner"] = current;
                object["provenance"]["model_version"] = crate::runtime::MODEL_VERSION.into();
            }
            if let Some(scanner) = object
                .get_mut("parameters")
                .and_then(|p| p.get_mut("scanner"))
                .and_then(Value::as_object_mut)
            {
                scanner.remove("lamp");
            }
            object.insert("schema_version".into(), PRESET_SCHEMA_VERSION.into());
        }
        if value
            .pointer("/parameters/scanner/conversion_style")
            .and_then(Value::as_str)
            == Some("neutral")
        {
            value["parameters"]["scanner"]["conversion_style"] = "standard".into();
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
            let expected = serde_json::to_value(crate::ScannerProvenance::from_params(&params))
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
        let mut value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        if matches!(self.parameters, PresetParameters::Look(_)) {
            remove_null_values(&mut value);
        } else if let PresetParameters::Section(payload) = &self.parameters {
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
        let value = toml::Value::try_from(value).map_err(|e| e.to_string())?;
        toml::to_string_pretty(&value).map_err(|e| e.to_string())
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
        // Neutral print filtration is material calibration, not a look value: an axis
        // is protected only when the document carries an authored value for it, so the
        // per-(paper, illuminant, film) database still supplies the baseline for every
        // other look instead of one paper's pack being reused everywhere.
        let defaults = crate::params::EnlargerParams::default();
        params.neutral_print_filters_protected = Some([
            look.enlarger.c_filter_neutral != defaults.c_filter_neutral,
            look.enlarger.m_filter_neutral != defaults.m_filter_neutral,
            look.enlarger.y_filter_neutral != defaults.y_filter_neutral,
        ]);
        let default_route = workflow_default(&film);
        // A caller that already declared a film-scan route keeps it. The route and
        // the scan mode it selects are a workflow choice; overriding the route here
        // would silently discard the scan mode an authored document carries.
        let caller_declared_film_scan = matches!(
            current.workflow.route.as_str(),
            "input > film > scan" | "input > convert-film > scan"
        );
        params.workflow.route = if caller_declared_film_scan {
            current.workflow.route.clone()
        } else {
            default_route.clone()
        };
        params.io.scan_film = film.is_positive();
        if film.is_positive()
            || !matches!(
                params.workflow.route.as_str(),
                "input > film > scan" | "input > convert-film > scan"
            )
        {
            params.scanner.scan_output = "direct_scan".into();
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
            serde_json::to_value(crate::ScannerProvenance::from_params(params))
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
            serde_json::to_value(crate::ScannerProvenance::from_params(&captured))
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
