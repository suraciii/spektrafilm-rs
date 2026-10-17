use super::*;
use crate::test_support::data_dir;
fn request(sources: &[String]) -> ProcessParamsRequest<'_> {
    ProcessParamsRequest {
        film: Some("kodak_portra_400"),
        paper: None,
        presets: &[],
        params_file: None,
        sources,
        route: None,
        scan_film: false,
        scan_output: None,
        input_is_raw: false,
        input_color_space: None,
        input_cctf_decoding: None,
        require_input_declaration: false,
        digest_mode: DigestMode::ApplyStockSpecifics,
    }
}
#[test]
fn legacy_resolution_matches_original_digest() {
    let sources = [];
    let result = resolve(request(&sources), &data_dir()).unwrap();
    let neutral = NeutralFilters::load(&data_dir()).unwrap();
    let original = digest_params(
        RuntimeParams::default(),
        &result.film,
        &result.print,
        Some(&neutral),
        true,
    );
    assert_eq!(
        serde_json::to_value(result.params).unwrap(),
        serde_json::to_value(original).unwrap()
    );
}

#[test]
fn internal_protocol_retains_admitted_scan_topology() {
    let sources = [];
    for (route, scan_film) in [
        ("input > film > scan", false),
        ("input > film > print > scan", true),
    ] {
        let mut req = request(&sources);
        req.route = Some(route);
        req.scan_film = scan_film;
        let resolved = resolve(req, &data_dir()).unwrap();
        assert!(resolved.params.io.scan_film);
        assert_eq!(resolved.params.workflow.route, route);
        assert_eq!(
            resolved.print_name,
            if scan_film || WorkflowRoute::parse(route) == WorkflowRoute::FilmScan {
                "kodak_portra_400"
            } else {
                "kodak_portra_endura"
            }
        );
        let expected = serde_json::to_value(&resolved.params).unwrap();
        let runtime = crate::runtime::Runtime::new(
            resolved.film,
            resolved.print,
            resolved.params,
            &data_dir(),
        )
        .unwrap();
        assert_eq!(serde_json::to_value(runtime.params()).unwrap(), expected);
    }
}

#[test]
fn raw_legacy_resolution_retains_original_digest_parameters() {
    let sources = [];
    let mut req = request(&sources);
    req.input_is_raw = true;
    let resolved = resolve(req, &data_dir()).unwrap();
    let mut baseline = RuntimeParams::default();
    force_raw_input(&mut baseline);
    let neutral = NeutralFilters::load(&data_dir()).unwrap();
    let original = digest_params(
        baseline,
        &resolved.film,
        &resolved.print,
        Some(&neutral),
        true,
    );
    assert_eq!(
        serde_json::to_value(resolved.params).unwrap(),
        serde_json::to_value(original).unwrap()
    );
}

