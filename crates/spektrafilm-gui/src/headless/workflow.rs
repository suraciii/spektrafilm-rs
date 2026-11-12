use super::*;

// experimental_gui.py:402–428: every route, canonical state, preview/scan/save.
#[test]
fn every_supported_route_preview_scan_state_roundtrip_and_save() {
    let mut s = session();
    let h = &mut s.harness;
    for (index, (route, label)) in routes::entries().enumerate() {
        reveal_control(h, "workflow.route");
        choose(h, "Film route", label);
        assert_eq!(h.state().params.workflow.route, route);
        let preview = preview(h);
        assert_eq!((preview.width, preview.height), (128, 64), "{route}");
        let scan = scan(h);
        assert_eq!((scan.width, scan.height), (160, 80), "{route}");
        let saved = s.directory.path().join(format!("route-{index}.json"));
        reveal_control(h, "action.save_state");
        script(h, chooser::ChooserResult::Scripted(saved.clone()));
        click(h, "Save current to file");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&saved).unwrap()).unwrap();
        assert_eq!(value["simulation"]["route"], route);
        assert!(value["simulation"].get("scan_film").is_none());
        assert!(value["simulation"].get("workflow").is_none());
        assert!(value["rust"]["runtime"]["workflow"].get("route").is_none());
        assert!(value["rust"]["runtime"]["io"].get("scan_film").is_none());
        script(h, chooser::ChooserResult::Scripted(saved));
        click(h, "Load from file");
        assert!(h.query_by_label("Discard changes").is_some());
        assert_eq!(h.state().params.workflow.route, route);
        click(h, "Discard changes");
        assert!(h.query_by_label("Discard changes").is_none());
        assert_eq!(h.state().params.workflow.route, route);
        close_settings(h);
        let reloaded = super::scan(h);
        assert_eq!(reloaded.data, scan.data, "state reload changed {route}");
        let destination = s.directory.path().join(format!("route-{index}.png"));
        script(h, chooser::ChooserResult::Scripted(destination.clone()));
        click(h, "SAVE");
        wait_until(h, |app| app.pending_saves.is_empty());
        let expected = image_io::convert_image(
            &reloaded,
            &h.state().output_color_space,
            h.state().output_cctf_encoding,
            "sRGB",
            true,
        )
        .unwrap();
        assert_png(&destination, &expected);
    }
}

#[test]
fn glare_off_is_saved_and_survives_route_and_state_reload() {
    let mut s = session();
    let h = &mut s.harness;
    h.state_mut().params.workflow.route = "input > film > print > scan".into();
    h.state_mut().finish_route_change();
    h.state_mut().params.print_render.glare.active = true;
    h.state_mut().mark_parameters_changed();
    h.state_mut().params.print_render.glare.active = false;
    h.state_mut().mark_parameters_changed();

    let path = s.directory.path().join("glare.json");
    h.state().current_state().unwrap().save(&path).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(!saved.to_string().contains(concat!("scan_", "for_print")));
    let restored = state::GuiState::load(&path).unwrap();
    assert!(!restored.runtime_params().unwrap().print_render.glare.active);

    h.state_mut().params.workflow.route = "input".into();
    h.state_mut().finish_route_change();
    assert!(!h.state().params.print_render.glare.active);
    h.state_mut().params.workflow.route = "input > film > print > scan".into();
    h.state_mut().finish_route_change();
    assert!(!h.state().params.print_render.glare.active);

    h.state_mut().params.print_render.glare.active = true;
    h.state_mut().mark_parameters_changed();
    h.state_mut()
        .apply_state_with_source(restored, |_| {})
        .unwrap();
    await_input(h);
    assert_eq!(
        h.state().params.workflow.route,
        "input > film > print > scan"
    );
    assert!(!h.state().params.print_render.glare.active);
    h.state_mut().finish_route_change();
    assert!(!h.state().params.print_render.glare.active);
}

#[test]
fn loaded_state_reports_scanner_normalization_in_header_status() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "workflow.route");
    choose(h, "Film route", "input > film > scan");
    reveal_control(h, "scanner.scan_output");
    choose(h, "scan output", "Negative to positive");
    assert_eq!(h.state().params.scanner.scan_output, "negative_to_positive");
    // A route that cannot carry the negative polarity resets it, and the
    // header status names the control that was normalized.
    reveal_control(h, "workflow.route");
    choose(h, "Film route", "input > film > print > scan");
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
    assert!(
        h.state()
            .status
            .to_string()
            .contains("normalized scanner controls"),
        "{}",
        h.state().status
    );
    assert!(
        h.state().status.to_string().contains("scan output"),
        "{}",
        h.state().status
    );
}

