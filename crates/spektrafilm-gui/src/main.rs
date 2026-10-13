// Interactive spektrafilm preview and export GUI.
//
// The egui shell follows the pinned Python 0.3.4 workflow: a tabbed sidebar
// groups the main, film, print, advanced and configuration controls, while
// the central viewer owns the input/output/paper-back presentation.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;
use spektrafilm_core::image_io::{self, BitDepth, ImageMetadata, LoadedImage, SaveOptions};
use spektrafilm_core::params::{RuntimeParams, ScannerAutoSetup};
use spektrafilm_core::presets::{Section, workflow_default};
use spektrafilm_core::profile;
use spektrafilm_core::telemetry::{IssueBoundary, IssueCategory};
use spektrafilm_core::telemetry::{Operation, OperationKind, Outcome, SourceRender};
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_gpu::telemetry::{CollectionMode, ObservationKind, Purpose};
use spektrafilm_math::image::ImageBuf;
mod calibration;
mod controls;
mod diagnostics;
mod display;
mod export;
mod export_job;
#[cfg(test)]
mod headless;
mod numeric;
mod presets;
mod profiles;
mod render;
mod render_worker;
mod routes;
mod state;
mod ui;
use calibration::CalibrationJob;
#[cfg(test)]
use export::ExportBackend;
use export::{ExportDialog, ExportOptions};
use export_job::ExportJob;
use profiles::{ProfileEntry, dev_time_combo, profile_combo, profile_dev_times, scan_profiles};
#[cfg(test)]
use render::RuntimeCache;
use render::{RenderSnapshot, SharedScannerSetup, build_runtime_observed};
use render_worker::{RenderJob, RenderKind, RenderRequest, RenderWorker, observed_result};

const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    if let Err(err) = embedded_bundle::setup() {
        eprintln!("[spektrafilm] embedded bundle setup failed: {err:#}");
    }
    let mut args = std::env::args().skip(1);
    let mut initial_image = None;
    let mut initial_state = std::env::var_os("SPEKTRAFILM_GUI_STATE").map(PathBuf::from);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--state" => {
                if let Some(path) = args.next() {
                    initial_state = Some(PathBuf::from(path));
                } else {
                    eprintln!("--state requires a GUI state JSON path");
                    std::process::exit(2);
                }
            }
            "--help" | "-h" => {
                println!("Usage: spektrafilm-gui [IMAGE] [--state GUI_STATE.json]");
                println!(
                    "SPEKTRAFILM_GUI_DIAGNOSTICS_DIR publishes each completed operation's report \
                     as operation-<id>.json in that existing directory."
                );
                return Ok(());
            }
            _ if arg.starts_with('-') => {
                eprintln!("Unknown GUI option: {arg}");
                std::process::exit(2);
            }
            _ => {
                initial_image = Some(PathBuf::from(arg));
            }
        }
    }
    let backend = gui_backend();
    let diagnostics_dir = std::env::var_os("SPEKTRAFILM_GUI_DIAGNOSTICS_DIR").map(PathBuf::from);
    if let Some(directory) = diagnostics_dir.as_deref()
        && !directory.is_dir()
    {
        eprintln!(
            "SPEKTRAFILM_GUI_DIAGNOSTICS_DIR must name an existing directory: {}",
            directory.display()
        );
        std::process::exit(2);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1460.0, 980.0]),
        renderer: gui_renderer(),
        // egui's default device descriptor hardcodes
        // `max_texture_dimension_2d: 8192`, so uploading the preview of an
        // image wider or taller than 8192 px (a single texture) panics.
        // Raise the cap to the adapter's real limit (16384 on Apple
        // Silicon Metal, higher on discrete GPUs), floored at egui's 8192
        // so we never regress. This is eframe's OWN render device — the
        // compute pipeline's wgpu device (WgpuBackend::new) is separate
        // and already lifts its storage-buffer limits.
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetupCreateNew {
                device_descriptor: Arc::new(|adapter: &eframe::wgpu::Adapter| {
                    let base_limits = if adapter.get_info().backend == eframe::wgpu::Backend::Gl {
                        eframe::wgpu::Limits::downlevel_webgl2_defaults()
                    } else {
                        eframe::wgpu::Limits::default()
                    };
                    eframe::wgpu::DeviceDescriptor {
                        label: Some("spektrafilm egui wgpu device"),
                        required_features: eframe::wgpu::Features::default(),
                        required_limits: eframe::wgpu::Limits {
                            max_texture_dimension_2d: adapter
                                .limits()
                                .max_texture_dimension_2d
                                .max(8192),
                            ..base_limits
                        },
                        memory_hints: eframe::wgpu::MemoryHints::default(),
                    }
                }),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "spektrafilm",
        options,
        Box::new(|_cc| {
            Ok(Box::new(App::new(
                backend,
                initial_image,
                initial_state,
                diagnostics_dir,
            )))
        }),
    )
}

#[cfg(embed_bundle)]
mod embedded_bundle {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};

    include!(env!("SPEKTRAFILM_EMBED_MANIFEST"));

    pub fn setup() -> Result<()> {
        let root = cache_root().join(BUNDLE_ID);
        let data_dir = root.join("data");
        let f64_cli = root.join(f64_cli_name());
        let marker = root.join(".ready");

        if !marker.is_file() || !data_dir.join("profiles").is_dir() || !f64_cli.is_file() {
            std::fs::create_dir_all(&data_dir)
                .with_context(|| format!("creating {}", data_dir.display()))?;
            for (rel, bytes) in DATA_FILES {
                write_file(&data_dir.join(rel), bytes)?;
            }
            write_file(&f64_cli, F64_EXE)?;
            write_file(&marker, BUNDLE_ID.as_bytes())?;
        }

        // This runs before any worker threads are spawned.
        unsafe {
            std::env::set_var("SPEKTRAFILM_DATA_DIR", &data_dir);
            let adjacent = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join(f64_cli_name())))
                .filter(|path| path.is_file());
            if std::env::var_os("SPEKTRAFILM_F64_CLI").is_none() {
                std::env::set_var("SPEKTRAFILM_F64_CLI", adjacent.as_ref().unwrap_or(&f64_cli));
            }
        }
        Ok(())
    }

    fn cache_root() -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("spektrafilm")
            .join("embedded")
    }

    fn f64_cli_name() -> &'static str {
        if cfg!(windows) {
            "spektrafilm-f64.exe"
        } else {
            "spektrafilm-f64"
        }
    }

    fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut file =
            std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

#[cfg(not(embed_bundle))]
mod embedded_bundle {
    use anyhow::Result;

