//! Photograph records exercised through the production context/save/reopen boundary.
use super::*;
use serde_json::json;

fn source(path: &Path, level: f32) {
    let image = ImageBuf::from_data(
        24,
        16,
        vec![spektrafilm_math::precision::from_f32(level); 24 * 16 * 3],
    );
    image_io::save(
        path,
        &image,
        image_io::SaveOptions {
            depth: image_io::BitDepth::Eight,
            color_space: "sRGB",
            cctf_encoding: true,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
        },
        None,
    )
    .unwrap();
}

fn app() -> (App, tempfile::TempDir, egui::Context) {
    let directory = tempfile::tempdir().unwrap();
    let state_path = directory.path().join("state.json");
    let factory = state::GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.settings.rgb_to_raw_method = "mallett2019".into();
    params.camera.auto_exposure = false;
    params.io.crop = false;
    params.debug.deactivate_spatial_effects = true;
    params.debug.deactivate_stochastic_effects = true;
    state::GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
        .unwrap()
        .save(&state_path)
        .unwrap();
    let mut app = App::new_with_browser_path(
        Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
        None,
        Some(state_path),
        None,
        directory.path().join("browser.json"),
        crate::headless::data_dir(),
    );
    let path = directory.path().join("accepted.png");
    source(&path, 0.18);
    app.open_photograph(&path).unwrap();
    let ctx = egui::Context::default();
    settle(&mut app, &ctx);
    (app, directory, ctx)
}

