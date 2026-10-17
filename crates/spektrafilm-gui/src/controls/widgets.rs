//! Editors for the pinned experimental GUI sections.
use super::row_modified;
use super::{ControlFilter, filter_matches};
use crate::editor_ranges;
use crate::numeric::{numeric, numeric_field, numeric_field_validated, numeric_marked};
use crate::theme;
use egui::Ui;
use serde_json::{Map, Value};
use spektrafilm_core::params::diffusion::DiffusionFilterParams;

/// Editor range for `path`, from the single per-path table. A missing entry
/// is a defect, not a silent default to the machine range.
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
    let mark = row_modified::RowMark::resolve(ui.ctx(), Some(path));
    numeric_marked(
        ui,
        label,
        value,
        min,
        max,
        step,
        decimals,
        tooltip,
        crate::numeric::unit_suffix(path),
        false,
        mark,
    )
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
        crate::numeric::unit_suffix(path),
        false,
    )
}

/// Per-component identifiers for a tuple row. Empty means the components are
/// numbered by position, which is what the accessible names already carry.
pub(crate) const POSITION_IDENTIFIERS: &[&str] = &[];
/// Component identifiers for a film-scan point tuple.
pub(crate) const CHANNEL_IDENTIFIERS: &[&str] = &["R", "G", "B"];
/// Component identifiers for a gamut knee: core declares the component
/// domains, and the identifiers name them.
pub(crate) const KNEE_IDENTIFIERS: &[&str] = &["threshold", "limit", "power"];
/// Component identifiers for a crop rectangle: relative coordinates, ordered
/// x then y, as the parameter's own description states.
pub(crate) const XY_IDENTIFIERS: &[&str] = &["x", "y"];
/// Component identifiers for the digital-sharpening pair: sigma then amount,
/// as the parameter's unit states.
pub(crate) const USM_IDENTIFIERS: &[&str] = &["sigma", "amount"];
/// Component identifiers for the micro-structure pair: the clumping-field
/// blur width, then its standard deviation.
pub(crate) const MICRO_STRUCTURE_IDENTIFIERS: &[&str] = &["blur", "clump"];

/// Per-component identifiers for the tuple row at `path`.
///
/// A tuple whose components are colour channels names them `R`/`G`/`B`, and a
/// tuple whose components the parameter itself names keeps those names. A
/// tuple whose components are only ordered positions stays positional:
/// inventing a name the parameter does not carry would mislead, and the
/// identifier becomes part of the row's accessible name.
fn tuple_identifiers(path: &str) -> &'static [&'static str] {
    match path {
        // Colour-channel tuples: one component per emulsion channel.
        "film_render.dir_couplers.gamma_samelayer_rgb"
        | "film_render.dir_couplers.langmuir_donor_k_rgb"
        | "film_render.dir_couplers.langmuir_receiver_k_rgb"
        | "film_render.halation.scatter_core_um"
        | "film_render.halation.scatter_tail_um"
        | "film_render.halation.scatter_tail_weight"
        | "film_render.halation.halation_strength"
        | "film_render.halation.halation_first_sigma_um"
        | "film_render.grain.rms_granularity"
        | "film_render.grain.density_min"
        | "film_render.grain.uniformity"
        | "scanner.reference_balance_ev"
        | "scanner.conversion_balance_ev" => CHANNEL_IDENTIFIERS,
        // Gamut knee: core owns the component names.
        "io.input_gamut_compress.knee" | "io.output_gamut_compress.knee" => KNEE_IDENTIFIERS,
        // Crop rectangle in relative coordinates.
        "io.crop_center" | "io.crop_size" => XY_IDENTIFIERS,
        // Digital sharpening: sigma then amount.
        "scanner.unsharp_mask" => USM_IDENTIFIERS,
        // Micro-structure: scale then clump size.
        "film_render.grain.micro_structure" => MICRO_STRUCTURE_IDENTIFIERS,
        // Interlayer inhibition, ordered as the parameter's own description
        // states: the receiving layer of each pair.
        "film_render.dir_couplers.gamma_interlayer_r_to_gb" => &["G", "B"],
        "film_render.dir_couplers.gamma_interlayer_g_to_rb" => &["R", "B"],
        "film_render.dir_couplers.gamma_interlayer_b_to_rg" => &["R", "G"],
        // Everything else is positional: the channel swaps write to a channel
        // by index, and the emulsion-layer triplets have no documented
        // ordering the identifiers could name.
        _ => POSITION_IDENTIFIERS,
    }
}

