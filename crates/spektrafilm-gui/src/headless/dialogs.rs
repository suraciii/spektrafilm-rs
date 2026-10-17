use super::*;
use crate::export_job::ExportPause;
use parking_lot::{Condvar, Mutex};
use spektrafilm_core::presets::{Preset, PresetLibrary, Provenance};
use std::sync::mpsc;

fn expand_preset_library(h: &mut Harness<'_, App>) {
    if h.query_by_label("▶").is_some() {
        click(h, "▶");
    }
    click(h, "Refresh");
}

fn context_click(h: &mut Harness<'_, App>, label: &str) {
    let bounds = h
        .get_by_label(label)
        .raw_bounds()
        .unwrap_or_else(|| panic!("missing context-menu target {label}"));
    let position = egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    );
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(position));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        h.step();
    }
    h.run_steps(3);
}

fn name_field_bounds(h: &Harness<'_, App>) -> egui::accesskit::Rect {
    let label = h
        .get_by_label("Name in My Presets")
        .raw_bounds()
        .expect("preset name prompt bounds");
    h.query_all_by_role(egui::accesskit::Role::TextInput)
        .filter_map(|node| node.raw_bounds())
        .find(|bounds| {
            (bounds.x0 - label.x0).abs() < 12.0
                && bounds.y0 >= label.y0 - 2.0
                && bounds.y0 <= label.y1 + 100.0
        })
        .unwrap_or_else(|| panic!("preset name text field is missing near {label:?}"))
}

fn edit_preset_name(h: &mut Harness<'_, App>, text: &str) {
    let bounds = name_field_bounds(h);
    let inputs: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .filter(|node| node.raw_bounds() == Some(bounds))
        .collect();
    assert_eq!(inputs.len(), 1, "preset name field must be unique");
    inputs[0].focus();
    h.step();
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: !cfg!(target_os = "macos"),
            mac_cmd: cfg!(target_os = "macos"),
            command: true,
            ..Default::default()
        },
    });
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    h.run_steps(3);
    let current = name_field_bounds(h);
    let field = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .find(|node| node.raw_bounds() == Some(current))
        .expect("preset name text field remains visible");
    assert_eq!(field.value().as_deref(), Some(text));
}

fn preset_row(h: &Harness<'_, App>, name: &str) -> String {
    let nodes: Vec<_> = h
        .query_all_by_label_contains(name)
        .filter(|node| node.node().role() == egui::accesskit::Role::Button)
        .collect();
    assert_eq!(nodes.len(), 1, "preset row for {name}");
    nodes[0].label().unwrap().to_string()
}

fn assert_quality_hidden(h: &Harness<'_, App>) {
    assert!(
        h.query_all_by_role_and_label(egui::accesskit::Role::Slider, "JPEG quality")
            .next()
            .is_none()
    );
    assert!(
        h.query_all_by_role_and_label(egui::accesskit::Role::SpinButton, "JPEG quality")
            .next()
            .is_none()
    );
}
fn assert_quality_controls(h: &Harness<'_, App>) {
    assert!(
        h.query_all_by_role_and_label(egui::accesskit::Role::Slider, "JPEG quality")
            .next()
            .is_some()
    );
    assert!(
        h.query_all_by_role_and_label(egui::accesskit::Role::SpinButton, "JPEG quality")
            .next()
            .is_some()
    );
}

fn choose_diagnostics_mode(h: &mut Harness<'_, App>, next: &str) {
    let combos: Vec<_> = h
        .query_all_by_role(egui::accesskit::Role::ComboBox)
        .filter(|node| node.label().unwrap_or_default().is_empty())
        .collect();
    assert_eq!(combos.len(), 1, "diagnostics mode selector");
    combos[0].simulate_click();
    h.run_steps(3);
    let options: Vec<_> = h.query_all_by_label(next).collect();
    assert_eq!(options.len(), 1, "diagnostics option {next}");
    options[0].simulate_click();
    h.run_steps(3);
}

