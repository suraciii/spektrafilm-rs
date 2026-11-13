//! `spektrafilm-lab`: quantify delivered images, compare, and report.
//!
//! The lab renders nothing. It decodes delivered rasters with the measurement
//! library's reader, runs the shared metric implementations, and publishes
//! immutable comparison and report records. Rendering endpoints live in
//! `spektrafilm-f64`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use spektrafilm_benchmark::{
    Error, common, compare, documents, inspect, local_input, regions, report,
};

mod images;
mod plan;
mod readings_compare;
mod sample;

#[derive(Parser)]
#[command(
    name = "spektrafilm-lab",
    about = "SpektraFilm measurement instrument: analysis, comparison, and reporting at reference precision",
    version = concat!(env!("CARGO_PKG_VERSION"), " (research documents schema_version 1)"),
    after_help = "Quantify two delivered image sets with `spektrafilm-lab compare images --baseline A --candidate B --declaration sRGB:encoded --roi name=x,y,w,h --report --out DIR`."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the authoritative schema for an authored or retained document.
    Schema {
        /// Document kind.
        #[arg(long, value_name = "KIND")]
        kind: String,
    },
    /// Validate document shape and locally decidable dependencies.
    Validate {
        /// Document kind.
        #[arg(
            long,
            value_name = "KIND",
            value_parser = [
                "suite", "target", "policy", "configuration", "reference", "regions",
                "preparation", "stimuli", "material_measurement", "correspondence_review",
                "experiment", "run", "comparison", "review"
            ]
        )]
        kind: String,
        /// JSON document to validate.
        #[arg(long, value_name = "FILE")]
        file: PathBuf,
    },
    /// Generate standalone measurement regions.
    Regions {
        #[command(subcommand)]
        command: RegionsCommand,
    },
    /// Compare two retained or delivered sources.
    Compare {
        #[command(subcommand)]
        command: CompareCommand,
    },
    /// Read retained measurements without recomputing.
    /// Read declared sample points from an image or a retained stage capture.
    Sample {
        #[command(flatten)]
        args: sample::MeasureSampleArgs,
    },
    Inspect {
        /// Retained comparison directory.
        #[arg(long, value_name = "DIR")]
        comparison: PathBuf,
        /// Exact case identities to select; repeats.
        #[arg(long = "case", value_name = "ID")]
        case_ids: Vec<String>,
        /// Exact region identities to select; repeats.
        #[arg(long = "region", value_name = "ID")]
        region_ids: Vec<String>,
        /// Exact metric identities to select; repeats.
        #[arg(long = "metric", value_name = "ID")]
        metrics: Vec<String>,
    },
    /// Build an offline report from one retained comparison.
    Report {
        #[command(subcommand)]
        command: ReportCommand,
    },
    /// A render endpoint that belongs to `spektrafilm-f64`.
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Subcommand)]
enum RegionsCommand {
    /// Expand one finite grid declaration into explicit regions.
    Grid(GridArgs),
}

#[derive(Args)]
struct GridArgs {
    /// Regions document identity.
    #[arg(long, value_name = "ID")]
    id: String,
    /// Number of grid rows.
    #[arg(long, value_name = "N")]
    rows: u64,
    /// Number of grid columns.
    #[arg(long, value_name = "N")]
    columns: u64,
    /// Top-left origin as X,Y.
    #[arg(long, value_name = "X,Y", value_parser = parse_pair)]
    origin: [u64; 2],
    /// Patch size as WIDTHxHEIGHT.
    #[arg(long, value_name = "WIDTHxHEIGHT", value_parser = parse_size)]
    patch_size: [u64; 2],
    /// Gap as X,Y.
    #[arg(long, value_name = "X,Y", value_parser = parse_pair)]
    gap: [u64; 2],
    /// Centered sample size as WIDTHxHEIGHT.
    #[arg(long, value_name = "WIDTHxHEIGHT", value_parser = parse_size)]
    sample: Option<[u64; 2]>,
    /// Coordinate space for generated rectangles.
    #[arg(long, value_name = "SPACE", default_value = "source")]
    space: String,
    /// Region role.
    #[arg(long, value_name = "ROLE", default_value = "other")]
    role: String,
    /// Measurement families, separated by commas.
    #[arg(long, value_name = "FAMILY,...", value_delimiter = ',', action = clap::ArgAction::Append)]
    measurements: Vec<String>,
    /// New regions document JSON file; must not exist.
    #[arg(long, value_name = "FILE")]
    out: PathBuf,
}

