//! Presentation of bounded image-analysis reductions. No image samples are read here.
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use serde::{Deserialize, Serialize};
use spektrafilm_backend::analysis::{AnalysisData, AnalysisMode, AnalysisPixel, BINS};

use crate::{i18n, theme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlotMode {
    #[default]
    Histogram,
    Waveform,
    Vectorscope,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WaveformView {
    #[default]
    Tone,
    RgbOverlay,
    RgbParade,
}

/// Presentation preferences only; neither selections nor computed data belong here.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PlotPreferences {
    pub mode: PlotMode,
    pub histogram_rgb: bool,
    pub waveform_view: WaveformView,
    pub channels: [bool; 3],
    pub chroma_zoom: u32,
    pub shelf_height: f32,
}

impl Default for PlotPreferences {
    fn default() -> Self {
        Self {
            mode: PlotMode::Histogram,
            histogram_rgb: false,
            waveform_view: WaveformView::Tone,
            channels: [true; 3],
            chroma_zoom: 1,
            shelf_height: theme::HEIGHT_ANALYSIS_DEFAULT,
        }
    }
}

impl PlotPreferences {
    pub(crate) fn analysis_mode(&self) -> AnalysisMode {
        match self.mode {
            PlotMode::Histogram => AnalysisMode::Histogram,
            PlotMode::Waveform if self.waveform_view == WaveformView::RgbParade => {
                AnalysisMode::Parade
            }
            PlotMode::Waveform => AnalysisMode::Waveform,
            PlotMode::Vectorscope => AnalysisMode::Vectorscope,
        }
    }
}

#[derive(Default)]
pub(crate) struct PlotState {
    column: usize,
    row: usize,
    raster: Option<RasterCache>,
    histogram: Option<HistogramCache>,
}

struct RasterCache {
    data: usize,
    mode: AnalysisMode,
    signals: [usize; 3],
    signal_count: usize,
    zoom: u32,
    texture: egui::TextureHandle,
}

struct HistogramCache {
    data: usize,
    rect: Rect,
    peak: f64,
    signals: [usize; 3],
    signal_count: usize,
    mesh: std::sync::Arc<egui::Mesh>,
}

impl PlotState {
    /// Accepted results may replace data at the same address; invalidate on replacement.
    pub(crate) fn invalidate_cache(&mut self) {
        self.raster = None;
        self.histogram = None;
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PlotSample {
    pub pixel: AnalysisPixel,
    pub x: u32,
    pub y: u32,
    pub region: [u32; 4],
    pub inside: bool,
}

#[derive(Default)]
pub(crate) struct PlotResponse {
    /// Half-open horizontal interval in the analyzed population, not a pixel.
    pub band: Option<(f32, f32)>,
    pub inspecting: bool,
}

fn label(ui: &egui::Ui, key: &str) -> std::borrow::Cow<'static, str> {
    i18n::text(ui.ctx(), key)
}

/// Controls only alter presentation preferences. The caller persists these independently of edits.
pub(crate) fn show_mode_controls(ui: &mut egui::Ui, prefs: &mut PlotPreferences) {
    let selected = match prefs.mode {
        PlotMode::Histogram => "analysis.histogram",
        PlotMode::Waveform => "analysis.waveform",
        PlotMode::Vectorscope => "analysis.vectorscope",
    };
    let response = egui::menu::menu_button(ui, label(ui, selected), |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        for (mode, name, help) in [
            (
                PlotMode::Histogram,
                "analysis.histogram",
                "analysis.histogram_help",
            ),
            (
                PlotMode::Waveform,
                "analysis.waveform",
                "analysis.waveform_help",
            ),
            (
                PlotMode::Vectorscope,
                "analysis.vectorscope",
                "analysis.vectorscope_help",
            ),
        ] {
            let name = label(ui, name);
            let help = label(ui, help);
            if ui
                .selectable_value(&mut prefs.mode, mode, name)
                .on_hover_text(help)
                .clicked()
            {
                ui.close_menu();
            }
        }
    })
    .response;
    let selected_name = label(ui, selected);
    let mode_label = label(ui, "analysis.mode");
    let help_key = match prefs.mode {
        PlotMode::Histogram => "analysis.histogram_help",
        PlotMode::Waveform => "analysis.waveform_help",
        PlotMode::Vectorscope => "analysis.vectorscope_help",
    };
    let help = format!(
        "{}\n{}\n{}{}",
        label(ui, help_key),
        label(ui, "analysis.interpretation"),
        label(ui, "surfaces.analysis.preview_limit"),
        if prefs.mode == PlotMode::Histogram {
            format!("\n{}", label(ui, "analysis.histogram_axes"))
        } else {
            String::new()
        },
    );
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_label(mode_label.as_ref());
        node.set_description(help.as_str());
        node.set_value(selected_name.as_ref());
    });
    response.on_hover_text(help);

    if prefs.mode == PlotMode::Histogram {
        let current = label(
            ui,
            if prefs.histogram_rgb {
                "analysis.rgb"
            } else {
                "analysis.tone"
            },
        );
        let response = egui::ComboBox::from_id_salt("analysis-signal")
            .width(72.0)
            .selected_text(current.clone())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut prefs.histogram_rgb, false, label(ui, "analysis.tone"))
                    .on_hover_text(label(ui, "analysis.histogram_help"));
                ui.selectable_value(&mut prefs.histogram_rgb, true, label(ui, "analysis.rgb"))
                    .on_hover_text(label(ui, "analysis.histogram_help"));
            })
            .response;
        let help = label(ui, "analysis.histogram_help");
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_label(current.as_ref());
            node.set_description(help.as_ref());
        });
        response.on_hover_text(help);
    } else if prefs.mode == PlotMode::Waveform {
        let views = [
            (
                WaveformView::Tone,
                "analysis.tone",
                "analysis.waveform_help",
            ),
            (
                WaveformView::RgbOverlay,
                "analysis.rgb_overlay",
                "analysis.waveform_help",
            ),
            (
                WaveformView::RgbParade,
                "analysis.parade",
                "analysis.parade_help",
            ),
        ];
        let selected = views
            .iter()
            .find(|(view, _, _)| *view == prefs.waveform_view)
            .unwrap();
        let response = egui::ComboBox::from_id_salt("analysis-waveform-view")
            .width(110.0)
            .selected_text(label(ui, selected.1))
            .show_ui(ui, |ui| {
                for (view, name, help) in views {
                    let name = label(ui, name);
                    let help = label(ui, help);
                    ui.selectable_value(&mut prefs.waveform_view, view, name)
                        .on_hover_text(help);
                }
            })
            .response;
        let view_label = label(ui, "analysis.waveform_view");
        let help = label(ui, selected.2);
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_label(view_label.as_ref());
            node.set_description(help.as_ref());
        });
        response.on_hover_text(help);
    }
    if (prefs.mode == PlotMode::Histogram && prefs.histogram_rgb)
        || (prefs.mode == PlotMode::Waveform && prefs.waveform_view == WaveformView::RgbOverlay)
    {
        for (index, (short, full)) in [
            ("R", "analysis.red"),
            ("G", "analysis.green"),
            ("B", "analysis.blue"),
        ]
        .into_iter()
        .enumerate()
        {
            let name = label(ui, full);
            let response = ui.selectable_label(prefs.channels[index], short);
            if response.clicked() {
                prefs.channels[index] = !prefs.channels[index];
            }
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_label(name.as_ref()));
            response.on_hover_text(name);
        }
    }
    if prefs.mode == PlotMode::Vectorscope {
        let zoom_help = label(ui, "analysis.chroma_zoom");
        for zoom in [1, 2, 4] {
            ui.selectable_value(&mut prefs.chroma_zoom, zoom, format!("{zoom}×"))
                .on_hover_text(zoom_help.clone());
        }
    }
}

