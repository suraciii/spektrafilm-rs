//! Consumer-visible viewer acceptance through the real App and display worker.
use super::*;
use crate::display::{Interpolation, ViewLayer, ViewerObservation};

use super::panels::assert_float_images_equal;

fn session() -> Session {
    let mut s = super::session();
    preview(&mut s.harness);
    settle_animation(&mut s.harness);
    click(&mut s.harness, "Original");
    s
}

fn assert_rendering_indicator(h: &Harness<'_, App>) {
    let text = crate::i18n::text(&h.ctx, "surfaces.viewer.rendering");
    let label = h.query_by_label(text.as_ref());
    assert_eq!(label.is_some(), h.state().viewer_render_pending());
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    await_input(h);
    h.ctx.style_mut(|style| style.animation_time = 0.0);
    // Settle egui collapsing-panel geometry before queuing pointer actions.
    // This disables only widget animation; Viewer development still uses
    // RawInput.time and waits for every real composition response.
    h.step();
    if matches!(label, "Original" | "Modified" | "Paper back") {
        let viewport = h.state().viewer.observation().viewport;
        let nodes: Vec<_> = h
            .query_all_by_label(label)
            .filter(|node| {
                node.raw_bounds().is_some_and(|bounds| {
                    // Layer controls are immediately above the canvas and share
                    // its horizontal bounds; same-named parameter headers do not.
                    let x = (bounds.x0 + bounds.x1) as f32 * 0.5;
                    x >= viewport.left() && x <= viewport.right()
                })
            })
            .collect();
        assert_eq!(
            nodes.len(),
            1,
            "viewer layer control is unavailable or ambiguous: {label}"
        );
        nodes[0].simulate_click();
        h.run_steps(3);
    } else {
        if matches!(label, "100%" | "200%" | "400%") {
            super::click(h, label);
            return;
        }
        assert!(
            h.query_by_label(label).is_some(),
            "viewer control is unavailable: {label}"
        );
        super::click(h, label);
    }
}
fn frame(h: &mut Harness<'_, App>, time: f64) {
    // UI helpers and worker waits advance the harness clock implicitly. Never
    // rewind it when returning to explicitly timed viewer frames.
    h.input_mut().time = Some(time.max(h.ctx.input(|input| input.time)));
    h.step();
}

// Freeze egui time while the real worker finishes. Repaint notifications wake
// this wait; elapsed wall time never determines an animation frame.
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
        // App repaint requests can arrive before worker completion. A bounded
        // notification wait handles both without a sleep or advancing time.
        let _ = rx.recv_timeout(Duration::from_millis(20));
    }
}

fn develop(h: &mut Harness<'_, App>, time: f64) {
    let deadline = Instant::now() + WAIT_DEADLINE;
    loop {
        frame(h, time);
        if !h.state().dirty && h.state().render_job.is_none() && h.state().output_image.is_some() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "render timed out: {}",
            h.state().status
        );
        wait_tick();
    }
}

fn finish_animation(h: &mut Harness<'_, App>, mut time: f64) -> f64 {
    let initial = composed(h, time);
    time = h.ctx.input(|input| input.time);
    let mut previous = initial.transition.expect("first output or Scan animates");
    let initial_pixels = initial.pixels.unwrap().pixels;
    for _ in 0..51 {
        time += 0.04;
        let observation = composed(h, time);
        assert_rendering_indicator(h);
        match observation.transition {
            Some(current) => {
                assert_eq!(current.1, previous.1);
                assert_eq!(current.0, previous.0 + 1);
                // Reveal changes the presented pixels; a repeated Scan can
                // crossfade identical accepted samples without a byte delta.
                if current.1 && current.0 == 25 {
                    let pixels = &observation.pixels.as_ref().unwrap().pixels;
                    let max_delta = pixels
                        .iter()
                        .zip(&initial_pixels)
                        .flat_map(|(a, b)| (0..3).map(move |c| (a[c] as i16 - b[c] as i16).abs()))
                        .max()
                        .unwrap();
                    assert!(
                        max_delta > 1,
                        "animation middle-frame byte delta: {max_delta}"
                    );
                }
                previous = current;
            }
            None => {
                if previous.1 {
                    assert_eq!(previous.0, 49);
                    assert_ne!(observation.pixels.unwrap().pixels, initial_pixels);
                } else {
                    assert_eq!(previous.0, 9);
                }
                return time;
            }
        }
    }
    panic!("animation did not complete");
}

fn display_controls(h: &mut Harness<'_, App>) {
    reveal_control(h, "extras.rust.display_profile");
}

fn save_and_export_float_images(h: &mut Harness<'_, App>, save: &Path, export: &Path) {
    close_settings(h);
    let retained = h.state().output_image.as_ref().unwrap().clone();
    script(h, chooser::Chooser::chooser_path(save));
    click(h, "SAVE");
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &retained
    ));
    open_export(h);
    choose(h, "Format", "EXR");
    choose(h, "Bit depth", "32 bit");
    script(h, chooser::Chooser::chooser_path(export));
    export_action(h, "Export file…");
    wait_until(h, |app| app.export_job.is_none() && export.is_file());
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &retained
    ));
}