    pub fn setup() -> Result<()> {
        Ok(())
    }
}

fn gui_renderer() -> eframe::Renderer {
    match std::env::var("SPEKTRAFILM_GUI_RENDERER") {
        Ok(v) if v.eq_ignore_ascii_case("wgpu") => eframe::Renderer::Wgpu,
        Ok(v) if v.eq_ignore_ascii_case("glow") => eframe::Renderer::Glow,
        _ => {
            if cfg!(target_os = "macos") {
                // Metal-backed CAMetalLayer lets us tag the colorspace as sRGB.
                eframe::Renderer::Wgpu
            } else {
                // Keep the UI compositor away from wgpu/D3D12 by default on
                // Windows/Linux; the WGPU compute backend is selected separately.
                eframe::Renderer::Glow
            }
        }
    }
}

/// GUI Preview and Scan prefer WGPU even in a reference-precision build.
/// An explicit CPU override and unavailable adapters use the native CPU backend.
fn gui_backend() -> Arc<dyn ComputeBackend> {
    match std::env::var("SPEKTRAFILM_BACKEND").ok().as_deref() {
        None => spektrafilm_gpu::wgpu_backend::WgpuBackend::new()
            .map(|backend| Arc::new(backend) as Arc<dyn ComputeBackend>)
            .unwrap_or_else(|| Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend)),
        Some(_) => Arc::from(spektrafilm_gpu::select_backend()),
    }
}

/// Effective backend label for Scan and Export reports. An active Dye field
/// resolves on the CPU and never enters the resident chain, so the selected
/// backend name alone would misreport the executed path.
fn effective_backend_name(selected: &str, params: &RuntimeParams) -> String {
    if spektrafilm_core::stages::grain_v3::active(params) {
        format!("{selected} · grain Dye on CPU")
    } else if spektrafilm_core::stages::grain_fotufilm::active(params) {
        format!("{selected} · grain Fotufilm on CPU")
    } else {
        selected.to_owned()
    }
}

struct App {
    backend: Arc<dyn ComputeBackend>,
    data_dir: PathBuf,
    /// Profiles where `info.support == "film"`.
    films: Vec<ProfileEntry>,
    /// Print stocks usable as the print stage (`Profile::is_print_stock`:
    /// paper/printing or film/printing).
    papers: Vec<ProfileEntry>,
    presets: presets::Browser,
    browser_state_path: PathBuf,
    observed_browser_state: presets::BrowserState,
    browser_state_changed_at: Option<Instant>,
    film_name: String,
    film_medium: controls::ScanMedium,
    print_name: String,
    /// Development-time family (minutes) of the selected film / paper —
    /// non-trivial only for B&W stocks. Drives the dev-time pickers.
    film_dev_times: Vec<f64>,
    print_dev_times: Vec<f64>,
    params: RuntimeParams,
    gui_state: state::GuiState,
    force_preview: bool,
    full_scan_requested: bool,
    /// Original correction switches captured while Scan-for-print is active.
    /// This transient workflow state is deliberately excluded from persistence.
    scan_for_print_snapshot: Option<(bool, bool, bool)>,
    image_path: Option<PathBuf>,
    image: Option<Arc<ImageBuf>>,
    raw_lens_info: Option<String>,
    /// Quarter turns applied to the loaded source, used for the rotation status.
    input_rotation: i32,
    source_metadata: Option<ImageMetadata>,
    export_options: ExportOptions,
    export_dialog: ExportDialog,
    input_epoch: u64,
    scanner_setup: Option<((u64, u64), SharedScannerSetup)>,
    /// Latest accepted floating output, shared with background viewing workers.
    output_image: Option<Arc<ImageBuf>>,
    output_metadata: Option<ImageMetadata>,
    source_render: Option<SourceRender>,
    diagnostics: diagnostics::Diagnostics,
    viewer: display::Viewer,
    gui_tab: ui::GuiTab,
    output_color_space: String,
    output_cctf_encoding: bool,
    last_render_ms: f32,
    last_pipeline_build_ms: f32,
    last_input_clone_ms: f32,
    last_scale_ms: f32,
    last_preview_ms: f32,
    last_worker_total_ms: f32,
    status: String,
    /// Controls changed since the latest submitted render snapshot.
    dirty: bool,
    dirty_since: Option<Instant>,
    /// macOS-only: tag the wgpu CAMetalLayer's `colorspace` as sRGB on
    /// the first `update()` tick (it's not yet wired up at
    /// `App::new` time). Set to `true` once the call succeeds so we
    /// don't retry every frame.
    #[cfg(target_os = "macos")]
    metal_colorspace_tagged: bool,
    /// Latest submitted render, including a request queued behind active work.
    render_job: Option<RenderJob>,
    render_worker: RenderWorker,
    /// In-flight export job. `Some` while the immutable snapshot is rendering;
    /// the `update()` loop polls the receiver each frame and
    /// surfaces success/failure in `status` when the worker thread
    /// completes. Joined eagerly to release the thread.
    export_job: Option<ExportJob>,
    /// In-flight Convert controller action. The epoch drops results made stale
    /// by later parameter edits or a newer action.
    calibration_job: Option<CalibrationJob>,
    calibration_epoch: u64,
    parameter_revision: u64,
}

fn scan_medium(film: &profile::Profile) -> controls::ScanMedium {
    if film.is_film() && film.is_negative() {
        controls::ScanMedium::Negative
    } else if film.is_film() && film.is_positive() {
        controls::ScanMedium::Slide
    } else {
        controls::ScanMedium::None
    }
}

fn normalize_scan_output(params: &mut RuntimeParams, film_medium: controls::ScanMedium) {
    controls::normalize_scanner_selections(params, film_medium);
}

impl App {
    fn new(
        backend: Arc<dyn ComputeBackend>,
        initial_image: Option<PathBuf>,
        initial_state: Option<PathBuf>,
        diagnostics_dir: Option<PathBuf>,
    ) -> Self {
        Self::new_with_browser_path(
            backend,
            initial_image,
            initial_state,
            diagnostics_dir,
            state::config_dir().join("preset_browser_state.json"),
            pick_data_dir(),
        )
    }

