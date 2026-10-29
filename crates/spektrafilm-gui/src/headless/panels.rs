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
    save_retained(h, &save_before);
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
    save_retained(h, &save_after);
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
fn analysis_follows_accepted_output_without_changing_the_photograph() {
    use spektrafilm_backend::analysis::{self, AnalysisMode, AnalysisRequest};
    use spektrafilm_backend::{Precision, RetainedImageMeta, SampleDomain};

    let mut s = session();
    let h = &mut s.harness;
    let output = preview(h);
    settle_animation(h);
    let render_id = h.state().source_render.as_ref().unwrap().operation_id;
    let request_id = h.state().render_worker.next_request_id;
    let params = h.state().params.clone();
    let revision = h.state().parameter_revision;

    click(h, "Image analysis");
    wait_until(h, analysis_output_ready);
    let result = h.state().analysis.result.as_ref().unwrap();
    assert_eq!(result.identity.modified_generation, Some(render_id));
    let actual = result.data[1]
        .as_ref()
        .unwrap()
        .as_ref()
        .expect("output analysis");
    let meta = RetainedImageMeta {
        width: output.width,
        height: output.height,
        space: spektrafilm_math::colorspace::resolve(&h.state().output_color_space).unwrap(),
        domain: if h.state().output_cctf_encoding {
            SampleDomain::Encoded
        } else {
            SampleDomain::Linear
        },
        precision: Precision::host(),
        pixel_pitch_um: None,
    };
    let expected = analysis::analyze_host(
        &output,
        meta,
        &AnalysisRequest {
            region: [0, 0, output.width, output.height],
            mode: AnalysisMode::Histogram,
            chroma_zoom: 1,
        },
        None,
    )
    .unwrap();
    assert_eq!(actual.histogram, expected.histogram);
    assert_eq!(
        (actual.total, actual.finite, actual.invalid),
        (expected.total, expected.finite, expected.invalid)
    );

    click(h, "Analysis mode");
    click(h, "Waveform");
    wait_until(h, |app| {
        analysis_output_ready(app)
            && app.analysis.result.as_ref().unwrap().identity.mode == AnalysisMode::Waveform
    });
    let waveform = h.state().analysis.result.as_ref().unwrap().data[1]
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(waveform.waveform.iter().sum::<u64>(), waveform.finite * 4);
    assert_eq!(waveform.total, expected.total);
    let spatial_counts = waveform.waveform.clone();

    choose(h, "Waveform display", "RGB parade");
    wait_until(h, |app| {
        analysis_output_ready(app)
            && app.analysis.result.as_ref().unwrap().identity.mode == AnalysisMode::Parade
    });
    let parade = h.state().analysis.result.as_ref().unwrap().data[1]
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(parade.waveform, spatial_counts);

    click(h, "Analysis mode");
    click(h, "Advanced");
    click(h, "Vectorscope");
    wait_until(h, |app| {
        analysis_output_ready(app)
            && app.analysis.result.as_ref().unwrap().identity.mode == AnalysisMode::Vectorscope
    });
    let colors = h.state().analysis.result.as_ref().unwrap().data[1]
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(colors.total, expected.total);
    assert_eq!(
        colors.vectorscope.iter().sum::<u64>() + colors.outside_window,
        colors.finite
    );

    click(h, "Analysis mode");
    click(h, "Waveform");
    wait_until(h, |app| {
        analysis_output_ready(app)
            && app.analysis.result.as_ref().unwrap().identity.mode == AnalysisMode::Parade
    });
    choose(h, "Waveform display", "Tone");
    wait_until(h, |app| {
        analysis_output_ready(app)
            && app.analysis.result.as_ref().unwrap().identity.mode == AnalysisMode::Waveform
    });

    let viewer_zoom = h.state().viewer.zoom_percent;
    let viewer_pan = h.state().viewer.pan;
    click(h, "Image analysis");
    assert!(!h.state().analysis.visible);
    assert!(h.state().analysis.result.is_none());
    assert!(!h.state().viewer.analysis.enabled);
    assert_eq!(h.state().viewer.zoom_percent, viewer_zoom);
    assert_eq!(h.state().viewer.pan, viewer_pan);
    click(h, "Image analysis");
    wait_until(h, |app| {
        analysis_output_ready(app)
            && app.analysis.result.as_ref().unwrap().identity.mode == AnalysisMode::Waveform
    });

    settle_animation(h);
    let after = h.state().viewer.observation();
    assert_eq!(after.generation, after.accepted_generation);
    assert!(
        composed(h, h.ctx.input(|input| input.time) + 1.0)
            .pixels
            .is_some()
    );
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &output
    ));
    assert_eq!(
        h.state().source_render.as_ref().unwrap().operation_id,
        render_id
    );
    assert_eq!(h.state().render_worker.next_request_id, request_id);
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().parameter_revision, revision);
    assert!(h.state().render_job.is_none());
    assert!(!h.state().dirty);
}

