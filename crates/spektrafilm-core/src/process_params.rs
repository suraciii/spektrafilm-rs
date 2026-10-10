//! Process-only composition of legacy baselines, complete looks and sparse edits.
use crate::neutral_filters::NeutralFilters;
use crate::params::RuntimeParams;
use crate::params::sources::{ParameterEdits, validate_finite};
use crate::params_builder::{
    apply_database_neutral_print_filters_protected, apply_film_specifics,
    apply_runtime_constraints, digest_params, normalize_runtime_topology,
};
use crate::presets;
use crate::profile::{self, Profile};
use crate::runtime::DigestMode;
use std::path::Path;

pub struct ProcessParamsRequest<'a> {
    pub film: Option<&'a str>,
    pub paper: Option<&'a str>,
    pub preset: Option<&'a str>,
    pub params_file: Option<&'a Path>,
    pub sources: &'a [String],
    pub route: Option<&'a str>,
    pub scan_film: bool,
    pub input_is_raw: bool,
    pub digest_mode: DigestMode,
}

pub struct ResolvedProcessParams {
    pub film: Profile,
    pub print: Profile,
    pub film_name: String,
    pub print_name: String,
    pub params: RuntimeParams,
}

/// Return effective parameters ready for `Runtime::new`, without a second digest.
pub fn resolve(
    request: ProcessParamsRequest<'_>,
    data_dir: &Path,
) -> Result<ResolvedProcessParams, String> {
    if request.preset.is_some()
        && (request.film.is_some() || request.paper.is_some() || request.params_file.is_some())
    {
        return Err("--preset conflicts with --film, --paper and --params".into());
    }
    let mut params = load_baseline(request.params_file)?;
    let added_sources = request.preset.is_some() || !request.sources.is_empty();
    let (film, print, film_name, print_name) = if let Some(selector) = request.preset {
        let preset = presets::load_selector(selector, data_dir)?;
        let resolved = preset.resolve(data_dir, &params)?;
        params = resolved.params;
        (
            resolved.film,
            resolved.print,
            resolved.film_name,
            resolved.print_name,
        )
    } else {
        let name = request.film.ok_or("--film is required without --preset")?;
        let film = profile::load_profile_by_name(data_dir, name)
            .map_err(|e| format!("loading film profile {name:?}: {e}"))?;
        let print_name = if request.scan_film {
            name.to_owned()
        } else if let Some(paper) = request.paper {
            paper.to_owned()
        } else {
            film.info.target_print.clone().ok_or(
                "no paper specified and film has no target_print — use --paper or --scan-film",
            )?
        };
        let print = profile::load_profile_by_name(data_dir, &print_name)
            .map_err(|e| format!("loading print profile {print_name:?}: {e}"))?;
        if added_sources {
            if !film.is_film() || !film.is_filming() {
                return Err(format!(
                    "--film {name:?}: incompatible support/stage; expected film/filming"
                ));
            }
            if !request.scan_film
                && (!(print.is_paper() || print.is_film()) || !print.is_printing())
            {
                return Err(format!(
                    "print profile {print_name:?}: incompatible support/stage; expected paper/printing or film/printing"
                ));
            }
        }
        if added_sources && request.digest_mode == DigestMode::ApplyStockSpecifics {
            apply_film_specifics(&mut params, &film);
        }
        (film, print, name.to_owned(), print_name)
    };

    if !added_sources {
        // Keep the legacy decoder, route/scan relationship and digest ordering.
        choose_workflow(&mut params, request.route, request.scan_film);
        params.io.scan_film = request.scan_film;
        params.validate()?;
        if request.input_is_raw {
            force_raw_input(&mut params);
        }
        params.validate_color()?;
        let neutral = NeutralFilters::load(data_dir)?;
        params = digest_params(
            params,
            &film,
            &print,
            Some(&neutral),
            request.digest_mode == DigestMode::ApplyStockSpecifics,
        );
    } else {
        let mut protected = params.neutral_print_filters_protected;
        for source in request.sources {
            let edits =
                ParameterEdits::parse(source).map_err(|e| format!("--set {source:?}: {e}"))?;
            if request.input_is_raw {
                edits
                    .validate_raw_input()
                    .map_err(|e| format!("--set {source:?}: {e}"))?;
            }
            edits
                .apply(&mut params)
                .map_err(|e| format!("--set {source:?}: {e}"))?;
            for path in edits.paths() {
                match path {
                    "enlarger.c_filter_neutral" => protected[0] = true,
                    "enlarger.m_filter_neutral" => protected[1] = true,
                    "enlarger.y_filter_neutral" => protected[2] = true,
                    _ => {}
                }
            }
        }
        choose_workflow(&mut params, request.route, request.scan_film);
        params.io.scan_film = params.workflow.route == "input > film > scan";
        if request.input_is_raw {
            force_raw_input(&mut params);
        }
        // Cross-field checks precede destructive preview/debug constraints.
        validate_finite(&params)?;
        params.validate()?;
        let neutral = NeutralFilters::load(data_dir)?;
        params = apply_database_neutral_print_filters_protected(
            params,
            &film,
            &print,
            Some(&neutral),
            protected,
        );
        params.neutral_print_filters_protected = protected;
        apply_runtime_constraints(&mut params);
        validate_finite(&params)?;
        params.validate()?;
    }
    normalize_runtime_topology(&film, &mut params);
    Ok(ResolvedProcessParams {
        film,
        print,
        film_name,
        print_name,
        params,
    })
}