    fn new_with_browser_path(
        backend: Arc<dyn ComputeBackend>,
        initial_image: Option<PathBuf>,
        initial_state: Option<PathBuf>,
        diagnostics_dir: Option<PathBuf>,
        browser_state_path: PathBuf,
        data_dir: PathBuf,
    ) -> Self {
        let explicit_state = initial_state.is_some();
        let (films, papers) = scan_profiles(&data_dir);
        let mut startup_error = None;
        let gui_state = match initial_state {
            Some(path) => state::GuiState::load(&path),
            None => state::startup(browser_state_path.parent().unwrap_or(Path::new("."))),
        }
        .and_then(|state| {
            // A startup state the runtime cannot accept (a removed workflow
            // route, an unsupported combination) must fall back to the factory
            // state with a visible reason instead of failing at construction.
            let params = state.runtime_params()?;
            let film = profile::load_profile_by_name(&data_dir, state.film())?;
            spektrafilm_core::validate_scan_output(&params, &film).map_err(anyhow::Error::msg)?;
            Ok(state)
        })
        .unwrap_or_else(|error| {
            startup_error = Some(format!("Startup state error: {error:#}"));
            state::GuiState::factory()
        });
        let film_name = gui_state.film().to_owned();
        let print_name = gui_state.paper().to_owned();
        let film_profile = profile::load_profile_by_name(&data_dir, &film_name).ok();
        let film_medium = film_profile
            .as_ref()
            .map(scan_medium)
            .unwrap_or(controls::ScanMedium::None);
        let mut params = gui_state
            .runtime_params()
            .expect("validated GUI factory state");
        normalize_scan_output(&mut params, film_medium);
        let film_dev_times = film_profile
            .as_ref()
            .map(|f| f.data.development_time.clone())
            .unwrap_or_default();
        let print_dev_times = profile_dev_times(&data_dir, &print_name);
        let export_options = ExportOptions::from_state(&gui_state.sections);
        let mut presets = presets::Browser::new(
            &gui_state.sections,
            spektrafilm_core::presets::PresetLibrary::new(
                browser_state_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("presets"),
            ),
        );
        match presets::BrowserState::restore(
            &gui_state.sections,
            explicit_state,
            &browser_state_path,
        ) {
            Ok(browser_state) => presets.state = browser_state,
            Err(error) => {
                let message = format!("Browser state load error: {error:#}");
                startup_error = Some(match startup_error {
                    Some(previous) => format!("{previous}; {message}"),
                    None => message,
                });
            }
        }

        let mut app = Self {
            backend,
            data_dir,
            films,
            papers,
            observed_browser_state: presets.state.clone(),
            browser_state_changed_at: explicit_state.then(Instant::now),
            film_medium,
            browser_state_path,
            presets,
            film_name,
            print_name,
            film_dev_times,
            print_dev_times,
            params,
            gui_state,
            force_preview: false,
            full_scan_requested: false,
            scan_for_print_snapshot: None,
            image_path: None,
            input_rotation: 0,
            image: None,
            raw_lens_info: None,
            source_metadata: None,
            export_options,
            export_dialog: ExportDialog::default(),
            input_epoch: 0,
            scanner_setup: None,
            output_image: None,
            output_metadata: None,
            source_render: None,
            diagnostics: diagnostics::Diagnostics::default(),
            viewer: display::Viewer::new(),
            gui_tab: ui::GuiTab::default(),
            output_color_space: "sRGB".into(),
            output_cctf_encoding: true,
            last_render_ms: 0.0,
            last_pipeline_build_ms: 0.0,
            last_input_clone_ms: 0.0,
            last_scale_ms: 0.0,
            last_preview_ms: 0.0,
            last_worker_total_ms: 0.0,
            dirty_since: None,
            render_job: None,
            render_worker: RenderWorker::new(),
            export_job: None,
            status: startup_error.unwrap_or_else(|| String::from("Load an image to start.")),
            dirty: false,
            #[cfg(target_os = "macos")]
            metal_colorspace_tagged: false,
            calibration_job: None,
            calibration_epoch: 0,
            parameter_revision: 0,
        };
        app.viewer.settings =
            display::DisplaySettings::from_json(&app.gui_state.sections["display"]);
        if let Some(directory) = diagnostics_dir {
            // Requesting reports selects summary collection for the session, as the
            // CLI's report request does; the panel can still change the mode.
            app.diagnostics.mode = CollectionMode::Summary;
            app.diagnostics.publish_completed_to(directory);
        }
        if let Some(p) = initial_image {
            app.load_image_from_path(&p);
        }
        app.viewer
            .restore_state(&app.gui_state.sections["rust"]["viewer"]);
        app
    }

    fn current_state(&self) -> Result<state::GuiState> {
        let mut extras = self.gui_state.sections.clone();
        let display = self.viewer.settings.to_json();
        for key in [
            "use_display_transform",
            "gray_18_canvas",
            "white_padding",
            "output_interpolation",
        ] {
            extras["display"][key] = display[key].clone();
        }
        if !extras["rust"].is_object() {
            extras["rust"] = serde_json::json!({"version":1});
        }
        extras["rust"]["viewer"] = self.viewer.persistent_state();
        self.export_options.write_state(&mut extras);
        extras["rust"]["presets"] = serde_json::to_value(&self.presets.state)?;
        state::GuiState::from_runtime(&self.params, &self.film_name, &self.print_name, &extras)
    }

    fn mark_parameters_changed(&mut self) {
        self.parameter_revision = self.parameter_revision.wrapping_add(1);
        self.dirty = true;
    }

    fn mark_parameters_changed_invalidating_calibration(&mut self) {
        self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
        self.mark_parameters_changed();
    }

    fn apply_state(&mut self, state: state::GuiState) -> Result<()> {
        let mut params = state.runtime_params()?;
        let film = profile::load_profile_by_name(&self.data_dir, state.film())?;
        profile::load_profile_by_name(&self.data_dir, state.paper())?;
        spektrafilm_core::validate_scan_output(&params, &film).map_err(anyhow::Error::msg)?;
        let film_medium = scan_medium(&film);
        normalize_scan_output(&mut params, film_medium);
        self.film_name = state.film().to_owned();
        self.film_medium = film_medium;
        self.print_name = state.paper().to_owned();
        self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
        self.parameter_revision = self.parameter_revision.wrapping_add(1);
        self.film_dev_times = profile_dev_times(&self.data_dir, &self.film_name);
        self.print_dev_times = profile_dev_times(&self.data_dir, &self.print_name);
        self.viewer.settings = display::DisplaySettings::from_json(&state.sections["display"]);
        self.params = params;
        self.export_options = ExportOptions::from_state(&state.sections);
        self.export_dialog = ExportDialog::default();
        self.presets = presets::Browser::new(
            &state.sections,
            spektrafilm_core::presets::PresetLibrary::new(
                self.browser_state_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("presets"),
            ),
        );
        self.browser_state_changed_at = Some(Instant::now());
        self.gui_state = state;
        self.scan_for_print_snapshot = None;
        self.raw_lens_info = None;
        self.dirty = true;
        self.force_preview = true;
        if let Some(path) = self.image_path.clone() {
            self.load_image_from_path(&path);
        }
        self.viewer
            .restore_state(&self.gui_state.sections["rust"]["viewer"]);
        Ok(())
    }

