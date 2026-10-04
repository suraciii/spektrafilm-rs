mod lut;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use spektrafilm_core::image_io::{self, BitDepth, SaveOptions};
use spektrafilm_core::neutral_filters::NeutralFilters;
use spektrafilm_core::params::{RuntimeParams, Tap};
use spektrafilm_core::params_builder::{digest_params, resize_for_preview};
use spektrafilm_core::pipeline::Pipeline;
use spektrafilm_core::profile;
use spektrafilm_math::image::ImageBuf;

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
        /// Input image (TIFF, EXR, PNG, JPEG, or camera RAW).
        input: PathBuf,
        /// Output image path (TIFF, EXR, PNG, or JPEG).
        #[arg(short, long)]
        output: PathBuf,
        /// Output bit depth: 8, 16, or 32 (PNG/JPEG use 8; EXR uses 16 or 32).
        #[arg(long, default_value = "16")]
        bit_depth: u8,
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
            bit_depth,
            workflow,
            film,
            paper,
            scan_film,
            timings,
            params: params_file,
            raw_out,
            iters,
            data_dir,
        } => {
            let data_dir = resolve_data_dir(data_dir);
            cmd_process(
                &input,
                &output,
                bit_depth,
                &workflow,
                &film,
                paper.as_deref(),
                scan_film,
                timings,
                params_file.as_deref(),
                raw_out.as_deref(),
                iters,
                &data_dir,
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
            lut::export_lut(&film, paper.as_deref(), size, &output, &data_dir)?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_process(
    input: &Path,
    output: &Path,
    bit_depth: u8,
    workflow: &WorkflowOptions,
    film_name: &str,
    paper_name: Option<&str>,
    scan_film: bool,
    show_timings: bool,
    params_file: Option<&Path>,
    raw_out: Option<&Path>,
    iters: usize,
    data_dir: &Path,
) -> Result<()> {
    let total_start = Instant::now();
    let depth = BitDepth::try_from(bit_depth)?;

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
    params
        .validate()
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("invalid params{}", params_file.map(|p| format!(" file {}", p.display())).unwrap_or_default()))?;
    params.io.scan_film = scan_film;

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
    let neutral_db = NeutralFilters::load(data_dir);
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

    // Select backend
    let backend = spektrafilm_gpu::select_backend();
    eprintln!("Backend: {}", backend.name());

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
    let saving_space = workflow.saving_color_space.as_deref().unwrap_or(&output_color_space);
    let saving_encoded = workflow.saving_cctf_encoding.unwrap_or(output_cctf_encoding);
    let saving_image = image_io::convert_image(&result, &output_color_space, output_cctf_encoding, saving_space, saving_encoded)?;
    let report = image_io::save(
        output,
        &saving_image,
        SaveOptions {
            depth,
            color_space: saving_space,
            cctf_encoding: saving_encoded,
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




fn resolve_data_dir(explicit: PathBuf) -> PathBuf {
    let mut candidates = vec![explicit.clone()];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("data"));
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
