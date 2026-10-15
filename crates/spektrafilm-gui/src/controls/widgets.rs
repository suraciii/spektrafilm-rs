//! Editors for the pinned experimental GUI sections.
use crate::editor_ranges;
use crate::numeric::{numeric, numeric_field, numeric_field_validated};
use egui::Ui;
use serde_json::{Map, Value};
use spektrafilm_core::params::diffusion::DiffusionFilterParams;

pub(crate) fn editor_range_for(path: &str) -> editor_ranges::EditorRange {
    editor_ranges::editor_range(path).unwrap_or_else(|| panic!("missing editor range for {path}"))
}

pub(crate) fn param_numeric<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    path: &str,
    tooltip: &str,
) -> bool {
    let (min, max, step, decimals) = editor_range_for(path);
    numeric(ui, label, value, min, max, step, decimals, tooltip)
}

pub(crate) fn param_slider<T>(ui: &mut Ui, value: &mut T, path: &str, tooltip: &str) -> bool
where
    T: egui::emath::Numeric,
{
    let (min, max, _step, _decimals) = editor_range_for(path);
    ui.add(egui::Slider::new(
        value,
        T::from_f64(min)..=T::from_f64(max),
    ))
    .on_hover_text(tooltip)
    .changed()
}

/// `param_numeric` whose effective maximum is additionally capped at
/// `max_cap`; the sublayer-share editor keeps the authored total at one.
pub(crate) fn param_numeric_capped<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    path: &str,
    max_cap: f64,
    tooltip: &str,
) -> bool {
    let (min, max, step, decimals) = editor_range_for(path);
    numeric(
        ui,
        label,
        value,
        min,
        max.min(max_cap),
        step,
        decimals,
        tooltip,
    )
}

pub(crate) fn param_tuple<T: egui::emath::Numeric, const N: usize>(
    ui: &mut Ui,
    label: &str,
    values: &mut [T; N],
    path: &str,
    tooltip: &str,
) -> bool {
    let (min, max, step, decimals) = editor_range_for(path);
    tuple(ui, label, values, min, max, step, decimals, tooltip)
}

pub(crate) fn param_numeric_field_validated<T, Id>(
    ui: &mut Ui,
    id_salt: Id,
    value: &mut T,
    path: &str,
    tooltip: &str,
    valid: impl Fn(T) -> bool,
) -> bool
where
    T: egui::emath::Numeric,
    Id: std::hash::Hash,
{
    let (min, max, step, decimals) = editor_range_for(path);
    numeric_field_validated(ui, id_salt, value, min, max, step, decimals, tooltip, valid)
}

/// Per-path table keys for the shared diffusion-filter editor. Camera and
/// enlarger diffusion are separate parameter paths with one range each.
pub(crate) struct DiffusionPaths {
    strength: &'static str,
    spatial_scale: &'static str,
    halo_warmth: &'static str,
    core_intensity: &'static str,
    core_size: &'static str,
    halo_intensity: &'static str,
    halo_size: &'static str,
    bloom_intensity: &'static str,
    bloom_size: &'static str,
}

pub(crate) const CAMERA_DIFFUSION_PATHS: DiffusionPaths = DiffusionPaths {
    strength: "camera.diffusion_filter.strength",
    spatial_scale: "camera.diffusion_filter.spatial_scale",
    halo_warmth: "camera.diffusion_filter.halo_warmth",
    core_intensity: "camera.diffusion_filter.core_intensity",
    core_size: "camera.diffusion_filter.core_size",
    halo_intensity: "camera.diffusion_filter.halo_intensity",
    halo_size: "camera.diffusion_filter.halo_size",
    bloom_intensity: "camera.diffusion_filter.bloom_intensity",
    bloom_size: "camera.diffusion_filter.bloom_size",
};

