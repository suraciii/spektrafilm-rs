//! Editors for the pinned experimental GUI sections.
use super::row_modified;
use super::{ControlFilter, filter_matches};
use crate::editor_ranges;
use crate::numeric::{numeric, numeric_field, numeric_marked};
use crate::theme;
use egui::Ui;
use serde_json::{Map, Value};
use spektrafilm_core::params::diffusion::DiffusionFilterParams;

/// Painter-backed symbols shared by actions and non-interactive marks.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Icon {
    Open,
    Undo,
    Redo,
    Settings,
    Export,
    Reset,
    Close,
    Add,
    Image,
    Help,
    Aperture,
    ChevronLeft,
    Scan,
    Save,
    Layers,
    Transitions,
}

/// Paint a symbol in the central 17-point square without relying on font glyphs.
pub(crate) fn paint_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: Icon,
    color: egui::Color32,
) {
    let center = rect.center();
    let stroke = egui::Stroke::new(1.7_f32, color);
    let point = |x, y| center + egui::vec2(x, y);
    let line = |a: [f32; 2], b: [f32; 2]| {
        painter.line_segment([point(a[0], a[1]), point(b[0], b[1])], stroke);
    };
    let path = |points: &[[f32; 2]]| {
        for pair in points.windows(2) {
            line(pair[0], pair[1]);
        }
    };
    match icon {
        Icon::Open => {
            path(&[
                [-7.0, 5.5],
                [-7.0, -5.0],
                [-2.5, -5.0],
                [0.0, -2.5],
                [6.0, -2.5],
            ]);
            path(&[
                [-7.0, 5.5],
                [-4.5, -0.5],
                [7.5, -0.5],
                [5.0, 5.5],
                [-7.0, 5.5],
            ]);
        }
        Icon::Undo | Icon::Redo => {
            let direction = if matches!(icon, Icon::Redo) {
                -1.0
            } else {
                1.0
            };
            let points = [
                [5.5 * direction, 5.0],
                [6.5 * direction, 1.5],
                [5.0 * direction, -2.0],
                [2.0 * direction, -4.0],
                [-2.0 * direction, -4.0],
                [-6.0 * direction, -1.0],
            ];
            path(&points);
            path(&[
                [-6.0 * direction, -5.5],
                [-6.0 * direction, -1.0],
                [-1.5 * direction, -1.0],
            ]);
        }
        Icon::Settings => {
            painter.circle_stroke(center, 4.0, stroke);
            painter.circle_stroke(center, 1.25, stroke);
            for (a, b) in [
                ([0.0, -7.0], [0.0, -4.0]),
                ([0.0, 4.0], [0.0, 7.0]),
                ([-7.0, 0.0], [-4.0, 0.0]),
                ([4.0, 0.0], [7.0, 0.0]),
                ([-5.0, -5.0], [-3.0, -3.0]),
                ([3.0, 3.0], [5.0, 5.0]),
                ([5.0, -5.0], [3.0, -3.0]),
                ([-3.0, 3.0], [-5.0, 5.0]),
            ] {
                line(a, b);
            }
        }
        Icon::Export => {
            line([0.0, -7.0], [0.0, 2.0]);
            path(&[[-3.0, -1.0], [0.0, 2.0], [3.0, -1.0]]);
            path(&[[-6.0, 1.0], [-6.0, 6.0], [6.0, 6.0], [6.0, 1.0]]);
        }
        Icon::Reset => {
            path(&[
                [-6.0, -2.0],
                [-3.0, -5.5],
                [1.5, -6.0],
                [5.5, -3.5],
                [6.5, 0.5],
                [4.5, 4.5],
                [0.5, 6.0],
                [-3.5, 4.5],
            ]);
            path(&[[-6.0, -6.0], [-6.0, -2.0], [-2.0, -2.0]]);
        }
        Icon::Close => {
            line([-5.0, -5.0], [5.0, 5.0]);
            line([5.0, -5.0], [-5.0, 5.0]);
        }
        Icon::Add => {
            line([-6.0, 0.0], [6.0, 0.0]);
            line([0.0, -6.0], [0.0, 6.0]);
        }
        Icon::Image => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(14.0, 12.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.circle_filled(point(-3.0, -2.0), 1.2, color);
            path(&[
                [-6.0, 4.0],
                [-1.0, 0.0],
                [2.0, 2.0],
                [4.5, -0.5],
                [6.0, 1.0],
            ]);
        }
        Icon::Layers => {
            path(&[
                [0.0, -6.0],
                [7.0, -2.0],
                [0.0, 2.0],
                [-7.0, -2.0],
                [0.0, -6.0],
            ]);
            path(&[[-7.0, 2.0], [0.0, 6.0], [7.0, 2.0]]);
        }
        Icon::Transitions => {
            path(&[[-6.0, -3.0], [6.0, -3.0], [3.0, -6.0]]);
            path(&[[6.0, 3.0], [-6.0, 3.0], [-3.0, 6.0]]);
        }
        Icon::Help => {
            painter.circle_stroke(center, 7.0, stroke);
            path(&[
                [-2.0, -2.5],
                [-1.0, -4.0],
                [1.5, -4.0],
                [3.0, -2.5],
                [2.0, -0.5],
                [0.0, 1.0],
            ]);
            painter.circle_filled(point(0.0, 4.0), 0.9, color);
        }
        Icon::Aperture => {
            painter.circle_stroke(center, 7.5, stroke);
            for (a, b) in [
                ([-3.5, -6.5], [3.0, -2.0]),
                ([3.5, -6.5], [3.0, 2.0]),
                ([7.0, 0.0], [0.0, 4.0]),
                ([3.5, 6.5], [-3.0, 2.0]),
                ([-3.5, 6.5], [-3.0, -2.0]),
                ([-7.0, 0.0], [0.0, -4.0]),
            ] {
                line(a, b);
            }
        }
        Icon::ChevronLeft => path(&[[2.0, -4.0], [-2.0, 0.0], [2.0, 4.0]]),
        Icon::Scan => {
            for points in [
                [[-3.0, -6.0], [-6.0, -6.0], [-6.0, -3.0]],
                [[3.0, -6.0], [6.0, -6.0], [6.0, -3.0]],
                [[-3.0, 6.0], [-6.0, 6.0], [-6.0, 3.0]],
                [[3.0, 6.0], [6.0, 6.0], [6.0, 3.0]],
            ] {
                path(&points);
            }
            line([-5.0, 0.0], [5.0, 0.0]);
        }
        Icon::Save => {
            path(&[
                [-6.0, -6.0],
                [4.0, -6.0],
                [6.0, -4.0],
                [6.0, 6.0],
                [-6.0, 6.0],
                [-6.0, -6.0],
            ]);
            path(&[[-3.0, -6.0], [-3.0, -1.0], [3.0, -1.0], [3.0, -6.0]]);
            path(&[[-3.0, 6.0], [-3.0, 2.0], [3.0, 2.0], [3.0, 6.0]]);
        }
    }
}

