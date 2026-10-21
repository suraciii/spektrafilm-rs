//! Real App acceptance without a display server or a native file chooser.
//! Only confirmed chooser paths cross the explicit Save/Export boundary here.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use spektrafilm_backend::cpu_backend::CpuBackend;
use tempfile::TempDir;
mod calibration;
mod dialogs;
mod files;
mod lifecycle;
mod pages;
mod panels;
mod raw;
mod viewer;
mod workflow;

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
    initial.sections["simulation"]["auto_preview"] = serde_json::json!(false);
    // The viewer's 50-frame Polaroid reveal paints one full-canvas raster per
    // frame. That costs about 0.7 s per frame at this harness size, so sessions
    // start with the animation off. The animation keeps its coverage in
    // `headless::viewer::explicit_preview_and_scan_animate_but_automatic_parameter_update_does_not`,
    // which turns it back on through `viewer.settings.reveal`.
    initial.sections["rust"]["viewer"]["settings"]["reveal"] = serde_json::json!(false);
    let state_path = directory.join("initial.json");
    initial.save(&state_path).unwrap();
    state_path
}

/// Deterministic synthetic input shared by the headless fixtures: a ramp image
/// plus the deterministic state from [`deterministic_state`].
fn deterministic_inputs(directory: &Path) -> (PathBuf, PathBuf) {
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
    assert_eq!(harness.state().image.as_ref().unwrap().width, 160);
    click(&mut harness, "◀");
    Session { harness, directory }
}

/// A session built for offscreen pixel capture: the production window size, the
/// wgpu renderer, and the presets panel left in its default expanded state.
#[cfg(feature = "gui-snapshots")]
fn snapshot_session() -> Session {
    let directory = tempfile::tempdir().unwrap();
    let (image_path, state_path) = deterministic_inputs(directory.path());
    let browser_path = directory.path().join("browser.json");
    let harness = Harness::builder()
        .with_size(egui::vec2(1460.0, 980.0))
        .wgpu()
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            app(state_path, image_path, browser_path)
        });
    assert_eq!(harness.state().image.as_ref().unwrap().width, 160);
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
    scroll_controls_into_view(h, label);
    h.get_by_label(label).simulate_click();
    h.run_steps(3);
}

/// Scroll the controls panel until every node carrying `label` that lives in
/// that panel sits inside the visible band.
///
/// The panel reserves `theme::HEIGHT_ACTION_BAR_RESERVE` at its bottom so the
/// action bar never sits under the screen edge, so the usable band ends that
/// much above the screen bottom rather than at the screen edge.
fn scroll_controls_into_view(h: &mut Harness<'_, App>, label: &str) {
    let screen = h.ctx.screen_rect();
    // The band starts below the panel's own top padding, and ends above the
    // reserve the panel keeps clear for its action bar. The bounds kittest
    // reports are f64, so the whole band is compared in f64.
    let band_top = 40.0_f64;
    let band_bottom = f64::from(screen.bottom()) - f64::from(theme::HEIGHT_ACTION_BAR_RESERVE);
    let panel_left = f64::from(screen.right()) - f64::from(theme::WIDTH_CONTROL_PANEL);
    /// How many scrolls a target may need before the helper gives up. A page's
    /// controls are taller than the viewport by a bounded amount, so a handful
    /// of full-height scrolls always reaches the bottom.
    const SCROLL_ATTEMPTS: usize = 16;
    for _ in 0..SCROLL_ATTEMPTS {
        let needs_scroll = h
            .query_all_by_label(label)
            .filter_map(|node| node.node().raw_bounds())
            // Only nodes inside the controls panel are scrollable.
            .filter(|bounds| bounds.x0 >= panel_left)
            .any(|bounds| bounds.y0 <= band_top || bounds.y1 >= band_bottom);
        if !needs_scroll {
            return;
        }
        scroll_controls_down(h);
    }
}

/// Scroll the controls panel down so elements below the fold come into view.
fn scroll_controls_down(h: &mut Harness<'_, App>) {
    let screen = h.ctx.screen_rect();
    let pointer = egui::pos2(screen.right() - 160.0, screen.center().y);
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(pointer));
    h.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -600.0),
        modifiers: Default::default(),
    });
    h.run_steps(10);
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
}