fn active_signals(prefs: &PlotPreferences) -> ([usize; 3], usize) {
    match prefs.analysis_mode() {
        AnalysisMode::Histogram if !prefs.histogram_rgb => ([0, 0, 0], 1),
        AnalysisMode::Waveform if prefs.waveform_view == WaveformView::Tone => ([0, 0, 0], 1),
        AnalysisMode::Histogram | AnalysisMode::Waveform => {
            let mut signals = [0; 3];
            let mut len = 0;
            for (index, visible) in prefs.channels.iter().enumerate() {
                if *visible {
                    signals[len] = index + 1;
                    len += 1;
                }
            }
            (signals, len)
        }
        AnalysisMode::Parade => ([1, 2, 3], 3),
        AnalysisMode::Vectorscope => ([0; 3], 0),
    }
}

fn signal_color(signal: usize) -> Color32 {
    [theme::FG, theme::SERIES1, theme::SERIES2, theme::SERIES3][signal]
}

fn fraction(count: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 / total as f64
    }
}

/// Shared pair peak includes every channel and every endpoint, whether visible or not.
fn pair_peak(data: &AnalysisData, other: Option<&AnalysisData>) -> f64 {
    [Some(data), other]
        .into_iter()
        .flatten()
        .flat_map(|d| {
            d.histogram
                .iter()
                .flat_map(move |hist| hist.iter().map(move |&n| fraction(n, d.finite)))
        })
        .fold(0.0, f64::max)
}

fn plot_rect(rect: Rect, mode: AnalysisMode) -> Rect {
    if mode == AnalysisMode::Vectorscope {
        let side = rect.width().min(rect.height());
        Rect::from_center_size(rect.center(), Vec2::splat(side))
    } else {
        rect
    }
}

fn grid(painter: &egui::Painter, rect: Rect, mode: AnalysisMode) {
    for f in [0.0, 0.25, 0.5, 0.75, 1.0] {
        if mode == AnalysisMode::Vectorscope && (f == 0.25 || f == 0.75) {
            continue;
        }
        let y = egui::lerp(rect.bottom()..=rect.top(), f);
        let color = if f == 0.25 || f == 0.75 {
            theme::BORDER
        } else {
            theme::BORDER_ACTIVE
        };
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0_f32, color),
        );
    }
}

fn axes(painter: &egui::Painter, rect: Rect, mode: AnalysisMode, zoom: u32) {
    let font = theme::text(theme::TEXT_READOUT, theme::SIZE_CHART_AXIS);
    let axis = |pos, align, text: &str| {
        painter.text(pos, align, text, font.clone(), theme::FG);
    };
    if mode == AnalysisMode::Vectorscope {
        let extent = 0.4 / zoom.clamp(1, 4) as f32;
        for (fraction, text) in [
            (0.0, format!("−{extent:.2}")),
            (0.5, "0".to_owned()),
            (1.0, format!("{extent:.2}")),
        ] {
            axis(
                Pos2::new(rect.left() + fraction * rect.width(), rect.bottom() + 2.0),
                egui::Align2::CENTER_TOP,
                &text,
            );
            axis(
                Pos2::new(rect.left() - 4.0, rect.bottom() - fraction * rect.height()),
                egui::Align2::RIGHT_CENTER,
                &text,
            );
        }
        axis(
            Pos2::new(rect.right() - 3.0, rect.center().y - 3.0),
            egui::Align2::RIGHT_BOTTOM,
            "a",
        );
        axis(
            Pos2::new(rect.center().x + 4.0, rect.top() + 3.0),
            egui::Align2::LEFT_TOP,
            "b",
        );
    } else if mode != AnalysisMode::Histogram {
        for (fraction, text) in [(0.0, "0"), (0.5, "0.5"), (1.0, "1")] {
            axis(
                Pos2::new(rect.left() - 4.0, rect.bottom() - fraction * rect.height()),
                egui::Align2::RIGHT_CENTER,
                text,
            );
        }
        axis(
            Pos2::new(rect.left(), rect.bottom() + 2.0),
            egui::Align2::LEFT_TOP,
            "L",
        );
        axis(
            Pos2::new(rect.right(), rect.bottom() + 2.0),
            egui::Align2::RIGHT_TOP,
            "R",
        );
        if mode == AnalysisMode::Parade {
            for (panel, name) in ["R", "G", "B"].into_iter().enumerate() {
                axis(
                    Pos2::new(
                        rect.left() + (panel as f32 + 0.5) * rect.width() / 3.0,
                        rect.top() + 3.0,
                    ),
                    egui::Align2::CENTER_TOP,
                    name,
                );
            }
        }
    }
}

