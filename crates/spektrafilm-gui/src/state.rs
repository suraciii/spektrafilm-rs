//! GUI state and runtime ownership pinned to upstream 28bf883.
use anyhow::{Result, bail};
use serde_json::{Value, json};
use spektrafilm_core::params::RuntimeParams;

mod io;
mod projection;
mod validation;

pub use io::{default_path, reset_factory, startup};
pub use spektrafilm_core::config::config_dir;
pub(crate) use validation::validate_stored_runtime;
use validation::{normalize_grain_section, validate_state_parameters};

/// Version 5 removes scanner light controls and emits the canonical standard
/// style. Explicit versions 1–4 migrate; current documents validate strictly.
pub(crate) const CURRENT_STATE_VERSION: u64 = 5;

/// Enlarger controls the upstream GUI also exposes under `simulation`
/// aliases, as `(simulation key, enlarger key)`. Both directions of the
/// state ⇄ runtime projection consume this one table, so an alias cannot be
/// added for only one of them. `print_exposure_compensation` is the one
/// exception: its runtime leaf was replaced by
/// `settings.print_exposure_ev_tracking`, so both projections translate it
/// instead of copying it.
const ENLARGER_SIMULATION_ALIASES: &[(&str, &str)] = &[
    ("print_illuminant", "illuminant"),
    ("print_exposure", "print_exposure"),
    ("print_exposure_compensation", "print_exposure_compensation"),
    ("print_y_filter_shift", "y_filter_shift"),
    ("print_m_filter_shift", "m_filter_shift"),
];

/// Translate the legacy print-exposure pair that the upstream-mirror state
/// sections (`preflashing`, `simulation.enlarger`) still carry. The
/// state-side keys stay boolean to mirror the upstream GUI; the runtime sees
/// `enlarger.print_exposure_mode` and, for the mixed states, the tracking
/// setting. A non-boolean pair passes through so unknown-field validation
/// rejects it.
fn translate_legacy_print_exposure(section: &Value) -> (Value, Option<Value>) {
    let legacy = section.get("normalize_print_exposure").is_some()
        || section.get("print_exposure_compensation").is_some();
    if !legacy {
        return (section.clone(), None);
    }
    let mut carrier = json!({"enlarger": section.clone()});
    match spektrafilm_core::params::sources::normalize_legacy_conversion_paths(&mut carrier, true) {
        Ok(()) => {
            let tracking = carrier.get("settings").cloned();
            (carrier["enlarger"].clone(), tracking)
        }
        Err(_) => (section.clone(), None),
    }
}

#[derive(Clone, Debug)]
pub struct GuiState {
    pub sections: Value,
}

/// Merge known leaves only, like upstream's dataclass merge. Unknown fields
/// never become runtime fields; Rust additions live in the versioned extension.
fn merge(target: &mut Value, source: &Value) {
    if let (Some(dst), Some(src)) = (target.as_object_mut(), source.as_object()) {
        for (key, value) in src {
            if let Some(current) = dst.get_mut(key) {
                if current.is_object() && value.is_object() {
                    merge(current, value);
                } else {
                    *current = value.clone();
                }
            }
        }
    }
}
fn copy_fields(target: &mut Value, source: &Value, names: &[&str]) {
    for name in names {
        if let Some(value) = source.get(*name) {
            target[*name] = value.clone();
        }
    }
}
fn flatten(source: &Value, groups: &[&str]) -> Value {
    let mut flat = source.clone();
    for group in groups {
        if let Some(values) = source.get(*group).and_then(Value::as_object) {
            for (key, value) in values {
                flat[key] = value.clone();
            }
        }
    }
    flat
}
fn canonicalize_grain_state(section: &mut Value) {
    let Some(object) = section.as_object_mut() else {
        return;
    };
    const CANONICAL: &[&str] = &[
        "active",
        "rms_granularity",
        "density_min",
        "uniformity",
        "particle_scale_sublayers",
        "blur",
        "mult_usm_sigma",
        "mult_usm_amount",
        "blur_dye_clouds_um",
        "micro_structure",
        "micro_sublayers",
    ];
    object.retain(|key, _| CANONICAL.contains(&key.as_str()));
}

/// Runtime snapshots preserve unknown keys until typed validation rejects them.
fn overlay_runtime(target: &mut Value, source: &Value) {
    if let (Some(dst), Some(src)) = (target.as_object_mut(), source.as_object()) {
        for (key, value) in src {
            if let Some(current) = dst.get_mut(key)
                && current.is_object()
                && value.is_object()
            {
                overlay_runtime(current, value);
            } else {
                dst.insert(key.clone(), value.clone());
            }
        }
    }
}