    fn persist_browser_state(&mut self, closing: bool) {
        if self.presets.state != self.observed_browser_state {
            self.observed_browser_state = self.presets.state.clone();
            self.browser_state_changed_at = Some(Instant::now());
        }
        if self
            .browser_state_changed_at
            .is_some_and(|changed| closing || changed.elapsed() >= Duration::from_millis(300))
        {
            self.browser_state_changed_at = None;
            if let Err(error) = self.presets.state.save(&self.browser_state_path) {
                self.status = format!("Browser state save error: {error:#}");
                if closing {
                    eprintln!("{}", self.status);
                }
            }
        }
    }

    fn render_snapshot(&mut self, params: RuntimeParams) -> RenderSnapshot {
        let key = (self.input_epoch, self.parameter_revision);
        if self
            .scanner_setup
            .as_ref()
            .is_none_or(|(cached, _)| *cached != key)
        {
            self.scanner_setup = Some((key, Arc::new(OnceLock::new())));
        }
        let scanner_setup = self
            .scanner_setup
            .as_ref()
            .expect("scanner setup slot inserted")
            .1
            .clone();
        RenderSnapshot::new(
            params,
            self.film_name.clone(),
            self.print_name.clone(),
            self.data_dir.clone(),
            self.gui_state.sections["special"].clone(),
            scanner_setup,
        )
    }

    /// Submit the latest snapshot without waiting for active computation.
    fn dispatch_render(&mut self, ctx: &egui::Context) {
        let t_clone = Instant::now();
        let Some(image) = self.image.as_ref().cloned() else {
            return;
        };
        let input_clone_ms = t_clone.elapsed().as_secs_f32() * 1000.0;
        let kind = if self.full_scan_requested {
            RenderKind::Scan
        } else {
            RenderKind::Preview
        };
        let input_epoch = self.input_epoch;
        let mut operation = Operation::new(
            match kind {
                RenderKind::Preview => OperationKind::Preview,
                RenderKind::Scan => OperationKind::Scan,
            },
            self.diagnostics.mode,
        );
        operation.set_revisions(Some(input_epoch), Some(self.parameter_revision));
        operation.set_input_dimensions(image.width, image.height);
        operation.set_backend_requested(diagnostics::preview_backend_requested());
        let params = match self
            .current_state()
            .and_then(|state| state.runtime_params())
        {
            Ok(params) => params,
            Err(error) => {
                self.status = format!("{} state error: {error:#}", kind.label());
                operation.issue(IssueCategory::Configuration, IssueBoundary::Operation);
                self.diagnostics.finish(operation, Outcome::Failed);
                return;
            }
        };
        // Report the Dye field's effective CPU route next to the selected backend.
        let backend_name = effective_backend_name(self.backend.name(), &params);
        let parameter_revision = self.parameter_revision;
        let snapshot = self
            .render_snapshot(params)
            .with_cancellation(Some(Arc::new(AtomicBool::new(false))));
        let request = RenderRequest {
            job: RenderJob {
                input_epoch,
                parameter_revision,
                request_id: 0,
                kind,
                animate: self.force_preview
                    || self.full_scan_requested
                    || self.output_image.is_none(),
                backend_name,
                collection_mode: operation.mode(),
            },
            image,
            input_clone_ms,
            snapshot,
            backend: self.backend.clone(),
            source_metadata: self.source_metadata.clone(),
            display_enabled: self.viewer.settings.use_display_transform,
            display_profile: self.gui_state.sections["rust"]["display_profile"]
                .as_str()
                .map(PathBuf::from),
            ctx: ctx.clone(),
            worker_wait: operation.scope("worker_wait", ObservationKind::Phase, Purpose::Image),
            operation,
        };
        let (job, replaced) = self.render_worker.submit(request);
        if let Some(operation) = replaced {
            self.diagnostics.finish(operation, Outcome::Superseded);
        }
        self.render_job = Some(job);
        self.full_scan_requested = false;
    }

