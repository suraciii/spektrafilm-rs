//! Multiple-exposure CLI entry point.
//!
//! This is the CLI surface for the core `process_exposures` API. It stays
//! separate from the single-image `process` path so the existing route,
//! telemetry and stage-artifact behavior is untouched.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spektrafilm_backend::ComputeBackend;
use spektrafilm_core::gamut_compression::OutputGamutCompress;
use spektrafilm_core::image_io::{self, BitDepth, Compression, JpegSubsampling, SaveOptions};
use spektrafilm_core::lut_look::{
    EffectsParams, GradeParams, LutDomainPolicy, LutLook, LutLookParams, LutParams, LutSpace,
    WorkflowMode,
};
use spektrafilm_core::params::sources::ParameterEdits;
use spektrafilm_core::params::{
    IoParams, OutputGamutCompressParams, PrintExposureEvTracking, PrintExposureMode, RuntimeParams,
    Tap,
};
use spektrafilm_core::runtime::{DigestMode, RuntimePhotoParams};
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

use crate::DataSelection;
use crate::DiscoveryFormat;
use crate::process::{CompressionArg, JpegSubsamplingArg, MeteringArg, OutputFormat, ProcessArgs};

const RECIPE_SCHEMA: &str = "spektrafilm.multiple-exposure/1";
const RECIPE_SCHEMA_V2: &str = "spektrafilm.multiple-exposure/2";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CropSpec {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeShot {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    exposure_ev: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transform: Option<TransformRecipe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformRecipe {
    #[serde(default)]
    translate_x: f64,
    #[serde(default)]
    translate_y: f64,
    #[serde(default = "default_scale")]
    scale: f64,
    #[serde(default)]
    rotation_deg: f64,
    #[serde(default)]
    fit: Option<String>,
}

fn default_scale() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrintRecipe {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exposure_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exposure: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ev_tracking: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    schema: String,
    film: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paper: Option<String>,
    seed: u32,
    metering: MeteringArg,
    shots: Vec<RecipeShot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_crop: Option<CropSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    print: Option<PrintRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_color_space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_cctf_decoding: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilmRecipeGroup {
    film: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paper: Option<String>,
    #[serde(default = "default_metering")]
    metering: MeteringArg,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    print: Option<PrintRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    route: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    scan_film: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scan_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scan_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw_white_balance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw_temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raw_tint: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    lens_correction: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    film_channel_swap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    print_channel_swap: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RgbRecipeGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lut: Option<LutParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effects: Option<EffectsParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    grade: Option<GradeParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeV2 {
    schema: String,
    method: String,
    seed: u32,
    shots: Vec<RecipeShot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_crop: Option<CropSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_color_space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_cctf_decoding: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    film: Option<FilmRecipeGroup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rgb: Option<RgbRecipeGroup>,
}

fn default_metering() -> MeteringArg {
    MeteringArg::Manual
}

#[derive(Debug, Clone)]
struct RgbConfig {
    lut: LutParams,
    effects: EffectsParams,
    grade: GradeParams,
}

#[derive(Debug, Clone, Default)]
struct FilmWorkflow {
    route: Option<String>,
    scan_film: bool,
    scan_output: Option<String>,
    scan_profile: Option<String>,
    raw_white_balance: Option<String>,
    raw_temperature: Option<f64>,
    raw_tint: Option<f64>,
    lens_correction: bool,
    film_channel_swap: Option<String>,
    print_channel_swap: Option<String>,
}

impl FilmWorkflow {
    fn from_args(args: &ProcessArgs) -> Self {
        Self {
            route: args.workflow.route.clone(),
            scan_film: args.scan_film,
            scan_output: args.workflow.scan_output.clone(),
            scan_profile: args.workflow.scan_profile.clone(),
            raw_white_balance: Some(args.workflow.raw_white_balance.clone()),
            raw_temperature: args.workflow.raw_temperature,
            raw_tint: args.workflow.raw_tint,
            lens_correction: args.workflow.lens_correction,
            film_channel_swap: Some(args.workflow.film_channel_swap.clone()),
            print_channel_swap: Some(args.workflow.print_channel_swap.clone()),
        }
    }

    fn raw_options(&self) -> spektrafilm_raw::RawOptions {
        spektrafilm_raw::RawOptions {
            white_balance: match self.raw_white_balance.as_deref().unwrap_or("as-shot") {
                "daylight" => spektrafilm_raw::WhiteBalance::Daylight,
                "tungsten" => spektrafilm_raw::WhiteBalance::Tungsten,
                "custom" => spektrafilm_raw::WhiteBalance::Custom,
                _ => spektrafilm_raw::WhiteBalance::AsShot,
            },
            temperature: self.raw_temperature,
            tint: self.raw_tint,
            lens_correction: self.lens_correction,
        }
    }
}

struct Resolved {
    film: String,
    paper: Option<String>,
    seed: u32,
    metering: MeteringArg,
    shots: Vec<(PathBuf, Option<String>, f64)>,
    output_crop: Option<CropSpec>,
    print: Option<PrintRecipe>,
    input_color_space: Option<String>,
    input_cctf_decoding: Option<bool>,
    film_workflow: FilmWorkflow,
    rgb: Option<RgbConfig>,
    recipe_parent: Option<PathBuf>,
}

fn parse_memory_limit(value: &str) -> Result<Option<u64>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto") {
        return Ok(None);
    }
    let lower = value.to_ascii_lowercase();
    let (number, multiplier) = if let Some(v) = lower.strip_suffix("gib") {
        (v, 1024_u64.pow(3))
    } else if let Some(v) = lower.strip_suffix("mib") {
        (v, 1024_u64.pow(2))
    } else if let Some(v) = lower.strip_suffix("kib") {
        (v, 1024_u64)
    } else {
        (lower.as_str(), 1)
    };
    let bytes = number
        .trim()
        .parse::<u64>()
        .with_context(|| format!("invalid --max-memory value {value:?}"))?
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("--max-memory value overflows u64"))?;
    Ok(Some(bytes))
}

fn estimate_peak_bytes(width: u32, height: u32) -> u64 {
    // CPU host estimate, calibrated against the measured f32 streaming runs:
    // 61 MP / 4 shots 6.45 GiB and 100 MP / 4-8 shots 10.46 GiB. The estimate
    // must cover the accumulator plus the pipeline's working f32 images, not
    // just one three-channel buffer; 37 bytes/sample is conservative for the
    // measured runs, and 80 bytes/sample covers the f64 reference lane. It
    // does not scale with shot count. GPU host/VRAM is intentionally unguarded.
    let samples = (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(3);
    let factor = if spektrafilm_backend::Precision::HOST.is_f64() {
        80.0
    } else {
        37.0
    };
    (samples as f64 * factor) as u64 + 512 * 1024_u64.pow(2)
}

fn default_memory_budget(width: u32, height: u32) -> u64 {
    const GIB: u64 = 1024_u64.pow(3);
    let estimate = estimate_peak_bytes(width, height);
    let with_headroom = estimate.saturating_add(GIB);
    if spektrafilm_backend::Precision::HOST.is_f64() {
        // The f64 build is the reference/debug lane, not the product f32
        // target; keep the old generous floor, but never undercut the
        // calibrated estimate.
        with_headroom.max(24 * GIB)
    } else if estimate <= 7 * GIB {
        7 * GIB
    } else if estimate <= 11 * GIB {
        11 * GIB
    } else {
        // The 90 MP step must not reject 102 MP or larger rasters merely
        // because the estimate crossed an old fixed branch. Keep a GiB of
        // host headroom above the estimate.
        with_headroom
    }
}

pub(crate) fn run(args: &ProcessArgs, data: &DataSelection) -> Result<()> {
    if !args.expose.is_empty() && args.recipe.is_some() {
        bail!("--expose and --recipe are mutually exclusive");
    }
    if args.input.is_some() {
        bail!("--expose/--recipe cannot be combined with a positional input");
    }

    let mut resolved = match &args.recipe {
        Some(path) => read_recipe(path)?,
        None => resolve_flags(args)?,
    };
    if resolved.rgb.is_some() {
        resolved.rgb = Some(resolve_rgb_config(args, resolved.rgb.as_ref())?);
        validate_stack_args(args, true)?;
        return run_rgb(args, data, &resolved);
    }
    validate_stack_args(args, false)?;

    if args.preview && args.preview_max_size == 0 {
        bail!("multiple-exposure.preview: --preview-max-size must be positive");
    }
    let memory_limit = parse_memory_limit(&args.max_memory)?;
    let film_name = resolved.film.clone();
    let input_is_raw = image_io::is_raw(&resolved.shots[0].0);
    if resolved
        .shots
        .iter()
        .any(|(path, _, _)| image_io::is_raw(path) != input_is_raw)
    {
        bail!("multiple-exposure.input-kind: all shots must share one input kind");
    }
    let raw_options = resolved.film_workflow.raw_options();
    if input_is_raw {
        raw_options.validate().map_err(anyhow::Error::from)?;
    }
    let resolved_params = spektrafilm_core::process_params::resolve(
        spektrafilm_core::process_params::ProcessParamsRequest {
            film: Some(spektrafilm_core::process_params::FilmSelection::Name(
                &film_name,
            )),
            paper: resolved.paper.as_deref(),
            presets: &[],
            params_file: None,
            sources: &[],
            feature_profiles: &[],
            route: resolved.film_workflow.route.as_deref(),
            scan_film: resolved.film_workflow.scan_film,
            scan_output: resolved.film_workflow.scan_output.as_deref(),
            input_is_raw,
            input_color_space: resolved.input_color_space.as_deref(),
            input_cctf_decoding: resolved.input_cctf_decoding,
            require_input_declaration: false,
            digest_mode: DigestMode::ApplyStockSpecifics,
        },
        &data.path,
    )
    .map_err(anyhow::Error::msg)?;
    let print_name = resolved_params.print_name;
    let mut film = resolved_params.film;
    let mut print = resolved_params.print;
    crate::process::apply_channel_swap(
        &mut film,
        resolved
            .film_workflow
            .film_channel_swap
            .as_deref()
            .unwrap_or("none"),
    )?;
    crate::process::apply_channel_swap(
        &mut print,
        resolved
            .film_workflow
            .print_channel_swap
            .as_deref()
            .unwrap_or("none"),
    )?;
    let mut params = resolved_params.params;
    crate::process::apply_scan_profile(
        &mut params,
        resolved.film_workflow.scan_profile.as_deref(),
    )?;
    params.camera.exposure_compensation_ev = 0.0;
    params.camera.auto_exposure = resolved.metering != MeteringArg::Manual;
    params.random_seed = resolved.seed as u64;
    if let Some(print_recipe) = &resolved.print {
        if let Some(mode) = &print_recipe.exposure_mode {
            params.enlarger.print_exposure_mode = match mode.as_str() {
                "auto" => PrintExposureMode::Auto,
                "manual" => PrintExposureMode::Manual,
                other => {
                    bail!("multiple-exposure.recipe-schema: unknown print exposure_mode {other:?}")
                }
            };
        }
        if let Some(exposure) = print_recipe.exposure {
            params.enlarger.print_exposure = exposure;
        }
        if let Some(tracking) = &print_recipe.ev_tracking {
            params.settings.print_exposure_ev_tracking = match tracking.as_str() {
                "never" => PrintExposureEvTracking::Never,
                "always" => PrintExposureEvTracking::Always,
                "follow_mode" => PrintExposureEvTracking::FollowMode,
                other => {
                    bail!("multiple-exposure.recipe-schema: unknown print ev_tracking {other:?}")
                }
            };
        }
    }

    if params.io.crop {
        bail!("multiple-exposure.input-crop: input cropping is not allowed; use --output-crop");
    }

    let route = params.workflow.route.clone();
    let scan_film = params.io.scan_film;
    let photo = RuntimePhotoParams {
        film,
        print,
        params,
        data_dir: data.path.clone(),
    };

    let metering = resolved.metering.core();
    if args.dry_run {
        // Validate the stack without initializing a compute device.
        let runtime = photo
            .into_runtime(DigestMode::ApplyStockSpecifics)
            .map_err(anyhow::Error::msg)?;
        let cpu = spektrafilm_backend::cpu_backend::CpuBackend;
        let _validated = runtime
            .begin_exposures(&cpu, metering, Tap::RgbOut)
            .map_err(anyhow::Error::msg)?;
        let backend_name = match args.backend {
            Some(crate::Backend::Gpu) => "gpu",
            Some(crate::Backend::Cpu) | None => "cpu",
        };
        let report = serde_json::json!({
            "schema": "spektrafilm.multiple-exposure-dry-run/1",
            "method": "film",
            "film": &film_name,
            "paper": &print_name,
            "route": &route,
            "scan_film": scan_film,
            "scan_output": &resolved.film_workflow.scan_output,
            "scan_profile": &resolved.film_workflow.scan_profile,
            "metering": resolved.metering,
            "raw": {
                "white_balance": &resolved.film_workflow.raw_white_balance,
                "temperature": resolved.film_workflow.raw_temperature,
                "tint": resolved.film_workflow.raw_tint,
                "lens_correction": resolved.film_workflow.lens_correction
            },
            "channel_swap": {
                "film": &resolved.film_workflow.film_channel_swap,
                "print": &resolved.film_workflow.print_channel_swap
            },
            "shots": resolved.shots.iter().enumerate().map(|(index, (path, _, ev))| {
                serde_json::json!({
                    "index": index,
                    "path": path.display().to_string(),
                    "exposure_ev": ev
                })
            }).collect::<Vec<_>>(),
            "backend": backend_name,
            "output": args.output.as_ref().map(|path| path.display().to_string()),
            "unchecked": ["pixel_decode", "compute_device", "output_geometry"]
        });
        match args.dry_run_format.unwrap_or(DiscoveryFormat::Json) {
            DiscoveryFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
            DiscoveryFormat::Text => {
                println!("method: film");
                println!("film: {film_name}");
                println!("paper: {print_name}");
                println!("route: {route}");
                println!("scan_film: {scan_film}");
                println!("shots: {}", resolved.shots.len());
                println!("backend: {backend_name}");
            }
        }
        return Ok(());
    }
    // Multiple exposure defaults to the CPU reference. A GPU must be
    // requested explicitly until the retained tail has a parity gate.
    let backend: Box<dyn ComputeBackend> = match args.backend {
        Some(crate::Backend::Gpu) => select_backend(args)?,
        Some(crate::Backend::Cpu) => Box::new(spektrafilm_backend::cpu_backend::CpuBackend),
        None => Box::new(spektrafilm_backend::cpu_backend::CpuBackend),
    };
    let runtime = if backend.is_gpu() {
        photo
            .into_runtime_with_backend(DigestMode::ApplyStockSpecifics, backend.as_ref())
            .map_err(anyhow::Error::msg)?
    } else {
        photo
            .into_runtime(DigestMode::ApplyStockSpecifics)
            .map_err(anyhow::Error::msg)?
    };
    let mut stack = runtime
        .begin_exposures(backend.as_ref(), metering, Tap::RgbOut)
        .map_err(anyhow::Error::msg)?;
    let mut full_dims = None;
    for (index, (path, hash, ev)) in resolved.shots.iter().enumerate() {
        if let Some(expected) = hash {
            verify_sha256(path, expected)
                .with_context(|| format!("verifying shot {index}: {}", path.display()))?;
        }
        let full_image = if input_is_raw {
            spektrafilm_raw::load(path, &raw_options)
                .map_err(|error| {
                    anyhow::anyhow!("loading shot {index}: {}: {error}", path.display())
                })?
                .image
        } else {
            image_io::load(path)
                .with_context(|| format!("loading shot {index}: {}", path.display()))?
                .image
        };
        if full_dims.is_none() {
            full_dims = Some((full_image.width, full_image.height));
        }
        let image = if args.preview {
            spektrafilm_core::params_builder::resize_for_preview(&full_image, args.preview_max_size)
        } else {
            full_image
        };
        if index == 0 && !backend.is_gpu() {
            let estimate = estimate_peak_bytes(image.width, image.height);
            let budget =
                memory_limit.unwrap_or_else(|| default_memory_budget(image.width, image.height));
            if estimate > budget {
                bail!(
                    "multiple-exposure.capacity: estimated host peak {} bytes exceeds budget {} bytes at {}x{} (cpu); use --max-memory or --preview",
                    estimate,
                    budget,
                    image.width,
                    image.height
                );
            }
        }
        stack.add_shot(&image, *ev).map_err(anyhow::Error::msg)?;
        drop(image);
    }
    let (rendered, report) = stack.finish().map_err(anyhow::Error::msg)?;

    let rendered = match resolved.output_crop.as_ref() {
        Some(crop) => {
            let crop = if args.preview {
                let full = full_dims.expect("at least one shot loaded");
                scale_crop(crop, full, (rendered.width, rendered.height))?
            } else {
                crop.clone()
            };
            crop_image(&rendered, &crop)?
        }
        None => rendered,
    };

    if let Some(path) = &args.recipe_out {
        let recipe = resolved_to_recipe(&resolved, &resolved.shots, args)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&recipe)?)
            .with_context(|| format!("writing recipe {}", path.display()))?;
    }

    let output = args
        .output
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("multiple-exposure.output: --output is required"))?;
    save_output(output, &rendered, args)?;

    if let Some(path) = &args.diagnostics {
        let report_json = serde_json::json!({
            "schema": "spektrafilm.multiple-exposure-diagnostics/1",
            "method": "film",
            "film": &film_name,
            "paper": &print_name,
            "route": &route,
            "scan_film": scan_film,
            "scan_output": &resolved.film_workflow.scan_output,
            "scan_profile": &resolved.film_workflow.scan_profile,
            "backend": if backend.is_gpu() { "gpu" } else { "cpu" },
            "shots": report.shots.iter().map(|shot| {
                serde_json::json!({
                    "index": shot.index,
                    "metered_ev": shot.metered_ev,
                    "exposure_ev": shot.exposure_ev,
                    "final_ev": shot.final_ev
                })
            }).collect::<Vec<_>>(),
            "composition": {
                "width": rendered.width,
                "height": rendered.height,
                "total_ev": report.total_ev,
                "log_e_range": [report.log_e_range.0, report.log_e_range.1],
                "highlight_fraction": report.highlight_fraction,
                "highlight_log_e": report.highlight_log_e
            },
            "raw": {
                "white_balance": &resolved.film_workflow.raw_white_balance,
                "temperature": resolved.film_workflow.raw_temperature,
                "tint": resolved.film_workflow.raw_tint,
                "lens_correction": resolved.film_workflow.lens_correction
            },
            "channel_swap": {
                "film": &resolved.film_workflow.film_channel_swap,
                "print": &resolved.film_workflow.print_channel_swap
            },
            "gpu_timings": args.gpu_timings
        });
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&report_json)?)
            .with_context(|| format!("writing diagnostics {}", path.display()))?;
    }
    if args.timings {
        eprintln!(
            "multiple exposure film: {} shots, {}x{}, route={}, backend={}",
            resolved.shots.len(),
            rendered.width,
            rendered.height,
            route,
            if backend.is_gpu() { "gpu" } else { "cpu" }
        );
    }

    eprintln!(
        "multiple exposure: {} shots, total_ev={:.3}, logE=[{:.3}, {:.3}], highlight={:.4}% (shoulder {:.3})",
        report.shots.len(),
        report.total_ev,
        report.log_e_range.0,
        report.log_e_range.1,
        report.highlight_fraction * 100.0,
        report.highlight_log_e
    );
    for shot in &report.shots {
        eprintln!(
            "  shot {}: metered_ev={:.3} exposure_ev={:.3} final_ev={:.3}",
            shot.index, shot.metered_ev, shot.exposure_ev, shot.final_ev
        );
    }
    Ok(())
}