#[test]
fn keyboard_analysis_selects_and_inspects_without_photographic_edits() {
    let mut s = session();
    let h = &mut s.harness;
    let output = preview(h);
    settle_animation(h);
    let params = h.state().params.clone();
    click(h, "Image analysis");
    wait_until(h, analysis_output_ready);
    click(h, "Select area");
    h.get_by_label("Move selection (arrow keys; Enter commits, Escape cancels)")
        .focus();
    h.run_steps(3);
    h.press_key(egui::Key::ArrowRight);
    h.run_steps(3);
    h.press_key(egui::Key::Enter);
    wait_until(h, |app| {
        analysis_output_ready(app) && app.analysis.selection.is_some()
    });
    let moved = h.state().analysis.selection.unwrap();
    assert!((moved[0] - (0.25 + 1.0 / f64::from(output.width))).abs() < 1e-12);

    click(h, "Select area");
    h.get_by_label("Resize selection (arrow keys; Enter commits, Escape cancels)")
        .focus();
    h.run_steps(3);
    h.press_key(egui::Key::ArrowRight);
    h.press_key(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().analysis.selection, Some(moved));
    assert!(!h.state().viewer.analysis.armed);

    click(h, "Select area");
    h.get_by_label("Resize selection (arrow keys; Enter commits, Escape cancels)")
        .focus();
    h.run_steps(3);
    h.press_key(egui::Key::ArrowRight);
    h.run_steps(3);
    assert!(
        h.get_by_label("Resize selection (arrow keys; Enter commits, Escape cancels)")
            .is_focused(),
        "arrow keys must retain resize-handle focus"
    );
    h.press_key(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        !h.state().viewer.analysis.armed,
        "Enter must commit the selection"
    );
    wait_until(h, |app| {
        analysis_output_ready(app)
            && !app.viewer.analysis.armed
            && app.analysis.selection != Some(moved)
    });
    let data = h.state().analysis.result.as_ref().unwrap().data[1]
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(
        data.total,
        u64::from(output.width / 2 + 1) * u64::from(output.height / 2)
    );
    assert!(!h.state().viewer.analysis.armed);

    h.get_by_label("Analysis pixel cursor (arrow keys to move)")
        .focus();
    h.run_steps(3);
    h.press_key(egui::Key::ArrowRight);
    h.press_key(egui::Key::ArrowDown);
    h.run_steps(3);
    wait_until(h, |app| app.analysis.sample.is_some());
    let sample = h.state().viewer.analysis.sample.unwrap();
    assert!(sample.keyboard);
    assert_eq!(
        [sample.x, sample.y],
        [output.width / 2 + 1, output.height / 2 + 1]
    );
    click(h, "Analysis mode");
    click(h, "Waveform");
    wait_until(h, analysis_output_ready);
    h.get_by_label("Analysis plot (arrow keys to inspect)")
        .focus();
    h.run_steps(3);
    let before = h.state().viewer.analysis.band.unwrap();
    h.press_key(egui::Key::ArrowRight);
    h.run_steps(3);
    let after = h.state().viewer.analysis.band.unwrap();
    assert!((after[0] - before[0] - 1.0 / f64::from(output.width)).abs() < 1e-6);
    assert_eq!([after[1], after[3]], [before[1], before[3]]);
    assert!(h.state().analysis.sample.is_none());

    let time = h.ctx.input(|input| input.time) + 1.0;
    let image = composed(h, time).image_rect;
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(image.center()));
    wait_until(h, |app| app.analysis.sample.is_some());
    assert!(h.state().viewer.analysis.band.is_none());
    assert_eq!(h.state().params, params);
}

