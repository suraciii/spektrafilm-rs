use std::collections::VecDeque;
use std::hash::Hash;

use egui::{TextEdit, Ui};
use serde_json::Value;

use crate::theme;

/// Numeric interaction edges collected during a frame for adjustment history.
///
/// `active` means an edit is still in progress, not that a value changed. A
/// wheel step (or a keyboard slider step) starts and completes in one frame.
/// Cancellation takes precedence over completion and restores no history entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AdjustmentSignals {
    pub started: bool,
    pub resumed: bool,
    pub active: bool,
    pub completed: bool,
    pub canceled: bool,
    pub label: Option<String>,
}

#[derive(Clone, Default)]
struct FrameAdjustmentSignals {
    pass: u64,
    signals: AdjustmentSignals,
}

/// Consume the numeric interaction signals after all controls have been drawn.
/// The active text owner has a separate lifetime: draining an empty frame must
/// not end a draft before its widget processes completion or cancellation.
pub(crate) fn take_adjustment_signals(ctx: &egui::Context) -> AdjustmentSignals {
    // `Context::run` advances the pass counter before callers can collect its
    // result; permit that just-completed pass as well as collection in-frame.
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        data.remove_temp::<FrameAdjustmentSignals>(egui::Id::new("numeric-adjustment-signals"))
            .filter(|frame| pass.saturating_sub(frame.pass) <= 1)
            .map(|frame| frame.signals)
            .unwrap_or_default()
    })
}

fn signal_adjustment(ctx: &egui::Context, label: &str, signals: AdjustmentSignals) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        let id = egui::Id::new("numeric-adjustment-signals");
        let mut frame = data
            .get_temp::<FrameAdjustmentSignals>(id)
            .filter(|frame| frame.pass == pass)
            .map(|frame| frame.signals)
            .unwrap_or_default();
        frame.started |= signals.started;
        frame.resumed |= signals.resumed;
        frame.active |= signals.active;
        frame.completed |= signals.completed;
        frame.canceled |= signals.canceled;
        if frame.label.is_none() {
            frame.label = Some(label.to_owned());
        }
        data.insert_temp(
            id,
            FrameAdjustmentSignals {
                pass,
                signals: frame,
            },
        );
    });
}

#[cfg(test)]
pub(crate) fn signal_for_test(ctx: &egui::Context, label: &str, signals: AdjustmentSignals) {
    signal_adjustment(ctx, label, signals);
}

fn adjustment_ended(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| {
        data.get_temp::<FrameAdjustmentSignals>(egui::Id::new("numeric-adjustment-signals"))
            .is_some_and(|frame| {
                frame.pass == pass && (frame.signals.completed || frame.signals.canceled)
            })
    })
}

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
    editing: bool,
    suspended_for_language: bool,
    resumed_after_language: bool,
    wheel_events: VecDeque<f32>,
}

#[derive(Clone, Debug, Default)]
struct SliderEditState {
    start: Option<f64>,
    canceled: bool,
    deferred: Option<f64>,
    suspended: bool,
}

#[derive(Clone, Default)]
struct NumericRegistry {
    text: Vec<egui::Id>,
    sliders: Vec<(egui::Id, egui::Id)>,
}

/// Photograph-local pending editors, separate from accepted adjustment state.
/// Moving a photograph out of the live GUI removes its draft owners, queued
/// wheel input, focus and slider drag from the shared egui context.
#[derive(Default)]
pub(crate) struct NumericDrafts {
    text: Vec<(egui::Id, NumericEditState)>,
    editors: Vec<(egui::Id, egui::text_edit::TextEditState)>,
    sliders: Vec<(egui::Id, egui::Id, SliderEditState)>,
    suspended: Option<egui::Id>,
}

impl NumericDrafts {
    pub(crate) fn take(ctx: &egui::Context) -> Self {
        let (drafts, focused, dragging) = ctx.data_mut(|data| {
            let registry = data
                .remove_temp::<NumericRegistry>(egui::Id::new("numeric-registry"))
                .unwrap_or_default();
            let active = data.get_temp::<egui::Id>(egui::Id::new("numeric-active-text"));
            data.remove::<egui::Id>(egui::Id::new("numeric-active-text"));
            let mut suspended = data.get_temp::<egui::Id>(egui::Id::new("numeric-suspended-text"));
            data.remove::<egui::Id>(egui::Id::new("numeric-suspended-text"));
            data.remove::<FrameAdjustmentSignals>(egui::Id::new("numeric-adjustment-signals"));
            let mut text = Vec::new();
            let mut editors = Vec::new();
            for id in &registry.text {
                let editor = data.get_persisted::<egui::text_edit::TextEditState>(*id);
                data.remove::<egui::text_edit::TextEditState>(*id);
                let state = data.get_temp::<NumericEditState>(*id);
                data.remove::<NumericEditState>(*id);
                if let Some(mut state) = state {
                    state.wheel_events.clear();
                    state.focused = false;
                    if state.editing {
                        state.suspended_for_language = true;
                        if !state.resumed_after_language
                            && (active == Some(*id) || suspended.is_none())
                        {
                            suspended = Some(*id);
                        }
                        text.push((*id, state));
                        if let Some(editor) = editor {
                            editors.push((*id, editor));
                        }
                    }
                }
            }
            let mut sliders = Vec::new();
            for &(widget, state_id) in &registry.sliders {
                if let Some(mut state) = data.remove_temp::<SliderEditState>(state_id)
                    && (state.start.is_some() || state.deferred.is_some())
                {
                    state.suspended = true;
                    sliders.push((widget, state_id, state));
                }
            }
            (
                Self {
                    text,
                    editors,
                    sliders,
                    suspended,
                },
                registry.text,
                registry.sliders,
            )
        });
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused()
                && (focused.contains(&id) || dragging.iter().any(|(widget, _)| *widget == id))
            {
                memory.surrender_focus(id);
            }
        });
        if ctx
            .dragged_id()
            .is_some_and(|id| dragging.iter().any(|(widget, _)| *widget == id))
        {
            ctx.stop_dragging();
        }
        drafts
    }

    /// Restore as suspended, not focused: merely reopening a photograph must
    /// neither commit its text nor turn the current pointer into its old drag.
    pub(crate) fn restore(self, ctx: &egui::Context) {
        let _ = Self::take(ctx);
        ctx.data_mut(|data| {
            let mut registry = NumericRegistry::default();
            for (id, state) in self.text {
                registry.text.push(id);
                data.insert_temp(id, state);
            }
            for (id, editor) in self.editors {
                data.insert_persisted(id, editor);
            }
            for (widget, id, state) in self.sliders {
                registry.sliders.push((widget, id));
                data.insert_temp(id, state);
            }
            data.insert_temp(egui::Id::new("numeric-registry"), registry);
            if let Some(id) = self.suspended {
                data.insert_temp(egui::Id::new("numeric-suspended-text"), id);
            }
        });
    }
}