fn run_rgb(args: &ProcessArgs, _data: &DataSelection, resolved: &Resolved) -> Result<()> {
    let rgb = resolved
        .rgb
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("multiple-exposure.method: RGB group is missing"))?;
    let look_params = LutLookParams::from_groups(&rgb.lut, Some(&rgb.effects), Some(&rgb.grade));
    let look = if rgb.lut.path.as_os_str().is_empty() {
        LutLook::identity(look_params)
    } else {
        look_params
            .load()
            .map_err(|error| anyhow::anyhow!("validating LUT look: {error}"))?
    };
    let effects_look = look.effects_only();
    let final_look = look.look_only();

    let output_format = args
        .output
        .as_deref()
        .and_then(|path| image_io::ImageFormat::detect(path).ok())
        .or_else(|| args.format.map(OutputFormat::image_format));
    let output_space_name = args.workflow.saving_color_space.clone().unwrap_or_else(|| {
        if output_format == Some(image_io::ImageFormat::Exr) {
            "ACES2065-1".into()
        } else {
            "sRGB".into()
        }
    });
    let output_space = colorspace::resolve(&output_space_name).map_err(|error| {
        anyhow::anyhow!("unsupported output color space {output_space_name:?}: {error}")
    })?;
    let output_encoded = args
        .workflow
        .saving_cctf_encoding
        .unwrap_or(output_format != Some(image_io::ImageFormat::Exr));
    let gamut_off = OutputGamutCompress::build(
        &OutputGamutCompressParams {
            algorithm: "off".into(),
            ..OutputGamutCompressParams::default()
        },
        "sRGB",
    )
    .map_err(anyhow::Error::msg)?;
    let output_gamut =
        OutputGamutCompress::build(&OutputGamutCompressParams::default(), &output_space_name)
            .map_err(anyhow::Error::msg)?;

    let io = IoParams {
        input_color_space: resolved
            .input_color_space
            .clone()
            .unwrap_or_else(|| "sRGB".into()),
        input_cctf_decoding: resolved.input_cctf_decoding.unwrap_or(true),
        ..IoParams::default()
    };
    if args.dry_run {
        let backend_name = match args.backend {
            Some(crate::Backend::Gpu) => "gpu",
            Some(crate::Backend::Cpu) | None => "cpu",
        };
        let report = serde_json::json!({
            "schema": "spektrafilm.multiple-exposure-dry-run/1",
            "method": "rgb",
            "shots": resolved.shots.iter().enumerate().map(|(index, (path, _, ev))| {
                serde_json::json!({
                    "index": index,
                    "path": path.display().to_string(),
                    "exposure_ev": ev
                })
            }).collect::<Vec<_>>(),
            "modules": {
                "lut": !rgb.lut.path.as_os_str().is_empty(),
                "effects": {
                    "diffusion": rgb.effects.diffusion.active,
                    "halation": rgb.effects.halation.active,
                    "grain": rgb.effects.grain.active
                },
                "grade": rgb.grade.active
            },
            "backend": backend_name,
            "output": args.output.as_ref().map(|path| path.display().to_string()),
            "unchecked": ["pixel_decode", "compute_device", "output_geometry"]
        });
        match args.dry_run_format.unwrap_or(DiscoveryFormat::Json) {
            DiscoveryFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
            DiscoveryFormat::Text => {
                println!("method: rgb");
                println!("shots: {}", resolved.shots.len());
                println!("lut: {}", !rgb.lut.path.as_os_str().is_empty());
                println!("grade: {}", rgb.grade.active);
                println!(
                    "effects: diffusion={} halation={} grain={}",
                    rgb.effects.diffusion.active,
                    rgb.effects.halation.active,
                    rgb.effects.grain.active
                );
                println!("backend: {backend_name}");
            }
        }
        return Ok(());
    }
    let backend: Box<dyn ComputeBackend> = match args.backend {
        Some(crate::Backend::Gpu) => select_backend(args)?,
        Some(crate::Backend::Cpu) | None => Box::new(spektrafilm_backend::cpu_backend::CpuBackend),
    };

    if args.preview && args.preview_max_size == 0 {
        bail!("multiple-exposure.preview: --preview-max-size must be positive");
    }
    let memory_limit = parse_memory_limit(&args.max_memory)?;
    let mut full_dims = None;
    let mut geometry = None;
    let mut accumulator: Option<ImageBuf> = None;
    let mut pitch_um = 0.0;
    let mut shot_report = Vec::with_capacity(resolved.shots.len());

    for (index, (path, hash, ev)) in resolved.shots.iter().enumerate() {
        if let Some(expected) = hash {
            verify_sha256(path, expected)
                .with_context(|| format!("verifying shot {index}: {}", path.display()))?;
        }
        let loaded = image_io::load(path)
            .with_context(|| format!("loading shot {index}: {}", path.display()))?;
        let full_image = loaded.image;
        if full_dims.is_none() {
            full_dims = Some((full_image.width, full_image.height));
        }
        let image = if args.preview {
            spektrafilm_core::params_builder::resize_for_preview(&full_image, args.preview_max_size)
        } else {
            full_image
        };
        if index == 0 && !backend.is_gpu() {
            let estimate = estimate_peak_bytes(image.width, image.height);
            let budget =
                memory_limit.unwrap_or_else(|| default_memory_budget(image.width, image.height));
            if estimate > budget {
                bail!(
                    "multiple-exposure.capacity: estimated host peak {} bytes exceeds budget {} bytes at {}x{} (cpu); use --max-memory or --preview",
                    estimate,
                    budget,
                    image.width,
                    image.height
                );
            }
        }
        let (working, pitch) = spektrafilm_core::resizing::crop_and_rescale(&image, &io, 35.0)
            .map_err(anyhow::Error::msg)?;
        if let Some(expected) = geometry {
            if (working.width, working.height) != expected {
                bail!(
                    "multiple-exposure.geometry: shot {index} is {}x{}, expected {}x{}",
                    working.width,
                    working.height,
                    expected.0,
                    expected.1
                );
            }
        } else {
            geometry = Some((working.width, working.height));
        }
        if index == 0 {
            pitch_um = pitch;
        } else if (pitch_um - pitch).abs() > f64::EPSILON {
            bail!("multiple-exposure.geometry: shots do not share one working pitch");
        }
        let mut linear = to_linear_srgb(&working, &io)?;
        apply_ev(&mut linear, *ev);
        for value in &mut linear.data {
            if to_f64(*value) < 0.0 {
                *value = from_f64(0.0);
            }
        }
        let affected = effects_look
            .render(
                &linear,
                &LutSpace::SrgbLinear.color_space(),
                false,
                &LutSpace::SrgbLinear.color_space(),
                false,
                pitch,
                &gamut_off,
                backend.as_ref(),
            )
            .map_err(anyhow::Error::msg)?;
        match &mut accumulator {
            Some(sum) => {
                if sum.width != affected.width || sum.height != affected.height {
                    bail!("multiple-exposure.geometry: composed frames do not share geometry");
                }
                for (sum, value) in sum.data.iter_mut().zip(affected.data) {
                    *sum += value;
                }
            }
            None => accumulator = Some(affected),
        }
        shot_report.push(serde_json::json!({
            "index": index,
            "path": path.display().to_string(),
            "exposure_ev": ev,
            "pixel_size_um": pitch
        }));
        drop(image);
    }

    let composite = accumulator
        .ok_or_else(|| anyhow::anyhow!("multiple-exposure: stack produced no exposures"))?;
    let rendered = final_look
        .render(
            &composite,
            &LutSpace::SrgbLinear.color_space(),
            false,
            output_space,
            output_encoded,
            pitch_um,
            &output_gamut,
            backend.as_ref(),
        )
        .map_err(anyhow::Error::msg)?;

    let rendered = match resolved.output_crop.as_ref() {
        Some(crop) => {
            let crop = if args.preview {
                let full = full_dims.expect("at least one shot loaded");
                scale_crop(crop, full, (rendered.width, rendered.height))?
            } else {
                crop.clone()
            };
            crop_image(&rendered, &crop)?
        }
        None => rendered,
    };

    if let Some(path) = &args.recipe_out {
        let recipe = resolved_to_recipe(resolved, &resolved.shots, args)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&recipe)?)
            .with_context(|| format!("writing recipe {}", path.display()))?;
    }

    let output = args
        .output
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("multiple-exposure.output: --output is required"))?;
    save_output(output, &rendered, args)?;

    if let Some(path) = &args.diagnostics {
        let report = serde_json::json!({
            "schema": "spektrafilm.multiple-exposure-diagnostics/1",
            "method": "rgb",
            "backend": if backend.is_gpu() { "gpu" } else { "cpu" },
            "shots": shot_report,
            "composition": {
                "width": rendered.width,
                "height": rendered.height,
                "pixel_size_um": pitch_um
            },
            "modules": {
                "lut": !rgb.lut.path.as_os_str().is_empty(),
                "effects": {
                    "diffusion": rgb.effects.diffusion.active,
                    "halation": rgb.effects.halation.active,
                    "grain": rgb.effects.grain.active
                },
                "grade": rgb.grade.active
            },
            "gpu_timings": args.gpu_timings
        });
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&report)?)
            .with_context(|| format!("writing diagnostics {}", path.display()))?;
    }
    if args.timings {
        eprintln!(
            "multiple exposure rgb: {} shots, {}x{}, pitch={:.4} um, backend={}",
            resolved.shots.len(),
            rendered.width,
            rendered.height,
            pitch_um,
            if backend.is_gpu() { "gpu" } else { "cpu" }
        );
    }
    eprintln!(
        "multiple exposure rgb: {} shots, {}x{}, lut={}, grade={}, effects(diffusion={}, halation={}, grain={})",
        resolved.shots.len(),
        rendered.width,
        rendered.height,
        !rgb.lut.path.as_os_str().is_empty(),
        rgb.grade.active,
        rgb.effects.diffusion.active,
        rgb.effects.halation.active,
        rgb.effects.grain.active
    );
    Ok(())
}

