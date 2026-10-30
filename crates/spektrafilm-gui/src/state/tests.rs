use super::*;

#[test]
fn inactive_chemistry_state_validates_mirrors_and_preserves_valid_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    let factory = GuiState::factory();
    for exhaustion in [0.0, 0.5, 1.0] {
        let mut params = factory.runtime_params().unwrap();
        params.film_render.chemistry.active = false;
        params.film_render.chemistry.developer_exhaustion = exhaustion;
        params.print_render.density_curves_morph.active = false;
        params
            .print_render
            .density_curves_morph
            .developer_exhaustion = exhaustion;
        params.camera.film_format_mm = 2000.0;
        let state =
            GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
                .unwrap();
        state.save(&path).unwrap();
        let restored = GuiState::load(&path).unwrap().runtime_params().unwrap();
        assert!(!restored.film_render.chemistry.active);
        assert!(!restored.print_render.density_curves_morph.active);
        assert_eq!(
            restored.film_render.chemistry.developer_exhaustion,
            exhaustion
        );
        assert_eq!(
            restored
                .print_render
                .density_curves_morph
                .developer_exhaustion,
            exhaustion
        );
        assert_eq!(restored.camera.film_format_mm, 2000.0);
    }
    let mut nonfinite = factory.runtime_params().unwrap();
    nonfinite.film_render.grain.procedural_size = Some(f32::NAN);
    assert!(
        GuiState::from_runtime(
            &nonfinite,
            factory.film(),
            factory.paper(),
            &factory.sections,
        )
        .is_err()
    );
    let accepted = std::fs::read(&path).unwrap();
    for (group, feature) in [
        ("film_render", "chemistry"),
        ("print_render", "density_curves_morph"),
    ] {
        for exhaustion in [-0.1, 2.0] {
            let mut invalid = factory.sections.clone();
            invalid["rust"]["runtime"][group][feature] =
                json!({"active": false, "developer_exhaustion": exhaustion});
            // A later GUI mirror with a valid value must not conceal invalid input.
            invalid[group] = json!({});
            invalid[group][feature] = json!({"active": false, "developer_exhaustion": 0.5});
            assert!(GuiState::from_value(invalid.clone()).is_err());
            assert!(GuiState { sections: invalid }.save(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), accepted);
        }
    }
}

#[test]
fn sparse_dye_state_preserves_omitted_and_authored_strength() {
    let inherited = GuiState::factory()
        .runtime_params()
        .unwrap()
        .film_render
        .grain
        .rms_granularity;
    for (nested, canonical, expected) in [
        (None, None, inherited),
        (Some([6.0, 8.0, 10.0]), None, [6.0, 8.0, 10.0]),
        (Some([6.0, 8.0, 10.0]), Some([9.0; 3]), [9.0; 3]),
    ] {
        let mut value = json!({"rust": {"version": CURRENT_STATE_VERSION,
            "runtime": {"film_render": {"grain": {"engine": "dye"}}}}});
        if let Some(rms) = nested {
            value["rust"]["runtime"]["film_render"]["grain"]["rms_granularity"] = json!(rms);
        }
        if let Some(rms) = canonical {
            value["grain"] = json!({"rms_granularity": rms});
        }
        let loaded = GuiState::from_value(value).unwrap();
        assert_eq!(
            loaded
                .runtime_params()
                .unwrap()
                .film_render
                .grain
                .rms_granularity,
            expected
        );
        let restored = GuiState::from_value(loaded.sections.clone()).unwrap();
        assert_eq!(
            restored
                .runtime_params()
                .unwrap()
                .film_render
                .grain
                .rms_granularity,
            expected
        );
    }
}

#[test]
fn flat_grain_state_preserves_dye_look_controls() {
    let state = GuiState::from_value(json!({
        "rust": {"version": CURRENT_STATE_VERSION},
        "grain": {"dye_strength": 0.5, "dye_scale": 2.0}
    }))
    .unwrap();
    let params = state.runtime_params().unwrap();
    assert_eq!(params.film_render.grain.dye_strength, 0.5);
    assert_eq!(params.film_render.grain.dye_scale, 2.0);
    let saved =
        GuiState::from_runtime(&params, state.film(), state.paper(), &state.sections).unwrap();
    let reopened = GuiState::from_value(saved.sections.clone()).unwrap();
    let restored = reopened.runtime_params().unwrap();
    assert_eq!(restored.film_render.grain.dye_strength, 0.5);
    assert_eq!(restored.film_render.grain.dye_scale, 2.0);
}

#[test]
fn sparse_startup_preferences_and_factory_reset_keep_the_authored_dye_default() {
    use spektrafilm_core::params::GrainEngine;

    let directory = tempfile::tempdir().unwrap();
    std::fs::write(default_path(directory.path()), "{}").unwrap();
    std::fs::write(directory.path().join("dialog_dirs.json"), "{}").unwrap();
    let state = startup(directory.path()).unwrap();
    assert_eq!(state.sections["rust"]["version"], CURRENT_STATE_VERSION);
    state.save(&default_path(directory.path())).unwrap();
    let reopened = startup(directory.path()).unwrap();
    assert_eq!(
        reopened.runtime_params().unwrap().film_render.grain.engine,
        GrainEngine::Dye
    );

    let legacy = json!({"rust": {"version": 5}});
    std::fs::write(default_path(directory.path()), legacy.to_string()).unwrap();
    let state = startup(directory.path()).unwrap();
    assert_eq!(
        state.runtime_params().unwrap().film_render.grain.engine,
        GrainEngine::Spektra
    );
    let reset = reset_factory(directory.path()).unwrap();
    assert_eq!(
        reset.runtime_params().unwrap().film_render.grain.engine,
        GrainEngine::Dye
    );
    assert!(!default_path(directory.path()).exists());
}

#[test]
fn grain_defaults_distinguish_authored_sparse_and_versioned_legacy_states() {
    use spektrafilm_core::params::GrainEngine;

    let factory = GuiState::factory();
    assert_eq!(factory.sections["rust"]["version"], CURRENT_STATE_VERSION);
    assert_eq!(
        factory.runtime_params().unwrap().film_render.grain.engine,
        GrainEngine::Dye
    );
    for source in [
        json!({}),
        json!({"grain": {"rms_granularity": [7.0, 8.0, 9.0]}}),
        json!({"rust": {"version": CURRENT_STATE_VERSION}}),
        json!({"rust": {"version": CURRENT_STATE_VERSION, "runtime": {"film_render": {"grain": {}}}}}),
    ] {
        let state = GuiState::from_value(source).unwrap();
        assert_eq!(
            state.runtime_params().unwrap().film_render.grain.engine,
            GrainEngine::Dye
        );
    }
    for version in 1..=5 {
        for runtime in [
            json!({}),
            json!({"film_render": {}}),
            json!({"film_render": {"grain": {"dye_event_area_um2": 2.25}}}),
        ] {
            let state =
                GuiState::from_value(json!({"rust": {"version": version, "runtime": runtime}}))
                    .unwrap();
            let params = state.runtime_params().unwrap();
            assert_eq!(params.film_render.grain.engine, GrainEngine::Spektra);
            assert_eq!(state.sections["rust"]["version"], CURRENT_STATE_VERSION);
            assert_eq!(
                state.sections["rust"]["runtime"]["film_render"]["grain"]["engine"],
                "spektra"
            );
            let reopened = GuiState::from_value(state.sections.clone()).unwrap();
            assert_eq!(
                reopened.runtime_params().unwrap().film_render.grain,
                params.film_render.grain
            );
        }
        let state = GuiState::from_value(json!({"rust": {"version": version}})).unwrap();
        assert_eq!(
            state.runtime_params().unwrap().film_render.grain.engine,
            GrainEngine::Spektra
        );
    }
}

#[test]
fn explicit_grain_engines_survive_every_state_boundary() {
    use spektrafilm_core::params::GrainEngine;

    for (name, engine) in [
        ("spektra", GrainEngine::Spektra),
        ("procedural", GrainEngine::Procedural),
        ("dye", GrainEngine::Dye),
        ("fotufilm", GrainEngine::Fotufilm),
    ] {
        for version in 1..=CURRENT_STATE_VERSION {
            for source in [
                json!({"rust": {"version": version, "runtime": {"film_render": {"grain": {"engine": name, "dye_event_area_um2": 2.25}}}}}),
                json!({"rust": {"version": version}, "grain": {"engine": name}}),
            ] {
                let state = GuiState::from_value(source).unwrap();
                let params = state.runtime_params().unwrap();
                assert_eq!(params.film_render.grain.engine, engine);
                let saved =
                    GuiState::from_runtime(&params, state.film(), state.paper(), &state.sections)
                        .unwrap();
                let reopened = GuiState::from_value(saved.sections).unwrap();
                assert_eq!(
                    reopened.runtime_params().unwrap().film_render.grain,
                    params.film_render.grain
                );
            }
        }
        let sparse = GuiState::from_value(json!({"grain": {"engine": name}})).unwrap();
        assert_eq!(
            sparse.runtime_params().unwrap().film_render.grain.engine,
            engine
        );
    }
}