/// A normal framed action button whose canonical name is independent of its icon.
pub(crate) fn icon_button(ui: &mut Ui, label: &str, icon: Icon) -> egui::Response {
    painted_button(ui, label, icon, false)
}

/// A framed icon-and-text action with the same accessible name as its text.
pub(crate) fn action_button(ui: &mut Ui, label: &str, icon: Icon) -> egui::Response {
    painted_button(ui, label, icon, true)
}

fn painted_button(ui: &mut Ui, label: &str, icon: Icon, show_label: bool) -> egui::Response {
    let galley = show_label.then(|| {
        egui::WidgetText::from(label).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Button,
        )
    });
    let width = galley.as_ref().map_or(theme::HEIGHT_CONTROL, |galley| {
        17.0 + theme::SPACE_2 + galley.size().x + ui.spacing().button_padding.x * 2.0
    });
    let response = ui
        .add_sized([width, theme::HEIGHT_CONTROL], egui::Button::new(""))
        .on_hover_text(label);
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_label(label);
    });
    let color = ui.style().interact(&response).text_color();
    let mut icon_rect = response.rect;
    if let Some(galley) = galley {
        icon_rect.min.x += ui.spacing().button_padding.x;
        icon_rect.max.x = icon_rect.min.x + 17.0;
        ui.painter().galley(
            egui::pos2(
                icon_rect.right() + theme::SPACE_2,
                response.rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            color,
        );
    }
    paint_icon(ui.painter(), icon_rect, icon, color);
    super::focus_feedback(ui, &response);
    response
}

