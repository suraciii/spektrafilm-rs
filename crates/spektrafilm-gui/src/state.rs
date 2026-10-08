//! GUI state and runtime ownership pinned to upstream 28bf883.
use std::path::{Path, PathBuf};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use spektrafilm_core::params::RuntimeParams;

const INPUT_IO: &[&str] = &["input_color_space", "input_cctf_decoding", "upscale_factor", "crop", "crop_center", "crop_size"];
const INPUT_SETTINGS: &[&str] = &["rgb_to_raw_method", "apply_hanatos2025_adaptation_window", "apply_hanatos2025_adaptation_surface", "spectral_gaussian_blur"];
const RUNTIME_GROUPS: &[(&str, &str, &str)] = &[
    ("grain", "film_render", "grain"),
    ("halation", "film_render", "halation"),
    ("couplers", "film_render", "dir_couplers"),
    ("film_chemistry", "film_render", "chemistry"),
    ("chemistry", "print_render", "density_curves_morph"),
    ("film_base", "film_render", "base"),
    ("print_base", "print_render", "base"),
    ("convert", "film_render", "convert"),
    ("glare", "print_render", "glare"),
];

#[derive(Clone, Debug)]
pub struct GuiState { pub sections: Value }

/// Merge known leaves only, like upstream's dataclass merge. Unknown fields
/// never become runtime fields; Rust additions live in the versioned extension.
fn merge(target: &mut Value, source: &Value) {
    if let (Some(dst), Some(src)) = (target.as_object_mut(), source.as_object()) {
        for (key, value) in src {
            if let Some(current) = dst.get_mut(key) {
                if current.is_object() { merge(current, value); }
                else { *current = value.clone(); }
            }
        }
    }
}
fn copy_fields(target: &mut Value, source: &Value, names: &[&str]) {
    for name in names { if let Some(value) = source.get(*name) { target[*name] = value.clone(); } }
}
fn flatten(source: &Value, groups: &[&str]) -> Value {
    let mut flat = source.clone();
    for group in groups {
        if let Some(values) = source.get(*group).and_then(Value::as_object) {
            for (key,value) in values { flat[key] = value.clone(); }
        }
    }
    flat
}
fn normalize_grain_section(section: &mut Value) {
    let Some(object) = section.as_object_mut() else { return; };
    if !object.contains_key("particle_scale_sublayers") {
        if let Some(value) = object.get("particle_scale_layers").cloned() {
            object.insert("particle_scale_sublayers".to_string(), value);
        }
    }
    for key in [
        "sublayers_active",
        "particle_area_um2",
        "particle_scale",
        "particle_scale_layers",
        "n_sub_layers",
        "monochrome",
    ] {
        object.remove(key);
    }
}

