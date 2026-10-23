use super::*;
use crate::test_support::data_dir as data;
fn test_film() -> Profile {
    profile::load_profile_by_name(&data(), "kodak_portra_400").unwrap()
}
fn provenance() -> Provenance {
    Provenance {
        implementation_version: "test".into(),
        model_version: "test".into(),
        scanner: None,
    }
}

#[test]
fn scanner_preset_import_rejects_removed_mode_and_preserves_canonical_or_omitted_output() {
    let mut current = RuntimeParams::default();
    current.workflow.route = "input > film > scan".into();
    current.io.scan_film = true;
    let film = test_film();
    let mut document = serde_json::json!({
        "schema_version": 2,
        "name": "Negative conversion",
        "parameters": {"scanner": {"scan_output": "negative_to_positive"}},
        "provenance": {"implementation_version": "test", "model_version": "test"}
    });
    let canonical = Preset::from_json_str(&document.to_string()).unwrap();
    for imported in [
        Preset::from_json_str(&canonical.to_json().unwrap()).unwrap(),
        Preset::from_toml_str(&canonical.to_toml().unwrap()).unwrap(),
    ] {
        let applied = imported.apply_section(&current, &film).unwrap();
        assert_eq!(applied.scanner.scan_output, "negative_to_positive");
    }
    document["parameters"]["scanner"]["scan_output"] = "positive_scan".into();
    let error = Preset::from_json_str(&document.to_string()).unwrap_err();
    assert!(error.contains("scanner.scan_output"));
    let removed_toml = canonical
        .to_toml()
        .unwrap()
        .replace("negative_to_positive", "positive_scan");
    assert!(Preset::from_toml_str(&removed_toml).is_err());
    document["parameters"]["scanner"] = serde_json::json!({"lens_blur": 0.0});
    let omitted = Preset::from_json_str(&document.to_string()).unwrap();
    let applied = omitted.apply_section(&current, &film).unwrap();
    assert_eq!(applied.scanner.scan_output, "direct_scan");
}

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "spektrafilm-library-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn version_one_maps_name_and_materializes_controls() {
    let text = r#"{"schema_version":1,"id":"old","name":"Portable","film_profile":{"stock":"kodak_portra_400"},"print_profile":{"stock":"kodak_portra_endura"},"parameters":{"enlarger":{},"scanner":{},"film_render":{},"print_render":{},"camera_diffusion_filter":{}},"provenance":{"implementation_version":"test","model_version":"test"}}"#;
    let p = Preset::from_json_str(text).unwrap();
    assert_eq!(p.schema_version, PRESET_SCHEMA_VERSION);
    assert_eq!(p.name, "Portable");
    assert!(!p.to_json().unwrap().contains("\"id\""));
    // The legacy document gains the material controls the migration supplies.
    assert!(p.to_toml().unwrap().contains("print_exposure_mode"));
    assert!(p.to_toml().unwrap().contains("output_mix"));
    let mut v = serde_json::to_value(p).unwrap();
    v["id"] = "invalid".into();
    assert!(Preset::from_json_str(&v.to_string()).is_err());
    v.as_object_mut().unwrap().remove("id");
    v["route"] = "input".into();
    assert!(Preset::from_json_str(&v.to_string()).is_err());
}

#[test]
fn version_three_looks_map_the_exposure_pair_and_reject_retired_scanner_spelling() {
    for (pair, mode, tracking) in [
        (
            serde_json::json!({"normalize_print_exposure": true, "print_exposure_compensation": true}),
            "auto",
            serde_json::json!("follow_mode"),
        ),
        (
            serde_json::json!({"normalize_print_exposure": false, "print_exposure_compensation": false}),
            "manual",
            serde_json::json!("follow_mode"),
        ),
        (
            serde_json::json!({"normalize_print_exposure": true, "print_exposure_compensation": false}),
            "auto",
            serde_json::json!("never"),
        ),
        (
            serde_json::json!({"normalize_print_exposure": false, "print_exposure_compensation": true}),
            "manual",
            serde_json::json!("always"),
        ),
    ] {
        let preset = builtin_presets().remove(0);
        let mut document = serde_json::to_value(preset).unwrap();
        document["schema_version"] = 3.into();
        // Strip the canonical spelling before injecting the legacy one; a
        // real v3 document predates both.
        document["parameters"]["enlarger"]
            .as_object_mut()
            .unwrap()
            .remove("print_exposure_mode");
        document["parameters"]["settings"]
            .as_object_mut()
            .unwrap()
            .remove("print_exposure_ev_tracking");
        document["parameters"]["enlarger"]
            .as_object_mut()
            .unwrap()
            .extend(pair.as_object().unwrap().clone());
        let migrated = Preset::from_json_str(&document.to_string()).unwrap();
        assert_eq!(migrated.schema_version, PRESET_SCHEMA_VERSION);
        let look = migrated.look_parameters().unwrap();
        assert_eq!(look.enlarger.print_exposure_mode.as_str(), mode);
        assert_eq!(
            serde_json::to_value(&look.settings).unwrap()["print_exposure_ev_tracking"],
            tracking
        );
        // A v4 document must not carry the legacy spelling.
        let mut current = serde_json::to_value(migrated).unwrap();
        current["parameters"]["enlarger"]["normalize_print_exposure"] = true.into();
        assert!(Preset::from_json_str(&current.to_string()).is_err());
    }
    // The retired scanner conversion spelling is rejected, not migrated:
    // the neutral scan model has no conversion controls to map onto.
    let preset = builtin_presets().remove(0);
    let mut document = serde_json::to_value(preset).unwrap();
    document["schema_version"] = 3.into();
    document["parameters"]["scanner"]["positive_density_ev"] = 0.5.into();
    assert!(Preset::from_json_str(&document.to_string()).is_err());
    // A single enlarger section cannot carry the tracking setting, so a
    // mixed legacy pair fails with an explicit error.
    let section = serde_json::json!({
        "schema_version": 3,
        "name": "Mixed",
        "parameters": {"enlarger": {"normalize_print_exposure": true, "print_exposure_compensation": false}},
    });
    assert!(
        Preset::from_json_str(&section.to_string())
            .unwrap_err()
            .contains("print_exposure_mode explicitly")
    );
}

