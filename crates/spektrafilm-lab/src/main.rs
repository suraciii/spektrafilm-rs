//! `spektrafilm-lab`: quantify the color science of delivered images.
//!
//! The lab renders nothing and presents nothing. It decodes delivered rasters
//! with the measurement library's reader, runs the shared metric
//! implementations, and publishes immutable analysis and comparison records.
//! Rendering endpoints live in `spektrafilm-f64`; evaluation and presentation
//! live in `spektrafilm-bench`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use spektrafilm_benchmark::{documents, local_input, regions};

mod images;

#[derive(Parser)]
#[command(
    name = "spektrafilm-lab",
    about = "SpektraFilm measurement instrument: color science of delivered images, at reference precision",
    max_term_width = 100,
    version = concat!(env!("CARGO_PKG_VERSION"), " (research documents schema_version 1)"),
    after_help = "Quantify one delivered image with \n  \
                  spektrafilm-lab analyze --source IMAGE --declaration sRGB:encoded --out DIR\n\
                  \n\
                  Quantify how two differ, and measure noise on a flat field, with \n  \
                  spektrafilm-lab compare --baseline A --candidate B --declaration sRGB:encoded --roi name=0,0,64,64:flat_field --out DIR"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Quantify the color science of one delivered image set.
    Analyze(images::AnalyzeArgs),
    /// Quantify how two delivered image sets differ.
    Compare(images::Args),
    /// Expand one finite grid declaration into explicit regions.
    Regions(GridArgs),
    /// Print the authoritative schema for an authored or retained document.
    Schema {
        /// Document kind.
        #[arg(long, value_name = "KIND", value_parser = documents::schema_kinds())]
        kind: String,
    },
    /// Validate document shape and locally decidable dependencies.
    Validate {
        /// Document kind.
        #[arg(long, value_name = "KIND", value_parser = documents::validatable_kinds())]
        kind: String,
        /// JSON document to validate.
        #[arg(long, value_name = "FILE")]
        file: PathBuf,
    },
    /// An endpoint that belongs to another binary.
    #[command(external_subcommand)]
    External(Vec<String>),
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
    /// Required: a generator does not decide which pixels a rectangle
    /// addresses. `render` is what the lab measures; other spaces address a
    /// stage capture and belong to `spektrafilm-bench sample`.
    #[arg(long, value_name = "SPACE")]
    space: String,
    /// What the generated regions are populations of. Required: a generator
    /// does not decide what a region measures.
    #[arg(
        long,
        value_name = "ROLE",
        value_parser = [
            "skin", "neutral", "shadow", "highlight", "color",
            "flat_field", "texture", "edge", "other"
        ]
    )]
    role: String,
    /// New regions document JSON file; must not exist.
    #[arg(long, value_name = "FILE")]
    out: PathBuf,
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
    let value = if matches!(kind, "run" | "comparison" | "analysis") {
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
        "analysis" => serde_json::json!([
            "image_decoding",
            "rendering",
            "compute_device",
            "measurement"
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
        "digest": spektrafilm_benchmark::common::digest(&value)?,
        "unchecked": unchecked
    });
    if kind == "suite" {
        result["protocol_digest"] = serde_json::json!(documents::protocol_digest(&value)?);
    }
    Ok(result)
}

/// The endpoints this binary owns, in help order.
const OWNED: &[&str] = &["analyze", "compare", "regions", "schema", "validate"];

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

/// Endpoints that belong to the benchmark's evaluation surface.
fn evaluated(name: &str) -> Option<String> {
    const EVALUATED: &[&str] = &["inspect", "report", "sample"];
    if EVALUATED.contains(&name) {
        return Some(format!("spektrafilm-bench {name} ..."));
    }
    None
}

/// Report a usage error in the record shape every other refusal uses.
///
/// A bad `--kind` is a question about the document vocabulary, so it carries
/// that fact's own code; every other usage error is `argument_syntax`.
fn usage(error: clap::Error) -> i32 {
    use clap::error::ErrorKind;
    let kind = error.kind();
    if matches!(kind, ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
        print!("{error}");
        return 0;
    }
    let code = match kind {
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => "unknown_document_kind",
        _ => "argument_syntax",
    };
    let message = error.render().to_string();
    let message = message.trim_end().to_owned();
    eprintln!("{code}: {message}");
    let endpoint = std::env::args()
        .skip(1)
        .find(|argument| !argument.starts_with('-'))
        .unwrap_or_else(|| "usage".to_owned());
    emit(&stamp(
        serde_json::json!({
            "schema_version": 1,
            "execution_status": "failed",
            "errors": [{"code": code, "message": message}],
        }),
        &endpoint,
    ));
    2
}

