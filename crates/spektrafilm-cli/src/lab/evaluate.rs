//! `spectral evaluate`: product-path roundtrip error on measured spectra.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::ArgMatches;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_core::params::RuntimeParams;
use spektrafilm_core::params::sources::{ParameterEdits, validate_finite};
use spektrafilm_core::presets::PresetLibrary;
use spektrafilm_core::process_params::{self, ProcessParamsRequest};
use spektrafilm_core::profile::{self, Profile};
use spektrafilm_core::runtime::DigestMode;
use spektrafilm_core::{spectral_eval, spectral_service};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::spectral as math_spectral;

use super::cli::EvaluateArgs;
use super::metrics::{self, SampleMetrics};
use super::report::{csv_field, number_field};
use super::spectra::SpectraSet;
use crate::select_data_dir;

/// The scene-linear RGB space the measured spectra are photographed into.
const SCENE_RGB_SPACE: &str = "ACES2065-1";

struct Condition {
    reference: String,
    target: String,
}

struct ResolvedChain {
    film: Profile,
    params: RuntimeParams,
    profile_path: PathBuf,
    profile_name: String,
}

pub fn run(args: &EvaluateArgs, matches: &ArgMatches) -> Result<()> {
    run_with_result(args, matches).map(|_| ())
}

/// Evaluate the roundtrip error and return the command result object.
pub fn run_with_result(args: &EvaluateArgs, matches: &ArgMatches) -> Result<(Value, i32)> {
    let data_dir = select_data_dir(args.data_dir.clone(), matches)?.path;
    let set = super::spectra::load(&args.spectra).map_err(anyhow::Error::msg)?;
    // The illuminant pair is a property of the measured set, not of the
    // invocation: the manifest is its single authority.
    let condition = Condition {
        reference: set.condition_reference.clone().unwrap_or_default(),
        target: set.condition_target.clone().unwrap_or_default(),
    };
    let ResolvedChain {
        film: film_profile,
        params,
        profile_path,
        profile_name,
    } = resolve_chain(args, &data_dir)?;

    if !film_profile.is_film() || !film_profile.is_filming() {
        bail!("profile {profile_name:?} is not a film/filming profile");
    }
    let method = params.settings.rgb_to_raw_method.as_str();
    if method != "hanatos2025" {
        bail!(
            "spectral evaluate reconstructs the hanatos2025 upsampling; \
             the resolved chain selects {method:?}"
        );
    }
    if film_profile.info.reference_illuminant != condition.reference {
        bail!(
            "condition reference {:?} does not match the profile reference illuminant {:?}; \
             the spectral LUT is built for the profile's own reference",
            condition.reference,
            film_profile.info.reference_illuminant
        );
    }
    for name in [&condition.reference, &condition.target] {
        if !spectral_service::is_supported_illuminant(name) {
            bail!(
                "unsupported illuminant {name:?}; supported names include {}",
                spectral_service::available_illuminants().join(", ")
            );
        }
    }
    if set.wavelengths != film_profile.data.wavelengths {
        bail!(
            "spectra grid does not match the profile wavelength grid ({} samples vs {})",
            set.wavelengths.len(),
            film_profile.data.wavelengths.len()
        );
    }

    let reference_f64 = spectral_service::select_illuminant_f64(&condition.reference);
    let target_f64 = spectral_service::select_illuminant_f64(&condition.target);
    let film = profile::resolve_for_render(film_profile, params.film_render.development_time);
    let front = spectral_service::build_filming_upsampler(&film, &params, &data_dir)
        .map_err(anyhow::Error::msg)?;
    let tc_lut = front
        .tc_lut
        .as_ref()
        .ok_or_else(|| anyhow!("hanatos2025 resolved no TC LUT"))?;

    // Conditional truth anchor: `gray · (E_ref @ S)`.
    let true_anchor = spectral_eval::midgray_anchor_f64(
        &reference_f64,
        &front.sensitivity,
        spectral_eval::MIDGRAY_REFLECTANCE,
    );

    // Photograph every sample plus the flat mid-gray anchor through the
    // scene conversion, then through the product film-raw front pass.
    let sample_count = set.sample_count();
    let mut pixels: Vec<f64> = Vec::with_capacity((sample_count + 1) * 3);
    for index in 0..sample_count {
        let rgb =
            spectral_eval::reflectance_to_rgb_f64(set.row(index), &target_f64, SCENE_RGB_SPACE)
                .map_err(anyhow::Error::msg)?;
        pixels.extend_from_slice(&rgb);
    }
    let flat = vec![spectral_eval::MIDGRAY_REFLECTANCE; set.wavelengths.len()];
    let anchor_rgb = spectral_eval::reflectance_to_rgb_f64(&flat, &target_f64, SCENE_RGB_SPACE)
        .map_err(anyhow::Error::msg)?;
    pixels.extend_from_slice(&anchor_rgb);
    let image = ImageBuf::from_data((sample_count + 1) as u32, 1, pixels);
    let front_illuminant = spectral_service::select_illuminant(&front.front_illuminant);
    let response = math_spectral::hanatos2025_rgb_to_raw(
        &image,
        tc_lut,
        SCENE_RGB_SPACE,
        &front_illuminant,
        params.settings.use_cat16,
    );
    let response_at = |index: usize| -> [f64; 3] {
        [
            response.data[index * 3],
            response.data[index * 3 + 1],
            response.data[index * 3 + 2],
        ]
    };
    let response_anchor = response_at(sample_count);

    let mut sample_metrics: Vec<SampleMetrics> = Vec::with_capacity(sample_count);
    let mut raw_responses: Vec<[f64; 3]> = Vec::with_capacity(sample_count);
    for index in 0..sample_count {
        let raw = response_at(index);
        raw_responses.push(raw);
        if !set.band_valid(index) {
            sample_metrics.push(SampleMetrics::invalid());
            continue;
        }
        let truth_raw = spectral_eval::sensitivity_integral_f64(
            set.row(index),
            &reference_f64,
            &front.sensitivity,
        );
        let truth_relative = [
            truth_raw[0] / true_anchor[0],
            truth_raw[1] / true_anchor[1],
            truth_raw[2] / true_anchor[2],
        ];
        let response_relative = [
            raw[0] / response_anchor[0],
            raw[1] / response_anchor[1],
            raw[2] / response_anchor[2],
        ];
        sample_metrics.push(SampleMetrics::from_relative(
            truth_relative,
            response_relative,
        ));
    }

    // Groups: the whole set, then every source and partition the set declares.
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    groups.insert("all".into(), (0..sample_count).collect());
    for (index, label) in set.labels.iter().enumerate() {
        groups
            .entry(format!("source:{}", label.source))
            .or_default()
            .push(index);
        groups
            .entry(format!("partition:{}", label.partition))
            .or_default()
            .push(index);
    }
    let summaries: BTreeMap<&String, metrics::GroupSummary> = groups
        .iter()
        .map(|(name, members)| {
            let rows: Vec<SampleMetrics> =
                members.iter().map(|&index| sample_metrics[index]).collect();
            (name, metrics::summarize(&rows))
        })
        .collect();
    let valid_count = sample_metrics.iter().filter(|m| m.valid).count();

    let resolved_params_sha256 = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&params).map_err(anyhow::Error::msg)?)
    );
    let document = json!({
        "schema_version": 1,
        "tool": {"name": "spektrafilm-lab", "version": env!("CARGO_PKG_VERSION")},
        "inputs": {
            "spectra_manifest": {"path": set.manifest_path, "sha256": set.manifest_sha256},
            "arrays": [{"path": set.array_path, "key": set.array_key, "sha256": set.array_sha256}],
            "profile": profile_record(&profile_path, &profile_name)?,
            "preset": args.preset,
            "set": args.sources,
            "resolved_params_sha256": resolved_params_sha256,
        },
        "condition": {"reference": condition.reference, "target": condition.target},
        "chain": {
            "film": {
                "stock": film.info.stock,
                "name": film.info.name,
                "reference_illuminant": film.info.reference_illuminant,
            },
            "rgb_to_raw_method": front.method,
            "front_illuminant": front.front_illuminant,
            "scene_rgb_space": SCENE_RGB_SPACE,
            "use_cat16": params.settings.use_cat16,
            "spectral_gaussian_blur": params.settings.spectral_gaussian_blur,
            "window": {
                "applied": params.settings.apply_hanatos2025_adaptation_window,
                "params": film.data.hanatos2025_adaptation_window_params,
            },
            "surface": {
                "applied": params.settings.apply_hanatos2025_adaptation_surface,
                "params": film.data.hanatos2025_adaptation_surface_params,
            },
            "input_gamut_compress": {
                "active": params.io.input_gamut_compress.active,
                "algorithm": params.io.input_gamut_compress.algorithm,
                "boundary": params.io.input_gamut_compress.boundary,
                "knee": params.io.input_gamut_compress.knee,
            },
        },
        "normalization": {
            "midgray_reflectance": spectral_eval::MIDGRAY_REFLECTANCE,
            "true_anchor_raw_rgb": true_anchor,
            "response_anchor_raw_rgb": response_anchor,
        },
        "counts": {
            "total": sample_count,
            "valid": valid_count,
            "invalid": sample_count - valid_count,
        },
        "groups": summaries,
    });

    let samples_csv = build_samples_csv(&set, &sample_metrics, &raw_responses);
    let report_text = format!("{}\n", serde_json::to_string_pretty(&document)?);
    super::report::write(&args.out, &report_text, &samples_csv).map_err(anyhow::Error::msg)?;
    eprintln!("Report: {}", args.out.join("report.json").display());
    eprintln!("Samples: {}", args.out.join("samples.csv").display());
    Ok((
        json!({
            "schema_version": 1,
            "command": "spectral evaluate",
            "execution_status": "completed",
            "artifact": args.out.display().to_string(),
        }),
        0,
    ))
}

