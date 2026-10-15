<<<<<<< HEAD
//! State ⇄ runtime projection. Both directions consume the shared group
//! tables below so an alias cannot exist in only one of them.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use spektrafilm_core::params::RuntimeParams;

use super::validation::validate_state_parameters;
use super::{
    ENLARGER_SIMULATION_ALIASES, GuiState, canonicalize_grain_state, copy_fields, merge,
    overlay_runtime,
};
=======
//! State ⇄ runtime projection.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use spektrafilm_core::params::{PrintExposureEvTracking, PrintExposureMode, RuntimeParams};

use super::{
    ENLARGER_SIMULATION_ALIASES, GuiState, canonicalize_grain_state, copy_fields, merge,
    overlay_runtime, translate_legacy_print_exposure,
};

>>>>>>> origin/main-0.3.4
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

impl GuiState {
<<<<<<< HEAD
    pub fn runtime_params(&self) -> Result<RuntimeParams> {
        validate_state_parameters(&mut self.sections.clone(), false)?;
=======
    /// Project the sections into runtime parameters. Validation is split from
    /// projection: the load, import and save paths each validate once, so this
    /// is a pure read that never clones or re-checks the carriers.
    pub fn runtime_params(&self) -> Result<RuntimeParams> {
>>>>>>> origin/main-0.3.4
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
<<<<<<< HEAD
        merge(&mut runtime["enlarger"], &s["preflashing"]);
        for &(simulation_key, enlarger_key) in ENLARGER_SIMULATION_ALIASES {
=======
        let (preflashing, tracking) = translate_legacy_print_exposure(&s["preflashing"]);
        merge(&mut runtime["enlarger"], &preflashing);
        if let Some(tracking) = tracking {
            overlay_runtime(&mut runtime["settings"], &tracking);
        }
        for &(simulation_key, enlarger_key) in ENLARGER_SIMULATION_ALIASES {
            if enlarger_key == "print_exposure_compensation" {
                // The upstream boolean is a derived mirror: it may only
                // refine the default (auto without tracking), never
                // overwrite the mapping an authoritative carrier
                // (`preflashing`, `rust.runtime`) already produced.
                if let Some(comp) = s["simulation"][simulation_key].as_bool() {
                    let tracking_default = runtime["settings"]["print_exposure_ev_tracking"]
                        .as_str()
                        .is_none_or(|v| v == "follow_mode");
                    let auto = runtime["enlarger"]["print_exposure_mode"]
                        .as_str()
                        .is_none_or(|v| v == "auto");
                    if tracking_default && auto && !comp {
                        runtime["settings"]["print_exposure_ev_tracking"] = json!("never");
                    }
                }
                continue;
            }
>>>>>>> origin/main-0.3.4
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
<<<<<<< HEAD
            if order.len() != 3 || order.iter().any(|v| v.as_u64().is_none_or(|x| x > 2)) {
                bail!("{name} must contain three channel indices in 0..2");
=======
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
>>>>>>> origin/main-0.3.4
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
<<<<<<< HEAD
=======
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
>>>>>>> origin/main-0.3.4
            if let Some(space) = rust.get("export_saving_color_space") {
                let space = space
                    .as_str()
                    .context("Export saving color space must be a string")?;
                spektrafilm_math::colorspace::resolve(space).map_err(anyhow::Error::msg)?;
<<<<<<< HEAD
                if rust.get("export_format").and_then(Value::as_str) == Some("exr")
                    && !matches!(space, "sRGB" | "ACES2065-1")
=======
                if format == Some(ImageFormat::Exr)
                    && save_option_violations(
                        ImageFormat::Exr,
                        &options(BitDepth::Sixteen, space, true),
                        false,
                    )
                    .contains(&V::ExrColorSpace)
>>>>>>> origin/main-0.3.4
                {
                    bail!("EXR saving color space must be sRGB or ACES2065-1");
                }
            }
            if let Some(encoded) = rust.get("export_saving_cctf_encoding") {
                let encoded = encoded
                    .as_bool()
                    .context("Export saving CCTF encoding must be a boolean")?;
<<<<<<< HEAD
                if rust.get("export_format").and_then(Value::as_str) == Some("exr") && encoded {
                    bail!("EXR exports must use linear color");
                }
                if matches!(
                    rust.get("export_format").and_then(Value::as_str),
                    Some("jpeg" | "png")
                ) && !encoded
                {
                    bail!("JPEG and PNG exports require encoded color");
=======
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
>>>>>>> origin/main-0.3.4
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
<<<<<<< HEAD
                let depth = rust
                    .get("save_bit_depth")
                    .and_then(Value::as_u64)
                    .unwrap_or(16);
                if matches!(export_format, "jpeg" | "png") && depth != 8 {
                    bail!("{export_format} exports require 8-bit depth");
                }
                if export_format == "exr" && !matches!(depth, 16 | 32) {
                    bail!("EXR exports require 16- or 32-bit depth");
                }
=======
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
>>>>>>> origin/main-0.3.4
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
<<<<<<< HEAD
=======
        // Keep the upstream boolean mirror in step with the mode and
        // tracking settings; the import mapping inverts this exactly, and
        // the degenerate tracking choices fold onto the mode default.
        let (normalize, compensation) = match (
            params.enlarger.print_exposure_mode,
            params.settings.print_exposure_ev_tracking,
        ) {
            (PrintExposureMode::Auto, PrintExposureEvTracking::Never) => (true, false),
            (PrintExposureMode::Auto, _) => (true, true),
            (PrintExposureMode::Manual, PrintExposureEvTracking::Always) => (false, true),
            (PrintExposureMode::Manual, _) => (false, false),
        };
        s["preflashing"]["normalize_print_exposure"] = json!(normalize);
        s["preflashing"]["print_exposure_compensation"] = json!(compensation);
        s["simulation"]["print_exposure_compensation"] = json!(compensation);
>>>>>>> origin/main-0.3.4
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
<<<<<<< HEAD
=======
            if enlarger_key == "print_exposure_compensation" {
                // The upstream mirror stays boolean and is derived from the
                // mode and tracking settings above, like the import mapping.
                continue;
            }
>>>>>>> origin/main-0.3.4
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
<<<<<<< HEAD
        s["rust"]["version"] = json!(2);
=======
        s["rust"]["version"] = json!(3);
>>>>>>> origin/main-0.3.4
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
