use super::*;
use crate::export_job::ExportPause;
use parking_lot::{Condvar, Mutex};
use spektrafilm_core::presets::{Preset, PresetLibrary, Provenance};
use std::sync::mpsc;

fn expand_preset_library(h: &mut Harness<'_, App>) {
    if h.query_by_label("▶").is_some() {
        click(h, "▶");
    }
    scroll_preset_into_view(h, "Refresh");
    preset_pointer_click(h, "Refresh", egui::PointerButton::Primary);
}

/// Scroll the preset browser itself, not the independent control inspector.
fn scroll_preset_into_view(h: &mut Harness<'_, App>, label: &str) {
    for _ in 0..32 {
        let search = h.get_by_label("Search presets").raw_bounds().unwrap();
        let filters = h.get_by_label("Recently used").raw_bounds().unwrap();
        let top = filters.y1 + 8.0;
        let bottom = h.ctx.screen_rect().bottom() as f64 - 8.0;
        let bounds = h.get_by_label(label).raw_bounds().unwrap();
        let delta = if bounds.y0 < top {
            top - bounds.y0 + 12.0
        } else if bounds.y1 > bottom {
            bottom - bounds.y1 - 12.0
        } else {
            return;
        };
        h.input_mut()
            .events
            .push(egui::Event::PointerMoved(egui::pos2(
                ((search.x0 + search.x1) * 0.5) as f32,
                ((top + bottom) * 0.5) as f32,
            )));
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta.clamp(-350.0, 350.0) as f32),
            modifiers: Default::default(),
        });
        h.run_steps(10);
    }
    panic!("preset target did not enter the browser viewport: {label}");
}

fn context_click(h: &mut Harness<'_, App>, label: &str) {
    scroll_preset_into_view(h, label);
    preset_pointer_click(h, label, egui::PointerButton::Secondary);
}

fn preset_pointer_click(h: &mut Harness<'_, App>, label: &str, button: egui::PointerButton) {
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
            button,
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
        .query_all_by_role(egui::accesskit::Role::Button)
        .filter(|node| node.label().is_some_and(|label| label.starts_with(name)))
        .collect();
    assert_eq!(nodes.len(), 1, "preset row for {name}");
    nodes[0].label().unwrap().to_string()
}

fn assert_preset_marker(h: &Harness<'_, App>, expected: presets::Marker) {
    let app = h.state();
    assert_eq!(
        presets::marker(
            app.presets.state.applied.as_ref().expect("applied look"),
            &app.presets.entries,
            &app.params,
            &app.film_name,
            &app.print_name,
        ),
        expected
    );
}

fn assert_quality_disabled(h: &Harness<'_, App>) {
    assert!(
        h.get_by_role_and_label(egui::accesskit::Role::Slider, "JPEG quality")
            .node()
            .is_disabled()
    );
    assert!(
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "JPEG subsampling")
            .node()
            .is_disabled()
    );
}
fn assert_quality_controls(h: &Harness<'_, App>) {
    assert!(
        !h.get_by_role_and_label(egui::accesskit::Role::Slider, "JPEG quality")
            .node()
            .is_disabled()
    );
    assert!(
        !h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "JPEG subsampling")
            .node()
            .is_disabled()
    );
}

fn choose_diagnostics_mode(h: &mut Harness<'_, App>, next: &str) {
    reveal_control(h, "action.save_report");
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
    scroll_controls_into_view(h, label);
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
    let invalid_name = "bad/name";
    edit_preset_name(h, invalid_name);
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
    assert_preset_marker(h, presets::Marker::Applied);

    reveal_control(h, "output_mix.amount");
    edit(h, "Simulation blend", "0.25");
    assert!((h.state().params.output_mix.amount - 0.25).abs() < 1e-6);
    let row = preset_row(h, "Dialog Look");
    assert_preset_marker(h, presets::Marker::Modified);
    click_preset_action(h, &row, "Revert");
    assert_preset_marker(h, presets::Marker::Applied);
    assert!((h.state().params.output_mix.amount - 1.0).abs() < 1e-6);

    let row = preset_row(h, "Dialog Look");
    click_preset_action(h, &row, "Rename…");
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
    assert_preset_marker(h, presets::Marker::Applied);

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
    click(h, "Delete");
    assert!(!duplicate.exists());

    let external = s.directory.path().join("external-look.toml");
    external_look(h, "Renamed Look", &external);
    script(h, chooser::ChooserResult::Scripted(external.clone()));
    scroll_preset_into_view(h, "Import");
    preset_pointer_click(h, "Import", egui::PointerButton::Primary);
    assert!(h.state().presets.name_dialog.is_some());
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
}