#[test]
fn legacy_whole_looks_gain_full_mix_and_preserve_caller_io() {
    for version in [1, 2] {
        let preset = builtin_presets().remove(0);
        let mut document = serde_json::to_value(preset).unwrap();
        document["schema_version"] = version.into();
        let parameters = document["parameters"].as_object_mut().unwrap();
        parameters.remove("output_mix");
        parameters.remove("camera");
        let migrated = Preset::from_json_str(&document.to_string()).unwrap();
        assert_eq!(migrated.schema_version, PRESET_SCHEMA_VERSION);
        assert_eq!(migrated.look_parameters().unwrap().output_mix.amount, 1.0);
        let mut caller = RuntimeParams::default();
        caller.output_mix.amount = 0.25;
        caller.io.input_color_space = "ACES2065-1".into();
        caller.io.output_color_space = "ProPhoto RGB".into();
        caller.io.output_gamut_compress.algorithm = "off".into();
        caller.io.scan_film =
            profile::load_profile_by_name(&data(), &migrated.film_profile.as_ref().unwrap().stock)
                .unwrap()
                .is_positive();
        let resolved = migrated.resolve(&data(), &caller).unwrap();
        assert_eq!(resolved.params.io, caller.io);
        assert_eq!(resolved.params.output_mix.amount, 1.0);
        let exported = serde_json::from_str::<Value>(&migrated.to_json().unwrap()).unwrap();
        assert_eq!(exported["schema_version"], PRESET_SCHEMA_VERSION);
        assert!(exported["parameters"].get("io").is_none());
        document["parameters"]["io"] = serde_json::json!({});
        assert!(Preset::from_json_str(&document.to_string()).is_err());
    }
}

