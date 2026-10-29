//! Real App acceptance without a display server or a native file chooser.
//! Only confirmed chooser paths cross the explicit Save/Export boundary here.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use spektrafilm_backend::cpu_backend::CpuBackend;
use tempfile::TempDir;
mod calibration;
mod dialogs;
mod files;
mod history_ui;
mod i18n;
mod lifecycle;
mod operator;
mod pages;
mod panels;
mod raw;
mod result;
mod viewer;
mod workflow;
mod worklist;

struct Session {
    harness: Harness<'static, App>,
    directory: TempDir,
}
impl Drop for Session {
    fn drop(&mut self) {
        eframe::App::on_exit(self.harness.state_mut(), None);
    }
}

/// Workspace `data/` directory, resolved from this crate's manifest directory.
///
/// Shared by every GUI acceptance/unit test that needs real profiles or presets.
pub(crate) fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

/// Repository root (the parent of [`data_dir`]), for assertions that generated
/// artifacts stay outside the checkout.
#[cfg(feature = "gui-snapshots")]
pub(crate) fn repository_root() -> PathBuf {
    data_dir().parent().unwrap().to_path_buf()
}

/// Write `image` under `SPEKTRAFILM_GUI_SNAPSHOT_DIR` when that absolute,
/// repository-external directory is set. Returns without writing otherwise,
/// so the default gate never depends on a snapshot directory.
#[cfg(feature = "gui-snapshots")]
pub(crate) fn save_snapshot(image: &image::RgbaImage, name: &str) {
    let Some(directory) = std::env::var_os("SPEKTRAFILM_GUI_SNAPSHOT_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    assert!(
        directory.is_absolute(),
        "snapshot directory must be absolute"
    );
    let repository = repository_root().canonicalize().unwrap();
    std::fs::create_dir_all(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    assert!(
        !directory.starts_with(repository),
        "snapshot artifacts must be outside the repository"
    );
    image.save(directory.join(name)).unwrap();
}

fn app(state_path: PathBuf, image_path: PathBuf, browser_path: PathBuf) -> App {
    App::new_with_browser_path(
        Arc::new(CpuBackend),
        Some(image_path),
        Some(state_path),
        None,
        browser_path,
        data_dir(),
    )
}

fn session() -> Session {
    session_fixture()
}

fn grain_session() -> Session {
    let mut session = session_fixture();
    session
        .harness
        .state_mut()
        .params
        .debug
        .deactivate_stochastic_effects = false;
    session
        .harness
        .state_mut()
        .params
        .debug
        .deactivate_spatial_effects = false;
    session.harness.run_steps(3);
    session
}

/// Queue a chooser result and forbid falling back to a native dialog.
/// A scripted action with no seeded result cancels instead, so a missing
/// expectation fails the test rather than opening a desktop dialog.
fn script(h: &mut Harness<'_, App>, result: chooser::ChooserResult) {
    h.state_mut().chooser.set_strict(true);
    h.state_mut().chooser.scripted(result);
}

/// Deterministic factory state shared by the headless fixtures: stochastic,
/// spatial, and reveal effects disabled so preview and scan comparisons stay
/// reproducible. Returns the saved state path.
fn deterministic_state(directory: &Path) -> PathBuf {
    crate::i18n::save_preference(directory, crate::i18n::LanguagePreference::English).unwrap();
    let factory = state::GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.settings.preview_max_size = 128;
    params.camera.auto_exposure = false;
    params.debug.deactivate_spatial_effects = true;
    params.debug.deactivate_stochastic_effects = true;
    params.io.crop = false;
    let mut initial =
        state::GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
            .unwrap();
    // The viewer's 50-frame Polaroid reveal paints one full-canvas raster per
    // frame. That costs about 0.7 s per frame at this harness size, so sessions
    // start with the animation off. The animation keeps its coverage in
    // `headless::viewer::first_output_and_scan_animate_but_automatic_parameter_update_does_not`,
    // which turns it back on through `viewer.settings.reveal`.
    initial.sections["rust"]["viewer"]["settings"]["reveal"] = serde_json::json!(false);
    let state_path = directory.join("initial.json");
    initial.save(&state_path).unwrap();
    state_path
}

/// Deterministic synthetic input shared by the headless fixtures: a ramp image
/// plus the deterministic state from [`deterministic_state`].
pub(crate) fn deterministic_inputs(directory: &Path) -> (PathBuf, PathBuf) {
    let image_path = directory.join("asymmetric-ramp.png");
    let mut image = ImageBuf::new(160, 80);
    for y in 0..image.height {
        for x in 0..image.width {
            let i = (y * image.width + x) as usize * 3;
            let ramp = 0.08 + 0.8 * x as f64 / (image.width - 1) as f64;
            image.data[i] = spektrafilm_math::precision::from_f64(ramp);
            image.data[i + 1] =
                spektrafilm_math::precision::from_f64(ramp * 0.85 + y as f64 * 0.0005);
            image.data[i + 2] = spektrafilm_math::precision::from_f64(ramp * 0.7 + 0.04);
        }
    }
    image_io::save(
        &image_path,
        &image,
        SaveOptions {
            depth: BitDepth::Eight,
            color_space: "sRGB",
            cctf_encoding: true,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
        },
        None,
    )
    .unwrap();
    let state_path = deterministic_state(directory);
    (image_path, state_path)
}

/// Real startup with production defaults for automatic-diagnostics
/// acceptance: no explicit diagnostics request, so every run records into its
/// own persistent `<config_root>/logs/diagnostics/session-*` folder while
/// scratch inputs stay in `work`. Returns after the startup preview completed
/// and its reports reached the background writer.
pub(crate) fn default_startup(config_root: &Path, work: &Path) -> Harness<'static, App> {
    std::fs::create_dir_all(config_root).unwrap();
    std::fs::create_dir_all(work).unwrap();
    let (image_path, state_path) = deterministic_inputs(work);
    let browser_path = config_root.join("preset_browser_state.json");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            app(state_path, image_path, browser_path)
        });
    await_input(&mut harness);
    wait_until(&mut harness, |app| {
        !app.dirty
            && app.render_job.is_none()
            && app.output_image.is_some()
            && app.diagnostics.pending_publications() == 0
    });
    harness
}

