//! RAW import acceptance through the production App.
//!
//! The fixture-backed test is opt-in behind the `gui-raw` feature: the pinned
//! Kodak DC50 image is not redistributed, so it stays out of the default gate
//! and runs through `just gui-raw`. Point `SPEKTRAFILM_GUI_RAW_FIXTURE` at a
//! file whose SHA-256 matches the pinned digest; a missing or mismatched
//! fixture fails instead of silently skipping. The fixture-free failure path
//! needs no external file and always runs.

use super::*;

/// Pinned Kodak DC50 KDC fixture digest.
#[cfg(feature = "gui-raw")]
const RAW_FIXTURE_SHA256: &str = "37e290dbd0053f00e508d02a6b3a2a990432dad1eb74c40a52ca899f0f225ecc";
/// Decoded geometry of the pinned fixture.
#[cfg(feature = "gui-raw")]
const RAW_WIDTH: u32 = 768;
#[cfg(feature = "gui-raw")]
const RAW_HEIGHT: u32 = 512;

#[cfg(feature = "gui-raw")]
fn raw_fixture() -> PathBuf {
    use sha2::{Digest, Sha256};
    let path = std::env::var_os("SPEKTRAFILM_GUI_RAW_FIXTURE")
        .expect("SPEKTRAFILM_GUI_RAW_FIXTURE is not set; run the RAW acceptance fixture");
    let path = PathBuf::from(path);
    if !path.is_file() {
        panic!(
            "SPEKTRAFILM_GUI_RAW_FIXTURE is not a file: {}",
            path.display()
        );
    }
    let bytes = std::fs::read(&path).unwrap();
    let observed = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(
        observed,
        RAW_FIXTURE_SHA256,
        "RAW fixture digest mismatch for {}",
        path.display()
    );
    path
}

/// Build a session without the shared synthetic image so the App starts empty.
#[cfg(feature = "gui-raw")]
fn empty_session() -> Session {
    use spektrafilm_gpu::cpu_backend::CpuBackend;

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
        .build_eframe(move |_| {
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

#[cfg(feature = "gui-raw")]
#[test]
fn raw_import_through_the_app_sets_linear_aces_and_renders() {
    let fixture = raw_fixture();
    let mut s = empty_session();
    let h = &mut s.harness;
    let destination = s.directory.path().join("raw-export.tiff");
    // Drive the production import button with the confirmed chooser path.
    script(h, chooser::Chooser::chooser_path(&fixture));
    import_file(h, true);

    let image = h.state().image.as_ref().expect("RAW image loaded");
    assert_eq!((image.width, image.height), (RAW_WIDTH, RAW_HEIGHT));
    assert_eq!(h.state().params.io.input_color_space, "ACES2065-1");
    assert!(!h.state().params.io.input_cctf_decoding);
    assert_eq!(
        h.state().image_path.as_deref().and_then(|p| p.file_name()),
        fixture.file_name(),
    );
    // The fixture has no lens model, so correction is reported as skipped.
    assert!(
        h.state().status.contains("Lens correction not applied"),
        "{}",
        h.state().status
    );

    let scan = render(h, "SCAN");
    assert_eq!((scan.width, scan.height), (RAW_WIDTH, RAW_HEIGHT));
    for value in &scan.data {
        assert!(value.is_finite(), "RAW scan produced a nonfinite sample");
    }

    // Export the full-resolution RAW result through the production worker.
    // The default TIFF export is 8-bit; request the float depth explicitly so
    // the RAW samples survive without integer quantization.
    let mut options = h.state().export_options.clone();
    options.format = export::ExportFormat::Tiff;
    options.depth = BitDepth::ThirtyTwo;
    options.saving_cctf_encoding = false;
    h.state_mut().export_options = options;
    script(h, chooser::Chooser::chooser_path(&destination));
    click(h, "Export options");
    export_action(h);
    assert!(h.state().export_dialog.is_open(), "export modal opened");
    export_action(h);
    h.run_steps(5);
    assert!(
        !h.state().export_dialog.is_open(),
        "modal still open after submit: status={}",
        h.state().status
    );
    wait_until(h, |a| a.export_job.is_some());
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.starts_with("Exported"),
        "{}",
        h.state().status
    );
    assert_float_tiff(&destination);
    let exported = image_io::load(&destination).unwrap().image;
    assert_eq!((exported.width, exported.height), (RAW_WIDTH, RAW_HEIGHT));

    // Persisted state keeps the RAW input configuration.
    let persisted = s.directory.path().join("raw-state.json");
    h.state().current_state().unwrap().save(&persisted).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&persisted).unwrap()).unwrap();
    assert_eq!(saved["input_image"]["input_color_space"], "ACES2065-1");
    assert_eq!(saved["input_image"]["input_cctf_decoding"], false);
}

#[test]
fn raw_import_failure_keeps_the_previous_image_and_state() {
    let mut s = session();
    let h = &mut s.harness;
    let before = h.state().image.clone().unwrap();
    let params_before = h.state().params.clone();
    let corrupt = s.directory.path().join("corrupt.KDC");
    std::fs::write(&corrupt, b"not a raw file").unwrap();
    script(h, chooser::Chooser::chooser_path(&corrupt));
    import_file(h, true);
    assert!(
        Arc::ptr_eq(h.state().image.as_ref().unwrap(), &before),
        "a failed RAW import must retain the previous image"
    );
    assert_eq!(&h.state().params, &params_before);
    assert!(
        h.state().status.starts_with("Load error:"),
        "{}",
        h.state().status
    );
}