#[test]
fn v3_mix_section_is_sparse_and_current_look_requires_mix() {
    let mut current = RuntimeParams::default();
    current.output_mix.amount = 0.3456789012345678;
    current.enlarger.preflash_exposure = 0.4;
    let captured =
        Preset::capture_section("Output", Section::OutputMix, &current, provenance()).unwrap();
    let loaded = Preset::from_toml_str(&captured.to_toml().unwrap()).unwrap();
    assert_eq!(loaded, captured);
    let applied = loaded
        .apply_section(&RuntimeParams::default(), &test_film())
        .unwrap();
    assert_eq!(applied.output_mix, current.output_mix);
    assert_eq!(applied.enlarger.preflash_exposure, 0.0);
    for version in [1, 2] {
        let legacy = serde_json::json!({"schema_version":version,"name":"Preflash","parameters":{"enlarger":{"preflash_exposure":0.2}},"provenance":provenance()});
        let preset = Preset::from_json_str(&legacy.to_string()).unwrap();
        let applied = preset.apply_section(&current, &test_film()).unwrap();
        assert_eq!(applied.output_mix, current.output_mix);
        let exported: Value = serde_json::from_str(&preset.to_json().unwrap()).unwrap();
        assert_eq!(exported["parameters"], legacy["parameters"]);
    }
    let mut document = serde_json::to_value(builtin_presets().remove(0)).unwrap();
    document["parameters"]
        .as_object_mut()
        .unwrap()
        .remove("output_mix");
    assert!(Preset::from_json_str(&document.to_string()).is_err());
    for path in ["film_render", "print_render"] {
        let mut draft = serde_json::to_value(builtin_presets().remove(0)).unwrap();
        draft["parameters"][path]["response"] = serde_json::json!({});
        assert!(Preset::from_json_str(&draft.to_string()).is_err());
    }
}
#[test]
fn portable_names() {
    for bad in [
        "", "a/b", "a\\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "a\nb", "a.",
        "a.TOML", "a.json", "CON", "NUL.txt", "com9", "LPT1",
    ] {
        assert!(validate_name(bad).is_err(), "{bad:?}");
    }
    assert!(validate_name(&"x".repeat(65)).is_err());
    for good in ["Kodak Portra 400", "胶片", "é", "a b", "COM10"] {
        validate_name(good).unwrap();
    }
}
#[test]
fn every_section_roundtrips_and_applies_only_carried_values() {
    fn contains_null(value: &Value) -> bool {
        value.is_null()
            || value
                .as_object()
                .is_some_and(|map| map.values().any(contains_null))
    }
    let params = RuntimeParams::default();
    for section in SECTIONS {
        let p = Preset::capture_section(section.name(), *section, &params, provenance()).unwrap();
        assert_eq!(p.kind().unwrap(), PresetKind::Section(*section));
        let json = p.to_json().unwrap();
        let q = Preset::from_json_str(&json).unwrap();
        assert_eq!(p, q);
        let payload = serde_json::to_value(&p.parameters).unwrap();
        if contains_null(&payload) {
            assert!(p.to_toml().unwrap_err().contains("explicit null"));
        } else {
            let toml = p.to_toml().unwrap();
            let q = Preset::from_toml_str(&toml).unwrap();
            assert_eq!(q.kind().unwrap(), p.kind().unwrap());
        }
        let actual = p.apply_section(&params, &test_film()).unwrap();
        assert!(section_matches(&p, &actual).unwrap());
        assert_eq!(q.kind().unwrap(), p.kind().unwrap());
        assert_eq!(actual.workflow.route, params.workflow.route);
        assert_eq!(
            actual.settings.neutral_print_filters_from_database,
            params.settings.neutral_print_filters_from_database
        );
    }
}
#[test]
fn kinds_reject_wrong_scope_and_unknown_values() {
    for payload in [
        serde_json::json!({}),
        serde_json::json!({"film_render":{"grain":{"active":{"below_leaf":true}}}}),
        serde_json::json!({"film_render":{"grain":{"active":true},"halation":{"active":true}}}),
        serde_json::json!({"film_render":{"grain":{"unknown":3}}}),
    ] {
        assert!(section_from_payload(&payload).is_err());
    }
    assert_eq!(
        section_from_payload(&serde_json::json!({"film_render":{"density_curve_gamma":1.0}}))
            .unwrap(),
        Section::Film
    );
    let p = Preset::capture_section(
        "Grain",
        Section::Grain,
        &RuntimeParams::default(),
        provenance(),
    )
    .unwrap();
    let mut v = serde_json::to_value(p).unwrap();
    v["film_profile"] = serde_json::json!({"stock":"kodak_portra_400"});
    assert!(Preset::from_json_str(&v.to_string()).is_err());
}
#[test]
fn sections_preserve_explicit_values_and_match_name_order() {
    let mut current = RuntimeParams::default();
    current.film_render.grain.rms_granularity = [1.234; 3];
    let a = Preset::capture_section("Zulu", Section::Grain, &current, provenance()).unwrap();
    let mut b = a.clone();
    b.name = "Alpha".into();
    assert_eq!(
        matching_section_preset(Section::Grain, [&a, &b], &current)
            .unwrap()
            .unwrap()
            .name,
        "Alpha"
    );
    let applied = a
        .apply_section(&RuntimeParams::default(), &test_film())
        .unwrap();
    let h = Preset::capture_section("Halation", Section::Halation, &applied, provenance()).unwrap();
    let mut applied = h.apply_section(&applied, &test_film()).unwrap();
    let film = profile::load_profile_by_name(&data(), "kodak_gold_200").unwrap();
    apply_film_specifics(&mut applied, &film);
    assert_eq!(applied.film_render.grain.rms_granularity, [1.234; 3]);
}
#[test]
fn library_faults_conflicts_shadowing_and_management() {
    let temp = Temp::new();
    let lib = PresetLibrary::new(temp.0.join("presets"));
    lib.scan().unwrap();
    assert!(!lib.directory().exists());
    let mut p = builtin_presets().remove(0);
    lib.save_new(&p, &data()).unwrap();
    let hit = lib.resolve_selector(&p.name).unwrap();
    assert_eq!(hit.group, Some(PresetGroup::User));
    assert!(hit.shadowed_builtin);
    assert_eq!(
        lib.load(PresetGroup::BuiltIn, &p.name).unwrap().group,
        Some(PresetGroup::BuiltIn)
    );
    std::fs::write(lib.directory().join("Broken.toml"), "not valid !!").unwrap();
    std::fs::write(lib.directory().join("Misnamed.json"), p.to_json().unwrap()).unwrap();
    let scanned = lib.scan().unwrap();
    assert_eq!(scanned.iter().filter(|e| e.fault.is_some()).count(), 2);
    assert!(
        scanned
            .iter()
            .any(|e| e.group == PresetGroup::User && e.fault.is_none())
    );
    assert!(
        lib.import(
            &lib.directory().join(format!("{}.toml", p.name)),
            None,
            &data()
        )
        .unwrap_err()
        .contains("renamed copy")
    );
    let original = p.name.clone();
    lib.rename(&original, "Renamed", &data()).unwrap();
    assert!(lib.load(PresetGroup::User, &original).is_err());
    p.name = "Renamed".into();
    lib.update(&p, &data()).unwrap();
    lib.duplicate(&p, "Copy", &data()).unwrap();
    let exported = temp.0.join("Renamed.json");
    lib.export(&p, &exported, &data()).unwrap();
    assert_eq!(load_file(&exported).unwrap(), p);
    lib.import(&exported, Some("Imported"), &data()).unwrap();
    lib.delete("Copy").unwrap();
    std::fs::write(
        lib.directory().join("renamed.json"),
        p.to_json().unwrap().replace("Renamed", "renamed"),
    )
    .unwrap();
    assert_eq!(
        lib.scan()
            .unwrap()
            .iter()
            .filter(|e| e.name.eq_ignore_ascii_case("renamed") && e.fault.is_some())
            .count(),
        2
    );
}
#[test]
fn atomic_failure_retains_document() {
    let temp = Temp::new();
    let lib = PresetLibrary::new(temp.0.clone());
    let p = builtin_presets().remove(0);
    let path = lib.save_new(&p, &data()).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut invalid = p.clone();
    invalid.name = "CON".into();
    assert!(lib.update(&invalid, &data()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let destination = temp.0.join("Directory.toml");
    std::fs::create_dir(&destination).unwrap();
    assert!(super::library::atomic_write(&destination, "test").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(
        std::fs::read_dir(&temp.0).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
}
#[test]
fn carrier_validation_and_precision() {
    let mut p = builtin_presets().remove(0);
    p.look_parameters_mut()
        .unwrap()
        .film_render
        .chemistry
        .gamma_factor = 1.2345678901234567;
    assert_eq!(Preset::from_toml_str(&p.to_toml().unwrap()).unwrap(), p);
    assert_eq!(Preset::from_json_str(&p.to_json().unwrap()).unwrap(), p);
    let text = p.to_json().unwrap().replacen(
        "\"schema_version\": 5",
        "\"schema_version\": 5, \"schema_version\": 5",
        1,
    );
    assert!(Preset::from_json_str(&text).is_err());
    let mut v = serde_json::to_value(p).unwrap();
    v["parameters"]["enlarger"]["print_exposure"] = serde_json::json!(1e100);
    assert!(Preset::from_json_str(&v.to_string()).is_err());
}
#[test]
fn explicit_null_is_json_only_and_never_discarded() {
    let text = r#"{"schema_version":2,"name":"Clear amount","parameters":{"film_render":{"grain":{"v2_amount":null}}},"provenance":{"implementation_version":"test","model_version":"test"}}"#;
    let preset = Preset::from_json_str(text).unwrap();
    let exported = preset.to_json().unwrap();
    let exported_value: Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(
        exported_value["parameters"],
        serde_json::json!({"film_render":{"grain":{"v2_amount":null}}})
    );
    let reloaded = Preset::from_json_str(&exported).unwrap();
    assert_eq!(
        reloaded.kind().unwrap(),
        PresetKind::Section(Section::Grain)
    );
    let error = preset.to_toml().unwrap_err();
    assert!(error.contains("film_render.grain.v2_amount"));
    assert!(error.contains("--format json"));
    let mut current = RuntimeParams::default();
    current.film_render.grain.v2_amount = Some(42.0);
    let applied = preset.apply_section(&current, &test_film()).unwrap();
    assert_eq!(applied.film_render.grain.v2_amount, None);
    assert!(section_matches(&preset, &applied).unwrap());
    let temp = Temp::new();
    let source = temp.0.join("external.json");
    std::fs::write(&source, exported).unwrap();
    let lib = PresetLibrary::new(temp.0.join("presets"));
    let path = lib.import(&source, None, &data()).unwrap();
    assert_eq!(path.extension().unwrap(), "json");
    assert_eq!(load_file(&path).unwrap(), preset);
}

#[test]
fn parity_only_film_glare_is_rejected_as_unowned() {
    let payload = serde_json::json!({"film_render":{"glare":{"active":true}}});
    let error = section_from_payload(&payload).unwrap_err();
    assert!(error.contains("film_render.glare"), "{error}");
    // Built-in documents no longer carry the never-read group, so a captured
    // grain/print section cannot acquire it and the section table stays clean.
    for preset in builtin_presets() {
        let value = serde_json::to_value(preset.look_parameters().unwrap()).unwrap();
        assert!(
            value["film_render"].get("glare").is_none(),
            "{} carries the non-effective film glare group",
            preset.name
        );
    }
    // Print glare stays effective and preset-owned.
    let mut document = serde_json::to_value(builtin_presets().remove(0)).unwrap();
    document["parameters"]["print_render"]["glare"]["percent"] = 0.25.into();
    let look = Preset::from_json_str(&document.to_string()).unwrap();
    assert_eq!(
        look.look_parameters().unwrap().print_render.glare.percent,
        0.25
    );
}

#[test]
fn builtin_look_documents_do_not_carry_unowned_leaves() {
    // Non-effective, derived, and calibration-only leaves stay out of preset
    // ownership, so a compiled document cannot publish a hidden surface.
    const UNOWNED: &[(&str, &[&str])] = &[
        ("film_render", &["glare"]),
        ("enlarger", &["lens_blur"]),
        (
            "film_render.grain",
            &["micro_sublayers", "monochrome", "particle_scale_layers"],
        ),
    ];
    for preset in builtin_presets() {
        let value = serde_json::to_value(preset.look_parameters().unwrap()).unwrap();
        for (group, fields) in UNOWNED {
            if let Some(object) = group
                .split('.')
                .try_fold(&value, |current, key| current.get(key))
                .and_then(Value::as_object)
            {
                for field in *fields {
                    assert!(
                        !object.contains_key(*field),
                        "{} carries {group}.{field}",
                        preset.name
                    );
                }
            }
        }
        // Effective runtime leaves the document still owns are unchanged.
        let grain = &value["film_render"]["grain"];
        for owned in [
            "active",
            "engine",
            "sublayers_active",
            "particle_area_um2",
            "particle_scale",
            "n_sub_layers",
        ] {
            assert!(
                grain.get(owned).is_some(),
                "{} lost effective grain leaf {owned}",
                preset.name
            );
        }
    }
}

#[test]
fn preparation_state_roundtrip_restores_explicit_values_and_drops_stale_paths() {
    use crate::params::RenderPreparationState;
    let mut current = RuntimeParams::default();
    current.film_render.grain.rms_granularity = [1.234; 3];
    current.enlarger.c_filter_neutral = 12.0;
    current.neutral_print_filters_protected = Some([true, false, true]);
    current
        .explicit_section_paths
        .insert("film_render.grain.rms_granularity".into());
    // Editable exposure values must survive preparation-state restoration.
    current
        .explicit_section_paths
        .insert("camera.exposure_compensation_ev".into());
    let state = current.render_preparation_state();
    let mut state: RenderPreparationState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(state, current.render_preparation_state());
    state
        .explicit_section_paths
        .push("film_render.grain.removed_control".into());
    state
        .explicit_section_paths
        .push("film_render.grain".into());
    let serialized = serde_json::to_value(&current).unwrap();
    assert!(serialized.get("explicit_section_paths").is_none());
    // The protection flags persist in the parameter document so replaying a
    // resolved document reproduces its render.
    assert!(serialized.get("neutral_print_filters_protected").is_some());
    let mut restored: RuntimeParams = serde_json::from_value(serialized.clone()).unwrap();
    restored.restore_render_preparation_state(&state);
    assert_eq!(
        restored.render_preparation_state(),
        current.render_preparation_state()
    );
    assert_eq!(serde_json::to_value(&restored).unwrap(), serialized);
    let film = profile::load_profile_by_name(&data(), "kodak_portra_400").unwrap();
    let print = profile::load_profile_by_name(&data(), "kodak_portra_endura").unwrap();
    apply_film_specifics(&mut restored, &film);
    assert_eq!(restored.film_render.grain.rms_granularity, [1.234; 3]);
    let database = crate::neutral_filters::NeutralFilters::load(&data()).unwrap();
    let restored = crate::params_builder::apply_database_neutral_print_filters(
        restored,
        &film,
        &print,
        Some(&database),
    );
    assert_eq!(restored.enlarger.c_filter_neutral, 12.0);
    assert_eq!(
        restored.neutral_print_filters_protected,
        Some([true, false, true])
    );
    assert_eq!(
        restored.settings.neutral_print_filters_from_database,
        current.settings.neutral_print_filters_from_database
    );
}

#[test]
fn captured_optional_section_controls_restore_unset_and_set_values() {
    let mut current = RuntimeParams::default();
    current.film_render.grain.v2_amount = None;
    let unset =
        Preset::capture_section("Unset grain", Section::Grain, &current, provenance()).unwrap();
    let json = unset.to_json().unwrap();
    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        value.pointer("/parameters/film_render/grain/v2_amount"),
        Some(&Value::Null)
    );
    let unset = Preset::from_json_str(&json).unwrap();
    current.film_render.grain.v2_amount = Some(42.0);
    let applied = unset.apply_section(&current, &test_film()).unwrap();
    assert_eq!(applied.film_render.grain.v2_amount, None);
    assert_eq!(
        Preset::capture_section("Unset grain", Section::Grain, &applied, provenance()).unwrap(),
        unset
    );
    let set = Preset::capture_section("Set grain", Section::Grain, &current, provenance()).unwrap();
    assert_eq!(
        set.apply_section(&applied, &test_film())
            .unwrap()
            .film_render
            .grain
            .v2_amount,
        Some(42.0)
    );
    for section in [Section::Film, Section::Print] {
        let captured =
            Preset::capture_section(section.name(), section, &applied, provenance()).unwrap();
        let mut changed = applied.clone();
        changed.film_render.development_time = Some(8.0);
        changed.print_render.development_time = Some(8.0);
        let restored = captured.apply_section(&changed, &test_film()).unwrap();
        let development_time = if section == Section::Film {
            restored.film_render.development_time
        } else {
            restored.print_render.development_time
        };
        assert_eq!(development_time, None);
    }
}

#[test]
fn section_matching_compares_typed_carried_values() {
    for (payload, mismatch) in [
        (
            serde_json::json!({"film_render":{"grain":{"v2_amount":25}}}),
            serde_json::json!({"film_render":{"grain":{"v2_amount":26}}}),
        ),
        (
            serde_json::json!({"film_render":{"halation":{"boost_ev":0.1}}}),
            serde_json::json!({"film_render":{"halation":{"boost_ev":0.2}}}),
        ),
        (
            serde_json::json!({"film_render":{"grain":{"v2_amount":null}}}),
            serde_json::json!({"film_render":{"grain":{"v2_amount":26}}}),
        ),
    ] {
        let preset = Preset::from_json_str(
            &serde_json::json!({
                "schema_version":2,"name":"Typed section","parameters":payload,
                "provenance":provenance()
            })
            .to_string(),
        )
        .unwrap();
        let applied = preset
            .apply_section(&RuntimeParams::default(), &test_film())
            .unwrap();
        assert!(section_matches(&preset, &applied).unwrap());
        let mut different = applied;
        parameter_edits(&mismatch)
            .unwrap()
            .apply(&mut different)
            .unwrap();
        assert!(!section_matches(&preset, &different).unwrap());
    }
}

#[test]
fn look_resolution_keeps_a_declared_film_scan_route_and_its_scan_mode() {
    let mut current = RuntimeParams::default();
    current.workflow.route = "input > film > scan".into();
    current.io.scan_film = true;
    current.scanner.scan_output = "negative_to_positive".into();
    for (stock, expected_mode) in [
        ("kodak_ektachrome_100", "direct_scan"),
        ("kodak_portra_400", "negative_to_positive"),
    ] {
        let preset = Preset::capture(
            "Scan look",
            stock,
            "kodak_portra_endura",
            &current,
            provenance(),
        );
        let resolved = preset.resolve(&data(), &current).unwrap();
        assert_eq!(resolved.params.scanner.scan_output, expected_mode);
        assert_eq!(resolved.params.workflow.route, "input > film > scan");
    }
}

#[test]
fn look_resolution_normalizes_scan_mode_without_a_declared_film_scan_route() {
    let mut current = RuntimeParams::default();
    current.scanner.scan_output = "negative_to_positive".into();
    let preset = Preset::capture(
        "Scan look",
        "kodak_portra_400",
        "kodak_portra_endura",
        &current,
        provenance(),
    );
    let resolved = preset.resolve(&data(), &current).unwrap();
    assert_eq!(resolved.params.scanner.scan_output, "direct_scan");
    assert_eq!(
        resolved.params.workflow.route,
        "input > film > print > scan"
    );
}

#[test]
fn section_application_rejects_new_invalid_scan_route_but_allows_legacy_state() {
    let preset = Preset::from_json_str(r#"{"schema_version":2,"name":"Negative conversion","parameters":{"scanner":{"scan_output":"negative_to_positive"}},"provenance":{"implementation_version":"test","model_version":"test"}}"#).unwrap();
    let mut current = RuntimeParams::default();
    current.workflow.route = "input > film > print > scan".into();
    let error = preset.apply_section(&current, &test_film()).unwrap_err();
    assert!(error.contains("scanner.scan_output=negative_to_positive"));
    assert!(error.contains("direct film scan route"));
    current.scanner.scan_output = "negative_to_positive".into();
    assert_eq!(
        preset
            .apply_section(&current, &test_film())
            .unwrap()
            .scanner
            .scan_output,
        "negative_to_positive"
    );
    let unrelated =
        Preset::capture_section("Halation", Section::Halation, &current, provenance()).unwrap();
    assert!(unrelated.apply_section(&current, &test_film()).is_ok());
    current.workflow.route = "input > film > scan".into();
    current.io.scan_film = true;
    current.scanner.scan_output = "direct_scan".into();
    assert_eq!(
        preset
            .apply_section(&current, &test_film())
            .unwrap()
            .scanner
            .scan_output,
        "negative_to_positive"
    );
}

#[test]
fn section_application_checks_grain_before_commit() {
    let film = test_film();
    let mut current = RuntimeParams::default();
    current.workflow.route = "input > film > scan".into();
    current.io.scan_film = true;
    current.scanner.scan_output = "negative_to_positive".into();
    current.film_render.grain.active = false;
    let section = Preset::from_json_str(
        &serde_json::json!({
            "schema_version":2,"name":"Invalid scan combination",
            "parameters":{"film_render":{"grain":{"active":true,"engine":"procedural"}}},
            "provenance":provenance()
        })
        .to_string(),
    )
    .unwrap();
    let before = current.clone();
    assert!(section.apply_section(&current, &film).is_err());
    assert_eq!(current, before);
    let section = Preset::from_json_str(r#"{"schema_version":2,"name":"Negative conversion","parameters":{"scanner":{"scan_output":"negative_to_positive"}},"provenance":{"implementation_version":"test","model_version":"test"}}"#).unwrap();
    current.scanner.scan_output = "direct_scan".into();
    let positive_film = profile::load_profile_by_name(&data(), "fujifilm_provia_100f").unwrap();
    assert!(section.apply_section(&current, &positive_film).is_err());
    assert_eq!(
        section
            .apply_section(&current, &film)
            .unwrap()
            .scanner
            .scan_output,
        "negative_to_positive"
    );
    // Matching a document never applies it to the active workflow.
    assert!(!section_matches(&section, &RuntimeParams::default()).unwrap());
}

#[test]
fn look_and_section_documents_reject_active_invalid_chemistry() {
    let mut look = builtin_presets().remove(0);
    look.look_parameters_mut()
        .unwrap()
        .film_render
        .chemistry
        .gamma_factor = 0.0;
    assert!(look.to_json().is_err());
    assert!(look.resolve(&data(), &RuntimeParams::default()).is_err());
    for payload in [
        serde_json::json!({"film_render":{"chemistry":{"active":true,"gamma_factor":0.0}}}),
        serde_json::json!({"print_render":{"density_curves_morph":{"active":true,"developer_exhaustion":2.0}}}),
    ] {
        let text = serde_json::json!({"schema_version":2,"name":"Invalid chemistry","parameters":payload,"provenance":provenance()}).to_string();
        assert!(Preset::from_json_str(&text).is_err());
    }
}

/// A look document at a legacy preset version, carrying the required look
/// groups from a built-in plus the supplied legacy extras. `settings` is
/// dropped because look documents carry only the look settings subset, and
/// `cmyk` is dropped because the caller supplies the legacy spelling.
fn legacy_cmyk_look(version: u32, parameters: serde_json::Map<String, Value>) -> Value {
    let mut value = serde_json::to_value(builtin_presets().remove(0)).unwrap();
    let object = value.as_object_mut().unwrap();
    object.insert("schema_version".into(), version.into());
    let mut look = serde_json::Map::new();
    for (key, value) in object["parameters"].as_object().unwrap() {
        if !matches!(key.as_str(), "cmyk" | "settings") {
            look.insert(key.clone(), value.clone());
        }
    }
    // Camera and output mix postdate the v1/v2 whole-look surface.
    if version <= 2 {
        look.remove("camera");
        look.remove("output_mix");
    }
    look.extend(parameters);
    object.insert("parameters".into(), Value::Object(look));
    value
}

#[test]
fn legacy_looks_migrate_the_retired_magazine_group_onto_canonical_cmyk() {
    for version in [1, 2, 3, 4] {
        let legacy = serde_json::json!({
            "magazine_print_color": {"active": true, "strength": 0.75}
        });
        let document = legacy_cmyk_look(version, legacy.as_object().unwrap().clone());
        let imported = Preset::from_json_str(&document.to_string()).unwrap();
        assert_eq!(imported.schema_version, PRESET_SCHEMA_VERSION);
        let look = imported.look_parameters().unwrap();
        assert!(look.cmyk.active);
        assert_eq!(look.cmyk.strength, 0.75);
        let exported = serde_json::from_str::<Value>(&imported.to_json().unwrap()).unwrap();
        assert_eq!(exported["parameters"]["cmyk"]["active"], true);
        assert_eq!(exported["parameters"]["cmyk"]["strength"], 0.75);
        assert!(exported["parameters"].get("magazine_print_color").is_none());
        // Resolving keeps the migrated ink style and the caller's io settings.
        let mut caller = RuntimeParams::default();
        caller.io.output_color_space = "ProPhoto RGB".into();
        let resolved = imported.resolve(&data(), &caller).unwrap();
        assert!(resolved.params.cmyk.active);
        assert_eq!(resolved.params.cmyk.strength, 0.75);
        assert_eq!(resolved.params.io.output_color_space, "ProPhoto RGB");
        // Re-importing the exported document is stable.
        let reimported = Preset::from_json_str(&imported.to_json().unwrap()).unwrap();
        assert_eq!(reimported, imported);
    }
}

#[test]
fn looks_never_carry_the_retired_magazine_route() {
    // Presets own look controls only; workflow routing is a caller setting, so
    // a preset document cannot carry the retired route in any schema version.
    // Complete runtime carriers migrate that route at their own boundary.
    for version in [1, 2, 3, 4, 5] {
        let mut value = legacy_cmyk_look(version, serde_json::Map::new());
        value["parameters"]["workflow"] =
            serde_json::json!({"route": "input > film > scan > magazine"});
        let error = Preset::from_json_str(&value.to_string()).unwrap_err();
        assert!(error.contains("workflow"), "{error}");
    }
    // The retired group migrates on every approved legacy version and the
    // exported document carries no workflow routing of its own.
    for version in [1, 2, 3, 4] {
        let legacy =
            serde_json::json!({"magazine_print_color": {"active": true, "strength": 0.75}});
        let imported = Preset::from_json_str(
            &legacy_cmyk_look(version, legacy.as_object().unwrap().clone()).to_string(),
        )
        .unwrap();
        let exported = serde_json::from_str::<Value>(&imported.to_json().unwrap()).unwrap();
        assert!(exported["parameters"].get("workflow").is_none());
        assert_eq!(
            exported["parameters"]["cmyk"]["active"], true,
            "version {version}"
        );
        assert_eq!(exported["parameters"]["cmyk"]["strength"], 0.75);
        let mut caller = RuntimeParams::default();
        caller.io.scan_film = true;
        let resolved = imported.resolve(&data(), &caller).unwrap();
        assert!(resolved.params.cmyk.active);
        assert_eq!(
            resolved.params.workflow.route, caller.workflow.route,
            "the preset must not select a workflow route"
        );
    }
}

#[test]
fn legacy_looks_reject_a_magazine_spelling_beside_its_canonical_counterpart() {
    for version in [1, 2, 3, 4] {
        let mut value = legacy_cmyk_look(version, serde_json::Map::new());
        let parameters = value["parameters"].as_object_mut().unwrap();
        parameters.insert(
            "cmyk".into(),
            serde_json::json!({"active": true, "strength": 1.0}),
        );
        parameters.insert(
            "magazine_print_color".into(),
            serde_json::json!({"active": false, "strength": 0.75}),
        );
        let original = serde_json::to_string(&value).unwrap();
        let error = Preset::from_json_str(&original).unwrap_err();
        assert!(error.contains("magazine_print_color"), "{error}");
        assert!(error.contains("cmyk"), "{error}");
        // A rejected document stays byte-identical: nothing is rewritten.
        assert_eq!(serde_json::to_string(&value).unwrap(), original);
    }
    // The retired route beside an explicit legacy active leaf is a conflict.
    for version in [1, 2, 3, 4] {
        let mut value = legacy_cmyk_look(version, serde_json::Map::new());
        let parameters = value["parameters"].as_object_mut().unwrap();
        parameters.insert(
            "workflow".into(),
            serde_json::json!({"route": "input > film > scan > magazine"}),
        );
        parameters.insert(
            "magazine_print_color".into(),
            serde_json::json!({"active": false}),
        );
        let error = Preset::from_json_str(&value.to_string()).unwrap_err();
        assert!(error.contains("retired route"), "{error}");
    }
}

#[test]
fn schema_five_looks_reject_the_retired_magazine_spelling() {
    let mut value = legacy_cmyk_look(5, serde_json::Map::new());
    value["parameters"]["magazine_print_color"] =
        serde_json::json!({"active": true, "strength": 0.75});
    let error = Preset::from_json_str(&value.to_string()).unwrap_err();
    assert!(error.contains("magazine_print_color"), "{error}");
    assert!(error.contains("was retired"), "{error}");
    assert!(error.contains("cmyk"), "{error}");
    // A document that names no retired field still reports its own fault.
    let clean = Preset::from_json_str(&legacy_cmyk_look(5, serde_json::Map::new()).to_string())
        .unwrap()
        .to_json()
        .unwrap();
    assert!(!clean.contains("magazine_print_color"));
    // Versions beyond the current schema are rejected outright.
    let mut future = serde_json::from_str::<Value>(&clean).unwrap();
    future["schema_version"] = 6.into();
    let error = Preset::from_json_str(&future.to_string()).unwrap_err();
    assert!(
        error.contains("unsupported preset schema version"),
        "{error}"
    );
    // The retired route is not a current-schema spelling either.
    let mut value = legacy_cmyk_look(5, serde_json::Map::new());
    value["parameters"]["workflow"] =
        serde_json::json!({"route": "input > film > scan > magazine"});
    assert!(Preset::from_json_str(&value.to_string()).is_err());
}

#[test]
fn migrated_legacy_ink_style_migration_is_idempotent_at_the_preset_boundary() {
    let legacy = serde_json::json!({"magazine_print_color": {"active": true, "strength": 0.4}});
    let document = legacy_cmyk_look(4, legacy.as_object().unwrap().clone());
    let imported = Preset::from_json_str(&document.to_string()).unwrap();
    let exported = serde_json::to_value(&imported).unwrap();
    // Replaying the migrated document as schema 4 must be a no-op.
    let mut replayed = exported.clone();
    replayed["schema_version"] = 4.into();
    let again = Preset::from_json_str(&replayed.to_string()).unwrap();
    assert_eq!(serde_json::to_value(&again).unwrap(), exported);
}

#[test]
fn builtin_schema_five_looks_decode_canonical_cmyk_fields_only() {
    for preset in builtin_presets() {
        assert_eq!(preset.schema_version, PRESET_SCHEMA_VERSION);
        let exported = serde_json::to_value(&preset).unwrap();
        let parameters = exported["parameters"].as_object().unwrap();
        assert!(parameters.get("magazine_print_color").is_none());
        let cmyk = parameters
            .get("cmyk")
            .unwrap_or_else(|| panic!("{} carries no cmyk group", preset.name));
        assert!(cmyk.get("active").is_some(), "{}", preset.name);
        assert!(cmyk.get("strength").is_some(), "{}", preset.name);
        let reimported = Preset::from_json_str(&preset.to_json().unwrap()).unwrap();
        assert_eq!(reimported, preset);
    }
}

#[test]
fn legacy_v1_v2_looks_gain_mix_and_migrate_ink_style_together() {
    for version in [1, 2] {
        let mut value = legacy_cmyk_look(
            version,
            serde_json::json!({"magazine_print_color": {"active": true, "strength": 0.75}})
                .as_object()
                .unwrap()
                .clone(),
        );
        // V1 carries the old id, which migration must discard.
        if version == 1 {
            value["id"] = "legacy-id".into();
        }
        let imported = Preset::from_json_str(&value.to_string()).unwrap();
        let look = imported.look_parameters().unwrap();
        // The whole-look migration still supplies the full mix.
        assert_eq!(look.output_mix.amount, 1.0);
        // The retired ink group lands on the canonical name in the same pass.
        assert!(look.cmyk.active);
        assert_eq!(look.cmyk.strength, 0.75);
        // V1's id is gone and material controls survive the migration.
        assert!(serde_json::to_value(&imported).unwrap().get("id").is_none());
        let exported = serde_json::from_str::<Value>(&imported.to_json().unwrap()).unwrap();
        assert_eq!(exported["parameters"]["output_mix"]["amount"], 1.0);
        assert!(exported["parameters"].get("magazine_print_color").is_none());
        assert_eq!(
            exported["parameters"]["enlarger"]["print_exposure_mode"],
            "auto"
        );
    }
}

#[test]
fn cmyk_section_presets_carry_only_the_ink_style_leaves() {
    let mut current = RuntimeParams::default();
    current.cmyk.active = true;
    current.cmyk.strength = 0.625;
    let captured = Preset::capture_section("Ink", Section::Cmyk, &current, provenance()).unwrap();
    assert_eq!(captured.kind().unwrap(), PresetKind::Section(Section::Cmyk));
    let exported = serde_json::from_str::<Value>(&captured.to_json().unwrap()).unwrap();
    assert_eq!(
        exported["parameters"],
        serde_json::json!({"cmyk": {"active": true, "strength": 0.625}})
    );
    let loaded = Preset::from_toml_str(&captured.to_toml().unwrap()).unwrap();
    assert_eq!(loaded, captured);
    // Applying replaces only the ink style and records its explicit paths.
    let mut caller = RuntimeParams::default();
    caller.cmyk.strength = 0.2;
    caller.enlarger.print_exposure = 0.75;
    let applied = loaded.apply_section(&caller, &test_film()).unwrap();
    assert!(applied.cmyk.active);
    assert_eq!(applied.cmyk.strength, 0.625);
    assert_eq!(applied.enlarger.print_exposure, 0.75);
    assert!(section_matches(&loaded, &applied).unwrap());
    // A legacy section spelling migrates only for an approved legacy version.
    let mut legacy = serde_json::to_value(&captured).unwrap();
    legacy["schema_version"] = 4.into();
    legacy["parameters"] =
        serde_json::json!({"magazine_print_color": {"active": true, "strength": 0.625}});
    let migrated = Preset::from_json_str(&legacy.to_string()).unwrap();
    assert_eq!(migrated.kind().unwrap(), PresetKind::Section(Section::Cmyk));
    let applied = migrated
        .apply_section(&RuntimeParams::default(), &test_film())
        .unwrap();
    assert!(applied.cmyk.active);
    assert_eq!(applied.cmyk.strength, 0.625);
    // A current section document rejects the retired spelling.
    let mut current_document = serde_json::to_value(&captured).unwrap();
    current_document["parameters"] =
        serde_json::json!({"magazine_print_color": {"active": true, "strength": 0.625}});
    assert!(Preset::from_json_str(&current_document.to_string()).is_err());
    // A legacy section that never carried the feature does not acquire `cmyk`.
    let mut plain = serde_json::to_value(&captured).unwrap();
    plain["schema_version"] = 4.into();
    plain["parameters"] = serde_json::json!({"enlarger": {"print_exposure": 0.5}});
    let untouched = Preset::from_json_str(&plain.to_string()).unwrap();
    let applied = untouched
        .apply_section(&RuntimeParams::default(), &test_film())
        .unwrap();
    assert!(!applied.cmyk.active);
    assert_eq!(applied.cmyk.strength, 1.0);
}