fn to_linear_srgb(image: &ImageBuf, io: &IoParams) -> Result<ImageBuf> {
    let source = colorspace::resolve(&io.input_color_space).map_err(|error| {
        anyhow::anyhow!(
            "unsupported input color space {:?}: {error}",
            io.input_color_space
        )
    })?;
    let linear = LutSpace::SrgbLinear.color_space();
    let matrix = colorspace::conversion_matrix(source, &linear);
    let mut out = image.clone();
    for pixel in out.data.as_chunks_mut::<3>().0 {
        let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
        let converted =
            colorspace::convert_rgb(rgb, source, io.input_cctf_decoding, &linear, false, &matrix);
        for channel in 0..3 {
            pixel[channel] = from_f64(converted[channel]);
        }
    }
    Ok(out)
}

fn apply_ev(image: &mut ImageBuf, ev: f64) {
    let gain = 2.0_f64.powf(ev);
    for pixel in image.data.as_chunks_mut::<3>().0 {
        for channel in pixel {
            let value = to_f64(*channel) * gain;
            *channel = from_f64(if value.is_nan() {
                value
            } else {
                value.max(0.0)
            });
        }
    }
}

fn zero_lut() -> LutParams {
    LutParams {
        input_space: LutSpace::SrgbLinear,
        output_space: LutSpace::SrgbLinear,
        strength: 0.0,
        ..LutParams::default()
    }
}