    fn poll_render_job(&mut self, ctx: &egui::Context) {
        loop {
            let (response_job, operation, result) = match self.render_worker.rx.try_recv() {
                Ok(response) => response,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.render_job = None;
                    self.status = "Render error: worker thread vanished".into();
                    return;
                }
            };
            if !response_job.matches(
                self.render_job.as_ref(),
                self.input_epoch,
                self.parameter_revision,
            ) {
                if self
                    .render_job
                    .as_ref()
                    .is_some_and(|job| job.request_id == response_job.request_id)
                {
                    self.render_job = None;
                }
                self.diagnostics.finish(operation, Outcome::Superseded);
                continue;
            }
            self.render_job = None;
            let kind = response_job.kind;
            let backend_name = response_job.backend_name;
            match result {
                Ok(r) => {
                    let handoff = operation.scope(
                        "presentation_handoff",
                        ObservationKind::Phase,
                        Purpose::Image,
                    );
                    self.source_render = Some(SourceRender::new(
                        operation.id(),
                        Some(response_job.input_epoch),
                        Some(response_job.parameter_revision),
                        r.output.width,
                        r.output.height,
                        response_job.collection_mode != CollectionMode::Off,
                        diagnostics::color_space(&r.output_color_space),
                        r.output_cctf_encoding,
                    ));
                    let display_profile = self.gui_state.sections["rust"]["display_profile"]
                        .as_str()
                        .map(Path::new);
                    let display_current = r.display_enabled
                        == self.viewer.settings.use_display_transform
                        && r.display_profile.as_deref() == display_profile
                        && r.display_max_size == self.params.settings.preview_max_size;
                    self.last_input_clone_ms = r.input_clone_ms;
                    self.last_scale_ms = r.scale_ms;
                    self.last_pipeline_build_ms = r.pipeline_build_ms;
                    self.last_render_ms = r.render_ms;
                    self.last_preview_ms = r.preview_ms;
                    self.last_worker_total_ms = r.worker_total_ms;
                    self.output_color_space = r.output_color_space;
                    self.output_cctf_encoding = r.output_cctf_encoding;
                    self.output_metadata = r.source_metadata;
                    self.viewer.transform_status = r.display_status;
                    self.viewer.set_output(
                        r.preview,
                        [r.output.width as usize, r.output.height as usize],
                        ctx.input(|i| i.time),
                        response_job.animate && display_current,
                    );
                    self.viewer.set_output_display_source(
                        &self.output_color_space,
                        self.output_cctf_encoding,
                        self.viewer.settings.use_display_transform,
                        self.gui_state.sections["rust"]["display_profile"]
                            .as_str()
                            .map(Path::new),
                    );
                    self.status = format!(
                        "{} · {} · {} × {} ({:.1} MP)",
                        kind.label(),
                        backend_name,
                        r.output.width,
                        r.output.height,
                        r.output.pixel_count() as f64 / 1e6
                    );
                    let output = Arc::new(r.output);
                    self.viewer.seed_prepared_output(
                        output.clone(),
                        &self.output_color_space,
                        self.output_cctf_encoding,
                        r.display_enabled,
                        r.display_profile.as_deref(),
                        r.display_max_size,
                    );
                    self.output_image = Some(output);
                    drop(handoff);
                    self.diagnostics.finish(operation, Outcome::Succeeded);
                }
                Err(msg) => {
                    eprintln!("[spektrafilm] render error: {msg}");
                    self.status = format!("Render error: {msg}");
                    self.diagnostics.finish(operation, Outcome::Failed);
                }
            }
        }
    }

    /// Save the latest accepted float output using current saving color settings.
    /// The typed extension selects the format; this never rerenders.
    fn save_dialog(&mut self) {
        if self.output_image.is_none() {
            self.status = "Nothing to save yet — load an image first.".into();
            return;
        }
        let default_name = match &self.image_path {
            Some(p) => {
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("spektrafilm");
                format!("{stem}.jpg")
            }
            None => "output.jpg".into(),
        };
        let Some(path) = self
            .file_dialog("save_output")
            .add_filter("Image", &image_io::container_extensions())
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        self.save_to_path(&path);
    }

    /// Save the retained output; `path` is the native chooser's confirmed result.
    fn save_to_path(&mut self, path: &Path) {
        if self.output_image.is_none() {
            self.status = "Nothing to save yet — load an image first.".into();
            return;
        }
        let mut operation = Operation::new(OperationKind::Save, self.diagnostics.mode);
        if let Some(source) = self.source_render.clone() {
            operation.set_source_render(source);
        }
        if let Some(source) = &self.source_render {
            operation.set_working_dimensions(source.dimensions[0], source.dimensions[1]);
        }
        self.remember_dialog("save_output", path);
        let actual_format = match image_io::ImageFormat::detect(path) {
            Ok(format) => format,
            Err(error) => {
                self.status = format!("Save error: {error}");
                operation.issue(IssueCategory::Writing, IssueBoundary::Operation);
                self.diagnostics.finish(operation, Outcome::Failed);
                return;
            }
        };
        let out = self
            .output_image
            .as_ref()
            .expect("output checked before dialog");
        let t = Instant::now();
        let destination = self.gui_state.sections["simulation"]["saving_color_space"]
            .as_str()
            .unwrap_or("sRGB");
        let encoded = self.gui_state.sections["simulation"]["saving_cctf_encoding"]
            .as_bool()
            .unwrap_or(true);
        let save_options = SaveOptions {
            depth: match actual_format {
                image_io::ImageFormat::Jpeg | image_io::ImageFormat::Png => BitDepth::Eight,
                image_io::ImageFormat::Tiff | image_io::ImageFormat::Exr => BitDepth::Sixteen,
            },
            color_space: destination,
            cctf_encoding: encoded,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
        };
        if operation.enabled() {
            operation.set_saving_configuration(
                spektrafilm_core::telemetry::SavingConfiguration::from_save_options(
                    actual_format,
                    &save_options,
                ),
            );
        }
        let saving_convert =
            operation.scope("saving_convert", ObservationKind::Phase, Purpose::Image);
        let converted = match image_io::convert_image(
            out,
            &self.output_color_space,
            self.output_cctf_encoding,
            destination,
            encoded,
        ) {
            Ok(image) => image,
            Err(error) => {
                self.status = format!("Save error: {error}");
                saving_convert
                    .context()
                    .issue(IssueCategory::Writing, IssueBoundary::SavingConvert);
                saving_convert.finish(Outcome::Failed);
                self.diagnostics.finish(operation, Outcome::Failed);
                return;
            }
        };
        drop(saving_convert);
        let file_write = operation.scope("file_write", ObservationKind::Phase, Purpose::Image);
        let result = image_io::save_rendered_output_observed(
            path,
            &converted,
            save_options,
            self.output_metadata.as_ref(),
            file_write.context(),
        );
        let result = observed_result(
            file_write,
            result,
            IssueCategory::Writing,
            IssueBoundary::FileWrite,
        );
        match result {
            Ok(report) => {
                self.status = format!(
                    "Saved {} in {:.0} ms",
                    path.file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("(file)"),
                    t.elapsed().as_secs_f32() * 1000.0
                );
                if let Some(warning) = report.metadata_warning {
                    self.status
                        .push_str(&format!(" — Metadata warning: {warning}"));
                }
                operation.set_published_dimensions(converted.width, converted.height);
                self.diagnostics.finish(operation, Outcome::Succeeded);
            }
            Err(e) => {
                self.status = format!("Save error: {e:#}");
                self.diagnostics.finish(operation, Outcome::Failed);
            }
        }
    }
}

/// Extract the human-readable payload from a caught panic. Panics carry
/// either a `&str` (literal messages) or a `String` (formatted ones);
/// anything else gets a generic label.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        format!("panic: {s}")
    } else if let Some(s) = panic.downcast_ref::<String>() {
        format!("panic: {s}")
    } else {
        "panic (no message)".to_string()
    }
}

/// Find the data directory. Probes, in order:
///   1. `$SPEKTRAFILM_DATA_DIR` — explicit override.
///   2. `<exe>/../Resources/data` — macOS `.app` bundle layout
///      (`Foo.app/Contents/MacOS/<exe>` → `Contents/Resources/data`).
///   3. `<exe>/data` — data shipped next to the binary.
///   4. `./data` — current working directory.
///   5. `<CARGO_MANIFEST_DIR>/../../data` — workspace root, for
///      `cargo run -p spektrafilm-gui`.
///
/// Finder launches apps with `cwd=/`, so the exe-relative probes (2–3)
/// matter for a double-clicked `.app`. Falls back to `./data` so the
/// downstream loader surfaces a clear "profiles not found" error.
fn pick_data_dir() -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("SPEKTRAFILM_DATA_DIR") {
        candidates.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("..").join("Resources").join("data"));
        candidates.push(dir.join("data"));
    }
    candidates.push(PathBuf::from("data"));
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        candidates.push(PathBuf::from(manifest).join("..").join("..").join("data"));
    }
    candidates
        .into_iter()
        .find(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("data"))
}

