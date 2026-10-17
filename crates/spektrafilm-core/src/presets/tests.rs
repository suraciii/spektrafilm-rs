use super::*;
use crate::test_support::data_dir as data;
fn provenance() -> Provenance {
    Provenance {
        implementation_version: "test".into(),
        model_version: "test".into(),
        scanner: None,
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
    assert_eq!(p.schema_version, 4);
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
fn version_three_looks_rename_conversion_paths_and_map_the_exposure_pair() {
    let legacy_scanner = serde_json::json!({
        "positive_interpretation": "separable",
        "positive_density_ev": 0.25,
        "positive_balance_ev": [0.05, 0.0, -0.05],
        "positive_contrast": 0.8,
        "positive_dmax": 1.4,
    });
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
        document["parameters"]["scanner"] = legacy_scanner.clone();
        document["parameters"]["enlarger"]
            .as_object_mut()
            .unwrap()
            .extend(pair.as_object().unwrap().clone());
        let migrated = Preset::from_json_str(&document.to_string()).unwrap();
        assert_eq!(migrated.schema_version, PRESET_SCHEMA_VERSION);
        let look = migrated.look_parameters();
        assert_eq!(
            look.scanner.negative_conversion_method.as_str(),
            "separable"
        );
        assert_eq!(look.scanner.conversion_brightness_ev, 0.25);
        assert_eq!(look.scanner.conversion_balance_ev, [0.05, 0.0, -0.05]);
        assert_eq!(look.scanner.conversion_contrast, 0.8);
        assert_eq!(look.scanner.conversion_dmax, 1.4);
        assert_eq!(look.enlarger.print_exposure_mode.as_str(), mode);
        assert_eq!(
            serde_json::to_value(&look.settings).unwrap()["print_exposure_ev_tracking"],
            tracking
        );
        // A v4 document must not carry the legacy spelling.
        let mut current = serde_json::to_value(&migrated).unwrap();
        current["parameters"]["scanner"]["positive_density_ev"] = 0.5.into();
        assert!(Preset::from_json_str(&current.to_string()).is_err());
        // The migrated whole look renames its conversion paths in place, so no
        // legacy spelling survives anywhere in the document.
        let migrated_text = migrated.to_json().unwrap();
        assert!(migrated_text.contains("conversion_brightness_ev"));
        assert!(!migrated_text.contains("positive_"));
    }
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
        assert_eq!(migrated.schema_version, 4);
        assert_eq!(migrated.look_parameters().output_mix.amount, 1.0);
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
        assert_eq!(exported["schema_version"], 4);
        assert!(exported["parameters"].get("io").is_none());
        document["parameters"]["io"] = serde_json::json!({});
        assert!(Preset::from_json_str(&document.to_string()).is_err());
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
    p.look_parameters_mut().film_render.chemistry.gamma_factor = 1.2345678901234567;
    assert_eq!(Preset::from_toml_str(&p.to_toml().unwrap()).unwrap(), p);
    assert_eq!(Preset::from_json_str(&p.to_json().unwrap()).unwrap(), p);
    let text = p.to_json().unwrap().replacen(
        "\"schema_version\": 4",
        "\"schema_version\": 4, \"schema_version\": 4",
        1,
    );
    assert!(Preset::from_json_str(&text).is_err());
    let mut v = serde_json::to_value(p).unwrap();
    v["parameters"]["enlarger"]["print_exposure"] = serde_json::json!(1e100);
    assert!(Preset::from_json_str(&v.to_string()).is_err());
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
fn look_resolution_normalizes_approximate_receiver_for_default_paper_route() {
    let mut current = RuntimeParams::default();
    current.scanner.receiver_profile = crate::params::ReceiverProfile::LabScanApprox;
    current.scanner.finish = crate::params::ScannerFinish::LabScan;
    current.scanner.finish_strength = 0.37;
    current.scanner.scan_output = "negative_to_positive".into();
    let preset = Preset::capture(
        "Approximate scan look",
        "kodak_portra_400",
        "kodak_portra_endura",
        &current,
        provenance(),
    );

    let resolved = preset.resolve(&data(), &RuntimeParams::default()).unwrap();
    assert_eq!(
        resolved.params.workflow.route,
        "input > film > print > scan"
    );
    assert_eq!(
        resolved.params.scanner.receiver_profile,
        crate::params::ReceiverProfile::LegacyColorimetric
    );
    assert_eq!(
        resolved.params.scanner.finish,
        crate::params::ScannerFinish::Neutral
    );
    assert_eq!(resolved.params.scanner.finish_strength, 0.37);
}

#[test]
fn look_resolution_preserves_default_lab_scan_strength_on_explicit_film_scan_route() {
    let mut current = RuntimeParams::default();
    current.scanner.receiver_profile = crate::params::ReceiverProfile::LabScanApprox;
    current.scanner.finish = crate::params::ScannerFinish::LabScan;
    current.scanner.scan_output = "negative_to_positive".into();
    let preset = Preset::capture(
        "Lab scan look",
        "kodak_portra_400",
        "kodak_portra_endura",
        &current,
        provenance(),
    );

    let presets = [preset];
    let resolved = crate::process_params::resolve(
        crate::process_params::ProcessParamsRequest {
            film: None,
            paper: None,
            presets: &presets,
            params_file: None,
            sources: &["scanner.receiver_profile=lab_scan_approx,scanner.finish=lab_scan".into()],
            route: Some("input > film > scan"),
            scan_film: true,
            scan_output: Some("negative_to_positive"),
            input_is_raw: false,
            input_color_space: Some("sRGB"),
            input_cctf_decoding: Some(false),
            require_input_declaration: true,
            digest_mode: crate::runtime::DigestMode::ApplyStockSpecifics,
        },
        &data(),
    )
    .unwrap();
    assert_eq!(resolved.params.workflow.route, "input > film > scan");
    assert_eq!(
        resolved.params.scanner.receiver_profile,
        crate::params::ReceiverProfile::LabScanApprox
    );
    assert_eq!(
        resolved.params.scanner.finish,
        crate::params::ScannerFinish::LabScan
    );
    assert_eq!(resolved.params.scanner.finish_strength, 1.0);
}