#[test]
fn preset_import_rejects_section_documents_without_writing_or_editing() {
    let mut s = session();
    let h = &mut s.harness;
    expand_preset_library(h);
    let preset = Preset::capture_section(
        "Section only",
        spektrafilm_core::presets::Section::OutputMix,
        &h.state().params,
        preset_provenance(),
    )
    .unwrap();
    let external = s.directory.path().join("section.json");
    std::fs::write(&external, preset.to_json().unwrap()).unwrap();
    let before = h.state().params.clone();
    let revision = h.state().parameter_revision;
    script(h, chooser::ChooserResult::Scripted(external.clone()));
    scroll_preset_into_view(h, "Import");
    preset_pointer_click(h, "Import", egui::PointerButton::Primary);
    assert!(h.state().status.localized(&h.ctx).contains("look"));
    edit_preset_name(h, "Renamed section");
    click(h, "Save");
    assert!(h.state().presets.name_dialog.is_some());
    let library = h.state().config_directory.join("presets");
    for name in ["Section only", "Renamed section"] {
        for extension in ["toml", "json"] {
            assert!(!library.join(format!("{name}.{extension}")).exists());
        }
    }
    assert_eq!(h.state().params, before);
    assert_eq!(h.state().parameter_revision, revision);
    assert_eq!(
        Preset::from_json_str(&std::fs::read_to_string(external).unwrap()).unwrap(),
        preset
    );
}

// specs/gui-product-design/spec.md#preset-browser: the card carries a visible
// action control, so the management actions are reachable without the context
// menu, and the control takes focus so a keyboard activates it.
#[test]
fn preset_card_actions_are_reachable_by_pointer_and_by_keyboard() {
    let mut s = session();
    let h = &mut s.harness;
    expand_preset_library(h);
    click(h, "Save preset…");
    edit_preset_name(h, "Card Look");
    click(h, "Save");
    assert_preset_marker(h, presets::Marker::Applied);

    // The card's own action control is addressable per entry, so the context
    // menu is not the only route to the actions.
    let actions = "preset actions for Card Look";
    scroll_preset_into_view(h, actions);
    preset_pointer_click(h, actions, egui::PointerButton::Primary);
    preset_pointer_click(h, "Rename…", egui::PointerButton::Primary);
    assert!(h.state().presets.name_dialog.is_some());
    click(h, "Cancel");
    h.press_key(egui::Key::Escape);
    h.run_steps(3);

    // Keyboard route: focus the control, then activate it.
    h.get_by_role_and_label(egui::accesskit::Role::Button, actions)
        .focus();
    h.run_steps(10);
    h.press_key(egui::Key::Space);
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Duplicate…")
        .focus();
    h.step();
    h.press_key(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().presets.name_dialog.is_some());
    h.press_key(egui::Key::Escape);
    h.run_steps(3);
    assert!(
        h.get_by_role_and_label(egui::accesskit::Role::Button, actions)
            .node()
            .is_focused(),
        "closing the dialog returns focus to its persistent card action control"
    );

    // The context menu remains an additional route.
    click_preset_action(h, &preset_row(h, "Card Look"), "Rename…");
    assert!(h.state().presets.name_dialog.is_some());
}

fn open_export_dialog(h: &mut Harness<'_, App>) {
    open_export(h);
    assert!(h.state().export_dialog.is_open());
}

fn change_export_format(h: &mut Harness<'_, App>, next: &str) {
    choose(h, "Format", next);
}

/// Move the Export draft away from the committed baseline.
fn edit_export_draft(h: &mut Harness<'_, App>) {
    choose(h, "Backend", export::ExportBackend::Cpu.label());
    choose(h, "Format", "PNG");
    choose(h, "Saving color space", "Display P3");
}

