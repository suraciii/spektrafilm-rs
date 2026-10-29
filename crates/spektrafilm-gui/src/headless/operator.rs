use super::*;

fn operator_controls(h: &mut Harness<'_, App>) {
    search_controls(h, "Operator correction");
}

fn standard_operator(h: &mut Harness<'_, App>) {
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input > film > scan");
    assert_eq!(h.state().film_medium, controls::ScanMedium::Negative);
    reveal_control(h, "scanner.scan_output");
    choose(h, "scan output", "Negative to positive");
    assert_eq!(h.state().params.scanner.conversion_style, "standard");
    // The full-frame preview contract belongs to the experimental setup: only
    // the setup needs the complete frame, so opt in where a test asserts it.
    h.state_mut().params.scanner.experimental_setup = true;
    h.run_steps(3);
    operator_controls(h);
}

fn operator_edit(h: &mut Harness<'_, App>, label: &str, value: &str) {
    edit(h, label, value);
    h.get_by_label("Search controls").focus();
    h.run_steps(3);
}

#[test]
fn standard_operator_preview_matches_scan_and_hold_compare_preserves_saved_correction() {
    let mut session = session();
    let h = &mut session.harness;
    standard_operator(h);
    h.state_mut().viewer.settings.crossfade = false;

    let automatic_preview = preview(h);
    assert_eq!(h.state().params.settings.preview_max_size, 128);
    assert_eq!(
        (automatic_preview.width, automatic_preview.height),
        (160, 80)
    );
    let automatic_scan = scan(h);
    assert_eq!(automatic_preview.data, automatic_scan.data);
    settle_animation(h);
    let automatic_display = h.state().viewer.output.as_ref().unwrap().clone();
    let automatic_space = h.state().output_color_space.clone();
    let automatic_encoding = h.state().output_cctf_encoding;
    click(h, "100%");
    settle_animation(h);
    assert_eq!(h.state().viewer.zoom_percent(), Some(100.0));
    let automatic_composed = h.state().viewer.observation().pixels.unwrap();

    operator_edit(h, "Finished exposure", "1.0");
    assert_eq!(h.state().params.scanner.operator.exposure_ev, 1.0);
    let corrected_preview = preview(h);
    let corrected = scan(h);
    assert_eq!(corrected_preview.data, corrected.data);
    assert_ne!(automatic_scan.data, corrected.data);
    assert!(mean(&corrected) > mean(&automatic_scan));
    settle_animation(h);
    let corrected_display = h.state().viewer.output.as_ref().unwrap().clone();
    assert_ne!(automatic_display.rgb, corrected_display.rgb);
    let corrected_composed = h.state().viewer.observation().pixels.unwrap();
    assert_ne!(automatic_composed.pixels, corrected_composed.pixels);
    let corrected_source = h.state().source_render.clone().unwrap();
    let corrected_space = h.state().output_color_space.clone();
    let corrected_encoding = h.state().output_cctf_encoding;
    let save_before = session.directory.path().join("operator-corrected.exr");
    script(h, chooser::ChooserResult::Scripted(save_before.clone()));
    click(h, "SAVE");
    wait_until(h, |app| app.pending_saves.is_empty());
    assert!(
        h.state().status.to_string().starts_with("Saved"),
        "{}",
        h.state().status
    );

    scroll_controls_into_view(h, "Hold automatic comparison");
    h.get_by_label("Hold automatic comparison").focus();
    h.step();
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    assert!(h.state().operator_compare);
    wait_until(h, |app| {
        app.operator_compare
            && !app.dirty
            && app.render_job.is_none()
            && app
                .status
                .to_string()
                .starts_with("Comparing automatic baseline")
    });
    settle_animation(h);
    let compared_display = h.state().viewer.output.as_ref().unwrap();
    assert_eq!(compared_display.size, automatic_display.size);
    assert_eq!(compared_display.rgb, automatic_display.rgb);
    let compared_composed = h.state().viewer.observation().pixels.unwrap();
    assert_eq!(compared_composed.size, automatic_composed.size);
    assert_eq!(compared_composed.pixels, automatic_composed.pixels);
    assert_eq!(h.state().viewer.zoom_percent(), Some(100.0));
    let comparison = h.state().operator_comparison.as_ref().unwrap();
    assert_eq!(
        comparison.image.host_image().unwrap().data,
        automatic_scan.data
    );
    assert_eq!(comparison.color_space, automatic_space);
    assert_eq!(comparison.encoded, automatic_encoding);
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &corrected
    ));
    assert_eq!(h.state().source_render.as_ref(), Some(&corrected_source));
    assert_eq!(h.state().output_color_space, corrected_space);
    assert_eq!(h.state().output_cctf_encoding, corrected_encoding);
    assert_eq!(h.state().params.scanner.operator.exposure_ev, 1.0);
    assert_eq!(
        h.state()
            .current_state()
            .unwrap()
            .runtime_params()
            .unwrap()
            .scanner
            .operator
            .exposure_ev,
        1.0,
    );

    let save_while_held = session
        .directory
        .path()
        .join("operator-while-comparing.exr");
    save_retained(h, &save_while_held);
    assert!(
        h.state().status.to_string().starts_with("Saved"),
        "{}",
        h.state().status
    );
    let before = image_io::load(&save_before).unwrap().image;
    let while_held = image_io::load(&save_while_held).unwrap().image;
    assert_eq!(
        (before.width, before.height),
        (corrected.width, corrected.height)
    );
    assert_eq!(
        (while_held.width, while_held.height),
        (before.width, before.height)
    );
    assert_eq!(
        before.data, while_held.data,
        "Save must retain corrected pixels during comparison"
    );
    assert_eq!(h.state().source_render.as_ref(), Some(&corrected_source));

    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    assert!(!h.state().operator_compare);
    wait_until(h, |app| {
        !app.dirty
            && app.render_job.is_none()
            && app
                .output_image
                .as_ref()
                .is_some_and(|image| !Arc::ptr_eq(image, &corrected))
    });
    assert_eq!(
        h.state().output_image.as_ref().unwrap().data,
        corrected.data
    );
    settle_animation(h);
    let restored_display = h.state().viewer.output.as_ref().unwrap();
    assert_eq!(restored_display.size, corrected_display.size);
    assert_eq!(restored_display.rgb, corrected_display.rgb);
    let restored_composed = h.state().viewer.observation().pixels.unwrap();
    assert_eq!(restored_composed.pixels, corrected_composed.pixels);
    assert_eq!(h.state().viewer.zoom_percent(), Some(100.0));
    assert!(h.state().operator_comparison.is_none());
    // Holding/releasing a transient baseline does not add history entries.
    click(h, "Undo");
    assert_eq!(h.state().params.scanner.operator.exposure_ev, 0.0);
    click(h, "Redo");
    assert_eq!(h.state().params.scanner.operator.exposure_ev, 1.0);
}

