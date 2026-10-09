mod contract;
mod lut;

use std::path::{Path, PathBuf};
use std::{fs, io::Write};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_core::image_io::{
    self, BitDepth, Compression, JpegSubsampling, SaveOptions,
};
use spektrafilm_core::neutral_filters::NeutralFilters;
use spektrafilm_core::params::{RuntimeParams, Tap};
use spektrafilm_core::params_builder::{digest_params, resize_for_preview};
use spektrafilm_core::pipeline::Pipeline;
use spektrafilm_core::profile;
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
        /// Film stock name (e.g. kodak_portra_400).
        #[arg(long)]
        film: String,
        /// Paper stock name (e.g. fujifilm_crystal_archive_typeii).
        /// If omitted and --scan-film is not set, uses the film's target_print.
        #[arg(long)]
        paper: Option<String>,
        /// Scan film directly (skip printing stage).
        #[arg(long)]
        scan_film: bool,
        /// Print per-stage timing information.
        #[arg(long)]
        timings: bool,
        /// Path to JSON params file for overrides.
        #[arg(long)]
        params: Option<PathBuf>,
        /// Dump the raw f64 output buffer (HxWx3, row-major, channel-interleaved)
        /// before sRGB encoding/clipping. Used for bit-exact parity comparison.
        #[arg(long)]
        raw_out: Option<PathBuf>,
        /// Run the pipeline N times in the same process. Each iteration is
        /// timed individually so you can see cold-start vs warm-cache speed.
        /// Default: 1 (no repetition).
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
    },
    /// Inspect an output without processing or modifying it.
    Inspect {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value = "json")]
        format: String,
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
            scan_film,
            timings,
            params: params_file,
            raw_out,
            iters,
            data_dir,
            backend,
        } => {
            let data_dir = resolve_data_dir(data_dir);
            cmd_process(
                &input,
                &output,
                format,
                bit_depth,
                jpeg_quality,
                jpeg_subsampling,
                compression,
                &workflow,
                &film,
                paper.as_deref(),
                scan_film,
                timings,
                params_file.as_deref(),
                raw_out.as_deref(),
                iters,
                &data_dir,
                backend,
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
            lut::export_lut(&film, paper.as_deref(), size, &output, &resolve_data_dir(data_dir))?;
        }
        Commands::Describe { format } => {
            if format != "json" {
                bail!("unsupported describe format {format}; use json");
            }
            println!("{}", serde_json::to_string_pretty(&contract::describe())?);
        }
        Commands::Render {
            input,
            recipe,
            output,
            data_dir,
        } => cmd_render(&input, &recipe, &output, &data_dir)?,
        Commands::Inspect { input, format } => {
            if format != "json" {
                bail!("unsupported inspect format {format}; use json");
            }
            println!("{}", serde_json::to_string_pretty(&inspect(&input)?)?);
        }
        Commands::Parity {
            corpus,
            report,
            data_dir,
        } => cmd_parity(&corpus, &report, &data_dir)?,

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
    film_name: &str,
    paper_name: Option<&str>,
    scan_film: bool,
    show_timings: bool,
    params_file: Option<&Path>,
    raw_out: Option<&Path>,
    iters: usize,
    data_dir: &Path,
    backend_choice: Option<Backend>,
) -> Result<()> {
    let total_start = Instant::now();
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
    if matches!(output_format, OutputFormat::Jpeg | OutputFormat::Png)
        && depth != BitDepth::Eight
    {
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
    let jpeg_quality = matches!(output_format, OutputFormat::Jpeg).then(|| jpeg_quality.unwrap_or(95));
    let jpeg_subsampling = matches!(output_format, OutputFormat::Jpeg)
        .then(|| jpeg_subsampling.unwrap_or(JpegSubsamplingArg::Yuv444).core());
    let compression = matches!(output_format, OutputFormat::Tiff | OutputFormat::Exr)
        .then(|| compression.unwrap_or(CompressionArg::Zip).core());
    let backend: Box<dyn spektrafilm_gpu::ComputeBackend> = match backend_choice {
        Some(Backend::Cpu) => Box::new(spektrafilm_gpu::cpu_backend::CpuBackend),
        Some(Backend::Gpu) => Box::new(
            spektrafilm_gpu::wgpu_backend::WgpuBackend::new().ok_or_else(|| {
                anyhow::anyhow!(
                    "GPU backend unavailable: no compatible WGPU adapter/device was found; \
                     check your graphics drivers or use --backend cpu"
                )
            })?,
        ),
        None => spektrafilm_gpu::select_backend(),
    };
    eprintln!("Backend: {}", backend.name());

    // Load profiles
    let t = Instant::now();
    let mut film = profile::load_profile_by_name(data_dir, film_name)
        .with_context(|| format!("loading film profile: {film_name}"))?;

    let print_stock = if scan_film {
        film_name.to_string()
    } else if let Some(p) = paper_name {
        p.to_string()
    } else if let Some(ref target) = film.info.target_print {
        target.clone()
    } else {
        bail!("no paper specified and film has no target_print — use --paper or --scan-film");
    };

    let mut print = profile::load_profile_by_name(data_dir, &print_stock)
        .with_context(|| format!("loading print profile: {print_stock}"))?;
    apply_channel_swap(&mut film, &workflow.film_channel_swap)?;
    apply_channel_swap(&mut print, &workflow.print_channel_swap)?;
    eprintln!("Profiles loaded: {} ms", t.elapsed().as_millis());

    // Load params (defaults + optional overrides). Strict: unknown fields,
    // unsupported algorithms/color spaces/filter families and unknown taps
    // fail here — before any artifact is produced.
    let mut params = if let Some(pf) = params_file {
        let f = std::fs::File::open(pf)
            .with_context(|| format!("opening params file: {}", pf.display()))?;
        serde_json::from_reader(std::io::BufReader::new(f))
            .with_context(|| format!("parsing params file {}", pf.display()))?
    } else {
        RuntimeParams::default()
    };
    let route = workflow
        .route
        .clone()
        .or_else(|| scan_film.then_some("input > film > scan".into()))
        .unwrap_or_else(|| params.workflow.route.clone());
    params.workflow.route = route;
    params.io.scan_film = scan_film;
    params
        .validate()
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("invalid params{}", params_file.map(|p| format!(" file {}", p.display())).unwrap_or_default()))?;

    // RAW supplies linear ACES; prepared images retain their samples.
    let input_is_raw = image_io::is_raw(input);
    if input_is_raw {
        params.io.input_color_space = "ACES2065-1".into();
        params.io.input_cctf_decoding = false;
    }
    params.validate_color().map_err(anyhow::Error::msg)?;

    // Digest to the static runtime form (0.3.4 order: database neutral
    // filters, preview deactivation, stock-specific overrides, debug
    // switches). `new_with_spectral` re-applies the idempotent parts.
    let neutral_db = NeutralFilters::load(data_dir).map_err(anyhow::Error::msg)?;
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
    let params = digest_params(params, &film, &print, Some(&neutral_db), true);

    let t = Instant::now();
    let (image, metadata) = if input_is_raw {
        let options = spektrafilm_raw::RawOptions {
            white_balance: match workflow.raw_white_balance.as_str() {
                "daylight" => spektrafilm_raw::WhiteBalance::Daylight,
                "tungsten" => spektrafilm_raw::WhiteBalance::Tungsten,
                "custom" => spektrafilm_raw::WhiteBalance::Custom,
                _ => spektrafilm_raw::WhiteBalance::AsShot,
            },
            temperature: workflow.raw_temperature, tint: workflow.raw_tint,
            lens_correction: workflow.lens_correction,
        };
        (spektrafilm_raw::load(input, &options).map_err(anyhow::Error::msg)?.image, image_io::read_metadata(input))
    } else {
        let loaded = image_io::load(input)
            .with_context(|| format!("loading image: {}", input.display()))?;
        (loaded.image, loaded.metadata)
    };
    // Preview mode: bound the long edge before processing (upstream
    // `simulate_preview` + `resize_for_preview`).
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
    eprintln!(
        "Image loaded: {}x{} ({} MP), {} ms",
        image.width,
        image.height,
        (image.pixel_count() as f64 / 1e6 * 10.0).round() / 10.0,
        t.elapsed().as_millis()
    );

    // Run pipeline — full Hanatos2025 spectral upsampling, no simplified
    // fallback: the identity front end is not a calibrated substitute, so
    // a missing LUT is a hard error.
    let t = Instant::now();
    let output_color_space = params.io.output_color_space.clone();
    let output_cctf_encoding = params.io.output_cctf_encoding;
    let pipeline = Pipeline::new_with_spectral(film, print, params, data_dir).map_err(|e| {
        anyhow::anyhow!(
            "spectral pipeline construction failed: {e} — the simplified no-LUT fallback was \
             removed; check that the profiles/data directory contains the spectral LUTs \
             (looked under {})",
             data_dir.display()
        )
    })?;
    let run_once = |image: ImageBuf| -> Result<ImageBuf> {
        let out = if inject.is_none() && collect.is_none() {
            pipeline.process(image, backend.as_ref()).map_err(anyhow::Error::msg)?
        } else {
            pipeline
                .process_with_taps(image, backend.as_ref(), inject, collect)
                .map_err(|e| anyhow::anyhow!("pipeline taps: {e}"))?
        };
        Ok(out)
    };
    let result = if iters > 1 {
        // Warm-cache bench: process the image `iters` times in the same backend
        // instance. The first iteration pays shader-compile cost; subsequent ones
        // hit the cache and reflect "GUI live preview" performance.
        let mut last = run_once(image.clone())?;
        let iter1_ms = t.elapsed().as_millis();
        eprintln!("Pipeline (iter 1): {} ms (cold)", iter1_ms);
        for i in 2..=iters {
            let ti = Instant::now();
            last = run_once(image.clone())?;
            eprintln!(
                "Pipeline (iter {}): {} ms (warm)",
                i,
                ti.elapsed().as_millis()
            );
        }
        last
    } else {
        let r = run_once(image)?;
        eprintln!("Pipeline: {} ms", t.elapsed().as_millis());
        r
    };

    // Save output
    let t = Instant::now();
    let saving_space = workflow.saving_color_space.as_deref().unwrap_or(
        if output_format == OutputFormat::Exr { "ACES2065-1" } else { &output_color_space }
    );
    let saving_encoded = workflow.saving_cctf_encoding.unwrap_or(output_format != OutputFormat::Exr && output_cctf_encoding);
    if matches!(output_format, OutputFormat::Jpeg | OutputFormat::Png) && !saving_encoded {
        bail!("{} output requires encoded saving output", output_format.name());
    }
    if output_format == OutputFormat::Exr && saving_encoded {
        bail!("EXR output requires linear saving output");
    }
    let saving_image = image_io::convert_image(
        &result,
        &output_color_space,
        output_cctf_encoding,
        saving_space,
        saving_encoded,
    )?;
    let report = image_io::save(
        output,
        &saving_image,
        SaveOptions {
            depth,
            color_space: saving_space,
            cctf_encoding: saving_encoded,
            jpeg_quality,
            jpeg_subsampling,
            compression,
        },
        metadata.as_ref(),
    )
    .with_context(|| format!("saving image: {}", output.display()))?;
    if let Some(warning) = report.metadata_warning {
        eprintln!("Warning: {warning}");
    }
    eprintln!(
        "Saved: {} ({}x{}), {} ms",
        output.display(),
        result.width,
        result.height,
        t.elapsed().as_millis()
    );

    // Optional raw f64 dump for bit-exact parity comparison.
    if let Some(p) = raw_out {
        use std::io::Write;
        let buf: Vec<f64> = result.data.iter().map(|&v| v as f64).collect();
        let bytes = bytemuck_cast_f64_to_bytes(&buf);
        let mut f = std::fs::File::create(p)
            .with_context(|| format!("creating raw_out file: {}", p.display()))?;
        f.write_all(bytes)?;
        eprintln!("raw f64 buffer: {} ({} values)", p.display(), buf.len());
    }

    if show_timings {
        eprintln!("Total: {} ms", total_start.elapsed().as_millis());
    }

    Ok(())
}