#[test]
fn expanded_analysis_keeps_inspector_and_delivery_actions_available() {
    let mut s = session();
    let h = &mut s.harness;
    h.set_size(egui::vec2(1460.0, 980.0));
    h.run_steps(3);
    preview(h);
    settle_animation(h);
    h.state_mut().analysis.preferences.height = 400.0;
    click(h, "Image analysis");
    wait_until(h, analysis_output_ready);
    let image = composed(h, 30.0).image_rect;
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(image.center()));
    wait_until(h, |app| app.analysis.sample.is_some());
    let large = h
        .get_by_label("Analysis plot (arrow keys to inspect)")
        .raw_bounds()
        .unwrap();
    h.state_mut().analysis.preferences.height = 100.0;
    h.run_steps(3);
    let small = h
        .get_by_label("Analysis plot (arrow keys to inspect)")
        .raw_bounds()
        .unwrap();
    assert!(
        large.y1 - large.y0 > small.y1 - small.y0,
        "plot must respond to requested height: {large:?} vs {small:?}"
    );
    h.state_mut().analysis.preferences.height = 400.0;

    for height in [980.0, 720.0] {
        h.set_size(egui::vec2(1460.0, height));
        h.run_steps(6);
        let screen = h.ctx.screen_rect();
        for label in ["SCAN", "SAVE", "Search controls"] {
            let bounds = h.get_by_label(label).raw_bounds().unwrap();
            assert!(bounds.y0 >= f64::from(screen.top()), "{label} above window");
            assert!(
                bounds.y1 <= f64::from(screen.bottom()),
                "{label} below window"
            );
        }
        let controls = h
            .get_by_role_and_label(egui::accesskit::Role::ScrollView, "Inspector controls")
            .raw_bounds()
            .unwrap();
        assert!(controls.y1 - controls.y0 >= 100.0);
        assert_eq!(h.state().analysis.preferences.height, 400.0);
    }
}

/// Compare the published distributions against independently chosen native
/// rectangles in the accepted carriers, not against geometry helper output.
fn assert_selected_native_populations(
    app: &App,
    original_region: [u32; 4],
    modified_region: [u32; 4],
) {
    use spektrafilm_backend::analysis::{self, AnalysisMode, AnalysisRequest};

    let result = app
        .analysis
        .result
        .as_ref()
        .expect("accepted selection analysis");
    assert_eq!(result.identity.original_region, Some(original_region));
    assert_eq!(result.identity.modified_region, Some(modified_region));
    for (index, region) in [original_region, modified_region].into_iter().enumerate() {
        let crate::analysis::Source::Host(image, meta) = result.sources[index].as_ref().unwrap()
        else {
            panic!("CPU fixture must analyze its accepted host carrier");
        };
        let expected = analysis::analyze_host(
            image,
            *meta,
            &AnalysisRequest {
                region,
                mode: AnalysisMode::Histogram,
                chroma_zoom: 1,
            },
            None,
        )
        .unwrap();
        let actual = result.data[index].as_ref().unwrap().as_ref().unwrap();
        assert_eq!(
            actual.total,
            u64::from(region[2] - region[0]) * u64::from(region[3] - region[1])
        );
        assert_eq!(actual.histogram, expected.histogram);
        assert_eq!(
            (actual.finite, actual.invalid),
            (expected.finite, expected.invalid)
        );
    }
}