pub(crate) fn param_tuple<T: egui::emath::Numeric, const N: usize>(
    ui: &mut Ui,
    label: &str,
    values: &mut [T; N],
    path: &str,
    tooltip: &str,
) -> bool {
    let (min, max, step, decimals) = editor_range_for(path);
    tuple_marked(
        ui,
        label,
        values,
        min,
        max,
        step,
        decimals,
        tooltip,
        tuple_identifiers(path),
        false,
        row_modified::RowMark::resolve(ui.ctx(), Some(path)),
    )
}

/// `param_tuple` on a row marked experimental.
pub(crate) fn param_tuple_experimental<T: egui::emath::Numeric, const N: usize>(
    ui: &mut Ui,
    label: &str,
    values: &mut [T; N],
    path: &str,
    tooltip: &str,
) -> bool {
    let (min, max, step, decimals) = editor_range_for(path);
    tuple_marked(
        ui,
        label,
        values,
        min,
        max,
        step,
        decimals,
        tooltip,
        tuple_identifiers(path),
        true,
        row_modified::RowMark::resolve(ui.ctx(), Some(path)),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn param_numeric_field_validated<T, Id>(
    ui: &mut Ui,
    label: &str,
    id_salt: Id,
    value: &mut T,
    path: &str,
    tooltip: &str,
    unit: Option<&str>,
    labelled_by: Option<egui::Id>,
    valid: impl Fn(T) -> bool,
) -> bool
where
    T: egui::emath::Numeric,
    Id: std::hash::Hash,
{
    let (min, max, step, decimals) = editor_range_for(path);
    numeric_field_validated(
        ui,
        label,
        id_salt,
        value,
        min,
        max,
        step,
        decimals,
        tooltip,
        unit,
        labelled_by,
        valid,
    )
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
    label: &str,
    value: &mut DiffusionFilterParams,
    paths: &DiffusionPaths,
    filter: &mut ControlFilter<'_>,
) -> bool {
    let mut changed = false;
    // The row's label is the section's own name, so a search for that name
    // finds the toggle; `filter_matches` needs a stable key, and the section
    // names are stable.
    let key: &'static str = if label == "Camera diffusion" {
        "Camera diffusion"
    } else {
        "Enlarger diffusion"
    };
    if filter_matches(filter, key) {
        changed |= toggle(
            ui,
            label,
            &mut value.active,
            "Toggle the diffusion filter (Pro-Mist family).",
        );
    }
    if filter_matches(filter, "filter family") {
        changed |= choice_tip(
            ui,
            "filter family",
            &mut value.filter_family,
            &["glimmerglass", "black_pro_mist", "pro_mist", "cinebloom"],
            "PSF family. pro_mist / glimmerglass / cinebloom are transparent (energy-preserving); black_pro_mist absorbs a fraction of the deflected light, lifting shadows by reducing local contrast.",
        );
    }
    if filter_matches(filter, "strength") {
        changed |= param_numeric(
            ui,
            "strength",
            &mut value.strength,
            paths.strength,
            "Commercial filter stop: 0, 1/8=0.125, 1/4=0.25, 1/2=0.5, 1, 2. Maps internally to the (p_s, p_a) deflected/absorbed photon fractions.",
        );
    }
    if filter_matches(filter, "spatial scale") {
        changed |= param_numeric(
            ui,
            "spatial scale",
            &mut value.spatial_scale,
            paths.spatial_scale,
            "Multiplier on the image-plane PSF widths (all per-group lambdas). Adjust for image-format / print-size differences.",
        );
    }
    if filter_matches(filter, "halo warmth") {
        changed |= param_numeric(
            ui,
            "halo warmth",
            &mut value.halo_warmth,
            paths.halo_warmth,
            "Additive offset on the family's halo warmth axis. Positive = warm outer halo / cool inner halo. Energy-preserving per channel. 0 = use family default.",
        );
    }
    if filter_matches(filter, "core intensity") {
        changed |= param_numeric(
            ui,
            "core intensity",
            &mut value.core_intensity,
            paths.core_intensity,
            "Advanced. Multiplier on the core weight; the three group weights are renormalized to sum to 1. 1.0 = use family default.",
        );
    }
    if filter_matches(filter, "core size") {
        changed |= param_numeric(
            ui,
            "core size",
            &mut value.core_size,
            paths.core_size,
            "Advanced. Multiplier on the core lambda. 1.0 = use family default.",
        );
    }
    if filter_matches(filter, "halo intensity") {
        changed |= param_numeric(
            ui,
            "halo intensity",
            &mut value.halo_intensity,
            paths.halo_intensity,
            "Advanced. Multiplier on the halo weight; the three group weights are renormalized to sum to 1. 1.0 = use family default.",
        );
    }
    if filter_matches(filter, "halo size") {
        changed |= param_numeric(
            ui,
            "halo size",
            &mut value.halo_size,
            paths.halo_size,
            "Advanced. Multiplier on the halo lambda. 1.0 = use family default.",
        );
    }
    if filter_matches(filter, "bloom intensity") {
        changed |= param_numeric(
            ui,
            "bloom intensity",
            &mut value.bloom_intensity,
            paths.bloom_intensity,
            "Advanced. Multiplier on the bloom weight; the three group weights are renormalized to sum to 1. 1.0 = use family default.",
        );
    }
    if filter_matches(filter, "bloom size") {
        changed |= param_numeric(
            ui,
            "bloom size",
            &mut value.bloom_size,
            paths.bloom_size,
            "Advanced. Multiplier on the bloom lambda. 1.0 = use family default.",
        );
    }
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

/// Fallback component identifier: the 0-based position, which matches the
/// accessible names the harness already queries.
fn index_identifier(index: usize) -> &'static str {
    ["0", "1", "2", "3", "4", "5"][index]
}

pub(crate) fn fotufilm_sublayer_shares(
    ui: &mut Ui,
    shares: &mut [f64; 4],
    filter: &mut ControlFilter<'_>,
) -> bool {
    if filter_matches(filter, "sublayer shares") {
        ui.label("Sublayer shares").on_hover_text("Edit the first three shares; the fourth is the remaining share so the authored total stays one. All four shares must be positive.");
    }
    let mut changed = false;
    for index in 0..3 {
        if !filter_matches(filter, ["sublayer 1", "sublayer 2", "sublayer 3"][index]) {
            continue;
        }
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
    if filter_matches(filter, "sublayer 4 (remainder)") {
        ui.label(format!("Sublayer 4 (remainder): {:.6}", shares[3]));
    }
    changed
}

pub(crate) fn scan_points(
    ui: &mut Ui,
    black: &mut [f64; 3],
    white: &mut [f64; 3],
    filter: &mut ControlFilter<'_>,
) -> bool {
    let mut changed = false;
    if filter_matches(filter, "film-scan black point") {
        changed |= point_tuple(
            ui,
            "film-scan black point",
            black,
            white,
            true,
            "scanner.black_point",
        );
    }
    if filter_matches(filter, "film-scan white point") {
        changed |= point_tuple(
            ui,
            "film-scan white point",
            white,
            black,
            false,
            "scanner.white_point",
        );
    }
    changed
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
        ui.horizontal_wrapped(|ui| {
            values
                .iter_mut()
                .enumerate()
                .map(|(channel, value)| {
                    let identifier_response = ui
                        .label(
                            egui::RichText::new(CHANNEL_IDENTIFIERS[channel])
                                .color(theme::FG_MUTED)
                                .size(theme::SIZE_HINT),
                        )
                        .on_hover_text(tooltip);
                    param_numeric_field_validated(
                        ui,
                        // The visible identifier and the accessible name agree,
                        // so the component is named the same way for every
                        // reader.
                        &format!("{label} {}", CHANNEL_IDENTIFIERS[channel]),
                        (label, channel),
                        value,
                        path,
                        tooltip,
                        None,
                        Some(identifier_response.id),
                        |next| {
                            if is_black {
                                next < other[channel]
                            } else {
                                next > other[channel]
                            }
                        },
                    )
                })
                .fold(false, |changed, field_changed| changed | field_changed)
        })
        .inner
    })
    .inner
}