#[test]
fn export_dialog_format_controls_are_conditional_and_both_cancel_paths_write_nothing() {
    let mut s = session();
    let h = &mut s.harness;
    let committed = h.state().export_options.clone();
    open_export_dialog(h);

    change_export_format(h, "JPEG");
    assert_quality_controls(h);
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "JPEG subsampling")
            .is_some()
    );
    assert!(
        h.get_by_label("Apply saving CCTF encoding")
            .node()
            .is_disabled()
    );
    choose(h, "JPEG subsampling", "4:2:0");

    change_export_format(h, "TIFF");
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "Compression")
            .is_some()
    );
    assert_quality_disabled(h);
    assert!(
        !h.get_by_label("Apply saving CCTF encoding")
            .node()
            .is_disabled()
    );
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
    assert_quality_disabled(h);
    assert!(
        h.get_by_label("Apply saving CCTF encoding")
            .node()
            .is_disabled()
    );
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
    assert_eq!(h.state().export_options.format, committed.format);
    assert_eq!(
        h.state().export_options.jpeg_quality,
        committed.jpeg_quality
    );
    assert_eq!(
        h.state().export_options.jpeg_subsampling,
        committed.jpeg_subsampling
    );
    assert_eq!(
        h.state().export_options.saving_cctf_encoding,
        committed.saving_cctf_encoding
    );

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

#[test]
fn export_draft_edits_are_local_cancelable_and_commit_normalized_without_adjustment_history() {
    let mut s = session();
    let h = &mut s.harness;
    let mut custom = h.state().export_options.clone();
    custom.backend = export::ExportBackend::Gpu;
    custom.format = export::ExportFormat::Tiff;
    custom.depth = BitDepth::ThirtyTwo;
    custom.jpeg_quality = 61;
    custom.jpeg_subsampling = image_io::JpegSubsampling::Yuv420;
    custom.compression = export::ExportCompression::None;
    custom.saving_color_space = "ACES2065-1".into();
    custom.saving_cctf_encoding = false;
    h.state_mut().export_options = custom.clone();
    let params = h.state().params.clone();
    let could_undo = h.state().can_undo();
    let could_redo = h.state().can_redo();

    // Draft edits stay in the dialog: they are not preferences until Export.
    open_export_dialog(h);
    edit_export_draft(h);
    assert_eq!(
        h.state().export_options,
        custom,
        "draft edits do not commit preferences"
    );
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().can_undo(), could_undo);
    assert_eq!(h.state().can_redo(), could_redo);
    click(h, "Cancel");
    assert_eq!(
        h.state().export_options,
        custom,
        "cancel preserves the committed preferences"
    );
    assert!(h.state().export_job.is_none());

    // Export commits the edited draft after the dialog normalizes it.
    let destination = s.directory.path().join("draft-normalized-export.png");
    open_export_dialog(h);
    edit_export_draft(h);
    script(h, chooser::Chooser::chooser_path(&destination));
    export_action(h, "Export file…");
    wait_until(h, |app| app.export_job.is_none() && destination.is_file());
    let mut expected = custom.clone();
    expected.backend = export::ExportBackend::Cpu;
    expected.format = export::ExportFormat::Png;
    expected.saving_color_space = "Display P3".into();
    expected.normalize();
    assert_eq!(
        &h.state().export_options,
        &expected,
        "export commits the normalized draft"
    );
    // PNG requires 8-bit encoded output even after a linear 32-bit TIFF draft.
    assert_eq!(h.state().export_options.depth, BitDepth::Eight);
    assert!(h.state().export_options.saving_cctf_encoding);
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().can_undo(), could_undo);
    assert_eq!(h.state().can_redo(), could_redo);
}