/// Editor range for `path`, from the single per-path table. A missing entry
/// is a defect, not a silent default to the machine range.
pub(crate) fn editor_range_for(path: &str) -> editor_ranges::EditorRange {
    editor_ranges::editor_range(path)
        .or_else(|| {
            path.strip_prefix("extras.")
                .and_then(editor_ranges::editor_range)
        })
        .unwrap_or_else(|| panic!("missing editor range for {path}"))
}

pub(crate) fn param_numeric<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    path: &str,
    tooltip: &str,
) -> bool {
    if !super::row_path_matches(ui.ctx(), path) {
        return false;
    }
    let _identity = super::row_identity(ui.ctx(), path);
    let (min, max, step, decimals) = editor_range_for(path);
    let mark = row_modified::RowMark::resolve(ui.ctx(), Some(path));
    let previous = *value;
    let mut changed = numeric_marked(
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
    );
    if changed
        && !row_modified::accepts_value(
            ui.ctx(),
            path,
            if T::INTEGRAL {
                Value::from(value.to_f64() as i64)
            } else {
                Value::from(value.to_f64())
            },
        )
    {
        *value = previous;
        changed = false;
    }
    if changed {
        super::record_edited_path(ui.ctx(), path);
    }
    changed
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
    if !super::row_path_matches(ui.ctx(), path) {
        return false;
    }
    let _identity = super::row_identity(ui.ctx(), path);
    let (min, max, step, decimals) = editor_range_for(path);
    let changed = numeric(
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
    );
    if changed {
        super::record_edited_path(ui.ctx(), path);
    }
    changed
}

/// Per-component identifiers for a tuple row. Empty means the components are
/// numbered by position, which is what the accessible names already carry.
pub(crate) const POSITION_IDENTIFIERS: &[&str] = &[];
/// Component identifiers for a colour-channel tuple.
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
pub(crate) fn tuple_identifiers(path: &str) -> &'static [&'static str] {
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
        | "film_render.grain.uniformity" => CHANNEL_IDENTIFIERS,
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
    param_tuple_applicable(ui, label, values, path, tooltip, &[None; N], false, false)
}

/// Component applicability comes from the core owner. Ordinary controls omit
/// inactive records; discovery retains their requested values read-only.
#[allow(clippy::too_many_arguments)]
pub(crate) fn param_tuple_applicable<T: egui::emath::Numeric, const N: usize>(
    ui: &mut Ui,
    label: &str,
    values: &mut [T; N],
    path: &str,
    tooltip: &str,
    component_reasons: &[Option<&str>; N],
    discovery: bool,
    experimental: bool,
) -> bool {
    if !super::row_path_matches(ui.ctx(), path) {
        return false;
    }
    let _identity = super::row_identity(ui.ctx(), path);
    let (min, max, step, decimals) = editor_range_for(path);
    let changed = tuple_marked(
        ui,
        label,
        values,
        min,
        max,
        step,
        decimals,
        tooltip,
        tuple_identifiers(path),
        experimental,
        row_modified::RowMark::resolve(ui.ctx(), Some(path)),
        component_reasons,
        discovery,
    );
    if changed {
        super::record_edited_path(ui.ctx(), path);
    }
    changed
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
    if !super::row_path_matches(ui.ctx(), path) {
        return false;
    }
    let mut displayed = value.unwrap_or(inherited);
    let numeric_changed = param_numeric(ui, label, &mut displayed, path, tooltip);
    let mut changed = numeric_changed;
    let mut inherited_selected = false;
    ui.horizontal(|ui| {
        ui.label(if value.is_none() {
            crate::tr!(ui.ctx(), "controls.optional.inherited", value = inherited)
        } else {
            crate::i18n::text(ui.ctx(), "controls.optional.explicit")
        });
        if value.is_some()
            && ui
                .small_button(crate::tr!(
                    ui.ctx(),
                    "controls.optional.inherit",
                    label = super::localized(ui.ctx(), label)
                ))
                .clicked()
        {
            *value = None;
            super::record_edited_path(ui.ctx(), path);
            inherited_selected = true;
            changed = true;
        }
    });
    if numeric_changed && !inherited_selected {
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
        ui.label(super::localized(ui.ctx(), "Sublayer shares")).on_hover_text(super::localized(ui.ctx(), "Edit the first three shares; the fourth is the remaining share so the authored total stays one. All four shares must be positive."));
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
        ui.label(crate::tr!(
            ui.ctx(),
            "controls.grain.remainder_value",
            value = format!("{:.6}", shares[3])
        ));
    }
    changed
}

