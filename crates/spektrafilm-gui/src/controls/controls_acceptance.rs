use egui_kittest::{Harness, kittest::Queryable};
use spektrafilm_core::params::RuntimeParams;

fn scanner_params() -> RuntimeParams {
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.scanner.scan_output = "negative_to_positive".into();
    params.scanner.conversion_brightness_ev = 0.0;
    params
}

fn scanner(ui: &mut egui::Ui, params: &mut RuntimeParams) {
    super::show(
        ui,
        params,
        &mut serde_json::Value::Null,
        "Scanner",
        super::ScanMedium::Negative,
        "",
        &mut Default::default(),
        "D55",
    );
}

#[cfg(test)]
mod headless {
    use super::*;
    use egui_kittest::kittest::Key;

    fn replace(harness: &mut Harness<'_, RuntimeParams>, label: &str, text: &str) {
        harness.get_by_label(label).focus();
        harness.run();
        harness.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label(label).type_text(text);
        harness.run();
    }

    fn wheel(harness: &mut Harness<'_, RuntimeParams>, label: &str, delta: f32) {
        harness.get_by_label(label).hover();
        harness.run();
        harness.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta),
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
        assert_eq!(harness.ctx.input(|input| input.raw_scroll_delta.y), 0.0);
        assert_eq!(harness.ctx.input(|input| input.smooth_scroll_delta.y), 0.0);
        let after_event = harness.state().scanner.clone();
        harness.run();
        assert_eq!(
            harness.state().scanner.conversion_brightness_ev,
            after_event.conversion_brightness_ev
        );
        assert_eq!(
            harness.state().scanner.conversion_balance_ev,
            after_event.conversion_balance_ev
        );
    }

    #[test]
    fn scanner_keyboard_enter_invalid_blur_and_tuple_channels() {
        let mut harness = Harness::new_ui_state(scanner, scanner_params());
        let input = harness.get_by_label("brightness");
        assert_eq!(input.role(), egui::accesskit::Role::TextInput);
        assert_eq!(
            harness
                .get_by_role_and_label(egui::accesskit::Role::ComboBox, "scan output")
                .role(),
            egui::accesskit::Role::ComboBox
        );
        let id = input.id();
        replace(&mut harness, "brightness", "-");
        assert_eq!(harness.state().scanner.conversion_brightness_ev, 0.0);
        assert_eq!(
            harness.get_by_label("brightness").value().as_deref(),
            Some("-")
        );
        harness.get_by_label("brightness").type_text("0.25");
        harness.run();
        harness.get_by_label("brightness").key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().scanner.conversion_brightness_ev, -0.25);
        assert!(!harness.get_by_label("brightness").is_focused());
        assert_eq!(harness.get_by_label("brightness").id(), id);

        replace(&mut harness, "brightness", "NaN");
        assert_eq!(harness.state().scanner.conversion_brightness_ev, -0.25);
        assert_eq!(
            harness.get_by_label("brightness").value().as_deref(),
            Some("NaN")
        );
        harness.get_by_label("Lens blur (readout optics)").focus();
        harness.run();
        assert_eq!(
            harness.get_by_label("brightness").value().as_deref(),
            Some("-0.250")
        );

        replace(&mut harness, "balance 1", "0.5");
        harness.get_by_label("balance 1").key_press(Key::Enter);
        harness.run();
        assert_eq!(
            harness.state().scanner.conversion_balance_ev,
            [0.0, 0.5, 0.0]
        );
        for label in ["balance 0", "balance 1", "balance 2"] {
            assert_eq!(
                harness.get_by_label(label).role(),
                egui::accesskit::Role::TextInput
            );
        }
    }

    #[test]
    fn scanner_wheel_steps_consumes_scroll_and_preserves_boundaries() {
        let mut harness = Harness::new_ui_state(scanner, scanner_params());
        wheel(&mut harness, "brightness", 120.0);
        assert!((harness.state().scanner.conversion_brightness_ev - 0.05).abs() < 1e-6);
        wheel(&mut harness, "brightness", -120.0);
        assert!(harness.state().scanner.conversion_brightness_ev.abs() < 1e-6);
        // Typing an out-of-range value is rejected, not silently clamped.
        replace(&mut harness, "brightness", "100");
        harness.get_by_label("brightness").key_press(Key::Enter);
        harness.run();
        assert!(harness.state().scanner.conversion_brightness_ev.abs() < 1e-6);
        // Typing an in-range value at the boundary is accepted.
        replace(&mut harness, "brightness", "16");
        harness.get_by_label("brightness").key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().scanner.conversion_brightness_ev, 16.0);
        // The wheel preserves the maximum boundary.
        wheel(&mut harness, "brightness", 120.0);
        assert_eq!(harness.state().scanner.conversion_brightness_ev, 16.0);
        // A negative out-of-range value is rejected too.
        replace(&mut harness, "brightness", "-100");
        harness.get_by_label("brightness").key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().scanner.conversion_brightness_ev, 16.0);
        // Typing an in-range value at the minimum boundary is accepted.
        replace(&mut harness, "brightness", "-16");
        harness.get_by_label("brightness").key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().scanner.conversion_brightness_ev, -16.0);
        // The wheel preserves the minimum boundary.
        wheel(&mut harness, "brightness", -120.0);
        assert_eq!(harness.state().scanner.conversion_brightness_ev, -16.0);
        wheel(&mut harness, "balance 2", 120.0);
        assert!((harness.state().scanner.conversion_balance_ev[2] - 0.05).abs() < 1e-6);
        assert_eq!(harness.state().scanner.conversion_balance_ev[0], 0.0);
        assert_eq!(harness.state().scanner.conversion_balance_ev[1], 0.0);
    }
}