fn lut_from_cube(path: &Path) -> Result<LutParams> {
    let look =
        LutLookParams::from_cube(path, LutSpace::Srgb, LutSpace::Srgb, LutDomainPolicy::Clamp)
            .map_err(anyhow::Error::msg)?;
    Ok(look.split().0)
}

fn resolve_rgb_config(args: &ProcessArgs, base: Option<&RgbConfig>) -> Result<RgbConfig> {
    let mut config = match base {
        Some(config) => config.clone(),
        None => RgbConfig {
            lut: match args.lut.as_deref() {
                Some(path) => lut_from_cube(path)?,
                None => zero_lut(),
            },
            effects: EffectsParams::default(),
            grade: GradeParams::default(),
        },
    };
    let mut params = RuntimeParams::default();
    params.workflow.mode = if config.lut.path.as_os_str().is_empty() {
        WorkflowMode::Off
    } else {
        WorkflowMode::Lut
    };
    params.lut = Some(config.lut.clone());
    params.effects = Some(config.effects.clone());
    params.grade = Some(config.grade.clone());
    for source in &args.sources {
        let trimmed = source.trim_start();
        if source.contains('=')
            && !(trimmed.starts_with("effects.") || trimmed.starts_with("grade."))
        {
            bail!(
                "multiple-exposure.unsupported: RGB --set accepts effects.* and grade.* only, got {source:?}"
            );
        }
        let edits = ParameterEdits::parse(source)
            .map_err(|error| anyhow::anyhow!("--set {source:?}: {error}"))?;
        edits
            .apply(&mut params)
            .map_err(|error| anyhow::anyhow!("--set {source:?}: {error}"))?;
    }
    config.lut = params.lut.take().unwrap_or_else(zero_lut);
    config.effects = params.effects.take().unwrap_or_default();
    config.grade = params.grade.take().unwrap_or_default();
    config.effects.validate().map_err(anyhow::Error::msg)?;
    if config.grade.active {
        config.grade.validate().map_err(anyhow::Error::msg)?;
    }
    let look = LutLookParams::from_groups(&config.lut, Some(&config.effects), Some(&config.grade));
    look.validate(!config.lut.path.as_os_str().is_empty())
        .map_err(anyhow::Error::msg)?;
    Ok(config)
}

