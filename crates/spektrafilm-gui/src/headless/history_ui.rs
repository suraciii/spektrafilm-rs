//! Adjustment history exercised through production controls and real workers.
use super::*;

fn camera_edit(h: &mut Harness<'_, App>, text: &str) {
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "exposure compensation ev", text);
}

fn release_text_focus(h: &mut Harness<'_, App>) {
    h.get_by_label("Search controls").focus();
    h.run_steps(3);
}

fn slider_point(h: &Harness<'_, App>, fraction: f32) -> egui::Pos2 {
    let bounds = h
        .query_all_by_role(egui::accesskit::Role::Slider)
        .find(|node| !node.node().is_disabled())
        .expect("editable camera slider")
        .node()
        .raw_bounds()
        .expect("camera slider bounds");
    egui::pos2(
        (bounds.x0 + (bounds.x1 - bounds.x0) * fraction as f64) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    )
}

fn pointer_frame(h: &mut Harness<'_, App>, position: egui::Pos2, button: Option<bool>) {
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(position));
    if let Some(pressed) = button {
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
    }
    h.step();
}

#[test]
fn accepted_text_edits_undo_redo_without_rewinding_revision_or_viewer_state() {
    let mut s = session();
    let h = &mut s.harness;
    let initial = h.state().params.camera.exposure_compensation_ev;
    camera_edit(h, "1.25");
    release_text_focus(h);
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 1.25);
    let revision = h.state().parameter_revision;
    h.state_mut().viewer.set_zoom_percent(200.0);
    let zoom = h.state().viewer.zoom_percent();
    click(h, "Undo");
    assert_eq!(h.state().params.camera.exposure_compensation_ev, initial);
    assert!(h.state().parameter_revision > revision);
    assert_eq!(h.state().viewer.zoom_percent(), zoom);
    click(h, "Redo");
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 1.25);
    assert_eq!(h.state().viewer.zoom_percent(), zoom);
    click(h, "Undo");
    camera_edit(h, "-0.75");
    release_text_focus(h);
    assert!(!h.state().can_redo());
    assert_eq!(h.state().params.camera.exposure_compensation_ev, -0.75);
}

#[test]
fn one_drag_with_multiple_intermediate_values_is_one_undo_step() {
    let mut s = session();
    let h = &mut s.harness;
    camera_edit(h, "0.5");
    release_text_focus(h);
    let baseline = h.state().params.camera.exposure_compensation_ev;
    let start = slider_point(h, 0.525);
    pointer_frame(h, start, Some(true));
    for fraction in [0.55, 0.6, 0.7] {
        let point = slider_point(h, fraction);
        pointer_frame(h, point, None);
    }
    let end = slider_point(h, 0.7);
    pointer_frame(h, end, Some(false));
    h.run_steps(3);
    let dragged = h.state().params.camera.exposure_compensation_ev;
    assert!(dragged > baseline + 1.0);
    click(h, "Undo");
    assert_eq!(h.state().params.camera.exposure_compensation_ev, baseline);
    click(h, "Undo");
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 0.0);
    click(h, "Redo");
    assert_eq!(h.state().params.camera.exposure_compensation_ev, baseline);
    click(h, "Redo");
    assert_eq!(h.state().params.camera.exposure_compensation_ev, dragged);
}

#[test]
fn released_slider_updates_preview_without_more_input() {
    let mut s = session();
    let h = &mut s.harness;
    let before = preview(h);
    reveal_control(h, "camera.exposure_compensation_ev");
    let (tx, rx) = mpsc::channel();
    h.ctx.set_request_repaint_callback(move |request| {
        let _ = tx.send(request.delay);
    });
    let start = slider_point(h, 0.5);
    pointer_frame(h, start, Some(true));
    let end = slider_point(h, 0.56);
    pointer_frame(h, end, None);
    pointer_frame(h, end, Some(false));
    let revision = h.state().parameter_revision;
    assert!(h.state().params.camera.exposure_compensation_ev > 0.5);

    // Drive only frames requested by the App or its workers, as the native
    // event loop does. Unconditional stepping would hide a lost wakeup.
    let deadline = Instant::now() + WAIT_DEADLINE;
    while h.state().source_render.as_ref().unwrap().parameter_revision != Some(revision) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "released slider did not publish a Preview"
        );
        let mut delay = h.output().viewport_output[&egui::ViewportId::ROOT]
            .repaint_delay
            .min(remaining);
        while let Ok(requested) = rx.try_recv() {
            delay = delay.min(requested);
        }
        if !delay.is_zero() {
            let _ = rx.recv_timeout(delay);
        }
        assert!(Instant::now() < deadline, "Preview lost its repaint wakeup");
        h.step();
    }
    assert_eq!(h.state().retained_kind, Some(RenderKind::Preview));
    assert_eq!(h.state().viewer.settings.layer, display::ViewLayer::Output);
    assert!(!h.state().accepted_frame_is_stale());
    let after = h.state().output_image.as_ref().unwrap();
    assert!(
        before
            .data
            .iter()
            .zip(&after.data)
            .any(|(a, b)| (*a - *b).abs() > 1e-4)
    );
}

