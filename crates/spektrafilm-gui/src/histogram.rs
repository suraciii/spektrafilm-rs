//! RGB histogram computation and rendering for the tonal-distribution readout.
//!
//! The histogram reads the retained output image and renders as a compact
//! strip in the control panel. It is viewer-only and never affects saved
//! or exported pixels.
//!
//! Each channel is passed through the sRGB OETF before binning (retaining the
//! existing readout rule even for encoded output). The encoded tone is clamped
//! into 64 bins, and all three channels share one peak normalization.
//!
//! GPU renders reduce their retained output before atomic publication and
//! return only the 192 normalized bins. CPU reference images are binned on a
//! dedicated worker: the readout keeps its previous distribution until that
//! replacement arrives. Neither route requests a second render.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use crate::theme;
use egui::{Context, Pos2, Rect, Ui};
use spektrafilm_math::colorspace::srgb_encode_f64;
use spektrafilm_math::image::ImageBuf;

/// Number of histogram bins per channel.
const BINS: usize = 64;

/// Computed histogram data for one image, binned over display-referred
/// (sRGB-encoded) tone.
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
    pub(crate) fn from_bins(bins: [[f32; BINS]; 3]) -> Self {
        let [r, g, b] = bins;
        Self {
            r: r.into(),
            g: g.into(),
            b: b.into(),
        }
    }

    /// Compute histogram data from an image buffer.
    ///
    /// Each channel of the linear output RGB is converted to a display-referred
    /// tone with the sRGB OETF before binning, and the encoded value is clamped
    /// to `0..=1`. The binning math runs in `f64` so it is identical under both
    /// `Scalar` precision settings; the bins themselves stay `f32` for display.
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
        let scale = (BINS - 1) as f64;
        for y in 0..image.height {
            for x in 0..image.width {
                let i = (y * image.width + x) as usize * 3;
                let r = srgb_encode_f64(image.data[i] as f64).clamp(0.0, 1.0);
                let g = srgb_encode_f64(image.data[i + 1] as f64).clamp(0.0, 1.0);
                let b = srgb_encode_f64(image.data[i + 2] as f64).clamp(0.0, 1.0);
                let r_bin = (r * scale).round() as usize;
                let g_bin = (g * scale).round() as usize;
                let b_bin = (b * scale).round() as usize;
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
        painter.rect_filled(rect, theme::RADIUS_CARD, theme::SURFACE2);

        // Draw each channel as a filled area
        for (bins, color) in [
            (
                &self.r,
                theme::SERIES1.gamma_multiply(theme::SERIES_FILL_ALPHA),
            ),
            (
                &self.g,
                theme::SERIES2.gamma_multiply(theme::SERIES_FILL_ALPHA),
            ),
            (
                &self.b,
                theme::SERIES3.gamma_multiply(theme::SERIES_FILL_ALPHA),
            ),
        ] {
            let mut points = Vec::with_capacity(BINS + 2);
            points.push(Pos2::new(rect.left(), rect.bottom()));
            for (i, bin) in bins.iter().enumerate() {
                let x = rect.left() + i as f32 * bin_w;
                let y = rect.bottom() - bin * h;
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

/// Allocation identities and operation IDs are distinct domains. The sequence
/// also prevents a recycled host allocation from adopting old worker results.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceGeneration {
    Host(u64),
    Device(u64),
}

type SourceId = (SourceGeneration, u64);

/// One binning request: the output image to bin and the identity its result
/// carries back to the readout.
struct HistogramRequest {
    source: SourceId,
    image: Arc<ImageBuf>,
    ctx: Context,
}

/// Background binning for the readout.
///
/// The worker holds one image at a time: a request that arrives while an
/// earlier one is still queued replaces it, so a run of renders does not
/// build a backlog of distributions nobody will look at.
struct HistogramWorker {
    requests: Option<Sender<HistogramRequest>>,
    results: Receiver<(SourceId, HistogramData)>,
    handle: Option<JoinHandle<()>>,
}

impl HistogramWorker {
    fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("spektrafilm-histogram".into())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let mut latest: HistogramRequest = request;
                    while let Ok(newer) = request_rx.try_recv() {
                        latest = newer;
                    }
                    let data = HistogramData::compute(&latest.image);
                    if result_tx.send((latest.source, data)).is_err() {
                        return; // the readout that asked for it is gone
                    }
                    // The result lands outside any frame, so wake the UI.
                    latest.ctx.request_repaint();
                }
            })
            .expect("OS thread spawn");
        Self {
            requests: Some(request_tx),
            results: result_rx,
            handle: Some(handle),
        }
    }

    fn submit(&mut self, source: SourceId, image: Arc<ImageBuf>, ctx: &Context) {
        if let Some(requests) = &self.requests {
            let _ = requests.send(HistogramRequest {
                source,
                image,
                ctx: ctx.clone(),
            });
        }
    }

    /// The next finished computation, if the worker has reported one.
    fn next_result(&self) -> Option<(SourceId, HistogramData)> {
        self.results.try_recv().ok()
    }
}

