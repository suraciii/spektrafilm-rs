use super::*;
use crate::state;
fn test_app(label: &str) -> (App, PathBuf) {
    let directory =
        std::env::temp_dir().join(format!("spektrafilm-gui-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let state_path = directory.join("state.json");
    state::GuiState::factory().save(&state_path).unwrap();
    let mut app = App::new_with_browser_path(
        std::sync::Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend),
        None,
        Some(state_path),
        None,
        directory.join("preset_browser_state.json"),
        crate::headless::data_dir(),
    );
    app.presets.library = PresetLibrary::new(directory.join("presets"));
    app.presets.refresh();
    (app, directory)
}

#[test]
fn failed_look_and_section_preserve_entire_application_transaction() {
    let (mut app, directory) = test_app("failed-transaction");
    app.params.output_mix.amount = 0.25;
    let mut look = builtin_presets().remove(0);
    look.name = "Missing profile".into();
    look.film_profile.as_mut().unwrap().stock = "missing_profile".into();
    let section = Preset::from_json_str(r#"{"schema_version":2,"name":"Incompatible scan","parameters":{"scanner":{"scan_output":"negative_to_positive"}},"provenance":{"implementation_version":"test","model_version":"test"}}"#).unwrap();
    std::fs::create_dir_all(directory.join("presets")).unwrap();
    for preset in [look, section] {
        std::fs::write(
            directory
                .join("presets")
                .join(format!("{}.json", preset.name)),
            preset.to_json().unwrap(),
        )
        .unwrap();
    }
    app.presets.refresh();
    app.presets.state.applied = Some(AppliedPreset {
        group: PresetGroup::BuiltIn,
        name: "Remembered".into(),
    });
    app.params.scanner.white_correction = false;
    app.params.scanner.black_correction = false;
    app.params.print_render.glare.active = true;
    app.toggle_scan_for_print();
    app.dirty_since = Some(std::time::Instant::now());
    for name in ["Missing profile", "Incompatible scan"] {
        let entry = app
            .presets
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap()
            .clone();
        let before = serde_json::to_value(&app.params).unwrap();
        let state = app.current_state().unwrap().sections;
        let browser = app.presets.state.clone();
        let film = app.film_name.clone();
        let print = app.print_name.clone();
        let film_times = app.film_dev_times.clone();
        let print_times = app.print_dev_times.clone();
        let snapshot = app.scan_for_print_snapshot;
        let revision = app.parameter_revision;
        let epoch = app.calibration_epoch;
        let dirty = app.dirty;
        let dirty_since = app.dirty_since;
        let force_preview = app.force_preview;
        app.load_preset(entry.group, &entry.name);
        assert!(app.status.starts_with("Preset error:"), "{}", app.status);
        assert_eq!(serde_json::to_value(&app.params).unwrap(), before);
        assert_eq!(app.current_state().unwrap().sections, state);
        assert_eq!(app.presets.state, browser);
        assert_eq!(app.film_name, film);
        assert_eq!(app.print_name, print);
        assert_eq!(app.film_dev_times, film_times);
        assert_eq!(app.print_dev_times, print_times);
        assert_eq!(app.scan_for_print_snapshot, snapshot);
        assert_eq!(app.parameter_revision, revision);
        assert_eq!(app.calibration_epoch, epoch);
        assert_eq!(app.dirty, dirty);
        assert_eq!(app.dirty_since, dirty_since);
        assert_eq!(app.params.output_mix.amount, 0.25);
        assert_eq!(app.force_preview, force_preview);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn ordinary_restart_restores_browser_only_and_explicit_state_wins() {
    let (mut app, directory) = test_app("browser-restart");
    app.params.enlarger.print_exposure = 7.123;
    let startup = app.current_state().unwrap();
    startup
        .save(&directory.join("gui_default_state.json"))
        .unwrap();
    let entry = app
        .presets
        .entries
        .iter()
        .find(|entry| entry.group == PresetGroup::BuiltIn)
        .unwrap()
        .clone();
    app.load_preset(entry.group, &entry.name);
    assert!(app.status.starts_with("Applied preset"), "{}", app.status);
    app.params.enlarger.print_exposure += 2.0;
    app.presets.state.width = 327.0;
    app.presets.state.expanded = false;
    let remembered = app.presets.state.clone();
    app.persist_browser_state(true);
    let carrier: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&app.browser_state_path).unwrap()).unwrap();
    assert!(carrier["rust"]["presets"].get("modified").is_none());
    assert!(carrier["rust"].get("runtime").is_none());
    let reopen = |initial_state| {
        App::new_with_browser_path(
            std::sync::Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend),
            None,
            initial_state,
            None,
            directory.join("preset_browser_state.json"),
            crate::headless::data_dir(),
        )
    };
    let mut restarted = reopen(None);
    assert_eq!(
        serde_json::to_value(&restarted.params).unwrap(),
        serde_json::to_value(startup.runtime_params().unwrap()).unwrap()
    );
    assert_eq!(restarted.film_name, startup.film());
    assert_eq!(restarted.print_name, startup.paper());
    assert_eq!(restarted.presets.state, remembered);
    assert!(!restarted.dirty);
    assert_eq!(restarted.parameter_revision, 0);
    assert!(restarted.scan_for_print_snapshot.is_none());
    assert_eq!(
        marker(
            remembered.applied.as_ref().unwrap(),
            &restarted.presets.entries,
            &restarted.params,
            &restarted.film_name,
            &restarted.print_name
        ),
        Marker::Modified
    );
    let mut explicit = state::GuiState::factory();
    explicit.sections["rust"] = serde_json::json!({"version":1,"presets":{"width":211.0,"expanded":true,"applied":{"group":PresetGroup::User,"name":"Absent"}}});
    let explicit_path = directory.join("explicit.json");
    explicit.save(&explicit_path).unwrap();
    let explicit_app = reopen(Some(explicit_path));
    assert_eq!(
        BrowserState::restore(
            &state::GuiState::factory().sections,
            true,
            &app.browser_state_path
        )
        .unwrap(),
        BrowserState::default()
    );
    assert_eq!(explicit_app.presets.state.width, 211.0);
    assert!(explicit_app.presets.state.expanded);
    assert_eq!(
        marker(
            explicit_app.presets.state.applied.as_ref().unwrap(),
            &explicit_app.presets.entries,
            &explicit_app.params,
            &explicit_app.film_name,
            &explicit_app.print_name
        ),
        Marker::Missing
    );
    restarted.apply_state(explicit.clone()).unwrap();
    restarted.persist_browser_state(true);
    assert_eq!(reopen(None).presets.state, explicit_app.presets.state);
    restarted.apply_state(state::GuiState::factory()).unwrap();
    restarted.persist_browser_state(true);
    assert_eq!(reopen(None).presets.state, BrowserState::default());
    std::fs::remove_file(directory.join("preset_browser_state.json")).unwrap();
    assert_eq!(reopen(None).presets.state, BrowserState::default());
    std::fs::write(directory.join("preset_browser_state.json"), "broken JSON").unwrap();
    assert!(reopen(None).status.starts_with("Browser state load error:"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn browser_file_errors_are_visible_and_explicit_state_ignores_bad_carrier() {
    let (mut app, directory) = test_app("browser-errors");
    std::fs::write(&app.browser_state_path, "broken JSON").unwrap();
    assert!(
        BrowserState::restore(&app.gui_state.sections, false, &app.browser_state_path).is_err()
    );
    assert_eq!(
        BrowserState::restore(&app.gui_state.sections, true, &app.browser_state_path).unwrap(),
        BrowserState::default()
    );
    let blocked = directory.join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    app.browser_state_path = blocked.join("browser.json");
    app.presets.state.width += 1.0;
    app.persist_browser_state(false);
    assert!(app.browser_state_changed_at.is_some());
    app.persist_browser_state(true);
    assert!(
        app.status.starts_with("Browser state save error:"),
        "{}",
        app.status
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn scanner_transitions_preserve_points_and_reset_illegal_output() {
    let (mut app, directory) = test_app("scanner-transitions");
    app.params.scanner.black_point = [0.1, 0.2, 0.3];
    app.params.scanner.white_point = [0.7, 0.8, 0.9];
    app.params.workflow.route = "input > film > scan".into();
    app.params.scanner.scan_output = "negative_to_positive".into();
    app.params.scanner.receiver_profile = spektrafilm_core::params::ReceiverProfile::LabScanApprox;
    app.params.scanner.negative_conversion_method =
        spektrafilm_core::params::NegativeConversionMethod::Coupled;
    app.params.scanner.auto_setup = spektrafilm_core::params::ScannerAutoSetup::PerFrame;
    app.params.scanner.finish = spektrafilm_core::params::ScannerFinish::LabScan;
    app.dirty = false;
    let revision = app.parameter_revision;
    app.params.workflow.route = "input > film > print > scan".into();
    app.finish_route_change();
    assert_eq!(app.params.scanner.scan_output, "direct_scan");
    assert_eq!(
        app.params.scanner.receiver_profile,
        spektrafilm_core::params::ReceiverProfile::LegacyColorimetric
    );
    assert_eq!(
        app.params.scanner.negative_conversion_method,
        spektrafilm_core::params::NegativeConversionMethod::Density
    );
    assert_eq!(
        app.params.scanner.auto_setup,
        spektrafilm_core::params::ScannerAutoSetup::Off
    );
    assert_eq!(
        app.params.scanner.finish,
        spektrafilm_core::params::ScannerFinish::Neutral
    );
    assert!(app.dirty);
    assert_eq!(app.parameter_revision, revision.wrapping_add(1));
    app.params.workflow.route = "input > convert-film > scan".into();
    app.finish_route_change();
    app.params.scanner.scan_output = "negative_to_positive".into();
    app.params.scanner.white_correction = false;
    app.params.scanner.black_correction = false;
    let revision = app.parameter_revision;
    app.toggle_scan_for_print();
    assert!(app.scan_for_print_snapshot.is_none());
    assert!(!app.params.scanner.white_correction);
    assert!(!app.params.scanner.black_correction);
    assert_eq!(app.parameter_revision, revision);
    app.params.scanner.receiver_profile = spektrafilm_core::params::ReceiverProfile::LabScanApprox;
    app.params.scanner.negative_conversion_method =
        spektrafilm_core::params::NegativeConversionMethod::Coupled;
    app.params.scanner.auto_setup = spektrafilm_core::params::ScannerAutoSetup::PerFrame;
    app.params.scanner.finish = spektrafilm_core::params::ScannerFinish::LabScan;
    app.select_film_profile("kodak_ektachrome_100".into());
    assert_eq!(app.film_medium, crate::controls::ScanMedium::Slide);
    assert_eq!(app.params.scanner.scan_output, "direct_scan");
    assert_eq!(
        app.params.scanner.receiver_profile,
        spektrafilm_core::params::ReceiverProfile::LabScanApprox
    );
    assert_eq!(
        app.params.scanner.negative_conversion_method,
        spektrafilm_core::params::NegativeConversionMethod::Density
    );
    assert_eq!(
        app.params.scanner.auto_setup,
        spektrafilm_core::params::ScannerAutoSetup::Off
    );
    assert_eq!(
        app.params.scanner.finish,
        spektrafilm_core::params::ScannerFinish::Neutral
    );
    let restored = app.current_state().unwrap().runtime_params().unwrap();
    assert_eq!(restored.scanner.black_point, [0.1, 0.2, 0.3]);
    assert_eq!(restored.scanner.white_point, [0.7, 0.8, 0.9]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn slide_startup_rejects_positive_output_with_visible_error() {
    let (app, directory) = test_app("slide-startup");
    let mut params = app.params.clone();
    params.workflow.route = "input > film > scan".into();
    params.scanner.scan_output = "negative_to_positive".into();
    params.scanner.white_correction = false;
    params.scanner.black_correction = false;
    params.scanner.black_point = [0.1, 0.2, 0.3];
    let state = state::GuiState::from_runtime(
        &params,
        "kodak_ektachrome_100",
        &app.print_name,
        &app.gui_state.sections,
    )
    .unwrap();
    let path = directory.join("slide.json");
    state.save(&path).unwrap();
    let restarted = App::new_with_browser_path(
        std::sync::Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend),
        None,
        Some(path),
        None,
        directory.join("preset_browser_state.json"),
        crate::headless::data_dir(),
    );
    assert!(
        restarted
            .status
            .contains("negative filming-stage film profile")
    );
    assert_eq!(
        restarted.params,
        state::GuiState::factory().runtime_params().unwrap()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn scanner_points_survive_section_preset_and_restart() {
    let (mut app, directory) = test_app("scanner-points");
    app.params.workflow.route = "input > film > scan".into();
    app.finish_route_change();
    app.params.scanner.scan_output = "negative_to_positive".into();
    app.params.scanner.receiver_profile = spektrafilm_core::params::ReceiverProfile::LabScanApprox;
    app.params.scanner.negative_conversion_method =
        spektrafilm_core::params::NegativeConversionMethod::Coupled;
    app.params.scanner.auto_setup = spektrafilm_core::params::ScannerAutoSetup::PerFrame;
    app.params.scanner.finish = spektrafilm_core::params::ScannerFinish::LabScan;
    app.params.scanner.finish_strength = 0.0;
    app.params.scanner.black_point = [0.1, 0.2, 0.3];
    app.params.scanner.white_point = [0.7, 0.8, 0.9];
    let preset =
        Preset::capture_section("Film points", Section::Scanner, &app.params, provenance())
            .unwrap();
    assert_eq!(
        serde_json::to_value(&preset).unwrap()["parameters"]["scanner"]["scan_output"],
        "negative_to_positive"
    );
    app.presets
        .library
        .save_new(&preset, &app.data_dir)
        .unwrap();
    app.presets.refresh();
    let entry = app
        .presets
        .entries
        .iter()
        .find(|entry| entry.name == "Film points")
        .unwrap()
        .clone();
    app.params.scanner.black_point = [0.0; 3];
    app.params.scanner.white_point = [1.0; 3];
    app.params.scanner.receiver_profile =
        spektrafilm_core::params::ReceiverProfile::LegacyColorimetric;
    app.params.scanner.negative_conversion_method =
        spektrafilm_core::params::NegativeConversionMethod::Separable;
    app.params.scanner.finish = spektrafilm_core::params::ScannerFinish::Neutral;
    app.params.scanner.finish_strength = 1.0;
    app.params.scanner.scan_output = "direct_scan".into();
    app.load_preset(entry.group, &entry.name);
    assert!(app.status.starts_with("Applied preset"), "{}", app.status);
    assert_eq!(app.params.scanner.scan_output, "negative_to_positive");
    assert_eq!(
        app.params.scanner.receiver_profile,
        spektrafilm_core::params::ReceiverProfile::LabScanApprox
    );
    assert_eq!(
        app.params.scanner.negative_conversion_method,
        spektrafilm_core::params::NegativeConversionMethod::Coupled
    );
    assert_eq!(
        app.params.scanner.auto_setup,
        spektrafilm_core::params::ScannerAutoSetup::PerFrame
    );
    assert_eq!(
        app.params.scanner.finish,
        spektrafilm_core::params::ScannerFinish::LabScan
    );
    assert_eq!(app.params.scanner.finish_strength, 0.0);
    let path = directory.join("saved.json");
    app.current_state().unwrap().save(&path).unwrap();
    let mut restarted = App::new_with_browser_path(
        std::sync::Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend),
        None,
        Some(path),
        None,
        directory.join("preset_browser_state.json"),
        crate::headless::data_dir(),
    );
    assert_eq!(restarted.params.scanner.scan_output, "negative_to_positive");
    assert_eq!(restarted.params.scanner.black_point, [0.1, 0.2, 0.3]);
    assert_eq!(restarted.params.scanner.white_point, [0.7, 0.8, 0.9]);
    assert_eq!(
        restarted.params.scanner.receiver_profile,
        spektrafilm_core::params::ReceiverProfile::LabScanApprox
    );
    assert_eq!(
        restarted.params.scanner.negative_conversion_method,
        spektrafilm_core::params::NegativeConversionMethod::Coupled
    );
    assert_eq!(
        restarted.params.scanner.auto_setup,
        spektrafilm_core::params::ScannerAutoSetup::PerFrame
    );
    assert_eq!(
        restarted.params.scanner.finish,
        spektrafilm_core::params::ScannerFinish::LabScan
    );
    assert_eq!(restarted.params.scanner.finish_strength, 0.0);
    restarted.params.scanner.black_point = [0.0; 3];
    restarted.apply_state(app.current_state().unwrap()).unwrap();
    assert_eq!(restarted.params.scanner.black_point, [0.1, 0.2, 0.3]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn section_application_restores_transient_scan_controls() {
    let (mut app, directory) = test_app("scan-section");
    app.params.scanner.white_correction = false;
    app.params.scanner.black_correction = false;
    app.params.print_render.glare.active = true;
    for section in [Section::Grain, Section::Base] {
        let preset =
            Preset::capture_section("Section", section, &app.params, provenance()).unwrap();
        app.presets
            .library
            .save_new(&preset, &app.data_dir)
            .unwrap();
        app.presets.refresh();
        let entry = app
            .presets
            .entries
            .iter()
            .find(|entry| entry.name == "Section")
            .unwrap()
            .clone();
        app.toggle_scan_for_print();
        assert_eq!(app.scan_for_print_snapshot, Some((false, false, true)));
        let revision = app.parameter_revision;
        let epoch = app.calibration_epoch;
        app.load_preset(entry.group, &entry.name);
        assert!(app.status.starts_with("Applied preset"), "{}", app.status);
        assert!(app.scan_for_print_snapshot.is_none());
        assert_eq!(app.parameter_revision, revision.wrapping_add(1));
        assert_eq!(app.calibration_epoch, epoch.wrapping_add(1));
        assert!(app.dirty);
        let restored = app.current_state().unwrap().runtime_params().unwrap();
        assert!(!restored.scanner.white_correction);
        assert!(!restored.scanner.black_correction);
        assert!(restored.print_render.glare.active);
        app.presets.library.delete("Section").unwrap();
    }
    let entry = app
        .presets
        .entries
        .iter()
        .find(|entry| entry.group == PresetGroup::BuiltIn)
        .unwrap()
        .clone();
    app.toggle_scan_for_print();
    app.load_preset(entry.group, &entry.name);
    assert!(app.status.starts_with("Applied preset"), "{}", app.status);
    assert!(app.scan_for_print_snapshot.is_none());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unusable_library_path_keeps_builtins_and_renders_fault() {
    let (mut app, directory) = test_app("library-fault");
    let path = directory.join("presets");
    std::fs::write(&path, "not a directory").unwrap();
    app.presets.refresh();
    assert!(app.presets.scan_error.is_none());
    assert_eq!(
        app.presets
            .entries
            .iter()
            .filter(|entry| entry.group == PresetGroup::BuiltIn)
            .count(),
        builtin_presets().len()
    );
    let fault = app
        .presets
        .entries
        .iter()
        .find(|entry| entry.fault.is_some())
        .unwrap();
    assert_eq!(fault.path.as_ref(), Some(&path));
    let fault_text = format!("{}: {}", path.display(), fault.fault.as_ref().unwrap());
    let builtin_name = app
        .presets
        .entries
        .iter()
        .find(|entry| entry.group == PresetGroup::BuiltIn)
        .unwrap()
        .name
        .clone();
    let context = egui::Context::default();
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1460.0, 2000.0),
            )),
            ..Default::default()
        },
        |context| app.preset_panel(context),
    );
    fn texts(shape: &egui::epaint::Shape, output: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(text) => output.push(text.galley.text().to_owned()),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    texts(shape, output);
                }
            }
            _ => {}
        }
    }
    let mut rendered = vec![];
    for shape in output.shapes {
        texts(&shape.shape, &mut rendered);
    }
    assert!(rendered.contains(&fault_text), "{rendered:?}");
    assert!(
        rendered.iter().any(|text| text.contains(&builtin_name)),
        "{rendered:?}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
use spektrafilm_core::presets::builtin_presets;
#[test]
fn marker_transitions_and_capture_roundtrip() {
    let p = Preset::capture(
        "Exposure marker",
        "kodak_gold_200",
        "kodak_portra_endura",
        &RuntimeParams::default(),
        provenance(),
    );
    let mut params = RuntimeParams::default();
    let look = p.look_parameters().unwrap();
    look.apply_to(&mut params);
    let film = &p.film_profile.as_ref().unwrap().stock;
    let print = &p.print_profile.as_ref().unwrap().stock;
    let entries = vec![LibraryEntry {
        group: PresetGroup::BuiltIn,
        name: p.name.clone(),
        kind: Some(PresetKind::Look),
        film_profile: p.film_profile.clone(),
        print_profile: p.print_profile.clone(),
        path: None,
        fault: None,
        preset: Some(p.clone()),
    }];
    let applied = AppliedPreset {
        group: PresetGroup::BuiltIn,
        name: p.name.clone(),
    };
    assert_eq!(
        marker(&applied, &entries, &params, film, print),
        Marker::Applied
    );
    assert_eq!(
        Preset::capture("Roundtrip", film, print, &params, provenance()).look_parameters(),
        p.look_parameters()
    );
    params.io.crop = true;
    assert_eq!(
        marker(&applied, &entries, &params, film, print),
        Marker::Applied
    );
    params.camera.exposure_compensation_ev += 0.5;
    assert_eq!(
        marker(&applied, &entries, &params, film, print),
        Marker::Modified
    );
    look.apply_to(&mut params);
    assert_eq!(
        marker(&applied, &entries, &params, film, print),
        Marker::Applied
    );
    params.camera.auto_exposure = !params.camera.auto_exposure;
    assert_eq!(
        marker(&applied, &entries, &params, film, print),
        Marker::Modified
    );
    look.apply_to(&mut params);
    params.camera.auto_exposure_method = "average".into();
    assert_eq!(
        marker(&applied, &entries, &params, film, print),
        Marker::Modified
    );
    look.apply_to(&mut params);
    params.enlarger.print_exposure += 1.0;
    assert_eq!(
        marker(&applied, &entries, &params, film, print),
        Marker::Modified
    );
    assert_eq!(marker(&applied, &[], &params, film, print), Marker::Missing);
}
#[test]
fn section_labels_and_names() {
    let params = RuntimeParams::default();
    let mut entries = vec![];
    for name in ["Zulu", "alpha"] {
        let preset = Preset::capture_section(name, Section::Grain, &params, provenance()).unwrap();
        entries.push(LibraryEntry {
            group: PresetGroup::User,
            name: name.into(),
            kind: Some(PresetKind::Section(Section::Grain)),
            film_profile: None,
            print_profile: None,
            path: None,
            fault: None,
            preset: Some(preset),
        });
    }
    assert_eq!(
        section_label(Section::Grain, &entries, &params).unwrap(),
        "alpha"
    );
    assert_eq!(
        section_label(Section::Base, &entries, &params).unwrap(),
        "custom"
    );
    assert!(name_error("ALPHA", &entries, None).is_some());
    assert!(name_error("bad/name", &entries, None).is_some());
    assert!(name_error("alpha", &entries, Some("alpha")).is_none());
}
#[test]
fn reveal_uses_platform_file_selection_arguments() {
    use std::ffi::OsString;

    let path = Path::new("/tmp/My Presets/a look.toml");
    let command = reveal_command(path);
    let (program, args) = if cfg!(target_os = "macos") {
        (
            "open",
            vec![OsString::from("-R"), path.as_os_str().to_owned()],
        )
    } else if cfg!(windows) {
        let mut argument = OsString::from("/select,");
        argument.push(path);
        ("explorer", vec![argument])
    } else {
        (
            "xdg-open",
            vec![path.parent().unwrap().as_os_str().to_owned()],
        )
    };
    assert_eq!(command.get_program(), program);
    assert_eq!(command.get_args().collect::<Vec<_>>(), args);
}
#[test]
fn browser_extension_preserves_existing_fields() {
    let mut extras = state::GuiState::factory().sections;
    extras["rust"]["presets"] = serde_json::to_value(BrowserState {
        width: 312.0,
        expanded: false,
        applied: Some(AppliedPreset {
            group: PresetGroup::User,
            name: "Remembered".into(),
        }),
    })
    .unwrap();
    extras["rust"]["viewer"] = serde_json::json!({"zoom":2});
    let s = state::GuiState::from_runtime(
        &RuntimeParams::default(),
        "kodak_gold_200",
        "kodak_portra_endura",
        &extras,
    )
    .unwrap();
    let restored = state::GuiState::from_value(s.sections.clone()).unwrap();
    let b = Browser::new(&restored.sections, PresetLibrary::default());
    assert_eq!(b.state.width, 312.0);
    assert!(!b.state.expanded);
    assert_eq!(b.state.applied.unwrap().name, "Remembered");
    assert_eq!(
        restored.sections["rust"]["viewer"],
        extras["rust"]["viewer"]
    );
    assert!(
        restored.sections["rust"]["presets"]
            .get("modified")
            .is_none()
    );
}
#[test]
fn section_explicitness_survives_state_roundtrip() {
    let mut params = RuntimeParams::default();
    params.enlarger.m_filter_neutral = 31.0;
    let preset =
        Preset::capture_section("Filters", Section::Enlarger, &params, provenance()).unwrap();
    let film = spektrafilm_core::profile::load_profile_by_name(
        &crate::headless::data_dir(),
        "kodak_gold_200",
    )
    .unwrap();
    let applied = preset
        .apply_section(&RuntimeParams::default(), &film)
        .unwrap();
    let state = state::GuiState::from_runtime(
        &applied,
        "kodak_gold_200",
        "kodak_portra_endura",
        &state::GuiState::factory().sections,
    )
    .unwrap();
    let restored = state::GuiState::from_value(state.sections)
        .unwrap()
        .runtime_params()
        .unwrap();
    assert_eq!(
        restored.render_preparation_state(),
        applied.render_preparation_state()
    );
    assert_eq!(restored.enlarger.m_filter_neutral, 31.0);
    assert_eq!(
        restored.settings.neutral_print_filters_from_database,
        applied.settings.neutral_print_filters_from_database
    );
}