#[test]
fn operator_edits_copy_paste_and_restore_share_global_adjustment_history() {
    use spektrafilm_core::params::ScanOperatorParams;

    let mut s = session();
    let h = &mut s.harness;
    standard_operator(h);
    let baseline = h.state().params.clone();
    let mut states = vec![baseline.scanner.operator];
    for (label, value) in [
        ("Finished exposure", "1.25"),
        ("Warmth", "0.35"),
        ("Green–magenta tint", "-0.20"),
        ("Contrast", "1.40"),
    ] {
        operator_edit(h, label, value);
        states.push(h.state().params.scanner.operator);
    }
    let copied = ScanOperatorParams {
        exposure_ev: 1.25,
        warmth_ev: 0.35,
        tint_ev: -0.2,
        contrast: 1.4,
    };
    assert_eq!(h.state().params.scanner.operator, copied);
    operator_edit(h, "Finished exposure", "3.01");
    assert_eq!(h.state().params.scanner.operator, copied);
    click(h, "Copy operator correction");
    // Invalid input and session-only Copy must not insert an adjustment.
    for expected in states[..states.len() - 1].iter().rev() {
        click(h, "Undo");
        assert_eq!(&h.state().params.scanner.operator, expected);
    }
    assert_eq!(h.state().params, baseline);
    for expected in &states[1..] {
        click(h, "Redo");
        assert_eq!(&h.state().params.scanner.operator, expected);
    }

    click(h, "Restore automatic");
    assert_eq!(
        h.state().params.scanner.operator,
        ScanOperatorParams::default()
    );
    click(h, "Undo");
    assert_eq!(h.state().params.scanner.operator, copied);
    click(h, "Redo");
    assert_eq!(
        h.state().params.scanner.operator,
        ScanOperatorParams::default()
    );

    // An unrelated control is ordered in the same history, while the clipboard
    // carries just the four corrections, not readout optics or camera settings.
    reveal_control(h, "scanner.lens_blur");
    operator_edit(h, "Lens blur (readout optics)", "0.5");
    let before_paste = h.state().params.clone();
    operator_controls(h);
    click(h, "Paste operator correction");
    let mut expected = before_paste.clone();
    expected.scanner.operator = copied;
    assert_eq!(h.state().params, expected);
    assert_eq!(
        h.state().current_state().unwrap().runtime_params().unwrap(),
        expected
    );
    click(h, "Undo");
    assert_eq!(h.state().params, before_paste);
    click(h, "Undo");
    assert_eq!(
        h.state().params.scanner.lens_blur,
        baseline.scanner.lens_blur
    );
    assert_eq!(
        h.state().params.scanner.operator,
        ScanOperatorParams::default()
    );
    click(h, "Redo");
    assert_eq!(h.state().params, before_paste);
    click(h, "Redo");
    assert_eq!(h.state().params, expected);
}

