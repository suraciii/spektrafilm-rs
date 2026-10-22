//! Per-section reset to the projected factory state.
//!
//! A section reset restores every parameter the section presents to what
//! [`crate::state::GuiState::factory`] projects, and changes nothing outside
//! that section. The comparison that marks a section as modified is an
//! exact-bit comparison of the projected values, so a value typed back to the
//! factory value clears the mark.
//!
//! The reset acts on the live model the rows edit — the runtime parameters
//! and the extras document — rather than on a projection of a captured
//! document. Replacing the whole document would discard live edits made since
//! the last load, in sections the reset does not own.

use crate::state::GuiState;
use serde_json::{Map, Value};
use spektrafilm_core::params::RuntimeParams;

/// One section's reset scope: the runtime parameter paths it presents, and the
/// extras paths it presents.
///
/// This table is the only source of a section's reset scope, and it is
/// deliberately not derived from the numeric range table, which holds numeric
/// parameters only. A runtime path and an extras path are separate carriers:
/// the grain engine and its engine-specific groups live only in
/// `rust.runtime`, while the Convert channel swaps live only in `extras`.
///
/// One section owns each carrier key, so a reset never disturbs a neighbour.
/// `Camera` and `Camera diffusion` are separate entries even though both feed
/// `camera.diffusion_filter` at projection time; the same holds for the two
/// Enlarger entries and `enlarger.diffusion_filter`.
const SECTION_PARAMETERS: &[(&str, &[&str], &[&str])] = &[
    ("Film base", &["film_render.base"], &[]),
    (
        "Film chemistry",
        &["film_render.chemistry", "film_render.development_time"],
        &[],
    ),
    ("Print base", &["print_render.base"], &[]),
    (
        "Print chemistry",
        &[
            "print_render.density_curves_morph",
            "print_render.development_time",
        ],
        &[],
    ),
    // The grain engine and its engine-specific groups are runtime-model
    // fields, so this one path carries them all.
    ("Grain", &["film_render.grain"], &[]),
    ("Halation", &["film_render.halation"], &[]),
    ("Couplers", &["film_render.dir_couplers"], &[]),
    (
        "Convert",
        &["film_render.convert"],
        &["special.film_channel_swap", "special.print_channel_swap"],
    ),
    // The Camera diffusion section owns `camera.diffusion_filter`, and the
    // camera group also carries calibration outputs (`lens_blur_um`, the UV
    // and IR taking filters) that no row presents, so the Camera section's own
    // rows are named individually.
    (
        "Camera",
        &[
            "camera.exposure_compensation_ev",
            "camera.auto_exposure",
            "camera.auto_exposure_method",
            "camera.color_filter",
            "camera.film_format_mm",
        ],
        &[],
    ),
    ("Camera diffusion", &["camera.diffusion_filter"], &[]),
    // The Enlarger diffusion section owns `enlarger.diffusion_filter`, and the
    // enlarger group also carries the calibrated neutral filter pack that no
    // row presents, so the Enlarger section's own rows are named individually.
    // `print_exposure_ev_tracking` rides on `settings` rather than the group,
    // but the Enlarger section owns it.
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
            "settings.print_exposure_ev_tracking",
        ],
        &[],
    ),
    ("Enlarger diffusion", &["enlarger.diffusion_filter"], &[]),
    ("Scanner", &["scanner"], &[]),
    ("Glare", &["print_render.glare"], &[]),
    (
        "Input gamut compress",
        &[
            "io.input_gamut_compress.active",
            "io.input_gamut_compress.algorithm",
            "io.input_gamut_compress.knee",
            "io.input_gamut_compress.hull_detail",
        ],
        &[],
    ),
    (
        "Output gamut compress",
        &[
            "io.output_gamut_compress.algorithm",
            "io.output_gamut_compress.knee",
        ],
        &[],
    ),
    ("CMYK ink style", &["cmyk"], &[]),
    (
        "Crop and upscale",
        &[
            "io.crop",
            "io.crop_center",
            "io.crop_size",
            "io.upscale_factor",
        ],
        &[],
    ),
    (
        "Spectral upsampling",
        &[
            "settings.rgb_to_raw_method",
            "settings.apply_hanatos2025_adaptation_window",
            "settings.apply_hanatos2025_adaptation_surface",
            "settings.spectral_gaussian_blur",
        ],
        &[],
    ),
    (
        "Input",
        &["io.input_color_space", "io.input_cctf_decoding"],
        &[],
    ),
    (
        "Output",
        &["output_mix.amount", "io.output_color_space"],
        &[
            "simulation.saving_color_space",
            "simulation.saving_cctf_encoding",
        ],
    ),
];

