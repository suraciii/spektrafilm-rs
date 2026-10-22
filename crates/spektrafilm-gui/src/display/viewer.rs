//! Viewer presentation: zoom, pan, layer controls and pixel inspection.
//! Immutable pipeline floats are shared; viewing pixels stay disposable.
use egui::{Color32, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use serde_json::Value;
use spektrafilm_math::image::ImageBuf;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[cfg(windows)]
use super::colortransform::discover_display_profile;
use super::raster::{DisplayRaster, Interpolation};
use super::worker::{DisplayRequest, DisplaySource, DisplayWorker, PreparedDisplay};
use super::{ANIMATION_MAX_PIXELS, DisplaySettings, INTERPOLATIONS, ViewLayer};
/// [width,height], with a common long edge of one. The output is fitted into
/// this input bounding box rather than independently normalized.
pub fn normalized_world_size(size: [usize; 2]) -> Vec2 {
    let long = size[0].max(size[1]).max(1) as f32;
    Vec2::new(size[0] as f32 / long, size[1] as f32 / long)
}
pub fn fitted_world_size(size: [usize; 2], bounds: Vec2) -> Vec2 {
    let p = Vec2::new(size[0].max(1) as f32, size[1].max(1) as f32);
    p * (bounds.x / p.x).min(bounds.y / p.y)
}

#[derive(Clone, Copy, Debug)]
pub struct PixelInspection {
    pub layer: ViewLayer,
    pub x: u32,
    pub y: u32,
    pub rgb: [f64; 3],
}

struct Transition {
    start: f64,
    frame: usize,
    previous: Option<DisplayRaster>,
    polaroid: bool,
}

#[derive(Default)]
pub struct Viewer {
    pub settings: DisplaySettings,
    pub zoom: f32,
    pub zoom_percent: Option<f32>,
    pub pan: Vec2,
    pub input: Option<DisplayRaster>,
    pub output: Option<DisplayRaster>,
    pub input_size: Option<[usize; 2]>,
    pub output_size: Option<[usize; 2]>,
    pub transform_status: String,
    display_max_size: usize,
    transition: Option<Transition>,
    texture: Option<TextureHandle>,
    cache_key: Option<Vec<u64>>,
    generation: u64,
    accepted_generation: u64,
    input_source_space: Option<String>,
    input_source_encoded: bool,
    output_source_space: Option<String>,
    output_source_encoded: bool,
    output_source_transform: bool,
    output_source_profile: Option<PathBuf>,
    prepared_input: Option<PreparedDisplay>,
    prepared_output: Option<PreparedDisplay>,
    /// Before/after split fraction (0..=1) of image width; `None` disables.
    compare_split: Option<f32>,
    /// Active marquee selection in view coordinates `(start, current)`; a
    /// Shift or secondary-button drag sets it and its release zooms to it.
    marquee: Option<(Pos2, Pos2)>,
    revision: u64,
    worker: DisplayWorker,
    #[cfg(test)]
    observed_geometry: Option<(Rect, Rect, Interpolation, Option<PixelInspection>)>,
    #[cfg(test)]
    observed_image: Option<egui::ColorImage>,
}
#[cfg(test)]
pub(crate) struct ViewerObservation {
    pub viewport: Rect,
    pub image_rect: Rect,
    pub interpolation: Interpolation,
    pub inspection: Option<PixelInspection>,
    pub pixels: Option<egui::ColorImage>,
    pub key: Option<Vec<u64>>,
    pub generation: u64,
    pub accepted_generation: u64,
    pub transition: Option<(usize, bool)>,
}

impl Viewer {
    #[cfg(test)]
    pub(crate) fn observation(&self) -> ViewerObservation {
        let (viewport, image_rect, interpolation, inspection) =
            self.observed_geometry.expect("viewer has been shown");
        ViewerObservation {
            viewport,
            image_rect,
            interpolation,
            inspection,
            pixels: self.observed_image.clone(),
            key: self.cache_key.clone(),
            generation: self.generation,
            accepted_generation: self.accepted_generation,
            transition: self.transition.as_ref().map(|t| (t.frame, t.polaroid)),
        }
    }
    /// The explicit ICC profile the disposable output raster is built with.
    /// Viewer-only: the profile never reaches retained, saved or exported
    /// pixels.
    #[cfg(test)]
    pub(crate) fn output_display_profile(&self) -> Option<&Path> {
        self.output_source_profile.as_deref()
    }
    pub fn invalidate_display(&mut self) {
        // Display controls change viewing pixels without retaining development animation.
        self.transition = None;
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn set_display_max_size(&mut self, max_size: u32) {
        let max_size = max_size.max(1) as usize;
        if self.display_max_size != max_size {
            self.display_max_size = max_size;
            self.revision = self.revision.wrapping_add(1);
        }
    }
    pub fn new() -> Self {
        Self {
            zoom: 1.0,
            zoom_percent: None,
            display_max_size: 1024,
            ..Self::default()
        }
    }
    /// Set the before/after split fraction (0..=1) of image width; `None`
    /// disables split rendering. Viewer-only, never affects saved pixels.
    pub fn set_compare_split(&mut self, fraction: Option<f32>) {
        let normalized = fraction.map(|f| f.clamp(0.0, 1.0));
        if self.compare_split != normalized {
            self.compare_split = normalized;
            self.revision = self.revision.wrapping_add(1);
        }
    }
    pub fn set_input(&mut self, raster: DisplayRaster, original_size: [usize; 2]) {
        self.input_size = Some(original_size);
        self.input = Some(raster);
        self.output = None;
        self.prepared_input = None;
        self.prepared_output = None;
        self.output_size = None;
        self.transition = None;
        self.settings.layer = ViewLayer::PaperBack;
        self.reset_view();
        self.revision += 1;
    }
    /// Call only after accepting the latest asynchronous job. Retains the old
    /// display frame for a ten-step fade when `animate` is true; it never
    /// retains or modifies export data.
    pub fn set_output(
        &mut self,
        raster: DisplayRaster,
        original_size: [usize; 2],
        now: f64,
        animate: bool,
    ) {
        let supported = original_size[0].saturating_mul(original_size[1]) <= ANIMATION_MAX_PIXELS;
        let reveal = self.settings.layer != ViewLayer::Output || self.output.is_none();
        let previous = if animate && !reveal && self.settings.crossfade && supported {
            self.output
                .as_ref()
                .filter(|p| p.size == raster.size)
                .cloned()
        } else {
            None
        };
        let polaroid = animate && reveal && self.settings.reveal && supported;
        self.accepted_generation = 0;
        self.transition = if previous.is_some() || polaroid {
            Some(Transition {
                start: now,
                frame: 0,
                previous,
                polaroid,
            })
        } else {
            None
        };
        self.output = Some(raster);
        self.prepared_output = None;
        self.output_size = Some(original_size);
        self.settings.layer = ViewLayer::Output;
        self.revision += 1;
    }
    pub fn set_input_display_source(&mut self, space: &str, encoded: bool) {
        self.input_source_space = Some(space.to_owned());
        self.input_source_encoded = encoded;
        self.revision = self.revision.wrapping_add(1);
    }
    /// Configure immutable full-resolution source metadata used by the worker.
    /// This invalidates disposable viewing pixels without copying pipeline data.
    pub fn set_output_display_source(
        &mut self,
        space: &str,
        encoded: bool,
        enabled: bool,
        profile: Option<&Path>,
    ) {
        self.output_source_space = Some(space.to_owned());
        self.output_source_encoded = encoded;
        self.output_source_transform = enabled;
        self.output_source_profile = profile.map(Path::to_path_buf);
        #[cfg(windows)]
        if enabled && self.output_source_profile.is_none() {
            self.output_source_profile = discover_display_profile().ok().flatten();
        }
        self.prepared_output = None;
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn seed_prepared_input(&mut self, source: Arc<ImageBuf>, max_size: u32) {
        let Some(raster) = self.input.clone() else {
            return;
        };
        self.prepared_input = Some(PreparedDisplay {
            source,
            metadata: DisplaySource {
                space: self.input_source_space.clone(),
                encoded: self.input_source_encoded,
                transform: true,
                profile: None,
            },
            max_size: max_size.max(1) as usize,
            raster,
        });
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn seed_prepared_output(
        &mut self,
        source: Arc<ImageBuf>,
        space: &str,
        encoded: bool,
        enabled: bool,
        profile: Option<&Path>,
        max_size: u32,
    ) {
        let Some(raster) = self.output.clone() else {
            return;
        };
        let profile = profile.map(Path::to_path_buf);
        #[cfg(windows)]
        let profile = if enabled && profile.is_none() {
            discover_display_profile().ok().flatten()
        } else {
            profile
        };
        self.prepared_output = Some(PreparedDisplay {
            source,
            metadata: DisplaySource {
                space: Some(space.to_owned()),
                encoded,
                transform: enabled,
                profile,
            },
            max_size: max_size.max(1) as usize,
            raster,
        });
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.zoom_percent = None;
        self.pan = Vec2::ZERO;
    }
    /// Return to the fitted zoom, leaving the pan where it is.
    ///
    /// Fitting is the zoom alone: an exact zoom and the fit multiplier share
    /// the same `zoom_percent`/`zoom` pair, so clearing both restores the
    /// viewport-fitted scale without moving the view. `reset_view` is the
    /// wider action that also recentres the pan.
    pub fn fit_view(&mut self) {
        self.zoom = 1.0;
        self.zoom_percent = None;
    }
    /// Set an exact source-pixel zoom. 100% means one source pixel per device
    /// pixel, independent of viewport size and white padding.
    pub fn set_zoom_percent(&mut self, percent: f32) {
        if percent.is_finite() {
            self.zoom_percent = Some(percent.max(0.0).clamp(0.1, 3200.0));
            self.zoom = 1.0;
        }
    }
    pub fn zoom_percent(&self) -> Option<f32> {
        self.zoom_percent
    }
    /// Zoom by `factor`, keeping the content at `at` fixed. Returns the
    /// effective factor after the zoom bounds.
    fn zoom_at(&mut self, at: Pos2, rect: Rect, factor: f32) -> f32 {
        let effective = if let Some(percent) = self.zoom_percent {
            let old = percent;
            self.zoom_percent = Some((old * factor).clamp(0.1, 3200.0));
            self.zoom_percent.expect("zoom percent set") / old
        } else {
            let old = self.zoom;
            self.zoom = (old * factor).clamp(0.1, 32.0);
            self.zoom / old
        };
        self.pan += (at - rect.center() - self.pan) * (1.0 - effective);
        effective
    }
    /// Zoom so `selection` fills the view and its center lands at the center.
    fn zoom_to_selection(&mut self, selection: Rect, rect: Rect) {
        let selection = selection.intersect(rect);
        if selection.width() < 1.0 || selection.height() < 1.0 {
            return;
        }
        let factor = (rect.width() / selection.width()).min(rect.height() / selection.height());
        let offset = selection.center() - rect.center() - self.pan;
        let effective = self.zoom_at(selection.center(), rect, factor);
        self.pan = -offset * effective;
    }
    pub fn persistent_state(&self) -> Value {
        serde_json::json!({"settings":self.settings.to_json(),"zoom":self.zoom,"zoom_percent":self.zoom_percent,"pan":[self.pan.x,self.pan.y]})
    }
    pub fn restore_state(&mut self, value: &Value) {
        if !value.is_object() {
            return;
        }
        if value["settings"].is_object() {
            self.settings = DisplaySettings::from_json(&value["settings"]);
        }
        self.zoom = value["zoom"]
            .as_f64()
            .filter(|x| x.is_finite())
            .unwrap_or(1.0)
            .clamp(0.1, 32.0) as f32;
        self.zoom_percent = value["zoom_percent"]
            .as_f64()
            .filter(|x| x.is_finite())
            .map(|x| (x as f32).clamp(0.1, 3200.0));
        self.pan = Vec2::new(
            value["pan"][0]
                .as_f64()
                .filter(|x| x.is_finite())
                .unwrap_or(0.0) as f32,
            value["pan"][1]
                .as_f64()
                .filter(|x| x.is_finite())
                .unwrap_or(0.0) as f32,
        );
    }
    pub fn layer_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let before = self.settings.layer;
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.settings.layer, ViewLayer::Input, "Input")
                .on_hover_text("Show the retained input, decoded from its input color space and transfer decoding. The viewer layer does not change the retained image itself.");
            ui.add_enabled_ui(self.output.is_some(), |ui| {
                ui.selectable_value(&mut self.settings.layer, ViewLayer::Output, "Output")
                    .on_hover_text("Show the retained output of the latest Preview or Scan. The viewer layer does not change the retained image itself.");
            });
            ui.selectable_value(&mut self.settings.layer, ViewLayer::PaperBack, "Paper back")
                .on_hover_text("Viewer-only: show the virtual paper-back rendering generated from paper texture, watermark, and noise. It is not an editable material stage and changes no retained, saved or exported pixels.");
        });
        before != self.settings.layer
    }

    pub fn controls(&mut self, ui: &mut egui::Ui) -> bool {
        let before = (
            self.settings.clone(),
            self.zoom,
            self.zoom_percent,
            self.pan,
        );
        ui.checkbox(&mut self.settings.use_display_transform, "use display transform")
            .on_hover_text("Apply the display transform and the selected display ICC profile to the viewer output only. Turning it off views the values in the output color space. Saved and exported pixels are unchanged either way.");
        ui.checkbox(&mut self.settings.gray_18_canvas, "gray 18% canvas")
            .on_hover_text(
                "Use neutral 18% gray as the canvas behind the image to judge exposure and neutral colors. Canvas decoration only; retained, saved and exported pixels are unchanged.",
            );
        ui.checkbox(&mut self.settings.reveal, "reveal transition")
            .on_hover_text("Animate a wipe reveal when switching from the input to the output view. Viewer presentation only; retained, saved and exported pixels are unchanged.");
        ui.checkbox(&mut self.settings.crossfade, "crossfade transition")
            .on_hover_text("Animate a crossfade between the previous and current output frame. Viewer presentation only; retained, saved and exported pixels are unchanged.");
        let tooltip = "Expand the white border layer around the normalized preview frame, expressed as a fraction of the image long edge. Viewer presentation only; retained, saved and exported pixels are unchanged.";
        crate::controls::param_numeric(
            ui,
            "white padding",
            &mut self.settings.white_padding,
            "display.white_padding",
            tooltip,
        );
        before
            != (
                self.settings.clone(),
                self.zoom,
                self.zoom_percent,
                self.pan,
            )
    }
    pub fn interpolation_control(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_label("output interpolation")
            .selected_text(self.settings.interpolation.name())
            .show_ui(ui, |ui| {
                for name in INTERPOLATIONS {
                    ui.selectable_value(
                        &mut self.settings.interpolation,
                        Interpolation::parse(name),
                        name,
                    );
                }
            })
            .response
            .on_hover_text("Interpolation used to display the output layer in the viewer. It resamples viewer pixels only; retained, saved and exported pixels are unchanged. The input layer always uses nearest and the paper back always uses spline36.");
    }
    fn display_source(&self, layer: ViewLayer) -> DisplaySource {
        if layer == ViewLayer::Input {
            DisplaySource {
                space: self.input_source_space.clone(),
                encoded: self.input_source_encoded,
                transform: true,
                profile: None,
            }
        } else {
            DisplaySource {
                space: self.output_source_space.clone(),
                encoded: self.output_source_encoded,
                transform: self.output_source_transform,
                profile: self.output_source_profile.clone(),
            }
        }
    }
    #[cfg(test)]
    fn full_source_view(
        &self,
        source: &Arc<ImageBuf>,
        layer: ViewLayer,
        image_rect: Rect,
        rect: Rect,
        size: [usize; 2],
        interpolation: Interpolation,
    ) -> Option<(DisplayRaster, Option<String>)> {
        self.display_source(layer).full_source_view(
            source,
            layer,
            image_rect,
            rect,
            size,
            interpolation,
            &mut super::full_transform::TransformCache::default(),
        )
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        input_float: Option<&Arc<ImageBuf>>,
        output_float: Option<&Arc<ImageBuf>>,
    ) -> Option<PixelInspection> {
        let response = ui.allocate_response(
            ui.available_size().max(Vec2::splat(1.0)),
            egui::Sense::click_and_drag(),
        );
        let rect = response.rect;
        if self.zoom <= 0.0 {
            self.zoom = 1.0;
        }
        // Viewer gestures change the retained view only. Wheel and pinch zoom
        // at the pointer; the primary button pans; a Shift or secondary-button
        // drag selects a region that fills the view on release; one click
        // steps the zoom at the pointer; a double click resets the view.
        let selecting = ui
            .input(|i| i.modifiers.shift || i.pointer.button_down(egui::PointerButton::Secondary));
        if response.drag_started()
            && selecting
            && let Some(pos) = response.interact_pointer_pos()
        {
            // The drag starts a frame after the press, so the selection
            // begins where the press began.
            let start = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            self.marquee = Some((start, pos));
        }
        let marquee_drag = response.dragged() && selecting;
        if marquee_drag
            && let Some(pos) = response.interact_pointer_pos()
            && let Some((start, _)) = self.marquee
        {
            self.marquee = Some((start, pos));
        }
        let selection = if response.drag_stopped() {
            self.marquee.take()
        } else {
            None
        };
        if response.hovered() {
            let (pinch, scroll, cursor) = ui.input(|i| {
                (
                    i.zoom_delta(),
                    i.smooth_scroll_delta.y,
                    i.pointer.hover_pos(),
                )
            });
            let factor = if (pinch - 1.0).abs() > 0.0001 {
                pinch
            } else {
                (scroll * 0.0015).exp()
            };
            if (factor - 1.0).abs() > 0.0001 {
                let at = cursor.unwrap_or_else(|| rect.center());
                self.zoom_at(at, rect, factor);
            }
        }
        if response.dragged() && !marquee_drag && self.marquee.is_none() {
            self.pan += response.drag_delta();
        }
        if response.double_clicked() {
            self.reset_view();
            self.marquee = None;
        } else if response.clicked()
            && selection.is_none()
            && let Some(at) = response.interact_pointer_pos()
        {
            self.zoom_at(at, rect, 2.0);
        }
        if let Some((start, end)) = selection {
            self.zoom_to_selection(Rect::from_two_pos(start, end), rect);
        }
        let bounds = normalized_world_size(self.input_size.or(self.output_size).unwrap_or([3, 2]));
        let padding = self.settings.white_padding.max(0.0);
        let padded = bounds + Vec2::splat(2.0 * padding);
        let dpi = ui.ctx().pixels_per_point().max(0.01);
        let scale = if let Some(percent) = self.zoom_percent {
            let size = match self.settings.layer {
                ViewLayer::Input => self.input_size,
                ViewLayer::Output => self.output_size,
                ViewLayer::PaperBack => self.input_size,
            };
            let size = size.unwrap_or([3, 2]);
            let layer_world = if self.settings.layer == ViewLayer::Output {
                fitted_world_size(size, bounds)
            } else {
                bounds
            };
            let px_world =
                (layer_world.x / size[0].max(1) as f32).min(layer_world.y / size[1].max(1) as f32);
            (percent / 100.0) / (dpi * px_world)
        } else {
            (rect.width() / padded.x).min(rect.height() / padded.y) * self.zoom
        };
        let center = rect.center() + self.pan;
        let border = Rect::from_center_size(center, padded * scale);
        let paper_rect = Rect::from_center_size(center, bounds * scale);
        let size = self.input_size.unwrap_or([3, 2]);
        let long = size[0].max(size[1]).max(1);
        let paper_size = [
            (size[0] as f64 * 1024.0 / long as f64).round().max(1.0) as usize,
            (size[1] as f64 * 1024.0 / long as f64).round().max(1.0) as usize,
        ];
        let layer = self.settings.layer;
        let interpolation = match layer {
            ViewLayer::Output => self.settings.interpolation,
            ViewLayer::Input => Interpolation::Nearest,
            ViewLayer::PaperBack => Interpolation::Spline36,
        };
        let selected = match layer {
            ViewLayer::Input => self.input.as_ref(),
            ViewLayer::Output => self.output.as_ref(),
            ViewLayer::PaperBack => None,
        };
        let source_size = match layer {
            ViewLayer::Input => self.input_size,
            ViewLayer::Output => self.output_size,
            ViewLayer::PaperBack => self.input_size,
        };
        let image_world = if layer == ViewLayer::Output {
            fitted_world_size(source_size.unwrap_or([1, 1]), bounds)
        } else {
            bounds
        };
        let image_rect = Rect::from_center_size(center, image_world * scale);
        let now = ui.input(|i| i.time);
        let mut frame = 0;
        if let Some(t) = self.transition.as_mut() {
            let count = if t.polaroid { 50 } else { 10 };
            if self.generation != 0 && self.accepted_generation == self.generation {
                if now - t.start >= 0.032 {
                    t.frame += 1;
                } else {
                    ui.ctx().request_repaint_after(Duration::from_secs_f64(
                        (0.032 - (now - t.start)).max(0.0),
                    ));
                }
            }
            frame = t.frame;
            if frame >= count {
                self.transition = None;
                frame = 0;
            }
        }
        let max_texture_side = ui.input(|i| i.max_texture_side).max(1) as f32;
        let texture_size = [
            (rect.width() * dpi).ceil().clamp(1.0, max_texture_side) as usize,
            (rect.height() * dpi).ceil().clamp(1.0, max_texture_side) as usize,
        ];
        let key = vec![
            self.revision,
            layer as u64,
            interpolation as u64,
            self.settings.gray_18_canvas as u64,
            padding.to_bits() as u64,
            self.zoom.to_bits() as u64,
            self.zoom_percent.map(f32::to_bits).unwrap_or(0) as u64,
            self.pan.x.to_bits() as u64,
            self.pan.y.to_bits() as u64,
            rect.width().to_bits() as u64,
            rect.height().to_bits() as u64,
            texture_size[0] as u64,
            texture_size[1] as u64,
            frame as u64,
            self.transition.is_some() as u64,
            input_float
                .map(|source| Arc::as_ptr(source) as usize as u64)
                .unwrap_or(0),
            output_float
                .map(|source| Arc::as_ptr(source) as usize as u64)
                .unwrap_or(0),
            self.compare_split.map(f32::to_bits).unwrap_or(0) as u64,
        ];
        if self.cache_key.as_ref() != Some(&key) {
            self.generation = self.generation.wrapping_add(1);
            let magnified = source_size.is_some_and(|size| {
                let factor = (self.display_max_size.max(1) as f64
                    / size[0].max(size[1]).max(1) as f64)
                    .min(1.0);
                let raster_size = size.map(|side| (side as f64 * factor).round().max(1.0) as usize);
                image_rect.width() * dpi > raster_size[0] as f32
                    || image_rect.height() * dpi > raster_size[1] as f32
            });
            let source = match layer {
                ViewLayer::Input => input_float,
                ViewLayer::Output => output_float,
                ViewLayer::PaperBack => None,
            };
            self.worker.submit(DisplayRequest {
                key: key.clone(),
                rect,
                border,
                paper_rect,
                image_rect,
                texture_size,
                generation: self.generation,
                gray: if self.settings.gray_18_canvas {
                    Color32::from_gray(118)
                } else {
                    Color32::BLACK
                },
                layer,
                interpolation,
                raster: selected.cloned(),
                paper_size,
                previous: self.transition.as_ref().and_then(|t| t.previous.clone()),
                split: if layer == ViewLayer::Output {
                    self.compare_split.and_then(|fraction| {
                        self.prepared_input
                            .as_ref()
                            .map(|p| (fraction, p.raster.clone()))
                    })
                } else {
                    None
                },
                source: source.cloned(),
                prepared: match layer {
                    ViewLayer::Input => self.prepared_input.clone(),
                    ViewLayer::Output => self.prepared_output.clone(),
                    ViewLayer::PaperBack => None,
                },
                metadata: self.display_source(layer),
                max_size: self.display_max_size.max(1),
                full_source: self.transition.is_none()
                    && (magnified || self.zoom_percent.is_some()),
                polaroid: layer == ViewLayer::Output
                    && self.transition.as_ref().is_some_and(|t| t.polaroid),
                frame,
                ctx: ui.ctx().clone(),
            });
            self.cache_key = Some(key.clone());
        }
        if let Some(response) = self.worker.take_response()
            && response.matches(self.generation, &key)
        {
            self.accepted_generation = response.generation;
            if let Some(t) = self.transition.as_mut() {
                t.start = now;
                ui.ctx().request_repaint_after(Duration::from_millis(32));
            }
            if let Some(prepared) = response.prepared {
                match layer {
                    ViewLayer::Input => {
                        self.input = Some(prepared.raster.clone());
                        self.prepared_input = Some(prepared);
                    }
                    ViewLayer::Output => {
                        self.output = Some(prepared.raster.clone());
                        self.prepared_output = Some(prepared);
                    }
                    ViewLayer::PaperBack => {}
                }
            }
            if let Some(status) = response.status {
                self.transform_status = status;
            }
            #[cfg(test)]
            {
                self.observed_image = Some(response.image.clone());
            }
            if let Some(texture) = self.texture.as_mut() {
                texture.set(response.image, TextureOptions::NEAREST);
            } else {
                self.texture = Some(ui.ctx().load_texture(
                    "viewer-presentation",
                    response.image,
                    TextureOptions::NEAREST,
                ));
            }
        }
        if let Some(texture) = self.texture.as_ref() {
            ui.painter_at(rect).image(
                texture.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if let Some((start, end)) = self.marquee {
            let selection = Rect::from_two_pos(start, end).intersect(rect);
            let painter = ui.painter_at(rect);
            painter.rect_filled(selection, 0.0, Color32::from_white_alpha(24));
            painter.rect_stroke(
                selection,
                0.0,
                egui::Stroke::new(1.0_f32, Color32::WHITE),
                egui::StrokeKind::Inside,
            );
        }
        if input_float.is_none() {
            let painter = ui.painter_at(rect);
            let message = "No image loaded\nDrop an image here, or use MAIN → Import";
            let galley = painter.layout(
                message.to_string(),
                egui::FontId::proportional(16.0),
                crate::theme::FG_SECONDARY,
                rect.width(),
            );
            let pos = rect.center() - galley.size() * 0.5;
            painter.galley(pos, galley, crate::theme::FG_SECONDARY);
        }
        let source = match layer {
            ViewLayer::Input => input_float,
            ViewLayer::Output => output_float,
            ViewLayer::PaperBack => None,
        };
        let inspection = response
            .hover_pos()
            .filter(|p| image_rect.contains(*p))
            .and_then(|p| {
                source.map(|source| {
                    let uv = (p - image_rect.min) / image_rect.size();
                    let x = (uv.x * source.width as f32).floor().max(0.0) as u32;
                    let y = (uv.y * source.height as f32).floor().max(0.0) as u32;
                    let x = x.min(source.width.saturating_sub(1));
                    let y = y.min(source.height.saturating_sub(1));
                    PixelInspection {
                        layer,
                        x,
                        y,
                        rgb: source.get(x, y).map(|v| v as f64),
                    }
                })
            });
        #[cfg(test)]
        {
            self.observed_geometry = Some((rect, image_rect, interpolation, inspection));
        }
        let text = if let Some(p) = inspection {
            format!(
                "{:?} ({}, {}) RGB {:.6}, {:.6}, {:.6}",
                p.layer, p.x, p.y, p.rgb[0], p.rgb[1], p.rgb[2]
            )
        } else {
            format!(
                "{:?} · zoom {:.2}% · click: zoom · drag: pan · shift-drag: select · double-click: fit",
                layer,
                self.zoom_percent.unwrap_or(self.zoom * 100.0)
            )
        };
        let painter = ui.painter_at(rect);
        let text_rect = Rect::from_min_size(
            rect.min + Vec2::splat(8.0),
            Vec2::new(rect.width().min(650.0) - 16.0, 24.0),
        );
        painter.rect_filled(text_rect, 3.0, Color32::from_black_alpha(180));
        painter.text(
            text_rect.min + Vec2::new(5.0, 4.0),
            egui::Align2::LEFT_TOP,
            text,
            egui::FontId::monospace(12.0),
            Color32::WHITE,
        );
        inspection
    }
}

#[cfg(test)]
mod tests {
    use super::super::colortransform::output_display_raster;
    use super::super::raster::capped_raster;
    use super::*;
    #[cfg(not(windows))]
    #[test]
    fn transform_change_updates_the_next_crossfade_at_exact_zoom() {
        let source = Arc::new(ImageBuf::from_data(
            1,
            1,
            vec![0.75 as _, 0.20 as _, 0.10 as _],
        ));
        let (transformed, _) =
            output_display_raster(&source, "Display P3", true, true, None, 1).unwrap();
        let raw = capped_raster(&source, 1).unwrap();
        assert_ne!(transformed.rgb[0], raw.rgb[0]);
        let mut viewer = Viewer::new();
        viewer.settings.white_padding = 0.0;
        viewer.set_display_max_size(1);
        viewer.set_output(transformed, [1, 1], 0.0, false);
        viewer.set_output_display_source("Display P3", true, true, None);
        viewer.seed_prepared_output(source.clone(), "Display P3", true, true, None, 1);
        viewer.set_zoom_percent(100.0);
        viewer.set_output_display_source("Display P3", true, false, None);
        let ctx = egui::Context::default();
        let (tx, rx) = std::sync::mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(32.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        viewer.show(ui, None, Some(&source));
                    });
                },
            );
            if viewer.output.as_ref().unwrap().rgb[0] == raw.rgb[0] {
                break;
            }
            rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("updated viewer repaint");
        }
        viewer.settings.crossfade = true;
        viewer.set_output(raw.clone(), [1, 1], 1.0, true);
        assert_eq!(
            viewer
                .transition
                .as_ref()
                .unwrap()
                .previous
                .as_ref()
                .unwrap()
                .rgb[0],
            raw.rgb[0]
        );
    }

    #[test]
    fn full_output_zoom_preserves_detail_and_transform_changes() {
        let source = Arc::new(ImageBuf::from_data(
            4,
            1,
            vec![
                0.0 as _, 0.0 as _, 0.0 as _, 1.0 as _, 1.0 as _, 1.0 as _, 0.0 as _, 0.0 as _,
                0.0 as _, 1.0 as _, 1.0 as _, 1.0 as _,
            ],
        ));
        let mut viewer = Viewer::new();
        viewer.set_output_display_source("sRGB", true, false, None);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(4.0, 1.0));
        let (view, _) = viewer
            .full_source_view(
                &source,
                ViewLayer::Output,
                rect,
                rect,
                [4, 1],
                Interpolation::Nearest,
            )
            .unwrap();
        assert_eq!(&*view.rgb, &[[0.0; 3], [1.0; 3], [0.0; 3], [1.0; 3]]);
        let zoomed = Rect::from_min_size(Pos2::ZERO, Vec2::new(16.0, 4.0));
        let (view, _) = viewer
            .full_source_view(
                &source,
                ViewLayer::Output,
                zoomed,
                zoomed,
                [16, 4],
                Interpolation::Nearest,
            )
            .unwrap();
        assert_eq!(view.rgb[3], [0.0; 3]);
        assert_eq!(view.rgb[4], [1.0; 3]);
        let source = Arc::new(ImageBuf::from_data(
            1,
            1,
            vec![0.18 as _, 0.18 as _, 0.18 as _],
        ));
        let revision = viewer.revision;
        viewer.set_output_display_source("sRGB", false, true, None);
        assert_ne!(viewer.revision, revision);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0));
        let (view, _) = viewer
            .full_source_view(
                &source,
                ViewLayer::Output,
                rect,
                rect,
                [1, 1],
                Interpolation::Nearest,
            )
            .unwrap();
        let (expected, _) = output_display_raster(&source, "sRGB", false, true, None, 1).unwrap();
        assert_eq!(view.rgb[0], expected.rgb[0]);
        viewer.set_output_display_source("sRGB", false, false, None);
        let (view, _) = viewer
            .full_source_view(
                &source,
                ViewLayer::Output,
                rect,
                rect,
                [1, 1],
                Interpolation::Nearest,
            )
            .unwrap();
        assert!((view.rgb[0][0] - 0.18).abs() < 1e-6);
    }
}