#[test]
fn selection_tracks_the_same_source_pixels_across_rotated_cropped_preview_and_scan() {
    let mut s = session();
    let h = &mut s.harness;
    preview(h);
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input");
    reveal_control(h, "action.rotate_cw");
    click(h, "cw rotate");
    assert_eq!(h.state().input_rotation, 3);
    for (path, label, value) in [
        ("io.crop_center", "crop center x", "0.5"),
        ("io.crop_center", "crop center y", "0.5"),
        ("io.crop_size", "crop size x", "0.5"),
        ("io.crop_size", "crop size y", "0.5"),
    ] {
        reveal_control(h, path);
        edit(h, label, value);
    }
    reveal_control(h, "io.crop");
    click(h, "crop");
    assert!(h.state().params.io.crop);
    let preview_image = preview(h);
    assert_eq!((preview_image.width, preview_image.height), (64, 64));
    let cropped_params = h.state().params.clone();
    settle_animation(h);
    click(h, "Image analysis");
    wait_until(h, analysis_output_ready);

    // Clockwise source orientation is 80×160. The centered crop covers
    // oriented y=40..120 at Scan and y=32..96 in the 64×128 Preview.
    // Dragging the central half therefore selects oriented source pixels
    // x=20..60, y=60..100, and Preview pixels x/y=16..48.
    click(h, "Select area");
    let time = h.ctx.input(|input| input.time) + 1.0;
    let rect = composed(h, time).image_rect;
    let start = rect.min + rect.size() * Vec2::splat(0.25);
    let end = rect.min + rect.size() * Vec2::splat(0.75);
    pointer(h, start, true, time + 0.1);
    h.input_mut().events.push(egui::Event::PointerMoved(end));
    frame(h, time + 0.2);
    pointer(h, end, false, time + 0.3);
    wait_until(h, |app| {
        analysis_output_ready(app)
            && app
                .analysis
                .result
                .as_ref()
                .unwrap()
                .identity
                .region
                .is_some()
    });
    assert_selected_native_populations(h.state(), [20, 60, 60, 100], [16, 16, 48, 48]);
    assert_eq!(
        h.state().params,
        cropped_params,
        "inspection must not change crop or photographic controls"
    );
    let selection = h.state().analysis.selection.expect("committed selection");

    let scan_image = scan(h);
    assert_eq!((scan_image.width, scan_image.height), (80, 80));
    wait_until(h, |app| {
        analysis_output_ready(app)
            && app.analysis.result.as_ref().unwrap().identity.region == Some(selection)
    });
    assert_eq!(h.state().analysis.selection, Some(selection));
    assert_selected_native_populations(h.state(), [20, 60, 60, 100], [20, 20, 60, 60]);
    let result = h.state().analysis.result.as_ref().unwrap();
    let crate::analysis::Source::Host(original, original_meta) =
        result.sources[0].as_ref().unwrap()
    else {
        panic!("CPU original carrier");
    };
    let crate::analysis::Source::Host(modified, modified_meta) =
        result.sources[1].as_ref().unwrap()
    else {
        panic!("CPU scan carrier");
    };
    for (x, y) in [(20, 20), (40, 40), (59, 59)] {
        let source =
            spektrafilm_backend::analysis::inspect_host(original, *original_meta, x, y + 40)
                .unwrap();
        let output =
            spektrafilm_backend::analysis::inspect_host(modified, *modified_meta, x, y).unwrap();
        for (source, output) in source.rgb.into_iter().zip(output.rgb) {
            assert!(
                (source - output).abs() < 1e-4,
                "rotated crop sampled a different source pixel"
            );
        }
    }
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