fn pointer(h: &mut Harness<'_, App>, position: egui::Pos2, pressed: bool, time: f64) {
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

#[test]
fn locating_viewer_popovers_keeps_choices_open_without_executing_them() {
    let mut s = session();
    let h = &mut s.harness;
    let params = h.state().params.clone();
    for (path, label, percent) in [
        ("viewer.zoom.100", "100%", 100.0),
        ("viewer.zoom.200", "200%", 200.0),
        ("viewer.zoom.400", "400%", 400.0),
    ] {
        let before = h.state().viewer.zoom_percent;
        reveal_control(h, path);
        assert_eq!(h.state().viewer.zoom_percent, before);
        for choice in ["100%", "200%", "400%"] {
            assert!(h.query_by_label(choice).is_some(), "missing {choice}");
        }
        // Select the already-open menu directly: searching again would conceal
        // a popup that closed on its locator's own outside-click event.
        h.get_by_label(label).simulate_click();
        h.run_steps(6);
        assert_eq!(h.state().viewer.zoom_percent, Some(percent));
    }
    for (path, label) in [
        ("viewer.reveal", "reveal transition"),
        ("viewer.crossfade", "crossfade transition"),
    ] {
        let before = (
            h.state().viewer.settings.reveal,
            h.state().viewer.settings.crossfade,
        );
        reveal_control(h, path);
        assert_eq!(
            (
                h.state().viewer.settings.reveal,
                h.state().viewer.settings.crossfade,
            ),
            before,
            "locating must not toggle either transition"
        );
        assert!(h.query_by_label("reveal transition").is_some());
        assert!(h.query_by_label("crossfade transition").is_some());
        h.get_by_label(label).simulate_click();
        h.run_steps(6);
        let expected = if path == "viewer.reveal" {
            (!before.0, before.1)
        } else {
            (before.0, !before.1)
        };
        assert_eq!(
            (
                h.state().viewer.settings.reveal,
                h.state().viewer.settings.crossfade,
            ),
            expected
        );
        click(h, "fit");
        assert!(h.query_by_label("reveal transition").is_none());
        assert!(h.query_by_label("crossfade transition").is_none());
    }
    assert_eq!(h.state().params, params);
}

#[test]
fn layers_and_interpolation_change_output_but_preserve_input_pixels() {
    let mut s = session();
    let h = &mut s.harness;
    develop(h, 10.0);
    click(h, "Original");
    let input = composed(h, 10.0);
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::Input);
    assert_eq!(input.interpolation, Interpolation::Nearest);
    click(h, "Paper back");
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::PaperBack);
    assert_eq!(composed(h, 10.0).interpolation, Interpolation::Spline36);
    develop(h, 10.0);
    settle_animation(h);
    click(h, "400%");
    display_controls(h);
    choose(h, "output interpolation", "nearest");
    close_settings(h);
    click(h, "Modified");
    let nearest = composed(h, 100.0);
    assert_eq!(nearest.interpolation, Interpolation::Nearest);
    display_controls(h);
    choose(h, "output interpolation", "spline36");
    close_settings(h);
    let spline = composed(h, 100.0);
    assert_eq!(spline.interpolation, Interpolation::Spline36);
    let nearest = nearest.pixels.unwrap();
    let spline = spline.pixels.unwrap();
    let delta = nearest
        .pixels
        .iter()
        .zip(&spline.pixels)
        .map(|(a, b)| {
            (0..3)
                .map(|c| (a[c] as f64 - b[c] as f64).abs())
                .sum::<f64>()
        })
        .sum::<f64>()
        / (nearest.pixels.len() * 3) as f64;
    assert!(
        delta > 0.01,
        "nearest/spline36 mean byte difference: {delta}"
    );
    click(h, "Original");
    let baseline = composed(h, 100.0).pixels.unwrap();
    display_controls(h);
    choose(h, "output interpolation", "nearest");
    close_settings(h);
    let input = composed(h, 100.0);
    assert_eq!(input.interpolation, Interpolation::Nearest);
    assert_eq!(baseline.pixels, input.pixels.unwrap().pixels);
}

#[test]
fn exact_zoom_obeys_dpi_drag_pans_double_click_resets_and_hover_reads_float_source() {
    for dpi in [1.0, 2.0] {
        let fixture = session();
        let state_path = fixture.directory.path().join("initial.json");
        let image_path = fixture.directory.path().join("asymmetric-ramp.png");
        let browser_path = fixture.directory.path().join(format!("dpi-{dpi}.json"));
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1501.0, 1401.0))
            .with_pixels_per_point(dpi)
            .build_eframe(move |cc| {
                theme::apply(&cc.egui_ctx);
                app(state_path, image_path, browser_path)
            });
        let h = &mut harness;
        h.input_mut().max_texture_side = Some(4096);
        preview(h);
        settle_animation(h);
        click(h, "Original");
        assert_eq!(h.state().viewer.settings.layer, ViewLayer::Input);
        h.state_mut().viewer.settings.gray_18_canvas = false;
        let mut time = h.ctx.input(|input| input.time);
        for (label, percent) in [("100%", 100.0), ("200%", 200.0), ("400%", 400.0)] {
            click(h, label);
            time = time.max(h.ctx.input(|input| input.time)) + 1.0;
            let (source_width, source_height) = {
                let source = h.state().image.as_ref().unwrap();
                (source.width(), source.height())
            };
            let expected_width = (source_width as f32 * percent / 100.0) as usize;
            let expected_height = (source_height as f32 * percent / 100.0) as usize;
            let viewport = h.state().viewer.observation().viewport;
            let image_min_x = viewport.center().x * dpi - expected_width as f32 / 2.0;
            let image_min_y = viewport.center().y * dpi - expected_height as f32 / 2.0;
            h.state_mut().viewer.pan = egui::vec2(
                (-6.5 - image_min_x.fract()) / dpi,
                (-4.5 - image_min_y.fract()) / dpi,
            );
            let observation = composed(h, time);
            assert_eq!(h.state().viewer.zoom_percent, Some(percent));
            let ratio =
                observation.image_rect.width() * h.ctx.pixels_per_point() / source_width as f32;
            assert!(
                (ratio - percent / 100.0).abs() < 1e-5,
                "device pixel mapping: {ratio}"
            );
            assert!(h.state().viewer.pan.x < 0.0 && h.state().viewer.pan.y < 0.0);
            for origin in [
                observation.image_rect.min.x * dpi,
                observation.image_rect.min.y * dpi,
            ] {
                assert!(
                    (origin - origin.round()).abs() > 0.05,
                    "fractional device origin: {origin}"
                );
            }
            let first_device_pixel =
                |coordinate: f32| (f64::from(coordinate) * f64::from(dpi) - 0.5).ceil() as isize;
            let pixels = observation
                .pixels
                .as_ref()
                .expect("composed presentation pixels");
            let viewport_pixel_extent = |min: f32, max: f32| {
                let start = (f64::from(min) * f64::from(dpi) - 0.5).ceil();
                let end = (f64::from(max) * f64::from(dpi) - 0.5).ceil();
                (end - start).max(1.0) as usize
            };
            let expected_viewport_size = [
                viewport_pixel_extent(observation.viewport.min.x, observation.viewport.max.x),
                viewport_pixel_extent(observation.viewport.min.y, observation.viewport.max.y),
            ];
            assert_eq!(
                pixels.size, expected_viewport_size,
                "unclipped viewer texture required: dpi={dpi}, pixels={:?}, viewport={:?}, image_rect={:?}",
                pixels.size, observation.viewport, observation.image_rect
            );
            let viewport_first_x = first_device_pixel(observation.viewport.min.x);
            let viewport_first_y = first_device_pixel(observation.viewport.min.y);
            let image_first_x = first_device_pixel(observation.image_rect.min.x);
            let image_first_y = first_device_pixel(observation.image_rect.min.y);
            let center_x = (image_first_x - viewport_first_x) as usize + expected_width / 2;
            let center_y = (image_first_y - viewport_first_y) as usize + expected_height / 2;
            assert!(center_x < pixels.size[0] && center_y < pixels.size[1]);
            let is_image_pixel = |pixel: egui::Color32| {
                pixel != egui::Color32::BLACK && pixel != egui::Color32::WHITE
            };
            let row = &pixels.pixels[center_y * pixels.size[0]..(center_y + 1) * pixels.size[0]];
            let image_width = row.iter().filter(|pixel| is_image_pixel(**pixel)).count();
            let black_width = row
                .iter()
                .filter(|pixel| **pixel == egui::Color32::BLACK)
                .count();
            let white_width = row
                .iter()
                .filter(|pixel| **pixel == egui::Color32::WHITE)
                .count();
            assert_eq!(
                image_width,
                expected_width,
                "{percent}% width: image={image_width}, expected={expected_width}, black={black_width}, white={white_width}, texture={:?}, viewport={:?}, image_rect={:?}, dpi={dpi}, texel_center=({center_x},{center_y}), point_center={:?}",
                pixels.size,
                observation.viewport,
                observation.image_rect,
                observation.image_rect.center()
            );
            let column = pixels.pixels.iter().skip(center_x).step_by(pixels.size[0]);
            let image_height = column
                .clone()
                .filter(|pixel| is_image_pixel(**pixel))
                .count();
            let black_height = column
                .clone()
                .filter(|pixel| **pixel == egui::Color32::BLACK)
                .count();
            let white_height = column
                .filter(|pixel| **pixel == egui::Color32::WHITE)
                .count();
            assert_eq!(
                image_height,
                expected_height,
                "{percent}% height: image={image_height}, expected={expected_height}, black={black_height}, white={white_height}, texture={:?}, viewport={:?}, image_rect={:?}, dpi={dpi}, texel_center=({center_x},{center_y}), point_center={:?}",
                pixels.size,
                observation.viewport,
                observation.image_rect,
                observation.image_rect.center()
            );
            let next_x = (image_first_x + expected_width as isize) as f32;
            assert!(
                ((image_first_x as f32 + 0.5) / dpi - observation.image_rect.min.x).abs() < 0.01
            );
            assert!(((next_x + 0.5) / dpi - observation.image_rect.max.x).abs() < 0.01);
            let next_y = (image_first_y + expected_height as isize) as f32;
            assert!(
                ((image_first_y as f32 + 0.5) / dpi - observation.image_rect.min.y).abs() < 0.01
            );
            assert!(((next_y + 0.5) / dpi - observation.image_rect.max.y).abs() < 0.01);
        }
        assert_eq!(h.ctx.pixels_per_point(), dpi);
        time += 1.0;
        h.state_mut().viewer.pan = egui::Vec2::ZERO;
        let observation = composed(h, time);
        let center = observation.viewport.center();
        pointer(h, center, true, time);
        h.input_mut()
            .events
            .push(egui::Event::PointerMoved(center + egui::vec2(34.0, 21.0)));
        frame(h, time + 0.1);
        pointer(h, center + egui::vec2(34.0, 21.0), false, time + 0.2);
        assert!((h.state().viewer.pan - egui::vec2(34.0, 21.0)).length() < 0.01);
        pointer(h, center, true, time + 1.0);
        pointer(h, center, false, time + 1.01);
        pointer(h, center, true, time + 1.10);
        pointer(h, center, false, time + 1.11);
        assert_eq!(h.state().viewer.pan, egui::Vec2::ZERO);
        assert_eq!(h.state().viewer.zoom_percent, None);
        assert_eq!(h.state().viewer.zoom, 1.0);
        let observation = composed(h, time + 2.0);
        let position =
            observation.image_rect.min + observation.image_rect.size() * egui::vec2(0.31, 0.63);
        h.input_mut()
            .events
            .push(egui::Event::PointerMoved(position));
        frame(h, time + 2.0);
        let inspection = h.state().viewer.observation().inspection.unwrap();
        let source = h.state().image.as_ref().unwrap().host_image().unwrap();
        assert_eq!(inspection.layer, ViewLayer::Input);
        assert_eq!((inspection.x, inspection.y), (49, 50));
        for (actual, expected) in inspection.rgb.into_iter().zip(source.get(49, 50)) {
            assert!((actual - expected as f64).abs() <= 1e-6);
        }
        eframe::App::on_exit(h.state_mut(), None);
    }
}