#[test]
fn grain_engine_null_unknown_and_unknown_leaves_never_become_omission() {
    for version in 1..=CURRENT_STATE_VERSION {
        for grain in [
            json!({"engine": null}),
            json!({"engine": "unknown"}),
            json!({"unknown": true}),
        ] {
            for source in [
                json!({"rust": {"version": version}, "grain": grain}),
                json!({"rust": {"version": version, "runtime": {"film_render": {"grain": grain}}}}),
                json!({"rust": {"version": version, "runtime": {"film_render": {"grain": grain}}}, "grain": {"engine": "dye"}}),
                json!({"rust": {"version": version, "runtime": {"film_render": {"grain": {"engine": "dye"}}}}, "grain": grain}),
            ] {
                assert!(GuiState::from_value(source).is_err());
            }
        }
    }
    for grain in [
        json!({"engine": null}),
        json!({"engine": "unknown"}),
        json!({"unknown": true}),
    ] {
        assert!(GuiState::from_value(json!({"grain": grain})).is_err());
    }
}
#[test]
fn output_mix_state_roundtrip_and_legacy_retirement_are_atomic() {
    let factory = GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.output_mix.amount = 0.25;
    let saved = GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
        .unwrap();
    assert_eq!(saved.sections["rust"]["version"], CURRENT_STATE_VERSION);
    let restored = GuiState::from_value(saved.sections.clone()).unwrap();
    assert_eq!(restored.runtime_params().unwrap().output_mix.amount, 0.25);
    for version in [1, 2] {
        for retired in [Value::Null, json!([0.95, 1.0, 1.6])] {
            let mut candidate = saved.sections.clone();
            candidate["rust"]["version"] = json!(version);
            candidate["output_gamut_compress"]["lightness_compression"] = retired.clone();
            let source = candidate.clone();
            let result = GuiState::from_value(candidate.clone());
            if version == 1 && retired.is_null() {
                assert!(
                    result.unwrap().sections["output_gamut_compress"]
                        .get("lightness_compression")
                        .is_none()
                );
            } else {
                assert!(
                    format!("{:#}", result.unwrap_err())
                        .contains("io.output_gamut_compress.lightness_compression")
                );
            }
            assert_eq!(candidate, source);
            assert_eq!(restored.runtime_params().unwrap().output_mix.amount, 0.25);
        }
    }
    for invalid in [
        json!({"film_render":{"response":{}}}),
        json!({"print_render":{"response":{}}}),
        json!({"enlarger":{"c_filter_shift":0.0}}),
        json!({"output_mix":{"amount":1.01}}),
        json!({"output_mix":{"amount":-0.01}}),
    ] {
        let mut candidate = saved.sections.clone();
        candidate["rust"]["runtime"] = invalid;
        assert!(GuiState::from_value(candidate).is_err());
    }
}

#[test]
fn operator_state_retains_inactive_edits_and_reactivates_after_real_io() {
    let factory = GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.scanner.operator = spektrafilm_core::params::ScanOperatorParams {
        exposure_ev: 0.5000000000000001,
        warmth_ev: -0.25,
        tint_ev: 0.125,
        contrast: 1.2,
    };
    let saved = GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
        .unwrap();
    assert_eq!(
        saved.runtime_params().unwrap().scanner.operator,
        params.scanner.operator
    );
    assert!(
        saved
            .effective_runtime_params()
            .unwrap()
            .scanner
            .operator
            .is_default()
    );
    let path = std::env::temp_dir().join(format!(
        "spektrafilm-operator-state-{}.json",
        std::process::id()
    ));
    saved.save(&path).unwrap();
    let loaded = GuiState::load(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        loaded.runtime_params().unwrap().scanner.operator,
        params.scanner.operator
    );
    let mut active = loaded.sections.clone();
    active["simulation"]["route"] = json!("input > film > scan");
    active["scanner"]["scan_output"] = json!("negative_to_positive");
    let active = GuiState::from_value(active).unwrap();
    assert_eq!(
        active.effective_runtime_params().unwrap().scanner.operator,
        params.scanner.operator
    );
    let mut fixed = active.sections.clone();
    fixed["scanner"]["conversion_style"] = json!("fixed");
    let fixed = GuiState::from_value(fixed).unwrap();
    assert!(
        fixed
            .effective_runtime_params()
            .unwrap()
            .scanner
            .operator
            .is_default()
    );
    assert_eq!(
        fixed.runtime_params().unwrap().scanner.operator,
        params.scanner.operator
    );
    for invalid in [
        json!({"contrast":0}),
        json!({"contrast":-1}),
        json!({"unknown":1}),
    ] {
        let mut candidate = loaded.sections.clone();
        candidate["scanner"]["operator"] = invalid;
        assert!(GuiState::from_value(candidate).is_err());
    }
    let extension = GuiState::from_value(json!({"rust":{"version":CURRENT_STATE_VERSION,"runtime":{"scanner":{"operator":{"warmth_ev":0.25}}}}})).unwrap();
    assert_eq!(
        extension
            .runtime_params()
            .unwrap()
            .scanner
            .operator
            .warmth_ev,
        0.25
    );
    assert!(
        GuiState::from_value(json!({}))
            .unwrap()
            .runtime_params()
            .unwrap()
            .scanner
            .operator
            .is_default()
    );
}

#[test]
fn state_carriers_reject_ignored_mix_and_nested_retired_io() {
    for amount in [0.25, 1.01, -0.01] {
        let error = GuiState::from_value(json!({"output_mix":{"amount":amount}})).unwrap_err();
        assert!(error.to_string().contains("rust.runtime.output_mix"));
    }
    for version in [1, 2] {
        for retired in [Value::Null, json!([0.95, 1.0, 1.6])] {
            let source = json!({"rust":{"version":version},"simulation":{"io":{"output_gamut_compress":{"lightness_compression":retired}}}});
            let result = GuiState::from_value(source.clone());
            if version == 1 && retired.is_null() {
                let accepted = result.unwrap();
                assert!(
                    accepted.sections["simulation"]["io"]["output_gamut_compress"]
                        .get("lightness_compression")
                        .is_none()
                );
                assert_eq!(accepted.runtime_params().unwrap().output_mix.amount, 1.0);
            } else {
                assert!(
                    format!("{:#}", result.unwrap_err())
                        .contains("output_gamut_compress.lightness_compression")
                );
            }
        }
        let error = GuiState::from_value(json!({"rust":{"version":version},"simulation":{"io":{"finished_rgb_output_processing":true}}})).unwrap_err();
        assert!(format!("{error:#}").contains("finished_rgb_output_processing"));
    }
}