/// The GUI sections whose parameters a reset restores.
pub fn sections() -> impl Iterator<Item = &'static str> {
    SECTION_PARAMETERS.iter().map(|&(section, _, _)| section)
}

/// One section's runtime paths and extras paths, or `None` when the section is
/// not resettable.
fn section_paths(section: &str) -> Option<&'static [&'static str]> {
    SECTION_PARAMETERS
        .iter()
        .find(|&&(name, _, _)| name == section)
        .map(|&(_, paths, _)| paths)
}

fn section_extras(section: &str) -> Option<&'static [&'static str]> {
    SECTION_PARAMETERS
        .iter()
        .find(|&&(name, _, _)| name == section)
        .map(|&(_, _, extras)| extras)
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

/// The serialized factory runtime tree, computed once.
///
/// A section header compares its own paths against this tree every frame, so
/// the serialization is hoisted out of the comparison rather than repeated
/// per path.
static FACTORY_ROOT: std::sync::LazyLock<Value> =
    std::sync::LazyLock::new(|| serde_json::to_value(&*FACTORY).expect("the factory serializes"));

/// The factory extras document, computed once. The channel swaps and other
/// extras-only values never reach the runtime model, so their factory values
/// are read from the document the factory projects.
static FACTORY_EXTRAS: std::sync::LazyLock<Value> =
    std::sync::LazyLock::new(|| GuiState::factory().sections.clone());

/// The value a dotted path resolves to inside one document, or `None` when the
/// document does not carry the path.
fn document_value(document: &Value, path: &str) -> Option<Value> {
    let mut current = document;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current.clone())
}

/// The value a runtime path resolves to inside one runtime tree.
///
/// The tree is the serialized runtime model, so a path's first segment is a
/// top-level field of `RuntimeParams`.
fn value_at(root: &Value, path: &str) -> Option<Value> {
    document_value(root, path)
}

/// Whether a section's live parameters differ from the projected factory state.
///
/// Exact-bit comparison, so a value typed back to the factory value clears the
/// mark and a value that differs only in representation does not set it.
pub fn is_modified(section: &str, live: &RuntimeParams, extras: &Value) -> bool {
    let Some(paths) = section_paths(section) else {
        return false;
    };
    let Some(live_root) = serde_json::to_value(live).ok() else {
        return false;
    };
    let factory_root = &*FACTORY_ROOT;
    for path in paths {
        if let (Some(live), Some(factory)) =
            (value_at(&live_root, path), value_at(factory_root, path))
            && !same_bits(&live, &factory)
        {
            return true;
        }
    }
    if let Some(extras_paths) = section_extras(section) {
        for path in extras_paths {
            if let (Some(live), Some(factory)) = (
                document_value(extras, path),
                document_value(&FACTORY_EXTRAS, path),
            ) && !same_bits(&live, &factory)
            {
                return true;
            }
        }
    }
    false
}

/// Whether one rendered runtime or extras leaf differs from the value its section reset restores.
///
/// The row path must be the leaf exposed by the control. A section scope may
/// name an ancestor object, while Camera and Enlarger deliberately name their
/// individual leaves; both forms use the same exact-bit comparison as the
/// section header and reset operation.
pub(crate) fn leaf_modified(
    section: &str,
    path: &str,
    live_root: &Value,
    live_extras: &Value,
) -> bool {
    if let Some(paths) = section_paths(section) {
        for scope in paths {
            let is_child = path == *scope
                || (path.starts_with(scope) && path.as_bytes().get(scope.len()) == Some(&b'.'));
            if is_child {
                let Some(live) = value_at(live_root, path) else {
                    continue;
                };
                let Some(factory) = value_at(&FACTORY_ROOT, path) else {
                    continue;
                };
                return !same_bits(&live, &factory);
            }
        }
    }
    if let Some(paths) = section_extras(section) {
        for scope in paths {
            let is_child = path == *scope
                || (path.starts_with(scope) && path.as_bytes().get(scope.len()) == Some(&b'.'));
            if is_child {
                let Some(live) = document_value(live_extras, path) else {
                    continue;
                };
                let Some(factory) = document_value(&FACTORY_EXTRAS, path) else {
                    continue;
                };
                return !same_bits(&live, &factory);
            }
        }
    }
    false
}