#[test]
fn monochrome_reports_match_runtime_for_legacy_and_added_sources() {
    let mut baseline = RuntimeParams::default();
    baseline.settings.rgb_to_raw_method = "mallett2019".into();
    baseline.film_render.grain.particle_scale = [1.0, 2.0, 3.0];
    baseline.film_render.grain.rms_granularity = [1.0, 2.0, 3.0];
    baseline.film_render.grain.density_min = [0.1, 0.2, 0.3];
    baseline.film_render.grain.uniformity = [0.5, 0.6, 0.7];
    baseline.film_render.dir_couplers.gamma_samelayer_rgb = [0.1, 0.2, 0.3];
    baseline.film_render.dir_couplers.gamma_interlayer_r_to_gb = [0.1, 0.2];
    baseline.film_render.dir_couplers.gamma_interlayer_g_to_rb = [0.2, 0.3];
    baseline.film_render.dir_couplers.gamma_interlayer_b_to_rg = [0.3, 0.4];
    baseline.film_render.halation.halation_strength = [0.1, 0.2, 0.3];
    baseline.film_render.halation.scatter_core_um = [1.0, 2.0, 3.0];
    baseline.film_render.halation.scatter_tail_um = [4.0, 5.0, 6.0];
    baseline.film_render.halation.scatter_tail_weight = [0.1, 0.2, 0.3];
    baseline.film_render.halation.halation_first_sigma_um = [7.0, 8.0, 9.0];
    let print_controls = serde_json::to_value(&baseline.print_render).unwrap();
    let path = std::env::temp_dir().join(format!(
        "spektrafilm-process-monochrome-{}.json",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_vec(&baseline).unwrap()).unwrap();
    let film = profile::load_profile_by_name(&data_dir(), "kodak_doublex").unwrap();
    let print = profile::load_profile_by_name(&data_dir(), "kodak_portra_endura").unwrap();
    let film_profile = serde_json::to_value(&film).unwrap();
    let print_profile = serde_json::to_value(&print).unwrap();
    for sources in [vec![], vec!["camera.auto_exposure=false".into()]] {
        let mut req = request(&sources);
        req.film = Some("kodak_doublex");
        req.paper = Some("kodak_portra_endura");
        req.params_file = Some(&path);
        req.digest_mode = DigestMode::PreserveUserEdits;
        let resolved = resolve(req, &data_dir()).unwrap();
        let params = &resolved.params;
        assert!(params.film_render.grain.monochrome);
        assert_eq!(params.film_render.grain.particle_scale, [1.0; 3]);
        assert_eq!(params.film_render.grain.rms_granularity, [1.0; 3]);
        assert_eq!(params.film_render.grain.density_min, [0.1; 3]);
        assert_eq!(params.film_render.grain.uniformity, [0.5; 3]);
        assert_eq!(
            params.film_render.dir_couplers.gamma_samelayer_rgb,
            [0.1; 3]
        );
        assert_eq!(
            params.film_render.dir_couplers.gamma_interlayer_r_to_gb,
            [0.0; 2]
        );
        assert_eq!(
            params.film_render.dir_couplers.gamma_interlayer_g_to_rb,
            [0.0; 2]
        );
        assert_eq!(
            params.film_render.dir_couplers.gamma_interlayer_b_to_rg,
            [0.0; 2]
        );
        assert_eq!(params.film_render.halation.halation_strength, [0.1; 3]);
        assert_eq!(params.film_render.halation.scatter_core_um, [1.0; 3]);
        assert_eq!(params.film_render.halation.scatter_tail_um, [4.0; 3]);
        assert_eq!(params.film_render.halation.scatter_tail_weight, [0.1; 3]);
        assert_eq!(
            params.film_render.halation.halation_first_sigma_um,
            [7.0; 3]
        );
        assert_eq!(
            serde_json::to_value(&params.print_render).unwrap(),
            print_controls
        );
        assert_eq!(serde_json::to_value(&resolved.film).unwrap(), film_profile);
        assert_eq!(
            serde_json::to_value(&resolved.print).unwrap(),
            print_profile
        );
        let expected = serde_json::to_value(params).unwrap();
        let runtime = crate::runtime::Runtime::new(
            resolved.film,
            resolved.print,
            resolved.params,
            &data_dir(),
        )
        .unwrap();
        assert_eq!(serde_json::to_value(runtime.params()).unwrap(), expected);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sparse_legacy_params_retain_field_deserialization_defaults() {
    let path = std::env::temp_dir().join(format!(
        "spektrafilm-process-legacy-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &path,
        r#"{"film_render":{"grain":{}},"enlarger":{"m_filter_neutral":12}}"#,
    )
    .unwrap();
    let sources = vec!["camera.auto_exposure=false".into()];
    let mut req = request(&sources);
    req.params_file = Some(&path);
    let result = resolve(req, &data_dir()).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(result.params.film_render.grain.blur, 0.65);
    assert_eq!(result.params.film_render.grain.blur_dye_clouds_um, 1.0);
    let neutral = NeutralFilters::load(&data_dir()).unwrap();
    assert_eq!(
        result.params.enlarger.m_filter_neutral,
        neutral
            .lookup("kodak_portra_endura", "TH-KG3", "kodak_portra_400")
            .unwrap()[1] as f32
    );
}

#[test]
fn preset_neutral_axes_remain_explicit_after_later_controls() {
    let sources = vec!["enlarger.illuminant=\"D65\"".into()];
    let mut req = request(&sources);
    req.film = None;
    let documents = vec![presets::load_selector("Kodak Portra 400", &data_dir()).unwrap()];
    req.presets = &documents;
    let result = resolve(req, &data_dir()).unwrap();
    let look = presets::load_selector("Kodak Portra 400", &data_dir()).unwrap();
    assert_eq!(
        result.params.enlarger.m_filter_neutral,
        look.look_parameters().enlarger.m_filter_neutral
    );
    assert_eq!(
        result.params.enlarger.y_filter_neutral,
        look.look_parameters().enlarger.y_filter_neutral
    );
    assert!(result.params.settings.neutral_print_filters_from_database);
}

#[test]
fn disabling_database_calibration_preserves_unprotected_baseline() {
    let sources = vec!["settings.neutral_print_filters_from_database=false".into()];
    let result = resolve(request(&sources), &data_dir()).unwrap();
    assert_eq!(result.params.enlarger.m_filter_neutral, 65.0);
    assert_eq!(result.params.enlarger.y_filter_neutral, 55.0);
    assert!(!result.params.settings.neutral_print_filters_from_database);
}

#[test]
fn final_constraints_can_disable_an_explicit_effect() {
    let sources =
        vec!["debug.deactivate_stochastic_effects=true,film_render.grain.active=true".into()];
    let result = resolve(request(&sources), &data_dir()).unwrap();
    assert!(!result.params.film_render.grain.active);
}

#[test]
fn internal_preserve_policy_does_not_apply_stock_initialization() {
    let sources = vec!["camera.auto_exposure=false".into()];
    let mut req = request(&sources);
    req.digest_mode = DigestMode::PreserveUserEdits;
    let result = resolve(req, &data_dir()).unwrap();
    assert_eq!(
        result.params.film_render.grain.rms_granularity,
        RuntimeParams::default().film_render.grain.rms_granularity
    );
}

#[test]
fn missing_final_illuminant_calibration_retains_baseline() {
    let sources = vec!["enlarger.illuminant=\"D65\"".into()];
    let result = resolve(request(&sources), &data_dir()).unwrap();
    let neutral = NeutralFilters::load(&data_dir()).unwrap();
    let expected = neutral.lookup("kodak_portra_endura", "D65", "kodak_portra_400");
    let baseline = RuntimeParams::default();
    assert_eq!(
        result.params.enlarger.m_filter_neutral,
        expected
            .map(|values| values[1] as f32)
            .unwrap_or(baseline.enlarger.m_filter_neutral)
    );
    assert_eq!(
        result.params.enlarger.y_filter_neutral,
        expected
            .map(|values| values[2] as f32)
            .unwrap_or(baseline.enlarger.y_filter_neutral)
    );
}
#[test]
fn explicit_stock_values_and_individual_filters_survive() {
    let sources =
        vec!["film_render.grain.rms_granularity=[1,2,3],enlarger.m_filter_neutral=65".into()];
    let result = resolve(request(&sources), &data_dir()).unwrap();
    assert_eq!(
        result.params.film_render.grain.rms_granularity,
        [1.0, 2.0, 3.0]
    );
    assert_eq!(result.params.enlarger.m_filter_neutral, 65.0);
    assert_ne!(result.params.enlarger.y_filter_neutral, 55.0);
    assert!(result.params.settings.neutral_print_filters_from_database);
}
#[test]
fn final_preview_switch_does_not_leave_intermediate_damage() {
    let sources = vec![
        "settings.preview_mode=true".into(),
        "settings.preview_mode=false,film_render.grain.active=true".into(),
    ];
    let result = resolve(request(&sources), &data_dir()).unwrap();
    assert!(result.params.film_render.grain.active);
    assert_ne!(result.params.film_render.grain.rms_granularity, [0.0; 3]);
}
#[test]
fn internal_protocol_route_precedes_scan_flag_with_sources() {
    let sources = vec!["camera.auto_exposure=false".into()];
    let mut req = request(&sources);
    let documents = vec![presets::load_selector("Kodak Portra 400", &data_dir()).unwrap()];
    req.presets = &documents;
    req.film = None;
    req.paper = None;
    req.scan_film = true;
    req.route = Some("input > film > print > scan");
    let result = resolve(req, &data_dir()).unwrap();
    assert!(!result.params.io.scan_film);
    assert_eq!(result.print_name, "kodak_portra_endura");
}
#[test]
fn raw_rejects_explicit_conflicts_but_normalizes_baseline() {
    let sources = vec!["io.input_cctf_decoding=true".into()];
    let mut req = request(&sources);
    req.input_is_raw = true;
    assert!(resolve(req, &data_dir()).err().unwrap().contains("RAW"));
    let sources = vec!["io.input_cctf_decoding=true,io.input_cctf_decoding=false".into()];
    let mut req = request(&sources);
    req.input_is_raw = true;
    assert!(resolve(req, &data_dir()).err().unwrap().contains("RAW"));
    let sources = vec!["camera.auto_exposure=false".into()];
    let mut req = request(&sources);
    req.input_is_raw = true;
    let result = resolve(req, &data_dir()).unwrap();
    assert_eq!(result.params.io.input_color_space, "ACES2065-1");
    assert!(!result.params.io.input_cctf_decoding);
}

#[test]
fn resolved_explicit_axes_survive_runtime_construction_and_updates() {
    let sources = vec![
        "enlarger.c_filter_neutral=12,enlarger.m_filter_neutral=34,enlarger.y_filter_neutral=56"
            .into(),
        "settings.rgb_to_raw_method=\"mallett2019\"".into(),
    ];
    let resolved = resolve(request(&sources), &data_dir()).unwrap();
    let mut runtime =
        crate::runtime::Runtime::new(resolved.film, resolved.print, resolved.params, &data_dir())
            .unwrap();
    let assert_axes = |params: &RuntimeParams| {
        assert_eq!(params.enlarger.c_filter_neutral, 12.0);
        assert_eq!(params.enlarger.m_filter_neutral, 34.0);
        assert_eq!(params.enlarger.y_filter_neutral, 56.0);
        assert!(params.settings.neutral_print_filters_from_database);
    };
    assert_axes(runtime.params());
    let mut updated = runtime.params().clone();
    updated.enlarger.m_filter_shift = 4.0;
    runtime = runtime.with_params(updated).unwrap();
    assert_axes(runtime.params());
    let mut updated = runtime.params().clone();
    updated.camera.exposure_compensation_ev = 0.5;
    runtime.update(updated).unwrap();
    assert_axes(runtime.params());
    let saved = serde_json::to_value(runtime.params()).unwrap();
    // The protection flags persist so a resolved document replays its render.
    assert!(saved.get("neutral_print_filters_protected").is_some());
    let restored: RuntimeParams = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.neutral_print_filters_protected, Some([true; 3]));
}

#[test]
fn preset_resolver_protects_actual_runtime_axes_for_all_consumers() {
    let mut look = presets::load_selector("Kodak Portra 400", &data_dir()).unwrap();
    look.look_parameters_mut().enlarger.c_filter_neutral = 12.0;
    look.look_parameters_mut().enlarger.m_filter_neutral = 34.0;
    look.look_parameters_mut().enlarger.y_filter_neutral = 56.0;
    let mut context = RuntimeParams::default();
    context.settings.rgb_to_raw_method = "mallett2019".into();
    let resolved = look.resolve(&data_dir(), &context).unwrap();
    let runtime =
        crate::runtime::Runtime::new(resolved.film, resolved.print, resolved.params, &data_dir())
            .unwrap();
    assert_eq!(runtime.params().enlarger.c_filter_neutral, 12.0);
    assert_eq!(runtime.params().enlarger.m_filter_neutral, 34.0);
    assert_eq!(runtime.params().enlarger.y_filter_neutral, 56.0);
    assert!(
        runtime
            .params()
            .settings
            .neutral_print_filters_from_database
    );
    let resolved = look.resolve(&data_dir(), &context).unwrap();
    let photo = crate::runtime::RuntimePhotoParams {
        film: resolved.film,
        print: resolved.print,
        params: resolved.params,
        data_dir: data_dir(),
    };
    let runtime = photo.into_runtime(DigestMode::PreserveUserEdits).unwrap();
    assert_eq!(runtime.params().enlarger.c_filter_neutral, 12.0);
    assert_eq!(runtime.params().enlarger.m_filter_neutral, 34.0);
    assert_eq!(runtime.params().enlarger.y_filter_neutral, 56.0);
    assert!(
        runtime
            .params()
            .settings
            .neutral_print_filters_from_database
    );
}

#[test]
fn partial_axis_calibration_preserves_unprotected_database_precision() {
    let sources =
        vec!["enlarger.m_filter_neutral=34,settings.rgb_to_raw_method=\"mallett2019\"".into()];
    let resolved = resolve(request(&sources), &data_dir()).unwrap();
    let db = NeutralFilters::load(&data_dir()).unwrap();
    let [c, m, y] = db
        .lookup(
            &resolved.print_name,
            &resolved.params.enlarger.illuminant,
            &resolved.film_name,
        )
        .unwrap();
    let illuminant = resolved.params.enlarger.illuminant.clone();
    let pipeline = crate::pipeline::Pipeline::new_with_spectral(
        resolved.film,
        resolved.print,
        resolved.params,
        &data_dir(),
    )
    .unwrap();
    assert_eq!(pipeline.params.enlarger.c_filter_neutral, c as f32);
    assert_eq!(pipeline.params.enlarger.m_filter_neutral, 34.0);
    assert_eq!(pipeline.params.enlarger.y_filter_neutral, y as f32);
    let expected = crate::enlarger::enlarger_filtered_illuminant_f64(&illuminant, c, 34.0, y);
    assert_eq!(pipeline.print_illuminant_slice(), expected);
    let database_only = crate::enlarger::enlarger_filtered_illuminant_f64(&illuminant, c, m, y);
    assert_ne!(pipeline.print_illuminant_slice(), database_only);
    let narrowed = crate::enlarger::enlarger_filtered_illuminant_f64(
        &illuminant,
        c as f32 as f64,
        34.0,
        y as f32 as f64,
    );
    assert_ne!(pipeline.print_illuminant_slice(), narrowed);

    // Ownership alone changes calibration, even when the public values match.
    let mut updated = pipeline.params.clone();
    updated.neutral_print_filters_protected = Some([false; 3]);
    let unprotected = pipeline.clone().with_params(updated).unwrap();
    assert_eq!(unprotected.print_illuminant_slice(), database_only);
    let mut updated = unprotected.params.clone();
    updated.neutral_print_filters_protected = Some([true; 3]);
    let protected = unprotected.with_params(updated).unwrap();
    let promoted = crate::enlarger::enlarger_filtered_illuminant_f64(
        &illuminant,
        c as f32 as f64,
        m as f32 as f64,
        y as f32 as f64,
    );
    assert_eq!(protected.print_illuminant_slice(), promoted);
}

#[test]
fn added_sources_accept_vision3_printing_film_target() {
    let sources = vec!["camera.auto_exposure=false".into()];
    for name in [
        "kodak_vision3_50d",
        "kodak_vision3_250d",
        "kodak_vision3_200t",
        "kodak_vision3_500t",
    ] {
        let mut req = request(&sources);
        req.film = Some(name);
        let resolved = resolve(req, &data_dir()).unwrap();
        assert_eq!(resolved.print_name, "kodak_2383");
        assert!(resolved.print.is_film());
        assert!(resolved.print.is_printing());
        assert!(!resolved.params.camera.auto_exposure);

        let mut req = request(&sources);
        req.film = Some(name);
        req.scan_film = true;
        let scanned = resolve(req, &data_dir()).unwrap();
        assert_eq!(scanned.print_name, name);
        assert!(scanned.params.io.scan_film);
    }
}

fn public_request(sources: &[String]) -> ProcessParamsRequest<'_> {
    let mut req = request(sources);
    req.require_input_declaration = true;
    req.input_color_space = Some("sRGB");
    req.input_cctf_decoding = Some(false);
    req
}

#[test]
fn public_raster_requires_both_supplied_components() {
    for (color, decoding) in [(None, None), (Some("sRGB"), None), (None, Some(false))] {
        let mut req = public_request(&[]);
        req.input_color_space = color;
        req.input_cctf_decoding = decoding;
        assert!(
            resolve(req, &data_dir())
                .err()
                .unwrap()
                .contains("declare both")
        );
    }
    let resolved = resolve(public_request(&[]), &data_dir()).unwrap();
    assert!(!resolved.input_declaration.cctf_decoding);
    assert_eq!(
        resolved.input_declaration.sources.cctf_decoding.kind,
        "argument"
    );
}

#[test]
fn legacy_supplied_declaration_does_not_protect_other_baseline_values() {
    let path = std::env::temp_dir().join(format!(
        "spektrafilm-input-declaration-{}.json",
        std::process::id()
    ));
    std::fs::write(&path, r#"{"io":{"input_color_space":"sRGB","input_cctf_decoding":false},"enlarger":{"m_filter_neutral":12}}"#).unwrap();
    let mut req = public_request(&[]);
    req.params_file = Some(&path);
    req.input_color_space = None;
    req.input_cctf_decoding = None;
    let result = resolve(req, &data_dir()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(result.input_declaration.sources.color_space.kind, "params");
    assert!(!result.input_declaration.cctf_decoding);
    assert_ne!(result.params.enlarger.m_filter_neutral, 12.0);
}

#[test]
fn dedicated_declaration_overrides_winning_set_with_typed_feedback() {
    let sources = vec!["io.input_color_space=ProPhoto RGB,io.input_cctf_decoding=true,camera.exposure_compensation_ev=.5".into()];
    let result = resolve(public_request(&sources), &data_dir()).unwrap();
    assert_eq!(result.input_declaration.color_space, "sRGB");
    assert!(!result.input_declaration.cctf_decoding);
    assert_eq!(result.edits[0].status, EditStatus::Active);
    for edit in &result.edits[1..] {
        assert_eq!(edit.status, EditStatus::Overridden);
        assert_eq!(edit.source.index, Some(1));
    }
    assert_eq!(result.edits[1].effective, Value::Bool(false));
}

#[test]
fn raw_rejects_early_sources_even_when_dedicated_flags_replace_them() {
    let sources = vec![
        "io.input_color_space=sRGB".into(),
        "io.input_color_space=ACES2065-1".into(),
    ];
    let mut req = public_request(&sources);
    req.input_is_raw = true;
    req.input_color_space = Some("ACES2065-1");
    assert!(resolve(req, &data_dir()).err().unwrap().contains("RAW"));
    let mut req = public_request(&[]);
    req.input_is_raw = true;
    req.input_color_space = None;
    req.input_cctf_decoding = None;
    let result = resolve(req, &data_dir()).unwrap();
    assert_eq!(result.input_declaration.sources.color_space.kind, "raw");
}

#[test]
fn public_routes_have_identical_topology_with_noop_sources() {
    for route in crate::params::sources::supported_routes() {
        let mut results = Vec::new();
        for sources in [vec![], vec!["camera.auto_exposure=true".into()]] {
            let mut req = public_request(&sources);
            req.route = Some(route);
            let result = resolve(req, &data_dir()).unwrap();
            assert_eq!(
                result.params.io.scan_film,
                WorkflowRoute::parse(route) == WorkflowRoute::FilmScan
            );
            results.push((
                result.params.workflow.route,
                result.params.io.scan_film,
                result.print_name,
            ));
        }
        assert_eq!(results[0], results[1], "{route}");
    }
}

#[test]
fn feedback_reports_conditions_constraints_and_only_winning_leaves() {
    let sources = vec![
        "film_render.grain.engine=procedural,film_render.grain.v2_profile=35mm250,film_render.grain.v2_size=12,film_render.grain.v2_amount=20,camera.lens_blur_um=2".into(),
        "film_render.grain.v2_amount=25,settings.preview_mode=true".into(),
    ];
    let result = resolve(public_request(&sources), &data_dir()).unwrap();
    let edit = |path: &str| result.edits.iter().find(|edit| edit.path == path).unwrap();
    assert_eq!(
        edit("film_render.grain.v2_size").status,
        EditStatus::Inactive
    );
    assert!(
        edit("film_render.grain.v2_size")
            .reason
            .as_ref()
            .unwrap()
            .contains("custom")
    );
    assert_eq!(
        edit("film_render.grain.v2_amount").status,
        EditStatus::Active
    );
    assert_eq!(edit("film_render.grain.v2_amount").source.index, Some(2));
    assert_eq!(edit("camera.lens_blur_um").status, EditStatus::Constrained);
    assert_eq!(
        result
            .edits
            .iter()
            .filter(|edit| edit.path == "film_render.grain.v2_amount")
            .count(),
        1
    );
}

#[test]
fn material_feedback_retains_inactive_values_on_non_development_routes() {
    for route in [
        "input",
        "input > convert-film > scan",
        "input > film > scan",
    ] {
        let sources = vec!["enlarger.preflash_exposure=0.2,film_render.chemistry.developer_exhaustion=0.3,print_render.density_curves_morph.developer_exhaustion=0.4".into()];
        let mut req = public_request(&sources);
        req.route = Some(route);
        let result = resolve(req, &data_dir()).unwrap();
        assert_eq!(result.params.enlarger.preflash_exposure, 0.2);
        assert_eq!(
            result.params.film_render.chemistry.developer_exhaustion,
            0.3
        );
        assert_eq!(
            result
                .params
                .print_render
                .density_curves_morph
                .developer_exhaustion,
            0.4
        );
        for edit in &result.edits {
            if edit.path.starts_with("film_render.")
                && WorkflowRoute::parse(route) == WorkflowRoute::FilmScan
            {
                continue;
            }
            assert_eq!(
                edit.status,
                EditStatus::Inactive,
                "{} on {route}",
                edit.path
            );
            assert!(edit.reason.as_ref().unwrap().contains("development"));
        }
    }
}

#[test]
fn current_params_reject_invalid_mix_and_retirement_before_overrides() {
    let path = std::env::temp_dir().join(format!(
        "spektrafilm-material-baseline-{}.json",
        std::process::id()
    ));
    for value in [
        serde_json::json!({"output_mix":{"amount":1.1}}),
        serde_json::json!({"io":{"output_gamut_compress":{"algorithm":"off","lightness_compression":null}}}),
    ] {
        let text = value.to_string();
        std::fs::write(&path, &text).unwrap();
        let sources = vec!["output_mix.amount=1".into()];
        let mut req = public_request(&sources);
        req.params_file = Some(&path);
        let error = resolve(req, &data_dir()).err().unwrap();
        assert!(
            error.contains("output_mix.amount")
                || error.contains("io.output_gamut_compress.lightness_compression")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn chemistry_feedback_identifies_disabled_and_unsupported_models() {
    let sources = vec![
        "film_render.chemistry.active=false,film_render.chemistry.developer_exhaustion=0.3".into(),
    ];
    let result = resolve(public_request(&sources), &data_dir()).unwrap();
    let exhaustion = result
        .edits
        .iter()
        .find(|edit| edit.path.ends_with("developer_exhaustion"))
        .unwrap();
    assert_eq!(exhaustion.status, EditStatus::Inactive);
    assert_eq!(
        result.params.film_render.chemistry.developer_exhaustion,
        0.3
    );

    let mut film = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    let mut print = profile::load_profile_by_name(&data_dir(), "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.film_render.chemistry.active = true;
    params.film_render.chemistry.developer_exhaustion = 0.3;
    params.print_render.density_curves_morph.active = true;
    params
        .print_render
        .density_curves_morph
        .developer_exhaustion = 0.4;
    let stored = serde_json::to_value(&params).unwrap();
    let paths = [
        "film_render.chemistry.developer_exhaustion",
        "print_render.density_curves_morph.developer_exhaustion",
    ];
    for path in paths {
        assert!(crate::params::sources::inactive_reason(path, &params, &film, &print).is_none());
    }
    film.data.density_curves_model = None;
    print.data.density_curves_model = None;
    for path in paths {
        assert!(crate::params::sources::inactive_reason(path, &params, &film, &print).is_some());
        assert_eq!(serde_json::to_value(&params).unwrap(), stored);
    }
}