#[test]
fn click_steps_the_zoom_at_the_pointer_and_a_double_click_fits() {
    let mut s = session();
    let h = &mut s.harness;
    click(h, "Original");
    let baseline = composed(h, 10.0);
    let time = h.ctx.input(|input| input.time);
    let image = baseline.image_rect;
    let at = image.center() + egui::vec2(24.0, -18.0);
    let fraction = (at - image.min) / image.size();
    pointer(h, at, true, time + 1.0);
    pointer(h, at, false, time + 1.1);
    let zoomed = composed(h, time + 2.0);
    assert!((h.state().viewer.zoom - 2.0).abs() < 1e-6);
    assert_eq!(h.state().viewer.zoom_percent, None);
    assert!((zoomed.image_rect.size() - image.size() * 2.0).length() < 0.5);
    // The clicked content stays under the pointer.
    let landed = zoomed.image_rect.min + fraction * zoomed.image_rect.size();
    assert!(
        (landed - at).length() < 1.5,
        "clicked content moved: {landed:?}"
    );
    // A double click fits the view again.
    pointer(h, at, true, time + 3.0);
    pointer(h, at, false, time + 3.1);
    pointer(h, at, true, time + 3.2);
    pointer(h, at, false, time + 3.3);
    assert_eq!(h.state().viewer.zoom, 1.0);
    assert_eq!(h.state().viewer.zoom_percent, None);
    assert_eq!(h.state().viewer.pan, egui::Vec2::ZERO);
}

#[test]
fn marquee_release_zooms_to_the_selection() {
    let mut s = session();
    let h = &mut s.harness;
    click(h, "Original");
    let baseline = composed(h, 20.0);
    let time = h.ctx.input(|input| input.time);
    let viewport = baseline.viewport;
    let image = baseline.image_rect;
    let size = egui::vec2(80.0, 50.0);
    let start = image.min + image.size() * egui::vec2(0.3, 0.4);
    let end = start + size;
    assert!(image.contains_rect(egui::Rect::from_two_pos(start, end)));
    let fraction = (start + size * 0.5 - image.min) / image.size();
    let shift = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    // The harness input resets modifiers between frames, so each event batch
    // carries the held Shift.
    h.input_mut().modifiers = shift;
    h.input_mut().events.extend([
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: shift,
        },
    ]);
    frame(h, time + 1.0);
    h.input_mut().modifiers = shift;
    h.input_mut().events.push(egui::Event::PointerMoved(end));
    frame(h, time + 1.1);
    h.input_mut().modifiers = shift;
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: shift,
    });
    frame(h, time + 1.2);
    h.input_mut().modifiers = egui::Modifiers::default();
    let zoomed = composed(h, time + 2.0);
    let expected = (viewport.width() / size.x).min(viewport.height() / size.y);
    assert!(
        (h.state().viewer.zoom - expected).abs() < 1e-4,
        "marquee zoom: {} expected {expected}",
        h.state().viewer.zoom
    );
    // The selection center lands at the view center.
    let landed = zoomed.image_rect.min + fraction * zoomed.image_rect.size();
    assert!(
        (landed - viewport.center()).length() < 1.5,
        "selection center moved: {landed:?}"
    );
    // The secondary button selects without a modifier.
    let before = h.state().viewer.zoom;
    let start = image.center();
    let end = start + egui::vec2(60.0, 40.0);
    h.input_mut().events.extend([
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        },
    ]);
    frame(h, time + 3.0);
    h.input_mut().events.push(egui::Event::PointerMoved(end));
    frame(h, time + 3.1);
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    frame(h, time + 3.2);
    assert!(
        h.state().viewer.zoom > before,
        "secondary marquee must zoom further"
    );
}

