use super::*;
use crate::display::{DisplayRaster, Interpolation, ViewLayer, ViewerObservation};
use egui::{Color32, Pos2, Rect, Vec2};
use egui_kittest::kittest::Queryable;

/// Distance from the compare split at which before/after pixels are sampled,
/// and the accepted-value budget for driving the compare slider.
const SPLIT_SAMPLE_MARGIN: f32 = 0.1;

fn frame(h: &mut Harness<'_, App>, time: f64) {
    h.input_mut().time = Some(time);
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

fn compare_slider_bounds(h: &Harness<'_, App>) -> [f32; 4] {
    let viewport = h.state().viewer.observation().viewport;
    let sliders: Vec<_> = h
        .query_all(egui_kittest::kittest::by().role(egui::accesskit::Role::Slider))
        .filter_map(|node| node.raw_bounds())
        .filter(|bounds| {
            let center_x = (bounds.x0 + bounds.x1) * 0.5;
            center_x >= viewport.left() as f64 && center_x <= viewport.right() as f64
        })
        .map(|bounds| {
            [
                bounds.x0 as f32,
                bounds.y0 as f32,
                bounds.x1 as f32,
                bounds.y1 as f32,
            ]
        })
        .collect();
    assert_eq!(
        sliders.len(),
        1,
        "expected the compare split slider in the viewer footer"
    );
    sliders[0]
}

fn set_compare_split(h: &mut Harness<'_, App>, fraction: f32, time: f64) {
    let bounds = compare_slider_bounds(h);
    let current = h.state().compare_split;
    let point = |value: f32| {
        Pos2::new(
            bounds[0] + (bounds[2] - bounds[0]) * value,
            (bounds[1] + bounds[3]) * 0.5,
        )
    };
    pointer(h, point(current), true, time);
    pointer(h, point(fraction), false, time + 0.01);
    h.run_steps(2);
    // egui's slider maps the pointer through its shrunk x-range with smart aim,
    // so the accepted value snaps a few hundredths from the request. The budget
    // is the pixel-sampling margin: a larger drift would put the sampled points
    // on the wrong side of the split and fail the pixel assertions anyway.
    assert!(
        (h.state().compare_split - fraction).abs() < SPLIT_SAMPLE_MARGIN,
        "compare slider requested {fraction}, accepted {}",
        h.state().compare_split
    );
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
fn compare_split_selects_before_and_after_pixels_and_never_changes_saved_or_exported_output() {
    let mut s = session();
    let h = &mut s.harness;
    click_viewer_layer(h, "Input");
    let _ = composed(h, 10.0);
    let _output = render(h, "PREVIEW");
    let (_, time) = settled(h, 20.0);
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::Output);
    assert!(h.state().output_image.is_some());

    let mut options = h.state().export_options.clone();
    options.format = export::ExportFormat::Exr;
    options.depth = BitDepth::ThirtyTwo;
    options.saving_cctf_encoding = false;
    let ctx = h.ctx.clone();
    let save_before = s.directory.path().join("compare-off-save.exr");
    let export_before = s.directory.path().join("compare-off-export.exr");
    h.state_mut().save_to_path(&save_before);
    h.state_mut()
        .start_export_to_path(&ctx, options.clone(), &export_before);
    wait_until(h, |app| app.export_job.is_none());

    click(h, "compare");
    assert!(h.state().compare_active);
    assert!(h.query_by_label("compare ON").is_some());

    set_compare_split(h, 1.0, time + 1.0);
    let input_reference = composed(h, time + 1.1);
    set_compare_split(h, 0.0, time + 2.0);
    let output_reference = composed(h, time + 2.1);
    assert_ne!(
        pixel_at_fraction(&input_reference, 0.5, 0.5),
        pixel_at_fraction(&output_reference, 0.5, 0.5),
        "before and after source rasters must be observably different"
    );

    set_compare_split(h, 0.25, time + 3.0);
    let left_split = composed(h, time + 3.1);
    set_compare_split(h, 0.75, time + 4.0);
    let right_split = composed(h, time + 4.1);
    assert_eq!(
        pixel_at_fraction(&left_split, SPLIT_SAMPLE_MARGIN, 0.5),
        pixel_at_fraction(&input_reference, SPLIT_SAMPLE_MARGIN, 0.5),
        "left side must sample the before/input raster"
    );
    assert_eq!(
        pixel_at_fraction(&left_split, 1.0 - SPLIT_SAMPLE_MARGIN, 0.5),
        pixel_at_fraction(&output_reference, 1.0 - SPLIT_SAMPLE_MARGIN, 0.5),
        "right side must sample the after/output raster"
    );
    assert_eq!(
        pixel_at_fraction(&right_split, SPLIT_SAMPLE_MARGIN, 0.5),
        pixel_at_fraction(&input_reference, SPLIT_SAMPLE_MARGIN, 0.5)
    );
    assert_eq!(
        pixel_at_fraction(&right_split, 1.0 - SPLIT_SAMPLE_MARGIN, 0.5),
        pixel_at_fraction(&output_reference, 1.0 - SPLIT_SAMPLE_MARGIN, 0.5)
    );

    set_compare_split(h, 0.4, time + 5.0);
    let below_split = composed(h, time + 5.1);
    set_compare_split(h, 0.6, time + 6.0);
    let above_split = composed(h, time + 6.1);
    assert_ne!(
        pixel_at_fraction(&below_split, 0.5, 0.5),
        pixel_at_fraction(&above_split, 0.5, 0.5),
        "moving the split must exchange the before/after source at the center"
    );

    let save_after = s.directory.path().join("compare-on-save.exr");
    let export_after = s.directory.path().join("compare-on-export.exr");
    h.state_mut().save_to_path(&save_after);
    h.state_mut()
        .start_export_to_path(&ctx, options, &export_after);
    wait_until(h, |app| app.export_job.is_none());
    assert_float_images_equal(&save_before, &save_after);
    assert_float_images_equal(&export_before, &export_after);
}

#[test]
fn empty_histogram_shows_its_no_output_paint_branch_without_a_retained_render() {
    let mut s = session();
    let h = &mut s.harness;
    // `apply_state` arms a startup preview (`force_preview`/`dirty`) behind the
    // wall-clock debounce, so a heavily loaded run could land an output image
    // during the fixture's frames. Neutralize the pending preview and drive the
    // strip from a deterministically absent output through a real frame.
    h.state_mut().force_preview = false;
    h.state_mut().dirty = false;
    h.state_mut().output_image = None;
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
    let _ = render(h, "PREVIEW");
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
    click_viewer_layer(h, "Input");
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

fn paper_pixel(raster: &DisplayRaster, position: Pos2, rect: Rect) -> Color32 {
    let uv = (position - rect.min) / rect.size();
    let rgb = raster.sample(
        uv.x * raster.size[0] as f32 - 0.5,
        uv.y * raster.size[1] as f32 - 0.5,
        Interpolation::Spline36,
    );
    Color32::from_rgb(
        (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[test]
fn paper_back_composition_shows_watermark_pixels_in_the_documented_center_region() {
    let mut s = session();
    let h = &mut s.harness;
    click_viewer_layer(h, "Paper back");
    let observation = composed(h, 50.0);
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::PaperBack);
    let image = observation.pixels.as_ref().unwrap();
    let source = h.state().image.as_ref().unwrap();
    let long = source.width().max(source.height()).max(1);
    let paper_size = [
        (source.width() as f64 * 1024.0 / long as f64)
            .round()
            .max(1.0) as usize,
        (source.height() as f64 * 1024.0 / long as f64)
            .round()
            .max(1.0) as usize,
    ];
    let watermarked = display::virtual_paper_back(paper_size);
    let unwatermarked = display::virtual_paper_back_without_watermark(paper_size);

    // Match gui_viewer_acceptance.py's crop: 190 × 150 pixels around canvas center.
    let center = observation.image_rect.center();
    let region = Rect::from_min_max(
        center - Vec2::new(190.0, 150.0),
        center + Vec2::new(190.0, 150.0),
    )
    .intersect(observation.viewport)
    .intersect(observation.image_rect);
    let x0 = (((region.left() - observation.viewport.left()) / observation.viewport.width())
        * image.size[0] as f32)
        .floor()
        .max(0.0) as usize;
    let x1 = (((region.right() - observation.viewport.left()) / observation.viewport.width())
        * image.size[0] as f32)
        .ceil()
        .min(image.size[0] as f32) as usize;
    let y0 = (((region.top() - observation.viewport.top()) / observation.viewport.height())
        * image.size[1] as f32)
        .floor()
        .max(0.0) as usize;
    let y1 = (((region.bottom() - observation.viewport.top()) / observation.viewport.height())
        * image.size[1] as f32)
        .ceil()
        .min(image.size[1] as f32) as usize;
    let mut watermark_pixels = 0usize;
    let mut observed_pixels = 0usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let position = Pos2::new(
                observation.viewport.left()
                    + (x as f32 + 0.5) * observation.viewport.width() / image.size[0] as f32,
                observation.viewport.top()
                    + (y as f32 + 0.5) * observation.viewport.height() / image.size[1] as f32,
            );
            if !observation.image_rect.contains(position) {
                continue;
            }
            let actual = image.pixels[y * image.size[0] + x];
            let expected_watermarked = paper_pixel(&watermarked, position, observation.image_rect);
            let expected_unwatermarked =
                paper_pixel(&unwatermarked, position, observation.image_rect);
            assert_eq!(
                actual, expected_watermarked,
                "paper-back composition at ({x}, {y})"
            );
            observed_pixels += 1;
            if actual != expected_unwatermarked {
                watermark_pixels += 1;
            }
        }
    }
    assert!(
        observed_pixels > 0,
        "documented region must overlap the paper image"
    );
    assert!(
        watermark_pixels > 0,
        "watermark pixels must differ from the glare-only paper back"
    );
}
