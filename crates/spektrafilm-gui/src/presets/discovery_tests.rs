use super::discovery::{Filter, Preferences};
use super::*;

fn identity(group: PresetGroup, name: &str) -> AppliedPreset {
    AppliedPreset {
        group,
        name: name.into(),
    }
}

#[test]
fn recent_keeps_ten_distinct_group_qualified_successful_applications() {
    let (mut app, directory) = super::tests::test_app("discovery-recent");
    let template = spektrafilm_core::presets::builtin_presets().remove(0);
    for index in 0..12 {
        let mut preset = template.clone();
        preset.name = format!("Recent {index:02}");
        app.presets
            .library
            .save_new(&preset, &app.data_dir)
            .unwrap();
        app.load_preset(PresetGroup::User, &preset.name);
        assert_eq!(
            app.presets.preferences.recent[0],
            identity(PresetGroup::User, &preset.name)
        );
    }
    assert_eq!(app.presets.preferences.recent.len(), 10);
    assert!(
        !app.presets
            .preferences
            .recent
            .contains(&identity(PresetGroup::User, "Recent 00"))
    );
    app.load_preset(PresetGroup::User, "Recent 03");
    assert_eq!(
        app.presets.preferences.recent[0],
        identity(PresetGroup::User, "Recent 03")
    );
    assert_eq!(app.presets.preferences.recent.len(), 10);
    let before = app.presets.preferences.clone();
    app.load_preset(PresetGroup::User, "missing look");
    assert_eq!(app.presets.preferences, before);
    let section =
        Preset::capture_section("A section", Section::OutputMix, &app.params, provenance())
            .unwrap();
    app.presets
        .library
        .save_new(&section, &app.data_dir)
        .unwrap();
    app.load_preset(PresetGroup::User, &section.name);
    assert_eq!(app.presets.preferences, before);
    let mut preferences = Preferences::default();
    preferences.applied(identity(PresetGroup::BuiltIn, "Same"));
    preferences.applied(identity(PresetGroup::User, "Same"));
    assert_eq!(preferences.recent.len(), 2);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn favorites_recent_and_search_keep_library_order_groups_context_and_controls() {
    let (mut app, directory) = super::tests::test_app("discovery-filters");
    let builtin = app
        .presets
        .entries
        .iter()
        .find(|entry| {
            entry.group == PresetGroup::BuiltIn
                && entry
                    .preset
                    .as_ref()
                    .is_some_and(|preset| preset.kind() == Ok(PresetKind::Look))
        })
        .unwrap()
        .clone();
    app.presets
        .library
        .save_new(builtin.preset.as_ref().unwrap(), &app.data_dir)
        .unwrap();
    app.presets.refresh();
    app.load_preset(PresetGroup::BuiltIn, &builtin.name);
    app.params.output_mix.amount = 0.25;
    let controls = app.params.clone();
    let applied = app.presets.state.applied.clone();
    let revision = app.parameter_revision;
    let dirty = app.dirty;
    app.presets
        .preferences
        .toggle_favorite(identity(PresetGroup::User, &builtin.name));
    app.presets.preferences.filter = Filter::Favorites;
    app.presets.save_preferences();
    app.presets.update_search(&app.films, &app.papers);
    assert_eq!(app.presets.visible_indices().len(), 1);
    assert_eq!(
        app.presets.entries[app.presets.visible_indices()[0]].group,
        PresetGroup::User
    );
    app.presets
        .preferences
        .toggle_favorite(identity(PresetGroup::BuiltIn, &builtin.name));
    app.presets.save_preferences();
    app.presets.update_search(&app.films, &app.papers);
    let visible = app.presets.visible_indices().to_vec();
    assert_eq!(visible.len(), 2);
    assert!(visible.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(app.presets.entries[visible[0]].group, PresetGroup::BuiltIn);
    assert_eq!(app.presets.entries[visible[1]].group, PresetGroup::User);
    app.presets.query = "unmatched query".into();
    app.presets.update_search(&app.films, &app.papers);
    assert!(app.presets.visible_indices().is_empty());
    assert_eq!(
        marker(
            applied.as_ref().unwrap(),
            &app.presets.entries,
            &app.params,
            &app.film_name,
            &app.print_name
        ),
        Marker::Modified
    );
    app.presets.query.clear();
    app.presets.preferences.filter = Filter::Recent;
    app.presets.save_preferences();
    app.presets.update_search(&app.films, &app.papers);
    assert_eq!(app.presets.visible_indices().len(), 1);
    assert_eq!(
        app.presets.entries[app.presets.visible_indices()[0]].group,
        PresetGroup::BuiltIn
    );
    app.presets.state.applied = Some(identity(PresetGroup::User, "Missing favorite"));
    app.presets
        .preferences
        .toggle_favorite(identity(PresetGroup::User, "Missing favorite"));
    app.presets.save_preferences();
    assert_eq!(
        marker(
            app.presets.state.applied.as_ref().unwrap(),
            &app.presets.entries,
            &app.params,
            &app.film_name,
            &app.print_name
        ),
        Marker::Missing
    );
    assert_eq!(app.params, controls);
    assert_eq!(app.parameter_revision, revision);
    assert_eq!(app.dirty, dirty);
    let state = app.current_state().unwrap();
    let browser_json = serde_json::to_value(&app.presets.state).unwrap();
    for key in ["favorites", "recent", "filter"] {
        assert!(browser_json.get(key).is_none());
        assert!(state.sections["rust"]["presets"].get(key).is_none());
    }
    let reopened = Browser::new(
        &serde_json::json!({}),
        PresetLibrary::new(directory.join("presets")),
    );
    assert_eq!(reopened.preferences, app.presets.preferences);
    assert!(reopened.query.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn favorite_filter_does_not_hide_faults_or_missing_applied_context() {
    let (mut app, directory) = super::tests::test_app("discovery-fault-context");
    std::fs::create_dir_all(directory.join("presets")).unwrap();
    let fault_path = directory.join("presets/Broken.json");
    std::fs::write(&fault_path, "broken document").unwrap();
    app.presets.refresh();
    app.presets.preferences.filter = Filter::Favorites;
    app.presets.save_preferences();
    app.presets.query = "no matches".into();
    app.presets.state.applied = Some(identity(PresetGroup::User, "Missing applied look"));
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
    fn collect(shape: &egui::epaint::Shape, text: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(shape) => text.push(shape.galley.text().to_owned()),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, text);
                }
            }
            _ => {}
        }
    }
    let mut text = Vec::new();
    for shape in output.shapes {
        collect(&shape.shape, &mut text);
    }
    assert!(
        text.iter()
            .any(|text| text.contains("Missing applied look")),
        "{text:?}"
    );
    assert!(
        text.iter()
            .any(|text| text.contains(&fault_path.display().to_string())),
        "{text:?}"
    );
    assert!(app.presets.visible_indices().is_empty());
    assert!(app.presets.preferences.recent.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn application_owned_rename_migrates_only_user_references_and_failed_rename_does_not() {
    let (mut app, directory) = super::tests::test_app("discovery-rename");
    let mut look = spektrafilm_core::presets::builtin_presets().remove(0);
    look.name = "Old name".into();
    app.presets.library.save_new(&look, &app.data_dir).unwrap();
    app.presets.refresh();
    app.load_preset(PresetGroup::User, &look.name);
    app.presets
        .preferences
        .toggle_favorite(identity(PresetGroup::BuiltIn, &look.name));
    app.presets
        .preferences
        .toggle_favorite(identity(PresetGroup::User, &look.name));
    let entry = app
        .presets
        .entries
        .iter()
        .position(|entry| entry.group == PresetGroup::User && entry.name == look.name)
        .unwrap();
    app.apply_preset_action(entry, PresetAction::Rename);
    app.presets.name_dialog.as_mut().unwrap().name = "New name".into();
    let context = egui::Context::default();
    let _ = context.run(egui::RawInput::default(), |context| {
        app.preset_panel(context)
    });
    let _ = context.run(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: Some(egui::Key::Enter),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |context| app.preset_panel(context),
    );
    assert_eq!(
        app.presets.state.applied,
        Some(identity(PresetGroup::User, "New name"))
    );
    assert_eq!(
        app.presets.preferences.recent[0],
        identity(PresetGroup::User, "New name")
    );
    assert!(
        app.presets
            .preferences
            .favorites
            .contains(&identity(PresetGroup::User, "New name"))
    );
    assert!(
        app.presets
            .preferences
            .favorites
            .contains(&identity(PresetGroup::BuiltIn, "Old name"))
    );
    assert!(
        !app.presets
            .preferences
            .favorites
            .contains(&identity(PresetGroup::User, "Old name"))
    );
    let saved = Preferences::load(&directory.join("preset_discovery_preferences.json")).unwrap();
    assert_eq!(saved, app.presets.preferences);
    let before = app.presets.preferences.clone();
    assert!(
        app.presets
            .library
            .rename("does not exist", "Another", &app.data_dir)
            .is_err()
    );
    assert_eq!(app.presets.preferences, before);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn malformed_preferences_are_preserved_and_atomic_write_failure_does_not_corrupt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preset_discovery_preferences.json");
    std::fs::write(&path, "broken preferences").unwrap();
    assert!(Preferences::load(&path).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "broken preferences"
    );
    let mut preferences = Preferences::default();
    preferences.toggle_favorite(identity(PresetGroup::User, "Missing"));
    preferences.applied(identity(PresetGroup::User, "Missing"));
    preferences.filter = Filter::Favorites;
    preferences.save(&path).unwrap();
    assert_eq!(Preferences::load(&path).unwrap(), preferences);
    let blocked = directory.path().join("blocked");
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("retained"), "untouched").unwrap();
    assert!(preferences.save(&blocked).is_err());
    assert_eq!(
        std::fs::read_to_string(blocked.join("retained")).unwrap(),
        "untouched"
    );
}

#[test]
fn favorite_controls_identify_preset_and_group_and_only_toggle_the_addressed_identity() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, directory) = super::tests::test_app("discovery-accessible-favorites");
    let mut preset = spektrafilm_core::presets::builtin_presets().remove(0);
    app.presets
        .library
        .save_new(&preset, &app.data_dir)
        .unwrap();
    app.presets.refresh();
    app.presets.query = preset.name.clone();
    let name = std::mem::take(&mut preset.name);
    let before = app.params.clone();
    let revision = app.parameter_revision;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1460.0, 1600.0))
        .build_eframe(move |cc| {
            crate::theme::apply(&cc.egui_ctx);
            app
        });
    let builtin = format!("Add to favorites: Built-in / {name}");
    let user = format!("Add to favorites: My Presets / {name}");
    assert!(harness.query_by_label(&builtin).is_some());
    assert!(harness.query_by_label(&user).is_some());
    harness.get_by_label(&user).click();
    harness.run_steps(2);
    assert!(
        harness
            .query_by_label(&format!("Remove from favorites: My Presets / {name}"))
            .is_some()
    );
    assert!(harness.query_by_label(&builtin).is_some());
    assert_eq!(
        harness.state().presets.preferences.favorites,
        vec![identity(PresetGroup::User, &name)]
    );
    assert_eq!(harness.state().params, before);
    assert_eq!(harness.state().parameter_revision, revision);
    drop(harness);
    std::fs::remove_dir_all(directory).unwrap();
}