#[test]
fn gray_canvas_and_white_padding_change_composed_pixels_and_restore_baseline() {
    let mut s = session();
    let h = &mut s.harness;
    click(h, "Original");
    display_controls(h);
    let time = h.ctx.input(|input| input.time) + 1.0;
    let baseline = composed(h, time);
    assert_eq!(
        baseline.pixels.as_ref().unwrap().pixels[0],
        egui::Color32::from_gray(118)
    );
    click(h, "gray 18% canvas");
    let time = h.ctx.input(|input| input.time) + 1.0;
    let black = composed(h, time);
    assert_ne!(baseline.key, black.key);
    assert_ne!(
        baseline.pixels.as_ref().unwrap().pixels,
        black.pixels.as_ref().unwrap().pixels
    );
    assert_eq!(
        black.pixels.as_ref().unwrap().pixels[0],
        egui::Color32::BLACK
    );
    click(h, "gray 18% canvas");
    let time = h.ctx.input(|input| input.time) + 1.0;
    assert_eq!(
        baseline.pixels.as_ref().unwrap().pixels,
        composed(h, time).pixels.unwrap().pixels
    );
    edit(h, "white padding", "0.2");
    let time = h.ctx.input(|input| input.time) + 1.0;
    let padded = composed(h, time);
    assert!(padded.image_rect.width() < baseline.image_rect.width());
    let image = padded.pixels.as_ref().unwrap();
    let border_point = egui::pos2(padded.image_rect.left() - 5.0, padded.image_rect.center().y);
    let uv = (border_point - padded.viewport.min) / padded.viewport.size();
    let x = (uv.x * image.size[0] as f32).floor() as usize;
    let y = (uv.y * image.size[1] as f32).floor() as usize;
    assert_eq!(image.pixels[y * image.size[0] + x], egui::Color32::WHITE);
    assert!((h.state().viewer.settings.white_padding - 0.2).abs() < 1e-6);
    assert_ne!(baseline.key, padded.key);
    assert_ne!(
        baseline.pixels.as_ref().unwrap().pixels,
        padded.pixels.unwrap().pixels
    );
    edit(h, "white padding", "0.03");
    let time = h.ctx.input(|input| input.time) + 1.0;
    assert_eq!(
        baseline.pixels.unwrap().pixels,
        composed(h, time).pixels.unwrap().pixels
    );
}

#[test]
fn white_padding_modified_locator_and_display_reset_follow_live_and_persisted_viewer_settings() {
    let mut s = session();
    let h = &mut s.harness;
    let factory = state::GuiState::factory().sections["display"]["white_padding"]
        .as_f64()
        .expect("factory white padding");
    let params = h.state().params.clone();
    reveal_control(h, "extras.display.white_padding");
    edit(h, "white padding", "0.2");
    assert!((h.state().viewer.settings.white_padding as f64 - 0.2).abs() < 1e-6);
    assert!(
        (h.state().current_state().unwrap().sections["display"]["white_padding"]
            .as_f64()
            .unwrap()
            - 0.2)
            .abs()
            < 1e-6
    );

    close_settings(h);
    click(h, "Only modified");
    let locator = "Locate Settings: Display";
    assert!(h.query_by_label(locator).is_some());
    click(h, locator);
    click(h, "reset Display");
    assert!((h.state().viewer.settings.white_padding as f64 - factory).abs() < 1e-6);
    assert!(
        (h.state().current_state().unwrap().sections["display"]["white_padding"]
            .as_f64()
            .unwrap()
            - factory)
            .abs()
            < 1e-6
    );
    assert_eq!(h.state().params, params);
    close_settings(h);
    assert!(
        h.query_by_label(locator).is_none(),
        "factory white padding leaves the modified index"
    );
}

#[test]
fn display_reset_restores_the_full_live_viewer_settings_without_adjustment_history() {
    let mut s = session();
    let h = &mut s.harness;
    let factory =
        display::DisplaySettings::from_json(&state::GuiState::factory().sections["display"]);
    let params = h.state().params.clone();
    let revision = h.state().parameter_revision;
    let undo_label = h.state().undo_label_localized(&h.ctx);
    reveal_control(h, "extras.display.use_display_transform");
    click(h, "use display transform");
    click(h, "gray 18% canvas");
    choose(
        h,
        "output interpolation",
        if factory.interpolation == Interpolation::Nearest {
            "spline36"
        } else {
            "nearest"
        },
    );
    edit(h, "white padding", "0.2");
    let changed = h.state().viewer.settings.clone();
    assert_ne!(changed.use_display_transform, factory.use_display_transform);
    assert_ne!(changed.gray_18_canvas, factory.gray_18_canvas);
    assert_ne!(changed.interpolation, factory.interpolation);
    assert!((changed.white_padding - factory.white_padding).abs() > 1e-6);

    click(h, "reset Display");
    let restored = &h.state().viewer.settings;
    assert_eq!(
        restored.use_display_transform,
        factory.use_display_transform
    );
    assert_eq!(restored.gray_18_canvas, factory.gray_18_canvas);
    assert_eq!(restored.interpolation, factory.interpolation);
    assert!((restored.white_padding - factory.white_padding).abs() < 1e-6);

    let persisted = h.state().current_state().unwrap();
    let display = &persisted.sections["display"];
    assert_eq!(
        display["use_display_transform"],
        serde_json::json!(factory.use_display_transform)
    );
    assert_eq!(
        display["gray_18_canvas"],
        serde_json::json!(factory.gray_18_canvas)
    );
    assert_eq!(
        display["output_interpolation"],
        serde_json::json!(factory.interpolation.name())
    );
    assert!(
        (display["white_padding"].as_f64().unwrap() - factory.white_padding as f64).abs() < 1e-6
    );
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().parameter_revision, revision);
    assert_eq!(h.state().undo_label_localized(&h.ctx), undo_label);
}