#[test]
fn legacy_disabled_preview_state_still_renders_accepted_numeric_edits_automatically() {
    let mut s = session();
    let h = &mut s.harness;
    let baseline = preview(h);
    let mut legacy = h.state().current_state().unwrap().sections;
    legacy["simulation"]["auto_preview"] = serde_json::json!(false);
    let path = s.directory.path().join("legacy-disabled-preview.json");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    reveal_control(h, "action.save_state");
    script(h, chooser::ChooserResult::Scripted(path));
    click(h, "Load from file");
    if h.query_by_label("Discard changes").is_some() {
        click(h, "Discard changes");
    }
    assert!(h.query_by_label("auto preview").is_none());
    assert!(h.query_by_label("Update").is_none());
    close_settings(h);
    preview(h);
    assert!(h.query_by_label("PREVIEW").is_none());
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "Exposure compensation", "1.0");
    let revision = h.state().parameter_revision;
    let current = preview(h);
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 1.0);
    assert_eq!(
        h.state().source_render.as_ref().unwrap().parameter_revision,
        Some(revision)
    );
    assert!(!h.state().accepted_frame_is_stale());
    assert_ne!(current.data, baseline.data);
    assert!(
        h.state().current_state().unwrap().sections["simulation"]
            .get("auto_preview")
            .is_none()
    );
}

#[test]
fn workflow_modes_keep_stage_navigation_visible_and_mark_inactive_stages() {
    let mut s = session();
    let h = &mut s.harness;
    navigate_stage(h, controls::Stage::Film);
    assert!(h.query_by_label("Film material").is_some());
    // The Look stage stays reachable in film mode because it owns the mode
    // switch; its controls explain that they are inactive.
    navigate_stage(h, controls::Stage::Look);
    assert!(h.query_by_label("Look").is_some());
    assert!(
        h.query_all_by_label("Inactive in the film workflow. Stored values are retained.")
            .next()
            .is_some()
    );

    // Give the active LUT a valid external file so the production workflow
    // switch does not open a chooser; import itself has separate coverage.
    let cube = s.directory.path().join("layout.cube");
    std::fs::write(
        &cube,
        "TITLE \"layout\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n\
         0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
    )
    .unwrap();
    h.state_mut().params.lut_look.path = cube;
    h.state_mut().params.lut_look.sha256 =
        spektrafilm_core::lut_look::hash_file(&h.state().params.lut_look.path).unwrap();
    let retained_route = h.state().params.workflow.route.clone();
    // A module in a stage that remains active in both workflows keeps its
    // disclosure state across the real workflow selector.
    navigate_stage(h, controls::Stage::Input);
    if h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "input color space")
        .is_none()
    {
        click(h, "Input");
    }
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "input color space")
            .is_some()
    );
    navigate_stage(h, controls::Stage::Look);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "LUT")
        .simulate_click();
    h.run_steps(6);
    await_input(h);
    assert_eq!(
        h.state().params.workflow.mode,
        spektrafilm_core::params::WorkflowMode::Lut
    );
    assert_eq!(
        h.state().film_route.as_deref(),
        Some(retained_route.as_str())
    );
    assert!(h.query_by_label("Print").is_some());
    navigate_stage(h, controls::Stage::Input);
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "input color space")
            .is_some(),
        "an always-active module must keep its disclosure state across a workflow switch"
    );
    // Inactive film modules keep their headings and values readable but cannot
    // reset the inactive film chain from LUT mode.
    navigate_stage(h, controls::Stage::Film);
    assert!(h.get_by_label("reset Film material").node().is_disabled());
    // The retained route is visible at its owner, but neither its selector nor
    // its reset may edit or normalize the inactive film chain from LUT mode.
    navigate_stage(h, controls::Stage::Input);
    let route = h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Film route");
    assert!(route.node().is_disabled());
    assert!(h.get_by_label("reset Workflow").node().is_disabled());
    // The product label is also the search and accessibility label.
    search_controls(h, "Film route");
    assert!(
        h.query_all_by_role(egui::accesskit::Role::Button)
            .any(|node| node
                .label()
                .is_some_and(|label| label.contains("Film route")))
    );
    search_controls(h, "");
    // Inactive film controls remain present at their owner with their stored
    // values and explain the LUT workflow's boundary.
    search_controls(h, "Film material");
    assert!(h.query_by_label("Film material").is_some());
    assert!(
        h.query_all_by_label("Inactive in the LUT workflow. Stored values are retained.")
            .next()
            .is_some()
    );
    search_controls(h, "");
    h.run_steps(3);
    assert!(h.query_by_label("Look").is_some());
}

