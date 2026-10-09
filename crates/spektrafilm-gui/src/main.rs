// Interactive spektrafilm preview and export GUI.
//
// The egui shell follows the pinned Python 0.3.4 workflow: a tabbed sidebar
// groups the main, film, print, advanced and configuration controls, while
// the central viewer owns the input/output/paper-back presentation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use eframe::egui;
use spektrafilm_core::image_io::{
    self, BitDepth, Compression, ImageMetadata, LoadedImage, SaveOptions,
};
use spektrafilm_core::params::RuntimeParams;
use spektrafilm_core::profile;
use spektrafilm_core::runtime::{DigestMode, Runtime, RuntimePhotoParams};
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
mod controls;
mod display;
mod export;
use export::{ExportBackend, ExportCompression, ExportDialog, ExportFormat, ExportOptions};
mod panels;
mod state;

const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);
const IN_FLIGHT_REPAINT: Duration = Duration::from_millis(16);
const IMAGE_FILE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "tif", "tiff", "exr",
    // Keep this list in lockstep with image_io::is_raw.
    "dng", "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef", "srw",
    "x3f", "iiq", "3fr", "crw", "rwl", "mrw", "mef", "kdc", "ari", "bay", "dcr", "drf", "erf",
    "fff", "k25", "mos", "ptx",
];

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
        Box::new(|cc| {
            Ok(Box::new(App::new(
                cc,
                backend,
                initial_image,
                initial_state,
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

/// One entry in the film / paper combo box. `stock` is the filename
/// stem (the unique key the profile loader expects); `display` is the
/// human-readable label from the profile's `info.name`, falling back
/// to the stock id when missing.
#[derive(Debug, Clone)]
struct ProfileEntry {
    stock: String,
    display: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GuiTab {
    #[default]
    Main,
    Film,
    Print,
    Advanced,
    Config,
}

impl GuiTab {
    const ALL: [Self; 5] = [
        Self::Main,
        Self::Film,
        Self::Print,
        Self::Advanced,
        Self::Config,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Main => "MAIN",
            Self::Film => "FILM",
            Self::Print => "PRINT",
            Self::Advanced => "ADVANCED",
            Self::Config => "CONFIG",
        }
    }
}

struct App {
    backend: Arc<dyn ComputeBackend>,
    data_dir: PathBuf,
    /// Profiles where `info.support == "film"`.
    films: Vec<ProfileEntry>,
    /// Profiles where `info.support == "paper"` (or other print-stage
    /// supports).
    papers: Vec<ProfileEntry>,
    film_name: String,
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
    /// Last rendered pipeline output (post sRGB encode + clip). Retained
    /// so the Save button can write it without re-running the pipeline.
    output_image: Option<ImageBuf>,
    output_metadata: Option<ImageMetadata>,
    viewer: display::Viewer,
    gui_tab: GuiTab,
    output_color_space: String,
    output_cctf_encoding: bool,
    pipeline_cache_key: Option<String>,
    pipeline_cache: Option<Runtime>,
    last_render_ms: f32,
    last_pipeline_build_ms: f32,
    last_input_clone_ms: f32,
    last_scale_ms: f32,
    last_preview_ms: f32,
    last_worker_total_ms: f32,
    status: String,
    /// Set when any control change should trigger a re-render. The
    /// next `update()` tick dispatches the render onto a worker
    /// thread so the UI stays responsive while the pipeline (up to
    /// ~500 ms on 6 MP) runs in the background.
    dirty: bool,
    /// Marked when the user changes a param while a previous render
    /// is still in flight. After that render finishes we re-arm
    /// `dirty` so the latest state gets a fresh pass instead of
    /// dropping the user's mid-render edits on the floor.
    pending_dirty: bool,
    dirty_since: Option<Instant>,
    /// macOS-only: tag the wgpu CAMetalLayer's `colorspace` as sRGB on
    /// the first `update()` tick (it's not yet wired up at
    /// `App::new` time). Set to `true` once the call succeeds so we
    /// don't retry every frame.
    #[cfg(target_os = "macos")]
    metal_colorspace_tagged: bool,
    /// In-flight pipeline render. `Some` while the worker thread is
    /// running; main thread polls the receiver each frame and
    /// uploads the resulting texture once it lands. Decoupling the
    /// render from the UI thread is what keeps sliders responsive —
    /// the 250–500 ms pipeline used to block input handling.
    render_job: Option<RenderJob>,
    /// In-flight export job. `Some` while the immutable snapshot is rendering;
    /// the `update()` loop polls the receiver each frame and
    /// surfaces success/failure in `status` when the worker thread
    /// completes. Joined eagerly to release the thread.
    export_job: Option<ExportJob>,
    /// In-flight Convert controller action. The epoch drops results made stale
    /// by later parameter edits or a newer action.
    calibration_job: Option<CalibrationJob>,
    calibration_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderKind {
    Preview,
    Scan,
}

impl RenderKind {
    fn label(self) -> &'static str {
        match self {
            Self::Preview => "Preview",
            Self::Scan => "Scan",
        }
    }
}

/// One in-flight preview render. The worker owns a Runtime + the
/// ImageBuf clone and, when it finishes, sends back the output buffer
/// plus the two timings the status bar shows.
struct RenderJob {
    input_epoch: u64,
    kind: RenderKind,
    backend_name: String,
    rx: mpsc::Receiver<Result<RenderResult, String>>,
    handle: Option<JoinHandle<()>>,
}

struct RenderResult {
    output: ImageBuf,
    preview: display::DisplayRaster,
    display_status: String,
    display_enabled: bool,
    display_profile: Option<PathBuf>,
    display_max_size: u32,
    output_color_space: String,
    output_cctf_encoding: bool,
    source_metadata: Option<ImageMetadata>,
    input_clone_ms: f32,
    scale_ms: f32,
    pipeline_build_ms: f32,
    render_ms: f32,
    preview_ms: f32,
    worker_total_ms: f32,
}

/// One in-flight export owning an immutable input, pipeline and option snapshot.
/// Cancellation discards its staged file once the current operation completes.
/// The UI alone publishes successful, uncancelled work.
struct ExportJob {
    rx: mpsc::Receiver<Result<ExportResult, String>>,
    handle: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    started_at: Instant,
    backend: ExportBackend,
}

struct ExportResult {
    elapsed: f32,
    filename: String,
    backend_name: String,
    size: [u32; 2],
    metadata_warning: Option<String>,
    staged: TempPath,
}

enum CalibrationResult {
    Base {
        params: spektrafilm_core::params::FilmBaseParams,
        exposure_ev: f64,
    },
    BlindCalibration(String),
    NeutralizeFilters {
        m_shift: f32,
        y_shift: f32,
    },
}

struct CalibrationJob {
    epoch: u64,
    rx: mpsc::Receiver<Result<CalibrationResult, String>>,
    handle: Option<JoinHandle<()>>,
}

impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        backend: Arc<dyn ComputeBackend>,
        initial_image: Option<PathBuf>,
        initial_state: Option<PathBuf>,
    ) -> Self {
        let _ = cc;

        let data_dir = pick_data_dir();
        let (films, papers) = scan_profiles(&data_dir);
        let startup = match initial_state {
            Some(path) => state::GuiState::load(&path),
            None => state::startup(),
        };
        let startup_error = startup
            .as_ref()
            .err()
            .map(|e| format!("Startup state error: {e:#}"));
        let gui_state = startup.unwrap_or_else(|_| state::GuiState::factory());
        let film_name = gui_state.film().to_owned();
        let print_name = gui_state.paper().to_owned();
        let film_profile = profile::load_profile_by_name(&data_dir, &film_name).ok();
        let params = gui_state
            .runtime_params()
            .expect("validated GUI factory state");
        let film_dev_times = film_profile
            .as_ref()
            .map(|f| f.data.development_time.clone())
            .unwrap_or_default();
        let print_dev_times = profile_dev_times(&data_dir, &print_name);
        let export_options = ExportOptions::from_state(&gui_state.sections);

        let mut app = Self {
            backend,
            data_dir,
            films,
            papers,
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
            output_image: None,
            output_metadata: None,
            viewer: display::Viewer::new(),
            gui_tab: GuiTab::default(),
            output_color_space: "sRGB".into(),
            output_cctf_encoding: true,
            pipeline_cache_key: None,
            pipeline_cache: None,
            last_render_ms: 0.0,
            last_pipeline_build_ms: 0.0,
            last_input_clone_ms: 0.0,
            last_scale_ms: 0.0,
            last_preview_ms: 0.0,
            last_worker_total_ms: 0.0,
            pending_dirty: false,
            dirty_since: None,
            render_job: None,
            export_job: None,
            status: startup_error.unwrap_or_else(|| String::from("Load an image to start.")),
            dirty: false,
            #[cfg(target_os = "macos")]
            metal_colorspace_tagged: false,
            calibration_job: None,
            calibration_epoch: 0,
        };
        app.viewer.settings =
            display::DisplaySettings::from_json(&app.gui_state.sections["display"]);
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
        state::GuiState::from_runtime(&self.params, &self.film_name, &self.print_name, &extras)
    }

    fn apply_state(&mut self, state: state::GuiState) -> Result<()> {
        let params = state.runtime_params()?;
        profile::load_profile_by_name(&self.data_dir, state.film())?;
        profile::load_profile_by_name(&self.data_dir, state.paper())?;
        self.film_name = state.film().to_owned();
        self.print_name = state.paper().to_owned();
        self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
        self.film_dev_times = profile_dev_times(&self.data_dir, &self.film_name);
        self.print_dev_times = profile_dev_times(&self.data_dir, &self.print_name);
        self.viewer.settings = display::DisplaySettings::from_json(&state.sections["display"]);
        self.params = params;
        self.export_options = ExportOptions::from_state(&state.sections);
        self.export_dialog = ExportDialog::default();
        self.gui_state = state;
        self.scan_for_print_snapshot = None;
        self.raw_lens_info = None;
        self.pipeline_cache_key = None;
        self.pipeline_cache = None;
        self.dirty = true;
        self.force_preview = true;
        if let Some(path) = self.image_path.clone() {
            self.load_image_from_path(&path);
        }
        self.viewer
            .restore_state(&self.gui_state.sections["rust"]["viewer"]);
        Ok(())
    }

    fn refresh_viewing_artifacts(&mut self) {
        if let Some(image) = self.image.as_ref() {
            match display::input_display_raster(
                image,
                &self.params.io.input_color_space,
                self.params.io.input_cctf_decoding,
                self.params.settings.preview_max_size as usize,
            ) {
                Ok(raster) => {
                    self.viewer.replace_input_display(raster);
                    self.viewer.set_input_display_source(
                        &self.params.io.input_color_space,
                        self.params.io.input_cctf_decoding,
                    );
                }
                Err(e) => self.status = format!("Viewer input error: {e}"),
            }
        }
        if let Some(output) = self.output_image.as_ref() {
            match display::output_display_raster(
                output,
                &self.output_color_space,
                self.output_cctf_encoding,
                self.viewer.settings.use_display_transform,
                self.gui_state.sections["rust"]["display_profile"]
                    .as_str()
                    .map(Path::new),
                self.params.settings.preview_max_size as usize,
            ) {
                Ok((raster, status)) => {
                    self.viewer.replace_output_display(raster);
                    self.viewer.transform_status = status;
                }
                Err(e) => self.status = format!("Viewer display error: {e}"),
            }
            self.viewer.set_output_display_source(
                &self.output_color_space,
                self.output_cctf_encoding,
                self.viewer.settings.use_display_transform,
                self.gui_state.sections["rust"]["display_profile"]
                    .as_str()
                    .map(Path::new),
            );
        }
    }

    fn state_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Save current to file").clicked() {
                if let Some(path) = self
                    .file_dialog("state")
                    .set_file_name("gui_state.json")
                    .add_filter("JSON", &["json"])
                    .save_file()
                {
                    self.remember_dialog("state", &path);
                    let result = self.current_state().and_then(|s| s.save(&path));
                    self.status = match result {
                        Ok(()) => format!("Saved GUI state to {}", path.display()),
                        Err(e) => format!("State save error: {e:#}"),
                    };
                }
            }
            if ui.button("Load from file").clicked() {
                if let Some(path) = self
                    .file_dialog("state")
                    .add_filter("JSON", &["json"])
                    .pick_file()
                {
                    self.remember_dialog("state", &path);
                    let result = state::GuiState::load(&path).and_then(|s| self.apply_state(s));
                    self.status = match result {
                        Ok(()) => format!("Loaded GUI state from {}", path.display()),
                        Err(e) => format!("State load error: {e:#}"),
                    };
                }
            }
            if ui.button("Save current as default").clicked() {
                let result = self
                    .current_state()
                    .and_then(|s| s.save(&state::default_path()));
                self.status = match result {
                    Ok(()) => "Saved current GUI state as startup default".into(),
                    Err(e) => format!("Startup save error: {e:#}"),
                };
            }
            if ui.button("Restore factory default").clicked() {
                let result = state::reset_factory().and_then(|s| self.apply_state(s));
                self.status = match result {
                    Ok(()) => "Restored factory default GUI state".into(),
                    Err(e) => format!("Factory reset error: {e:#}"),
                };
            }
        });
    }

    fn toggle_scan_for_print(&mut self) {
        if let Some((white, black, glare)) = self.scan_for_print_snapshot.take() {
            self.params.scanner.white_correction = white;
            self.params.scanner.black_correction = black;
            self.params.print_render.glare.active = glare;
        } else {
            self.scan_for_print_snapshot = Some((
                self.params.scanner.white_correction,
                self.params.scanner.black_correction,
                self.params.print_render.glare.active,
            ));
            self.params.scanner.white_correction = true;
            self.params.scanner.black_correction = true;
            self.params.print_render.glare.active = false;
        }
        self.pipeline_cache_key = None;
        self.pipeline_cache = None;
        self.dirty = true;
        self.force_preview = true;
    }

    fn simulation_action_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            controls::extra_bool_tip(ui, &mut self.gui_state.sections, "simulation", "auto_preview", "auto preview", true, "trigger the preview after every change of gui parameters, use mouse scrollwheel on parameters field, read preview tooltip for details");
            let mut scan_for_print = self.scan_for_print_snapshot.is_some();
            if ui.checkbox(&mut scan_for_print, "black and white correction").on_hover_text("White and black correction of the scanner are active, and glare is deactivated.").changed() {
                self.toggle_scan_for_print();
            }
        });
        ui.horizontal(|ui| {
            if controls::choice_tip(
                ui,
                "workflow",
                &mut self.params.workflow.route,
                &[
                    "input",
                    "input > film > scan",
                    "input > film > print > scan",
                    "input > convert-film > print > scan",
                    "input > convert-film > scan-minus-base",
                    "input > convert-film > scan",
                ],
                "Which path the image takes through the pipeline: input (passthrough: just colour-manage the input to the output space for viewing), input > film > scan (scan the negative directly), input > film > print > scan (full chain), input > convert-film > print > scan (print a scene-referred input and scan it), input > convert-film > scan-minus-base (convert input and scan with base removed), input > convert-film > scan (convert input, then scan the film with its base).",
            ) {
                self.params.io.scan_film = false;
                self.dirty = true;
                self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
            }
        });
        ui.horizontal(|ui| {
            let busy = self.render_job.is_some()
                || self.export_job.is_some()
                || self.calibration_job.is_some();
            let ready = self.image.is_some() && !busy;
            if ui
                .add_enabled(ready, egui::Button::new("PREVIEW"))
                .on_hover_text("run the simulation on a small preview and deactivates grain, halation, blurs, unsharp mask (diffusion filters are active)")
                .clicked()
            {
                self.dirty = true;
                self.force_preview = true;
                self.full_scan_requested = false;
            }
            if ui.add_enabled(ready, egui::Button::new("SCAN")).on_hover_text("Run the full simulation on the full-resolution input").clicked() {
                self.dirty = true;
                self.force_preview = true;
                self.full_scan_requested = true;
            }
            if ui
                .add_enabled(
                    self.output_image.is_some() && !busy,
                    egui::Button::new("SAVE"),
                )
                .on_hover_text("Save the current output layer to an image file")
                .clicked()
            {
                self.save_dialog();
            }
        });
    }

    fn file_dialog(&self, key: &str) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match self.gui_state.sections["rust"]["dialog_dirs"][key].as_str() {
            Some(path) => dialog.set_directory(path),
            None => dialog,
        }
    }
    fn remember_dialog(&mut self, key: &str, path: &Path) {
        if let Some(parent) = path.parent() {
            if !self.gui_state.sections["rust"].is_object() {
                self.gui_state.sections["rust"] = serde_json::json!({"version":1,"dialog_dirs":{}});
            }
            if !self.gui_state.sections["rust"]["dialog_dirs"].is_object() {
                self.gui_state.sections["rust"]["dialog_dirs"] = serde_json::json!({});
            }
            self.gui_state.sections["rust"]["dialog_dirs"][key] =
                serde_json::json!(parent.to_string_lossy());
            let path = state::config_dir().join("dialog_dirs.json");
            if let Err(e) = std::fs::create_dir_all(state::config_dir()).and_then(|_| {
                std::fs::write(
                    path,
                    self.gui_state.sections["rust"]["dialog_dirs"].to_string(),
                )
            }) {
                self.status = format!("Dialog directory persistence error: {e}");
            }
        }
    }

    fn digested_params(&self, preview: bool) -> Result<RuntimeParams> {
        let mut params = self.current_state()?.runtime_params()?;
        params.settings.preview_mode = preview;
        let film = profile::load_profile_by_name(&self.data_dir, &self.film_name)?;
        let print = profile::load_profile_by_name(&self.data_dir, &self.print_name)?;
        let photo = RuntimePhotoParams {
            film,
            print,
            params,
            data_dir: self.data_dir.clone(),
        };
        photo
            .digested_params(DigestMode::PreserveUserEdits)
            .map_err(anyhow::Error::msg)
    }

    fn sync_profile_defaults(&mut self) {
        let result = (|| -> Result<()> {
            let params = self.current_state()?.runtime_params()?;
            let film = profile::load_profile_by_name(&self.data_dir, &self.film_name)?;
            let scan_film = film.is_positive();
            let print = profile::load_profile_by_name(&self.data_dir, &self.print_name)?;
            let photo = RuntimePhotoParams {
                film,
                print,
                params,
                data_dir: self.data_dir.clone(),
            };
            self.params = photo
                .digested_params(DigestMode::ApplyStockSpecifics)
                .map_err(anyhow::Error::msg)?;
            self.params.io.scan_film = scan_film;
            self.scan_for_print_snapshot = None;
            self.pipeline_cache_key = None;
            self.pipeline_cache = None;
            Ok(())
        })();
        if let Err(e) = result {
            self.status = format!("Profile selection error: {e:#}");
        }
    }

    fn load_image_from_path(&mut self, path: &Path) {
        let t = Instant::now();
        let raw = image_io::is_raw(path);
        let mut raw_lens_info = None;
        let loaded = if raw {
            let settings = &self.gui_state.sections["load_raw"];
            let options = spektrafilm_raw::RawOptions {
                white_balance: match settings["white_balance"].as_str().unwrap_or("as_shot") {
                    "daylight" => spektrafilm_raw::WhiteBalance::Daylight,
                    "tungsten" => spektrafilm_raw::WhiteBalance::Tungsten,
                    "custom" => spektrafilm_raw::WhiteBalance::Custom,
                    _ => spektrafilm_raw::WhiteBalance::AsShot,
                },
                temperature: settings["temperature"].as_f64(),
                tint: settings["tint"].as_f64(),
                lens_correction: settings["lens_correction"].as_bool().unwrap_or(false),
            };
            spektrafilm_raw::load(path, &options)
                .map(|result| {
                    raw_lens_info = Some(result.lens_info);
                    LoadedImage {
                        image: result.image,
                        metadata: image_io::read_metadata(path),
                    }
                })
                .map_err(anyhow::Error::msg)
        } else {
            image_io::load(path).map_err(anyhow::Error::from)
        };
        match loaded {
            Ok(LoadedImage {
                image: img,
                metadata,
            }) => {
                if raw {
                    self.params.io.input_color_space = "ACES2065-1".into();
                    self.params.io.input_cctf_decoding = false;
                }
                self.source_metadata = metadata;
                self.input_rotation = 0;
                self.status = format!(
                    "Loaded {} × {} ({:.1} MP) in {:.0} ms",
                    img.width,
                    img.height,
                    (img.pixel_count() as f64 / 1e6),
                    t.elapsed().as_secs_f32() * 1000.0
                );
                self.raw_lens_info = raw_lens_info;
                if raw {
                    match self.raw_lens_info.as_deref() {
                        Some(info) if !info.is_empty() => self
                            .status
                            .push_str(&format!("; Lens correction applied ({info})")),
                        _ => self.status.push_str("; Lens correction not applied"),
                    }
                }
                self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
                self.input_epoch = self.input_epoch.wrapping_add(1);
                self.full_scan_requested = false;
                self.image = Some(Arc::new(img));
                match display::input_display_raster(
                    self.image.as_ref().unwrap(),
                    &self.params.io.input_color_space,
                    self.params.io.input_cctf_decoding,
                    self.params.settings.preview_max_size as usize,
                ) {
                    Ok(raster) => {
                        self.viewer.set_input(
                            raster,
                            [
                                self.image.as_ref().unwrap().width as usize,
                                self.image.as_ref().unwrap().height as usize,
                            ],
                        );
                        self.viewer.set_input_display_source(
                            &self.params.io.input_color_space,
                            self.params.io.input_cctf_decoding,
                        );
                    }
                    Err(e) => self.status = format!("Viewer input error: {e}"),
                }
                self.image_path = Some(path.to_path_buf());
                self.output_image = None;
                self.output_metadata = None;
                self.dirty = true;
            }
            Err(e) => {
                self.status = format!("Load error: {e:#}");
            }
        }
    }

    fn rotate_input_image(&mut self, clockwise: bool) {
        let Some(image) = self.image.as_ref() else {
            self.status = "Load an image before rotating.".into();
            return;
        };
        let quarter_turns = if clockwise { -1 } else { 1 };
        let rotated = image.rotated_quarter_turns(quarter_turns);
        self.image = Some(Arc::new(rotated));
        self.input_rotation = (self.input_rotation + quarter_turns).rem_euclid(4);
        self.input_epoch = self.input_epoch.wrapping_add(1);
        self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
        self.full_scan_requested = false;
        self.output_image = None;
        self.output_metadata = None;
        if let Some(image) = self.image.as_ref() {
            match display::input_display_raster(
                image,
                &self.params.io.input_color_space,
                self.params.io.input_cctf_decoding,
                self.params.settings.preview_max_size as usize,
            ) {
                Ok(raster) => {
                    self.viewer
                        .set_input(raster, [image.width as usize, image.height as usize]);
                    self.viewer.set_input_display_source(
                        &self.params.io.input_color_space,
                        self.params.io.input_cctf_decoding,
                    );
                }
                Err(e) => self.status = format!("Viewer input error: {e}"),
            }
        }
        self.dirty = true;
        self.force_preview = true;
        self.status = format!("Rotated input {}°", self.input_rotation as i32 * 90);
    }

    fn rotate_input_image_clockwise(&mut self) {
        self.rotate_input_image(true);
    }

    fn rotate_input_image_counterclockwise(&mut self) {
        self.rotate_input_image(false);
    }

    fn preview_pipeline(
        &mut self,
        film_name: &str,
        print_name: &str,
        params: &RuntimeParams,
    ) -> Result<(Runtime, f32), String> {
        let t = Instant::now();
        let key = format!(
            "{}|{}",
            preview_pipeline_cache_key(film_name, print_name, params),
            self.gui_state.sections["special"]
        );
        if self.pipeline_cache_key.as_deref() == Some(key.as_str())
            && let Some(runtime) = self.pipeline_cache.as_ref()
        {
            return Ok((
                runtime.clone().with_params(params.clone())?,
                t.elapsed().as_secs_f32() * 1000.0,
            ));
        }

        let mut film = profile::load_profile_by_name(&self.data_dir, film_name)
            .map_err(|e| format!("film profile '{film_name}': {e}"))?;
        let effective_print_name = if params.io.scan_film {
            film_name
        } else {
            print_name
        };
        let mut print = profile::load_profile_by_name(&self.data_dir, effective_print_name)
            .map_err(|e| format!("print profile '{effective_print_name}': {e}"))?;
        for (profile, key) in [
            (&mut film, "film_channel_swap"),
            (&mut print, "print_channel_swap"),
        ] {
            if let Some(order) = self.gui_state.sections["special"][key].as_array() {
                for row in &mut profile.data.channel_density {
                    if row.len() >= 3 {
                        let original = [row[0], row[1], row[2]];
                        for ch in 0..3 {
                            row[ch] = original[order[ch].as_u64().unwrap_or(ch as u64) as usize];
                        }
                    }
                }
            }
        }
        let runtime = Runtime::new(film, print, params.clone(), &self.data_dir)
            .map_err(|e| format!("runtime build: {e}"))?;
        self.pipeline_cache_key = Some(key);
        self.pipeline_cache = Some(runtime.clone());
        Ok((runtime, t.elapsed().as_secs_f32() * 1000.0))
    }

    /// Spawn a render on a worker thread. The UI stays interactive
    /// during the 250–500 ms pipeline run (previously this blocked
    /// the main thread, dropping mid-drag slider events). If a job
    /// is already in flight, mark `pending_dirty` so the latest
    /// params get re-rendered as soon as the in-flight one returns.
    fn dispatch_render(&mut self, ctx: &egui::Context) {
        if self.render_job.is_some() {
            self.pending_dirty = true;
            return;
        }
        let t_clone = Instant::now();
        let Some(image) = self.image.as_ref().cloned() else {
            return;
        };
        let input_clone_ms = t_clone.elapsed().as_secs_f32() * 1000.0;
        let film_name = self.film_name.clone();
        let print_name = self.print_name.clone();
        let kind = if self.full_scan_requested {
            RenderKind::Scan
        } else {
            RenderKind::Preview
        };
        let input_epoch = self.input_epoch;
        let backend_name = self.backend.name().to_owned();
        let params = match self.digested_params(kind == RenderKind::Preview) {
            Ok(params) => params,
            Err(e) => {
                self.status = format!("{} state error: {e:#}", kind.label());
                return;
            }
        };
        let (pipeline_template, pipeline_build_ms) =
            match self.preview_pipeline(&film_name, &print_name, &params) {
                Ok(p) => p,
                Err(msg) => {
                    eprintln!("[spektrafilm] render error: {msg}");
                    self.status = format!("Render error: {msg}");
                    return;
                }
            };
        let backend = self.backend.clone();
        let source_metadata = self.source_metadata.clone();
        let (tx, rx) = mpsc::channel();
        let ctx_for_worker = ctx.clone();
        let display_enabled = self.viewer.settings.use_display_transform;
        let display_profile = self.gui_state.sections["rust"]["display_profile"]
            .as_str()
            .map(PathBuf::from);
        let handle = std::thread::Builder::new()
            .name("spektrafilm-render".into())
            .spawn(move || {
                // A panic inside the pipeline must still produce a channel
                // message — otherwise the receiver only sees a disconnect
                // and the user gets a uselessly vague status line.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let t_total = Instant::now();
                    let t_scale = Instant::now();
                    let working_image = if params.settings.preview_mode {
                        spektrafilm_core::params_builder::resize_for_preview(
                            &image,
                            params.settings.preview_max_size,
                        )
                    } else {
                        (*image).clone()
                    };
                    let scale_ms = t_scale.elapsed().as_secs_f32() * 1000.0;
                    let t = Instant::now();
                    let output = pipeline_template.process(working_image, backend.as_ref())?;
                    let render_ms = t.elapsed().as_secs_f32() * 1000.0;
                    let runtime_params = pipeline_template.params();
                    let t_preview = Instant::now();
                    let (preview, display_status) = display::output_display_raster(
                        &output,
                        &runtime_params.io.output_color_space,
                        runtime_params.io.output_cctf_encoding,
                        display_enabled,
                        display_profile.as_deref(),
                        runtime_params.settings.preview_max_size as usize,
                    )?;
                    let preview_ms = t_preview.elapsed().as_secs_f32() * 1000.0;
                    let worker_total_ms = t_total.elapsed().as_secs_f32() * 1000.0;
                    Ok(RenderResult {
                        source_metadata,
                        output_color_space: runtime_params.io.output_color_space.clone(),
                        output_cctf_encoding: runtime_params.io.output_cctf_encoding,
                        output,
                        preview,
                        display_status,
                        display_enabled,
                        display_profile,
                        display_max_size: runtime_params.settings.preview_max_size,
                        input_clone_ms,
                        scale_ms,
                        pipeline_build_ms,
                        render_ms,
                        preview_ms,
                        worker_total_ms,
                    })
                }))
                .unwrap_or_else(|panic| Err(panic_message(&panic)));
                let _ = tx.send(result);
                ctx_for_worker.request_repaint();
            })
            .expect("OS thread spawn");
        self.render_job = Some(RenderJob {
            input_epoch,
            kind,
            backend_name,
            rx,
            handle: Some(handle),
        });
        self.full_scan_requested = false;
    }

    /// Called once per `update()`. If the in-flight render finished,
    /// upload the texture and unblock the next pass. If more changes
    /// arrived during the render, re-arm `dirty`.
    fn poll_render_job(&mut self, ctx: &egui::Context) {
        let Some(job) = self.render_job.as_mut() else {
            return;
        };
        let result = match job.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.render_job = None;
                self.status = "Render error: worker thread vanished".into();
                return;
            }
        };
        if let Some(h) = job.handle.take() {
            let _ = h.join();
        }
        let current_input = job.input_epoch == self.input_epoch;
        let kind = job.kind;
        let backend_name = job.backend_name.clone();
        self.render_job = None;
        if !current_input {
            self.pending_dirty = false;
            self.dirty = true;
            return;
        }
        match result {
            Ok(mut r) => {
                let display_profile = self.gui_state.sections["rust"]["display_profile"]
                    .as_str()
                    .map(Path::new);
                if r.display_enabled != self.viewer.settings.use_display_transform
                    || r.display_profile.as_deref() != display_profile
                    || r.display_max_size != self.params.settings.preview_max_size
                {
                    match display::output_display_raster(
                        &r.output,
                        &r.output_color_space,
                        r.output_cctf_encoding,
                        self.viewer.settings.use_display_transform,
                        display_profile,
                        self.params.settings.preview_max_size as usize,
                    ) {
                        Ok((preview, status)) => {
                            r.preview = preview;
                            r.display_status = status;
                        }
                        Err(error) => {
                            self.status = format!("Viewer display error: {error}");
                            return;
                        }
                    }
                }
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
                self.output_image = Some(r.output);
            }
            Err(msg) => {
                eprintln!("[spektrafilm] render error: {msg}");
                self.status = format!("Render error: {msg}");
            }
        }
        if self.pending_dirty {
            self.pending_dirty = false;
            self.dirty = true;
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
            .add_filter("Image", &["jpg", "jpeg", "png", "tif", "tiff", "exr"])
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        self.remember_dialog("save_output", &path);
        let actual_format = match image_io::ImageFormat::detect(&path) {
            Ok(format) => format,
            Err(error) => {
                self.status = format!("Save error: {error}");
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
                return;
            }
        };
        match image_io::save_rendered_output(
            &path,
            &converted,
            SaveOptions {
                depth: match actual_format {
                    image_io::ImageFormat::Jpeg | image_io::ImageFormat::Png => BitDepth::Eight,
                    image_io::ImageFormat::Tiff | image_io::ImageFormat::Exr => BitDepth::Sixteen,
                },
                color_space: destination,
                cctf_encoding: encoded,
                jpeg_quality: None,
                jpeg_subsampling: None,
                compression: None,
            },
            self.output_metadata.as_ref(),
        ) {
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
            }
            Err(e) => {
                self.status = format!("Save error: {e:#}");
            }
        }
    }

    /// Render an immutable snapshot of the loaded full input independently of Save.
    fn start_export(&mut self, ctx: &egui::Context, mut options: ExportOptions) {
        if self.render_job.is_some() || self.export_job.is_some() || self.calibration_job.is_some()
        {
            self.status = "Wait for the current operation before exporting.".into();
            return;
        }
        let Some(image) = self.image.as_ref().cloned() else {
            self.status = "Load an image before exporting.".into();
            return;
        };
        options.normalize();
        let stem = self
            .image_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("spektrafilm");
        let default_name = format!(
            "{stem}_{}_spektra_{}.{}",
            self.film_name,
            options.backend.argument(),
            options.format.extension()
        );
        let Some(chosen_path) = self
            .file_dialog("export")
            .add_filter(options.format.label(), options.format.extensions())
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        let out_path = options.format.output_path(&chosen_path);
        if out_path != chosen_path && out_path.exists() {
            self.status = format!(
                "Export destination already exists: {}. Choose that exact filename to confirm replacement.",
                out_path.display()
            );
            return;
        }
        let params = match self.digested_params(false) {
            Ok(params) => params,
            Err(e) => {
                self.status = format!("Export state error: {e:#}");
                return;
            }
        };
        let film = self.film_name.clone();
        let paper = self.print_name.clone();
        let pipeline = match self.preview_pipeline(&film, &paper, &params) {
            Ok((pipeline, _)) => pipeline,
            Err(e) => {
                self.status = format!("Export pipeline error: {e}");
                return;
            }
        };
        self.remember_dialog("export", &out_path);
        self.export_options = options.clone();
        let metadata = self.source_metadata.clone();
        let export_backend = options.backend;
        let selected_backend = self.backend.clone();
        let (tx, rx) = mpsc::channel();
        let ctx_for_worker = ctx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_worker = Arc::clone(&cancel);
        let started_at = Instant::now();
        let handle = std::thread::Builder::new()
            .name("spektrafilm-export".into())
            .spawn(move || {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<ExportResult> {
                        if cancel_for_worker.load(Ordering::SeqCst) {
                            anyhow::bail!("cancelled");
                        }
                        let backend: Arc<dyn ComputeBackend> = match export_backend {
                            ExportBackend::Cpu => {
                                Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend)
                            }
                            ExportBackend::Gpu if selected_backend.is_gpu() => selected_backend,
                            ExportBackend::Gpu => Arc::new(
                                spektrafilm_gpu::wgpu_backend::WgpuBackend::new().context(
                                    "GPU export requested but no WGPU adapter is available",
                                )?,
                            ),
                        };
                        let output = pipeline
                            .process((*image).clone(), backend.as_ref())
                            .map_err(anyhow::Error::msg)?;
                        if cancel_for_worker.load(Ordering::SeqCst) {
                            anyhow::bail!("cancelled");
                        }
                        let converted = image_io::convert_image(
                            &output,
                            &pipeline.params().io.output_color_space,
                            pipeline.params().io.output_cctf_encoding,
                            &options.saving_color_space,
                            options.saving_cctf_encoding,
                        )?;
                        let nanos = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?
                            .as_nanos();
                        let staged = TempPath(
                            out_path.with_file_name(format!(
                                "spektrafilm-export-{}-{nanos}.{}",
                                std::process::id(),
                                options.format.extension()
                            )),
                            Some(out_path.clone()),
                        );
                        let report = image_io::save(
                            &staged.0,
                            &converted,
                            SaveOptions {
                                depth: options.depth,
                                color_space: &options.saving_color_space,
                                cctf_encoding: options.saving_cctf_encoding,
                                jpeg_quality: (options.format == ExportFormat::Jpeg)
                                    .then_some(options.jpeg_quality),
                                jpeg_subsampling: (options.format == ExportFormat::Jpeg)
                                    .then_some(options.jpeg_subsampling),
                                compression: matches!(
                                    options.format,
                                    ExportFormat::Tiff | ExportFormat::Exr
                                )
                                .then_some(
                                    if options.compression == ExportCompression::Zip {
                                        Compression::Zip
                                    } else {
                                        Compression::None
                                    },
                                ),
                            },
                            metadata.as_ref(),
                        )?;
                        if cancel_for_worker.load(Ordering::SeqCst) {
                            anyhow::bail!("cancelled");
                        }
                        Ok(ExportResult {
                            elapsed: started_at.elapsed().as_secs_f32(),
                            filename: out_path
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("(file)")
                                .to_owned(),
                            backend_name: backend.name().to_owned(),
                            size: [output.width, output.height],
                            metadata_warning: report.metadata_warning,
                            staged,
                        })
                    },
                ))
                .unwrap_or_else(|panic| Err(anyhow::anyhow!(panic_message(&panic))));
                let msg = res.map_err(|e| format!("{e:#}"));
                let _ = tx.send(msg);
                ctx_for_worker.request_repaint();
            })
            .expect("OS thread spawn");
        self.status = format!("Exporting with {}…", export_backend.label());
        self.export_job = Some(ExportJob {
            rx,
            handle: Some(handle),
            cancel,
            started_at,
            backend: export_backend,
        });
    }

    /// Cancellation discards the staged file after the current pipeline operation.
    fn cancel_export(&mut self) {
        let Some(job) = self.export_job.as_ref() else {
            return;
        };
        if !job.cancel.swap(true, Ordering::SeqCst) {
            self.status = "Cancelling export…".into();
        }
    }

    /// Called once per `update()`. If the export is still in-flight,
    /// refreshes the status with elapsed time and requests a repaint a
    /// second from now (so the timer ticks without us busy-looping).
    /// If the job finished, drains the result into the status bar and
    /// joins the worker thread.
    fn poll_export_job(&mut self, ctx: &egui::Context) {
        let Some(job) = self.export_job.as_mut() else {
            return;
        };
        let msg = match job.rx.try_recv() {
            Ok(m) => m,
            Err(mpsc::TryRecvError::Empty) => {
                let secs = job.started_at.elapsed().as_secs();
                self.status = if job.cancel.load(Ordering::SeqCst) {
                    format!("Cancelling export… ({secs}s)")
                } else {
                    format!("Exporting with {}… ({secs}s)", job.backend.label())
                };
                ctx.request_repaint_after(Duration::from_secs(1));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.status = "Export error: worker thread vanished".into();
                self.export_job = None;
                return;
            }
        };
        if let Some(h) = job.handle.take() {
            let _ = h.join();
        }
        let cancelled = job.cancel.load(Ordering::SeqCst);
        self.export_job = None;
        self.status = match msg {
            Ok(_) if cancelled => "Export cancelled.".into(),
            Ok(result) => match result.staged.publish() {
                Ok(()) => {
                    let mut status = format!(
                        "Exported ({}) {} · {} × {} in {:.1} s",
                        result.backend_name,
                        result.filename,
                        result.size[0],
                        result.size[1],
                        result.elapsed
                    );
                    if let Some(warning) = result.metadata_warning {
                        status.push_str(&format!(" — Metadata warning: {warning}"));
                    }
                    status
                }
                Err(e) => format!("Export error: {e:#}"),
            },
            Err(e) if e.contains("cancelled") => "Export cancelled.".into(),
            Err(e) => format!("Export error: {e}"),
        };
    }

    fn start_calibration(&mut self, action: controls::CalibrationAction) {
        if self.calibration_job.is_some() || self.render_job.is_some() || self.export_job.is_some()
        {
            self.status = "Wait for the current operation before running a Convert action.".into();
            return;
        }
        let Some(image) = self.image.as_ref().map(|image| (**image).clone()) else {
            self.status = "Load an input image before running a Convert action.".into();
            return;
        };
        let mut params = match self
            .current_state()
            .and_then(|state| state.runtime_params())
        {
            Ok(params) => params,
            Err(error) => {
                self.status = format!("Calibration state error: {error:#}");
                return;
            }
        };
        let film = match profile::load_profile_by_name(&self.data_dir, &self.film_name) {
            Ok(profile) => profile,
            Err(error) => {
                self.status = format!("Calibration film error: {error:#}");
                return;
            }
        };
        let print = match profile::load_profile_by_name(&self.data_dir, &self.print_name) {
            Ok(profile) => profile,
            Err(error) => {
                self.status = format!("Calibration print error: {error:#}");
                return;
            }
        };
        let data_dir = self.data_dir.clone();
        let backend = Arc::clone(&self.backend);
        let epoch = self.calibration_epoch.wrapping_add(1);
        self.calibration_epoch = epoch;
        let (tx, rx) = mpsc::channel();
        let action_name = match action {
            controls::CalibrationAction::DetectBase => "Detecting film base…",
            controls::CalibrationAction::BlindCalibration => "Fitting blind calibration…",
            controls::CalibrationAction::NeutralizeFilters => "Neutralizing print filters…",
        };
        self.status = action_name.into();
        let handle = std::thread::Builder::new()
            .name("spektrafilm-calibration".into())
            .spawn(move || {
                params.workflow.route = "input > convert-film > scan".into();
                let result = match action {
                    controls::CalibrationAction::DetectBase => {
                        spektrafilm_core::stages::converting::detect_base(&image, &film, &params)
                            .map(|(params, exposure_ev)| CalibrationResult::Base {
                                params,
                                exposure_ev,
                            })
                    }
                    controls::CalibrationAction::BlindCalibration => {
                        spektrafilm_core::stages::converting::blind_calibration(
                            &image, &film, &params,
                        )
                        .map(CalibrationResult::BlindCalibration)
                    }
                    controls::CalibrationAction::NeutralizeFilters => {
                        spektrafilm_core::stages::converting::neutralize_filters(
                            &film,
                            &print,
                            &params,
                            &data_dir,
                            backend.as_ref(),
                        )
                        .map(|(m_shift, y_shift)| {
                            CalibrationResult::NeutralizeFilters { m_shift, y_shift }
                        })
                    }
                };
                let _ = tx.send(result);
            })
            .map_err(|error| error.to_string());
        match handle {
            Ok(handle) => {
                self.calibration_job = Some(CalibrationJob {
                    epoch,
                    rx,
                    handle: Some(handle),
                });
            }
            Err(error) => {
                self.status = format!("Calibration worker error: {error}");
            }
        }
    }

    fn poll_calibration_job(&mut self, ctx: &egui::Context) {
        let Some(job) = self.calibration_job.as_ref() else {
            return;
        };
        let result = match job.rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(IN_FLIGHT_REPAINT);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let mut job = self.calibration_job.take().expect("calibration job exists");
                if let Some(handle) = job.handle.take() {
                    let _ = handle.join();
                }
                self.status = "Calibration worker vanished.".into();
                return;
            }
        };
        let mut job = self.calibration_job.take().expect("calibration job exists");
        let epoch = job.epoch;
        if let Some(handle) = job.handle.take() {
            let _ = handle.join();
        }
        if epoch != self.calibration_epoch {
            self.status = "Discarded stale Convert calibration result.".into();
            return;
        }
        match result {
            Ok(CalibrationResult::Base {
                params,
                exposure_ev,
            }) => {
                self.params.film_render.base = params;
                self.params.film_render.convert.exposure_compensation_ev = exposure_ev;
                self.dirty = true;
                self.force_preview = true;
                self.status =
                    format!("Film base detected; exposure compensation {exposure_ev:+.2} EV.");
            }
            Ok(CalibrationResult::BlindCalibration(calibration)) => {
                self.params.film_render.convert.calibration = calibration;
                self.dirty = true;
                self.force_preview = true;
                self.status = "Blind calibration fitted.".into();
            }
            Ok(CalibrationResult::NeutralizeFilters { m_shift, y_shift }) => {
                self.params.enlarger.m_filter_shift = m_shift;
                self.params.enlarger.y_filter_shift = y_shift;
                self.dirty = true;
                self.force_preview = true;
                self.status =
                    format!("Print filters neutralized: M {m_shift:+.2}, Y {y_shift:+.2}.");
            }
            Err(error) => {
                self.status = format!("Calibration failed: {error}");
            }
        }
        ctx.request_repaint();
    }

    fn parameter_section(&mut self, ui: &mut egui::Ui, section: &str) {
        let changes = controls::show(ui, &mut self.params, &mut self.gui_state.sections, section);
        if changes.runtime_changed {
            self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
            self.dirty = true;
        }
        if let Some(action) = changes.action {
            self.start_calibration(action);
        }
        if changes.preview_requested {
            self.dirty = true;
            self.force_preview = true;
        }
        if changes.raw_reload {
            if let Some(path) = self.image_path.clone() {
                self.load_image_from_path(&path);
            }
        }
    }

    fn import_section(&mut self, ui: &mut egui::Ui, raw: bool) {
        ui.collapsing(if raw { "Import Raw" } else { "Import RGB" }, |ui| {
            if ui.button("select file").on_hover_text(if raw {
                "Load and process a raw file with the selected white balance and lens correction settings."
            } else {
                "Select an input image"
            }).clicked() {
                if let Some(path) = self.file_dialog("load").add_filter("Image", IMAGE_FILE_EXTENSIONS).pick_file() {
                    self.remember_dialog("load", &path);
                    self.load_image_from_path(&path);
                }
            }
            if raw {
                self.parameter_section(ui, "Import Raw");
            }
            if let Some(p) = &self.image_path {
                ui.label(egui::RichText::new(p.file_name().and_then(|s| s.to_str()).unwrap_or("")).small());
            }
            ui.add_space(4.0);
        });
    }

    fn export_actions(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Export options")
            .default_open(false)
            .show(ui, |ui| {
                if self.export_job.is_some() {
                    if ui.button("Cancel")
                        .on_hover_text("Cancel export; the current operation finishes before its output is discarded.")
                        .clicked()
                    {
                        self.cancel_export();
                    }
                } else {
                    let enabled = self.image.is_some()
                        && self.render_job.is_none()
                        && self.calibration_job.is_none()
                        && !self.export_dialog.is_open();
                    if ui.add_enabled(enabled, egui::Button::new("Export…"))
                        .on_hover_text("Choose settings and render the full image independently of Save.")
                        .clicked()
                    {
                        self.export_dialog.open(&self.export_options);
                    }
                }
            });
    }
    fn chemistry_section(&mut self, ui: &mut egui::Ui, film: bool) {
        ui.collapsing("Chemistry", |ui| {
            let (times, selected) = if film {
                (
                    &self.film_dev_times,
                    &mut self.params.film_render.development_time,
                )
            } else {
                (
                    &self.print_dev_times,
                    &mut self.params.print_render.development_time,
                )
            };
            if dev_time_combo(
                ui,
                if film { "film-time" } else { "print-time" },
                "development time",
                times,
                selected,
            ) {
                self.dirty = true;
            }
            self.parameter_section(
                ui,
                if film {
                    "Film chemistry"
                } else {
                    "Print chemistry"
                },
            );
        });
    }

    fn controls_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let input_view_before = (
            self.params.io.input_color_space.clone(),
            self.params.io.input_cctf_decoding,
            self.params.settings.preview_max_size,
        );
        match self.gui_tab {
            GuiTab::Main => {
                self.import_section(ui, false);
                self.import_section(ui, true);
                for section in ["Crop and upscale", "Input", "Camera"] {
                    self.parameter_section(ui, section);
                }
                ui.collapsing("Profiles", |ui| {
                    if profile_combo(ui, "film", "film profile", &self.films, &mut self.film_name) {
                        self.params.film_render.development_time = None;
                        self.film_dev_times = profile_dev_times(&self.data_dir, &self.film_name);
                        self.sync_profile_defaults();
                        self.dirty = true;
                    }
                    if profile_combo(
                        ui,
                        "paper",
                        "print profile",
                        &self.papers,
                        &mut self.print_name,
                    ) {
                        self.params.print_render.development_time = None;
                        self.print_dev_times = profile_dev_times(&self.data_dir, &self.print_name);
                        self.sync_profile_defaults();
                        self.dirty = true;
                    }
                });
                for section in ["Enlarger", "Scanner"] {
                    self.parameter_section(ui, section);
                }
                egui::CollapsingHeader::new("Output")
                    .default_open(true)
                    .show(ui, |ui| {
                        self.parameter_section(ui, "Output");
                        self.export_actions(ui);
                    });
            }
            GuiTab::Film => {
                self.chemistry_section(ui, true);
                for section in [
                    "Film base",
                    "Halation",
                    "Couplers",
                    "Grain",
                    "Camera diffusion",
                    "Convert",
                ] {
                    self.parameter_section(ui, section);
                }
            }
            GuiTab::Print => {
                self.chemistry_section(ui, false);
                for section in ["Print base", "Preflash", "Glare", "Enlarger diffusion"] {
                    self.parameter_section(ui, section);
                }
            }
            GuiTab::Advanced => {
                for section in [
                    "Spectral upsampling",
                    "Input gamut compress",
                    "Output gamut compress",
                    "Experimental",
                ] {
                    self.parameter_section(ui, section);
                }
            }
            GuiTab::Config => {
                ui.collapsing("GUI parameters", |ui| {
                    self.state_toolbar(ui);
                });
                ui.collapsing("Display", |ui| {
                    let display_transform_before = self.viewer.settings.use_display_transform;
                    self.viewer.controls(ui);
                    self.parameter_section(ui, "Display");
                    self.viewer.interpolation_control(ui);
                    if display_transform_before != self.viewer.settings.use_display_transform {
                        self.refresh_viewing_artifacts();
                    }
                });
                ui.collapsing("napari layers", |ui| {
                    self.viewer.layer_controls(ui);
                });
            }
        }
        ui.label(egui::RichText::new(&self.status).small());
        if input_view_before
            != (
                self.params.io.input_color_space.clone(),
                self.params.io.input_cctf_decoding,
                self.params.settings.preview_max_size,
            )
        {
            self.refresh_viewing_artifacts();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // First-paint hook: tag the wgpu Metal layer's colorspace as sRGB.
        // Has to happen here (not in `App::new`) because the CAMetalLayer
        // isn't wired up at construction time.
        #[cfg(target_os = "macos")]
        if !self.metal_colorspace_tagged {
            match tag_metal_layer_srgb(frame) {
                Ok(()) => {
                    eprintln!("[spektrafilm] CAMetalLayer.colorspace = sRGB — tagged OK");
                    self.metal_colorspace_tagged = true;
                }
                Err(e) => {
                    eprintln!("[spektrafilm] colorspace tag attempt: {e}");
                }
            }
        }
        let _ = frame;

        if self.dirty
            && self.image.is_some()
            && self.export_job.is_none()
            && self.calibration_job.is_none()
            && !self.export_dialog.is_open()
            && (self.gui_state.auto_preview() || self.force_preview)
        {
            let now = Instant::now();
            let dirty_since = *self.dirty_since.get_or_insert(now);
            if self.render_job.is_some() {
                self.pending_dirty = true;
                self.dirty = false;
                self.dirty_since = None;
            } else {
                let elapsed = now.saturating_duration_since(dirty_since);
                if elapsed >= PREVIEW_DEBOUNCE {
                    self.dispatch_render(ctx);
                    self.force_preview = false;
                    self.dirty = false;
                    self.dirty_since = None;
                } else {
                    ctx.request_repaint_after(PREVIEW_DEBOUNCE - elapsed);
                }
            }
        }
        if self.render_job.is_some() {
            ctx.request_repaint_after(IN_FLIGHT_REPAINT);
        }
        self.poll_render_job(ctx);
        self.poll_calibration_job(ctx);
        self.poll_export_job(ctx);
        if let Some(options) = self.export_dialog.show(ctx) {
            self.start_export(ctx, options);
        }
        egui::SidePanel::right("controls")
            .resizable(false)
            .exact_width(420.0)
            .show(ctx, |ui| {
                ui.heading("spektrafilm");
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    for tab in GuiTab::ALL {
                        ui.selectable_value(&mut self.gui_tab, tab, tab.label());
                    }
                });
                ui.separator();
                let scroll_height = (ui.available_height() - 95.0).max(1.0);
                ui.allocate_ui(egui::vec2(ui.available_width(), scroll_height), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("controls-scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.controls_panel(ui, ctx));
                });
                ui.separator();
                self.simulation_action_bar(ui);
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::TopBottomPanel::bottom("viewer-footer").show_inside(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("ccw rotate").clicked() {
                        self.rotate_input_image_counterclockwise();
                        ui.ctx().request_repaint();
                    }
                    if ui.button("cw rotate").clicked() {
                        self.rotate_input_image_clockwise();
                        ui.ctx().request_repaint();
                    }
                    for (label, percent) in [("100%", 100.0), ("200%", 200.0), ("400%", 400.0)] {
                        if ui.button(label).clicked() {
                            self.viewer.set_zoom_percent(percent);
                            ui.ctx().request_repaint();
                        }
                    }
                    if ui.button("reset view").clicked() {
                        self.viewer.reset_view();
                        ui.ctx().request_repaint();
                    }
                    let zoom = self.viewer.zoom_percent().map_or_else(
                        || format!("{:.0}% fit", self.viewer.zoom * 100.0),
                        |percent| format!("{percent:.0}%"),
                    );
                    ui.label(format!("{} · zoom {zoom}", self.status));
                });
            });
            self.viewer.layer_controls(ui);
            self.viewer
                .show(ui, self.image.as_deref(), self.output_image.as_ref());
        });

        // Accept drag-and-dropped image files.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            self.load_image_from_path(&path);
        }
    }

    /// Cancel pending export publication and drain its worker before exiting.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let Some(job) = self.export_job.take() else {
            return;
        };
        job.cancel.store(true, Ordering::SeqCst);
        if let Some(h) = job.handle {
            let _ = h.join();
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

/// Scan `<data_dir>/profiles/*.json`, parse each profile's `info`, and
/// bucket the results by `info.support`. Films go into the first vec,
/// papers (and any other print-stage supports) into the second.
/// Each entry carries the filename stem (the unique loader key) plus a
/// human-readable display label.
fn scan_profiles(data_dir: &Path) -> (Vec<ProfileEntry>, Vec<ProfileEntry>) {
    let mut films = Vec::new();
    let mut papers = Vec::new();
    let dir = data_dir.join("profiles");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (films, papers);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_lossy = name.to_string_lossy().to_string();
        let Some(stem) = name_lossy.strip_suffix(".json") else {
            continue;
        };
        let stock = stem.to_string();
        // Cheap probe: just deserialize the file's `info` field. We
        // could skip the rest of the profile but `Profile` already does
        // the right thing — and we pay this once at startup.
        let (display, is_paper) = match profile::load_profile_by_name(data_dir, &stock) {
            Ok(p) => {
                let display = p.info.name.clone().unwrap_or_else(|| stock.clone());
                let is_paper = p.info.support == "paper" || p.info.stage == "printing";
                (display, is_paper)
            }
            Err(_) => (stock.clone(), false),
        };
        let entry = ProfileEntry { stock, display };
        if is_paper {
            papers.push(entry);
        } else {
            films.push(entry);
        }
    }
    films.sort_by(|a, b| a.display.cmp(&b.display));
    papers.sort_by(|a, b| a.display.cmp(&b.display));
    (films, papers)
}