/// Click the import button of one section. Each import section now names its
/// own action, so the section is disambiguated by its label alone.
fn import_file(h: &mut Harness<'_, App>, raw: bool) {
    click(h, "MAIN");
    click(h, if raw { "Import Raw" } else { "Import RGB" });
    let label = if raw {
        "Select RAW file…"
    } else {
        "Select RGB image…"
    };
    let buttons: Vec<_> = h.query_all_by_label(label).collect();
    assert_eq!(buttons.len(), 1, "one import action per section: {label}");
    buttons[0].simulate_click();
    h.run_steps(3);
}
/// Pick `option` from the combo labeled `label`.
///
/// The option is looked up by button role: a grain engine group is a
/// `CollapsingHeader` whose title carries the engine's label, so a plain
/// label query would match both the header and the popup option.
fn choose(h: &mut Harness<'_, App>, label: &str, option: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, label)
        .simulate_click();
    h.run_steps(3);
    if h.state().export_dialog.is_open() {
        h.get_by_role_and_label(egui::accesskit::Role::Button, option)
            .simulate_click();
        h.run_steps(3);
    } else {
        h.get_by_role_and_label(egui::accesskit::Role::Button, option)
            .click();
        h.run_steps(3);
        h.press_key(egui::Key::Escape);
        h.run_steps(3);
    }
}

