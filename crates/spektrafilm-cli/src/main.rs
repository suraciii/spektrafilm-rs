mod contract;
mod lut;
mod preset;
mod runner;
mod telemetry;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
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
    #[command(
        after_long_help = "Examples:\n  spektrafilm process input.png -o output.tif --preset classic-kodak-portra-400\n  spektrafilm process input.png -o output.tif --preset classic-kodak-portra-400 --set camera.exposure_compensation_ev=0.5\n  spektrafilm preset list --format text\n  spektrafilm list-profiles\n  spektrafilm describe --format text\n  spektrafilm describe --module film_render.grain --format json\n\nScanner io.output_color_space controls simulation output; --saving-color-space converts it for the writer. --route accepts the routes listed under that option and validates them after parameter sources."
    )]
    Process {
        /// Compute backend; otherwise use SPEKTRAFILM_BACKEND/default selection.
        #[arg(long, value_enum, help_heading = "Execution and diagnostics")]
        backend: Option<Backend>,
        /// Input TIFF, EXR, PNG, JPEG, or camera RAW image.
        #[arg(help_heading = "Input and RAW loading")]
        input: PathBuf,
        /// Destination TIFF, EXR, PNG, or JPEG image.
        #[arg(short, long, help_heading = "Output")]
        output: PathBuf,
        /// Output container; otherwise inferred from the destination extension.
        #[arg(long, value_enum, help_heading = "Output")]
        format: Option<OutputFormat>,
        /// Output sample depth: 8, 16, or 32 bits (JPEG/PNG require 8).
        #[arg(long, help_heading = "Output")]
        bit_depth: Option<u8>,
        /// JPEG quality from 1 to 100; applies only to JPEG.
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=100), help_heading = "Output")]
        jpeg_quality: Option<u8>,
        /// JPEG chroma subsampling; applies only to JPEG.
        #[arg(long, value_enum, help_heading = "Output")]
        jpeg_subsampling: Option<JpegSubsamplingArg>,
        /// TIFF/EXR compression (EXR requires zip).
        #[arg(long, value_enum, help_heading = "Output")]
        compression: Option<CompressionArg>,
        #[command(flatten)]
        workflow: WorkflowOptions,
        /// Film stock ID; discover with list-profiles. Required without --preset.
        #[arg(
            long,
            required_unless_present = "preset",
            conflicts_with = "preset",
            help_heading = "Look and workflow"
        )]
        film: Option<String>,
        /// Print stock ID; otherwise uses film for direct scans or its target_print.
        #[arg(long, conflicts_with = "preset", help_heading = "Look and workflow")]
        paper: Option<String>,
        /// Built-in ID (preset list) or a .toml/.json preset file.
        #[arg(long, conflicts_with_all = ["film", "paper", "params"], help_heading = "Look and workflow")]
        preset: Option<String>,
        /// Scan film directly, skipping printing unless --route overrides it.
        #[arg(long, help_heading = "Look and workflow")]
        scan_film: bool,
        /// Print per-stage timing information to stderr.
        #[arg(long, help_heading = "Execution and diagnostics")]
        timings: bool,
        /// Save a local JSON diagnostics report without replacing an existing file.
        #[arg(long, help_heading = "Execution and diagnostics")]
        diagnostics: Option<PathBuf>,
        /// Collect available GPU pass timings; requires --diagnostics.
        #[arg(
            long,
            requires = "diagnostics",
            help_heading = "Execution and diagnostics"
        )]
        gpu_timings: bool,
        /// Legacy RuntimeParams JSON snapshot used as the parameter baseline.
        #[arg(long, conflicts_with = "preset", help_heading = "Look and workflow")]
        params: Option<PathBuf>,
        /// Apply PATH=VALUE assignments or a sparse .toml/.json file in order.
        /// Strings need no JSON quotes; arrays use [1,2,3]; null clears optional values.
        #[arg(long = "set", value_name = "PATH=VALUE|FILE", action = clap::ArgAction::Append, help_heading = "Look and workflow")]
        sources: Vec<String>,
        /// Validate and print effective configuration without decoding or rendering.
        #[arg(long, help_heading = "Execution and diagnostics")]
        dry_run: bool,
        /// Dump pre-saving output as HxWx3 channel-interleaved f64 values.
        #[arg(long, help_heading = "Output")]
        raw_out: Option<PathBuf>,
        /// Run the pipeline this many times in one process (at least 1).
        #[arg(long, default_value = "1", help_heading = "Execution and diagnostics")]
        iters: usize,
        /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
        #[arg(
            long,
            default_value = "data",
            help_heading = "Execution and diagnostics"
        )]
        data_dir: PathBuf,
    },
    /// List available film and paper profiles.
    ListProfiles {
        /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
        #[arg(long, default_value = "data")]
        data_dir: PathBuf,
    },
    /// Build LUT bundles or list stocks and transport color spaces.
    Lut {
        #[command(subcommand)]
        command: lut::LutCommand,
    },
    /// Export a canonical 1-LUT cube (encoded ProPhoto RGB to sRGB, native headroom).
    ExportLut {
        /// Film stock ID; discover with list-profiles.
        #[arg(long)]
        film: String,
        /// Print stock ID; otherwise use the film's target_print.
        #[arg(long)]
        paper: Option<String>,
        /// LUT cube edge resolution (for example 33 or 65).
        #[arg(long, default_value = "33")]
        size: u32,
        /// Destination .cube file.
        #[arg(short, long)]
        output: PathBuf,
        /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
        #[arg(long, default_value = "data")]
        data_dir: PathBuf,
    },
    /// Describe runtime fields or the machine-readable render contract.
    Describe {
        /// Discovery output format.
        #[arg(long, default_value = "json", value_enum)]
        format: DiscoveryFormat,
        /// Canonical parameter group; discover groups with --format text.
        #[arg(long, conflicts_with = "field")]
        module: Option<String>,
        /// One canonical editable parameter leaf.
        #[arg(long, conflicts_with = "module")]
        field: Option<String>,
    },
    /// Render one structured recipe under the fork runner contract.
    Render {
        /// Input image path.
        #[arg(long)]
        input: PathBuf,
        /// RenderRecipe JSON file.
        #[arg(long)]
        recipe: PathBuf,
        /// Destination image path.
        #[arg(long)]
        output: PathBuf,
        /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
        #[arg(long, default_value = "data")]
        data_dir: PathBuf,
        /// Save one local JSON diagnostics report without replacing an existing file.
        #[arg(long)]
        diagnostics: Option<PathBuf>,
        /// Collect GPU pass timings when available; requires --diagnostics.
        #[arg(long, requires = "diagnostics")]
        gpu_timings: bool,
    },
    /// Inspect an output without processing or modifying it.
    Inspect {
        /// Existing image to inspect.
        #[arg(long)]
        input: PathBuf,
        /// Inspection format (json).
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
        /// Declared parity corpus JSON file.
        #[arg(long)]
        corpus: PathBuf,
        /// Destination parity report JSON file.
        #[arg(long)]
        report: PathBuf,
        /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
        #[arg(long, default_value = "data")]
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

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DiscoveryFormat {
    Json,
    Text,
}

#[derive(clap::Args)]
struct WorkflowOptions {
    /// Writer-stage color conversion; defaults to the scanner color space and
    /// ACES2065-1 for EXR. Names are shared with the scanner registry: discover
    /// them with `describe --field io.output_color_space`.
    #[arg(long, help_heading = "Output")]
    saving_color_space: Option<String>,
    /// Writer-stage transfer-function encoding; EXR requires false, JPEG/PNG true.
    #[arg(long, action = clap::ArgAction::Set, help_heading = "Output")]
    saving_cctf_encoding: Option<bool>,
    /// RAW loading white balance; custom requires --raw-temperature.
    #[arg(long, default_value = "as-shot", value_parser = ["as-shot", "daylight", "tungsten", "custom"], help_heading = "Input and RAW loading")]
    raw_white_balance: String,
    /// Custom RAW white-balance temperature in Kelvin.
    #[arg(
        long,
        allow_negative_numbers = true,
        help_heading = "Input and RAW loading"
    )]
    raw_temperature: Option<f64>,
    /// Green-channel tint multiplier for custom RAW white balance; 1 is neutral.
    /// Requires --raw-white-balance custom.
    #[arg(
        long,
        allow_negative_numbers = true,
        help_heading = "Input and RAW loading"
    )]
    raw_tint: Option<f64>,
    /// Apply lens correction while loading RAW input.
    #[arg(long, help_heading = "Input and RAW loading")]
    lens_correction: bool,
    /// Workflow route; this help lists the supported values.
    #[arg(long, help_heading = "Look and workflow")]
    route: Option<String>,
    /// Film scanner output mode; applied after parameter sources.
    #[arg(long, value_parser = ["direct_scan", "positive_scan"], help_heading = "Look and workflow")]
    scan_output: Option<String>,
    /// Film density channel order: none or three comma-separated indices in 0..2.
    #[arg(long, default_value = "none", help_heading = "Look and workflow")]
    film_channel_swap: String,
    /// Print density channel order: none or three comma-separated indices in 0..2.
    #[arg(long, default_value = "none", help_heading = "Look and workflow")]
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

    static IDENTITY: std::sync::LazyLock<(String, String)> = std::sync::LazyLock::new(|| {
        let name = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "spektrafilm".into());
        let precision = if cfg!(feature = "precision-f64") {
            "f64"
        } else {
            "f32"
        };
        (
            name,
            format!("{} (native CPU {})", env!("CARGO_PKG_VERSION"), precision),
        )
    });
    let (name, version) = &*IDENTITY;
    let route_help = format!(
        "Workflow route; supported values: {}",
        spektrafilm_core::params::sources::supported_routes().join(", ")
    );
    let matches = Cli::command()
        .name(name.as_str())
        .version(version.as_str())
        .mut_subcommand("process", |command| {
            command.mut_arg("route", |arg| {
                arg.help(route_help.clone()).long_help(route_help.clone())
            })
        })
        .get_matches();
    let cli = Cli::from_arg_matches(&matches)?;
    let (_, command_matches) = matches.subcommand().expect("required subcommand");

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
                    let data = select_data_dir(data_dir, command_matches)?;
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
                        &data,
                        backend,
                        invocation,
                    )
                },
            )?;
        }
        Commands::ListProfiles { data_dir } => {
            let data = select_data_dir(data_dir, command_matches)?;
            cmd_list_profiles(&data.path)?;
        }
        Commands::Lut { command } => lut::run(command, command_matches)?,
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
                &select_data_dir(data_dir, command_matches)?.path,
            )?;
        }
        Commands::Describe {
            format,
            module,
            field,
        } => {
            cmd_describe(format, module.as_deref(), field.as_deref())?;
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
            command_matches,
        )?,
        Commands::Inspect { input, format } => {
            if format != "json" {
                bail!("unsupported inspect format {format}; use json");
            }
            println!("{}", serde_json::to_string_pretty(&inspect(&input)?)?);
        }
        Commands::Preset { command } => preset::run(command, command_matches)?,
        Commands::Parity {
            corpus,
            report,
            data_dir,
        } => runner::cmd_parity(
            &corpus,
            &report,
            &select_data_dir(data_dir, command_matches)?.path,
        )?,
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
    data: &DataSelection,
    backend_choice: Option<Backend>,
    invocation: &mut telemetry::Invocation,
) -> Result<()> {
    let data_dir = &data.path;
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
    let raw_options = raw_options(workflow);
    if input_is_raw {
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
    if dry_run {
        let backend_env = std::env::var("SPEKTRAFILM_BACKEND").ok();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "film_profile": resolved.film_name,
                "print_profile": resolved.print_name,
                "parameters": params,
                "data": {"path": data.path, "source": data.source},
                "input": {
                    "kind": if input_is_raw { "raw" } else { "raster" },
                    "raw_loading": input_is_raw.then(|| json!({
                        "white_balance": workflow.raw_white_balance,
                        "temperature": raw_options.temperature,
                        "tint": raw_options.tint,
                        "lens_correction": raw_options.lens_correction
                    }))
                },
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
                let options = &raw_options;
                (
                    spektrafilm_raw::load(input, options)
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

fn cmd_list_profiles(data_dir: &Path) -> Result<()> {
    let profiles_dir = data_dir.join("profiles");
    let mut names = Vec::new();
    for entry in fs::read_dir(&profiles_dir)
        .with_context(|| format!("reading profile directory: {}", profiles_dir.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            names.push(
                path.file_stem()
                    .context("profile has no filename")?
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    names.sort();
    for name in &names {
        let p = profile::load_profile_by_name(data_dir, name).with_context(|| {
            format!(
                "loading profile: {}",
                profiles_dir.join(format!("{name}.json")).display()
            )
        })?;
        let display_name = p.info.name.as_deref().unwrap_or(name);
        let film_type = &p.info.film_type;
        let support = &p.info.support;
        println!("{name:<40} {display_name:<30} ({film_type}, {support})");
    }
    Ok(())
}

pub(crate) struct DataSelection {
    path: PathBuf,
    source: &'static str,
}

pub(crate) fn explicit_data_selection(matches: &ArgMatches) -> bool {
    matches.value_source("data_dir") == Some(clap::parser::ValueSource::CommandLine)
        || std::env::var_os("SPEKTRAFILM_DATA_DIR").is_some()
}

pub(crate) fn select_data_dir(argument: PathBuf, matches: &ArgMatches) -> Result<DataSelection> {
    let selection = if matches.value_source("data_dir")
        == Some(clap::parser::ValueSource::CommandLine)
    {
        Some((argument, "argument", "--data-dir"))
    } else if let Some(value) = std::env::var_os("SPEKTRAFILM_DATA_DIR") {
        if value.is_empty() {
            bail!("SPEKTRAFILM_DATA_DIR is empty; set it to a data directory or supply --data-dir");
        }
        Some((PathBuf::from(value), "environment", "SPEKTRAFILM_DATA_DIR"))
    } else {
        None
    };
    if let Some((path, source, option)) = selection {
        if !path.is_dir() {
            bail!(
                "{option} selected {} which is not an existing directory; supply a valid data directory",
                path.display()
            );
        }
        return Ok(DataSelection {
            path: fs::canonicalize(&path)
                .with_context(|| format!("resolving data directory: {}", path.display()))?,
            source,
        });
    }
    let executable = std::env::current_exe().context("locating executable for data discovery")?;
    let directory = executable
        .parent()
        .context("executable has no parent directory")?;
    let candidates = [
        directory.join("data"),
        directory.join("../share/data"),
        directory.join("../Resources/data"),
        std::env::current_dir()?.join("data"),
    ];
    for path in &candidates {
        if path.is_dir() {
            return Ok(DataSelection {
                path: fs::canonicalize(path)
                    .with_context(|| format!("resolving data directory: {}", path.display()))?,
                source: "automatic",
            });
        }
    }
    bail!(
        "no data directory found; tried {}; set --data-dir or SPEKTRAFILM_DATA_DIR",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

fn cmd_describe(format: DiscoveryFormat, module: Option<&str>, field: Option<&str>) -> Result<()> {
    use spektrafilm_core::params::sources::{
        describe_field, describe_module, describe_root_groups,
    };
    if matches!(format, DiscoveryFormat::Text) && module.is_none() && field.is_none() {
        for group in describe_root_groups() {
            println!("{group}");
        }
        return Ok(());
    }
    let value = if let Some(field) = field {
        describe_field(field).map_err(anyhow::Error::msg)?
    } else if let Some(module) = module {
        describe_module(module).map_err(anyhow::Error::msg)?
    } else {
        contract::describe()
    };
    match format {
        DiscoveryFormat::Json => println!("{}", serde_json::to_string_pretty(&value)?),
        DiscoveryFormat::Text => {
            if field.is_some() {
                print_metadata_record(&value);
            } else {
                let mut fields = value["fields"]
                    .as_array()
                    .context("module description has no fields")?
                    .iter()
                    .collect::<Vec<_>>();
                fields.sort_by_key(|record| record["path"].as_str().unwrap_or(""));
                for record in fields {
                    print_metadata_record(record);
                }
            }
        }
    }
    Ok(())
}

fn print_metadata_record(record: &Value) {
    if let Some(fields) = record.as_object() {
        for (key, value) in fields {
            println!("{key}: {value}");
        }
        println!();
    }
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