pub(crate) const ENLARGER_DIFFUSION_PATHS: DiffusionPaths = DiffusionPaths {
    strength: "enlarger.diffusion_filter.strength",
    spatial_scale: "enlarger.diffusion_filter.spatial_scale",
    halo_warmth: "enlarger.diffusion_filter.halo_warmth",
    core_intensity: "enlarger.diffusion_filter.core_intensity",
    core_size: "enlarger.diffusion_filter.core_size",
    halo_intensity: "enlarger.diffusion_filter.halo_intensity",
    halo_size: "enlarger.diffusion_filter.halo_size",
    bloom_intensity: "enlarger.diffusion_filter.bloom_intensity",
    bloom_size: "enlarger.diffusion_filter.bloom_size",
};

pub(crate) fn diffusion(
    ui: &mut Ui,
    value: &mut DiffusionFilterParams,
    paths: &DiffusionPaths,
) -> bool {
    let mut changed = toggle(
        ui,
        "active",
        &mut value.active,
        "Toggle the diffusion filter (Pro-Mist family).",
    );
    changed |= choice_tip(
        ui,
        "filter family",
        &mut value.filter_family,
        &["glimmerglass", "black_pro_mist", "pro_mist", "cinebloom"],
        "PSF family. pro_mist / glimmerglass / cinebloom are transparent (energy-preserving); black_pro_mist absorbs a fraction of the deflected light, lifting shadows by reducing local contrast.",
    );
    changed |= param_numeric(
        ui,
        "strength",
        &mut value.strength,
        paths.strength,
        "Commercial filter stop: 0, 1/8=0.125, 1/4=0.25, 1/2=0.5, 1, 2. Maps internally to the (p_s, p_a) deflected/absorbed photon fractions.",
    );
    changed |= param_numeric(
        ui,
        "spatial scale",
        &mut value.spatial_scale,
        paths.spatial_scale,
        "Multiplier on the image-plane PSF widths (all per-group lambdas). Adjust for image-format / print-size differences.",
    );
    changed |= param_numeric(
        ui,
        "halo warmth",
        &mut value.halo_warmth,
        paths.halo_warmth,
        "Additive offset on the family's halo warmth axis. Positive = warm outer halo / cool inner halo. Energy-preserving per channel. 0 = use family default.",
    );
    changed |= param_numeric(
        ui,
        "core intensity",
        &mut value.core_intensity,
        paths.core_intensity,
        "Advanced. Multiplier on the core weight; the three group weights are renormalized to sum to 1. 1.0 = use family default.",
    );
    changed |= param_numeric(
        ui,
        "core size",
        &mut value.core_size,
        paths.core_size,
        "Advanced. Multiplier on the core lambda. 1.0 = use family default.",
    );
    changed |= param_numeric(
        ui,
        "halo intensity",
        &mut value.halo_intensity,
        paths.halo_intensity,
        "Advanced. Multiplier on the halo weight; the three group weights are renormalized to sum to 1. 1.0 = use family default.",
    );
    changed |= param_numeric(
        ui,
        "halo size",
        &mut value.halo_size,
        paths.halo_size,
        "Advanced. Multiplier on the halo lambda. 1.0 = use family default.",
    );
    changed |= param_numeric(
        ui,
        "bloom intensity",
        &mut value.bloom_intensity,
        paths.bloom_intensity,
        "Advanced. Multiplier on the bloom weight; the three group weights are renormalized to sum to 1. 1.0 = use family default.",
    );
    changed |= param_numeric(
        ui,
        "bloom size",
        &mut value.bloom_size,
        paths.bloom_size,
        "Advanced. Multiplier on the bloom lambda. 1.0 = use family default.",
    );
    changed
}

// UI helpers keep their existing parameter interfaces.
pub(crate) fn optional_numeric(
    ui: &mut Ui,
    label: &str,
    value: &mut Option<f32>,
    inherited: f32,
    path: &str,
    tooltip: &str,
) -> bool {
    let mut displayed = value.unwrap_or(inherited);
    let changed = param_numeric(ui, label, &mut displayed, path, tooltip);
    if changed {
        *value = Some(displayed);
    }
    changed
}