fn click_status_row(h: &mut Harness<'_, App>, label: &str) {
    let rows: Vec<_> = h
        .query_all_by_label_contains(label)
        .filter(|node| node.node().role() == egui::accesskit::Role::Button)
        .collect();
    assert_eq!(rows.len(), 1, "report selection row {label}");
    rows[0].simulate_click();
    h.run_steps(3);
}

fn click_preset_action(h: &mut Harness<'_, App>, row: &str, action: &str) {
    context_click(h, row);
    assert!(
        h.query_by_label(action).is_some(),
        "{action} should be available for {row}"
    );
    click(h, action);
}

fn preset_provenance() -> Provenance {
    Provenance {
        implementation_version: env!("CARGO_PKG_VERSION").into(),
        model_version: "spektrafilm".into(),
        scanner: None,
    }
}

fn external_look(h: &Harness<'_, App>, name: &str, path: &Path) {
    let preset = Preset::capture(
        name,
        &h.state().film_name,
        &h.state().print_name,
        &h.state().params,
        preset_provenance(),
    );
    std::fs::write(path, preset.to_toml().unwrap()).unwrap();
}

#[test]
fn preset_save_revert_rename_duplicate_delete_and_import_conflict_use_real_dialogs() {
    let mut s = session();
    let h = &mut s.harness;
    expand_preset_library(h);

    click(h, "Save preset…");
    assert!(h.query_by_label("Name in My Presets").is_some());
    let invalid_name = "bad/name";
    edit_preset_name(h, invalid_name);
    let expected_error = spektrafilm_core::presets::validate_name(invalid_name).unwrap_err();
    assert!(h.query_by_label(&expected_error).is_some());
    assert!(h.get_by_label("Save").node().is_disabled());
    edit_preset_name(h, "Dialog Look");
    assert!(!h.get_by_label("Save").node().is_disabled());
    click(h, "Save");
    let library = PresetLibrary::new(h.state().config_directory.join("presets"));
    let original = library.directory().join("Dialog Look.toml");
    assert!(original.is_file());
    assert_eq!(
        Preset::from_toml_str(&std::fs::read_to_string(&original).unwrap())
            .unwrap()
            .name,
        "Dialog Look"
    );
    assert_eq!(
        h.state().status,
        format!("Saved preset 'Dialog Look' to {}", original.display())
    );
    assert_eq!(preset_row(h, "Dialog Look"), "Dialog Look • applied");

    edit(h, "Simulation blend", "0.25");
    assert!((h.state().params.output_mix.amount - 0.25).abs() < 1e-6);
    let row = preset_row(h, "Dialog Look");
    assert_eq!(row, "Dialog Look • modified");
    click_preset_action(h, &row, "Revert");
    assert_eq!(h.state().status, "Applied preset 'Dialog Look'");
    assert!((h.state().params.output_mix.amount - 1.0).abs() < 1e-6);

    let row = preset_row(h, "Dialog Look");
    click_preset_action(h, &row, "Rename…");
    assert!(h.query_by_label("Name in My Presets").is_some());
    edit_preset_name(h, "Renamed Look");
    click(h, "Save");
    let renamed = library.directory().join("Renamed Look.toml");
    assert!(renamed.is_file());
    assert!(!original.exists());
    assert_eq!(
        Preset::from_toml_str(&std::fs::read_to_string(&renamed).unwrap())
            .unwrap()
            .name,
        "Renamed Look"
    );
    let row = preset_row(h, "Renamed Look");
    assert!(row.ends_with("• applied"));

    click_preset_action(h, &row, "Duplicate…");
    edit_preset_name(h, "Duplicate Look");
    click(h, "Save");
    let duplicate = library.directory().join("Duplicate Look.toml");
    assert!(duplicate.is_file());
    assert_eq!(
        Preset::from_toml_str(&std::fs::read_to_string(&duplicate).unwrap())
            .unwrap()
            .name,
        "Duplicate Look"
    );
    let row = preset_row(h, "Duplicate Look");
    click_preset_action(h, &row, "Delete…");
    assert!(
        h.query_by_label("Delete 'Duplicate Look' from My Presets?")
            .is_some()
    );
    click(h, "Delete");
    assert!(!duplicate.exists());
    assert_eq!(h.state().status, "Deleted preset 'Duplicate Look'");

    let external = s.directory.path().join("external-look.toml");
    external_look(h, "Renamed Look", &external);
    script(h, chooser::ChooserResult::Scripted(external.clone()));
    click(h, "Import");
    let conflict =
        "My Presets name \"Renamed Look\" already exists; choose a name for a renamed copy";
    assert!(
        h.query_all_by_label_contains("My Presets name")
            .next()
            .is_some()
    );
    assert!(h.query_all_by_label_contains(conflict).next().is_some());
    assert!(h.state().status.contains(conflict), "{}", h.state().status);
    edit_preset_name(h, "Imported Copy");
    click(h, "Save");
    let imported = library.directory().join("Imported Copy.toml");
    assert!(external.is_file());
    assert!(imported.is_file());
    assert_eq!(
        Preset::from_toml_str(&std::fs::read_to_string(&imported).unwrap())
            .unwrap()
            .name,
        "Imported Copy"
    );
    assert_eq!(
        h.state().status,
        format!("Saved preset 'Imported Copy' to {}", imported.display())
    );
}

