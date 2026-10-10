mod contract;
mod lut;
mod preset;
mod runner;
mod telemetry;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_core::image_io::{self, BitDepth, Compression, JpegSubsampling, SaveOptions};
use spektrafilm_core::params::Tap;
use spektrafilm_core::params_builder::resize_for_preview;
use spektrafilm_core::profile;
use spektrafilm_core::runtime::{DigestMode, Runtime};
use spektrafilm_math::image::ImageBuf;

use std::time::Instant;
#[derive(Parser)]
#[command(
    name = "spektrafilm",
    about = "Physically-based spectral film emulation"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Process an image through the film simulation pipeline.
    Process {
        /// Compute backend (cpu or gpu). If omitted, uses SPEKTRAFILM_BACKEND/default selection.
        #[arg(long, value_enum)]
        backend: Option<Backend>,
        /// Input image (TIFF, EXR, PNG, JPEG, or camera RAW).
        input: PathBuf,
        /// Output image path (TIFF, EXR, PNG, or JPEG).
        #[arg(short, long)]
        output: PathBuf,
        /// Output format. If omitted, inferred from the output extension.
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,
        /// Output bit depth: 8, 16, or 32.
        #[arg(long)]
        bit_depth: Option<u8>,
        /// JPEG quality (1..=100).
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=100))]
        jpeg_quality: Option<u8>,
        /// JPEG chroma subsampling (444 or 420).
        #[arg(long, value_enum)]
        jpeg_subsampling: Option<JpegSubsamplingArg>,
        /// TIFF/EXR compression.
        #[arg(long, value_enum)]
        compression: Option<CompressionArg>,
        #[command(flatten)]
        workflow: WorkflowOptions,
        /// Film stock name (e.g. kodak_portra_400). Required unless --preset is used.
        #[arg(long, required_unless_present = "preset", conflicts_with = "preset")]
        film: Option<String>,
        /// Paper stock name (e.g. fujifilm_crystal_archive_typeii).
        /// If omitted for --scan-film or a direct scan route, uses the film as
        /// the scan profile; otherwise it uses the film's target_print.
        #[arg(long, conflicts_with = "preset")]
        paper: Option<String>,
        /// A built-in preset ID or .toml/.json preset file.
        #[arg(long, conflicts_with_all = ["film", "paper", "params"])]
        preset: Option<String>,
        /// Scan film directly (skip printing stage).
        #[arg(long)]
        scan_film: bool,
        /// Print per-stage timing information.
        #[arg(long)]
        timings: bool,
        /// Save one local JSON diagnostics report without replacing an existing file.
        #[arg(long)]
        diagnostics: Option<PathBuf>,
        /// Collect GPU pass timings when available; unsupported devices retain summary.
        #[arg(long, requires = "diagnostics")]
        gpu_timings: bool,
        /// Path to JSON params file for overrides.
        #[arg(long, conflicts_with = "preset")]
        params: Option<PathBuf>,
        /// Sparse parameter source (a .toml/.json file or comma-separated assignments).
        #[arg(long = "set", action = clap::ArgAction::Append)]
        sources: Vec<String>,
        /// Resolve and validate without decoding or rendering.
        #[arg(long)]
        dry_run: bool,
        /// Dump the raw f64 output buffer (HxWx3, row-major, channel-interleaved)
        /// before sRGB encoding/clipping. Used for bit-exact parity comparison.
        #[arg(long)]
        raw_out: Option<PathBuf>,
        /// Run the pipeline N times in the same process.
        #[arg(long, default_value = "1")]
        iters: usize,
        /// Path to the data directory.
        #[arg(long, default_value = "data", env = "SPEKTRAFILM_DATA_DIR")]
        data_dir: PathBuf,
    },
    /// List available film and paper profiles.
    ListProfiles {
        /// Path to the data directory.
        #[arg(long, default_value = "data", env = "SPEKTRAFILM_DATA_DIR")]
        data_dir: PathBuf,
    },
    /// Build LUT bundles or list stocks and transport color spaces.
    Lut {
        #[command(subcommand)]
        command: lut::LutCommand,
    },
    /// Export a canonical 1-LUT cube (encoded ProPhoto RGB to sRGB, native headroom).
    ExportLut {
        /// Film stock name.
        #[arg(long)]
        film: String,
        /// Paper stock name.
        #[arg(long)]
        paper: Option<String>,
        /// LUT cube size (e.g. 33, 65).
        #[arg(long, default_value = "33")]
        size: u32,
        /// Output .cube file path.
        #[arg(short, long)]
        output: PathBuf,
        /// Path to the data directory.
        #[arg(long, default_value = "data", env = "SPEKTRAFILM_DATA_DIR")]
        data_dir: PathBuf,
    },
    /// Describe the machine-readable runtime and admitted contract.
    Describe {
        #[arg(long, default_value = "json")]
        format: String,
        #[arg(long)]
        module: Option<String>,
    },
    /// Render one structured recipe under the fork runner contract.
    Render {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        recipe: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value = "data", env = "SPEKTRAFILM_DATA_DIR")]
        data_dir: PathBuf,
        /// Save one local JSON diagnostics report without replacing an existing file.
        #[arg(long)]
        diagnostics: Option<PathBuf>,
        /// Collect GPU pass timings when available; unsupported devices retain summary.
        #[arg(long, requires = "diagnostics")]
        gpu_timings: bool,
    },
    /// Inspect an output without processing or modifying it.
    Inspect {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Inspect immutable built-in look presets.
    Preset {
        #[command(subcommand)]
        command: preset::PresetCommand,
    },
    /// Run the declared corpus and write a structured parity report.
    Parity {
        #[arg(long)]
        corpus: PathBuf,
        #[arg(long)]
        report: PathBuf,
        #[arg(long, default_value = "data", env = "SPEKTRAFILM_DATA_DIR")]
        data_dir: PathBuf,
    },
}
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Backend {
    Cpu,
    Gpu,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Jpeg,
    Png,
    Tiff,
    Exr,
}

