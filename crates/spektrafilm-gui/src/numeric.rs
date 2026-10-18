use std::hash::Hash;

use egui::{TextEdit, Ui};
use serde_json::Value;

use crate::theme;

/// Whether a numeric row's slider spans its range linearly or logarithmically.
///
/// Derived from the declared editor range, which stays the only source of
/// value decimals and slider bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SliderScale {
    Linear,
    Logarithmic,
}

/// Pick the slider scale for a declared editor range.
///
/// Logarithmic when the range spans more than three decades, or when the
/// declared minimum is zero and the declared maximum reaches one thousand or
/// more. A linear slider over those puts every working value within a
/// fraction of a pixel of the track's zero end.
///
/// A signed range stays linear: a symmetric range is usable as declared, and
/// a logarithmic track would only add a zero cutoff that hides half of it.
pub(crate) fn slider_scale(min: f64, max: f64) -> SliderScale {
    if min > 0.0 && (max / min).log10() > 3.0 {
        return SliderScale::Logarithmic;
    }
    if min == 0.0 && max >= 1000.0 {
        return SliderScale::Logarithmic;
    }
    SliderScale::Linear
}

#[derive(Clone, Debug)]
struct NumericEditState {
    text: String,
    last_value: f64,
    edit_start: f64,
    focused: bool,
}

/// Unit suffix for a parameter path, from core parameter discovery.
///
/// Core owns the unit vocabulary, so the GUI reads it rather than keeping a
/// second table. The result is cached because `describe_field` walks JSON.
///
/// A handful of rows display an abbreviated suffix so the row stays on one
/// line inside the fixed-width panel. The abbreviation is display-only: the
/// tooltip and core discovery keep the full unit.
pub(crate) fn unit_suffix(path: &str) -> Option<&'static str> {
    static DISPLAY_SUFFIXES: std::sync::LazyLock<std::collections::HashMap<String, &'static str>> =
        std::sync::LazyLock::new(|| {
            let mut suffixes: std::collections::HashMap<String, &'static str> =
                std::collections::HashMap::new();
            // "linear exposure multiplier" is the longest unit in the core
            // vocabulary; the abbreviated form keeps the row single-line
            // without truncating the label.
            suffixes.insert("enlarger.preflash_exposure".to_owned(), "exposure ×");
            suffixes
        });
    static UNITS: std::sync::LazyLock<std::collections::HashMap<String, Option<String>>> =
        std::sync::LazyLock::new(|| {
            let mut units: std::collections::HashMap<String, Option<String>> =
                crate::editor_ranges::paths()
                    .map(|path| {
                        let unit = spektrafilm_core::params::sources::describe_field(path)
                            .ok()
                            .and_then(|field| {
                                field.get("unit").and_then(Value::as_str).map(str::to_owned)
                            })
                            .filter(|unit| !unit.is_empty());
                        (path.to_owned(), unit)
                    })
                    .collect();
            // These two numeric rows live only in GUI extras state, so core has
            // no entry for them.
            units.insert(
                "extras.load_raw.temperature".to_owned(),
                Some("K".to_owned()),
            );
            units.insert("extras.load_raw.tint".to_owned(), Some("CC".to_owned()));
            units
        });
    if let Some(display) = DISPLAY_SUFFIXES.get(path) {
        return Some(display);
    }
    UNITS
        .get(path)
        .and_then(|unit| unit.as_deref())
        // A composite unit such as `[px (sigma), dimensionless (amount)]`
        // describes a whole tuple and is not shown per component.
        .filter(|unit| !unit.starts_with('['))
}

fn format_value(value: f64, integral: bool, decimals: usize) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    if integral {
        format!("{value:.0}")
    } else {
        format!("{value:.decimals$}")
    }
}

fn parse_value<T: egui::emath::Numeric>(text: &str, min: f64, max: f64) -> Option<f64> {
    let parsed = if text.chars().any(|ch| ch.is_whitespace() || ch == '−') {
        text.chars()
            .filter(|ch| !ch.is_whitespace())
            .map(|ch| if ch == '−' { '-' } else { ch })
            .collect::<String>()
            .parse::<f64>()
            .ok()?
    } else {
        text.parse::<f64>().ok()?
    };
    if !parsed.is_finite() || (T::INTEGRAL && parsed.fract() != 0.0) {
        return None;
    }
    if parsed < min || parsed > max {
        return None;
    }
    Some(parsed)
}

