use super::*;
fn data() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
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
fn compiled_assets_match_original_golden() {
    let golden: Vec<Value> =
        serde_json::from_str(include_str!("../fixtures/preset_golden.json")).unwrap();
    let mut names = std::collections::BTreeSet::new();
    assert_eq!(BUILTIN_ASSETS.len(), 5);
    for (stem, text) in BUILTIN_ASSETS {
        let p = Preset::from_toml_str(text).unwrap();
        assert_eq!(&p.name, stem);
        assert!(names.insert(p.name.to_lowercase()));
        let g = golden.iter().find(|g| g["name"] == p.name).unwrap();
        let r = p.resolve(&data(), &RuntimeParams::default()).unwrap();
        let actual = LookParameters::capture(&r.params);
        let expected: LookParameters = serde_json::from_value(g["parameters"].clone()).unwrap();
        assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        let film: ProfileRef = serde_json::from_value(g["film_profile"].clone()).unwrap();
        let print: ProfileRef = serde_json::from_value(g["print_profile"].clone()).unwrap();
        assert_eq!(p.film_profile.as_ref(), Some(&film));
        assert_eq!(p.print_profile.as_ref(), Some(&print));
        assert_eq!(
            serde_json::to_value(&p.provenance).unwrap(),
            g["provenance"]
        );
        assert_eq!(r.default_route, g["default_route"]);
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
    let params = RuntimeParams::default();
    for section in SECTIONS {
        let p = Preset::capture_section(section.name(), *section, &params, provenance()).unwrap();
        assert_eq!(p.kind().unwrap(), PresetKind::Section(*section));
        let json = p.to_json().unwrap();
        let q = Preset::from_json_str(&json).unwrap();
        assert_eq!(p, q);
        let toml = p.to_toml().unwrap();
        let q = Preset::from_toml_str(&toml).unwrap();
        let actual = p.apply_section(&params).unwrap();
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
    let applied = a.apply_section(&RuntimeParams::default()).unwrap();
    let h = Preset::capture_section("Halation", Section::Halation, &applied, provenance()).unwrap();
    let mut applied = h.apply_section(&applied).unwrap();
    let film = profile::load_profile_by_name(&data(), "kodak_gold_200").unwrap();
    apply_film_specifics(&mut applied, &film);
    assert_eq!(applied.film_render.grain.rms_granularity, [1.234; 3]);
}
#[test]
fn library_faults_conflicts_shadowing_and_management() {
    let temp = Temp::new();
    let lib = PresetLibrary::new(temp.0.join("presets"));
    assert_eq!(lib.scan().unwrap().len(), 5);
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
    let applied = preset.apply_section(&current).unwrap();
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
    let applied = preset.apply_section(&current).unwrap();
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
