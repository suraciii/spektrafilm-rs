use std::hash::Hash;

use egui::{TextEdit, Ui};

#[derive(Clone, Debug)]
struct NumericEditState {
    text: String,
    last_value: f64,
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
        ui.label(label).on_hover_text(tooltip);
        numeric_field(ui, label, value, min, max, step, decimals, tooltip)
    })
    .inner
}

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
    let decimals = if T::INTEGRAL { 0 } else { decimals };
    let current = value.to_f64();
    let id = ui.make_persistent_id(("numeric-field", id_salt));
    let mut state = ui
        .ctx()
        .data(|data| data.get_temp::<NumericEditState>(id))
        .unwrap_or_else(|| NumericEditState {
            text: format_value(current, T::INTEGRAL, decimals),
            last_value: current,
            focused: false,
        });

    if state.last_value.to_bits() != current.to_bits()
        || (state.focused && !ui.memory(|memory| memory.has_focus(id)))
    {
        state.text = format_value(current, T::INTEGRAL, decimals);
    }

    let response = ui
        .add(
            TextEdit::singleline(&mut state.text)
                .id(id)
                .desired_width(72.0),
        )
        .on_hover_text(tooltip);

    let mut changed = false;
    if response.changed() {
        if let Some(parsed) = parse_value::<T>(&state.text, min, max) {
            let next = T::from_f64(parsed);
            if *value != next {
                *value = next;
                changed = true;
            }
        }
    }

    let wheel_changed = wheel_adjust(ui, &response, value, min, max, step);
    if wheel_changed {
        state.text = format_value(value.to_f64(), T::INTEGRAL, decimals);
        changed = true;
    }

    if response.lost_focus() {
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
    let (raw_delta, smooth_delta) =
        ui.input(|input| (input.raw_scroll_delta.y, input.smooth_scroll_delta.y));
    if raw_delta == 0.0 {
        if smooth_delta != 0.0 {
            ui.input_mut(|input| input.smooth_scroll_delta.y = 0.0);
        }
        return false;
    }
    if apply_wheel_delta(value, raw_delta, min, max, step) {
        ui.input_mut(|input| {
            input.raw_scroll_delta.y = 0.0;
            input.smooth_scroll_delta.y = 0.0;
        });
        true
    } else {
        false
    }
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
}