/// Restore one section's parameters to the projected factory state.
///
/// Only the section's own paths are restored, so neighbouring sections keep
/// their values. Returns whether anything changed.
pub fn reset(params: &mut RuntimeParams, extras: &mut Value, section: &str) -> bool {
    let Some(paths) = section_paths(section) else {
        return false;
    };
    let mut changed = false;
    if let (Some(mut live_root), Some(factory_root)) = (
        serde_json::to_value(&*params).ok(),
        serde_json::to_value(&*FACTORY).ok(),
    ) {
        for path in paths {
            let Some(factory) = value_at(&factory_root, path) else {
                continue;
            };
            // The mark and the reset must agree on what differs, so both use
            // the exact-bit comparison: a `-0.0` leaf differs from the
            // factory's `0.0` by its bits while comparing equal as numbers.
            if value_at(&live_root, path)
                .as_ref()
                .is_some_and(|live| same_bits(live, &factory))
            {
                continue;
            }
            if !set_value(&mut live_root, path, factory) {
                continue;
            }
            changed = true;
        }
        if changed {
            // The round trip through the runtime model keeps a reset from
            // writing a value the model cannot hold. A restore the model
            // rejects leaves the section unrestored, so the caller is told
            // nothing changed rather than marking a reset that did not happen.
            match serde_json::from_value::<RuntimeParams>(live_root) {
                Ok(restored) if restored.validate().is_ok() => *params = restored,
                _ => return false,
            }
        }
    }
    if let Some(extras_paths) = section_extras(section) {
        for path in extras_paths {
            let Some(factory) = document_value(&FACTORY_EXTRAS, path) else {
                continue;
            };
            if document_value(extras, path)
                .as_ref()
                .is_some_and(|live| same_bits(live, &factory))
            {
                continue;
            }
            if set_value(extras, path, factory) {
                changed = true;
            }
        }
    }
    changed
}

