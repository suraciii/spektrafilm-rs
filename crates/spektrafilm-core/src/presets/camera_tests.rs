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
        scanner: None,
    }
}

fn captured_look(params: &RuntimeParams) -> Preset {
    Preset::capture(
        "Camera ownership",
        "kodak_portra_400",
        "kodak_portra_endura",
        params,
        provenance(),
    )
}

fn exposure_params() -> RuntimeParams {
    let mut params = RuntimeParams::default();
    params.camera.exposure_compensation_ev = 1.25;
    params.camera.auto_exposure = false;
    params.camera.auto_exposure_method = "median".into();
    params
}

fn current_params() -> RuntimeParams {
    let mut params = RuntimeParams::default();
    params.camera.exposure_compensation_ev = -0.75;
    params.camera.auto_exposure = true;
    params.camera.auto_exposure_method = "average".into();
    params.camera.lens_blur_um = 12.0;
    params.camera.film_format_mm = 60.0;
    params.camera.filter_uv = [0.2, 405.0, 10.0];
    params.camera.filter_ir = [0.3, 680.0, 20.0];
    params.camera.color_filter = "hoya_r1".into();
    params.camera.diffusion_filter.active = true;
    params.camera.diffusion_filter.strength = 0.25;
    params.io.input_color_space = "sRGB".into();
    params.io.input_cctf_decoding = true;
    params.io.crop = true;
    params.io.crop_center = [0.4, 0.6];
    params.io.crop_size = [0.7, 0.8];
    params.io.upscale_factor = 1.5;
    params
}

fn carrier_roundtrips(preset: &Preset) -> [Preset; 2] {
    [
        Preset::from_json_str(&preset.to_json().unwrap()).unwrap(),
        Preset::from_toml_str(&preset.to_toml().unwrap()).unwrap(),
    ]
}

#[test]
fn captured_exposure_roundtrips_and_restores_only_owned_camera_controls() {
    let source = exposure_params();
    let preset = captured_look(&source);
    let expected: Option<LookCameraParams> = Some(LookCameraParams {
        exposure_compensation_ev: source.camera.exposure_compensation_ev,
        auto_exposure: source.camera.auto_exposure,
        auto_exposure_method: source.camera.auto_exposure_method.clone(),
    });
    assert_eq!(preset.look_parameters().unwrap().camera, expected);

    let current = current_params();
    for imported in carrier_roundtrips(&preset) {
        let look = imported.look_parameters().unwrap();
        assert_eq!(look.camera, expected);
        assert!(!look.matches(&current));

        let mut applied = current.clone();
        look.apply_to(&mut applied);
        let mut expected_camera = current.camera.clone();
        expected_camera.exposure_compensation_ev = source.camera.exposure_compensation_ev;
        expected_camera.auto_exposure = source.camera.auto_exposure;
        expected_camera.auto_exposure_method = source.camera.auto_exposure_method.clone();
        // Diffusion is independently carried by a full look.
        expected_camera.diffusion_filter = source.camera.diffusion_filter.clone();
        assert_eq!(applied.camera, expected_camera);
        assert_eq!(applied.io, current.io);
        assert!(look.matches(&applied));

        let resolved = imported.resolve(&data(), &current).unwrap();
        assert_eq!(resolved.params.camera, expected_camera);
        assert_eq!(resolved.params.io, current.io);
        assert!(look.matches(&resolved.params));
    }
}

#[test]
fn captured_look_matching_checks_each_owned_exposure_control() {
    let source = exposure_params();
    let look = LookParameters::capture(&source);
    assert!(look.matches(&source));

    let mut changed = source.clone();
    changed.camera.exposure_compensation_ev += 0.5;
    assert!(!look.matches(&changed));
    let mut changed = source.clone();
    changed.camera.auto_exposure = !changed.camera.auto_exposure;
    assert!(!look.matches(&changed));
    let mut changed = source.clone();
    changed.camera.auto_exposure_method = "average".into();
    assert!(!look.matches(&changed));
    let mut changed = source.clone();
    changed.output_mix.amount = 0.5;
    assert!(!look.matches(&changed));

    let mut changed = source.clone();
    changed.camera.lens_blur_um = 15.0;
    changed.camera.film_format_mm = 70.0;
    changed.camera.filter_uv = [0.1, 400.0, 12.0];
    changed.camera.filter_ir = [0.1, 690.0, 12.0];
    changed.camera.color_filter = "hoya_r1".into();
    changed.io.crop = true;
    assert!(look.matches(&changed));
    changed.camera.diffusion_filter.active = !changed.camera.diffusion_filter.active;
    assert!(!look.matches(&changed));
}