#[cfg(feature = "gui-snapshots")]
mod layout_snapshots {
    use super::*;

    #[test]
    fn scanner_critical_labels_and_fields_render_without_clipping_or_overlap() {
        for (size, dpi, name) in [
            (egui::vec2(1024.0, 768.0), 1.0, "scanner-small"),
            (egui::vec2(1460.0, 980.0), 2.0, "scanner-large-2x"),
        ] {
            let mut harness = Harness::builder()
                .with_size(size)
                .with_pixels_per_point(dpi)
                .wgpu()
                .build_state(
                    |ctx, params| {
                        egui::SidePanel::right("controls")
                            .exact_width(420.0)
                            .show(ctx, |ui| scanner(ui, params));
                        egui::CentralPanel::default().show(ctx, |ui| {
                            ui.label("Preview");
                        });
                    },
                    scanner_params(),
                );
            harness.run();
            let image = harness
                .render()
                .expect("offscreen scanner rendering failed");
            assert_eq!(image.width(), (size.x * dpi) as u32);
            assert_eq!(image.height(), (size.y * dpi) as u32);
            let labels = [
                "scan output",
                "brightness",
                "balance 0",
                "balance 1",
                "balance 2",
                "contrast",
                "maximum density",
                "Lens blur (readout optics)",
                "film-scan black point 0",
                "film-scan black point 1",
                "film-scan black point 2",
                "film-scan white point 0",
                "film-scan white point 1",
                "film-scan white point 2",
                "Unsharp mask (digital sharpening) 0",
                "Unsharp mask (digital sharpening) 1",
            ];
            let mut bounds = Vec::new();
            for label in labels {
                let node = if label == "scan output" {
                    harness.get_by_role_and_label(egui::accesskit::Role::ComboBox, label)
                } else {
                    harness.get_by_label(label)
                };
                let rect = node.raw_bounds().expect("critical control has no bounds");
                let panel_left = f64::from(size.x - 420.0);
                assert!(
                    rect.x0 >= panel_left
                        && rect.y0 >= 0.0
                        && rect.x1 <= f64::from(size.x)
                        && rect.y1 <= f64::from(size.y),
                    "{name}: {label} is clipped: {rect:?}"
                );
                assert!(
                    rect.width() >= 40.0 && rect.height() >= 12.0,
                    "{name}: {label} is unusably small"
                );
                for (other, other_rect) in &bounds {
                    let other_rect: &egui::accesskit::Rect = other_rect;
                    assert!(
                        rect.x1 <= other_rect.x0
                            || other_rect.x1 <= rect.x0
                            || rect.y1 <= other_rect.y0
                            || other_rect.y1 <= rect.y0,
                        "{name}: {label} overlaps {other}"
                    );
                }
                // Labelled controls must keep the entire visible label inside the sidebar,
                // not merely keep their input rectangles on screen.
                for visible_label in node.labelled_by() {
                    let label_rect = visible_label
                        .raw_bounds()
                        .expect("visible label has no bounds");
                    assert!(
                        label_rect.x0 >= panel_left
                            && label_rect.x1 <= f64::from(size.x)
                            && label_rect.y1 <= f64::from(size.y),
                        "{name}: visible label for {label} is clipped"
                    );
                    assert!(
                        label_rect.x1 <= rect.x0
                            || rect.x1 <= label_rect.x0
                            || label_rect.y1 <= rect.y0
                            || rect.y1 <= label_rect.y0,
                        "{name}: visible label overlaps {label}"
                    );
                }
                let x0 = (rect.x0 * f64::from(dpi)).floor() as u32;
                let y0 = (rect.y0 * f64::from(dpi)).floor() as u32;
                let x1 = (rect.x1 * f64::from(dpi)).ceil() as u32;
                let y1 = (rect.y1 * f64::from(dpi)).ceil() as u32;
                let first = image.get_pixel(x0, y0);
                assert!(
                    (y0..y1).any(|y| (x0..x1).any(|x| image.get_pixel(x, y) != first)),
                    "{name}: {label} rendered blank"
                );
                bounds.push((label, rect));
            }
            crate::headless::save_snapshot(&image, &format!("{name}.png"));
        }
    }
}