fn settle(app: &mut App, ctx: &egui::Context) {
    let deadline = Instant::now() + crate::headless::WAIT_DEADLINE;
    while app.input_pending() {
        assert!(
            Instant::now() < deadline,
            "input did not finish: {}",
            app.status
        );
        app.poll_input_job(ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn install(app: &mut App, ctx: &egui::Context, preparation: PhotoPreparation) {
    app.queue_photograph(preparation, None).unwrap();
    settle(app, ctx);
}

fn exposure(app: &mut App, ctx: &egui::Context, value: f32) {
    app.begin_adjustment_frame(ctx);
    app.params.camera.exposure_compensation_ev = value as _;
    app.mark_parameters_changed_invalidating_calibration();
    app.end_adjustment_frame(ctx);
}

#[test]
fn escape_cannot_dismiss_a_confirmed_transition_while_its_preparation_runs() {
    let (mut app, _directory, ctx) = app();
    let (started, ready) = std::sync::mpsc::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    app.photograph.pending = Some(PhotoAction::Close);
    app.queue_input_command(
        vec![],
        Some(Box::new(move |_, _| {
            started.send(()).unwrap();
            blocked.recv().unwrap();
            Ok(())
        })),
        |_, _| Ok(()),
    )
    .unwrap();
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    let escape = || egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };
    let _ = ctx.run(escape(), |ctx| app.photograph_dialogs(ctx));
    let still_pending = app.photograph.pending.is_some();
    release.send(()).unwrap();
    settle(&mut app, &ctx);
    assert!(
        still_pending,
        "confirmed transition was dismissed while it could still commit"
    );
    let _ = ctx.run(escape(), |ctx| app.photograph_dialogs(ctx));
    assert!(app.photograph.pending.is_none());
    assert!(!app.photograph.allow_close);
}

#[test]
fn record_save_waits_for_pending_input_before_changing_record_ownership() {
    let (mut app, directory, ctx) = app();
    let original = directory.path().join("original.spektra-photo.json");
    app.save_photograph_record_to(&original).unwrap();
    let original_bytes = std::fs::read(&original).unwrap();
    exposure(&mut app, &ctx, 1.0);
    let baseline = app.photograph.baseline.clone();
    let source = app.image_path.clone().unwrap();
    app.load_image_from_path(&source);
    let destination = directory.path().join("after-reprocess.spektra-photo.json");
    assert!(app.save_photograph_record_to(&destination).is_err());
    assert!(!destination.exists());
    assert_eq!(app.photograph.record_path.as_ref(), Some(&original));
    assert_eq!(app.photograph.baseline, baseline);
    settle(&mut app, &ctx);
    app.save_photograph_record_to(&destination).unwrap();
    assert_eq!(app.photograph.record_path.as_ref(), Some(&destination));
    assert!(!app.photograph_has_unsaved_changes());
    assert_eq!(std::fs::read(original).unwrap(), original_bytes);
}

#[test]
fn changed_accepted_source_cannot_be_saved_under_replacement_identity() {
    let (mut app, directory, ctx) = app();
    exposure(&mut app, &ctx, 0.75);
    let record = directory.path().join("accepted.spektra-photo.json");
    app.save_photograph_record_to(&record).unwrap();
    exposure(&mut app, &ctx, 1.25);
    let saved_bytes = std::fs::read(&record).unwrap();
    let baseline = app.photograph.baseline.clone();
    let digest = app.photograph.source_digest.clone();
    let image = app.image.clone().unwrap();
    source(app.image_path.as_deref().unwrap(), 0.65);
    assert!(app.save_photograph_record_to(&record).is_err());
    assert_eq!(std::fs::read(&record).unwrap(), saved_bytes);
    assert_eq!(app.photograph.baseline, baseline);
    assert_eq!(app.photograph.source_digest, digest);
    assert!(image.same_source(app.image.as_ref().unwrap()));
    assert!(app.photograph_has_unsaved_changes());
    assert!(app.undo_adjustment());
    assert_eq!(app.params.camera.exposure_compensation_ev, 0.75);
}

#[test]
fn failed_next_load_and_changed_record_source_preserve_pixels_and_history() {
    let (mut app, directory, ctx) = app();
    let record = directory.path().join("accepted.spektra-photo.json");
    app.save_photograph_record_to(&record).unwrap();
    exposure(&mut app, &ctx, 1.0);
    let image = app.image.clone().unwrap();
    let settings = app.capture_photograph_settings().unwrap();
    let baseline = app.photograph.baseline.clone();
    let epoch = app.input_epoch;
    let invalid = directory.path().join("broken.png");
    std::fs::write(&invalid, b"not an image").unwrap();
    app.open_photograph(&invalid).unwrap();
    settle(&mut app, &ctx);
    assert!(image.same_source(app.image.as_ref().unwrap()));
    assert_eq!(app.capture_photograph_settings().unwrap(), settings);
    assert_eq!(app.photograph.baseline, baseline);
    assert_eq!(app.input_epoch, epoch);
    source(app.image_path.as_deref().unwrap(), 0.7);
    assert!(app.reopen_photograph_record(&record, None).is_err());
    assert!(image.same_source(app.image.as_ref().unwrap()));
    assert_eq!(app.capture_photograph_settings().unwrap(), settings);
    assert_eq!(app.input_epoch, epoch);
    assert!(app.undo_adjustment());
    assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
    assert!(app.redo_adjustment());
    assert_eq!(app.params.camera.exposure_compensation_ev, 1.0);
}

#[test]
fn saved_record_excludes_pending_controls_and_reopens_the_accepted_endpoint() {
    let (mut app, directory, ctx) = app();
    exposure(&mut app, &ctx, 0.5);
    let accepted_raw = app.gui_state.sections["load_raw"].clone();
    app.raw_applied_settings = Some(accepted_raw.clone());
    // Neither value has crossed the accepted editing/decode boundary.
    app.params.camera.exposure_compensation_ev = 3.0;
    app.gui_state.sections["load_raw"]["temperature"] = json!(6200.0);
    let record = directory.path().join("accepted.spektra-photo.json");
    app.save_photograph_record_to(&record).unwrap();
    assert!(
        !app.photograph_has_unsaved_changes(),
        "record baseline covers accepted settings, not pending numeric or RAW drafts"
    );
    let saved = PhotographRecord::load(&record).unwrap();
    assert_eq!(saved.settings.params.camera.exposure_compensation_ev, 0.5);
    assert_eq!(saved.settings.raw, accepted_raw);
    assert_eq!(
        app.params.camera.exposure_compensation_ev, 3.0,
        "saving must not commit the pending controls"
    );
    assert_eq!(
        app.gui_state.sections["load_raw"]["temperature"],
        json!(6200.0)
    );
    app.reopen_photograph_record(&record, None).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(app.params.camera.exposure_compensation_ev, 0.5);
    assert_eq!(app.gui_state.sections["load_raw"], accepted_raw);
    assert!(
        !app.can_undo(),
        "complete-state reopen starts a new photograph history"
    );
    assert!(!app.photograph_has_unsaved_changes());
}

fn render(app: &mut App, ctx: &egui::Context) -> Arc<ImageBuf> {
    app.full_scan_requested = true;
    app.dispatch_render(ctx);
    let deadline = Instant::now() + crate::headless::WAIT_DEADLINE;
    while app.render_job.is_some() {
        assert!(
            Instant::now() < deadline,
            "photograph render did not finish: {}",
            app.status
        );
        app.poll_render_job(ctx);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    app.output_image
        .clone()
        .unwrap_or_else(|| panic!("photograph render failed: {}", app.status))
}

#[test]
fn accepted_context_replacement_invalidates_materials_cached_by_real_worker() {
    let (mut app, directory, ctx) = app();
    let data = directory.path().join("isolated-data");
    std::fs::create_dir_all(data.join("profiles")).unwrap();
    for name in [&app.film_name, &app.print_name] {
        std::fs::copy(
            app.data_dir.join(format!("profiles/{name}.json")),
            data.join(format!("profiles/{name}.json")),
        )
        .unwrap();
    }
    app.data_dir = data;
    let mut settings = app.capture_photograph_settings().unwrap();
    settings.params.workflow.route = "input > film > scan".into();
    settings.params.io.scan_film = true;
    let path = app.image_path.clone().unwrap();
    let context = app
        .photo_preparation(&path, Some(settings.clone()))
        .unwrap();
    install(&mut app, &ctx, context);
    let before = render(&mut app, &ctx);
    let material = app
        .data_dir
        .join(format!("profiles/{}.json", app.film_name));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&material).unwrap()).unwrap();
    let rows = value["data"]["channel_density"].as_array_mut().unwrap();
    for row in rows {
        if let Some(channels) = row.as_array_mut() {
            for value in channels.iter_mut().take(3) {
                if let Some(density) = value.as_f64() {
                    *value = json!(density * 1.75);
                }
            }
        }
    }
    std::fs::write(&material, serde_json::to_vec(&value).unwrap()).unwrap();
    let context = app.photo_preparation(&path, Some(settings)).unwrap();
    install(&mut app, &ctx, context);
    assert!(
        app.output_image.is_none(),
        "replacement clears old retained pixels"
    );
    let after = render(&mut app, &ctx);
    assert!(
        before
            .data
            .iter()
            .zip(&after.data)
            .any(|(a, b)| (*a - *b).abs() > 1e-5),
        "same-named changed material must not reuse the worker's old runtime"
    );
    let params = app
        .current_state()
        .unwrap()
        .effective_runtime_params()
        .unwrap();
    let snapshot = app.render_snapshot(params);
    let mut fresh = crate::render::RuntimeCache::default();
    let observation = spektrafilm_backend::telemetry::ObservationContext::new(
        spektrafilm_backend::telemetry::CollectionMode::Off,
    );
    let runtime = fresh
        .prepare_observed(&snapshot, false, &observation)
        .unwrap();
    let expected = runtime
        .process_observed(
            (**app.image.as_ref().unwrap().host_image().unwrap()).clone(),
            app.backend.as_ref(),
            &observation,
        )
        .unwrap();
    assert_eq!(
        after.data, expected.data,
        "worker replacement must match a fresh runtime with the accepted dependencies"
    );
}

#[cfg(feature = "gui-raw")]
#[test]
fn raw_record_reopens_saved_decode_and_interpretation_not_pending_raw_draft() {
    let (mut app, directory, ctx) = app();
    let raw = PathBuf::from(
        std::env::var_os("SPEKTRAFILM_GUI_RAW_FIXTURE")
            .expect("SPEKTRAFILM_GUI_RAW_FIXTURE is required for gui-raw"),
    );
    assert_eq!(
        App::accepted_source_digest(&raw).unwrap(),
        "37e290dbd0053f00e508d02a6b3a2a990432dad1eb74c40a52ca899f0f225ecc"
    );
    app.open_photograph(&raw).unwrap();
    settle(&mut app, &ctx);
    exposure(&mut app, &ctx, 0.25);
    let mut accepted = app.capture_photograph_settings().unwrap();
    accepted.raw["lens_correction"] = json!(true);
    accepted.params.io.input_color_space = "ProPhoto RGB".into();
    accepted.params.io.input_cctf_decoding = true;
    let context = app.photo_preparation(&raw, Some(accepted)).unwrap();
    install(&mut app, &ctx, context);
    let pixels = app.image.clone().unwrap();
    let raw_accepted = app.raw_applied_settings.clone().unwrap();
    app.gui_state.sections["load_raw"]["white_balance"] = json!("custom");
    app.gui_state.sections["load_raw"]["temperature"] = json!(6200.0);
    let record = directory.path().join("raw.spektra-photo.json");
    app.save_photograph_record_to(&record).unwrap();
    let decoded = spektrafilm_raw::load(
        &raw,
        &spektrafilm_raw::RawOptions {
            white_balance: spektrafilm_raw::WhiteBalance::Daylight,
            lens_correction: true,
            ..Default::default()
        },
    )
    .unwrap();
    let accepted_model = decoded
        .lens_model_identity
        .as_deref()
        .expect("correction returns its selected model even when no lens matches");
    assert_eq!(
        accepted_model,
        spektrafilm_raw::lens_model_identity(&raw).unwrap(),
        "immutable database lookup must match the model returned by the native decode operation"
    );
    let saved = PhotographRecord::load(&record).unwrap();
    assert!(
        saved
            .validate_accepted_lens_model(Some(accepted_model))
            .is_ok()
    );
    assert!(
        saved
            .validate_accepted_lens_model(Some("changed-calibration-after-decode"))
            .is_err()
    );
    app.open_photograph(&directory.path().join("accepted.png"))
        .unwrap();
    settle(&mut app, &ctx);
    app.reopen_photograph_record(&record, None).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(app.params.io.input_color_space, "ProPhoto RGB");
    assert!(app.params.io.input_cctf_decoding);
    assert_eq!(app.raw_applied_settings.as_ref(), Some(&raw_accepted));
    assert_eq!(app.gui_state.sections["load_raw"], raw_accepted);
    assert_eq!(
        app.image.as_ref().unwrap().host_image().unwrap().data,
        pixels.host_image().unwrap().data
    );
    exposure(&mut app, &ctx, 0.75);
    let saved_bytes = std::fs::read(&record).unwrap();
    let baseline = app.photograph.baseline.clone();
    let accepted_settings = app.capture_photograph_settings().unwrap();
    let source_pixels = app.image.clone().unwrap();
    app.photograph.lens_model_identity = Some("stale-accepted-raw-calibration".into());
    assert!(app.save_photograph_record_to(&record).is_err());
    assert_eq!(std::fs::read(&record).unwrap(), saved_bytes);
    assert_eq!(app.photograph.baseline, baseline);
    assert!(source_pixels.same_source(app.image.as_ref().unwrap()));
    app.add_work_paths(vec![directory.path().join("accepted.png")]);
    app.perform_work_select(1).unwrap();
    settle(&mut app, &ctx);
    assert!(app.save_inactive_record_to(0, &record).is_err());
    assert_eq!(std::fs::read(&record).unwrap(), saved_bytes);
    app.perform_work_select(0).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(app.photograph.baseline, baseline);
    assert_eq!(
        app.capture_photograph_settings().unwrap(),
        accepted_settings
    );
    assert!(app.undo_adjustment());
    assert_eq!(app.params.camera.exposure_compensation_ev, 0.25);
}

#[test]
fn active_and_inactive_lens_identity_mismatch_preserves_records_and_history() {
    let (mut app, directory, ctx) = app();
    let record = directory.path().join("calibration.spektra-photo.json");
    app.save_photograph_record_to(&record).unwrap();
    let saved_bytes = std::fs::read(&record).unwrap();
    let baseline = app.photograph.baseline.clone();
    exposure(&mut app, &ctx, 0.75);
    let settings = app.capture_photograph_settings().unwrap();
    let pixels = app.image.clone().unwrap();
    // A retained accepted model may never be silently replaced by whichever
    // model capture resolves now. Exercise the consumer with stale evidence;
    // the fixture-backed RAW test below covers native acquisition itself.
    app.photograph.lens_model_identity = Some("stale-accepted-calibration".into());
    assert!(app.save_photograph_record_to(&record).is_err());
    assert_eq!(std::fs::read(&record).unwrap(), saved_bytes);
    assert_eq!(app.photograph.baseline, baseline);
    assert_eq!(app.capture_photograph_settings().unwrap(), settings);
    assert!(pixels.same_source(app.image.as_ref().unwrap()));
    let second = directory.path().join("second.png");
    source(&second, 0.4);
    app.add_work_paths(vec![second]);
    app.perform_work_select(1).unwrap();
    settle(&mut app, &ctx);
    let active_pixels = app.image.clone().unwrap();
    assert!(app.save_inactive_record_to(0, &record).is_err());
    assert_eq!(std::fs::read(&record).unwrap(), saved_bytes);
    assert!(active_pixels.same_source(app.image.as_ref().unwrap()));
    app.perform_work_select(0).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(app.photograph.baseline, baseline);
    assert_eq!(app.capture_photograph_settings().unwrap(), settings);
    assert!(app.photograph_has_unsaved_changes());
    assert!(app.undo_adjustment());
    assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
    assert!(app.redo_adjustment());
    assert_eq!(app.params.camera.exposure_compensation_ev, 0.75);
}