#[test]
fn operator_restore_paste_and_text_edits_replace_global_redo_branches() {
    use spektrafilm_core::params::ScanOperatorParams;

    let mut s = session();
    let h = &mut s.harness;
    standard_operator(h);
    for (label, value) in [
        ("Finished exposure", "0.25"),
        ("Warmth", "0.35"),
        ("Green–magenta tint", "-0.20"),
        ("Contrast", "1.40"),
    ] {
        operator_edit(h, label, value);
    }
    let copied = h.state().params.scanner.operator;
    click(h, "Copy operator correction");
    operator_edit(h, "Finished exposure", "1.5");
    let first = h.state().params.clone();
    for (action, operator) in [
        ("Restore automatic", ScanOperatorParams::default()),
        ("Paste operator correction", copied),
    ] {
        operator_edit(h, "Finished exposure", "2.0");
        click(h, "Undo");
        assert_eq!(h.state().params, first, "before {action}");
        assert!(h.state().can_redo(), "before {action}");
        click(h, action);
        let mut expected = first.clone();
        expected.scanner.operator = operator;
        assert_eq!(h.state().params, expected, "after {action}");
        assert!(!h.state().can_redo(), "after {action}");
        click(h, "Undo");
        assert_eq!(h.state().params, first, "undo {action}");
        click(h, "Redo");
        assert_eq!(h.state().params, expected, "redo {action}");
        click(h, "Undo");
        assert_eq!(h.state().params, first);
    }
    operator_edit(h, "Finished exposure", "0.9");
    assert_eq!(h.state().params.scanner.operator.exposure_ev, 0.9);
    assert!(!h.state().can_redo());
    click(h, "Undo");
    assert_eq!(h.state().params, first);
}

#[test]
fn inactive_operator_values_remain_stored_while_rendering_neutrally() {
    let mut s = session();
    let h = &mut s.harness;
    standard_operator(h);
    operator_edit(h, "Finished exposure", "1.0");
    operator_edit(h, "Warmth", "0.25");
    let stored = h.state().params.scanner.operator;
    let corrected = preview(h);

    reveal_control(h, "scanner.scan_output");
    choose(h, "scan output", "Preserve medium polarity");
    operator_controls(h);
    assert!(h.get_by_label("Finished exposure").node().is_disabled());
    assert!(h.get_by_label("Restore automatic").node().is_disabled());
    assert_eq!(h.state().params.scanner.operator, stored);
    let inactive = preview(h);
    // Undoing the route choice reactivates precisely the retained correction.
    click(h, "Undo");
    assert_eq!(h.state().params.scanner.operator, stored);
    let reactivated = preview(h);
    assert_eq!(reactivated.data, corrected.data);
    click(h, "Restore automatic");
    reveal_control(h, "scanner.scan_output");
    choose(h, "scan output", "Preserve medium polarity");
    let neutral = preview(h);
    assert_eq!(inactive.data, neutral.data);
}

#[test]
fn neighboring_module_reset_retains_inactive_operator_and_uses_global_history() {
    let mut s = session();
    let h = &mut s.harness;
    standard_operator(h);
    for (label, value) in [
        ("Finished exposure", "1.0"),
        ("Warmth", "0.25"),
        ("Green–magenta tint", "-0.20"),
        ("Contrast", "1.40"),
    ] {
        operator_edit(h, label, value);
    }
    let stored = h.state().params.scanner.operator;
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input");
    assert_eq!(h.state().params.workflow.route, "input");
    assert_eq!(h.state().params.scanner.operator, stored);

    reveal_control(h, "output_mix.amount");
    operator_edit(h, "Simulation blend", "0.25");
    assert_eq!(h.state().params.output_mix.amount, 0.25);
    assert_eq!(h.state().params.scanner.operator, stored);
    let edited = h.state().params.clone();
    let factory = state::GuiState::factory().runtime_params().unwrap();
    let mut expected = edited.clone();
    expected.output_mix.amount = factory.output_mix.amount;
    expected.explicit_section_paths.remove("output_mix.amount");
    click(h, "reset Output");
    assert_eq!(h.state().params, expected);
    assert_eq!(h.state().params.scanner.operator, stored);
    click(h, "Undo");
    assert_eq!(h.state().params, edited);
    assert_eq!(h.state().params.scanner.operator, stored);
    click(h, "Redo");
    assert_eq!(h.state().params, expected);
    assert_eq!(h.state().params.scanner.operator, stored);
}

