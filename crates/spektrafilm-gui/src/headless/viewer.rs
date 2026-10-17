//! Consumer-visible viewer acceptance through the real App and display worker.
use super::*;
use crate::display::{Interpolation, ViewLayer, ViewerObservation};

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.ctx.style_mut(|style| style.animation_time = 0.0);
    // Settle egui collapsing-panel geometry before queuing pointer actions.
    // This disables only widget animation; Viewer development still uses
    // RawInput.time and waits for every real composition response.
    h.step();
    if matches!(label, "Input" | "Output" | "Paper back") {
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
        assert!(
            h.query_by_label(label).is_some(),
            "viewer control is unavailable: {label}"
        );
        super::click(h, label);
    }
}
fn frame(h: &mut Harness<'_, App>, time: f64) {
    h.input_mut().time = Some(time);
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

fn develop(h: &mut Harness<'_, App>, button: &str, time: f64) {
    let before = h.state().output_image.clone();
    h.get_by_label(button).simulate_click();
    let deadline = Instant::now() + WAIT_DEADLINE;
    loop {
        frame(h, time);
        if !h.state().dirty
            && h.state().render_job.is_none()
            && h.state()
                .output_image
                .as_ref()
                .is_some_and(|out| before.as_ref().is_none_or(|old| !Arc::ptr_eq(old, out)))
        {
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
    let mut previous = initial.transition.expect("explicit render animates");
    let initial_pixels = initial.pixels.unwrap().pixels;
    for _ in 0..51 {
        time += 0.04;
        let observation = composed(h, time);
        match observation.transition {
            Some(current) => {
                assert_eq!(current.1, previous.1);
                assert_eq!(current.0, previous.0 + 1);
                if (current.1 && current.0 == 25) || (!current.1 && current.0 == 5) {
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
    click(h, "CONFIG");
    click(h, "Display");
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
fn layers_and_interpolation_change_output_but_preserve_input_pixels() {
    let mut s = session();
    let h = &mut s.harness;
    assert!(h.state().viewer.output.is_none());
    let layer = h.state().viewer.settings.layer;
    click(h, "Output");
    assert_eq!(
        h.state().viewer.settings.layer,
        layer,
        "unavailable output is disabled"
    );
    click(h, "Input");
    let input = composed(h, 10.0);
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::Input);
    assert_eq!(input.interpolation, Interpolation::Nearest);
    click(h, "Paper back");
    assert_eq!(h.state().viewer.settings.layer, ViewLayer::PaperBack);
    assert_eq!(composed(h, 10.0).interpolation, Interpolation::Spline36);
    develop(h, "PREVIEW", 10.0);
    settle_animation(h);
    display_controls(h);
    click(h, "400%");
    choose(h, "output interpolation", "nearest");
    click(h, "Output");
    let nearest = composed(h, 100.0);
    assert_eq!(nearest.interpolation, Interpolation::Nearest);
    choose(h, "output interpolation", "spline36");
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
    click(h, "Input");
    let baseline = composed(h, 100.0).pixels.unwrap();
    choose(h, "output interpolation", "nearest");
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
            .with_size(egui::vec2(1500.0, 1400.0))
            .with_pixels_per_point(dpi)
            .build_eframe(move |_| app(state_path, image_path, browser_path));
        let h = &mut harness;
        click(h, "Input");
        let mut time = 20.0;
        for (label, percent) in [("100%", 100.0), ("200%", 200.0), ("400%", 400.0)] {
            click(h, label);
            time += 1.0;
            let observation = composed(h, time);
            assert_eq!(h.state().viewer.zoom_percent, Some(percent));
            let source = h.state().image.as_ref().unwrap();
            let ratio =
                observation.image_rect.width() * h.ctx.pixels_per_point() / source.width as f32;
            assert!(
                (ratio - percent / 100.0).abs() < 1e-5,
                "device pixel mapping: {ratio}"
            );
        }
        assert_eq!(h.ctx.pixels_per_point(), dpi);
        time += 1.0;
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
        let source = h.state().image.as_ref().unwrap();
        assert_eq!(inspection.layer, ViewLayer::Input);
        assert_eq!((inspection.x, inspection.y), (49, 50));
        for (actual, expected) in inspection.rgb.into_iter().zip(source.get(49, 50)) {
            assert!((actual - expected as f64).abs() <= 1e-6);
        }
        eframe::App::on_exit(h.state_mut(), None);
    }
}

#[test]
fn gray_canvas_and_white_padding_change_composed_pixels_and_restore_baseline() {
    let mut s = session();
    let h = &mut s.harness;
    click(h, "Input");
    display_controls(h);
    let baseline = composed(h, 30.0);
    assert_eq!(
        baseline.pixels.as_ref().unwrap().pixels[0],
        egui::Color32::from_gray(118)
    );
    click(h, "gray 18% canvas");
    let black = composed(h, 30.0);
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
    assert_eq!(
        baseline.pixels.as_ref().unwrap().pixels,
        composed(h, 30.0).pixels.unwrap().pixels
    );
    edit(h, "white padding", "0.2");
    let padded = composed(h, 30.0);
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
    assert_eq!(
        baseline.pixels.unwrap().pixels,
        composed(h, 30.0).pixels.unwrap().pixels
    );
}

#[test]
fn explicit_preview_and_scan_animate_but_automatic_parameter_update_does_not() {
    let mut s = session();
    let h = &mut s.harness;
    // This test owns the reveal animation's coverage, so it enables the
    // animation that `session_fixture` disables before asserting its frames.
    h.state_mut().viewer.settings.reveal = true;
    develop(h, "PREVIEW", 40.0);
    let reveal = composed(h, 40.0).transition.unwrap();
    assert_eq!(reveal, (0, true));
    let time = finish_animation(h, 40.0);
    edit(h, "exposure compensation ev", "0.75");
    develop(h, "PREVIEW", time);
    assert_eq!(composed(h, time).transition, Some((0, false)));
    let time = finish_animation(h, time);
    click(h, "Paper back");
    develop(h, "SCAN", time + 1.0);
    assert_eq!(composed(h, time + 1.0).transition, Some((0, true)));
    let time = finish_animation(h, time + 1.0);
    click(h, "auto preview");
    let before = h.state().output_image.clone().unwrap();
    edit(h, "exposure compensation ev", "0.5");
    let deadline = Instant::now() + WAIT_DEADLINE;
    loop {
        frame(h, time + 10.0);
        if h.state().render_job.is_none()
            && !h.state().dirty
            && !Arc::ptr_eq(h.state().output_image.as_ref().unwrap(), &before)
        {
            break;
        }
        assert!(Instant::now() < deadline, "automatic preview timed out");
        wait_tick();
    }
    assert_eq!(composed(h, time + 10.0).transition, None);
}

#[test]
fn viewer_settings_do_not_change_saved_or_exported_pixels() {
    let mut s = session();
    let h = &mut s.harness;
    develop(h, "PREVIEW", 50.0);
    settle_animation(h);
    let mut options = h.state().export_options.clone();
    options.format = export::ExportFormat::Exr;
    options.depth = BitDepth::ThirtyTwo;
    options.saving_cctf_encoding = false;
    let ctx = h.ctx.clone();
    let save_before = s.directory.path().join("save-before.exr");
    let export_before = s.directory.path().join("export-before.exr");
    h.state_mut().save_to_path(&save_before);
    h.state_mut()
        .start_export_to_path(&ctx, options.clone(), &export_before);
    wait_until(h, |a| a.export_job.is_none());
    display_controls(h);
    click(h, "gray 18% canvas");
    click(h, "use display transform");
    edit(h, "white padding", "0.2");
    choose(h, "output interpolation", "nearest");
    click(h, "Input");
    click(h, "400%");
    let save_after = s.directory.path().join("save-after.exr");
    let export_after = s.directory.path().join("export-after.exr");
    h.state_mut().save_to_path(&save_after);
    h.state_mut()
        .start_export_to_path(&ctx, options, &export_after);
    wait_until(h, |a| a.export_job.is_none());
    for (before, after) in [(save_before, save_after), (export_before, export_after)] {
        let before = image_io::load(&before).unwrap().image;
        let after = image_io::load(&after).unwrap().image;
        assert_eq!((before.width, before.height), (after.width, after.height));
        for (a, b) in before.data.iter().zip(&after.data) {
            assert!((*a as f64 - *b as f64).abs() <= 1e-6);
        }
    }
}