#[test]
fn first_output_and_scan_animate_but_automatic_parameter_update_does_not() {
    let directory = tempfile::tempdir().unwrap();
    let (image_path, state_path) = deterministic_inputs(directory.path());
    let browser_path = directory.path().join("browser.json");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            let mut app = app(state_path, image_path, browser_path);
            assert!(app.viewer_render_pending());
            // Enable reveal before the first automatic output can be accepted.
            app.viewer.settings.reveal = true;
            app
        });
    let h = &mut harness;
    develop(h, 0.0);
    assert_eq!(composed(h, 0.0).transition, Some((0, true)));
    assert_rendering_indicator(h);
    let time = finish_animation(h, 0.0);
    reveal_control(h, "camera.exposure_compensation_ev");
    let before = h.state().output_image.clone().unwrap();
    edit(h, "exposure compensation ev", "0.75");
    develop(h, time);
    assert!(!Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &before
    ));
    assert_eq!(composed(h, time).transition, None);
    assert_rendering_indicator(h);

    // An explicit Scan on the output layer crossfades the previous output.
    close_settings(h);
    h.get_by_label("SCAN").simulate_click();
    develop(h, time + 1.0);
    assert!(h.state().status.to_string().starts_with("Scan ·"));
    assert_eq!(composed(h, time + 1.0).transition, Some((0, false)));
    let time = finish_animation(h, time + 1.0);

    // Scanning from another layer instead reveals the newly accepted output.
    click(h, "Paper back");
    h.get_by_label("SCAN").simulate_click();
    develop(h, time + 1.0);
    assert_eq!(composed(h, time + 1.0).transition, Some((0, true)));
    let time = finish_animation(h, time + 1.0);
    reveal_control(h, "camera.exposure_compensation_ev");
    let before = h.state().output_image.clone().unwrap();
    edit(h, "exposure compensation ev", "0.5");
    develop(h, time + 1.0);
    assert!(!Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &before
    ));
    assert!(h.state().status.to_string().starts_with("Preview ·"));
    assert_eq!(composed(h, time + 1.0).transition, None);
    assert_rendering_indicator(h);
}

#[test]
fn rendering_overlay_is_bilingual_and_settles_after_import_adjustment_and_failure() {
    let directory = tempfile::tempdir().unwrap();
    let (image_path, state_path) = deterministic_inputs(directory.path());
    let browser_path = directory.path().join("browser.json");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            App::new_with_browser_path(
                Arc::new(CpuBackend),
                None,
                Some(state_path),
                None,
                browser_path,
                data_dir(),
            )
        });
    assert!(!h.state().viewer_render_pending());
    assert_rendering_indicator(&h);
    let layer = h.state().viewer.settings.layer;
    click(&mut h, "Modified");
    assert_eq!(
        h.state().viewer.settings.layer,
        layer,
        "unavailable output is disabled"
    );

    let ctx = h.ctx.clone();
    h.state_mut()
        .select_language(&ctx, crate::i18n::LanguagePreference::SimplifiedChinese)
        .unwrap();
    let active = h.state().render_worker.active.clone();
    let worker_guard = active.lock();
    h.state_mut().load_image_from_path(&image_path);
    assert!(
        h.state().viewer_render_pending(),
        "input preparation and render debounce belong to viewer work"
    );
    frame(&mut h, 0.0);
    assert_rendering_indicator(&h);
    assert!(h.query_by_label("Rendering…").is_none());
    wait_until(&mut h, |app| app.render_job.is_some());
    assert!(h.state().viewer_render_pending());
    assert_rendering_indicator(&h);
    assert!(h.query_by_label("渲染中……").is_some());
    drop(worker_guard);
    develop(&mut h, 0.0);
    composed(&mut h, 0.0);
    assert!(!h.state().viewer_render_pending());
    assert_rendering_indicator(&h);

    h.state_mut()
        .select_language(&ctx, crate::i18n::LanguagePreference::English)
        .unwrap();
    h.step();
    reveal_control(&mut h, "camera.exposure_compensation_ev");
    let worker_guard = active.lock();
    edit(&mut h, "exposure compensation ev", "0.5");
    wait_until(&mut h, |app| app.render_job.is_some());
    assert!(h.state().viewer_render_pending());
    assert_rendering_indicator(&h);
    assert!(h.query_by_label("Rendering…").is_some());
    drop(worker_guard);
    develop(&mut h, 1.0);
    assert!(h.state().render_job.is_none());
    assert_eq!(
        h.state().viewer_render_pending(),
        h.state().viewer.presentation_pending()
    );
    composed(&mut h, 1.0);
    assert!(!h.state().viewer_render_pending());
    assert_rendering_indicator(&h);

    // An unavailable real profile must fail without retaining a spinner or
    // replacing the last successfully accepted output.
    let accepted = h.state().output_image.clone().unwrap();
    let reports = directory.path().join("failed-render-reports");
    std::fs::create_dir(&reports).unwrap();
    h.state_mut().diagnostics.mode = spektrafilm_core::telemetry::CollectionMode::Summary;
    h.state_mut()
        .diagnostics
        .publish_completed_to(reports.clone());
    // Change the external resource location, not accepted photograph controls.
    // History correctly rejects an invalid film-name edit before rendering.
    h.state_mut().data_dir = directory.path().join("missing-profile-data");
    h.state_mut().dispatch_render(&ctx);
    if h.state().render_job.is_some() {
        assert!(
            h.state().viewer_render_pending(),
            "unconsumed pipeline response is pending"
        );
    }
    wait_until(&mut h, |app| {
        app.render_job.is_none() && app.diagnostics.pending_publications() == 0
    });
    let report_path = std::fs::read_dir(&reports)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report["operation"]["outcome"], "failed");
    wait_until(&mut h, |app| !app.viewer_render_pending());
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &accepted
    ));
    assert_rendering_indicator(&h);
    assert!(h.query_by_label("渲染中……").is_none());
    assert!(h.query_by_label("PREVIEW").is_none());
}

#[test]
fn viewer_settings_do_not_change_saved_or_exported_pixels() {
    let mut s = session();
    let h = &mut s.harness;
    develop(h, 50.0);
    settle_animation(h);
    let save_before = s.directory.path().join("save-before.exr");
    let export_before = s.directory.path().join("export-before.exr");
    save_and_export_float_images(h, &save_before, &export_before);
    display_controls(h);
    click(h, "gray 18% canvas");
    click(h, "use display transform");
    edit(h, "white padding", "0.2");
    choose(h, "output interpolation", "nearest");
    close_settings(h);
    click(h, "Original");
    click(h, "400%");
    let save_after = s.directory.path().join("save-after.exr");
    let export_after = s.directory.path().join("export-after.exr");
    save_and_export_float_images(h, &save_after, &export_after);
    for (before, after) in [(save_before, save_after), (export_before, export_after)] {
        let before = image_io::load(&before).unwrap().image;
        let after = image_io::load(&after).unwrap().image;
        assert_eq!((before.width, before.height), (after.width, after.height));
        for (a, b) in before.data.iter().zip(&after.data) {
            assert!((*a as f64 - *b as f64).abs() <= 1e-6);
        }
    }
}