fn edit(h: &mut Harness<'_, App>, label: &str, text: &str) {
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

fn wait_until(h: &mut Harness<'_, App>, condition: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT_DEADLINE;
    loop {
        h.step();
        if condition(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "App completion timed out: {}",
            h.state().status
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

fn render(h: &mut Harness<'_, App>, button: &str) -> Arc<ImageBuf> {
    // State loading can schedule a preview even with auto-preview off. Drain it
    // before clicking: a disabled action does not enqueue the requested render.
    wait_until(h, |a| {
        a.render_job.is_none()
            && a.export_job.is_none()
            && a.calibration_job.is_none()
            && !a.force_preview
    });
    let previous = h.state().output_image.clone();
    click(h, button);
    wait_until(h, |a| {
        !a.dirty
            && a.render_job.is_none()
            && a.output_image
                .as_ref()
                .is_some_and(|out| previous.as_ref().is_none_or(|old| !Arc::ptr_eq(old, out)))
    });
    assert!(
        h.state().status.starts_with(if button == "SCAN" {
            "Scan ·"
        } else {
            "Preview ·"
        }),
        "{}",
        h.state().status
    );
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

/// The export dialog "GPU" selection is an explicit request: a configuration
/// that still needs a host-executed stage must fail before publication instead
/// of being silently demoted to CPU. The harness preview backend is the CPU
/// backend, so this also covers opening an explicit WGPU backend for the job.
#[test]
fn explicit_gpu_export_fails_before_publication() {
    let mut s = session();
    let h = &mut s.harness;
    render(h, "PREVIEW");
    let mut state = h.state().current_state().unwrap();
    state.sections["input_image"]["input_cctf_decoding"] = serde_json::json!(true);
    state.sections["input_image"]["input_color_space"] = serde_json::json!("sRGB");
    h.state_mut().apply_state(state).unwrap();
    assert!(h.state().params.io.input_cctf_decoding);

    let destination = s.directory.path().join("explicit-gpu.png");
    let mut options = h.state().export_options.clone();
    options.backend = ExportBackend::Gpu;
    let ctx = h.ctx.clone();
    h.state_mut()
        .start_export_to_path(&ctx, options, &destination);
    wait_until(h, |a| a.export_job.is_none());
    let status = h.state().status.clone();
    assert!(
        status.contains("explicit GPU request") || status.contains("no WGPU adapter is available"),
        "{status}"
    );
    assert!(
        !destination.exists(),
        "a failed explicit GPU export published an output"
    );
}

#[test]
fn scanner_ui_real_preview_scan_persistence_save_and_export() {
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "input > film > scan");
    assert_eq!(h.state().params.workflow.route, "input > film > scan");
    click(h, "Profiles");
    let profile = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    let profile_label = profile.info.name.as_deref().unwrap_or("kodak_portra_400");
    choose(h, "film profile", profile_label);
    assert_eq!(h.state().film_name, "kodak_portra_400");
    // Selecting a stock intentionally resets its workflow; choose the eligible
    // scanner route after that production profile-selection behavior.
    choose(h, "workflow", "input > film > scan");
    click(h, "Scanner");
    choose(h, "scan output", "Preserve medium polarity");
    assert!(h.query_by_label("brightness").is_none());
    let direct = render(h, "PREVIEW");
    assert_eq!((direct.width, direct.height), (128, 64));
    choose(h, "scan output", "Negative to positive");
    assert_eq!(h.state().params.scanner.scan_output, "negative_to_positive");
    choose(h, "Negative conversion method", "separable");
    assert_eq!(
        h.state().params.scanner.negative_conversion_method,
        spektrafilm_core::params::NegativeConversionMethod::Separable
    );
    choose(h, "Negative conversion method", "density");
    assert!(h.query_by_label("brightness").is_some());
    edit(h, "brightness", "0");
    let positive = render(h, "PREVIEW");
    assert!(
        edge_difference(&direct) < -0.01,
        "negative capture must reverse the ramp"
    );
    assert!(
        edge_difference(&positive) > 0.01,
        "negative conversion must restore ramp polarity"
    );
    edit(h, "brightness", "1");
    assert_eq!(h.state().params.scanner.conversion_brightness_ev, 1.0);
    let brighter = render(h, "PREVIEW");
    assert!(mean(&brighter) > mean(&positive) + 0.01);
    edit(h, "balance R", "0.25");
    edit(h, "balance G", "-0.125");
    edit(h, "balance B", "0.5");
    assert_eq!(
        h.state().params.scanner.conversion_balance_ev,
        [0.25, -0.125, 0.5]
    );
    edit(h, "film-scan white point R", "0.95");
    edit(h, "film-scan black point R", "0.025");
    assert_eq!(h.state().params.scanner.white_point, [0.95, 1.0, 1.0]);
    assert_eq!(h.state().params.scanner.black_point, [0.025, 0.0, 0.0]);
    edit(h, "brightness", "not-a-number");
    assert_eq!(h.state().params.scanner.conversion_brightness_ev, 1.0);
    assert_eq!(
        h.get_by_label("brightness").value().as_deref(),
        Some("1.000")
    );
    choose(h, "scan output", "Preserve medium polarity");
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
    // The rest of this test exercises the negative-capture scanner path, so
    // select it through the real control rather than writing the parameter.
    choose(h, "scan output", "Negative to positive");
    choose(h, "scanner auto setup", "full");
    assert_eq!(h.state().params.scanner.scan_output, "negative_to_positive");
    assert_eq!(
        h.state().params.scanner.auto_setup,
        spektrafilm_core::params::ScannerAutoSetup::Full
    );
    let preview = render(h, "PREVIEW");
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
        let result =
            state::GuiState::load(&path).and_then(|state| h.state_mut().apply_state(state));
        let error = result.unwrap_err();
        assert!(format!("{error:#}").contains("positive_scan"), "{error:#}");
        assert_eq!(h.state().params, accepted_params);
        assert_eq!(h.state().parameter_revision, revision);
        assert_eq!(h.state().calibration_epoch, calibration_epoch);
        assert_eq!(
            h.state().current_state().unwrap().sections,
            accepted.sections
        );
        assert!(Arc::ptr_eq(
            h.state().image.as_ref().unwrap(),
            &accepted_input
        ));
        assert!(Arc::ptr_eq(
            h.state().output_image.as_ref().unwrap(),
            &accepted_output
        ));
        assert_eq!(h.state().output_image.as_ref().unwrap().data, preview.data);
    }
    let save_path = s.directory.path().join("retained.png");
    h.state_mut().save_to_path(&save_path);
    assert!(
        h.state().status.starts_with("Saved retained.png"),
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
    let scan = render(h, "SCAN");
    assert_eq!((scan.width, scan.height), (160, 80));
    // Export must ignore the most recently retained preview's reduced resolution.
    render(h, "PREVIEW");
    // The controls panel is taller than the viewport now that every numeric
    // row carries a slider line, so bring the action bar into view before
    // clicking it.
    scroll_controls_down(h);
    click(h, "Export options");
    scroll_controls_down(h);
    click(h, "Open export settings…");
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
        h.state().status.starts_with("Exported"),
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
    let sidecar_path = export_path.with_extension("scanner-setup.json");
    assert!(sidecar_path.is_file(), "scanner setup sidecar missing");
    let sidecar: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&sidecar_path).unwrap()).unwrap();
    assert_eq!(sidecar["policy"], "full");
    assert_eq!(sidecar["completeFrameMeasurement"], true);
    assert_eq!(sidecar["interpretationOnly"], true);
    assert_eq!(sidecar["acquisitionChangesModeled"], false);
    assert_eq!(sidecar["inputIdentity"]["width"], 160);
    assert_eq!(sidecar["inputIdentity"]["height"], 80);
    assert_eq!(h.state().params.scanner.scan_output, "negative_to_positive");
    let mut off_state = h.state().current_state().unwrap();
    off_state.sections["scanner"]["auto_setup"] = serde_json::json!("off");
    h.state_mut().apply_state(off_state).unwrap();
    let off_export = s.directory.path().join("off.png");
    let off_options = h.state().export_options.clone();
    h.state_mut()
        .start_export_to_path(&ctx, off_options, &off_export);
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.starts_with("Exported"),
        "{}",
        h.state().status
    );
    assert!(!off_export.with_extension("scanner-setup.json").exists());
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
            .starts_with("Export destination already exists:")
    );
    assert_eq!(std::fs::read(&export_path).unwrap(), before);
    h.state_mut().apply_state(accepted.clone()).unwrap();
    assert_eq!(
        h.state().params.scanner.auto_setup,
        spektrafilm_core::params::ScannerAutoSetup::Full
    );
    let mut float_options = h.state().export_options.clone();
    float_options.format = export::ExportFormat::Tiff;
    float_options.depth = BitDepth::ThirtyTwo;
    float_options.saving_cctf_encoding = false;
    let float_path = s.directory.path().join("full-float.tiff");
    h.state_mut()
        .start_export_to_path(&ctx, float_options, &float_path);
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.starts_with("Exported"),
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
    assert_eq!(restored.params.scanner.conversion_brightness_ev, 1.0);
    assert_eq!(
        restored.params.scanner.conversion_balance_ev,
        [0.25, -0.125, 0.5]
    );
    assert_eq!(restored.film_name, "kodak_portra_400");
    assert_eq!(restored.params.scanner.white_point, [0.95, 1.0, 1.0]);
    assert_eq!(restored.params.scanner.black_point, [0.025, 0.0, 0.0]);
    eframe::App::on_exit(&mut restored, None);
    choose(h, "workflow", "Finished RGB");
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
    assert!(h.query_by_label("brightness").is_none());
}