#[test]
fn nonprimary_pointer_holds_do_not_compare_or_invalidate_the_corrected_frame() {
    let mut s = session();
    let h = &mut s.harness;
    standard_operator(h);
    operator_edit(h, "Finished exposure", "0.5");
    let corrected = preview(h);
    scroll_controls_into_view(h, "Hold automatic comparison");
    let bounds = h
        .get_by_label("Hold automatic comparison")
        .node()
        .raw_bounds()
        .unwrap();
    let position = egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    );
    let revision = h.state().parameter_revision;
    for button in [egui::PointerButton::Secondary, egui::PointerButton::Middle] {
        h.input_mut()
            .events
            .push(egui::Event::PointerMoved(position));
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: position,
            button,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(4);
        assert!(!h.state().operator_compare);
        assert_eq!(h.state().parameter_revision, revision);
        assert!(Arc::ptr_eq(
            h.state().output_image.as_ref().unwrap(),
            &corrected
        ));
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: position,
            button,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(3);
    }
}

#[test]
fn gpu_operator_comparison_keeps_the_corrected_device_carrier() {
    let _device = spektrafilm_gpu::test_support::device_lock();
    let directory = tempfile::tempdir().unwrap();
    let (image, state) = deterministic_inputs(directory.path());
    let browser = directory.path().join("browser.json");
    let backend: Arc<dyn ComputeBackend> = Arc::new(
        spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit().expect("GPU adapter required"),
    );
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 1400.0))
        .build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            App::new_with_browser_path(backend, Some(image), Some(state), None, browser, data_dir())
        });
    await_input(&mut h);
    standard_operator(&mut h);
    operator_edit(&mut h, "Finished exposure", "1.0");
    let revision = h.state().parameter_revision;
    let corrected_params = h.state().params.clone();
    wait_until(&mut h, |app| {
        app.render_job.is_none()
            && !app.dirty
            && !app.accepted_frame_is_stale()
            && app.output_retained.is_some()
            && app
                .source_render
                .as_ref()
                .is_some_and(|source| source.parameter_revision == Some(revision))
    });
    click(&mut h, "SCAN");
    wait_until(&mut h, |app| {
        app.render_job.is_none()
            && !app.dirty
            && app.retained_kind == Some(RenderKind::Scan)
            && !app.accepted_frame_is_stale()
            && app
                .source_render
                .as_ref()
                .is_some_and(|source| source.parameter_revision == Some(revision))
            && app.output_retained.is_some()
    });
    let corrected = h.state().output_retained.clone().unwrap();
    let source = h.state().image.clone().unwrap();
    settle_animation(&mut h);
    assert!(h.state().viewer.observation().pixels.is_some());
    h.state_mut().set_operator_compare(true);
    wait_until(&mut h, |app| {
        app.operator_comparison.is_some() && app.render_job.is_none() && !app.dirty
    });
    let baseline = &h.state().operator_comparison.as_ref().unwrap().image;
    assert!(
        baseline.host_image().is_none(),
        "GPU automatic baseline must stay retained"
    );
    let crate::render::InputImage::Device(baseline) = baseline else {
        unreachable!()
    };
    assert!(!Arc::ptr_eq(
        baseline.payload().unwrap(),
        corrected.payload().unwrap()
    ));
    settle_animation(&mut h);
    assert!(h.state().viewer.observation().pixels.is_some());
    assert!(h.state().output_image.is_none());
    assert!(Arc::ptr_eq(
        h.state()
            .output_retained
            .as_ref()
            .unwrap()
            .payload()
            .unwrap(),
        corrected.payload().unwrap()
    ));
    assert!(h.state().image.as_ref().unwrap().same_source(&source));
    let path = directory.path().join("corrected-comparison.tiff");
    save_retained(&mut h, &path);
    assert!(path.is_file(), "{}", h.state().status);
    assert!(Arc::ptr_eq(
        h.state()
            .output_retained
            .as_ref()
            .unwrap()
            .payload()
            .unwrap(),
        corrected.payload().unwrap()
    ));
    h.state_mut().set_operator_compare(false);
    wait_until(&mut h, |app| {
        !app.operator_compare && app.render_job.is_none() && !app.dirty
    });
    settle_animation(&mut h);
    assert!(h.state().operator_comparison.is_none());
    assert_eq!(
        h.state().viewer.settings.layer,
        crate::display::ViewLayer::Output
    );
    assert!(h.state().output_image.is_none());
    // Always-auto Preview can republish the corrected frame on release.
    // Its carrier identity is disposable; its saved pixels must be unchanged.
    let released_path = directory.path().join("corrected-released.tiff");
    save_retained(&mut h, &released_path);
    super::panels::assert_float_images_equal(&path, &released_path);
    assert_eq!(h.state().params, corrected_params);
}
