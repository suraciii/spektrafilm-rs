//! Per-section reset to the projected factory state.
//!
//! A section reset restores the GUI sections that own the section's runtime
//! parameters to what [`crate::state::GuiState::factory`] projects, and leaves
//! every other section untouched. The comparison that marks a section as
//! modified is an exact-bit comparison of the projected values, so a value
//! typed back to the factory value clears the mark.

use crate::state::GuiState;
use serde_json::Value;
use spektrafilm_core::params::RuntimeParams;

/// The GUI sections a reset restores, and the runtime parameter paths they own.
///
/// The reset is per section, never whole-table: sections such as
/// `simulation.route` and `film_stock` are owned by other surfaces, so a
/// whole-table reset would clobber them.
const SECTION_PARAMETERS: &[(&str, &[&str])] = &[
    ("Film base", &["film_render.base"]),
    ("Print base", &["print_render.base"]),
    ("Grain", &["film_render.grain"]),
    ("Halation", &["film_render.halation"]),
    ("Couplers", &["film_render.dir_couplers"]),
    (
        "Convert",
        &["film_render.convert", "special.film_channel_swap"],
    ),
    (
        "Camera",
        &[
            "camera.exposure_compensation_ev",
            "camera.auto_exposure",
            "camera.auto_exposure_method",
            "camera.color_filter",
            "camera.lens_blur",
            "camera.film_format_mm",
        ],
    ),
    ("Camera diffusion", &["camera.diffusion_filter"]),
    ("Enlarger diffusion", &["enlarger.diffusion_filter"]),
    (
        "Enlarger",
        &[
            "enlarger.illuminant",
            "enlarger.print_exposure",
            "enlarger.print_exposure_mode",
            "enlarger.y_filter_shift",
            "enlarger.m_filter_shift",
            "enlarger.preflash_exposure",
            "enlarger.preflash_y_filter_shift",
            "enlarger.preflash_m_filter_shift",
        ],
    ),
    ("Scanner", &["scanner"]),
    ("Print viewing glare", &["print_render.glare"]),
    ("Input gamut compress", &["io.input_gamut_compress"]),
    ("Output gamut compress", &["io.output_gamut_compress"]),
    ("Magazine finish color", &["magazine_print_color"]),
    (
        "Crop and upscale",
        &[
            "io.crop",
            "io.crop_center",
            "io.crop_size",
            "io.upscale_factor",
        ],
    ),
    (
        "Spectral upsampling",
        &[
            "settings.rgb_to_raw_method",
            "settings.apply_hanatos2025_adaptation_window",
            "settings.apply_hanatos2025_adaptation_surface",
            "settings.spectral_gaussian_blur",
        ],
    ),
];

/// The GUI sections whose runtime parameters a reset restores.
pub fn sections() -> impl Iterator<Item = &'static str> {
    SECTION_PARAMETERS.iter().map(|&(section, _)| section)
}

/// The projected factory state of one section's runtime parameters.
///
/// `None` when the section is not resettable.
fn factory_paths(section: &str) -> Option<&'static [&'static str]> {
    SECTION_PARAMETERS
        .iter()
        .find(|&&(name, _)| name == section)
        .map(|&(_, paths)| paths)
}

/// The projected factory value of a dotted runtime parameter path.
fn factory_value(params: &RuntimeParams, path: &str) -> Option<Value> {
    let mut current = serde_json::to_value(params).ok()?;
    for segment in path.split('.') {
        current = current.get(segment)?.clone();
    }
    Some(current)
}

/// The projected factory state, computed once.
///
/// `GuiState::factory` parses the embedded JSON, so reading it per frame
/// would make every section header pay for it.
static FACTORY: std::sync::LazyLock<RuntimeParams> = std::sync::LazyLock::new(|| {
    GuiState::factory()
        .runtime_params()
        .expect("the pinned factory state projects")
});