impl GuiState {
    pub fn factory() -> Self {
        let mut sections: Value =
            serde_json::from_str(include_str!("factory_state.json")).expect("pinned factory JSON");
        sections["rust"]["scanner_provenance"] = serde_json::to_value(
            spektrafilm_core::scanner_provenance::ScannerProvenance::from_params(
                &RuntimeParams::default(),
            ),
        )
        .expect("scanner provenance serializes");
        sections["rust"]["model_version"] = json!(spektrafilm_core::runtime::MODEL_VERSION);
        Self { sections }
    }
    pub fn from_value(mut value: Value) -> Result<Self> {
        if !value.is_object() {
            bail!(crate::app::Message::new("operations.state.object"));
        }
        let version = value["rust"].get("version").and_then(Value::as_u64);
        if value.get("rust").is_some() && !matches!(version, Some(1..=CURRENT_STATE_VERSION)) {
            bail!(
                crate::app::Message::new("operations.state.version")
                    .arg("value", &value["rust"]["version"])
            );
        }
        if matches!(version, Some(1..=4)) {
            let mut top = json!({"scanner": value["scanner"].clone(),
                "film_render": {"convert": value.get("convert").cloned().or_else(|| value.pointer("/rust/runtime/film_render/convert").cloned()).unwrap_or_else(|| json!({}))}});
            spektrafilm_core::params::sources::normalize_legacy_scan_readout(&mut top, true)
                .map_err(anyhow::Error::msg)?;
            if value.get("scanner").is_some() {
                value["scanner"] = top["scanner"].clone();
            }
            if top
                .pointer("/film_render/convert/scan_illuminant")
                .is_some()
            {
                value["convert"] = top["film_render"]["convert"].clone();
            }
            if let Some(runtime) = value.pointer_mut("/rust/runtime") {
                spektrafilm_core::params::sources::normalize_legacy_scan_readout(runtime, true)
                    .map_err(anyhow::Error::msg)?;
            }
        }
        if value.pointer("/convert/scan_illuminant").is_none()
            && let Some(light) = value
                .pointer("/rust/runtime/film_render/convert/scan_illuminant")
                .cloned()
        {
            if value.get("convert").is_none() {
                value["convert"] = json!({});
            }
            if let Some(convert) = value.get_mut("convert").and_then(Value::as_object_mut) {
                convert.insert("scan_illuminant".into(), light);
            }
        }
        validate_state_parameters(&mut value, version)?;
        for scanner in [&value["scanner"], &value["rust"]["runtime"]["scanner"]] {
            if let Some(selection) = scanner.get("scan_output") {
                spektrafilm_core::params::sources::ParameterEdits::parse(&format!(
                    "scanner.scan_output={selection}"
                ))
                .map_err(anyhow::Error::msg)?;
            }
        }
        let mut state = Self::factory();
        let mut normalized = value.clone();
        normalized["input_image"] = flatten(&value["input_image"], &["io", "settings"]);
        for (old, new) in [
            ("apply_cctf_decoding", "input_cctf_decoding"),
            ("spectral_upsampling_method", "rgb_to_raw_method"),
        ] {
            if normalized["input_image"].get(new).is_none()
                && let Some(v) = value["input_image"].get(old)
            {
                normalized["input_image"][new] = v.clone();
            }
        }
        normalized["simulation"] = flatten(&value["simulation"], &["selection", "io"]);
        copy_fields(
            &mut normalized["simulation"],
            &value["simulation"]["workflow"],
            &["saving_color_space", "saving_cctf_encoding", "auto_preview"],
        );
        if let Some(enlarger) = value["simulation"].get("enlarger") {
            for &(simulation_key, enlarger_key) in ENLARGER_SIMULATION_ALIASES {
                if let Some(v) = enlarger.get(enlarger_key) {
                    normalized["simulation"][simulation_key] = v.clone();
                }
            }
        }
        // Upstream accepts the former scan_film boolean only on flat input.
        if value["simulation"].get("route").is_none()
            && value["simulation"].get("workflow").is_none()
            && value["simulation"].get("io").is_none()
            && value["simulation"].get("enlarger").is_none()
            && value["simulation"]["scan_film"].as_bool() == Some(true)
        {
            normalized["simulation"]["route"] = json!("input > film > scan");
        }
        for section in ["load_raw", "display"] {
            // Flat top-level GUI-only sections override nested gui_only.
            let original = value
                .get(section)
                .or_else(|| value["gui_only"].get(section));
            if let Some(original) = original {
                normalized[section] = flatten(original, &["settings"]);
            }
        }
        normalize_grain_section(&mut normalized["grain"]);
        for section in ["input_image", "simulation", "grain"] {
            if value.get(section).is_none() {
                normalized
                    .as_object_mut()
                    .expect("state object")
                    .remove(section);
            }
        }
        // Promote extension-only controls before canonical factory defaults merge.
        for key in ["scan_output", "conversion_style", "operator"] {
            if normalized["scanner"].get(key).is_none()
                && let Some(selection) = value["rust"]["runtime"]["scanner"].get(key)
            {
                if !normalized["scanner"].is_object() {
                    normalized["scanner"] = json!({});
                }
                normalized["scanner"][key] = selection.clone();
            }
        }
        // Older Rust states stored bit depth before export format existed.
        // Infer only an absent format; explicit combinations still face validation.
        if let Some(rust) = normalized.get_mut("rust").and_then(Value::as_object_mut)
            && !rust.contains_key("export_format")
        {
            let format = match rust.get("save_bit_depth").and_then(Value::as_u64) {
                Some(16 | 32) => "tiff",
                _ => "png",
            };
            rust.insert("export_format".into(), json!(format));
        }
        merge(&mut state.sections, &normalized);
        canonicalize_grain_state(&mut state.sections["grain"]);
        if let Some(extension) = normalized.get("rust") {
            if !matches!(
                extension["version"].as_u64(),
                Some(1..=CURRENT_STATE_VERSION)
            ) {
                bail!(
                    crate::app::Message::new("operations.state.version")
                        .arg("value", &extension["version"])
                );
            }
            if let Some(fields) = extension.as_object() {
                for (key, value) in fields {
                    state.sections["rust"][key] = value.clone();
                }
            }
        }
        state.sections["rust"]["version"] = json!(CURRENT_STATE_VERSION);
        state.sections["rust"]["model_version"] = json!(spektrafilm_core::runtime::MODEL_VERSION);
        let params = state.runtime_params()?;
        state.sections["scanner"]["conversion_style"] =
            json!(params.scanner.conversion_style_name());
        if state.sections["rust"]["runtime"]["scanner"].is_object() {
            state.sections["rust"]["runtime"]["scanner"]["conversion_style"] =
                json!(params.scanner.conversion_style_name());
        }

        // Metadata is derived from the selected built-ins, never trusted from an import.
        state.sections["rust"]["scanner_provenance"] = serde_json::to_value(
            spektrafilm_core::scanner_provenance::ScannerProvenance::from_params(&params),
        )?;
        if normalized.get("rust").is_none() {
            state
                .sections
                .as_object_mut()
                .expect("GUI state object")
                .remove("rust");
        }
        Ok(state)
    }
    /// Reject a state the runtime cannot accept. Load, import and save each
    /// call this once; [`Self::runtime_params`] is a pure projection that no
    /// longer re-validates.
    pub fn validate(&self) -> Result<()> {
        validate_state_parameters(&mut self.sections.clone(), None)
    }
    pub fn film(&self) -> &str {
        self.sections["simulation"]["film_stock"]
            .as_str()
            .unwrap_or("kodak_gold_200")
    }
    pub fn paper(&self) -> &str {
        self.sections["simulation"]["print_paper"]
            .as_str()
            .unwrap_or("kodak_supra_endura")
    }
    pub fn auto_preview(&self) -> bool {
        self.sections["simulation"]["auto_preview"]
            .as_bool()
            .unwrap_or(true)
    }
    pub fn display_profile(&self) -> Option<&str> {
        self.sections["rust"]["display_profile"].as_str()
    }
    /// Select the viewer's ICC display profile. An absent or empty selection
    /// keeps the platform discovery and the encoded-sRGB fallback, so the
    /// carrier stays optional in the persisted document.
    pub fn set_display_profile(&mut self, selection: Option<&str>) {
        if !self.sections["rust"].is_object() {
            self.sections["rust"] = json!({"version": CURRENT_STATE_VERSION});
        }
        match selection.filter(|path| !path.is_empty()) {
            Some(path) => self.sections["rust"]["display_profile"] = json!(path),
            None => {
                if let Some(extension) = self.sections["rust"].as_object_mut() {
                    extension.remove("display_profile");
                }
            }
        }
    }
    pub fn saving_color_space(&self) -> &str {
        self.sections["simulation"]["saving_color_space"]
            .as_str()
            .unwrap_or("sRGB")
    }
    pub fn saving_cctf_encoding(&self) -> bool {
        self.sections["simulation"]["saving_cctf_encoding"]
            .as_bool()
            .unwrap_or(true)
    }
    pub fn special(&self) -> &Value {
        &self.sections["special"]
    }
    pub fn dialog_dirs(&self) -> &Value {
        &self.sections["rust"]["dialog_dirs"]
    }
}

#[cfg(test)]
mod tests;
