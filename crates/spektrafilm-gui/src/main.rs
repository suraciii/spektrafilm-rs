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
use spektrafilm_core::image_io::{self, BitDepth, ImageMetadata, LoadedImage, SaveOptions};
use spektrafilm_core::params::{GrainEngine, GrainV2Mode, RuntimeParams};
use spektrafilm_core::pipeline::Pipeline;
use spektrafilm_core::profile;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
mod state;
mod controls;
mod display;

const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);
const IN_FLIGHT_REPAINT: Duration = Duration::from_millis(16);
const IMAGE_FILE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "tif", "tiff", "exr",
    // Keep this list in lockstep with image_io::is_raw.
    "dng", "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef",
    "srw", "x3f", "iiq", "3fr", "crw", "rwl", "mrw", "mef", "kdc", "ari", "bay", "dcr",
    "drf", "erf", "fff", "k25", "mos", "ptx",
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
                if let Some(path) = args.next() { initial_state = Some(PathBuf::from(path)); }
                else { eprintln!("--state requires a GUI state JSON path"); std::process::exit(2); }
            }
            "--help" | "-h" => { println!("Usage: spektrafilm-gui [IMAGE] [--state GUI_STATE.json]"); return Ok(()); }
            _ if arg.starts_with('-') => { eprintln!("Unknown GUI option: {arg}"); std::process::exit(2); }
            _ => { initial_image = Some(PathBuf::from(arg)); }
        }
    }
    let backend = spektrafilm_gpu::select_backend();
    let backend: Arc<dyn ComputeBackend> = Arc::from(backend);

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
        Box::new(|cc| Ok(Box::new(App::new(cc, backend, initial_image, initial_state)))),
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
            let adjacent = std::env::current_exe().ok().and_then(|exe|exe.parent().map(|dir|dir.join(f64_cli_name()))).filter(|path|path.is_file());
            if std::env::var_os("SPEKTRAFILM_F64_CLI").is_none() { std::env::set_var("SPEKTRAFILM_F64_CLI", adjacent.as_ref().unwrap_or(&f64_cli)); }
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
    const ALL: [Self; 5] = [Self::Main, Self::Film, Self::Print, Self::Advanced, Self::Config];

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
    /// Number of quarter turns applied to the in-memory input relative to the
    /// file on disk. Export uses the rotated buffer when this is non-zero.
    input_rotation: i32,
    source_metadata: Option<ImageMetadata>,
    save_depth: BitDepth,
    export_backend: ExportBackend,
    /// Last rendered pipeline output (post sRGB encode + clip). Retained
    /// so the Save button can write it without re-running the pipeline.
    output_image: Option<ImageBuf>,
    viewer: display::Viewer,
    gui_tab: GuiTab,
    output_color_space: String,
    output_cctf_encoding: bool,
    pipeline_cache_key: Option<String>,
    pipeline_cache: Option<Pipeline>,
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
    /// In-flight export job. `Some` while the subprocess is
    /// running; the `update()` loop polls the receiver each frame and
    /// surfaces success/failure in `status` when the worker thread
    /// completes. Joined eagerly to release the thread.
    export_job: Option<ExportJob>,
    /// In-flight Convert controller action. The epoch drops results made stale
    /// by later parameter edits or a newer action.
    calibration_job: Option<CalibrationJob>,
    calibration_epoch: u64,
}


/// One in-flight preview render. The worker owns a Pipeline + the
/// ImageBuf clone and, when it finishes, sends back the output buffer
/// plus the two timings the status bar shows.
struct RenderJob {
    rx: mpsc::Receiver<Result<RenderResult, String>>,
    handle: Option<JoinHandle<()>>,
}

struct RenderResult {
    output: ImageBuf,
    preview: display::DisplayRaster,
    display_status: String,
    output_color_space: String,
    output_cctf_encoding: bool,
    input_clone_ms: f32,
    scale_ms: f32,
    pipeline_build_ms: f32,
    render_ms: f32,
    preview_ms: f32,
    worker_total_ms: f32,
}


#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ExportBackend {
    #[default]
    Cpu,
    Gpu,
}

impl ExportBackend {
    fn from_state(state: &serde_json::Value) -> Self {
        match state["rust"]["export_backend"].as_str() {
            Some("gpu") => Self::Gpu,
            _ => Self::Cpu,
        }
    }

    fn argument(self) -> &'static str {
        match self { Self::Cpu => "cpu", Self::Gpu => "gpu" }
    }

    fn label(self) -> &'static str {
        match self { Self::Cpu => "CPU (f64)", Self::Gpu => "GPU (WGPU f32)" }
    }
}

/// One in-flight export. The worker thread owns the child
/// process and polls `cancel` in its wait loop. On completion the
/// worker sends the staged image with `Ok(elapsed_seconds, output_filename)`
/// or `Err(msg)`. The UI publishes it only if cancellation was not requested.
/// The join handle is held so we can `join()` after consuming the
/// message and on `on_exit` to drain the worker before the process dies.
struct ExportJob {
    rx: mpsc::Receiver<Result<(f32, String, TempPath), String>>,
    handle: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    started_at: Instant,
    backend: ExportBackend,
}