/// Inspect the report returned by the real App worker, not a reconstructed
/// execution snapshot. Device rendering and its transfers must share the
/// operation's phase tree.
fn assert_retained_gpu_report(
    app: &App,
    path: &Path,
    kind: &str,
    working_dimensions: [u32; 2],
) -> spektrafilm_core::telemetry::Report {
    use spektrafilm_backend::telemetry::{BackendSelected, ExecutionPath, Executor};
    app.diagnostics.save_selected(path).unwrap();
    let report: spektrafilm_core::telemetry::Report =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(report.operation.kind).unwrap(), kind);
    assert_eq!(
        report.operation.outcome,
        spektrafilm_core::telemetry::Outcome::Succeeded
    );
    assert_eq!(
        report.execution.backend_selected,
        Some(BackendSelected::Wgpu)
    );
    assert_eq!(report.execution.path, Some(ExecutionPath::GpuResident));
    assert_eq!(report.execution.adapter, app.backend.adapter_description());
    assert!(
        report
            .execution
            .adapter
            .as_ref()
            .is_some_and(|adapter| !adapter.name.is_empty())
    );
    assert_eq!(
        report.configuration.working_dimensions,
        Some(working_dimensions)
    );
    let simulation = report
        .phases
        .iter()
        .find(|phase| phase.name == "simulation")
        .expect("retained rendering has a simulation phase");
    let chain = report
        .stages
        .iter()
        .find(|stage| stage.name == "film_chain")
        .expect("the actual film chain must be observed");
    assert_eq!(chain.parent_id, simulation.id);
    assert_eq!(chain.executor, Some(Executor::Gpu));
    assert!(chain.complete);
    assert!(chain.work.dispatch_count.is_some_and(|count| count > 0));
    assert!(chain.start_offset >= simulation.start_offset);
    assert!(chain.start_offset + chain.duration <= simulation.start_offset + simulation.duration);
    report
}

/// The shared harness deactivates stochastic effects so preview and scan
/// comparisons stay deterministic.
fn session_fixture() -> Session {
    let directory = tempfile::tempdir().unwrap();
    let (image_path, state_path) = deterministic_inputs(directory.path());
    let browser_path = directory.path().join("browser.json");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 1400.0))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            app(state_path, image_path, browser_path)
        });
    await_input(&mut harness);
    assert_eq!(harness.state().image.as_ref().unwrap().width(), 160);
    click(&mut harness, "Collapse or expand Presets");
    Session { harness, directory }
}

/// A session built for offscreen pixel capture: the production window size, the
/// wgpu renderer, and the presets panel left in its default expanded state.
#[cfg(feature = "gui-snapshots")]
fn snapshot_session() -> Session {
    let directory = tempfile::tempdir().unwrap();
    let (image_path, state_path) = deterministic_inputs(directory.path());
    let browser_path = directory.path().join("browser.json");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1460.0, 980.0))
        .wgpu()
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            app(state_path, image_path, browser_path)
        });
    await_input(&mut harness);
    assert_eq!(harness.state().image.as_ref().unwrap().width(), 160);
    Session { harness, directory }
}

/// Build a session whose App starts without an image, so a test can drive its
/// own import path. Shares the deterministic state from [`deterministic_state`].
#[cfg(feature = "gui-raw")]
fn empty_session() -> Session {
    let directory = tempfile::tempdir().unwrap();
    let state_path = deterministic_state(directory.path());
    let browser_path = directory.path().join("browser.json");
    let harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 1400.0))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            App::new_with_browser_path(
                Arc::new(CpuBackend),
                None,
                Some(state_path),
                None,
                browser_path,
                data_dir(),
            )
        });
    assert!(harness.state().image.is_none());
    Session { harness, directory }
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    await_input(h);
    // Widget disclosure animation must settle before sampling pointer bounds.
    // This does not disable the Viewer's composition/development animation.
    h.ctx.style_mut(|style| style.animation_time = 0.0);
    h.run_steps(3);
    if matches!(label, "100%" | "200%" | "400%") {
        h.query_all_by_role(egui::accesskit::Role::Button)
            .find(|node| {
                node.label()
                    .is_some_and(|caption| caption.starts_with("zoom "))
            })
            .expect("Viewer exact zoom menu")
            .simulate_click();
        h.run_steps(3);
    }
    scroll_controls_into_view(h, label);
    h.get_by_label(label).simulate_click();
    // Centered dialogs measure their content before their final placement.
    h.run_steps(6);
    await_input(h);
}

fn search_controls(h: &mut Harness<'_, App>, text: &str) {
    await_input(h);
    let field = h.get_by_label("Search controls");
    field.focus();
    h.step();
    h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    if text.is_empty() {
        h.press_key(egui::Key::Backspace);
    } else {
        h.get_by_label("Search controls").type_text(text);
    }
    h.run_steps(3);
}

fn open_settings(h: &mut Harness<'_, App>) {
    if !h.state().settings_open {
        click(h, "Settings");
    }
}

fn close_settings(h: &mut Harness<'_, App>) {
    if h.state().settings_open {
        click(h, "Close settings");
    }
}

fn navigate_stage(h: &mut Harness<'_, App>, stage: controls::Stage) {
    close_settings(h);
    search_controls(h, "");
    h.query_all_by_role_and_label(egui::accesskit::Role::Button, stage.label())
        .min_by(|a, b| {
            a.node()
                .raw_bounds()
                .unwrap()
                .y0
                .total_cmp(&b.node().raw_bounds().unwrap().y0)
        })
        .expect("persistent stage jump")
        .simulate_click();
    h.run_steps(3);
}

fn open_export(h: &mut Harness<'_, App>) {
    close_settings(h);
    if !h.state().export_dialog.is_open() {
        click(h, "Export");
        // The centered Area first measures its content, then places it.
        h.run_steps(3);
    }
}

fn locator_label(h: &Harness<'_, App>, path: &str) -> String {
    let app = h.state();
    controls::SearchIndex::default()
        .results_filtered(
            path,
            &app.params,
            &app.gui_state.sections,
            app.film_medium,
            app.parameter_revision,
            &app.data_dir,
            false,
        )
        .first()
        .unwrap_or_else(|| panic!("Missing control locator: {path}"))
        .localized_locator_label(&h.ctx)
}

fn reveal_control(h: &mut Harness<'_, App>, path: &str) {
    h.ctx.style_mut(|style| style.animation_time = 0.0);
    h.run_steps(3);
    close_settings(h);
    if h.state().export_dialog.is_open() {
        click(h, "Cancel");
    }
    let entry =
        controls::lookup(path).unwrap_or_else(|| panic!("Missing control inventory: {path}"));
    let locator = locator_label(h, path);
    let separate = !matches!(entry.owner, controls::ControlOwner::Stage(_));
    search_controls(h, path);
    if separate {
        // Locator results are above the inspector's parameter ScrollView.
        h.get_by_label(&locator).simulate_click();
        h.run_steps(6);
    }
}