/// A boolean parameter row's editor: a switch.
///
/// Built on a focusable button so pointer and keyboard operability come from
/// the toolkit, then named as a switch for assistive technology. The role is
/// set after the widget is added, because egui rewrites the node's role from
/// its own widget info on every frame.
pub(crate) fn toggle(ui: &mut Ui, label: &str, value: &mut bool, tooltip: &str) -> bool {
    toggle_row(ui, label, value, tooltip).clicked()
}

/// The one switch implementation, returning its response.
///
/// A draft row that owns no canonical row identity — the Export dialog edits
/// options, not inspector parameters — locates itself through its response's
/// canonical path, which [`super::locate_row`] needs.
pub(crate) fn toggle_row(
    ui: &mut Ui,
    label: &str,
    value: &mut bool,
    tooltip: &str,
) -> egui::Response {
    let mut changed = false;
    let response = ui
        .horizontal(|ui| {
            let label_width = (ui.available_width() - 29.0 - ui.spacing().item_spacing.x).max(0.0);
            let label_response = ui
                .allocate_ui_with_layout(
                    egui::vec2(label_width, theme::HEIGHT_CONTROL),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(label_width);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        row_label(ui, label, false)
                    },
                )
                .inner
                .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
            let mark = row_modified::RowMark::resolve(ui.ctx(), Some(label));
            mark.paint(ui, &label_response);
            mark.gold(ui, |ui| {
                let response = ui
                    .add_sized(
                        [29.0, theme::HEIGHT_CONTROL],
                        egui::Button::new("").frame(false),
                    )
                    .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
                super::focus_response(ui, label, &response);
                if response.clicked() {
                    *value = !*value;
                    changed = true;
                }
                let (fill, knob) = if *value {
                    (theme::ACCENT, theme::CONTRAST_ON_ACCENT)
                } else {
                    (theme::SURFACE3, theme::FG_SECONDARY)
                };
                let track =
                    egui::Rect::from_center_size(response.rect.center(), egui::vec2(29.0, 17.0));
                let border = if response.hovered() {
                    theme::ACCENT_DIM
                } else {
                    theme::BORDER_ACTIVE
                };
                ui.painter().rect(
                    track,
                    9.0,
                    fill,
                    theme::stroke(border),
                    egui::StrokeKind::Inside,
                );
                let knob_x = if *value {
                    track.right() - 8.5
                } else {
                    track.left() + 8.5
                };
                ui.painter()
                    .circle_filled(egui::pos2(knob_x, track.center().y), 5.5, knob);
                super::focus_feedback(ui, &response);
                let accessible_label = super::localized(ui.ctx(), label);
                let enabled = ui.is_enabled();
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Checkbox,
                        enabled,
                        *value,
                        accessible_label.as_ref(),
                    )
                });
                // Name the switch and declare the visible label as its labelling
                // node, so a label query resolves to the switch itself.
                let id = response.id;
                let response = response.labelled_by(label_response.id);
                ui.ctx().accesskit_node_builder(id, |node| {
                    node.set_role(egui::accesskit::Role::Switch);
                    node.set_toggled(if *value {
                        egui::accesskit::Toggled::True
                    } else {
                        egui::accesskit::Toggled::False
                    });
                    node.set_label(accessible_label.as_ref());
                });
                response
            })
        })
        .inner;
    if changed {
        super::record_label_edit(ui.ctx(), label);
    }
    response
}
/// The module factory-reset action presented on a module header.
///
/// A quiet frameless text action beside the module disclosure. The caller owns
/// the reset transaction and its history label; this owner names its target
/// and states the factory baseline in accessible help without relying on hover.
pub(crate) fn module_reset(ui: &mut Ui, section: &str) -> egui::Response {
    let response = ui
        .add(
            egui::Button::new(
                egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.state.reset"))
                    .color(theme::FG_SECONDARY)
                    .size(theme::SIZE_UNIT),
            )
            .frame(false)
            .min_size(egui::vec2(theme::SPACE_4 * 3.0, theme::SPACE_3 * 2.0)),
        )
        .on_hover_text(crate::tr!(
            ui.ctx(),
            "surfaces.section.restore",
            section = super::localized(ui.ctx(), section)
        ));
    // Name the action after its section, so one reset is addressable among
    // the sections on a page, and carry the factory-baseline sentence as the
    // node's description.
    let accessible_name = crate::tr!(
        ui.ctx(),
        "surfaces.section.reset_name",
        section = super::localized(ui.ctx(), section)
    );
    let baseline = crate::tr!(
        ui.ctx(),
        "surfaces.section.restore",
        section = super::localized(ui.ctx(), section)
    );
    let enabled = ui.is_enabled();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, accessible_name.as_ref())
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_label(accessible_name);
        node.set_description(baseline);
    });
    super::focus_feedback(ui, &response);
    response
}