impl Drop for HistogramWorker {
    fn drop(&mut self) {
        // Closing the request channel ends the worker's loop; joining keeps
        // the thread from outliving the readout that owns it.
        self.requests.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// State for the histogram strip in the control panel.
pub struct HistogramStrip {
    /// Whether the strip is expanded. Default-on; the collapse state is
    /// persisted as the readout's visibility state.
    pub visible: bool,
    /// Cached histogram data for the output the strip shows or is waiting for.
    data: Option<HistogramData>,
    /// Identity of the output image the cached data belongs to.
    source_id: Option<SourceId>,
    /// Sequence handed to the next accepted image.
    sequence: u64,
    /// Worker that bins new output images off the UI thread.
    worker: HistogramWorker,
}

impl Default for HistogramStrip {
    fn default() -> Self {
        Self {
            visible: true,
            data: None,
            source_id: None,
            sequence: 0,
            worker: HistogramWorker::new(),
        }
    }
}

impl HistogramStrip {
    /// Create a new histogram strip, default visible.
    pub fn new() -> Self {
        Self::default()
    }

    /// Serialize the persisted visibility state.
    pub fn persistent_state(&self) -> serde_json::Value {
        serde_json::json!({"visible": self.visible})
    }

    /// Restore the persisted visibility state; a missing or malformed value
    /// keeps the default-on state.
    pub fn restore_state(&mut self, value: &serde_json::Value) {
        if let Some(visible) = value.get("visible").and_then(|v| v.as_bool()) {
            self.visible = visible;
        }
    }
    /// Adopt the reduction completed in the same transaction as a retained
    /// GPU output. No image samples or numerical reduction reach the UI.
    pub(crate) fn update_retained(&mut self, data: &HistogramData, generation: u64) {
        let generation = SourceGeneration::Device(generation);
        if self.source_id.map(|source| source.0) != Some(generation) {
            self.source_id = Some((generation, self.sequence));
            self.sequence += 1;
            self.data = Some(data.clone());
        }
    }

    /// Update the histogram from the current output image.
    ///
    /// The image itself is binned on the worker thread, so this returns
    /// without a pass over the pixels: it adopts a finished computation and
    /// hands a newly accepted output to the worker. The cached data stays on
    /// screen until its replacement arrives, so a new render never blanks the
    /// readout. A result for an output the strip has already moved past is
    /// dropped, including one whose image shared an address with the output
    /// that replaced it.
    pub fn update(&mut self, output_image: &Option<Arc<ImageBuf>>, generation: u64, ctx: &Context) {
        let generation = SourceGeneration::Host(generation);
        while let Some((finished, data)) = self.worker.next_result() {
            if self.source_id == Some(finished) {
                self.data = Some(data);
            }
        }
        match output_image {
            Some(image) => {
                if self.source_id.map(|source| source.0) != Some(generation) {
                    let accepted = (generation, self.sequence);
                    self.source_id = Some(accepted);
                    self.sequence += 1;
                    self.worker.submit(accepted, Arc::clone(image), ctx);
                }
            }
            None => {
                self.data = None;
                self.source_id = None;
            }
        }
    }

    /// Whether the strip's paint path has retained histogram data.
    #[cfg(test)]
    pub(crate) fn cached_data(&self) -> Option<&HistogramData> {
        self.data.as_ref()
    }

    /// Render the histogram strip. The header row stays visible when the strip
    /// is collapsed so the readout can be re-expanded.
    pub fn render(&mut self, ui: &mut Ui) {
        let response =
            egui::CollapsingHeader::new(crate::i18n::text(ui.ctx(), "surfaces.histogram.title"))
                .id_salt("Tonal distribution")
                .open(Some(self.visible))
                .show(ui, |ui| {
                    let available_width = ui.available_width();
                    let height = 80.0;
                    let rect =
                        Rect::from_min_size(ui.cursor().min, egui::vec2(available_width, height));
                    ui.allocate_rect(rect, egui::Sense::hover());
                    if let Some(data) = &self.data {
                        data.render(ui, rect);
                    } else {
                        let painter = ui.painter_at(rect);
                        painter.rect_filled(rect, theme::RADIUS_CARD, theme::SURFACE2);
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            crate::i18n::text(ui.ctx(), "surfaces.histogram.empty"),
                            theme::text(theme::TEXT_READOUT, theme::SIZE_CHART_AXIS),
                            theme::FG_MUTED,
                        );
                    }
                    // Legend
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("R")
                                .color(theme::SERIES1)
                                .size(theme::SIZE_HINT),
                        );
                        ui.label(
                            egui::RichText::new("G")
                                .color(theme::SERIES2)
                                .size(theme::SIZE_HINT),
                        );
                        ui.label(
                            egui::RichText::new("B")
                                .color(theme::SERIES3)
                                .size(theme::SIZE_HINT),
                        );
                    });
                });
        // `.open(Some(..))` makes egui mirror our state instead of owning it,
        // so the strip is the single source of truth for the collapse state.
        if response.header_response.clicked() {
            self.visible = !self.visible;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak_bin(bins: &[f32]) -> usize {
        bins.iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap()
    }

    #[test]
    fn bins_use_display_referred_srgb_tone() {
        // Linear 0.5 encodes to display tone ~0.7354 -> bin 46, not the
        // linear midpoint bin 32 the old pre-encode binning produced.
        let image = ImageBuf::from_data(
            2,
            1,
            vec![0.5 as _, 0.5 as _, 0.5 as _, 0.5 as _, 0.5 as _, 0.5 as _],
        );
        let data = HistogramData::compute(&image);
        assert_eq!(peak_bin(&data.r), 46);
        assert_eq!(peak_bin(&data.g), 46);
        assert_eq!(peak_bin(&data.b), 46);
    }

    #[test]
    fn out_of_gamut_linear_values_clamp_to_edge_bins() {
        // -0.25 linear is out of gamut (below 0) and 4.0 linear exceeds
        // display white; both clamp after encoding to the edge bins.
        let image = ImageBuf::from_data(
            2,
            1,
            vec![-0.25 as _, 0.0 as _, 0.0 as _, 4.0 as _, 0.0 as _, 0.0 as _],
        );
        let data = HistogramData::compute(&image);
        assert!(data.r[0] > 0.0);
        assert!(data.r[BINS - 1] > 0.0);
        assert!(data.r[1..BINS - 1].iter().all(|&v| v == 0.0));
    }
    #[test]
    fn retained_histogram_matches_reference_at_every_bin_boundary_and_nonfinite_value() {
        use spektrafilm_backend::{Precision, RetainedImageMeta, SampleDomain};
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let context = spektrafilm_backend::telemetry::ObservationContext::new(
            spektrafilm_backend::telemetry::CollectionMode::Summary,
        );
        let observed = gpu
            .backend()
            .with_observation_context(context.clone())
            .unwrap();
        let mut values = vec![
            f32::NEG_INFINITY,
            -1.0,
            -0.0,
            0.0,
            f32::from_bits(1),
            1.0,
            4.0,
            f32::INFINITY,
            f32::NAN,
            f32::from_bits(0xffc00001),
        ];
        for bin in 0..63 {
            // Invert the actual CPU rule independently, then straddle the
            // adjacent representable values rather than trusting shader pow.
            let mut low = 0u32;
            let mut high = 1.0f32.to_bits();
            while low < high {
                let middle = low + (high - low) / 2;
                if (srgb_encode_f64(f32::from_bits(middle) as f64) * 63.0).round() as usize > bin {
                    high = middle;
                } else {
                    low = middle + 1;
                }
            }
            values.extend([
                f32::from_bits(low - 1),
                f32::from_bits(low),
                f32::from_bits(low + 1),
            ]);
        }
        let image = ImageBuf::from_data(
            values.len() as u32,
            1,
            values
                .iter()
                .enumerate()
                .flat_map(|(i, &v)| {
                    [
                        v as _,
                        values[(i * 7) % values.len()] as _,
                        if i % 3 == 0 { 0.0 as _ } else { v as _ },
                    ]
                })
                .collect(),
        );
        let retained = gpu
            .backend()
            .retain_image(
                &image,
                &RetainedImageMeta {
                    width: image.width,
                    height: image.height,
                    domain: SampleDomain::Linear,
                    precision: Precision::F32,
                    space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
                    pixel_pitch_um: None,
                },
            )
            .unwrap();
        let actual = HistogramData::from_bins(observed.retained_histogram(&retained).unwrap());
        assert_eq!(context.snapshot().totals.readback_bytes, Some(192 * 4));
        let expected = HistogramData::compute(&image);
        for (actual, expected) in [&actual.r, &actual.g, &actual.b].into_iter().zip([
            &expected.r,
            &expected.g,
            &expected.b,
        ]) {
            for (bin, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(
                    actual == 0.0,
                    expected == 0.0,
                    "bin {bin} occupancy differs"
                );
                assert!(
                    (actual - expected).abs() < 2e-6,
                    "bin {bin}: device {actual}, reference {expected}"
                );
            }
        }
        // Inspection is an explicitly requested source pixel, not a display
        // transform or histogram approximation, and reductions never edit it.
        let pixel = observed.inspect_retained_pixel(&retained, 1, 0).unwrap();
        assert_eq!(context.snapshot().totals.readback_bytes, Some(192 * 4 + 12));
        for (actual, expected) in pixel.into_iter().zip(&image.data[3..6]) {
            assert_eq!(actual.to_bits(), (*expected as f32).to_bits());
        }
    }

    /// Pump `update` until the worker has reported a result. `update` only
    /// adopts work the worker already finished, so repeating it with the same
    /// arguments neither resubmits the image nor discards a finished result.
    fn settle(
        strip: &mut HistogramStrip,
        output_image: &Option<Arc<ImageBuf>>,
        generation: u64,
        ctx: &egui::Context,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while strip.cached_data().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "the histogram worker never reported a result"
            );
            strip.update(output_image, generation, ctx);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn flat_image(value: f32) -> Option<Arc<ImageBuf>> {
        Some(Arc::new(ImageBuf::from_data(
            2,
            1,
            vec![
                value as _, value as _, value as _, value as _, value as _, value as _,
            ],
        )))
    }

    #[test]
    fn binning_runs_off_the_ui_thread() {
        let ctx = egui::Context::default();
        let image = flat_image(0.5);
        let mut strip = HistogramStrip::new();
        strip.update(&image, 1, &ctx);
        assert!(
            strip.cached_data().is_none(),
            "the frame must not bin the image itself"
        );
        settle(&mut strip, &image, 1, &ctx);
        let data = strip.cached_data().expect("worker reported a result");
        assert_eq!(peak_bin(&data.r), 46);
        assert_eq!(peak_bin(&data.g), 46);
        assert_eq!(peak_bin(&data.b), 46);
    }

    #[test]
    fn the_readout_ends_on_the_output_it_is_showing() {
        // Two outputs land before the worker reports the first one. Whichever
        // request the worker picks up, the readout settles on the second
        // image's distribution, never on the one a later render replaced.
        let ctx = egui::Context::default();
        let first = flat_image(0.5);
        let second = flat_image(0.25);
        let mut strip = HistogramStrip::new();
        strip.update(&first, 1, &ctx);
        strip.update(&second, 2, &ctx);
        settle(&mut strip, &second, 2, &ctx);
        let data = strip.cached_data().expect("worker reported a result");
        // 0.25 linear encodes to display tone ~0.5371 -> bin 34, not the
        // bin 46 of the superseded image.
        assert_eq!(peak_bin(&data.r), 34);
        assert_eq!(peak_bin(&data.g), 34);
        assert_eq!(peak_bin(&data.b), 34);
    }

    #[test]
    fn a_new_render_keeps_the_previous_distribution_until_its_replacement_arrives() {
        let ctx = egui::Context::default();
        let first = flat_image(0.5);
        let second = flat_image(0.25);
        let mut strip = HistogramStrip::new();
        strip.update(&first, 1, &ctx);
        settle(&mut strip, &first, 1, &ctx);
        strip.update(&second, 2, &ctx);
        let data = strip
            .cached_data()
            .expect("the readout keeps the previous distribution while binning");
        assert_eq!(peak_bin(&data.r), 46);
    }

    #[test]
    fn a_recycled_generation_cannot_adopt_a_superseded_distribution() {
        // Loading an image clears the retained output, which frees the `Arc`
        // the readout asked about; a later output can be allocated at that
        // same address. The identity of the accepted image must therefore
        // survive the address being reused, or a result for the image that
        // went away is adopted as the image that took its place.
        let ctx = egui::Context::default();
        let first = flat_image(0.5);
        let second = flat_image(0.25);
        let (requests, request_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let mut strip = HistogramStrip {
            visible: true,
            data: None,
            source_id: None,
            sequence: 0,
            worker: HistogramWorker {
                requests: Some(requests),
                results: result_rx,
                handle: None,
            },
        };

        strip.update(&first, 7, &ctx);
        let superseded = request_rx.try_recv().expect("the worker is asked to bin");
        // The output goes away and a later one is accepted at the same
        // generation before the worker answers the request it was given.
        strip.update(&None, 7, &ctx);
        strip.update(&second, 7, &ctx);
        let accepted = request_rx.try_recv().expect("the replacement is binned");
        result_tx
            .send((superseded.source, HistogramData::compute(&superseded.image)))
            .unwrap();

        // The superseded result lands while the readout shows the image that
        // took its place, so it must be dropped rather than adopted.
        strip.update(&second, 7, &ctx);
        assert!(
            strip.cached_data().is_none(),
            "a result for an output the readout moved past is not adopted"
        );

        result_tx
            .send((accepted.source, HistogramData::compute(&accepted.image)))
            .unwrap();
        strip.update(&second, 7, &ctx);
        let data = strip.cached_data().expect("worker reported a result");
        assert_eq!(
            peak_bin(&data.r),
            34,
            "the readout shows the image it is showing, not the one it replaced"
        );
    }
    #[test]
    fn retained_bins_cannot_adopt_a_pending_host_distribution_with_the_same_numeric_identity() {
        let ctx = egui::Context::default();
        let image = flat_image(0.5);
        let mut strip = HistogramStrip::new();
        strip.update(&image, 7, &ctx);
        let retained = HistogramData::compute(&flat_image(0.25).unwrap());
        strip.update_retained(&retained, 7);
        assert_eq!(peak_bin(&strip.cached_data().unwrap().r), 34);
        strip.update(&None, 0, &ctx);
        assert!(strip.cached_data().is_none());
    }

    #[test]
    fn visibility_state_persists_across_restore() {
        let mut strip = HistogramStrip::new();
        assert!(strip.visible, "the strip is default-on");
        strip.visible = false;
        let state = strip.persistent_state();
        let mut restored = HistogramStrip::new();
        restored.restore_state(&state);
        assert!(!restored.visible, "a collapsed strip restores collapsed");
        restored.visible = true;
        restored.restore_state(&strip.persistent_state());
        assert!(!restored.visible, "re-expansion also round-trips");
    }

    #[test]
    fn restore_state_keeps_default_on_for_malformed_values() {
        let mut strip = HistogramStrip::new();
        strip.restore_state(&serde_json::json!({}));
        assert!(strip.visible);
        strip.restore_state(&serde_json::json!({"visible": "yes"}));
        assert!(
            strip.visible,
            "a non-boolean value does not collapse the strip"
        );
    }
}