/// macOS only: walk from the eframe `RawWindowHandle` down to the
/// `CAMetalLayer` and tag its colorspace as sRGB. Without this the
/// metal layer's `colorspace` is null and macOS treats the framebuffer
/// pixels as raw display primaries — sRGB content rendered into a
/// wide-gamut panel comes out oversaturated. Setting the layer's
/// colorspace makes the OS gamut-map exactly like it does for a
/// regular sRGB-tagged PNG opened in Preview, so what you see in the
/// GUI matches what you export.
#[cfg(target_os = "macos")]
fn tag_metal_layer_srgb<H: raw_window_handle::HasWindowHandle>(h: &H) -> Result<(), String> {
    use core_graphics::color_space::{CGColorSpace, kCGColorSpaceSRGB};
    use foreign_types::ForeignType;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use raw_window_handle::RawWindowHandle;

    let handle = h
        .window_handle()
        .map_err(|e| format!("no window handle: {e}"))?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return Err("not an AppKit window".into());
    };
    let ns_view: *mut AnyObject = appkit.ns_view.as_ptr().cast();
    if ns_view.is_null() {
        return Err("ns_view is null".into());
    }

    // `kCGColorSpaceSRGB` is an extern static CFStringRef, accessing it
    // is unsafe in the 2024 edition's stricter model.
    let srgb_cs = unsafe { CGColorSpace::create_with_name(kCGColorSpaceSRGB) }
        .ok_or_else(|| "CGColorSpaceCreateWithName(sRGB) returned null".to_string())?;
    // `foreign_types::ForeignType::as_ptr` returns the opaque
    // `CGColorSpaceRef` (i.e. `*mut sys::CGColorSpace`). `setColorspace:`
    // is declared as taking a `CGColorSpaceRef`, whose ObjC type encoding
    // is `^{CGColorSpace=}`. A bare `*mut c_void` encodes as `^v`, which
    // objc2's debug-build argument-encoding verification rejects (the
    // release build skips that check, which is why only debug crashed).
    // Cast through a zero-field opaque struct whose `Encode` matches the
    // expected `^{CGColorSpace=}` so both builds pass the same pointer.
    #[repr(C)]
    struct CGColorSpaceOpaque {
        _private: [u8; 0],
    }
    // A pointer to this type encodes as `^{CGColorSpace=}` — what
    // `setColorspace:` expects. `RefEncode` (not `Encode`) is the trait
    // objc2 consults for a `*const T` argument.
    unsafe impl objc2::RefEncode for CGColorSpaceOpaque {
        const ENCODING_REF: objc2::Encoding =
            objc2::Encoding::Pointer(&objc2::Encoding::Struct("CGColorSpace", &[]));
    }
    let cs_ref = srgb_cs.as_ptr() as *const CGColorSpaceOpaque;

    // Only `CAMetalLayer` has a `colorspace` property; calling
    // `setColorspace:` on a plain `CALayer` is an unrecognized selector
    // (crash), so we always gate on `respondsToSelector:`.
    let selector = objc2::sel!(setColorspace:);
    unsafe {
        let root_layer: *mut AnyObject = msg_send![ns_view, layer];
        if root_layer.is_null() {
            return Err("ns_view.layer is null".into());
        }
        // Find the CAMetalLayer. wgpu-hal 24 (the raw-window-metal
        // approach) does NOT replace the view's layer: when the root
        // layer isn't already a CAMetalLayer — the default for an
        // eframe/winit NSView — it installs the metal layer as a
        // *sublayer*. So the root layer is a plain CALayer with no
        // `colorspace`; the layer we must tag is the metal sublayer.
        // (Older eframe set the view's own layer to the CAMetalLayer, so
        // check the root first and fall back to scanning sublayers.)
        let metal_layer = if msg_send![root_layer, respondsToSelector: selector] {
            root_layer
        } else {
            let sublayers: *mut AnyObject = msg_send![root_layer, sublayers];
            let mut found: *mut AnyObject = std::ptr::null_mut();
            if !sublayers.is_null() {
                let count: usize = msg_send![sublayers, count];
                for i in 0..count {
                    let sub: *mut AnyObject = msg_send![sublayers, objectAtIndex: i];
                    if !sub.is_null() && msg_send![sub, respondsToSelector: selector] {
                        found = sub;
                        break;
                    }
                }
            }
            found
        };
        if metal_layer.is_null() {
            // The metal sublayer may not exist yet on the first frame;
            // the caller retries until this succeeds.
            return Err("no CAMetalLayer on the view or its sublayers yet".into());
        }
        // CAMetalLayer.colorspace is `retain`-strong, so ObjC takes its
        // own reference; we can let `srgb_cs` drop after the call.
        let _: () = msg_send![metal_layer, setColorspace: cs_ref];
        tracing::info!("tagged CAMetalLayer.colorspace = sRGB");
    }
    Ok(())
}

#[cfg(test)]
mod telemetry_lifecycle_tests {
    use super::*;
    use spektrafilm_gpu::telemetry::ObservationContext;