// Each primary contributes the same fixed amount wherever its curve covers a
// point. Coverage, rather than curve height, determines overlap color.
fn histogram_color(signals: &[usize]) -> Color32 {
    if signals.contains(&0) {
        return Color32::from_rgb(145, 145, 145);
    }
    let mut rgb = [
        theme::SURFACE2.r(),
        theme::SURFACE2.g(),
        theme::SURFACE2.b(),
    ];
    for &signal in signals {
        rgb[signal - 1] = rgb[signal - 1].saturating_add(120);
    }
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

fn mesh_quad(mesh: &mut egui::Mesh, points: [Pos2; 4], color: Color32) {
    let first = mesh.vertices.len() as u32;
    for point in points {
        mesh.colored_vertex(point, color);
    }
    mesh.add_triangle(first, first + 1, first + 2);
    mesh.add_triangle(first, first + 2, first + 3);
}

fn histogram(
    painter: &egui::Painter,
    state: &mut PlotState,
    rect: Rect,
    data: &AnalysisData,
    other: Option<&AnalysisData>,
    (signals, signal_count): ([usize; 3], usize),
) {
    let peak = pair_peak(data, other);
    if peak <= 0.0 {
        return;
    }
    let data_ptr = data as *const AnalysisData as usize;
    let stale = state.histogram.as_ref().is_none_or(|cache| {
        cache.data != data_ptr
            || cache.rect != rect
            || cache.peak != peak
            || cache.signals != signals
            || cache.signal_count != signal_count
    });
    if stale {
        let mut heights = [[0.0_f32; BINS]; 3];
        let normalizer = (256.0 * peak).ln_1p();
        for (channel, &signal) in signals[..signal_count].iter().enumerate() {
            for (i, &count) in data.histogram[signal].iter().take(BINS).enumerate() {
                let p = fraction(count, data.finite);
                heights[channel][i] = ((256.0 * p).ln_1p() / normalizer) as f32;
            }
        }
        let mut mesh = egui::Mesh::default();
        // Extend endpoint bins to the plot edges; split at curve crossings so
        // each trapezoid has a single, order-independent additive color.
        for segment in 0..=BINS {
            let left_bin = segment.saturating_sub(1).min(BINS - 1);
            let right_bin = segment.min(BINS - 1);
            let x0 = if segment == 0 {
                rect.left()
            } else {
                rect.left() + (segment as f32 - 0.5) * rect.width() / BINS as f32
            };
            let x1 = if segment == BINS {
                rect.right()
            } else {
                rect.left() + (segment as f32 + 0.5) * rect.width() / BINS as f32
            };
            let mut cuts = [0.0_f32; 5];
            cuts[1] = 1.0;
            let mut count = 2;
            for a in 0..signal_count {
                for b in a + 1..signal_count {
                    let d0 = heights[a][left_bin] - heights[b][left_bin];
                    let d1 = heights[a][right_bin] - heights[b][right_bin];
                    if d0 * d1 < 0.0 {
                        cuts[count] = d0 / (d0 - d1);
                        count += 1;
                    }
                }
            }
            cuts[..count].sort_by(|a, b| a.total_cmp(b));
            for interval in cuts[..count].windows(2) {
                let [t0, t1] = [interval[0], interval[1]];
                let height = |channel: usize, t: f32| {
                    egui::lerp(heights[channel][left_bin]..=heights[channel][right_bin], t)
                };
                let mut order = [0_usize, 1, 2];
                order[..signal_count].sort_by(|&a, &b| {
                    height(a, (t0 + t1) * 0.5).total_cmp(&height(b, (t0 + t1) * 0.5))
                });
                let mut bottom = [0.0, 0.0];
                for layer in 0..signal_count {
                    let top = [height(order[layer], t0), height(order[layer], t1)];
                    let mut covered = [0_usize; 3];
                    for (i, &channel) in order[layer..signal_count].iter().enumerate() {
                        covered[i] = signals[channel];
                    }
                    let x = [egui::lerp(x0..=x1, t0), egui::lerp(x0..=x1, t1)];
                    let y = |h: f32| rect.bottom() - h * rect.height();
                    mesh_quad(
                        &mut mesh,
                        [
                            Pos2::new(x[0], y(bottom[0])),
                            Pos2::new(x[1], y(bottom[1])),
                            Pos2::new(x[1], y(top[1])),
                            Pos2::new(x[0], y(top[0])),
                        ],
                        histogram_color(&covered[..signal_count - layer]),
                    );
                    bottom = top;
                }
            }
        }
        // Fine contours use exactly the same bin heights as the fills.
        for (channel, &signal) in signals[..signal_count].iter().enumerate() {
            for i in 0..BINS - 1 {
                let x = |bin: usize| rect.left() + (bin as f32 + 0.5) * rect.width() / BINS as f32;
                let a = Pos2::new(x(i), rect.bottom() - heights[channel][i] * rect.height());
                let b = Pos2::new(
                    x(i + 1),
                    rect.bottom() - heights[channel][i + 1] * rect.height(),
                );
                let normal = (b - a).normalized().rot90() * 0.5;
                mesh_quad(
                    &mut mesh,
                    [a + normal, b + normal, b - normal, a - normal],
                    signal_color(signal),
                );
            }
        }
        state.histogram = Some(HistogramCache {
            data: data_ptr,
            rect,
            peak,
            signals,
            signal_count,
            mesh: std::sync::Arc::new(mesh),
        });
    }
    painter.add(egui::Shape::mesh(
        state.histogram.as_ref().unwrap().mesh.clone(),
    ));
}

fn density(p: f64, bins: f64) -> f32 {
    ((bins * p).ln_1p() / bins.ln_1p()) as f32
}

// Composite bright traces over the opaque plot surface. Alpha is the unchanged
// frequency density, not a gamma multiplication of an already dark color.
fn composite(base: Color32, color: Color32, alpha: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 * (1.0 - alpha) + b as f32 * alpha).round() as u8;
    Color32::from_rgb(
        mix(base.r(), color.r()),
        mix(base.g(), color.g()),
        mix(base.b(), color.b()),
    )
}