impl OutputFormat {
    fn image_format(self) -> image_io::ImageFormat {
        match self {
            Self::Jpeg => image_io::ImageFormat::Jpeg,
            Self::Png => image_io::ImageFormat::Png,
            Self::Tiff => image_io::ImageFormat::Tiff,
            Self::Exr => image_io::ImageFormat::Exr,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Exr => "exr",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum JpegSubsamplingArg {
    #[value(name = "444")]
    Yuv444,
    #[value(name = "420")]
    Yuv420,
}

impl JpegSubsamplingArg {
    fn core(self) -> JpegSubsampling {
        match self {
            Self::Yuv444 => JpegSubsampling::Yuv444,
            Self::Yuv420 => JpegSubsampling::Yuv420,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CompressionArg {
    Zip,
    None,
}

impl CompressionArg {
    fn core(self) -> Compression {
        match self {
            Self::Zip => Compression::Zip,
            Self::None => Compression::None,
        }
    }
}

#[derive(clap::Args)]
struct WorkflowOptions {
    #[arg(long)]
    saving_color_space: Option<String>,
    #[arg(long, action = clap::ArgAction::Set)]
    saving_cctf_encoding: Option<bool>,
    #[arg(long, default_value = "as-shot", value_parser = ["as-shot", "daylight", "tungsten", "custom"])]
    raw_white_balance: String,
    #[arg(long)]
    raw_temperature: Option<f64>,
    #[arg(long)]
    raw_tint: Option<f64>,
    #[arg(long)]
    lens_correction: bool,
    #[arg(long)]
    route: Option<String>,
    #[arg(long, value_parser = ["direct_scan", "positive_scan"])]
    scan_output: Option<String>,
    #[arg(long, default_value = "none")]
    film_channel_swap: String,
    #[arg(long, default_value = "none")]
    print_channel_swap: String,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Process {
            input,
            output,
            format,
            bit_depth,
            jpeg_quality,
            jpeg_subsampling,
            compression,
            workflow,
            film,
            paper,
            preset,
            scan_film,
            timings,
            params: params_file,
            sources,
            dry_run,
            raw_out,
            iters,
            data_dir,
            backend,
            diagnostics,
            gpu_timings,
        } => {
            let data_dir = resolve_data_dir(data_dir);
            let mut protected = vec![input.as_path(), output.as_path()];
            protected.extend(params_file.as_deref());
            protected.extend(raw_out.as_deref());
            protected.extend(
                preset
                    .as_deref()
                    .filter(|selector| selector.ends_with(".toml") || selector.ends_with(".json"))
                    .map(Path::new),
            );
            protected.extend(
                sources
                    .iter()
                    .filter(|source| source.ends_with(".toml") || source.ends_with(".json"))
                    .map(Path::new),
            );
            telemetry::run(
                spektrafilm_core::telemetry::OperationKind::Process,
                diagnostics.as_deref(),
                gpu_timings,
                &protected,
                telemetry::requested_backend(backend),
                |invocation| {
                    cmd_process(
                        &input,
                        &output,
                        format,
                        bit_depth,
                        jpeg_quality,
                        jpeg_subsampling,
                        compression,
                        &workflow,
                        film.as_deref(),
                        paper.as_deref(),
                        preset.as_deref(),
                        scan_film,
                        timings,
                        params_file.as_deref(),
                        &sources,
                        dry_run,
                        raw_out.as_deref(),
                        iters,
                        &data_dir,
                        backend,
                        invocation,
                    )
                },
            )?;
        }
        Commands::ListProfiles { data_dir } => {
            let data_dir = resolve_data_dir(data_dir);
            cmd_list_profiles(&data_dir);
        }
        Commands::Lut { command } => lut::run(command)?,
        Commands::ExportLut {
            film,
            paper,
            size,
            output,
            data_dir,
        } => {
            lut::export_lut(
                &film,
                paper.as_deref(),
                size,
                &output,
                &resolve_data_dir(data_dir),
            )?;
        }
        Commands::Describe { format, module } => {
            if format != "json" {
                bail!("unsupported describe format {format}; use json");
            }
            let value = if let Some(module) = module {
                spektrafilm_core::params::sources::describe_module(&module)
                    .map_err(anyhow::Error::msg)?
            } else {
                contract::describe()
            };
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        Commands::Render {
            input,
            recipe,
            output,
            data_dir,
            diagnostics,
            gpu_timings,
        } => runner::cmd_render(
            &input,
            &recipe,
            &output,
            &data_dir,
            diagnostics.as_deref(),
            gpu_timings,
        )?,
        Commands::Inspect { input, format } => {
            if format != "json" {
                bail!("unsupported inspect format {format}; use json");
            }
            println!("{}", serde_json::to_string_pretty(&inspect(&input)?)?);
        }
        Commands::Preset { command } => preset::run(command)?,
        Commands::Parity {
            corpus,
            report,
            data_dir,
        } => runner::cmd_parity(&corpus, &report, &data_dir)?,
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_process(
    input: &Path,
    output: &Path,
    format: Option<OutputFormat>,
    bit_depth: Option<u8>,
    jpeg_quality: Option<u8>,
    jpeg_subsampling: Option<JpegSubsamplingArg>,
    compression: Option<CompressionArg>,
    workflow: &WorkflowOptions,
    film_name: Option<&str>,
    paper_name: Option<&str>,
    preset: Option<&str>,
    scan_film: bool,
    show_timings: bool,
    params_file: Option<&Path>,
    sources: &[String],
    dry_run: bool,
    raw_out: Option<&Path>,
    iters: usize,
    data_dir: &Path,
    backend_choice: Option<Backend>,
    invocation: &mut telemetry::Invocation,
) -> Result<()> {
    let total_start = Instant::now();
    use spektrafilm_core::telemetry::{IssueBoundary, IssueCategory};
    let context = invocation.operation.context();
    if iters == 0 {
        bail!("--iters must be at least 1");
    }
    let extension_format = image_io::ImageFormat::detect(output)
        .with_context(|| format!("detecting output format: {}", output.display()))?;
    let output_format = format.unwrap_or_else(|| match extension_format {
        image_io::ImageFormat::Jpeg => OutputFormat::Jpeg,
        image_io::ImageFormat::Png => OutputFormat::Png,
        image_io::ImageFormat::Tiff => OutputFormat::Tiff,
        image_io::ImageFormat::Exr => OutputFormat::Exr,
    });
    if output_format.image_format() != extension_format {
        bail!(
            "--format {} disagrees with output extension ({})",
            output_format.name(),
            format_name(extension_format)
        );
    }
    let default_depth = match output_format {
        OutputFormat::Jpeg | OutputFormat::Png => 8,
        OutputFormat::Tiff | OutputFormat::Exr => 16,
    };
    let depth = BitDepth::try_from(bit_depth.unwrap_or(default_depth))?;
    if matches!(output_format, OutputFormat::Jpeg | OutputFormat::Png) && depth != BitDepth::Eight {
        bail!("{} output requires 8-bit depth", output_format.name());
    }
    if matches!(output_format, OutputFormat::Exr) && depth == BitDepth::Eight {
        bail!("EXR output requires 16-bit or 32-bit depth");
    }
    if matches!(output_format, OutputFormat::Jpeg) {
        if let Some(quality) = jpeg_quality {
            if !(1..=100).contains(&quality) {
                bail!("JPEG quality must be in 1..=100");
            }
        }
    } else if jpeg_quality.is_some() {
        bail!("--jpeg-quality is only valid for JPEG output");
    }
    if !matches!(output_format, OutputFormat::Jpeg) && jpeg_subsampling.is_some() {
        bail!("--jpeg-subsampling is only valid for JPEG output");
    }
    if matches!(output_format, OutputFormat::Jpeg | OutputFormat::Png) && compression.is_some() {
        bail!("--compression is only valid for TIFF or EXR output");
    }
    let jpeg_quality =
        matches!(output_format, OutputFormat::Jpeg).then(|| jpeg_quality.unwrap_or(95));
    let jpeg_subsampling = matches!(output_format, OutputFormat::Jpeg).then(|| {
        jpeg_subsampling
            .unwrap_or(JpegSubsamplingArg::Yuv444)
            .core()
    });
    let compression = matches!(output_format, OutputFormat::Tiff | OutputFormat::Exr)
        .then(|| compression.unwrap_or(CompressionArg::Zip).core());
    invocation.boundary(IssueCategory::Input, IssueBoundary::InputLoad);
    if !input.is_file() {
        bail!("input is not a regular file: {}", input.display());
    }
    std::fs::File::open(input)
        .with_context(|| format!("input is not readable: {}", input.display()))?;
    let input_is_raw = image_io::is_raw(input);
    if !input_is_raw {
        image_io::ImageFormat::detect(input)
            .with_context(|| format!("unsupported input format: {}", input.display()))?;
    }
    if input_is_raw {
        // Validate RAW option consistency without decoding pixels.
        let raw_options = raw_options(&workflow);
        validate_raw_options(&raw_options)?;
    }
    invocation.boundary(IssueCategory::Configuration, IssueBoundary::RuntimePrepare);
    let resolved = spektrafilm_core::process_params::resolve(
        spektrafilm_core::process_params::ProcessParamsRequest {
            film: film_name,
            paper: paper_name,
            preset,
            params_file,
            sources,
            route: workflow.route.as_deref(),
            scan_film,
            scan_output: workflow.scan_output.as_deref(),
            input_is_raw,
            digest_mode: if std::env::var_os("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS").as_deref()
                == Some(std::ffi::OsStr::new("1"))
            {
                DigestMode::PreserveUserEdits
            } else {
                DigestMode::ApplyStockSpecifics
            },
        },
        data_dir,
    )
    .map_err(anyhow::Error::msg)?;
    let mut film = resolved.film;
    let mut print = resolved.print;
    apply_channel_swap(&mut film, &workflow.film_channel_swap)?;
    apply_channel_swap(&mut print, &workflow.print_channel_swap)?;
    let params = resolved.params;
    let inject = params
        .taps
        .inject
        .as_deref()
        .map(Tap::parse)
        .transpose()
        .map_err(anyhow::Error::msg)
        .with_context(|| "params taps.inject")?;
    let collect = params
        .taps
        .collect
        .as_deref()
        .map(Tap::parse)
        .transpose()
        .map_err(anyhow::Error::msg)
        .with_context(|| "params taps.collect")?;

    let output_color_space = params.io.output_color_space.clone();
    let output_cctf_encoding = params.io.output_cctf_encoding;
    let saving_space = workflow.saving_color_space.clone().unwrap_or_else(|| {
        if output_format == OutputFormat::Exr {
            "ACES2065-1".into()
        } else {
            output_color_space.clone()
        }
    });
    let saving_space_resolved = spektrafilm_math::colorspace::resolve(&saving_space)
        .map_err(|e| anyhow::anyhow!("unsupported saving color space {saving_space:?}: {e}"))?;
    let output_space_resolved = spektrafilm_math::colorspace::resolve(&output_color_space)
        .map_err(|e| {
            anyhow::anyhow!("unsupported output color space {output_color_space:?}: {e}")
        })?;
    let _conversion_matrix = spektrafilm_math::colorspace::conversion_matrix(
        output_space_resolved,
        saving_space_resolved,
    );
    let saving_encoded = workflow
        .saving_cctf_encoding
        .unwrap_or(output_format != OutputFormat::Exr && output_cctf_encoding);
    if matches!(output_format, OutputFormat::Jpeg | OutputFormat::Png) && !saving_encoded {
        bail!(
            "{} output requires encoded saving output",
            output_format.name()
        );
    }
    if output_format == OutputFormat::Exr && saving_encoded {
        bail!("EXR output requires linear saving output");
    }
    if output_format == OutputFormat::Exr {
        if !matches!(saving_space.as_str(), "sRGB" | "ACES2065-1") {
            bail!("EXR color space must be sRGB or ACES2065-1");
        }
        if matches!(compression, Some(Compression::None)) {
            bail!("EXR compression is always zip");
        }
    }
    // Active V3 resolves its field on the CPU and never enters the resident
    // WGPU chain, whatever backend the surrounding stages use. Its supported
    // condition is validated here so the dry run reports and rejects exactly
    // what a render does, and the effective backend is reported next to the
    // selected one.
    spektrafilm_core::stages::grain_v3::validate_supported(&film, &params)
        .map_err(anyhow::Error::msg)?;
    let grain_v3_active = spektrafilm_core::stages::grain_v3::active(&params);
    if grain_v3_active {
        eprintln!(
            "Backend (effective): {}",
            spektrafilm_core::stages::grain_v3::effective_backend_note()
        );
    }
    // An active V3 field resolves on the CPU whatever backend the rest of
    // the chain uses, so the dry run reports the effective backend it did
    // not select.
    let grain_v3 = json!({
        "active": grain_v3_active,
        "effective_backend": if grain_v3_active { Some("cpu") } else { None },
        "note": if grain_v3_active {
            Some(spektrafilm_core::stages::grain_v3::effective_backend_note())
        } else {
            None
        },
    });
    if dry_run {
        let backend_env = std::env::var("SPEKTRAFILM_BACKEND").ok();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "film_profile": resolved.film_name,
                "print_profile": resolved.print_name,
                "parameters": params,
                "film_grain_v3": grain_v3,
                "output": {
                    "path": output,
                    "format": output_format.name(),
                    "bit_depth": depth.bits(),
                    "color_space": saving_space,
                    "cctf_encoding": saving_encoded,
                    "compression": compression.map(|c| match c { Compression::Zip => "zip", Compression::None => "none" }),
                    "jpeg_quality": jpeg_quality,
                    "jpeg_subsampling": jpeg_subsampling.map(|s| match s { JpegSubsampling::Yuv444 => "444", JpegSubsampling::Yuv420 => "420" })
                },
                "backend": {
                    "requested": backend_choice.map(|b| match b { Backend::Cpu => "cpu", Backend::Gpu => "gpu" }),
                    "policy": backend_choice.map(|b| match b { Backend::Cpu => "cpu", Backend::Gpu => "gpu" }).unwrap_or("auto"),
                    "environment": backend_env
                }
            }))?
        );
        return Ok(());
    }
    let backend = invocation.phase(
        "backend_init",
        IssueCategory::Backend,
        IssueBoundary::BackendInit,
        &context,
        |backend_context| {
            let backend: Box<dyn spektrafilm_gpu::ComputeBackend> = match backend_choice {
                Some(Backend::Cpu) => Box::new(spektrafilm_gpu::cpu_backend::CpuBackend),
                Some(Backend::Gpu) => Box::new(
                    spektrafilm_gpu::wgpu_backend::WgpuBackend::new().ok_or_else(|| {
                        anyhow::anyhow!("GPU backend unavailable; use --backend cpu")
                    })?,
                ),
                None => spektrafilm_gpu::select_backend(),
            };
            telemetry::observe_backend(backend.as_ref(), backend_context);
            Ok(backend)
        },
    )?;
    eprintln!("Backend: {}", backend.name());

    let t = Instant::now();
    let (image, metadata) = invocation.phase(
        "input_load",
        IssueCategory::Input,
        IssueBoundary::InputLoad,
        &context,
        |_| {
            let (image, metadata) = if input_is_raw {
                let options = spektrafilm_raw::RawOptions {
                    white_balance: match workflow.raw_white_balance.as_str() {
                        "daylight" => spektrafilm_raw::WhiteBalance::Daylight,
                        "tungsten" => spektrafilm_raw::WhiteBalance::Tungsten,
                        "custom" => spektrafilm_raw::WhiteBalance::Custom,
                        _ => spektrafilm_raw::WhiteBalance::AsShot,
                    },
                    temperature: workflow.raw_temperature,
                    tint: workflow.raw_tint,
                    lens_correction: workflow.lens_correction,
                };
                (
                    spektrafilm_raw::load(input, &options)
                        .map_err(anyhow::Error::msg)?
                        .image,
                    image_io::read_metadata(input),
                )
            } else {
                let loaded = image_io::load(input)
                    .with_context(|| format!("loading image: {}", input.display()))?;
                (loaded.image, loaded.metadata)
            };
            Ok((image, metadata))
        },
    )?;
    invocation
        .operation
        .set_input_dimensions(image.width as u32, image.height as u32);
    // Preview mode: bound the long edge before processing (upstream
    // `simulate_preview` + `resize_for_preview`).
    let image = invocation.phase(
        "input_prepare",
        IssueCategory::Input,
        IssueBoundary::InputPrepare,
        &context,
        |_| {
            let image = if params.settings.preview_mode {
                let resized = resize_for_preview(&image, params.settings.preview_max_size);
                eprintln!(
                    "Preview: {}x{} → {}x{} (max edge {})",
                    image.width,
                    image.height,
                    resized.width,
                    resized.height,
                    params.settings.preview_max_size
                );
                resized
            } else {
                image
            };
            Ok(image)
        },
    )?;
    eprintln!(
        "Image loaded: {}x{} ({} MP), {} ms",
        image.width,
        image.height,
        (image.pixel_count() as f64 / 1e6 * 10.0).round() / 10.0,
        t.elapsed().as_millis()
    );

    // Run the calibrated runtime — a missing spectral LUT is a hard error.
    let t = Instant::now();
    let runtime = invocation.phase(
        "runtime_prepare",
        IssueCategory::Configuration,
        IssueBoundary::RuntimePrepare,
        &context,
        |prepare_context| {
            Runtime::new_observed(film, print, params, data_dir, prepare_context).map_err(|e| {
        anyhow::anyhow!(
            "spectral pipeline construction failed: {e} — the simplified no-LUT fallback was \
             removed; check that the profiles/data directory contains the spectral LUTs \
             (looked under {})",
            data_dir.display()
        )
    })
        },
    )?;
    if context.enabled() {
        invocation
            .operation
            .set_render_configuration(runtime.telemetry_configuration());
    }
    let output_attempt = std::cell::Cell::new(0);
    let run_once = |image: ImageBuf, index: u64| -> Result<ImageBuf> {
        let attempt = invocation.operation.attempt(index);
        let attempt_id = attempt.context().id();
        let result = invocation.phase(
            "simulation",
            IssueCategory::Simulation,
            IssueBoundary::Simulation,
            attempt.context(),
            |simulation_context| {
                let out = if inject.is_none() && collect.is_none() {
                    runtime
                        .process_observed(image, backend.as_ref(), simulation_context)
                        .map_err(anyhow::Error::msg)?
                } else {
                    runtime
                        .process_with_taps_observed(
                            image,
                            backend.as_ref(),
                            inject,
                            collect,
                            simulation_context,
                        )
                        .map_err(|e| anyhow::anyhow!("pipeline taps: {e}"))?
                };
                Ok(out)
            },
        );
        attempt.finish(if result.is_ok() {
            spektrafilm_gpu::telemetry::Outcome::Succeeded
        } else {
            spektrafilm_gpu::telemetry::Outcome::Failed
        });
        if result.is_ok() {
            output_attempt.set(attempt_id);
        }
        result
    };
    let result = if iters > 1 {
        // Warm-cache bench: process the image `iters` times in the same backend
        // instance. The first iteration pays shader-compile cost; subsequent ones
        // hit the cache and reflect "GUI live preview" performance.
        let mut last = run_once(image.clone(), 1)?;
        let iter1_ms = t.elapsed().as_millis();
        eprintln!("Pipeline (iter 1): {} ms (cold)", iter1_ms);
        for i in 2..=iters {
            let ti = Instant::now();
            last = run_once(image.clone(), i as u64)?;
            eprintln!(
                "Pipeline (iter {}): {} ms (warm)",
                i,
                ti.elapsed().as_millis()
            );
        }
        last
    } else {
        let r = run_once(image, 1)?;
        eprintln!("Pipeline: {} ms", t.elapsed().as_millis());
        r
    };

    // Save output
    let t = Instant::now();
    invocation.operation.set_saved_attempt(output_attempt.get());
    let options = SaveOptions {
        depth,
        color_space: &saving_space,
        cctf_encoding: saving_encoded,
        jpeg_quality,
        jpeg_subsampling,
        compression,
    };
    invocation.operation.set_saving_configuration(
        spektrafilm_core::telemetry::SavingConfiguration::from_save_options(
            output_format.image_format(),
            &options,
        ),
    );
    let saving_image = invocation.phase(
        "saving_convert",
        IssueCategory::Writing,
        IssueBoundary::SavingConvert,
        &context,
        |_| {
            image_io::convert_image(
                &result,
                &output_color_space,
                output_cctf_encoding,
                &saving_space,
                saving_encoded,
            )
            .map_err(anyhow::Error::from)
        },
    )?;
    let report = invocation.phase(
        "file_write",
        IssueCategory::Writing,
        IssueBoundary::FileWrite,
        &context,
        |write_context| {
            image_io::save_observed(
                output,
                &saving_image,
                options,
                metadata.as_ref(),
                write_context,
            )
            .with_context(|| format!("saving image: {}", output.display()))
        },
    )?;
    if let Some(warning) = report.metadata_warning {
        eprintln!("Warning: {warning}");
    }
    invocation
        .operation
        .set_published_dimensions(result.width as u32, result.height as u32);
    eprintln!(
        "Saved: {} ({}x{}), {} ms",
        output.display(),
        result.width,
        result.height,
        t.elapsed().as_millis()
    );

    // Optional raw f64 dump for bit-exact parity comparison.
    if let Some(p) = raw_out {
        invocation.phase(
            "file_write",
            IssueCategory::Writing,
            IssueBoundary::FileWrite,
            &context,
            |_| {
                use std::io::Write;
                let buf: Vec<f64> = result.data.iter().map(|&v| v as f64).collect();
                let bytes = bytemuck_cast_f64_to_bytes(&buf);
                let mut f = std::fs::File::create(p)
                    .with_context(|| format!("creating raw_out file: {}", p.display()))?;
                f.write_all(bytes)?;
                eprintln!("raw f64 buffer: {} ({} values)", p.display(), buf.len());
                Ok(())
            },
        )?;
    }

    if show_timings {
        eprintln!("Total: {} ms", total_start.elapsed().as_millis());
    }

    Ok(())
}

fn inspect(input: &Path) -> Result<Value> {
    if !input.is_file() {
        bail!("input is not a regular file: {}", input.display());
    }
    let bytes = fs::read(input)?;
    let loaded =
        image_io::load(input).with_context(|| format!("inspecting image {}", input.display()))?;
    let format = image_io::ImageFormat::detect(input)?;
    let precision_bits = match format {
        image_io::ImageFormat::Png | image_io::ImageFormat::Jpeg => Some(8),
        image_io::ImageFormat::Tiff => tiff_precision_bits(input).ok(),
        image_io::ImageFormat::Exr => Some(32),
    };
    Ok(json!({
        "path": input,
        "format": format_name(format),
        "width": loaded.image.width,
        "height": loaded.image.height,
        "precisionBits": precision_bits,
        "size": bytes.len(),
        "sha256": format!("{:x}", Sha256::digest(&bytes)),
        "metadata": {"read": loaded.metadata.is_some()},
        "inspection": "read-only",
    }))
}

fn format_name(format: image_io::ImageFormat) -> &'static str {
    match format {
        image_io::ImageFormat::Jpeg => "jpeg",
        image_io::ImageFormat::Png => "png",
        image_io::ImageFormat::Tiff => "tiff",
        image_io::ImageFormat::Exr => "exr",
    }
}

fn tiff_precision_bits(path: &Path) -> Result<u8> {
    let file = fs::File::open(path)?;
    let mut decoder = tiff::decoder::Decoder::new(file)?;
    match decoder.colortype()? {
        tiff::ColorType::RGB(bits) | tiff::ColorType::RGBA(bits) => Ok(bits),
        tiff::ColorType::Gray(bits) | tiff::ColorType::GrayA(bits) => Ok(bits),
        other => bail!("unsupported TIFF color type {other:?}"),
    }
}

fn bytemuck_cast_f64_to_bytes(v: &[f64]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

fn cmd_list_profiles(data_dir: &Path) {
    let profiles_dir = data_dir.join("profiles");
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&profiles_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy().to_string();
            if name.ends_with(".json") {
                names.push(name.trim_end_matches(".json").to_string());
            }
        }
    } else {
        eprintln!("Profile directory not found: {}", profiles_dir.display());
        return;
    }
    names.sort();
    for name in &names {
        // Try to load and show the human-readable name
        if let Ok(p) = profile::load_profile_by_name(data_dir, name) {
            let display_name = p.info.name.as_deref().unwrap_or(name);
            let film_type = &p.info.film_type;
            let support = &p.info.support;
            println!("{name:<40} {display_name:<30} ({film_type}, {support})");
        } else {
            println!("{name}");
        }
    }
}

pub(crate) fn resolve_data_dir(explicit: PathBuf) -> PathBuf {
    let mut candidates = vec![explicit.clone()];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("data"));
            candidates.push(dir.join("..").join("share").join("data"));
            candidates.push(dir.join("..").join("Resources").join("data"));
        }
    }
    candidates.push(PathBuf::from("data"));
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        candidates.push(PathBuf::from(manifest).join("..").join("..").join("data"));
    }
    candidates
        .into_iter()
        .find(|path| path.is_dir())
        .unwrap_or(explicit)
}

