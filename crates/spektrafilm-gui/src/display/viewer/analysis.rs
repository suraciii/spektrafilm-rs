//! Viewer analysis interaction: temporary selection drafts and linked pixel
//! samples. The analysis workspace owns the committed selection and plot;
//! this component owns only the gesture and in-flight viewer state.
use egui::{Id, Rect, Response, Ui, Vec2};

use super::super::ViewLayer;
use super::super::geometry::{contains_half_open, image_overlay, image_point};
use crate::theme;

/// Per-frame presentation inputs supplied by the analysis workspace.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ViewerAnalysisInput {
    pub enabled: bool,
    /// Normalized image rectangle; the caller maps its source-area identity.
    pub selection: Option<[f64; 4]>,
    /// Normalized image rectangle for plot-to-image inspection.
    pub band: Option<[f64; 4]>,
    /// Plot inspection currently owns linked correspondence.
    pub plot_inspecting: bool,
    pub cursor_control: Option<Id>,
}

/// Viewer-only correspondence in the selected native image.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewerAnalysisSample {
    pub layer: ViewLayer,
    pub x: u32,
    pub y: u32,
    pub keyboard: bool,
}

/// Viewer-owned analysis interaction state. Outputs stay here until the
/// workspace consumes them, so cancellation clears pending publication
/// without a second copy.
#[derive(Default)]
pub(super) struct ViewerAnalysis {
    armed: bool,
    sample: Option<ViewerAnalysisSample>,
    suspended: bool,
    completed: Option<[f64; 4]>,
    draft: Option<[f64; 4]>,
    drag_start: Option<[f64; 2]>,
    suppress_drag: bool,
    draft_subject: Option<(ViewLayer, [usize; 2])>,
    cursor: Option<[u32; 2]>,
}

impl ViewerAnalysis {
    pub(super) fn arm(&mut self, selection: Option<[f64; 4]>) {
        self.armed = true;
        self.completed = None;
        self.draft = selection;
        self.drag_start = None;
        self.draft_subject = None;
        self.sample = None;
    }

    pub(super) fn cancel(&mut self) {
        self.suppress_drag |= self.drag_start.is_some();
        self.armed = false;
        self.draft = None;
        self.drag_start = None;
        self.draft_subject = None;
        self.completed = None;
    }

    /// Discard presentation that belongs to the previous source. A committed
    /// selection is workspace-owned and therefore not touched here.
    pub(super) fn reset_after_input(&mut self) {
        self.cancel();
        self.sample = None;
        self.cursor = None;
    }

    pub(super) fn armed(&self) -> bool {
        self.armed
    }

    pub(super) fn sample(&self) -> Option<ViewerAnalysisSample> {
        self.sample
    }

    pub(super) fn suspended(&self) -> bool {
        self.suspended
    }

    pub(super) fn take_completed(&mut self) -> Option<[f64; 4]> {
        self.completed.take()
    }

    pub(super) fn clear_sample(&mut self) {
        self.sample = None;
    }

    /// Resolve selection gestures before the normal marquee, pan and zoom
    /// handlers run. The boolean tells those handlers that this frame's
    /// primary drag belongs to the analysis selection tool.
    pub(super) fn begin_frame(
        &mut self,
        ui: &Ui,
        response: &Response,
        gesture_image: Rect,
        gesture_size: Option<[usize; 2]>,
        layer: ViewLayer,
        input: &ViewerAnalysisInput,
    ) -> bool {
        if !input.enabled || gesture_size.is_none() {
            self.cancel();
        } else if self.armed
            && let Some(size) = gesture_size
        {
            let subject = (layer, size);
            if self
                .draft_subject
                .is_some_and(|previous| previous != subject)
            {
                self.cancel();
            } else {
                self.draft_subject = Some(subject);
            }
        }
        if self.armed && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel();
        }
        let mut analysis_drag = self.drag_start.is_some() || self.suppress_drag;
        if self.armed
            && response.drag_started_by(egui::PointerButton::Primary)
            && !ui.input(|i| i.modifiers.shift)
            && let Some(start) = ui.input(|i| i.pointer.press_origin())
            && contains_half_open(gesture_image, start)
        {
            self.drag_start = Some(image_point(gesture_image, start));
            analysis_drag = true;
        }
        if let Some(start) = self.drag_start
            && let Some(pos) = response
                .interact_pointer_pos()
                .or_else(|| ui.input(|i| i.pointer.latest_pos()))
        {
            let end = image_point(gesture_image, pos);
            self.draft = Some([
                start[0].min(end[0]),
                start[1].min(end[1]),
                start[0].max(end[0]),
                start[1].max(end[1]),
            ]);
        }
        if analysis_drag && response.drag_stopped_by(egui::PointerButton::Primary) {
            let draft = self.draft;
            self.drag_start = None;
            if let (Some(bounds), Some(size)) = (draft, gesture_size)
                && nonempty_image_area(bounds, size)
            {
                self.cancel();
                self.completed = Some(bounds);
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            self.suppress_drag = false;
        }
        analysis_drag
    }