fn validate_stack_args(args: &ProcessArgs, rgb: bool) -> Result<()> {
    if args.params.is_some() || !args.preset.is_empty() || !args.feature_profiles.is_empty() {
        bail!(
            "multiple-exposure.unsupported: presets, --params and --feature-profile are not supported; use explicit flags or a recipe"
        );
    }
    if args.iters != 1 {
        bail!("multiple-exposure.unsupported: --iters must be 1");
    }
    if args.raw_out.is_some() {
        bail!("multiple-exposure.unsupported: --raw-out is outside multiple exposure");
    }
    if args.recipe.is_some()
        && (args.film.is_some()
            || args.paper.is_some()
            || args.output_crop.is_some()
            || args.seed.is_some()
            || args.multiple_exposure_meter.is_some()
            || args.input_color_space.is_some()
            || args.input_cctf_decoding.is_some()
            || args.lut.is_some()
            || args.scan_film
            || args.workflow.route.is_some()
            || args.workflow.scan_output.is_some()
            || args.workflow.scan_profile.is_some()
            || args.workflow.raw_white_balance != "as-shot"
            || args.workflow.raw_temperature.is_some()
            || args.workflow.raw_tint.is_some()
            || args.workflow.lens_correction
            || args.workflow.film_channel_swap != "none"
            || args.workflow.print_channel_swap != "none"
            || !args.expose_ev.is_empty())
    {
        bail!(
            "multiple-exposure.recipe-schema: --recipe carries the method group, crop, seed, and input interpretation; do not combine those flags with --recipe"
        );
    }
    if rgb {
        if args.film.is_some() || args.paper.is_some() {
            bail!("multiple-exposure.method: --rgb conflicts with --film and --paper");
        }
        if args.multiple_exposure_meter.is_some() {
            bail!("multiple-exposure.method: metering is Film only");
        }
        if args.scan_film
            || args.workflow.route.is_some()
            || args.workflow.scan_output.is_some()
            || args.workflow.scan_profile.is_some()
            || args.workflow.raw_white_balance != "as-shot"
            || args.workflow.raw_temperature.is_some()
            || args.workflow.raw_tint.is_some()
            || args.workflow.lens_correction
            || args.workflow.film_channel_swap != "none"
            || args.workflow.print_channel_swap != "none"
        {
            bail!(
                "multiple-exposure.method: RGB stacks do not accept RAW, lens correction, channel swap, route, scan profile, scan output, or direct scan"
            );
        }
        return Ok(());
    }
    if args.rgb || args.lut.is_some() {
        bail!("multiple-exposure.method: --rgb or --lut selects an RGB stack, not Film");
    }
    if !args.sources.is_empty() {
        bail!("multiple-exposure.unsupported: --set is available to RGB stacks only");
    }
    Ok(())
}