fn resolve_chain(args: &EvaluateArgs, data_dir: &Path) -> Result<ResolvedChain> {
    if let Some(path) = &args.profile {
        let film = profile::load_profile(path)
            .map_err(|e| anyhow!("loading profile {}: {e}", path.display()))?;
        let mut params = RuntimeParams::default();
        for source in &args.sources {
            let edits =
                ParameterEdits::parse(source).map_err(|e| anyhow!("--set {source:?}: {e}"))?;
            edits
                .apply(&mut params)
                .map_err(|e| anyhow!("--set {source:?}: {e}"))?;
        }
        validate_finite(&params).map_err(anyhow::Error::msg)?;
        params.validate().map_err(anyhow::Error::msg)?;
        let profile_name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("profile")
            .to_owned();
        return Ok(ResolvedChain {
            film,
            params,
            profile_path: path.clone(),
            profile_name,
        });
    }
    let library = PresetLibrary::default();
    let presets = args
        .preset
        .iter()
        .map(|selector| crate::preset::select(&library, selector).map(|selected| selected.preset))
        .collect::<Result<Vec<_>>>()?;
    let resolved = process_params::resolve(
        ProcessParamsRequest {
            film: None,
            paper: None,
            presets: &presets,
            params_file: None,
            sources: &args.sources,
            route: None,
            scan_film: false,
            scan_output: None,
            input_is_raw: false,
            input_color_space: None,
            input_cctf_decoding: None,
            require_input_declaration: false,
            digest_mode: DigestMode::ApplyStockSpecifics,
        },
        data_dir,
    )
    .map_err(anyhow::Error::msg)
    .context("resolving the spectral evaluation chain")?;
    let profile_path = data_dir
        .join("profiles")
        .join(format!("{}.json", resolved.film_name));
    Ok(ResolvedChain {
        film: resolved.film,
        params: resolved.params,
        profile_path,
        profile_name: resolved.film_name,
    })
}