/// Mark a row's label as experimental.
///
/// The marker is a chip beside the label rather than a suffix in the label
/// text, so the row's accessible name and its search match stay the bare
/// parameter name.
pub(crate) fn experimental_badge(ui: &mut Ui) {
    ui.label(
        egui::RichText::new(crate::i18n::text(ui.ctx(), "controls.row.experimental"))
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
            egui::RichText::new(crate::controls::localized(ui.ctx(), label))
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

/// Selector/editor for an optional `String` leaf whose unset state means
/// "inherit". Catalog values are offered in the dropdown; a valid custom
/// `BB<kelvin>` value is retained and can be entered through the text field.
pub(crate) fn optional_choice_tip(
    ui: &mut Ui,
    label: &str,
    value: &mut Option<String>,
    unset: &str,
    choices: &[&str],
    tooltip: &str,
) -> bool {
    let mut changed = false;
    let mut selected = value.clone().unwrap_or_else(|| unset.to_owned());
    ui.vertical(|ui| {
        ui.label(crate::controls::localized(ui.ctx(), label))
            .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
        let selector = egui::ComboBox::from_id_salt(label)
            .width(ui.available_width().min(theme::WIDTH_COMBO_MAX))
            .wrap_mode(egui::TextWrapMode::Truncate)
            .selected_text(crate::controls::localized(ui.ctx(), &selected))
            .show_ui(ui, |ui| {
                let entries =
                    std::iter::once((unset, unset)).chain(choices.iter().map(|c| (*c, *c)));
                for (option, display_label) in entries {
                    let is_selected = if value.is_none() {
                        option == unset
                    } else {
                        selected == option
                    };
                    if ui
                        .selectable_label(
                            is_selected,
                            crate::controls::localized(ui.ctx(), display_label),
                        )
                        .on_hover_text(crate::controls::localized(ui.ctx(), tooltip))
                        .clicked()
                        && selected != option
                    {
                        selected = option.to_owned();
                        *value = if option == unset {
                            None
                        } else {
                            Some(option.to_owned())
                        };
                        changed = true;
                    }
                }
            })
            .response
            .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
        super::focus_response(ui, label, &selector);
        name_choice(ui, label);
        let draft_id = ui.make_persistent_id(("optional-illuminant-draft", label));
        let mut text_value = ui
            .ctx()
            .data(|data| data.get_temp::<String>(draft_id))
            .unwrap_or_else(|| value.clone().unwrap_or_default());
        let response = ui
            .add(
                egui::TextEdit::singleline(&mut text_value)
                    .desired_width(ui.available_width().min(theme::WIDTH_COMBO_MAX))
                    .hint_text(crate::i18n::text(
                        ui.ctx(),
                        "controls.illuminant.custom_hint",
                    )),
            )
            .on_hover_text(super::localized(
                ui.ctx(),
                "Enter any shared catalog illuminant or BB<kelvin> (1667..=25000).",
            ));
        let accessible_label = crate::i18n::text(ui.ctx(), "controls.illuminant.custom_capture");
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_label(accessible_label.as_ref())
        });
        if response.changed()
            && (text_value.is_empty()
                || spektrafilm_core::spectral_service::is_supported_illuminant(&text_value))
        {
            *value = (!text_value.is_empty()).then(|| text_value.clone());
            changed = true;
        }
        let stored_draft = if response.lost_focus() {
            value.clone().unwrap_or_default()
        } else {
            text_value
        };
        ui.ctx()
            .data_mut(|data| data.insert_temp(draft_id, stored_draft));
    });
    if changed {
        super::record_label_edit(ui.ctx(), label);
    }
    changed
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
        true,
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
    choice_tip_options(ui, label, value, choices, tooltip, false, true)
}