fn open_export_dialog(h: &mut Harness<'_, App>) {
    click(h, "MAIN");
    if h.query_by_label("Open export settings…").is_none() {
        click(h, "Export options");
    }
    export_action(h, "Open export settings…");
    assert!(h.state().export_dialog.is_open());
}

fn change_export_format(h: &mut Harness<'_, App>, next: &str) {
    choose(h, "Format", next);
}

#[test]
fn export_dialog_format_controls_are_conditional_and_both_cancel_paths_write_nothing() {
    let mut s = session();
    let h = &mut s.harness;
    open_export_dialog(h);

    change_export_format(h, "JPEG");
    assert_quality_controls(h);
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "JPEG subsampling")
            .is_some()
    );
    assert!(h.query_by_label("Apply saving CCTF encoding").is_none());
    choose(h, "JPEG subsampling", "4:2:0");

    change_export_format(h, "TIFF");
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "Compression")
            .is_some()
    );
    assert_quality_hidden(h);
    assert!(h.query_by_label("Apply saving CCTF encoding").is_some());
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Compression")
        .simulate_click();
    h.run_steps(3);
    assert!(h.query_by_label("ZIP").is_some());
    assert!(h.query_by_label("None").is_some());
    // Selecting an option closes the popup; Escape would cancel the whole
    // export dialog (Escape is its cancel path).
    h.get_by_label("ZIP").simulate_click();
    h.run_steps(3);
    assert!(h.state().export_dialog.is_open());

    change_export_format(h, "EXR");
    assert!(
        h.query_by_label("EXR uses linear color and ZIP compression.")
            .is_some()
    );
    assert!(h.query_by_label("Apply saving CCTF encoding").is_none());
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Saving color space")
        .simulate_click();
    h.run_steps(3);
    assert!(h.query_by_label("sRGB").is_some());
    assert!(h.query_by_label("ACES2065-1").is_some());
    assert!(h.query_by_label("Display P3").is_none());
    h.get_by_label("ACES2065-1").simulate_click();
    h.run_steps(3);
    click(h, "Cancel");
    assert!(!h.state().export_dialog.is_open());
    assert_eq!(h.state().export_options.format, export::ExportFormat::Png);

    let escape_path = s.directory.path().join("escape-export.png");
    open_export_dialog(h);
    change_export_format(h, "JPEG");
    script(h, chooser::ChooserResult::Scripted(escape_path.clone()));
    h.press_key(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.state().export_dialog.is_open());
    assert!(h.state().export_job.is_none());
    assert!(!escape_path.exists());

    let close_path = s.directory.path().join("close-export.png");
    open_export_dialog(h);
    script(h, chooser::ChooserResult::Scripted(close_path.clone()));
    click(h, "Close window");
    assert!(!h.state().export_dialog.is_open());
    assert!(h.state().export_job.is_none());
    assert!(!close_path.exists());
}

