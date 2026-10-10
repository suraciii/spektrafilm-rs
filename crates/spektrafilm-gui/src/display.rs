// Viewer presentation. Pipeline floats are borrowed for inspection and bounded
// full-resolution viewport sampling; DisplayRaster remains disposable fit data.
use egui::{Color32, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use serde_json::Value;
use spektrafilm_math::image::ImageBuf;
use std::path::{Path, PathBuf};
use std::{sync::Arc, time::Duration};

pub const ANIMATION_MAX_PIXELS: usize = 1_500_000;
pub const INTERPOLATIONS: [&str; 7] = [
    "nearest", "linear", "cubic", "spline16", "spline36", "lanczos", "blackman",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewLayer {
    Input,
    #[default]
    Output,
    PaperBack,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    Nearest,
    Linear,
    Cubic,
    Spline16,
    #[default]
    Spline36,
    Lanczos,
    Blackman,
}
impl Interpolation {
    pub fn parse(s: &str) -> Self {
        match s {
            "nearest" => Self::Nearest,
            "linear" => Self::Linear,
            "cubic" => Self::Cubic,
            "spline16" => Self::Spline16,
            "lanczos" => Self::Lanczos,
            "blackman" => Self::Blackman,
            _ => Self::Spline36,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Linear => "linear",
            Self::Cubic => "cubic",
            Self::Spline16 => "spline16",
            Self::Spline36 => "spline36",
            Self::Lanczos => "lanczos",
            Self::Blackman => "blackman",
        }
    }
    fn radius(self) -> i32 {
        match self {
            Self::Nearest | Self::Linear => 1,
            Self::Cubic | Self::Spline16 => 2,
            Self::Spline36 => 3,
            _ => 4,
        }
    }
    pub fn weight(self, x: f32) -> f32 {
        let x = x.abs();
        if x >= self.radius() as f32 {
            return 0.0;
        }
        match self {
            Self::Nearest => {
                if x < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Linear => 1.0 - x,
            Self::Cubic => {
                if x < 1.0 {
                    (4.0 - 6.0 * x * x + 3.0 * x * x * x) / 6.0
                } else {
                    (2.0 - x).powi(3) / 6.0
                }
            }
            Self::Spline16 => {
                if x < 1.0 {
                    ((x - 1.8) * x - 0.2) * x + 1.0
                } else {
                    let y = x - 1.0;
                    ((-y / 3.0 + 0.8) * y - 7.0 / 15.0) * y
                }
            }
            Self::Spline36 => {
                if x < 1.0 {
                    ((13.0 / 11.0 * x - 453.0 / 209.0) * x - 3.0 / 209.0) * x + 1.0
                } else if x < 2.0 {
                    let y = x - 1.0;
                    ((-6.0 / 11.0 * y + 270.0 / 209.0) * y - 156.0 / 209.0) * y
                } else {
                    let y = x - 2.0;
                    ((y / 11.0 - 45.0 / 209.0) * y + 26.0 / 209.0) * y
                }
            }
            Self::Lanczos | Self::Blackman => {
                if x < 1e-7 {
                    return 1.0;
                }
                let p = std::f32::consts::PI * x;
                let q = p / 4.0;
                let window = if self == Self::Lanczos {
                    q.sin() / q
                } else {
                    0.42 + 0.5 * q.cos() + 0.08 * (2.0 * q).cos()
                };
                p.sin() / p * window
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DisplaySettings {
    pub layer: ViewLayer,
    pub interpolation: Interpolation,
    pub gray_18_canvas: bool,
    /// Padding on each side, as a fraction of the INPUT long edge.
    pub white_padding: f32,
    pub reveal: bool,
    pub crossfade: bool,
    pub use_display_transform: bool,
}
impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            layer: ViewLayer::Output,
            interpolation: Interpolation::Spline36,
            gray_18_canvas: true,
            white_padding: 0.03,
            reveal: true,
            crossfade: true,
            use_display_transform: true,
        }
    }
}
impl DisplaySettings {
    pub fn from_json(v: &Value) -> Self {
        let mut s = Self::default();
        s.gray_18_canvas = v["gray_18_canvas"].as_bool().unwrap_or(s.gray_18_canvas);
        s.white_padding = v["white_padding"]
            .as_f64()
            .filter(|x| x.is_finite())
            .unwrap_or(0.03)
            .clamp(0.0, 1.0) as f32;
        s.interpolation =
            Interpolation::parse(v["output_interpolation"].as_str().unwrap_or("spline36"));
        s.use_display_transform = v["use_display_transform"]
            .as_bool()
            .unwrap_or(s.use_display_transform);
        s.reveal = v["reveal"].as_bool().unwrap_or(true);
        s.crossfade = v["crossfade"].as_bool().unwrap_or(true);
        s.layer = match v["viewer_layer"].as_str() {
            Some("input") => ViewLayer::Input,
            Some("paper_back") => ViewLayer::PaperBack,
            _ => ViewLayer::Output,
        };
        s
    }
    pub fn to_json(&self) -> Value {
        serde_json::json!({"viewer_layer":match self.layer { ViewLayer::Input=>"input",ViewLayer::Output=>"output",ViewLayer::PaperBack=>"paper_back" },"output_interpolation":self.interpolation.name(),"gray_18_canvas":self.gray_18_canvas,"white_padding":self.white_padding,"reveal":self.reveal,"crossfade":self.crossfade,"use_display_transform":self.use_display_transform})
    }
}

#[derive(Clone)]
pub struct DisplayRaster {
    pub size: [usize; 2],
    pub rgb: Arc<[[f32; 3]]>,
}
impl DisplayRaster {
    pub fn new(size: [usize; 2], rgb: Vec<[f32; 3]>) -> Result<Self, String> {
        if size.contains(&0) || size[0].checked_mul(size[1]) != Some(rgb.len()) {
            return Err("Invalid viewer raster dimensions".into());
        }
        Ok(Self {
            size,
            rgb: rgb.into(),
        })
    }
    pub fn from_rgba(size: [usize; 2], rgba: &[u8]) -> Result<Self, String> {
        if size[0].checked_mul(size[1]).and_then(|n| n.checked_mul(4)) != Some(rgba.len()) {
            return Err("Invalid viewer RGBA dimensions".into());
        }
        Self::new(
            size,
            rgba.chunks_exact(4)
                .map(|p| {
                    [
                        p[0] as f32 / 255.0,
                        p[1] as f32 / 255.0,
                        p[2] as f32 / 255.0,
                    ]
                })
                .collect(),
        )
    }
    pub fn from_float(image: &ImageBuf) -> Result<Self, String> {
        Self::new(
            [image.width as usize, image.height as usize],
            image
                .data
                .chunks_exact(3)
                .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
                .collect(),
        )
    }
    pub fn sample(&self, x: f32, y: f32, mode: Interpolation) -> [f32; 3] {
        let pixel = |x: i32, y: i32| {
            self.rgb[y.clamp(0, self.size[1] as i32 - 1) as usize * self.size[0]
                + x.clamp(0, self.size[0] as i32 - 1) as usize]
        };
        if mode == Interpolation::Nearest {
            return pixel((x + 0.5).floor() as i32, (y + 0.5).floor() as i32);
        }
        let r = mode.radius();
        let bx = x.floor() as i32;
        let by = y.floor() as i32;
        let mut wx = [0.0; 8];
        let mut wy = [0.0; 8];
        for i in 0..2 * r {
            wx[i as usize] = mode.weight(x - (bx + i - r + 1) as f32);
            wy[i as usize] = mode.weight(y - (by + i - r + 1) as f32);
        }
        let mut out = [0.0; 3];
        for j in 0..2 * r {
            for i in 0..2 * r {
                let w = wx[i as usize] * wy[j as usize];
                let p = pixel(bx + i - r + 1, by + j - r + 1);
                for c in 0..3 {
                    out[c] += w * p[c];
                }
            }
        }
        out
    }
}

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
    previous: Option<DisplayRaster>,
    polaroid: Option<PolaroidState>,
}
#[derive(Default)]
pub struct Viewer {
    pub settings: DisplaySettings,
    /// Fit-relative zoom retained for legacy state and freehand scrolling.
    pub zoom: f32,
    /// Exact display zoom in source pixels per device pixel, when selected.
    /// `None` means the legacy fit-relative mode.
    pub zoom_percent: Option<f32>,
    pub pan: Vec2,
    pub input: Option<DisplayRaster>,
    pub output: Option<DisplayRaster>,
    /// Original source dimensions, independent of the capped display raster.
    pub input_size: Option<[usize; 2]>,
    pub output_size: Option<[usize; 2]>,
    pub transform_status: String,
    paper: Option<DisplayRaster>,
    transition: Option<Transition>,
    texture: Option<TextureHandle>,
    cache_key: Option<Vec<u64>>,
    input_source_space: Option<String>,
    input_source_encoded: bool,
    output_source_space: Option<String>,
    output_source_encoded: bool,
    output_source_transform: bool,
    output_source_profile: Option<PathBuf>,
    revision: u64,
}
impl Viewer {
    pub fn new() -> Self {
        Self {
            zoom: 1.0,
            zoom_percent: None,
            ..Self::default()
        }
    }
    pub fn set_input(&mut self, raster: DisplayRaster, original_size: [usize; 2]) {
        self.input_size = Some(original_size);
        self.paper = None;
        self.input = Some(raster);
        self.output = None;
        self.output_size = None;
        self.transition = None;
        self.settings.layer = ViewLayer::PaperBack;
        self.reset_view();
        self.revision += 1;
    }
    /// Call only after accepting the latest asynchronous job. Retains the old
    /// display frame for a ten-step fade; it never retains or modifies export data.
    pub fn set_output(&mut self, raster: DisplayRaster, original_size: [usize; 2], now: f64) {
        let supported = original_size[0].saturating_mul(original_size[1]) <= ANIMATION_MAX_PIXELS;
        let reveal = self.settings.layer != ViewLayer::Output || self.output.is_none();
        let previous = if !reveal && self.settings.crossfade && supported {
            self.output
                .as_ref()
                .filter(|p| p.size == raster.size)
                .cloned()
        } else {
            None
        };
        let polaroid = if reveal && self.settings.reveal && supported {
            Some(PolaroidState::new(&raster))
        } else {
            None
        };
        self.transition = if previous.is_some() || polaroid.is_some() {
            Some(Transition {
                start: now,
                previous,
                polaroid,
            })
        } else {
            None
        };
        self.output = Some(raster);
        self.output_size = Some(original_size);
        self.settings.layer = ViewLayer::Output;
        self.revision += 1;
    }
    /// Replace only the viewing artifact (e.g. ICC toggle), without development animation.
    pub fn replace_output_display(&mut self, raster: DisplayRaster) {
        self.output = Some(raster);
        self.transition = None;
        self.revision += 1;
    }
    pub fn replace_input_display(&mut self, raster: DisplayRaster) {
        self.input = Some(raster);
        self.revision += 1;
    }
    pub fn set_input_display_source(&mut self, space: &str, encoded: bool) {
        self.input_source_space = Some(space.to_owned());
        self.input_source_encoded = encoded;
        self.revision = self.revision.wrapping_add(1);
    }
    /// Configure the borrowed full-resolution output source used for exact zoom.
    /// This does not copy the source; it only invalidates the bounded viewport cache.
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
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.zoom_percent = None;
        self.pan = Vec2::ZERO;
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
            ui.selectable_value(&mut self.settings.layer, ViewLayer::Input, "Input");
            ui.add_enabled_ui(self.output.is_some(), |ui| {
                ui.selectable_value(&mut self.settings.layer, ViewLayer::Output, "Output");
            });
            ui.selectable_value(&mut self.settings.layer, ViewLayer::PaperBack, "Paper back");
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
            .on_hover_text("Apply the display transform to the viewer output only; saved and exported pixels are unchanged.");
        ui.checkbox(&mut self.settings.gray_18_canvas, "gray 18% canvas")
            .on_hover_text(
                "Use neutral 18% gray as backgroung to judge the exposure and neutral colors",
            );
        ui.horizontal(|ui| {
            let tooltip = "Expand the white border layer around the normalized preview frame, expressed as a fraction of the image long edge.";
            ui.label("white padding").on_hover_text(tooltip);
            ui.add(egui::DragValue::new(&mut self.settings.white_padding)
                .range(0.0..=1.0).clamp_existing_to_range(false).speed(0.01).fixed_decimals(2))
                .on_hover_text(tooltip);
        });
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
            .on_hover_text(
                "Napari interpolation mode used to display the output layer in the viewer.",
            );
    }
    /// Draws only visible pixels into a bounded texture. Large source images and
    /// offscreen pans therefore cannot exceed the GPU texture dimension limit.
    fn full_source_view(
        &self,
        source: &ImageBuf,
        layer: ViewLayer,
        image_rect: Rect,
        rect: Rect,
        texture_size: [usize; 2],
        interpolation: Interpolation,
    ) -> Option<(DisplayRaster, Option<String>)> {
        if source.width == 0 || source.height == 0 {
            return None;
        }
        let (name, encoded, enabled) = if layer == ViewLayer::Input {
            (
                self.input_source_space.as_deref(),
                self.input_source_encoded,
                true,
            )
        } else {
            (
                self.output_source_space.as_deref(),
                self.output_source_encoded,
                self.output_source_transform,
            )
        };
        // Missing metadata must keep the prepared viewing artifact, never show raw output.
        let name = name?;
        let space = if enabled {
            Some(spektrafilm_math::colorspace::resolve(name).ok()?)
        } else {
            None
        };
        let matrix = space.map(spektrafilm_math::colorspace::display_matrix);
        let mut status = None;
        let profile = if layer == ViewLayer::Output && enabled {
            self.output_source_profile.as_deref().and_then(|path| {
                use lcms2::{Intent, PixelFormat, Profile, Transform};
                let result = Profile::new_file(path)
                    .map_err(|error| error.to_string())
                    .and_then(|destination| {
                        Transform::<[u8; 3], [u8; 3]>::new(
                            &Profile::new_srgb(),
                            PixelFormat::RGB_8,
                            &destination,
                            PixelFormat::RGB_8,
                            Intent::Perceptual,
                        )
                        .map_err(|error| error.to_string())
                    });
                match result {
                    Ok(transform) => Some(transform),
                    Err(e) => {
                        status = Some(format!(
                            "Display transform: ICC failed ({e}); viewing sRGB preview"
                        ));
                        None
                    }
                }
            })
        } else {
            None
        };
        let mut rgb = Vec::with_capacity(texture_size[0] * texture_size[1]);
        for y in 0..texture_size[1] {
            for x in 0..texture_size[0] {
                let p = Pos2::new(
                    rect.min.x + (x as f32 + 0.5) * rect.width() / texture_size[0] as f32,
                    rect.min.y + (y as f32 + 0.5) * rect.height() / texture_size[1] as f32,
                );
                rgb.push(if image_rect.contains(p) {
                    sample_image_with(source, p, image_rect, interpolation, |raw| {
                        let value = match (space, matrix.as_ref()) {
                            (Some(space), Some(matrix)) => {
                                spektrafilm_math::colorspace::display_rgb(
                                    raw.map(|v| v as f64),
                                    space,
                                    encoded,
                                    matrix,
                                )
                                .map(|v| v.clamp(0.0, 1.0) as f32)
                            }
                            _ => raw,
                        };
                        if let Some(transform) = profile.as_ref() {
                            let mut pixel = [value.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)];
                            transform.transform_in_place(&mut pixel);
                            pixel[0].map(|v| v as f32 / 255.0)
                        } else {
                            value
                        }
                    })
                } else {
                    [0.0; 3]
                });
            }
        }
        let raster = DisplayRaster::new(texture_size, rgb).ok()?;
        Some((raster, status))
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        input_float: Option<&ImageBuf>,
        output_float: Option<&ImageBuf>,
    ) -> Option<PixelInspection> {
        let response = ui.allocate_response(
            ui.available_size().max(Vec2::splat(1.0)),
            egui::Sense::click_and_drag(),
        );
        let rect = response.rect;
        if self.zoom <= 0.0 {
            self.zoom = 1.0;
        }
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
            if let Some(percent) = self.zoom_percent {
                let old = percent;
                self.zoom_percent = Some((old * factor).clamp(0.1, 3200.0));
                if let Some(p) = cursor {
                    self.pan += (p - rect.center() - self.pan) * (1.0 - factor);
                }
            } else {
                let old = self.zoom;
                self.zoom = (old * factor).clamp(0.1, 32.0);
                if let Some(p) = cursor {
                    self.pan += (p - rect.center() - self.pan) * (1.0 - self.zoom / old);
                }
            }
        }
        if response.dragged() {
            self.pan += response.drag_delta();
        }
        if response.double_clicked() {
            self.reset_view();
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
        if self.paper.is_none() {
            let size = self.input_size.unwrap_or([3, 2]);
            let long = size[0].max(size[1]).max(1);
            let raster_size = [
                (size[0] as f64 * 1024.0 / long as f64).round().max(1.0) as usize,
                (size[1] as f64 * 1024.0 / long as f64).round().max(1.0) as usize,
            ];
            self.paper = Some(virtual_paper_back(raster_size));
        }
        let layer = self.settings.layer;
        let interpolation = match layer {
            ViewLayer::Output => self.settings.interpolation,
            ViewLayer::Input => Interpolation::Nearest,
            ViewLayer::PaperBack => Interpolation::Spline36,
        };
        let selected = match layer {
            ViewLayer::Input => self.input.as_ref(),
            ViewLayer::Output => self.output.as_ref(),
            ViewLayer::PaperBack => self.paper.as_ref(),
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
        if let Some(t) = self.transition.as_ref() {
            let count = if t.polaroid.is_some() { 50 } else { 10 };
            frame = ((now - t.start) / 0.032).floor().max(0.0) as usize;
            if frame >= count {
                self.transition = None;
                frame = 0;
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(32));
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
        ];
        if self.cache_key.as_ref() != Some(&key) {
            let animation = if layer == ViewLayer::Output {
                self.transition
                    .as_ref()
                    .and_then(|t| t.polaroid.as_ref())
                    .map(|p| p.frame(frame as f32 / 49.0))
            } else {
                None
            };
            let raster = animation.as_ref().or(selected);
            let magnified = selected.is_some_and(|r| {
                image_rect.width() * dpi > r.size[0] as f32
                    || image_rect.height() * dpi > r.size[1] as f32
            });
            let full_source =
                if self.transition.is_none() && (magnified || self.zoom_percent.is_some()) {
                    let source = match layer {
                        ViewLayer::Input => input_float,
                        ViewLayer::Output => output_float,
                        ViewLayer::PaperBack => None,
                    };
                    source.and_then(|source| {
                        self.full_source_view(
                            source,
                            layer,
                            image_rect,
                            rect,
                            texture_size,
                            interpolation,
                        )
                    })
                } else {
                    None
                };
            if let Some((_, Some(status))) = full_source.as_ref() {
                self.transform_status = status.clone();
            }
            let full_source_ref = full_source.as_ref().map(|(raster, _)| raster);
            let gray = if self.settings.gray_18_canvas {
                Color32::from_gray(118)
            } else {
                Color32::BLACK
            };
            let mut pixels = Vec::with_capacity(texture_size[0] * texture_size[1]);
            for y in 0..texture_size[1] {
                for x in 0..texture_size[0] {
                    let p = Pos2::new(
                        rect.min.x + (x as f32 + 0.5) * rect.width() / texture_size[0] as f32,
                        rect.min.y + (y as f32 + 0.5) * rect.height() / texture_size[1] as f32,
                    );
                    let mut color = if border.contains(p) {
                        Color32::WHITE
                    } else {
                        gray
                    };
                    if paper_rect.contains(p) && !image_rect.contains(p) {
                        if let Some(paper) = self.paper.as_ref() {
                            color =
                                pack(sample_rect(paper, p, paper_rect, Interpolation::Spline36));
                        }
                    }
                    if image_rect.contains(p) {
                        if let Some(raster) = raster {
                            let mut rgb = if let Some(view) = full_source_ref {
                                view.rgb[y * texture_size[0] + x]
                            } else {
                                sample_rect(raster, p, image_rect, interpolation)
                            };
                            if layer == ViewLayer::Output {
                                if let Some(previous) =
                                    self.transition.as_ref().and_then(|t| t.previous.as_ref())
                                {
                                    let a = frame as f32 / 10.0;
                                    let old = sample_rect(previous, p, image_rect, interpolation);
                                    for c in 0..3 {
                                        rgb[c] = old[c].clamp(0.0, 1.0) * (1.0 - a)
                                            + rgb[c].clamp(0.0, 1.0) * a;
                                    }
                                }
                            }
                            color = pack(rgb);
                        }
                    }
                    pixels.push(color);
                }
            }
            let image = egui::ColorImage {
                size: texture_size,
                pixels,
            };
            if let Some(texture) = self.texture.as_mut() {
                texture.set(image, TextureOptions::NEAREST);
            } else {
                self.texture = Some(ui.ctx().load_texture(
                    "viewer-presentation",
                    image,
                    TextureOptions::NEAREST,
                ));
            }
            self.cache_key = Some(key);
        }
        if let Some(texture) = self.texture.as_ref() {
            ui.painter_at(rect).image(
                texture.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
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
        let text = if let Some(p) = inspection {
            format!(
                "{:?} ({}, {}) RGB {:.6}, {:.6}, {:.6}",
                p.layer, p.x, p.y, p.rgb[0], p.rgb[1], p.rgb[2]
            )
        } else {
            format!(
                "{:?} · zoom {:.2}% · double-click to fit",
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
fn sample_rect(raster: &DisplayRaster, p: Pos2, rect: Rect, mode: Interpolation) -> [f32; 3] {
    let uv = (p - rect.min) / rect.size();
    raster.sample(
        uv.x * raster.size[0] as f32 - 0.5,
        uv.y * raster.size[1] as f32 - 0.5,
        mode,
    )
}
fn sample_image_with(
    image: &ImageBuf,
    p: Pos2,
    rect: Rect,
    mode: Interpolation,
    transform: impl Fn([f32; 3]) -> [f32; 3],
) -> [f32; 3] {
    let uv = (p - rect.min) / rect.size();
    let x = uv.x * image.width as f32 - 0.5;
    let y = uv.y * image.height as f32 - 0.5;
    let pixel = |x: i32, y: i32| {
        transform(
            image
                .get(
                    x.clamp(0, image.width as i32 - 1) as u32,
                    y.clamp(0, image.height as i32 - 1) as u32,
                )
                .map(|v| v as f32),
        )
    };
    if mode == Interpolation::Nearest {
        return pixel((x + 0.5).floor() as i32, (y + 0.5).floor() as i32);
    }
    let r = mode.radius();
    let bx = x.floor() as i32;
    let by = y.floor() as i32;
    let mut wx = [0.0; 8];
    let mut wy = [0.0; 8];
    for i in 0..2 * r {
        wx[i as usize] = mode.weight(x - (bx + i - r + 1) as f32);
        wy[i as usize] = mode.weight(y - (by + i - r + 1) as f32);
    }
    let mut out = [0.0; 3];
    for j in 0..2 * r {
        for i in 0..2 * r {
            let w = wx[i as usize] * wy[j as usize];
            let p = pixel(bx + i - r + 1, by + j - r + 1);
            for c in 0..3 {
                out[c] += w * p[c];
            }
        }
    }
    out
}
fn pack(p: [f32; 3]) -> Color32 {
    Color32::from_rgb(
        (p[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (p[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (p[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// Prepared nine-layer chemical development, using the pinned Python formulas.
pub struct PolaroidState {
    size: [usize; 2],
    layers: Vec<[[f32; 3]; 9]>,
}
impl PolaroidState {
    pub fn new(image: &DisplayRaster) -> Self {
        let divisor = if image.rgb.iter().flatten().copied().fold(0.0, f32::max) > 1.0 {
            255.0
        } else {
            1.0
        };
        let density: Vec<[f32; 3]> = image
            .rgb
            .iter()
            .map(|p| p.map(|v| (-(v / divisor).clamp(1e-4, 1.0).powf(2.2).ln()).clamp(0.0, 6.0)))
            .collect();
        let mut dmax = [0.0_f32; 3];
        let mut cmax = [0.0_f32; 3];
        let mut nmax = 0.0_f32;
        for d in &density {
            let n = d.iter().copied().fold(f32::INFINITY, f32::min);
            nmax = nmax.max(n);
            for c in 0..3 {
                dmax[c] = dmax[c].max(d[c]);
                cmax[c] = cmax[c].max(d[c] - n);
            }
        }
        let layers = density
            .iter()
            .map(|d| {
                let n = d.iter().copied().fold(f32::INFINITY, f32::min);
                let ch = d.map(|v| v - n);
                let dl = std::array::from_fn::<_, 3, _>(|c| d[c] / (dmax[c] + 1e-4));
                let cl = std::array::from_fn::<_, 3, _>(|c| ch[c] / (cmax[c] + 1e-4));
                let shadow = (d[0] * 0.34 + d[1] * 0.39 + d[2] * 0.27).clamp(0.0, 1.75) / 1.75;
                let h = 1.0 - shadow;
                let strength = cl.iter().copied().fold(0.0, f32::max);
                let warmth = ((ch[1] + 1.25 * ch[2] - 0.85 * ch[0])
                    / (ch.iter().sum::<f32>() + 1e-4))
                    .clamp(0.0, 1.0);
                let silver = (0.24 + 0.46 * shadow + 0.30 * n / (nmax + 1e-4)).clamp(0.0, 1.0);
                let mut l = [[0.0; 3]; 9];
                l[0] = [n; 3];
                l[1][2] = ch[2];
                l[2][1] = ch[1];
                l[3][0] = ch[0];
                for c in 0..3 {
                    l[4][c] = (0.30 + 0.70 * h) * [0.04, 0.08, 0.15][c];
                    l[5][c] = (silver * [0.18, 0.19, 0.20][c]
                        + cl[(c + 1) % 3] * [0.03, 0.02, 0.04][c])
                        .clamp(0.0, 0.28);
                    l[6][c] = (h * [0.10, 0.06, 0.02][c]
                        + (1.0 - dl[(c + 2) % 3]) * [0.13, 0.07, 0.18][c]
                        + cl[(c + 1) % 3] * [0.04, 0.02, 0.06][c])
                        * (0.40 + 0.60 * h);
                    l[7][c] =
                        (0.24 + 0.42 * h + 0.34 * warmth.max(strength)) * [0.02, 0.08, 0.19][c];
                    l[8][c] = h * [0.03, 0.06, 0.11][c];
                }
                l
            })
            .collect();
        Self {
            size: image.size,
            layers,
        }
    }
    pub fn frame(&self, t: f32) -> DisplayRaster {
        let k = layer_coefficients(t);
        let rgb = self
            .layers
            .iter()
            .map(|l| {
                std::array::from_fn(|c| {
                    let d = (0..9).map(|i| k[i] * l[i][c]).sum::<f32>().clamp(0.0, 8.0);
                    (-d / 2.2).exp()
                })
            })
            .collect();
        DisplayRaster {
            size: self.size,
            rgb,
        }
    }
}
pub fn layer_coefficients(t: f32) -> [f32; 9] {
    let t = t.clamp(0.0, 1.0);
    let sigmoid = |t: f32, c: f32, s: f32| {
        let expit = |x: f32| 1.0 / (1.0 + (-x.clamp(-60.0, 60.0)).exp());
        let start = expit(-c * s);
        ((expit((t - c) * s) - start) / (expit((1.0 - c) * s) - start + 1e-4)).clamp(0.0, 1.0)
    };
    let rise = |c, s, p| {
        let r = sigmoid(t, c, s);
        r + (1.0 - r) * t.powf(p)
    };
    let pulse = |a, b, c, d, p| sigmoid(t, a, b) * (1.0 - sigmoid(t, c, d)) * (1.0 - t).powf(p);
    let body = rise(0.19, 7.4, 3.8);
    [
        body * rise(0.12, 8.0, 2.8),
        body * rise(0.22, 11.0, 3.0),
        body * rise(0.31, 10.2, 3.4),
        body * rise(0.52, 9.4, 5.4),
        1.08 * (1.0 - t).powf(0.46),
        pulse(0.05, 32.0, 0.44, 8.0, 0.24),
        1.22 * pulse(0.10, 54.0, 0.28, 28.0, 0.18),
        0.96 * pulse(0.12, 20.0, 0.80, 6.5, 0.34),
        0.68 * pulse(0.03, 24.0, 0.98, 4.0, 0.08),
    ]
}

// Original pinned virtual_paper_watermark.png embedded so the viewer requires no
// Python installation or runtime asset path.
fn logo_alpha() -> (usize, usize, Vec<f32>) {
    let image = image::load_from_memory(include_bytes!("../assets/virtual_paper_watermark.png"))
        .expect("embedded watermark PNG")
        .to_rgba8();
    (
        image.width() as usize,
        image.height() as usize,
        image.pixels().map(|p| p[3] as f32 / 255.0).collect(),
    )
}
// NumPy default_rng(0): PCG64 with the pinned SeedSequence state. NumPy consumes
// low then high u32 words for float32, and discards their low eight bits.
struct PaperRng {
    state: u128,
    pending: Option<u32>,
}
impl PaperRng {
    fn new() -> Self {
        Self {
            state: 35399562948360463058890781895381311971,
            pending: None,
        }
    }
    fn next(&mut self) -> f32 {
        let word = if let Some(word) = self.pending.take() {
            word
        } else {
            self.state = self
                .state
                .wrapping_mul(47026247687942121848144207491837523525)
                .wrapping_add(87136372517582989555478159403783844777);
            let hi = (self.state >> 64) as u64;
            let lo = self.state as u64;
            let value = (hi ^ lo).rotate_right((self.state >> 122) as u32);
            self.pending = Some((value >> 32) as u32);
            value as u32
        };
        (word >> 8) as f32 * (1.0 / 16777216.0)
    }
}
fn alpha_sample(alpha: &[f32], width: usize, height: usize, x: f32, y: f32) -> f32 {
    if x < 0.0 || y < 0.0 || x > (width - 1) as f32 || y > (height - 1) as f32 {
        return 0.0;
    }
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let a = x - x0 as f32;
    let b = y - y0 as f32;
    (alpha[y0 * width + x0] * (1.0 - a) + alpha[y0 * width + x1] * a) * (1.0 - b)
        + (alpha[y1 * width + x0] * (1.0 - a) + alpha[y1 * width + x1] * a) * b
}
/// Pinned 72° rhombic watermark lattice, 16° rotation, seed-zero paper glare.
pub fn virtual_paper_back(size: [usize; 2]) -> DisplayRaster {
    let (sw, sh, mut alpha) = logo_alpha();
    let long = size[0].max(size[1]) as f32;
    let scale = 0.22 * long / sw.max(sh) as f32;
    let sigma = (0.5 / scale - 0.5).max(0.0);
    if sigma > 0.0 {
        let radius = (4.0 * sigma + 0.5).floor() as i32;
        let mut kernel: Vec<f32> = (-radius..=radius)
            .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
            .collect();
        let sum = kernel.iter().sum::<f32>();
        for w in &mut kernel {
            *w /= sum;
        }
        let mut temp = vec![0.0; alpha.len()];
        for y in 0..sh {
            for x in 0..sw {
                temp[y * sw + x] = (-radius..=radius)
                    .map(|i| {
                        alpha[y * sw + (x as i32 + i).clamp(0, sw as i32 - 1) as usize]
                            * kernel[(i + radius) as usize]
                    })
                    .sum();
            }
        }
        for y in 0..sh {
            for x in 0..sw {
                alpha[y * sw + x] = (-radius..=radius)
                    .map(|i| {
                        temp[(y as i32 + i).clamp(0, sh as i32 - 1) as usize * sw + x]
                            * kernel[(i + radius) as usize]
                    })
                    .sum();
            }
        }
    }
    let tw = (sw as f32 * scale).round().max(1.0) as usize;
    let th = (sh as f32 * scale).round().max(1.0) as usize;
    let mut tile = vec![0.0; tw * th];
    for y in 0..th {
        for x in 0..tw {
            tile[y * tw + x] = alpha_sample(
                &alpha,
                sw,
                sh,
                x as f32 * (sw - 1) as f32 / (tw - 1).max(1) as f32,
                y as f32 * (sh - 1) as f32 / (th - 1).max(1) as f32,
            );
        }
    }
    let angle = 16_f32.to_radians();
    let (sin, cos) = angle.sin_cos();
    let rw = (cos.abs() * tw as f32 + sin.abs() * th as f32 + 0.5).floor() as usize;
    let rh = (sin.abs() * tw as f32 + cos.abs() * th as f32 + 0.5).floor() as usize;
    let mut rotated = vec![0.0; rw * rh];
    let mut xmin = rw;
    let mut xmax = 0;
    let mut ymin = rh;
    let mut ymax = 0;
    for y in 0..rh {
        for x in 0..rw {
            let dx = x as f32 - (rw - 1) as f32 / 2.0;
            let dy = y as f32 - (rh - 1) as f32 / 2.0;
            let a = alpha_sample(
                &tile,
                tw,
                th,
                cos * dx - sin * dy + (tw - 1) as f32 / 2.0,
                sin * dx + cos * dy + (th - 1) as f32 / 2.0,
            );
            rotated[y * rw + x] = a;
            if a > 1e-4 {
                xmin = xmin.min(x);
                xmax = xmax.max(x);
                ymin = ymin.min(y);
                ymax = ymax.max(y);
            }
        }
    }
    let stamp_w = xmax - xmin + 1;
    let stamp_h = ymax - ymin + 1;
    let radius = 0.5 * ((stamp_w * stamp_w + stamp_h * stamp_h) as f32).sqrt();
    let half = 36_f32.to_radians();
    let distance = 0.25 * long;
    let basis = [
        [
            distance * (half.cos() * cos + half.sin() * sin),
            distance * (-half.cos() * sin + half.sin() * cos),
        ],
        [
            distance * (half.cos() * cos - half.sin() * sin),
            distance * (-half.cos() * sin - half.sin() * cos),
        ],
    ];
    let limit =
        (((size[0] as f32).hypot(size[1] as f32) + 2.0 * radius) / distance).ceil() as i32 + 2;
    let mut centers = Vec::new();
    for i in -limit..=limit {
        for j in -limit..=limit {
            let x = size[0] as f32 * 0.5 + i as f32 * basis[0][0] + j as f32 * basis[1][0];
            let y = size[1] as f32 * 0.5 + i as f32 * basis[0][1] + j as f32 * basis[1][1];
            if x >= -radius
                && x <= size[0] as f32 + radius
                && y >= -radius
                && y <= size[1] as f32 + radius
            {
                centers.push((x, y));
            }
        }
    }
    centers.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut trans = vec![1.0; size[0] * size[1]];
    for (cx, cy) in centers {
        let x0 = (cx - 0.5 * stamp_w as f32).round_ties_even() as i32;
        let y0 = (cy - 0.5 * stamp_h as f32).round_ties_even() as i32;
        for y in 0..stamp_h {
            let dy = y0 + y as i32;
            if dy < 0 || dy >= size[1] as i32 {
                continue;
            }
            for x in 0..stamp_w {
                let dx = x0 + x as i32;
                if dx >= 0 && dx < size[0] as i32 {
                    trans[dy as usize * size[0] + dx as usize] *=
                        1.0 - rotated[(y + ymin) * rw + x + xmin];
                }
            }
        }
    }
    let mut rng = PaperRng::new();
    let a: Vec<f32> = (0..trans.len()).map(|_| rng.next()).collect();
    let rgb = trans
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let g = 0.88 * a[i] + 0.12 * rng.next().powf(10.0);
            let paper = 0.30 + 0.70 * t;
            let variation = 1.0 + (2.0 * g - 1.0) * paper * 0.07 * 0.25;
            let lift = g.powi(6) * paper * 0.42 * 0.25;
            let base = (215.0 / 255.0 + t * 27.0 / 255.0) * variation;
            let value = (base + (1.0 - base) * lift).clamp(0.0, 1.0);
            let value = (value * 255.0).round_ties_even() / 255.0;
            [value; 3]
        })
        .collect();
    DisplayRaster { size, rgb }
}

/// Primary display profile discovery matching Pillow get_display_profile().
#[cfg(windows)]
pub fn discover_display_profile() -> Result<Option<std::path::PathBuf>, String> {
    use std::{ffi::c_void, os::windows::ffi::OsStringExt};
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetDC(hwnd: *mut c_void) -> *mut c_void;
        fn ReleaseDC(hwnd: *mut c_void, dc: *mut c_void) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn GetICMProfileW(dc: *mut c_void, size: *mut u32, path: *mut u16) -> i32;
    }
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return Err("Cannot acquire Windows display context".into());
        }
        let mut len = 0;
        GetICMProfileW(dc, &mut len, std::ptr::null_mut());
        if len == 0 {
            ReleaseDC(std::ptr::null_mut(), dc);
            return Ok(None);
        }
        let mut path = vec![0_u16; len as usize];
        let ok = GetICMProfileW(dc, &mut len, path.as_mut_ptr());
        ReleaseDC(std::ptr::null_mut(), dc);
        if ok == 0 {
            return Err("Cannot discover Windows display ICC profile".into());
        }
        let end = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        Ok(Some(std::ffi::OsString::from_wide(&path[..end]).into()))
    }
}
#[cfg(not(windows))]
#[allow(dead_code)]
pub fn discover_display_profile() -> Result<Option<std::path::PathBuf>, String> {
    Ok(None)
}

/// Takes already encoded sRGB viewing pixels, never an export ImageBuf.
/// ICC conversion is quantized to RGB8 as in the pinned Pillow workflow.
pub fn apply_display_profile(
    raster: &DisplayRaster,
    path: &std::path::Path,
) -> Result<DisplayRaster, String> {
    use lcms2::{Intent, PixelFormat, Profile, Transform};
    let source = Profile::new_srgb();
    let destination = Profile::new_file(path).map_err(|e| e.to_string())?;
    let transform: Transform<[u8; 3], [u8; 3]> = Transform::new(
        &source,
        PixelFormat::RGB_8,
        &destination,
        PixelFormat::RGB_8,
        Intent::Perceptual,
    )
    .map_err(|e| e.to_string())?;
    let mut rgb: Vec<[u8; 3]> = raster
        .rgb
        .iter()
        .map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8))
        .collect();
    transform.transform_in_place(&mut rgb);
    DisplayRaster::new(
        raster.size,
        rgb.into_iter()
            .map(|p| p.map(|v| v as f32 / 255.0))
            .collect(),
    )
}

/// A disabled transform explicitly views values in the output space. An
/// enabled transform always starts from the separate encoded-sRGB preview;
/// an ICC profile only adds the final device conversion.
pub fn prepare_display_raster(
    raw_output: DisplayRaster,
    srgb_preview: Option<DisplayRaster>,
    enabled: bool,
    selected_profile: Option<&std::path::Path>,
) -> (DisplayRaster, String) {
    if !enabled {
        return (
            raw_output,
            "Display transform: disabled; viewing output-space values".into(),
        );
    }
    let Some(srgb) = srgb_preview else {
        return (
            raw_output,
            "Display transform: missing sRGB preview; viewing output-space values".into(),
        );
    };
    #[cfg(windows)]
    {
        let discovered = if selected_profile.is_none() {
            discover_display_profile()
        } else {
            Ok(None)
        };
        let profile = selected_profile
            .map(std::path::Path::to_path_buf)
            .or_else(|| discovered.ok().flatten());
        if let Some(path) = profile {
            match apply_display_profile(&srgb, &path) {
                Ok(raster) => {
                    return (
                        raster,
                        format!("Display transform: active ({})", path.display()),
                    );
                }
                Err(e) => {
                    return (
                        srgb,
                        format!("Display transform: ICC failed ({e}); viewing sRGB preview"),
                    );
                }
            }
        }
        return (
            srgb,
            "Display transform: sRGB preview; no display profile".into(),
        );
    }
    #[cfg(not(windows))]
    {
        if let Some(path) = selected_profile {
            match apply_display_profile(&srgb, path) {
                Ok(raster) => {
                    return (
                        raster,
                        format!("Display transform: active ({})", path.display()),
                    );
                }
                Err(e) => {
                    return (
                        srgb,
                        format!("Display transform: ICC failed ({e}); viewing sRGB preview"),
                    );
                }
            }
        }
        (
            srgb,
            "Display transform: sRGB preview; no display profile".into(),
        )
    }
}

/// Capped disposable float raster; sampling happens before color conversion.
/// Full-resolution pipeline buffers remain borrowed and never get copied here.
/// Create a disposable viewer raster bounded by the caller's preview size.
fn capped_raster(image: &ImageBuf, max_edge: usize) -> Result<DisplayRaster, String> {
    if image.width == 0 || image.height == 0 {
        return Err("Cannot view an empty image".into());
    }
    let max_edge = max_edge.max(1);
    let w = image.width as usize;
    let h = image.height as usize;
    let factor = (max_edge as f64 / w.max(h) as f64).min(1.0);
    let size = [
        ((w as f64 * factor).round() as usize).max(1),
        ((h as f64 * factor).round() as usize).max(1),
    ];
    let mut rgb = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        let sy = ((y as f64 + 0.5) * h as f64 / size[1] as f64 - 0.5).clamp(0.0, (h - 1) as f64);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(h - 1);
        let fy = sy - y0 as f64;
        for x in 0..size[0] {
            let sx =
                ((x as f64 + 0.5) * w as f64 / size[0] as f64 - 0.5).clamp(0.0, (w - 1) as f64);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(w - 1);
            let fx = sx - x0 as f64;
            rgb.push(std::array::from_fn(|c| {
                let a = image.data[(y0 * w + x0) * 3 + c] as f64;
                let b = image.data[(y0 * w + x1) * 3 + c] as f64;
                let d = image.data[(y1 * w + x0) * 3 + c] as f64;
                let e = image.data[(y1 * w + x1) * 3 + c] as f64;
                ((a * (1.0 - fx) + b * fx) * (1.0 - fy) + (d * (1.0 - fx) + e * fx) * fy) as f32
            }));
        }
    }
    DisplayRaster::new(size, rgb)
}
pub fn input_display_raster(
    image: &ImageBuf,
    space: &str,
    decode: bool,
    max_edge: usize,
) -> Result<DisplayRaster, String> {
    use spektrafilm_math::colorspace::{display_matrix, display_rgb, resolve};
    let space = resolve(space)?;
    let matrix = display_matrix(space);
    let raw = capped_raster(image, max_edge)?;
    DisplayRaster::new(
        raw.size,
        raw.rgb
            .iter()
            .map(|p| {
                display_rgb(p.map(|v| v as f64), space, decode, &matrix)
                    .map(|v| v.clamp(0.0, 1.0) as f32)
            })
            .collect(),
    )
}
pub fn output_display_raster(
    image: &ImageBuf,
    space: &str,
    encoded: bool,
    enabled: bool,
    profile: Option<&std::path::Path>,
    max_edge: usize,
) -> Result<(DisplayRaster, String), String> {
    let raw = capped_raster(image, max_edge)?;
    let srgb = enabled
        .then(|| input_display_raster(image, space, encoded, max_edge))
        .transpose()?;
    Ok(prepare_display_raster(raw, srgb, enabled, profile))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_output_zoom_preserves_detail_and_transform_changes() {
        let source = ImageBuf::from_data(
            4,
            1,
            vec![
                0.0 as _, 0.0 as _, 0.0 as _, 1.0 as _, 1.0 as _, 1.0 as _, 0.0 as _, 0.0 as _,
                0.0 as _, 1.0 as _, 1.0 as _, 1.0 as _,
            ],
        );
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
        let source = ImageBuf::from_data(1, 1, vec![0.18 as _, 0.18 as _, 0.18 as _]);
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

    #[test]
    fn raster_constructors_preserve_dimensions_and_rgb_values() {
        let rgba = DisplayRaster::from_rgba([1, 1], &[64, 128, 255, 7]).unwrap();
        assert_eq!(rgba.size, [1, 1]);
        assert_eq!(rgba.rgb[0], [64.0 / 255.0, 128.0 / 255.0, 1.0]);

        let image = ImageBuf::from_data(1, 1, vec![0.25 as _, 0.5 as _, 0.75 as _]);
        let raster = DisplayRaster::from_float(&image).unwrap();
        assert_eq!(raster.size, [1, 1]);
        assert_eq!(raster.rgb[0], [0.25, 0.5, 0.75]);
    }

    #[cfg(not(windows))]
    #[test]
    fn enabled_non_srgb_output_matches_colour_conversion() {
        use spektrafilm_math::colorspace::{display_matrix, display_rgb, resolve};

        let image = ImageBuf::from_data(1, 1, vec![0.75 as _, 0.20 as _, 0.10 as _]);
        let source = image.data.clone();
        let (actual, status) =
            output_display_raster(&image, "Display P3", true, true, None, 1).unwrap();
        let space = resolve("Display P3").unwrap();
        let expected = display_rgb([0.75, 0.20, 0.10], space, true, &display_matrix(space));

        for (actual, expected) in actual.rgb[0].into_iter().zip(expected) {
            assert!((actual as f64 - expected.clamp(0.0, 1.0)).abs() < 2e-6);
        }
        assert!(status.contains("sRGB preview"));
        assert_eq!(image.data, source);
    }

    #[cfg(not(windows))]
    #[test]
    fn enabled_preview_applies_source_cctf_decoding() {
        use spektrafilm_math::colorspace::{display_matrix, display_rgb, resolve};

        let image = ImageBuf::from_data(1, 1, vec![0.18 as _, 0.18 as _, 0.18 as _]);
        let (actual, _) =
            output_display_raster(&image, "ITU-R BT.2020", true, true, None, 1).unwrap();
        let space = resolve("ITU-R BT.2020").unwrap();
        let expected = display_rgb([0.18, 0.18, 0.18], space, true, &display_matrix(space));

        for (actual, expected) in actual.rgb[0].into_iter().zip(expected) {
            assert!((actual as f64 - expected.clamp(0.0, 1.0)).abs() < 2e-6);
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn enabled_preview_clips_out_of_gamut_display_values_only() {
        use spektrafilm_math::colorspace::{display_matrix, display_rgb, resolve};

        let image = ImageBuf::from_data(1, 1, vec![1.3 as _, -0.2 as _, 0.4 as _]);
        let source = image.data.clone();
        let raw = capped_raster(&image, 1).unwrap();
        let (preview, _) =
            output_display_raster(&image, "Display P3", false, true, None, 1).unwrap();
        let space = resolve("Display P3").unwrap();
        let expected = display_rgb([1.3, -0.2, 0.4], space, false, &display_matrix(space));

        assert_eq!(raw.rgb[0], [1.3, -0.2, 0.4]);
        for (actual, expected) in preview.rgb[0].into_iter().zip(expected) {
            assert!((actual as f64 - expected.clamp(0.0, 1.0)).abs() < 2e-6);
            assert!((0.0..=1.0).contains(&actual));
        }
        assert_eq!(image.data, source);
    }

    #[test]
    fn disabled_transform_keeps_output_space_values() {
        let image = ImageBuf::from_data(1, 1, vec![1.3 as _, -0.2 as _, 0.4 as _]);
        let (raster, status) =
            output_display_raster(&image, "Display P3", false, false, None, 1).unwrap();

        assert_eq!(raster.rgb[0], [1.3, -0.2, 0.4]);
        assert!(status.contains("output-space values"));
    }
}