fn register_numeric_text(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|data| {
        let registry =
            data.get_temp_mut_or_default::<NumericRegistry>(egui::Id::new("numeric-registry"));
        if !registry.text.contains(&id) {
            registry.text.push(id);
        }
    });
}

fn register_numeric_slider(ctx: &egui::Context, widget: egui::Id, state: egui::Id) {
    ctx.data_mut(|data| {
        let registry =
            data.get_temp_mut_or_default::<NumericRegistry>(egui::Id::new("numeric-registry"));
        if !registry.sliders.iter().any(|(_, id)| *id == state) {
            registry.sliders.push((widget, state));
        }
    });
}

/// Record the language control's stable focus/pointer target. The inspector
/// renders before Settings, so pending numeric edits inspect the previous
/// frame's target before the language selector receives a click.
pub(crate) fn register_language_selector(ctx: &egui::Context, response: &egui::Response) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        *data.get_temp_mut_or_insert_with(egui::Id::new("numeric-language-selector"), || {
            (response.id, response.rect, pass)
        }) = (response.id, response.rect, pass);
    });
}

/// Opening the language settings is presentation navigation, not completion of
/// a photographic edit. Keep its pending draft until its editor is resumed.
pub(crate) fn suspend_numeric_draft(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        if let Some(id) = data.get_temp::<egui::Id>(egui::Id::new("numeric-active-text"))
            && let Some(mut state) = data.get_temp::<NumericEditState>(id)
        {
            state.suspended_for_language = true;
            data.insert_temp(egui::Id::new("numeric-suspended-text"), id);
            state.wheel_events.clear();
            if data.get_temp::<egui::Id>(egui::Id::new("numeric-active-text")) == Some(id) {
                data.remove::<egui::Id>(egui::Id::new("numeric-active-text"));
            }
            data.insert_temp(id, state);
        }
    });
}

/// A pending presentation-suspended draft can outlive the editor's visibility.
pub(crate) fn has_suspended_adjustment(ctx: &egui::Context) -> bool {
    ctx.data(|data| {
        data.get_temp::<egui::Id>(egui::Id::new("numeric-suspended-text"))
            .is_some()
    })
}

/// History closes the suspended gesture before accepting an unrelated edit.
/// Its draft remains available, but later cancellation restores only its field.
pub(crate) fn detach_suspended_adjustment(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        let owner_id = egui::Id::new("numeric-suspended-text");
        let owner = data.get_temp::<egui::Id>(owner_id);
        data.remove::<egui::Id>(owner_id);
        if let Some(id) = owner
            && let Some(mut state) = data.get_temp::<NumericEditState>(id)
        {
            state.resumed_after_language = true;
            data.insert_temp(id, state);
        }
    });
}