/// A boolean parameter row's editor: a switch.
///
/// Built on a focusable button so pointer and keyboard operability come from
/// the toolkit, then named as a switch for assistive technology. The role is
/// set after the widget is added, because egui rewrites the node's role from
/// its own widget info on every frame.
pub(crate) fn toggle(ui: &mut Ui, label: &str, value: &mut bool, tooltip: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let label_response = ui.label(label).on_hover_text(tooltip);
        let mark = row_modified::RowMark::resolve(ui.ctx(), Some(label));
        mark.paint(ui, &label_response);
        mark.gold(ui, |ui| {
            let (fill, knob, text) = if *value {
                (theme::ACCENT, theme::CONTRAST_ON_ACCENT, "on")
            } else {
                (theme::SURFACE3, theme::FG_MUTED, "off")
            };
            let response = ui
                .add(
                    egui::Button::new(egui::RichText::new(text).color(knob).size(theme::SIZE_HINT))
                        .fill(fill)
                        .corner_radius(theme::RADIUS_CARD)
                        .min_size(egui::vec2(44.0, 18.0))
                        .selected(*value),
                )
                .on_hover_text(tooltip);
            if response.clicked() {
                *value = !*value;
                changed = true;
            }
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Checkbox,
                    ui.is_enabled(),
                    *value,
                    label,
                )
            });
            // Name the switch and declare the visible label as its labelling
            // node, so a label query resolves to the switch itself.
            let id = response.id;
            response.labelled_by(label_response.id);
            ui.ctx().accesskit_node_builder(id, |node| {
                node.set_role(egui::accesskit::Role::Switch);
                node.set_toggled(if *value {
                    egui::accesskit::Toggled::True
                } else {
                    egui::accesskit::Toggled::False
                });
                node.set_label(label);
            });
        });
    });
    changed
}