    /// Draw and resolve the analysis overlay after the presentation texture
    /// has been painted.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        response: &Response,
        rect: Rect,
        image_rect: Rect,
        layer: ViewLayer,
        source_size: Option<[usize; 2]>,
        input: &ViewerAnalysisInput,
        analysis_drag: bool,
        transitioning: bool,
    ) {
        self.sample = None;
        self.suspended = input.enabled
            && (self.armed
                || analysis_drag
                || response.dragged()
                || transitioning
                || (response.hovered()
                    && ui.input(|i| i.zoom_delta() != 1.0 || i.smooth_scroll_delta.y != 0.0)));
        let mut keyboard_cursor_focused = false;
        if input.enabled
            && layer != ViewLayer::PaperBack
            && let Some(size) = source_size
        {
            if let Some(band) = input.band {
                let band_rect = image_overlay(image_rect, band);
                let painter = ui.painter_at(rect.intersect(image_rect));
                painter.rect_filled(band_rect, 0.0, theme::FG.gamma_multiply(0.12));
                painter.rect_stroke(
                    band_rect,
                    0.0,
                    theme::stroke(theme::FG),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(area) = self.draft.filter(|_| self.armed).or(input.selection) {
                let painter = ui.painter_at(rect.intersect(image_rect));
                let area = image_overlay(image_rect, area);
                painter.rect_filled(area, 0.0, theme::FG.gamma_multiply(0.08));
                painter.rect_stroke(
                    area,
                    0.0,
                    theme::stroke(theme::FG),
                    egui::StrokeKind::Inside,
                );
            }
            let controls_rect = Rect::from_min_size(
                rect.min + Vec2::splat(theme::SPACE_2),
                Vec2::new(rect.width().max(1.0), theme::HEIGHT_CONTROL * 2.0),
            );
            let mut controls = ui.new_child(egui::UiBuilder::new().max_rect(controls_rect));
            controls.set_clip_rect(rect);
            controls.horizontal(|ui| {
                if self.armed {
                    let move_handle =
                        ui.button(crate::i18n::text(ui.ctx(), "surfaces.viewer.analysis_move"));
                    let resize_handle = ui.button(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.viewer.analysis_resize",
                    ));
                    for handle in [&move_handle, &resize_handle] {
                        if handle.clicked() {
                            handle.request_focus();
                        }
                        if handle.has_focus() {
                            ui.memory_mut(|memory| {
                                memory.set_focus_lock_filter(
                                    handle.id,
                                    egui::EventFilter {
                                        horizontal_arrows: true,
                                        vertical_arrows: true,
                                        ..Default::default()
                                    },
                                );
                            });
                        }
                    }
                    let focused = move_handle.has_focus() || resize_handle.has_focus();
                    if focused {
                        let area = self.draft.get_or_insert([0.25, 0.25, 0.75, 0.75]);
                        let (dx, dy, confirm) = ui.input(|i| {
                            (
                                i.key_pressed(egui::Key::ArrowRight) as i32
                                    - i.key_pressed(egui::Key::ArrowLeft) as i32,
                                i.key_pressed(egui::Key::ArrowDown) as i32
                                    - i.key_pressed(egui::Key::ArrowUp) as i32,
                                i.key_pressed(egui::Key::Enter),
                            )
                        });
                        for (axis, delta) in [(0, dx), (1, dy)] {
                            let step = delta as f64 / size[axis].max(1) as f64;
                            if move_handle.has_focus() {
                                let step = step.clamp(-area[axis], 1.0 - area[axis + 2]);
                                area[axis] += step;
                                area[axis + 2] += step;
                            } else {
                                area[axis + 2] = (area[axis + 2] + step).clamp(
                                    (area[axis] + 1.0 / size[axis].max(1) as f64).min(1.0),
                                    1.0,
                                );
                            }
                        }
                        if confirm && nonempty_image_area(*area, size) {
                            let completed = *area;
                            self.cancel();
                            self.completed = Some(completed);
                        }
                    }
                }
            });
            if !self.armed
                && !input.plot_inspecting
                && let Some(id) = input.cursor_control
            {
                if ui.memory(|memory| memory.has_focus(id))
                    && response
                        .hover_pos()
                        .is_some_and(|p| contains_half_open(image_rect, p))
                    && ui.input(|i| i.pointer.delta() != Vec2::ZERO)
                {
                    ui.memory_mut(|memory| memory.surrender_focus(id));
                }
                keyboard_cursor_focused = ui.memory(|memory| memory.has_focus(id));
                if keyboard_cursor_focused {
                    let point = self
                        .cursor
                        .get_or_insert([size[0] as u32 / 2, size[1] as u32 / 2]);
                    let (dx, dy) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::ArrowRight) as i64
                                - i.key_pressed(egui::Key::ArrowLeft) as i64,
                            i.key_pressed(egui::Key::ArrowDown) as i64
                                - i.key_pressed(egui::Key::ArrowUp) as i64,
                        )
                    });
                    point[0] =
                        (point[0] as i64 + dx).clamp(0, size[0].saturating_sub(1) as i64) as u32;
                    point[1] =
                        (point[1] as i64 + dy).clamp(0, size[1].saturating_sub(1) as i64) as u32;
                    let position = image_rect.min
                        + image_rect.size()
                            * Vec2::new(
                                (point[0] as f32 + 0.5) / size[0].max(1) as f32,
                                (point[1] as f32 + 0.5) / size[1].max(1) as f32,
                            );
                    ui.painter_at(rect.intersect(image_rect)).circle_stroke(
                        position,
                        5.0,
                        theme::stroke(theme::FG),
                    );
                }
            }
            if !self.suspended && !input.plot_inspecting {
                let pixel = if keyboard_cursor_focused {
                    self.cursor.map(|p| (p, true))
                } else {
                    response
                        .hover_pos()
                        .filter(|p| contains_half_open(image_rect, *p))
                        .map(|p| {
                            let uv = image_point(image_rect, p);
                            (
                                [
                                    ((uv[0] * size[0] as f64) as u32)
                                        .min(size[0].saturating_sub(1) as u32),
                                    ((uv[1] * size[1] as f64) as u32)
                                        .min(size[1].saturating_sub(1) as u32),
                                ],
                                false,
                            )
                        })
                };
                self.sample = pixel.map(|(point, keyboard)| ViewerAnalysisSample {
                    layer,
                    x: point[0],
                    y: point[1],
                    keyboard,
                });
            }
        }
    }
}

fn nonempty_image_area(bounds: [f64; 4], size: [usize; 2]) -> bool {
    (0..2).all(|axis| {
        let lo = (bounds[axis] * size[axis] as f64 - 0.5).ceil().max(0.0);
        let hi = (bounds[axis + 2] * size[axis] as f64 - 0.5)
            .ceil()
            .min(size[axis] as f64);
        hi > lo
    })
}

#[cfg(test)]
pub(super) fn nonempty_image_area_for_test(bounds: [f64; 4], size: [usize; 2]) -> bool {
    nonempty_image_area(bounds, size)
}

#[cfg(test)]
impl ViewerAnalysis {
    pub(super) fn set_drag_for_test(&mut self, start: [f64; 2], draft: Option<[f64; 4]>) {
        self.drag_start = Some(start);
        self.draft = draft;
    }

    pub(super) fn draft_for_test(&self) -> Option<[f64; 4]> {
        self.draft
    }

    pub(super) fn suppresses_drag_for_test(&self) -> bool {
        self.suppress_drag
    }
}