// The export dialog is modal, but a modal window does not block drag-and-drop,
// so the input the export renders can be replaced while the dialog is open. The
// estimate reads the current input, not the one the dialog was opened with.
#[test]
fn the_export_estimate_follows_the_input_while_the_dialog_is_open() {
    let mut s = session();
    let h = &mut s.harness;
    open_export_dialog(h);
    let opened_with = estimated_output_size_row(h);

    let path = s.directory.path().join("wider-input.png");
    let mut image = ImageBuf::new(320, 240);
    for value in image.data.iter_mut() {
        *value = spektrafilm_math::precision::from_f64(0.25);
    }
    image_io::save(
        &path,
        &image,
        SaveOptions {
            depth: BitDepth::Eight,
            color_space: "sRGB",
            cctf_encoding: true,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
        },
        None,
    )
    .unwrap();
    h.state_mut().load_image_from_path(&path);
    h.run_steps(3);

    assert_eq!(h.state().image.as_ref().unwrap().width, 320);
    assert!(h.state().export_dialog.is_open());
    let replaced_with = estimated_output_size_row(h);
    assert_ne!(
        replaced_with, opened_with,
        "the estimate must describe the input the export now renders"
    );
    assert!(
        replaced_with.contains("225.0 KB"),
        "320 x 240 x 3 samples is a 225.0 KB payload, got {replaced_with}"
    );
}

fn estimated_output_size_row(h: &Harness<'_, App>) -> String {
    h.query_by_label_contains("Estimated output size")
        .and_then(|node| node.value())
        .unwrap_or_else(|| panic!("the export dialog states an estimated output size"))
}