fn cmd_render(input: &Path, recipe_path: &Path, output: &Path, data_dir: &Path) -> Result<()> {
    if output.exists() {
        bail!("refusing to replace existing output {}", output.display());
    }
    if !input.is_file() {
        bail!("input is not a regular file: {}", input.display());
    }
    let recipe = contract::read_recipe(recipe_path)?;
    let facts = render_recipe(input, &recipe, output, data_dir)?;
    println!("{}", serde_json::to_string_pretty(&facts)?);
    Ok(())
}

fn render_recipe(
    input: &Path,
    recipe: &contract::RenderRecipe,
    output: &Path,
    data_dir: &Path,
) -> Result<Value> {
    contract::validate_output(&recipe.output)?;
    contract::validate_output_path(output, &recipe.output)?;
    contract::validate_input_path(input)?;
    if let Some(seed) = recipe.seed {
        contract::validate_seed(seed)?;
    }
    let mut params = contract::normalize_parameters(recipe.parameters.clone())?;
    params.random_seed = recipe.seed.unwrap_or(0);
    if recipe.output.format == "png" {
        params.settings.preview_mode = true;
        params.settings.preview_max_size = recipe.output.max_edge.unwrap_or(640);
    }
    params.io.input_color_space = "ProPhoto RGB".into();
    params.io.input_cctf_decoding = false;
    params.io.output_color_space = "sRGB".into();
    params.io.output_cctf_encoding = true;
    params.io.scan_film = false;
    params.validate_color().map_err(anyhow::Error::msg)?;
    let film = profile::load_profile_by_name(data_dir, &recipe.film_profile)
        .with_context(|| format!("loading film profile {}", recipe.film_profile))?;
    let print_name = recipe
        .print_profile
        .as_deref()
        .or(film.info.target_print.as_deref())
        .ok_or_else(|| anyhow::anyhow!("recipe does not select a print profile"))?;
    let print = profile::load_profile_by_name(data_dir, print_name)
        .with_context(|| format!("loading print profile {print_name}"))?;
    let neutral_db = NeutralFilters::load(data_dir).map_err(anyhow::Error::msg)?;
    let params = digest_params(params, &film, &print, Some(&neutral_db), true);
    let loaded = image_io::load(input)
        .with_context(|| format!("loading staged input {}", input.display()))?;
    contract::validate_input_dimensions(loaded.image.width, loaded.image.height)?;
    let input_size = [loaded.image.width, loaded.image.height];
    let image = if recipe.output.format == "png" {
        resize_for_preview(&loaded.image, params.settings.preview_max_size)
    } else {
        loaded.image
    };
    let backend = spektrafilm_gpu::select_backend();
    let pipeline = Pipeline::new_with_spectral(film, print, params, data_dir)
        .map_err(anyhow::Error::msg)
        .context("building spektrafilm-rs spectral pipeline")?;
    let result = pipeline
        .process(image, backend.as_ref())
        .map_err(anyhow::Error::msg)
        .context("running spektrafilm-rs pipeline")?;
    let parent = output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("output has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let extension = output.extension().and_then(|value| value.to_str()).unwrap_or("tmp");
    let temporary = parent.join(format!(
        ".spektrafilm-{}-{}.{}",
        std::process::id(),
        format!("{:x}", Sha256::digest(output.as_os_str().as_encoded_bytes())),
        extension
    ));
    if temporary.exists() {
        bail!("temporary output already exists: {}", temporary.display());
    }
    if let Err(error) = image_io::save_jpeg_quality(
        &temporary,
        &result,
        SaveOptions {
            depth: BitDepth::Eight,
            color_space: "sRGB",
            cctf_encoding: true,
            jpeg_quality: None,
            jpeg_subsampling: Some(spektrafilm_core::image_io::JpegSubsampling::Yuv444),
            compression: None,
        },
        loaded.metadata.as_ref(),
        contract::FINISHED_JPEG_QUALITY,
    ) {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("writing spektrafilm-rs output");
    }
    if let Err(error) = fs::rename(&temporary, output) {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("committing spektrafilm-rs output");
    }
    let bytes = fs::read(output)?;
    Ok(json!({
        "implementation": contract::IMPLEMENTATION,
        "adapterVersion": contract::ADAPTER_VERSION,
        "parameterSchemaVersion": contract::PARAMETER_SCHEMA_VERSION,
        "parameterDigest": contract::parameter_digest(&recipe.parameters),
        "input": {"path": input, "width": input_size[0], "height": input_size[1]},
        "output": {
            "path": output,
            "format": recipe.output.format,
            "width": result.width,
            "height": result.height,
            "size": bytes.len(),
            "sha256": format!("{:x}", Sha256::digest(&bytes)),
        },
        "seed": recipe.seed,
        "resource": {"backend": backend.name()},
        "terminalOutcome": "succeeded",
    }))
}

fn inspect(input: &Path) -> Result<Value> {
    if !input.is_file() {
        bail!("input is not a regular file: {}", input.display());
    }
    let bytes = fs::read(input)?;
    let loaded = image_io::load(input)
        .with_context(|| format!("inspecting image {}", input.display()))?;
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

fn cmd_parity(corpus: &Path, report: &Path, data_dir: &Path) -> Result<()> {
    if report.exists() {
        bail!("refusing to replace existing report {}", report.display());
    }
    let base = if corpus.is_dir() {
        corpus
    } else {
        corpus.parent().unwrap_or_else(|| Path::new("."))
    };
    let documents = if corpus.is_file() {
        vec![(corpus.to_path_buf(), serde_json::from_slice::<Value>(&fs::read(corpus)?)?)]
    } else if corpus.is_dir() {
        let mut paths = fs::read_dir(corpus)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .into_iter()
            .map(|path| Ok((path.clone(), serde_json::from_slice::<Value>(&fs::read(&path)?)?)))
            .collect::<Result<Vec<_>>>()?
    } else {
        bail!("corpus is neither a file nor a directory: {}", corpus.display());
    };
    let mut cases = Vec::new();
    for (source, document) in documents {
        let entries = document.as_array().cloned().unwrap_or_else(|| vec![document]);
        for (index, entry) in entries.into_iter().enumerate() {
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("case");
            let result = run_parity_case(
                base,
                &entry,
                data_dir,
                report,
                cases.len() + index,
            );
            let case = match result {
                Ok(case) => case,
                Err(error) => json!({
                    "name": name,
                    "source": source,
                    "status": "failed",
                    "passed": false,
                    "failure": error.to_string(),
                }),
            };
            cases.push(case);
        }
    }
    let passed = !cases.is_empty()
        && cases.iter().all(|case| case["passed"].as_bool() == Some(true));
    let document = json!({
        "implementation": contract::IMPLEMENTATION,
        "adapterVersion": contract::ADAPTER_VERSION,
        "parameterSchemaVersion": contract::PARAMETER_SCHEMA_VERSION,
        "corpus": corpus,
        "passed": passed,
        "status": if passed { "passed" } else { "failed" },
        "cases": cases,
        "dataDir": data_dir,
    });
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(report)?;
    file.write_all(serde_json::to_string_pretty(&document)?.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn run_parity_case(
    base: &Path,
    entry: &Value,
    data_dir: &Path,
    report: &Path,
    index: usize,
) -> Result<Value> {
    let input = resolve_case_path(base, entry.get("input"))?;
    let recipe = resolve_case_path(base, entry.get("recipe"))?;
    let input_digest = format!("{:x}", Sha256::digest(fs::read(&input)?));
    let recipe_bytes = fs::read(&recipe)?;
    let recipe_digest = format!("{:x}", Sha256::digest(&recipe_bytes));
    let recipe_value: contract::RenderRecipe = serde_json::from_slice(&recipe_bytes)?;
    let extension = if recipe_value.output.format == "jpeg" {
        "jpg"
    } else {
        "png"
    };
    let output = report.with_file_name(format!(
        ".spektrafilm-parity-{}-{index}.{extension}",
        std::process::id()
    ));
    if output.exists() {
        bail!("parity output already exists: {}", output.display());
    }
    let render = render_recipe(&input, &recipe_value, &output, data_dir);
    let result = match render {
        Ok(facts) => {
            let reference_path = entry
                .get("reference")
                .map(|reference| resolve_case_path(base, Some(reference)))
                .transpose()?;
            let reference_digest = if let Some(path) = &reference_path {
                Some(format!("{:x}", Sha256::digest(fs::read(path)?)))
            } else {
                entry
                    .get("referenceSha256")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            };
            let image_diff = if let Some(path) = &reference_path {
                Some(compare_images(
                    path,
                    &output,
                    entry
                        .get("pixelTolerance")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.02),
                )?)
            } else {
                None
            };
            let passed = if let Some(diff) = &image_diff {
                diff["dimensionsMatch"].as_bool() == Some(true)
                    && diff["maxAbs"].as_f64().is_some_and(|value| {
                        value <= entry
                            .get("pixelTolerance")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.02)
                    })
            } else {
                reference_digest == facts["output"]["sha256"].as_str().map(ToOwned::to_owned)
            };
            json!({
                "name": entry.get("name").and_then(Value::as_str).unwrap_or("case"),
                "status": if passed { "passed" } else if reference_digest.is_some() { "mismatch" } else { "unqualified" },
                "passed": passed,
                "inputSha256": input_digest,
                "recipeSha256": recipe_digest,
                "referenceSha256": reference_digest,
                "imageDiff": image_diff,
                "output": facts["output"],
                "resource": facts["resource"],
            })
        }
        Err(error) => json!({
            "name": entry.get("name").and_then(Value::as_str).unwrap_or("case"),
            "status": "failed",
            "passed": false,
            "inputSha256": input_digest,
            "recipeSha256": recipe_digest,
            "failure": error.to_string(),
        }),
    };
    let _ = fs::remove_file(&output);
    Ok(result)
}

fn resolve_case_path(base: &Path, value: Option<&Value>) -> Result<PathBuf> {
    let value = value
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("parity case requires a string path"))?;
    let path = PathBuf::from(value);
    Ok(if path.is_absolute() { path } else { base.join(path) })
}

fn compare_images(reference: &Path, output: &Path, tolerance: f64) -> Result<Value> {
    let reference_image = image_io::load(reference)
        .with_context(|| format!("loading parity reference {}", reference.display()))?;
    let output_image = image_io::load(output)
        .with_context(|| format!("loading parity output {}", output.display()))?;
    if reference_image.image.data.len() != output_image.image.data.len() {
        return Ok(json!({
            "dimensionsMatch": false,
            "maxAbs": null,
            "rmse": null,
            "tolerance": tolerance,
        }));
    }
    let mut max_abs: f64 = 0.0;
    let mut sum_squared: f64 = 0.0;
    for (reference, output) in reference_image
        .image
        .data
        .iter()
        .zip(&output_image.image.data)
    {
        let delta = *reference as f64 - *output as f64;
        max_abs = max_abs.max(delta.abs());
        sum_squared += delta * delta;
    }
    let count = reference_image.image.data.len() as f64;
    Ok(json!({
        "dimensionsMatch": reference_image.image.width == output_image.image.width
            && reference_image.image.height == output_image.image.height,
        "maxAbs": max_abs,
        "rmse": (sum_squared / count).sqrt(),
        "tolerance": tolerance,
    }))
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
    candidates.into_iter().find(|path| path.is_dir()).unwrap_or(explicit)
}

fn apply_channel_swap(profile: &mut profile::Profile, selection: &str) -> Result<()> {
    if selection == "none" { return Ok(()); }
    let order: Vec<usize> = selection.split(',').map(str::parse).collect::<std::result::Result<_, _>>()
        .with_context(|| "channel swap must contain three indices such as 2,1,0")?;
    if order.len() != 3 || order.iter().any(|&channel| channel > 2) {
        bail!("channel swap must contain three indices in 0..2");
    }
    for row in &mut profile.data.channel_density {
        if row.len() >= 3 {
            let original = [row[0], row[1], row[2]];
            for channel in 0..3 { row[channel] = original[order[channel]]; }
        }
    }
    Ok(())
}
