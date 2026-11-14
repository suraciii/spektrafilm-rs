//! Scan Profile curve editor.
//!
//! The transform owns three monotone per-channel curves. This module edits
//! their knots and paints them. Validation, runtime storage, and preset
//! carriers stay in core; the editor only produces values core already
//! accepts.

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use spektrafilm_core::params::scan_profile::{
    MAX_SCAN_PROFILE_KNOTS, ScanCurve, ScanTransformKind,
};

use crate::controls::theme;
use crate::controls::{
    focus_response, localized, record_edited_path, row_identity, row_modified, row_path_matches,
    widgets,
};
use crate::numeric::{AdjustmentSignals, signal_adjustment};

/// The composite row this editor owns. The knot leaves stay out of the
/// control inventory; this row locates the whole transform.
pub(crate) const CURVE_PATH: &str = "scanner.scan_profile.transform";
const CURVE_LABEL: &str = "Scan Profile Curve";
/// The visible row label. The module already names its subject, so the row
/// names the control.
const CURVE_ROW_LABEL: &str = "Curve";
const CHANNEL_NAMES: [&str; 3] = ["R", "G", "B"];
const SERIES: [Color32; 3] = [theme::SERIES1, theme::SERIES2, theme::SERIES3];
const MIN_KNOTS: usize = 2;
/// One editor step for an input position. Core only requires strictly
/// increasing inputs; a visible minimum keeps two knots distinguishable.
const MIN_GAP: f32 = 1.0 / 1024.0;
const HIT_RADIUS: f32 = 10.0;
const KNOT_RADIUS: f32 = 3.0;
const PLOT_HEIGHT: f32 = 132.0;
const KEY_STEP: f32 = 1.0 / 256.0;

/// Which channel curves an edit writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Scope {
    #[default]
    All,
    Channel(usize),
}

impl Scope {
    fn channels(self) -> impl Iterator<Item = usize> {
        match self {
            Self::All => 0..3,
            Self::Channel(channel) => channel..channel + 1,
        }
    }

    fn includes(self, channel: usize) -> bool {
        match self {
            Self::All => true,
            Self::Channel(selected) => selected == channel,
        }
    }
}

/// One pointer-down gesture. Every frame re-applies the gesture to the curves
/// captured at pointer-down, so clamping never accumulates drift.
#[derive(Clone)]
struct Drag {
    knots: Vec<(usize, usize)>,
    start: [ScanCurve; 3],
    pointer: (f32, f32),
}

#[derive(Clone, Default)]
struct State {
    scope: Scope,
    selected: Option<f32>,
    drag: Option<Drag>,
}

/// Read the editable curves of a `curve` transform. A `matrix_curve`
/// transform has no editor.
fn editable(
    transform: &mut spektrafilm_core::params::ScanProfileTransform,
) -> Option<&mut [ScanCurve; 3]> {
    if transform.kind != ScanTransformKind::Curve {
        return None;
    }
    transform.curves.as_mut()
}

/// Piecewise-linear sample with endpoint clamp, mirroring the runtime.
fn sample(curve: &ScanCurve, x: f32) -> f32 {
    if curve.x.is_empty() {
        return 0.0;
    }
    let first = curve.x.first().copied().unwrap_or(0.0);
    let last = curve.x.last().copied().unwrap_or(1.0);
    let x = x.clamp(first, last);
    let upper = curve.x.partition_point(|candidate| *candidate < x);
    if upper == 0 {
        return curve.y[0];
    }
    if upper >= curve.x.len() {
        return *curve.y.last().unwrap();
    }
    let (x0, x1) = (curve.x[upper - 1], curve.x[upper]);
    let (y0, y1) = (curve.y[upper - 1], curve.y[upper]);
    if x1 == x0 {
        y1
    } else {
        y0 + (y1 - y0) * (x - x0) / (x1 - x0)
    }
}

fn index_at(curve: &ScanCurve, x: f32) -> Option<usize> {
    curve
        .x
        .iter()
        .position(|candidate| (candidate - x).abs() < MIN_GAP)
}