// The primary contributions are added independently, so changing channel
// iteration order cannot change an RGB waveform pixel.
fn add_waveform_channel(base: Color32, signal: usize, intensity: f32) -> Color32 {
    let mut rgb = [base.r(), base.g(), base.b()];
    rgb[signal - 1] = rgb[signal - 1].saturating_add((180.0 * intensity).round() as u8);
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// The texture represents statistical cells, independent of widget geometry and hover.
/// Backend spatial bins are bounded by BINS; parade is at most 3 * BINS wide.
fn raster(
    ui: &egui::Ui,
    state: &mut PlotState,
    data: &AnalysisData,
    prefs: &PlotPreferences,
    signals: [usize; 3],
    signal_count: usize,
) -> egui::TextureId {
    let mode = prefs.analysis_mode();
    let zoom = prefs.chroma_zoom.clamp(1, 4);
    let data_ptr = data as *const AnalysisData as usize;
    let stale = state.raster.as_ref().is_none_or(|cache| {
        cache.data != data_ptr
            || cache.mode != mode
            || cache.signals != signals
            || cache.signal_count != signal_count
            || cache.zoom != zoom
    });
    if stale {
        let columns = (data.spatial_bins as usize).clamp(1, BINS);
        let width = match mode {
            AnalysisMode::Vectorscope => BINS,
            AnalysisMode::Parade => columns * 3,
            _ => columns,
        };
        let mut image = egui::ColorImage::new([width, BINS], theme::SURFACE2);
        if mode == AnalysisMode::Vectorscope {
            for y in 0..BINS {
                for x in 0..BINS {
                    let count = data.vectorscope.get(y * BINS + x).copied().unwrap_or(0);
                    if count == 0 {
                        continue;
                    }
                    let intensity = density(fraction(count, data.finite), 65536.0);
                    let a = (x as f32 + 0.5) / BINS as f32 * 2.0 - 1.0;
                    let b = (y as f32 + 0.5) / BINS as f32 * 2.0 - 1.0;
                    let angle = b.atan2(a);
                    let chroma = (a.hypot(b) * 2.0).min(1.0);
                    let tint = |phase: f32| {
                        (255.0 * (0.7 + 0.3 * chroma * (angle + phase).cos())).clamp(0.0, 255.0)
                            as u8
                    };
                    let color = Color32::from_rgb(tint(0.0), tint(-2.094_395), tint(2.094_395));
                    image.pixels[(BINS - 1 - y) * width + x] =
                        composite(theme::SURFACE2, color, intensity);
                }
            }
        } else if data.spatial_bins > 0 {
            for (panel, &signal) in signals[..signal_count].iter().enumerate() {
                for col in 0..columns {
                    let population = data.column_population.get(col).copied().unwrap_or(0);
                    if population == 0 {
                        continue;
                    }
                    for value in 0..BINS {
                        let index = signal * columns * BINS + col * BINS + value;
                        let count = data.waveform.get(index).copied().unwrap_or(0);
                        if count == 0 {
                            continue;
                        }
                        let x = col
                            + if mode == AnalysisMode::Parade {
                                panel * columns
                            } else {
                                0
                            };
                        let pixel = &mut image.pixels[(BINS - 1 - value) * width + x];
                        let intensity = density(fraction(count, population), 256.0);
                        *pixel = if mode == AnalysisMode::Waveform && signal != 0 {
                            add_waveform_channel(*pixel, signal, intensity)
                        } else {
                            composite(*pixel, signal_color(signal), intensity)
                        };
                    }
                }
            }
        }
        let texture =
            ui.ctx()
                .load_texture("analysis-density", image, egui::TextureOptions::NEAREST);
        state.raster = Some(RasterCache {
            data: data_ptr,
            mode,
            signals,
            signal_count,
            zoom,
            texture,
        });
    }
    state.raster.as_ref().unwrap().texture.id()
}

// Fixed Oklab coordinates of unit linear-sRGB primaries and secondaries.
const HUES: [(&str, f32, f32); 6] = [
    ("R", 0.2249, 0.1258),
    ("Y", -0.0714, 0.1986),
    ("G", -0.2339, 0.1795),
    ("C", -0.1494, -0.0394),
    ("B", -0.0325, -0.3115),
    ("M", 0.2746, -0.1692),
];
fn vector_pos(rect: Rect, a: f32, b: f32, extent: f32) -> Pos2 {
    Pos2::new(
        rect.center().x + a / (2.0 * extent) * rect.width(),
        rect.center().y - b / (2.0 * extent) * rect.height(),
    )
}
fn vectorscope(painter: &egui::Painter, rect: Rect, zoom: u32) {
    let extent = 0.4 / zoom as f32;
    let center = rect.center();
    painter.line_segment(
        [
            Pos2::new(center.x, rect.top()),
            Pos2::new(center.x, rect.bottom()),
        ],
        Stroke::new(1.0_f32, theme::BORDER_ACTIVE),
    );
    painter.line_segment(
        [
            Pos2::new(rect.left(), center.y),
            Pos2::new(rect.right(), center.y),
        ],
        Stroke::new(1.0_f32, theme::BORDER_ACTIVE),
    );
    painter.text(
        center + Vec2::new(5.0, -5.0),
        egui::Align2::LEFT_BOTTOM,
        "N",
        theme::text(theme::TEXT_READOUT, theme::SIZE_CHART_AXIS),
        theme::FG,
    );
    for (name, a, b) in HUES {
        let pos = vector_pos(rect, a, b, extent);
        if rect.contains(pos) {
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                name,
                theme::text(theme::TEXT_READOUT, theme::SIZE_CHART_AXIS),
                theme::ACCENT,
            );
        }
    }
}

// The translucent overlay remains visible over the opaque density texture,
// while leaving traces legible. Panel clipping keeps endpoint bands local.
fn inspection_band(painter: &egui::Painter, rect: Rect, start: f32, end: f32, panels: usize) {
    let width = rect.width() / panels as f32;
    for panel in 0..panels {
        let left = rect.left() + panel as f32 * width;
        let x = left + (start + end) * 0.5 * width;
        let half = ((end - start) * width * 0.5).max(1.5);
        let band = Rect::from_min_max(
            Pos2::new((x - half).max(left), rect.top()),
            Pos2::new((x + half).min(left + width), rect.bottom()),
        );
        painter.rect_filled(band, 0, Color32::from_white_alpha(24));
        let stroke = Stroke::new(1.0_f32, theme::FG_SECONDARY);
        for x in [band.left(), band.right()] {
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.top() + 6.0)],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(x, rect.bottom() - 6.0),
                    Pos2::new(x, rect.bottom()),
                ],
                stroke,
            );
        }
    }
}