fn resolve_flags(args: &ProcessArgs) -> Result<Resolved> {
    if args.expose.len() < 2 {
        bail!("multiple-exposure.too-few: provide at least 2 --expose images");
    }
    if args.expose.len() > 8 {
        bail!("multiple-exposure.too-many: at most 8 --expose images");
    }
    if !args.expose_ev.is_empty() && args.expose_ev.len() != args.expose.len() {
        bail!("multiple-exposure.ev-count: --expose-ev count must equal --expose count");
    }
    // Manual is the explicit physical default: auto-exposure normalizes each
    // shot and can change the relative exposure the user is trying to make.
    let metering = args.multiple_exposure_meter.unwrap_or(MeteringArg::Manual);
    let mut shots = Vec::with_capacity(args.expose.len());
    for (index, path) in args.expose.iter().enumerate() {
        let ev = args.expose_ev.get(index).copied().unwrap_or(0.0);
        validate_shot(index, ev)?;
        shots.push((path.clone(), None, ev));
    }
    let output_crop = args.output_crop.as_deref().map(parse_crop).transpose()?;
    if args.rgb || (args.lut.is_some() && args.film.is_none()) {
        let lut = match args.lut.as_deref() {
            Some(path) => lut_from_cube(path)?,
            None => zero_lut(),
        };
        return Ok(Resolved {
            film: String::new(),
            paper: None,
            seed: args.seed.unwrap_or(0),
            metering: MeteringArg::Manual,
            shots,
            output_crop,
            print: None,
            input_color_space: args.input_color_space.clone(),
            input_cctf_decoding: args.input_cctf_decoding,
            film_workflow: FilmWorkflow::default(),
            rgb: Some(RgbConfig {
                lut,
                effects: EffectsParams::default(),
                grade: GradeParams::default(),
            }),
            recipe_parent: None,
        });
    }
    Ok(Resolved {
        film: args
            .film
            .clone()
            .ok_or_else(|| anyhow::anyhow!("multiple-exposure.stock: --film is required"))?,
        paper: args.paper.clone(),
        seed: args.seed.unwrap_or(0),
        metering,
        shots,
        output_crop,
        print: None,
        input_color_space: args.input_color_space.clone(),
        input_cctf_decoding: args.input_cctf_decoding,
        film_workflow: FilmWorkflow::from_args(args),
        rgb: None,
        recipe_parent: None,
    })
}

fn read_recipe(path: &Path) -> Result<Resolved> {
    let bytes = fs::read(path).with_context(|| format!("reading recipe {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(recipe_error)?;
    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let parent = path.parent().map(Path::to_path_buf);
    if schema == RECIPE_SCHEMA {
        let recipe: Recipe = serde_json::from_value(value).map_err(recipe_error)?;
        if recipe.shots.len() < 2 || recipe.shots.len() > 8 {
            bail!("multiple-exposure.recipe-schema: recipe must contain 2–8 shots");
        }
        let shots = read_shots(&parent, &recipe.shots)?;
        return Ok(Resolved {
            film: recipe.film,
            paper: recipe.paper,
            seed: recipe.seed,
            metering: recipe.metering,
            shots,
            output_crop: recipe.output_crop,
            print: recipe.print,
            input_color_space: recipe.input_color_space,
            input_cctf_decoding: recipe.input_cctf_decoding,
            film_workflow: FilmWorkflow::default(),
            rgb: None,
            recipe_parent: parent,
        });
    }
    if schema != RECIPE_SCHEMA_V2 {
        bail!("multiple-exposure.recipe-schema: expected {RECIPE_SCHEMA_V2:?}, got {schema:?}");
    }
    let recipe: RecipeV2 = serde_json::from_value(value).map_err(recipe_error)?;
    if recipe.shots.len() < 2 || recipe.shots.len() > 8 {
        bail!("multiple-exposure.recipe-schema: recipe must contain 2–8 shots");
    }
    let shots = read_shots(&parent, &recipe.shots)?;
    match recipe.method.as_str() {
        "film" => {
            if recipe.rgb.is_some() {
                bail!(
                    "multiple-exposure.recipe-schema: a Film recipe must not carry the RGB group"
                );
            }
            let group = recipe.film.ok_or_else(|| {
                anyhow::anyhow!("multiple-exposure.recipe-schema: Film recipe has no film group")
            })?;
            let film_workflow = FilmWorkflow {
                route: group.route,
                scan_film: group.scan_film,
                scan_output: group.scan_output,
                scan_profile: group.scan_profile,
                raw_white_balance: group.raw_white_balance,
                raw_temperature: group.raw_temperature,
                raw_tint: group.raw_tint,
                lens_correction: group.lens_correction,
                film_channel_swap: group.film_channel_swap,
                print_channel_swap: group.print_channel_swap,
            };
            Ok(Resolved {
                film: group.film,
                paper: group.paper,
                seed: recipe.seed,
                metering: group.metering,
                shots,
                output_crop: recipe.output_crop,
                print: group.print,
                input_color_space: recipe.input_color_space,
                input_cctf_decoding: recipe.input_cctf_decoding,
                film_workflow,
                rgb: None,
                recipe_parent: parent,
            })
        }
        "rgb" => {
            if recipe.film.is_some() {
                bail!(
                    "multiple-exposure.recipe-schema: an RGB recipe must not carry the Film group"
                );
            }
            let group = recipe.rgb.unwrap_or_default();
            let mut lut = group.lut.unwrap_or_else(zero_lut);
            if !lut.path.as_os_str().is_empty() && lut.path.is_relative() {
                lut.path = parent
                    .as_ref()
                    .map(|p| p.join(&lut.path))
                    .unwrap_or(lut.path);
            }
            Ok(Resolved {
                film: String::new(),
                paper: None,
                seed: recipe.seed,
                metering: MeteringArg::Manual,
                shots,
                output_crop: recipe.output_crop,
                print: None,
                input_color_space: recipe.input_color_space,
                input_cctf_decoding: recipe.input_cctf_decoding,
                film_workflow: FilmWorkflow::default(),
                rgb: Some(RgbConfig {
                    lut,
                    effects: group.effects.unwrap_or_default(),
                    grade: group.grade.unwrap_or_default(),
                }),
                recipe_parent: parent,
            })
        }
        other => bail!(
            "multiple-exposure.recipe-schema: method must be \"film\" or \"rgb\", got {other:?}"
        ),
    }
}

fn recipe_error(error: serde_json::Error) -> anyhow::Error {
    if error.to_string().contains("weight") {
        anyhow::anyhow!(
            "multiple-exposure.weight-removed: replace each shot weight w with exposure_ev += log2(w); the old weight was applied after optics"
        )
    } else {
        anyhow::Error::new(error)
    }
}

fn read_shots(
    parent: &Option<PathBuf>,
    shots: &[RecipeShot],
) -> Result<Vec<(PathBuf, Option<String>, f64)>> {
    let mut resolved = Vec::with_capacity(shots.len());
    for (index, shot) in shots.iter().enumerate() {
        validate_shot(index, shot.exposure_ev)?;
        if shot.transform.is_some() {
            bail!(
                "multiple-exposure.unsupported: per-shot transform is not implemented; remove transform from shot {index}"
            );
        }
        let shot_path = if Path::new(&shot.path).is_absolute() {
            PathBuf::from(&shot.path)
        } else {
            parent
                .as_ref()
                .map(|p| p.join(&shot.path))
                .unwrap_or_else(|| PathBuf::from(&shot.path))
        };
        resolved.push((shot_path, shot.sha256.clone(), shot.exposure_ev));
    }
    Ok(resolved)
}

fn validate_shot(index: usize, ev: f64) -> Result<()> {
    if !ev.is_finite() || !(-10.0..=10.0).contains(&ev) {
        bail!(
            "multiple-exposure.ev-range: shot {index} exposure_ev must be finite and in [-10, 10]"
        );
    }
    Ok(())
}

fn scale_crop(crop: &CropSpec, full: (u32, u32), preview: (u32, u32)) -> Result<CropSpec> {
    let validate =
        |c: &CropSpec| -> Result<()> {
            let x1 = c.x.checked_add(c.width).ok_or_else(|| {
                anyhow::anyhow!("multiple-exposure.output-crop: x+width overflows")
            })?;
            let y1 = c.y.checked_add(c.height).ok_or_else(|| {
                anyhow::anyhow!("multiple-exposure.output-crop: y+height overflows")
            })?;
            if x1 > full.0 || y1 > full.1 {
                bail!(
                    "multiple-exposure.output-crop: crop {}x{}+{}+{} exceeds full output {}x{}",
                    c.width,
                    c.height,
                    c.x,
                    c.y,
                    full.0,
                    full.1
                );
            }
            Ok(())
        };
    validate(crop)?;
    let sx = preview.0 as f64 / full.0 as f64;
    let sy = preview.1 as f64 / full.1 as f64;
    let x = ((crop.x as f64) * sx).round() as u32;
    let y = ((crop.y as f64) * sy).round() as u32;
    let x1 = (((crop.x + crop.width) as f64) * sx).round() as u32;
    let y1 = (((crop.y + crop.height) as f64) * sy).round() as u32;
    let width = x1.saturating_sub(x).max(1).min(preview.0.saturating_sub(x));
    let height = y1.saturating_sub(y).max(1).min(preview.1.saturating_sub(y));
    Ok(CropSpec {
        x,
        y,
        width,
        height,
    })
}

fn parse_crop(value: &str) -> Result<CropSpec> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        bail!("multiple-exposure.output-crop: expected X,Y,W,H, got {value:?}");
    }
    let parse = |s: &str, name: &str| -> Result<u32> {
        s.parse::<u32>()
            .with_context(|| format!("multiple-exposure.output-crop: invalid {name} {s:?}"))
    };
    let crop = CropSpec {
        x: parse(parts[0], "x")?,
        y: parse(parts[1], "y")?,
        width: parse(parts[2], "width")?,
        height: parse(parts[3], "height")?,
    };
    if crop.width == 0 || crop.height == 0 {
        bail!("multiple-exposure.output-crop: width and height must be positive");
    }
    Ok(crop)
}