/// specs/gui-workflow/spec.md#viewer-presentation: the viewer must let the
/// user select a display ICC profile. The selection applies to the disposable
/// viewer raster only; the retained output, Save, and Export keep their own
/// pixels.
#[test]
fn display_profile_selection_clear_persistence_and_viewer_only_effect() {
    let mut s = session();
    let h = &mut s.harness;
    develop(h, 60.0);
    click(h, "Modified");
    settle_animation(h);
    display_controls(h);

    // The chooser boundary: a scripted result runs the real selection path.
    let profile = data_dir().join("icc/saucecontrol/DisplayP3-v4.icc");
    assert!(profile.is_file());
    script(h, chooser::Chooser::chooser_path(&profile));
    click(h, "select ICC profile…");
    assert_eq!(
        h.state().gui_state.display_profile(),
        Some(profile.to_str().unwrap()),
        "the explicit selection reaches the state carrier"
    );
    assert_eq!(
        h.state().viewer.output_display_profile(),
        Some(profile.as_path()),
        "the prepared viewing raster carries the profile"
    );

    // The retained output is not re-rendered, and the profile reaches the
    // composed viewer pixels through the display transform.
    let before = h.state().output_image.clone().unwrap();
    let without = composed(h, 60.0).pixels.unwrap();
    let with_profile = composed(h, 61.0).pixels.unwrap();
    assert_eq!(
        with_profile.pixels, without.pixels,
        "the composed raster is keyed on the accepted generation, not re-composed"
    );
    assert!(
        h.state()
            .output_image
            .as_ref()
            .is_some_and(|out| Arc::ptr_eq(out, &before)),
        "the retained output is not re-rendered by a viewer-only selection"
    );

    // Persistence: save the state and reload it through the real paths.
    let saved = s.directory.path().join("display-profile.json");
    let accepted = h.state().current_state().unwrap();
    accepted.save(&saved).unwrap();
    let restored = state::GuiState::load(&saved).unwrap();
    assert_eq!(restored.display_profile(), Some(profile.to_str().unwrap()));
    h.state_mut()
        .apply_state_with_source(restored, |_| {})
        .unwrap();
    await_input(h);
    assert_eq!(
        h.state().gui_state.display_profile(),
        Some(profile.to_str().unwrap()),
        "the reloaded state keeps the explicit selection"
    );
    // The reloaded state reloads the input, so the retained output is produced
    // again; the selection then reaches the freshly seeded viewing raster.
    close_settings(h);
    develop(h, 62.0);
    settle_animation(h);
    assert_eq!(
        h.state().viewer.output_display_profile(),
        Some(profile.as_path()),
        "the reloaded state seeds the viewing raster with the profile"
    );

    // Save and Export ignore the viewer-only selection.
    let save_profile = s.directory.path().join("profile-save.exr");
    let export_profile = s.directory.path().join("profile-export.exr");
    save_and_export_float_images(h, &save_profile, &export_profile);

    display_controls(h);
    click(h, "clear");
    assert_eq!(
        h.state().gui_state.display_profile(),
        None,
        "the selection clears"
    );
    #[cfg(windows)]
    assert_ne!(
        h.state().viewer.output_display_profile(),
        Some(profile.as_path()),
        "clearing the selection uses the platform fallback rather than the explicit profile"
    );
    #[cfg(not(windows))]
    assert_eq!(
        h.state().viewer.output_display_profile(),
        None,
        "clearing the selection removes the explicit profile on non-Windows platforms"
    );
    let cleared = h.state().current_state().unwrap();
    assert_eq!(cleared.display_profile(), None);
    let save_cleared = s.directory.path().join("cleared-save.exr");
    let export_cleared = s.directory.path().join("cleared-export.exr");
    save_and_export_float_images(h, &save_cleared, &export_cleared);
    assert_float_images_equal(&save_profile, &save_cleared);
    assert_float_images_equal(&export_profile, &export_cleared);
}

/// An unusable profile path must stay safe: the selection is retained, the
/// existing transform fallback keeps the encoded sRGB preview, and the state
/// document still loads.
#[test]
fn an_unusable_display_profile_path_falls_back_without_breaking_state() {
    let mut s = session();
    let h = &mut s.harness;
    develop(h, 70.0);
    click(h, "Modified");
    settle_animation(h);
    display_controls(h);

    let baseline = composed(h, 70.0).pixels.unwrap();
    let retained = h.state().output_image.as_ref().unwrap().clone();
    let missing = s.directory.path().join("absent-profile.icc");
    script(h, chooser::Chooser::chooser_path(&missing));
    click(h, "select ICC profile…");
    assert_eq!(
        h.state().gui_state.display_profile(),
        Some(missing.to_str().unwrap()),
        "the selection is retained so the user can see and correct it"
    );
    assert_eq!(composed(h, 70.0).pixels.unwrap().pixels, baseline.pixels);
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &retained
    ));

    let broken = s.directory.path().join("broken.icc");
    std::fs::write(&broken, b"not an icc profile").unwrap();
    script(h, chooser::Chooser::chooser_path(&broken));
    click(h, "select ICC profile…");
    assert_eq!(
        h.state().gui_state.display_profile(),
        Some(broken.to_str().unwrap())
    );

    assert_eq!(composed(h, 70.0).pixels.unwrap().pixels, baseline.pixels);
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &retained
    ));
    let saved = s.directory.path().join("broken-state.json");
    h.state_mut().current_state().unwrap().save(&saved).unwrap();
    let restored = state::GuiState::load(&saved).unwrap();
    assert_eq!(restored.display_profile(), Some(broken.to_str().unwrap()));
    h.state_mut()
        .apply_state_with_source(restored, |_| {})
        .unwrap();
    await_input(h);
    assert_eq!(
        h.state().gui_state.display_profile(),
        Some(broken.to_str().unwrap()),
        "state loading survives an unusable profile"
    );
    let composed = composed(h, 72.0);
    assert!(
        composed.pixels.is_some(),
        "the viewer still composes with the fallback raster"
    );
}