#[test]
fn cancelled_export_chooser_and_rejected_destination_do_not_commit_draft_preferences() {
    let mut s = session();
    let h = &mut s.harness;
    let committed = h.state().export_options.clone();
    let params = h.state().params.clone();
    let could_undo = h.state().can_undo();
    let could_redo = h.state().can_redo();
    open_export(h);
    choose(h, "Format", "TIFF");
    choose(h, "Bit depth", "32 bit");
    script(h, chooser::ChooserResult::Canceled);
    export_action(h, "Export file…");
    assert!(!h.state().export_dialog.is_open());
    assert!(h.state().export_job.is_none());
    assert_eq!(h.state().export_options, committed);

    let existing = s.directory.path().join("rejected-destination.tiff");
    let original = b"existing user file";
    std::fs::write(&existing, original).unwrap();
    open_export(h);
    choose(h, "Format", "TIFF");
    choose(h, "Bit depth", "32 bit");
    script(
        h,
        chooser::Chooser::chooser_path(&s.directory.path().join("rejected-destination.wrong")),
    );
    export_action(h, "Export file…");
    assert!(h.state().export_job.is_none());
    assert_eq!(h.state().export_options, committed);
    assert_eq!(std::fs::read(&existing).unwrap(), original);
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().can_undo(), could_undo);
    assert_eq!(h.state().can_redo(), could_redo);
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
    await_input(h);

    assert_eq!(h.state().image.as_ref().unwrap().width(), 320);
    assert!(h.state().export_dialog.is_open());
    let replaced_with = estimated_output_size_row(h);
    assert_ne!(
        replaced_with, opened_with,
        "the estimate must describe the input the export now renders"
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
    assert!(path.is_file());
}

#[test]
fn diagnostics_gui_modes_capture_reports_select_saved_reports_and_cancel_save() {
    let mut s = session();
    let h = &mut s.harness;
    preview(h);
    let published = s.directory.path().join("published-reports");
    std::fs::create_dir(&published).unwrap();
    h.state_mut()
        .diagnostics
        .publish_completed_to(published.clone());

    open_settings(h);
    choose_diagnostics_mode(h, "Summary");
    close_settings(h);
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "exposure compensation ev", "0.25");
    preview(h);
    open_settings(h);
    let first_reports = report_paths(&published);
    assert_eq!(first_reports.len(), 1);
    let first = first_reports
        .iter()
        .map(|path| read_report(path))
        .find(|report| {
            report["operation"]["kind"] == "preview"
                && report["configuration"]["collection_mode_requested"] == "summary"
        })
        .expect("Summary report for the automatic Preview");
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
    close_settings(h);
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "exposure compensation ev", "0.5");
    preview(h);
    open_settings(h);
    let reports = report_paths(&published);
    assert_eq!(reports.len(), 2);
    let gpu_report_path = reports
        .iter()
        .find(|path| {
            let report = read_report(path);
            report["operation"]["kind"] == "preview"
                && report["configuration"]["collection_mode_requested"] == "gpu_timing"
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
    let selected_summary_path = s.directory.path().join("selected-summary-report.json");
    save_report_from_panel(h, &selected_summary_path);
    let selected = read_report(&selected_summary_path);
    assert_eq!(selected["operation"]["id"], first_id);
    assert_eq!(
        selected["configuration"]["collection_mode_requested"],
        "summary"
    );

    choose_diagnostics_mode(h, "Off");
    close_settings(h);
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "exposure compensation ev", "0.75");
    preview(h);
    open_settings(h);
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
    preview(h);
    let published = s.directory.path().join("published-reports");
    std::fs::create_dir(&published).unwrap();
    h.state_mut()
        .diagnostics
        .publish_completed_to(published.clone());

    open_settings(h);
    choose_diagnostics_mode(h, "Summary");
    close_settings(h);
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "exposure compensation ev", "0.25");
    preview(h);
    open_settings(h);

    let reports = report_paths(&published);
    assert_eq!(reports.len(), 1);
    let first_id = read_report(&reports[0])["operation"]["id"]
        .as_u64()
        .unwrap();

    // The single report is auto-selected.
    assert_eq!(h.state().diagnostics.selected_report_id(), Some(first_id));

    // Simulate the user clicking the older report row (sets user_selected = true).
    h.get_by_label(&format!("#{} {} · {}", first_id, "preview", "succeeded"))
        .click();
    h.run_steps(2);
    assert_eq!(
        h.state().diagnostics.selected_report_id(),
        Some(first_id),
        "user click must preserve the selection"
    );

    // Trigger another render; the user's selection must survive.
    close_settings(h);
    edit(h, "exposure compensation ev", "0.5");
    preview(h);
    open_settings(h);
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
    await_input(h);
}