/// User/profile identities are not product-language choice labels.
pub(crate) fn choice_tip_profile_entries<'a>(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    choices: impl Iterator<Item = (&'a str, &'a str)> + Clone,
    tooltip: &str,
) -> bool {
    choice_tip_options(ui, label, value, choices, tooltip, false, false)
}

pub(crate) fn choice_tip_options<'a>(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    choices: impl Iterator<Item = (&'a str, &'a str)> + Clone,
    tooltip: &str,
    experimental: bool,
    translate_options: bool,
) -> bool {
    let selected_label = choices
        .clone()
        .find(|&(option, _)| option == value.as_str())
        .map(|(_, display_label)| display_label)
        .unwrap_or_else(|| value.as_str())
        .to_owned();
    let prev = value.clone();
    let stacked = label == "workflow";
    let row = |ui: &mut Ui| {
        row_label(ui, label, experimental)
            .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
        let width = if stacked {
            ui.available_width()
        } else {
            ui.available_width()
                .clamp(theme::WIDTH_COMBO_MIN, theme::WIDTH_COMBO_MAX)
        };
        if !stacked {
            ui.add_space((ui.available_width() - width).max(0.0));
        }
        let response = egui::ComboBox::from_id_salt(label)
            .selected_text(if translate_options {
                crate::controls::localized(ui.ctx(), &selected_label)
            } else {
                std::borrow::Cow::Borrowed(selected_label.as_str())
            })
            .width(width)
            .wrap_mode(egui::TextWrapMode::Truncate)
            .show_ui(ui, |ui| {
                for (option, display_label) in choices {
                    if ui
                        .selectable_label(
                            value.as_str() == option,
                            if translate_options {
                                crate::controls::localized(ui.ctx(), display_label)
                            } else {
                                std::borrow::Cow::Borrowed(display_label)
                            },
                        )
                        .on_hover_text(crate::controls::localized(ui.ctx(), tooltip))
                        .clicked()
                        && value.as_str() != option
                    {
                        *value = option.to_owned();
                    }
                }
            })
            .response
            .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
        super::focus_response(ui, label, &response);
        name_choice(ui, label);
    };
    if stacked {
        ui.vertical(row);
    } else {
        ui.horizontal(row);
    }
    if prev != *value {
        super::record_label_edit(ui.ctx(), label);
    }
    prev != *value
}