fn marker(painter: &egui::Painter, pos: Pos2) {
    painter.circle_stroke(pos, 4.0, Stroke::new(2.0_f32, theme::BG));
    painter.circle_stroke(pos, 3.0, Stroke::new(1.5_f32, theme::ACCENT));
}

fn sample_column(x: u32, start: u32, width: u32, columns: u32) -> u32 {
    let offset = u64::from(x.saturating_sub(start));
    (((2 * offset + 1) * u64::from(columns)) / (2 * u64::from(width))).min(u64::from(columns - 1))
        as u32
}

fn sample_markers(
    painter: &egui::Painter,
    rect: Rect,
    data: &AnalysisData,
    prefs: &PlotPreferences,
    signals: &[usize],
    sample: PlotSample,
) -> [Rect; 3] {
    let mut occupied = [Rect::NOTHING; 3];
    if !sample.inside || !sample.pixel.valid {
        return occupied;
    }
    let mode = prefs.analysis_mode();
    let values = [
        sample.pixel.tone,
        sample.pixel.rgb[0],
        sample.pixel.rgb[1],
        sample.pixel.rgb[2],
    ];
    if mode == AnalysisMode::Vectorscope {
        let extent = 0.4 / prefs.chroma_zoom.clamp(1, 4) as f64;
        let [a, b] = sample.pixel.ab;
        if a.abs() <= extent && b.abs() <= extent {
            let position = vector_pos(rect, a as f32, b as f32, extent as f32);
            marker(painter, position);
            occupied[0] = Rect::from_center_size(position, Vec2::splat(10.0));
        }
        return occupied;
    }
    let width = sample.region[2].saturating_sub(sample.region[0]);
    if width == 0 {
        return occupied;
    }
    let columns = data.spatial_bins.max(1);
    let col = sample_column(sample.x, sample.region[0], width, columns);
    for (panel, &signal) in signals.iter().enumerate() {
        let value = values[signal];
        if !(0.0..=1.0).contains(&value) {
            continue;
        }
        if mode == AnalysisMode::Histogram {
            let bin = ((value * BINS as f64) as usize).min(BINS - 1);
            let x = rect.left() + (bin as f32 + 0.5) * rect.width() / BINS as f32;
            occupied[panel] = Rect::from_min_max(
                Pos2::new(x - 2.0, rect.top()),
                Pos2::new(x + 2.0, rect.bottom()),
            );
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(2.0_f32, theme::ACCENT),
            );
        } else {
            let panel_width = if mode == AnalysisMode::Parade {
                rect.width() / 3.0
            } else {
                rect.width()
            };
            let x = rect.left()
                + if mode == AnalysisMode::Parade {
                    panel as f32 * panel_width
                } else {
                    0.0
                }
                + (col as f32 + 0.5) * panel_width / columns as f32;
            let bin = ((value * BINS as f64) as usize).min(BINS - 1);
            let y = rect.bottom() - (bin as f32 + 0.5) * rect.height() / BINS as f32;
            marker(painter, Pos2::new(x, y));
            occupied[panel] = Rect::from_center_size(Pos2::new(x, y), Vec2::splat(10.0));
        }
    }
    occupied
}

/// At most three linked markers split the plot into at most 4³ clear rectangles.
fn readout_rect(rect: Rect, occupied: [Rect; 3]) -> Rect {
    if !occupied.iter().any(|marker| marker.is_positive()) {
        return rect;
    }
    let mut free = [Rect::NOTHING; 64];
    free[0] = rect;
    let mut count = 1;
    for marker in occupied {
        if !marker.is_positive() {
            continue;
        }
        let mut next = [Rect::NOTHING; 64];
        let mut next_count = 0;
        for region in &free[..count] {
            let cut = region.intersect(marker.expand(2.0));
            if !cut.is_positive() {
                next[next_count] = *region;
                next_count += 1;
                continue;
            }
            for candidate in [
                Rect::from_min_max(region.min, Pos2::new(region.right(), cut.top())),
                Rect::from_min_max(Pos2::new(region.left(), cut.bottom()), region.max),
                Rect::from_min_max(region.min, Pos2::new(cut.left(), region.bottom())),
                Rect::from_min_max(Pos2::new(cut.right(), region.top()), region.max),
            ] {
                if candidate.is_positive() {
                    next[next_count] = candidate;
                    next_count += 1;
                }
            }
        }
        free = next;
        count = next_count;
    }
    free[..count]
        .iter()
        .copied()
        .max_by(|a, b| a.area().total_cmp(&b.area()))
        .unwrap_or(Rect::NOTHING)
}

fn information_label(ui: &mut egui::Ui, text: String) -> egui::Response {
    ui.add(
        egui::Label::new(egui::RichText::new(text).monospace())
            .wrap()
            .selectable(false)
            .sense(Sense::empty()),
    )
}

fn overlay(
    ui: &mut egui::Ui,
    rect: Rect,
    notices: &[String],
    readout: &[String],
    keyboard_scroll: f32,
) -> Option<egui::scroll_area::ScrollAreaOutput<egui::Response>> {
    if (notices.is_empty() && readout.is_empty()) || !rect.is_positive() {
        return None;
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect);
    egui::Frame::new()
        .fill(theme::SURFACE2)
        .inner_margin(3)
        .show(&mut child, |ui| {
            if !readout.is_empty() {
                information_label(ui, readout.join("  ·  "));
            }
            if notices.is_empty() {
                return None;
            }
            Some(
                egui::ScrollArea::vertical()
                    .id_salt("analysis-notices")
                    .max_height(ui.available_height().max(0.0))
                    .min_scrolled_height(0.0)
                    .auto_shrink([true, true])
                    .drag_to_scroll(false)
                    .show(ui, |ui| {
                        if keyboard_scroll != 0.0 {
                            ui.scroll_with_delta(Vec2::new(0.0, -keyboard_scroll));
                        }
                        information_label(ui, notices.join("  ·  "))
                    }),
            )
        })
        .inner
}

