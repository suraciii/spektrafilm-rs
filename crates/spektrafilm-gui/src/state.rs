//! GUI state and runtime ownership pinned to upstream 28bf883.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use spektrafilm_core::params::RuntimeParams;
use std::path::{Path, PathBuf};

const INPUT_IO: &[&str] = &[
    "input_color_space",
    "input_cctf_decoding",
    "upscale_factor",
    "crop",
    "crop_center",
    "crop_size",
];
const INPUT_SETTINGS: &[&str] = &[
    "rgb_to_raw_method",
    "apply_hanatos2025_adaptation_window",
    "apply_hanatos2025_adaptation_surface",
    "spectral_gaussian_blur",
];
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
/// Enlarger controls the upstream GUI also exposes under `simulation`
/// aliases, as `(simulation key, enlarger key)`. Both directions of the
/// state ⇄ runtime projection consume this one table, so an alias cannot be
/// added for only one of them.
const ENLARGER_SIMULATION_ALIASES: &[(&str, &str)] = &[
    ("print_illuminant", "illuminant"),
    ("print_exposure", "print_exposure"),
    ("print_exposure_compensation", "print_exposure_compensation"),
    ("print_y_filter_shift", "y_filter_shift"),
    ("print_m_filter_shift", "m_filter_shift"),
];

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
fn normalize_grain_section(section: &mut Value) {
    let Some(object) = section.as_object_mut() else {
        return;
    };
    if let Some(value) = object.get("particle_scale_layers").cloned() {
        object.insert("particle_scale_sublayers".to_string(), value);
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

fn validate_state_parameters(value: &mut Value, legacy: bool) -> Result<()> {
    if value.get("output_mix").is_some() {
        bail!("output_mix belongs in rust.runtime.output_mix in GUI state");
    }
    let mut explicit = value["rust"]["runtime"].clone();
    if explicit.is_null() {
        explicit = json!({});
    }
    if legacy {
        spektrafilm_core::params::sources::normalize_legacy_complete_params(value)
            .map_err(anyhow::Error::msg)?;
    } else if value["io"]["output_gamut_compress"]
        .get("lightness_compression")
        .is_some()
    {
        bail!(
            "io.output_gamut_compress.lightness_compression is retired; explicitly remove this setting from the document"
        );
    }
    if !legacy
        && (explicit["io"]["output_gamut_compress"]
            .get("lightness_compression")
            .is_some()
            || value["output_gamut_compress"]
                .get("lightness_compression")
                .is_some())
    {
        bail!(
            "io.output_gamut_compress.lightness_compression is retired; explicitly remove this setting from the document"
        );
    }
    if legacy {
        spektrafilm_core::params::sources::normalize_legacy_complete_params(&mut explicit)
            .map_err(anyhow::Error::msg)?;
    }
    let _: RuntimeParams =
        serde_json::from_value(explicit.clone()).context("Invalid GUI Rust runtime parameters")?;
    for (section, group, leaf) in [
        ("output_gamut_compress", "io", "output_gamut_compress"),
        ("preflashing", "enlarger", ""),
    ] {
        if let Some(fields) = value.get(section) {
            if !explicit[group].is_object() {
                explicit[group] = json!({});
            }
            if leaf.is_empty() {
                overlay_runtime(&mut explicit[group], fields);
            } else {
                explicit[group][leaf] = fields.clone();
            }
        }
    }
    for group in [
        "film_render",
        "print_render",
        "enlarger",
        "reproduction",
        "io",
    ] {
        if let Some(fields) = value.get(group) {
            explicit[group] = fields.clone();
        }
    }
    if let Some(enlarger) = value["simulation"].get("enlarger") {
        if !explicit["enlarger"].is_object() {
            explicit["enlarger"] = json!({});
        }
        overlay_runtime(&mut explicit["enlarger"], enlarger);
    }
    if let Some(io) = value["simulation"].get("io") {
        let mut projected = json!({"io": io});
        if legacy {
            spektrafilm_core::params::sources::normalize_legacy_complete_params(&mut projected)
                .map_err(anyhow::Error::msg)?;
        } else if io["output_gamut_compress"]
            .get("lightness_compression")
            .is_some()
        {
            bail!(
                "simulation.io.output_gamut_compress.lightness_compression is retired; explicitly remove this setting from the document"
            );
        }
        let _: RuntimeParams = serde_json::from_value(projected.clone())
            .context("Invalid GUI simulation.io parameters")?;
        if legacy {
            value["simulation"]["io"] = projected["io"].clone();
        }
    }
    // Inspect both carriers independently so an ignored duplicate cannot hide a retired value.
    if let Some(output) = value.get_mut("output_gamut_compress") {
        let mut projected = json!({"io": {"output_gamut_compress": output.clone()}});
        if legacy {
            spektrafilm_core::params::sources::normalize_legacy_complete_params(&mut projected)
                .map_err(anyhow::Error::msg)?;
        }
        let _: RuntimeParams =
            serde_json::from_value(projected).context("Invalid GUI io.output_gamut_compress")?;
        if legacy {
            output
                .as_object_mut()
                .map(|fields| fields.remove("lightness_compression"));
        }
    }
    if legacy {
        spektrafilm_core::params::sources::normalize_legacy_complete_params(&mut explicit)
            .map_err(anyhow::Error::msg)?;
        if let Some(runtime) = value["rust"].get_mut("runtime") {
            spektrafilm_core::params::sources::normalize_legacy_complete_params(runtime)
                .map_err(anyhow::Error::msg)?;
        }
    }
    let _: RuntimeParams =
        serde_json::from_value(explicit).context("Invalid GUI runtime parameters")?;
    Ok(())
}

impl GuiState {
    pub fn factory() -> Self {
        let mut sections: Value =
            serde_json::from_str(include_str!("factory_state.json")).expect("pinned factory JSON");
        let scanner = serde_json::to_value(RuntimeParams::default().scanner)
            .expect("scanner defaults serialize");
        for key in [
            "receiver_profile",
            "positive_interpretation",
            "auto_setup",
            "finish",
            "finish_strength",
            "reference_exposure_ev",
            "reference_balance_ev",
        ] {
            sections["scanner"][key] = scanner[key].clone();
        }
        sections["rust"]["scanner_provenance"] = serde_json::to_value(
            spektrafilm_core::scan_receiver::ScannerProvenance::from_params(
                &RuntimeParams::default(),
            ),
        )
        .expect("scanner provenance serializes");
        sections["rust"]["model_version"] = json!(spektrafilm_core::runtime::MODEL_VERSION);
        Self { sections }
    }
    pub fn from_value(mut value: Value) -> Result<Self> {
        if !value.is_object() {
            bail!("GUI state must be a JSON object");
        }
        let version = value["rust"].get("version").and_then(Value::as_u64);
        if value.get("rust").is_some() && !matches!(version, Some(1 | 2)) {
            bail!("Unsupported Rust GUI state extension version");
        }
        validate_state_parameters(&mut value, version == Some(1))?;
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
        for key in [
            "scan_output",
            "receiver_profile",
            "positive_interpretation",
            "auto_setup",
            "finish",
            "finish_strength",
            "reference_exposure_ev",
            "reference_balance_ev",
        ] {
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
            if !matches!(extension["version"].as_u64(), Some(1 | 2)) {
                bail!("Unsupported Rust GUI state extension version");
            }
            if let Some(fields) = extension.as_object() {
                for (key, value) in fields {
                    state.sections["rust"][key] = value.clone();
                }
            }
        }
        state.sections["rust"]["version"] = json!(2);
        state.sections["rust"]["model_version"] = json!(spektrafilm_core::runtime::MODEL_VERSION);
        let params = state.runtime_params()?;
        // Metadata is derived from the selected built-ins, never trusted from an import.
        state.sections["rust"]["scanner_provenance"] = serde_json::to_value(
            spektrafilm_core::scan_receiver::ScannerProvenance::from_params(&params),
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
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Read GUI state {}", path.display()))?;
        Self::from_value(serde_json::from_str(&text).context("Invalid GUI state JSON")?)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        self.runtime_params()?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(&self.sections)?)?;
        Ok(())
    }
    /// Reject a state the runtime cannot accept. Load, import and save each
    /// call this once; [`Self::runtime_params`] is a pure projection that no
    /// longer re-validates.
    pub fn validate(&self) -> Result<()> {
        validate_state_parameters(&mut self.sections.clone(), false)
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
    /// Project the sections into runtime parameters. Validation is split from
    /// projection: the load, import and save paths each validate once, so this
    /// is a pure read that never clones or re-checks the carriers.
    pub fn runtime_params(&self) -> Result<RuntimeParams> {
        let s = &self.sections;
        let mut runtime = serde_json::to_value(RuntimeParams::default())?;
        if let Some(extra) = s["rust"].get("runtime") {
            overlay_runtime(&mut runtime, extra);
        }
        for group in ["camera", "scanner"] {
            merge(&mut runtime[group], &s[group]);
        }
        merge(
            &mut runtime["magazine_print_color"],
            &s["magazine_print_color"],
        );
        for &(section, group, leaf) in RUNTIME_GROUPS {
            merge(&mut runtime[group][leaf], &s[section]);
        }
        runtime["film_render"]["development_time"] =
            s["film_chemistry"]["development_time"].clone();
        runtime["print_render"]["development_time"] = s["chemistry"]["development_time"].clone();
        copy_fields(&mut runtime["io"], &s["input_image"], INPUT_IO);
        copy_fields(&mut runtime["settings"], &s["input_image"], INPUT_SETTINGS);
        for section in ["input_gamut_compress", "output_gamut_compress"] {
            merge(&mut runtime["io"][section], &s[section]);
        }
        // Dedicated diffusion panels override passthrough camera/preflash groups.
        merge(
            &mut runtime["camera"]["diffusion_filter"],
            &s["camera_diffusion"],
        );
        merge(
            &mut runtime["enlarger"]["diffusion_filter"],
            &s["enlarger_diffusion"],
        );
        merge(&mut runtime["enlarger"], &s["preflashing"]);
        for &(simulation_key, enlarger_key) in ENLARGER_SIMULATION_ALIASES {
            runtime["enlarger"][enlarger_key] = s["simulation"][simulation_key].clone();
        }
        runtime["workflow"]["route"] = s["simulation"]["route"].clone();
        runtime["io"]["scan_film"] = json!(matches!(
            s["simulation"]["route"].as_str(),
            Some("input > film > scan")
        ));
        copy_fields(
            &mut runtime["io"],
            &s["simulation"],
            &["output_color_space"],
        );
        runtime["settings"]["preview_max_size"] = s["display"]["preview_max_size"].clone();
        for (key, value) in [
            ("use_enlarger_lut", json!(true)),
            ("use_scanner_lut", json!(true)),
            ("lut_resolution", json!(17)),
            ("use_fast_stats", json!(true)),
        ] {
            runtime["settings"][key] = value;
        }
        let mut params: RuntimeParams =
            serde_json::from_value(runtime).context("Invalid GUI control value")?;
        if let Some(preparation) = s["rust"].get("render_preparation") {
            let preparation = serde_json::from_value(preparation.clone())
                .context("Invalid render preparation ownership")?;
            params.restore_render_preparation_state(&preparation);
        }
        params.validate().map_err(anyhow::Error::msg)?;
        for name in ["film_channel_swap", "print_channel_swap"] {
            let order = s["special"][name]
                .as_array()
                .context("Channel swap must be an array")?;
            if order.len() != 3 || order.iter().any(|v| v.as_u64().is_none_or(|x| x > 2)) || {
                let mut seen = [false; 3];
                let mut duplicate = false;
                for v in order {
                    let idx = v.as_u64().unwrap_or(0) as usize;
                    if seen[idx] {
                        duplicate = true;
                        break;
                    }
                    seen[idx] = true;
                }
                duplicate
            } {
                bail!("{name} must be a permutation of [0, 1, 2]");
            }
        }
        if !matches!(
            s["load_raw"]["white_balance"].as_str(),
            Some("as_shot" | "as-shot" | "daylight" | "tungsten" | "custom")
        ) {
            bail!("Unknown RAW white balance");
        }
        for (section, key) in [
            ("simulation", "auto_preview"),
            ("simulation", "saving_cctf_encoding"),
            ("display", "use_display_transform"),
            ("display", "gray_18_canvas"),
            ("load_raw", "lens_correction"),
        ] {
            if !s[section][key].is_boolean() {
                bail!("{section}.{key} must be a boolean");
            }
        }
        for (section, key) in [
            ("display", "white_padding"),
            ("load_raw", "temperature"),
            ("load_raw", "tint"),
        ] {
            if s[section][key].as_f64().is_none_or(|v| !v.is_finite()) {
                bail!("{section}.{key} must be a finite number");
            }
        }
        if s["simulation"]["film_stock"].as_str().is_none()
            || s["simulation"]["print_paper"].as_str().is_none()
        {
            bail!("Film and paper selections must be strings");
        }
        let saving_space = s["simulation"]["saving_color_space"]
            .as_str()
            .context("Saving color space must be a string")?;
        spektrafilm_math::colorspace::resolve(saving_space).map_err(anyhow::Error::msg)?;
        if let Some(rust) = s["rust"].as_object() {
            if let Some(depth) = rust.get("save_bit_depth")
                && !matches!(depth.as_u64(), Some(8 | 16 | 32))
            {
                bail!("Saving bit depth must be 8, 16 or 32");
            }
            use spektrafilm_core::image_io::{
                BitDepth, ImageFormat, SaveOptionViolation as V, SaveOptions,
                save_option_violations,
            };
            let format = rust
                .get("export_format")
                .and_then(Value::as_str)
                .and_then(|name| match name {
                    "jpeg" => Some(ImageFormat::Jpeg),
                    "png" => Some(ImageFormat::Png),
                    "tiff" => Some(ImageFormat::Tiff),
                    "exr" => Some(ImageFormat::Exr),
                    _ => None,
                });
            // The container rules live in core; each site maps a violation to
            // its own wording, so this document check reads the shared table
            // instead of restating it.
            fn options<'a>(depth: BitDepth, color_space: &'a str, cctf: bool) -> SaveOptions<'a> {
                SaveOptions {
                    depth,
                    color_space,
                    cctf_encoding: cctf,
                    jpeg_quality: None,
                    jpeg_subsampling: None,
                    compression: None,
                }
            }
            if let Some(space) = rust.get("export_saving_color_space") {
                let space = space
                    .as_str()
                    .context("Export saving color space must be a string")?;
                spektrafilm_math::colorspace::resolve(space).map_err(anyhow::Error::msg)?;
                if format == Some(ImageFormat::Exr)
                    && save_option_violations(
                        ImageFormat::Exr,
                        &options(BitDepth::Sixteen, space, true),
                        false,
                    )
                    .contains(&V::ExrColorSpace)
                {
                    bail!("EXR saving color space must be sRGB or ACES2065-1");
                }
            }
            if let Some(encoded) = rust.get("export_saving_cctf_encoding") {
                let encoded = encoded
                    .as_bool()
                    .context("Export saving CCTF encoding must be a boolean")?;
                if let Some(format) = format {
                    for violation in save_option_violations(
                        format,
                        &options(BitDepth::Sixteen, "sRGB", encoded),
                        false,
                    ) {
                        match violation {
                            V::RequiresLinear => bail!("EXR exports must use linear color"),
                            V::RequiresEncoded => {
                                bail!("JPEG and PNG exports require encoded color")
                            }
                            _ => {}
                        }
                    }
                }
            }
            if let Some(export_format) = rust.get("export_format") {
                let export_format = export_format
                    .as_str()
                    .context("Export format must be a string")?;
                if !matches!(export_format, "jpeg" | "png" | "tiff" | "exr") {
                    bail!("Unknown export format");
                }
                let quality = rust
                    .get("jpeg_quality")
                    .and_then(Value::as_u64)
                    .context("JPEG quality must be an integer")?;
                if !(1..=100).contains(&quality) {
                    bail!("JPEG quality must be 1..=100");
                }
                if !matches!(
                    rust.get("jpeg_subsampling").and_then(Value::as_str),
                    Some("444" | "420")
                ) {
                    bail!("JPEG subsampling must be 444 or 420");
                }
                if !matches!(
                    rust.get("export_compression").and_then(Value::as_str),
                    Some("zip" | "none")
                ) {
                    bail!("Unknown export compression");
                }
                let depth = match rust
                    .get("save_bit_depth")
                    .and_then(Value::as_u64)
                    .unwrap_or(16)
                {
                    8 => BitDepth::Eight,
                    32 => BitDepth::ThirtyTwo,
                    _ => BitDepth::Sixteen,
                };
                // Only the depth variant carries a state-document message; the
                // transfer and color variants were already rejected above.
                let format = format.expect("export format validated above");
                if save_option_violations(format, &options(depth, "sRGB", true), false)
                    .contains(&V::Depth)
                {
                    if format == ImageFormat::Exr {
                        bail!("EXR exports require 16- or 32-bit depth");
                    }
                    bail!("{export_format} exports require 8-bit depth");
                }
            }
        }
        if s["display"]["output_interpolation"].as_str().is_none() {
            bail!("Output interpolation must be a string");
        }
        Ok(params)
    }
    pub fn from_runtime(
        params: &RuntimeParams,
        film: &str,
        paper: &str,
        extras: &Value,
    ) -> Result<Self> {
        let mut base = Self::factory().sections;
        merge(&mut base, extras);
        let mut state = Self::from_value(base)?;
        let mut runtime = serde_json::to_value(params)?;
        let s = &mut state.sections;
        for group in ["camera", "scanner"] {
            merge(&mut s[group], &runtime[group]);
        }
        s["scanner"]["scan_output"] = json!(
            runtime["scanner"]["scan_output"]
                .as_str()
                .unwrap_or("direct_scan")
        );
        merge(
            &mut s["magazine_print_color"],
            &runtime["magazine_print_color"],
        );
        for &(section, group, leaf) in RUNTIME_GROUPS {
            merge(&mut s[section], &runtime[group][leaf]);
        }
        s["film_chemistry"]["development_time"] =
            runtime["film_render"]["development_time"].clone();
        s["chemistry"]["development_time"] = runtime["print_render"]["development_time"].clone();
        merge(&mut s["preflashing"], &runtime["enlarger"]);
        s["camera_diffusion"] = runtime["camera"]["diffusion_filter"].clone();
        s["enlarger_diffusion"] = runtime["enlarger"]["diffusion_filter"].clone();
        copy_fields(&mut s["input_image"], &runtime["io"], INPUT_IO);
        copy_fields(&mut s["input_image"], &runtime["settings"], INPUT_SETTINGS);
        for section in ["input_gamut_compress", "output_gamut_compress"] {
            merge(&mut s[section], &runtime["io"][section]);
        }
        s["simulation"]["film_stock"] = json!(film);
        s["simulation"]["print_paper"] = json!(paper);
        for &(simulation_key, enlarger_key) in ENLARGER_SIMULATION_ALIASES {
            s["simulation"][simulation_key] = runtime["enlarger"][enlarger_key].clone();
        }
        s["simulation"]["route"] = runtime["workflow"]["route"].clone();
        copy_fields(
            &mut s["simulation"],
            &runtime["io"],
            &["output_color_space"],
        );
        s["display"]["preview_max_size"] = runtime["settings"]["preview_max_size"].clone();
        if let Some(extra) = extras["rust"].as_object() {
            for (key, value) in extra {
                if key != "runtime" {
                    s["rust"][key] = value.clone();
                }
            }
        }
        s["rust"]["version"] = json!(2);
        s["rust"]["model_version"] = json!(spektrafilm_core::runtime::MODEL_VERSION);
        s["rust"]["render_preparation"] = serde_json::to_value(params.render_preparation_state())?;
        s["rust"]["scanner_provenance"] = serde_json::to_value(
            spektrafilm_core::scan_receiver::ScannerProvenance::from_params(params),
        )?;
        // Route owns the derived scan decision; do not persist the legacy
        // `scan_film` flag or a second workflow owner in the Rust snapshot.
        runtime["io"]
            .as_object_mut()
            .map(|io| io.remove("scan_film"));
        runtime
            .as_object_mut()
            .and_then(|root| root.get_mut("workflow"))
            .and_then(Value::as_object_mut)
            .map(|workflow| workflow.remove("route"));
        if let Some(grain) = s.get_mut("grain") {
            canonicalize_grain_state(grain);
        }
        s["rust"]["runtime"] = runtime;
        Ok(state)
    }
}

pub use spektrafilm_core::config::config_dir;
/// Startup default state inside an already resolved session directory.
/// Production resolves that directory to the platform configuration
/// directory; acceptance supplies an isolated one.
pub fn default_path(directory: &Path) -> PathBuf {
    directory.join("gui_default_state.json")
}
pub fn startup(directory: &Path) -> Result<GuiState> {
    let path = directory.join("gui_default_state.json");
    let mut state = if path.exists() {
        GuiState::load(&path)?
    } else {
        GuiState::factory()
    };
    let directories = directory.join("dialog_dirs.json");
    if directories.exists() {
        let value: Value = serde_json::from_str(&std::fs::read_to_string(directories)?)?;
        if !value.is_object() {
            bail!("File dialog directories must be a JSON object");
        }
        if !state.sections["rust"].is_object() {
            state.sections["rust"] = json!({"version":1});
        }
        state.sections["rust"]["dialog_dirs"] = value;
    }
    Ok(state)
}
pub fn reset_factory(directory: &Path) -> Result<GuiState> {
    let path = default_path(directory);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(GuiState::factory())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_mix_state_roundtrip_and_legacy_retirement_are_atomic() {
        let factory = GuiState::factory();
        let mut params = factory.runtime_params().unwrap();
        params.output_mix.amount = 0.25;
        let saved =
            GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
                .unwrap();
        assert_eq!(saved.sections["rust"]["version"], 2);
        let restored = GuiState::from_value(saved.sections.clone()).unwrap();
        assert_eq!(restored.runtime_params().unwrap().output_mix.amount, 0.25);
        for version in [1, 2] {
            for retired in [Value::Null, json!([0.95, 1.0, 1.6])] {
                let mut candidate = saved.sections.clone();
                candidate["rust"]["version"] = json!(version);
                candidate["output_gamut_compress"]["lightness_compression"] = retired.clone();
                let source = candidate.clone();
                let result = GuiState::from_value(candidate.clone());
                if version == 1 && retired.is_null() {
                    assert!(
                        result.unwrap().sections["output_gamut_compress"]
                            .get("lightness_compression")
                            .is_none()
                    );
                } else {
                    assert!(
                        format!("{:#}", result.unwrap_err())
                            .contains("io.output_gamut_compress.lightness_compression")
                    );
                }
                assert_eq!(candidate, source);
                assert_eq!(restored.runtime_params().unwrap().output_mix.amount, 0.25);
            }
        }
        for invalid in [
            json!({"film_render":{"response":{}}}),
            json!({"print_render":{"response":{}}}),
            json!({"enlarger":{"c_filter_shift":0.0}}),
            json!({"output_mix":{"amount":1.01}}),
            json!({"output_mix":{"amount":-0.01}}),
        ] {
            let mut candidate = saved.sections.clone();
            candidate["rust"]["runtime"] = invalid;
            assert!(GuiState::from_value(candidate).is_err());
        }
    }

    #[test]
    fn state_carriers_reject_ignored_mix_and_nested_retired_io() {
        for amount in [0.25, 1.01, -0.01] {
            let error = GuiState::from_value(json!({"output_mix":{"amount":amount}})).unwrap_err();
            assert!(error.to_string().contains("rust.runtime.output_mix"));
        }
        for version in [1, 2] {
            for retired in [Value::Null, json!([0.95, 1.0, 1.6])] {
                let source = json!({"rust":{"version":version},"simulation":{"io":{"output_gamut_compress":{"lightness_compression":retired}}}});
                let result = GuiState::from_value(source.clone());
                if version == 1 && retired.is_null() {
                    let accepted = result.unwrap();
                    assert!(
                        accepted.sections["simulation"]["io"]["output_gamut_compress"]
                            .get("lightness_compression")
                            .is_none()
                    );
                    assert_eq!(accepted.runtime_params().unwrap().output_mix.amount, 1.0);
                } else {
                    assert!(
                        format!("{:#}", result.unwrap_err())
                            .contains("output_gamut_compress.lightness_compression")
                    );
                }
            }
            let error = GuiState::from_value(json!({"rust":{"version":version},"simulation":{"io":{"finished_rgb_output_processing":true}}})).unwrap_err();
            assert!(format!("{error:#}").contains("finished_rgb_output_processing"));
        }
    }

    #[test]
    fn legacy_rust_grain_snapshot_preserves_supported_controls() {
        for retired in [None, Some(Value::Null)] {
            let mut source = json!({"rust":{"version":1,"runtime":{"film_render":{"grain":{"sublayers_active":false,"particle_area_um2":9.0,"particle_scale":[2.0,3.0,4.0],"n_sub_layers":3,"monochrome":true}}}}});
            if let Some(retired) = retired {
                source["rust"]["runtime"]["io"] =
                    json!({"output_gamut_compress":{"lightness_compression":retired}});
            }
            let accepted = GuiState::from_value(source).unwrap();
            let params = accepted.runtime_params().unwrap();
            let grain = &params.film_render.grain;
            assert!(!grain.sublayers_active);
            assert_eq!(grain.particle_area_um2, 9.0);
            assert_eq!(grain.particle_scale, [2.0, 3.0, 4.0]);
            assert_eq!(grain.n_sub_layers, 3);
            assert!(grain.monochrome);
            let saved = GuiState::from_runtime(
                &params,
                accepted.film(),
                accepted.paper(),
                &accepted.sections,
            )
            .unwrap();
            let reopened = GuiState::from_value(saved.sections).unwrap();
            assert_eq!(
                reopened.runtime_params().unwrap().film_render.grain,
                params.film_render.grain
            );
        }
    }

    #[test]
    fn factory_matches_independently_generated_upstream_state() {
        // Generated from pinned Python PROJECT_DEFAULT_GUI_STATE with real
        // profile/preset loading; provenance lives beside this retained oracle.
        let mut expected: Value = serde_json::from_str(include_str!(
            "../../../scripts/parity/fixtures/gui_28bf883/factory_state.json"
        ))
        .unwrap();
        expected["output_gamut_compress"]
            .as_object_mut()
            .unwrap()
            .remove("lightness_compression");
        fn compare(actual: &Value, expected: &Value, path: &str) -> usize {
            match (actual, expected) {
                (Value::Object(actual), Value::Object(expected)) => {
                    assert_eq!(actual.len(), expected.len(), "section keys at {path}");
                    expected
                        .iter()
                        .map(|(key, value)| {
                            compare(
                                actual
                                    .get(key)
                                    .unwrap_or_else(|| panic!("missing {path}.{key}")),
                                value,
                                &format!("{path}.{key}"),
                            )
                        })
                        .sum()
                }
                (Value::Array(actual), Value::Array(expected)) => {
                    assert_eq!(actual.len(), expected.len(), "array size at {path}");
                    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                        compare(actual, expected, &format!("{path}[{index}]"));
                    }
                    1
                }
                (Value::Number(actual), Value::Number(expected)) => {
                    assert_eq!(actual.as_f64(), expected.as_f64(), "number at {path}");
                    1
                }
                _ => {
                    assert_eq!(actual, expected, "value at {path}");
                    1
                }
            }
        }
        let factory = GuiState::factory();
        let mut shared = factory.sections.clone();
        let magazine = shared
            .as_object_mut()
            .and_then(|sections| sections.remove("magazine_print_color"));
        assert_eq!(
            magazine,
            Some(json!({"active": false, "strength": 1.0})),
            "magazine print color factory defaults",
        );
        shared.as_object_mut().unwrap().remove("rust");
        // These controls belong to the Rust scanner extension, not the
        // independently generated Python factory-state oracle.
        let scanner = shared["scanner"].as_object_mut().unwrap();
        for key in [
            "scan_output",
            "positive_density_ev",
            "positive_balance_ev",
            "positive_contrast",
            "positive_dmax",
            "black_point",
            "white_point",
            "receiver_profile",
            "positive_interpretation",
            "auto_setup",
            "finish",
            "finish_strength",
            "reference_exposure_ev",
            "reference_balance_ev",
        ] {
            scanner.remove(key);
        }
        assert_eq!(compare(&shared, &expected, "factory"), 186);
        let params = factory.runtime_params().unwrap();
        assert_eq!(params.film_render.grain.rms_granularity, [5.0; 3]);
        assert_eq!(params.io.upscale_factor, 1.0);
    }

    #[test]
    fn saved_zero_rms_and_nondefault_upscale_survive_state_file_roundtrip() {
        let loaded = GuiState::from_value(json!({
            "grain": {"rms_granularity": [0.0, 0.0, 0.0]},
            "input_image": {"upscale_factor": 2.5},
            "rust": {"version": 1, "runtime": {
                "film_render": {"grain": {"rms_granularity": [9.0, 9.0, 9.0]}},
                "io": {"upscale_factor": 4.0}
            }}
        }))
        .unwrap();
        let params = loaded.runtime_params().unwrap();
        assert_eq!(params.film_render.grain.rms_granularity, [0.0; 3]);
        assert_eq!(params.io.upscale_factor, 2.5);
        let saved =
            GuiState::from_runtime(&params, loaded.film(), loaded.paper(), &loaded.sections)
                .unwrap();
        let path = std::env::temp_dir().join(format!(
            "spektrafilm-saved-zero-rms-{}.json",
            std::process::id()
        ));
        saved.save(&path).unwrap();
        let restored = GuiState::load(&path);
        std::fs::remove_file(&path).unwrap();
        let restored = restored.unwrap().runtime_params().unwrap();
        assert_eq!(restored.film_render.grain.rms_granularity, [0.0; 3]);
        assert_eq!(restored.io.upscale_factor, 2.5);
    }

    #[test]
    fn rust_grain_controls_preserve_python_sections_and_state_roundtrip() {
        use spektrafilm_core::presets::{Preset, Provenance, Section};

        fn negative_film_profile() -> spektrafilm_core::profile::Profile {
            // This test exercises preset/state projection, so keep the profile
            // small while satisfying negative-conversion profile validation.
            serde_json::from_str(
                r#"{"metadata":{},"info":{"stock":"test-negative","type":"negative","support":"film","stage":"filming","use":"still"},"data":{"density_curves":[[0.0,0.0,0.0]]}}"#,
            )
            .unwrap()
        }

        let factory = GuiState::factory();
        let baseline = GuiState::from_runtime(
            &factory.runtime_params().unwrap(),
            factory.film(),
            factory.paper(),
            &factory.sections,
        )
        .unwrap();
        let mut source = factory.runtime_params().unwrap();
        source.film_render.grain.particle_area_um2 = 9.0;
        source.film_render.grain.sublayers_active = false;
        source.film_render.grain.monochrome = true;
        let preset = Preset::capture_section(
            "Rust grain controls",
            Section::Grain,
            &source,
            Provenance {
                implementation_version: "test".into(),
                model_version: "test".into(),
                scanner: None,
            },
        )
        .unwrap();
        // Monochrome is derived from the film, so sections preserve the target's value.
        let mut target = factory.runtime_params().unwrap();
        target.film_render.grain.monochrome = true;
        let film = negative_film_profile();
        let applied = preset.apply_section(&target, &film).unwrap();
        let saved =
            GuiState::from_runtime(&applied, factory.film(), factory.paper(), &factory.sections)
                .unwrap();
        let mut before = baseline.sections;
        let mut after = saved.sections.clone();
        before.as_object_mut().unwrap().remove("rust");
        after.as_object_mut().unwrap().remove("rust");
        assert_eq!(
            serde_json::to_vec(&before).unwrap(),
            serde_json::to_vec(&after).unwrap()
        );

        let serialized = serde_json::to_vec(&saved.sections).unwrap();
        let loaded = GuiState::from_value(serde_json::from_slice(&serialized).unwrap()).unwrap();
        for state in [saved, loaded] {
            let restored = state.runtime_params().unwrap();
            assert_eq!(restored.film_render.grain.particle_area_um2, 9.0);
            assert!(!restored.film_render.grain.sublayers_active);
            assert!(restored.film_render.grain.monochrome);
            assert_eq!(
                restored.render_preparation_state(),
                applied.render_preparation_state()
            );
        }
    }

    #[test]
    fn partial_legacy_sections_use_factory_and_flat_gui_only_precedence() {
        let state = GuiState::from_value(json!({
            "input_image":{"apply_cctf_decoding":true,"crop":true,"crop_center":[0.2,0.7]},
            "simulation":{"saving_color_space":"ITU-R BT.2020"},
            "gui_only":{"display":{"gray_18_canvas":false}},
            "display":{"white_padding":0.1}
        }))
        .unwrap();
        let params = state.runtime_params().unwrap();
        assert!(params.io.crop && params.io.input_cctf_decoding);
        assert_eq!(params.io.crop_center, [0.2, 0.7]);
        assert_eq!(state.sections["display"]["gray_18_canvas"], true);
        assert_eq!(state.sections["display"]["white_padding"], 0.1);
        assert_eq!(state.paper(), "kodak_supra_endura");
    }
    #[test]
    fn dedicated_diffusion_and_simulation_panels_win_over_passthrough_groups() {
        let state = GuiState::from_value(json!({
            "camera":{"diffusion_filter":{"strength":1.5}},
            "camera_diffusion":{"strength":0.25},
            "preflashing":{"print_exposure":8.0,"preflash_exposure":0.2},
            "simulation":{"print_exposure":1.75}
        }))
        .unwrap();
        let params = state.runtime_params().unwrap();
        assert_eq!(params.camera.diffusion_filter.strength, 0.25);
        assert_eq!(params.enlarger.print_exposure, 1.75);
        assert_eq!(params.enlarger.preflash_exposure, 0.2);
    }
    #[test]
    fn legacy_export_depth_and_recipe_survive_save_reload() {
        for (depth, format) in [(8, "png"), (16, "tiff"), (32, "tiff")] {
            let state = GuiState::from_value(json!({
                "grain": {"rms_granularity": [0.0, 0.0, 0.0]},
                "input_image": {"upscale_factor": 1.5},
                "simulation": {"film_stock": "kodak_portra_400"},
                "rust": {"version": 1, "save_bit_depth": depth, "viewer": {"zoom": 3.0}}
            }))
            .unwrap();
            assert_eq!(state.sections["rust"]["export_format"], format);
            assert_eq!(state.sections["rust"]["save_bit_depth"], depth);
            assert_eq!(state.sections["rust"]["viewer"]["zoom"], 3.0);
            assert_eq!(state.film(), "kodak_portra_400");
            let params = state.runtime_params().unwrap();
            assert_eq!(params.film_render.grain.rms_granularity, [0.0; 3]);
            assert_eq!(params.io.upscale_factor, 1.5);
            let saved =
                GuiState::from_runtime(&params, state.film(), state.paper(), &state.sections)
                    .unwrap();
            let path = std::env::temp_dir().join(format!(
                "spektrafilm-legacy-export-{}-{depth}.json",
                std::process::id()
            ));
            saved.save(&path).unwrap();
            let restored = GuiState::load(&path);
            std::fs::remove_file(&path).unwrap();
            let restored = restored.unwrap();
            assert_eq!(restored.film(), "kodak_portra_400");
            assert_eq!(restored.sections["rust"]["viewer"]["zoom"], 3.0);
            assert_eq!(restored.sections["rust"]["export_format"], format);
            assert_eq!(restored.sections["rust"]["save_bit_depth"], depth);
            let params = restored.runtime_params().unwrap();
            assert_eq!(params.film_render.grain.rms_granularity, [0.0; 3]);
            assert_eq!(params.io.upscale_factor, 1.5);
        }
    }

    #[test]
    fn export_migration_does_not_repair_explicit_invalid_settings() {
        for (format, depth, error) in [
            (json!("png"), 16, "png exports require 8-bit depth"),
            (json!("jpeg"), 32, "jpeg exports require 8-bit depth"),
            (json!("exr"), 8, "EXR exports require 16- or 32-bit depth"),
            (json!("unknown"), 16, "Unknown export format"),
            (Value::Null, 16, "Export format must be a string"),
            (json!("tiff"), 12, "Saving bit depth must be 8, 16 or 32"),
        ] {
            let result = GuiState::from_value(json!({
                "rust": {"version": 1, "export_format": format, "save_bit_depth": depth}
            }));
            assert_eq!(result.unwrap_err().to_string(), error);
        }
        let result = GuiState::from_value(json!({"rust": {"version": 1, "save_bit_depth": 12}}));
        assert_eq!(
            result.unwrap_err().to_string(),
            "Saving bit depth must be 8, 16 or 32"
        );
    }

    #[test]
    fn fotufilm_effective_controls_roundtrip_only_in_rust_extension() {
        use spektrafilm_core::params::grain::{FotufilmGrainParams, FotufilmMaterial};
        let factory = GuiState::factory();
        let mut params = factory.runtime_params().unwrap();
        params.film_render.grain.engine = spektrafilm_core::params::GrainEngine::Fotufilm;
        params.film_render.grain.fotufilm = FotufilmGrainParams {
            amount: 2.0,
            size: 1.5,
            softness: 2.5,
            color: 0.4,
            layers: [0.5, 1.0, 2.0],
            rms_density: 0.015,
            clump_radius_um: 7.5,
            layer_weights: [0.8, 1.2, 1.5],
            layer_size_ratio: [0.75, 1.0, 1.25],
            radius_span: 8.0,
            sublayer_shares: [0.1, 0.2, 0.3, 0.4],
            coating_density_scale: 1.25,
            fog_density: 0.04,
            material: FotufilmMaterial::DyeNegative,
        };
        let saved = GuiState::from_runtime(
            &params,
            "kodak_gold_200",
            "kodak_supra_endura",
            &factory.sections,
        )
        .unwrap();
        assert!(saved.sections["grain"].get("fotufilm").is_none());
        assert!(saved.sections["grain"].get("engine").is_none());
        assert!(factory.sections["grain"].get("fotufilm").is_none());
        assert_eq!(
            saved.sections["rust"]["runtime"]["film_render"]["grain"]["engine"],
            "fotufilm"
        );
        let encoded = serde_json::to_vec(&saved.sections).unwrap();
        let loaded = GuiState::from_value(serde_json::from_slice(&encoded).unwrap()).unwrap();
        let restored = loaded.runtime_params().unwrap();
        assert_eq!(
            restored.film_render.grain.engine,
            params.film_render.grain.engine
        );
        assert_eq!(
            restored.film_render.grain.fotufilm,
            params.film_render.grain.fotufilm
        );
        let changed_stock = GuiState::from_runtime(
            &restored,
            "kodak_gold_200",
            "kodak_supra_endura",
            &loaded.sections,
        )
        .unwrap();
        assert_eq!(
            changed_stock
                .runtime_params()
                .unwrap()
                .film_render
                .grain
                .fotufilm,
            params.film_render.grain.fotufilm
        );
    }

    #[test]
    fn partial_fotufilm_state_keeps_nested_defaults_and_old_engine_controls() {
        let state = GuiState::from_value(json!({"rust": {"version": 1, "runtime": {
            "film_render": {"grain": {"engine": "fotufilm", "fotufilm": {"amount": 0.0}, "v3_dye_support_um": 4.5}}
        }}})).unwrap();
        let grain = state.runtime_params().unwrap().film_render.grain;
        assert_eq!(grain.fotufilm.amount, 0.0);
        assert_eq!(grain.fotufilm.rms_density, 0.009);
        assert_eq!(grain.fotufilm.sublayer_shares, [0.25; 4]);
        assert_eq!(grain.v3_dye_support_um, 4.5);
    }

    #[test]
    fn runtime_extensions_roundtrip_without_overriding_upstream_selection() {
        let factory = GuiState::factory();
        let mut params = factory.runtime_params().unwrap();
        params.io.output_cctf_encoding = false;
        params.film_render.development_time = Some(9.0);
        params.film_render.grain.engine = spektrafilm_core::params::GrainEngine::Dye;
        params.film_render.grain.v3_dye_support_um = 4.5;
        params.film_render.grain.v3_event_area_um2 = 0.6;
        params.film_render.grain.v2_profile = "8mm500".into();
        params.film_render.grain.v2_amount = Some(0.0);
        let mut extras = factory.sections.clone();
        extras["rust"] = json!({"version":1,"viewer":{"zoom":3.0}});
        let saved =
            GuiState::from_runtime(&params, "kodak_gold_200", "kodak_supra_endura", &extras)
                .unwrap();
        assert_eq!(
            saved.sections["rust"]["runtime"]["film_render"]["grain"]["engine"],
            "dye"
        );
        assert_eq!(
            saved.sections["rust"]["runtime"]["film_render"]["grain"]["v3_dye_support_um"],
            4.5
        );
        let recipe = serde_json::from_slice(&serde_json::to_vec(&saved.sections).unwrap()).unwrap();
        let loaded = GuiState::from_value(recipe).unwrap();
        let restored = loaded.runtime_params().unwrap();
        assert!(!restored.io.output_cctf_encoding);
        assert_eq!(restored.film_render.development_time, Some(9.0));
        assert_eq!(
            restored.film_render.grain.engine,
            spektrafilm_core::params::GrainEngine::Dye
        );
        assert_eq!(restored.film_render.grain.v3_dye_support_um, 4.5);
        assert_eq!(restored.film_render.grain.v3_event_area_um2, 0.6);
        assert_eq!(restored.film_render.grain.v2_profile, "8mm500");
        assert_eq!(restored.film_render.grain.resolved_grain_v2().amount, 0.0);
        assert_eq!(loaded.sections["rust"]["viewer"]["zoom"], 3.0);
        let serialized_saved: Value =
            serde_json::from_slice(&serde_json::to_vec(&saved.sections).unwrap()).unwrap();
        assert_eq!(loaded.sections, serialized_saved);
    }
    #[test]
    fn unknown_grain_engine_fails_state_validation() {
        let error = GuiState::from_value(json!({
            "rust": {"version": 1, "runtime": {
                "film_render": {"grain": {"engine": "unknown"}}
            }}
        }))
        .unwrap_err();
        assert!(format!("{error:#}").contains("unknown variant `unknown`"));
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
            assert!(
                !grain.contains_key(key),
                "legacy field leaked into state: {key}"
            );
        }
        let params = state.runtime_params().unwrap();
        let runtime_grain = serde_json::to_value(params).unwrap()["film_render"]["grain"].clone();
        assert!(runtime_grain.get("particle_scale_layers").is_none());
        assert_eq!(
            runtime_grain["particle_scale_sublayers"],
            json!([2.0, 1.0, 0.5])
        );
    }
    #[test]
    fn grain_v2_canonical_fields_roundtrip_through_state_and_runtime() {
        let factory = GuiState::factory();
        let mut params = factory.runtime_params().unwrap();
        let grain = &mut params.film_render.grain;
        grain.engine = spektrafilm_core::params::GrainEngine::Procedural;
        grain.v2_profile = "custom".into();
        grain.v2_resolution_type = 0;
        grain.v2_timer = 0.375;
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
        assert_eq!(restored.v2_resolution_type, 0);
        assert_eq!(restored.v2_timer, 0.375);
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
        let mut saved =
            GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
                .unwrap()
                .sections;
        saved.as_object_mut().unwrap().remove("rust");
        let loaded = GuiState::from_value(saved.clone()).unwrap();
        let restored = loaded.runtime_params().unwrap();
        assert_eq!(restored.camera.film_format_mm, 70.0);
        assert_eq!(restored.camera.color_filter, "hoya_y2");
        assert_eq!(restored.film_render.grain.micro_sublayers, 3);
        assert_eq!(
            restored.film_render.dir_couplers.langmuir_donor_k_rgb,
            [0.5, 1.5, 2.5]
        );
        assert_eq!(
            restored.film_render.dir_couplers.langmuir_receiver_k_rgb,
            [0.75, 1.75, 2.75]
        );
        assert_eq!(restored.io.input_gamut_compress.hull_detail, 9.0);
        assert_eq!(restored.io.input_gamut_compress.boundary, "locus");
        assert_eq!(restored.film_render.development_time, Some(9.0));
        assert_eq!(restored.print_render.development_time, Some(4.0));
        assert_eq!(restored.film_render.chemistry.gamma_factor_red, 1.25);
        assert_eq!(
            restored.print_render.density_curves_morph.gamma_factor_blue,
            0.75
        );
        assert_eq!(restored.film_render.base.tilt, 0.2);
        assert_eq!(restored.film_render.base.cyan, 1.2);
        assert_eq!(restored.print_render.base.scale, 0.8);
        assert_eq!(restored.film_render.convert.scan_illuminant, "D65");
        assert_eq!(restored.film_render.convert.base_percentile, 97.5);
        assert_eq!(
            restored.film_render.convert.calibration,
            "1.1 0 0  0 1 0  0 0 0.9"
        );
        assert_eq!(
            restored.workflow.route,
            "input > convert-film > scan-minus-base"
        );
        assert_eq!(loaded.sections, saved);
        assert!(saved["simulation"].get("workflow").is_none());
        assert!(saved["simulation"].get("scan_film").is_none());
        assert!(saved["camera"].get("filter_uv").is_none());
        assert!(saved["camera"].get("filter_ir").is_none());
        assert!(saved["special"].get("film_gamma_factor").is_none());
    }
    #[test]
    fn runtime_snapshot_does_not_reown_workflow_route() -> anyhow::Result<()> {
        let factory = GuiState::factory();
        let mut params = factory.runtime_params().unwrap();
        params.workflow.route = "input > film > scan".into();
        let saved =
            GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
                .unwrap();
        assert!(
            saved.sections["rust"]["runtime"]["io"]
                .get("scan_film")
                .is_none()
        );
        assert!(
            saved.sections["rust"]["runtime"]["workflow"]
                .get("route")
                .is_none()
        );
        assert_eq!(saved.sections["simulation"]["route"], "input > film > scan");
        let loaded = GuiState::from_value(saved.sections).unwrap();
        assert_eq!(
            loaded.runtime_params()?.workflow.route,
            "input > film > scan"
        );
        Ok::<(), anyhow::Error>(())
    }
    #[test]
    fn negative_to_positive_convert_route_roundtrip() -> anyhow::Result<()> {
        let factory = GuiState::factory();
        let mut params = factory.runtime_params().unwrap();
        params.workflow.route = "input > convert-film > scan".into();
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.black_point = [0.1, 0.2, 0.3];
        params.scanner.white_point = [0.7, 0.8, 0.9];
        params.scanner.receiver_profile = spektrafilm_core::params::ReceiverProfile::LabScanApprox;
        params.scanner.positive_interpretation =
            spektrafilm_core::params::PositiveInterpretation::Coupled;
        params.scanner.auto_setup = spektrafilm_core::params::ScannerAutoSetup::PerFrame;
        params.scanner.finish = spektrafilm_core::params::ScannerFinish::LabScan;
        params.scanner.finish_strength = 0.0;
        let saved =
            GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)?;
        assert_eq!(
            saved.sections["simulation"]["route"],
            "input > convert-film > scan"
        );
        assert_eq!(
            saved.sections["rust"]["scanner_provenance"]["receiver"]["calibration_status"],
            "approximate_uncalibrated"
        );
        assert_eq!(
            saved.sections["rust"]["scanner_provenance"]["receiver"]["version"],
            "fotufilm_lab_scan_approx_v1"
        );
        assert_eq!(
            saved.sections["rust"]["scanner_provenance"]["finish"],
            "lab_scan"
        );
        assert!(
            saved.sections["rust"]["scanner_provenance"]["finish_version"]
                .as_str()
                .is_some()
        );
        let loaded = GuiState::from_value(saved.sections)?;
        let restored = loaded.runtime_params()?;
        assert_eq!(restored.workflow.route, "input > convert-film > scan");
        assert_eq!(restored.scanner.scan_output, "negative_to_positive");
        assert_eq!(restored.scanner.black_point, [0.1, 0.2, 0.3]);
        assert_eq!(restored.scanner.white_point, [0.7, 0.8, 0.9]);
        assert_eq!(
            restored.scanner.receiver_profile,
            params.scanner.receiver_profile
        );
        assert_eq!(
            restored.scanner.positive_interpretation,
            params.scanner.positive_interpretation
        );
        assert_eq!(restored.scanner.auto_setup, params.scanner.auto_setup);
        assert_eq!(restored.scanner.finish, params.scanner.finish);
        assert_eq!(restored.scanner.finish_strength, 0.0);
        Ok(())
    }
    #[test]
    fn legacy_runtime_extension_scan_output_survives_state_load() {
        // States saved before scanner.scan_output became canonical carry the
        // selection only under the Rust runtime extension.
        let state = GuiState::from_value(json!({
            "simulation": {"route": "input > convert-film > scan"},
            "rust": {"version": 1, "runtime": {"scanner": {
                "scan_output": "negative_to_positive", "receiver_profile": "lab_scan_approx",
                "positive_interpretation": "coupled", "auto_setup": "per_frame",
                "finish": "lab_scan", "finish_strength": 0.0
            }}}
        }))
        .unwrap();
        let params = state.runtime_params().unwrap();
        assert_eq!(params.scanner.scan_output, "negative_to_positive");
        assert_eq!(
            state.sections["scanner"]["scan_output"],
            "negative_to_positive"
        );
        for (key, expected) in [
            ("receiver_profile", json!("lab_scan_approx")),
            ("positive_interpretation", json!("coupled")),
            ("auto_setup", json!("per_frame")),
            ("finish", json!("lab_scan")),
            ("finish_strength", json!(0.0)),
        ] {
            assert_eq!(state.sections["scanner"][key], expected);
            assert_eq!(
                serde_json::to_value(&params.scanner).unwrap()[key],
                expected
            );
        }
        // An explicit canonical selection takes precedence over the extension.
        let mut canonical = state.sections.clone();
        canonical["scanner"]["scan_output"] = json!("direct_scan");
        canonical["scanner"]["positive_interpretation"] = json!("density");
        canonical["scanner"]["auto_setup"] = json!("off");
        canonical["scanner"]["finish"] = json!("neutral");
        let canonical = GuiState::from_value(canonical).unwrap();
        assert_eq!(
            canonical.runtime_params().unwrap().scanner.scan_output,
            "direct_scan"
        );
    }
    #[test]
    fn scanner_defaults_and_invalid_combinations_use_runtime_contract() {
        let state = GuiState::from_value(json!({})).unwrap();
        let runtime_defaults = serde_json::to_value(RuntimeParams::default().scanner).unwrap();
        for key in [
            "receiver_profile",
            "positive_interpretation",
            "auto_setup",
            "finish",
            "finish_strength",
        ] {
            assert_eq!(state.sections["scanner"][key], runtime_defaults[key]);
        }
        for scanner in [
            json!({"receiver_profile": "lab_scan_approx"}),
            json!({"positive_interpretation": "coupled"}),
            json!({"auto_setup": "per_frame"}),
            json!({"finish": "lab_scan"}),
            json!({"finish_strength": -0.01}),
            json!({"finish_strength": 1.01}),
        ] {
            assert!(GuiState::from_value(json!({"scanner": scanner})).is_err());
        }
    }

    #[test]
    fn scanner_provenance_is_rederived_from_builtin_selection() {
        let state = GuiState::from_value(json!({
            "simulation": {"route": "input > film > scan"},
            "scanner": {"receiver_profile": "lab_scan_approx"},
            "rust": {"version": 1, "scanner_provenance": {
                "receiver": {"source": "unavailable device", "calibration_status": "measured"},
                "finish": "unavailable finish"
            }}
        }))
        .unwrap();
        assert_eq!(
            state.sections["rust"]["scanner_provenance"]["receiver"]["calibration_status"],
            "approximate_uncalibrated"
        );
        assert_eq!(
            state.sections["rust"]["scanner_provenance"]["finish"],
            "neutral"
        );
        assert!(
            GuiState::from_value(json!({
                "simulation": {"route": "input > film > scan"},
                "scanner": {"receiver_profile": "unavailable_device"}
            }))
            .is_err()
        );
    }

    #[test]
    fn invalid_scan_points_are_rejected_on_state_load() {
        let factory = GuiState::factory();
        for (black, white) in [
            ([0.5; 3], [0.5; 3]),
            ([0.0, -0.1, 0.0], [1.0; 3]),
            ([0.0; 3], [1.0, 1.1, 1.0]),
        ] {
            let mut sections = factory.sections.clone();
            sections["scanner"]["black_point"] = json!(black);
            sections["scanner"]["white_point"] = json!(white);
            assert!(GuiState::from_value(sections).is_err());
        }
    }

    #[test]
    fn factory_enlarger_neutral_filters_survive_state_conversion() {
        let factory = GuiState::factory();
        let params = factory.runtime_params().unwrap();
        assert_eq!(params.enlarger.m_filter_neutral, 58.8453);
        assert_eq!(params.enlarger.y_filter_neutral, 55.2848);

        let loaded = GuiState::from_value(factory.sections.clone()).unwrap();
        let restored = loaded.runtime_params().unwrap();
        assert_eq!(restored.enlarger.m_filter_neutral, 58.8453);
        assert_eq!(restored.enlarger.y_filter_neutral, 55.2848);
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
        assert_eq!(
            params.print_render.density_curves_morph.gamma_factor_blue,
            0.75
        );
        assert_eq!(params.film_render.base.cyan, 1.2);
        assert_eq!(params.print_render.base.scale, 0.8);
        assert_eq!(params.film_render.convert.base_percentile, 97.5);
        assert_eq!(params.workflow.route, "input > film > scan");
    }

    #[test]
    fn upstream_legacy_scan_flag_is_read_only_and_route_wins() {
        let legacy = GuiState::from_value(json!({"simulation": {"scan_film": true}})).unwrap();
        assert_eq!(
            legacy.runtime_params().unwrap().workflow.route,
            "input > film > scan"
        );
        assert!(legacy.sections["simulation"].get("scan_film").is_none());
        let explicit =
            GuiState::from_value(json!({"simulation": {"scan_film": true, "route": "input"}}))
                .unwrap();
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
        assert_eq!(
            state.sections["simulation"]["saving_color_space"],
            "ITU-R BT.2020"
        );
        assert!(state.sections["simulation"].get("workflow").is_none());
    }

    #[test]
    fn malformed_controls_and_future_extensions_fail_before_application() {
        for value in [
            json!([]),
            json!({"camera":{"auto_exposure":"bad"}}),
            json!({"rust":{"version":9}}),
            json!({"special":{"film_channel_swap":[0,1,3]}}),
            json!({"simulation":{"auto_preview":"false"}}),
        ] {
            assert!(GuiState::from_value(value).is_err());
        }
    }
}