#[test]
fn fotufilm_engine_selection_reaches_the_native_app_runtime() {
    use spektrafilm_core::params::GrainEngine;
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "input > film > scan");
    click(h, "Profiles");
    let profile = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    let profile_label = profile.info.name.as_deref().unwrap_or("kodak_portra_400");
    choose(h, "film profile", profile_label);
    // Selecting a stock intentionally resets its workflow.
    choose(h, "workflow", "input > film > scan");
    click(h, "FILM");
    if !h.state().params.film_render.grain.active {
        click(h, "Enable grain");
    }
    assert!(h.state().params.film_render.grain.active);
    assert_eq!(
        h.state().params.film_render.grain.engine,
        GrainEngine::Spektra
    );
    let spektra = render(h, "PREVIEW");
    choose(h, "Engine", "Fotufilm Film");
    assert_eq!(
        h.state().params.film_render.grain.engine,
        GrainEngine::Fotufilm
    );
    assert_eq!(h.state().film_name, "kodak_portra_400");
    assert!(h.query_by_label("Layer amounts").is_some());
    assert!(
        h.query_by_label("Dye support (µm)").is_none(),
        "another engine's controls must not appear"
    );
    // The harness deactivates stochastic effects, so the selected engine must
    // remain an exact bypass; the release-binary CLI smoke exercises the
    // prepared bank and residual sampling.
    let fotufilm = render(h, "PREVIEW");
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
    choose(h, "workflow", "Finished RGB");
    let baseline = render(h, "PREVIEW");
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
    wait_until(h, |a| a.render_job.is_none());
    assert!(
        Arc::ptr_eq(&baseline, h.state().output_image.as_ref().unwrap()),
        "stale real worker response must not replace retained output"
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
    assert!(h.state().status.starts_with("Scan ·"));
}

#[cfg(feature = "gui-snapshots")]
mod layout_snapshots {
    use super::*;

