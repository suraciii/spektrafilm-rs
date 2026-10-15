//! GUI state parameter validation and section normalization.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use spektrafilm_core::params::RuntimeParams;

use super::overlay_runtime;
pub(super) fn normalize_grain_section(section: &mut Value) {
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
pub(super) fn validate_state_parameters(value: &mut Value, legacy: bool) -> Result<()> {
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