#[derive(Subcommand)]
enum CompareCommand {
    /// Quantify two delivered image sets. No renderer is involved.
    Images(images::Args),
    /// Join two retained reading records.
    Readings(readings_compare::MeasureCompareArgs),
    /// Compare two completed image-domain runs.
    Runs(RunComparisonArgs),
    /// Select explicit evidence pairs from one retained run.
    Pairs(PairArgs),
}

#[derive(Args)]
struct RunComparisonArgs {
    /// Baseline retained run directory.
    #[arg(long, value_name = "DIR")]
    baseline: PathBuf,
    /// Candidate retained run directory.
    #[arg(long, value_name = "DIR")]
    candidate: PathBuf,
    /// Policy document.
    #[arg(long, value_name = "FILE")]
    policy: Option<PathBuf>,
    /// Fail the command when the policy does not pass.
    #[arg(long, requires = "policy")]
    gate: bool,
    /// New comparison directory; must not exist.
    #[arg(long, value_name = "DIR")]
    out: PathBuf,
}

#[derive(Args)]
struct PairArgs {
    /// Retained run directory.
    #[arg(long, value_name = "DIR")]
    run: PathBuf,
    /// Pair to select.
    #[arg(long, value_name = "input|reference|both")]
    against: String,
    /// New comparison directory; must not exist.
    #[arg(long, value_name = "DIR")]
    out: PathBuf,
}