#[test]
fn canonical_search_preserves_inspector_and_viewer_geometry_at_supported_desktop_sizes() {
    for size in [egui::vec2(1460.0, 980.0), egui::vec2(1100.0, 800.0)] {
        let directory = tempfile::tempdir().unwrap();
        let (image_path, state_path) = deterministic_inputs(directory.path());
        let browser_path = directory.path().join("browser.json");
        let mut h = Harness::builder().with_size(size).build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            app(state_path, image_path, browser_path)
        });
        h.ctx.style_mut(|style| style.animation_time = 0.0);
        await_input(&mut h);
        h.run_steps(3);
        // Compact workspaces already start with the browser's reopen rail.
        if size.x > theme::WIDTH_COMPACT_WORKSPACE {
            click(&mut h, "◀");
        }
        assert!(h.query_by_label("▶").is_some());
        let params = h.state().params.clone();
        let initial_search = h.get_by_label("Search controls").raw_bounds().unwrap();
        let initial_viewer = h.state().viewer.observation().viewport;
        assert!(initial_viewer.width() > 0.0 && initial_viewer.height() > 0.0);

        for query in [
            "film_render.halation.halation_first_sigma_um",
            "film_render.grain.fotufilm.sublayer_shares",
            "",
        ] {
            search_controls(&mut h, query);
            let search = h.get_by_label("Search controls").raw_bounds().unwrap();
            let viewer = h.state().viewer.observation().viewport;
            assert!(
                (search.x0 - initial_search.x0).abs() <= 2.0,
                "inspector moved for {query:?} at {size:?}"
            );
            assert!(
                (viewer.left() - initial_viewer.left()).abs() <= 2.0,
                "viewer left moved for {query:?} at {size:?}"
            );
            assert!(
                (viewer.width() - initial_viewer.width()).abs() <= 2.0,
                "viewer width changed for {query:?} at {size:?}"
            );
            assert!(
                (viewer.height() - initial_viewer.height()).abs() <= 2.0,
                "viewer height changed for {query:?} at {size:?}"
            );
            assert_eq!(h.state().params, params, "search must not edit controls");
            assert!(h.query_by_label("PREVIEW").is_none());
            for action in ["SCAN", "SAVE"] {
                let bounds = h.get_by_label(action).raw_bounds().unwrap();
                assert!(
                    bounds.x0 >= search.x0 - 2.0 && bounds.x1 <= size.x as f64,
                    "{action} clipped horizontally at {size:?}"
                );
                assert!(
                    bounds.y0 >= 0.0 && bounds.y1 <= size.y as f64,
                    "{action} clipped vertically at {size:?}"
                );
            }
        }
    }
}

fn wait_device_output(h: &mut Harness<'_, App>) {
    wait_until(h, |app| {
        !app.dirty
            && app.render_job.is_none()
            && app.calibration_job.is_none()
            && !app.accepted_frame_is_stale()
            && app.output_retained.is_some()
    });
}

fn scan_device_output(h: &mut Harness<'_, App>) {
    // Unlike the CPU pixel helpers, readiness is the accepted device carrier
    // and its provenance, never a compatibility raster or a host readback.
    wait_device_output(h);
    let previous = h.state().source_render.as_ref().unwrap().operation_id;
    h.get_by_label("SCAN").simulate_click();
    h.run_steps(3);
    wait_until(h, |app| {
        !app.dirty
            && app.render_job.is_none()
            && !app.accepted_frame_is_stale()
            && app.output_retained.is_some()
            && app.retained_kind == Some(RenderKind::Scan)
            && app
                .source_render
                .as_ref()
                .is_some_and(|render| render.operation_id != previous)
    });
}