#[test]
fn legacy_schema_versions_preserve_exposure_and_export_camera_absence() {
    let source = exposure_params();
    let mut document = serde_json::to_value(captured_look(&source)).unwrap();
    document["parameters"]
        .as_object_mut()
        .unwrap()
        .remove("camera");
    let current = current_params();

    for version in [1, 2, PRESET_SCHEMA_VERSION] {
        let mut document = document.clone();
        if version < PRESET_SCHEMA_VERSION {
            document["parameters"]
                .as_object_mut()
                .unwrap()
                .remove("output_mix");
        }
        document["schema_version"] = version.into();
        let imported = Preset::from_json_str(&document.to_string()).unwrap();
        assert_eq!(imported.look_parameters().unwrap().camera, None);
        for legacy in carrier_roundtrips(&imported) {
            let look = legacy.look_parameters().unwrap();
            assert_eq!(look.camera, None);
            let exported: Value = serde_json::from_str(&legacy.to_json().unwrap()).unwrap();
            assert!(exported["parameters"].get("camera").is_none());
            let exported: toml::Value = toml::from_str(&legacy.to_toml().unwrap()).unwrap();
            assert!(exported["parameters"].get("camera").is_none());

            let mut applied = current.clone();
            look.apply_to(&mut applied);
            let mut expected_camera = current.camera.clone();
            expected_camera.diffusion_filter = source.camera.diffusion_filter.clone();
            assert_eq!(applied.camera, expected_camera);
            assert!(look.matches(&applied));
            applied.camera.exposure_compensation_ev += 2.0;
            applied.camera.auto_exposure = !applied.camera.auto_exposure;
            applied.camera.auto_exposure_method = "center_weighted".into();
            assert!(look.matches(&applied));
            applied.enlarger.print_exposure += 0.25;
            assert!(!look.matches(&applied));

            let resolved = legacy.resolve(&data(), &current).unwrap();
            assert_eq!(resolved.params.camera, expected_camera);
            assert!(look.matches(&resolved.params));
        }
    }
}

#[test]
fn look_camera_requires_all_three_fields_and_rejects_unknown_fields() {
    let preset = captured_look(&exposure_params());
    let document = serde_json::to_value(&preset).unwrap();
    let camera = document["parameters"]["camera"].clone();
    let mut invalid_cameras = vec![serde_json::json!({})];
    for field in [
        "exposure_compensation_ev",
        "auto_exposure",
        "auto_exposure_method",
    ] {
        let mut partial = camera.clone();
        partial.as_object_mut().unwrap().remove(field);
        invalid_cameras.push(partial);
    }
    for (field, value) in [
        ("lens_blur_um", serde_json::json!(10.0)),
        ("diffusion_filter", serde_json::json!({"active": true})),
        ("unknown", serde_json::json!(true)),
    ] {
        let mut unknown = camera.clone();
        unknown.as_object_mut().unwrap().insert(field.into(), value);
        invalid_cameras.push(unknown);
    }
    let mut null_camera = document.clone();
    null_camera["parameters"]["camera"] = Value::Null;
    assert!(Preset::from_json_str(&null_camera.to_string()).is_err());

    for invalid in invalid_cameras {
        let mut json = document.clone();
        json["parameters"]["camera"] = invalid.clone();
        assert!(Preset::from_json_str(&json.to_string()).is_err());
        let mut toml: toml::Value = toml::from_str(&preset.to_toml().unwrap()).unwrap();
        toml["parameters"]["camera"] = toml::Value::try_from(&invalid).unwrap();
        assert!(Preset::from_toml_str(&toml::to_string(&toml).unwrap()).is_err());
    }
}