#[test]
fn version_two_states_rename_conversion_paths_and_map_the_exposure_pair() {
    let legacy_scanner = json!({
        "scan_output": "negative_to_positive",
    });
    for (pair, mode, compensation, tracking) in [
        (
            json!({"normalize_print_exposure": true, "print_exposure_compensation": true}),
            "auto",
            true,
            "follow_mode",
        ),
        (
            json!({"normalize_print_exposure": false, "print_exposure_compensation": false}),
            "manual",
            false,
            "follow_mode",
        ),
        (
            json!({"normalize_print_exposure": true, "print_exposure_compensation": false}),
            "auto",
            false,
            "never",
        ),
        (
            json!({"normalize_print_exposure": false, "print_exposure_compensation": true}),
            "manual",
            true,
            "always",
        ),
    ] {
        // A saved version-2 state carries the legacy spellings in both
        // the upstream-mirror sections and the Rust runtime extension.
        let mut source = json!({
            "rust": {"version": 2, "runtime": {
                "scanner": legacy_scanner.clone(),
                "enlarger": pair.clone(),
            }},
            "preflashing": pair.clone(),
            "scanner": legacy_scanner.clone(),
            "simulation": {"print_exposure_compensation": compensation},
        });
        let state = GuiState::from_value(source.clone()).unwrap();
        let params = state.runtime_params().unwrap();
        assert_eq!(params.scanner.scan_output, "negative_to_positive");
        assert_eq!(params.enlarger.print_exposure_mode.as_str(), mode);
        assert_eq!(
            params.settings.print_exposure_ev_tracking.as_str(),
            tracking
        );
        // The stored document migrates to the canonical paths and the
        // upstream boolean mirror stays in step on save.
        let saved =
            GuiState::from_runtime(&params, state.film(), state.paper(), &state.sections).unwrap();
        assert_eq!(
            saved.sections["rust"]["version"],
            json!(CURRENT_STATE_VERSION)
        );
        let runtime = serde_json::to_string(&saved.sections["rust"]["runtime"]).unwrap();
        assert!(runtime.contains("negative_to_positive"));
        assert_eq!(
            saved.sections["preflashing"]["normalize_print_exposure"],
            json!(mode == "auto")
        );
        assert_eq!(
            saved.sections["preflashing"]["print_exposure_compensation"],
            json!(compensation)
        );
        assert_eq!(
            saved.sections["simulation"]["print_exposure_compensation"],
            json!(compensation)
        );
        let runtime = serde_json::to_string(&saved.sections["rust"]["runtime"]).unwrap();
        assert!(runtime.contains("\"print_exposure_mode\""));
        source["rust"]["version"] = json!(4);
    }
    // The upstream compensation boolean projects onto the tracking
    // setting; the state-side key stays boolean.
    let upstream = GuiState::from_value(json!({
        "rust": {"version": 4},
        "simulation": {"enlarger": {"print_exposure_compensation": false}}
    }))
    .unwrap();
    assert_eq!(
        upstream
            .runtime_params()
            .unwrap()
            .settings
            .print_exposure_ev_tracking
            .as_str(),
        "never"
    );
    // A current-version document must fail unknown-field validation.
    let current = GuiState::from_value(json!({"rust": {"version": 4, "runtime": {
        "scanner": {"lamp": "D55", "retired_receiver_profile": "lab_scan_approx"}
    }}}));
    assert!(current.is_err());
}

#[test]
fn legacy_cmyk_state_migration_is_transactional_and_idempotent() {
    // A legacy complete document carrying the retired group migrates onto
    // the canonical name, preserving active and strength.
    for version in [1, 2, 3] {
        let source = json!({
            "rust": {"version": version},
            "magazine_print_color": {"active": true, "strength": 0.75},
        });
        let state = GuiState::from_value(source).unwrap();
        let params = state.runtime_params().unwrap();
        assert!(params.cmyk.active);
        assert_eq!(params.cmyk.strength, 0.75);
        assert!(state.sections.get("magazine_print_color").is_none());
        assert_eq!(state.sections["cmyk"]["active"], json!(true));
        assert_eq!(state.sections["cmyk"]["strength"], json!(0.75));
        assert_eq!(
            state.sections["rust"]["version"],
            json!(CURRENT_STATE_VERSION)
        );

        // Re-serializing a migrated document emits only the canonical group.
        let saved =
            GuiState::from_runtime(&params, state.film(), state.paper(), &state.sections).unwrap();
        let runtime = serde_json::to_string(&saved.sections).unwrap();
        assert!(runtime.contains("\"cmyk\""));
        assert!(!runtime.contains("magazine_print_color"));
    }

    // The retired route migrates onto the base route with cmyk active.
    let source = json!({
        "rust": {"version": 3},
        "simulation": {"route": "input > film > scan > magazine"},
    });
    let state = GuiState::from_value(source).unwrap();
    let params = state.runtime_params().unwrap();
    assert_eq!(params.workflow.route, "input > film > scan");
    assert!(params.cmyk.active);
    assert_eq!(
        state.sections["simulation"]["route"],
        json!("input > film > scan")
    );

    // A legacy document carrying both spellings is rejected atomically.
    let both = json!({
        "rust": {"version": 3},
        "magazine_print_color": {"active": true, "strength": 0.75},
        "cmyk": {"active": false, "strength": 1.0},
    });
    let error = GuiState::from_value(both).unwrap_err();
    assert!(
        error.to_string().contains("magazine_print_color"),
        "{error}"
    );
    assert!(error.to_string().contains("cmyk"), "{error}");

    // A current-version document rejects the retired group.
    let current = json!({
        "rust": {"version": 4},
        "magazine_print_color": {"active": true, "strength": 0.75},
    });
    assert!(GuiState::from_value(current).is_err());

    // A current-version document rejects the retired route.
    let current_route = json!({
        "rust": {"version": 4},
        "simulation": {"route": "input > film > scan > magazine"},
    });
    assert!(GuiState::from_value(current_route).is_err());
}

#[test]
fn legacy_runtime_and_simulation_routes_migrate_together() {
    for version in [2, 3] {
        let source = json!({
            "rust": {"version": version, "runtime": {
                "workflow": {"route": "input > film > scan > magazine"},
                "magazine_print_color": {"strength": 0.75}
            }},
            "simulation": {"route": "input > film > scan > magazine"},
            "magazine_print_color": {"strength": 0.75}
        });
        let state = GuiState::from_value(source).unwrap();
        let params = state.runtime_params().unwrap();
        assert_eq!(params.workflow.route, "input > film > scan");
        assert!(params.cmyk.active);
        assert_eq!(params.cmyk.strength, 0.75);
        assert_eq!(
            state.sections["rust"]["runtime"]["workflow"]["route"],
            json!("input > film > scan")
        );
        assert_eq!(
            state.sections["rust"]["runtime"]["cmyk"],
            json!({"active": true, "strength": 0.75})
        );
        assert!(
            state.sections["rust"]["runtime"]
                .get("magazine_print_color")
                .is_none()
        );
        let reloaded = GuiState::from_value(state.sections.clone()).unwrap();
        assert_eq!(reloaded.runtime_params().unwrap().cmyk, params.cmyk);

        let conflict = json!({"rust": {"version": version, "runtime": {
            "workflow": {"route": "input > film > scan > magazine"},
            "magazine_print_color": {"active": false}
        }}});
        assert!(GuiState::from_value(conflict).is_err());
    }
    assert!(
        GuiState::from_value(json!({"rust": {"version": 4, "runtime": {
            "workflow": {"route": "input > film > scan > magazine"}
        }}}))
        .is_err()
    );
}

#[test]
fn legacy_rust_grain_snapshot_preserves_supported_controls() {
    for retired in [None, Some(Value::Null)] {
        let mut source = json!({"rust":{"version":1,"runtime":{"film_render":{"grain":{"rms_granularity":[9.0,12.0,15.0],"particle_scale_sublayers":[2.0,1.0,0.5],"monochrome":true}}}}});
        if let Some(retired) = retired {
            source["rust"]["runtime"]["io"] =
                json!({"output_gamut_compress":{"lightness_compression":retired}});
        }
        let accepted = GuiState::from_value(source).unwrap();
        let params = accepted.runtime_params().unwrap();
        let grain = &params.film_render.grain;
        assert_eq!(grain.rms_granularity, [9.0, 12.0, 15.0]);
        assert_eq!(grain.particle_scale_sublayers, [2.0, 1.0, 0.5]);
        assert!(grain.monochrome);
        let saved = GuiState::from_runtime(
            &params,
            accepted.film(),
            accepted.paper(),
            &accepted.sections,
        )
        .unwrap();
        let reopened = GuiState::from_value(saved.sections).unwrap();
        assert_eq!(
            reopened.runtime_params().unwrap().film_render.grain,
            params.film_render.grain
        );
    }
}

#[test]
fn saved_zero_rms_and_nondefault_upscale_survive_state_file_roundtrip() {
    let loaded = GuiState::from_value(json!({
        "grain": {"rms_granularity": [0.0, 0.0, 0.0]},
        "input_image": {"upscale_factor": 2.5},
        "rust": {"version": 1, "runtime": {
            "film_render": {"grain": {"rms_granularity": [9.0, 9.0, 9.0]}},
            "io": {"upscale_factor": 4.0}
        }}
    }))
    .unwrap();
    let params = loaded.runtime_params().unwrap();
    assert_eq!(params.film_render.grain.rms_granularity, [0.0; 3]);
    assert_eq!(params.io.upscale_factor, 2.5);
    let saved =
        GuiState::from_runtime(&params, loaded.film(), loaded.paper(), &loaded.sections).unwrap();
    let path = std::env::temp_dir().join(format!(
        "spektrafilm-saved-zero-rms-{}.json",
        std::process::id()
    ));
    saved.save(&path).unwrap();
    let restored = GuiState::load(&path);
    std::fs::remove_file(&path).unwrap();
    let restored = restored.unwrap().runtime_params().unwrap();
    assert_eq!(restored.film_render.grain.rms_granularity, [0.0; 3]);
    assert_eq!(restored.io.upscale_factor, 2.5);
}

