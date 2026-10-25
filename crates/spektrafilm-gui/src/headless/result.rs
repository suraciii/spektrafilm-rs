//! Result identity and the UI-only stale Save publication boundary.
use super::*;

#[test]
fn stale_save_cancel_and_continue_preserve_retained_pixels_and_adjustments() {
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "input");
    let retained = preview(h);
    open_export(h);
    assert!(h.state_mut().undo_adjustment());
    assert!(h.state().accepted_frame_is_stale());
    let adjustments = h.state().params.clone();
    let revision = h.state().parameter_revision;
    let status = h.state().status.clone();
    let destination = s.directory.path().join("confirmed-retained.png");
    script(h, chooser::ChooserResult::Scripted(destination.clone()));

    // Enter the SAVE handler while the existing Export modal guards automatic
    // submission. The real confirmation and chooser still own publication.
    h.state_mut().save_dialog();
    h.run_steps(3);
    assert!(h.state().stale_save_confirmation);
    assert!(!destination.exists());
    assert!(h.query_by_label("The retained 128 × 64 px result does not match current adjustments. Continue to save these retained pixels without rendering?").is_some());
    let continue_bounds = h
        .get_by_label("Continue saving retained pixels")
        .node()
        .raw_bounds()
        .unwrap();
    h.query_all_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
        .find(|node| {
            node.node()
                .raw_bounds()
                .is_some_and(|bounds| bounds.y0 == continue_bounds.y0)
        })
        .expect("Cancel in the stale-save confirmation")
        .simulate_click();
    h.run_steps(3);
    assert!(h.state().export_dialog.is_open());
    assert!(h.state().dirty);
    assert!(h.state().render_job.is_none());
    assert!(!h.state().stale_save_confirmation);
    assert!(!destination.exists());
    assert_eq!(h.state().params, adjustments);
    assert_eq!(h.state().parameter_revision, revision);
    assert_eq!(h.state().status, status);
    assert!(Arc::ptr_eq(
        &retained,
        h.state().output_image.as_ref().unwrap()
    ));

    // The canceled confirmation must not consume the chooser result. Reusing
    // that result proves destination selection happens only after Continue.
    h.state_mut().save_dialog();
    h.run_steps(3);
    h.get_by_label("Continue saving retained pixels")
        .simulate_click();
    h.run_steps(3);
    wait_until(h, |app| app.pending_saves.is_empty());
    assert!(!h.state().stale_save_confirmation);
    let expected = image_io::convert_image(
        &retained,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        h.state().gui_state.saving_color_space(),
        h.state().gui_state.saving_cctf_encoding(),
    )
    .unwrap();
    assert_png(&destination, &expected);
    assert_eq!(h.state().params, adjustments);
    assert_eq!(h.state().parameter_revision, revision);
    assert!(Arc::ptr_eq(
        &retained,
        h.state().output_image.as_ref().unwrap()
    ));
    assert!(h.state().accepted_frame_is_stale());
}

#[test]
fn escape_cancels_stale_save_without_opening_the_chooser() {
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "input");
    let retained = scan(h);
    open_export(h);
    assert!(h.state_mut().undo_adjustment());
    let destination = s.directory.path().join("escape-retained.png");
    script(h, chooser::ChooserResult::Scripted(destination.clone()));
    let status = h.state().status.clone();
    // Keep automatic submission paused by the real Export modal, not a test
    // opt-out, while exercising the SAVE handler and topmost confirmation.
    h.state_mut().save_dialog();
    h.run_steps(3);
    assert!(h.state().stale_save_confirmation);
    let active = h.state().render_worker.active.clone();
    let worker_guard = active.lock();
    h.press_key(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().stale_save_confirmation);
    assert!(!destination.exists());
    assert_eq!(h.state().status, status);
    assert!(Arc::ptr_eq(
        &retained,
        h.state().output_image.as_ref().unwrap()
    ));
    drop(worker_guard);
}