/// One parameter row with an optional modified marker.
#[allow(clippy::too_many_arguments)]
pub(crate) fn numeric<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
    unit: Option<&str>,
    experimental: bool,
) -> bool {
    numeric_marked(
        ui,
        label,
        value,
        min,
        max,
        step,
        decimals,
        tooltip,
        unit,
        experimental,
        crate::controls::row_modified::RowMark::NONE,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn numeric_marked<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
    unit: Option<&str>,
    experimental: bool,
    mark: crate::controls::row_modified::RowMark,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let label_response = crate::controls::row_label(ui, label, experimental);
        label_response.clone().on_hover_text(tooltip);
        mark.paint(ui, &label_response);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed |= mark.gold(ui, |ui| {
                numeric_field(
                    ui,
                    label,
                    label,
                    value,
                    min,
                    max,
                    step,
                    decimals,
                    tooltip,
                    unit,
                    Some(label_response.id),
                )
            });
        });
    });
    changed |= row_slider(ui, label, value, min, max, step);
    changed
}

/// The slider that sits under a numeric row's value.
///
/// Salted by the row label plus the declared bounds, so two rows that share a
/// label in different sections keep separate sliders.
#[allow(clippy::too_many_arguments)]
fn row_slider<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    min: f64,
    max: f64,
    step: f64,
) -> bool {
    let logarithmic = slider_scale(min, max) == SliderScale::Logarithmic;
    // A logarithmic track anchors its positive floor at the declared step, so
    // the granularity the value field offers is the granularity the track
    // spans. egui honours `smallest_positive` only when the range minimum is
    // zero, so a positive minimum moves the floor onto the track itself; a
    // zero minimum keeps its zero, which is a value the editor accepts.
    let track_min = if logarithmic && min > 0.0 {
        min.max(step)
    } else {
        min
    };
    let mut slider = egui::Slider::new(value, T::from_f64(track_min)..=T::from_f64(max))
        .show_value(false)
        .clamping(egui::SliderClamping::Never);
    if logarithmic {
        slider = slider
            .logarithmic(true)
            .smallest_positive(step.max(f64::MIN_POSITIVE));
    }
    let response = ui.add(slider).on_hover_text(tooltip_for(label));
    response.changed()
}

/// Tooltip text for a row slider: the value field carries the parameter's own
/// tooltip, so the slider names the interaction instead.
fn tooltip_for(label: &str) -> String {
    format!("Drag to change {label}")
}

// Numeric widgets keep their existing parameter interfaces.
#[allow(clippy::too_many_arguments)]
pub(crate) fn numeric_field<T, Id>(
    ui: &mut Ui,
    label: &str,
    id_salt: Id,
    value: &mut T,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
    unit: Option<&str>,
    labelled_by: Option<egui::Id>,
) -> bool
where
    T: egui::emath::Numeric,
    Id: Hash,
{
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
        |_| true,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn numeric_field_validated<T, Id>(
    ui: &mut Ui,
    label: &str,
    id_salt: Id,
    value: &mut T,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
    unit: Option<&str>,
    labelled_by: Option<egui::Id>,
    valid: impl Fn(T) -> bool,
) -> bool
where
    T: egui::emath::Numeric,
    Id: Hash,
{
    let decimals = if T::INTEGRAL { 0 } else { decimals };
    let current = value.to_f64();
    let id = ui.make_persistent_id(("numeric-field", id_salt));
    let mut state = ui
        .ctx()
        .data(|data| data.get_temp::<NumericEditState>(id))
        .unwrap_or_else(|| NumericEditState {
            text: format_value(current, T::INTEGRAL, decimals),
            last_value: current,
            edit_start: current,
            focused: false,
        });

    if state.last_value.to_bits() != current.to_bits()
        || (state.focused && !ui.memory(|memory| memory.has_focus(id)))
    {
        state.text = format_value(current, T::INTEGRAL, decimals);
    }

    // Determine if the current text is invalid for visual feedback.
    let parsed_for_validation = parse_value::<T>(&state.text, min, max);
    let is_invalid = match parsed_for_validation {
        None => true,
        Some(v) => !valid(T::from_f64(v)),
    };

    let mut changed = false;
    ui.horizontal(|ui| {
        let mut text_edit = TextEdit::singleline(&mut state.text)
            .id(id)
            .font(theme::text(theme::TEXT_VALUE, theme::SIZE_VALUE))
            .desired_width(theme::WIDTH_VALUE);
        if is_invalid {
            // Invalid state is colour plus a border, never colour alone.
            text_edit = text_edit.text_color(theme::ERROR);
        }
        let mut response = ui.add(text_edit).on_hover_text(tooltip);
        // Name the editor itself, so assistive technology and the
        // accessibility tree's label lookup both find the field rather than a
        // separate label node, and declare the visible label as the field's
        // labelling node so the two read as one control.
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_label(label);
        });
        if let Some(label_id) = labelled_by {
            response = response.labelled_by(label_id);
        }
        if let Some(unit) = unit {
            ui.label(
                egui::RichText::new(unit)
                    .color(theme::FG_MUTED)
                    .size(theme::SIZE_UNIT),
            )
            .on_hover_text(tooltip);
        }
        if is_invalid {
            ui.painter().rect_stroke(
                response.rect.expand(theme::INVALID_RING_EXPAND),
                0.0_f32,
                egui::Stroke::new(theme::STROKE_INVALID, theme::ERROR),
                egui::StrokeKind::Inside,
            );
        }
        value_row_events(
            ui,
            &response,
            value,
            &mut state,
            id,
            min,
            max,
            step,
            decimals,
            valid,
            &mut changed,
        );
    });

    changed
}