// UI helpers keep their existing parameter interfaces.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tuple<T: egui::emath::Numeric, const N: usize>(
    ui: &mut Ui,
    label: &str,
    values: &mut [T; N],
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
) -> bool {
    ui.push_id(label, |ui| {
        ui.label(label).on_hover_text(tooltip);
        ui.horizontal(|ui| {
            values
                .iter_mut()
                .enumerate()
                .map(|(index, value)| {
                    let changed =
                        numeric_field(ui, (label, index), value, min, max, step, decimals, tooltip);
                    let id = ui.make_persistent_id(("numeric-field", (label, index)));
                    // Build the indexed name only when accessibility is requested, not in
                    // the ordinary per-frame widget path. The visible group stays unchanged.
                    ui.ctx().accesskit_node_builder(id, |node| {
                        node.set_label(format!("{label} {index}"));
                    });
                    changed
                })
                .fold(false, |changed, field_changed| changed | field_changed)
        })
        .inner
    })
    .inner
}

pub(crate) fn fotufilm_sublayer_shares(ui: &mut Ui, shares: &mut [f64; 4]) -> bool {
    ui.label("Sublayer shares").on_hover_text("Edit the first three shares; the fourth is the remaining share so the authored total stays one. All four shares must be positive.");
    let mut changed = false;
    for index in 0..3 {
        let other_sum: f64 = shares[..3]
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, share)| *share)
            .sum();
        let maximum = 1.0 - other_sum - 1e-9;
        let edited = param_numeric_capped(
            ui,
            ["Sublayer 1", "Sublayer 2", "Sublayer 3"][index],
            &mut shares[index],
            "film_render.grain.fotufilm.sublayer_shares",
            maximum,
            "Authored share; transfers weight to or from sublayer 4.",
        );
        if edited {
            shares[3] = 1.0 - shares[..3].iter().sum::<f64>();
            changed = true;
        }
    }
    ui.label(format!("Sublayer 4 (remainder): {:.6}", shares[3]));
    changed
}

pub(crate) fn scan_points(ui: &mut Ui, black: &mut [f64; 3], white: &mut [f64; 3]) -> bool {
    point_tuple(
        ui,
        "film-scan black point",
        black,
        white,
        true,
        "scanner.black_point",
    ) | point_tuple(
        ui,
        "film-scan white point",
        white,
        black,
        false,
        "scanner.white_point",
    )
}

pub(crate) fn point_tuple(
    ui: &mut Ui,
    label: &str,
    values: &mut [f64; 3],
    other: &[f64; 3],
    is_black: bool,
    path: &str,
) -> bool {
    let tooltip = "Red, green, and blue positions in the film scan range. Each black point must be below its white point; values outside the points are not clipped.";
    ui.push_id(label, |ui| {
        ui.label(label).on_hover_text(tooltip);
        ui.horizontal(|ui| {
            values
                .iter_mut()
                .enumerate()
                .map(|(channel, value)| {
                    let changed = param_numeric_field_validated(
                        ui,
                        (label, channel),
                        value,
                        path,
                        tooltip,
                        |next| {
                            if is_black {
                                next < other[channel]
                            } else {
                                next > other[channel]
                            }
                        },
                    );
                    let id = ui.make_persistent_id(("numeric-field", (label, channel)));
                    ui.ctx().accesskit_node_builder(id, |node| {
                        node.set_label(format!("{label} {channel}"));
                    });
                    changed
                })
                .fold(false, |changed, field_changed| changed | field_changed)
        })
        .inner
    })
    .inner
}

pub(crate) fn toggle(ui: &mut Ui, label: &str, value: &mut bool, tooltip: &str) -> bool {
    ui.checkbox(value, label).on_hover_text(tooltip).changed()
}

