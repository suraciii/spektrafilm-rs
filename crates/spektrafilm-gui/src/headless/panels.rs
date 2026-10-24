use super::*;
use crate::display::{ViewLayer, ViewerObservation};
use egui::{Color32, Pos2, Vec2};
use egui_kittest::kittest::Queryable;

fn frame(h: &mut Harness<'_, App>, time: f64) {
    h.input_mut().time = Some(time.max(h.ctx.input(|input| input.time)));
    h.step();
}

fn composed(h: &mut Harness<'_, App>, time: f64) -> ViewerObservation {
    let (tx, rx) = std::sync::mpsc::channel();
    h.ctx.set_request_repaint_callback(move |_| {
        let _ = tx.send(());
    });
    let deadline = Instant::now() + WAIT_DEADLINE;
    loop {
        frame(h, time);
        let observation = h.state().viewer.observation();
        if observation.generation == observation.accepted_generation {
            return observation;
        }
        assert!(Instant::now() < deadline, "display worker timed out");
        let _ = rx.recv_timeout(Duration::from_millis(20));
    }
}

fn settled(h: &mut Harness<'_, App>, mut time: f64) -> (ViewerObservation, f64) {
    for _ in 0..60 {
        let observation = composed(h, time);
        if observation.transition.is_none() {
            return (observation, time);
        }
        time += 0.04;
    }
    panic!("viewer transition did not settle");
}

fn click_viewer_layer(h: &mut Harness<'_, App>, label: &str) {
    h.ctx.style_mut(|style| style.animation_time = 0.0);
    h.step();
    let viewport = h.state().viewer.observation().viewport;
    let nodes: Vec<_> = h
        .query_all_by_label(label)
        .filter(|node| {
            node.raw_bounds().is_some_and(|bounds| {
                let x = (bounds.x0 + bounds.x1) as f32 * 0.5;
                x >= viewport.left() && x <= viewport.right()
            })
        })
        .collect();
    assert_eq!(nodes.len(), 1, "viewer layer control unavailable: {label}");
    nodes[0].simulate_click();
    h.run_steps(3);
}

fn pointer(h: &mut Harness<'_, App>, position: Pos2, pressed: bool, time: f64) {
    h.input_mut().events.extend([
        egui::Event::PointerMoved(position),
        egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        },
    ]);
    frame(h, time);
}

fn pixel_at_fraction(observation: &ViewerObservation, x: f32, y: f32) -> Color32 {
    let image = observation.pixels.as_ref().expect("composed viewer pixels");
    let position = observation.image_rect.min + observation.image_rect.size() * Vec2::new(x, y);
    let x = (((position.x - observation.viewport.left()) / observation.viewport.width())
        * image.size[0] as f32)
        .floor()
        .clamp(0.0, image.size[0].saturating_sub(1) as f32) as usize;
    let y = (((position.y - observation.viewport.top()) / observation.viewport.height())
        * image.size[1] as f32)
        .floor()
        .clamp(0.0, image.size[1].saturating_sub(1) as f32) as usize;
    image.pixels[y * image.size[0] + x]
}

pub(crate) fn assert_float_images_equal(before: &Path, after: &Path) {
    let before = image_io::load(before).unwrap().image;
    let after = image_io::load(after).unwrap().image;
    assert_eq!((before.width, before.height), (after.width, after.height));
    for (a, b) in before.data.iter().zip(&after.data) {
        assert!(
            (*a as f64 - *b as f64).abs() <= 1e-6,
            "retained output changed"
        );
    }
}

#[test]
fn original_modified_switching_preserves_adjustments_view_and_saved_exported_pixels() {
    let mut s = session();
    let h = &mut s.harness;

    let output = preview(h);
    let (modified, time) = settled(h, 20.0);
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::Output);
    assert!(!h.get_by_label("Modified").node().is_disabled());
    let params = h.state().params.clone();
    let revision = h.state().parameter_revision;
    let source_render = h.state().source_render.clone();
    let render_request = h.state().render_worker.next_request_id;

    let mut options = h.state().export_options.clone();
    options.format = export::ExportFormat::Exr;
    options.depth = BitDepth::ThirtyTwo;
    options.saving_cctf_encoding = false;
    let ctx = h.ctx.clone();
    let save_before = s.directory.path().join("modified-save.exr");
    let export_before = s.directory.path().join("modified-export.exr");
    h.state_mut().save_to_path(&save_before);
    h.state_mut()
        .start_export_to_path(&ctx, options.clone(), &export_before);
    wait_until(h, |app| app.export_job.is_none());

    click_viewer_layer(h, "Original");
    let original = composed(h, time + 1.0);
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::Input);
    assert_ne!(
        pixel_at_fraction(&original, 0.5, 0.5),
        pixel_at_fraction(&modified, 0.5, 0.5),
        "Original must show input, not the accepted modified pixels"
    );
    click(h, "200%");
    let zoomed = composed(h, time + 2.0);
    let center = zoomed.viewport.center();
    pointer(h, center, true, time + 3.0);
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(center + Vec2::new(24.0, 18.0)));
    frame(h, time + 3.1);
    pointer(h, center + Vec2::new(24.0, 18.0), false, time + 3.2);
    let pan = h.state().viewer.pan;
    assert!((pan - Vec2::new(24.0, 18.0)).length() < 0.01);
    for label in ["Modified", "Original", "Modified", "Original"] {
        click_viewer_layer(h, label);
        let _ = composed(h, time + 4.0);
        assert_eq!(h.state().viewer.zoom_percent(), Some(200.0));
        assert_eq!(h.state().viewer.pan, pan);
    }
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().parameter_revision, revision);
    assert_eq!(
        h.state().render_worker.next_request_id,
        render_request,
        "switching must not render"
    );
    assert!(h.state().render_job.is_none());
    assert!(!h.state().dirty);
    assert_eq!(h.state().source_render, source_render);
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &output
    ));
    assert!(h.get_by_label("Undo").node().is_disabled());

    let save_after = s.directory.path().join("original-save.exr");
    let export_after = s.directory.path().join("original-export.exr");
    h.state_mut().save_to_path(&save_after);
    h.state_mut()
        .start_export_to_path(&ctx, options, &export_after);
    wait_until(h, |app| app.export_job.is_none());
    assert_float_images_equal(&save_before, &save_after);
    assert_float_images_equal(&export_before, &export_after);
}

