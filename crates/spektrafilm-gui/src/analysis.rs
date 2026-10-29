//! Demand-driven analysis and linked sampling of accepted image carriers.
use parking_lot::{Condvar, Mutex};
use spektrafilm_backend::analysis::{self, AnalysisData, AnalysisPixel, AnalysisRequest};
use spektrafilm_backend::{ComputeBackend, DeviceImage, RetainedImageMeta};
use spektrafilm_math::image::ImageBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::JoinHandle;

#[derive(Clone)]
pub(crate) enum Source {
    Host(Arc<ImageBuf>, RetainedImageMeta),
    Device(DeviceImage),
}

struct Slot<T> {
    request: Option<T>,
    stop: bool,
    active: Option<Arc<AtomicBool>>,
}
struct Worker<T, R> {
    slot: Arc<(Mutex<Slot<T>>, Condvar)>,
    results: mpsc::Receiver<R>,
    thread: Option<JoinHandle<()>>,
}
impl<T: Send + 'static, R: Send + 'static> Worker<T, R> {
    fn new<W: FnOnce()>(
        name: &str,
        mut run: impl FnMut(T, Arc<AtomicBool>) -> (R, W) + Send + 'static,
    ) -> Self {
        let slot = Arc::new((
            Mutex::new(Slot {
                request: None,
                stop: false,
                active: None,
            }),
            Condvar::new(),
        ));
        let shared = slot.clone();
        let (tx, results) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                loop {
                    let (request, cancel) = {
                        let (lock, wake) = &*shared;
                        let mut state = lock.lock();
                        while state.request.is_none() && !state.stop {
                            wake.wait(&mut state);
                        }
                        if state.stop {
                            break;
                        }
                        let request = state.request.take().unwrap();
                        let cancel = Arc::new(AtomicBool::new(false));
                        state.active = Some(cancel.clone());
                        (request, cancel)
                    };
                    let (result, wake) = run(request, cancel.clone());
                    if !cancel.load(Ordering::Relaxed) {
                        if tx.send(result).is_err() {
                            break;
                        }
                        wake();
                    }
                }
            })
            .expect("analysis worker thread");
        Self {
            slot,
            results,
            thread: Some(thread),
        }
    }
    fn submit(&self, request: T) {
        let (lock, wake) = &*self.slot;
        let mut state = lock.lock();
        if let Some(cancel) = &state.active {
            cancel.store(true, Ordering::Relaxed);
        }
        state.request = Some(request);
        wake.notify_one();
    }
    fn clear(&self) {
        let mut state = self.slot.0.lock();
        if let Some(cancel) = &state.active {
            cancel.store(true, Ordering::Relaxed);
        }
        state.request = None;
    }
    fn receive(&self) -> Option<R> {
        self.results.try_recv().ok()
    }
}
impl<T, R> Drop for Worker<T, R> {
    fn drop(&mut self) {
        let (lock, wake) = &*self.slot;
        let mut state = lock.lock();
        state.stop = true;
        if let Some(cancel) = &state.active {
            cancel.store(true, Ordering::Relaxed);
        }
        wake.notify_one();
        drop(state);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Identity {
    pub original_generation: u64,
    pub modified_generation: Option<u64>,
    pub region: Option<[f64; 4]>,
    pub original_region: Option<[u32; 4]>,
    pub modified_region: Option<[u32; 4]>,
    pub mode: analysis::AnalysisMode,
    pub zoom: u32,
}
struct DistributionJob {
    identity: Identity,
    serial: u64,
    sources: [Option<Source>; 2],
    backend: Arc<dyn ComputeBackend>,
    ctx: egui::Context,
}
pub(crate) struct DistributionResult {
    pub identity: Identity,
    serial: u64,
    pub sources: [Option<Source>; 2],
    pub data: [Option<Result<AnalysisData, String>>; 2],
}
struct SampleJob {
    serial: u64,
    source: Source,
    x: u32,
    y: u32,
    backend: Arc<dyn ComputeBackend>,
    ctx: egui::Context,
}
struct SampleResult {
    serial: u64,
    value: Result<AnalysisPixel, String>,
}

#[derive(PartialEq)]
struct SampleIdentity {
    distribution: Identity,
    subject: usize,
    point: [u32; 2],
    preferences: crate::analysis_plot::PlotPreferences,
    keyboard: bool,
}

pub(crate) struct Analysis {
    pub visible: bool,
    pub preferences: crate::analysis_plot::PlotPreferences,
    pub plot: crate::analysis_plot::PlotState,
    pub selection: Option<[f64; 4]>,
    pub original_generation: u64,
    pub original_space: String,
    pub original_encoded: bool,
    pub original_rotation: u32,
    active_subject: Option<(crate::display::ViewLayer, u64)>,
    pub identity: Option<Identity>,
    distribution_serial: u64,
    pub result: Option<DistributionResult>,
    sample_identity: Option<SampleIdentity>,
    sample_serial: u64,
    pub sample: Option<AnalysisPixel>,
    sample_failed: bool,
    distributions: Worker<DistributionJob, DistributionResult>,
    samples: Worker<SampleJob, SampleResult>,
}
impl Default for Analysis {
    fn default() -> Self {
        Self {
            visible: false,
            preferences: Default::default(),
            plot: Default::default(),
            selection: None,
            original_generation: 0,
            original_space: "sRGB".into(),
            original_encoded: false,
            original_rotation: 0,
            active_subject: None,
            identity: None,
            distribution_serial: 0,
            result: None,
            sample_identity: None,
            sample_serial: 0,
            sample: None,
            sample_failed: false,
            distributions: {
                let mut resident: [Option<(u64, DeviceImage)>; 2] = [None, None];
                Worker::new("image-analysis", move |mut job: DistributionJob, cancel| {
                    let derived = job.backend.with_cancellation(Some(cancel.clone()));
                    let backend = derived.as_deref().unwrap_or(job.backend.as_ref());
                    let regions = [job.identity.original_region, job.identity.modified_region];
                    let mut data = [None, None];
                    for index in 0..2 {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let (Some(source), Some(region)) =
                            (job.sources[index].as_mut(), regions[index])
                        else {
                            continue;
                        };
                        let request = AnalysisRequest {
                            region,
                            mode: job.identity.mode,
                            chroma_zoom: job.identity.zoom,
                        };
                        let result = match source {
                            Source::Host(image, meta) if backend.is_gpu() => {
                                let generation = if index == 0 {
                                    job.identity.original_generation
                                } else {
                                    job.identity.modified_generation.unwrap_or(0)
                                };
                                let cached = resident[index].as_ref().and_then(|(g, image)| {
                                    (*g == generation).then(|| image.clone())
                                });
                                let retained = cached
                                    .map(Ok)
                                    .unwrap_or_else(|| backend.retain_image(image, meta));
                                match retained {
                                    Ok(device) => {
                                        resident[index] = Some((generation, device.clone()));
                                        let result = backend.analyze_retained(&device, &request);
                                        *source = Source::Device(device);
                                        result
                                    }
                                    Err(error) => Err(error),
                                }
                            }
                            Source::Host(image, meta) => analysis::analyze_host(
                                image,
                                *meta,
                                &request,
                                Some(cancel.as_ref()),
                            ),
                            Source::Device(image) => backend.analyze_retained(image, &request),
                        };
                        data[index] = Some(result);
                    }
                    (
                        DistributionResult {
                            identity: job.identity,
                            serial: job.serial,
                            sources: job.sources,
                            data,
                        },
                        move || job.ctx.request_repaint(),
                    )
                })
            },
            samples: Worker::new("image-analysis-pixel", |job: SampleJob, cancel| {
                let derived = job.backend.with_cancellation(Some(cancel));
                let backend = derived.as_deref().unwrap_or(job.backend.as_ref());
                let value = match &job.source {
                    Source::Host(image, meta) => analysis::inspect_host(image, *meta, job.x, job.y),
                    Source::Device(image) => backend.inspect_analysis_pixel(image, job.x, job.y),
                };
                (
                    SampleResult {
                        serial: job.serial,
                        value,
                    },
                    move || job.ctx.request_repaint(),
                )
            }),
        }
    }
}
impl Analysis {
    pub fn persistent_state(&self) -> serde_json::Value {
        serde_json::json!({"visible": self.visible, "preferences": self.preferences})
    }
    /// Return whether restored preferences need publication in the current format.
    pub fn restore_state(&mut self, value: &serde_json::Value) -> bool {
        if let Some(visible) = value["visible"].as_bool() {
            self.visible = visible;
        }
        let mut preferences = value["preferences"].clone();
        let mut migrated = false;
        if let Some(fields) = preferences.as_object_mut() {
            let legacy_rgb = fields.remove("waveform_rgb");
            migrated = legacy_rgb.is_some();
            if fields.get("mode").and_then(|mode| mode.as_str()) == Some("parade") {
                fields.insert("mode".into(), serde_json::json!("waveform"));
                fields.insert("waveform_view".into(), serde_json::json!("rgb_parade"));
                migrated = true;
            } else if !fields.contains_key("waveform_view")
                && let Some(rgb) = legacy_rgb
            {
                let view = match rgb {
                    serde_json::Value::Bool(true) => serde_json::json!("rgb_overlay"),
                    serde_json::Value::Bool(false) => serde_json::json!("tone"),
                    invalid => invalid,
                };
                fields.insert("waveform_view".into(), view);
            }
        }
        if let Ok(mut prefs) =
            serde_json::from_value::<crate::analysis_plot::PlotPreferences>(preferences)
        {
            if ![1, 2, 4].contains(&prefs.chroma_zoom) {
                prefs.chroma_zoom = 1;
            }
            if !prefs.height.is_finite() {
                prefs.height = 180.0;
            }
            self.preferences = prefs;
            return migrated;
        }
        false
    }
    pub fn clear(&mut self) {
        self.distributions.clear();
        self.samples.clear();
        self.identity = None;
        self.result = None;
        self.distribution_serial = self.distribution_serial.wrapping_add(1);
        self.clear_sample();
    }
    pub fn clear_sample(&mut self) {
        self.samples.clear();
        self.sample_identity = None;
        self.sample = None;
        self.sample_failed = false;
        self.sample_serial = self.sample_serial.wrapping_add(1);
    }
    fn update(
        &mut self,
        identity: Identity,
        sources: [Option<Source>; 2],
        backend: Arc<dyn ComputeBackend>,
        ctx: &egui::Context,
    ) {
        if self.identity.as_ref() != Some(&identity) {
            self.clear_sample();
            self.result = None;
            self.distribution_serial = self.distribution_serial.wrapping_add(1);
            self.identity = Some(identity.clone());
            self.distributions.submit(DistributionJob {
                identity,
                serial: self.distribution_serial,
                sources,
                backend,
                ctx: ctx.clone(),
            });
        }
        while let Some(result) = self.distributions.receive() {
            if result.serial == self.distribution_serial
                && self.identity.as_ref() == Some(&result.identity)
            {
                self.plot.invalidate_cache();
                self.result = Some(result);
            }
        }
    }
    fn update_sample(
        &mut self,
        key: SampleIdentity,
        backend: Arc<dyn ComputeBackend>,
        ctx: &egui::Context,
    ) {
        if self.sample_identity.as_ref() != Some(&key) {
            let index = key.subject;
            let point = key.point;
            self.clear_sample();
            self.sample_identity = Some(key);
            if let Some(source) = self.result.as_ref().and_then(|r| r.sources[index].clone()) {
                self.samples.submit(SampleJob {
                    serial: self.sample_serial,
                    source,
                    x: point[0],
                    y: point[1],
                    backend,
                    ctx: ctx.clone(),
                });
            }
        }
        while let Some(response) = self.samples.receive() {
            if response.serial == self.sample_serial {
                self.sample_failed = response.value.is_err();
                self.sample = response.value.ok();
            }
        }
    }
}

fn image_band(region: [u32; 4], size: [u32; 2], start: f32, end: f32) -> [f64; 4] {
    let [width, height] = size.map(f64::from);
    let x0 = region[0] as f64;
    let span = (region[2] - region[0]) as f64;
    [
        (x0 + span * start as f64) / width,
        region[1] as f64 / height,
        (x0 + span * end as f64) / width,
        region[3] as f64 / height,
    ]
}

impl crate::App {
    pub(crate) fn show_image_analysis(&mut self, ui: &mut egui::Ui) {
        use crate::analysis_geometry::AnalysisGeometry;
        use crate::analysis_plot::{self, PlotSample};
        use crate::display::ViewLayer;
        let max_height = ui.available_height();
        let panel_top = ui.cursor().top();
        let layer = self.viewer.settings.layer;
        ui.horizontal_wrapped(|ui| {
            let close_label = crate::i18n::text(ui.ctx(), "surfaces.analysis.close");
            let close = crate::controls::icon_button(
                ui,
                close_label.as_ref(),
                crate::controls::Icon::Close,
            );
            if close.clicked() {
                self.analysis.visible = false;
            }
            analysis_plot::show_mode_controls(ui, &mut self.analysis.preferences);
            egui::menu::menu_button(ui, crate::i18n::text(ui.ctx(), "analysis.settings"), |ui| {
                analysis_plot::show_settings(ui, &mut self.analysis.preferences)
            });
            let selected = self.analysis.selection.is_some();
            if ui
                .selectable_label(
                    selected,
                    crate::i18n::text(ui.ctx(), "surfaces.analysis.select"),
                )
                .clicked()
                && layer != ViewLayer::PaperBack
            {
                self.viewer.arm_analysis_selection();
                self.analysis.clear_sample();
            }
            if ui
                .selectable_label(
                    !selected,
                    crate::i18n::text(ui.ctx(), "surfaces.analysis.full"),
                )
                .clicked()
            {
                self.analysis.selection = None;
                self.viewer.cancel_analysis_selection();
                self.analysis.clear_sample();
            }
            ui.label(egui::RichText::new("sRGB").color(crate::theme::FG_MUTED))
                .on_hover_text(crate::i18n::text(ui.ctx(), "analysis.interpretation"));
        });
        if !self.analysis.visible {
            return;
        }
        ui.add_space(crate::theme::SPACE_1);
        {
            let layer = self.viewer.settings.layer;
            let original_geometry = self.image.as_ref().map(|image| {
                AnalysisGeometry::original(
                    [image.width(), image.height()],
                    self.analysis.original_rotation,
                )
            });
            let geometries = [original_geometry, self.output_geometry];
            let index = usize::from(layer == ViewLayer::Output);
            let generation = if index == 0 {
                self.analysis.original_generation
            } else {
                self.source_render.as_ref().map_or(0, |r| r.operation_id)
            };
            let subject = (layer, generation);
            if self.analysis.active_subject != Some(subject) {
                self.viewer.cancel_analysis_selection();
                self.viewer.analysis.sample = None;
                self.analysis.clear_sample();
                self.analysis.active_subject = Some(subject);
            }
            if let Some(rect) = self.viewer.analysis.completed.take()
                && let Some(geometry) = geometries[index]
            {
                let source = geometry.source_rect_from_image(rect);
                if geometry.pixel_region(source).is_some() {
                    self.analysis.selection = Some(source);
                }
            }
            self.viewer.analysis.enabled = layer != ViewLayer::PaperBack;
            self.viewer.analysis.selection = self
                .analysis
                .selection
                .and_then(|s| geometries[index].and_then(|g| g.image_rect_from_source(s)));
            self.viewer.analysis.band = None;
            if layer == ViewLayer::PaperBack {
                self.analysis.clear();
                ui.label(crate::i18n::text(ui.ctx(), "surfaces.analysis.paper"));
                return;
            }
            let Some(geometry) = geometries[index] else {
                self.analysis.clear();
                let key = if layer == ViewLayer::Output
                    && (self.output_image.is_some() || self.output_retained.is_some())
                {
                    "surfaces.analysis.failed"
                } else {
                    "surfaces.analysis.no_image"
                };
                ui.label(crate::i18n::text(ui.ctx(), key));
                return;
            };
            let regions = geometries.map(|g| {
                g.and_then(|g| match self.analysis.selection {
                    Some(selection) => g.pixel_region(selection),
                    None => Some([0, 0, g.image_size()[0], g.image_size()[1]]),
                })
            });
            if regions[index].is_none() {
                self.analysis.clear();
                ui.label(crate::i18n::text(ui.ctx(), "surfaces.analysis.unavailable"));
                return;
            }
            let host_source = |image: &Arc<ImageBuf>, space: &str, encoded: bool| {
                spektrafilm_math::colorspace::resolve(space)
                    .ok()
                    .map(|space| {
                        Source::Host(
                            image.clone(),
                            RetainedImageMeta {
                                width: image.width,
                                height: image.height,
                                space,
                                domain: if encoded {
                                    spektrafilm_backend::SampleDomain::Encoded
                                } else {
                                    spektrafilm_backend::SampleDomain::Linear
                                },
                                precision: spektrafilm_backend::Precision::host(),
                                pixel_pitch_um: None,
                            },
                        )
                    })
            };
            let original = self
                .oriented_input
                .as_ref()
                .and_then(|source| match source {
                    crate::render::InputImage::Host(image) => self
                        .viewer
                        .analysis_retained(ViewLayer::Input)
                        .map(Source::Device)
                        .or_else(|| {
                            host_source(
                                image,
                                &self.analysis.original_space,
                                self.analysis.original_encoded,
                            )
                        }),
                    crate::render::InputImage::Device(image) => Some(Source::Device(image.clone())),
                });
            let modified = self
                .output_retained
                .clone()
                .map(Source::Device)
                .or_else(|| {
                    self.output_image.as_ref().and_then(|image| {
                        host_source(image, &self.output_color_space, self.output_cctf_encoding)
                    })
                });
            if (index == 0 && original.is_none()) || (index == 1 && modified.is_none()) {
                self.analysis.clear();
                ui.label(crate::i18n::text(ui.ctx(), "surfaces.analysis.no_image"));
                return;
            }
            let identity = Identity {
                original_generation: self.analysis.original_generation,
                modified_generation: self.source_render.as_ref().map(|r| r.operation_id),
                region: self.analysis.selection,
                original_region: regions[0],
                modified_region: regions[1],
                mode: self.analysis.preferences.analysis_mode(),
                zoom: self.analysis.preferences.chroma_zoom,
            };
            let has_counterpart = if index == 0 {
                modified.is_some()
            } else {
                original.is_some()
            };
            self.analysis.update(
                identity.clone(),
                [original, modified],
                self.backend.clone(),
                ui.ctx(),
            );
            if self.viewer.analysis_retained(ViewLayer::Input).is_none()
                && let Some(Source::Device(image)) = self
                    .analysis
                    .result
                    .as_ref()
                    .and_then(|r| r.sources[0].as_ref())
            {
                self.viewer.seed_prepared_device_input(
                    image.clone(),
                    self.params.settings.preview_max_size,
                );
            }
            let mut status = Vec::new();
            if layer == ViewLayer::Output && self.accepted_frame_is_stale() {
                status.push(crate::i18n::text(ui.ctx(), "surfaces.analysis.earlier").into_owned());
            }
            if layer == ViewLayer::Output && self.retained_kind == Some(crate::RenderKind::Preview)
            {
                status.push(
                    crate::i18n::text(ui.ctx(), "surfaces.analysis.preview_limit").into_owned(),
                );
            }
            if self.analysis.selection.is_none()
                && geometries[0]
                    .zip(geometries[1])
                    .is_some_and(|(a, b)| a.coverage() != b.coverage())
            {
                status.push(crate::i18n::text(ui.ctx(), "surfaces.analysis.framing").into_owned());
            }
            if !status.is_empty() {
                ui.label(status.join(" · "));
            }
            let suspended = self.viewer.analysis.suspended
                || self.operator_compare
                || self.operator_comparison.is_some();
            let target = self
                .viewer
                .analysis
                .sample
                .filter(|sample| sample.layer == layer && !suspended);
            if suspended {
                self.analysis.clear_sample();
                ui.label(crate::i18n::text(ui.ctx(), "surfaces.analysis.suspended"));
            } else if let Some(sample) = target {
                if self.analysis.result.is_some() {
                    self.analysis.update_sample(
                        SampleIdentity {
                            distribution: identity.clone(),
                            subject: index,
                            point: [sample.x, sample.y],
                            preferences: self.analysis.preferences,
                            keyboard: sample.keyboard,
                        },
                        self.backend.clone(),
                        ui.ctx(),
                    );
                }
            } else {
                self.analysis.clear_sample();
            }
            if self.analysis.sample_failed {
                ui.label(crate::i18n::text(ui.ctx(), "analysis.sample_unavailable"));
            }
            let Some(result) = self.analysis.result.as_ref() else {
                ui.label(crate::i18n::text(ui.ctx(), "surfaces.analysis.analyzing"));
                return;
            };
            let Some(Ok(data)) = result.data[index].as_ref() else {
                ui.label(crate::i18n::text(ui.ctx(), "surfaces.analysis.failed"));
                return;
            };
            let counterpart = result.data[1 - index]
                .as_ref()
                .and_then(|result| result.as_ref().ok());
            if has_counterpart && counterpart.is_none() {
                ui.label(crate::i18n::text(
                    ui.ctx(),
                    if regions[1 - index].is_none() {
                        "surfaces.analysis.counterpart_unavailable"
                    } else if result.data[1 - index].as_ref().is_some_and(Result::is_err) {
                        "surfaces.analysis.counterpart_failed"
                    } else {
                        "surfaces.analysis.counterpart_pending"
                    },
                ));
            }
            let region = regions[index].unwrap();
            let population = crate::i18n::text(ui.ctx(), "analysis.population_hint")
                .replace("{total}", &data.total.to_string());
            let subject_key = match layer {
                ViewLayer::Input => "surfaces.analysis.original",
                ViewLayer::Output if self.retained_kind == Some(crate::RenderKind::Scan) => {
                    "surfaces.analysis.scan"
                }
                ViewLayer::Output => "surfaces.analysis.preview",
                ViewLayer::PaperBack => "surfaces.analysis.paper",
            };
            let scope_key = if self.analysis.selection.is_some() {
                "surfaces.analysis.selection"
            } else {
                "surfaces.analysis.full"
            };
            let scope = format!(
                "{} · {} · {}",
                crate::i18n::text(ui.ctx(), subject_key),
                crate::i18n::text(ui.ctx(), scope_key),
                crate::i18n::text(ui.ctx(), "surfaces.analysis.interpretation")
            );
            ui.label(egui::RichText::new(scope).color(crate::theme::FG_MUTED))
                .on_hover_text(population);
            let sample = target
                .zip(self.analysis.sample)
                .map(|(sample, pixel)| PlotSample {
                    pixel,
                    x: sample.x,
                    y: sample.y,
                    region,
                    inside: sample.x >= region[0]
                        && sample.x < region[2]
                        && sample.y >= region[1]
                        && sample.y < region[3],
                });
            let plot_max_height = (max_height - (ui.cursor().top() - panel_top) - 56.0).max(40.0);
            let plot = analysis_plot::show_plot(
                ui,
                &mut self.analysis.plot,
                &self.analysis.preferences,
                data,
                counterpart,
                sample,
                plot_max_height,
            );
            if plot.inspecting {
                self.analysis.clear_sample();
            }
            if let Some((start, end)) = plot.band {
                self.viewer.analysis.band =
                    Some(image_band(region, geometry.image_size(), start, end));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn closing_pending_analysis_cancels_work_and_selection_from_both_controls() {
        use egui_kittest::kittest::Queryable;

        let directory = tempfile::tempdir().unwrap();
        let mut harness = crate::headless::default_startup(
            &directory.path().join("config"),
            &directory.path().join("work"),
        );
        for close_key in ["surfaces.analysis.close", "surfaces.analysis.title"] {
            let (started_tx, started_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            harness.state_mut().analysis.distributions = Worker::new(
                "pending-analysis-close",
                move |job: DistributionJob, cancel| {
                    started_tx.send(cancel).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    (
                        DistributionResult {
                            identity: job.identity,
                            serial: job.serial,
                            sources: job.sources,
                            data: [None, None],
                        },
                        || {},
                    )
                },
            );
            harness.state_mut().analysis.visible = true;
            harness.step();
            let cancellation = started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            assert!(harness.state().analysis.result.is_none());
            harness.state_mut().viewer.arm_analysis_selection();
            harness.step();
            let label = crate::i18n::text(&harness.ctx, close_key);
            harness.get_by_label(label.as_ref()).simulate_click();
            harness.run_steps(3);
            let cancelled = cancellation.load(Ordering::Relaxed);
            release_tx.send(()).unwrap();
            assert!(!harness.state().analysis.visible);
            assert!(
                cancelled,
                "hidden pending work was not cancelled: {close_key}"
            );
            assert!(harness.state().analysis.identity.is_none());
            assert!(harness.state().analysis.result.is_none());
            assert!(!harness.state().viewer.analysis.enabled);
            assert!(!harness.state().viewer.analysis.armed);
        }
        eframe::App::on_exit(harness.state_mut(), None);
    }

    #[test]
    fn saved_scope_choices_upgrade_without_losing_display_preferences() {
        use crate::analysis_plot::{PlotMode, WaveformView};
        use serde_json::json;

        let directory = tempfile::tempdir().unwrap();
        let mut analysis = Analysis::default();
        for (mode, legacy_rgb, view, expected_mode, expected_view) in [
            (
                "parade",
                false,
                None,
                PlotMode::Waveform,
                WaveformView::RgbParade,
            ),
            (
                "waveform",
                true,
                None,
                PlotMode::Waveform,
                WaveformView::RgbOverlay,
            ),
            (
                "histogram",
                false,
                None,
                PlotMode::Histogram,
                WaveformView::Tone,
            ),
            (
                "waveform",
                true,
                Some("rgb_parade"),
                PlotMode::Waveform,
                WaveformView::RgbParade,
            ),
            (
                "vectorscope",
                true,
                None,
                PlotMode::Vectorscope,
                WaveformView::RgbOverlay,
            ),
        ] {
            let mut saved = json!({
                "visible": true,
                "preferences": {
                    "mode": mode,
                    "waveform_rgb": legacy_rgb,
                    "histogram_rgb": true,
                    "channels": [false, true, false],
                    "height": 320.0,
                    "chroma_zoom": 2
                }
            });
            if let Some(view) = view {
                saved["preferences"]["waveform_view"] = json!(view);
            }
            crate::analysis_preferences::save(directory.path(), &saved).unwrap();
            let loaded = crate::analysis_preferences::load(directory.path())
                .unwrap()
                .unwrap();
            assert!(analysis.restore_state(&loaded));
            assert!(analysis.visible);
            assert_eq!(analysis.preferences.mode, expected_mode);
            assert_eq!(analysis.preferences.waveform_view, expected_view);
            assert_eq!(analysis.preferences.channels, [false, true, false]);
            assert!(analysis.preferences.histogram_rgb);
            assert_eq!(analysis.preferences.height, 320.0);
            assert_eq!(analysis.preferences.chroma_zoom, 2);

            let canonical = analysis.persistent_state();
            assert!(canonical["preferences"].get("waveform_rgb").is_none());
            assert_ne!(canonical["preferences"]["mode"], "parade");
            crate::analysis_preferences::save(directory.path(), &canonical).unwrap();
            let loaded = crate::analysis_preferences::load(directory.path())
                .unwrap()
                .unwrap();
            let expected = analysis.preferences;
            analysis.preferences = Default::default();
            assert!(!analysis.restore_state(&loaded));
            assert_eq!(analysis.preferences, expected);
        }
    }

    #[test]
    fn worker_wakes_only_after_publishing_result() {
        let (wake_tx, wake_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let worker = Worker::new("analysis-wake-test", move |request: u32, _cancel| {
            let wake_tx = wake_tx.clone();
            let release_rx = release_rx.clone();
            (request, move || {
                wake_tx.send(()).unwrap();
                release_rx
                    .lock()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
            })
        });
        worker.submit(42);
        wake_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let published = worker.receive();
        release_tx.send(()).unwrap();
        assert_eq!(published, Some(42));
    }

    #[test]
    fn plot_band_is_limited_to_analyzed_pixel_rows() {
        assert_eq!(
            image_band([20, 30, 60, 70], [100, 200], 0.25, 0.75),
            [0.3, 0.15, 0.5, 0.35]
        );
        assert_eq!(
            image_band([0, 0, 100, 200], [100, 200], 0.0, 1.0),
            [0.0, 0.0, 1.0, 1.0]
        );
    }

    #[test]
    fn superseded_active_work_and_intermediate_queue_do_not_publish() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = Worker::new("analysis-test", move |request: u32, _cancel| {
            if request == 1 {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
            (request, || {})
        });
        worker.submit(1);
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.submit(2);
        worker.submit(3);
        release_tx.send(()).unwrap();
        assert_eq!(
            worker.results.recv_timeout(Duration::from_secs(2)).unwrap(),
            3
        );
        assert!(worker.receive().is_none());
    }
}