pub fn choice_tip(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    choices: &[&str],
    tooltip: &str,
) -> bool {
    choice_tip_options(
        ui,
        label,
        value,
        choices.iter().map(|&option| (option, option)),
        tooltip,
    )
}

/// Selector over an iterator of `(value, label)` pairs, so callers can render a
/// table that also carries per-choice descriptions without building a slice.
pub fn choice_tip_entries<'a>(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    choices: impl Iterator<Item = (&'a str, &'a str)> + Clone,
    tooltip: &str,
) -> bool {
    choice_tip_options(ui, label, value, choices, tooltip)
}

pub(crate) fn choice_tip_options<'a>(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    choices: impl Iterator<Item = (&'a str, &'a str)> + Clone,
    tooltip: &str,
) -> bool {
    let selected_label = choices
        .clone()
        .find(|&(option, _)| option == value.as_str())
        .map(|(_, display_label)| display_label)
        .unwrap_or(value.as_str());
    let mut changed = false;
    egui::ComboBox::from_label(label)
        .selected_text(selected_label)
        .show_ui(ui, |ui| {
            for (option, display_label) in choices {
                // Allocate only when a selection changes, not for every option each frame.
                if ui
                    .selectable_label(value.as_str() == option, display_label)
                    .on_hover_text(tooltip)
                    .clicked()
                    && value.as_str() != option
                {
                    *value = option.to_owned();
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    changed
}

// UI helpers keep their existing parameter interfaces.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extra_choice(
    ui: &mut Ui,
    extras: &mut Value,
    section: &str,
    key: &str,
    label: &str,
    default: &str,
    choices: &[&str],
    tooltip: &str,
) -> bool {
    let current = extras[section][key].as_str().unwrap_or(default);
    let mut selected = None;
    egui::ComboBox::from_label(label)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for &option in choices {
                if ui
                    .selectable_label(current == option, option)
                    .on_hover_text(tooltip)
                    .clicked()
                    && current != option
                {
                    selected = Some(option);
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    if let Some(selected) = selected {
        set_extra(extras, section, key, Value::from(selected));
        true
    } else {
        false
    }
}

pub fn extra_bool_tip(
    ui: &mut Ui,
    extras: &mut Value,
    section: &str,
    key: &str,
    label: &str,
    default: bool,
    tooltip: &str,
) -> bool {
    let mut value = extras[section][key].as_bool().unwrap_or(default);
    if toggle(ui, label, &mut value, tooltip) {
        set_extra(extras, section, key, Value::from(value));
        true
    } else {
        false
    }
}

// UI helpers keep their existing parameter interfaces.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extra_number(
    ui: &mut Ui,
    extras: &mut Value,
    section: &str,
    key: &str,
    label: &str,
    default: f64,
    path: &str,
    tooltip: &str,
) -> bool {
    let mut value = extras[section][key].as_f64().unwrap_or(default);
    if param_numeric(ui, label, &mut value, path, tooltip) {
        set_extra(extras, section, key, Value::from(value));
        true
    } else {
        false
    }
}

pub(crate) fn extra_channels(
    ui: &mut Ui,
    extras: &mut Value,
    key: &str,
    label: &str,
    path: &str,
) -> bool {
    let mut values = [0_u32, 1, 2];
    if let Some(saved) = extras["special"][key].as_array() {
        for (value, saved) in values.iter_mut().zip(saved) {
            if let Some(channel) = saved.as_u64().filter(|&channel| channel <= 2) {
                *value = channel as u32;
            }
        }
    }
    if param_tuple(ui, label, &mut values, path, "") {
        set_extra(extras, "special", key, Value::from(values.to_vec()));
        true
    } else {
        false
    }
}

pub(crate) fn set_extra(extras: &mut Value, section: &str, key: &str, value: Value) {
    if !extras.is_object() {
        *extras = Value::Object(Map::new());
    }
    if !extras[section].is_object() {
        extras[section] = Value::Object(Map::new());
    }
    extras[section][key] = value;
}