#[test]
fn scan_selects_modified_and_original_remains_available() {
    let mut s = session();
    let h = &mut s.harness;
    preview(h);
    click_viewer_layer(h, "Original");
    let output = scan(h);
    let _ = settled(h, 20.0);
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::Output);
    click_viewer_layer(h, "Original");
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::Input);
    click_viewer_layer(h, "Modified");
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::Output);
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &output
    ));
}

#[test]
fn empty_histogram_shows_its_no_output_paint_branch_without_a_retained_render() {
    let mut s = super::calibration::session_without_image();
    let h = &mut s.harness;
    h.run_steps(1);
    assert!(h.state().output_image.is_none());
    assert!(h.state().histogram.visible, "the empty strip is visible");
    // The painter's `data == None` branch is the real `No output` empty state.
    assert!(h.state().histogram.cached_data().is_none());
}

#[test]
fn histogram_draws_retained_output_without_advancing_viewer_or_render_generations() {
    let mut s = session();
    let h = &mut s.harness;
    let _ = preview(h);
    settle_animation(h);
    // Binning runs on the worker thread, so the strip adopts the finished
    // distribution a frame or more after the output is retained.
    wait_until(h, |a| a.histogram.cached_data().is_some());
    let observation = h.state().viewer.observation();
    let output = h.state().output_image.as_ref().unwrap().clone();
    let expected = histogram::HistogramData::compute(&output);
    let cached = h
        .state()
        .histogram
        .cached_data()
        .expect("retained output histogram");
    assert_eq!(cached.r, expected.r);
    assert_eq!(cached.g, expected.g);
    assert_eq!(cached.b, expected.b);
    let generation = observation.generation;
    let status = h.state().status.clone();
    assert!(!h.state().dirty);
    assert!(h.state().render_job.is_none());

    h.run_steps(3);
    let after = h.state().viewer.observation();
    assert_eq!(after.generation, generation);
    assert_eq!(after.accepted_generation, generation);
    assert!(
        h.state()
            .output_image
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &output))
    );
    assert!(h.state().render_job.is_none());
    assert!(!h.state().dirty);
    assert_eq!(h.state().status, status);
}

#[test]
fn footer_reset_view_restores_fit_zoom_and_zero_pan_after_zoom_and_drag() {
    let mut s = session();
    let h = &mut s.harness;
    preview(h);
    settle_animation(h);
    click_viewer_layer(h, "Original");
    let baseline = composed(h, 40.0);
    click(h, "200%");
    let zoomed = composed(h, 41.0);
    assert_eq!(h.state().viewer.zoom_percent, Some(200.0));

    let center = zoomed.viewport.center();
    pointer(h, center, true, 42.0);
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(center + Vec2::new(36.0, 24.0)));
    frame(h, 42.1);
    pointer(h, center + Vec2::new(36.0, 24.0), false, 42.2);
    assert!((h.state().viewer.pan - Vec2::new(36.0, 24.0)).length() < 0.01);

    click(h, "reset view");
    assert_eq!(h.state().viewer.zoom_percent, None);
    assert_eq!(h.state().viewer.zoom, 1.0);
    assert_eq!(h.state().viewer.pan, Vec2::ZERO);
    let reset = composed(h, 43.0);
    assert_eq!(reset.image_rect, baseline.image_rect);

    // Fit is the zoom alone: it clears the exact zoom and leaves the pan, so a
    // panned view stays where the user put it. Reset view recentres as well.
    click(h, "200%");
    pointer(h, center, true, 44.0);
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(center + Vec2::new(12.0, 6.0)));
    frame(h, 44.1);
    pointer(h, center + Vec2::new(12.0, 6.0), false, 44.2);
    let panned = Vec2::new(12.0, 6.0);
    assert!((h.state().viewer.pan - panned).length() < 0.01);
    click(h, "fit");
    assert_eq!(h.state().viewer.zoom_percent, None);
    assert_eq!(h.state().viewer.zoom, 1.0);
    assert!((h.state().viewer.pan - panned).length() < 0.01);
    let fitted = composed(h, 45.0);
    // The fitted zoom matches the baseline's, and the surviving pan offsets
    // the image by exactly the pan the user applied.
    assert_eq!(
        fitted.image_rect.size(),
        baseline.image_rect.size(),
        "fit must restore the fitted zoom"
    );
    assert!((fitted.image_rect.center() - panned - baseline.image_rect.center()).length() < 0.01);
    // Reset view then recentres the pan as well.
    click(h, "reset view");
    assert_eq!(h.state().viewer.pan, Vec2::ZERO);
    assert_eq!(composed(h, 46.0).image_rect, baseline.image_rect);
}