#[test]
fn dropped_png_uses_the_import_path_and_bad_drop_preserves_accepted_images() {
    let mut s = session();
    let h = &mut s.harness;
    let dropped = s.directory.path().join("dropped.png");
    save_test_png(&dropped, 7, 3, 0.4);
    drop_file(h, &dropped);
    let expected = image_io::load(&dropped).unwrap().image;
    let loaded = h.state().image.as_ref().unwrap().host_image().unwrap();
    assert_eq!(h.state().image_path.as_ref(), Some(&dropped));
    assert_eq!((loaded.width, loaded.height), (7, 3));
    assert_eq!(loaded.data, expected.data);
    let accepted_output = preview(h);
    assert_eq!((accepted_output.width, accepted_output.height), (7, 3));

    let mut imported = session();
    script(
        &mut imported.harness,
        chooser::ChooserResult::Scripted(dropped.clone()),
    );
    import_file(&mut imported.harness, false);
    assert_eq!(
        h.state().image.as_ref().unwrap().host_image().unwrap().data,
        imported
            .harness
            .state()
            .image
            .as_ref()
            .unwrap()
            .host_image()
            .unwrap()
            .data
    );
    assert_eq!(h.state().params, imported.harness.state().params);
    assert_eq!(h.state().image_path, imported.harness.state().image_path);
    assert_eq!(h.state().input_epoch, imported.harness.state().input_epoch);

    let retained_image = h.state().image.as_ref().unwrap().clone();
    let retained_path = h.state().image_path.clone();
    let params = h.state().params.clone();
    let revision = h.state().parameter_revision;
    let history = (h.state().can_undo(), h.state().can_redo());
    let corrupt = s.directory.path().join("dropped-corrupt.png");
    std::fs::write(&corrupt, b"not an image").unwrap();
    drop_file(h, &corrupt);
    assert_eq!(h.state().image_path, retained_path);
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().parameter_revision, revision);
    assert_eq!((h.state().can_undo(), h.state().can_redo()), history);
    assert!(
        h.state()
            .image
            .as_ref()
            .unwrap()
            .same_source(&retained_image)
    );
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &accepted_output
    ));
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
    let initial =
        state::GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
            .unwrap();
    let state_path = directory.path().join("initial.json");
    initial.save(&state_path).unwrap();
    let browser_path = directory.path().join("browser.json");
    let harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 1400.0))
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
    assert!(harness.state().image.is_none());
    Session { harness, directory }
}

pub(super) fn stage_export(h: &mut Harness<'_, App>, destination: &Path) -> Arc<ExportPause> {
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
fn guarded_operations_and_unknown_save_extension_create_no_file() {
    let mut empty = empty_session_for_guards();
    let h = &mut empty.harness;
    click(h, "SAVE");
    assert!(h.state().output_image.is_none());
    reveal_control(h, "action.rotate_cw");
    assert!(h.get_by_label("cw rotate").node().is_disabled());
    let image_epoch = h.state().input_epoch;
    click(h, "cw rotate");
    assert_eq!(h.state().input_epoch, image_epoch);
    assert!(h.get_by_label("Export").node().is_disabled());
    let ctx = h.ctx.clone();
    let options = h.state().export_options.clone();
    h.state_mut().start_export(&ctx, options);
    assert!(h.state().export_job.is_none());

    let mut s = session();
    let h = &mut s.harness;
    let destination = s.directory.path().join("paused-export.png");
    let pause = stage_export(h, &destination);
    let active_cancel = h.state().export_job.as_ref().unwrap().cancel.clone();
    let ctx = h.ctx.clone();
    let options = h.state().export_options.clone();
    h.state_mut().start_export(&ctx, options);
    assert!(h.state().export_job.is_some());
    assert!(Arc::ptr_eq(
        &h.state().export_job.as_ref().unwrap().cancel,
        &active_cancel
    ));
    pause.release();
    wait_until(h, |app| app.export_job.is_none());
    assert!(destination.is_file());

    preview(h);
    let unsupported = s.directory.path().join("unsupported-container.xyz");
    script(h, chooser::ChooserResult::Scripted(unsupported.clone()));
    click(h, "SAVE");
    assert!(matches!(
        &h.state().status,
        crate::app::Message::Text {
            key: "operations.save_error",
            ..
        }
    ));
    assert!(!unsupported.exists());
}