#[test]
fn gpu_gui_scan_view_transition_and_save_keep_the_accepted_device_samples() {
    let _device = spektrafilm_gpu::test_support::device_lock();
    let directory = tempfile::tempdir().unwrap();
    let (image_path, state_path) = deterministic_inputs(directory.path());
    let browser_path = directory.path().join("browser.json");
    let backend: Arc<dyn ComputeBackend> = Arc::new(
        spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit().expect("GPU adapter required"),
    );
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
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
    await_input(&mut h);
    wait_device_output(&mut h);
    let source = h.state().image.as_ref().unwrap().clone();
    let original = source.host_image().unwrap().clone();
    for (space, encoded) in [("Display P3", true), ("ACES2065-1", false)] {
        for turns in [-1, 1, 2, 4] {
            let cpu = source.rotated(turns, &CpuBackend, space, encoded).unwrap();
            let gpu = source
                .rotated(turns, h.state().backend.as_ref(), space, encoded)
                .unwrap();
            let expected = cpu.host_image().unwrap();
            let actual = gpu.to_host(h.state().backend.as_ref()).unwrap();
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height)
            );
            for (&actual, &expected) in actual.data.iter().zip(&expected.data) {
                assert_eq!(actual as f32, expected as f32, "quarter turns: {turns}");
            }
            if turns == 4 {
                assert!(
                    gpu.same_source(&source),
                    "a complete turn reuses the source"
                );
            } else {
                let crate::render::InputImage::Device(retained) = &gpu else {
                    panic!("selected GPU rotated host source samples on the CPU");
                };
                assert_eq!(retained.space().name, space);
                assert_eq!(
                    retained.domain(),
                    if encoded {
                        spektrafilm_backend::SampleDomain::Encoded
                    } else {
                        spektrafilm_backend::SampleDomain::Linear
                    }
                );
                assert_eq!(retained.pixel_pitch_um(), None);
                // Reinterpretation changes only this consumer's declaration;
                // rotation still restores the exact immutable source samples.
                let restored = gpu
                    .rotated(-turns, h.state().backend.as_ref(), "sRGB", !encoded)
                    .unwrap();
                let crate::render::InputImage::Device(restored) = restored else {
                    panic!("retained rotation lost its device carrier");
                };
                assert_eq!(restored.space().name, "sRGB");
                assert_eq!(
                    restored.domain(),
                    if !encoded {
                        spektrafilm_backend::SampleDomain::Encoded
                    } else {
                        spektrafilm_backend::SampleDomain::Linear
                    }
                );
                assert_eq!(restored.pixel_pitch_um(), None);
                let restored = restored.to_host(h.state().backend.as_ref()).unwrap();
                assert_eq!(
                    (restored.width, restored.height),
                    (original.width, original.height)
                );
                for (&actual, &expected) in restored.data.iter().zip(&original.data) {
                    assert_eq!(actual as f32, expected as f32);
                }
            }
        }
    }
    let cancelled = h
        .state()
        .backend
        .with_cancellation(Some(Arc::new(std::sync::atomic::AtomicBool::new(true))))
        .unwrap();
    assert!(
        source
            .rotated(1, cancelled.as_ref(), "sRGB", true)
            .unwrap_err()
            .contains("cancelled")
    );
    drop(cancelled);
    assert!(h.state().image.as_ref().unwrap().same_source(&source));
    assert_eq!(source.host_image().unwrap().data, original.data);
    h.state_mut().diagnostics.mode = spektrafilm_backend::telemetry::CollectionMode::Summary;
    scan_device_output(&mut h);
    assert!(
        h.state().output_image.is_none(),
        "GPU Scan must not retain a compatibility host raster"
    );
    assert!(
        h.state().output_histogram.is_some(),
        "GPU histogram belongs to accepted render"
    );
    let accepted = h.state().output_retained.clone().unwrap();
    let samples = accepted.to_host(h.state().backend.as_ref()).unwrap();
    assert_eq!(
        (samples.width, samples.height),
        (original.width, original.height)
    );
    let report = assert_retained_gpu_report(
        h.state(),
        &directory.path().join("scan-report.json"),
        "scan",
        [original.width, original.height],
    );
    let simulation = report
        .phases
        .iter()
        .find(|phase| phase.name == "simulation")
        .unwrap();
    assert_eq!(
        simulation.work.image_upload_count,
        Some(0),
        "Scan reuses the source uploaded by the accepted Preview"
    );
    assert_eq!(
        simulation.work.image_upload_bytes,
        Some(0),
        "Preview-to-Scan reuse must not upload the same host samples again"
    );
    let display = report
        .phases
        .iter()
        .find(|phase| phase.name == "display_prepare")
        .unwrap();
    assert!(
        report
            .stages
            .iter()
            .any(|stage| stage.name == "retained_histogram" && stage.parent_id == display.id)
    );
    h.state().render_worker.invalidate_runtime_cache();
    super::click(&mut h, "SCAN");
    wait_until(&mut h, |a| {
        !a.dirty && a.render_job.is_none() && a.output_retained.is_some()
    });
    let invalidated = assert_retained_gpu_report(
        h.state(),
        &directory.path().join("invalidated-scan-report.json"),
        "scan",
        [original.width, original.height],
    );
    let simulation = invalidated
        .phases
        .iter()
        .find(|phase| phase.name == "simulation")
        .unwrap();
    assert_eq!(
        simulation.work.image_upload_count,
        Some(1),
        "context invalidation releases the raw upload"
    );
    assert_eq!(
        simulation.work.image_upload_bytes,
        Some(u64::from(original.width) * u64::from(original.height) * 3 * 4),
    );
    // Worker waits advance egui's implicit clock by an unpredictable number
    // of frames. Continue from that clock rather than resetting it to a fixed
    // timestamp when switching to explicitly timed viewer composition.
    let time = h.ctx.input(|input| input.time) + 1.0;
    let fit = composed(&mut h, time);
    assert!(fit.pixels.is_some());
    assert!(
        h.state().histogram.cached_data().is_some(),
        "accepted device histogram is visible"
    );
    h.state_mut().viewer.set_zoom_percent(200.0);
    let magnified = composed(&mut h, time + 1.0);
    assert!(magnified.pixels.is_some());
    assert_eq!(
        accepted.to_host(h.state().backend.as_ref()).unwrap().data,
        samples.data
    );
    let saved = directory.path().join("accepted.tiff");
    save_retained(&mut h, &saved);
    assert!(saved.exists(), "{}", h.state().status);
    let bytes = std::fs::read(&saved).unwrap();
    h.state_mut().viewer.set_zoom_percent(100.0);
    composed(&mut h, time + 2.0);
    save_retained(&mut h, &saved);
    assert_eq!(
        std::fs::read(&saved).unwrap(),
        bytes,
        "view changes must not change Save samples"
    );
    assert_eq!(
        accepted.to_host(h.state().backend.as_ref()).unwrap().data,
        samples.data
    );
    // The real rotation caller publishes a device presentation source, but
    // keeps the unturned import for Scan: orientation must apply exactly once.
    super::reveal_control(&mut h, "action.rotate_cw");
    super::click(&mut h, "cw rotate");
    assert_eq!(h.state().input_rotation, 3);
    assert!(h.state().image.as_ref().unwrap().same_source(&source));
    let oriented = h.state().oriented_input.clone().unwrap();
    let crate::render::InputImage::Device(presentation) = &oriented else {
        panic!("GPU input viewer must retain device-rotated source samples");
    };
    let expected_source = original.rotated_quarter_turns(3);
    assert_eq!(
        (presentation.width(), presentation.height()),
        (original.height, original.width)
    );
    let presentation_samples = presentation.to_host(h.state().backend.as_ref()).unwrap();
    for (&actual, &expected) in presentation_samples.data.iter().zip(&expected_source.data) {
        assert_eq!(actual as f32, expected as f32);
    }
    scan_device_output(&mut h);
    let turned = h.state().output_retained.clone().unwrap();
    let turned_samples = turned.to_host(h.state().backend.as_ref()).unwrap();
    let expected_scan = samples.rotated_quarter_turns(3);
    assert_eq!(
        (turned_samples.width, turned_samples.height),
        (expected_scan.width, expected_scan.height)
    );
    for (&actual, &expected) in turned_samples.data.iter().zip(&expected_scan.data) {
        assert!(
            (actual - expected).abs() < 2e-6,
            "rotation must not be applied twice"
        );
    }
    let time = h.ctx.input(|input| input.time) + 1.0;
    assert!(composed(&mut h, time).pixels.is_some());

    super::reveal_control(&mut h, "action.rotate_ccw");
    super::click(&mut h, "ccw rotate");
    assert_eq!(h.state().input_rotation, 0);
    assert!(h.state().image.as_ref().unwrap().same_source(&source));
    assert!(
        h.state()
            .oriented_input
            .as_ref()
            .unwrap()
            .same_source(&source)
    );
    assert_eq!(source.host_image().unwrap().data, original.data);
    wait_device_output(&mut h);
    // A real active Fotufilm request must retain the same worker observation
    // boundary. The aggregate film_chain stage is the executor authority;
    // no grain-specific stage is synthesized by the GUI.
    {
        let app = h.state_mut();
        app.params.debug.deactivate_stochastic_effects = false;
        app.params.film_render.grain.active = true;
        app.params.film_render.grain.engine = spektrafilm_core::params::GrainEngine::Fotufilm;
        app.params.film_render.grain.fotufilm.amount = 1.65;
        app.params.random_seed = 17;
        app.mark_parameters_changed();
    }
    scan_device_output(&mut h);
    let report = assert_retained_gpu_report(
        h.state(),
        &directory.path().join("fotufilm-scan-report.json"),
        "scan",
        [original.width, original.height],
    );
    let grain = &report.configuration.render.as_ref().unwrap().grain;
    assert!(grain.active);
    assert_eq!(
        grain.engine,
        spektrafilm_core::telemetry::GrainEngine::Fotufilm
    );
    eframe::App::on_exit(h.state_mut(), None);
}