/// Bring a retained AccessKit node into its real owner's visible scroll band.
fn scroll_controls_into_view(h: &mut Harness<'_, App>, label: &str) {
    if matches!(
        label,
        "Close settings" | "Close export" | "Cancel" | "Export file…" | "SCAN" | "SAVE"
    ) {
        return;
    }
    let Some(viewport_label) = [
        "Settings content",
        "Export options content",
        "Inspector controls",
    ]
    .into_iter()
    .find(|title| {
        h.query_by_role_and_label(egui::accesskit::Role::ScrollView, title)
            .is_some()
    }) else {
        return;
    };
    for _ in 0..24 {
        let viewport = h
            .get_by_role_and_label(egui::accesskit::Role::ScrollView, viewport_label)
            .raw_bounds()
            .expect("visible scroll owner");
        let (left, top, bottom) = (viewport.x0, viewport.y0, viewport.y1);
        let pointer = egui::pos2(
            ((viewport.x0 + viewport.x1) * 0.5) as f32,
            ((viewport.y0 + viewport.y1) * 0.5) as f32,
        );
        // Inspector content has its own accessibility subtree. Modal owners
        // cover the active dialog, including their retained offscreen nodes.
        let owner = if viewport_label == "Inspector controls" {
            h.get_by_role_and_label(egui::accesskit::Role::ScrollView, viewport_label)
        } else {
            h.node()
        };
        let target = owner
            .query_all_by_label(label)
            .filter_map(|node| node.node().raw_bounds())
            .find(|bounds| bounds.x0 >= left && (bounds.y0 < top || bounds.y1 > bottom));
        let Some(bounds) = target else {
            return;
        };
        h.input_mut()
            .events
            .push(egui::Event::PointerMoved(pointer));
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(
                0.0,
                if bounds.y0 < top {
                    top - bounds.y0 + 8.0
                } else {
                    bottom - bounds.y1 - 8.0
                }
                .clamp(-350.0, 350.0) as f32,
            ),
            modifiers: Default::default(),
        });
        h.run_steps(10);
    }
}

/// Click an export action. The sidebar button that opens the modal and the
/// modal's own submit now carry distinct labels, so the caller names the step
/// it wants. The sidebar action is disabled while the modal is open, so an
/// enabled-step query resolves the actionable node either way.
fn export_action(h: &mut Harness<'_, App>, step: &str) {
    scroll_controls_into_view(h, step);
    let buttons: Vec<_> = h
        .query_all_by_label(step)
        .filter(|node| !node.node().is_disabled())
        .collect();
    assert_eq!(buttons.len(), 1, "expected one enabled {step} action");
    buttons[0].simulate_click();
    h.run_steps(3);
    if step == "Export file…" {
        assert!(
            !h.state().export_dialog.is_open(),
            "Export submit closes its draft"
        );
    }
}

/// The production Open action selects the decoder from the accepted file kind.
fn import_file(h: &mut Harness<'_, App>, _raw: bool) {
    close_settings(h);
    click(h, "Open");
}
/// Pick `option` from the combo labeled `label`.
///
/// The option is looked up by button role so popup choices remain distinct
/// from any matching labels already present in the inspector.
fn choose(h: &mut Harness<'_, App>, label: &str, option: &str) {
    await_input(h);
    scroll_controls_into_view(h, label);
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, label)
        .focus();
    h.run_steps(3);
    h.press_key(egui::Key::Space);
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, option)
        .click();
    h.run_steps(3);
    await_input(h);
    if h.ctx.memory(|memory| memory.any_popup_open()) {
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, label)
            .click();
        h.run_steps(3);
    }
}

fn edit(h: &mut Harness<'_, App>, label: &str, text: &str) {
    await_input(h);
    scroll_controls_into_view(h, label);
    h.get_by_label(label).focus();
    h.step();
    // egui's command modifier is platform dependent; no global environment changes.
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: !cfg!(target_os = "macos"),
            mac_cmd: cfg!(target_os = "macos"),
            command: true,
            ..Default::default()
        },
    });
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    h.step();
    h.press_key(egui::Key::Enter);
    h.run_steps(3);
    await_input(h);
}

/// Wall-clock budget for a headless wait. The waits assert that the app
/// finishes, never how fast, and CI runners are several times slower than
/// developer machines while running the bin's tests in parallel.
pub(crate) const WAIT_DEADLINE: Duration = Duration::from_secs(180);

/// Yield the runner's cores to the worker thread a wait is waiting for. The
/// workers are separate threads, so a spinning waiter competes with the very
/// work it waits on and can starve it past the deadline on few-core runners.
pub(crate) fn wait_tick() {
    std::thread::sleep(Duration::from_millis(1));
}

#[track_caller]
fn wait_until(h: &mut Harness<'_, App>, condition: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT_DEADLINE;
    loop {
        h.step();
        if condition(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "App completion timed out: {}; dirty={}; full_scan={}; job={:?}; calibration={}; retained_scan={:?}; host={}; device={}; current=({}, {}); source={:?}",
            h.state().status,
            h.state().dirty,
            h.state().full_scan_requested,
            h.state()
                .render_job
                .as_ref()
                .map(|job| (job.kind == RenderKind::Scan, job.request_id)),
            h.state().calibration_job.is_some(),
            h.state().retained_kind.map(|kind| kind == RenderKind::Scan),
            h.state().output_image.is_some(),
            h.state().output_retained.is_some(),
            h.state().input_epoch,
            h.state().parameter_revision,
            h.state().source_render.as_ref().map(|source| (
                source.operation_id,
                source.input_revision,
                source.parameter_revision
            ))
        );
        wait_tick();
    }
}

fn analysis_output_ready(app: &App) -> bool {
    let Some(render) = app.source_render.as_ref() else {
        return false;
    };
    app.analysis.result.as_ref().is_some_and(|result| {
        app.analysis.identity.as_ref() == Some(&result.identity)
            && result.identity.modified_generation == Some(render.operation_id)
            && result.data[1].is_some()
    })
}

fn save_retained(h: &mut Harness<'_, App>, path: &Path) {
    h.state_mut().save_to_path(path);
    wait_until(h, |app| app.pending_saves.is_empty());
}

/// Wait for the production input worker, including its UI-side commit callback.
fn await_input(h: &mut Harness<'_, App>) {
    if h.state().input_pending() {
        wait_until(h, |app| !app.input_pending());
    }
}

