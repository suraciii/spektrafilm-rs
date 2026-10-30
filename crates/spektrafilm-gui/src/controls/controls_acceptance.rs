use egui_kittest::{Harness, kittest::Queryable};
use spektrafilm_core::params::RuntimeParams;

fn scanner_params() -> RuntimeParams {
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.scanner.scan_output = "negative_to_positive".into();
    params
}

fn scanner(ui: &mut egui::Ui, params: &mut RuntimeParams) {
    scanner_changes(ui, params);
}

fn scanner_changes(ui: &mut egui::Ui, params: &mut RuntimeParams) -> super::ChangeFlags {
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
    )
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

fn preflash(ui: &mut egui::Ui, params: &mut RuntimeParams) {
    super::show(
        ui,
        params,
        &mut serde_json::Value::Null,
        "Preflash",
        super::ScanMedium::Negative,
        "",
        &mut Default::default(),
        "D65",
        &crate::profiles::pick_data_dir(),
    );
}

#[test]
fn camera_rows_follow_route_without_losing_values() {
    let mut params = scanner_params();
    params.workflow.route = "input".into();
    params.camera.exposure_compensation_ev = 0.75;
    params.camera.film_format_mm = 50.0;
    let mut harness = Harness::new_ui_state(camera, params);
    assert!(
        harness
            .get_by_label("Exposure compensation")
            .node()
            .is_disabled()
    );
    assert!(!harness.get_by_label("Frame Long Edge").node().is_disabled());
    harness.state_mut().workflow.route = "input > film > scan".into();
    harness.run();
    assert!(
        !harness
            .get_by_label("Exposure compensation")
            .node()
            .is_disabled()
    );
    assert_eq!(harness.state().camera.exposure_compensation_ev, 0.75);
    assert_eq!(harness.state().camera.film_format_mm, 50.0);
}

#[test]
fn glare_selector_follows_print_readout_not_print_creation() {
    let mut params = scanner_params();
    params.workflow.route = "input > film > print".into();
    params.print_render.glare.active = false;
    let mut harness = Harness::new_ui_state(
        |ui, params: &mut RuntimeParams| {
            super::show(
                ui,
                params,
                &mut serde_json::Value::Null,
                "Glare",
                super::ScanMedium::Negative,
                "",
                &mut Default::default(),
                "D65",
                &crate::profiles::pick_data_dir(),
            );
        },
        params,
    );
    assert!(
        harness
            .get_by_label("Enable print glare")
            .node()
            .is_disabled()
    );
    harness.state_mut().workflow.route = "print > scan".into();
    harness.run();
    assert!(
        !harness
            .get_by_label("Enable print glare")
            .node()
            .is_disabled()
    );
    assert!(!harness.state().print_render.glare.active);
}