#[test]
fn escape_cancels_a_drag_and_invalid_text_adds_no_adjustment() {
    let mut s = session();
    let h = &mut s.harness;
    camera_edit(h, "0.75");
    release_text_focus(h);
    let initial = h.state().params.camera.exposure_compensation_ev;
    let start = slider_point(h, 0.5375);
    pointer_frame(h, start, Some(true));
    let moved = slider_point(h, 0.8);
    pointer_frame(h, moved, None);
    h.press_key(egui::Key::Escape);
    pointer_frame(h, moved, Some(false));
    h.run_steps(3);
    assert_eq!(h.state().params.camera.exposure_compensation_ev, initial);
    edit(h, "exposure compensation ev", "not-a-number");
    release_text_focus(h);
    assert_eq!(h.state().params.camera.exposure_compensation_ev, initial);
    click(h, "Undo");
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 0.0);
}

#[test]
fn undo_after_dispatch_supersedes_old_work_and_renders_the_restored_adjustment() {
    let mut s = session();
    let h = &mut s.harness;
    let baseline = preview(h);
    camera_edit(h, "1.0");
    release_text_focus(h);
    let ctx = h.ctx.clone();
    h.state_mut().dispatch_render(&ctx);
    let abandoned_revision = h.state().parameter_revision;
    click(h, "Undo");
    assert!(h.state().parameter_revision > abandoned_revision);
    let restored = preview(h);
    assert_eq!(restored.data, baseline.data);
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 0.0);
}

#[test]
fn keyboard_history_works_with_nontext_focus_and_preserves_text_edit_ownership() {
    let mut s = session();
    let h = &mut s.harness;
    camera_edit(h, "1.25");
    h.query_all_by_role(egui::accesskit::Role::Slider)
        .find(|node| !node.node().is_disabled())
        .expect("editable camera slider")
        .focus();
    h.run_steps(3);
    h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 0.0);
    h.press_key_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Z,
    );
    h.run_steps(3);
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 1.25);
    h.get_by_label("Search controls").focus();
    h.run_steps(3);
    h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 1.25);
}

#[test]
fn rotation_undo_redo_automatically_refreshes_viewable_and_saveable_output() {
    let _device = spektrafilm_gpu::test_support::device_lock();
    for gpu in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (image_path, state_path) = deterministic_inputs(directory.path());
        let browser_path = directory.path().join("browser.json");
        let backend: Arc<dyn ComputeBackend> = if gpu {
            Arc::new(
                spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit()
                    .expect("GPU adapter required"),
            )
        } else {
            Arc::new(CpuBackend)
        };
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
        let source = h.state().image.as_ref().unwrap().clone();
        reveal_control(&mut h, "action.rotate_cw");
        click(&mut h, "cw rotate");
        assert_eq!(h.state().input_rotation, 3);
        wait_until(&mut h, |a| !a.dirty && a.render_job.is_none());
        click(&mut h, "SCAN");
        wait_until(&mut h, |a| {
            !a.dirty && a.render_job.is_none() && a.status.to_string().starts_with("Scan ·")
        });
        // Paper geometry follows the newly oriented input. Hold output at an
        // exact source-pixel zoom and compare its interior, not that changing
        // fit framing or the surrounding paper/canvas.
        h.state_mut().viewer.set_zoom_percent(100.0);
        settle_animation(&mut h);
        let baseline = directory.path().join("rotated.tiff");
        save_retained(&mut h, &baseline);
        assert!(baseline.exists(), "{}", h.state().status);
        let rotated = image_io::load(&baseline).unwrap().image;
        assert_eq!((rotated.width, rotated.height), (80, 160));
        for (action, rotation, dimensions) in [("Undo", 0, (128, 64)), ("Redo", 3, (64, 128))] {
            click(&mut h, action);
            assert_eq!(h.state().input_rotation, rotation);
            let revision = h.state().parameter_revision;
            wait_until(&mut h, |a| {
                !a.dirty
                    && a.render_job.is_none()
                    && !a.accepted_frame_is_stale()
                    && a.source_render
                        .as_ref()
                        .is_some_and(|source| source.parameter_revision == Some(revision))
            });
            settle_animation(&mut h);
            assert_eq!(h.state().retained_kind, Some(RenderKind::Preview));
            assert!(h.state().image.as_ref().unwrap().same_source(&source));
            assert!(!h.get_by_label("Modified").node().is_disabled());
            let saved = directory.path().join(format!("{action}.tiff"));
            save_retained(&mut h, &saved);
            assert!(saved.exists(), "{}", h.state().status);
            let output = image_io::load(&saved).unwrap().image;
            assert_eq!((output.width, output.height), dimensions);
            h.state_mut().viewer.settings.layer = crate::display::ViewLayer::Input;
            settle_animation(&mut h);
            assert_eq!(
                h.state().viewer.settings.layer,
                crate::display::ViewLayer::Input
            );
            h.state_mut().viewer.settings.layer = crate::display::ViewLayer::Output;
            settle_animation(&mut h);
            let viewed = directory.path().join(format!("{action}-viewed.tiff"));
            save_retained(&mut h, &viewed);
            super::panels::assert_float_images_equal(&saved, &viewed);
        }
        eframe::App::on_exit(h.state_mut(), None);
    }
}
