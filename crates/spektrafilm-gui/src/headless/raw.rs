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
pub(super) fn raw_fixture() -> PathBuf {
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
    assert_eq!((image.width(), image.height()), (RAW_WIDTH, RAW_HEIGHT));
    assert_eq!(h.state().params.io.input_color_space, "ACES2065-1");
    assert!(!h.state().params.io.input_cctf_decoding);
    assert_eq!(
        h.state().image_path.as_deref().and_then(|p| p.file_name()),
        fixture.file_name(),
    );

    let scan = render(h, "SCAN");
    assert_eq!((scan.width, scan.height), (RAW_WIDTH, RAW_HEIGHT));
    for value in &scan.data {
        assert!(value.is_finite(), "RAW scan produced a nonfinite sample");
    }

    // Export the full-resolution RAW result through the production worker.
    // The default TIFF export is 8-bit; request the float depth explicitly so
    // the RAW samples survive without integer quantization.
    open_export(h);
    choose(h, "Format", "TIFF");
    choose(h, "Bit depth", "32 bit");
    click(h, "Apply saving CCTF encoding");
    script(h, chooser::Chooser::chooser_path(&destination));
    assert!(h.state().export_dialog.is_open(), "export modal opened");
    export_action(h, "Export file…");
    h.run_steps(5);
    assert!(
        !h.state().export_dialog.is_open(),
        "modal still open after submit: status={}",
        h.state().status
    );
    wait_until(h, |a| a.export_job.is_some());
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.to_string().starts_with("Exported"),
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

/// The GPU import/render/export entrypoints must keep the actual RAW source
/// and accepted output on the device, not rebuild them from host RGB samples.
#[cfg(feature = "gui-raw")]
#[test]
fn gpu_raw_import_preview_scan_and_export_reuse_retained_samples() {
    let fixture = raw_fixture();
    let _device = spektrafilm_gpu::test_support::device_lock();
    let backend: Arc<dyn ComputeBackend> = Arc::new(
        spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit().expect("GPU adapter required"),
    );
    let directory = tempfile::tempdir().unwrap();
    let state_path = deterministic_state(directory.path());
    let browser_path = directory.path().join("browser.json");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 1400.0))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            App::new_with_browser_path(backend, None, Some(state_path), None, browser_path, data_dir())
        });
    click(&mut h, "◀");
    script(&mut h, chooser::Chooser::chooser_path(&fixture));
    import_file(&mut h, true);
    let source = h.state().image.clone().expect("GPU RAW import accepted");
    assert!(source.host_image().is_none(), "GPU RAW must not retain decoded host RGB");
    let crate::render::InputImage::Device(device_source) = &source else {
        panic!("RAW source is not device-retained");
    };
    assert_eq!((device_source.width(), device_source.height()), (RAW_WIDTH, RAW_HEIGHT));
    assert_eq!(h.state().params.io.input_color_space, "ACES2065-1");
    assert!(!h.state().params.io.input_cctf_decoding);
    // Auto preview is disabled by the deterministic state: import marks the
    // input dirty without scheduling work. The explicit Preview below consumes
    // that dirty state; waiting for !dirty here would never complete.
    wait_until(&mut h, |a| a.render_job.is_none() && !a.force_preview);
    h.state_mut().diagnostics.mode = spektrafilm_backend::telemetry::CollectionMode::Summary;
    for button in ["PREVIEW", "SCAN"] {
        let previous = h.state().output_retained.clone();
        click(&mut h, button);
        wait_until(&mut h, |a| {
            !a.dirty && !a.force_preview && a.render_job.is_none()
                && a.status.to_string().starts_with(if button == "SCAN" { "Scan ·" } else { "Preview ·" })
                && a.output_retained.as_ref().is_some_and(|image| {
                previous.as_ref().is_none_or(|old| {
                    !Arc::ptr_eq(image.payload().unwrap(), old.payload().unwrap())
                })
            })
        });
        assert!(h.state().status.to_string().starts_with(if button == "SCAN" { "Scan ·" } else { "Preview ·" }), "{}", h.state().status);
        assert!(h.state().output_image.is_none(), "GPU render must not publish a host compatibility raster");
        assert!(h.state().output_histogram.is_some());
        assert!(h.state().image.as_ref().unwrap().same_source(&source));
        let output = h.state().output_retained.as_ref().unwrap();
        let kind = if button == "SCAN" { "scan" } else { "preview" };
        let report = assert_retained_gpu_report(
            h.state(), &directory.path().join(format!("{kind}-report.json")), kind,
            [output.width(), output.height()],
        );
        let simulation = report.phases.iter().find(|phase| phase.name == "simulation").unwrap();
        assert_eq!(simulation.work.image_upload_count, Some(0), "RAW carrier is never reuploaded");
        settle_animation(&mut h);
    }
    let scan = h.state().output_retained.clone().unwrap();
    assert_eq!((scan.width(), scan.height()), (RAW_WIDTH, RAW_HEIGHT));
    // Explicit test diagnostic boundary only: production viewing and Export
    // must not materialize this full image. Freeze samples to detect mutation.
    let accepted = scan.to_host(h.state().backend.as_ref()).unwrap();
    assert!(accepted.data.iter().all(|value| value.is_finite()));
    h.state_mut().viewer.set_zoom_percent(200.0);
    settle_animation(&mut h);
    assert!(h.state().viewer.observation().pixels.is_some());

    let destination = directory.path().join("gpu-raw-export.tiff");
    let mut options = h.state().export_options.clone();
    options.backend = export::ExportBackend::Gpu;
    options.format = export::ExportFormat::Tiff;
    options.depth = BitDepth::ThirtyTwo;
    options.saving_color_space = h.state().output_color_space.clone();
    options.saving_cctf_encoding = h.state().output_cctf_encoding;
    h.state_mut().export_options = options;
    script(&mut h, chooser::Chooser::chooser_path(&destination));
    open_export(&mut h);
    assert!(h.state().export_dialog.is_open());
    export_action(&mut h, "Export file…");
    wait_until(&mut h, |a| a.export_job.is_none() && a.status.to_string().starts_with("Exported"));
    assert_float_tiff(&destination);
    assert_retained_gpu_report(
        h.state(), &directory.path().join("export-report.json"), "export",
        [RAW_WIDTH, RAW_HEIGHT],
    );
    let exported = image_io::load(&destination).unwrap().image;
    assert_eq!((exported.width, exported.height), (RAW_WIDTH, RAW_HEIGHT));
    assert_eq!(exported.data, accepted.data, "independent deterministic Export preserves the full Scan samples");
    assert!(h.state().output_image.is_none());
    assert!(h.state().image.as_ref().unwrap().same_source(&source));
    assert!(Arc::ptr_eq(h.state().output_retained.as_ref().unwrap().payload().unwrap(), scan.payload().unwrap()));
    assert_eq!(scan.to_host(h.state().backend.as_ref()).unwrap().data, accepted.data);

    // A failed import is not allowed to replace either device-owned carrier.
    let corrupt = directory.path().join("corrupt.KDC");
    std::fs::write(&corrupt, b"not a raw file").unwrap();
    let params = h.state().params.clone();
    let input_epoch = h.state().input_epoch;
    // Exercise the production operation after the chooser has returned its
    // path. The successful import above covers the chooser; this failure
    // assertion must not depend on sidebar scrolling/focus after Export.
    h.state_mut().load_image_from_path(&corrupt);
    assert!(h.state().status.to_string().starts_with("Load error:"), "{}", h.state().status);
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().input_epoch, input_epoch);
    assert!(h.state().image.as_ref().unwrap().same_source(&source));
    assert!(Arc::ptr_eq(h.state().output_retained.as_ref().unwrap().payload().unwrap(), scan.payload().unwrap()));
    assert_eq!(scan.to_host(h.state().backend.as_ref()).unwrap().data, accepted.data);
    eframe::App::on_exit(h.state_mut(), None);
}

#[test]
fn raw_import_failure_keeps_the_previous_image_and_state() {
    let mut s = session();
    let h = &mut s.harness;
    let before = h.state().image.clone().unwrap();
    let params_before = h.state().params.clone();
    let path_before = h.state().image_path.clone();
    let revision_before = h.state().parameter_revision;
    let history_before = (h.state().can_undo(), h.state().can_redo());
    let corrupt = s.directory.path().join("corrupt.KDC");
    std::fs::write(&corrupt, b"not a raw file").unwrap();
    script(h, chooser::Chooser::chooser_path(&corrupt));
    import_file(h, true);
    assert!(
        h.state().image.as_ref().unwrap().same_source(&before),
        "a failed RAW import must retain the previous image"
    );
    assert_eq!(&h.state().params, &params_before);
    assert_eq!(h.state().image_path, path_before);
    assert_eq!(h.state().parameter_revision, revision_before);
    assert_eq!((h.state().can_undo(), h.state().can_redo()), history_before);
}