/// Mark a row's label as experimental.
///
/// The marker is a chip beside the label rather than a suffix in the label
/// text, so the row's accessible name and its search match stay the bare
/// parameter name.
pub(crate) fn experimental_badge(ui: &mut Ui) {
    ui.label(
        egui::RichText::new("experimental")
            .color(theme::WARNING)
            .size(theme::SIZE_UNIT),
    );
}

/// A row label with the experimental marker when the parameter is marked.
///
/// The badge sits beside the label, so the row's accessible name and its
/// search match stay the bare parameter name.
pub(crate) fn row_label(ui: &mut Ui, label: &str, experimental: bool) -> egui::Response {
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .color(theme::FG_SECONDARY)
                .size(theme::SIZE_LABEL),
        )
        .sense(egui::Sense::click()),
    );
    if experimental {
        experimental_badge(ui);
    }
    response
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
        false,
    )
}

/// `choice_tip` on a row marked experimental.
pub fn choice_tip_experimental(
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
        true,
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
    choice_tip_options(ui, label, value, choices, tooltip, false)
}

pub(crate) fn choice_tip_options<'a>(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    choices: impl Iterator<Item = (&'a str, &'a str)> + Clone,
    tooltip: &str,
    experimental: bool,
) -> bool {
    let selected_label = choices
        .clone()
        .find(|&(option, _)| option == value.as_str())
        .map(|(_, display_label)| display_label)
        .unwrap_or_else(|| value.as_str())
        .to_owned();
    let prev = value.clone();
    // Label first, selector second, so a choice row reads like every other
    // parameter row. The selector paints in its own horizontal row inside a
    // vertical section body, so both share one explicit row here. `from_id_salt`
    // paints the box where it is placed instead of appending its own label
    // after it. The width is capped so a selector sharing a row with another
    // control leaves room for it.
    ui.horizontal(|ui| {
        row_label(ui, label, experimental).on_hover_text(tooltip);
        // The selector takes the width still unclaimed in its row, capped, so a
        // selector sharing a row with another control leaves room for it and
        // never pushes the row past its container.
        let width = ui
            .available_width()
            .clamp(theme::WIDTH_COMBO_MIN, theme::WIDTH_COMBO_MAX);
        egui::ComboBox::from_id_salt(label)
            .selected_text(&selected_label)
            .width(width)
            // Truncate rather than extend: an extending selector takes its
            // text's full width and pushes whatever shares the row past the
            // container.
            .wrap_mode(egui::TextWrapMode::Truncate)
            .show_ui(ui, |ui| {
                for (option, display_label) in choices {
                    // Allocate only when a selection changes, not for every
                    // option each frame.
                    if ui
                        .selectable_label(value.as_str() == option, display_label)
                        .on_hover_text(tooltip)
                        .clicked()
                        && value.as_str() != option
                    {
                        *value = option.to_owned();
                    }
                }
            })
            .response
            .on_hover_text(tooltip);
        name_choice(ui, label);
    });
    prev != *value
}