#[test]
fn internal_monochrome_preserves_python_sections_and_state_roundtrip() {
    let factory = GuiState::factory();
    let baseline = GuiState::from_runtime(
        &factory.runtime_params().unwrap(),
        factory.film(),
        factory.paper(),
        &factory.sections,
    )
    .unwrap();
    let mut source = factory.runtime_params().unwrap();
    source.film_render.grain.monochrome = true;
    // Monochrome is derived from the film, so sections preserve the target's value.
    let applied = source;
    let saved =
        GuiState::from_runtime(&applied, factory.film(), factory.paper(), &factory.sections)
            .unwrap();
    let mut before = baseline.sections;
    let mut after = saved.sections.clone();
    before.as_object_mut().unwrap().remove("rust");
    after.as_object_mut().unwrap().remove("rust");
    assert_eq!(
        serde_json::to_vec(&before).unwrap(),
        serde_json::to_vec(&after).unwrap()
    );

    let serialized = serde_json::to_vec(&saved.sections).unwrap();
    let loaded = GuiState::from_value(serde_json::from_slice(&serialized).unwrap()).unwrap();
    for state in [saved, loaded] {
        let restored = state.runtime_params().unwrap();
        assert!(restored.film_render.grain.monochrome);
        assert_eq!(
            restored.render_preparation_state(),
            applied.render_preparation_state()
        );
    }
}

#[test]
fn partial_legacy_sections_use_factory_and_flat_gui_only_precedence() {
    let state = GuiState::from_value(json!({
        "input_image":{"apply_cctf_decoding":true,"crop":true,"crop_center":[0.2,0.7]},
        "simulation":{"saving_color_space":"ITU-R BT.2020"},
        "gui_only":{"display":{"gray_18_canvas":false}},
        "display":{"white_padding":0.1}
    }))
    .unwrap();
    let params = state.runtime_params().unwrap();
    assert!(params.io.crop && params.io.input_cctf_decoding);
    assert_eq!(params.io.crop_center, [0.2, 0.7]);
    assert_eq!(state.sections["display"]["gray_18_canvas"], true);
    assert_eq!(state.sections["display"]["white_padding"], 0.1);
    assert_eq!(state.paper(), "kodak_supra_endura");
}
#[test]
fn dedicated_diffusion_and_simulation_panels_win_over_passthrough_groups() {
    let state = GuiState::from_value(json!({
        "camera":{"diffusion_filter":{"strength":1.5}},
        "camera_diffusion":{"strength":0.25},
        "preflashing":{"print_exposure":8.0,"preflash_exposure":0.2},
        "simulation":{"print_exposure":1.75}
    }))
    .unwrap();
    let params = state.runtime_params().unwrap();
    assert_eq!(params.camera.diffusion_filter.strength, 0.25);
    assert_eq!(params.enlarger.print_exposure, 1.75);
    assert_eq!(params.enlarger.preflash_exposure, 0.2);
}

#[test]
fn camera_mirror_is_validated_before_dedicated_diffusion_overrides_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    let invalid_path = directory.path().join("invalid.json");
    let accepted = GuiState::factory();
    accepted.save(&path).unwrap();
    let accepted_bytes = std::fs::read(&path).unwrap();

    for dedicated in [None, Some(json!({"filter_family": "black_pro_mist"}))] {
        let mut invalid = json!({
            "rust": {"version": CURRENT_STATE_VERSION},
            "camera": {"diffusion_filter": {"active": false, "filter_family": "unknown"}}
        });
        if let Some(dedicated) = dedicated {
            invalid["camera_diffusion"] = dedicated;
        }
        let source = serde_json::to_vec(&invalid).unwrap();
        std::fs::write(&invalid_path, &source).unwrap();
        assert!(GuiState::load(&invalid_path).is_err());
        assert_eq!(std::fs::read(&invalid_path).unwrap(), source);
        assert!(GuiState::from_value(invalid.clone()).is_err());
        assert!(GuiState { sections: invalid }.save(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), accepted_bytes);
    }

    // Legacy null color filters normalize to the runtime's "none" value.
    let state = GuiState::from_value(json!({
        "rust": {"version": 2},
        "camera": {"color_filter": null, "diffusion_filter": {"filter_family": "pro_mist"}},
        "camera_diffusion": {"filter_family": "black_pro_mist"}
    }))
    .unwrap();
    let camera = state.runtime_params().unwrap().camera;
    assert_eq!(camera.color_filter, "none");
    assert_eq!(camera.diffusion_filter.filter_family, "black_pro_mist");
}

#[test]
fn invalid_enlarger_mirrors_are_rejected_before_later_overrides() {
    let directory = tempfile::tempdir().unwrap();
    let accepted_path = directory.path().join("accepted.json");
    let source_path = directory.path().join("source.json");
    GuiState::factory().save(&accepted_path).unwrap();
    let accepted_bytes = std::fs::read(&accepted_path).unwrap();

    for invalid in [
        json!({
            "preflashing": {"illuminant": "unknown"},
            "simulation": {"enlarger": {"illuminant": "TH-KG3"}}
        }),
        json!({
            "preflashing": {"illuminant": null},
            "simulation": {"enlarger": {"illuminant": "TH-KG3"}}
        }),
        json!({
            "preflashing": {"normalize_print_exposure": "yes"},
            "enlarger": {"print_exposure_mode": "auto"}
        }),
        json!({
            "preflashing": {"illuminant": "unknown"},
            "enlarger": {"illuminant": "TH-KG3"}
        }),
        json!({
            "enlarger": {"illuminant": "unknown"},
            "simulation": {"enlarger": {"illuminant": "TH-KG3"}}
        }),
        json!({
            "simulation": {"enlarger": {"print_exposure_compensation": "yes"}}
        }),
    ] {
        let mut invalid = invalid;
        invalid["rust"] = json!({"version": CURRENT_STATE_VERSION});
        invalid["camera"] = json!({"exposure_compensation_ev": 1.25});
        let source = serde_json::to_vec(&invalid).unwrap();
        std::fs::write(&source_path, &source).unwrap();
        assert!(GuiState::load(&source_path).is_err());
        assert_eq!(std::fs::read(&source_path).unwrap(), source);
        assert!(GuiState::from_value(invalid.clone()).is_err());
        assert!(GuiState { sections: invalid }.save(&accepted_path).is_err());
        assert_eq!(std::fs::read(&accepted_path).unwrap(), accepted_bytes);
    }
}

