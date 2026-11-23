//! Multiple exposure through the production panel, input worker, and renderer.
use super::*;

#[test]
fn work_list_export_loads_multiple_exposure_sources() {
    let mut session = session();
    let h = &mut session.harness;
    let primary = h.state().image_path.clone().unwrap();
    let second = session.directory.path().join("second-exposure.png");
    std::fs::copy(&primary, &second).unwrap();
    click(h, "Photographs");
    click(h, "Multiple exposure");
    script(h, chooser::ChooserResult::Scripted(second));
    click(h, "Add shots…");
    await_input(h);
    assert!(h.state().multiple_exposure.active());
    let output = session.directory.path().join("batch");
    std::fs::create_dir_all(&output).unwrap();
    h.state_mut().add_work_paths(Vec::new());
    h.state_mut().select_work_destination(0, true);
    let ctx = h.ctx.clone();
    h.state_mut()
        .submit_work_exports(&ctx, &output, false)
        .unwrap();
    let stem = primary.file_stem().unwrap().to_string_lossy();
    let expected = output.join(format!("{stem}.png"));
    wait_until(h, |app| app.export_job.is_none() && expected.is_file());
    assert!(expected.is_file(), "{}", h.state().status);
}

#[test]
fn multiple_exposure_panel_composes_preview_and_scan() {
    let mut session = session();
    let h = &mut session.harness;
    let primary = h.state().image_path.clone().unwrap();
    let second = session.directory.path().join("second-exposure.png");
    std::fs::copy(&primary, &second).unwrap();
    let third = session.directory.path().join("third-exposure.png");
    std::fs::copy(&primary, &third).unwrap();
    let single = preview(h);
    let single_mean = mean(&single);

    click(h, "Photographs");
    click(h, "Multiple exposure");
    script(h, chooser::ChooserResult::Scripted(second));
    click(h, "Add shots…");
    await_input(h);
    assert_eq!(h.state().multiple_exposure.settings.total_shots(), 2);
    assert!(h.state().multiple_exposure.active());
    assert!(h.state().multiple_exposure.stack().extras.len() == 1);
    click(h, "Undo");
    assert!(!h.state().multiple_exposure.active());
    click(h, "Redo");
    assert!(h.state().multiple_exposure.active());
    h.state_mut().multiple_exposure.settings.shots[0].exposure_ev = 1.25;
    h.state_mut().mark_parameters_changed();
    h.state_mut().reload_multiple_exposure_sources();
    await_input(h);
    assert_eq!(
        h.state().multiple_exposure.settings.shots[0].exposure_ev,
        1.25
    );
    h.state_mut().multiple_exposure.settings.shots[0].exposure_ev = 0.0;
    h.state_mut().mark_parameters_changed();
    click(h, "2. second-exposure.png");
    assert_eq!(h.state().multiple_exposure.selected, 1);
    script(h, chooser::ChooserResult::Scripted(third.clone()));
    click(h, "…");
    await_input(h);
    assert_eq!(
        h.state().multiple_exposure.settings.shots[0].path,
        third.canonicalize().unwrap()
    );
    assert_eq!(h.state().multiple_exposure.selected, 1);
    let previewed = preview(h);
    assert!(
        mean(&previewed) > single_mean,
        "two equal exposures must add light before the look: {} vs {}",
        mean(&previewed),
        single_mean
    );
    assert_eq!(
        [previewed.width, previewed.height],
        [single.width, single.height]
    );

    let scanned = scan(h);
    assert_eq!([scanned.width, scanned.height], [160, 80]);
    assert!(h.state().multiple_exposure.settings.primary_exposure_ev == 0.0);

    // The RGB branch is the other composition model. Use the product's Off
    // workflow so the test does not need an external LUT file.
    h.state_mut().params.workflow.mode = spektrafilm_core::params::WorkflowMode::Off;
    h.state_mut().params.workflow.route = "input".into();
    h.state_mut().multiple_exposure.settings.method =
        crate::multiple_exposure::MultipleExposureMethod::Rgb;
    h.state_mut().mark_parameters_changed();
    h.state_mut().render_worker.invalidate_runtime_cache();
    h.state_mut().source_render = None;
    let rgb = preview(h);
    assert_eq!([rgb.width, rgb.height], [128, 64]);
    reveal_control(h, "effects.light_leak.active");
    click(h, "RGB Light Leak");
    assert!(h.state().params.effects.as_ref().unwrap().light_leak.active);
    let leaked = preview(h);
    assert_ne!(rgb.data, leaked.data);
    reveal_control(h, "effects.light_leak.exposure_ev");
    edit(h, "Leak exposure ev", "1.5");
    let leak = &h.state().params.effects.as_ref().unwrap().light_leak;
    assert!(
        (leak.exposure_ev - 1.5).abs() < 1e-3,
        "exposure {}",
        leak.exposure_ev
    );
    reveal_control(h, "effects.light_leak.width");
    edit(h, "Leak width", "0.75");
    let leak = &h.state().params.effects.as_ref().unwrap().light_leak;
    assert!((leak.width - 0.75).abs() < 1e-3, "width {}", leak.width);
    assert!((leak.spread - 0.32).abs() < 1e-3, "spread {}", leak.spread);
    reveal_control(h, "effects.light_leak.spread");
    edit(h, "Leak spread", "0.30");
    let leak = &h.state().params.effects.as_ref().unwrap().light_leak;
    assert!((leak.spread - 0.30).abs() < 1e-3, "spread {}", leak.spread);
    assert!((leak.width - 0.75).abs() < 1e-3, "width {}", leak.width);
    search_controls(h, "");
    navigate_stage(h, controls::Stage::Input);
    reveal_control(h, "action.rotate_cw");
    click(h, "cw rotate");
    await_input(h);
    let rotated = preview(h);
    assert_eq!([rotated.width, rotated.height], [64, 128]);
    let export_path = session.directory.path().join("multiple-exposure.png");
    let options = h.state().export_options.clone();
    let ctx = h.ctx.clone();
    h.state_mut()
        .start_export_to_path(&ctx, options, &export_path);
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.to_string().starts_with("Exported"),
        "{}",
        h.state().status
    );
    assert!(export_path.is_file());
}
