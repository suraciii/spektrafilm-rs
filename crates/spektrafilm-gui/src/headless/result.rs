//! Result identity and the UI-only stale Save publication boundary.
use super::*;

#[test]
fn stale_save_cancel_and_continue_preserve_retained_pixels_and_adjustments() {
    let mut s = session();
    let h = &mut s.harness;
    let retained = render(h, "PREVIEW");
    choose(h, "workflow", "input");
    assert!(h.state().accepted_frame_is_stale());
    let adjustments = h.state().params.clone();
    let revision = h.state().parameter_revision;
    let status = h.state().status.clone();
    let destination = s.directory.path().join("confirmed-retained.png");
    script(h, chooser::ChooserResult::Scripted(destination.clone()));

    click(h, "SAVE");
    assert!(h.state().stale_save_confirmation);
    assert!(!destination.exists());
    assert!(h.query_by_label("The retained 128 × 64 px result does not match current adjustments. Continue to save these retained pixels without rendering?").is_some());
    click(h, "Cancel");
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
    click(h, "SAVE");
    click(h, "Continue saving retained pixels");
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
    assert!(h.state().render_job.is_none());
    assert!(h.state().accepted_frame_is_stale());
}

#[test]
fn escape_cancels_stale_save_without_opening_the_chooser() {
    let mut s = session();
    let h = &mut s.harness;
    let retained = render(h, "SCAN");
    choose(h, "workflow", "input");
    let destination = s.directory.path().join("escape-retained.png");
    script(h, chooser::ChooserResult::Scripted(destination.clone()));
    let status = h.state().status.clone();
    click(h, "SAVE");
    h.press_key(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().stale_save_confirmation);
    assert!(!destination.exists());
    assert_eq!(h.state().status, status);
    assert!(Arc::ptr_eq(
        &retained,
        h.state().output_image.as_ref().unwrap()
    ));
}

#[test]
fn header_keeps_preview_identity_and_quality_caveat_while_scan_is_pending() {
    let mut s = session();
    let h = &mut s.harness;
    let retained = render(h, "PREVIEW");
    assert_eq!(h.state().retained_kind, Some(RenderKind::Preview));
    let identity = h.state().retained_result_identity(&h.ctx);
    assert!(identity.contains("Preview"), "{identity}");
    assert!(identity.contains("128 × 64"), "{identity}");
    assert!(
        identity.contains("Matches current adjustments"),
        "{identity}"
    );
    assert!(h.query_by_label(&identity).is_some());

    choose(h, "workflow", "input");
    let active = h.state().render_worker.active.clone();
    let guard = active.lock();
    let ctx = h.ctx.clone();
    h.state_mut().full_scan_requested = true;
    h.state_mut().dispatch_render(&ctx);
    h.run_steps(3);
    assert!(h.state().render_job.is_some());
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
    assert!(
        h.state()
            .operation_status_localized(&h.ctx)
            .contains("Scan")
    );
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
    assert_painted_focus(h, "Reset upscale factor to factory");
    assert_painted_focus(h, "reset Crop and upscale");
    reveal_control(h, "extras.simulation.film_stock");
    assert_painted_focus(h, "Reset film profile to factory");
    reveal_control(h, "action.save_state");
    for label in [
        "Save current to file",
        "Load from file",
        "Save current as default",
        "Restore factory default",
    ] {
        assert_painted_focus(h, label);
    }
    open_export(h);
    choose(h, "Format", "TIFF");
    assert_painted_focus(h, "Reset Format to factory");
    click(h, "Cancel");
}