/// Write `value` at `path` inside `document`, creating the intermediate
/// objects the path names. Returns whether the write happened.
fn set_value(document: &mut Value, path: &str, value: Value) -> bool {
    let mut segments = path.split('.').peekable();
    let mut current = document;
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            let Some(map) = current.as_object_mut() else {
                return false;
            };
            map.insert(segment.to_string(), value);
            return true;
        }
        let Some(map) = current.as_object_mut() else {
            return false;
        };
        current = map
            .entry(segment.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    false
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

    fn factory() -> RuntimeParams {
        GuiState::factory().runtime_params().unwrap()
    }

    #[test]
    fn every_section_path_resolves_against_the_runtime_model() {
        let params = factory();
        let root = serde_json::to_value(&params).unwrap();
        let extras = GuiState::factory().sections.clone();
        for (section, paths, extras_paths) in SECTION_PARAMETERS {
            for path in *paths {
                assert!(
                    value_at(&root, path).is_some(),
                    "{section}: runtime path {path} does not resolve"
                );
            }
            for path in *extras_paths {
                assert!(
                    document_value(&extras, path).is_some(),
                    "{section}: extras path {path} does not resolve"
                );
            }
        }
    }

    #[test]
    fn a_section_is_unmodified_until_a_value_changes_and_clears_when_typed_back() {
        let mut params = factory();
        let extras = GuiState::factory().sections.clone();
        assert!(!is_modified("Grain", &params, &extras));
        params.film_render.grain.active = !params.film_render.grain.active;
        assert!(is_modified("Grain", &params, &extras));
        params.film_render.grain.active = !params.film_render.grain.active;
        assert!(!is_modified("Grain", &params, &extras));
    }

    #[test]
    fn a_reset_restores_the_section_and_leaves_a_neighbour_untouched() {
        let mut params = factory();
        let mut extras = GuiState::factory().sections.clone();
        let neighbour = params.film_render.base.scale;
        params.film_render.base.scale = neighbour + 0.5;
        params.film_render.grain.blur = 0.25;
        assert!(reset(&mut params, &mut extras, "Film base"));
        assert_eq!(params.film_render.base.scale, neighbour);
        // The neighbour's edit survives the reset.
        assert_eq!(params.film_render.grain.blur, 0.25);
    }

    #[test]
    fn a_reset_of_an_unknown_section_changes_nothing() {
        let mut params = factory();
        let mut extras = GuiState::factory().sections.clone();
        params.film_render.grain.active = !params.film_render.grain.active;
        assert!(!reset(&mut params, &mut extras, "No such section"));
        assert!(is_modified("Grain", &params, &extras));
    }

    #[test]
    fn the_channel_swap_edit_marks_the_convert_section_modified() {
        let mut params = factory();
        let mut extras = GuiState::factory().sections.clone();
        assert!(!is_modified("Convert", &params, &extras));
        extras["special"]["film_channel_swap"] = serde_json::json!([1, 0, 2]);
        assert!(is_modified("Convert", &params, &extras));
        assert!(reset(&mut params, &mut extras, "Convert"));
        assert!(!is_modified("Convert", &params, &extras));
    }

    #[test]
    fn a_reset_of_the_crop_section_leaves_the_input_section_untouched() {
        let mut params = factory();
        let mut extras = GuiState::factory().sections.clone();
        params.io.input_color_space = "ProPhoto RGB".to_string();
        params.io.upscale_factor = 3.0;
        assert!(reset(&mut params, &mut extras, "Crop and upscale"));
        // The Input section owns the color space; the crop reset must not
        // restore it.
        assert_eq!(params.io.input_color_space, "ProPhoto RGB");
        assert_eq!(params.io.upscale_factor, 1.0);
    }

    #[test]
    fn a_camera_reset_leaves_the_camera_diffusion_section_untouched() {
        let mut params = factory();
        let mut extras = GuiState::factory().sections.clone();
        // The Camera diffusion section owns the diffusion filter, so a Camera
        // reset must not reach it even though the camera group carries it.
        let diffusion = params.camera.diffusion_filter.strength;
        params.camera.diffusion_filter.strength = diffusion + 1.0;
        assert!(is_modified("Camera diffusion", &params, &extras));
        assert!(
            !is_modified("Camera", &params, &extras),
            "a diffusion edit marks its own section, not the Camera header"
        );
        params.camera.exposure_compensation_ev = 0.5;
        assert!(is_modified("Camera", &params, &extras));
        assert!(reset(&mut params, &mut extras, "Camera"));
        assert_eq!(params.camera.exposure_compensation_ev, 0.0);
        assert_eq!(
            params.camera.diffusion_filter.strength,
            diffusion + 1.0,
            "the Camera reset leaves the Camera diffusion section alone"
        );
        assert!(is_modified("Camera diffusion", &params, &extras));
    }

    #[test]
    fn an_enlarger_reset_leaves_the_enlarger_diffusion_section_untouched() {
        let mut params = factory();
        let mut extras = GuiState::factory().sections.clone();
        let diffusion = params.enlarger.diffusion_filter.strength;
        params.enlarger.diffusion_filter.strength = diffusion + 1.0;
        assert!(is_modified("Enlarger diffusion", &params, &extras));
        assert!(
            !is_modified("Enlarger", &params, &extras),
            "a diffusion edit marks its own section, not the Enlarger header"
        );
        params.enlarger.print_exposure = 2.0;
        assert!(is_modified("Enlarger", &params, &extras));
        assert!(reset(&mut params, &mut extras, "Enlarger"));
        assert_eq!(params.enlarger.print_exposure, 1.0);
        assert_eq!(
            params.enlarger.diffusion_filter.strength,
            diffusion + 1.0,
            "the Enlarger reset leaves the Enlarger diffusion section alone"
        );
    }

    #[test]
    fn a_reset_keeps_the_calibrated_neutral_filter_pack() {
        let mut params = factory();
        let mut extras = GuiState::factory().sections.clone();
        // The neutral filter pack is a calibration output that no Enlarger row
        // presents, so a reset must not discard it.
        let neutral = params.enlarger.y_filter_neutral;
        params.enlarger.y_filter_neutral = neutral + 10.0;
        params.enlarger.print_exposure = 2.0;
        assert!(reset(&mut params, &mut extras, "Enlarger"));
        assert_eq!(
            params.enlarger.y_filter_neutral,
            neutral + 10.0,
            "a reset keeps a calibrated value no row presents"
        );
    }

    #[test]
    fn the_output_reset_restores_the_simulation_blend() {
        let mut params = factory();
        let mut extras = GuiState::factory().sections.clone();
        params.output_mix.amount = 0.25;
        assert!(is_modified("Output", &params, &extras));
        assert!(reset(&mut params, &mut extras, "Output"));
        assert_eq!(params.output_mix.amount, 1.0);
        assert!(!is_modified("Output", &params, &extras));
    }
}