    fn scanner_app() -> App {
        let mut app = App::new_with_browser_path(
            Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend),
            None,
            None,
            None,
            std::env::temp_dir().join(format!(
                "scanner-snapshots-{}-{}.json",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data"),
        );
        app.film_name = "kodak_portra_400".into();
        app.print_name = "kodak_portra_endura".into();
        app.params.workflow.route = "input > film > scan".into();
        app.params.io.scan_film = true;
        app.params.scanner.scan_output = "negative_to_positive".into();
        app.params.scanner.auto_setup = ScannerAutoSetup::PerFrame;
        app.params.scanner.white_correction = false;
        app.params.scanner.black_correction = false;
        app.params.film_render.grain.active = false;
        app.params.camera.auto_exposure = false;
        app.params.print_render.glare.active = false;
        app.params.settings.preview_max_size = 4;
        app.image = Some(Arc::new(ImageBuf::from_data(
            24,
            8,
            (0..24 * 8 * 3)
                .map(|i| {
                    spektrafilm_math::precision::from_f64(if i / 3 % 24 < 5 { 0.01 } else { 0.7 })
                })
                .collect(),
        )));
        app
    }

    #[test]
    fn app_source_remeasurement_and_manual_scanner_edits_preserve_user_brightness() {
        let mut app = scanner_app();
        let context = ObservationContext::default();
        let original = app.render_snapshot(app.params.clone());
        let initial = original
            .prepare_scanner_setup(app.image.as_ref().unwrap(), &context)
            .unwrap()
            .unwrap();
        let mut changed_input = (**app.image.as_ref().unwrap()).clone();
        for (index, sample) in changed_input.data.iter_mut().enumerate() {
            *sample *= spektrafilm_math::precision::from_f64([0.18, 0.45, 0.8][index % 3]);
        }
        app.image = Some(Arc::new(changed_input));
        app.input_epoch = app.input_epoch.wrapping_add(1);
        let source = app.render_snapshot(app.params.clone());
        let source_setup = source
            .prepare_scanner_setup(app.image.as_ref().unwrap(), &context)
            .unwrap()
            .unwrap();
        assert_ne!(
            initial, source_setup,
            "new chromatic source must change measured scanner setup"
        );
        app.params.scanner.positive_density_ev += 1.0;
        app.parameter_revision = app.parameter_revision.wrapping_add(1);
        let edited = app.render_snapshot(app.params.clone());
        assert!(!Arc::ptr_eq(source.scanner_setup(), edited.scanner_setup()));
        assert!(
            edited.scanner_setup().get().is_none(),
            "parameter revision must create a fresh measurement slot"
        );
        let edited_setup = edited
            .prepare_scanner_setup(app.image.as_ref().unwrap(), &context)
            .unwrap()
            .unwrap();
        assert_eq!(
            source_setup, edited_setup,
            "derived setup must preserve the user's additional brightness"
        );
        let input = app.image.as_ref().unwrap();
        let source_runtime = build_runtime_observed(&source, false, &context)
            .unwrap()
            .with_scanner_setup(source_setup);
        let edited_runtime = build_runtime_observed(&edited, false, &context)
            .unwrap()
            .with_scanner_setup(edited_setup);
        assert_eq!(
            edited_runtime.params().scanner.positive_density_ev
                - source_runtime.params().scanner.positive_density_ev,
            1.0
        );
        let before = source_runtime
            .process((**input).clone(), app.backend.as_ref())
            .unwrap();
        let after = edited_runtime
            .process((**input).clone(), app.backend.as_ref())
            .unwrap();
        assert!(
            after
                .data
                .iter()
                .all(|sample| spektrafilm_math::precision::to_f64(*sample).is_finite())
        );
        let brightness = |image: &ImageBuf| {
            image
                .data
                .iter()
                .map(|sample| spektrafilm_math::precision::to_f64(*sample))
                .sum::<f64>()
        };
        assert!(
            brightness(&after) > brightness(&before),
            "manual +1 EV must increase rendered brightness with the measured setup retained"
        );
        assert_eq!(original.scanner_setup().get().unwrap(), &Ok(initial));
        assert_eq!(source.scanner_setup().get().unwrap(), &Ok(source_setup));
    }

    #[test]
    fn app_preview_scan_and_export_measure_canonical_full_source_once() {
        for first in [Some(true), Some(false), None] {
            let mut app = scanner_app();
            let snapshot = app.render_snapshot(app.params.clone());
            let context = ObservationContext::default();
            let input = app.image.as_ref().unwrap().clone();
            let canonical = build_runtime_observed(&snapshot, false, &context).unwrap();
            let expected = canonical.prepare_scanner_setup(&input).unwrap();
            let mut cache = RuntimeCache::default();
            if let Some(preview_first) = first {
                let runtime = cache
                    .prepare_observed(&snapshot, preview_first, &input, &context)
                    .unwrap();
                let working = if preview_first {
                    spektrafilm_core::params_builder::resize_for_preview(
                        &input,
                        runtime.params().settings.preview_max_size,
                    )
                } else {
                    (*input).clone()
                };
                let rendered = runtime.process(working, app.backend.as_ref()).unwrap();
                assert!(
                    rendered
                        .data
                        .iter()
                        .all(|sample| spektrafilm_math::precision::to_f64(*sample).is_finite())
                );
                assert_eq!(runtime.params().settings.preview_mode, preview_first);
            } else {
                assert_eq!(
                    snapshot.prepare_scanner_setup(&input, &context).unwrap(),
                    Some(expected)
                );
            }
            assert_eq!(snapshot.scanner_setup().get().unwrap(), &Ok(expected));
            let original_params = serde_json::to_value(snapshot.params()).unwrap();
            let export = app.render_snapshot(app.params.clone());
            assert_eq!(
                export.prepare_scanner_setup(&input, &context).unwrap(),
                Some(expected)
            );
            let mut alternative_input = (*input).clone();
            for (index, sample) in alternative_input.data.iter_mut().enumerate() {
                *sample *= spektrafilm_math::precision::from_f64([0.18, 0.45, 0.8][index % 3]);
            }
            assert_ne!(
                canonical.prepare_scanner_setup(&alternative_input).unwrap(),
                expected
            );
            assert_eq!(
                export
                    .prepare_scanner_setup(&alternative_input, &context)
                    .unwrap(),
                Some(expected),
                "initialized snapshot must reuse its measured full source rather than remeasure another raster"
            );
            let scan_runtime = cache
                .prepare_observed(&export, false, &input, &context)
                .unwrap();
            let scan = scan_runtime
                .process((*input).clone(), app.backend.as_ref())
                .unwrap();
            let export_runtime = build_runtime_observed(&export, false, &context)
                .unwrap()
                .with_scanner_setup(expected);
            let exported = export_runtime
                .process((*input).clone(), app.backend.as_ref())
                .unwrap();
            assert_eq!(
                scan.data, exported.data,
                "shared full setup must produce the same Scan and Export raster"
            );
            let preview_runtime = cache
                .prepare_observed(&export, true, &input, &context)
                .unwrap();
            let working = spektrafilm_core::params_builder::resize_for_preview(
                &input,
                preview_runtime.params().settings.preview_max_size,
            );
            let preview = preview_runtime
                .process(working, app.backend.as_ref())
                .unwrap();
            assert_eq!(preview.width.max(preview.height), 4);
            assert!(
                preview
                    .data
                    .iter()
                    .all(|sample| spektrafilm_math::precision::to_f64(*sample).is_finite())
            );
            assert!(Arc::ptr_eq(
                snapshot.scanner_setup(),
                export.scanner_setup()
            ));
            assert_eq!(
                serde_json::to_value(snapshot.params()).unwrap(),
                original_params
            );
        }
    }

    #[test]
    fn app_real_negative_setup_ignores_virtual_camera_metering() {
        let mut app = scanner_app();
        app.params.workflow.route = "input > convert-film > scan".into();
        let context = ObservationContext::default();
        let input = app.image.as_ref().unwrap().clone();
        app.params.camera.auto_exposure = false;
        let first = app.render_snapshot(app.params.clone());
        let expected = first.prepare_scanner_setup(&input, &context).unwrap();
        app.parameter_revision = app.parameter_revision.wrapping_add(1);
        app.params.camera.auto_exposure = true;
        app.params.camera.exposure_compensation_ev += 3.0;
        let changed = app.render_snapshot(app.params.clone());
        assert_eq!(
            changed.prepare_scanner_setup(&input, &context).unwrap(),
            expected
        );
        let first_runtime = build_runtime_observed(&first, false, &context)
            .unwrap()
            .with_scanner_setup(expected.unwrap());
        let changed_runtime = build_runtime_observed(&changed, false, &context)
            .unwrap()
            .with_scanner_setup(expected.unwrap());
        let first_output = first_runtime
            .process((*input).clone(), app.backend.as_ref())
            .unwrap();
        let changed_output = changed_runtime
            .process((*input).clone(), app.backend.as_ref())
            .unwrap();
        assert_eq!(
            first_output.data, changed_output.data,
            "real negative render must bypass virtual camera metering"
        );
    }

    #[test]
    fn app_channel_swaps_use_identical_canonical_and_render_calibration() {
        let mut app = scanner_app();
        app.gui_state.sections["special"]["film_channel_swap"] = serde_json::json!([2, 0, 1]);
        app.gui_state.sections["special"]["print_channel_swap"] = serde_json::json!([1, 2, 0]);
        let snapshot = app.render_snapshot(app.params.clone());
        let input = app.image.as_ref().unwrap().clone();
        let context = ObservationContext::default();
        let canonical = build_runtime_observed(&snapshot, false, &context).unwrap();
        let mut cache = RuntimeCache::default();
        let actual = cache
            .prepare_without_setup(&snapshot, false, &context)
            .unwrap();
        let expected = canonical.prepare_scanner_setup(&input).unwrap();
        assert_eq!(
            actual.prepare_scanner_setup(&input).unwrap(),
            expected,
            "full setup and cached runtime must apply the same film and print channel swaps"
        );
        let canonical_output = canonical
            .with_scanner_setup(expected)
            .process((*input).clone(), app.backend.as_ref())
            .unwrap();
        let actual = cache
            .prepare_observed(&snapshot, false, &input, &context)
            .unwrap();
        let actual_output = actual
            .process((*input).clone(), app.backend.as_ref())
            .unwrap();
        assert_eq!(actual_output.data, canonical_output.data);
    }

    #[test]
    fn imported_negative_conversion_slide_state_fails_without_mutating_current_scan() {
        let mut app = scanner_app();
        let original = app.params.clone();
        let revision = app.parameter_revision;
        let mut invalid = app.current_state().unwrap();
        invalid.sections["simulation"]["film_stock"] = "fujifilm_provia_100f".into();
        invalid.sections["scanner"]["positive_interpretation"] = "coupled".into();
        invalid.sections["scanner"]["auto_setup"] = "per_frame".into();
        invalid.sections["scanner"]["finish"] = "lab_scan".into();
        let error = app.apply_state(invalid).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("negative filming-stage film profile")
        );
        assert_eq!(app.params, original);
        assert_eq!(app.parameter_revision, revision);
        assert_eq!(app.film_name, "kodak_portra_400");
        app.image_path = None;
        let valid = app.current_state().unwrap();
        app.apply_state(valid).unwrap();
        assert_eq!(app.params.scanner.scan_output, "negative_to_positive");
        assert_eq!(app.params.scanner.auto_setup, ScannerAutoSetup::PerFrame);
    }