enum CalibrationResult {
    Base {
        params: spektrafilm_core::params::FilmBaseParams,
        exposure_ev: f64,
    },
    BlindCalibration(String),
    NeutralizeFilters { m_shift: f32, y_shift: f32 },
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
        let startup_error = startup.as_ref().err().map(|e| format!("Startup state error: {e:#}"));
        let gui_state = startup.unwrap_or_else(|_| state::GuiState::factory());
        let film_name = gui_state.film().to_owned();
        let print_name = gui_state.paper().to_owned();
        let film_profile = profile::load_profile_by_name(&data_dir, &film_name).ok();
        let params = gui_state.runtime_params().expect("validated GUI factory state");
        let film_dev_times = film_profile
            .as_ref()
            .map(|f| f.data.development_time.clone())
            .unwrap_or_default();
        let print_dev_times = profile_dev_times(&data_dir, &print_name);
        let save_depth = match gui_state.sections["rust"]["save_bit_depth"].as_u64() { Some(8)=>BitDepth::Eight,Some(32)=>BitDepth::ThirtyTwo,_=>BitDepth::Sixteen };
        let export_backend = ExportBackend::from_state(&gui_state.sections);

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
            save_depth,
            export_backend,
            output_image: None,
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
            status: startup_error.unwrap_or_else(|| String::from("Load an image to start.")),
            dirty: false,
            #[cfg(target_os = "macos")]
            metal_colorspace_tagged: false,
            export_job: None,
            calibration_job: None,
            calibration_epoch: 0,
        };
        app.viewer.settings = display::DisplaySettings::from_json(&app.gui_state.sections["display"]);
        if let Some(p) = initial_image {
            app.load_image_from_path(&p);
        }
        app.viewer.restore_state(&app.gui_state.sections["rust"]["viewer"]);
        app
    }

    fn current_state(&self) -> Result<state::GuiState> {
        let mut extras = self.gui_state.sections.clone();
        let display = self.viewer.settings.to_json();
        for key in ["use_display_transform","gray_18_canvas","white_padding","output_interpolation"] { extras["display"][key] = display[key].clone(); }
        if !extras["rust"].is_object() { extras["rust"] = serde_json::json!({"version":1}); }
        extras["rust"]["viewer"] = self.viewer.persistent_state();
        extras["rust"]["save_bit_depth"] = serde_json::json!(self.save_depth.bits());
        extras["rust"]["export_backend"] = serde_json::json!(self.export_backend.argument());
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
        self.save_depth = match state.sections["rust"]["save_bit_depth"].as_u64() { Some(8)=>BitDepth::Eight,Some(32)=>BitDepth::ThirtyTwo,_=>BitDepth::Sixteen };
        self.export_backend = ExportBackend::from_state(&state.sections);
        self.gui_state = state;
        self.scan_for_print_snapshot = None;
        self.raw_lens_info = None;
        self.pipeline_cache_key = None;
        self.pipeline_cache = None;
        self.dirty = true;
        self.force_preview = true;
        if let Some(path) = self.image_path.clone() { self.load_image_from_path(&path); }
        self.viewer.restore_state(&self.gui_state.sections["rust"]["viewer"]);
        Ok(())
    }

    fn refresh_viewing_artifacts(&mut self) {
        if let Some(image) = self.image.as_ref() {
            match display::input_display_raster(image,&self.params.io.input_color_space,self.params.io.input_cctf_decoding,self.params.settings.preview_max_size as usize) {
                Ok(raster) => self.viewer.replace_input_display(raster),
                Err(e) => self.status = format!("Viewer input error: {e}"),
            }
        }
        if let Some(output) = self.output_image.as_ref() {
            match display::output_display_raster(output,&self.output_color_space,self.output_cctf_encoding,self.viewer.settings.use_display_transform,self.gui_state.sections["rust"]["display_profile"].as_str().map(Path::new),self.params.settings.preview_max_size as usize) {
                Ok((raster,status)) => { self.viewer.replace_output_display(raster); self.viewer.transform_status=status; }
                Err(e) => self.status = format!("Viewer display error: {e}"),
            }
        }
    }

    fn state_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Save state…").clicked() {
                if let Some(path) = self.file_dialog("state").set_file_name("gui_state.json").add_filter("JSON", &["json"]).save_file() {
                    self.remember_dialog("state", &path);
                    let result = self.current_state().and_then(|s| s.save(&path));
                    self.status = match result { Ok(()) => format!("Saved GUI state to {}",path.display()), Err(e) => format!("State save error: {e:#}") };
                }
            }
            if ui.button("Load state…").clicked() {
                if let Some(path) = self.file_dialog("state").add_filter("JSON", &["json"]).pick_file() {
                    self.remember_dialog("state", &path);
                    let result = state::GuiState::load(&path).and_then(|s|self.apply_state(s));
                    self.status = match result { Ok(()) => format!("Loaded GUI state from {}",path.display()), Err(e) => format!("State load error: {e:#}") };
                }
            }
            if ui.button("Save startup default").clicked() {
                let result = self.current_state().and_then(|s|s.save(&state::default_path()));
                self.status = match result { Ok(()) => "Saved current GUI state as startup default".into(), Err(e) => format!("Startup save error: {e:#}") };
            }
            if ui.button("Restore factory default").clicked() {
                let result = state::reset_factory().and_then(|s|self.apply_state(s));
                self.status = match result { Ok(()) => "Restored factory default GUI state".into(), Err(e) => format!("Factory reset error: {e:#}") };
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
        ui.horizontal(|ui| {
            let scan_label = if self.scan_for_print_snapshot.is_some() { "Scan-for-print: ON" } else { "Scan-for-print" };
            if ui.button(scan_label).clicked() {
                self.toggle_scan_for_print();
            }
            if ui.button("Preview").clicked() {
                self.dirty = true;
                self.force_preview = true;
                self.full_scan_requested = false;
            }
            if ui.button("Scan").clicked() {
                self.dirty = true;
                self.force_preview = true;
                self.full_scan_requested = true;
            }
        });
    }

    fn file_dialog(&self, key: &str) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match self.gui_state.sections["rust"]["dialog_dirs"][key].as_str() {
            Some(path) => dialog.set_directory(path), None => dialog,
        }
    }
    fn remember_dialog(&mut self, key: &str, path: &Path) {
        if let Some(parent) = path.parent() {
            if !self.gui_state.sections["rust"].is_object() { self.gui_state.sections["rust"] = serde_json::json!({"version":1,"dialog_dirs":{}}); }
            if !self.gui_state.sections["rust"]["dialog_dirs"].is_object() { self.gui_state.sections["rust"]["dialog_dirs"] = serde_json::json!({}); }
            self.gui_state.sections["rust"]["dialog_dirs"][key] = serde_json::json!(parent.to_string_lossy());
            let path = state::config_dir().join("dialog_dirs.json");
            if let Err(e) = std::fs::create_dir_all(state::config_dir()).and_then(|_|std::fs::write(path, self.gui_state.sections["rust"]["dialog_dirs"].to_string())) { self.status = format!("Dialog directory persistence error: {e}"); }
        }
    }

    fn digested_params(&self, preview: bool) -> Result<RuntimeParams> {
        let mut params = self.current_state()?.runtime_params()?;
        params.settings.preview_mode = preview;
        let film = profile::load_profile_by_name(&self.data_dir, &self.film_name)?;
        let paper = profile::load_profile_by_name(&self.data_dir, &self.print_name)?;
        let database = spektrafilm_core::neutral_filters::NeutralFilters::load(&self.data_dir)
            .map_err(anyhow::Error::msg)?;
        Ok(spektrafilm_core::params_builder::digest_params(params, &film, &paper, Some(&database), false))
    }

    fn sync_profile_defaults(&mut self) {
        let result = (|| -> Result<()> {
            let params = self.current_state()?.runtime_params()?;
            let film = profile::load_profile_by_name(&self.data_dir,&self.film_name)?;
            let paper = profile::load_profile_by_name(&self.data_dir,&self.print_name)?;
            let database = spektrafilm_core::neutral_filters::NeutralFilters::load(&self.data_dir)
                .map_err(anyhow::Error::msg)?;
            self.params = spektrafilm_core::params_builder::digest_params(params,&film,&paper,Some(&database),true);
            self.params.io.scan_film = film.is_positive();
            self.scan_for_print_snapshot = None;
            self.pipeline_cache_key = None;
            self.pipeline_cache = None;
            Ok(())
        })();
        if let Err(e) = result { self.status = format!("Profile selection error: {e:#}"); }
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
            spektrafilm_raw::load(path, &options).map(|result| {
                raw_lens_info = Some(result.lens_info);
                LoadedImage { image: result.image, metadata: image_io::read_metadata(path) }
            }).map_err(anyhow::Error::msg)
        } else { image_io::load(path).map_err(anyhow::Error::from) };
        match loaded {
            Ok(LoadedImage { image: img, metadata }) => {
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
                        Some(info) if !info.is_empty() => self.status.push_str(&format!("; Lens correction applied ({info})")),
                        _ => self.status.push_str("; Lens correction not applied"),
                    }
                }
                self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
                self.image = Some(Arc::new(img));
                match display::input_display_raster(self.image.as_ref().unwrap(), &self.params.io.input_color_space, self.params.io.input_cctf_decoding,self.params.settings.preview_max_size as usize) {
                    Ok(raster) => self.viewer.set_input(raster,[self.image.as_ref().unwrap().width as usize,self.image.as_ref().unwrap().height as usize]),
                    Err(e) => self.status = format!("Viewer input error: {e}"),
                }
                self.image_path = Some(path.to_path_buf());
                self.output_image = None;
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
        self.output_image = None;
        if let Some(image) = self.image.as_ref() {
            match display::input_display_raster(image, &self.params.io.input_color_space, self.params.io.input_cctf_decoding,self.params.settings.preview_max_size as usize) {
                Ok(raster) => self.viewer.set_input(raster, [image.width as usize, image.height as usize]),
                Err(e) => self.status = format!("Viewer input error: {e}"),
            }
        }
        self.dirty = true;
        self.force_preview = true;
        self.status = format!(
            "Rotated input {}°",
            self.input_rotation as i32 * 90
        );
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
    ) -> Result<(Pipeline, f32), String> {
        let t = Instant::now();
        let key = format!("{}|{}", preview_pipeline_cache_key(film_name, print_name, params), self.gui_state.sections["special"]);
        if self.pipeline_cache_key.as_deref() == Some(key.as_str())
            && let Some(pipeline) = self.pipeline_cache.as_ref()
        {
            return Ok((pipeline.clone().with_params(params.clone())?, t.elapsed().as_secs_f32() * 1000.0));
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
        for (profile, key) in [(&mut film,"film_channel_swap"),(&mut print,"print_channel_swap")] {
            if let Some(order) = self.gui_state.sections["special"][key].as_array() {
                for row in &mut profile.data.channel_density {
                    if row.len() >= 3 {
                        let original = [row[0],row[1],row[2]];
                        for ch in 0..3 { row[ch] = original[order[ch].as_u64().unwrap_or(ch as u64) as usize]; }
                    }
                }
            }
        }
        let pipeline = Pipeline::new_with_spectral(film, print, params.clone(), &self.data_dir)
            .map_err(|e| format!("pipeline build: {e}"))?;
        self.pipeline_cache_key = Some(key);
        self.pipeline_cache = Some(pipeline.clone());
        Ok((pipeline, t.elapsed().as_secs_f32() * 1000.0))
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
        let params = match self.digested_params(!self.full_scan_requested) {
            Ok(params) => params,
            Err(e) => { self.status = format!("Preview state error: {e:#}"); return; }
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
        let (tx, rx) = mpsc::channel();
        let ctx_for_worker = ctx.clone();
        let display_enabled = self.viewer.settings.use_display_transform;
        let display_profile = self.gui_state.sections["rust"]["display_profile"].as_str().map(PathBuf::from);
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
                        spektrafilm_core::params_builder::resize_for_preview(&image, params.settings.preview_max_size)
                    } else { (*image).clone() };
                    let scale_ms = t_scale.elapsed().as_secs_f32() * 1000.0;
                    let pipeline = pipeline_template.with_params(params)?;
                    let t = Instant::now();
                    let output = pipeline.process(working_image, backend.as_ref())?;
                    let render_ms = t.elapsed().as_secs_f32() * 1000.0;
                    let t_preview = Instant::now();
                    let (preview, display_status) = display::output_display_raster(&output,&pipeline.params.io.output_color_space,pipeline.params.io.output_cctf_encoding,display_enabled,display_profile.as_deref(),pipeline.params.settings.preview_max_size as usize)?;
                    let preview_ms = t_preview.elapsed().as_secs_f32() * 1000.0;
                    let worker_total_ms = t_total.elapsed().as_secs_f32() * 1000.0;
                    Ok(RenderResult {
                        output_color_space: pipeline.params.io.output_color_space.clone(),
                        output_cctf_encoding: pipeline.params.io.output_cctf_encoding,
                        output,
                        preview,
                        display_status,
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
        self.render_job = None;
        if self.pending_dirty || self.dirty {
            self.pending_dirty = false;
            self.dirty = true;
            return;
        }
        match result {
            Ok(r) => {
                self.last_input_clone_ms = r.input_clone_ms;
                self.last_scale_ms = r.scale_ms;
                self.last_pipeline_build_ms = r.pipeline_build_ms;
                self.last_render_ms = r.render_ms;
                self.last_preview_ms = r.preview_ms;
                self.last_worker_total_ms = r.worker_total_ms;
                self.output_color_space = r.output_color_space;
                self.output_cctf_encoding = r.output_cctf_encoding;
                self.viewer.transform_status = r.display_status;
                self.viewer.set_output(r.preview,[r.output.width as usize,r.output.height as usize],ctx.input(|i|i.time));
                self.status = format!(
                    "Rendered {} × {} ({:.1} MP)",
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

    /// Open a save dialog and write the most recent rendered output to
    /// disk. Suggested filename is the input stem + the chosen film
    /// stock + the chosen extension; default extension is PNG (8-bit
    /// sRGB-encoded, matching what's on screen).
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
                format!("{stem}_{}_spektra.png", self.film_name)
            }
            None => "spektrafilm.png".into(),
        };
        let Some(path) = self.file_dialog("save_output")
            .add_filter("Image", &["jpg", "jpeg", "png", "tif", "tiff", "exr"])
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        self.remember_dialog("save_output", &path);
        let out = self.output_image.as_ref().expect("output checked before dialog");
        let t = Instant::now();
        let destination = self.gui_state.sections["simulation"]["saving_color_space"].as_str().unwrap_or("sRGB");
        let encoded = self.gui_state.sections["simulation"]["saving_cctf_encoding"].as_bool().unwrap_or(true);
        let converted = match image_io::convert_image(out, &self.output_color_space, self.output_cctf_encoding, destination, encoded) {
            Ok(image) => image,
            Err(error) => { self.status = format!("Save error: {error}"); return; }
        };
        match image_io::save(
            &path,
            &converted,
            SaveOptions {
                depth: self.save_depth,
                color_space: destination,
                cctf_encoding: encoded,
            },
            self.source_metadata.as_ref(),
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
                    self.status.push_str(&format!(" — Metadata warning: {warning}"));
                }
            }
            Err(e) => {
                self.status = format!("Save error: {e:#}");
            }
        }
    }

    /// Re-render at full export resolution using the selected compute backend.
    /// The f64 executable preserves reference CPU arithmetic; WGPU shaders use f32.
    fn export_dialog(&mut self, ctx: &egui::Context) {
        let Some(input_path) = self.image_path.clone() else {
            self.status = "Load an image before exporting.".into();
            return;
        };
        let cli_path = match locate_f64_cli() {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Export: {e}");
                return;
            }
        };
        let stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("spektrafilm");
        let export_backend = self.export_backend;
        let default_name = format!("{stem}_{}_spektra_{}.png", self.film_name, export_backend.argument());
        let Some(out_path) = self.file_dialog("export")
            .add_filter("Image", &["jpg", "jpeg", "png", "tif", "tiff", "exr"])
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        self.remember_dialog("export", &out_path);

        // Spawn the export on a worker thread so the egui event loop
        // keeps drawing. The cancel flag is shared with the worker so
        // the Cancel button (and `on_exit`) can kill the child cleanly
        // instead of letting it orphan after the GUI window closes.
        let film = self.film_name.clone();
        let paper = self.print_name.clone();
        let params = match self.digested_params(false) {
            Ok(params) => params,
            Err(e) => { self.status = format!("Export state error: {e:#}"); return; }
        };
        let data_dir = self.data_dir.clone();
        let save_depth = self.save_depth;
        let export_state = self.gui_state.sections.clone();
        let rotated_input = (self.input_rotation != 0).then(|| (
            Arc::clone(self.image.as_ref().expect("export requires loaded image")),
            self.source_metadata.clone(),
        ));
        let (tx, rx) = mpsc::channel();
        let ctx_for_worker = ctx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_worker = Arc::clone(&cancel);
        let started_at = Instant::now();
        let handle = std::thread::Builder::new()
            .name("spektrafilm-export".into())
            .spawn(move || {
                let res = (|| -> Result<_> {
                    if cancel_for_worker.load(Ordering::SeqCst) { anyhow::bail!("cancelled"); }
                    let rotated_input_guard = if let Some((image, metadata)) = rotated_input {
                        let nanos = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?.as_nanos();
                        let guard = TempPath(std::env::temp_dir().join(format!(
                            "spektrafilm-export-input-{}-{nanos}.tif", std::process::id()
                        )), None);
                        image_io::save(&guard.0, &image, SaveOptions {
                            depth: BitDepth::ThirtyTwo,
                            color_space: &params.io.input_color_space,
                            cctf_encoding: params.io.input_cctf_decoding,
                        }, metadata.as_ref())?;
                        Some(guard)
                    } else { None };
                    if cancel_for_worker.load(Ordering::SeqCst) { anyhow::bail!("cancelled"); }
                    let input_for_export = rotated_input_guard.as_ref()
                        .map(|guard| guard.0.as_path()).unwrap_or(&input_path);
                    run_export(
                    &cli_path,
                    input_for_export,
                    &out_path,
                    &film,
                    &paper,
                    &params,
                    &data_dir,
                    save_depth,
                    &export_state,
                    export_backend,
                    &cancel_for_worker,
                )
                })();
                let name = out_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("(file)")
                    .to_string();
                let msg = match res {
                    Ok(staged) => Ok((started_at.elapsed().as_secs_f32(), name, staged)),
                    Err(e) => Err(format!("{e:#}")),
                };
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

    /// Request cancellation of the in-flight export. The worker thread
    /// observes the flag in its `try_wait` poll loop and SIGKILLs the
    /// child; `poll_export_job` then drains the resulting error message
    /// on the next `update()` tick.
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
        let backend = job.backend;
        self.export_job = None;
        self.status = match msg {
            Ok((_, _, _)) if cancelled => "Export cancelled.".into(),
            Ok((secs, name, staged)) => match staged.publish() {
                Ok(()) => format!("Exported ({}) {name} in {secs:.1} s", backend.label()),
                Err(e) => format!("Export error: {e:#}"),
            },
            Err(e) if e.contains("cancelled") => "Export cancelled.".into(),
            Err(e) => format!("Export error: {e}"),
        };
    }

    fn start_calibration(&mut self, action: controls::CalibrationAction) {
        if self.calibration_job.is_some() {
            self.status = "A Convert calibration action is already running.".into();
            return;
        }
        let Some(image) = self.image.as_ref().map(|image| (**image).clone()) else {
            self.status = "Load an input image before running a Convert action.".into();
            return;
        };
        let mut params = match self.current_state().and_then(|state| state.runtime_params()) {
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
                            .map(|(params, exposure_ev)| CalibrationResult::Base { params, exposure_ev })
                    }
                    controls::CalibrationAction::BlindCalibration => {
                        spektrafilm_core::stages::converting::blind_calibration(&image, &film, &params)
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
                        .map(|(m_shift, y_shift)| CalibrationResult::NeutralizeFilters { m_shift, y_shift })
                    }
                };
                let _ = tx.send(result);
            })
            .map_err(|error| error.to_string());
        match handle {
            Ok(handle) => {
                self.calibration_job = Some(CalibrationJob { epoch, rx, handle: Some(handle) });
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
            Ok(CalibrationResult::Base { params, exposure_ev }) => {
                self.params.film_render.base = params;
                self.params.film_render.convert.exposure_compensation_ev = exposure_ev;
                self.dirty = true;
                self.force_preview = true;
                self.status = format!("Film base detected; exposure compensation {exposure_ev:+.2} EV.");
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
                self.status = format!("Print filters neutralized: M {m_shift:+.2}, Y {y_shift:+.2}.");
            }
            Err(error) => {
                self.status = format!("Calibration failed: {error}");
            }
        }
        ctx.request_repaint();
    }

    fn controls_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let input_view_before = (self.params.io.input_color_space.clone(), self.params.io.input_cctf_decoding, self.params.settings.preview_max_size);
        self.state_toolbar(ui);
        let changes = controls::show(
            ui,
            &mut self.params,
            &mut self.gui_state.sections,
            self.gui_tab.label(),
        );
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
            if let Some(path) = self.image_path.clone() { self.load_image_from_path(&path); }
        }

        if self.gui_tab == GuiTab::Main {
        // ── File ────────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            if ui.button("Open…").clicked() {
                if let Some(path) = self.file_dialog("load")
                    .add_filter(
                        "Image",
                        IMAGE_FILE_EXTENSIONS,
                    )
                    .pick_file()
                {
                    self.remember_dialog("load", &path);
                    self.load_image_from_path(&path);
                }
            }
            let save_enabled = self.output_image.is_some();
            if ui
                .add_enabled(save_enabled, egui::Button::new("Save…"))
                .on_disabled_hover_text("Render an image first")
                .clicked()
            {
                self.save_dialog();
            }
            let export_busy = self.export_job.is_some();
            if export_busy {
                if ui
                    .button("Cancel")
                    .on_hover_text("Stop the in-flight export and kill the child process.")
                    .clicked()
                {
                    self.cancel_export();
                }
            } else {
                let export_enabled = self.image_path.is_some();
                if ui
                    .add_enabled(export_enabled, egui::Button::new("Export…"))
                    .on_hover_text(
                        "Re-render the full image using the selected export backend and write PNG/TIFF/JPEG/EXR.",
                    )
                    .on_disabled_hover_text("Load an image first")
                    .clicked()
                {
                    self.export_dialog(ctx);
                }
            }
        });
        ui.add_enabled_ui(self.export_job.is_none(), |ui| {
            egui::ComboBox::from_label("Export backend")
                .selected_text(self.export_backend.label())
                .show_ui(ui, |ui| {
                    for backend in [ExportBackend::Cpu, ExportBackend::Gpu] {
                        ui.selectable_value(&mut self.export_backend, backend, backend.label());
                    }
                });
            if self.export_backend == ExportBackend::Gpu {
                ui.small("GPU uses f32; unsupported effects run on CPU. A GPU adapter is required.");
            }
        });
        egui::ComboBox::from_label("Save bit depth")
            .selected_text(format!("{} bit", self.save_depth.bits()))
            .show_ui(ui, |ui| {
                for depth in [BitDepth::Eight, BitDepth::Sixteen, BitDepth::ThirtyTwo] {
                    ui.selectable_value(
                        &mut self.save_depth,
                        depth,
                        format!("{} bit", depth.bits()),
                    );
                }
            });
        if let Some(p) = &self.image_path {
            ui.label(
                egui::RichText::new(p.file_name().and_then(|s| s.to_str()).unwrap_or("")).small(),
            );
        }
        ui.add_space(4.0);

        egui::CollapsingHeader::new("Input image")
            .default_open(false)
            .show(ui, |ui| {
                let mut changed = false;
                egui::ComboBox::from_label("Input colour space")
                    .selected_text(self.params.io.input_color_space.clone())
                    .show_ui(ui, |ui| {
                        for opt in [
                            "sRGB",
                            "ProPhoto RGB",
                            "ITU-R BT.2020",
                            "ACES2065-1",
                            "Adobe RGB (1998)",
                            "Display P3",
                            "DCI-P3",
                        ] {
                            changed |= ui
                                .selectable_value(
                                    &mut self.params.io.input_color_space,
                                    opt.to_string(),
                                    opt,
                                )
                                .changed();
                        }
                    });
                changed |= ui
                    .checkbox(
                        &mut self.params.io.input_cctf_decoding,
                        "Decode input transfer function",
                    )
                    .changed();
                if changed {
                    self.dirty = true;
                }
            });

        // ── Profiles ────────────────────────────────────────────────────
        egui::CollapsingHeader::new("Profiles")
            .default_open(true)
            .show(ui, |ui| {
                let film_changed =
                    profile_combo(ui, "film", "Film stock", &self.films, &mut self.film_name);
                if film_changed {
                    self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
                    self.params.film_render.development_time = None;
                    if let Ok(film) = profile::load_profile_by_name(&self.data_dir, &self.film_name)
                    {
                        // Slide/positive stocks have no print paper — they
                        // are scanned directly. Negatives print onto their
                        // paired `target_print`. Auto-follow the film type
                        // so switching stocks doesn't leave a wrong (or, for
                        // a paperless slide, a failed) render.
                        self.params.io.scan_film = film.is_positive();
                        self.film_dev_times = film.data.development_time.clone();
                        if let Some(target) = film.info.target_print.as_deref()
                            && self.papers.iter().any(|p| p.stock == target)
                            && self.print_name != target
                        {
                            self.print_name = target.to_string();
                            self.print_dev_times =
                                profile_dev_times(&self.data_dir, &self.print_name);
                            self.params.print_render.development_time = None;
                        }
                    }
                    self.sync_profile_defaults();
                    self.dirty = true;
                }
                // B&W stocks are profiled at several development times —
                // pick one (longer = more contrast). Hidden for the
                // single-time / colour case.
                if self.film_dev_times.len() > 1
                    && dev_time_combo(
                        ui,
                        "film_dev_time",
                        "Development time",
                        &self.film_dev_times.clone(),
                        &mut self.params.film_render.development_time,
                    )
                {
                    self.dirty = true;
                }
                // Slide films are scanned directly, so the print paper is
                // unused — disable the picker and say why rather than
                // letting it silently affect nothing.
                if self.params.io.scan_film {
                    ui.label(
                        egui::RichText::new("Slide film — scanned directly (no print paper).")
                            .italics()
                            .small(),
                    );
                }
                let paper_changed = ui
                    .add_enabled_ui(!self.params.io.scan_film, |ui| {
                        profile_combo(
                            ui,
                            "paper",
                            "Print paper",
                            &self.papers,
                            &mut self.print_name,
                        )
                    })
                    .inner;
                if paper_changed {
                    self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
                    self.params.print_render.development_time = None;
                    self.sync_profile_defaults();
                    self.dirty = true;
                }
                if !self.params.io.scan_film
                    && self.print_dev_times.len() > 1
                    && dev_time_combo(
                        ui,
                        "print_dev_time",
                        "Print development time",
                        &self.print_dev_times.clone(),
                        &mut self.params.print_render.development_time,
                    )
                {
                    self.dirty = true;
                }
            });

        // ── Exposure ────────────────────────────────────────────────────
        egui::CollapsingHeader::new("Exposure")
            .default_open(true)
            .show(ui, |ui| {
                let mut changed = false;
                changed |= ui
                    .checkbox(&mut self.params.camera.auto_exposure, "Auto exposure")
                    .changed();
                if self.params.camera.auto_exposure {
                    let method = &mut self.params.camera.auto_exposure_method;
                    egui::ComboBox::from_label("Metering")
                        .selected_text(method.clone())
                        .show_ui(ui, |ui| {
                            for opt in [
                                "average",
                                "median",
                                "center_weighted",
                                "partial",
                                "matrix",
                                "multi_zone",
                                "highlight_weighted",
                            ] {
                                changed |=
                                    ui.selectable_value(method, opt.to_string(), opt).changed();
                            }
                        });
                }
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut self.params.camera.exposure_compensation_ev,
                            -5.0..=5.0,
                        )
                        .text("EV compensation")
                        .step_by(0.1),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.params.camera.film_format_mm, 4.0..=120.0)
                            .text("Film format (mm)")
                            .logarithmic(true),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut self.params.camera.lens_blur_um, 0.0..=100.0)
                            .text("Lens blur (µm)"),
                    )
                    .changed();
                if changed {
                    self.dirty = true;
                }
            });

        }
        if self.gui_tab == GuiTab::Film {
        // ── Halation ────────────────────────────────────────────────────
        egui::CollapsingHeader::new("Halation")
            .default_open(true)
            .show(ui, |ui| {
                let h = &mut self.params.film_render.halation;
                let mut changed = false;
                changed |= ui.checkbox(&mut h.active, "Active").changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut h.halation_amount, 0.0..=3.0)
                            .text("Halation amount"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut h.halation_spatial_scale, 0.1..=5.0)
                            .text("Halation scale"),
                    )
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut h.scatter_amount, 0.0..=3.0).text("Scatter amount"))
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut h.scatter_spatial_scale, 0.1..=5.0)
                            .text("Scatter scale"),
                    )
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut h.halation_n_bounces, 1..=5).text("Bounces"))
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut h.halation_bounce_decay, 0.0..=1.0)
                            .text("Bounce decay"),
                    )
                    .changed();
                changed |= ui
                    .checkbox(&mut h.halation_renormalize, "Renormalize")
                    .changed();
                if changed {
                    self.dirty = true;
                }
            });

        // ── DIR couplers ────────────────────────────────────────────────
        egui::CollapsingHeader::new("DIR couplers")
            .default_open(true)
            .show(ui, |ui| {
                let d = &mut self.params.film_render.dir_couplers;
                let mut changed = false;
                changed |= ui.checkbox(&mut d.active, "Active").changed();
                changed |= ui
                    .add(egui::Slider::new(&mut d.amount, 0.0..=2.0).text("Amount"))
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut d.diffusion_size_um, 0.0..=100.0)
                            .text("Diffusion size (µm)"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut d.diffusion_tail_um, 0.0..=400.0)
                            .text("Diffusion tail (µm)"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut d.diffusion_tail_weight, 0.0..=1.0)
                            .text("Tail weight"),
                    )
                    .changed();
                if changed {
                    self.dirty = true;
                }
            });

        // ── Diffusion filter (lens) ─────────────────────────────────────
        egui::CollapsingHeader::new("Diffusion filter (lens)")
            .default_open(true)
            .show(ui, |ui| {
                let df = &mut self.params.camera.diffusion_filter;
                let mut changed = false;
                changed |= ui.checkbox(&mut df.active, "Active").changed();
                egui::ComboBox::from_label("Family")
                    .selected_text(df.filter_family.clone())
                    .show_ui(ui, |ui| {
                        for fam in ["black_pro_mist", "glimmerglass", "pro_mist", "cinebloom"] {
                            changed |= ui
                                .selectable_value(&mut df.filter_family, fam.to_string(), fam)
                                .changed();
                        }
                    });
                changed |= ui
                    .add(egui::Slider::new(&mut df.strength, 0.0..=2.0).text("Strength"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut df.spatial_scale, 0.1..=3.0).text("Spatial scale"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut df.halo_warmth, -1.5..=1.5).text("Halo warmth"))
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut df.core_intensity, 0.0..=2.0).text("Core intensity"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut df.halo_intensity, 0.0..=2.0).text("Halo intensity"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut df.bloom_intensity, 0.0..=2.0)
                            .text("Bloom intensity"),
                    )
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut df.halo_size, 0.1..=3.0).text("Halo size"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut df.bloom_size, 0.1..=3.0).text("Bloom size"))
                    .changed();
                if changed {
                    self.dirty = true;
                }
            });

        // ── Grain ───────────────────────────────────────────────────────
        egui::CollapsingHeader::new("Grain")
            .default_open(true)
            .show(ui, |ui| {
                let g = &mut self.params.film_render.grain;
                let mut changed = false;
                changed |= ui.checkbox(&mut g.active, "Active").changed();
                egui::ComboBox::from_label("Engine")
                    .selected_text(match g.engine {
                        GrainEngine::V1 => "V1 — emulsion grain",
                        GrainEngine::V2 => "V2 — procedural grain",
                    })
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(
                                &mut g.engine,
                                GrainEngine::V1,
                                "V1 — emulsion grain",
                            )
                            .changed();
                        changed |= ui
                            .selectable_value(
                                &mut g.engine,
                                GrainEngine::V2,
                                "V2 — procedural grain",
                            )
                            .changed();
                    });
                if matches!(g.engine, GrainEngine::V2) {
                    let mut profile_changed = false;
                    egui::ComboBox::from_label("V2 profile")
                        .selected_text(g.v2_profile.clone())
                        .show_ui(ui, |ui| {
                            for profile in spektrafilm_model::grain_v2::PROFILE_NAMES {
                                profile_changed |= ui
                                    .selectable_value(
                                        &mut g.v2_profile,
                                        profile.to_owned(),
                                        profile,
                                    )
                                    .changed();
                            }
                        });
                    if profile_changed || ui.button("Reset V2 controls to profile").clicked() {
                        for value in [
                            &mut g.v2_size,
                            &mut g.v2_amount,
                            &mut g.v2_shadows,
                            &mut g.v2_midtones,
                            &mut g.v2_highlights,
                            &mut g.v2_chroma,
                            &mut g.v2_resolution_factor,
                        ] {
                            *value = None;
                        }
                        changed = true;
                    }
                    egui::ComboBox::from_label("V2 mode")
                        .selected_text(match g.v2_mode {
                            GrainV2Mode::Analogue => "Analogue",
                            GrainV2Mode::Noise => "Noise",
                        })
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(
                                    &mut g.v2_mode,
                                    GrainV2Mode::Analogue,
                                    "Analogue",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(&mut g.v2_mode, GrainV2Mode::Noise, "Noise")
                                .changed();
                        });
                    let resolved = g.resolved_grain_v2();
                    for (label, value, inherited, min, max) in [
                        ("Size", &mut g.v2_size, resolved.size, 1.0, 48.0),
                        ("Amount", &mut g.v2_amount, resolved.amount, 0.0, 1.0),
                        ("Shadows", &mut g.v2_shadows, resolved.shadows, 0.0, 1.0),
                        ("Midtones", &mut g.v2_midtones, resolved.midtones, 0.0, 1.0),
                        ("Highlights", &mut g.v2_highlights, resolved.highlights, 0.0, 1.0),
                        ("Chroma", &mut g.v2_chroma, resolved.color, 0.0, 1.0),
                        ("Film Resolution", &mut g.v2_resolution_factor, resolved.resolution_factor, 0.0, 100.0),
                    ] {
                        let mut displayed = value.unwrap_or(inherited);
                        if ui.add(egui::Slider::new(&mut displayed, min..=max).text(label)).changed() {
                            *value = Some(displayed);
                            changed = true;
                        }
                    }
                    egui::ComboBox::from_label("Resolution filter")
                        .selected_text(if g.v2_resolution_type == 0 { "Gaussian" } else { "Fast box FIR" })
                        .show_ui(ui, |ui| {
                            changed |= ui.selectable_value(&mut g.v2_resolution_type, 0, "Gaussian").changed();
                            changed |= ui.selectable_value(&mut g.v2_resolution_type, 1, "Fast box FIR").changed();
                        });
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut g.v2_timer, 0.0..=65535.0)
                                .text("V2 timer"),
                        )
                        .changed();
                } else {
                changed |= ui
                    .checkbox(&mut g.sublayers_active, "Layered sublayer grain")
                    .on_hover_text(
                        "Split the composite density into the emulsion's sublayers and grain \
                         each with its own particle field, dye-cloud blur and micro-structure \
                         (the Python 0.3.4 default). Off = single composite-density sampler.",
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut g.particle_area_um2, 0.05..=1.0)
                            .text("Particle area (µm²)"),
                    )
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut g.blur, 0.0..=3.0).text("Post-blur σ"))
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut g.blur_dye_clouds_um, 0.0..=10.0)
                            .text("Dye-cloud blur (µm)"),
                    )
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut g.n_sub_layers, 1..=4).text("Sub-layers"))
                    .on_hover_text(
                        "Composite-sampler sub-layer count (layered grain always uses the \
                         profile's 3 emulsion sublayers).",
                    )
                    .changed();
                }
                if changed {
                    self.dirty = true;
                }
            });

        }
        if self.gui_tab == GuiTab::Print {
        // ── Glare ───────────────────────────────────────────────────────
        // Print-paper viewing glare only — upstream 0.3.4 disables glare
        // entirely for direct-film scans (`glare = None`), so the panel is
        // inert in scan-film mode; disable it and say why rather than let
        // it silently affect nothing.
        egui::CollapsingHeader::new("Glare")
            .default_open(false)
            .show(ui, |ui| {
                if self.params.io.scan_film {
                    ui.label(
                        egui::RichText::new(
                            "Direct film scan — viewing glare is disabled (print-only effect).",
                        )
                        .italics()
                        .small(),
                    );
                }
                let scan_film = self.params.io.scan_film;
                let g = &mut self.params.print_render.glare;
                let mut changed = false;
                ui.add_enabled_ui(!scan_film, |ui| {
                    changed |= ui.checkbox(&mut g.active, "Active").changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut g.percent, 0.0..=0.2).text("Percent"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut g.roughness, 0.0..=2.0).text("Roughness"))
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut g.blur, 0.0..=5.0).text("Blur σ (px)"))
                        .changed();
                });
                if changed {
                    self.dirty = true;
                }
            });

        // ── Print curves (s023 morph) ───────────────────────────────────
        egui::CollapsingHeader::new("Print curves")
            .default_open(false)
            .show(ui, |ui| {
                let m = &mut self.params.print_render.density_curves_morph;
                let mut changed = false;
                changed |= ui
                    .checkbox(&mut m.active, "Morph density curves")
                    .on_hover_text(
                        "Rebuild the print density curves from the profile's parametric \
                         model with coupled-gamma morphing. Off = use the stored curves.",
                    )
                    .changed();
                if m.active {
                    changed |= ui
                        .add(egui::Slider::new(&mut m.gamma_factor, 0.5..=2.0).text("Gamma"))
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut m.gamma_factor_fast, 0.5..=2.0)
                                .text("Gamma fast"),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut m.gamma_factor_slow, 0.5..=2.0)
                                .text("Gamma slow"),
                        )
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut m.gamma_factor_red, 0.5..=2.0).text("Gamma R"))
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut m.gamma_factor_green, 0.5..=2.0).text("Gamma G"),
                        )
                        .changed();
                    changed |= ui
                        .add(egui::Slider::new(&mut m.gamma_factor_blue, 0.5..=2.0).text("Gamma B"))
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut m.developer_exhaustion, 0.0..=1.0)
                                .text("Developer exhaustion"),
                        )
                        .changed();
                }
                if changed {
                    self.dirty = true;
                }
            });

        }
        if self.gui_tab == GuiTab::Main {
        // ── Scanner ─────────────────────────────────────────────────────
        egui::CollapsingHeader::new("Scanner")
            .default_open(false)
            .show(ui, |ui| {
                let s = &mut self.params.scanner;
                let mut changed = false;
                changed |= ui
                    .add(egui::Slider::new(&mut s.lens_blur, 0.0..=5.0).text("Lens blur σ (px)"))
                    .changed();
                let [mut sigma, mut amount] = s.unsharp_mask;
                changed |= ui
                    .add(egui::Slider::new(&mut sigma, 0.0..=3.0).text("Unsharp σ (px)"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut amount, 0.0..=2.0).text("Unsharp amount"))
                    .changed();
                if changed {
                    s.unsharp_mask = [sigma, amount];
                }
                changed |= ui
                    .checkbox(&mut s.white_correction, "White correction")
                    .changed();
                if s.white_correction {
                    changed |= ui
                        .add(egui::Slider::new(&mut s.white_level, 0.5..=1.0).text("White level"))
                        .changed();
                }
                changed |= ui
                    .checkbox(&mut s.black_correction, "Black correction")
                    .changed();
                if s.black_correction {
                    changed |= ui
                        .add(egui::Slider::new(&mut s.black_level, 0.0..=0.5).text("Black level"))
                        .changed();
                }
                if changed {
                    self.dirty = true;
                }
            });

        }
        if self.gui_tab == GuiTab::Advanced {
        // ── Color management ────────────────────────────────────────────
        egui::CollapsingHeader::new("Color management")
            .default_open(false)
            .show(ui, |ui| {
                let algo = &mut self.params.io.output_gamut_compress.algorithm;
                let mut changed = false;
                egui::ComboBox::from_label("Gamut compression")
                    .selected_text(algo.clone())
                    .show_ui(ui, |ui| {
                        for opt in ["off", "oklch", "oklrab", "jzazbz", "cam16ucs", "aces_rgc"] {
                            changed |= ui.selectable_value(algo, opt.to_string(), opt).changed();
                        }
                    });
                changed |= ui.checkbox(&mut self.params.io.input_gamut_compress.active, "Compress input gamut").changed();
                // Input gamut compression — baked into the tc_lut at build time,
                // so changing it rebuilds the LUT on the next pass. "xy" is the
                // ACES-RGC-style radial compression toward the spectral locus.
                let in_algo = &mut self.params.io.input_gamut_compress.algorithm;
                egui::ComboBox::from_label("Input gamut compression")
                    .selected_text(in_algo.clone())
                    .show_ui(ui, |ui| {
                        for opt in ["xy", "oklch"] {
                            changed |= ui.selectable_value(in_algo, opt.to_string(), opt).changed();
                        }
                    });
                // CAT16 (vs CAT02) for the input chromatic adaptation feeding
                // Hanatos — better blue/violet behavior; matches upstream >=0.3.3.
                changed |= ui
                    .checkbox(
                        &mut self.params.settings.use_cat16,
                        "CAT16 input adaptation",
                    )
                    .changed();
                // RGB → film raw spectral upsampler. hanatos2025 is the default
                // spectral LUT; arctic2026alpha02 is the new memory-color
                // reflectance LUT; mallett2019 is a faster matrix basis.
                let method = &mut self.params.settings.rgb_to_raw_method;
                egui::ComboBox::from_label("RGB→raw upsampling")
                    .selected_text(method.clone())
                    .show_ui(ui, |ui| {
                        for opt in ["hanatos2025", "arctic2026alpha02", "mallett2019"] {
                            changed |= ui.selectable_value(method, opt.to_string(), opt).changed();
                        }
                    });
                if changed {
                    self.dirty = true;
                }
            });

        }
        if self.gui_tab == GuiTab::Main {
        // ── Enlarger ────────────────────────────────────────────────────
        egui::CollapsingHeader::new("Enlarger")
            .default_open(false)
            .show(ui, |ui| {
                let e = &mut self.params.enlarger;
                let mut changed = false;
                changed |= ui
                    .add(
                        egui::Slider::new(&mut e.print_exposure, 0.1..=5.0)
                            .text("Print exposure")
                            .logarithmic(true),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut e.m_filter_shift, -50.0..=50.0)
                            .text("Magenta filter shift"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut e.y_filter_shift, -50.0..=50.0)
                            .text("Yellow filter shift"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut e.preflash_exposure, 0.0..=0.5)
                            .text("Preflash exposure"),
                    )
                    .on_hover_text(
                        "Uniform low pre-exposure of the print through the film base. \
                         Lifts shadow density and lowers print contrast. 0 = off.",
                    )
                    .changed();
                if e.preflash_exposure > 0.0 {
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut e.preflash_m_filter_shift, -50.0..=50.0)
                                .text("Preflash magenta shift"),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut e.preflash_y_filter_shift, -50.0..=50.0)
                                .text("Preflash yellow shift"),
                        )
                        .changed();
                }
                if changed {
                    self.dirty = true;
                }
            });

        // ── Output ──────────────────────────────────────────────────────
        egui::CollapsingHeader::new("Output")
            .default_open(false)
            .show(ui, |ui| {
                let io = &mut self.params.io;
                let mut changed = false;
                egui::ComboBox::from_label("Output colour space")
                    .selected_text(io.output_color_space.clone())
                    .show_ui(ui, |ui| {
                        for opt in [
                            "sRGB",
                            "ProPhoto RGB",
                            "ITU-R BT.2020",
                            "ACES2065-1",
                            "Adobe RGB (1998)",
                            "Display P3",
                            "DCI-P3",
                        ] {
                            changed |= ui
                                .selectable_value(
                                    &mut io.output_color_space,
                                    opt.to_string(),
                                    opt,
                                )
                                .changed();
                        }
                    });
                let mut scan_film = io.scan_film;
                changed |= ui
                    .checkbox(&mut scan_film, "Scan film (skip printing)")
                    .on_hover_text(
                        "Scan the developed film directly instead of printing onto paper. \
                         Auto-enabled for positive/slide stocks (no print paper); toggle \
                         manually to scan a negative as-is.",
                    )
                    .changed();
                if changed {
                    io.scan_film = scan_film;
                }
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut io.upscale_factor).range(0.0..=f32::MAX).speed(0.5).prefix("Upscale factor "),
                    )
                    .on_hover_text(
                        "Resize the working image before processing (Python upscale_factor). \
                         Lower = faster preview + export at reduced resolution; 1.0 = full \
                         resolution. The diffusion-filter cost scales with this.",
                    )
                    .changed();
                changed |= ui
                    .checkbox(&mut io.output_cctf_encoding, "Encode output transfer function")
                    .changed();
                if changed {
                    self.dirty = true;
                }
            });

        }
        if self.gui_tab == GuiTab::Config {
            ui.separator();
            ui.heading("Display");
            let display_transform_before = self.viewer.settings.use_display_transform;
            self.viewer.controls(ui);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Display ICC…").clicked() {
                    if let Some(path) = self
                        .file_dialog("display_icc")
                        .add_filter("ICC profile", &["icc", "icm"])
                        .pick_file()
                    {
                        self.remember_dialog("display_icc", &path);
                        self.gui_state.sections["rust"]["display_profile"] =
                            serde_json::json!(path.to_string_lossy());
                        self.refresh_viewing_artifacts();
                    }
                }
                ui.label(&self.viewer.transform_status);
            });
            if display_transform_before != self.viewer.settings.use_display_transform {
                self.refresh_viewing_artifacts();
            }
        }
        // ── Metrics ─────────────────────────────────────────────────────
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.monospace(format!(
            "render:        {:>6.1} ms   {}",
            self.last_render_ms,
            fps_label(self.last_render_ms)
        ));
        ui.monospace(format!(
            "pipeline build:{:>6.1} ms",
            self.last_pipeline_build_ms
        ));
        ui.monospace(format!(
            "input/resize:  {:>6.1} / {:>5.1} ms",
            self.last_input_clone_ms, self.last_scale_ms
        ));
        ui.monospace(format!(
            "preview pack:  {:>6.1} ms",
            self.last_preview_ms
        ));
        ui.monospace(format!(
            "worker total:  {:>6.1} ms",
            self.last_worker_total_ms
        ));
        ui.monospace(format!("backend: {}", self.backend.name()));
        ui.label(egui::RichText::new(&self.status).small());
        if let Some(info) = self.raw_lens_info.as_deref() {
            ui.label(if info.is_empty() { "Lens correction not applied".to_owned() } else { format!("Lens correction applied ({info})") });
        }
        if input_view_before != (self.params.io.input_color_space.clone(), self.params.io.input_cctf_decoding, self.params.settings.preview_max_size) { self.refresh_viewing_artifacts(); }
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

        if self.dirty && self.image.is_some() && (self.gui_state.auto_preview() || self.force_preview) {
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
        self.poll_export_job(ctx);
        self.poll_calibration_job(ctx);
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
                if self.gui_tab == GuiTab::Config {
                    self.state_toolbar(ui);
                    ui.separator();
                }
                let scroll_height = (ui.available_height() - 36.0).max(1.0);
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
            self.viewer.show(ui,self.image.as_deref(),self.output_image.as_ref());
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

    /// Called once when the window is closing. If an export is still
    /// in-flight, set the cancel flag and join the worker thread so
    /// the child process is stopped before the GUI exits.
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
    // Same resolution as the render path, so the combo always highlights
    // exactly the entry the pipeline will use.
    let current_idx = profile::development_time_index(times, *selection);
    let mut changed = false;
    ui.label(label);
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
        });
    changed
}