/// Name a choice row's selector with the parameter's own accessible name.
///
/// The selector's own widget info carries an empty label, and a `labelled_by`
/// link to the visible label would make that label node the one a label query
/// returns, so the selector is named directly instead.
pub(crate) fn name_choice(ui: &Ui, label: &str) {
    let id = ui.make_persistent_id(egui::Id::new(label));
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_label(label);
    });
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
    // Label first, selector second, so a choice row reads like every other
    // parameter row. The selector paints in its own horizontal row inside a
    // vertical section body, so both share one explicit row here.
    ui.horizontal(|ui| {
        row_label(ui, label, false).on_hover_text(tooltip);
        egui::ComboBox::from_id_salt(label)
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
        name_choice(ui, label);
    });
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
    let mark = row_modified::RowMark::resolve_extras(ui.ctx(), section, key);
    if mark.gold(ui, |ui| toggle(ui, label, &mut value, tooltip)) {
        set_extra(extras, section, key, Value::from(value));
        true
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
fn tuple_marked<T: egui::emath::Numeric, const N: usize>(
    ui: &mut Ui,
    label: &str,
    values: &mut [T; N],
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
    identifiers: &[&str],
    experimental: bool,
    mark: row_modified::RowMark,
) -> bool {
    ui.push_id(label, |ui| {
        let label_response = row_label(ui, label, experimental).on_hover_text(tooltip);
        mark.paint(ui, &label_response);
        mark.gold(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                values
                    .iter_mut()
                    .enumerate()
                    .map(|(index, value)| {
                        let identifier = identifiers
                            .get(index)
                            .copied()
                            .unwrap_or_else(|| index_identifier(index));
                        let identifier_response = ui.label(
                            egui::RichText::new(identifier)
                                .color(theme::FG_MUTED)
                                .size(theme::SIZE_HINT),
                        );
                        numeric_field(
                            ui,
                            &format!("{label} {identifier}"),
                            (label, index),
                            value,
                            min,
                            max,
                            step,
                            decimals,
                            tooltip,
                            None,
                            Some(identifier_response.id),
                        )
                    })
                    .fold(false, |changed, field_changed| changed | field_changed)
            })
            .inner
        })
    })
    .inner
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
    if param_tuple_experimental(ui, label, &mut values, path, "") {
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