/// Poll without rendering for unit tests exercising a source transaction directly.
pub(crate) fn await_app_input(app: &mut App, ctx: &egui::Context) {
    let deadline = Instant::now() + WAIT_DEADLINE;
    while app.input_pending() {
        app.poll_input_job(ctx);
        assert!(
            Instant::now() < deadline,
            "input preparation timed out: {}",
            app.status
        );
        wait_tick();
    }
}

/// Advance the viewer until it has painted the last frame of its
/// reveal/crossfade transition and accepted the new output. Sessions start with
/// the reveal animation disabled, so this returns once the display worker
/// accepts the render.
fn settle_animation(h: &mut Harness<'_, App>) {
    wait_until(h, |a| {
        let observation = a.viewer.observation();
        observation.generation == observation.accepted_generation
            && observation.transition.is_none()
    });
}

/// Wait for the accepted automatic result, including an already-current Scan.
/// This never manufactures a render request: accepted UI edits own scheduling.
fn preview(h: &mut Harness<'_, App>) -> Arc<ImageBuf> {
    wait_until(h, |a| {
        !a.dirty
            && a.render_job.is_none()
            && a.calibration_job.is_none()
            && !a.accepted_frame_is_stale()
            && a.output_image.is_some()
    });
    h.state().output_image.as_ref().unwrap().clone()
}

fn scan(h: &mut Harness<'_, App>) -> Arc<ImageBuf> {
    preview(h);
    wait_until(h, |a| a.export_job.is_none() && a.calibration_job.is_none());
    let previous = h.state().output_image.clone();
    click(h, "SCAN");
    wait_until(h, |a| {
        !a.dirty
            && a.render_job.is_none()
            && a.retained_kind == Some(RenderKind::Scan)
            && !a.accepted_frame_is_stale()
            && a.output_image
                .as_ref()
                .is_some_and(|out| previous.as_ref().is_none_or(|old| !Arc::ptr_eq(old, out)))
    });
    h.state().output_image.as_ref().unwrap().clone()
}

fn mean(image: &ImageBuf) -> f64 {
    image.data.iter().map(|&x| x as f64).sum::<f64>() / image.data.len() as f64
}

fn edge_difference(image: &ImageBuf) -> f64 {
    let row = image.height / 2;
    let pixel = |x| image.data[((row * image.width + x) * 3) as usize] as f64;
    pixel(image.width * 3 / 4) - pixel(image.width / 4)
}

fn assert_png(path: &Path, expected: &ImageBuf) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(bytes[24], 8, "PNG must carry 8-bit samples");
    let actual = image_io::load(path).unwrap().image;
    assert_eq!(
        (actual.width, actual.height),
        (expected.width, expected.height)
    );
    assert_eq!(actual.data.len(), expected.data.len());
    for (&a, &e) in actual.data.iter().zip(&expected.data) {
        let clipped = (e as f64).clamp(0.0, 1.0);
        assert!(
            (a as f64 - clipped).abs() <= 1.01 / 255.0,
            "saved pixel {a} differs from {e}"
        );
    }
}

// Read standard TIFF tags rather than infer the depth from roundtrip pixels.
fn assert_float_tiff(path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    let little = match &bytes[..2] {
        b"II" => true,
        b"MM" => false,
        other => panic!("TIFF byte order: {other:?}"),
    };
    let u16_at = |at| {
        let value = [bytes[at], bytes[at + 1]];
        if little {
            u16::from_le_bytes(value)
        } else {
            u16::from_be_bytes(value)
        }
    };
    let u32_at = |at| {
        let value = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        if little {
            u32::from_le_bytes(value)
        } else {
            u32::from_be_bytes(value)
        }
    };
    assert_eq!(u16_at(2), 42, "small exported TIFF uses classic IFD");
    let ifd = u32_at(4) as usize;
    let mut bits = None;
    let mut sample_format = None;
    for index in 0..u16_at(ifd) as usize {
        let entry = ifd + 2 + index * 12;
        let tag = u16_at(entry);
        if matches!(tag, 258 | 339) {
            assert_eq!(u16_at(entry + 2), 3, "sample tag must be SHORT");
            let count = u32_at(entry + 4) as usize;
            let values = if count * 2 <= 4 {
                entry + 8
            } else {
                u32_at(entry + 8) as usize
            };
            let samples: Vec<_> = (0..count).map(|i| u16_at(values + i * 2)).collect();
            if tag == 258 {
                bits = Some(samples);
            } else {
                sample_format = Some(samples);
            }
        }
    }
    let bits = bits.expect("BitsPerSample TIFF tag");
    assert!(!bits.is_empty() && bits.iter().all(|&x| x == 32));
    let format = sample_format.expect("SampleFormat TIFF tag");
    assert!(
        !format.is_empty() && format.iter().all(|&x| x == 3),
        "IEEE floating point samples"
    );
}