fn load_baseline(path: Option<&Path>) -> Result<RuntimeParams, String> {
    match path {
        Some(path) => {
            let file = std::fs::File::open(path)
                .map_err(|e| format!("opening params file {}: {e}", path.display()))?;
            serde_json::from_reader(std::io::BufReader::new(file))
                .map_err(|e| format!("parsing params file {}: {e}", path.display()))
        }
        None => Ok(RuntimeParams::default()),
    }
}

fn choose_workflow(params: &mut RuntimeParams, route: Option<&str>, scan_film: bool) {
    if let Some(route) = route {
        params.workflow.route = route.to_owned();
    } else if scan_film {
        params.workflow.route = "input > film > scan".into();
    }
}

fn force_raw_input(params: &mut RuntimeParams) {
    params.io.input_color_space = "ACES2065-1".into();
    params.io.input_cctf_decoding = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn data_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }
    fn request(sources: &[String]) -> ProcessParamsRequest<'_> {
        ProcessParamsRequest {
            film: Some("kodak_portra_400"),
            paper: None,
            preset: None,
            params_file: None,
            sources,
            route: None,
            scan_film: false,
            input_is_raw: false,
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
    fn legacy_scan_topology_matches_runtime_without_resetting_explicit_scan() {
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
                if scan_film {
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
        req.preset = Some("classic-kodak-portra-400");
        let result = resolve(req, &data_dir()).unwrap();
        let look = presets::load_selector("classic-kodak-portra-400", &data_dir()).unwrap();
        assert_eq!(
            result.params.enlarger.m_filter_neutral,
            look.parameters.enlarger.m_filter_neutral
        );
        assert_eq!(
            result.params.enlarger.y_filter_neutral,
            look.parameters.enlarger.y_filter_neutral
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
    fn route_precedes_scan_flag_and_derives_topology() {
        let sources = vec!["camera.auto_exposure=false".into()];
        let mut req = request(&sources);
        req.preset = Some("classic-kodak-portra-400");
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
            "enlarger.c_filter_neutral=12,enlarger.m_filter_neutral=34,enlarger.y_filter_neutral=56".into(),
            "settings.rgb_to_raw_method=\"mallett2019\"".into(),
        ];
        let resolved = resolve(request(&sources), &data_dir()).unwrap();
        let mut runtime = crate::runtime::Runtime::new(
            resolved.film,
            resolved.print,
            resolved.params,
            &data_dir(),
        )
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
        assert!(saved.get("neutral_print_filters_protected").is_none());
        let restored: RuntimeParams = serde_json::from_value(saved).unwrap();
        assert_eq!(restored.neutral_print_filters_protected, [false; 3]);
    }

    #[test]
    fn preset_resolver_protects_actual_runtime_axes_for_all_consumers() {
        let mut look = presets::load_selector("classic-kodak-portra-400", &data_dir()).unwrap();
        look.parameters.enlarger.c_filter_neutral = 12.0;
        look.parameters.enlarger.m_filter_neutral = 34.0;
        look.parameters.enlarger.y_filter_neutral = 56.0;
        let mut context = RuntimeParams::default();
        context.settings.rgb_to_raw_method = "mallett2019".into();
        let resolved = look.resolve(&data_dir(), &context).unwrap();
        let runtime = crate::runtime::Runtime::new(
            resolved.film,
            resolved.print,
            resolved.params,
            &data_dir(),
        )
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
        updated.neutral_print_filters_protected = [false; 3];
        let unprotected = pipeline.clone().with_params(updated).unwrap();
        assert_eq!(unprotected.print_illuminant_slice(), database_only);
        let mut updated = unprotected.params.clone();
        updated.neutral_print_filters_protected = [true; 3];
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
}