    fn job(request_id: u64) -> RenderJob {
        RenderJob {
            input_epoch: 2,
            parameter_revision: 5,
            request_id,
            kind: RenderKind::Preview,
            animate: false,
            backend_name: "CPU".into(),
            collection_mode: CollectionMode::Summary,
        }
    }

    #[test]
    fn acceptance_rejects_both_revision_changes_and_newer_requests() {
        let old = job(1);
        let latest = job(2);
        assert!(!old.matches(Some(&latest), 2, 5));
        assert!(!latest.matches(Some(&latest), 3, 5));
        assert!(!latest.matches(Some(&latest), 2, 6));
        assert!(latest.matches(Some(&latest), 2, 5));
    }

    #[test]
    fn failed_write_keeps_partial_scope_and_failed_terminal_outcome() {
        let operation = Operation::new(OperationKind::Save, CollectionMode::Summary);
        let scope = operation.scope("file_write", ObservationKind::Phase, Purpose::Image);
        let result: Result<(), &str> = observed_result(
            scope,
            Err("write failure"),
            IssueCategory::Writing,
            IssueBoundary::FileWrite,
        );
        assert!(result.is_err());
        let report = operation.finish(Outcome::Failed).unwrap();
        assert_eq!(report.operation.outcome, Outcome::Failed);
        assert!(report.phases.iter().any(|phase| phase.name == "file_write"
            && !phase.complete
            && phase.outcome == Some(Outcome::Failed)));
        assert!(
            report
                .diagnostic_issues
                .iter()
                .any(|issue| issue.category == IssueCategory::Writing
                    && issue.boundary == IssueBoundary::FileWrite)
        );
    }

    #[test]
    fn concurrent_contexts_do_not_share_observations() {
        let preview = Operation::new(OperationKind::Preview, CollectionMode::Summary);
        let export = Operation::new(OperationKind::Export, CollectionMode::Summary);
        let preview_context = preview.context();
        let export_context = export.context();
        let handle = std::thread::spawn(move || {
            let phase = export_context.scope("file_write", ObservationKind::Phase, Purpose::Image);
            phase.finish(Outcome::Succeeded);
        });
        let phase =
            preview_context.scope("display_prepare", ObservationKind::Phase, Purpose::Image);
        phase.finish(Outcome::Succeeded);
        handle.join().unwrap();
        let preview = preview.finish(Outcome::Superseded).unwrap();
        let export = export.finish(Outcome::Cancelled).unwrap();
        assert!(
            preview
                .phases
                .iter()
                .all(|phase| phase.name != "file_write")
        );
        assert!(
            export
                .phases
                .iter()
                .all(|phase| phase.name != "display_prepare")
        );
        assert_ne!(preview.operation.id, export.operation.id);
    }
}