    #[test]
    fn export_modal_offscreen_layout() {
        let mut s = session();
        let h = &mut s.harness;
        click(h, "Export options");
        click(h, "Open export settings…");
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
            .map(|image| [image.width, image.height])
            .unwrap_or([0, 0]);
        let mut modal = Harness::builder()
            .with_size(egui::vec2(700.0, 650.0))
            .wgpu()
            .build_state(
                |ctx, dialog: &mut ExportDialog| {
                    theme::apply(ctx);
                    dialog.show(ctx, dimensions);
                },
                std::mem::take(&mut h.state_mut().export_dialog),
            );
        modal.run_steps(3);
        let image = modal.render().expect("offscreen export modal rendering");
        assert_eq!((image.width(), image.height()), (700, 650));
        let mut rows = Vec::new();
        for label in [
            "Backend",
            "Format",
            "Bit depth",
            "Compression",
            "Saving color space",
            "Apply saving CCTF encoding",
            "Export file…",
            "Cancel",
        ] {
            let role = match label {
                "Apply saving CCTF encoding" => egui::accesskit::Role::CheckBox,
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
        for pair in rows.windows(2) {
            assert!(
                pair[0].1.y1 <= pair[1].1.y0,
                "modal rows overlap: {} and {}",
                pair[0].0,
                pair[1].0
            );
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

    /// Render the real App window for every page offscreen. The PNGs are layout
    /// evidence; they are written outside the repository when
    /// `SPEKTRAFILM_GUI_SNAPSHOT_DIR` is set.
    #[test]
    fn app_window_renders_every_page_offscreen() {
        let mut previous: Option<image::RgbaImage> = None;
        for label in ["MAIN", "FILM", "PRINT", "ADVANCED", "CONFIG"] {
            let mut s = snapshot_session();
            let h = &mut s.harness;
            click(h, label);
            render(h, "PREVIEW");
            settle_animation(h);
            // The histogram is binned off the UI thread; the PNG is layout
            // evidence for the readout, so wait for the distribution to land.
            wait_until(h, |a| a.histogram.cached_data().is_some());
            h.run_steps(3);
            let image = h.render().expect("offscreen app render");
            assert_eq!((image.width(), image.height()), (1460, 980));
            if let Some(previous) = &previous {
                assert_ne!(
                    previous.as_raw(),
                    image.as_raw(),
                    "page {label} rendered identically to the previous page"
                );
            }
            save_snapshot(&image, &format!("app-{label}.png"));
            previous = Some(image);
        }
    }
}

#[test]
fn digital_reference_real_app_selection_render_transactions_and_outputs() {
    use spektrafilm_core::params::{NegativeConversionMethod, ReceiverProfile, ScannerAutoSetup};
    let mut s = session();
    let h = &mut s.harness;
    click(h, "Profiles");
    let profile = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    choose(h, "film profile", profile.info.name.as_deref().unwrap());
    choose(h, "workflow", "input > film > scan");
    click(h, "Scanner");
    choose(h, "scan output", "Negative to positive");
    choose(h, "scanner auto setup", "full");
    edit(h, "brightness", "1");
    edit(h, "contrast", "1.5");
    edit(h, "film-scan black point R", "0.05");
    choose(
        h,
        "Negative conversion method",
        "Digital Reference (modeled)",
    );
    let scanner = &h.state().params.scanner;
    assert_eq!(
        scanner.negative_conversion_method,
        NegativeConversionMethod::DigitalReference
    );
    assert_eq!(scanner.receiver_profile, ReceiverProfile::DigitalReference);
    assert_eq!(scanner.auto_setup, ScannerAutoSetup::Off);
    assert_eq!(scanner.black_point, [0.0; 3]);
    assert_eq!(scanner.white_point, [1.0; 3]);
    let defaults = spektrafilm_core::params::ScannerParams::default();
    assert_eq!(
        scanner.conversion_brightness_ev,
        defaults.conversion_brightness_ev
    );
    assert_eq!(scanner.conversion_contrast, defaults.conversion_contrast);
    assert!(
        h.query_by_label("Reference Exposure: 18% neutral (fixed)")
            .is_some()
    );
    for label in [
        "receiver profile",
        "scanner auto setup",
        "brightness",
        "balance R",
        "contrast",
        "maximum density",
        "film-scan black point R",
        "film-scan white point R",
    ] {
        assert!(
            h.query_by_label(label).is_none(),
            "incompatible control {label}"
        );
    }
    let neutral = render(h, "PREVIEW");
    assert!(neutral.data.iter().all(|x| x.is_finite()));
    assert!(edge_difference(&neutral) > 0.0);
    edit(h, "Reference exposure EV", "1");
    let brighter = render(h, "PREVIEW");
    assert!(mean(&brighter) > mean(&neutral));
    edit(h, "Reference balance EV R", "0.25");
    edit(h, "Reference balance EV G", "-0.125");
    let balanced = render(h, "PREVIEW");
    assert_ne!(balanced.data, brighter.data);
    let accepted = h.state().current_state().unwrap();
    let accepted_params = h.state().params.clone();
    let accepted_input = h.state().image.as_ref().unwrap().clone();
    let accepted_output = h.state().output_image.as_ref().unwrap().clone();
    let revision = h.state().parameter_revision;
    let epoch = h.state().calibration_epoch;
    for (key, value) in [
        ("receiver_profile", serde_json::json!("legacy_colorimetric")),
        ("auto_setup", serde_json::json!("full")),
        ("black_point", serde_json::json!([0.1, 0.0, 0.0])),
        ("conversion_contrast", serde_json::json!(1.5)),
    ] {
        let mut invalid = accepted.sections.clone();
        invalid["scanner"][key] = value;
        let result =
            state::GuiState::from_value(invalid).and_then(|state| h.state_mut().apply_state(state));
        assert!(result.is_err(), "accepted incompatible {key}");
        assert_eq!(h.state().params, accepted_params);
        assert_eq!(
            h.state().current_state().unwrap().sections,
            accepted.sections
        );
        assert_eq!(h.state().parameter_revision, revision);
        assert_eq!(h.state().calibration_epoch, epoch);
        assert!(Arc::ptr_eq(
            h.state().image.as_ref().unwrap(),
            &accepted_input
        ));
        assert!(Arc::ptr_eq(
            h.state().output_image.as_ref().unwrap(),
            &accepted_output
        ));
        assert_eq!(
            h.get_by_label("Reference exposure EV").value().as_deref(),
            Some("1.000")
        );
    }
    for (section, key, value) in [
        ("simulation", "route", "input > convert-film > scan"),
        ("simulation", "film_stock", "kodak_ektachrome_100"),
    ] {
        let mut invalid = accepted.sections.clone();
        invalid[section][key] = serde_json::json!(value);
        let result =
            state::GuiState::from_value(invalid).and_then(|state| h.state_mut().apply_state(state));
        assert!(result.is_err(), "accepted incompatible {key}");
        assert_eq!(h.state().params, accepted_params);
        assert_eq!(h.state().parameter_revision, revision);
        assert_eq!(h.state().calibration_epoch, epoch);
        assert!(Arc::ptr_eq(
            h.state().image.as_ref().unwrap(),
            &accepted_input
        ));
        assert!(Arc::ptr_eq(
            h.state().output_image.as_ref().unwrap(),
            &accepted_output
        ));
    }
    let persisted = s.directory.path().join("reference-state.json");
    accepted.save(&persisted).unwrap();
    let mut restored = app(
        persisted,
        s.directory.path().join("asymmetric-ramp.png"),
        s.directory.path().join("reference-browser.json"),
    );
    assert_eq!(restored.params.scanner, accepted_params.scanner);
    eframe::App::on_exit(&mut restored, None);
    let mut extension_only = accepted.sections.clone();
    for key in [
        "receiver_profile",
        "negative_conversion_method",
        "reference_exposure_ev",
        "reference_balance_ev",
    ] {
        extension_only["scanner"]
            .as_object_mut()
            .unwrap()
            .remove(key);
    }
    let extension_state = state::GuiState::from_value(extension_only).unwrap();
    assert_eq!(
        extension_state.runtime_params().unwrap().scanner,
        accepted_params.scanner
    );
    h.state_mut().apply_state(extension_state).unwrap();
    h.run_steps(3);
    choose(h, "scanner finish", "lab_scan");
    edit(h, "finish strength", "0");
    let unfinished = render(h, "PREVIEW");
    assert_eq!(unfinished.data, balanced.data);
    choose(h, "scanner finish", "neutral");
    let save_path = s.directory.path().join("reference-preview.png");
    h.state_mut().save_to_path(&save_path);
    let expected = image_io::convert_image(
        &balanced,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&save_path, &expected);
    let scan = render(h, "SCAN");
    assert_eq!((scan.width, scan.height), (160, 80));
    render(h, "PREVIEW");
    let export_path = s.directory.path().join("reference-full.png");
    let ctx = h.ctx.clone();
    let options = h.state().export_options.clone();
    h.state_mut()
        .start_export_to_path(&ctx, options, &export_path);
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.starts_with("Exported"),
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
    choose(h, "Negative conversion method", "density");
    assert_eq!(
        h.state().params.scanner.receiver_profile,
        ReceiverProfile::LegacyColorimetric
    );
    assert_eq!(h.state().params.scanner.reference_exposure_ev, 0.0);
    assert_eq!(h.state().params.scanner.reference_balance_ev, [0.0; 3]);
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "receiver profile")
            .is_some()
    );
    assert!(h.query_by_label("Reference exposure EV").is_none());
    choose(
        h,
        "Negative conversion method",
        "Digital Reference (modeled)",
    );
    choose(h, "workflow", "input > convert-film > scan");
    assert_eq!(
        h.state().params.scanner.negative_conversion_method,
        NegativeConversionMethod::Density
    );
    assert_eq!(
        h.state().params.scanner.receiver_profile,
        ReceiverProfile::LegacyColorimetric
    );
    h.get_by_role_and_label(
        egui::accesskit::Role::ComboBox,
        "Negative conversion method",
    )
    .simulate_click();
    h.run_steps(3);
    assert!(h.query_by_label("Digital Reference (modeled)").is_none());
    h.press_key(egui::Key::Escape);
    h.run_steps(3);
    choose(h, "workflow", "input > film > scan");
    choose(
        h,
        "Negative conversion method",
        "Digital Reference (modeled)",
    );
    choose(h, "workflow", "Finished RGB");
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
    assert_eq!(
        h.state().params.scanner.negative_conversion_method,
        NegativeConversionMethod::Density
    );
    assert_eq!(
        h.state().params.scanner.receiver_profile,
        ReceiverProfile::LegacyColorimetric
    );
    render(h, "PREVIEW");
    choose(h, "workflow", "input > film > scan");
    choose(h, "scan output", "Negative to positive");
    choose(
        h,
        "Negative conversion method",
        "Digital Reference (modeled)",
    );
    if h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "film profile")
        .is_none()
    {
        click(h, "Profiles");
    }
    let slide = profile::load_profile_by_name(&data_dir(), "kodak_ektachrome_100").unwrap();
    choose(h, "film profile", slide.info.name.as_deref().unwrap());
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
    assert_eq!(
        h.state().params.scanner.negative_conversion_method,
        NegativeConversionMethod::Density
    );
    assert_eq!(
        h.state().params.scanner.receiver_profile,
        ReceiverProfile::LegacyColorimetric
    );
    render(h, "PREVIEW");
}

#[test]
fn american_reminiscence_browser_restores_virtual_exposure_and_render() {
    let mut s = session();
    let h = &mut s.harness;
    h.state_mut().params.camera.exposure_compensation_ev = -1.0;
    h.state_mut().params.camera.auto_exposure = true;
    click(h, "▶");
    click(h, "American Reminiscence");
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 0.5);
    assert!(!h.state().params.camera.auto_exposure);
    assert!(
        h.query_by_label("American Reminiscence • applied")
            .is_some()
    );
    let initial = render(h, "PREVIEW");
    assert!(edge_difference(&initial) > 0.01);
    edit(h, "exposure compensation ev", "-1.0");
    assert!(
        h.query_by_label("American Reminiscence • modified")
            .is_some()
    );
    let changed = render(h, "PREVIEW");
    assert!(
        initial
            .data
            .iter()
            .zip(&changed.data)
            .any(|(a, b)| (*a as f64 - *b as f64).abs() > 0.001)
    );
    click(h, "American Reminiscence • modified");
    let restored = render(h, "PREVIEW");
    assert!(
        h.query_by_label("American Reminiscence • applied")
            .is_some()
    );
    for (a, b) in initial.data.iter().zip(&restored.data) {
        assert!((*a as f64 - *b as f64).abs() < 1e-6);
    }
}