fn report_paths(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

fn read_report(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn save_report_from_panel(h: &mut Harness<'_, App>, path: &Path) {
    script(h, chooser::ChooserResult::Scripted(path.to_path_buf()));
    click(h, "Save report…");
    assert_eq!(h.state().status, "Saved diagnostic report.");
    assert!(path.is_file());
}

#[test]
fn diagnostics_gui_modes_capture_reports_select_saved_reports_and_cancel_save() {
    let mut s = session();
    let h = &mut s.harness;
    let published = s.directory.path().join("published-reports");
    std::fs::create_dir(&published).unwrap();
    h.state_mut()
        .diagnostics
        .publish_completed_to(published.clone());

    click(h, "CONFIG");
    choose_diagnostics_mode(h, "Summary");
    click(h, "MAIN");
    render(h, "PREVIEW");
    click(h, "CONFIG");
    let first_reports = report_paths(&published);
    assert_eq!(first_reports.len(), 1);
    let first = read_report(&first_reports[0]);
    assert_eq!(
        first["configuration"]["collection_mode_requested"],
        "summary"
    );
    let first_id = first["operation"]["id"].as_u64().unwrap();
    let summary_report_path = s.directory.path().join("summary-report.json");
    save_report_from_panel(h, &summary_report_path);
    assert_eq!(
        read_report(&summary_report_path)["operation"]["id"],
        first_id
    );

    choose_diagnostics_mode(h, "GPU timing");
    click(h, "MAIN");
    render(h, "PREVIEW");
    click(h, "CONFIG");
    let reports = report_paths(&published);
    assert_eq!(reports.len(), 2);
    let gpu_report_path = reports
        .iter()
        .find(|path| {
            read_report(path)["configuration"]["collection_mode_requested"] == "gpu_timing"
        })
        .unwrap();
    let gpu_report = read_report(gpu_report_path);
    assert_eq!(
        gpu_report["configuration"]["collection_mode_requested"],
        "gpu_timing"
    );
    let gpu_report_save = s.directory.path().join("gpu-timing-report.json");
    save_report_from_panel(h, &gpu_report_save);
    assert_eq!(
        read_report(&gpu_report_save)["operation"]["id"],
        gpu_report["operation"]["id"]
    );

    let row = format!("#{} preview · succeeded", first_id);
    assert!(h.query_by_label(&row).is_some(), "report list row {row}");
    click_status_row(h, &row);
    assert!(h.query_all_by_label_contains("Requested:").next().is_some());
    let selected_summary_path = s.directory.path().join("selected-summary-report.json");
    save_report_from_panel(h, &selected_summary_path);
    let selected = read_report(&selected_summary_path);
    assert_eq!(selected["operation"]["id"], first_id);
    assert_eq!(
        selected["configuration"]["collection_mode_requested"],
        "summary"
    );

    choose_diagnostics_mode(h, "Off");
    click(h, "MAIN");
    render(h, "PREVIEW");
    click(h, "CONFIG");
    assert_eq!(
        report_paths(&published).len(),
        2,
        "Off does not retain or publish a report"
    );
    let status = h.state().status.clone();
    let canceled = s.directory.path().join("canceled-report.json");
    script(h, chooser::ChooserResult::Canceled);
    click(h, "Save report…");
    assert_eq!(h.state().status, status);
    assert!(!canceled.exists());
}

/// A user-selected report row must stay selected across subsequent renders.
/// The `user_selected` flag in `retain()` prevents auto-jump from the newest
/// report to a row the user has explicitly inspected.
#[test]
fn diagnostics_user_selected_report_survives_later_renders() {
    let mut s = session();
    let h = &mut s.harness;
    let published = s.directory.path().join("published-reports");
    std::fs::create_dir(&published).unwrap();
    h.state_mut()
        .diagnostics
        .publish_completed_to(published.clone());

    click(h, "CONFIG");
    choose_diagnostics_mode(h, "Summary");
    click(h, "MAIN");
    render(h, "PREVIEW");
    click(h, "CONFIG");

    let reports = report_paths(&published);
    assert_eq!(reports.len(), 1);
    let first_id = read_report(&reports[0])["operation"]["id"]
        .as_u64()
        .unwrap();

    // The single report is auto-selected.
    assert_eq!(h.state().diagnostics.selected_report_id(), Some(first_id));

    // Simulate the user clicking the older report row (sets user_selected = true).
    click(h, &format!("#{} {} · {}", first_id, "preview", "succeeded"));
    assert_eq!(
        h.state().diagnostics.selected_report_id(),
        Some(first_id),
        "user click must preserve the selection"
    );

    // Trigger another render; the user's selection must survive.
    click(h, "MAIN");
    render(h, "PREVIEW");
    click(h, "CONFIG");
    assert_eq!(
        h.state().diagnostics.selected_report_id(),
        Some(first_id),
        "selection must persist across a subsequent render"
    );
}

fn save_test_png(path: &Path, width: u32, height: u32, value: f64) {
    let image = ImageBuf {
        width,
        height,
        data: vec![spektrafilm_math::precision::from_f64(value); (width * height * 3) as usize],
    };
    image_io::save(
        path,
        &image,
        SaveOptions {
            depth: BitDepth::Eight,
            color_space: "sRGB",
            cctf_encoding: true,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
        },
        None,
    )
    .unwrap();
}

fn drop_file(h: &mut Harness<'_, App>, path: &Path) {
    h.input_mut().dropped_files.push(egui::DroppedFile {
        path: Some(path.to_path_buf()),
        ..Default::default()
    });
    h.step();
}

#[test]
fn dropped_png_uses_the_import_path_and_bad_drop_preserves_accepted_images() {
    let mut s = session();
    let h = &mut s.harness;
    let dropped = s.directory.path().join("dropped.png");
    save_test_png(&dropped, 7, 3, 0.4);
    drop_file(h, &dropped);
    let expected = image_io::load(&dropped).unwrap().image;
    let loaded = h.state().image.as_ref().unwrap();
    assert_eq!(h.state().image_path.as_ref(), Some(&dropped));
    assert_eq!((loaded.width, loaded.height), (7, 3));
    assert_eq!(loaded.data, expected.data);
    assert!(h.state().output_image.is_none());
    assert!(h.state().status.starts_with("Loaded 7 × 3"));

    let mut imported = session();
    script(
        &mut imported.harness,
        chooser::ChooserResult::Scripted(dropped.clone()),
    );
    import_file(&mut imported.harness, false);
    assert_eq!(
        h.state().image.as_ref().unwrap().data,
        imported.harness.state().image.as_ref().unwrap().data
    );
    assert_eq!(h.state().params, imported.harness.state().params);
    assert_eq!(h.state().image_path, imported.harness.state().image_path);
    assert_eq!(h.state().input_epoch, imported.harness.state().input_epoch);

    let retained_image = h.state().image.as_ref().unwrap().clone();
    let retained_path = h.state().image_path.clone();
    let params = h.state().params.clone();
    let corrupt = s.directory.path().join("dropped-corrupt.png");
    std::fs::write(&corrupt, b"not an image").unwrap();
    drop_file(h, &corrupt);
    assert!(
        h.state().status.starts_with("Load error:"),
        "{}",
        h.state().status
    );
    assert_eq!(h.state().image_path, retained_path);
    assert_eq!(h.state().params, params);
    assert!(Arc::ptr_eq(
        h.state().image.as_ref().unwrap(),
        &retained_image
    ));
    assert!(h.state().output_image.is_none());
}

fn empty_session_for_guards() -> Session {
    let directory = tempfile::tempdir().unwrap();
    let factory = state::GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.settings.preview_max_size = 128;
    params.camera.auto_exposure = false;
    params.debug.deactivate_spatial_effects = true;
    params.debug.deactivate_stochastic_effects = true;
    params.io.crop = false;
    let mut initial =
        state::GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
            .unwrap();
    initial.sections["simulation"]["auto_preview"] = serde_json::json!(false);
    let state_path = directory.path().join("initial.json");
    initial.save(&state_path).unwrap();
    let browser_path = directory.path().join("browser.json");
    let harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 1400.0))
        .build_eframe(move |_| {
            App::new_with_browser_path(
                Arc::new(CpuBackend),
                None,
                Some(state_path),
                None,
                browser_path,
                data_dir(),
            )
        });
    assert!(harness.state().image.is_none());
    Session { harness, directory }
}

