//! RGB histogram computation and rendering for the tonal-distribution readout.
//!
//! The histogram reads the retained output image and renders as a compact
//! strip in the control panel. It is viewer-only and never affects saved
//! or exported pixels.

use egui::{Color32, Pos2, Rect, Ui};
use spektrafilm_math::image::ImageBuf;
use std::sync::Arc;

/// Number of histogram bins per channel.
const BINS: usize = 64;

/// Computed histogram data for one image.
#[derive(Clone, Debug, Default)]
pub struct HistogramData {
    /// Red channel bins, normalized to [0, 1].
    pub r: Vec<f32>,
    /// Green channel bins, normalized to [0, 1].
    pub g: Vec<f32>,
    /// Blue channel bins, normalized to [0, 1].
    pub b: Vec<f32>,
}

impl HistogramData {
    /// Compute histogram data from an image buffer.
    pub fn compute(image: &ImageBuf) -> Self {
        let mut data = Self {
            r: vec![0.0; BINS],
            g: vec![0.0; BINS],
            b: vec![0.0; BINS],
        };
        let pixel_count = image.width * image.height;
        if pixel_count == 0 {
            return data;
        }
        let total = pixel_count as f32;
        for y in 0..image.height {
            for x in 0..image.width {
                let i = (y * image.width + x) as usize * 3;
                let r = image.data[i].max(0.0).min(1.0);
                let g = image.data[i + 1].max(0.0).min(1.0);
                let b = image.data[i + 2].max(0.0).min(1.0);
                let r_bin = (r * (BINS - 1) as f32).round() as usize;
                let g_bin = (g * (BINS - 1) as f32).round() as usize;
                let b_bin = (b * (BINS - 1) as f32).round() as usize;
                data.r[r_bin] += 1.0 / total;
                data.g[g_bin] += 1.0 / total;
                data.b[b_bin] += 1.0 / total;
            }
        }
        // Normalize to max = 1.0 for display
        let max_val = data
            .r
            .iter()
            .chain(data.g.iter())
            .chain(data.b.iter())
            .copied()
            .fold(0.0f32, f32::max);
        if max_val > 0.0 {
            for v in data.r.iter_mut() {
                *v /= max_val;
            }
            for v in data.g.iter_mut() {
                *v /= max_val;
            }
            for v in data.b.iter_mut() {
                *v /= max_val;
            }
        }
        data
    }

    /// Render the histogram into the given UI area.
    pub fn render(&self, ui: &mut Ui, rect: Rect) {
        let painter = ui.painter_at(rect);
        let bin_w = rect.width() / BINS as f32;
        let h = rect.height();

        // Background
        painter.rect_filled(rect, 2.0, Color32::from_rgb(0x1a, 0x1a, 0x1d));

        // Draw each channel as a filled area
        for (bins, color) in [
            (
                &self.r,
                Color32::from_rgba_premultiplied(0xe0, 0x55, 0x55, 150),
            ),
            (
                &self.g,
                Color32::from_rgba_premultiplied(0x6a, 0xbf, 0x69, 150),
            ),
            (
                &self.b,
                Color32::from_rgba_premultiplied(0x6a, 0x9a, 0xd4, 150),
            ),
        ] {
            let mut points = Vec::with_capacity(BINS + 2);
            points.push(Pos2::new(rect.left(), rect.bottom()));
            for i in 0..BINS {
                let x = rect.left() + i as f32 * bin_w;
                let y = rect.bottom() - bins[i] * h;
                points.push(Pos2::new(x, y));
            }
            points.push(Pos2::new(rect.right(), rect.bottom()));
            painter.add(egui::Shape::convex_polygon(
                points,
                color,
                egui::Stroke::NONE,
            ));
        }
    }
}

/// State for the histogram strip in the control panel.
#[derive(Clone, Debug)]
pub struct HistogramStrip {
    /// Whether the strip is visible.
    pub visible: bool,
    /// Cached histogram data.
    data: Option<HistogramData>,
    /// Source image identity for cache invalidation.
    source_id: Option<u64>,
}

impl Default for HistogramStrip {
    fn default() -> Self {
        Self {
            visible: true,
            data: None,
            source_id: None,
        }
    }
}

impl HistogramStrip {
    /// Create a new histogram strip, default visible.
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the histogram from the current output image.
    pub fn update(&mut self, output_image: &Option<Arc<ImageBuf>>, generation: u64) {
        if let Some(img) = output_image {
            if self.source_id != Some(generation) {
                self.data = Some(HistogramData::compute(img));
                self.source_id = Some(generation);
            }
        } else {
            self.data = None;
            self.source_id = None;
        }
    }

    /// Render the histogram strip. Returns the height consumed.
    pub fn render(&mut self, ui: &mut Ui) -> f32 {
        if !self.visible {
            return 0.0;
        }
        let available_width = ui.available_width();
        let height = 80.0;
        let rect = Rect::from_min_size(ui.cursor().min, egui::vec2(available_width, height));
        ui.allocate_rect(rect, egui::Sense::hover());
        if let Some(data) = &self.data {
            data.render(ui, rect);
        } else {
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 2.0, Color32::from_rgb(0x1a, 0x1a, 0x1d));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No output",
                egui::FontId::proportional(10.0),
                Color32::from_rgb(0x6a, 0x6a, 0x72),
            );
        }
        // Legend
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("R")
                    .color(Color32::from_rgb(0xe0, 0x55, 0x55))
                    .size(9.0),
            );
            ui.label(
                egui::RichText::new("G")
                    .color(Color32::from_rgb(0x6a, 0xbf, 0x69))
                    .size(9.0),
            );
            ui.label(
                egui::RichText::new("B")
                    .color(Color32::from_rgb(0x6a, 0x9a, 0xd4))
                    .size(9.0),
            );
        });
        height + 14.0
    }

    /// Toggle visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}
