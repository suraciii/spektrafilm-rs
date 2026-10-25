use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_cli::{DiscoveryFormat, contract, lut, preset, process, runner, select_data_dir};
use spektrafilm_core::image_io;
use spektrafilm_core::profile;

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
        after_long_help = "Examples:\n  spektrafilm process input.png -o output.tif --preset 'Kodak Portra 400' --input-color-space sRGB --input-cctf-decoding true\n  spektrafilm process input.png -o output.tif --preset 'Kodak Portra 400' --input-color-space sRGB --input-cctf-decoding true --set camera.exposure_compensation_ev=0.5 --seed 42\n  spektrafilm process input.png -o output.tif --preset 'Kodak Portra 400' --input-color-space sRGB --input-cctf-decoding true --dry-run --dry-run-format text\n  spektrafilm preset list --format text\n  spektrafilm list-profiles\n  spektrafilm describe --format text\n  spektrafilm describe --module film_render.grain --format json\n\nRaster source color and transfer state must be declared explicitly; RAW uses linear ACES2065-1 from its decoder. Scanner io.output_color_space controls simulation output; --saving-color-space converts it for the writer. --saving-cctf-encoding controls writer encoding. Output bit depth does not select computation precision. Dry run does not decode pixels or prove device availability."
    )]
    Process(process::ProcessArgs),
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
        /// Save a JSON diagnostics report and live .progress.jsonl sidecar; never replace files.
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
    /// Inspect presets from the built-in group and user library.
    Preset {
        #[command(subcommand)]
        command: preset::PresetCommand,
    },
}

fn main() -> Result<()> {
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
    let debug = matches.subcommand_name() == Some("process") && command_matches.get_flag("debug");
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new(if debug { "debug" } else { "info" })
            }),
        )
        .with_writer(std::io::stderr)
        .init();

    match cli.command {
        Commands::Process(args) => {
            process::run(args, command_matches)?;
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
        "format": format.name(),
        "width": loaded.image.width,
        "height": loaded.image.height,
        "precisionBits": precision_bits,
        "size": bytes.len(),
        "sha256": format!("{:x}", Sha256::digest(&bytes)),
        "metadata": {"read": loaded.metadata.is_some()},
        "inspection": "read-only",
    }))
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

fn cmd_describe(format: DiscoveryFormat, module: Option<&str>, field: Option<&str>) -> Result<()> {
    use spektrafilm_core::params::sources::{
        describe_field, describe_module, describe_root_groups, non_settable_leaves,
    };
    if matches!(format, DiscoveryFormat::Text) && module.is_none() && field.is_none() {
        for group in describe_root_groups() {
            println!("{group}");
        }
        for (path, reason) in non_settable_leaves() {
            println!("{path} (runtime field outside settable parameter surface: {reason})");
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
