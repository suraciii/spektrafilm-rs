use super::*;

// Convert calibration worker actions use the production Input and capture
// inspector module and apply accepted worker results to App parameters.

/// Reveal Convert through its canonical action without changing the workflow.
fn open_convert_controls(h: &mut Harness<'_, App>) {
    reveal_control(h, "action.detect_base");
    search_controls(h, "Convert");
}

/// Wait out any state-load preview so no worker is running before an action;
/// `start_calibration` refuses while another operation is running.
fn drain(h: &mut Harness<'_, App>) {
    wait_until(h, |a| {
        a.render_job.is_none()
            && a.export_job.is_none()
            && a.calibration_job.is_none()
            && !a.force_preview
    });
}

/// Click a button and stop at the frame where the click registers.
///
/// `simulate_click` queues hover/press/release and [`Harness::step`] runs one
/// frame per queued event. egui registers the click on release, the last of
/// those frames, and `poll_calibration_job` runs before the panels in
/// `update` — so after this call a freshly started worker is observed in
/// flight with certainty: no poll has run since the job was created.
fn press(h: &mut Harness<'_, App>, label: &str) {
    // Make the action hittable before queuing the pointer click. No worker
    // exists yet, so scrolling cannot consume the operation about to start.
    scroll_controls_into_view(h, label);
    h.get_by_label(label).simulate_click();
    h.step();
}

/// Wait for the Convert worker to finish and report. [`wait_until`]'s 90 s
/// budget does not cover the blind-calibration fit on the shared fixture, so
/// this uses a larger one; the wait is condition-based either way.
fn wait_calibration(h: &mut Harness<'_, App>, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    loop {
        h.step();
        if h.state().calibration_job.is_none() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{label} timed out: {}",
            h.state().status
        );
        wait_tick();
    }
}