#[test]
fn scanner_ui_real_preview_scan_persistence_save_and_export() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input > film > scan");
    assert_eq!(h.state().params.workflow.route, "input > film > scan");
    reveal_control(h, "extras.simulation.film_stock");
    let profile = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    let profile_label = profile.info.name.as_deref().unwrap_or("kodak_portra_400");
    choose(h, "film profile", profile_label);
    assert_eq!(h.state().film_name, "kodak_portra_400");
    // Selecting a stock intentionally resets its workflow; choose the eligible
    // scanner route after that production profile-selection behavior.
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input > film > scan");
    search_controls(h, "Scanner");
    choose(h, "scan output", "Preserve medium polarity");
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "scanner lamp")
            .is_none(),
        "the readout light is a product constant, not a control"
    );
    let direct = preview(h);
    assert_eq!((direct.width, direct.height), (128, 64));
    edit(h, "Lens blur (readout optics)", "0.5");
    assert_eq!(h.state().params.scanner.lens_blur, 0.5);
    edit(h, "Lens blur (readout optics)", "not-a-number");
    assert_eq!(h.state().params.scanner.lens_blur, 0.5);
    edit(h, "Unsharp mask (digital sharpening) sigma", "1.5");
    assert_eq!(h.state().params.scanner.unsharp_mask[0], 1.5);
    edit(h, "Lens blur (readout optics)", "0");
    assert_eq!(h.state().params.scanner.lens_blur, 0.0);
    // The rest of this test exercises the negative-capture scanner path, so
    // select it through the real control rather than writing the parameter.
    choose(h, "scan output", "Negative to positive");
    assert_eq!(h.state().params.scanner.scan_output, "negative_to_positive");
    // Negative conversion previews stay frame-independent until the
    // experimental setup asks for the complete frame.
    h.state_mut().params.scanner.experimental_setup = true;
    h.run_steps(3);
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "scanner lamp")
            .is_none()
    );
    // The conversion style belongs to the negative interpretation, so the
    // combo appears only once that polarity is selected. Its end-to-end
    // behaviour is covered by `conversion_style_combo_offers_installed_styles_and_renders_them`,
    // which needs a data directory the test authors styles into.
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "conversion style")
            .is_some()
    );
    let preview = preview(h);
    let accepted = h.state().current_state().unwrap();
    let accepted_params = h.state().params.clone();
    let accepted_input = h.state().image.as_ref().unwrap().clone();
    let accepted_output = h.state().output_image.as_ref().unwrap().clone();
    let revision = h.state().parameter_revision;
    let calibration_epoch = h.state().calibration_epoch;
    for carrier in ["canonical", "extension", "shadowed_extension"] {
        let mut invalid = accepted.sections.clone();
        if carrier == "canonical" {
            invalid["scanner"]["scan_output"] = serde_json::json!("positive_scan");
        } else {
            invalid["rust"]["runtime"]["scanner"]["scan_output"] =
                serde_json::json!("positive_scan");
            if carrier == "extension" {
                invalid["scanner"]
                    .as_object_mut()
                    .unwrap()
                    .remove("scan_output");
            }
        }
        let path = s.directory.path().join(format!("removed-{carrier}.json"));
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let result = state::GuiState::load(&path)
            .and_then(|state| h.state_mut().apply_state_with_source(state, |_| {}));
        let error = result.unwrap_err();
        assert!(format!("{error:#}").contains("positive_scan"), "{error:#}");
        assert_eq!(h.state().params, accepted_params);
        assert_eq!(h.state().parameter_revision, revision);
        assert_eq!(h.state().calibration_epoch, calibration_epoch);
        assert_eq!(
            h.state().current_state().unwrap().sections,
            accepted.sections
        );
        assert!(
            h.state()
                .image
                .as_ref()
                .unwrap()
                .same_source(&accepted_input)
        );
        assert!(Arc::ptr_eq(
            h.state().output_image.as_ref().unwrap(),
            &accepted_output
        ));
        assert_eq!(h.state().output_image.as_ref().unwrap().data, preview.data);
    }
    let save_path = s.directory.path().join("retained.png");
    save_retained(h, &save_path);
    assert!(
        h.state()
            .status
            .to_string()
            .starts_with("Saved retained.png"),
        "{}",
        h.state().status
    );
    let dialog_dirs: serde_json::Value = serde_json::from_slice(
        &std::fs::read(s.directory.path().join("dialog_dirs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        dialog_dirs["save_output"].as_str(),
        s.directory.path().to_str()
    );
    let expected_saved = image_io::convert_image(
        &preview,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&save_path, &expected_saved);
    reveal_control(h, "settings.preview_max_size");
    edit(h, "preview max size", "256");
    close_settings(h);
    let scan = scan(h);
    assert_eq!((scan.width, scan.height), (160, 80));
    let scan_operation = h.state().source_render.as_ref().unwrap().operation_id;
    // A size edit schedules automatic Preview. Negative conversion retains
    // the exact full frame; only its viewer raster uses the size limit.
    reveal_control(h, "settings.preview_max_size");
    edit(h, "preview max size", "128");
    assert_eq!(h.state().params.settings.preview_max_size, 128);
    assert!(h.query_by_label("Update").is_none());
    assert!(h.query_by_label("auto preview").is_none());
    close_settings(h);
    let bounded = self::preview(h);
    assert_eq!((bounded.width, bounded.height), (160, 80));
    assert_eq!(h.state().retained_kind, Some(RenderKind::Preview));
    assert_ne!(
        h.state().source_render.as_ref().unwrap().operation_id,
        scan_operation
    );
    open_export(h);
    assert!(h.state().export_dialog.is_open());
    choose(h, "Format", "TIFF");
    choose(h, "Bit depth", "32 bit");
    choose(h, "Backend", ExportBackend::Gpu.label());
    click(h, "Cancel");
    assert!(!h.state().export_dialog.is_open());
    assert_eq!(h.state().export_options.format, export::ExportFormat::Png);
    assert_eq!(h.state().export_options.backend, ExportBackend::Cpu);
    assert_eq!(h.state().export_options.depth, BitDepth::Eight);
    assert!(h.state().export_job.is_none());
    // The native chooser itself remains a native-gate concern. Its confirmed
    // result enters exactly the same production path used by the modal submit.
    let export_path = s.directory.path().join("full.png");
    let options = h.state().export_options.clone();
    let ctx = h.ctx.clone();
    h.state_mut()
        .start_export_to_path(&ctx, options, &export_path);
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.to_string().starts_with("Exported"),
        "{}",
        h.state().status
    );
    let expected_export = image_io::convert_image(
        &scan,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&export_path, &expected_export);
    assert_eq!(h.state().params.scanner.scan_output, "negative_to_positive");
    // Extension correction must never silently overwrite an existing file.
    let before = std::fs::read(&export_path).unwrap();
    let options = h.state().export_options.clone();
    h.state_mut().start_export_to_path(
        &ctx,
        options,
        &s.directory.path().join("full.wrong-extension"),
    );
    assert!(h.state().export_job.is_none());
    assert!(
        h.state()
            .status
            .to_string()
            .starts_with("Export destination already exists:")
    );
    assert_eq!(std::fs::read(&export_path).unwrap(), before);
    h.state_mut()
        .apply_state_with_source(accepted.clone(), |_| {})
        .unwrap();
    await_input(h);
    let mut float_options = h.state().export_options.clone();
    float_options.format = export::ExportFormat::Tiff;
    float_options.depth = BitDepth::ThirtyTwo;
    float_options.saving_cctf_encoding = false;
    let float_path = s.directory.path().join("full-float.tiff");
    h.state_mut()
        .start_export_to_path(&ctx, float_options, &float_path);
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.to_string().starts_with("Exported"),
        "{}",
        h.state().status
    );
    assert_float_tiff(&float_path);
    let float_output = image_io::load(&float_path).unwrap().image;
    let linear = image_io::convert_image(
        &scan,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        false,
    )
    .unwrap();
    assert_eq!((float_output.width, float_output.height), (160, 80));
    for (&actual, &expected) in float_output.data.iter().zip(&linear.data) {
        assert!((actual as f64 - expected as f64).abs() < 2e-5);
    }
    let persisted = s.directory.path().join("persisted.json");
    h.state().current_state().unwrap().save(&persisted).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&persisted).unwrap()).unwrap();
    assert_eq!(saved["scanner"]["scan_output"], "negative_to_positive");
    assert_eq!(
        saved["rust"]["runtime"]["scanner"]["scan_output"],
        "negative_to_positive"
    );
    let mut restored = app(
        persisted,
        s.directory.path().join("asymmetric-ramp.png"),
        s.directory.path().join("restored-browser.json"),
    );
    assert_eq!(restored.params.workflow.route, "input > film > scan");
    assert_eq!(restored.params.scanner.scan_output, "negative_to_positive");
    assert_eq!(restored.params.scanner.lens_blur, 0.0);
    assert_eq!(restored.params.scanner.unsharp_mask[0], 1.5);
    assert_eq!(restored.film_name, "kodak_portra_400");
    eframe::App::on_exit(&mut restored, None);
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input");
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
}