#[test]
fn film_gamma_remains_editable_for_auto_print_reference_without_film_chemistry() {
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > convert-film > print > scan".into();
    params.enlarger.print_exposure_mode = spektrafilm_core::params::PrintExposureMode::Auto;
    params.settings.print_exposure_ev_tracking =
        spektrafilm_core::params::PrintExposureEvTracking::Never;
    params.film_render.density_curve_gamma = 1.25;
    let mut harness = Harness::new_ui_state(
        |ui, params: &mut RuntimeParams| {
            super::show(
                ui,
                params,
                &mut serde_json::Value::Null,
                "Film chemistry",
                super::ScanMedium::Negative,
                "",
                &mut Default::default(),
                "D65",
                &crate::profiles::pick_data_dir(),
            );
        },
        params,
    );
    assert!(
        harness
            .get_by_label("Enable film density curves")
            .node()
            .is_disabled()
    );
    assert!(
        !harness
            .get_by_label("density curve gamma")
            .node()
            .is_disabled()
    );

    harness.state_mut().enlarger.print_exposure_mode =
        spektrafilm_core::params::PrintExposureMode::Manual;
    harness.run();
    assert!(
        harness
            .get_by_label("density curve gamma")
            .node()
            .is_disabled()
    );
    assert_eq!(harness.state().film_render.density_curve_gamma, 1.25);

    harness.state_mut().enlarger.print_exposure_mode =
        spektrafilm_core::params::PrintExposureMode::Auto;
    harness.run();
    assert!(
        !harness
            .get_by_label("density curve gamma")
            .node()
            .is_disabled()
    );
    assert_eq!(harness.state().film_render.density_curve_gamma, 1.25);
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
        harness.run_steps(10);
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
        let rows = ["Frame Long Edge", "Exposure compensation"];
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
        let harness = Harness::new_ui_state(preflash, params);
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

#[cfg(test)]
mod operator_interactions {
    use super::*;
    use spektrafilm_core::params::ScanOperatorParams;

    fn harness(params: RuntimeParams) -> Harness<'static, RuntimeParams> {
        Harness::builder()
            .with_size(egui::vec2(720.0, 1400.0))
            .build_state(
                |ctx, params| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        if scanner_changes(ui, params).runtime_changed {
                            ctx.data_mut(|data| {
                                *data.get_temp_mut_or_default::<usize>(egui::Id::new(
                                    "operator_changes",
                                )) += 1;
                            });
                        }
                    });
                },
                params,
            )
    }

    fn edit(h: &mut Harness<'_, RuntimeParams>, label: &str, value: &str) {
        h.get_by_label(label).focus();
        h.run();
        h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label(label).type_text(value);
        h.run();
        h.get_by_label(label)
            .key_press(egui_kittest::kittest::Key::Enter);
        h.run();
    }

    fn click(h: &mut Harness<'_, RuntimeParams>, label: &str) {
        h.get_by_label(label).simulate_click();
        h.run();
    }

    #[test]
    fn operator_edit_restore_and_copy_apply_only_four_values() {
        let mut h = harness(scanner_params());
        for (label, value) in [
            ("Finished exposure", "1.25"),
            ("Warmth", "0.35"),
            ("Green–magenta tint", "-0.20"),
            ("Contrast", "1.40"),
        ] {
            edit(&mut h, label, value);
        }
        let copied = h.state().scanner.operator;
        assert_eq!(copied.exposure_ev, 1.25);
        assert_eq!(copied.warmth_ev, 0.35);
        assert_eq!(copied.tint_ev, -0.2);
        assert_eq!(copied.contrast, 1.4);
        edit(&mut h, "Finished exposure", "3.01");
        assert_eq!(h.state().scanner.operator, copied);
        click(&mut h, "Copy operator correction");
        click(&mut h, "Restore automatic");
        assert_eq!(h.state().scanner.operator, ScanOperatorParams::default());

        // A new frame/state has its own upstream settings. The clipboard lives
        // in the session context and must carry none of those settings.
        let mut target = scanner_params();
        target.camera.exposure_compensation_ev = -1.5;
        target.film_render.convert.exposure_compensation_ev = 0.75;
        target.scanner.lens_blur = 0.5;
        *h.state_mut() = target;
        h.run();
        click(&mut h, "Paste operator correction");
        assert_eq!(h.state().scanner.operator, copied);
        assert_eq!(h.state().camera.exposure_compensation_ev, -1.5);
        assert_eq!(h.state().film_render.convert.exposure_compensation_ev, 0.75);
        assert_eq!(h.state().scanner.lens_blur, 0.5);
    }

    #[test]
    fn operator_restore_and_paste_report_runtime_edits_without_session_history() {
        let copied = ScanOperatorParams {
            exposure_ev: 0.25,
            warmth_ev: 0.35,
            tint_ev: -0.2,
            contrast: 1.4,
        };
        let action = "Restore automatic";
        let expected = ScanOperatorParams::default();
        let mut params = scanner_params();
        params.scanner.operator = copied;
        params.camera.exposure_compensation_ev = -1.5;
        params.scanner.lens_blur = 0.5;
        let mut h = harness(params);
        click(&mut h, "Copy operator correction");
        assert_eq!(runtime_changes(&h), 0, "copy is not an adjustment");
        click(&mut h, action);
        assert_eq!(h.state().scanner.operator, expected, "{action}");
        assert_eq!(runtime_changes(&h), 1, "{action}");
        let edited = super::super::take_edited_paths(&h.ctx);
        for (path, before, after) in [
            (
                "scanner.operator.exposure_ev",
                copied.exposure_ev,
                expected.exposure_ev,
            ),
            (
                "scanner.operator.warmth_ev",
                copied.warmth_ev,
                expected.warmth_ev,
            ),
            ("scanner.operator.tint_ev", copied.tint_ev, expected.tint_ev),
            (
                "scanner.operator.contrast",
                copied.contrast,
                expected.contrast,
            ),
        ] {
            assert_eq!(
                edited.iter().any(|edited| edited == path),
                before != after,
                "{action}: {path}"
            );
        }
        click(&mut h, "Paste operator correction");
        assert_eq!(h.state().scanner.operator, copied);
        assert_eq!(runtime_changes(&h), 2, "paste is one runtime adjustment");
        click(&mut h, "Paste operator correction");
        assert_eq!(
            runtime_changes(&h),
            2,
            "unchanged paste is not an adjustment"
        );
        assert_eq!(h.state().camera.exposure_compensation_ev, -1.5);
        assert_eq!(h.state().scanner.lens_blur, 0.5);

        let fresh = harness(h.state().clone());
        assert_eq!(fresh.state().scanner.operator, copied);
        assert!(
            fresh
                .get_by_label("Paste operator correction")
                .node()
                .is_disabled()
        );
        assert!(fresh.query_by_label("Undo operator correction").is_none());
        assert!(fresh.query_by_label("Redo operator correction").is_none());
    }

    fn runtime_changes(h: &Harness<'_, RuntimeParams>) -> usize {
        h.ctx
            .data(|data| data.get_temp::<usize>(egui::Id::new("operator_changes")))
            .unwrap_or_default()
    }

    #[test]
    fn inactive_operator_values_remain_visible_disabled_and_return_unchanged() {
        let mut params = scanner_params();
        params.scanner.operator.exposure_ev = 0.75;
        let mut h = harness(params);
        for (route, output, style) in [
            ("input", "direct_scan", "standard"),
            ("input > film > scan", "direct_scan", "standard"),
            ("input > film > print > scan", "direct_scan", "standard"),
            ("input > film > scan", "negative_to_positive", "custom"),
        ] {
            h.state_mut().workflow.route = route.into();
            h.state_mut().scanner.scan_output = output.into();
            h.state_mut().scanner.conversion_style = style.into();
            h.run();
            assert!(h.get_by_label("Finished exposure").node().is_disabled());
            assert!(
                h.get_by_label("Paste operator correction")
                    .node()
                    .is_disabled()
            );
            assert_eq!(h.state().scanner.operator.exposure_ev, 0.75);
        }
        h.state_mut().scanner.conversion_style = "standard".into();
        h.run();
        assert!(!h.get_by_label("Finished exposure").node().is_disabled());
        assert_eq!(h.state().scanner.operator.exposure_ev, 0.75);
    }

    #[test]
    fn held_automatic_comparison_emits_transitions_without_editing_operator() {
        let mut params = scanner_params();
        params.scanner.operator.exposure_ev = 1.0;
        let original = params.scanner.operator;
        let mut h = Harness::builder()
            .with_size(egui::vec2(720.0, 1400.0))
            .build_state(
                |ctx, state: &mut (RuntimeParams, Vec<bool>)| {
                    if super::super::release_operator_compare(ctx) {
                        state.1.push(false);
                    }
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let changes = super::super::show(
                            ui,
                            &mut state.0,
                            &mut serde_json::Value::Null,
                            "Scanner",
                            super::super::ScanMedium::Negative,
                            "",
                            &mut Default::default(),
                            "D55",
                            &crate::profiles::pick_data_dir(),
                        );
                        assert!(
                            !changes.runtime_changed,
                            "comparison must not enter adjustment history"
                        );
                        if let Some(comparing) = changes.operator_compare {
                            state.1.push(comparing);
                        }
                    });
                },
                (params, Vec::new()),
            );
        h.get_by_label("Hold automatic comparison").focus();
        h.run();
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.run();
            assert_eq!(h.state().0.scanner.operator, original);
        }
        assert_eq!(h.state().1, [true, false]);
    }
}