#[test]
fn valid_enlarger_mirror_precedence_survives_runtime_and_file_roundtrip() {
    let state = GuiState::from_value(json!({
        "rust": {"version": 2},
        "preflashing": {
            "illuminant": "D65",
            "normalize_print_exposure": true,
            "print_exposure_compensation": false
        },
        "enlarger": {"illuminant": "TH-KG3"},
        "simulation": {"enlarger": {"illuminant": "BB4000"}}
    }))
    .unwrap();
    assert_eq!(state.sections["preflashing"]["illuminant"], "D65");
    let params = state.runtime_params().unwrap();
    assert_eq!(params.enlarger.illuminant, "BB4000");
    assert_eq!(params.enlarger.print_exposure_mode.as_str(), "auto");
    assert_eq!(params.settings.print_exposure_ev_tracking.as_str(), "never");

    let saved =
        GuiState::from_runtime(&params, state.film(), state.paper(), &state.sections).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    saved.save(&path).unwrap();
    let restored = GuiState::load(&path).unwrap().runtime_params().unwrap();
    assert_eq!(restored.enlarger.illuminant, "BB4000");
    assert_eq!(restored.enlarger.print_exposure_mode.as_str(), "auto");
    assert_eq!(
        restored.settings.print_exposure_ev_tracking.as_str(),
        "never"
    );
}
#[test]
fn camera_capture_illuminant_roundtrips_through_gui_state() {
    let factory = GuiState::factory();
    // `None` is a real unset value (inherit the film profile reference
    // illuminant), not a missing field: it must survive the save/load
    // projection as an explicit null rather than collapse to a default.
    let mut params = factory.runtime_params().unwrap();
    assert_eq!(params.camera.capture_illuminant, None);
    let mut saved =
        GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
            .unwrap()
            .sections;
    assert_eq!(saved["camera"]["capture_illuminant"], Value::Null);
    saved.as_object_mut().unwrap().remove("rust");
    let restored = GuiState::from_value(saved.clone()).unwrap();
    assert_eq!(
        restored.runtime_params().unwrap().camera.capture_illuminant,
        None
    );

    // An explicit value survives the same round trip.
    params.camera.capture_illuminant = Some("BB4000".into());
    let mut saved =
        GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
            .unwrap()
            .sections;
    assert_eq!(saved["camera"]["capture_illuminant"], json!("BB4000"));
    saved.as_object_mut().unwrap().remove("rust");
    let restored = GuiState::from_value(saved.clone()).unwrap();
    assert_eq!(
        restored
            .runtime_params()
            .unwrap()
            .camera
            .capture_illuminant
            .as_deref(),
        Some("BB4000")
    );

    // The leaf stays validated by the shared core validator.
    saved["camera"]["capture_illuminant"] = json!("typo");
    assert!(GuiState::from_value(saved).is_err());
}
#[test]
fn legacy_export_depth_and_recipe_survive_save_reload() {
    for (depth, format) in [(8, "png"), (16, "tiff"), (32, "tiff")] {
        let state = GuiState::from_value(json!({
            "grain": {"rms_granularity": [0.0, 0.0, 0.0]},
            "input_image": {"upscale_factor": 1.5},
            "simulation": {"film_stock": "kodak_portra_400"},
            "rust": {"version": 1, "save_bit_depth": depth, "viewer": {"zoom": 3.0}}
        }))
        .unwrap();
        assert_eq!(state.sections["rust"]["export_format"], format);
        assert_eq!(state.sections["rust"]["save_bit_depth"], depth);
        assert_eq!(state.sections["rust"]["viewer"]["zoom"], 3.0);
        assert_eq!(state.film(), "kodak_portra_400");
        let params = state.runtime_params().unwrap();
        assert_eq!(params.film_render.grain.rms_granularity, [0.0; 3]);
        assert_eq!(params.io.upscale_factor, 1.5);
        let saved =
            GuiState::from_runtime(&params, state.film(), state.paper(), &state.sections).unwrap();
        let path = std::env::temp_dir().join(format!(
            "spektrafilm-legacy-export-{}-{depth}.json",
            std::process::id()
        ));
        saved.save(&path).unwrap();
        let restored = GuiState::load(&path);
        std::fs::remove_file(&path).unwrap();
        let restored = restored.unwrap();
        assert_eq!(restored.film(), "kodak_portra_400");
        assert_eq!(restored.sections["rust"]["viewer"]["zoom"], 3.0);
        assert_eq!(restored.sections["rust"]["export_format"], format);
        assert_eq!(restored.sections["rust"]["save_bit_depth"], depth);
        let params = restored.runtime_params().unwrap();
        assert_eq!(params.film_render.grain.rms_granularity, [0.0; 3]);
        assert_eq!(params.io.upscale_factor, 1.5);
    }
}

#[test]
fn export_migration_does_not_repair_explicit_invalid_settings() {
    for (format, depth) in [
        (json!("png"), 16),
        (json!("jpeg"), 32),
        (json!("exr"), 8),
        (json!("unknown"), 16),
        (Value::Null, 16),
        (json!("tiff"), 12),
    ] {
        let result = GuiState::from_value(json!({
            "rust": {"version": 1, "export_format": format, "save_bit_depth": depth}
        }));
        assert!(result.is_err());
    }
    let result = GuiState::from_value(json!({"rust": {"version": 1, "save_bit_depth": 12}}));
    assert!(result.is_err());
}

#[test]
fn fotufilm_effective_controls_roundtrip_only_in_rust_extension() {
    use spektrafilm_core::params::grain::{FotufilmGrainParams, FotufilmMaterial};
    let factory = GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.film_render.grain.engine = spektrafilm_core::params::GrainEngine::Fotufilm;
    params.film_render.grain.fotufilm = FotufilmGrainParams {
        amount: 2.0,
        size: 1.5,
        softness: 2.5,
        color: 0.4,
        layers: [0.5, 1.0, 2.0],
        rms_density: 0.015,
        clump_radius_um: 7.5,
        layer_weights: [0.8, 1.2, 1.5],
        layer_size_ratio: [0.75, 1.0, 1.25],
        radius_span: 8.0,
        sublayer_shares: [0.1, 0.2, 0.3, 0.4],
        coating_density_scale: 1.25,
        fog_density: 0.04,
        material: FotufilmMaterial::DyeNegative,
    };
    let saved = GuiState::from_runtime(
        &params,
        "kodak_gold_200",
        "kodak_supra_endura",
        &factory.sections,
    )
    .unwrap();
    assert!(saved.sections["grain"].get("fotufilm").is_none());
    assert!(saved.sections["grain"].get("engine").is_none());
    assert!(factory.sections["grain"].get("fotufilm").is_none());
    assert_eq!(
        saved.sections["rust"]["runtime"]["film_render"]["grain"]["engine"],
        "fotufilm"
    );
    let encoded = serde_json::to_vec(&saved.sections).unwrap();
    let loaded = GuiState::from_value(serde_json::from_slice(&encoded).unwrap()).unwrap();
    let restored = loaded.runtime_params().unwrap();
    assert_eq!(
        restored.film_render.grain.engine,
        params.film_render.grain.engine
    );
    assert_eq!(
        restored.film_render.grain.fotufilm,
        params.film_render.grain.fotufilm
    );
    let changed_stock = GuiState::from_runtime(
        &restored,
        "kodak_gold_200",
        "kodak_supra_endura",
        &loaded.sections,
    )
    .unwrap();
    assert_eq!(
        changed_stock
            .runtime_params()
            .unwrap()
            .film_render
            .grain
            .fotufilm,
        params.film_render.grain.fotufilm
    );
}

#[test]
fn partial_fotufilm_state_keeps_nested_defaults_and_old_engine_controls() {
    let state = GuiState::from_value(json!({"rust": {"version": 1, "runtime": {
        "film_render": {"grain": {"engine": "fotufilm", "fotufilm": {"amount": 0.0}, "dye_support_um": 4.5}}
    }}})).unwrap();
    let grain = state.runtime_params().unwrap().film_render.grain;
    assert_eq!(grain.fotufilm.amount, 0.0);
    assert_eq!(grain.fotufilm.rms_density, 0.009);
    assert_eq!(grain.fotufilm.sublayer_shares, [0.25; 4]);
    assert_eq!(grain.dye_support_um, 4.5);
}

