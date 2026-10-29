//! Viewer presentation: zoom, pan, layer controls and pixel inspection.
//! Immutable pipeline floats are shared; viewing pixels stay disposable.
use egui::{Color32, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use serde_json::Value;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::theme;

#[cfg(windows)]
use super::colortransform::discover_display_profile;
use super::raster::{DisplayRaster, Interpolation};
use super::worker::{
    DisplayRequest, DisplaySource, DisplayWorker, InspectionRequest, InspectionResponse,
    PreparedDisplay,
};
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
/// Places presentation samples at device-pixel centers inside the viewport.
/// The returned rectangle maps those centers to bounded texture texels.
fn presentation_grid(rect: Rect, dpi: f32, max_side: usize) -> (Rect, [usize; 2]) {
    let axis = |min: f32, max: f32| {
        // Pixel indices k satisfy min*dpi <= k+0.5 < max*dpi.
        let start = (f64::from(min) * f64::from(dpi) - 0.5).ceil();
        let end = (f64::from(max) * f64::from(dpi) - 0.5).ceil();
        (start, (end - start).max(1.0))
    };
    let (x, width) = axis(rect.min.x, rect.max.x);
    let (y, height) = axis(rect.min.y, rect.max.y);
    let size = [
        width.min(max_side as f64) as usize,
        height.min(max_side as f64) as usize,
    ];
    let grid = Rect::from_min_max(
        Pos2::new((x / f64::from(dpi)) as f32, (y / f64::from(dpi)) as f32),
        Pos2::new(
            ((x + width) / f64::from(dpi)) as f32,
            ((y + height) / f64::from(dpi)) as f32,
        ),
    );
    (grid, size)
}

#[derive(Clone, Copy, Debug)]
pub struct PixelInspection {
    pub layer: ViewLayer,
    pub x: u32,
    pub y: u32,
    pub rgb: [f64; 3],
}

/// Viewer-only correspondence in the selected native image.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewerAnalysisSample {
    pub layer: ViewLayer,
    pub x: u32,
    pub y: u32,
    pub keyboard: bool,
}

#[derive(Default)]
pub(crate) struct ViewerAnalysis {
    pub enabled: bool,
    pub armed: bool,
    /// Normalized image rectangle; the caller maps its source-area identity.
    pub selection: Option<[f64; 4]>,
    /// Normalized image rectangle for plot-to-image inspection.
    pub band: Option<[f64; 4]>,
    pub sample: Option<ViewerAnalysisSample>,
    pub suspended: bool,
    /// Completed normalized image rectangle, consumed by the caller after show.
    pub completed: Option<[f64; 4]>,
    draft: Option<[f64; 4]>,
    drag_start: Option<[f64; 2]>,
    suppress_drag: bool,
    draft_subject: Option<(ViewLayer, [usize; 2])>,
    cursor: Option<[u32; 2]>,
}

fn image_point(rect: Rect, pos: Pos2) -> [f64; 2] {
    let uv = (pos - rect.min) / rect.size();
    [uv.x.clamp(0.0, 1.0) as f64, uv.y.clamp(0.0, 1.0) as f64]
}