fn raw_options(workflow: &WorkflowOptions) -> spektrafilm_raw::RawOptions {
    spektrafilm_raw::RawOptions {
        white_balance: match workflow.raw_white_balance.as_str() {
            "daylight" => spektrafilm_raw::WhiteBalance::Daylight,
            "tungsten" => spektrafilm_raw::WhiteBalance::Tungsten,
            "custom" => spektrafilm_raw::WhiteBalance::Custom,
            _ => spektrafilm_raw::WhiteBalance::AsShot,
        },
        temperature: workflow.raw_temperature,
        tint: workflow.raw_tint,
        lens_correction: workflow.lens_correction,
    }
}

fn validate_raw_options(options: &spektrafilm_raw::RawOptions) -> Result<()> {
    options.validate().map_err(anyhow::Error::from)
}

fn apply_channel_swap(profile: &mut profile::Profile, selection: &str) -> Result<()> {
    if selection == "none" {
        return Ok(());
    }
    let order: Vec<usize> = selection
        .split(',')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()
        .with_context(|| "channel swap must contain three indices such as 2,1,0")?;
    if order.len() != 3 || order.iter().any(|&channel| channel > 2) {
        bail!("channel swap must contain three indices in 0..2");
    }
    for row in &mut profile.data.channel_density {
        if row.len() >= 3 {
            let original = [row[0], row[1], row[2]];
            for channel in 0..3 {
                row[channel] = original[order[channel]];
            }
        }
    }
    Ok(())
}
