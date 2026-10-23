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
    let baseline = render(h, "PREVIEW");
    camera_edit(h, "1.0");
    release_text_focus(h);
    let ctx = h.ctx.clone();
    h.state_mut().dispatch_render(&ctx);
    let abandoned_revision = h.state().parameter_revision;
    click(h, "Undo");
    assert!(h.state().parameter_revision > abandoned_revision);
    let restored = render(h, "PREVIEW");
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