/// Renders a matching result and optional matching viewer sample. Counterpart must
/// describe the same source-area population; absent counterpart is not synthesized.
#[allow(clippy::too_many_arguments)]
pub(crate) fn show_plot(
    ui: &mut egui::Ui,
    state: &mut PlotState,
    prefs: &PlotPreferences,
    data: &AnalysisData,
    counterpart: Option<&AnalysisData>,
    sample: Option<PlotSample>,
    max_height: f32,
    mut messages: Vec<String>,
) -> PlotResponse {
    let mode = prefs.analysis_mode();
    let mut result = PlotResponse::default();
    let (allocated, allocation) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), max_height.max(0.0)),
        Sense::hover(),
    );
    if mode == AnalysisMode::Vectorscope && data.outside_window != 0 {
        messages.push(format!(
            "{}: {} ({:.2}%)",
            label(ui, "analysis.outside_window"),
            data.outside_window,
            fraction(data.outside_window, data.finite) * 100.0
        ));
    }
    if data.below != 0 {
        messages.push(format!("{}: {}", label(ui, "analysis.below"), data.below));
    }
    if data.above != 0 {
        messages.push(format!("{}: {}", label(ui, "analysis.above"), data.above));
    }
    if data.invalid != 0 {
        messages.push(
            label(ui, "analysis.invalid_short")
                .replace("{invalid}", &data.invalid.to_string())
                .replace("{total}", &data.total.to_string()),
        );
    }
    if data.finite == 0 {
        messages.push(label(ui, "analysis.no_valid").into_owned());
        let _ = overlay(ui, allocated, &messages, &[], 0.0);
        return result;
    }
    let (signal_array, signal_count) = active_signals(prefs);
    let signals = &signal_array[..signal_count];
    if signals.is_empty() && mode != AnalysisMode::Vectorscope {
        messages.push(label(ui, "analysis.choose_channel").into_owned());
        let _ = overlay(ui, allocated, &messages, &[], 0.0);
        return result;
    }
    let bounds = if mode == AnalysisMode::Histogram {
        allocated.shrink2(Vec2::new(2.0, 2.0))
    } else {
        Rect::from_min_max(
            allocated.min + Vec2::new(32.0, 8.0),
            allocated.max - Vec2::new(8.0, 18.0),
        )
    };
    let rect = plot_rect(bounds, mode);
    let response = ui.interact(rect, allocation.id.with("plot"), Sense::click());
    let cursor_label = label(ui, "analysis.plot_cursor");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, cursor_label.as_ref())
    });
    if response.clicked() {
        response.request_focus();
    }
    if response.has_focus()
        && !response.clicked()
        && ui.input(|input| input.pointer.delta() != Vec2::ZERO)
    {
        response.surrender_focus();
    }
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
    }
    let painter = ui.painter_at(rect);
    if mode == AnalysisMode::Histogram {
        painter.rect_filled(rect, 0, theme::SURFACE2);
        grid(&painter, rect, mode);
        histogram(
            &painter,
            state,
            rect,
            data,
            counterpart,
            (signal_array, signal_count),
        );
    } else {
        let texture = raster(ui, state, data, prefs, signal_array, signal_count);
        painter.image(
            texture,
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        grid(&painter, rect, mode);
        if mode == AnalysisMode::Vectorscope {
            vectorscope(&painter, rect, prefs.chroma_zoom.clamp(1, 4));
        }
    }
    axes(ui.painter(), rect, mode, prefs.chroma_zoom);
    let axes_help = label(
        ui,
        match mode {
            AnalysisMode::Histogram => "analysis.histogram_axes",
            AnalysisMode::Vectorscope => "analysis.oklab",
            _ => "analysis.waveform_axes",
        },
    );
    response.clone().on_hover_text(axes_help);
    let bins = match mode {
        AnalysisMode::Histogram | AnalysisMode::Vectorscope => BINS,
        _ => data.spatial_bins.max(1) as usize,
    };
    if let Some(pos) = response
        .hover_pos()
        .filter(|pos| rect.contains(*pos) && (!response.has_focus() || response.clicked()))
    {
        let mut x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 0.999_999);
        if mode == AnalysisMode::Parade {
            x = (x * 3.0).fract();
        }
        state.column = (x * bins as f32) as usize;
        state.row = (((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 0.999_999) * BINS as f32)
            as usize;
        result.inspecting = true;
    } else if response.has_focus() {
        let (left, right, up, down) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
            )
        });
        if left {
            state.column = state.column.saturating_sub(1);
        }
        if right {
            state.column = (state.column + 1).min(bins - 1);
        }
        if up {
            state.row = (state.row + 1).min(BINS - 1);
        }
        if down {
            state.row = state.row.saturating_sub(1);
        }
        result.inspecting = true;
    }
    state.column = state.column.min(bins - 1);
    state.row = state.row.min(BINS - 1);
    let mut occupied = [Rect::NOTHING; 3];
    let notice_count = messages.len();
    if result.inspecting {
        let start = state.column as f32 / bins as f32;
        let end = (state.column + 1) as f32 / bins as f32;
        if matches!(mode, AnalysisMode::Waveform | AnalysisMode::Parade) {
            result.band = Some((start, end));
            inspection_band(
                &painter,
                rect,
                start,
                end,
                if mode == AnalysisMode::Parade { 3 } else { 1 },
            );
            let value = (state.row as f32 + 0.5) / BINS as f32;
            messages.push(format!(
                "{:.1}–{:.1}%  {value:.3}",
                start * 100.0,
                end * 100.0
            ));
        } else if mode == AnalysisMode::Histogram {
            inspection_band(&painter, rect, start, end, 1);
            let mut text = format!("{start:.3}–{end:.3}");
            for &signal in signals {
                let count = data.histogram[signal]
                    .get(state.column)
                    .copied()
                    .unwrap_or(0);
                let name = if signal == 0 {
                    label(ui, "analysis.tone").into_owned()
                } else {
                    ["", "R", "G", "B"][signal].to_owned()
                };
                text.push_str(&format!(
                    "  {name} {:.2}%",
                    fraction(count, data.finite) * 100.0
                ));
            }
            messages.push(text);
        } else {
            let extent = 0.4 / prefs.chroma_zoom.clamp(1, 4) as f32;
            let a = -extent + (state.column as f32 + 0.5) * 2.0 * extent / BINS as f32;
            let b = -extent + (state.row as f32 + 0.5) * 2.0 * extent / BINS as f32;
            marker(&painter, vector_pos(rect, a, b, extent));
            let count = data
                .vectorscope
                .get(state.row * BINS + state.column)
                .copied()
                .unwrap_or(0);
            messages.push(format!(
                "a {a:.3}  b {b:.3}  {:.2}%",
                fraction(count, data.finite) * 100.0
            ));
        }
    } else if let Some(sample) = sample {
        occupied = sample_markers(&painter, rect, data, prefs, signals, sample);
        if !sample.pixel.valid {
            messages.push(label(ui, "analysis.invalid_sample").into_owned());
        } else {
            if !sample.inside {
                messages.push(label(ui, "analysis.outside_selection").into_owned());
            }
            let mut text = format!("({}, {})", sample.x, sample.y);
            if mode == AnalysisMode::Vectorscope {
                text.push_str(&format!(
                    "  a {:.3}  b {:.3}",
                    sample.pixel.ab[0], sample.pixel.ab[1]
                ));
            } else {
                for &signal in signals {
                    let (name, value) = if signal == 0 {
                        (label(ui, "analysis.tone").into_owned(), sample.pixel.tone)
                    } else {
                        (
                            ["", "R", "G", "B"][signal].to_owned(),
                            sample.pixel.rgb[signal - 1],
                        )
                    };
                    text.push_str(&format!("  {name} {value:.3}"));
                }
            }
            messages.push(text);
            let off_scale = if mode == AnalysisMode::Vectorscope {
                let extent = 0.4 / prefs.chroma_zoom.clamp(1, 4) as f64;
                sample.pixel.ab.iter().any(|v| v.abs() > extent)
            } else {
                signals.iter().any(|&s| {
                    let v = if s == 0 {
                        sample.pixel.tone
                    } else {
                        sample.pixel.rgb[s - 1]
                    };
                    !(0.0..=1.0).contains(&v)
                })
            };
            if off_scale {
                messages.push(label(ui, "analysis.off_scale").into_owned());
            }
        }
    }
    let readout_area = match mode {
        AnalysisMode::Vectorscope => {
            // Side space leaves hue references and the neutral center unobstructed.
            Rect::from_min_max(
                allocated.min,
                Pos2::new(rect.left() - 32.0, allocated.bottom()),
            )
        }
        AnalysisMode::Parade => Rect::from_min_max(rect.min + Vec2::new(0.0, 16.0), rect.max),
        _ => rect,
    };
    if !messages.is_empty() {
        let area = readout_rect(readout_area, occupied);
        let keyboard_scroll = if response.has_focus() {
            ui.input(|input| {
                (i32::from(input.key_pressed(egui::Key::PageDown))
                    - i32::from(input.key_pressed(egui::Key::PageUp))) as f32
                    * area.height()
            })
        } else {
            0.0
        };
        let _ = overlay(
            ui,
            area,
            &messages[..notice_count],
            &messages[notice_count..],
            keyboard_scroll,
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(finite: u64) -> AnalysisData {
        AnalysisData {
            histogram: std::array::from_fn(|_| vec![0; BINS]),
            waveform: Vec::new(),
            column_population: Vec::new(),
            vectorscope: Vec::new(),
            spatial_bins: 0,
            total: finite,
            finite,
            invalid: 0,
            below: 0,
            above: 0,
            outside_window: 0,
        }
    }

    #[test]
    fn trace_density_composites_between_surface_and_signal() {
        assert_eq!(composite(theme::SURFACE2, theme::FG, 0.0), theme::SURFACE2);
        assert_eq!(composite(theme::SURFACE2, theme::FG, 1.0), theme::FG);
        let visible = composite(theme::SURFACE2, theme::FG, density(0.01, 256.0));
        assert!(visible.r() > theme::SURFACE2.r() && visible.r() < theme::FG.r());
        assert_eq!(visible.a(), 255);
    }

    #[test]
    fn rgb_compositing_is_additive_and_order_independent() {
        let contributions = [(1, 0.2), (2, 0.7), (3, 0.4)];
        let forward = contributions
            .iter()
            .fold(theme::SURFACE2, |pixel, &(signal, value)| {
                add_waveform_channel(pixel, signal, value)
            });
        let reverse = contributions
            .iter()
            .rev()
            .fold(theme::SURFACE2, |pixel, &(signal, value)| {
                add_waveform_channel(pixel, signal, value)
            });
        assert_eq!(forward, reverse);
        for (signal, component) in [(1, 0), (2, 1), (3, 2)] {
            let base = theme::SURFACE2.to_array();
            let low = add_waveform_channel(theme::SURFACE2, signal, 0.2).to_array();
            let high = add_waveform_channel(theme::SURFACE2, signal, 0.7).to_array();
            assert!(base[component] < low[component] && low[component] < high[component]);
            for other in 0..3 {
                if other != component {
                    assert_eq!(low[other], base[other]);
                    assert_eq!(high[other], base[other]);
                }
            }
        }
        assert_eq!(histogram_color(&[1, 2, 3]), histogram_color(&[3, 1, 2]));
        let neutral = histogram_color(&[1, 2, 3]);
        assert!(neutral.r().abs_diff(neutral.b()) <= 3);
        assert_eq!(histogram_color(&[1, 2]).b(), theme::SURFACE2.b());
    }

    #[test]
    fn paired_peak_includes_hidden_channels_and_endpoints_with_population_normalization() {
        let mut original = data(100);
        let mut modified = data(200);
        original.histogram[0][40] = 40;
        original.histogram[3][0] = 70;
        modified.histogram[1][BINS - 1] = 160;
        let prefs = PlotPreferences {
            histogram_rgb: true,
            channels: [false, true, false],
            ..Default::default()
        };
        let (visible, count) = active_signals(&prefs);
        assert_eq!(&visible[..count], &[2]);
        assert_eq!(pair_peak(&original, None), 0.7);
        assert_eq!(pair_peak(&original, Some(&modified)), 0.8);
        assert_eq!(pair_peak(&modified, Some(&original)), 0.8);
    }

    #[test]
    fn waveform_density_is_fixed_by_frequency_not_population_or_peak() {
        let a = density(fraction(25, 100), 256.0);
        let b = density(fraction(250, 1000), 256.0);
        assert_eq!(a, b);
        let expected = (65.0_f64.ln() / 257.0_f64.ln()) as f32;
        assert!((a - expected).abs() < 1e-6);
        assert_eq!(density(0.0, 256.0), 0.0);
        assert_eq!(density(1.0, 256.0), 1.0);
    }

    #[test]
    fn tone_rgb_visibility_and_parade_are_independent() {
        let mut prefs = PlotPreferences {
            channels: [false, true, false],
            ..Default::default()
        };
        let (signals, count) = active_signals(&prefs);
        assert_eq!(&signals[..count], &[0]);
        prefs.histogram_rgb = true;
        let (signals, count) = active_signals(&prefs);
        assert_eq!(&signals[..count], &[2]);
        prefs.mode = PlotMode::Waveform;
        let (signals, count) = active_signals(&prefs);
        assert_eq!(&signals[..count], &[0]);
        prefs.waveform_view = WaveformView::RgbOverlay;
        let (signals, count) = active_signals(&prefs);
        assert_eq!(&signals[..count], &[2]);
        prefs.channels = [false; 3];
        assert_eq!(active_signals(&prefs).1, 0);
        prefs.waveform_view = WaveformView::RgbParade;
        let (signals, count) = active_signals(&prefs);
        assert_eq!(&signals[..count], &[1, 2, 3]);
    }

    #[test]
    fn sample_column_uses_pixel_centers_and_region_origin() {
        let columns: Vec<_> = (10..17).map(|x| sample_column(x, 10, 7, 3)).collect();
        assert_eq!(columns, [0, 0, 1, 1, 1, 2, 2]);
        let narrow: Vec<_> = (20..24).map(|x| sample_column(x, 20, 4, 4)).collect();
        assert_eq!(narrow, [0, 1, 2, 3]);
    }

    #[test]
    fn linked_readouts_leave_endpoint_and_rgb_markers_visible() {
        let ctx = egui::Context::default();
        let painter = ctx.layer_painter(egui::LayerId::background());
        let mut data = data(100);
        data.spatial_bins = 100;
        for height in [82.0, 110.0] {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(700.0, height));
            for (mode, view, histogram_rgb) in [
                (PlotMode::Histogram, WaveformView::Tone, false),
                (PlotMode::Histogram, WaveformView::Tone, true),
                (PlotMode::Waveform, WaveformView::Tone, false),
                (PlotMode::Waveform, WaveformView::RgbOverlay, false),
                (PlotMode::Waveform, WaveformView::RgbParade, false),
            ] {
                let prefs = PlotPreferences {
                    mode,
                    waveform_view: view,
                    histogram_rgb,
                    ..Default::default()
                };
                let (signals, count) = active_signals(&prefs);
                for x in [0, 50, 99] {
                    if mode == PlotMode::Histogram && x != 0 {
                        continue;
                    }
                    let sample = PlotSample {
                        pixel: AnalysisPixel {
                            rgb: [0.0, 0.5, 1.0],
                            tone: 1.0,
                            ab: [0.0; 2],
                            valid: true,
                        },
                        x,
                        y: 0,
                        region: [0, 0, 100, 1],
                        inside: true,
                    };
                    let occupied =
                        sample_markers(&painter, rect, &data, &prefs, &signals[..count], sample);
                    let readout = readout_rect(rect, occupied);
                    assert!(rect.contains_rect(readout));
                    assert!(readout.width() >= 100.0 && readout.height() >= 16.0);
                    for marker in occupied.into_iter().filter(|marker| marker.is_positive()) {
                        assert!(
                            !readout.intersects(marker),
                            "readout covers a linked marker: {readout:?}, {marker:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn long_notices_scroll_without_hiding_active_pixel_values() {
        use egui_kittest::kittest::Queryable;
        let readout = vec!["(1024, 512) R 0.250 G 0.500 B 0.750".to_owned()];
        let messages = vec![
            "Earlier result; Original and Modified cover different source areas.".to_owned(),
            "Below 10; Above 20; Invalid 1 / 100; Outside chroma window 25 (25%).".to_owned(),
        ];
        let mut h = egui_kittest::Harness::new_ui_state(
            |ui, state: &mut (f32, f32, Rect, Rect, f32)| {
                let bounds = Rect::from_min_size(ui.max_rect().min, Vec2::new(170.0, 82.0));
                let scroll = std::mem::take(&mut state.4);
                let output = overlay(ui, bounds, &messages, &readout, scroll).unwrap();
                state.0 = output.state.offset.y;
                state.1 = output.content_size.y - output.inner_rect.height();
                state.2 = output.inner_rect;
                state.3 = output.inner.rect;
                assert!(bounds.contains_rect(output.inner_rect));
            },
            (0.0, 0.0, Rect::NOTHING, Rect::NOTHING, 0.0),
        );
        h.run_steps(5);
        let before = h.state().2;
        let pinned = h.get_by_label(&readout[0]).raw_bounds().unwrap();
        assert!(pinned.y1 <= f64::from(before.top()));
        assert!(h.state().1 > 0.0, "this case must require scrolling");
        h.state_mut().4 = 10000.0;
        h.run_steps(12);
        assert!(
            (h.state().0 - h.state().1).abs() < 1.0,
            "the last readout line must remain reachable"
        );
        assert!(h.state().3.bottom() <= h.state().2.bottom() + 1.0);
        assert_eq!(
            before,
            h.state().2,
            "scrolling must not resize the plot region"
        );
        assert_eq!(
            pinned,
            h.get_by_label(&readout[0]).raw_bounds().unwrap(),
            "scrolling notices must not hide the active sample"
        );
    }
}