fn profile_record(path: &Path, name: &str) -> Result<Value> {
    if path.is_file() {
        let bytes =
            std::fs::read(path).with_context(|| format!("hashing profile {}", path.display()))?;
        Ok(json!({
            "path": path.display().to_string(),
            "sha256": format!("{:x}", Sha256::digest(&bytes)),
        }))
    } else {
        Ok(json!({"name": name}))
    }
}

fn build_samples_csv(
    set: &SpectraSet,
    metrics: &[SampleMetrics],
    responses: &[[f64; 3]],
) -> String {
    let mut out = String::from(
        "sample_id,source,partition,row,valid,rho,max_abs_ev,common_ev,balance_rms_ev,\
         delta_ev_R,delta_ev_G,delta_ev_B,rb_ev,gb_ev,\
         log10_response_R,log10_response_G,log10_response_B\n",
    );
    for (index, label) in set.labels.iter().enumerate() {
        let m = &metrics[index];
        let mut fields: Vec<String> = Vec::with_capacity(17);
        fields.push(csv_field(&label.id));
        fields.push(csv_field(&label.source));
        fields.push(csv_field(&label.partition));
        fields.push(label.row.unwrap_or(index).to_string());
        fields.push(if m.valid { "true" } else { "false" }.to_owned());
        for value in [
            m.rho,
            m.max_abs_ev,
            m.common_ev,
            m.balance_rms_ev,
            m.delta_ev_rgb[0],
            m.delta_ev_rgb[1],
            m.delta_ev_rgb[2],
            m.rb_ev,
            m.gb_ev,
        ] {
            fields.push(number_field(value));
        }
        for raw in responses[index] {
            fields.push(number_field(if raw > 0.0 { raw.log10() } else { f64::NAN }));
        }
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}