#[derive(Subcommand)]
enum ReportCommand {
    /// Present one retained comparison as an offline page.
    Comparison {
        /// Retained comparison directory.
        #[arg(long, value_name = "DIR")]
        comparison: PathBuf,
        /// New report directory; must not exist.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },
}

fn parse_pair(value: &str) -> Result<[u64; 2], String> {
    let mut parts = value.split(',');
    let first = parts
        .next()
        .ok_or_else(|| "expected X,Y".to_owned())?
        .parse::<u64>()
        .map_err(|_| format!("invalid pair {value:?}; expected X,Y"))?;
    let second = parts
        .next()
        .ok_or_else(|| "expected X,Y".to_owned())?
        .parse::<u64>()
        .map_err(|_| format!("invalid pair {value:?}; expected X,Y"))?;
    if parts.next().is_some() {
        return Err(format!("invalid pair {value:?}; expected X,Y"));
    }
    Ok([first, second])
}

fn parse_size(value: &str) -> Result<[u64; 2], String> {
    let mut parts = value.split(['x', 'X']);
    let width = parts
        .next()
        .ok_or_else(|| "expected WIDTHxHEIGHT".to_owned())?
        .parse::<u64>()
        .map_err(|_| format!("invalid size {value:?}; expected WIDTHxHEIGHT"))?;
    let height = parts
        .next()
        .ok_or_else(|| "expected WIDTHxHEIGHT".to_owned())?
        .parse::<u64>()
        .map_err(|_| format!("invalid size {value:?}; expected WIDTHxHEIGHT"))?;
    if parts.next().is_some() {
        return Err(format!("invalid size {value:?}; expected WIDTHxHEIGHT"));
    }
    Ok([width, height])
}

fn validate(
    kind: &str,
    file: &std::path::Path,
) -> spektrafilm_benchmark::Result<serde_json::Value> {
    let display = file.display().to_string();
    let value = if matches!(kind, "run" | "comparison") {
        let root = file.parent().ok_or_else(|| {
            spektrafilm_benchmark::Error::new("invalid_path", "Manifest has no parent")
        })?;
        documents::load_json_document(file).and_then(|value| {
            documents::validate_generated_document(kind, &value, root)?;
            Ok(value)
        })
    } else {
        match kind {
            "suite" => documents::load_suite(file),
            "target" => documents::load_target(file),
            "policy" => documents::load_policy(file),
            "configuration" | "reference" | "regions" | "preparation" => {
                local_input::load(kind, file)
            }
            "stimuli" | "material_measurement" | "correspondence_review" | "experiment" => {
                documents::validate_authored_document(kind, file)
            }
            _ => Err(spektrafilm_benchmark::Error::new(
                "unknown_document_kind",
                format!("unknown document kind: {kind}"),
            )),
        }
    }
    .map_err(|e| e.file(display))?;
    let unchecked = match kind {
        "policy" => serde_json::json!([
            "selector_applicability",
            "measurement_selection",
            "protocol_binding",
            "rendering",
            "compute_device"
        ]),
        "preparation" => serde_json::json!([
            "reference_derivation",
            "cross_document_applicability",
            "rendering",
            "compute_device"
        ]),
        "run" => serde_json::json!([
            "image_decoding",
            "rendering",
            "compute_device",
            "measurement"
        ]),
        "comparison" => serde_json::json!([
            "image_decoding",
            "rendering",
            "compute_device",
            "measurement",
            "rule_evaluation"
        ]),
        _ => serde_json::json!([
            "rendering",
            "compute_device",
            "cross_document_applicability"
        ]),
    };
    let mut result = serde_json::json!({
        "schema_version": 1,
        "command": "validate",
        "execution_status": "completed",
        "kind": kind,
        "digest": common::digest(&value)?,
        "unchecked": unchecked
    });
    if kind == "suite" {
        result["protocol_digest"] = serde_json::json!(documents::protocol_digest(&value)?);
    }
    Ok(result)
}

/// Endpoints that belong to the research renderer.
fn moved(name: &str) -> Option<String> {
    const MOVED: &[&str] = &[
        "run",
        "measure",
        "experiment",
        "fixtures",
        "target",
        "spectral",
        "fit",
    ];
    MOVED
        .contains(&name)
        .then(|| format!("spektrafilm-f64 {name} ..."))
}

/// Review documents belong to the benchmark's policy surface.
fn benchmark_owned_review() -> spektrafilm_benchmark::Error {
    spektrafilm_benchmark::Error::new(
        "unsupported_document_kind",
        "review documents are benchmark-owned and unsupported by spektrafilm-lab",
    )
}

fn emit(value: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

fn run() -> i32 {
    let cli = Cli::parse();
    if let Commands::External(args) = &cli.command {
        let name = args.first().map(String::as_str).unwrap_or("<unknown>");
        match moved(name) {
            Some(replacement) => eprintln!(
                "argument_syntax: `{name}` belongs to spektrafilm-f64; run `{replacement}`"
            ),
            None => eprintln!("argument_syntax: unknown endpoint `{name}`"),
        }
        return 2;
    }
    let command = match &cli.command {
        Commands::Schema { .. } => "schema",
        Commands::Validate { .. } => "validate",
        Commands::Regions { .. } => "regions",
        Commands::Compare { .. } => "compare",
        Commands::Sample { .. } => "sample",
        Commands::Inspect { .. } => "inspect",
        Commands::Report { .. } => "report",
        Commands::External(_) => "unknown",
    };
    let result: spektrafilm_benchmark::Result<(serde_json::Value, i32)> = match cli.command {
        Commands::Schema { kind } if kind == "review" => Err(benchmark_owned_review()),
        Commands::Schema { kind } => documents::schema_for(&kind).map(|schema| (schema, 0)),
        Commands::Validate { kind, .. } if kind == "review" => Err(benchmark_owned_review()),
        Commands::Validate { kind, file } => documents::normalize_path(&file)
            .and_then(|path| validate(&kind, &path))
            .map(|value| (value, 0)),
        Commands::Regions {
            command: RegionsCommand::Grid(args),
        } => regions::generate_grid(
            &args.id,
            args.rows,
            args.columns,
            args.origin,
            args.patch_size,
            args.gap,
            args.sample,
            &args.space,
            &args.role,
            &args.measurements,
            &args.out,
        )
        .map(|value| (value, 0)),
        Commands::Compare { command } => match command {
            CompareCommand::Images(args) => images::run(&args),
            CompareCommand::Readings(args) => readings_compare::compare_with_result(&args)
                .map_err(|error| Error::with_message("operation_failed", error)),
            CompareCommand::Runs(args) => compare::compare(
                &args.baseline,
                &args.candidate,
                &args.out,
                args.policy.as_deref(),
                args.gate,
            ),
            CompareCommand::Pairs(args) => {
                compare::compare_single(&args.run, &args.against, &args.out)
            }
        },
        Commands::Sample { args } => sample::sample_with_result(&args)
            .map_err(|error| Error::with_message("operation_failed", error)),
        Commands::Inspect {
            comparison,
            case_ids,
            region_ids,
            metrics,
        } => {
            inspect::inspect(&comparison, &case_ids, &region_ids, &metrics).map(|value| (value, 0))
        }
        Commands::Report {
            command: ReportCommand::Comparison { comparison, out },
        } => documents::normalize_path(&out)
            .and_then(|out| report::build_report(&comparison, &out))
            .map(|value| (value, 0)),
        Commands::External(_) => unreachable!("handled before dispatch"),
    };
    match result {
        Ok((value, status)) => {
            emit(&value);
            status
        }
        Err(error) => {
            eprintln!("{error}");
            emit(
                &serde_json::json!({"schema_version":1,"command":command,"execution_status":"failed","errors":[error.to_value()]}),
            );
            // A rejected invocation is an argument-syntax error and must not
            // share the exit status of a failure to execute.
            if error.code == "argument_syntax" {
                2
            } else {
                1
            }
        }
    }
}

fn main() {
    std::process::exit(run())
}