/// Name a choice row's selector with the parameter's own accessible name.
///
/// The selector's own widget info carries an empty label, and a `labelled_by`
/// link to the visible label would make that label node the one a label query
/// returns, so the selector is named directly instead.
pub(crate) fn name_choice(ui: &Ui, label: &str) {
    let id = ui.make_persistent_id(egui::Id::new(label));
    let accessible_label = super::localized(ui.ctx(), label);
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_label(accessible_label.as_ref());
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
        row_label(ui, label, false).on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
        egui::ComboBox::from_id_salt(label)
            .selected_text(crate::controls::localized(ui.ctx(), current))
            .show_ui(ui, |ui| {
                for &option in choices {
                    if ui
                        .selectable_label(
                            current == option,
                            crate::controls::localized(ui.ctx(), option),
                        )
                        .on_hover_text(crate::controls::localized(ui.ctx(), tooltip))
                        .clicked()
                        && current != option
                    {
                        selected = Some(option);
                    }
                }
            })
            .response
            .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
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
    component_reasons: &[Option<&str>; N],
    discovery: bool,
) -> bool {
    ui.push_id(label, |ui| {
        let label_response = row_label(ui, label, experimental)
            .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
        mark.paint(ui, &label_response);
        mark.gold(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                values
                    .iter_mut()
                    .enumerate()
                    .map(|(index, value)| {
                        let reason = component_reasons[index];
                        if reason.is_some() && !discovery {
                            return false;
                        }
                        ui.add_enabled_ui(reason.is_none(), |ui| {
                            let identifier = identifiers
                                .get(index)
                                .copied()
                                .unwrap_or_else(|| index_identifier(index));
                            let identifier_response = ui.label(
                                egui::RichText::new(crate::controls::localized(
                                    ui.ctx(),
                                    identifier,
                                ))
                                .color(theme::FG_MUTED)
                                .size(theme::SIZE_HINT),
                            );
                            let changed = numeric_field(
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
                            );
                            let field_id = ui.make_persistent_id(("numeric-field", (label, index)));
                            let accessible_label = crate::tr!(
                                ui.ctx(),
                                "controls.component.name",
                                label = super::localized(ui.ctx(), label),
                                component = super::localized(ui.ctx(), identifier)
                            );
                            ui.ctx().accesskit_node_builder(field_id, |node| {
                                node.set_label(accessible_label.as_ref());
                            });
                            if let Some(reason) = reason {
                                ui.label(
                                    egui::RichText::new(super::localized(ui.ctx(), reason))
                                        .small()
                                        .color(theme::FG_MUTED),
                                );
                            }
                            changed
                        })
                        .inner
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
    let mut changed = false;
    ui.label(crate::controls::localized(ui.ctx(), label)).on_hover_text(super::localized(ui.ctx(), "Permutation of film density channels. Selecting a channel swaps its prior position so no record is duplicated or dropped."));
    ui.horizontal(|ui| {
        for index in 0..3 {
            egui::ComboBox::from_id_salt((path, index))
                .selected_text(values[index].to_string())
                .show_ui(ui, |ui| {
                    for channel in 0..3_u32 {
                        if ui
                            .selectable_label(values[index] == channel, channel.to_string())
                            .clicked()
                            && values[index] != channel
                        {
                            let other = values
                                .iter()
                                .position(|value| *value == channel)
                                .expect("channel permutation");
                            values.swap(index, other);
                            changed = true;
                        }
                    }
                });
        }
    });
    if changed {
        set_extra(extras, "special", key, Value::from(values.to_vec()));
        super::record_edited_path(ui.ctx(), path);
    }
    changed
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

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn painter_actions_keep_button_names_and_keyboard_activation() {
        use egui_kittest::{Harness, kittest::Queryable};

        const ACTIONS: [(&str, Icon); 11] = [
            ("Open image", Icon::Open),
            ("Undo adjustment", Icon::Undo),
            ("Redo adjustment", Icon::Redo),
            ("Settings", Icon::Settings),
            ("Export image", Icon::Export),
            ("Reset module", Icon::Reset),
            ("Close dialog", Icon::Close),
            ("Add preset", Icon::Add),
            ("Show image", Icon::Image),
            ("Help", Icon::Help),
            ("Aperture", Icon::Aperture),
        ];
        let mut harness = Harness::new_ui_state(
            |ui, activated: &mut Vec<&'static str>| {
                for (label, icon) in ACTIONS {
                    if icon_button(ui, label, icon).clicked() {
                        activated.push(label);
                    }
                }
                if action_button(ui, "Named text action", Icon::Open).clicked() {
                    activated.push("Named text action");
                }
            },
            Vec::new(),
        );
        for label in ACTIONS
            .map(|(label, _)| label)
            .into_iter()
            .chain(["Named text action"])
        {
            let action = harness.get_by_role_and_label(egui::accesskit::Role::Button, label);
            if label != "Named text action" {
                let bounds = action.raw_bounds().expect("icon button bounds");
                assert!((bounds.x1 - bounds.x0 - f64::from(theme::HEIGHT_CONTROL)).abs() < 1.0);
                assert!((bounds.y1 - bounds.y0 - f64::from(theme::HEIGHT_CONTROL)).abs() < 1.0);
            }
            action.focus();
            harness.run();
            harness.press_key(egui::Key::Space);
            harness.run();
            assert_eq!(harness.state().last(), Some(&label));
        }
        assert_eq!(harness.state().len(), ACTIONS.len() + 1);
    }

    #[test]
    fn compact_switch_keeps_its_canonical_name_value_and_keyboard_behavior() {
        use egui_kittest::{Harness, kittest::Queryable};

        let mut harness = Harness::new_ui_state(
            |ui, value| {
                toggle_row(ui, "Enable diffusion", value, "Toggle diffusion filter");
            },
            false,
        );
        let switch =
            harness.get_by_role_and_label(egui::accesskit::Role::Switch, "Enable diffusion");
        assert_eq!(switch.toggled(), Some(egui::accesskit::Toggled::False));
        switch.focus();
        harness.run();
        harness.press_key(egui::Key::Space);
        harness.run();
        assert!(*harness.state());
        assert_eq!(
            harness.get_by_label("Enable diffusion").toggled(),
            Some(egui::accesskit::Toggled::True),
        );
        harness.get_by_label("Enable diffusion").simulate_click();
        harness.run();
        assert!(!*harness.state());
    }

    #[test]
    fn choice_rows_wrap_long_labels_without_growing_the_module() {
        for width in [280.0_f32, 344.0_f32] {
            let ctx = egui::Context::default();
            crate::theme::apply(&ctx);
            let mut selected = 0;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let available = ui.available_width();
                        let row = ui.scope(|ui| {
                            super::super::enum_choice(
                                ui,
                                "A long choice label that must remain readable",
                                &mut selected,
                                &[(0, "First"), (1, "Second")],
                                "Choice tooltip",
                            )
                        });
                        assert!(!row.inner);
                        assert!(row.response.rect.width() <= available + 1.0);
                        assert!(row.response.rect.height() > theme::HEIGHT_CONTROL);
                    });
                },
            );
            assert_eq!(selected, 0);
        }
    }

    #[test]
    fn optional_capture_illuminant_stays_inside_a_narrow_control_panel() {
        for width in [280.0_f32, 360.0_f32, 460.0_f32] {
            let ctx = egui::Context::default();
            let mut value = None;
            let mut content_width = 0.0;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let available = ui.available_width();
                        let content = ui.scope(|ui| {
                            optional_choice_tip(
                                ui,
                                "capture illuminant",
                                &mut value,
                                "inherit (film profile)",
                                &["D50", "D65"],
                                "Effective inherited capture illuminant: D65",
                            );
                        });
                        content_width = content.response.rect.width();
                        assert!(
                            content_width <= available + 1.0,
                            "{width}: {content_width} > {available}"
                        );
                    });
                },
            );
            assert!(value.is_none());
            assert!(content_width > 0.0);
        }
    }
}
