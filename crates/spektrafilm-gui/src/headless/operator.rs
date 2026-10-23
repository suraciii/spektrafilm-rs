use super::*;

#[test]
fn standard_operator_preview_matches_scan_and_hold_compare_preserves_saved_correction() {
    let mut session = session();
    let h = &mut session.harness;
    choose(h, "workflow", "input > film > scan");
    assert_eq!(h.state().film_medium, controls::ScanMedium::Negative);
    if h.query_by_label("scan output").is_none() {
        click(h, "Scanner");
    }
    choose(h, "scan output", "Negative to positive");
    assert_eq!(h.state().params.scanner.conversion_style, "standard");
    h.state_mut().viewer.settings.crossfade = false;

    let automatic_preview = render(h, "PREVIEW");
    assert_eq!(h.state().params.settings.preview_max_size, 128);
    assert_eq!(
        (automatic_preview.width, automatic_preview.height),
        (160, 80)
    );
    let automatic_scan = render(h, "SCAN");
    assert_eq!(automatic_preview.data, automatic_scan.data);
    settle_animation(h);
    let automatic_display = h.state().viewer.output.as_ref().unwrap().clone();
    click(h, "100%");
    settle_animation(h);
    let automatic_composed = h.state().viewer.observation().pixels.unwrap();

    scroll_controls_into_view(h, "Finished exposure");
    edit(h, "Finished exposure", "1.0");
    assert_eq!(h.state().params.scanner.operator.exposure_ev, 1.0);
    let corrected = render(h, "PREVIEW");
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
    h.state_mut().save_to_path(&save_before);
    assert!(
        h.state().status.starts_with("Saved"),
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
            && !app.force_preview
            && !app.dirty
            && app.render_job.is_none()
            && app.status.starts_with("Comparing automatic baseline")
    });
    settle_animation(h);
    let compared_display = h.state().viewer.output.as_ref().unwrap();
    assert_eq!(compared_display.size, automatic_display.size);
    assert_eq!(compared_display.rgb, automatic_display.rgb);
    let compared_composed = h.state().viewer.observation().pixels.unwrap();
    assert_eq!(compared_composed.size, automatic_composed.size);
    assert_eq!(compared_composed.pixels, automatic_composed.pixels);
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
    h.state_mut().save_to_path(&save_while_held);
    assert!(
        h.state().status.starts_with("Saved"),
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
        !app.force_preview
            && !app.dirty
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
}