fn image_overlay(rect: Rect, bounds: [f64; 4]) -> Rect {
    Rect::from_min_max(
        rect.min + rect.size() * Vec2::new(bounds[0] as f32, bounds[1] as f32),
        rect.min + rect.size() * Vec2::new(bounds[2] as f32, bounds[3] as f32),
    )
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

struct InspectionTarget {
    image: spektrafilm_backend::DeviceImage,
    layer: ViewLayer,
    x: u32,
    y: u32,
}
impl InspectionTarget {
    fn matches(&self, other: &Self) -> bool {
        self.layer == other.layer
            && self.x == other.x
            && self.y == other.y
            && self
                .image
                .payload()
                .zip(other.image.payload())
                .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
    }
}

#[derive(Default)]
struct InspectionState {
    target: Option<InspectionTarget>,
    generation: u64,
    value: Option<PixelInspection>,
}
impl InspectionState {
    fn set_target(&mut self, target: Option<InspectionTarget>) -> bool {
        let unchanged = match (&self.target, &target) {
            (Some(old), Some(new)) => old.matches(new),
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            return false;
        }
        self.target = target;
        self.generation = self.generation.wrapping_add(1);
        self.value = None;
        true
    }

    fn accept(&mut self, response: InspectionResponse) {
        if response.generation == self.generation
            && let Some(target) = self.target.as_ref()
        {
            self.value = response.rgb.map(|rgb| PixelInspection {
                layer: target.layer,
                x: target.x,
                y: target.y,
                rgb,
            });
        }
    }
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
    pub transform_status: crate::app::Message,
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
    retained_output: Option<spektrafilm_backend::DeviceImage>,
    retained_input: Option<spektrafilm_backend::DeviceImage>,
    uploaded_input: Option<spektrafilm_backend::DeviceImage>,
    /// Active marquee selection in view coordinates `(start, current)`; a
    /// Shift or secondary-button drag sets it and its release zooms to it.
    marquee: Option<(Pos2, Pos2)>,
    revision: u64,
    worker: DisplayWorker,
    inspection: InspectionState,
    pub(crate) analysis: ViewerAnalysis,
    /// Compute backend for the viewer's resample and display transform.
    /// `None` keeps the host reference implementation, which is the same
    /// fallback a backend reports when it has no device path.
    backend: Option<Arc<dyn ComputeBackend>>,
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
    pub(crate) fn arm_analysis_selection(&mut self) {
        self.analysis.armed = true;
        self.analysis.completed = None;
        self.analysis.draft = self.analysis.selection;
        self.analysis.drag_start = None;
        self.analysis.draft_subject = None;
        self.analysis.sample = None;
    }

    pub(crate) fn cancel_analysis_selection(&mut self) {
        self.analysis.suppress_drag |= self.analysis.drag_start.is_some();
        self.analysis.armed = false;
        self.analysis.draft = None;
        self.analysis.drag_start = None;
        self.analysis.draft_subject = None;
        self.analysis.completed = None;
    }

    pub(crate) fn analysis_retained(
        &self,
        layer: ViewLayer,
    ) -> Option<spektrafilm_backend::DeviceImage> {
        match layer {
            ViewLayer::Input => self
                .retained_input
                .clone()
                .or_else(|| self.uploaded_input.clone()),
            ViewLayer::Output => self.retained_output.clone(),
            ViewLayer::PaperBack => None,
        }
    }

    fn inspect_retained(
        &mut self,
        target: Option<InspectionTarget>,
        ctx: &egui::Context,
    ) -> Option<PixelInspection> {
        if self.inspection.set_target(target) {
            let request = self.inspection.target.as_ref().and_then(|target| {
                self.backend.as_ref().map(|backend| InspectionRequest {
                    generation: self.inspection.generation,
                    image: target.image.clone(),
                    x: target.x,
                    y: target.y,
                    backend: backend.clone(),
                    ctx: ctx.clone(),
                })
            });
            self.worker.inspect(request);
        }
        if let Some(response) = self.worker.take_inspection() {
            self.inspection.accept(response);
        }
        self.inspection.value
    }

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
    /// True until the current composition finishes or its animation ends.
    /// Completed failures are not pending; they retain the last good texture.
    pub(crate) fn presentation_pending(&self) -> bool {
        self.worker.has_pending() || self.transition.is_some()
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
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
            zoom: 1.0,
            zoom_percent: None,
            display_max_size: 1024,
            ..Self::default()
        }
    }
    /// Give the viewer a compute backend so its resample and display
    /// transform run on the device when the backend has a device path. The
    /// application binds its backend here once at construction.
    pub fn with_backend(backend: Arc<dyn ComputeBackend>) -> Self {
        Self {
            zoom: 1.0,
            zoom_percent: None,
            display_max_size: 1024,
            backend: Some(backend),
            ..Self::default()
        }
    }
    pub fn set_input(&mut self, raster: DisplayRaster, original_size: [usize; 2]) {
        self.cancel_analysis_selection();
        self.analysis.selection = None;
        self.analysis.band = None;
        self.analysis.sample = None;
        self.analysis.cursor = None;
        self.input_size = Some(original_size);
        self.input = Some(raster);
        self.output = None;
        self.prepared_input = None;
        self.prepared_output = None;
        self.retained_input = None;
        self.uploaded_input = None;
        self.retained_output = None;
        self.output_size = None;
        self.transition = None;
        self.settings.layer = ViewLayer::Input;
        self.reset_view();
        self.revision += 1;
    }
    /// Refresh this source's presentation without discarding its accepted
    /// output. Adjustment history can leave that output intentionally stale.
    pub(crate) fn refresh_input(&mut self, raster: DisplayRaster, original_size: [usize; 2]) {
        self.cancel_analysis_selection();
        self.input_size = Some(original_size);
        self.input = Some(raster);
        self.prepared_input = None;
        self.retained_input = None;
        self.uploaded_input = None;
        self.revision = self.revision.wrapping_add(1);
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
        self.cancel_analysis_selection();
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
        self.retained_output = None;
        self.output_size = Some(original_size);
        self.settings.layer = ViewLayer::Output;
        self.revision += 1;
    }
    pub fn set_input_display_source(&mut self, space: &str, encoded: bool) {
        if self.input_source_space.as_deref() != Some(space) || self.input_source_encoded != encoded
        {
            self.uploaded_input = None;
        }
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
    pub(crate) fn seed_prepared_device_input(
        &mut self,
        source: spektrafilm_backend::DeviceImage,
        _max_size: u32,
    ) {
        self.retained_input = Some(source);
        self.uploaded_input = None;
        self.prepared_input = None;
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn seed_prepared_input(&mut self, source: Arc<ImageBuf>, max_size: u32) {
        self.retained_input = None;
        self.uploaded_input = None;
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
    pub(crate) fn set_retained_output(&mut self, image: Option<spektrafilm_backend::DeviceImage>) {
        self.retained_output = image;
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
        if self.settings.layer == ViewLayer::Output && self.output.is_none() {
            self.settings.layer = ViewLayer::Input;
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
    /// Original and Modified share a quiet segment; Paper back is a separate
    /// secondary action, but still selects the same mutually exclusive layer.
    pub fn layer_controls(&mut self, ui: &mut egui::Ui) -> bool {
        let before = self.settings.layer;
        let compact = ui.available_width() < 600.0;
        ui.horizontal(|ui| {
            egui::Frame::new()
                .fill(theme::SURFACE2)
                .stroke(theme::stroke(theme::BORDER))
                .corner_radius(theme::RADIUS_CARD)
                .inner_margin(2)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.visuals_mut().selection.bg_fill = theme::SURFACE3;
                    ui.visuals_mut().selection.stroke = theme::stroke(theme::FG);
                    ui.horizontal(|ui| {
                        let input = ui
                            .selectable_value(
                                &mut self.settings.layer,
                                ViewLayer::Input,
                                crate::i18n::text(ui.ctx(), "surfaces.viewer.input"),
                            )
                            .on_hover_text(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.viewer.input_help",
                            ));
                        crate::controls::locate_row(ui, "viewer.layer.input", &input);
                        ui.add_enabled_ui(self.output.is_some(), |ui| {
                            let output =
                                ui.selectable_value(
                                    &mut self.settings.layer,
                                    ViewLayer::Output,
                                    crate::i18n::text(ui.ctx(), "surfaces.viewer.output"),
                                )
                                .on_hover_text(crate::i18n::text(
                                    ui.ctx(),
                                    "surfaces.viewer.output_help",
                                ))
                                .on_disabled_hover_text(
                                    crate::i18n::text(ui.ctx(), "surfaces.viewer.output_disabled"),
                                );
                            crate::controls::locate_row(ui, "viewer.layer.output", &output);
                        });
                    });
                });
            ui.separator();
            let paper_label = crate::i18n::text(ui.ctx(), "surfaces.viewer.paper_back");
            let paper_back = if compact {
                crate::controls::icon_button(
                    ui,
                    paper_label.as_ref(),
                    crate::controls::Icon::Layers,
                )
            } else {
                ui.add(
                    egui::Button::new(paper_label)
                        .selected(self.settings.layer == ViewLayer::PaperBack),
                )
            }
            .on_hover_text(crate::i18n::text(
                ui.ctx(),
                "surfaces.viewer.paper_back_help",
            ));
            ui.ctx().accesskit_node_builder(paper_back.id, |node| {
                node.set_toggled(if self.settings.layer == ViewLayer::PaperBack {
                    egui::accesskit::Toggled::True
                } else {
                    egui::accesskit::Toggled::False
                });
            });
            crate::controls::locate_row(ui, "viewer.layer.paper_back", &paper_back);
            if paper_back.clicked() {
                self.settings.layer = ViewLayer::PaperBack;
            }
        });
        before != self.settings.layer
    }
    /// One nonwrapping footer row: layer segments and paper/transition actions
    /// on the left, with the retained-view controls aligned to the right.
    /// Transitions stay independent; all actions affect viewing only, never
    /// retained, saved or exported pixels.
    pub fn footer_controls(&mut self, ui: &mut egui::Ui, analysis_visible: &mut bool) {
        ui.allocate_ui_with_layout(
            ui.available_size(),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_1;
                self.layer_controls(ui);
                self.transition_menu(ui);
                ui.add_space(theme::SPACE_GROUP);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let analysis_label = crate::i18n::text(ui.ctx(), "surfaces.analysis.title");
                    if ui
                        .selectable_label(*analysis_visible, analysis_label)
                        .clicked()
                    {
                        *analysis_visible = !*analysis_visible;
                    }
                    ui.separator();
                    let reset_label = crate::i18n::text(ui.ctx(), "surfaces.viewer.reset");
                    let reset = crate::controls::icon_button(
                        ui,
                        reset_label.as_ref(),
                        crate::controls::Icon::Reset,
                    )
                    .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.viewer.reset_help"));
                    crate::controls::locate_row(ui, "viewer.reset", &reset);
                    if reset.clicked() {
                        self.reset_view();
                        ui.ctx().request_repaint();
                    }
                    let fit = ui
                        .button(crate::i18n::text(ui.ctx(), "surfaces.viewer.fit"))
                        .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.viewer.fit_help"));
                    crate::controls::locate_row(ui, "viewer.fit", &fit);
                    if fit.clicked() {
                        self.fit_view();
                        ui.ctx().request_repaint();
                    }
                    self.zoom_menu(ui);
                    ui.separator();
                });
            },
        );
    }
    /// The accepted-result transitions: two independent booleans in one
    /// labelled popover, never one exclusive choice. A pending control-search
    /// locate for either toggle opens the popover, so the hidden rows stay
    /// reachable and focusable like any other control.
    fn transition_menu(&mut self, ui: &mut egui::Ui) {
        let popup_id = ui.make_persistent_id("viewer-transitions-menu");
        let located = ui
            .ctx()
            .data(|data| data.get_temp::<(String, String)>(egui::Id::new("control_focus_request")))
            .is_some_and(|(section, target)| {
                section == "Viewer"
                    && [
                        ("viewer.reveal", "reveal transition"),
                        ("viewer.crossfade", "crossfade transition"),
                    ]
                    .into_iter()
                    .any(|(path, label)| target == path || target.eq_ignore_ascii_case(label))
            });
        if located {
            ui.memory_mut(|memory| memory.open_popup(popup_id));
        }
        let label = crate::i18n::text(ui.ctx(), "surfaces.viewer.transitions");
        let menu = if ui.available_width() < 360.0 {
            crate::controls::icon_button(ui, label.as_ref(), crate::controls::Icon::Transitions)
        } else {
            ui.button(label)
        }
        .on_hover_text(crate::i18n::text(
            ui.ctx(),
            "surfaces.viewer.transitions_help",
        ));
        if menu.clicked() {
            ui.memory_mut(|memory| memory.toggle_popup(popup_id));
        }
        // The footer sits at the window bottom, so the popover opens above its
        // button instead of dropping below the fold.
        egui::popup::popup_above_or_below_widget(
            ui,
            popup_id,
            &menu,
            egui::AboveOrBelow::Above,
            if located {
                // The locator's activating click is outside this new popup.
                // Ignore it only in the opening frame, then close normally.
                egui::popup::PopupCloseBehavior::IgnoreClicks
            } else {
                egui::popup::PopupCloseBehavior::CloseOnClickOutside
            },
            |ui| {
                ui.set_min_width(240.0);
                let reveal = crate::controls::toggle_row(
                    ui,
                    "reveal transition",
                    &mut self.settings.reveal,
                    "Viewer-only wipe between input and output; retained and written pixels are unchanged.",
                );
                crate::controls::locate_row(ui, "viewer.reveal", &reveal);
                let crossfade = crate::controls::toggle_row(
                    ui,
                    "crossfade transition",
                    &mut self.settings.crossfade,
                    "Viewer-only transition between accepted outputs.",
                );
                crate::controls::locate_row(ui, "viewer.crossfade", &crossfade);
            },
        );
    }
    /// The exact source-pixel zoom choices in one menu; the menu button carries
    /// the current zoom as read-only text. Fit stays a separate button, so the
    /// menu holds no duplicate fit action. A pending control-search locate for
    /// an exact zoom opens the menu, so the hidden choices stay reachable and
    /// focusable like any other control.
    fn zoom_menu(&mut self, ui: &mut egui::Ui) {
        let popup_id = ui.make_persistent_id("viewer-zoom-menu");
        let located = ui
            .ctx()
            .data(|data| data.get_temp::<(String, String)>(egui::Id::new("control_focus_request")))
            .is_some_and(|(section, target)| {
                section == "Viewer"
                    && [
                        ("viewer.zoom.100", "100%"),
                        ("viewer.zoom.200", "200%"),
                        ("viewer.zoom.400", "400%"),
                    ]
                    .into_iter()
                    .any(|(path, label)| target == path || target.eq_ignore_ascii_case(label))
            });
        if located {
            ui.memory_mut(|memory| memory.open_popup(popup_id));
        }
        // Read-only: the current zoom, carried by the menu button.
        let zoom = self.zoom_percent().map_or_else(
            || {
                crate::tr!(
                    ui.ctx(),
                    "surfaces.viewer.zoom_fit",
                    percent = format!("{:.0}", self.zoom * 100.0)
                )
                .into_owned()
            },
            |percent| format!("{percent:.0}%"),
        );
        let accessible_name = crate::tr!(ui.ctx(), "surfaces.viewer.zoom", zoom = zoom.as_str());
        let menu = ui
            .add(
                egui::Button::new(zoom.as_str())
                    .min_size(egui::vec2(theme::WIDTH_VALUE, theme::HEIGHT_CONTROL)),
            )
            .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.viewer.zoom_help"));
        // Keep the canonical action name, including the zoom prefix, separate
        // from the compact visible readout. Do not localize under Context locks.
        menu.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                accessible_name.as_ref(),
            )
        });
        ui.ctx().accesskit_node_builder(menu.id, |node| {
            node.set_label(accessible_name.as_ref());
        });
        crate::controls::focus_feedback(ui, &menu);
        if menu.clicked() {
            ui.memory_mut(|memory| memory.toggle_popup(popup_id));
        }
        // The footer sits at the window bottom, so the menu opens above its
        // button instead of dropping below the fold.
        egui::popup::popup_above_or_below_widget(
            ui,
            popup_id,
            &menu,
            egui::AboveOrBelow::Above,
            if located {
                // The locator's activating click is outside this new popup.
                // Ignore it only in the opening frame, then close normally.
                egui::popup::PopupCloseBehavior::IgnoreClicks
            } else {
                egui::popup::PopupCloseBehavior::CloseOnClickOutside
            },
            |ui| {
                for (label, percent, path) in [
                    ("100%", 100.0, "viewer.zoom.100"),
                    ("200%", 200.0, "viewer.zoom.200"),
                    ("400%", 400.0, "viewer.zoom.400"),
                ] {
                    let response = ui.button(label).on_hover_text(crate::tr!(
                        ui.ctx(),
                        "surfaces.viewer.zoom_exact_help",
                        percent = format!("{percent:.0}")
                    ));
                    crate::controls::locate_row(ui, path, &response);
                    if response.clicked() {
                        self.set_zoom_percent(percent);
                        ui.ctx().request_repaint();
                        ui.memory_mut(|memory| memory.close_popup());
                    }
                }
            },
        );
    }
    pub fn interpolation_control(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(crate::i18n::text(ui.ctx(), "surfaces.viewer.interpolation"));
            let width = ui
                .available_width()
                .clamp(theme::WIDTH_COMBO_MIN, theme::WIDTH_COMBO_MAX);
            ui.add_space((ui.available_width() - width).max(0.0));
            let response = egui::ComboBox::from_id_salt("output interpolation")
                .width(width)
                .selected_text(crate::controls::localized(
                    ui.ctx(),
                    self.settings.interpolation.name(),
                ))
                .show_ui(ui, |ui| {
                    for name in INTERPOLATIONS {
                        ui.selectable_value(
                            &mut self.settings.interpolation,
                            Interpolation::parse(name),
                            crate::controls::localized(ui.ctx(), name),
                        );
                    }
                })
                .response
                .on_hover_text(crate::i18n::text(
                    ui.ctx(),
                    "surfaces.viewer.interpolation_help",
                ));
            crate::controls::name_choice(ui, "output interpolation");
            crate::controls::locate_row(ui, "extras.display.output_interpolation", &response);
        });
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
    ) -> Option<(DisplayRaster, Option<crate::app::Message>)> {
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
        input_source: Option<&crate::render::InputImage>,
        output_float: Option<&Arc<ImageBuf>>,
    ) -> Option<PixelInspection> {
        let input_float = input_source.and_then(|source| source.host_image());
        let response = ui.allocate_response(
            ui.available_size().max(Vec2::splat(1.0)),
            egui::Sense::click_and_drag(),
        );
        let rect = response.rect;
        if self.zoom <= 0.0 {
            self.zoom = 1.0;
        }
        let gesture_layer = self.settings.layer;
        let gesture_size = match gesture_layer {
            ViewLayer::Input => self.input_size,
            ViewLayer::Output => self.output_size,
            ViewLayer::PaperBack => None,
        };
        let gesture_bounds =
            normalized_world_size(self.input_size.or(self.output_size).unwrap_or([3, 2]));
        let gesture_world = if gesture_layer == ViewLayer::Output {
            fitted_world_size(gesture_size.unwrap_or([1, 1]), gesture_bounds)
        } else {
            gesture_bounds
        };
        let gesture_scale = if let Some(percent) = self.zoom_percent {
            let size = gesture_size.unwrap_or([3, 2]);
            let px_world = (gesture_world.x / size[0].max(1) as f32)
                .min(gesture_world.y / size[1].max(1) as f32);
            (percent / 100.0) / (ui.ctx().pixels_per_point().max(0.01) * px_world)
        } else {
            let padded = gesture_bounds + Vec2::splat(2.0 * self.settings.white_padding.max(0.0));
            (rect.width() / padded.x).min(rect.height() / padded.y) * self.zoom
        };
        let gesture_image =
            Rect::from_center_size(rect.center() + self.pan, gesture_world * gesture_scale);
        if !self.analysis.enabled || gesture_size.is_none() {
            self.cancel_analysis_selection();
        } else if self.analysis.armed
            && let Some(size) = gesture_size
        {
            let subject = (gesture_layer, size);
            if self
                .analysis
                .draft_subject
                .is_some_and(|previous| previous != subject)
            {
                self.cancel_analysis_selection();
            } else {
                self.analysis.draft_subject = Some(subject);
            }
        }
        if self.analysis.armed && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel_analysis_selection();
        }
        let mut analysis_drag = self.analysis.drag_start.is_some() || self.analysis.suppress_drag;
        if self.analysis.armed
            && response.drag_started_by(egui::PointerButton::Primary)
            && !ui.input(|i| i.modifiers.shift)
            && let Some(start) = ui.input(|i| i.pointer.press_origin())
            && super::worker::contains_half_open(gesture_image, start)
        {
            self.analysis.drag_start = Some(image_point(gesture_image, start));
            analysis_drag = true;
        }
        if let Some(start) = self.analysis.drag_start
            && let Some(pos) = response
                .interact_pointer_pos()
                .or_else(|| ui.input(|i| i.pointer.latest_pos()))
        {
            let end = image_point(gesture_image, pos);
            self.analysis.draft = Some([
                start[0].min(end[0]),
                start[1].min(end[1]),
                start[0].max(end[0]),
                start[1].max(end[1]),
            ]);
        }
        if analysis_drag && response.drag_stopped_by(egui::PointerButton::Primary) {
            let draft = self.analysis.draft;
            self.analysis.drag_start = None;
            if let (Some(bounds), Some(size)) = (draft, gesture_size)
                && nonempty_image_area(bounds, size)
            {
                self.cancel_analysis_selection();
                self.analysis.completed = Some(bounds);
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            self.analysis.suppress_drag = false;
        }
        // Viewer gestures change the retained view only. Wheel and pinch zoom
        // at the pointer; the primary button pans; a Shift or secondary-button
        // drag selects a region that fills the view on release; one click
        // steps the zoom at the pointer; a double click resets the view.
        let selecting = ui
            .input(|i| i.modifiers.shift || i.pointer.button_down(egui::PointerButton::Secondary));
        if response.drag_started()
            && selecting
            && !analysis_drag
            && let Some(pos) = response.interact_pointer_pos()
        {
            // The drag starts a frame after the press, so the selection
            // begins where the press began.
            let start = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            self.marquee = Some((start, pos));
        }
        let marquee_drag = response.dragged() && selecting && !analysis_drag;
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
        if response.hovered() && !analysis_drag {
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
        if response.dragged() && !marquee_drag && !analysis_drag && self.marquee.is_none() {
            self.pan += response.drag_delta();
        }
        if response.double_clicked() && !analysis_drag {
            self.reset_view();
            self.marquee = None;
        } else if response.clicked()
            && selection.is_none()
            && !analysis_drag
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
        let max_texture_side = ui.input(|i| i.max_texture_side).max(1) as usize;
        let (sampling_rect, texture_size) = presentation_grid(rect, dpi, max_texture_side);
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
            dpi.to_bits() as u64,
            rect.min.x.to_bits() as u64,
            rect.min.y.to_bits() as u64,
            rect.width().to_bits() as u64,
            rect.height().to_bits() as u64,
            texture_size[0] as u64,
            texture_size[1] as u64,
            frame as u64,
            self.transition.is_some() as u64,
            input_source
                .map(|source| match source {
                    crate::render::InputImage::Host(image) => Arc::as_ptr(image) as usize as u64,
                    crate::render::InputImage::Device(image) => image
                        .payload()
                        .map(|p| Arc::as_ptr(p) as *const () as usize as u64)
                        .unwrap_or(0),
                })
                .unwrap_or(0),
            output_float
                .map(|source| Arc::as_ptr(source) as usize as u64)
                .or_else(|| {
                    self.retained_output
                        .as_ref()
                        .and_then(|i| i.payload())
                        .map(|p| Arc::as_ptr(p) as *const () as usize as u64)
                })
                .unwrap_or(0),
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
                rect: sampling_rect,
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
                source: source.cloned(),
                retained_source: if layer == ViewLayer::Output {
                    self.retained_output.clone()
                } else if layer == ViewLayer::Input {
                    self.retained_input
                        .clone()
                        .or_else(|| self.uploaded_input.clone())
                } else {
                    None
                },
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
                backend: self
                    .backend
                    .clone()
                    .unwrap_or_else(|| Arc::new(spektrafilm_backend::cpu_backend::CpuBackend)),
                ctx: ui.ctx().clone(),
            });
            self.cache_key = Some(key.clone());
        }
        if let Some(response) = self.worker.take_response()
            && response.matches(self.generation, &key)
        {
            if let Some(error) = response.error.as_ref() {
                self.transform_status =
                    crate::app::Message::new("surfaces.display.error").arg("error", error);
                self.transition = None;
            } else {
                self.accepted_generation = response.generation;
                if layer == ViewLayer::Input
                    && let Some(uploaded) = response.uploaded_input
                {
                    self.uploaded_input = Some(uploaded);
                }
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
        }
        if let Some(texture) = self.texture.as_ref() {
            ui.painter_at(rect).image(
                texture.id(),
                sampling_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        self.analysis.sample = None;
        self.analysis.suspended = self.analysis.enabled
            && (self.analysis.armed
                || analysis_drag
                || response.dragged()
                || self.transition.is_some()
                || (response.hovered()
                    && ui.input(|i| i.zoom_delta() != 1.0 || i.smooth_scroll_delta.y != 0.0)));
        let mut keyboard_cursor_focused = false;
        if self.analysis.enabled
            && layer != ViewLayer::PaperBack
            && let Some(size) = source_size
        {
            if let Some(band) = self.analysis.band {
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
            if let Some(area) = self
                .analysis
                .draft
                .filter(|_| self.analysis.armed)
                .or(self.analysis.selection)
            {
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
                if self.analysis.armed {
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
                        let area = self.analysis.draft.get_or_insert([0.25, 0.25, 0.75, 0.75]);
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
                            self.cancel_analysis_selection();
                            self.analysis.completed = Some(completed);
                        }
                    }
                } else {
                    let cursor = ui.button(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.viewer.analysis_cursor",
                    ));
                    if cursor.clicked() {
                        cursor.request_focus();
                    }
                    if cursor.has_focus() {
                        ui.memory_mut(|memory| {
                            memory.set_focus_lock_filter(
                                cursor.id,
                                egui::EventFilter {
                                    horizontal_arrows: true,
                                    vertical_arrows: true,
                                    ..Default::default()
                                },
                            );
                        });
                    }
                    if cursor.has_focus()
                        && !cursor.hovered()
                        && response
                            .hover_pos()
                            .is_some_and(|p| super::worker::contains_half_open(image_rect, p))
                        && ui.input(|i| i.pointer.delta() != Vec2::ZERO)
                    {
                        cursor.surrender_focus();
                    }
                    keyboard_cursor_focused = cursor.has_focus();
                    if keyboard_cursor_focused {
                        let point = self
                            .analysis
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
                        point[0] = (point[0] as i64 + dx).clamp(0, size[0].saturating_sub(1) as i64)
                            as u32;
                        point[1] = (point[1] as i64 + dy).clamp(0, size[1].saturating_sub(1) as i64)
                            as u32;
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
            });
            if !self.analysis.suspended {
                let pixel = if keyboard_cursor_focused {
                    self.analysis.cursor.map(|p| (p, true))
                } else {
                    response
                        .hover_pos()
                        .filter(|p| super::worker::contains_half_open(image_rect, *p))
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
                self.analysis.sample = pixel.map(|(point, keyboard)| ViewerAnalysisSample {
                    layer,
                    x: point[0],
                    y: point[1],
                    keyboard,
                });
            }
        }
        if let Some((start, end)) = self.marquee {
            let selection = Rect::from_two_pos(start, end).intersect(rect);
            let painter = ui.painter_at(rect);
            painter.rect_filled(selection, 0.0, crate::theme::FG.gamma_multiply(0.1));
            painter.rect_stroke(
                selection,
                0.0,
                crate::theme::stroke(crate::theme::FG),
                egui::StrokeKind::Inside,
            );
        }
        if input_source.is_none() {
            let painter = ui.painter_at(rect);
            let message = crate::i18n::text(ui.ctx(), "surfaces.viewer.empty");
            let galley = painter.layout(
                message.to_string(),
                crate::theme::text(egui::TextStyle::Body, crate::theme::SIZE_TITLE),
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
        let mut host_inspection = None;
        let target = response
            .hover_pos()
            .filter(|p| super::worker::contains_half_open(image_rect, *p))
            .and_then(|p| {
                let image = match layer {
                    ViewLayer::Input => self.retained_input.as_ref(),
                    ViewLayer::Output => self.retained_output.as_ref(),
                    ViewLayer::PaperBack => None,
                };
                let host = source
                    .map(Arc::as_ref)
                    .or_else(|| image.and_then(|image| image.host_image()));
                let size = host
                    .map(|host| [host.width, host.height])
                    .or_else(|| image.map(|image| [image.width(), image.height()]))?;
                let uv = (p - image_rect.min) / image_rect.size();
                let x = ((uv.x * size[0] as f32).floor().max(0.0) as u32)
                    .min(size[0].saturating_sub(1));
                let y = ((uv.y * size[1] as f32).floor().max(0.0) as u32)
                    .min(size[1].saturating_sub(1));
                if let Some(host) = host {
                    host_inspection = Some(PixelInspection {
                        layer,
                        x,
                        y,
                        rgb: host.get(x, y).map(|v| v as f64),
                    });
                    None
                } else {
                    Some(InspectionTarget {
                        image: image?.clone(),
                        layer,
                        x,
                        y,
                    })
                }
            });
        let inspection = self.inspect_retained(target, ui.ctx()).or(host_inspection);
        #[cfg(test)]
        {
            self.observed_geometry = Some((rect, image_rect, interpolation, inspection));
        }
        // Pixel inspection is the only chrome painted over the image, and it
        // sits along the bottom edge: the parent's result and rendering status
        // strips own the top of the viewer. Gesture help is documented at the
        // top of `show` and lives in the application help, not over the canvas.
        if let Some(p) = inspection {
            let text = crate::tr!(
                ui.ctx(),
                "surfaces.viewer.inspection",
                layer = crate::i18n::text(
                    ui.ctx(),
                    match p.layer {
                        ViewLayer::Input => "surfaces.viewer.input",
                        ViewLayer::Output => "surfaces.viewer.output",
                        ViewLayer::PaperBack => "surfaces.viewer.paper_back",
                    }
                ),
                x = p.x,
                y = p.y,
                r = format!("{:.6}", p.rgb[0]),
                g = format!("{:.6}", p.rgb[1]),
                b = format!("{:.6}", p.rgb[2])
            );
            let painter = ui.painter_at(rect);
            let text_rect = Rect::from_min_size(
                Pos2::new(
                    rect.min.x + crate::theme::SPACE_2,
                    (rect.max.y - 24.0 - crate::theme::SPACE_2)
                        .max(rect.min.y + crate::theme::SPACE_2),
                ),
                Vec2::new(rect.width().min(650.0) - 2.0 * crate::theme::SPACE_2, 24.0),
            );
            // Viewer-only chrome over the image. The token set has no translucent
            // overlay-scrim role, so the panel surface is the closest existing
            // token at reduced alpha.
            painter.rect_filled(
                text_rect,
                crate::theme::RADIUS_CARD,
                crate::theme::SURFACE.gamma_multiply(0.85),
            );
            painter.text(
                text_rect.min + Vec2::splat(crate::theme::SPACE_1),
                egui::Align2::LEFT_TOP,
                text,
                crate::theme::text(crate::theme::TEXT_READOUT, crate::theme::SIZE_HINT),
                crate::theme::FG,
            );
        }
        inspection
    }
}

#[cfg(test)]
mod tests {
    use super::super::colortransform::output_display_raster_with;
    use super::super::raster::capped_raster;
    use super::*;
    use spektrafilm_backend::cpu_backend::CpuBackend;

    #[test]
    fn selection_requires_native_pixel_centers() {
        assert!(!nonempty_image_area([0.0, 0.0, 0.1, 1.0], [4, 2]));
        assert!(nonempty_image_area([0.0, 0.0, 0.1251, 1.0], [4, 2]));
        assert!(!nonempty_image_area([0.125, 0.0, 0.125, 1.0], [4, 2]));
        assert!(nonempty_image_area([0.0, 0.0, 1.0, 1.0], [1, 1]));
    }

    #[test]
    fn cancel_preserves_committed_selection_and_consumes_ongoing_drag() {
        let mut viewer = Viewer::new();
        let selection = [0.1, 0.2, 0.7, 0.8];
        viewer.analysis.selection = Some(selection);
        viewer.arm_analysis_selection();
        viewer.analysis.drag_start = Some([0.2, 0.3]);
        viewer.analysis.draft = Some([0.2, 0.3, 0.4, 0.5]);
        viewer.cancel_analysis_selection();
        assert_eq!(viewer.analysis.selection, Some(selection));
        assert!(!viewer.analysis.armed);
        assert!(viewer.analysis.draft.is_none());
        assert!(viewer.analysis.suppress_drag);
        assert!(viewer.analysis.completed.is_none());
    }

    #[test]
    fn retained_hover_reuses_raw_values_and_rejects_stale_movement_layer_and_source() {
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let backend: Arc<dyn ComputeBackend> = Arc::new(gpu.derived_explicit());
        let meta = spektrafilm_backend::RetainedImageMeta {
            width: 2,
            height: 1,
            domain: spektrafilm_backend::SampleDomain::Linear,
            precision: spektrafilm_backend::Precision::host(),
            space: spektrafilm_math::colorspace::resolve("Display P3").unwrap(),
            pixel_pitch_um: None,
        };
        let source = ImageBuf::from_data(
            2,
            1,
            vec![
                -0.25 as _, 1.5 as _, 0.5 as _, 0.75 as _, 0.0 as _, 1.0 as _,
            ],
        );
        let image = backend.retain_image(&source, &meta).unwrap();
        let replacement = backend.retain_image(&source, &meta).unwrap();
        let mut viewer = Viewer::with_backend(backend);
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |_| {});
        }
        let (tx, rx) = std::sync::mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = tx.send(());
        });
        let target = |image: &spektrafilm_backend::DeviceImage, layer, x| InspectionTarget {
            image: image.clone(),
            layer,
            x,
            y: 0,
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let inspected = loop {
            if let Some(value) =
                viewer.inspect_retained(Some(target(&image, ViewLayer::Output, 0)), &ctx)
            {
                break value;
            }
            rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("retained hover worker repaint");
        };
        assert_eq!(
            inspected.rgb,
            [-0.25, 1.5, 0.5],
            "raw, not display-transformed RGB"
        );
        let accepted_generation = viewer.inspection.generation;
        for _ in 0..3 {
            let reused = viewer
                .inspect_retained(Some(target(&image, ViewLayer::Output, 0)), &ctx)
                .unwrap();
            assert_eq!(reused.rgb, inspected.rgb);
            assert_eq!(viewer.inspection.generation, accepted_generation);
        }

        let mut stale_generation = accepted_generation;
        for changed in [
            Some(target(&image, ViewLayer::Output, 1)),
            Some(target(&replacement, ViewLayer::Output, 1)),
            Some(target(&replacement, ViewLayer::Input, 1)),
            None,
        ] {
            assert!(viewer.inspection.set_target(changed));
            assert!(viewer.inspection.value.is_none());
            viewer.inspection.accept(InspectionResponse {
                generation: stale_generation,
                rgb: Some(inspected.rgb),
            });
            assert!(
                viewer.inspection.value.is_none(),
                "stale hover must stay hidden"
            );
            stale_generation = viewer.inspection.generation;
        }
        viewer.inspection.accept(InspectionResponse {
            generation: stale_generation,
            rgb: Some(inspected.rgb),
        });
        assert!(
            viewer.inspection.value.is_none(),
            "leaving the image clears inspection"
        );
    }

    #[test]
    fn host_carrier_hover_looks_up_raw_samples_without_a_backend_readback() {
        let source = ImageBuf::from_data(1, 1, vec![-0.25 as _, 1.5 as _, 0.5 as _]);
        let image = spektrafilm_backend::DeviceImage::host(
            source.clone(),
            spektrafilm_backend::RetainedImageMeta {
                width: 1,
                height: 1,
                domain: spektrafilm_backend::SampleDomain::Linear,
                precision: spektrafilm_backend::Precision::host(),
                space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
                pixel_pitch_um: None,
            },
        );
        let mut viewer = Viewer::with_backend(Arc::new(CpuBackend));
        viewer.set_output(capped_raster(&source, 1).unwrap(), [1, 1], 0.0, false);
        viewer.set_retained_output(Some(image));
        let ctx = egui::Context::default();
        let mut hover = None;
        for _ in 0..2 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(64.0))),
                    events: hover.map(egui::Event::PointerMoved).into_iter().collect(),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        viewer.show(ui, None, None);
                    });
                },
            );
            hover = Some(viewer.observation().image_rect.center());
        }
        let inspected = viewer.observation().inspection.expect("host carrier hover");
        assert_eq!(inspected.rgb, [-0.25, 1.5, 0.5]);
        assert!(viewer.inspection.target.is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn transform_change_updates_the_next_crossfade_at_exact_zoom() {
        let source = Arc::new(ImageBuf::from_data(
            1,
            1,
            vec![0.75 as _, 0.20 as _, 0.10 as _],
        ));
        let (transformed, _) =
            output_display_raster_with(&CpuBackend, &source, "Display P3", true, true, None, 1)
                .unwrap();
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
    fn capped_viewport_paints_the_sample_grid_on_cpu_and_gpu() {
        let device = spektrafilm_gpu::test_support::explicit_adapter();
        let gpu: Arc<dyn ComputeBackend> = Arc::new(device.derived_explicit());
        for backend in [Arc::new(CpuBackend) as Arc<dyn ComputeBackend>, gpu] {
            let source = Arc::new(ImageBuf::from_data(
                4,
                1,
                [1.0 as _, 0.0 as _, 0.0 as _].repeat(4),
            ));
            let raster =
                output_display_raster_with(backend.as_ref(), &source, "sRGB", true, false, None, 4)
                    .unwrap()
                    .0;
            let mut viewer = Viewer::with_backend(backend);
            viewer.set_output(raster, [4, 1], 0.0, false);
            viewer.set_output_display_source("sRGB", true, false, None);
            viewer.set_zoom_percent(100.0);
            viewer.pan = Vec2::new(1.25, 0.25);
            let viewport = Rect::from_min_max(Pos2::new(0.25, 0.25), Pos2::new(4097.25, 8.25));
            let at = Pos2::new(2048.5, 4.5);
            let ctx = egui::Context::default();
            let (tx, rx) = std::sync::mpsc::channel();
            ctx.set_request_repaint_callback(move |_| {
                let _ = tx.send(());
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut shown_frames = 0;
            let output = loop {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(4100.0, 10.0))),
                        max_texture_side: Some(4096),
                        events: vec![egui::Event::PointerMoved(at)],
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                let mut child =
                                    ui.new_child(egui::UiBuilder::new().max_rect(viewport));
                                child.set_clip_rect(viewport);
                                viewer.show(&mut child, None, Some(&source));
                            });
                    },
                );
                shown_frames += 1;
                if viewer.generation != 0 && viewer.generation == viewer.accepted_generation {
                    // egui hit-tests the previous frame's widgets. A fast worker can
                    // finish in the first show, before the viewer can receive hover.
                    if shown_frames >= 2 {
                        break output;
                    }
                    continue;
                }
                rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .expect("capped viewer repaint");
            };
            let observation = viewer.observation();
            assert_eq!(observation.viewport, viewport);
            assert_eq!(observation.image_rect.min, Pos2::new(2048.0, 4.0));
            assert!(
                observation.inspection.is_some(),
                "hover lies inside the image"
            );
            let texture = viewer.texture.as_ref().unwrap();
            let shape = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Mesh(mesh) = &shape.shape {
                        (mesh.texture_id == texture.id()).then_some((shape.clip_rect, mesh))
                    } else {
                        None
                    }
                })
                .expect("viewer presentation mesh");
            assert_eq!(shape.0, viewport);
            let painted = shape.1.calc_bounds();
            let pixels = observation.pixels.unwrap();
            assert_eq!(pixels.size, [4096, 8]);
            let uv = (at - painted.min) / painted.size();
            let x = (uv.x * pixels.size[0] as f32).floor() as usize;
            let y = (uv.y * pixels.size[1] as f32).floor() as usize;
            assert_eq!(
                pixels.pixels[y * pixels.size[0] + x],
                Color32::RED,
                "the pixel under an inside-image hover must not show the canvas"
            );
        }
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
        let (expected, _) =
            output_display_raster_with(&CpuBackend, &source, "sRGB", false, true, None, 1).unwrap();
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
