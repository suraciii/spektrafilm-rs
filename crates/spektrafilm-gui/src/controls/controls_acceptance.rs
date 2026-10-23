use egui_kittest::{Harness, kittest::Queryable};
use spektrafilm_core::params::RuntimeParams;

fn scanner_params() -> RuntimeParams {
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.scanner.scan_output = "negative_to_positive".into();
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
        &crate::profiles::pick_data_dir(),
    );
}

fn camera(ui: &mut egui::Ui, params: &mut RuntimeParams) {
    super::show(
        ui,
        params,
        &mut serde_json::Value::Null,
        "Camera",
        super::ScanMedium::Negative,
        "",
        &mut Default::default(),
        "D65",
        &crate::profiles::pick_data_dir(),
    );
}

fn enlarger(ui: &mut egui::Ui, params: &mut RuntimeParams) {
    super::show(
        ui,
        params,
        &mut serde_json::Value::Null,
        "Enlarger",
        super::ScanMedium::Negative,
        "",
        &mut Default::default(),
        "D65",
        &crate::profiles::pick_data_dir(),
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
        assert_eq!(harness.state().scanner.lens_blur, after_event.lens_blur);
        assert_eq!(
            harness.state().scanner.unsharp_mask,
            after_event.unsharp_mask
        );
    }

    #[test]
    fn scanner_keyboard_enter_invalid_blur_and_tuple_channels() {
        let mut harness = Harness::new_ui_state(scanner, scanner_params());
        let input = harness.get_by_label("Lens blur (readout optics)");
        assert_eq!(input.role(), egui::accesskit::Role::TextInput);
        assert_eq!(
            harness
                .get_by_role_and_label(egui::accesskit::Role::ComboBox, "scan output")
                .role(),
            egui::accesskit::Role::ComboBox
        );
        let id = input.id();
        replace(&mut harness, "Lens blur (readout optics)", "0.25");
        harness.run();
        assert_eq!(harness.state().scanner.lens_blur, 0.25);
        harness
            .get_by_label("Lens blur (readout optics)")
            .key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().scanner.lens_blur, 0.25);
        assert!(
            !harness
                .get_by_label("Lens blur (readout optics)")
                .is_focused()
        );
        assert_eq!(harness.get_by_label("Lens blur (readout optics)").id(), id);
        // An out-of-range draft is rejected rather than clamped.
        replace(&mut harness, "Lens blur (readout optics)", "-0.5");
        harness.run();
        assert_eq!(harness.state().scanner.lens_blur, 0.25);
        assert_eq!(
            harness
                .get_by_label("Lens blur (readout optics)")
                .value()
                .as_deref(),
            Some("-0.5")
        );
        // A non-numeric draft is rejected too.
        replace(&mut harness, "Lens blur (readout optics)", "NaN");
        assert_eq!(harness.state().scanner.lens_blur, 0.25);
        harness
            .get_by_label("Unsharp mask (digital sharpening) sigma")
            .focus();
        harness.run();
        assert_eq!(
            harness
                .get_by_label("Lens blur (readout optics)")
                .value()
                .as_deref(),
            Some("0.25")
        );

        replace(
            &mut harness,
            "Unsharp mask (digital sharpening) sigma",
            "0.5",
        );
        harness
            .get_by_label("Unsharp mask (digital sharpening) sigma")
            .key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().scanner.unsharp_mask[0], 0.5);
        for label in [
            "Unsharp mask (digital sharpening) sigma",
            "Unsharp mask (digital sharpening) amount",
        ] {
            assert_eq!(
                harness.get_by_label(label).role(),
                egui::accesskit::Role::TextInput
            );
        }
    }

    #[test]
    fn scanner_wheel_steps_consumes_scroll_and_preserves_boundaries() {
        let mut harness = Harness::new_ui_state(scanner, scanner_params());
        wheel(&mut harness, "Lens blur (readout optics)", 120.0);
        assert!((harness.state().scanner.lens_blur - 0.05).abs() < 1e-6);
        wheel(&mut harness, "Lens blur (readout optics)", -120.0);
        assert!(harness.state().scanner.lens_blur.abs() < 1e-6);
        // Typing an out-of-range value is rejected, not silently clamped.
        replace(&mut harness, "Lens blur (readout optics)", "100");
        harness
            .get_by_label("Lens blur (readout optics)")
            .key_press(Key::Enter);
        harness.run();
        assert!(harness.state().scanner.lens_blur.abs() < 1e-6);
        // Typing an in-range value at the boundary is accepted.
        replace(&mut harness, "Lens blur (readout optics)", "5");
        harness
            .get_by_label("Lens blur (readout optics)")
            .key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().scanner.lens_blur, 5.0);
        // The wheel preserves the maximum boundary.
        wheel(&mut harness, "Lens blur (readout optics)", 120.0);
        assert_eq!(harness.state().scanner.lens_blur, 5.0);
        // A negative out-of-range value is rejected too.
        replace(&mut harness, "Lens blur (readout optics)", "-100");
        harness
            .get_by_label("Lens blur (readout optics)")
            .key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().scanner.lens_blur, 5.0);
        // Typing an in-range value at the minimum boundary is accepted.
        replace(&mut harness, "Lens blur (readout optics)", "0");
        harness
            .get_by_label("Lens blur (readout optics)")
            .key_press(Key::Enter);
        harness.run();
        assert_eq!(harness.state().scanner.lens_blur, 0.0);
        // The wheel preserves the minimum boundary.
        wheel(&mut harness, "Lens blur (readout optics)", -120.0);
        assert_eq!(harness.state().scanner.lens_blur, 0.0);
        wheel(
            &mut harness,
            "Unsharp mask (digital sharpening) amount",
            120.0,
        );
        assert!((harness.state().scanner.unsharp_mask[1] - 0.75).abs() < 1e-6);
        assert_eq!(harness.state().scanner.unsharp_mask[0], 0.7);
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
                "conversion style",
                "Lens blur (readout optics)",
                "Unsharp mask (digital sharpening) sigma",
                "Unsharp mask (digital sharpening) amount",
            ];
            let mut bounds = Vec::new();
            for label in labels {
                let node = if matches!(label, "scan output" | "conversion style") {
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

#[cfg(test)]
mod roles {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn a_parameter_row_boolean_is_a_switch() {
        let mut params = scanner_params();
        params.camera.auto_exposure = false;
        let mut harness = Harness::new_ui_state(camera, params);
        let node = harness.get_by_label("auto exposure");
        assert_eq!(node.role(), egui::accesskit::Role::Switch);
        assert_eq!(node.toggled(), Some(egui::accesskit::Toggled::False));
        harness.get_by_label("auto exposure").simulate_click();
        harness.run();
        assert_eq!(
            harness.get_by_label("auto exposure").toggled(),
            Some(egui::accesskit::Toggled::True)
        );
    }
}

#[cfg(test)]
mod row_layout {
    use super::*;
    use egui_kittest::kittest::Queryable;

    /// A parameter row presents its label before its value, on one line, and
    /// every row on a page aligns to one label column.
    #[test]
    fn a_row_presents_its_label_before_its_value_on_one_line() {
        let params = scanner_params();
        let harness = Harness::new_ui_state(camera, params);
        // The label sits left of its editor and shares its vertical band, so
        // the pair reads as one row rather than a stacked pair.
        let rows = ["film format mm", "exposure compensation ev"];
        let mut left_edges: Vec<f64> = Vec::new();
        let mut value_lefts: Vec<f64> = Vec::new();
        for label in rows {
            let node = harness.get_by_label(label);
            let rect = node.raw_bounds().expect("row editor has no bounds");
            let mut labels = node.labelled_by();
            let visible = labels.next().expect("row editor has no visible label");
            let label_rect = visible.raw_bounds().expect("visible label has no bounds");
            assert!(
                label_rect.x1 <= rect.x0,
                "{label}: label must sit before its value (label {:?}, value {:?})",
                label_rect,
                rect
            );
            let shared = label_rect.y0 < rect.y1 && rect.y0 < label_rect.y1;
            assert!(shared, "{label}: label and value must share one line");
            left_edges.push(label_rect.x0);
            value_lefts.push(rect.x0);
        }
        // One label column per page: the labels share a left edge and the values
        // share a left edge, so rows align rather than float independently.
        let label_left = left_edges[0];
        let value_left = value_lefts[0];
        for (i, (l, v)) in left_edges.iter().zip(value_lefts.iter()).enumerate() {
            assert!(
                (l - label_left).abs() < 1.0,
                "row {i} label column drifted: {l} vs {label_left}"
            );
            assert!(
                (v - value_left).abs() < 1.0,
                "row {i} value column drifted: {v} vs {value_left}"
            );
        }
    }

    /// The preflash exposure row carries the longest unit in the vocabulary, so
    /// its abbreviated display suffix is what keeps the row on one line inside
    /// the fixed-width panel. A unit that wraps the label breaks the single-line
    /// row rhythm the spec requires.
    #[test]
    fn the_longest_unit_row_stays_on_one_line() {
        let mut params = scanner_params();
        params.workflow.route = "input > film > print".into();
        let harness = Harness::new_ui_state(enlarger, params);
        let node = harness.get_by_label("preflash exposure");
        let rect = node.raw_bounds().expect("row editor has no bounds");
        let label_rect = node
            .labelled_by()
            .next()
            .expect("row editor has no visible label")
            .raw_bounds()
            .expect("visible label has no bounds");
        let shared = label_rect.y0 < rect.y1 && rect.y0 < label_rect.y1;
        assert!(
            shared,
            "preflash exposure: label and value must share one line \
             (label {label_rect:?}, value {rect:?})"
        );
    }
}