fn language_selector_interaction(ctx: &egui::Context) -> bool {
    let Some((id, rect, pass)) = ctx.data(|data| {
        data.get_temp::<(egui::Id, egui::Rect, u64)>(egui::Id::new("numeric-language-selector"))
    }) else {
        return false;
    };
    if ctx.cumulative_pass_nr().saturating_sub(pass) > 1 {
        return false;
    }
    ctx.memory(|memory| memory.focused() == Some(id))
        || ctx.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|position| rect.contains(position))
        })
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
        let editor_width = theme::WIDTH_VALUE + theme::WIDTH_UNIT + ui.spacing().item_spacing.x;
        let label_width =
            (ui.available_width() - editor_width - ui.spacing().item_spacing.x).max(40.0);
        let label_response = ui
            .allocate_ui_with_layout(
                egui::vec2(label_width, theme::HEIGHT_CONTROL),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(label_width);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    crate::controls::row_label(ui, label, experimental)
                },
            )
            .inner;
        label_response
            .clone()
            .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
        mark.paint(ui, &label_response);
        changed |= ui
            .allocate_ui_with_layout(
                egui::vec2(editor_width, theme::HEIGHT_CONTROL),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(editor_width);
                    if unit.is_none() {
                        ui.add_space(theme::WIDTH_UNIT + ui.spacing().item_spacing.x);
                    }
                    mark.gold(ui, |ui| {
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
                    })
                },
            )
            .inner;
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
    let before = *value;
    let mut candidate = before;
    let response = ui
        .push_id(
            ("numeric-slider", label, min.to_bits(), max.to_bits()),
            |ui| {
                // Slider::new otherwise retains egui's 100-point default even
                // in a wider inspector. Its own input/history owner is unchanged.
                ui.spacing_mut().slider_width = ui.available_width();
                ui.spacing_mut().slider_rail_height = 3.0;
                ui.spacing_mut().interact_size.y = 16.0;
                let visuals = &mut ui.visuals_mut().widgets;
                visuals.inactive.fg_stroke = theme::stroke(theme::FG);
                visuals.hovered.bg_fill = theme::FG;
                visuals.active.bg_fill = theme::FG;
                let mut slider =
                    egui::Slider::new(&mut candidate, T::from_f64(track_min)..=T::from_f64(max))
                        .show_value(false)
                        .clamping(egui::SliderClamping::Never);
                if logarithmic {
                    slider = slider
                        .logarithmic(true)
                        .smallest_positive(step.max(f64::MIN_POSITIVE));
                }
                ui.add(slider).on_hover_text(tooltip_for(ui.ctx(), label))
            },
        )
        .inner;
    // The slider's track and handle carry no toolkit frame, so its keyboard
    // focus comes from the one shared tokenized ring, the same cue framed
    // controls get from the theme's active stroke.
    crate::controls::focus_feedback(ui, &response);
    let state_id = response.id.with("adjustment");
    register_numeric_slider(ui.ctx(), response.id, state_id);
    let mut state = ui
        .ctx()
        .data(|data| data.get_temp::<SliderEditState>(state_id))
        .unwrap_or_default();
    let pointer_active = response.is_pointer_button_down_on() || response.dragged();
    if state.suspended
        && !pointer_active
        && !response.changed()
        && !ui.input(|input| input.key_pressed(egui::Key::Escape))
    {
        signal_adjustment(
            ui.ctx(),
            label,
            AdjustmentSignals {
                active: true,
                ..Default::default()
            },
        );
        return false;
    }
    state.suspended = false;
    if state.start.is_none()
        && (adjustment_ended(ui.ctx())
            || ui.ctx().data(|data| {
                data.get_temp::<egui::Id>(egui::Id::new("numeric-active-text"))
                    .is_some()
            }))
        && (pointer_active || response.changed())
    {
        // Let history close the preceding text edit before the slider's
        // first mutation. Retain even a press-and-release in this frame.
        state.deferred = Some(candidate.to_f64());
        ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
        ui.ctx().request_repaint();
        return false;
    }
    let mut signals = AdjustmentSignals::default();
    if state.start.is_none()
        && (pointer_active
            || response.drag_started()
            || response.drag_stopped()
            || state.deferred.is_some())
    {
        state.start = Some(before.to_f64());
        signals.started = true;
        if let Some(deferred) = state.deferred.take()
            && !pointer_active
        {
            candidate = T::from_f64(deferred);
        }
    }
    if let Some(start) = state.start {
        if !state.canceled && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            candidate = T::from_f64(start);
            state.canceled = true;
            signals.canceled = true;
            response.surrender_focus();
        } else if state.canceled {
            // egui keeps moving its slider until release; ignore those values
            // after Escape instead of opening another history transaction.
            candidate = before;
        }
        signals.active = pointer_active && !state.canceled;
        if !pointer_active {
            signals.completed = !state.canceled;
            state = SliderEditState::default();
        }
    } else if response.changed() {
        // Keyboard and accessibility steps are discrete accepted edits.
        signals.started = true;
        signals.completed = true;
    }
    *value = candidate;
    ui.ctx().data_mut(|data| data.insert_temp(state_id, state));
    if signals != AdjustmentSignals::default() {
        signal_adjustment(ui.ctx(), label, signals);
    }
    *value != before
}