#[test]
fn runtime_extensions_roundtrip_without_overriding_upstream_selection() {
    let factory = GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.io.output_cctf_encoding = false;
    params.film_render.development_time = Some(9.0);
    params.film_render.grain.engine = spektrafilm_core::params::GrainEngine::Dye;
    params.film_render.grain.dye_support_um = 4.5;
    params.film_render.grain.dye_event_area_um2 = 0.6;
    params.film_render.grain.procedural_profile = "8mm500".into();
    params.film_render.grain.procedural_amount = Some(0.0);
    let mut extras = factory.sections.clone();
    extras["rust"] = json!({"version":1,"viewer":{"zoom":3.0}});
    let saved =
        GuiState::from_runtime(&params, "kodak_gold_200", "kodak_supra_endura", &extras).unwrap();
    assert_eq!(
        saved.sections["rust"]["runtime"]["film_render"]["grain"]["engine"],
        "dye"
    );
    assert_eq!(
        saved.sections["rust"]["runtime"]["film_render"]["grain"]["dye_support_um"],
        4.5
    );
    let recipe = serde_json::from_slice(&serde_json::to_vec(&saved.sections).unwrap()).unwrap();
    let loaded = GuiState::from_value(recipe).unwrap();
    let restored = loaded.runtime_params().unwrap();
    assert!(!restored.io.output_cctf_encoding);
    assert_eq!(restored.film_render.development_time, Some(9.0));
    assert_eq!(
        restored.film_render.grain.engine,
        spektrafilm_core::params::GrainEngine::Dye
    );
    assert_eq!(restored.film_render.grain.dye_support_um, 4.5);
    assert_eq!(restored.film_render.grain.dye_event_area_um2, 0.6);
    assert_eq!(restored.film_render.grain.procedural_profile, "8mm500");
    assert_eq!(
        restored
            .film_render
            .grain
            .resolved_grain_procedural()
            .amount,
        0.0
    );
    assert_eq!(loaded.sections["rust"]["viewer"]["zoom"], 3.0);
    let serialized_saved: Value =
        serde_json::from_slice(&serde_json::to_vec(&saved.sections).unwrap()).unwrap();
    assert_eq!(loaded.sections, serialized_saved);
}
#[test]
fn unknown_grain_engine_fails_state_validation() {
    let error = GuiState::from_value(json!({
        "rust": {"version": 1, "runtime": {
            "film_render": {"grain": {"engine": "unknown"}}
        }}
    }))
    .unwrap_err();
    assert!(format!("{error:#}").contains("unknown variant `unknown`"));
}
#[test]
fn retired_grain_controls_fail_in_every_state_carrier() {
    for (key, value) in [
        ("particle_area_um2", json!(0.2)),
        ("particle_scale", json!([2.0, 1.0, 0.5])),
        ("particle_scale_layers", json!([2.0, 1.0, 0.5])),
        ("sublayers_active", json!(false)),
        ("n_sub_layers", json!(3)),
    ] {
        for version in std::iter::once(None).chain((1..=CURRENT_STATE_VERSION).map(Some)) {
            for carrier in ["grain", "film_render", "rust.runtime"] {
                if version.is_none() && carrier == "rust.runtime" {
                    continue;
                }
                let fields = json!({(key): value.clone()});
                let mut source = match carrier {
                    "grain" => json!({"grain": fields}),
                    "film_render" => json!({"film_render": {"grain": fields}}),
                    _ => json!({"rust": {"runtime": {"film_render": {"grain": fields}}}}),
                };
                if let Some(version) = version {
                    source["rust"]["version"] = json!(version);
                }
                let error = GuiState::from_value(source).unwrap_err();
                assert!(
                    format!("{error:#}").contains(key),
                    "{carrier}, version {version:?}, {key}: {error:#}"
                );
            }
        }
    }
}
#[test]
fn grain_procedural_canonical_fields_roundtrip_through_state_and_runtime() {
    let factory = GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    let grain = &mut params.film_render.grain;
    grain.engine = spektrafilm_core::params::GrainEngine::Procedural;
    grain.procedural_profile = "custom".into();
    grain.procedural_resolution_type = 0;
    grain.procedural_timer = 0.375;
    grain.active = false;
    grain.rms_granularity = [11.0, 13.0, 17.0];
    grain.density_min = [0.11, 0.22, 0.33];
    grain.uniformity = [0.91, 0.92, 0.93];
    grain.particle_scale_sublayers = [1.0, 0.75, 0.5];
    grain.blur = 1.25;
    grain.mult_usm_sigma = 1.1;
    grain.mult_usm_amount = 2.2;
    grain.blur_dye_clouds_um = 3.3;
    grain.micro_structure = [0.4, 24.0];

    let saved = GuiState::from_runtime(
        &params,
        "kodak_gold_200",
        "kodak_supra_endura",
        &factory.sections,
    )
    .unwrap();
    let loaded = GuiState::from_value(saved.sections.clone()).unwrap();
    let restored = loaded.runtime_params().unwrap().film_render.grain;

    assert!(!restored.active);
    assert_eq!(restored.procedural_resolution_type, 0);
    assert_eq!(restored.procedural_timer, 0.375);
    assert_eq!(restored.rms_granularity, [11.0, 13.0, 17.0]);
    assert_eq!(restored.density_min, [0.11, 0.22, 0.33]);
    assert_eq!(restored.uniformity, [0.91, 0.92, 0.93]);
    assert_eq!(restored.particle_scale_sublayers, [1.0, 0.75, 0.5]);
    assert_eq!(restored.blur, 1.25);
    assert_eq!(restored.mult_usm_sigma, 1.1);
    assert_eq!(restored.mult_usm_amount, 2.2);
    assert_eq!(restored.blur_dye_clouds_um, 3.3);
    assert_eq!(restored.micro_structure, [0.4, 24.0]);

    let object = loaded.sections["grain"].as_object().unwrap();
    assert_eq!(object.len(), 13);
    for key in [
        "active",
        "rms_granularity",
        "dye_strength",
        "dye_scale",
        "density_min",
        "uniformity",
        "particle_scale_sublayers",
        "blur",
        "mult_usm_sigma",
        "mult_usm_amount",
        "blur_dye_clouds_um",
        "micro_structure",
        "micro_sublayers",
    ] {
        assert!(object.contains_key(key), "canonical field missing: {key}");
    }
}

#[test]
fn canonical_sections_roundtrip_without_runtime_extension() {
    let factory = GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.camera.film_format_mm = 70.0;
    params.camera.color_filter = "hoya_y2".into();
    params.film_render.grain.micro_sublayers = 3;
    params.film_render.dir_couplers.langmuir_donor_k_rgb = [0.5, 1.5, 2.5];
    params.film_render.dir_couplers.langmuir_receiver_k_rgb = [0.75, 1.75, 2.75];
    params.io.input_gamut_compress.hull_detail = 9.0;
    params.io.input_gamut_compress.boundary = "locus".into();
    params.film_render.development_time = Some(9.0);
    params.print_render.development_time = Some(4.0);
    params.film_render.chemistry.gamma_factor_red = 1.25;
    params.print_render.density_curves_morph.gamma_factor_blue = 0.75;
    params.film_render.base.tilt = 0.2;
    params.film_render.base.cyan = 1.2;
    params.print_render.base.scale = 0.8;
    params.film_render.convert.base_percentile = 97.5;
    params.film_render.convert.calibration = "1.1 0 0  0 1 0  0 0 0.9".into();
    params.workflow.route = "input > convert-film > scan-minus-base".into();
    let mut saved =
        GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
            .unwrap()
            .sections;
    saved.as_object_mut().unwrap().remove("rust");
    let loaded = GuiState::from_value(saved.clone()).unwrap();
    let restored = loaded.runtime_params().unwrap();
    assert_eq!(restored.camera.film_format_mm, 70.0);
    assert_eq!(restored.camera.color_filter, "hoya_y2");
    assert_eq!(restored.film_render.grain.micro_sublayers, 3);
    assert_eq!(
        restored.film_render.dir_couplers.langmuir_donor_k_rgb,
        [0.5, 1.5, 2.5]
    );
    assert_eq!(
        restored.film_render.dir_couplers.langmuir_receiver_k_rgb,
        [0.75, 1.75, 2.75]
    );
    assert_eq!(restored.io.input_gamut_compress.hull_detail, 9.0);
    assert_eq!(restored.io.input_gamut_compress.boundary, "locus");
    assert_eq!(restored.film_render.development_time, Some(9.0));
    assert_eq!(restored.print_render.development_time, Some(4.0));
    assert_eq!(restored.film_render.chemistry.gamma_factor_red, 1.25);
    assert_eq!(
        restored.print_render.density_curves_morph.gamma_factor_blue,
        0.75
    );
    assert_eq!(restored.film_render.base.tilt, 0.2);
    assert_eq!(restored.film_render.base.cyan, 1.2);
    assert_eq!(restored.print_render.base.scale, 0.8);
    assert_eq!(restored.film_render.convert.base_percentile, 97.5);
    assert_eq!(
        restored.film_render.convert.calibration,
        "1.1 0 0  0 1 0  0 0 0.9"
    );
    assert_eq!(
        restored.workflow.route,
        "input > convert-film > scan-minus-base"
    );
    assert_eq!(loaded.sections, saved);
    assert!(saved["simulation"].get("workflow").is_none());
    assert!(saved["simulation"].get("scan_film").is_none());
    assert!(saved["camera"].get("filter_uv").is_none());
    assert!(saved["camera"].get("filter_ir").is_none());
    assert!(saved["special"].get("film_gamma_factor").is_none());
}
#[test]
fn runtime_snapshot_does_not_reown_workflow_route() -> anyhow::Result<()> {
    let factory = GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.workflow.route = "input > film > scan".into();
    let saved = GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
        .unwrap();
    assert!(
        saved.sections["rust"]["runtime"]["io"]
            .get("scan_film")
            .is_none()
    );
    assert!(
        saved.sections["rust"]["runtime"]["workflow"]
            .get("route")
            .is_none()
    );
    assert_eq!(saved.sections["simulation"]["route"], "input > film > scan");
    let loaded = GuiState::from_value(saved.sections).unwrap();
    assert_eq!(
        loaded.runtime_params()?.workflow.route,
        "input > film > scan"
    );
    Ok::<(), anyhow::Error>(())
}
#[test]
fn negative_to_positive_convert_route_roundtrip() -> anyhow::Result<()> {
    let factory = GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.workflow.route = "input > convert-film > scan".into();
    params.scanner.scan_output = "negative_to_positive".into();
    params.scanner.lens_blur = 0.25;
    let saved =
        GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)?;
    assert_eq!(
        saved.sections["simulation"]["route"],
        "input > convert-film > scan"
    );
    assert_eq!(
        saved.sections["rust"]["scanner_provenance"]["model"],
        "lab_scan_v1"
    );
    assert_eq!(
        saved.sections["rust"]["scanner_provenance"]["sensor"],
        "cie_1931_cmf"
    );
    assert_eq!(
        saved.sections["rust"]["scanner_provenance"]["white_reference"],
        "clear_film_base"
    );
    assert_eq!(
        saved.sections["rust"]["scanner_provenance"]["output_white"],
        "output_color_space_white"
    );
    assert_eq!(
        saved.sections["rust"]["scanner_provenance"]["matrix"],
        "identity"
    );
    let loaded = GuiState::from_value(saved.sections)?;
    let restored = loaded.runtime_params()?;
    assert_eq!(restored.workflow.route, "input > convert-film > scan");
    assert_eq!(restored.scanner.scan_output, "negative_to_positive");
    assert_eq!(restored.scanner.lens_blur, 0.25);
    assert_eq!(restored.scanner.unsharp_mask, [0.7, 0.7]);
    Ok(())
}