fn stage_export(h: &mut Harness<'_, App>, destination: &Path) -> Arc<ExportPause> {
    let (staged, ready) = mpsc::channel();
    let pause = Arc::new(ExportPause {
        staged,
        released: (Mutex::new(false), Condvar::new()),
        cancelled: (Mutex::new(false), Condvar::new()),
    });
    let ctx = h.ctx.clone();
    let options = h.state().export_options.clone();
    h.state_mut()
        .start_export_to_path_inner(&ctx, options, destination, Some(pause.clone()));
    ready
        .recv_timeout(WAIT_DEADLINE)
        .expect("real export writer did not produce staged output");
    h.run_steps(3);
    pause
}

#[test]
fn guard_statuses_are_exact_and_unknown_save_extension_creates_no_file() {
    let mut empty = empty_session_for_guards();
    let h = &mut empty.harness;
    h.state_mut().save_dialog();
    assert_eq!(
        h.state().status,
        "Nothing to save yet — load an image first."
    );
    click(h, "cw rotate");
    assert_eq!(h.state().status, "Load an image before rotating.");
    let ctx = h.ctx.clone();
    let options = h.state().export_options.clone();
    h.state_mut().start_export(&ctx, options);
    assert_eq!(h.state().status, "Load an image before exporting.");

    let mut s = session();
    let h = &mut s.harness;
    let destination = s.directory.path().join("paused-export.png");
    let pause = stage_export(h, &destination);
    let ctx = h.ctx.clone();
    let options = h.state().export_options.clone();
    h.state_mut().start_export(&ctx, options);
    assert_eq!(
        h.state().status,
        "Wait for the current operation before exporting."
    );
    assert!(h.state().export_job.is_some());
    pause.release();
    wait_until(h, |app| app.export_job.is_none());
    assert!(destination.is_file());

    render(h, "PREVIEW");
    let unsupported = s.directory.path().join("unsupported-container.xyz");
    script(h, chooser::ChooserResult::Scripted(unsupported.clone()));
    click(h, "SAVE");
    assert_eq!(
        h.state().status,
        "Save error: Unsupported image extension 'xyz'; use JPEG, PNG, TIFF or EXR"
    );
    assert!(!unsupported.exists());
}