#[test]
fn fotufilm_engine_selection_reaches_the_native_app_runtime() {
    use spektrafilm_core::params::GrainEngine;
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input > film > scan");
    reveal_control(h, "extras.simulation.film_stock");
    let profile = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    let profile_label = profile.info.name.as_deref().unwrap_or("kodak_portra_400");
    choose(h, "film profile", profile_label);
    // Selecting a stock intentionally resets its workflow.
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input > film > scan");
    search_controls(h, "Grain");
    if !h.state().params.film_render.grain.active {
        click(h, "Enable grain");
    }
    assert!(h.state().params.film_render.grain.active);
    assert_eq!(h.state().params.film_render.grain.engine, GrainEngine::Dye);
    choose(h, "Engine", "Spektra");
    let spektra = preview(h);
    choose(h, "Engine", "Fotufilm Film");
    assert_eq!(
        h.state().params.film_render.grain.engine,
        GrainEngine::Fotufilm
    );
    assert_eq!(h.state().film_name, "kodak_portra_400");
    assert!(h.query_by_label("Layer amounts").is_some());
    // The harness deactivates stochastic effects, so the selected engine must
    // remain an exact bypass; the release-binary CLI smoke exercises the
    // prepared bank and residual sampling.
    let fotufilm = preview(h);
    assert_eq!((fotufilm.width, fotufilm.height), (128, 64));
    assert_eq!(
        fotufilm.data, spektra.data,
        "stochastic deactivation must bypass the selected grain engine"
    );
}

#[test]
fn real_worker_accepts_only_latest_request_and_current_revision() {
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "input");
    let baseline = preview(h);
    let ctx = h.ctx.clone();
    // Hold the worker's existing start marker while UI events change the
    // revision. This schedules real work deterministically, not a fake response.
    let active = h.state().render_worker.active.clone();
    let start_guard = active.lock();
    h.state_mut().dispatch_render(&ctx);
    let stale_revision = h.state().render_job.as_ref().unwrap().parameter_revision;
    choose(h, "workflow", "input > film > scan");
    assert!(h.state().parameter_revision > stale_revision);
    drop(start_guard);
    preview(h);
    assert_eq!(
        h.state().source_render.as_ref().unwrap().parameter_revision,
        Some(h.state().parameter_revision),
        "only the latest automatic request may publish output"
    );
    let start_guard = active.lock();
    h.state_mut().dispatch_render(&ctx);
    let first_request = h.state().render_job.as_ref().unwrap().request_id;
    // The worker takes the preview before blocking on its start marker. Keep
    // that preview in flight so submitting a full scan cannot replace it in
    // the pending slot; both real responses must reach the App.
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().render_worker.state.0.lock().is_some() {
        assert!(
            Instant::now() < deadline,
            "worker did not take first request"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    h.state_mut().full_scan_requested = true;
    h.state_mut().dispatch_render(&ctx);
    assert!(h.state().render_job.as_ref().unwrap().request_id > first_request);
    drop(start_guard);
    wait_until(h, |a| {
        a.render_job.is_none()
            && a.output_image
                .as_ref()
                .is_some_and(|out| !Arc::ptr_eq(out, &baseline))
    });
    assert!(edge_difference(h.state().output_image.as_ref().unwrap()) < -0.01);
    let accepted = h.state().output_image.as_ref().unwrap();
    assert_eq!((accepted.width, accepted.height), (160, 80));
    assert!(h.state().status.to_string().starts_with("Scan ·"));
}

#[cfg(feature = "gui-snapshots")]
mod layout_snapshots {
    use super::*;

    #[test]
    fn export_modal_offscreen_layout() {
        let mut s = session();
        let h = &mut s.harness;
        open_export(h);
        choose(h, "Format", "TIFF");
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "Compression")
                .is_some()
        );
        assert!(h.query_by_label("Apply saving CCTF encoding").is_some());
        // Attach the offscreen renderer only in this opt-in gate; render the
        // real dialog's TIFF-specific controls selected above.
        let dimensions = h
            .state()
            .image
            .as_ref()
            .map(|image| [image.width(), image.height()])
            .unwrap_or([0, 0]);
        let mut modal = Harness::builder()
            .with_size(egui::vec2(700.0, 650.0))
            .wgpu()
            .build_state(
                |ctx, dialog: &mut ExportDialog| {
                    theme::apply(ctx);
                    dialog.show(ctx, dimensions, None);
                },
                std::mem::take(&mut h.state_mut().export_dialog),
            );
        modal.run_steps(3);
        let image = modal.render().expect("offscreen export modal rendering");
        assert_eq!((image.width(), image.height()), (700, 650));
        let mut rows = Vec::new();
        for label in [
            "Format",
            "Bit depth",
            "Compression",
            "Saving color space",
            "Backend",
            "Apply saving CCTF encoding",
            "Export file…",
            "Cancel",
        ] {
            let role = match label {
                "Apply saving CCTF encoding" => egui::accesskit::Role::Switch,
                "Export file…" | "Cancel" => egui::accesskit::Role::Button,
                _ => egui::accesskit::Role::ComboBox,
            };
            let bounds = modal
                .get_by_role_and_label(role, label)
                .raw_bounds()
                .unwrap();
            assert!(
                bounds.x0 >= 0.0 && bounds.y0 >= 0.0 && bounds.x1 <= 700.0 && bounds.y1 <= 650.0,
                "{label} is clipped: {bounds:?}"
            );
            assert!(bounds.x1 > bounds.x0 && bounds.y1 > bounds.y0);
            let region_colors: std::collections::HashSet<_> = (bounds.y0.ceil() as u32
                ..bounds.y1.floor() as u32)
                .flat_map(|y| {
                    let image = &image;
                    (bounds.x0.ceil() as u32..bounds.x1.floor() as u32)
                        .map(move |x| image.get_pixel(x, y).0)
                })
                .collect();
            assert!(
                region_colors.len() > 2,
                "{label} must have visible offscreen pixels"
            );
            if !matches!(label, "Export file…" | "Cancel") {
                rows.push((label, bounds));
            }
        }
        for (index, (label, bounds)) in rows.iter().enumerate() {
            for (other, other_bounds) in &rows[..index] {
                assert!(
                    bounds.x1 <= other_bounds.x0
                        || other_bounds.x1 <= bounds.x0
                        || bounds.y1 <= other_bounds.y0
                        || other_bounds.y1 <= bounds.y0,
                    "modal rows overlap: {label} and {other}"
                );
            }
        }
        // The dialog must state what the chosen settings will write; the row
        // is a read-only label, so it carries no widget of its own to query.
        assert!(
            modal
                .query_by_label_contains("Estimated output size")
                .is_some(),
            "the export dialog must state the estimated output size"
        );
        let colors: std::collections::HashSet<_> = image.pixels().map(|p| p.0).collect();
        assert!(
            colors.len() > 20,
            "modal text and controls must render, not just a background"
        );
        save_snapshot(&image, "export-modal.png");
    }

    /// Render every stage at the supported size and inspect actionable geometry.
    #[test]
    fn app_window_stage_actions_remain_inside_the_native_viewport() {
        let mut s = snapshot_session();
        let h = &mut s.harness;
        preview(h);
        settle_animation(h);
        click(h, "Image analysis");
        wait_until(h, analysis_output_ready);
        use spektrafilm_backend::analysis::AnalysisMode;
        for (mode, expected) in [
            ("Histogram", AnalysisMode::Histogram),
            ("Waveform", AnalysisMode::Waveform),
            ("RGB parade", AnalysisMode::Parade),
            ("Vectorscope", AnalysisMode::Vectorscope),
        ] {
            match expected {
                AnalysisMode::Parade => choose(h, "Waveform display", mode),
                _ => {
                    click(h, "Analysis mode");
                    if expected == AnalysisMode::Vectorscope {
                        click(h, "Advanced");
                    }
                    click(h, mode);
                }
            }
            wait_until(h, |app| {
                analysis_output_ready(app)
                    && app.analysis.result.as_ref().unwrap().identity.mode == expected
            });
            h.run_steps(3);
            let image = h.render().expect("offscreen analysis render");
            assert_eq!((image.width(), image.height()), (1460, 980));
            save_snapshot(&image, &format!("analysis-{mode}.png"));
        }
        click(h, "Image analysis");
        h.run_steps(3);
        save_snapshot(
            &h.render().expect("collapsed analysis render"),
            "analysis-collapsed.png",
        );
        for stage in controls::Stage::ALL {
            navigate_stage(h, stage);
            h.run_steps(3);
            let image = h.render().expect("offscreen inspector render");
            assert_eq!((image.width(), image.height()), (1460, 980));
            for label in [
                "Open",
                "Undo",
                "Redo",
                "Settings",
                "Export",
                "SCAN",
                "SAVE",
                "Search controls",
            ] {
                let bounds = h
                    .get_by_label(label)
                    .node()
                    .raw_bounds()
                    .expect("action bounds");
                assert!(
                    bounds.x0 >= 0.0
                        && bounds.y0 >= 0.0
                        && bounds.x1 <= 1460.0
                        && bounds.y1 <= 980.0,
                    "unreachable action {label}: {bounds:?}"
                );
            }
            save_snapshot(
                &image,
                &format!("stage-{}.png", stage.label().replace(' ', "-")),
            );
        }
        open_settings(h);
        let image = h.render().expect("offscreen settings render");
        save_snapshot(&image, "settings.png");
    }
}