/// Whether a section's live parameters differ from the projected factory state.
///
/// Exact-bit comparison, so a value typed back to the factory value clears the
/// mark and a value that differs only in representation does not set it.
pub fn is_modified(section: &str, live: &RuntimeParams) -> bool {
    let Some(paths) = factory_paths(section) else {
        return false;
    };
    let factory = &*FACTORY;
    paths.iter().any(|path| {
        let (Some(live), Some(factory)) = (factory_value(live, path), factory_value(factory, path))
        else {
            return false;
        };
        !same_bits(&live, &factory)
    })
}

/// Restore one section's GUI state to the projected factory state.
///
/// Only the section's own keys are replaced, so neighbouring sections keep
/// their values. Returns whether anything changed.
pub fn reset(state: &mut GuiState, section: &str) -> bool {
    if section_keys(section).is_empty() {
        return false;
    }
    let factory = GuiState::factory();
    let mut changed = false;
    for key in section_keys(section) {
        // The factory sections carry the pinned defaults for every key the
        // GUI owns, so replacing the whole key keeps the section consistent
        // rather than merging field by field.
        let replacement = factory.sections.get(key).cloned();
        if state.sections.get(key) != replacement.as_ref() {
            if let Some(sections) = state.sections.as_object_mut() {
                match replacement {
                    Some(replacement) => {
                        sections.insert(key.to_string(), replacement);
                    }
                    None => {
                        sections.remove(&key.to_string());
                    }
                }
            }
            changed = true;
        }
    }
    changed
}

/// The GUI section keys a reset restores for a named section.
///
/// One GUI section key belongs to exactly one resettable section, so a reset
/// never disturbs a neighbour. `camera` and `camera_diffusion` are separate
/// keys even though both feed `camera.diffusion_filter` at projection time.
fn section_keys(section: &str) -> &'static [&'static str] {
    match section {
        "Film base" => &["film_base"],
        "Print base" => &["print_base"],
        "Grain" => &["grain"],
        "Halation" => &["halation"],
        "Couplers" => &["couplers"],
        "Convert" => &["convert", "special"],
        "Camera" => &["camera"],
        "Camera diffusion" => &["camera_diffusion"],
        "Enlarger diffusion" => &["enlarger_diffusion"],
        "Enlarger" => &["enlarger"],
        "Scanner" => &["scanner"],
        "Print viewing glare" => &["glare"],
        "Input gamut compress" => &["input_gamut_compress"],
        "Output gamut compress" => &["output_gamut_compress"],
        "Magazine finish color" => &["magazine_print_color"],
        "Crop and upscale" => &["input_image"],
        "Spectral upsampling" => &["settings"],
        _ => &[],
    }
}

/// Compare two JSON numbers by their bit pattern.
fn same_bits(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => a.to_bits() == b.to_bits(),
            _ => a == b,
        },
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_restores_a_section_and_leaves_neighbours_untouched() {
        let mut state = GuiState::factory();
        state.sections["film_base"]["scale"] = serde_json::json!(1.5);
        state.sections["print_base"]["scale"] = serde_json::json!(2.0);
        assert!(reset(&mut state, "Film base"));
        assert_eq!(state.sections["film_base"]["scale"], serde_json::json!(1.0));
        assert_eq!(
            state.sections["print_base"]["scale"],
            serde_json::json!(2.0)
        );
    }

    #[test]
    fn reset_of_an_unknown_section_changes_nothing() {
        let mut state = GuiState::factory();
        state.sections["grain"]["active"] = serde_json::json!(false);
        assert!(!reset(&mut state, "No such section"));
        assert_eq!(state.sections["grain"]["active"], serde_json::json!(false));
    }

    #[test]
    fn a_section_is_unmodified_until_a_value_changes_and_clears_when_typed_back() {
        let mut params = GuiState::factory().runtime_params().unwrap();
        assert!(!is_modified("Grain", &params));
        params.film_render.grain.active = !params.film_render.grain.active;
        assert!(is_modified("Grain", &params));
        params.film_render.grain.active = !params.film_render.grain.active;
        assert!(!is_modified("Grain", &params));
    }
}
