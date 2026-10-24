//! One worker with separate source, presentation, and explicit-command ownership.
use crate::*;
use parking_lot::{Condvar, Mutex};
use std::sync::atomic::Ordering;

pub(crate) enum InputSource {
    File {
        path: PathBuf,
        raw: serde_json::Value,
    },
    Existing(crate::render::InputImage),
}
pub(crate) struct InputSpec {
    pub(crate) source: InputSource,
    pub(crate) rotation: i32,
    pub(crate) space: String,
    pub(crate) decode: bool,
    pub(crate) max_size: Option<u32>,
}
pub(crate) struct InputView {
    pub(crate) oriented: crate::render::InputImage,
    pub(crate) raster: display::DisplayRaster,
}
pub(crate) struct PreparedInput {
    pub(crate) source: crate::render::InputImage,
    pub(crate) view: Option<InputView>,
    pub(crate) metadata: Option<ImageMetadata>,
    pub(crate) lens_info: Option<String>,
    pub(crate) lens_model_identity: Option<String>,
    pub(crate) digest: Option<String>,
    pub(crate) elapsed: Duration,
}

pub(crate) fn prepare(spec: InputSpec, backend: &dyn ComputeBackend) -> Result<PreparedInput> {
    let start = Instant::now();
    let mut metadata = None;
    let mut lens_info = None;
    let mut lens_model_identity = None;
    let mut digest = None;
    let space = spec.space;
    let decode = spec.decode;
    let source = match spec.source {
        InputSource::Existing(source) => source,
        InputSource::File { path, raw } => {
            let before = App::accepted_source_digest(&path)?;
            let source = if image_io::is_raw(&path) {
                let options = spektrafilm_raw::RawOptions {
                    white_balance: match raw["white_balance"].as_str().unwrap_or("as_shot") {
                        "daylight" => spektrafilm_raw::WhiteBalance::Daylight,
                        "tungsten" => spektrafilm_raw::WhiteBalance::Tungsten,
                        "custom" => spektrafilm_raw::WhiteBalance::Custom,
                        _ => spektrafilm_raw::WhiteBalance::AsShot,
                    },
                    temperature: raw["temperature"].as_f64(),
                    tint: raw["tint"].as_f64(),
                    lens_correction: raw["lens_correction"].as_bool().unwrap_or(false),
                };
                metadata = image_io::read_metadata(&path);
                if backend.is_gpu() {
                    let loaded =
                        spektrafilm_raw::load_retained_with_backend(&path, &options, backend)
                            .map_err(anyhow::Error::msg)?;
                    lens_info = Some(loaded.lens_info);
                    lens_model_identity = loaded.lens_model_identity;
                    crate::render::InputImage::Device(loaded.image)
                } else {
                    let loaded =
                        spektrafilm_raw::load(&path, &options).map_err(anyhow::Error::msg)?;
                    lens_info = Some(loaded.lens_info);
                    lens_model_identity = loaded.lens_model_identity;
                    crate::render::InputImage::Host(Arc::new(loaded.image))
                }
            } else {
                let loaded = image_io::load(&path)?;
                metadata = loaded.metadata;
                crate::render::InputImage::Host(Arc::new(loaded.image))
            };
            anyhow::ensure!(
                before == App::accepted_source_digest(&path)?,
                crate::app::Message::new("photo.error.source_changed")
            );
            digest = Some(before);
            source
        }
    };
    let view = if let Some(max_size) = spec.max_size {
        let oriented = source
            .rotated(spec.rotation, backend, &space, decode)
            .map_err(anyhow::Error::msg)?;
        let raster = match &oriented {
            crate::render::InputImage::Host(image) => display::input_display_raster_with(
                backend,
                image,
                &space,
                decode,
                max_size as usize,
            ),
            crate::render::InputImage::Device(image) => display::output_display_raster_retained(
                backend,
                image,
                image.space().name,
                image.domain() == spektrafilm_backend::SampleDomain::Encoded,
                true,
                None,
                max_size as usize,
            )
            .map(|(raster, _)| raster),
        }
        .map_err(anyhow::Error::msg)?;
        Some(InputView { oriented, raster })
    } else {
        None
    };
    Ok(PreparedInput {
        source,
        view,
        metadata,
        lens_info,
        lens_model_identity,
        digest,
        elapsed: start.elapsed(),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Domain {
    Source = 0,
    View = 1,
    Command = 2,
}
type Validation = Box<dyn FnOnce(&dyn ComputeBackend, &[PreparedInput]) -> Result<()> + Send>;
struct Request {
    generation: u64,
    domain: Domain,
    inputs: Vec<InputSpec>,
    validate: Option<Validation>,
    backend: Arc<dyn ComputeBackend>,
    context: Option<egui::Context>,
}
type Response = (u64, Result<Vec<PreparedInput>>);
struct WorkerState {
    generations: [u64; 3],
    requests: [Option<Request>; 3],
    responses: [Option<Response>; 3],
    active: Option<(Domain, Arc<AtomicBool>)>,
    stopping: bool,
}
pub(crate) struct InputWorker {
    state: Arc<(Mutex<WorkerState>, Condvar)>,
}
impl InputWorker {
    pub(crate) fn new() -> Self {
        let state = Arc::new((
            Mutex::new(WorkerState {
                generations: [0; 3],
                requests: [None, None, None],
                responses: [None, None, None],
                active: None,
                stopping: false,
            }),
            Condvar::new(),
        ));
        let shared = state.clone();
        std::thread::Builder::new()
            .name("input-preparation".into())
            .spawn(move || {
                loop {
                    let (request, cancellation) = {
                        let (lock, wake) = &*shared;
                        let mut state = lock.lock();
                        while state.requests.iter().all(Option::is_none) && !state.stopping {
                            wake.wait(&mut state);
                        }
                        if state.stopping {
                            break;
                        }
                        let request = state.requests[Domain::Command as usize]
                            .take()
                            .or_else(|| state.requests[Domain::Source as usize].take())
                            .or_else(|| state.requests[Domain::View as usize].take())
                            .unwrap();
                        let cancellation = Arc::new(AtomicBool::new(false));
                        state.active = Some((request.domain, cancellation.clone()));
                        (request, cancellation)
                    };
                    let bound = request
                        .backend
                        .with_cancellation(Some(cancellation.clone()));
                    let backend = bound.as_deref().unwrap_or(request.backend.as_ref());
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut prepared = Vec::with_capacity(request.inputs.len());
                        for input in request.inputs {
                            anyhow::ensure!(
                                !cancellation.load(Ordering::Relaxed),
                                "operation cancelled"
                            );
                            prepared.push(prepare(input, backend)?);
                        }
                        if let Some(validate) = request.validate {
                            validate(backend, &prepared)?;
                        }
                        Ok(prepared)
                    }))
                    .unwrap_or_else(|panic| {
                        Err(crate::render_worker::panic_message(&panic).into())
                    });
                    let mut state = shared.0.lock();
                    state.active = None;
                    let index = request.domain as usize;
                    if !state.stopping
                        && state.generations[index] == request.generation
                        && !cancellation.load(Ordering::Relaxed)
                    {
                        state.responses[index] = Some((request.generation, result));
                        if let Some(ctx) = request.context {
                            ctx.request_repaint();
                        }
                    }
                }
            })
            .expect("input preparation worker");
        Self { state }
    }
    fn submit(
        &self,
        domain: Domain,
        inputs: Vec<InputSpec>,
        validate: Option<Validation>,
        backend: Arc<dyn ComputeBackend>,
        context: Option<egui::Context>,
    ) -> u64 {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock();
        let index = domain as usize;
        state.generations[index] = state.generations[index].wrapping_add(1);
        if let Some((active, cancel)) = &state.active
            && *active == domain
        {
            cancel.store(true, Ordering::Relaxed);
        }
        let generation = state.generations[index];
        state.requests[index] = Some(Request {
            generation,
            domain,
            inputs,
            validate,
            backend,
            context,
        });
        state.responses[index] = None;
        wake.notify_one();
        generation
    }
    fn take_response(&self, domain: Domain) -> Option<Response> {
        self.state.0.lock().responses[domain as usize].take()
    }
    fn cancel(&self, domain: Domain) {
        let mut state = self.state.0.lock();
        let index = domain as usize;
        state.generations[index] = state.generations[index].wrapping_add(1);
        if let Some((active, cancel)) = &state.active
            && *active == domain
        {
            cancel.store(true, Ordering::Relaxed);
        }
        state.requests[index] = None;
        state.responses[index] = None;
    }
}
impl Drop for InputWorker {
    fn drop(&mut self) {
        for domain in [Domain::Source, Domain::View, Domain::Command] {
            self.cancel(domain);
        }
        let mut state = self.state.0.lock();
        state.stopping = true;
        self.state.1.notify_one();
        // A native compile cannot be interrupted. Captured resources stay with
        // the worker until it returns; the UI never joins that driver wait.
    }
}

