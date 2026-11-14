//! Execution-surface acceptance: the backend request is a real GUI control,
//! persists in the GUI state, and replaces the running selection.
use super::*;

fn choose_value(h: &mut Harness<'_, App>, combo: &str, option: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, combo)
        .focus();
    h.run_steps(3);
    h.press_key(egui::Key::Space);
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, option)
        .simulate_click();
    h.run_steps(3);
    await_input(h);
}

#[test]
fn execution_surface_shows_the_running_selection_and_persists_the_request() {
    let mut session = session();
    let h = &mut session.harness;
    assert!(!h.state().backend.is_gpu());
    assert_eq!(
        h.state().gui_state.backend_choice(),
        spektrafilm_gpu::BackendChoice::Auto
    );
    open_settings(h);
    // Locating the control opens its section and scrolls to the real row.
    reveal_control(h, "extras.rust.backend");
    assert!(h.state().settings_open);
    // The running selection is reported beside the request.
    assert!(
        h.query_all_by_label_contains("Running: ").next().is_some(),
        "the execution surface must show the running backend"
    );

    let cpu_label = if cfg!(feature = "precision-f64") {
        "CPU (f64)"
    } else {
        "CPU (f32)"
    };
    choose_value(h, "Compute backend", cpu_label);
    assert_eq!(
        h.state().gui_state.backend_choice(),
        spektrafilm_gpu::BackendChoice::Cpu
    );
    // The requested CPU is already the running backend, so no device is
    // rebuilt and the status names the kept backend.
    assert!(!h.state().backend.is_gpu());
    assert!(
        h.state()
            .status
            .to_string()
            .starts_with("Backend unchanged:"),
        "{}",
        h.state().status
    );
    // The request is part of the saved GUI state, not session-only.
    let saved = h.state().current_state().unwrap();
    assert_eq!(saved.sections["rust"]["backend"], "cpu");

    // Reloading rebuilds the same selection on demand (driver-reset recovery).
    let reload = h.get_by_label("Reload device");
    reload.simulate_click();
    h.run_steps(3);
    await_input(h);
    assert!(
        h.state().status.to_string().starts_with("Backend: "),
        "{}",
        h.state().status
    );
    assert_eq!(
        h.state().gui_state.backend_choice(),
        spektrafilm_gpu::BackendChoice::Cpu
    );
}

#[test]
fn an_unavailable_backend_request_is_reported_without_stopping_the_shell() {
    let mut session = session();
    let h = &mut session.harness;
    // Selecting an explicit GPU API off its platform fails by name and keeps
    // the running backend instead of leaving the shell without one.
    let unsupported = if cfg!(target_os = "windows") {
        spektrafilm_gpu::GpuApi::Metal
    } else {
        spektrafilm_gpu::GpuApi::Dx12
    };
    let before = h.state().backend.name().to_owned();
    h.state_mut()
        .choose_execution(spektrafilm_gpu::BackendChoice::Gpu, unsupported);
    assert_eq!(h.state().backend.name(), before);
    assert!(
        h.state()
            .status
            .to_string()
            .starts_with("Backend request failed:"),
        "{}",
        h.state().status
    );
    assert!(
        h.state().gpu_selection.is_none(),
        "a failed request must not claim a device"
    );
    // The request stays recorded so the panel can show what was asked for.
    assert_eq!(
        h.state().gui_state.backend_choice(),
        spektrafilm_gpu::BackendChoice::Gpu
    );
    assert_eq!(h.state().gui_state.gpu_api(), unsupported);
}
