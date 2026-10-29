use super::*;
use crate::state;
use spektrafilm_core::presets::Section;
pub(super) fn test_app(label: &str) -> (App, PathBuf) {
    let directory =
        std::env::temp_dir().join(format!("spektrafilm-gui-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let state_path = directory.join("state.json");
    state::GuiState::factory().save(&state_path).unwrap();
    let mut app = App::new_with_browser_path(
        std::sync::Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
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
fn a_failed_preset_preserves_the_entire_application_transaction() {
    let (mut app, directory) = test_app("failed-transaction");
    app.params.output_mix.amount = 0.25;
    let mut look = builtin_presets().remove(0);
    look.name = "Missing profile".into();
    look.film_profile.as_mut().unwrap().stock = "missing_profile".into();
    std::fs::create_dir_all(directory.join("presets")).unwrap();
    std::fs::write(
        directory.join("presets").join("Missing profile.json"),
        look.to_json().unwrap(),
    )
    .unwrap();
    app.presets.refresh();
    app.presets.state.applied = Some(AppliedPreset {
        group: PresetGroup::BuiltIn,
        name: "Remembered".into(),
    });
    app.params.print_render.glare.active = true;
    app.toggle_scan_for_print();
    app.dirty_since = Some(std::time::Instant::now());
    {
        let name = "Missing profile";
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
        let preferences = app.presets.preferences.clone();
        let film = app.film_name.clone();
        let print = app.print_name.clone();
        let film_times = app.film_dev_times.clone();
        let print_times = app.print_dev_times.clone();
        let snapshot = app.scan_for_print_snapshot;
        let revision = app.parameter_revision;
        let epoch = app.calibration_epoch;
        let dirty = app.dirty;
        let dirty_since = app.dirty_since;
        app.load_preset(entry.group, &entry.name);
        assert!(
            app.status.to_string().starts_with("Preset error:"),
            "{}",
            app.status
        );
        assert_eq!(serde_json::to_value(&app.params).unwrap(), before);
        assert_eq!(app.current_state().unwrap().sections, state);
        assert_eq!(app.presets.state, browser);
        assert_eq!(app.presets.preferences, preferences);
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
            std::sync::Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
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
    let failed_restart = reopen(None);
    assert!(
        BrowserState::restore(
            &failed_restart.gui_state.sections,
            false,
            &failed_restart.browser_state_path,
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(&failed_restart.browser_state_path).unwrap(),
        "broken JSON"
    );
    assert!(matches!(
        &failed_restart.status,
        crate::app::Message::Text { key: "operations.startup_error", arguments }
            if arguments.iter().any(|(_, detail)| matches!(detail,
                crate::app::Message::Text { key: "app.browser.load_failed", .. }
            ))
    ));
    assert_eq!(failed_restart.presets.state, BrowserState::default());
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
    assert!(matches!(
        app.status,
        crate::app::Message::Text {
            key: "operations.browser_save_error",
            ..
        }
    ));
    assert!(!app.browser_state_path.exists());
    assert_eq!(
        std::fs::read_to_string(&blocked).unwrap(),
        "not a directory"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn scanner_transitions_reset_the_scan_polarity() {
    let (mut app, directory) = test_app("scanner-transitions");
    app.params.workflow.route = "input > film > scan".into();
    app.params.scanner.scan_output = "negative_to_positive".into();
    app.params.scanner.lens_blur = 2.5;
    let revision = app.parameter_revision;
    app.params.workflow.route = "input > film > print > scan".into();
    app.finish_route_change();
    assert!(
        app.status.to_string().contains("scan output"),
        "route normalization status must name the reset polarity: {}",
        app.status
    );
    assert_eq!(app.params.scanner.scan_output, "direct_scan");
    assert_eq!(app.params.scanner.lens_blur, 2.5);
    assert!(app.dirty);
    assert_eq!(app.parameter_revision, revision.wrapping_add(1));
    app.params.workflow.route = "input".into();
    let revision = app.parameter_revision;
    app.finish_route_change();
    assert!(
        app.status
            .to_string()
            .contains("scanner controls inactive for this route"),
        "route normalization must identify inactive scanner controls: {}",
        app.status
    );
    assert!(
        app.status
            .to_string()
            .contains("stored selections retained"),
        "{}",
        app.status
    );
    assert_eq!(app.parameter_revision, revision.wrapping_add(1));
    app.params.workflow.route = "input > convert-film > scan".into();
    app.finish_route_change();
    app.params.scanner.scan_output = "negative_to_positive".into();
    app.toggle_scan_for_print();
    app.select_film_profile("kodak_ektachrome_100".into());
    assert_eq!(app.film_medium, crate::controls::ScanMedium::Slide);
    assert_eq!(app.params.scanner.scan_output, "direct_scan");
    assert_eq!(app.params.scanner.lens_blur, 2.5);
    let restored = app.current_state().unwrap().runtime_params().unwrap();
    assert_eq!(restored.scanner.unsharp_mask, [0.7, 0.7]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn direct_material_selection_rebases_unowned_defaults_and_failed_selection_is_atomic() {
    let (mut app, directory) = test_app("material-default-selection");
    app.params.camera.exposure_compensation_ev = 1.25;
    app.params.scanner.lens_blur = 2.5;
    app.select_film_profile("kodak_kodachrome_64".into());
    let dir = &app.params.film_render.dir_couplers;
    assert!(dir.active);
    assert_eq!(dir.gamma_samelayer_rgb, [0.2942, 0.0841, 0.2492]);
    assert_eq!(dir.gamma_interlayer_r_to_gb, [0.1699, 0.2188]);
    assert_eq!(dir.gamma_interlayer_g_to_rb, [0.0625, 0.0692]);
    assert_eq!(dir.gamma_interlayer_b_to_rg, [0.1153, 0.2359]);
    assert_eq!(app.params.film_render.grain.rms_granularity, [10.0; 3]);
    assert_eq!(app.params.camera.exposure_compensation_ev, 1.25);
    assert_eq!(app.params.scanner.lens_blur, 2.5);
    let accepted = app.params.clone();
    let revision = app.parameter_revision;
    app.select_film_profile("missing_material".into());
    assert_eq!(app.film_name, "kodak_kodachrome_64");
    assert_eq!(app.params, accepted);
    assert_eq!(app.parameter_revision, revision);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn preset_application_reports_scanner_normalization() {
    let (mut app, directory) = test_app("preset-scanner-normalization");
    app.params.workflow.route = "input > film > scan".into();
    app.params.scanner.scan_output = "negative_to_positive".into();
    app.params.scanner.lens_blur = 2.5;
    let entry = app
        .presets
        .entries
        .iter()
        .find(|entry| entry.group == PresetGroup::BuiltIn)
        .unwrap()
        .clone();
    app.load_preset(entry.group, &entry.name);
    assert!(
        app.status.to_string().contains("scanner controls"),
        "{}",
        app.status
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn slide_startup_rejects_positive_output_with_visible_error() {
    let (app, directory) = test_app("slide-startup");
    let mut params = app.params.clone();
    params.workflow.route = "input > film > scan".into();
    params.scanner.scan_output = "negative_to_positive".into();
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
        std::sync::Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
        None,
        Some(path),
        None,
        directory.join("preset_browser_state.json"),
        crate::headless::data_dir(),
    );
    assert!(
        restarted
            .status
            .to_string()
            .contains("negative filming-stage film profile")
    );
    assert_eq!(
        restarted.params,
        state::GuiState::factory().runtime_params().unwrap()
    );
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
    let spektrafilm_core::presets::LibraryFault::DirectoryRead {
        detail: fault_detail,
    } = fault.fault.as_ref().unwrap()
    else {
        panic!("expected a directory-read fault");
    };
    let fault_detail = fault_detail.clone();
    let builtin_name = app
        .presets
        .entries
        .iter()
        .find(|entry| entry.group == PresetGroup::BuiltIn)
        .unwrap()
        .name
        .clone();
    let missing_name = "Unavailable user look";
    app.presets.state.applied = Some(AppliedPreset {
        group: PresetGroup::User,
        name: missing_name.into(),
    });
    assert_eq!(
        marker(
            app.presets.state.applied.as_ref().unwrap(),
            &app.presets.entries,
            &app.params,
            &app.film_name,
            &app.print_name
        ),
        Marker::Missing,
    );
    let context = egui::Context::default();
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1460.0, 900.0),
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
    assert!(
        rendered
            .iter()
            .any(|text| text.contains(&path.display().to_string()) && text.contains(&fault_detail)),
        "{rendered:?}"
    );
    assert!(
        rendered.iter().any(|text| text.contains(&builtin_name)),
        "{rendered:?}"
    );
    assert!(
        rendered.iter().any(|text| text.contains(missing_name)),
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
    let look = p.look_parameters().expect("captured look preset");
    look.apply_to(&mut params);
    let film = &p.film_profile.as_ref().unwrap().stock;
    let print = &p.print_profile.as_ref().unwrap().stock;
    let entries = vec![LibraryEntry {
        group: PresetGroup::BuiltIn,
        name: p.name.clone(),
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
fn browser_search_matches_both_material_names_and_ids_without_changing_order_or_state() {
    let (mut app, directory) = test_app("browser-search");
    let initial = app
        .presets
        .entries
        .iter()
        .find(|entry| {
            entry
                .preset
                .as_ref()
                .is_some_and(|preset| preset.kind() == Ok(PresetKind::Look))
        })
        .unwrap()
        .clone();
    app.load_preset(initial.group, &initial.name);
    app.params.enlarger.print_exposure += 0.5;
    let controls = serde_json::to_value(&app.params).unwrap();
    let preparation = app.params.render_preparation_state();
    let state = app.presets.state.clone();
    let revision = app.parameter_revision;
    let film = initial.film_profile.as_ref().unwrap().stock.clone();
    let print = initial.print_profile.as_ref().unwrap().stock.clone();
    let films = vec![crate::profiles::ProfileEntry {
        stock: film.clone(),
        display: "Film Search Display".into(),
    }];
    let prints = vec![crate::profiles::ProfileEntry {
        stock: print.clone(),
        display: "Print Search Display".into(),
    }];
    app.presets.update_search(&films, &prints);
    let unfiltered = app.presets.visible_indices().to_vec();
    for query in [
        initial.name.to_uppercase(),
        film.to_uppercase(),
        print.to_uppercase(),
        "fILm sEARch dISPlay".into(),
        "pRINT seARch diSPLAY".into(),
    ] {
        app.presets.query = query;
        app.presets.update_search(&films, &prints);
        let visible = app.presets.visible_indices();
        assert!(
            visible
                .iter()
                .any(|index| app.presets.entries[*index].name == initial.name
                    && app.presets.entries[*index].group == initial.group)
        );
        assert!(visible.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(visible.iter().all(|index| unfiltered.contains(index)));
    }
    app.presets.query = "not-a-stock-or-look-9d8e7f".into();
    app.presets.update_search(&films, &prints);
    assert!(app.presets.visible_indices().is_empty());
    assert_eq!(
        marker(
            state.applied.as_ref().unwrap(),
            &app.presets.entries,
            &app.params,
            &app.film_name,
            &app.print_name
        ),
        Marker::Modified
    );
    app.presets.query.clear();
    app.presets.update_search(&films, &prints);
    assert_eq!(app.presets.visible_indices(), unfiltered);
    assert_eq!(serde_json::to_value(&app.params).unwrap(), controls);
    assert_eq!(app.params.render_preparation_state(), preparation);
    assert_eq!(app.presets.state, state);
    assert_eq!(app.parameter_revision, revision);
    assert!(
        serde_json::to_value(&app.presets.state)
            .unwrap()
            .get("query")
            .is_none()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn browser_filters_only_looks_and_does_not_hide_library_faults() {
    let (mut app, directory) = test_app("search-faults");
    let section = Preset::capture_section(
        "Needle section",
        Section::OutputMix,
        &app.params,
        provenance(),
    )
    .unwrap();
    app.presets
        .library
        .save_new(&section, &app.data_dir)
        .unwrap();
    std::fs::write(directory.join("presets/Broken.json"), "not JSON").unwrap();
    app.presets.refresh();
    app.presets.query = "Needle section".into();
    app.presets.update_search(&app.films, &app.papers);
    assert!(app.presets.visible_indices().is_empty());
    assert!(
        app.presets
            .entries
            .iter()
            .any(|entry| entry.fault.is_some())
    );
    app.presets.query.clear();
    app.presets.update_search(&app.films, &app.papers);
    assert!(app.presets.visible_indices().iter().all(|index| {
        app.presets.entries[*index].preset.as_ref().unwrap().kind() == Ok(PresetKind::Look)
    }));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn browser_clear_collapse_and_reopen_preserve_controls_and_missing_context() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, directory) = test_app("browser-interaction");
    app.presets.state.applied = Some(AppliedPreset {
        group: PresetGroup::User,
        name: "Absent look".into(),
    });
    app.presets.query = "query-with-no-look-match".into();
    let params = serde_json::to_value(&app.params).unwrap();
    let revision = app.parameter_revision;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1460.0, 980.0))
        .build_state(
            |ctx, app: &mut App| {
                app.preset_panel(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Retained photograph");
                });
            },
            app,
        );
    harness.run();
    assert!(
        harness
            .query_by_label("Current look: My Presets / Absent look • missing")
            .is_some()
    );
    let name_bounds = harness
        .query_all_by_label("Absent look")
        .next()
        .and_then(|node| node.raw_bounds())
        .expect("visible current-look name");
    let group_bounds: Vec<_> = harness
        .query_all_by_label("My Presets")
        .filter_map(|node| node.raw_bounds())
        .collect();
    assert!(
        group_bounds
            .iter()
            .any(|group| { (group.y0 - name_bounds.y0).abs() < 4.0 && group.x1 <= name_bounds.x0 }),
        "the fixed current-look summary must show group identity beside the name: \
         name={name_bounds:?} groups={group_bounds:?}"
    );
    assert!(
        harness
            .query_by_label("No matching look presets. Clear preset search to show all looks.")
            .is_some()
    );
    let preferred_width = harness.state().presets.state.width;
    harness.set_size(egui::vec2(1100.0, 760.0));
    harness.run();
    assert!(harness.state().presets.state.expanded);
    assert!(
        harness
            .query_by_label("Current look: My Presets / Absent look • missing")
            .is_none()
    );
    harness.get_by_label("Collapse or expand Presets").click();
    harness.run();
    assert!(
        harness
            .query_by_label("Current look: My Presets / Absent look • missing")
            .is_some()
    );
    assert!(harness.state().presets.state.expanded);
    assert_eq!(harness.state().presets.state.width, preferred_width);
    harness.get_by_label("Collapse or expand Presets").click();
    harness.run();
    assert!(harness.state().presets.state.expanded);
    harness.set_size(egui::vec2(1460.0, 980.0));
    harness.run();
    assert_eq!(harness.state().presets.state.width, preferred_width);
    assert!(
        harness
            .query_by_label("Current look: My Presets / Absent look • missing")
            .is_some()
    );
    harness.get_by_label("Collapse or expand Presets").click();
    harness.run();
    assert!(!harness.state().presets.state.expanded);
    assert_eq!(harness.state().presets.query, "query-with-no-look-match");
    harness.get_by_label("Collapse or expand Presets").click();
    harness.run();
    assert!(harness.state().presets.state.expanded);
    harness.get_by_label("Clear preset search").click();
    harness.run();
    assert!(harness.state().presets.query.is_empty());
    assert!(!harness.state().presets.visible_indices().is_empty());
    assert_eq!(
        serde_json::to_value(&harness.state().params).unwrap(),
        params
    );
    assert_eq!(harness.state().parameter_revision, revision);
    assert!(
        harness
            .query_by_label("Current look: My Presets / Absent look • missing")
            .is_some()
    );
    drop(harness);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn look_application_preserves_existing_explicit_ownership_and_legacy_camera_omission() {
    let (mut app, directory) = test_app("look-ownership");
    let mut ownership = app.params.render_preparation_state();
    ownership.explicit_section_paths = vec!["film_render.halation.active".into()];
    app.params.restore_render_preparation_state(&ownership);
    let mut authored = app.params.clone();
    authored.camera.exposure_compensation_ev = 2.0;
    let mut preset = Preset::capture(
        "Authored look",
        &app.film_name,
        &app.print_name,
        &authored,
        provenance(),
    );
    app.presets
        .library
        .save_new(&preset, &app.data_dir)
        .unwrap();
    app.load_preset(PresetGroup::User, &preset.name);
    assert_eq!(app.params.camera.exposure_compensation_ev, 2.0);
    assert_eq!(
        app.params.render_preparation_state().explicit_section_paths,
        ownership.explicit_section_paths
    );
    if let spektrafilm_core::presets::PresetParameters::Look(look) = &mut preset.parameters {
        look.camera = None;
    }
    preset.name = "Legacy camera omission".into();
    app.presets
        .library
        .save_new(&preset, &app.data_dir)
        .unwrap();
    app.params.camera.exposure_compensation_ev = 1.25;
    app.load_preset(PresetGroup::User, &preset.name);
    assert_eq!(app.params.camera.exposure_compensation_ev, 1.25);
    assert_eq!(
        app.params.render_preparation_state().explicit_section_paths,
        ownership.explicit_section_paths
    );
    std::fs::remove_dir_all(directory).unwrap();
}