/// A session whose data directory is a symlink farm over the repository data
/// plus a real `scan-styles/` the test authors. Nothing ships a style, so an
/// end-to-end style check needs a directory the test controls; symlinks keep
/// the heavy `luts`/`profiles` payloads shared instead of copied, and no test
/// ever writes into the repository data dir.
fn style_session(styles: &[(&str, &str)]) -> Session {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    std::fs::create_dir_all(data.join("scan-styles")).unwrap();
    for entry in std::fs::read_dir(data_dir()).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "scan-styles" {
            continue;
        }
        std::os::unix::fs::symlink(entry.path(), data.join(name)).unwrap();
    }
    for (name, body) in styles {
        std::fs::write(data.join("scan-styles").join(format!("{name}.json")), body).unwrap();
    }
    let (image_path, _) = deterministic_inputs(directory.path());
    // The state document carries no data-dir reference, so the shared
    // deterministic state is reused verbatim.
    let state_path = deterministic_state(directory.path());
    let browser_path = directory.path().join("browser.json");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 1400.0))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            App::new_with_browser_path(
                Arc::new(CpuBackend),
                Some(image_path),
                Some(state_path),
                None,
                browser_path,
                data,
            )
        });
    await_input(&mut harness);
    assert_eq!(harness.state().image.as_ref().unwrap().width(), 160);
    click(&mut harness, "Collapse or expand Presets");
    Session { harness, directory }
}

/// One channel of a scan style: the same logistic tone triple the neutral
/// reference uses, written as the `data/scan-styles/<name>.json` shape.
const PROBE_STYLE_BODY: &str = r#"{"channels":[
 {"brightness_ev":1.2,"contrast":0.9,"dmax":1.8},
 {"brightness_ev":1.2,"contrast":0.9,"dmax":1.8},
 {"brightness_ev":1.2,"contrast":0.9,"dmax":1.8}
]}"#;