/// Commit a value field's edits: typed text, the wheel, and focus loss.
///
/// Called inside the value row's horizontal scope so the field's own widget
/// state is written once per frame, whatever else the row renders.
#[allow(clippy::too_many_arguments)]
fn value_row_events<T: egui::emath::Numeric>(
    ui: &Ui,
    response: &egui::Response,
    value: &mut T,
    state: &mut NumericEditState,
    id: egui::Id,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    valid: impl Fn(T) -> bool,
    changed: &mut bool,
) {
    if response.gained_focus() {
        state.edit_start = value.to_f64();
    }
    if response.changed()
        && let Some(parsed) = parse_value::<T>(&state.text, min, max)
    {
        let next = T::from_f64(parsed);
        if valid(next)
            && (T::INTEGRAL || state.text.trim() != "0" || response.lost_focus())
            && *value != next
        {
            *value = next;
            *changed = true;
        }
    }
    let mut wheel_value = *value;
    let wheel_changed =
        wheel_adjust(ui, response, &mut wheel_value, min, max, step) && valid(wheel_value);
    if wheel_changed {
        *value = wheel_value;
        state.text = format_value(value.to_f64(), T::INTEGRAL, decimals);
        *changed = true;
    }

    if response.lost_focus() {
        let final_value = parse_value::<T>(&state.text, min, max).map(T::from_f64);
        let target = match final_value {
            // Commit a valid draft deferred while focused (e.g. a bare "0").
            Some(next) if valid(next) => Some(next),
            // Roll back to the focus-start value only while it still passes
            // validation; a paired field may have moved since focus started.
            _ => {
                let start = T::from_f64(state.edit_start);
                valid(start).then_some(start)
            }
        };
        if let Some(next) = target
            && *value != next
        {
            *value = next;
            *changed = true;
        }
        state.text = format_value(value.to_f64(), T::INTEGRAL, decimals);
    }
    state.last_value = value.to_f64();
    state.focused = response.has_focus();
    ui.ctx()
        .data_mut(|data| data.insert_temp(id, state.clone()));
}

fn apply_wheel_delta<T: egui::emath::Numeric>(
    value: &mut T,
    delta: f32,
    min: f64,
    max: f64,
    step: f64,
) -> bool {
    if delta == 0.0 {
        return false;
    }
    let direction = if delta.is_sign_positive() { 1.0 } else { -1.0 };
    let current = value.to_f64();
    let next = (current + direction * step).clamp(min, max);
    if next == current {
        return false;
    }
    *value = T::from_f64(next);
    true
}