#[test]
fn conversion_style_round_trips_through_the_param_set_path() -> anyhow::Result<()> {
    // The value routes through the same schema-driven parameter-set mechanism
    // as scanner.scan_output. Validation checks name sanity
    // and the negative-to-positive gate only, so the file need not exist here.
    let mut params = GuiState::factory().runtime_params()?;
    spektrafilm_core::params::sources::ParameterEdits::parse(
        "scanner.scan_output=negative_to_positive,scanner.conversion_style=warm-probe",
    )
    .map_err(anyhow::Error::msg)?
    .apply(&mut params)
    .map_err(anyhow::Error::msg)?;
    assert_eq!(params.scanner.conversion_style, "warm-probe");
    // A non-neutral style without the negative interpretation is rejected.
    let mut invalid = params.clone();
    invalid.scanner.scan_output = "direct_scan".into();
    let error = invalid.validate().unwrap_err();
    assert!(
        error.starts_with(
            "scanner.conversion_style requires scanner.scan_output=negative_to_positive"
        ),
        "{error}"
    );
    // The GUI section carries the value, and a document round-trip keeps it.
    let factory = GuiState::factory();
    params.workflow.route = "input > convert-film > scan".into();
    let saved =
        GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)?;
    assert_eq!(saved.sections["scanner"]["conversion_style"], "warm-probe");
    let restored = GuiState::from_value(saved.sections)?.runtime_params()?;
    assert_eq!(restored.scanner.conversion_style, "warm-probe");
    Ok(())
}
#[test]
fn legacy_runtime_extension_scan_output_survives_state_load() {
    // States saved before scanner.scan_output became canonical carry the
    // selection only under the Rust runtime extension.
    let state = GuiState::from_value(json!({
        "simulation": {"route": "input > convert-film > scan"},
        "rust": {"version": 1, "runtime": {"scanner": {
            "scan_output": "negative_to_positive"
        }}}
    }))
    .unwrap();
    let params = state.runtime_params().unwrap();
    assert_eq!(params.scanner.scan_output, "negative_to_positive");
    assert_eq!(
        state.sections["scanner"]["scan_output"],
        "negative_to_positive"
    );
    // An explicit canonical selection takes precedence over the extension.
    let mut canonical = state.sections.clone();
    canonical["scanner"]["scan_output"] = json!("direct_scan");
    let canonical = GuiState::from_value(canonical).unwrap();
    assert_eq!(
        canonical.runtime_params().unwrap().scanner.scan_output,
        "direct_scan"
    );
}

#[test]
fn scanner_defaults_and_invalid_combinations_use_runtime_contract() {
    let state = GuiState::from_value(json!({})).unwrap();
    let runtime_defaults = serde_json::to_value(RuntimeParams::default().scanner).unwrap();
    for key in [
        "scan_output",
        "conversion_style",
        "operator",
        "lens_blur",
        "unsharp_mask",
    ] {
        assert_eq!(state.sections["scanner"][key], runtime_defaults[key]);
    }
    for scanner in [
        json!({"scan_output": "positive_scan"}),
        json!({"conversion_style": ""}),
        json!({"conversion_style": "../etc/passwd"}),
    ] {
        assert!(GuiState::from_value(json!({"scanner": scanner})).is_err());
    }
    // A non-neutral style requires the negative interpretation.
    assert!(
        GuiState::from_value(json!({"scanner": {
            "scan_output": "negative_to_positive", "conversion_style": "warm-probe"
        }}))
        .is_ok()
    );
    assert!(
        GuiState::from_value(json!({"scanner": {
            "scan_output": "direct_scan", "conversion_style": "warm-probe"
        }}))
        .is_err()
    );
    // Retired scanner keys are no longer part of the schema.
    for scanner in [
        json!({"receiver_profile": "legacy_colorimetric"}),
        json!({"finish": "lab_scan"}),
        json!({"auto_setup": "full"}),
        json!({"finish_strength": 0.5}),
        json!({"white_point": [1.0, 1.0, 1.0]}),
    ] {
        assert!(GuiState::from_value(json!({"scanner": scanner})).is_err());
    }
}

#[test]
fn scanner_provenance_is_rederived_from_builtin_selection() {
    let state = GuiState::from_value(json!({
        "simulation": {"route": "input > film > scan"},
        "rust": {"version": 1, "scanner_provenance": {
            "model": "unavailable model", "sensor": "unavailable sensor"
        }}
    }))
    .unwrap();
    assert_eq!(
        state.sections["rust"]["scanner_provenance"]["model"],
        "lab_scan_v1"
    );
    assert_eq!(
        state.sections["rust"]["scanner_provenance"]["sensor"],
        "cie_1931_cmf"
    );
}

#[test]
fn factory_enlarger_neutral_filters_survive_state_conversion() {
    let factory = GuiState::factory();
    let params = factory.runtime_params().unwrap();
    assert_eq!(params.enlarger.m_filter_neutral, 58.8453);
    assert_eq!(params.enlarger.y_filter_neutral, 55.2848);

    let loaded = GuiState::from_value(factory.sections.clone()).unwrap();
    let restored = loaded.runtime_params().unwrap();
    assert_eq!(restored.enlarger.m_filter_neutral, 58.8453);
    assert_eq!(restored.enlarger.y_filter_neutral, 55.2848);
}

#[test]
fn canonical_chemistry_base_and_route_override_runtime_extension() {
    let state = GuiState::from_value(json!({
        "film_chemistry": {"development_time": 9.0, "gamma_factor_red": 1.25},
        "chemistry": {"development_time": 4.0, "gamma_factor_blue": 0.75},
        "film_base": {"cyan": 1.2},
        "print_base": {"scale": 0.8},
        "convert": {"base_percentile": 97.5},
        "simulation": {"route": "input > film > scan"},
        "rust": {"version": 1, "runtime": {
            "film_render": {"development_time": 5.0, "chemistry": {"gamma_factor_red": 2.0}, "base": {"cyan": 2.0}, "convert": {"base_percentile": 99.0}},
            "print_render": {"development_time": 2.0, "density_curves_morph": {"gamma_factor_blue": 2.0}, "base": {"scale": 2.0}},
            "workflow": {"route": "input"}
        }}
    })).unwrap();
    let params = state.runtime_params().unwrap();
    assert_eq!(params.film_render.development_time, Some(9.0));
    assert_eq!(params.print_render.development_time, Some(4.0));
    assert_eq!(params.film_render.chemistry.gamma_factor_red, 1.25);
    assert_eq!(
        params.print_render.density_curves_morph.gamma_factor_blue,
        0.75
    );
    assert_eq!(params.film_render.base.cyan, 1.2);
    assert_eq!(params.print_render.base.scale, 0.8);
    assert_eq!(params.film_render.convert.base_percentile, 97.5);
    assert_eq!(params.workflow.route, "input > film > scan");
}

