use std::hash::Hash;

use egui::{TextEdit, Ui};

#[derive(Clone, Debug)]
struct NumericEditState {
    text: String,
    last_value: f64,
    edit_start: f64,
    focused: bool,
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
    Some(parsed.clamp(min, max))
}

// Numeric widgets keep their existing parameter interfaces.
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
) -> bool {
    ui.horizontal(|ui| {
        let label_response = ui.label(label).on_hover_text(tooltip);
        let changed = numeric_field(ui, label, value, min, max, step, decimals, tooltip);
        let id = ui.make_persistent_id(("numeric-field", label));
        if let Some(response) = ui.ctx().read_response(id) {
            response.labelled_by(label_response.id);
        }
        changed
    })
    .inner
}

// Numeric widgets keep their existing parameter interfaces.
#[allow(clippy::too_many_arguments)]
pub(crate) fn numeric_field<T, Id>(
    ui: &mut Ui,
    id_salt: Id,
    value: &mut T,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
) -> bool
where
    T: egui::emath::Numeric,
    Id: Hash,
{
    numeric_field_validated(
        ui,
        id_salt,
        value,
        min,
        max,
        step,
        decimals,
        tooltip,
        |_| true,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn numeric_field_validated<T, Id>(
    ui: &mut Ui,
    id_salt: Id,
    value: &mut T,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
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

    let mut text_edit = TextEdit::singleline(&mut state.text)
        .id(id)
        .desired_width(72.0);
    if is_invalid {
        text_edit = text_edit.text_color(egui::Color32::from_rgb(0xe0, 0x55, 0x55));
    }
    let response = ui.add(text_edit).on_hover_text(tooltip);
    if is_invalid {
        ui.painter().rect_stroke(
            response.rect.expand(1.5),
            0.0_f32,
            egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(0xe0, 0x55, 0x55)),
            egui::StrokeKind::Inside,
        );
    }
    if response.gained_focus() {
        state.edit_start = current;
    }

    let mut changed = false;
    if response.changed()
        && let Some(parsed) = parse_value::<T>(&state.text, min, max)
    {
        let next = T::from_f64(parsed);
        if valid(next)
            && (T::INTEGRAL || state.text.trim() != "0" || response.lost_focus())
            && *value != next
        {
            *value = next;
            changed = true;
        }
    }
    let mut wheel_value = *value;
    let wheel_changed =
        wheel_adjust(ui, &response, &mut wheel_value, min, max, step) && valid(wheel_value);
    if wheel_changed {
        *value = wheel_value;
    }
    if wheel_changed {
        state.text = format_value(value.to_f64(), T::INTEGRAL, decimals);
        changed = true;
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
            changed = true;
        }
        state.text = format_value(value.to_f64(), T::INTEGRAL, decimals);
    }
    state.last_value = value.to_f64();
    state.focused = response.has_focus();
    ui.ctx().data_mut(|data| data.insert_temp(id, state));

    changed
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
    use super::{apply_wheel_delta, numeric_field, parse_value};

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
        assert_eq!(parse_value::<f64>("25", -10.0, 10.0), Some(10.0));
        assert_eq!(parse_value::<u32>("2.5", 1.0, 10.0), None);
        assert_eq!(parse_value::<u32>("0", 1.0, 10.0), Some(1.0));
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
                        numeric_field(ui, "exposure", &mut value, -10.0, 10.0, 0.25, 2, "Exposure");
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
    fn wheel_delta_changes_by_step() {
        let mut value = 1.0_f64;
        assert!(apply_wheel_delta(&mut value, 1.0, 0.0, 2.0, 0.25));
        assert_eq!(value, 1.25);
    }

    #[test]
    fn wheel_delta_preserves_bounds() {
        let mut min = 0.0_f64;
        assert!(!apply_wheel_delta(&mut min, -1.0, 0.0, 2.0, 0.25));
        assert_eq!(min, 0.0);
        let mut max = 2.0_f64;
        assert!(!apply_wheel_delta(&mut max, 1.0, 0.0, 2.0, 0.25));
        assert_eq!(max, 2.0);
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
                    numeric_field(ui, "exposure", &mut value, -10.0, 10.0, 0.25, 2, "Exposure");
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
                    numeric_field(ui, "exposure", &mut value, -10.0, 10.0, 0.25, 2, "Exposure");
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
                    *changed = numeric_field(ui, "exposure", value, 0.0, 1.0, 0.05, 3, "tip");
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
                            value,
                            0.0,
                            1.0,
                            0.05,
                            3,
                            "tip",
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