pub(crate) enum InputFailure {
    Load,
    Viewer,
    Photograph,
}
type Commit = Box<dyn FnOnce(&mut App, Vec<PreparedInput>) -> Result<()>>;
pub(crate) struct InputJob {
    generation: u64,
    failure: InputFailure,
    commit: Commit,
}
impl App {
    pub(crate) fn queue_input(
        &mut self,
        spec: InputSpec,
        failure: InputFailure,
        commit: impl FnOnce(&mut Self, PreparedInput) -> Result<()> + 'static,
    ) {
        self.queue_input_batch(vec![spec], failure, move |app, mut prepared| {
            commit(app, prepared.remove(0))
        });
    }
    pub(crate) fn queue_input_batch(
        &mut self,
        inputs: Vec<InputSpec>,
        failure: InputFailure,
        commit: impl FnOnce(&mut Self, Vec<PreparedInput>) -> Result<()> + 'static,
    ) {
        let generation = self.input_worker.submit(
            Domain::Source,
            inputs,
            None,
            self.backend.clone(),
            self.photograph.context.clone(),
        );
        self.input_job = Some(InputJob {
            generation,
            failure,
            commit: Box::new(commit),
        });
    }
    pub(crate) fn queue_input_view(
        &mut self,
        spec: InputSpec,
        commit: impl FnOnce(&mut Self, PreparedInput) -> Result<()> + 'static,
    ) {
        let generation = self.input_worker.submit(
            Domain::View,
            vec![spec],
            None,
            self.backend.clone(),
            self.photograph.context.clone(),
        );
        self.input_view_job = Some(InputJob {
            generation,
            failure: InputFailure::Viewer,
            commit: Box::new(move |app, mut prepared| commit(app, prepared.remove(0))),
        });
    }
    pub(crate) fn queue_input_command(
        &mut self,
        inputs: Vec<InputSpec>,
        validate: Option<Validation>,
        commit: impl FnOnce(&mut Self, Vec<PreparedInput>) -> Result<()> + 'static,
    ) -> Result<()> {
        anyhow::ensure!(
            !self.input_transition_pending(),
            crate::app::Message::new("surfaces.status.preparing_input")
        );
        let epoch = self.input_epoch;
        let revision = self.parameter_revision;
        let generation = self.input_worker.submit(
            Domain::Command,
            inputs,
            validate,
            self.backend.clone(),
            self.photograph.context.clone(),
        );
        self.input_command_job = Some(InputJob {
            generation,
            failure: InputFailure::Photograph,
            commit: Box::new(move |app, prepared| {
                anyhow::ensure!(
                    app.input_epoch == epoch && app.parameter_revision == revision,
                    crate::app::Message::new("operations.operation_cancelled")
                );
                commit(app, prepared)
            }),
        });
        Ok(())
    }
    pub(crate) fn cancel_input_view(&mut self) {
        self.input_worker.cancel(Domain::View);
        self.input_view_job = None;
    }
    pub(crate) fn input_transition_pending(&self) -> bool {
        self.input_job.is_some() || self.input_command_job.is_some()
    }
    pub(crate) fn input_pending(&self) -> bool {
        self.input_job.is_some()
            || self.input_view_job.is_some()
            || self.input_command_job.is_some()
    }
    pub(crate) fn poll_input_job(&mut self, ctx: &egui::Context) {
        for domain in [Domain::Source, Domain::Command, Domain::View] {
            let Some((generation, result)) = self.input_worker.take_response(domain) else {
                continue;
            };
            let pending = match domain {
                Domain::Source => &mut self.input_job,
                Domain::View => &mut self.input_view_job,
                Domain::Command => &mut self.input_command_job,
            };
            if pending
                .as_ref()
                .is_none_or(|job| job.generation != generation)
            {
                continue;
            }
            let job = pending.take().unwrap();
            let previous_input = self.input_epoch;
            let result = result.and_then(|prepared| (job.commit)(self, prepared));
            if self.input_epoch != previous_input && result.is_ok() {
                self.cancel_input_view();
            }
            if let Err(error) = result {
                let detail = crate::app::Message::error(&error);
                self.status = match job.failure {
                    InputFailure::Load => crate::app::Message::new("surfaces.status.load_error")
                        .message_arg("error", detail),
                    InputFailure::Viewer => {
                        crate::app::Message::new("surfaces.status.viewer_error")
                            .message_arg("error", detail)
                    }
                    InputFailure::Photograph => {
                        crate::app::Message::new("photo.error.action").message_arg("detail", detail)
                    }
                };
            }
        }
        if self.input_pending() {
            ctx.request_repaint_after(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn response(worker: &InputWorker, domain: Domain) -> Response {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(response) = worker.take_response(domain) {
                return response;
            }
            assert!(Instant::now() < deadline, "input worker response timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn replaced_source_cannot_publish_and_view_refresh_remains_independent() {
        let worker = InputWorker::new();
        let backend: Arc<dyn ComputeBackend> =
            Arc::new(spektrafilm_backend::cpu_backend::CpuBackend);
        let (started, ready) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let ui_thread = std::thread::current().id();
        worker.submit(
            Domain::Source,
            vec![],
            Some(Box::new(move |_, _| {
                started.send(std::thread::current().id()).unwrap();
                blocked.recv().unwrap();
                anyhow::bail!("obsolete source")
            })),
            backend.clone(),
            None,
        );
        assert_ne!(
            ready.recv_timeout(Duration::from_secs(10)).unwrap(),
            ui_thread
        );
        worker.submit(
            Domain::View,
            vec![],
            Some(Box::new(|_, _| anyhow::bail!("view"))),
            backend.clone(),
            None,
        );
        let generation = worker.submit(
            Domain::Source,
            vec![],
            Some(Box::new(|_, _| anyhow::bail!("current source"))),
            backend,
            None,
        );
        release.send(()).unwrap();
        let (accepted, result) = response(&worker, Domain::Source);
        assert_eq!(accepted, generation);
        assert_eq!(result.err().unwrap().to_string(), "current source");
        assert_eq!(
            response(&worker, Domain::View).1.err().unwrap().to_string(),
            "view"
        );
    }

    #[test]
    fn shutdown_does_not_join_a_blocked_native_operation() {
        let worker = InputWorker::new();
        let (started, ready) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        worker.submit(
            Domain::Source,
            vec![],
            Some(Box::new(move |_, _| {
                started.send(()).unwrap();
                blocked.recv().unwrap();
                Ok(())
            })),
            Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
            None,
        );
        ready.recv_timeout(Duration::from_secs(10)).unwrap();
        let (finished, dropped) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(worker);
            finished.send(()).unwrap();
        });
        let result = dropped.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        dropper.join().unwrap();
        assert!(
            result.is_ok(),
            "shutdown waited for the blocked driver call"
        );
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn failed_source_and_unrelated_command_preserve_pending_input_view() {
        for failed_source in [true, false] {
            let directory = tempfile::tempdir().unwrap();
            let mut app = App::new_with_browser_path(
                Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
                None,
                None,
                None,
                directory.path().join("browser.json"),
                crate::headless::data_dir(),
            );
            let source = crate::render::InputImage::Host(Arc::new(ImageBuf::from_data(
                2,
                1,
                vec![0.1, 0.2, 0.3, 0.6, 0.7, 0.8],
            )));
            app.image = Some(source.clone());
            let (started, ready) = mpsc::channel();
            let (release, blocked) = mpsc::channel();
            let spec = InputSpec {
                source: InputSource::Existing(source.clone()),
                rotation: 1,
                space: "sRGB".into(),
                decode: true,
                max_size: Some(128),
            };
            let generation = app.input_worker.submit(
                Domain::View,
                vec![spec],
                Some(Box::new(move |_, _| {
                    started.send(()).unwrap();
                    blocked.recv().unwrap();
                    Ok(())
                })),
                app.backend.clone(),
                None,
            );
            app.input_view_job = Some(InputJob {
                generation,
                failure: InputFailure::Viewer,
                commit: Box::new(|app, mut prepared| {
                    let view = prepared.remove(0).view.unwrap();
                    app.publish_input_viewer(view.oriented, view.raster);
                    Ok(())
                }),
            });
            ready.recv_timeout(Duration::from_secs(10)).unwrap();
            if failed_source {
                app.load_image_from_path(&directory.path().join("missing.png"));
            } else {
                app.queue_input_command(vec![], None, |_, _| Ok(()))
                    .unwrap();
            }
            release.send(()).unwrap();
            // Poll only after both responses exist. Commands are deliberately
            // consumed before View, including a view that finished earlier.
            let domain = if failed_source {
                Domain::Source
            } else {
                Domain::Command
            };
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let ready = {
                    let state = app.input_worker.state.0.lock();
                    state.responses[Domain::View as usize].is_some()
                        && state.responses[domain as usize].is_some()
                };
                if ready {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "both input responses were not published"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            crate::headless::await_app_input(&mut app, &egui::Context::default());
            let oriented = app
                .oriented_input
                .as_ref()
                .expect("accepted source view was not published");
            assert_eq!((oriented.width(), oriented.height()), (1, 2));
            assert!(source.same_source(app.image.as_ref().unwrap()));
            assert!(!app.input_pending());
        }
    }
}