#[test]
fn conversion_style_combo_offers_installed_styles_and_renders_them() {
    // No style ships with the install, so this session authors one in a
    // temporary data directory the test controls.
    let mut s = style_session(&[("warm-probe", PROBE_STYLE_BODY)]);
    let h = &mut s.harness;
    assert_eq!(
        h.state().params.scanner.conversion_style,
        "standard",
        "the shipped default is the automatic standard"
    );
    reveal_control(h, "extras.simulation.film_stock");
    let profile = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    choose(h, "film profile", profile.info.name.as_deref().unwrap());
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input > film > scan");
    search_controls(h, "Scanner");
    choose(h, "scan output", "Negative to positive");
    // The selector lists exactly what the data directory carries: the
    // installed style is offered, and the neutral reference is too.
    scroll_controls_into_view(h, "conversion style");
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "conversion style")
        .focus();
    h.run_steps(3);
    h.press_key(egui::Key::Space);
    h.run_steps(3);
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Button, "warm-probe")
            .is_some()
    );
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Button, "standard")
            .is_some()
    );
    h.press_key(egui::Key::Escape);
    h.run_steps(3);
    let reference = preview(h);
    choose(h, "conversion style", "warm-probe");
    assert_eq!(h.state().params.scanner.conversion_style, "warm-probe");
    let styled = preview(h);
    assert_ne!(
        styled.data, reference.data,
        "an installed style must change the rendered capture"
    );
    choose(h, "conversion style", "standard");
    assert_eq!(h.state().params.scanner.conversion_style, "standard");
    let unstyled = preview(h);
    assert_eq!(unstyled.data, reference.data);
}

#[test]
fn scanner_state_rejection_uses_the_neutral_scan_model() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "extras.simulation.film_stock");
    let profile = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    choose(h, "film profile", profile.info.name.as_deref().unwrap());
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input > film > scan");
    search_controls(h, "Scanner");
    // The readout light is a product constant; it is not a control on any
    // route. The retired-control checks run on the direct scan this route
    // defaults to.
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
    for label in [
        "receiver profile",
        "scanner lamp",
        "scanner auto setup",
        "scanner finish",
        "finish strength",
        "brightness",
        "balance R",
        "contrast",
        "maximum density",
        "film-scan black point R",
        "film-scan white point R",
        "white and black correction",
    ] {
        assert!(h.query_by_label(label).is_none(), "retired control {label}");
    }
    let neutral = preview(h);
    assert!(neutral.data.iter().all(|x| x.is_finite()));
    let accepted = h.state().current_state().unwrap();
    let accepted_params = h.state().params.clone();
    let accepted_input = h.state().image.as_ref().unwrap().clone();
    let accepted_output = h.state().output_image.as_ref().unwrap().clone();
    let revision = h.state().parameter_revision;
    let epoch = h.state().calibration_epoch;
    for (key, value) in [
        ("lens_blur", serde_json::json!("not-a-number")),
        ("scan_output", serde_json::json!("positive_scan")),
    ] {
        let mut invalid = accepted.sections.clone();
        invalid["scanner"][key] = value;
        let result = state::GuiState::from_value(invalid)
            .and_then(|state| h.state_mut().apply_state_with_source(state, |_| {}));
        assert!(result.is_err(), "accepted incompatible {key}");
        assert_eq!(h.state().params, accepted_params);
        assert_eq!(
            h.state().current_state().unwrap().sections,
            accepted.sections
        );
        assert_eq!(h.state().parameter_revision, revision);
        assert_eq!(h.state().calibration_epoch, epoch);
        assert!(
            h.state()
                .image
                .as_ref()
                .unwrap()
                .same_source(&accepted_input)
        );
        assert!(Arc::ptr_eq(
            h.state().output_image.as_ref().unwrap(),
            &accepted_output
        ));
    }
    {
        // An unknown stock cannot resolve.
        let (section, key, value) = ("simulation", "film_stock", "not_a_film_profile");
        let mut invalid = accepted.sections.clone();
        invalid[section][key] = serde_json::json!(value);
        let result = state::GuiState::from_value(invalid)
            .and_then(|state| h.state_mut().apply_state_with_source(state, |_| {}));
        assert!(result.is_err(), "accepted incompatible {key}");
        assert_eq!(h.state().params, accepted_params);
        assert_eq!(h.state().parameter_revision, revision);
        assert_eq!(h.state().calibration_epoch, epoch);
    }
    let persisted = s.directory.path().join("scanner-state.json");
    accepted.save(&persisted).unwrap();
    let mut restored = app(
        persisted,
        s.directory.path().join("asymmetric-ramp.png"),
        s.directory.path().join("scanner-browser.json"),
    );
    await_app_input(&mut restored, &egui::Context::default());
    assert_eq!(restored.params.scanner, accepted_params.scanner);
    eframe::App::on_exit(&mut restored, None);
    let save_path = s.directory.path().join("scanner-preview.png");
    save_retained(h, &save_path);
    let expected = image_io::convert_image(
        &accepted_output,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&save_path, &expected);
    let scan = scan(h);
    assert_eq!((scan.width, scan.height), (160, 80));
    let export_path = s.directory.path().join("lamp-full.png");
    let ctx = h.ctx.clone();
    let options = h.state().export_options.clone();
    h.state_mut()
        .start_export_to_path(&ctx, options, &export_path);
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.to_string().starts_with("Exported"),
        "{}",
        h.state().status
    );
    let expected = image_io::convert_image(
        &scan,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&export_path, &expected);
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input");
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
    choose(h, "workflow", "input > film > scan");
    search_controls(h, "Scanner");
    choose(h, "scan output", "Negative to positive");
    reveal_control(h, "extras.simulation.film_stock");
    let slide = profile::load_profile_by_name(&data_dir(), "kodak_ektachrome_100").unwrap();
    choose(h, "film profile", slide.info.name.as_deref().unwrap());
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
    preview(h);
}

#[test]
fn american_reminiscence_browser_restores_virtual_exposure_and_render() {
    let mut s = session();
    let h = &mut s.harness;
    h.state_mut().params.camera.exposure_compensation_ev = -1.0;
    h.state_mut().params.camera.auto_exposure = true;
    click(h, "Collapse or expand Presets");
    click(h, "American Reminiscence");
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 0.5);
    assert!(!h.state().params.camera.auto_exposure);
    assert!(
        h.query_by_label("American Reminiscence • applied")
            .is_some()
    );
    let initial = preview(h);
    assert!(edge_difference(&initial) > 0.01);
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "Exposure compensation", "-1.0");
    assert!(
        h.query_by_label("American Reminiscence • modified")
            .is_some()
    );
    let changed = preview(h);
    assert!(
        initial
            .data
            .iter()
            .zip(&changed.data)
            .any(|(a, b)| (*a as f64 - *b as f64).abs() > 0.001)
    );
    click(h, "American Reminiscence • modified");
    let restored = preview(h);
    assert!(
        h.query_by_label("American Reminiscence • applied")
            .is_some()
    );
    for (a, b) in initial.data.iter().zip(&restored.data) {
        assert!((*a as f64 - *b as f64).abs() < 1e-6);
    }
}