fn profile_combo(
    ui: &mut egui::Ui,
    salt: &str,
    label: &str,
    entries: &[ProfileEntry],
    selected_stock: &mut String,
) -> bool {
    ui.label(label);
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
        });
    prev != *selected_stock
}

fn fps_label(ms: f32) -> &'static str {
    if ms < 16.0 {
        "60 fps"
    } else if ms < 33.0 {
        "30 fps"
    } else if ms < 67.0 {
        "15 fps"
    } else if ms < 200.0 {
        "5 fps"
    } else if ms < 400.0 {
        "2 fps"
    } else {
        ""
    }
}

fn preview_pipeline_cache_key(
    film_name: &str,
    print_name: &str,
    params: &RuntimeParams,
) -> String {
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



/// Locate the f64-built `spektrafilm` CLI binary. Search order:
///   1. `$SPEKTRAFILM_F64_CLI` — explicit override, full path.
///   2. `spektrafilm-f64` next to the running GUI executable (release
///      builds: both binaries live in `target/release/`).
///   3. `spektrafilm-f64` on `$PATH`.
///   4. `target/release/spektrafilm-f64` relative to `CARGO_MANIFEST_DIR`
///      (handy when running via `cargo run`).
///
/// Returns a path that exists and is executable, or an explanatory
/// error pointing to the build command.
fn locate_f64_cli() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("SPEKTRAFILM_F64_CLI") {
        let path = PathBuf::from(&p);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "SPEKTRAFILM_F64_CLI points at {p} but no such file"
        ));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for name in f64_cli_names() {
            let p = dir.join(name);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    for name in f64_cli_names() {
        if let Some(p) = which_on_path(name) {
            return Ok(p);
        }
    }
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        for name in f64_cli_names() {
            let p = PathBuf::from(&manifest)
                .join("..")
                .join("..")
                .join("target")
                .join("release")
                .join(name);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    Err("no export CLI found. Build it with \
         `cargo build --release -p spektrafilm-cli --features precision-f64 --bin spektrafilm-f64`, \
         and either put it on PATH or set $SPEKTRAFILM_F64_CLI."
        .into())
}

fn f64_cli_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["spektrafilm-f64.exe", "spektrafilm-f64"]
    } else {
        &["spektrafilm-f64"]
    }
}