fn wheel_adjust<T: egui::emath::Numeric>(
    ui: &Ui,
    response: &egui::Response,
    value: &mut T,
    min: f64,
    max: f64,
    step: f64,
) -> bool {
    if !response.hovered() {
        return false;
    }
    // Numeric steps follow physical wheel events, not egui's smoothing tail:
    // one notch must not change the value again on later repaint frames.
    let delta = ui.input(|input| input.raw_scroll_delta.y);
    let changed = apply_wheel_delta(value, delta, min, max, step);
    ui.input_mut(|input| {
        input.raw_scroll_delta.y = 0.0;
        input.smooth_scroll_delta.y = 0.0;
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::{SliderScale, numeric_field, parse_value, slider_scale};

    #[test]
    fn slider_scale_picks_logarithmic_only_for_wide_positive_ranges() {
        // Signed ranges stay linear.
        assert_eq!(slider_scale(-10.0, 10.0), SliderScale::Linear);
        // A zero floor with a narrow span stays linear.
        assert_eq!(slider_scale(0.0, 100.0), SliderScale::Linear);
        // A zero floor at or past a thousand goes logarithmic.
        assert_eq!(slider_scale(0.0, 1000.0), SliderScale::Logarithmic);
        assert_eq!(slider_scale(0.0, 1000000.0), SliderScale::Logarithmic);
        // A positive floor spans more than three decades.
        assert_eq!(slider_scale(1.0, 100.0), SliderScale::Linear);
        assert_eq!(slider_scale(1.0, 10000.0), SliderScale::Logarithmic);
        // Exactly three decades is not more than three decades.
        assert_eq!(slider_scale(1000.0, 1000000.0), SliderScale::Linear);
    }

    #[test]
    fn incomplete_and_nonfinite_text_does_not_change_parameters() {
        for text in ["", "-", "1e", "NaN", "inf", "-inf"] {
            assert_eq!(parse_value::<f64>(text, -10.0, 10.0), None);
        }
        assert_eq!(parse_value::<f64>("-0.25", -10.0, 10.0), Some(-0.25));
        assert_eq!(parse_value::<f64>("−0.25", -10.0, 10.0), Some(-0.25));
        assert_eq!(
            parse_value::<f64>("1\u{202f}000", 0.0, 10000.0),
            Some(1000.0)
        );
        // Out-of-range input is rejected, not clamped.
        assert_eq!(parse_value::<f64>("25", -10.0, 10.0), None);
        assert_eq!(parse_value::<f64>("5", -10.0, 10.0), Some(5.0));
        assert_eq!(parse_value::<u32>("2.5", 1.0, 10.0), None);
        assert_eq!(parse_value::<u32>("0", 1.0, 10.0), None);
        assert_eq!(parse_value::<u32>("5", 1.0, 10.0), Some(5.0));
    }

    #[test]
    fn typing_uses_text_cursor_and_preserves_incomplete_edits() {
        let ctx = egui::Context::default();
        let mut value = 1.0;
        let mut field_rect = egui::Rect::NOTHING;
        let mut frame = |events| {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(300.0, 200.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let id = ui.make_persistent_id(("numeric-field", "exposure"));
                        if ctx.input(|input| {
                            input.events.iter().any(|event| {
                                matches!(event, egui::Event::Text(_) | egui::Event::Key { .. })
                            })
                        }) {
                            ctx.memory_mut(|memory| memory.request_focus(id));
                        }
                        numeric_field(
                            ui, "exposure", "exposure", &mut value, -10.0, 10.0, 0.25, 2,
                            "Exposure", None, None,
                        );
                        field_rect = ctx.read_response(id).unwrap().rect;
                    });
                },
            );
            (output.platform_output.cursor_icon, value, field_rect)
        };
        let (_, _, rect) = frame(vec![]);
        let position = rect.center();
        let (cursor, _, _) = frame(vec![egui::Event::PointerMoved(position)]);
        assert_eq!(cursor, egui::CursorIcon::Text);
        frame(vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        frame(vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        };
        let (_, unchanged, _) = frame(vec![select_all.clone(), egui::Event::Text("-".into())]);
        assert_eq!(unchanged, 1.0);
        let (_, typed, _) = frame(vec![egui::Event::Text("0.25".into())]);
        assert_eq!(typed, -0.25);
        let (_, unchanged, _) = frame(vec![select_all, egui::Event::Text("NaN".into())]);
        assert_eq!(unchanged, -0.25);
    }

    #[test]
    fn mouse_wheel_event_changes_hovered_text_field() {
        let ctx = egui::Context::default();
        let mut value = 1.0_f64;
        let field_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 200.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    numeric_field(
                        ui, "exposure", "exposure", &mut value, -10.0, 10.0, 0.25, 2, "Exposure",
                        None, None,
                    );
                    let id = ui.make_persistent_id(("numeric-field", "exposure"));
                    field_rect.set(ctx.read_response(id).unwrap().rect);
                });
            },
        );
        let position = field_rect.get().center();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 200.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta: egui::vec2(0.0, 1.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    numeric_field(
                        ui, "exposure", "exposure", &mut value, -10.0, 10.0, 0.25, 2, "Exposure",
                        None, None,
                    );
                });
            },
        );
        assert_eq!(value, 1.25);
    }

    fn run_numeric_frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        value: &mut f64,
        changed: &mut bool,
        rect: &std::cell::Cell<egui::Rect>,
    ) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 200.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let id = ui.make_persistent_id(("numeric-field", "exposure"));
                    if ctx.input(|input| {
                        input.events.iter().any(|event| {
                            matches!(event, egui::Event::Text(_) | egui::Event::Key { .. })
                        })
                    }) {
                        ctx.memory_mut(|memory| memory.request_focus(id));
                    }
                    *changed = numeric_field(
                        ui, "exposure", "exposure", value, 0.0, 1.0, 0.05, 3, "tip", None, None,
                    );
                    rect.set(ctx.read_response(id).unwrap().rect);
                });
            },
        );
    }

    fn focus_events(position: egui::Pos2) -> Vec<Vec<egui::Event>> {
        vec![
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        ]
    }

    fn enter_events() -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    fn replace_events(text: &str) -> Vec<egui::Event> {
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            },
            egui::Event::Text(text.into()),
        ]
    }

    #[test]
    fn zero_text_edit_commits_on_focus_loss() {
        let ctx = egui::Context::default();
        let mut value = 0.5;
        let mut changed = false;
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        for events in focus_events(rect.get().center()) {
            run_numeric_frame(&ctx, events, &mut value, &mut changed, &rect);
        }
        run_numeric_frame(&ctx, replace_events("0"), &mut value, &mut changed, &rect);
        assert!(!changed);
        assert_eq!(value, 0.5);
        run_numeric_frame(&ctx, enter_events(), &mut value, &mut changed, &rect);
        assert!(changed);
        assert_eq!(value, 0.0);
    }

    #[test]
    fn invalid_draft_rollback_reports_change() {
        let ctx = egui::Context::default();
        let mut value = 0.5;
        let mut changed = false;
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        for events in focus_events(rect.get().center()) {
            run_numeric_frame(&ctx, events, &mut value, &mut changed, &rect);
        }
        run_numeric_frame(
            &ctx,
            replace_events("0.25"),
            &mut value,
            &mut changed,
            &rect,
        );
        assert!(changed);
        assert_eq!(value, 0.25);
        run_numeric_frame(&ctx, replace_events("abc"), &mut value, &mut changed, &rect);
        assert!(!changed);
        assert_eq!(value, 0.25);
        run_numeric_frame(&ctx, enter_events(), &mut value, &mut changed, &rect);
        assert!(changed);
        assert_eq!(value, 0.5);
    }

    #[test]
    fn rollback_skips_invalid_edit_start_after_pair_shift() {
        // Reviewer scenario: the paired white point moves while the black
        // field keeps focus, so the focus-start value no longer validates.
        let ctx = egui::Context::default();
        let other = std::cell::Cell::new(0.8);
        let mut value = 0.7;
        let mut changed = false;
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let run = |events: Vec<egui::Event>,
                   value: &mut f64,
                   changed: &mut bool,
                   other: &std::cell::Cell<f64>,
                   rect: &std::cell::Cell<egui::Rect>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(300.0, 200.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let id = ui.make_persistent_id(("numeric-field", "black"));
                        if ctx.input(|input| {
                            input.events.iter().any(|event| {
                                matches!(event, egui::Event::Text(_) | egui::Event::Key { .. })
                            })
                        }) {
                            ctx.memory_mut(|memory| memory.request_focus(id));
                        }
                        *changed = super::numeric_field_validated(
                            ui,
                            "black",
                            "black",
                            value,
                            0.0,
                            1.0,
                            0.05,
                            3,
                            "tip",
                            None,
                            None,
                            |next| next < other.get(),
                        );
                        rect.set(ctx.read_response(id).unwrap().rect);
                    });
                },
            );
        };
        run(vec![], &mut value, &mut changed, &other, &rect);
        for events in focus_events(rect.get().center()) {
            run(events, &mut value, &mut changed, &other, &rect);
        }
        run(
            replace_events("0.1"),
            &mut value,
            &mut changed,
            &other,
            &rect,
        );
        assert!(changed);
        assert_eq!(value, 0.1);
        // The paired field moves below the focus-start value without taking focus.
        other.set(0.5);
        run(
            replace_events("0.6"),
            &mut value,
            &mut changed,
            &other,
            &rect,
        );
        assert!(!changed);
        assert_eq!(value, 0.1);
        run(enter_events(), &mut value, &mut changed, &other, &rect);
        assert!(!changed);
        assert_eq!(value, 0.1);
    }
}