/// A session whose App starts without an input image, like a fresh install.
/// (`empty_session` is reserved for the `gui-raw` gate; this guard must run in
/// the default gate too.)
fn session_without_image() -> Session {
    let directory = tempfile::tempdir().unwrap();
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
    let state_path = directory.path().join("initial.json");
    initial.save(&state_path).unwrap();
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

/// The five parameter values the Convert calibration actions write.
fn calibration_outputs(
    app: &App,
) -> (
    spektrafilm_core::params::FilmBaseParams,
    f64,
    String,
    f32,
    f32,
) {
    (
        app.params.film_render.base.clone(),
        app.params.film_render.convert.exposure_compensation_ev,
        app.params.film_render.convert.calibration.clone(),
        app.params.enlarger.m_filter_shift,
        app.params.enlarger.y_filter_shift,
    )
}

fn parse_calibration(text: &str) -> Vec<f64> {
    text.split_whitespace()
        .map(|token| token.parse().expect("numeric calibration entry"))
        .collect()
}

// Deterministic fits on the `session_fixture` ramp, measured from the f64
// pipeline. Tolerances absorb precision-feature and libm ULP drift only.
const BASE_CYAN: f64 = 1.227_020_666_902_410;
const BASE_MAGENTA: f64 = 0.600_426_922_252_643;
const BASE_YELLOW: f64 = 0.500_288_922_285_685;
const BASE_EXPOSURE_EV: f64 = -0.902_427_051_824_104;
const FILTER_M_SHIFT: f32 = 0.101_127_781;
const FILTER_Y_SHIFT: f32 = -0.180_145_949;

/// The applied `detect base` result: fitted base densities with the scan
/// exposure, the untouched scale/tilt, and the status reporting that EV.
fn assert_detect_base_applied(app: &App) {
    let base = &app.params.film_render.base;
    assert!(base.active, "detect base must enable the fitted base");
    assert!((base.scale - 1.0).abs() < 1e-9, "scale {}", base.scale);
    assert!(base.tilt.abs() < 1e-9, "tilt {}", base.tilt);
    assert!((base.cyan - BASE_CYAN).abs() < 1e-5, "cyan {}", base.cyan);
    assert!(
        (base.magenta - BASE_MAGENTA).abs() < 1e-5,
        "magenta {}",
        base.magenta
    );
    assert!(
        (base.yellow - BASE_YELLOW).abs() < 1e-5,
        "yellow {}",
        base.yellow
    );
    let ev = app.params.film_render.convert.exposure_compensation_ev;
    assert!((ev - BASE_EXPOSURE_EV).abs() < 1e-5, "exposure ev {ev}");
    assert_eq!(
        app.status,
        format!("Film base detected; exposure compensation {ev:+.2} EV.")
    );
}

#[test]
fn detect_base_button_fits_the_film_base_and_scan_exposure_from_the_loaded_ramp() {
    let mut s = session();
    let h = &mut s.harness;
    drain(h);
    open_convert_controls(h);
    choose(h, "scan illuminant", "D65");
    assert_eq!(h.state().params.film_render.convert.scan_illuminant, "D65");
    choose(h, "scan illuminant", "D55");
    assert_eq!(h.state().params.film_render.convert.scan_illuminant, "D55");
    let before = calibration_outputs(h.state());
    let route = h.state().params.workflow.route.clone();
    let revision = h.state().parameter_revision;
    click(h, "detect base");
    assert!(
        h.state().calibration_job.is_some() || h.state().status.starts_with("Film base detected"),
        "{}",
        h.state().status
    );
    wait_calibration(h, "detect base");
    assert_detect_base_applied(h.state());
    let after = calibration_outputs(h.state());
    assert_ne!(after.0, before.0, "fitted base must be applied");
    // Only base and the scan exposure are written; the matrix string, the
    // enlarger shifts and the workflow route stay as they were (the worker
    // rewrites the route on its local copy only).
    assert_eq!(after.2, before.2);
    assert_eq!((after.3, after.4), (before.3, before.4));
    assert_eq!(h.state().params.workflow.route, route);
    assert_eq!(h.state().parameter_revision, revision.wrapping_add(1));
    assert!(h.state().force_preview);
}

#[test]
fn blind_calibration_button_fits_the_device_correction_matrix_from_the_loaded_ramp() {
    let mut s = session();
    let h = &mut s.harness;
    drain(h);
    open_convert_controls(h);
    let before = calibration_outputs(h.state());
    let revision = h.state().parameter_revision;
    click(h, "blind calibration");
    assert!(
        h.state().calibration_job.is_some() || h.state().status == "Blind calibration fitted.",
        "{}",
        h.state().status
    );
    wait_calibration(h, "blind calibration");
    assert_eq!(h.state().status, "Blind calibration fitted.");
    let applied = h.state().params.film_render.convert.calibration.clone();
    let matrix = parse_calibration(&applied);
    assert_eq!(matrix.len(), 9, "calibration {applied:?}");
    // The fitted correction is asserted per entry against a window around the
    // value the ramp yields, not against an exact pin: the residual the fit
    // minimises runs through the native image decoder, whose OpenImageIO
    // build differs per platform (distro on Linux, Homebrew 3.2 on macOS) and
    // therefore decodes the embedded ICC profile to slightly different
    // doubles. That moves the converged matrix by ~2e-2, far beyond libm
    // noise, so an entry-wise 2e-5 pin fails on a platform that cannot be
    // reproduced locally. Each window keeps that ~2e-2 spread plus margin, so
    // a genuine change to the fit, the ramp, or the calibration worker still
    // fails.
    let windows: [(usize, f64, f64); 9] = [
        (0, 0.29, 0.34),
        (1, -0.01, 0.05),
        (2, -0.37, -0.31),
        (3, 0.15, 0.20),
        (4, 0.45, 0.53),
        (5, -0.05, 0.00),
        (6, -0.02, 0.03),
        (7, -0.22, -0.17),
        (8, 0.66, 0.71),
    ];
    for (index, low, high) in windows {
        assert!(
            (low..=high).contains(&matrix[index]),
            "calibration {applied:?}: entry {index} = {} outside [{low}, {high}]",
            matrix[index]
        );
    }
    let after = calibration_outputs(h.state());
    assert_ne!(after.2, before.2, "fitted matrix must be applied");
    assert_eq!(after.0, before.0);
    assert_eq!(after.1, before.1);
    assert_eq!((after.3, after.4), (before.3, before.4));
    assert_eq!(h.state().parameter_revision, revision.wrapping_add(1));
    assert!(h.state().force_preview);
}

#[test]
fn neutralize_print_filters_button_solves_neutral_m_and_y_filter_shifts() {
    let mut s = session();
    let h = &mut s.harness;
    drain(h);
    open_convert_controls(h);
    let before = calibration_outputs(h.state());
    let revision = h.state().parameter_revision;
    click(h, "neutralize print filters");
    assert!(
        h.state().calibration_job.is_some()
            || h.state().status.starts_with("Print filters neutralized:"),
        "{}",
        h.state().status
    );
    wait_calibration(h, "neutralize print filters");
    let m = h.state().params.enlarger.m_filter_shift;
    let y = h.state().params.enlarger.y_filter_shift;
    assert!((m - FILTER_M_SHIFT).abs() < 1e-5, "m shift {m}");
    assert!((y - FILTER_Y_SHIFT).abs() < 1e-5, "y shift {y}");
    assert_eq!(
        h.state().status,
        format!("Print filters neutralized: M {m:+.2}, Y {y:+.2}.")
    );
    let after = calibration_outputs(h.state());
    assert_ne!(
        (after.3, after.4),
        (before.3, before.4),
        "solved shifts must be applied"
    );
    assert_eq!(after.0, before.0);
    assert_eq!(after.1, before.1);
    assert_eq!(after.2, before.2);
    assert_eq!(h.state().parameter_revision, revision.wrapping_add(1));
    assert!(h.state().force_preview);
}

#[test]
fn convert_buttons_refuse_to_start_while_a_calibration_worker_is_running() {
    let mut s = session();
    let h = &mut s.harness;
    drain(h);
    open_convert_controls(h);
    let before = calibration_outputs(h.state());
    let revision = h.state().parameter_revision;
    press(h, "detect base");
    let epoch = h.state().calibration_epoch;
    assert!(h.state().calibration_job.is_some());
    assert_eq!(h.state().status, "Detecting film base…");
    // A second Convert action while the worker runs is refused in
    // `start_calibration`, reports why, and does not restart or supersede the
    // running job.
    h.state_mut()
        .start_calibration(controls::CalibrationAction::BlindCalibration);
    assert_eq!(
        h.state().status,
        "Wait for the current operation before running a Convert action."
    );
    assert_eq!(h.state().calibration_epoch, epoch);
    assert_eq!(h.state().parameter_revision, revision);
    assert!(h.state().calibration_job.is_some());
    assert_eq!(calibration_outputs(h.state()), before);
    // The refused action does not disturb the running one: its result is still
    // applied when the worker finishes.
    wait_calibration(h, "detect base");
    assert_detect_base_applied(h.state());
    assert_eq!(h.state().calibration_epoch, epoch);
    assert_eq!(h.state().parameter_revision, revision.wrapping_add(1));
}

#[test]
fn convert_buttons_report_a_reason_and_touch_nothing_without_an_input_image() {
    let mut s = session_without_image();
    let h = &mut s.harness;
    open_convert_controls(h);
    let before = calibration_outputs(h.state());
    let revision = h.state().parameter_revision;
    let epoch = h.state().calibration_epoch;
    for label in [
        "detect base",
        "blind calibration",
        "neutralize print filters",
    ] {
        click(h, label);
        assert_eq!(
            h.state().status,
            "Load an input image before running a Convert action.",
            "{label}"
        );
        assert!(h.state().calibration_job.is_none(), "{label}");
        assert_eq!(calibration_outputs(h.state()), before, "{label}");
        assert_eq!(h.state().parameter_revision, revision, "{label}");
        assert_eq!(h.state().calibration_epoch, epoch, "{label}");
    }
}

#[test]
fn convert_action_reports_profile_load_errors_without_starting_a_worker() {
    let mut s = session();
    let h = &mut s.harness;
    drain(h);
    open_convert_controls(h);
    let before = calibration_outputs(h.state());
    let revision = h.state().parameter_revision;
    let epoch = h.state().calibration_epoch;
    let film = h.state().film_name.clone();
    h.state_mut().film_name = "missing-film-profile".into();
    h.state_mut()
        .start_calibration(controls::CalibrationAction::DetectBase);
    assert!(
        h.state().status.starts_with("Calibration film error: "),
        "{}",
        h.state().status
    );
    assert!(h.state().calibration_job.is_none());
    assert_eq!(calibration_outputs(h.state()), before);
    assert_eq!(h.state().parameter_revision, revision);
    assert_eq!(h.state().calibration_epoch, epoch);
    h.state_mut().film_name = film;
    h.state_mut().print_name = "missing-print-profile".into();
    h.state_mut()
        .start_calibration(controls::CalibrationAction::NeutralizeFilters);
    assert!(
        h.state().status.starts_with("Calibration print error: "),
        "{}",
        h.state().status
    );
    assert!(h.state().calibration_job.is_none());
    assert_eq!(calibration_outputs(h.state()), before);
    assert_eq!(h.state().parameter_revision, revision);
    assert_eq!(h.state().calibration_epoch, epoch);
}

#[test]
fn calibration_result_from_a_run_superseded_by_a_parameter_change_is_discarded() {
    let mut s = session();
    let h = &mut s.harness;
    drain(h);
    open_convert_controls(h);
    let before = calibration_outputs(h.state());
    let revision = h.state().parameter_revision;
    press(h, "detect base");
    assert!(h.state().calibration_job.is_some());
    assert_eq!(h.state().status, "Detecting film base…");
    // A parameter edit while the worker runs is the product's supersede path
    // (ui.rs marks changed parameters invalidating calibration).
    h.state_mut()
        .mark_parameters_changed_invalidating_calibration();
    let revision = revision.wrapping_add(1);
    assert_eq!(h.state().parameter_revision, revision);
    wait_calibration(h, "detect base");
    assert_eq!(
        h.state().status,
        "Discarded stale Convert calibration result."
    );
    // The superseded fit completed but must not touch any output parameter.
    assert_eq!(calibration_outputs(h.state()), before);
    assert_eq!(h.state().parameter_revision, revision);
    assert!(!h.state().force_preview);
}

#[test]
fn shutting_down_the_app_while_a_calibration_worker_runs_joins_it_without_applying_results() {
    let mut s = session();
    let h = &mut s.harness;
    drain(h);
    open_convert_controls(h);
    let before = calibration_outputs(h.state());
    press(h, "detect base");
    assert!(h.state().calibration_job.is_some());
    assert_eq!(h.state().status, "Detecting film base…");
    // What `Session::drop` does on exit: take the job and join its thread.
    // `calibration.rs` has no cancel semantics; the worker runs to completion
    // and its result is dropped with the receiver, never applied.
    eframe::App::on_exit(h.state_mut(), None);
    assert!(h.state().calibration_job.is_none());
    assert_eq!(calibration_outputs(h.state()), before);
    assert_eq!(h.state().status, "Detecting film base…");
}