const BINARY: &str = "spektrafilm-lab";

fn emit(value: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

/// `compare` takes an image pair, not a subcommand. The forms that were
/// subcommands belong to the benchmark's evaluation surface, and clap would
/// report them as a missing argument rather than as a command another binary
/// owns. The guard runs before parsing so that message stays exact.
fn reject_moved_subcommand() -> Option<i32> {
    let mut argv = std::env::args().skip(1);
    if argv.next().as_deref() != Some("compare") {
        return None;
    }
    let subcommand = argv.next()?;
    if !["runs", "readings", "pairs"].contains(&subcommand.as_str()) {
        return None;
    }
    eprintln!(
        "argument_syntax: `compare {subcommand}` belongs to spektrafilm-bench; \
         run `spektrafilm-bench compare {subcommand} ...`"
    );
    Some(2)
}

/// The binary and the endpoint that produced a result. A reader can tell a lab
/// result from a render result without inspecting the payload.
fn stamp(mut value: serde_json::Value, endpoint: &str) -> serde_json::Value {
    value["binary"] = serde_json::json!(BINARY);
    value["endpoint"] = serde_json::json!(endpoint);
    value
}

fn run() -> i32 {
    if let Some(status) = reject_moved_subcommand() {
        return status;
    }
    // A usage error is a refusal like any other, so it reaches the caller in
    // the same shape. Clap's own exit would print prose and no record, which
    // makes a pipeline special-case it.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return usage(error),
    };
    if let Commands::External(args) = &cli.command {
        let name = args.first().map(String::as_str).unwrap_or("<unknown>");
        let owner = moved(name)
            .map(|replacement| ("spektrafilm-f64", replacement))
            .or_else(|| evaluated(name).map(|replacement| ("spektrafilm-bench", replacement)));
        let message = match owner {
            Some((binary, replacement)) => {
                format!("`{name}` belongs to {binary}; run `{replacement}`")
            }
            None if name == "compare" => "`compare` takes no subcommand in spektrafilm-lab; \
                 `compare runs`, `compare readings`, and `compare pairs` belong to \
                 spektrafilm-bench compare ..."
                .to_owned(),
            None => format!(
                "unknown endpoint `{name}`; this binary owns {}",
                OWNED.join(", ")
            ),
        };
        // A refused endpoint names its owner and reaches the caller in the
        // record shape, so a script learns where the endpoint lives from the
        // same document it parses everywhere else.
        eprintln!("argument_syntax: {message}");
        emit(&stamp(
            serde_json::json!({
                "schema_version": 1,
                "execution_status": "failed",
                "errors": [{"code": "argument_syntax", "message": message}],
            }),
            name,
        ));
        return 2;
    }
    let command = match &cli.command {
        Commands::Analyze { .. } => "analyze",
        Commands::Compare { .. } => "compare",
        Commands::Regions { .. } => "regions",
        Commands::Schema { .. } => "schema",
        Commands::Validate { .. } => "validate",
        Commands::External(_) => "unknown",
    };
    let result: spektrafilm_benchmark::Result<(serde_json::Value, i32)> = match cli.command {
        // The payload is the schema itself, so it is printed verbatim rather
        // than stamped: a caller may diff it against the packaged asset.
        Commands::Schema { kind } => match documents::schema_for(&kind) {
            Ok(schema) => {
                emit(&schema);
                return 0;
            }
            Err(error) => Err(error),
        },
        Commands::Validate { kind, file } => documents::normalize_path(&file)
            .and_then(|path| validate(&kind, &path))
            .map(|value| (value, 0)),
        Commands::Regions(args) => regions::generate_grid(
            &args.id,
            args.rows,
            args.columns,
            args.origin,
            args.patch_size,
            args.gap,
            &args.space,
            &args.role,
            &args.out,
        )
        .map(|value| (value, 0)),
        Commands::Analyze(args) => images::analyze_run(&args),
        Commands::Compare(args) => images::run(&args),
        Commands::External(_) => unreachable!("handled before dispatch"),
    };
    match result {
        Ok((value, status)) => {
            emit(&stamp(value, command));
            status
        }
        Err(error) => {
            eprintln!("{error}");
            emit(&stamp(
                serde_json::json!({"schema_version":1,"command":command,"execution_status":"failed","errors":[error.to_value()]}),
                command,
            ));
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