#[test]
fn look_camera_metering_method_must_match_the_sparse_source_vocabulary() {
    let preset = captured_look(&exposure_params());
    let mut document = serde_json::to_value(&preset).unwrap();
    document["parameters"]["camera"]["auto_exposure_method"] = "typo".into();
    assert!(Preset::from_json_str(&document.to_string()).is_err());
    let mut toml: toml::Value = toml::from_str(&preset.to_toml().unwrap()).unwrap();
    toml["parameters"]["camera"]["auto_exposure_method"] = toml::Value::String("typo".into());
    assert!(Preset::from_toml_str(&toml::to_string(&toml).unwrap()).is_err());

    // A valid method still resolves and reaches the runtime parameters.
    let mut valid = serde_json::to_value(captured_look(&exposure_params())).unwrap();
    valid["parameters"]["camera"]["auto_exposure_method"] = "multi_zone".into();
    let imported = Preset::from_json_str(&valid.to_string()).unwrap();
    let resolved = imported.resolve(&data(), &current_params()).unwrap();
    assert_eq!(resolved.params.camera.auto_exposure_method, "multi_zone");
}

#[test]
fn legacy_schema_versions_reject_the_schema_three_camera_group() {
    let preset = captured_look(&exposure_params());
    for version in [1, 2] {
        let mut document = serde_json::to_value(&preset).unwrap();
        document["schema_version"] = version.into();
        document["parameters"]
            .as_object_mut()
            .unwrap()
            .remove("output_mix");
        let error = Preset::from_json_str(&document.to_string()).unwrap_err();
        assert!(error.contains("legacy preset parameters.camera is unknown"));
        let mut toml: toml::Value = toml::from_str(&preset.to_toml().unwrap()).unwrap();
        let table = toml.as_table_mut().unwrap();
        table.insert("schema_version".into(), version.into());
        table["parameters"]
            .as_table_mut()
            .unwrap()
            .remove("output_mix");
        assert!(Preset::from_toml_str(&toml::to_string(&toml).unwrap()).is_err());
    }
}

#[test]
fn camera_section_applies_only_its_leaves_and_records_explicit_paths() {
    let source = exposure_params();
    let preset =
        Preset::capture_section("Camera section", Section::Camera, &source, provenance()).unwrap();
    let expected_paths = [
        "camera.exposure_compensation_ev",
        "camera.auto_exposure",
        "camera.auto_exposure_method",
    ];
    assert_eq!(Section::Camera.name(), "camera");
    assert_eq!(Section::Camera.paths(), expected_paths);
    let current = current_params();

    for imported in carrier_roundtrips(&preset) {
        assert_eq!(
            imported.kind().unwrap(),
            PresetKind::Section(Section::Camera)
        );
        assert!(!section_matches(&imported, &current).unwrap());
        let applied = imported.apply_section(&current, &test_film()).unwrap();
        let mut expected = current.clone();
        expected.camera.exposure_compensation_ev = source.camera.exposure_compensation_ev;
        expected.camera.auto_exposure = source.camera.auto_exposure;
        expected.camera.auto_exposure_method = source.camera.auto_exposure_method.clone();
        assert_eq!(
            serde_json::to_value(&applied).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(
            applied.explicit_section_paths,
            expected_paths.into_iter().map(str::to_owned).collect()
        );
        assert_eq!(
            applied.neutral_print_filters_protected,
            current.neutral_print_filters_protected
        );
        assert!(section_matches(&imported, &applied).unwrap());

        let mut changed = applied.clone();
        changed.camera.diffusion_filter.active = !changed.camera.diffusion_filter.active;
        changed.camera.lens_blur_um += 1.0;
        changed.io.crop = !changed.io.crop;
        assert!(section_matches(&imported, &changed).unwrap());
        for field in expected_paths {
            let mut changed = applied.clone();
            match field {
                "camera.exposure_compensation_ev" => changed.camera.exposure_compensation_ev += 0.5,
                "camera.auto_exposure" => {
                    changed.camera.auto_exposure = !changed.camera.auto_exposure
                }
                "camera.auto_exposure_method" => {
                    changed.camera.auto_exposure_method = "average".into()
                }
                _ => unreachable!(),
            }
            assert!(!section_matches(&imported, &changed).unwrap());
        }
    }
}