#[test]
fn lut_import_previews_at_the_bounded_size_and_preserves_the_declaration() {
    let mut s = session();
    let h = &mut s.harness;
    h.state_mut().diagnostics.mode = spektrafilm_core::telemetry::CollectionMode::Off;
    let cube = s.directory.path().join("identity.cube");
    std::fs::write(
        &cube,
        "TITLE \"identity\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n\
         0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
    )
    .unwrap();

    script(h, chooser::ChooserResult::Scripted(cube.clone()));
    h.state_mut()
        .switch_workflow(spektrafilm_core::params::WorkflowMode::Lut);
    h.run_steps(6);
    click(h, "Import");
    assert_eq!(
        h.state().params.lut_look.domain_policy,
        spektrafilm_core::lut_look::LutDomainPolicy::Clamp
    );

    let bounded = preview(h);
    assert_eq!((bounded.width, bounded.height), (128, 64));
    assert_eq!(h.state().image.as_ref().unwrap().width(), 160);
    assert_eq!(h.state().params.lut_look.path, cube);

    h.state_mut().params.lut_look.diffusion.active = true;
    h.state_mut().mark_parameters_changed();
    let with_diffusion = preview(h);
    assert_eq!((with_diffusion.width, with_diffusion.height), (128, 64));

    h.state_mut()
        .switch_workflow(spektrafilm_core::params::WorkflowMode::Film);
    h.state_mut()
        .switch_workflow(spektrafilm_core::params::WorkflowMode::Lut);
    assert_eq!(h.state().params.lut_look.path, cube);
    assert_eq!(
        h.state().params.workflow.mode,
        spektrafilm_core::params::WorkflowMode::Lut
    );
}

#[test]
fn lut_import_dialog_explains_clamp_spaces_and_advanced_declaration() {
    let mut s = session();
    let h = &mut s.harness;
    let cube = s.directory.path().join("dialog.cube");
    std::fs::write(
        &cube,
        "TITLE \"dialog\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n\
         0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
    )
    .unwrap();
    script(h, chooser::ChooserResult::Scripted(cube));
    h.state_mut()
        .switch_workflow(spektrafilm_core::params::WorkflowMode::Lut);
    h.run_steps(8);
    assert!(h.query_by_label("Clamp and report").is_some());
    assert!(h.query_by_label("Fail before output").is_some());
    assert!(h.query_by_label("Advanced declaration").is_some());
    assert!(
        h.query_all_by_label_contains("does not carry color-space metadata")
            .next()
            .is_some()
    );
    assert!(h.query_by_label("Cancel").is_some());
}

#[test]
fn replacing_a_cube_preserves_finishing_and_reports_clipping_in_the_lut_section() {
    let mut s = session();
    let h = &mut s.harness;
    h.state_mut().diagnostics.mode = spektrafilm_core::telemetry::CollectionMode::Off;
    let cube = |name: &str, minimum: f64, maximum: f64| {
        let path = s.directory.path().join(name);
        std::fs::write(
            &path,
            format!(
                "TITLE \"{name}\"\nLUT_3D_SIZE 2\nDOMAIN_MIN {minimum} {minimum} {minimum}\nDOMAIN_MAX {maximum} {maximum} {maximum}\n\
                 0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n"
            ),
        )
        .unwrap();
        path
    };
    let first = cube("first.cube", 0.0, 1.0);
    h.state_mut().params.lut_look = spektrafilm_core::lut_look::LutLookParams::from_cube(
        &first,
        spektrafilm_core::lut_look::LutSpace::Srgb,
        spektrafilm_core::lut_look::LutSpace::Srgb,
        spektrafilm_core::lut_look::LutDomainPolicy::Clamp,
    )
    .unwrap();
    h.state_mut()
        .switch_workflow(spektrafilm_core::params::WorkflowMode::Lut);
    h.run_steps(3);
    await_input(h);

    h.state_mut().params.lut_look.strength = 0.4;
    h.state_mut().params.lut_look.grade_active = true;
    h.state_mut().params.lut_look.grade.contrast = 0.3;
    h.state_mut().params.lut_look.diffusion.active = true;
    h.state_mut().mark_parameters_changed();

    let second = cube("second.cube", 0.1, 0.9);
    script(h, chooser::ChooserResult::Scripted(second.clone()));
    h.state_mut().begin_import_lut(true);
    h.run_steps(6);
    click(h, "Import");

    let params = &h.state().params.lut_look;
    assert_eq!(params.path, second);
    assert_eq!(params.strength, 0.4);
    assert!(params.grade_active);
    assert_eq!(params.grade.contrast, 0.3);
    assert!(params.diffusion.active);
    assert_eq!(
        params.domain_policy,
        spektrafilm_core::lut_look::LutDomainPolicy::Clamp
    );

    let _ = preview(h);
    assert!(h.state().lut_clipped_fraction.is_some());
    navigate_stage(h, controls::Stage::Look);
    h.run_steps(3);
    assert!(
        h.query_all_by_label_contains("LUT input clamped")
            .next()
            .is_some()
    );
    h.state_mut().mark_parameters_changed();
    assert!(
        h.state().lut_clipped_fraction.is_none(),
        "a changed parameter invalidates the accepted clipping report"
    );
}