fn crop_image(image: &ImageBuf, crop: &CropSpec) -> Result<ImageBuf> {
    let x1 = crop
        .x
        .checked_add(crop.width)
        .ok_or_else(|| anyhow::anyhow!("multiple-exposure.output-crop: x+width overflows"))?;
    let y1 = crop
        .y
        .checked_add(crop.height)
        .ok_or_else(|| anyhow::anyhow!("multiple-exposure.output-crop: y+height overflows"))?;
    if x1 > image.width || y1 > image.height {
        bail!(
            "multiple-exposure.output-crop: crop {}x{}+{}+{} exceeds output {}x{}",
            crop.width,
            crop.height,
            crop.x,
            crop.y,
            image.width,
            image.height
        );
    }
    let row_len = (crop.width * 3) as usize;
    let mut data = Vec::with_capacity(row_len * crop.height as usize);
    for y in crop.y..y1 {
        let start = ((y * image.width + crop.x) * 3) as usize;
        data.extend_from_slice(&image.data[start..start + row_len]);
    }
    Ok(ImageBuf::from_data(crop.width, crop.height, data))
}

fn select_backend(args: &ProcessArgs) -> Result<Box<dyn ComputeBackend>> {
    match args.backend {
        Some(crate::Backend::Cpu) => Ok(Box::new(spektrafilm_backend::cpu_backend::CpuBackend)),
        Some(crate::Backend::Gpu) => Ok(Box::new(
            spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit()
                .ok_or_else(|| anyhow::anyhow!("GPU backend unavailable; use --backend cpu"))?,
        )),
        None => spektrafilm_gpu::select_backend().map_err(|e| anyhow::anyhow!("{e}")),
    }
}

fn save_output(path: &Path, image: &ImageBuf, args: &ProcessArgs) -> Result<()> {
    let detected = image_io::ImageFormat::detect(path)
        .with_context(|| format!("detecting output format: {}", path.display()))?;
    let format = args
        .format
        .map(OutputFormat::image_format)
        .unwrap_or(detected);
    if format != detected {
        bail!(
            "--format {} disagrees with output extension ({})",
            format.name(),
            detected.name()
        );
    }
    let default_depth = match format {
        image_io::ImageFormat::Jpeg | image_io::ImageFormat::Png => BitDepth::Eight,
        image_io::ImageFormat::Tiff => BitDepth::ThirtyTwo,
        image_io::ImageFormat::Exr => BitDepth::Sixteen,
    };
    let depth = match args.bit_depth {
        Some(bits) => BitDepth::try_from(bits).map_err(anyhow::Error::msg)?,
        None => default_depth,
    };
    let cctf_encoding = args
        .workflow
        .saving_cctf_encoding
        .unwrap_or(format != image_io::ImageFormat::Exr);
    let options = SaveOptions {
        depth,
        color_space: args.workflow.saving_color_space.as_deref().unwrap_or(
            if format == image_io::ImageFormat::Exr {
                "ACES2065-1"
            } else {
                "sRGB"
            },
        ),
        cctf_encoding,
        jpeg_quality: args.jpeg_quality,
        jpeg_subsampling: args.jpeg_subsampling.map(|value| match value {
            JpegSubsamplingArg::Yuv444 => JpegSubsampling::Yuv444,
            JpegSubsamplingArg::Yuv420 => JpegSubsampling::Yuv420,
        }),
        compression: args.compression.map(|value| match value {
            CompressionArg::Zip => Compression::Zip,
            CompressionArg::None => Compression::None,
        }),
    };
    image_io::save(path, image, options, None)
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("saving {}", path.display()))?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        bail!(
            "multiple-exposure.recipe-hash: {} expected {}, got {}",
            path.display(),
            expected,
            actual
        );
    }
    Ok(())
}