/// Insert the input position into every channel in `scope`. The inserted knot
/// takes that channel's current output there, so inserting alone never changes
/// the rendered image.
fn insert_at(curves: &mut [ScanCurve; 3], scope: Scope, x: f32) -> bool {
    if !(MIN_GAP..=1.0 - MIN_GAP).contains(&x) {
        return false;
    }
    let channels: Vec<usize> = scope
        .channels()
        .filter(|channel| index_at(&curves[*channel], x).is_none())
        .collect();
    if channels
        .iter()
        .any(|channel| curves[*channel].x.len() >= MAX_SCAN_PROFILE_KNOTS)
    {
        return false;
    }
    for channel in channels {
        let curve = &mut curves[channel];
        let y = sample(curve, x);
        let position = curve.x.partition_point(|candidate| *candidate < x);
        curve.x.insert(position, x);
        curve.y.insert(position, y);
    }
    true
}

/// Remove the knot at `x` from every channel in `scope` that can lose it.
fn remove_at(curves: &mut [ScanCurve; 3], scope: Scope, x: f32) -> bool {
    let mut changed = false;
    for channel in scope.channels() {
        let curve = &mut curves[channel];
        let Some(index) = index_at(curve, x) else {
            continue;
        };
        if index == 0 || index + 1 == curve.x.len() || curve.x.len() <= MIN_KNOTS {
            continue;
        }
        curve.x.remove(index);
        curve.y.remove(index);
        changed = true;
    }
    changed
}

/// Apply one gesture delta. The delta is clamped to the intersection of every
/// selected knot's legal range, so a linked edit moves all channels by the same
/// amount and each channel stays monotone.
fn apply_drag(drag: &Drag, curves: &mut [ScanCurve; 3], target: (f32, f32)) -> bool {
    let (raw_dx, raw_dy) = (target.0 - drag.pointer.0, target.1 - drag.pointer.1);
    let mut dx = (f32::NEG_INFINITY, f32::INFINITY);
    let mut dy = (f32::NEG_INFINITY, f32::INFINITY);
    for &(channel, index) in &drag.knots {
        let curve = &drag.start[channel];
        if index >= curve.x.len() {
            return false;
        }
        let (x0, y0) = (curve.x[index], curve.y[index]);
        if index > 0 && index + 1 < curve.x.len() {
            dx.0 = dx.0.max(curve.x[index - 1] + MIN_GAP - x0);
            dx.1 = dx.1.min(curve.x[index + 1] - MIN_GAP - x0);
        } else {
            // A domain endpoint keeps its input position.
            dx.0 = dx.0.max(0.0);
            dx.1 = dx.1.min(0.0);
        }
        let floor = if index > 0 { curve.y[index - 1] } else { 0.0 };
        let ceiling = if index + 1 < curve.y.len() {
            curve.y[index + 1]
        } else {
            1.0
        };
        dy.0 = dy.0.max(floor - y0);
        dy.1 = dy.1.min(ceiling - y0);
    }
    let dx = if dx.0 <= dx.1 {
        raw_dx.clamp(dx.0, dx.1)
    } else {
        0.0
    };
    let dy = if dy.0 <= dy.1 {
        raw_dy.clamp(dy.0, dy.1)
    } else {
        0.0
    };
    let mut changed = false;
    for &(channel, index) in &drag.knots {
        let (x0, y0) = (drag.start[channel].x[index], drag.start[channel].y[index]);
        let (x, y) = ((x0 + dx).clamp(0.0, 1.0), (y0 + dy).clamp(0.0, 1.0));
        let curve = &mut curves[channel];
        if curve.x[index] != x || curve.y[index] != y {
            curve.x[index] = x;
            curve.y[index] = y;
            changed = true;
        }
    }
    changed
}

/// Map the transform domain to the plot rectangle.
fn to_screen(rect: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(
        rect.left() + x * rect.width(),
        rect.bottom() - y * rect.height(),
    )
}