#[test]
fn upstream_legacy_scan_flag_is_read_only_and_route_wins() {
    let legacy = GuiState::from_value(json!({"simulation": {"scan_film": true}})).unwrap();
    assert_eq!(
        legacy.runtime_params().unwrap().workflow.route,
        "input > film > scan"
    );
    assert!(legacy.sections["simulation"].get("scan_film").is_none());
    let explicit =
        GuiState::from_value(json!({"simulation": {"scan_film": true, "route": "input"}})).unwrap();
    assert_eq!(explicit.runtime_params().unwrap().workflow.route, "input");
}

#[test]
fn nested_upstream_state_flattens_only_owned_workflow_leaves() {
    let state = GuiState::from_value(json!({
        "input_image": {"io": {"crop": true}, "settings": {"spectral_gaussian_blur": 2.0}},
        "simulation": {
            "selection": {"film_stock": "kodak_doublex"},
            "route": "input > film > scan",
            "workflow": {"route": "input", "auto_preview": false, "saving_color_space": "ITU-R BT.2020"},
            "enlarger": {"print_exposure": 1.5, "y_filter_shift": 7.0},
            "io": {"output_color_space": "ITU-R BT.2020"}
        }
    })).unwrap();
    let params = state.runtime_params().unwrap();
    assert!(params.io.crop);
    assert_eq!(params.settings.spectral_gaussian_blur, 2.0);
    assert_eq!(params.workflow.route, "input > film > scan");
    assert_eq!(params.enlarger.print_exposure, 1.5);
    assert_eq!(params.enlarger.y_filter_shift, 7.0);
    assert_eq!(params.io.output_color_space, "ITU-R BT.2020");
    assert_eq!(state.film(), "kodak_doublex");
    assert!(state.sections["simulation"].get("auto_preview").is_none());
    assert_eq!(
        state.sections["simulation"]["saving_color_space"],
        "ITU-R BT.2020"
    );
    assert!(state.sections["simulation"].get("workflow").is_none());
}

#[test]
fn retired_auto_preview_is_ignored_and_omitted_on_save() {
    let directory = tempfile::tempdir().unwrap();
    for value in [
        json!({"simulation": {"auto_preview": false}}),
        json!({"simulation": {"auto_preview": "false"}}),
        json!({"simulation": {"workflow": {"auto_preview": false}}}),
    ] {
        let state = GuiState::from_value(value).unwrap();
        assert!(state.sections["simulation"].get("auto_preview").is_none());
        let path = directory.path().join("migrated.json");
        state.save(&path).unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(saved["simulation"].get("auto_preview").is_none());
        assert!(saved["simulation"].get("workflow").is_none());
    }
}

#[test]
fn malformed_controls_and_future_extensions_fail_before_application() {
    for value in [
        json!([]),
        json!({"camera":{"auto_exposure":"bad"}}),
        json!({"rust":{"version":9}}),
        json!({"special":{"film_channel_swap":[0,1,3]}}),
    ] {
        assert!(GuiState::from_value(value).is_err());
    }
}

#[test]
fn legacy_scanner_state_migrates_light_and_style_but_current_unknown_fields_still_fail() {
    for version in 1..=4 {
        let state = GuiState::from_value(json!({
            "rust": {"version": version, "runtime": {"scanner": {
                "lamp": "D65", "conversion_style": "neutral"
            }}},
            "scanner": {"lamp": "D65", "conversion_style": "neutral"}
        }))
        .unwrap();
        assert_eq!(
            state.runtime_params().unwrap().scanner.conversion_style,
            "standard"
        );
        assert_eq!(state.sections["scanner"]["conversion_style"], "standard");
        assert!(state.sections["scanner"].get("lamp").is_none());
        assert_eq!(
            state
                .runtime_params()
                .unwrap()
                .film_render
                .convert
                .scan_illuminant,
            "D65"
        );
        let round = GuiState::from_value(state.sections.clone()).unwrap();
        assert_eq!(state.sections, round.sections);
    }
    let current = GuiState::from_value(json!({
        "rust": {"version": super::CURRENT_STATE_VERSION}, "scanner": {"lamp": "D50"}
    }));
    assert!(current.is_err());
}

#[test]
fn extension_only_capture_light_survives_legacy_and_current_state_projection() {
    let explicit = GuiState::from_value(json!({"rust":{"version":4,"runtime":{
        "film_render":{"convert":{"scan_illuminant":"D50"}}}},
        "scanner":{"lamp":"T"}}))
    .unwrap();
    assert_eq!(
        explicit
            .runtime_params()
            .unwrap()
            .film_render
            .convert
            .scan_illuminant,
        "D50"
    );

    for version in [4, super::CURRENT_STATE_VERSION] {
        let scanner = if version == 4 {
            json!({"lamp":"T"})
        } else {
            json!({})
        };
        let mut value = json!({"rust":{"version":version,"runtime":{"scanner":scanner}}});
        if version == super::CURRENT_STATE_VERSION {
            value["rust"]["runtime"]["film_render"] = json!({"convert":{"scan_illuminant":"T"}});
        }
        let state = GuiState::from_value(value).unwrap();
        assert_eq!(
            state
                .runtime_params()
                .unwrap()
                .film_render
                .convert
                .scan_illuminant,
            "T"
        );
        let saved = GuiState::from_runtime(
            &state.runtime_params().unwrap(),
            state.film(),
            state.paper(),
            &state.sections,
        )
        .unwrap();
        assert_eq!(saved.sections["convert"]["scan_illuminant"], "T");
    }
}

#[test]
fn historical_grain_control_names_migrate_before_factory_overlay() {
    for nested in [false, true] {
        let mut value = json!({"rust":{"version":CURRENT_STATE_VERSION}});
        let controls = json!({"engine":"procedural","v2_profile":"custom",
            "v2_mode":"noise","v2_amount":27.0,"v2_timer":0.25,
            "v3_dye_support_um":4.5,"v3_event_area_um2":0.6});
        if nested {
            value["rust"]["runtime"] = json!({"film_render":{"grain":controls}});
        } else {
            value["grain"] = controls;
        }
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("historical.json");
        std::fs::write(&input, serde_json::to_vec(&value).unwrap()).unwrap();
        let state = GuiState::load(&input).unwrap();
        let saved_path = directory.path().join("canonical.json");
        state.save(&saved_path).unwrap();
        let state = GuiState::load(&saved_path).unwrap();
        let stored: serde_json::Value =
            serde_json::from_slice(&std::fs::read(saved_path).unwrap()).unwrap();
        assert!(
            stored["rust"]["runtime"]["film_render"]["grain"]
                .get("v2_amount")
                .is_none()
        );
        let params = state.runtime_params().unwrap();
        let grain = &params.film_render.grain;
        assert_eq!(grain.procedural_profile, "custom");
        assert_eq!(
            grain.procedural_mode,
            spektrafilm_core::params::grain::ProceduralGrainMode::Noise
        );
        assert_eq!(grain.procedural_amount, Some(27.0));
        assert_eq!(grain.procedural_timer, 0.25);
        assert_eq!(grain.dye_support_um, 4.5);
        assert_eq!(grain.dye_event_area_um2, 0.6);
        let saved = serde_json::to_value(params).unwrap();
        assert!(saved["film_render"]["grain"].get("v2_amount").is_none());
        assert_eq!(
            saved["film_render"]["grain"]["procedural_amount"],
            json!(27.0)
        );
    }
    for pointer in ["/grain", "/rust/runtime/film_render/grain"] {
        let value = if pointer == "/grain" {
            json!({"grain":{"v2_amount":20.0,"procedural_amount":30.0}})
        } else {
            json!({"rust":{"version":CURRENT_STATE_VERSION,"runtime":{"film_render":{"grain":{"v2_amount":20.0,"procedural_amount":30.0}}}}})
        };
        assert!(GuiState::from_value(value).is_err());
    }
}

#[test]
fn cross_carrier_grain_aliases_retain_top_level_precedence_and_ownership() {
    let value = json!({"grain":{"engine":"procedural","v2_amount":27.0},"rust":{"version":CURRENT_STATE_VERSION,
        "runtime":{"film_render":{"grain":{"procedural_amount":42.0}}},
        "render_preparation":{"explicit_section_paths":["film_render.grain.v2_amount"],"neutral_print_filters_protected":[false,false,false]}}});
    let state = GuiState::from_value(value).unwrap();
    let params = state.runtime_params().unwrap();
    assert_eq!(params.film_render.grain.procedural_amount, Some(27.0));
    assert_eq!(
        params.render_preparation_state().explicit_section_paths,
        ["film_render.grain.procedural_amount"]
    );
}
