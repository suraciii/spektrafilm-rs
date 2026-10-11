use super::*;
fn data() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}
fn test_film() -> Profile {
    profile::load_profile_by_name(&data(), "kodak_portra_400").unwrap()
}
fn provenance() -> Provenance {
    Provenance {
        implementation_version: "test".into(),
        model_version: "test".into(),
    }
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
    assert_eq!(p.schema_version, 2);
    assert_eq!(p.name, "Portable");
    assert!(!p.to_json().unwrap().contains("\"id\""));
    assert!(p.to_toml().unwrap().contains("y_filter_neutral"));
    let mut v = serde_json::to_value(p).unwrap();
    v["id"] = "invalid".into();
    assert!(Preset::from_json_str(&v.to_string()).is_err());
    v.as_object_mut().unwrap().remove("id");
    v["route"] = "input".into();
    assert!(Preset::from_json_str(&v.to_string()).is_err());
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
        "\"schema_version\": 2",
        "\"schema_version\": 2, \"schema_version\": 2",
        1,
    );
    assert!(Preset::from_json_str(&text).is_err());
    let mut v = serde_json::to_value(p).unwrap();
    v["parameters"]["enlarger"]["print_exposure"] = serde_json::json!(1e100);
    assert!(Preset::from_json_str(&v.to_string()).is_err());
}
#[test]
fn sparse_section_roundtrip_and_database_protection() {
    let document = serde_json::json!({"schema_version":2,"name":"Neutral cyan","parameters":{"enlarger":{"c_filter_neutral":12.0}},"provenance":{"implementation_version":"test","model_version":"test"}});
    let preset = Preset::from_json_str(&document.to_string()).unwrap();
    assert_eq!(
        preset.kind().unwrap(),
        PresetKind::Section(Section::Enlarger)
    );
    assert_eq!(
        serde_json::to_value(&preset).unwrap()["parameters"],
        document["parameters"]
    );
    assert_eq!(
        Preset::from_toml_str(&preset.to_toml().unwrap()).unwrap(),
        preset
    );
    let mut current = RuntimeParams::default();
    current.enlarger.m_filter_neutral = 34.0;
    current.neutral_print_filters_protected = [false, true, false];
    let applied = preset.apply_section(&current, &test_film()).unwrap();
    assert_eq!(applied.enlarger.m_filter_neutral, 34.0);
    assert_eq!(applied.neutral_print_filters_protected, [true, true, false]);
    let film = profile::load_profile_by_name(&data(), "kodak_portra_400").unwrap();
    let print = profile::load_profile_by_name(&data(), "kodak_portra_endura").unwrap();
    let database = crate::neutral_filters::NeutralFilters::load(&data()).unwrap();
    let prepared = crate::params_builder::apply_database_neutral_print_filters(
        applied,
        &film,
        &print,
        Some(&database),
    );
    assert_eq!(prepared.enlarger.c_filter_neutral, 12.0);
    assert_eq!(prepared.enlarger.m_filter_neutral, 34.0);
    assert!(prepared.settings.neutral_print_filters_from_database);
}
#[test]
fn explicit_null_is_json_only_and_never_discarded() {
    let text = r#"{"schema_version":2,"name":"Clear amount","parameters":{"film_render":{"grain":{"v2_amount":null}}},"provenance":{"implementation_version":"test","model_version":"test"}}"#;
    let preset = Preset::from_json_str(text).unwrap();
    let exported = preset.to_json().unwrap();
    assert_eq!(
        serde_json::to_vec(&serde_json::from_str::<Value>(text).unwrap()).unwrap(),
        serde_json::to_vec(&serde_json::from_str::<Value>(&exported).unwrap()).unwrap()
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
fn parity_only_film_glare_has_no_section() {
    assert_eq!(SECTIONS.len(), 11);
    let error = section_from_payload(&serde_json::json!({"film_render":{"glare":{"active":true}}}))
        .unwrap_err();
    assert!(error.contains("exactly one section"));
}

#[test]
fn preparation_state_roundtrip_restores_explicit_values_and_drops_stale_paths() {
    use crate::params::RenderPreparationState;
    let mut current = RuntimeParams::default();
    current.film_render.grain.rms_granularity = [1.234; 3];
    current.enlarger.c_filter_neutral = 12.0;
    current.neutral_print_filters_protected = [true, false, true];
    current
        .explicit_section_paths
        .insert("film_render.grain.rms_granularity".into());
    // A valid editable path outside the section table must survive restoration.
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
    assert!(serialized.get("neutral_print_filters_protected").is_none());
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
        [true, false, true]
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
fn look_resolution_normalizes_scan_output_for_film_and_route() {
    let mut current = RuntimeParams::default();
    current.workflow.route = "input > film > scan".into();
    current.io.scan_film = true;
    current.scanner.scan_output = "positive_scan".into();
    for stock in ["kodak_ektachrome_100", "kodak_portra_400"] {
        let preset = Preset::capture(
            "Scan look",
            stock,
            "kodak_portra_endura",
            &current,
            provenance(),
        );
        let resolved = preset.resolve(&data(), &current).unwrap();
        assert_eq!(resolved.params.scanner.scan_output, "direct_scan");
        if resolved.film.is_positive() {
            assert_eq!(resolved.params.workflow.route, "input > film > scan");
        } else {
            assert_eq!(
                resolved.params.workflow.route,
                "input > film > print > scan"
            );
        }
    }
}

#[test]
fn section_application_rejects_new_invalid_scan_route_but_allows_legacy_state() {
    let preset = Preset::from_json_str(r#"{"schema_version":2,"name":"Positive scan","parameters":{"scanner":{"scan_output":"positive_scan"}},"provenance":{"implementation_version":"test","model_version":"test"}}"#).unwrap();
    let mut current = RuntimeParams::default();
    current.workflow.route = "input > film > print > scan".into();
    let error = preset.apply_section(&current, &test_film()).unwrap_err();
    assert!(error.contains("scanner.scan_output=positive_scan"));
    assert!(error.contains("direct film scan route"));
    current.scanner.scan_output = "positive_scan".into();
    assert_eq!(
        preset
            .apply_section(&current, &test_film())
            .unwrap()
            .scanner
            .scan_output,
        "positive_scan"
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
        "positive_scan"
    );
}

#[test]
fn section_application_checks_film_corrections_and_grain_before_commit() {
    let film = test_film();
    let mut current = RuntimeParams::default();
    current.workflow.route = "input > film > scan".into();
    current.io.scan_film = true;
    current.scanner.scan_output = "positive_scan".into();
    current.scanner.white_correction = false;
    current.scanner.black_correction = false;
    current.film_render.grain.active = false;
    for payload in [
        serde_json::json!({"film_render":{"grain":{"active":true,"engine":"v2"}}}),
        serde_json::json!({"scanner":{"white_correction":true}}),
        serde_json::json!({"scanner":{"black_correction":true}}),
    ] {
        let section = Preset::from_json_str(
            &serde_json::json!({
                "schema_version":2,"name":"Invalid scan combination","parameters":payload,
                "provenance":provenance()
            })
            .to_string(),
        )
        .unwrap();
        let before = current.clone();
        assert!(section.apply_section(&current, &film).is_err());
        assert_eq!(current, before);
    }
    let section = Preset::from_json_str(r#"{"schema_version":2,"name":"Positive scan","parameters":{"scanner":{"scan_output":"positive_scan"}},"provenance":{"implementation_version":"test","model_version":"test"}}"#).unwrap();
    current.scanner.scan_output = "direct_scan".into();
    let positive_film = profile::load_profile_by_name(&data(), "fujifilm_provia_100f").unwrap();
    assert!(section.apply_section(&current, &positive_film).is_err());
    assert_eq!(
        section
            .apply_section(&current, &film)
            .unwrap()
            .scanner
            .scan_output,
        "positive_scan"
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