fn to_value(rect: Rect, position: Pos2) -> (f32, f32) {
    (
        ((position.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0),
        ((rect.bottom() - position.y) / rect.height().max(1.0)).clamp(0.0, 1.0),
    )
}

/// The nearest knot within the hit radius, as a channel and index.
fn hit_test(
    curves: &[ScanCurve; 3],
    rect: Rect,
    scope: Scope,
    position: Pos2,
) -> Option<(usize, usize)> {
    let mut best: Option<(f32, usize, usize)> = None;
    for channel in scope.channels() {
        let curve = &curves[channel];
        for (index, (&x, &y)) in curve.x.iter().zip(curve.y.iter()).enumerate() {
            let distance = to_screen(rect, x, y).distance(position);
            if distance <= HIT_RADIUS && best.is_none_or(|(nearest, ..)| distance < nearest) {
                best = Some((distance, channel, index));
            }
        }
    }
    best.map(|(_, channel, index)| (channel, index))
}

fn knot_at(curves: &[ScanCurve; 3], channel: usize, index: usize) -> Option<f32> {
    curves[channel].x.get(index).copied()
}

pub(crate) fn editor(
    ui: &mut egui::Ui,
    transform: &mut spektrafilm_core::params::ScanProfileTransform,
) -> bool {
    if !row_path_matches(ui.ctx(), CURVE_PATH) {
        return false;
    }
    let Some(curves) = editable(transform) else {
        ui.label(localized(
            ui.ctx(),
            "This Scan Profile carries a fixed matrix; it has no editable curve.",
        ));
        return false;
    };
    let _identity = row_identity(ui.ctx(), CURVE_PATH);
    let mark = row_modified::RowMark::resolve(ui.ctx(), Some(CURVE_PATH));
    ui.push_id(("scan_profile_curve", CURVE_PATH), |ui| {
        edit(ui, curves, mark)
    })
    .inner
}

fn edit(ui: &mut egui::Ui, curves: &mut [ScanCurve; 3], mark: row_modified::RowMark) -> bool {
    let mut changed = false;
    let label = widgets::row_label(ui, CURVE_ROW_LABEL, false);
    mark.paint(ui, &label);
    ui.label(
        egui::RichText::new(localized(ui.ctx(), "Channel"))
            .color(theme::FG_MUTED)
            .size(theme::SIZE_HINT),
    )
    .on_hover_text(localized(
        ui.ctx(),
        "All edits every channel by the same amount. R, G, or B edits one curve alone.",
    ));

    let id = ui.id().with("curve_state");
    let mut state = ui
        .data(|data| data.get_temp::<State>(id))
        .unwrap_or_default();
    ui.horizontal(|ui| {
        for (index, name) in std::iter::once("All")
            .chain(CHANNEL_NAMES.iter().copied())
            .enumerate()
        {
            let scope = if index == 0 {
                Scope::All
            } else {
                Scope::Channel(index - 1)
            };
            if ui
                .selectable_label(state.scope == scope, localized(ui.ctx(), name))
                .clicked()
                && state.scope != scope
            {
                // The channel scope is presentation. Switching it is not a
                // parameter edit and must not create a history entry.
                state.scope = scope;
                state.selected = None;
            }
        }
    });

    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, PLOT_HEIGHT), Sense::click_and_drag());
    let response = response.on_hover_text(localized(
        ui.ctx(),
        "Drag a knot to reshape the tone, click empty space to add one, and press Delete to remove the selected knot. Each channel holds up to nine knots.",
    ));
    // Resolve the accessible name before `widget_info`: egui runs that closure
    // under its context write lock, and reading context data there deadlocks.
    let accessible_name = localized(ui.ctx(), CURVE_LABEL).into_owned();
    let enabled = ui.is_enabled();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, enabled, accessible_name.clone())
    });
    focus_response(ui, CURVE_LABEL, &response);

    paint(ui, rect, curves, &state, &response);

    if !ui.is_enabled() {
        ui.data_mut(|data| data.insert_temp(id, state));
        return changed;
    }

    // Pointer: grab a knot at the press origin, or insert one and drag it. A
    // click that never becomes a drag still commits its insertion.
    if response.drag_started()
        && let Some(position) = ui.input(|input| input.pointer.press_origin())
    {
        let target = to_value(rect, position);
        let mut knots = Vec::new();
        match hit_test(curves, rect, state.scope, position) {
            Some((channel, index)) => {
                // A linked drag moves every selected channel that already has a
                // knot at the grabbed input. A channel with a different input
                // grid keeps its own knot.
                let input = curves[channel].x[index];
                for candidate in state.scope.channels() {
                    if let Some(index) = index_at(&curves[candidate], input) {
                        knots.push((candidate, index));
                    }
                }
            }
            None => {
                changed |= insert_at(curves, state.scope, target.0);
                for channel in state.scope.channels() {
                    if let Some(index) = index_at(&curves[channel], target.0) {
                        knots.push((channel, index));
                    }
                }
            }
        }
        if !knots.is_empty() {
            state.selected = Some(target.0);
            state.drag = Some(Drag {
                knots,
                start: curves.clone(),
                pointer: target,
            });
            signal_adjustment(
                ui.ctx(),
                CURVE_LABEL,
                AdjustmentSignals {
                    started: true,
                    active: true,
                    ..Default::default()
                },
            );
        }
    }

    if response.dragged()
        && let Some(position) = response.interact_pointer_pos()
        && let Some(drag) = state.drag.clone()
    {
        let target = to_value(rect, position);
        changed |= apply_drag(&drag, curves, target);
        state.selected = state
            .drag
            .as_ref()
            .and_then(|drag| drag.knots.first().copied())
            .and_then(|(channel, index)| knot_at(curves, channel, index));
        signal_adjustment(
            ui.ctx(),
            CURVE_LABEL,
            AdjustmentSignals {
                active: true,
                ..Default::default()
            },
        );
    }

    if response.drag_stopped() {
        state.drag = None;
        signal_adjustment(
            ui.ctx(),
            CURVE_LABEL,
            AdjustmentSignals {
                completed: true,
                ..Default::default()
            },
        );
    }

    if response.clicked()
        && let Some(position) = response.interact_pointer_pos()
    {
        if let Some((channel, index)) = hit_test(curves, rect, state.scope, position) {
            state.selected = knot_at(curves, channel, index);
        } else if insert_at(curves, state.scope, to_value(rect, position).0) {
            state.selected = Some(to_value(rect, position).0);
            changed = true;
            signal_adjustment(
                ui.ctx(),
                CURVE_LABEL,
                AdjustmentSignals {
                    started: true,
                    completed: true,
                    ..Default::default()
                },
            );
        }
    }

    if ui.input(|input| input.key_pressed(egui::Key::Escape)) && state.drag.is_some() {
        state.drag = None;
        signal_adjustment(
            ui.ctx(),
            CURVE_LABEL,
            AdjustmentSignals {
                canceled: true,
                ..Default::default()
            },
        );
    }

    // Delete removes the selected interior knot from the active scope.
    if response.has_focus() || response.hovered() {
        let delete = ui.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        });
        if delete && let Some(x) = state.selected {
            changed |= remove_at(curves, state.scope, x);
            state.selected = None;
            if changed {
                signal_adjustment(
                    ui.ctx(),
                    CURVE_LABEL,
                    AdjustmentSignals {
                        started: true,
                        completed: true,
                        ..Default::default()
                    },
                );
            }
        }
    }

    // Keyboard editing of the selected knot: arrows move it by one step.
    if response.has_focus()
        && let Some(x) = state.selected
    {
        let step = ui.input(|input| {
            if input.modifiers.shift {
                KEY_STEP
            } else {
                KEY_STEP * 4.0
            }
        });
        let delta = ui.input(|input| {
            let axis = |positive: bool, negative: bool| (positive as i32 - negative as i32) as f32;
            (
                axis(
                    input.key_pressed(egui::Key::ArrowRight),
                    input.key_pressed(egui::Key::ArrowLeft),
                ),
                axis(
                    input.key_pressed(egui::Key::ArrowUp),
                    input.key_pressed(egui::Key::ArrowDown),
                ),
            )
        });
        if delta != (0.0, 0.0) {
            let mut knots = Vec::new();
            for channel in state.scope.channels() {
                if let Some(index) = index_at(&curves[channel], x) {
                    knots.push((channel, index));
                }
            }
            if !knots.is_empty() {
                let (first_channel, first_index) = knots[0];
                let start_y = curves[first_channel].y[first_index];
                let drag = Drag {
                    knots,
                    start: curves.clone(),
                    pointer: (x, start_y),
                };
                let target = (x + delta.0 * step, start_y + delta.1 * step);
                changed |= apply_drag(&drag, curves, target);
                state.selected = knot_at(curves, first_channel, first_index);
                signal_adjustment(
                    ui.ctx(),
                    CURVE_LABEL,
                    AdjustmentSignals {
                        started: true,
                        completed: true,
                        ..Default::default()
                    },
                );
            }
        }
    }

    if changed {
        record_edited_path(ui.ctx(), CURVE_PATH);
    }
    ui.data_mut(|data| data.insert_temp(id, state));
    changed
}