fn resolved_to_recipe(
    resolved: &Resolved,
    shots: &[(PathBuf, Option<String>, f64)],
    _args: &ProcessArgs,
) -> Result<serde_json::Value> {
    let mut recipe_shots = Vec::with_capacity(shots.len());
    for (path, sha256, ev) in shots {
        let recipe_path = match &resolved.recipe_parent {
            Some(parent) => path
                .strip_prefix(parent)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string()),
            None => path.display().to_string(),
        };
        let hash = match sha256 {
            Some(hash) => hash.clone(),
            None => sha256_file(path)?,
        };
        recipe_shots.push(RecipeShot {
            path: recipe_path,
            sha256: Some(hash),
            exposure_ev: *ev,
            transform: None,
        });
    }
    let recipe = if let Some(rgb) = &resolved.rgb {
        RecipeV2 {
            schema: RECIPE_SCHEMA_V2.into(),
            method: "rgb".into(),
            seed: resolved.seed,
            shots: recipe_shots,
            output_crop: resolved.output_crop.clone(),
            input_color_space: resolved.input_color_space.clone(),
            input_cctf_decoding: resolved.input_cctf_decoding,
            film: None,
            rgb: Some(RgbRecipeGroup {
                lut: Some(rgb.lut.clone()),
                effects: Some(rgb.effects.clone()),
                grade: Some(rgb.grade.clone()),
            }),
        }
    } else {
        RecipeV2 {
            schema: RECIPE_SCHEMA_V2.into(),
            method: "film".into(),
            seed: resolved.seed,
            shots: recipe_shots,
            output_crop: resolved.output_crop.clone(),
            input_color_space: resolved.input_color_space.clone(),
            input_cctf_decoding: resolved.input_cctf_decoding,
            film: Some(FilmRecipeGroup {
                film: resolved.film.clone(),
                paper: resolved.paper.clone(),
                metering: resolved.metering,
                print: resolved.print.clone(),
                route: resolved.film_workflow.route.clone(),
                scan_film: resolved.film_workflow.scan_film,
                scan_output: resolved.film_workflow.scan_output.clone(),
                scan_profile: resolved.film_workflow.scan_profile.clone(),
                raw_white_balance: resolved.film_workflow.raw_white_balance.clone(),
                raw_temperature: resolved.film_workflow.raw_temperature,
                raw_tint: resolved.film_workflow.raw_tint,
                lens_correction: resolved.film_workflow.lens_correction,
                film_channel_swap: resolved.film_workflow.film_channel_swap.clone(),
                print_channel_swap: resolved.film_workflow.print_channel_swap.clone(),
            }),
            rgb: None,
        }
    };
    Ok(serde_json::to_value(recipe)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::precision::from_f64;

    #[test]
    fn parses_memory_limit() {
        assert!(parse_memory_limit("auto").unwrap().is_none());
        assert_eq!(parse_memory_limit("1GiB").unwrap(), Some(1024_u64.pow(3)));
        assert_eq!(
            parse_memory_limit("2MiB").unwrap(),
            Some(2 * 1024_u64.pow(2))
        );
        assert_eq!(parse_memory_limit("4096").unwrap(), Some(4096));
        assert!(parse_memory_limit("nope").is_err());
        assert!(parse_memory_limit("999999999999999999999GiB").is_err());
    }

    #[test]
    fn peak_estimate_covers_measured_high_res_cpu_runs() {
        // Measured f32 streaming peaks: 6.45 GiB at 61 MP and 10.46 GiB at
        // 100 MP. Both builds must estimate at least those peaks; the old
        // 9-bytes/sample formula only estimated 2.0/3.0 GiB.
        let gib = 1024_u64.pow(3);
        let e61 = estimate_peak_bytes(9568, 6376) as f64 / gib as f64;
        let e100 = estimate_peak_bytes(10000, 10000) as f64 / gib as f64;
        assert!(
            e61 >= 6.4,
            "61 MP estimate {e61:.3} GiB below measured peak"
        );
        assert!(
            e100 >= 10.4,
            "100 MP estimate {e100:.3} GiB below measured peak"
        );
    }

    #[test]
    fn default_budget_covers_estimate_across_sensor_sizes() {
        for (width, height) in [
            (9568, 6376),   // 61 MP
            (10000, 10000), // 100 MP acceptance case
            (11648, 8736),  // 102 MP GFX 100 II
            (12000, 9000),  // 108 MP
        ] {
            let estimate = estimate_peak_bytes(width, height);
            let budget = default_memory_budget(width, height);
            assert!(
                budget >= estimate,
                "{width}x{height}: default budget {budget} below estimate {estimate}"
            );
        }
    }

    #[test]
    fn parses_output_crop() {
        let crop = parse_crop("1, 2, 30, 40").unwrap();
        assert_eq!((crop.x, crop.y, crop.width, crop.height), (1, 2, 30, 40));
        assert!(parse_crop("0,0,0,1").is_err());
        assert!(parse_crop("0,0,1").is_err());
        assert!(parse_crop("0,0,-1,1").is_err());
    }

    #[test]
    fn crops_output_pixels() {
        let mut data = Vec::new();
        for i in 0..(4 * 4 * 3) {
            data.push(from_f64(i as f64));
        }
        let image = ImageBuf::from_data(4, 4, data);
        let crop = parse_crop("1,1,2,2").unwrap();
        let out = crop_image(&image, &crop).unwrap();
        assert_eq!((out.width, out.height), (2, 2));
        let expected = [
            15.0, 16.0, 17.0, 18.0, 19.0, 20.0, 27.0, 28.0, 29.0, 30.0, 31.0, 32.0,
        ];
        for (got, expected) in out.data.iter().zip(expected) {
            assert_eq!(spektrafilm_math::precision::to_f64(*got), expected);
        }
    }

    #[test]
    fn scales_output_crop_to_preview() {
        let crop = CropSpec {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        };
        let scaled = scale_crop(&crop, (64, 64), (32, 32)).unwrap();
        assert_eq!(
            (scaled.x, scaled.y, scaled.width, scaled.height),
            (0, 0, 16, 16)
        );
        let bad = CropSpec {
            x: 60,
            y: 60,
            width: 8,
            height: 8,
        };
        assert!(scale_crop(&bad, (64, 64), (32, 32)).is_err());
    }

    #[test]
    fn rejects_input_crop_in_recipe() {
        let json = r#"{
          "schema": "spektrafilm.multiple-exposure/1",
          "film": "kodak_portra_400",
          "seed": 0,
          "metering": "manual",
          "shots": [
            {"path": "a.png", "exposure_ev": 0.0, "crop": {"x":0,"y":0,"width":1,"height":1}},
            {"path": "b.png", "exposure_ev": 0.0}
          ]
        }"#;
        assert!(serde_json::from_str::<Recipe>(json).is_err());
    }

    #[test]
    fn rejects_recipe_with_removed_weight_field() {
        let json = r#"{
          "schema": "spektrafilm.multiple-exposure/1",
          "film": "kodak_portra_400",
          "seed": 0,
          "metering": "manual",
          "shots": [
            {"path": "a.png", "exposure_ev": -1.0, "weight": 0.5},
            {"path": "b.png", "exposure_ev": -1.0, "weight": 0.5}
          ]
        }"#;
        let error = serde_json::from_str::<Recipe>(json)
            .unwrap_err()
            .to_string();
        assert!(error.contains("weight"), "unexpected recipe error: {error}");
    }

    #[test]
    fn default_flag_metering_is_manual() {
        let args = ProcessArgs {
            expose: vec![PathBuf::from("a.png"), PathBuf::from("b.png")],
            film: Some("kodak_portra_400".into()),
            ..Default::default()
        };
        let resolved = resolve_flags(&args).unwrap();
        assert_eq!(resolved.metering, MeteringArg::Manual);
    }

    #[test]
    fn validate_shot_rejects_out_of_range_ev() {
        assert!(validate_shot(0, -1.0).is_ok());
        assert!(validate_shot(1, 10.0).is_ok());
        assert!(validate_shot(2, 10.01).is_err());
        assert!(validate_shot(3, f64::NAN).is_err());
    }

    #[test]
    fn recipe_round_trip_keeps_input_interpretation() {
        let recipe = Recipe {
            schema: RECIPE_SCHEMA.into(),
            film: "kodak_portra_400".into(),
            paper: Some("kodak_portra_endura".into()),
            seed: 0,
            metering: MeteringArg::Manual,
            shots: vec![
                RecipeShot {
                    path: "a.png".into(),
                    sha256: None,
                    exposure_ev: -1.0,
                    transform: None,
                },
                RecipeShot {
                    path: "b.png".into(),
                    sha256: None,
                    exposure_ev: -1.0,
                    transform: None,
                },
            ],
            output_crop: Some(CropSpec {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            }),
            print: None,
            input_color_space: Some("ProPhoto RGB".into()),
            input_cctf_decoding: Some(false),
        };
        let text = serde_json::to_string(&recipe).unwrap();
        assert!(!text.contains("weight"));
        let back: Recipe = serde_json::from_str(&text).unwrap();
        assert_eq!(back.input_color_space.as_deref(), Some("ProPhoto RGB"));
        assert_eq!(back.input_cctf_decoding, Some(false));
    }
}
