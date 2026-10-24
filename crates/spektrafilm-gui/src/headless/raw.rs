//! RAW import acceptance through the production App.
//!
//! The fixture-backed test is opt-in behind the `gui-raw` feature: the pinned
//! Kodak DC50 image is not redistributed, so it stays out of the default gate
//! and runs through `just gui-raw`. Point `SPEKTRAFILM_GUI_RAW_FIXTURE` at a
//! file whose SHA-256 matches the pinned digest; a missing or mismatched
//! fixture fails instead of silently skipping. The fixture-free failure path
//! needs no external file and always runs.

use super::*;

/// Fixture-free RAW carrier regression: decoding has produced linear ACES
/// samples, but accepted Input controls may reinterpret those exact values.
#[test]
fn retained_raw_interpretation_matches_cpu_and_queued_exports_freeze_the_declaration() {
    let _device = spektrafilm_gpu::test_support::device_lock();
    let directory = tempfile::tempdir().unwrap();
    let (image_path, state_path) = deterministic_inputs(directory.path());
    let browser_path = directory.path().join("browser.json");
    let backend: Arc<dyn ComputeBackend> = Arc::new(
        spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit().expect("GPU adapter required"),
    );
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 1400.0))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            App::new_with_browser_path(
                backend,
                Some(image_path),
                Some(state_path),
                None,
                browser_path,
                data_dir(),
            )
        });
    click(&mut h, "◀");
    wait_until(&mut h, |a| !a.dirty && a.render_job.is_none());
    let samples = h
        .state()
        .image
        .as_ref()
        .unwrap()
        .host_image()
        .unwrap()
        .clone();
    let carrier = h
        .state()
        .backend
        .retain_image(
            &samples,
            &spektrafilm_backend::RetainedImageMeta {
                width: samples.width,
                height: samples.height,
                domain: spektrafilm_backend::SampleDomain::Linear,
                precision: spektrafilm_backend::Precision::host(),
                space: spektrafilm_math::colorspace::resolve("ACES2065-1").unwrap(),
                pixel_pitch_um: None,
            },
        )
        .unwrap();
    let source = crate::render::InputImage::Device(carrier.clone());
    h.state_mut().image = Some(source.clone());
    h.state_mut().params.io.input_color_space = "ACES2065-1".into();
    h.state_mut().params.io.input_cctf_decoding = false;
    h.state_mut().refresh_input_viewer();
    h.state_mut().reset_adjustment_history();
    choose(&mut h, "workflow", "input");
    h.state_mut().params.settings.preview_max_size = 256;
    let original = carrier.to_host(h.state().backend.as_ref()).unwrap();
    let mut submissions = Vec::new();
    let mut expected_exports = Vec::new();
    for (index, (space, encoded)) in [
        ("ProPhoto RGB", true),
        ("ProPhoto RGB", false),
        ("ACES2065-1", false),
    ]
    .into_iter()
    .enumerate()
    {
        reveal_control(&mut h, "io.input_color_space");
        choose(&mut h, "input color space", space);
        if h.state().params.io.input_cctf_decoding != encoded {
            reveal_control(&mut h, "io.input_cctf_decoding");
            click(&mut h, "apply cctf decoding");
        }
        assert_eq!(h.state().params.io.input_color_space, space);
        assert_eq!(h.state().params.io.input_cctf_decoding, encoded);
        let crate::render::InputImage::Device(view) = h.state().oriented_input.as_ref().unwrap()
        else {
            panic!("input viewing lost its retained carrier");
        };
        assert_eq!(view.space().name, space);
        assert_eq!(
            view.domain(),
            if encoded {
                spektrafilm_backend::SampleDomain::Encoded
            } else {
                spektrafilm_backend::SampleDomain::Linear
            }
        );
        assert!(source.same_source(h.state().oriented_input.as_ref().unwrap()));
        let params = h
            .state()
            .current_state()
            .unwrap()
            .effective_runtime_params()
            .unwrap();
        let snapshot = h.state_mut().render_snapshot(params);
        for (kind, preview) in [("Preview", true), ("Scan", false)] {
            if !preview {
                click(&mut h, "SCAN");
            }
            wait_until(&mut h, |a| {
                !a.dirty
                    && a.render_job.is_none()
                    && a.status.to_string().starts_with(if preview {
                        "Preview ·"
                    } else {
                        "Scan ·"
                    })
            });
            let runtime = crate::render::build_runtime_observed(
                &snapshot,
                preview,
                &spektrafilm_backend::telemetry::ObservationContext::default(),
            )
            .unwrap();
            let input = if runtime.params().settings.preview_mode {
                spektrafilm_core::params_builder::resize_for_preview(
                    &original,
                    runtime.params().settings.preview_max_size,
                )
            } else {
                original.clone()
            };
            let expected = runtime.process(input, &CpuBackend).unwrap();
            let output = h
                .state()
                .output_retained
                .as_ref()
                .expect("GPU accepted retained output");
            let actual = output.to_host(h.state().backend.as_ref()).unwrap();
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height)
            );
            for (&actual, &expected) in actual.data.iter().zip(&expected.data) {
                assert!(
                    (actual - expected).abs() < 3e-5,
                    "{space}, encoded={encoded}, {kind}: {actual} != {expected}"
                );
            }
            if !preview {
                expected_exports.push(actual);
            }
        }
        let mut options = h.state().export_options.clone();
        options.backend = export::ExportBackend::Gpu;
        options.format = export::ExportFormat::Tiff;
        options.depth = BitDepth::ThirtyTwo;
        options.saving_color_space = h.state().output_color_space.clone();
        options.saving_cctf_encoding = h.state().output_cctf_encoding;
        let direct = directory
            .path()
            .join(format!("direct-interpretation-{index}.tiff"));
        let ctx = h.ctx.clone();
        h.state_mut()
            .start_export_to_path(&ctx, options.clone(), &direct);
        wait_until(&mut h, |a| a.export_job.is_none());
        assert!(direct.exists(), "{}", h.state().status);
        assert_eq!(
            image_io::load(&direct).unwrap().image.data,
            expected_exports[index].data,
            "independent Export uses the accepted Input declaration"
        );
        submissions.push(crate::app::export_queue::QueueSubmission {
            source_name: "decoded RAW carrier".into(),
            image: source.clone(),
            rotation_quarter_turns: 0,
            snapshot,
            metadata: None,
            options,
            destination: directory
                .path()
                .join(format!("interpretation-{index}.tiff")),
        });
    }
    assert_ne!(expected_exports[0].data, expected_exports[1].data);
    assert_ne!(expected_exports[1].data, expected_exports[2].data);
    // Neither active controls nor the active source may leak into submissions.
    h.state_mut().params.io.input_color_space = "sRGB".into();
    h.state_mut().params.io.input_cctf_decoding = true;
    h.state_mut().image = Some(crate::render::InputImage::Host(Arc::new(ImageBuf::new(
        1, 1,
    ))));
    let ctx = h.ctx.clone();
    h.state_mut()
        .enqueue_exports(&ctx, submissions, false)
        .unwrap();
    wait_until(&mut h, |a| {
        a.export_job.is_none() && directory.path().join("interpretation-2.tiff").exists()
    });
    for (index, expected) in expected_exports.iter().enumerate() {
        let exported = image_io::load(
            &directory
                .path()
                .join(format!("interpretation-{index}.tiff")),
        )
        .unwrap()
        .image;
        assert_eq!(
            exported.data, expected.data,
            "queued Export keeps accepted interpretation {index}"
        );
    }
    assert_eq!(carrier.space().name, "ACES2065-1");
    assert_eq!(carrier.domain(), spektrafilm_backend::SampleDomain::Linear);
    assert_eq!(
        carrier.to_host(h.state().backend.as_ref()).unwrap().data,
        original.data
    );
    assert!(h.state().output_image.is_none(), "no host fallback output");
    eframe::App::on_exit(h.state_mut(), None);
}

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

    let scan = scan(h);
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
            App::new_with_browser_path(
                backend,
                None,
                Some(state_path),
                None,
                browser_path,
                data_dir(),
            )
        });
    click(&mut h, "◀");
    // Observe the automatic import request from its first dispatch.
    h.state_mut().diagnostics.mode = spektrafilm_backend::telemetry::CollectionMode::Summary;
    script(&mut h, chooser::Chooser::chooser_path(&fixture));
    import_file(&mut h, true);
    let source = h.state().image.clone().expect("GPU RAW import accepted");
    assert!(
        source.host_image().is_none(),
        "GPU RAW must not retain decoded host RGB"
    );
    let crate::render::InputImage::Device(device_source) = &source else {
        panic!("RAW source is not device-retained");
    };
    assert_eq!(
        (device_source.width(), device_source.height()),
        (RAW_WIDTH, RAW_HEIGHT)
    );
    assert_eq!(h.state().params.io.input_color_space, "ACES2065-1");
    assert!(!h.state().params.io.input_cctf_decoding);
    for kind in ["preview", "scan"] {
        if kind == "scan" {
            let previous = h.state().output_retained.clone().unwrap();
            click(&mut h, "SCAN");
            wait_until(&mut h, |a| {
                !a.dirty
                    && a.render_job.is_none()
                    && a.status.to_string().starts_with("Scan ·")
                    && a.output_retained.as_ref().is_some_and(|image| {
                        !Arc::ptr_eq(image.payload().unwrap(), previous.payload().unwrap())
                    })
            });
        } else {
            wait_until(&mut h, |a| {
                !a.dirty
                    && a.render_job.is_none()
                    && a.output_retained.is_some()
                    && a.status.to_string().starts_with("Preview ·")
            });
        }
        assert!(
            h.state().status.to_string().starts_with(if kind == "scan" {
                "Scan ·"
            } else {
                "Preview ·"
            }),
            "{}",
            h.state().status
        );
        assert!(
            h.state().output_image.is_none(),
            "GPU render must not publish a host compatibility raster"
        );
        assert!(h.state().output_histogram.is_some());
        assert!(h.state().image.as_ref().unwrap().same_source(&source));
        let output = h.state().output_retained.as_ref().unwrap();
        let report = assert_retained_gpu_report(
            h.state(),
            &directory.path().join(format!("{kind}-report.json")),
            kind,
            [output.width(), output.height()],
        );
        let simulation = report
            .phases
            .iter()
            .find(|phase| phase.name == "simulation")
            .unwrap();
        assert_eq!(
            simulation.work.image_upload_count,
            Some(0),
            "RAW carrier is never reuploaded"
        );
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
    wait_until(&mut h, |a| {
        a.export_job.is_none() && a.status.to_string().starts_with("Exported")
    });
    assert_float_tiff(&destination);
    assert_retained_gpu_report(
        h.state(),
        &directory.path().join("export-report.json"),
        "export",
        [RAW_WIDTH, RAW_HEIGHT],
    );
    let exported = image_io::load(&destination).unwrap().image;
    assert_eq!((exported.width, exported.height), (RAW_WIDTH, RAW_HEIGHT));
    assert_eq!(
        exported.data, accepted.data,
        "independent deterministic Export preserves the full Scan samples"
    );
    assert!(h.state().output_image.is_none());
    assert!(h.state().image.as_ref().unwrap().same_source(&source));
    assert!(Arc::ptr_eq(
        h.state()
            .output_retained
            .as_ref()
            .unwrap()
            .payload()
            .unwrap(),
        scan.payload().unwrap()
    ));
    assert_eq!(
        scan.to_host(h.state().backend.as_ref()).unwrap().data,
        accepted.data
    );

    // A failed import is not allowed to replace either device-owned carrier.
    let corrupt = directory.path().join("corrupt.KDC");
    std::fs::write(&corrupt, b"not a raw file").unwrap();
    let params = h.state().params.clone();
    let input_epoch = h.state().input_epoch;
    // Exercise the production operation after the chooser has returned its
    // path. The successful import above covers the chooser; this failure
    // assertion must not depend on sidebar scrolling/focus after Export.
    h.state_mut().load_image_from_path(&corrupt);
    await_input(&mut h);
    assert!(
        h.state().status.to_string().starts_with("Load error:"),
        "{}",
        h.state().status
    );
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().input_epoch, input_epoch);
    assert!(h.state().image.as_ref().unwrap().same_source(&source));
    assert!(Arc::ptr_eq(
        h.state()
            .output_retained
            .as_ref()
            .unwrap()
            .payload()
            .unwrap(),
        scan.payload().unwrap()
    ));
    assert_eq!(
        scan.to_host(h.state().backend.as_ref()).unwrap().data,
        accepted.data
    );
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