fn paint(
    ui: &egui::Ui,
    rect: Rect,
    curves: &[ScanCurve; 3],
    state: &State,
    response: &egui::Response,
) {
    let painter = ui.painter();
    painter.rect_filled(rect, theme::RADIUS_CARD, theme::SURFACE2);
    painter.rect_stroke(
        rect,
        theme::RADIUS_CARD,
        Stroke::new(theme::STROKE_DEFAULT, theme::BORDER),
        StrokeKind::Inside,
    );
    for step in 1..4 {
        let t = step as f32 / 4.0;
        painter.line_segment(
            [to_screen(rect, t, 0.0), to_screen(rect, t, 1.0)],
            Stroke::new(1.0_f32, theme::BORDER),
        );
        painter.line_segment(
            [to_screen(rect, 0.0, t), to_screen(rect, 1.0, t)],
            Stroke::new(1.0_f32, theme::BORDER),
        );
    }
    for (channel, curve) in curves.iter().enumerate() {
        let points: Vec<Pos2> = curve
            .x
            .iter()
            .zip(curve.y.iter())
            .map(|(&x, &y)| to_screen(rect, x, y))
            .collect();
        if points.len() < 2 {
            continue;
        }
        let color = if state.scope.includes(channel) {
            SERIES[channel]
        } else {
            SERIES[channel].gamma_multiply(0.35)
        };
        painter.add(egui::Shape::line(points, Stroke::new(1.5_f32, color)));
    }
    for channel in state.scope.channels() {
        let curve = &curves[channel];
        for (&x, &y) in curve.x.iter().zip(curve.y.iter()) {
            let center = to_screen(rect, x, y);
            let selected = state
                .selected
                .is_some_and(|selected| (selected - x).abs() < MIN_GAP);
            let radius = if selected {
                KNOT_RADIUS + 1.5
            } else {
                KNOT_RADIUS
            };
            if selected {
                painter.circle_stroke(
                    center,
                    radius + 2.0,
                    Stroke::new(theme::STROKE_DEFAULT, theme::ACCENT),
                );
            }
            painter.circle_filled(center, radius, SERIES[channel]);
            painter.circle_stroke(center, radius, Stroke::new(1.0_f32, theme::SURFACE2));
        }
    }
    if let Some(position) = response
        .hover_pos()
        .filter(|position| rect.contains(*position))
    {
        let (x, _) = to_value(rect, position);
        painter.line_segment(
            [to_screen(rect, x, 0.0), to_screen(rect, x, 1.0)],
            Stroke::new(1.0_f32, theme::FG_MUTED),
        );
        let mut text = format!("{x:.3}");
        for channel in state.scope.channels() {
            text.push_str(&format!(
                "  {} {:.3}",
                CHANNEL_NAMES[channel],
                sample(&curves[channel], x)
            ));
        }
        painter.text(
            Pos2::new(rect.left() + 6.0, rect.top() + 4.0),
            Align2::LEFT_TOP,
            text,
            theme::text(egui::TextStyle::Small, theme::SIZE_HINT),
            theme::FG_SECONDARY,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curves() -> [ScanCurve; 3] {
        let curve = || ScanCurve {
            x: vec![0.0, 0.5, 1.0],
            y: vec![0.0, 0.5, 1.0],
        };
        [curve(), curve(), curve()]
    }

    #[test]
    fn inserting_keeps_the_rendered_image_and_respects_the_knot_limit() {
        let mut curves = curves();
        let before = curves.clone();
        assert!(insert_at(&mut curves, Scope::All, 0.25));
        for (after, before) in curves.iter().zip(before.iter()) {
            assert_eq!(after.x.len(), 4);
            assert!((sample(after, 0.3) - sample(before, 0.3)).abs() < 1e-6);
        }
        for step in 0..MAX_SCAN_PROFILE_KNOTS {
            insert_at(&mut curves, Scope::All, 0.05 + step as f32 * 0.01);
        }
        assert_eq!(curves[0].x.len(), MAX_SCAN_PROFILE_KNOTS);
        assert!(!insert_at(&mut curves, Scope::All, 0.15));
    }

    #[test]
    fn a_linked_drag_keeps_every_channel_monotone_and_aligned() {
        let mut curves = curves();
        let start = curves.clone();
        let drag = Drag {
            knots: vec![(0, 1), (1, 1), (2, 1)],
            start: start.clone(),
            pointer: (0.5, 0.5),
        };
        assert!(apply_drag(&drag, &mut curves, (0.6, 0.9)));
        for curve in &curves {
            assert!(curve.y.windows(2).all(|pair| pair[0] <= pair[1]));
            assert!(curve.x.windows(2).all(|pair| pair[0] < pair[1]));
            assert!((curve.y[1] - 0.9).abs() < 1e-6);
            assert!((curve.x[1] - 0.6).abs() < 1e-6);
        }
        // A delta past the lower neighbour clamps instead of reversing.
        let mut reversed = start.clone();
        let mut down = Drag {
            knots: vec![(0, 1), (1, 1), (2, 1)],
            start,
            pointer: (0.5, 0.5),
        };
        down.pointer = (0.5, 0.5);
        assert!(apply_drag(&down, &mut reversed, (0.5, -5.0)));
        assert!(reversed.iter().all(|curve| curve.y[1] >= 0.0));
        assert!(reversed.iter().all(|curve| curve.y[0] <= curve.y[1]));
    }

    #[test]
    fn plot_coordinates_roundtrip_and_find_the_nearest_knot() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(200.0, 100.0));
        let (x, y) = to_value(rect, to_screen(rect, 0.3, 0.7));
        assert!((x - 0.3).abs() < 1e-6);
        assert!((y - 0.7).abs() < 1e-6);
        let curves = curves();
        let middle = to_screen(rect, 0.5, 0.5);
        assert_eq!(hit_test(&curves, rect, Scope::All, middle), Some((0, 1)));
        let outside = to_screen(rect, 0.25, 0.25);
        assert_eq!(hit_test(&curves, rect, Scope::All, outside), None);
    }

    #[test]
    fn endpoints_keep_their_inputs_and_deletion_spares_them() {
        let mut curves = curves();
        let start = curves.clone();
        let drag = Drag {
            knots: vec![(0, 0)],
            start,
            pointer: (0.0, 0.0),
        };
        assert!(apply_drag(&drag, &mut curves, (0.4, 0.2)));
        assert_eq!(curves[0].x[0], 0.0);
        assert!((curves[0].y[0] - 0.2).abs() < 1e-6);
        assert!(!remove_at(&mut curves, Scope::All, 0.0));
        assert!(remove_at(&mut curves, Scope::All, 0.5));
        assert_eq!(curves[0].x.len(), MIN_KNOTS);
    }
}