#[test]
fn viewer_keeps_retained_preview_identity_and_caveat_while_scan_is_pending() {
    let mut s = session();
    let h = &mut s.harness;
    let retained = preview(h);
    assert_eq!(h.state().retained_kind, Some(RenderKind::Preview));
    let identity = h.state().retained_result_identity(&h.ctx);
    assert!(identity.contains("Preview"), "{identity}");
    assert!(identity.contains("128 × 64"), "{identity}");
    assert!(
        identity.contains("Matches current adjustments"),
        "{identity}"
    );
    assert!(h.query_by_label(&identity).is_some());

    let active = h.state().render_worker.active.clone();
    let guard = active.lock();
    choose(h, "workflow", "input");
    // Queue the Scan through the same scheduler as its button. A direct
    // dispatch would leave dirty set and schedule a superseding Preview.
    h.state_mut().full_scan_requested = true;
    h.state_mut().dirty = true;
    wait_until(h, |app| {
        !app.dirty
            && app
                .render_job
                .as_ref()
                .is_some_and(|job| job.kind == RenderKind::Scan)
    });
    let identity = h.state().retained_result_identity(&h.ctx);
    assert!(identity.contains("Preview"), "{identity}");
    assert!(identity.contains("128 × 64"), "{identity}");
    assert!(
        identity.contains("Does not match current adjustments"),
        "{identity}"
    );
    assert!(h.query_by_label(&identity).is_some());
    assert!(
        h.query_by_label("Preview detail effects can differ from Scan.")
            .is_some()
    );
    assert!(h.query_by_label("Rendering…").is_some());
    assert!(Arc::ptr_eq(
        &retained,
        h.state().output_image.as_ref().unwrap()
    ));
    drop(guard);
    wait_until(h, |app| app.render_job.is_none());
    assert_eq!(h.state().retained_kind, Some(RenderKind::Scan));
    let identity = h.state().retained_result_identity(&h.ctx);
    assert!(identity.contains("Scan"), "{identity}");
    assert!(identity.contains("160 × 80"), "{identity}");
    assert!(h.query_by_label(&identity).is_some());
    assert!(
        h.query_by_label("Preview detail effects can differ from Scan.")
            .is_none()
    );
}

#[test]
fn export_guidance_and_written_file_follow_crop_and_upscale() {
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "input");
    for (path, label, value) in [
        ("io.crop_size", "crop size x", "0.5"),
        ("io.crop_size", "crop size y", "0.25"),
        ("io.upscale_factor", "upscale factor", "1.5"),
    ] {
        reveal_control(h, path);
        edit(h, label, value);
    }
    reveal_control(h, "io.crop");
    click(h, "crop");
    assert!(h.state().params.io.crop);
    open_export(h);
    assert!(h.query_by_label("Export dimensions: 120 × 60 px").is_some());
    let destination = s.directory.path().join("crop-upscale.png");
    script(h, chooser::ChooserResult::Scripted(destination.clone()));
    export_action(h, "Export file…");
    wait_until(h, |app| app.export_job.is_none());
    let exported = image_io::load(&destination).unwrap().image;
    assert_eq!((exported.width, exported.height), (120, 60));
}

/// Inspect the real egui paint output, not merely focus in AccessKit: a
/// fixed text color must not erase the focused button's visible border.
fn assert_painted_focus(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label)
        .focus();
    h.run_steps(3);
    let node = h.get_by_role_and_label(egui::accesskit::Role::Button, label);
    assert!(node.is_focused(), "{label} must receive keyboard focus");
    let bounds = node.raw_bounds().expect("focused action bounds");
    let rect = egui::Rect::from_min_max(
        egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
        egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
    );
    fn has_border(shape: &egui::epaint::Shape, target: egui::Rect) -> bool {
        match shape {
            egui::epaint::Shape::Rect(rect) => {
                rect.rect.expand(1.0).contains_rect(target)
                    && rect.rect.center().distance(target.center()) < 1.0
                    && rect.stroke.color == theme::ACCENT
                    && rect.stroke.width >= theme::STROKE_DEFAULT
            }
            egui::epaint::Shape::Vec(shapes) => {
                shapes.iter().any(|shape| has_border(shape, target))
            }
            _ => false,
        }
    }
    assert!(
        h.output()
            .shapes
            .iter()
            .any(|shape| has_border(&shape.shape, rect)),
        "{label} must paint an accent keyboard-focus border"
    );
}

#[test]
fn quiet_secondary_actions_keep_visible_keyboard_focus() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "io.upscale_factor");
    assert_painted_focus(h, "reset Crop and upscale");
    reveal_control(h, "extras.simulation.film_stock");
    assert_painted_focus(h, "reset Film material");
    reveal_control(h, "action.save_state");
    for label in [
        "Save current to file",
        "Load from file",
        "Save current as default",
        "Restore factory default",
    ] {
        assert_painted_focus(h, label);
    }
}