/// Combo box that picks one of `entries` by its `stock` id (the
/// underlying file stem) while showing `display` (the human-readable
/// name) as the label. Falls back to showing the raw stock id if no
/// entry with the current `selected_stock` exists.
/// Development-time family of a profile (empty for colour stocks or on
/// load failure — the picker hides itself for ≤1 entries either way).
fn profile_dev_times(data_dir: &Path, stock: &str) -> Vec<f64> {
    profile::load_profile_by_name(data_dir, stock)
        .map(|p| p.data.development_time)
        .unwrap_or_default()
}

/// Development-time picker for a B&W development-time family. `selection`
/// of `None` means the profile's default (the floor-middle entry, matching
/// upstream's `select_development_time`). Returns true when changed.
fn dev_time_combo(
    ui: &mut egui::Ui,
    salt: &str,
    label: &str,
    times: &[f64],
    selection: &mut Option<f64>,
) -> bool {
    let times = if times.is_empty() { &[1.0][..] } else { times };
    // Same resolution as the render path, so the combo always highlights
    // exactly the entry the pipeline will use.
    let current_idx = profile::development_time_index(times, *selection);
    let mut changed = false;
    let tooltip = "Development time for a BW development-time family: selects the density curve and base+fog to render. '—' uses the representative middle development; ignored for single-curve and color stocks.";
    ui.label(label).on_hover_text(tooltip);
    egui::ComboBox::from_id_salt(salt)
        .selected_text(format!("{} min", times[current_idx]))
        .width(ui.available_width().min(280.0))
        .show_ui(ui, |ui| {
            for (i, t) in times.iter().enumerate() {
                if ui
                    .selectable_label(i == current_idx, format!("{t} min"))
                    .clicked()
                    && i != current_idx
                {
                    *selection = Some(*t);
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    changed
}

fn profile_combo(
    ui: &mut egui::Ui,
    salt: &str,
    label: &str,
    entries: &[ProfileEntry],
    selected_stock: &mut String,
) -> bool {
    let tooltip = if salt == "film" {
        "Film stock to simulate"
    } else {
        "Print stock to simulate"
    };
    ui.label(label).on_hover_text(tooltip);
    let display = entries
        .iter()
        .find(|e| &e.stock == selected_stock)
        .map(|e| e.display.clone())
        .unwrap_or_else(|| selected_stock.clone());
    let prev = selected_stock.clone();
    egui::ComboBox::from_id_salt(salt)
        .selected_text(&display)
        .width(ui.available_width().min(280.0))
        .show_ui(ui, |ui| {
            for entry in entries {
                ui.selectable_value(selected_stock, entry.stock.clone(), &entry.display);
            }
        })
        .response
        .on_hover_text(tooltip);
    prev != *selected_stock
}

fn preview_pipeline_cache_key(film_name: &str, print_name: &str, params: &RuntimeParams) -> String {
    serde_json::json!({
        "film": film_name,
        "print": print_name,
        "film_base": params.film_render.base,
        "print_base": params.print_render.base,
        "film_chemistry": params.film_render.chemistry,
        "film_dev": params.film_render.development_time,
        "print_dev": params.print_render.development_time,
        "scan_film": params.io.scan_film,
        "input_color_space": params.io.input_color_space,
        "input_cctf_decoding": params.io.input_cctf_decoding,
        "input_gamut": params.io.input_gamut_compress,
        "output_color_space": params.io.output_color_space,
        "output_gamut": params.io.output_gamut_compress,
        "settings": {
            "rgb_to_raw_method": params.settings.rgb_to_raw_method,
            "apply_hanatos2025_adaptation_window": params.settings.apply_hanatos2025_adaptation_window,
            "apply_hanatos2025_adaptation_surface": params.settings.apply_hanatos2025_adaptation_surface,
            "spectral_gaussian_blur": params.settings.spectral_gaussian_blur,
            "lut_resolution": params.settings.lut_resolution,
            "neutral_print_filters_from_database": params.settings.neutral_print_filters_from_database,
            "use_cat16": params.settings.use_cat16,
        },
        "camera": {
            "color_filter": params.camera.color_filter,
            "filter_uv": params.camera.filter_uv,
            "filter_ir": params.camera.filter_ir,
        },
        "enlarger": {
            "illuminant": params.enlarger.illuminant,
            "c_filter_neutral": params.enlarger.c_filter_neutral,
            "m_filter_neutral": params.enlarger.m_filter_neutral,
            "y_filter_neutral": params.enlarger.y_filter_neutral,
            "m_filter_shift": params.enlarger.m_filter_shift,
            "y_filter_shift": params.enlarger.y_filter_shift,
            "preflash_exposure": params.enlarger.preflash_exposure,
            "preflash_m_filter_shift": params.enlarger.preflash_m_filter_shift,
            "preflash_y_filter_shift": params.enlarger.preflash_y_filter_shift,
            "normalize_print_exposure": params.enlarger.normalize_print_exposure,
            "print_exposure_compensation": params.enlarger.print_exposure_compensation,
        },
        "exposure_compensation_ev": if params.enlarger.print_exposure_compensation {
            params.camera.exposure_compensation_ev
        } else {
            0.0
        },
    })
    .to_string()
}

struct TempPath(PathBuf, Option<PathBuf>);

impl TempPath {
    fn publish(self) -> Result<()> {
        let destination = self.1.as_ref().context("missing export destination")?;
        std::fs::rename(&self.0, destination)
            .with_context(|| format!("publishing export {}", destination.display()))
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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