/// Tooltip text for a row slider: the value field carries the parameter's own
/// tooltip, so the slider names the interaction instead.
fn tooltip_for(ctx: &egui::Context, label: &str) -> String {
    crate::tr!(
        ctx,
        "controls.numeric.drag",
        label = crate::controls::localized(ctx, label)
    )
    .into_owned()
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
    register_numeric_text(ui.ctx(), id);
    let mut state = ui
        .ctx()
        .data(|data| data.get_temp::<NumericEditState>(id))
        .unwrap_or_else(|| NumericEditState {
            text: format_value(current, T::INTEGRAL, decimals),
            last_value: current,
            edit_start: current,
            focused: false,
            editing: false,
            suspended_for_language: false,
            resumed_after_language: false,
            wheel_events: VecDeque::new(),
        });

    if !state.editing && state.last_value.to_bits() != current.to_bits() {
        state.text = format_value(current, T::INTEGRAL, decimals);
        state.edit_start = current;
    }

    // Determine if the current text is invalid for visual feedback.
    let parsed_for_validation = parse_value::<T>(&state.text, min, max);
    let is_invalid = match parsed_for_validation {
        None => true,
        Some(v) => !valid(T::from_f64(v)),
    };

    let mut changed = false;
    ui.horizontal(|ui| {
        if let Some(unit) = unit {
            ui.add_sized(
                [theme::WIDTH_UNIT, theme::HEIGHT_CONTROL],
                egui::Label::new(
                    egui::RichText::new(crate::controls::localized(ui.ctx(), unit))
                        .color(theme::FG_MUTED)
                        .size(theme::SIZE_UNIT),
                )
                .wrap(),
            )
            .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
        }
        let mut text_edit = TextEdit::singleline(&mut state.text)
            .id(id)
            .font(theme::text(theme::TEXT_VALUE, theme::SIZE_VALUE))
            .horizontal_align(egui::Align::Max)
            .desired_width(theme::WIDTH_VALUE);
        if is_invalid {
            // Invalid state is colour plus a border, never colour alone.
            text_edit = text_edit.text_color(theme::ERROR);
        }
        let mut response = ui
            .add_sized([theme::WIDTH_VALUE, theme::HEIGHT_CONTROL], text_edit)
            .on_hover_text(crate::controls::localized(ui.ctx(), tooltip));
        crate::controls::focus_response(ui, label, &response);
        // Name the editor itself, so assistive technology and the
        // accessibility tree's label lookup both find the field rather than a
        // separate label node, and declare the visible label as the field's
        // labelling node so the two read as one control.
        let accessible_label = crate::controls::localized(ui.ctx(), label);
        let accessible_description = unit.map(|unit| {
            format!(
                "{}; {}",
                crate::controls::localized(ui.ctx(), tooltip),
                crate::controls::localized(ui.ctx(), unit),
            )
        });
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_label(accessible_label.as_ref());
            if let Some(description) = accessible_description {
                node.set_description(description);
            }
        });
        if let Some(label_id) = labelled_by {
            response = response.labelled_by(label_id);
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
            label,
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
    label: &str,
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
    let mut signals = AdjustmentSignals::default();
    if state.editing && language_selector_interaction(ui.ctx()) {
        state.suspended_for_language = true;
        state.wheel_events.clear();
    }
    if response.gained_focus() || (!state.focused && response.has_focus()) {
        if state.suspended_for_language {
            signals.resumed = state.editing && !state.resumed_after_language;
            signals.started = state.editing && state.resumed_after_language;
        } else {
            state.edit_start = value.to_f64();
        }
        state.suspended_for_language = false;
    }
    let escape = !state.suspended_for_language
        && (state.focused || response.has_focus() || response.gained_focus())
        && ui.input(|input| input.key_pressed(egui::Key::Escape));
    if escape {
        let start = T::from_f64(state.edit_start);
        if state.editing && valid(start) && *value != start {
            *value = start;
            *changed = true;
        }
        // Once an unrelated edit detached this draft, cancellation restores only
        // its field rather than rolling back the newer accepted document.
        signals.canceled = state.editing && !state.resumed_after_language;
        signals.completed = state.editing && state.resumed_after_language;
        state.resumed_after_language = false;
        state.editing = false;
        state.text = format_value(value.to_f64(), T::INTEGRAL, decimals);
        response.surrender_focus();
    } else {
        if response.changed() && !state.suspended_for_language {
            if !state.editing {
                state.editing = true;
                signals.started = true;
            }
            if let Some(parsed) = parse_value::<T>(&state.text, min, max) {
                let next = T::from_f64(parsed);
                if valid(next)
                    && (T::INTEGRAL || state.text.trim() != "0" || response.lost_focus())
                    && *value != next
                {
                    *value = next;
                    *changed = true;
                }
            }
        }

        if !state.suspended_for_language {
            collect_wheel_events(ui, response, &mut state.wheel_events);
        }
        let finish_for_wheel = state.editing && !state.wheel_events.is_empty();
        if !state.editing
            && !adjustment_ended(ui.ctx())
            && ui.ctx().data(|data| {
                data.get_temp::<egui::Id>(egui::Id::new("numeric-active-text"))
                    .is_none_or(|active| active == id)
            })
            && let Some(delta) = state.wheel_events.pop_front()
        {
            let mut wheel_value = *value;
            if apply_wheel_delta(&mut wheel_value, delta, min, max, step) && valid(wheel_value) {
                *value = wheel_value;
                state.text = format_value(value.to_f64(), T::INTEGRAL, decimals);
                *changed = true;
                signals.started = true;
                signals.completed = true;
                state.edit_start = value.to_f64();
            }
        }

        if state.editing
            && !state.suspended_for_language
            && (response.lost_focus()
                || (state.focused && !response.has_focus())
                || finish_for_wheel)
        {
            let final_value = parse_value::<T>(&state.text, min, max).map(T::from_f64);
            let target = match final_value {
                // Commit a valid draft deferred while focused (e.g. a bare "0").
                Some(next) if valid(next) => Some(next),
                // Paired controls may have made the focus-start value invalid.
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
            if state.editing {
                if final_value.is_some_and(&valid) {
                    signals.completed = true;
                } else {
                    signals.canceled = !state.resumed_after_language;
                    signals.completed = state.resumed_after_language;
                }
            }
            state.resumed_after_language = false;
            state.editing = false;
            state.text = format_value(value.to_f64(), T::INTEGRAL, decimals);
            state.edit_start = value.to_f64();
        }
    }
    if !state.suspended_for_language && !state.wheel_events.is_empty() {
        ui.ctx().request_repaint();
    }
    signals.active = state.editing && !state.suspended_for_language;
    if signals != AdjustmentSignals::default() {
        signal_adjustment(ui.ctx(), label, signals);
    }
    state.last_value = value.to_f64();
    state.focused = response.has_focus() && !escape;
    ui.ctx().data_mut(|data| {
        let active_id = egui::Id::new("numeric-active-text");
        if state.editing && !state.suspended_for_language {
            data.insert_temp(active_id, id);
        } else if data.get_temp::<egui::Id>(active_id) == Some(id) {
            data.remove::<egui::Id>(active_id);
        }
        let suspended_id = egui::Id::new("numeric-suspended-text");
        if state.editing && state.suspended_for_language && !state.resumed_after_language {
            if data.get_temp::<egui::Id>(suspended_id) != Some(id) {
                data.insert_temp(suspended_id, id);
            }
        } else if data.get_temp::<egui::Id>(suspended_id) == Some(id) {
            data.remove::<egui::Id>(suspended_id);
        }
        data.insert_temp(id, state.clone());
    });
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

fn collect_wheel_events(ui: &Ui, response: &egui::Response, pending: &mut VecDeque<f32>) {
    if !response.hovered() {
        return;
    }
    // Keep original events separate, including several events delivered in
    // one frame. Process one per frame so each has its own history boundary.
    // Never use egui's smooth scrolling residue as a new physical event.
    ui.input_mut(|input| {
        for event in &mut input.events {
            if let egui::Event::MouseWheel { delta, .. } = event
                && delta.y != 0.0
            {
                pending.push_back(delta.y);
                delta.y = 0.0;
            }
        }
        input.raw_scroll_delta.y = 0.0;
        input.smooth_scroll_delta.y = 0.0;
    });
}

#[cfg(test)]
mod tests {
    use super::{SliderScale, numeric_field, parse_value, slider_scale};

    #[test]
    fn rows_reserve_units_before_right_aligned_values_without_hiding_long_text() {
        use egui_kittest::{Harness, kittest::Queryable};

        const ROWS: [(&str, Option<&str>); 3] = [
            (
                "A long parameter label that must remain readable",
                Some("μm"),
            ),
            ("Preflash amount", Some("linear exposure multiplier")),
            ("Unitless parameter", None),
        ];
        for width in [280.0, 344.0] {
            let harness = Harness::builder()
                .with_size(egui::vec2(width, 600.0))
                .build_state(
                    |ctx, values: &mut [f64; 3]| {
                        crate::theme::apply(ctx);
                        egui::CentralPanel::default().show(ctx, |ui| {
                            for ((label, unit), value) in ROWS.iter().zip(values.iter_mut()) {
                                super::numeric(
                                    ui,
                                    label,
                                    value,
                                    0.0,
                                    1.0,
                                    0.01,
                                    2,
                                    "Parameter tooltip",
                                    *unit,
                                    false,
                                );
                            }
                        });
                    },
                    [0.5; 3],
                );
            let mut value_right: Option<f64> = None;
            for (label, unit) in ROWS {
                let node = harness.get_by_label(label);
                let value = node.raw_bounds().expect("numeric editor bounds");
                let label = node.labelled_by().next().expect("visible row label");
                let label = label.raw_bounds().expect("visible label bounds");
                assert!(label.x1 <= value.x0);
                assert!(label.y0 < value.y1 && value.y0 < label.y1);
                assert!(value.x1 <= f64::from(width));
                if let Some(right) = value_right {
                    assert!((value.x1 - right).abs() < 1.0, "value columns must align");
                }
                value_right = Some(value.x1);
                if let Some(unit) = unit {
                    let unit_bounds = harness
                        .get_by_label(unit)
                        .raw_bounds()
                        .expect("visible unit bounds");
                    assert!(label.x1 <= unit_bounds.x0 && unit_bounds.x1 <= value.x0);
                    assert!(node.description().is_some_and(|text| text.contains(unit)));
                }
            }
            assert_eq!(harness.state(), &[0.5; 3]);
        }
    }

    #[test]
    fn slider_uses_the_full_row_width_with_a_thin_track_and_small_handle() {
        for width in [280.0, 344.0] {
            let ctx = egui::Context::default();
            crate::theme::apply(&ctx);
            let mut value = 0.5_f64;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 200.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let available = ui.available_width();
                        let row = ui.scope(|ui| {
                            super::row_slider(ui, "Exposure", &mut value, 0.0, 1.0, 0.01)
                        });
                        assert!(!row.inner);
                        assert!((row.response.rect.width() - available).abs() < 1.0);
                    });
                },
            );
            let mut handles = 0;
            let mut tracks = 0;
            for shape in output.shapes {
                match shape.shape {
                    egui::Shape::Circle(circle) => {
                        handles += 1;
                        assert!(circle.radius <= 7.0);
                    }
                    egui::Shape::Rect(rect) if (rect.rect.height() - 3.0).abs() < 0.1 => {
                        tracks += 1;
                    }
                    _ => {}
                }
            }
            assert_eq!(handles, 1);
            assert!(tracks >= 1);
            assert_eq!(value, 0.5);
        }
    }

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
    fn focusing_and_confirming_without_editing_preserves_unrounded_value() {
        let ctx = egui::Context::default();
        let mut value = 0.123456;
        let mut changed = false;
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        for events in focus_events(rect.get().center()) {
            run_numeric_frame(&ctx, events, &mut value, &mut changed, &rect);
        }
        run_numeric_frame(&ctx, enter_events(), &mut value, &mut changed, &rect);
        assert!(!changed);
        assert_eq!(value, 0.123456);
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
        super::take_adjustment_signals(&ctx);
        run_numeric_frame(&ctx, enter_events(), &mut value, &mut changed, &rect);
        assert!(changed);
        assert_eq!(value, 0.5);
        let canceled = super::take_adjustment_signals(&ctx);
        assert!(canceled.canceled && !canceled.completed && !canceled.active);
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
        super::take_adjustment_signals(&ctx);
        run(escape_events(), &mut value, &mut changed, &other, &rect);
        assert!(!changed);
        assert_eq!(value, 0.1);
        let canceled = super::take_adjustment_signals(&ctx);
        assert!(canceled.canceled && !canceled.active && !canceled.completed);
    }

    #[test]
    fn accepted_text_keeps_focus_and_draft_until_one_completion() {
        let ctx = egui::Context::default();
        let mut value = 0.5;
        let mut changed = false;
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        for events in focus_events(rect.get().center()) {
            run_numeric_frame(&ctx, events, &mut value, &mut changed, &rect);
            assert_eq!(super::take_adjustment_signals(&ctx), Default::default());
        }
        run_numeric_frame(
            &ctx,
            replace_events("0.2500"),
            &mut value,
            &mut changed,
            &rect,
        );
        assert_eq!(value, 0.25);
        let started = super::take_adjustment_signals(&ctx);
        assert!(started.started && started.active);
        assert!(!started.completed && !started.canceled);
        assert_eq!(started.label.as_deref(), Some("exposure"));

        // Rebuilding the UI with the accepted value must not format away the
        // focused text's trailing zeroes or discard keyboard focus.
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        let focused = ctx.memory(|memory| memory.focused()).unwrap();
        let draft = ctx
            .data(|data| data.get_temp::<super::NumericEditState>(focused))
            .unwrap();
        assert_eq!(draft.text, "0.2500");
        let active = super::take_adjustment_signals(&ctx);
        assert!(active.active && !active.started && !active.completed);

        run_numeric_frame(&ctx, enter_events(), &mut value, &mut changed, &rect);
        let completed = super::take_adjustment_signals(&ctx);
        assert!(completed.completed && !completed.active && !completed.canceled);
        assert_eq!(value, 0.25);
        assert_eq!(super::take_adjustment_signals(&ctx), Default::default());
    }

    fn escape_events() -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    #[test]
    fn escape_cancels_text_and_restores_focus_start() {
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
        super::take_adjustment_signals(&ctx);
        run_numeric_frame(&ctx, replace_events("-"), &mut value, &mut changed, &rect);
        assert_eq!(value, 0.25);
        assert!(!changed);
        assert!(super::take_adjustment_signals(&ctx).active);

        run_numeric_frame(&ctx, escape_events(), &mut value, &mut changed, &rect);
        assert!(changed);
        assert_eq!(value, 0.5);
        let canceled = super::take_adjustment_signals(&ctx);
        assert!(canceled.canceled && !canceled.completed && !canceled.active);
        assert!(ctx.memory(|memory| memory.focused()).is_none());
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        assert_eq!(super::take_adjustment_signals(&ctx), Default::default());
    }

    #[test]
    fn wheel_edit_completes_once_without_smoothing_transactions() {
        let ctx = egui::Context::default();
        let mut value = 0.5;
        let mut changed = false;
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        run_numeric_frame(
            &ctx,
            vec![
                egui::Event::PointerMoved(rect.get().center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, 1.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            &mut value,
            &mut changed,
            &rect,
        );
        assert_eq!(value, 0.55);
        let step = super::take_adjustment_signals(&ctx);
        assert!(step.started && step.completed && !step.active && !step.canceled);
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        assert_eq!(value, 0.55);
        assert!(!changed);
        assert_eq!(super::take_adjustment_signals(&ctx), Default::default());
    }

    fn run_slider_frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        value: &mut f64,
    ) -> (egui::Rect, super::AdjustmentSignals) {
        let mut rect = egui::Rect::NOTHING;
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
                    rect = ui
                        .scope(|ui| {
                            super::row_slider(ui, "exposure", value, 0.0, 1.0, 0.05);
                        })
                        .response
                        .rect;
                });
            },
        );
        (rect, super::take_adjustment_signals(ctx))
    }

    #[test]
    fn slider_drag_has_one_start_and_one_completion() {
        let ctx = egui::Context::default();
        let mut value = 0.5;
        let (rect, _) = run_slider_frame(&ctx, vec![], &mut value);
        let position = rect.center();
        let events = focus_events(position);
        let (_, started) = run_slider_frame(&ctx, events[0].clone(), &mut value);
        assert!(started.started && started.active && !started.completed);
        let moved = egui::pos2(rect.right() - 5.0, position.y);
        let (_, active) =
            run_slider_frame(&ctx, vec![egui::Event::PointerMoved(moved)], &mut value);
        assert!(active.active && !active.started && !active.completed);
        assert!(value > 0.5);
        let (_, completed) = run_slider_frame(&ctx, focus_events(moved)[1].clone(), &mut value);
        assert!(completed.completed && !completed.active && !completed.started);
    }

    #[test]
    fn slider_escape_restores_start_and_suppresses_until_release() {
        let ctx = egui::Context::default();
        let mut value = 0.5;
        let (rect, _) = run_slider_frame(&ctx, vec![], &mut value);
        let position = egui::pos2(rect.right() - 5.0, rect.center().y);
        let events = focus_events(position);
        run_slider_frame(&ctx, events[0].clone(), &mut value);
        assert!(value > 0.5);
        let (_, canceled) = run_slider_frame(&ctx, escape_events(), &mut value);
        assert_eq!(value, 0.5);
        assert!(canceled.canceled && !canceled.completed && !canceled.active);
        let moved = egui::pos2(rect.left() + 5.0, position.y);
        let (_, suppressed) =
            run_slider_frame(&ctx, vec![egui::Event::PointerMoved(moved)], &mut value);
        assert_eq!(value, 0.5);
        assert_eq!(suppressed, Default::default());
        let (_, released) = run_slider_frame(&ctx, focus_events(moved)[1].clone(), &mut value);
        assert_eq!(value, 0.5);
        assert_eq!(released, Default::default());
    }

    #[test]
    fn wheel_after_typing_completes_separately_and_keeps_each_original_event() {
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
        assert!(super::take_adjustment_signals(&ctx).started);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            modifiers: egui::Modifiers::NONE,
        };
        run_numeric_frame(
            &ctx,
            vec![
                egui::Event::PointerMoved(rect.get().center()),
                wheel.clone(),
                wheel,
            ],
            &mut value,
            &mut changed,
            &rect,
        );
        assert_eq!(value, 0.25);
        let text = super::take_adjustment_signals(&ctx);
        assert!(text.completed && !text.started && !text.active);
        for expected in [0.30, 0.35] {
            run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
            assert!((value - expected).abs() < 1e-12);
            let step = super::take_adjustment_signals(&ctx);
            assert!(step.started && step.completed && !step.active);
        }
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        assert!(!changed);
        assert_eq!(super::take_adjustment_signals(&ctx), Default::default());
    }

    #[test]
    fn slider_waits_for_text_completion_before_first_mutation() {
        let ctx = egui::Context::default();
        let mut value = 0.5;
        let (rect, _) = run_slider_frame(&ctx, vec![], &mut value);
        super::signal_adjustment(
            &ctx,
            "other",
            super::AdjustmentSignals {
                completed: true,
                ..Default::default()
            },
        );
        let position = egui::pos2(rect.right() - 5.0, rect.center().y);
        let events = focus_events(position);
        let (_, completing_text) = run_slider_frame(&ctx, events[0].clone(), &mut value);
        assert_eq!(value, 0.5);
        assert!(completing_text.completed && !completing_text.started);
        let (_, started_slider) = run_slider_frame(&ctx, vec![], &mut value);
        assert!(value > 0.5);
        assert!(started_slider.started && started_slider.active && !started_slider.completed);
        let (_, completed_slider) = run_slider_frame(&ctx, events[1].clone(), &mut value);
        assert!(completed_slider.completed && !completed_slider.active);
    }

    #[test]
    fn focused_idle_field_reconciles_external_value_before_blur() {
        let ctx = egui::Context::default();
        let mut value = 0.5;
        let mut changed = false;
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        for events in focus_events(rect.get().center()) {
            run_numeric_frame(&ctx, events, &mut value, &mut changed, &rect);
        }
        value = 0.75;
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        assert!(!changed);
        let focused = ctx.memory(|memory| memory.focused()).unwrap();
        let state = ctx
            .data(|data| data.get_temp::<super::NumericEditState>(focused))
            .unwrap();
        assert_eq!(state.text, "0.750");
        assert_eq!(state.edit_start, 0.75);
        run_numeric_frame(&ctx, enter_events(), &mut value, &mut changed, &rect);
        assert!(!changed);
        assert_eq!(value, 0.75);
        assert_eq!(super::take_adjustment_signals(&ctx), Default::default());
    }

    #[test]
    fn consuming_empty_signals_preserves_later_text_owner_before_slider() {
        let ctx = egui::Context::default();
        let mut slider_value = 0.5;
        let mut text_value = 0.5;
        let run = |events: Vec<egui::Event>, slider_value: &mut f64, text_value: &mut f64| {
            let mut slider_rect = egui::Rect::NOTHING;
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
                    // History begins each frame by draining stale signals.
                    assert_eq!(super::take_adjustment_signals(ctx), Default::default());
                    egui::CentralPanel::default().show(ctx, |ui| {
                        slider_rect = ui
                            .scope(|ui| {
                                super::row_slider(
                                    ui,
                                    "earlier slider",
                                    slider_value,
                                    0.0,
                                    1.0,
                                    0.05,
                                );
                            })
                            .response
                            .rect;
                        let id = ui.make_persistent_id(("numeric-field", "later text"));
                        if ctx.input(|input| {
                            input
                                .events
                                .iter()
                                .any(|event| matches!(event, egui::Event::Text(_)))
                        }) {
                            ctx.memory_mut(|memory| memory.request_focus(id));
                        }
                        numeric_field(
                            ui,
                            "later text",
                            "later text",
                            text_value,
                            0.0,
                            1.0,
                            0.05,
                            3,
                            "tip",
                            None,
                            None,
                        );
                    });
                },
            );
            (slider_rect, super::take_adjustment_signals(&ctx))
        };
        run(vec![], &mut slider_value, &mut text_value);
        let (rect, editing) = run(replace_events("0.25"), &mut slider_value, &mut text_value);
        assert!(editing.started && editing.active);
        assert_eq!(text_value, 0.25);
        // The next frame drains an empty signal object before drawing this
        // slider, but the later field still owns the active text transaction.
        let position = egui::pos2(rect.right() - 5.0, rect.center().y);
        let (_, text_completed) = run(
            focus_events(position)[0].clone(),
            &mut slider_value,
            &mut text_value,
        );
        assert_eq!(slider_value, 0.5);
        assert!(text_completed.completed && !text_completed.started && !text_completed.active);
        let (_, slider_started) = run(vec![], &mut slider_value, &mut text_value);
        assert!(slider_value > 0.5);
        assert!(slider_started.started && slider_started.active && !slider_started.completed);
        let (_, canceled) = run(escape_events(), &mut slider_value, &mut text_value);
        assert_eq!(slider_value, 0.5);
        assert!(canceled.canceled && !canceled.completed && !canceled.active);
    }

    #[test]
    fn standalone_wheel_events_do_not_require_history_signal_consumption() {
        let ctx = egui::Context::default();
        let mut value = 0.5;
        let mut changed = false;
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        for (delta, expected) in [(1.0, 0.55), (-1.0, 0.5)] {
            run_numeric_frame(
                &ctx,
                vec![
                    egui::Event::PointerMoved(rect.get().center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta: egui::vec2(0.0, delta),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                &mut value,
                &mut changed,
                &rect,
            );
            assert!(changed);
            assert!((value - expected).abs() < 1e-12);
            // Standalone widgets have no History consumer at frame end.
        }
        run_numeric_frame(&ctx, vec![], &mut value, &mut changed, &rect);
        assert!(!changed);
        assert!((value - 0.5).abs() < 1e-12);
        assert_eq!(super::take_adjustment_signals(&ctx), Default::default());
    }
}

#[cfg(test)]
mod photograph_draft_tests {
    use super::*;

    fn state(text: &str, value: f64, start: f64, detached: bool) -> NumericEditState {
        NumericEditState {
            text: text.into(),
            last_value: value,
            edit_start: start,
            focused: true,
            editing: true,
            suspended_for_language: detached,
            resumed_after_language: detached,
            wheel_events: VecDeque::from([1.0]),
        }
    }

    fn frame(ctx: &egui::Context, value: &mut f64) -> egui::Id {
        let mut id = egui::Id::NULL;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                id = ui.make_persistent_id(("numeric-field", "photo-exposure"));
                numeric_field(
                    ui,
                    "exposure",
                    "photo-exposure",
                    value,
                    0.0,
                    1.0,
                    0.05,
                    3,
                    "tip",
                    None,
                    None,
                );
            });
        });
        id
    }

    #[test]
    fn suspended_photograph_a_draft_cannot_commit_or_cancel_into_b() {
        let ctx = egui::Context::default();
        let mut a = 0.25;
        let id = frame(&ctx, &mut a);
        ctx.data_mut(|data| {
            data.insert_temp(id, state("not-a-number", a, 0.5, false));
            data.insert_temp(egui::Id::new("numeric-active-text"), id);
        });
        ctx.memory_mut(|memory| memory.request_focus(id));
        suspend_numeric_draft(&ctx);
        signal_adjustment(
            &ctx,
            "exposure",
            AdjustmentSignals {
                active: true,
                ..Default::default()
            },
        );
        let a_drafts = NumericDrafts::take(&ctx);
        assert!(ctx.memory(|memory| memory.focused()).is_none());
        assert!(!has_suspended_adjustment(&ctx));
        assert_eq!(take_adjustment_signals(&ctx), AdjustmentSignals::default());
        assert!(
            ctx.data(|data| data.get_temp::<NumericEditState>(id))
                .is_none()
        );
        let mut b = 0.9;
        assert_eq!(
            frame(&ctx, &mut b),
            id,
            "same stable control ID across photographs"
        );
        assert_eq!(b, 0.9);
        assert_eq!(take_adjustment_signals(&ctx), AdjustmentSignals::default());
        let b_drafts = NumericDrafts::take(&ctx);
        a_drafts.restore(&ctx);
        frame(&ctx, &mut a);
        assert_eq!(a, 0.25, "restoring must not lose-focus commit or cancel A");
        assert!(has_suspended_adjustment(&ctx));
        let restored = ctx
            .data(|data| data.get_temp::<NumericEditState>(id))
            .unwrap();
        assert_eq!(restored.text, "not-a-number");
        assert_eq!(restored.edit_start, 0.5);
        assert!(restored.wheel_events.is_empty());
        assert_eq!(take_adjustment_signals(&ctx), AdjustmentSignals::default());
        b_drafts.restore(&ctx);
        frame(&ctx, &mut b);
        assert_eq!(b, 0.9);
        assert!(!has_suspended_adjustment(&ctx));
    }

    #[test]
    fn detached_drafts_and_slider_interactions_are_moved_not_shared() {
        let ctx = egui::Context::default();
        let text = egui::Id::new("detached-photo-draft");
        let widget = egui::Id::new("photo-slider");
        let slider = widget.with("adjustment");
        register_numeric_text(&ctx, text);
        register_numeric_slider(&ctx, widget, slider);
        ctx.data_mut(|data| {
            data.insert_temp(text, state("0.1200", 0.12, 0.4, true));
            data.insert_temp(
                slider,
                SliderEditState {
                    start: Some(0.6),
                    deferred: Some(0.2),
                    ..Default::default()
                },
            );
        });
        ctx.set_dragged_id(widget);
        let drafts = NumericDrafts::take(&ctx);
        assert!(ctx.dragged_id().is_none());
        assert!(
            ctx.data(|data| data.get_temp::<SliderEditState>(slider))
                .is_none()
        );
        assert!(
            ctx.data(|data| data.get_temp::<NumericEditState>(text))
                .is_none()
        );
        drafts.restore(&ctx);
        assert!(
            ctx.dragged_id().is_none(),
            "restoring never resumes a pointer from another photograph"
        );
        let restored = ctx
            .data(|data| data.get_temp::<SliderEditState>(slider))
            .unwrap();
        assert!(restored.suspended);
        assert_eq!(restored.start, Some(0.6));
        assert_eq!(restored.deferred, Some(0.2));
        let restored = ctx
            .data(|data| data.get_temp::<NumericEditState>(text))
            .unwrap();
        assert_eq!(restored.text, "0.1200");
        assert!(restored.resumed_after_language);
        assert!(restored.suspended_for_language);
        assert!(
            !has_suspended_adjustment(&ctx),
            "detached draft must not reopen a closed history gesture"
        );
    }
}