#[test]
fn repeated_deferred_saves_are_bounded_and_preserve_captured_pixels_settings_and_provenance() {
    let mut s = session();
    let h = &mut s.harness;
    let earlier = preview(h);
    let earlier_source = h.state().source_render.clone();
    let earlier_color_space = h.state().output_color_space.clone();
    let earlier_encoded = h.state().output_cctf_encoding;
    let retained = scan(h);
    let source = h.state().source_render.clone().unwrap();
    h.state_mut().gui_state.sections["simulation"]["saving_color_space"] =
        serde_json::json!("sRGB");
    h.state_mut().gui_state.sections["simulation"]["saving_cctf_encoding"] =
        serde_json::json!(true);
    let expected = image_io::convert_image(
        &retained,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    let directory = h
        .state()
        .diagnostics
        .publication_directory()
        .unwrap()
        .to_path_buf();
    let (paused, release) = h.state_mut().diagnostics.pause_publication_for_test();
    paused.recv_timeout(WAIT_DEADLINE).unwrap();
    let destination = s.directory.path().join("captured-scan.png");
    h.state_mut().save_to_path(&destination);
    assert_eq!(h.state().pending_saves.len(), 1);
    assert!(!destination.exists());

    // The current retained result can change while journal creation is pending.
    // Reuse a real earlier Preview rather than manufacturing replacement pixels.
    let app = h.state_mut();
    app.output_image = Some(earlier);
    app.output_retained = None;
    app.source_render = earlier_source;
    app.output_color_space = earlier_color_space;
    app.output_cctf_encoding = earlier_encoded;
    app.retained_kind = Some(RenderKind::Preview);
    app.gui_state.sections["simulation"]["saving_color_space"] = serde_json::json!("ProPhoto RGB");
    app.gui_state.sections["simulation"]["saving_cctf_encoding"] = serde_json::json!(false);
    h.run_steps(2);
    assert_eq!(h.state().pending_saves.len(), 1);
    assert!(!destination.exists());

    let waiting_writes = h.state().diagnostics.pending_publications();
    let rejected = s.directory.path().join("rejected-repeat.png");
    for _ in 0..128 {
        h.state_mut().save_to_path(&rejected);
        assert_eq!(h.state().pending_saves.len(), 1);
        assert_eq!(h.state().diagnostics.pending_publications(), waiting_writes);
    }
    h.state_mut().save_dialog();
    assert!(!h.state().stale_save_confirmation);
    assert!(!rejected.exists());

    drop(release);
    wait_until(h, |app| app.pending_saves.is_empty());
    assert_png(&destination, &expected);
    assert!(!rejected.exists());
    let id = h.state().diagnostics.selected_report_id().unwrap();
    let report_path = directory.join(format!("operation-{id}.json"));
    wait_until(h, |app| {
        app.diagnostics.pending_publications() == 0 && report_path.is_file()
    });
    let report: spektrafilm_core::telemetry::Report =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(
        report.operation.kind,
        spektrafilm_core::telemetry::OperationKind::Save
    );
    assert_eq!(report.source_render.as_ref(), Some(&source));
    let saving = report.configuration.saving.unwrap();
    assert_eq!(
        saving.color_space,
        spektrafilm_core::telemetry::ColorSpaceRole::Srgb
    );
    assert!(saving.transfer_encoded);
}

#[test]
fn shutdown_cancels_save_waiting_for_journal_creation_without_writing_pixels() {
    let mut s = session();
    let h = &mut s.harness;
    preview(h);
    let directory = h
        .state()
        .diagnostics
        .publication_directory()
        .unwrap()
        .to_path_buf();
    let (paused, release) = h.state_mut().diagnostics.pause_publication_for_test();
    paused.recv_timeout(WAIT_DEADLINE).unwrap();
    let destination = s.directory.path().join("cancelled-save.png");
    h.state_mut().save_to_path(&destination);
    assert_eq!(h.state().pending_saves.len(), 1);
    assert!(!destination.exists());
    let waiting_writes = h.state().diagnostics.pending_publications();
    let rejected = s.directory.path().join("rejected-before-shutdown.png");
    for _ in 0..128 {
        h.state_mut().save_to_path(&rejected);
        assert_eq!(h.state().pending_saves.len(), 1);
        assert_eq!(h.state().diagnostics.pending_publications(), waiting_writes);
    }
    // Releasing the publisher does not run the UI-side pixel writer.
    drop(release);
    eframe::App::on_exit(h.state_mut(), None);
    assert!(h.state().pending_saves.is_empty());
    assert!(!destination.exists());
    assert!(!rejected.exists());
    // Shutdown deliberately discards in-memory history. The durable report,
    // rather than a selected UI row, owns the cancellation evidence afterward.
    assert!(h.state().diagnostics.selected_report_id().is_none());
    let mut saves: Vec<spektrafilm_core::telemetry::Report> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            spektrafilm_core::telemetry::Report::from_json(&std::fs::read_to_string(path).unwrap())
                .unwrap()
        })
        .filter(|report| report.operation.kind == spektrafilm_core::telemetry::OperationKind::Save)
        .collect();
    assert_eq!(
        saves.len(),
        1,
        "only the accepted Save owns a terminal report"
    );
    let report = saves.pop().unwrap();
    let id = report.operation.id;
    assert_eq!(
        report.operation.kind,
        spektrafilm_core::telemetry::OperationKind::Save
    );
    assert_eq!(
        report.operation.outcome,
        spektrafilm_core::telemetry::Outcome::Cancelled
    );
    let progress_path = directory.join(format!("operation-{id}.json.progress.jsonl"));
    assert!(
        std::fs::read(progress_path)
            .unwrap()
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
            .any(|record| record["event"] == "end" && record["outcome"] == "cancelled")
    );
}