/// Minimal PATH lookup so we don't pull in the `which` crate for one call.
fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Removes export state and staged images on cancellation, failure, or close.
/// A staged image keeps its destination so only the UI can publish completion.
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

/// Run the exporter with an explicit backend and a temporary parameter snapshot.
/// Cancellation kills and reaps the child; only a successful export is published.
fn run_export(
    cli_path: &Path,
    input: &Path,
    output: &Path,
    film: &str,
    paper: &str,
    params: &spektrafilm_core::params::RuntimeParams,
    data_dir: &Path,
    save_depth: BitDepth,
    gui_state: &serde_json::Value,
    backend: ExportBackend,
    cancel: &AtomicBool,
) -> Result<TempPath> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp = TempPath(std::env::temp_dir().join(format!(
        "spektrafilm-export-{}-{nanos}.json",
        std::process::id()
    )), None);
    // Preserve the extension for CLI format selection and keep staging on
    // the destination filesystem so publication is one atomic rename.
    let staged_name = format!(
        "spektrafilm-export-{}-{nanos}.{}",
        std::process::id(),
        output.extension().and_then(|s| s.to_str()).unwrap_or("png")
    );
    let staged = TempPath(output.with_file_name(staged_name), Some(output.to_path_buf()));
    {
        let f = std::fs::File::create(&temp.0)
            .with_context(|| format!("creating params tempfile {}", temp.0.display()))?;
        serde_json::to_writer(std::io::BufWriter::new(f), params)
            .context("serializing params to JSON")?;
    }

    let stderr_path = TempPath(temp.0.with_extension("stderr"), None);
    let stderr_file = std::fs::File::create(&stderr_path.0)
        .context("creating export error log")?;
    let mut cmd = std::process::Command::new(cli_path);
    cmd.arg("process")
        .arg("--backend")
        .arg(backend.argument())
        .arg(input)
        .arg("-o")
        .arg(&staged.0)
        .arg("--bit-depth")
        .arg(save_depth.bits().to_string())
        .arg("--film")
        .arg(film)
        .arg("--paper")
        .arg(paper)
        .arg("--params")
        .arg(&temp.0)
        .arg("--data-dir")
        .arg(data_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(stderr_file));
    if params.io.scan_film {
        cmd.arg("--scan-film");
    }
    cmd.arg("--saving-color-space").arg(gui_state["simulation"]["saving_color_space"].as_str().unwrap_or("sRGB"))
        .arg("--saving-cctf-encoding").arg(if gui_state["simulation"]["saving_cctf_encoding"].as_bool().unwrap_or(true) {"true"} else {"false"});
    for (key,flag) in [("film_channel_swap","--film-channel-swap"),("print_channel_swap","--print-channel-swap")] {
        if let Some(order) = gui_state["special"][key].as_array() {
            cmd.arg(flag).arg(order.iter().map(|v|v.as_u64().unwrap_or(0).to_string()).collect::<Vec<_>>().join(","));
        }
    }
    let raw = &gui_state["load_raw"];
    cmd.arg("--raw-white-balance").arg(raw["white_balance"].as_str().unwrap_or("as_shot").replace('_', "-"))
        .arg("--raw-temperature").arg(raw["temperature"].as_f64().unwrap_or(5500.0).to_string())
        .arg("--raw-tint").arg(raw["tint"].as_f64().unwrap_or(1.0).to_string());
    if raw["lens_correction"].as_bool().unwrap_or(false) { cmd.arg("--lens-correction"); }
    #[cfg(windows)]
    if let Some(parent) = cli_path.parent() {
        let mut directories = vec![parent.to_path_buf()];
        if let Some(path) = std::env::var_os("PATH") { directories.extend(std::env::split_paths(&path)); }
        if let Ok(path) = std::env::join_paths(directories) { cmd.env("PATH",path); }
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning {}", cli_path.display()))?;

    // Poll cancellation without blocking the UI; logs go to a file so verbose
    // GPU driver output cannot fill an unread pipe and stall the child.
    let status = loop {
        let completed = match child.try_wait() {
            Ok(status) => status,
            Err(e) => {
                let _ = child.kill();
                child.wait().context("reaping f64 CLI after wait failure")?;
                return Err(e).context("waiting for f64 CLI");
            }
        };
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            child.wait().context("reaping cancelled f64 CLI")?;
            anyhow::bail!("cancelled");
        }
        if let Some(status) = completed {
            break status;
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let stderr_buf = std::fs::read_to_string(&stderr_path.0)
        .context("reading export error log")?;

    if !status.success() {
        let trimmed = stderr_buf.trim();
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "(signal)".into());
        anyhow::bail!(
            "f64 CLI exited {code} — {}",
            if trimmed.is_empty() {
                "(no stderr)"
            } else {
                trimmed
            }
        );
    }
    Ok(staged)
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