impl GuiState {
    pub fn factory() -> Self {
        Self { sections: serde_json::from_str(include_str!("factory_state.json")).expect("pinned factory JSON") }
    }
    pub fn from_value(value: Value) -> Result<Self> {
        if !value.is_object() { bail!("GUI state must be a JSON object"); }
        let mut state = Self::factory();
        let mut normalized = value.clone();
        normalized["input_image"] = flatten(&value["input_image"], &["io", "settings"]);
        for (old,new) in [("apply_cctf_decoding","input_cctf_decoding"),("spectral_upsampling_method","rgb_to_raw_method")] {
            if normalized["input_image"].get(new).is_none() {
                if let Some(v) = value["input_image"].get(old) { normalized["input_image"][new] = v.clone(); }
            }
        }
        normalized["simulation"] = flatten(&value["simulation"], &["selection", "io"]);
        copy_fields(&mut normalized["simulation"], &value["simulation"]["workflow"], &["saving_color_space", "saving_cctf_encoding", "auto_preview"]);
        if let Some(enlarger) = value["simulation"].get("enlarger") {
            for (from,to) in [("illuminant","print_illuminant"),("print_exposure","print_exposure"),("print_exposure_compensation","print_exposure_compensation"),("y_filter_shift","print_y_filter_shift"),("m_filter_shift","print_m_filter_shift")] {
                if let Some(v) = enlarger.get(from) { normalized["simulation"][to] = v.clone(); }
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
            let original = value.get(section).or_else(|| value["gui_only"].get(section));
            if let Some(original) = original { normalized[section] = flatten(original, &["settings"]); }
        }
        normalize_grain_section(&mut normalized["grain"]);
        if let Some(grain) = normalized
            .get_mut("rust")
            .and_then(Value::as_object_mut)
            .and_then(|rust| rust.get_mut("runtime"))
            .and_then(Value::as_object_mut)
            .and_then(|runtime| runtime.get_mut("film_render"))
            .and_then(Value::as_object_mut)
            .and_then(|film_render| film_render.get_mut("grain"))
        {
            normalize_grain_section(grain);
        }
        merge(&mut state.sections, &normalized);
        if let Some(extension) = normalized.get("rust") {
            if extension["version"] != 1 { bail!("Unsupported Rust GUI state extension version"); }
            state.sections["rust"] = extension.clone();
        }
        state.runtime_params()?;
        Ok(state)
    }
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("Read GUI state {}",path.display()))?;
        Self::from_value(serde_json::from_str(&text).context("Invalid GUI state JSON")?)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.runtime_params()?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) { std::fs::create_dir_all(parent)?; }
        std::fs::write(path,serde_json::to_vec_pretty(&self.sections)?)?;
        Ok(())
    }
    pub fn film(&self) -> &str { self.sections["simulation"]["film_stock"].as_str().unwrap_or("kodak_gold_200") }
    pub fn paper(&self) -> &str { self.sections["simulation"]["print_paper"].as_str().unwrap_or("kodak_supra_endura") }
    pub fn auto_preview(&self) -> bool { self.sections["simulation"]["auto_preview"].as_bool().unwrap_or(true) }
    pub fn runtime_params(&self) -> Result<RuntimeParams> {
        let s = &self.sections;
        let mut runtime = serde_json::to_value(RuntimeParams::default())?;
        if let Some(extra) = s["rust"].get("runtime") { merge(&mut runtime,extra); }
        for group in ["camera","scanner"] { merge(&mut runtime[group], &s[group]); }
        for &(section, group, leaf) in RUNTIME_GROUPS { merge(&mut runtime[group][leaf], &s[section]); }
        runtime["film_render"]["development_time"] = s["film_chemistry"]["development_time"].clone();
        runtime["print_render"]["development_time"] = s["chemistry"]["development_time"].clone();
        copy_fields(&mut runtime["io"],&s["input_image"],INPUT_IO);
        copy_fields(&mut runtime["settings"],&s["input_image"],INPUT_SETTINGS);
        for section in ["input_gamut_compress","output_gamut_compress"] { merge(&mut runtime["io"][section],&s[section]); }
        // Dedicated diffusion panels override passthrough camera/preflash groups.
        merge(&mut runtime["camera"]["diffusion_filter"],&s["camera_diffusion"]);
        merge(&mut runtime["enlarger"]["diffusion_filter"],&s["enlarger_diffusion"]);
        copy_fields(&mut runtime["enlarger"],&s["preflashing"], &["preflash_exposure","preflash_y_filter_shift","preflash_m_filter_shift"]);
        for (from,to) in [("print_illuminant","illuminant"),("print_exposure","print_exposure"),("print_exposure_compensation","print_exposure_compensation"),("print_y_filter_shift","y_filter_shift"),("print_m_filter_shift","m_filter_shift")] { runtime["enlarger"][to] = s["simulation"][from].clone(); }
        runtime["workflow"]["route"] = s["simulation"]["route"].clone();
        runtime["io"]["scan_film"] = json!(s["simulation"]["route"] == "input > film > scan");
        copy_fields(&mut runtime["io"],&s["simulation"], &["output_color_space"]);
        runtime["settings"]["preview_max_size"] = s["display"]["preview_max_size"].clone();
        for (key,value) in [("use_enlarger_lut",json!(true)),("use_scanner_lut",json!(true)),("lut_resolution",json!(17)),("use_fast_stats",json!(true))] { runtime["settings"][key] = value; }
        let params: RuntimeParams = serde_json::from_value(runtime).context("Invalid GUI control value")?;
        params.validate().map_err(anyhow::Error::msg)?;
        for name in ["film_channel_swap","print_channel_swap"] {
            let order = s["special"][name].as_array().context("Channel swap must be an array")?;
            if order.len()!=3 || order.iter().any(|v| v.as_u64().is_none_or(|x| x>2)) { bail!("{name} must contain three channel indices in 0..2"); }
        }
        if !matches!(s["load_raw"]["white_balance"].as_str(),Some("as_shot"|"as-shot"|"daylight"|"tungsten"|"custom")) { bail!("Unknown RAW white balance"); }
        for (section,key) in [("simulation","auto_preview"),("simulation","saving_cctf_encoding"),("display","use_display_transform"),("display","gray_18_canvas"),("load_raw","lens_correction")] {
            if !s[section][key].is_boolean() { bail!("{section}.{key} must be a boolean"); }
        }
        for (section,key) in [("display","white_padding"),("load_raw","temperature"),("load_raw","tint")] {
            if s[section][key].as_f64().is_none_or(|v|!v.is_finite()) { bail!("{section}.{key} must be a finite number"); }
        }
        if s["simulation"]["film_stock"].as_str().is_none() || s["simulation"]["print_paper"].as_str().is_none() { bail!("Film and paper selections must be strings"); }
        let saving_space = s["simulation"]["saving_color_space"].as_str().context("Saving color space must be a string")?;
        spektrafilm_math::colorspace::resolve(saving_space).map_err(anyhow::Error::msg)?;
        if let Some(depth) = s["rust"].get("save_bit_depth") {
            if !matches!(depth.as_u64(),Some(8|16|32)) { bail!("Saving bit depth must be 8, 16 or 32"); }
        }
        if s["display"]["output_interpolation"].as_str().is_none() { bail!("Output interpolation must be a string"); }
        Ok(params)
    }
    pub fn from_runtime(params: &RuntimeParams, film: &str, paper: &str, extras: &Value) -> Result<Self> {
        let mut state = Self::from_value(extras.clone())?;
        let runtime = serde_json::to_value(params)?;
        let s = &mut state.sections;
        for group in ["camera","scanner"] { merge(&mut s[group],&runtime[group]); }
        for &(section, group, leaf) in RUNTIME_GROUPS { merge(&mut s[section], &runtime[group][leaf]); }
        s["film_chemistry"]["development_time"] = runtime["film_render"]["development_time"].clone();
        s["chemistry"]["development_time"] = runtime["print_render"]["development_time"].clone();
        merge(&mut s["preflashing"],&runtime["enlarger"]);
        s["camera_diffusion"] = runtime["camera"]["diffusion_filter"].clone();
        s["enlarger_diffusion"] = runtime["enlarger"]["diffusion_filter"].clone();
        copy_fields(&mut s["input_image"],&runtime["io"],INPUT_IO);
        copy_fields(&mut s["input_image"],&runtime["settings"],INPUT_SETTINGS);
        for section in ["input_gamut_compress","output_gamut_compress"] { merge(&mut s[section], &runtime["io"][section]); }
        s["simulation"]["film_stock"] = json!(film); s["simulation"]["print_paper"] = json!(paper);
        for (from,to) in [("illuminant","print_illuminant"),("print_exposure","print_exposure"),("print_exposure_compensation","print_exposure_compensation"),("y_filter_shift","print_y_filter_shift"),("m_filter_shift","print_m_filter_shift")] { s["simulation"][to]=runtime["enlarger"][from].clone(); }
        s["simulation"]["route"] = runtime["workflow"]["route"].clone();
        copy_fields(&mut s["simulation"],&runtime["io"], &["output_color_space"]);
        s["display"]["preview_max_size"] = runtime["settings"]["preview_max_size"].clone();
        s["rust"] = extras["rust"].as_object().map(|v| Value::Object(v.clone())).unwrap_or_else(||json!({}));
        s["rust"]["version"] = json!(1);
        s["rust"]["runtime"] = runtime;
        Ok(state)
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("SPEKTRAFILM_CONFIG_DIR") { return path.into(); }
    #[cfg(target_os="windows")]
    if let Some(path) = std::env::var_os("APPDATA") { return PathBuf::from(path).join("spektrafilm"); }
    #[cfg(target_os="macos")]
    if let Some(home) = std::env::var_os("HOME") { return PathBuf::from(home).join("Library/Application Support/spektrafilm"); }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") { return PathBuf::from(path).join("spektrafilm"); }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(||PathBuf::from(".")).join(".config/spektrafilm")
}
pub fn default_path() -> PathBuf { config_dir().join("gui_default_state.json") }
pub fn startup() -> Result<GuiState> {
    let path = default_path();
    let mut state = if path.exists() { GuiState::load(&path)? } else { GuiState::factory() };
    let directories = config_dir().join("dialog_dirs.json");
    if directories.exists() {
        let value: Value = serde_json::from_str(&std::fs::read_to_string(directories)?)?;
        if !value.is_object() { bail!("File dialog directories must be a JSON object"); }
        if !state.sections["rust"].is_object() { state.sections["rust"] = json!({"version":1}); }
        state.sections["rust"]["dialog_dirs"] = value;
    }
    Ok(state)
}
pub fn reset_factory() -> Result<GuiState> { let path=default_path(); if path.exists() { std::fs::remove_file(path)?; } Ok(GuiState::factory()) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_legacy_sections_use_factory_and_flat_gui_only_precedence() {
        let state=GuiState::from_value(json!({
            "input_image":{"apply_cctf_decoding":true,"crop":true,"crop_center":[0.2,0.7]},
            "simulation":{"saving_color_space":"ITU-R BT.2020"},
            "gui_only":{"display":{"gray_18_canvas":false}},
            "display":{"white_padding":0.1}
        })).unwrap();
        let params=state.runtime_params().unwrap();
        assert!(params.io.crop && params.io.input_cctf_decoding);
        assert_eq!(params.io.crop_center,[0.2,0.7]);
        assert_eq!(state.sections["display"]["gray_18_canvas"],true);
        assert_eq!(state.sections["display"]["white_padding"],0.1);
        assert_eq!(state.paper(),"kodak_supra_endura");
    }
    #[test]
    fn dedicated_diffusion_and_simulation_panels_win_over_passthrough_groups() {
        let state=GuiState::from_value(json!({
            "camera":{"diffusion_filter":{"strength":1.5}},
            "camera_diffusion":{"strength":0.25},
            "preflashing":{"print_exposure":8.0,"preflash_exposure":0.2},
            "simulation":{"print_exposure":1.75}
        })).unwrap();
        let params=state.runtime_params().unwrap();
        assert_eq!(params.camera.diffusion_filter.strength,0.25);
        assert_eq!(params.enlarger.print_exposure,1.75);
        assert_eq!(params.enlarger.preflash_exposure,0.2);
    }
    #[test]
    fn runtime_extensions_roundtrip_without_overriding_upstream_selection() {
        let factory=GuiState::factory();
        let mut params=factory.runtime_params().unwrap();
        params.io.output_cctf_encoding=false;
        params.film_render.development_time=Some(9.0);
        params.film_render.grain.engine=spektrafilm_core::params::GrainEngine::V2;
        params.film_render.grain.v2_profile="8mm500".into();
        params.film_render.grain.v2_amount=Some(0.0);
        let mut extras=factory.sections.clone();
        extras["rust"]=json!({"version":1,"viewer":{"zoom":3.0}});
        let saved=GuiState::from_runtime(&params,"kodak_gold_200","kodak_supra_endura",&extras).unwrap();
        let loaded=GuiState::from_value(saved.sections.clone()).unwrap();
        let restored=loaded.runtime_params().unwrap();
        assert!(!restored.io.output_cctf_encoding);
        assert_eq!(restored.film_render.development_time,Some(9.0));
        assert_eq!(restored.film_render.grain.engine,spektrafilm_core::params::GrainEngine::V2);
        assert_eq!(restored.film_render.grain.v2_profile,"8mm500");
        assert_eq!(restored.film_render.grain.resolved_grain_v2().amount,0.0);
        assert_eq!(loaded.sections["rust"]["viewer"]["zoom"],3.0);
        assert_eq!(loaded.sections,saved.sections);
    }
    #[test]
    fn grain_v2_migration_strips_runtime_only_fields() {
        let state = GuiState::from_value(json!({
            "grain": {
                "particle_scale_layers": [2.0, 1.0, 0.5],
                "particle_area_um2": 0.2,
                "n_sub_layers": 3
            },
            "rust": {
                "version": 1,
                "runtime": {
                    "film_render": {
                        "grain": {
                            "particle_scale_layers": [2.0, 1.0, 0.5],
                            "particle_area_um2": 0.2
                        }
                    }
                }
            }
        }))
        .unwrap();
        let grain = state.sections["grain"].as_object().unwrap();
        assert_eq!(grain["particle_scale_sublayers"], json!([2.0, 1.0, 0.5]));
        for key in [
            "sublayers_active",
            "particle_area_um2",
            "particle_scale",
            "particle_scale_layers",
            "n_sub_layers",
            "monochrome",
        ] {
            assert!(!grain.contains_key(key), "legacy field leaked into state: {key}");
        }
        let params = state.runtime_params().unwrap();
        let runtime_grain = serde_json::to_value(params).unwrap()["film_render"]["grain"].clone();
        assert!(runtime_grain.get("particle_scale_layers").is_none());
        assert_eq!(runtime_grain["particle_scale_sublayers"], json!([2.0, 1.0, 0.5]));
    }
    #[test]
    fn grain_v2_canonical_fields_roundtrip_through_state_and_runtime() {
        let factory = GuiState::factory();
        let mut params = factory.runtime_params().unwrap();
        let grain = &mut params.film_render.grain;
        grain.active = false;
        grain.rms_granularity = [11.0, 13.0, 17.0];
        grain.density_min = [0.11, 0.22, 0.33];
        grain.uniformity = [0.91, 0.92, 0.93];
        grain.particle_scale_sublayers = [1.0, 0.75, 0.5];
        grain.blur = 1.25;
        grain.mult_usm_sigma = 1.1;
        grain.mult_usm_amount = 2.2;
        grain.blur_dye_clouds_um = 3.3;
        grain.micro_structure = [0.4, 24.0];

        let saved = GuiState::from_runtime(
            &params,
            "kodak_gold_200",
            "kodak_supra_endura",
            &factory.sections,
        )
        .unwrap();
        let loaded = GuiState::from_value(saved.sections.clone()).unwrap();
        let restored = loaded.runtime_params().unwrap().film_render.grain;

        assert!(!restored.active);
        assert_eq!(restored.rms_granularity, [11.0, 13.0, 17.0]);
        assert_eq!(restored.density_min, [0.11, 0.22, 0.33]);
        assert_eq!(restored.uniformity, [0.91, 0.92, 0.93]);
        assert_eq!(restored.particle_scale_sublayers, [1.0, 0.75, 0.5]);
        assert_eq!(restored.blur, 1.25);
        assert_eq!(restored.mult_usm_sigma, 1.1);
        assert_eq!(restored.mult_usm_amount, 2.2);
        assert_eq!(restored.blur_dye_clouds_um, 3.3);
        assert_eq!(restored.micro_structure, [0.4, 24.0]);

        let object = loaded.sections["grain"].as_object().unwrap();
        assert_eq!(object.len(), 11);
        for key in [
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
        ] {
            assert!(object.contains_key(key), "canonical field missing: {key}");
        }
    }

    #[test]
    fn canonical_sections_roundtrip_without_runtime_extension() {
        let factory = GuiState::factory();
        let mut params = factory.runtime_params().unwrap();
        params.camera.film_format_mm = 70.0;
        params.camera.color_filter = "hoya_y2".into();
        params.film_render.grain.micro_sublayers = 3;
        params.film_render.dir_couplers.langmuir_donor_k_rgb = [0.5, 1.5, 2.5];
        params.film_render.dir_couplers.langmuir_receiver_k_rgb = [0.75, 1.75, 2.75];
        params.io.input_gamut_compress.hull_detail = 9.0;
        params.io.input_gamut_compress.boundary = "locus".into();
        params.film_render.development_time = Some(9.0);
        params.print_render.development_time = Some(4.0);
        params.film_render.chemistry.gamma_factor_red = 1.25;
        params.print_render.density_curves_morph.gamma_factor_blue = 0.75;
        params.film_render.base.tilt = 0.2;
        params.film_render.base.cyan = 1.2;
        params.print_render.base.scale = 0.8;
        params.film_render.convert.scan_illuminant = "D65".into();
        params.film_render.convert.base_percentile = 97.5;
        params.film_render.convert.calibration = "1.1 0 0  0 1 0  0 0 0.9".into();
        params.workflow.route = "input > convert-film > scan-minus-base".into();
        let mut saved = GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections).unwrap().sections;
        saved.as_object_mut().unwrap().remove("rust");
        let loaded = GuiState::from_value(saved.clone()).unwrap();
        let restored = loaded.runtime_params().unwrap();
        assert_eq!(restored.camera.film_format_mm, 70.0);
        assert_eq!(restored.camera.color_filter, "hoya_y2");
        assert_eq!(restored.film_render.grain.micro_sublayers, 3);
        assert_eq!(restored.film_render.dir_couplers.langmuir_donor_k_rgb, [0.5, 1.5, 2.5]);
        assert_eq!(restored.film_render.dir_couplers.langmuir_receiver_k_rgb, [0.75, 1.75, 2.75]);
        assert_eq!(restored.io.input_gamut_compress.hull_detail, 9.0);
        assert_eq!(restored.io.input_gamut_compress.boundary, "locus");
        assert_eq!(restored.film_render.development_time, Some(9.0));
        assert_eq!(restored.print_render.development_time, Some(4.0));
        assert_eq!(restored.film_render.chemistry.gamma_factor_red, 1.25);
        assert_eq!(restored.print_render.density_curves_morph.gamma_factor_blue, 0.75);
        assert_eq!(restored.film_render.base.tilt, 0.2);
        assert_eq!(restored.film_render.base.cyan, 1.2);
        assert_eq!(restored.print_render.base.scale, 0.8);
        assert_eq!(restored.film_render.convert.scan_illuminant, "D65");
        assert_eq!(restored.film_render.convert.base_percentile, 97.5);
        assert_eq!(restored.film_render.convert.calibration, "1.1 0 0  0 1 0  0 0 0.9");
        assert_eq!(restored.workflow.route, "input > convert-film > scan-minus-base");
        assert_eq!(loaded.sections, saved);
        assert!(saved["simulation"].get("workflow").is_none());
        assert!(saved["simulation"].get("scan_film").is_none());
        assert!(saved["camera"].get("filter_uv").is_none());
        assert!(saved["camera"].get("filter_ir").is_none());
        assert!(saved["special"].get("film_gamma_factor").is_none());
    }

    #[test]
    fn canonical_chemistry_base_and_route_override_runtime_extension() {
        let state = GuiState::from_value(json!({
            "film_chemistry": {"development_time": 9.0, "gamma_factor_red": 1.25},
            "chemistry": {"development_time": 4.0, "gamma_factor_blue": 0.75},
            "film_base": {"cyan": 1.2},
            "print_base": {"scale": 0.8},
            "convert": {"base_percentile": 97.5},
            "simulation": {"route": "input > film > scan"},
            "rust": {"version": 1, "runtime": {
                "film_render": {"development_time": 5.0, "chemistry": {"gamma_factor_red": 2.0}, "base": {"cyan": 2.0}, "convert": {"base_percentile": 99.0}},
                "print_render": {"development_time": 2.0, "density_curves_morph": {"gamma_factor_blue": 2.0}, "base": {"scale": 2.0}},
                "workflow": {"route": "input"}
            }}
        })).unwrap();
        let params = state.runtime_params().unwrap();
        assert_eq!(params.film_render.development_time, Some(9.0));
        assert_eq!(params.print_render.development_time, Some(4.0));
        assert_eq!(params.film_render.chemistry.gamma_factor_red, 1.25);
        assert_eq!(params.print_render.density_curves_morph.gamma_factor_blue, 0.75);
        assert_eq!(params.film_render.base.cyan, 1.2);
        assert_eq!(params.print_render.base.scale, 0.8);
        assert_eq!(params.film_render.convert.base_percentile, 97.5);
        assert_eq!(params.workflow.route, "input > film > scan");
    }

    #[test]
    fn upstream_legacy_scan_flag_is_read_only_and_route_wins() {
        let legacy = GuiState::from_value(json!({"simulation": {"scan_film": true}})).unwrap();
        assert_eq!(legacy.runtime_params().unwrap().workflow.route, "input > film > scan");
        assert!(legacy.sections["simulation"].get("scan_film").is_none());
        let explicit = GuiState::from_value(json!({"simulation": {"scan_film": true, "route": "input"}})).unwrap();
        assert_eq!(explicit.runtime_params().unwrap().workflow.route, "input");
    }

    #[test]
    fn nested_upstream_state_flattens_only_owned_workflow_leaves() {
        let state = GuiState::from_value(json!({
            "input_image": {"io": {"crop": true}, "settings": {"spectral_gaussian_blur": 2.0}},
            "simulation": {
                "selection": {"film_stock": "kodak_doublex"},
                "route": "input > film > scan",
                "workflow": {"route": "input", "auto_preview": false, "saving_color_space": "ITU-R BT.2020"},
                "enlarger": {"print_exposure": 1.5, "y_filter_shift": 7.0},
                "io": {"output_color_space": "ITU-R BT.2020"}
            }
        })).unwrap();
        let params = state.runtime_params().unwrap();
        assert!(params.io.crop);
        assert_eq!(params.settings.spectral_gaussian_blur, 2.0);
        assert_eq!(params.workflow.route, "input > film > scan");
        assert_eq!(params.enlarger.print_exposure, 1.5);
        assert_eq!(params.enlarger.y_filter_shift, 7.0);
        assert_eq!(params.io.output_color_space, "ITU-R BT.2020");
        assert_eq!(state.film(), "kodak_doublex");
        assert!(!state.auto_preview());
        assert_eq!(state.sections["simulation"]["saving_color_space"], "ITU-R BT.2020");
        assert!(state.sections["simulation"].get("workflow").is_none());
    }

    #[test]
    fn malformed_controls_and_future_extensions_fail_before_application() {
        for value in [json!([]),json!({"camera":{"auto_exposure":"bad"}}),json!({"rust":{"version":9}}),json!({"special":{"film_channel_swap":[0,1,3]}}),json!({"simulation":{"auto_preview":"false"}})] {
            assert!(GuiState::from_value(value).is_err());
        }
    }
}
