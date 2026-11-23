//! `spektrafilm-lab`: quantify the color science of delivered images.
//!
//! The lab renders nothing. It decodes delivered rasters with the measurement
//! library's reader, runs the shared metric implementations, and publishes
//! immutable analysis and comparison records. It also reads those records back,
//! joins retained reading records, projects what a comparison measured, and
//! builds the offline reports. Rendering endpoints live in `spektrafilm-f64`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use spektrafilm_lab::{documents, regions};

mod cli_images;
mod surface;

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
    Analyze(cli_images::AnalyzeArgs),
    /// Quantify a difference: two delivered image sets, or two retained reading records.
    Compare(CompareArgs),
    /// Expand one finite grid declaration into explicit regions.
    Regions(GridArgs),
    /// Print a document's authoritative schema, or decide one document.
    ///
    /// Without `--check` the packaged schema is printed verbatim. With it the
    /// file is decided against that schema and its locally decidable
    /// dependencies. One endpoint owns one document vocabulary, so a kind this
    /// binary can print but not decide says so in one message instead of
    /// disappearing from a second list.
    Document {
        /// Document kind; run `document --help` for the list.
        #[arg(long, value_name = "KIND", value_parser = documents::schema_kinds())]
        kind: String,
        /// JSON document to decide. Without it the schema is printed.
        #[arg(long, value_name = "FILE")]
        check: Option<PathBuf>,
    },
    /// Read the declared sample points of a delivered image.
    Sample {
        #[command(flatten)]
        args: spektrafilm_lab::sample::MeasureSampleArgs,
    },
    /// Project a retained comparison's evidence without presenting it.
    Inspect(surface::InspectArgs),
    /// Build one offline report.
    Report {
        #[command(subcommand)]
        command: surface::ReportCommand,
    },
    /// An endpoint that belongs to another binary.
    #[command(external_subcommand)]
    External(Vec<String>),
}

/// Image comparison takes no subcommand; a record comparison does. One binary,
/// two inputs: delivered images, or retained reading records.
#[derive(Args)]
struct CompareArgs {
    /// Compare two retained reading records instead of two images.
    #[command(subcommand)]
    command: Option<surface::CompareCommand>,
    /// Baseline file, or a directory of delivered images.
    #[arg(long, value_name = "PATH")]
    baseline: Option<PathBuf>,
    /// Candidate file, or a directory of delivered images.
    #[arg(long, value_name = "PATH")]
    candidate: Option<PathBuf>,
    /// Declared color space and transfer state of both sides.
    #[arg(long, value_name = "SPACE:STATE")]
    declaration: Option<cli_images::Declaration>,
    /// Declared region: name=x,y,w,h[:role]. Repeats.
    #[arg(
        long = "roi",
        value_name = "ID=X,Y,W,H[:ROLE]",
        conflicts_with = "regions"
    )]
    roi: Vec<String>,
    /// Authored regions document in render coordinates.
    #[arg(long, value_name = "FILE")]
    regions: Option<PathBuf>,
    /// Built-in analysis profile.
    #[arg(long, value_name = "ID", value_parser = cli_images::profiles())]
    analysis: Option<String>,
    /// Box-average retained difference maps to one sample per NxN block.
    #[arg(long, value_name = "N")]
    map_scale: Option<u32>,
    /// New comparison directory; must not exist.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
}

#[derive(Args)]
struct GridArgs {
    /// Document identity. Defaults to the output file stem.
    #[arg(long, value_name = "ID")]
    id: Option<String>,
    /// Number of grid rows.
    #[arg(long, value_name = "N")]
    rows: u64,
    /// Number of grid columns.
    #[arg(long, value_name = "N")]
    columns: u64,
    /// Top-left origin as X,Y.
    #[arg(long, value_name = "X,Y", default_value = "0,0", value_parser = parse_pair)]
    origin: [u64; 2],
    /// Patch size as WIDTHxHEIGHT.
    #[arg(long, value_name = "SIZE", value_parser = parse_size)]
    patch_size: [u64; 2],
    /// Gap between patches as X,Y.
    #[arg(long, value_name = "X,Y", default_value = "0,0", value_parser = parse_pair)]
    gap: [u64; 2],
    /// What each generated region is a population of. Required: a generator
    /// does not decide what a region is for.
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

fn decide(kind: &str, file: &std::path::Path) -> spektrafilm_lab::Result<serde_json::Value> {
    let display = file.display().to_string();
    let value = if matches!(kind, "comparison" | "analysis") {
        let root = file
            .parent()
            .ok_or_else(|| spektrafilm_lab::Error::new("invalid_path", "Manifest has no parent"))?;
        documents::load_json_document(file).and_then(|value| {
            documents::validate_generated_document(kind, &value, root)?;
            Ok(value)
        })
    } else {
        match kind {
            "regions" => documents::load_regions(file),
            // `document` prints any packaged schema; deciding one needs a rule.
            // A kind with a schema and no rule says exactly that, because the
            // alternative is a vocabulary that silently differs between the two
            // halves of one endpoint.
            _ if documents::schema_kinds().contains(&kind) => Err(spektrafilm_lab::Error::new(
                "unvalidatable_document_kind",
                format!(
                    "`document` has no rule for `{kind}`; the packaged schema is still \
                         printed by `document --kind {kind}`"
                ),
            )),
            _ => Err(spektrafilm_lab::Error::new(
                "unknown_document_kind",
                format!("unknown document kind: {kind}"),
            )),
        }
    }
    .map_err(|e| e.file(display))?;
    let unchecked = match kind {
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
    let result = serde_json::json!({
        "schema_version": 1,
        "command": "document",
        "execution_status": "completed",
        "kind": kind,
        "digest": spektrafilm_lab::common::digest(&value)?,
        "unchecked": unchecked
    });
    Ok(result)
}

/// The endpoints this binary owns, in help order.
const OWNED: &[&str] = &[
    "analyze", "compare", "sample", "inspect", "report", "regions", "document",
];

/// Endpoints another binary owns, and the command that owns them.
///
/// One row per endpoint and one fact per row: where the endpoint lives now.
/// The owner is the first word of the replacement, so a row cannot claim one
/// binary and point at another.
const ELSEWHERE: &[(&str, &str)] = &[
    ("measure", "spektrafilm-f64 measure ..."),
    ("spectral", "spektrafilm-f64 spectral ..."),
    ("fit", "spektrafilm-f64 fit ..."),
    ("schema", "spektrafilm-lab document --kind KIND ..."),
    (
        "validate",
        "spektrafilm-lab document --kind KIND --check FILE ...",
    ),
];

/// The owner and the replacement command for an endpoint this binary refuses.
fn elsewhere(endpoint: &str) -> Option<(&'static str, &'static str)> {
    ELSEWHERE
        .iter()
        .find(|(name, _)| *name == endpoint)
        .map(|(_, replacement)| {
            let owner = replacement.split(' ').next().unwrap_or_default();
            (owner, *replacement)
        })
}

/// Refuse an invocation in the shape every other refusal uses.
fn refuse(code: &str, message: &str, endpoint: &str) -> i32 {
    eprintln!("{code}: {message}");
    emit(&stamp(
        serde_json::json!({
            "schema_version": 1,
            "execution_status": "failed",
            "errors": [{"code": code, "message": message}],
        }),
        endpoint,
    ));
    2
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
    // A bad value is only a question about the document vocabulary when the
    // argument is `--kind`; every other rejected value is syntax.
    let invalid_argument = error
        .context()
        .find_map(|(context, value)| {
            matches!(context, clap::error::ContextKind::InvalidArg).then(|| value.to_string())
        })
        .unwrap_or_default();
    let code = if matches!(kind, ErrorKind::InvalidValue | ErrorKind::ValueValidation)
        && invalid_argument.contains("--kind")
    {
        "unknown_document_kind"
    } else {
        "argument_syntax"
    };
    let message = error.render().to_string();
    let message = message.trim_end().to_owned();
    // The endpoint a reader has to fix is what they typed: `compare readings`
    // must not be reported as `compare`, which is a different command.
    let endpoint = {
        let mut words = std::env::args()
            .skip(1)
            .filter(|word| !word.starts_with('-'));
        let first = words.next().unwrap_or_else(|| "usage".to_owned());
        match words.next() {
            Some(second) if first == "compare" && second == "readings" => {
                format!("{first} {second}")
            }
            _ => first,
        }
    };
    refuse(code, &message, &endpoint)
}

const BINARY: &str = "spektrafilm-lab";

fn emit(value: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

/// The binary and the endpoint that produced a result. A reader can tell a lab
/// result from a render result without inspecting the payload.
fn stamp(mut value: serde_json::Value, endpoint: &str) -> serde_json::Value {
    value["binary"] = serde_json::json!(BINARY);
    value["endpoint"] = serde_json::json!(endpoint);
    value
}

/// Clap cannot mark a flag "required unless a subcommand was given", so the
/// image comparison states its four required flags here, in the refusal shape
/// every other rejected invocation uses.
fn image_args(args: CompareArgs) -> std::result::Result<cli_images::Args, spektrafilm_lab::Error> {
    let missing: Vec<&str> = [
        ("--baseline", args.baseline.is_none()),
        ("--candidate", args.candidate.is_none()),
        ("--declaration", args.declaration.is_none()),
        ("--out", args.out.is_none()),
    ]
    .into_iter()
    .filter_map(|(flag, absent)| absent.then_some(flag))
    .collect();
    if !missing.is_empty() {
        return Err(spektrafilm_lab::Error::new(
            "argument_syntax",
            format!(
                "`compare` needs {}; compare retained records with `compare readings`",
                missing.join(", ")
            ),
        ));
    }
    Ok(cli_images::Args {
        baseline: args.baseline.expect("checked above"),
        candidate: args.candidate.expect("checked above"),
        declaration: args.declaration.expect("checked above"),
        roi: args.roi,
        regions: args.regions,
        analysis: args.analysis,
        map_scale: args.map_scale,
        out: args.out.expect("checked above"),
    })
}

fn run() -> i32 {
    // A usage error is a refusal like any other, so it reaches the caller in
    // the same shape. Clap's own exit would print prose and no record, which
    // makes a pipeline special-case it.
    let kinds = documents::schema_kinds().join("\n  ");
    let command = <Cli as clap::CommandFactory>::command()
        .max_term_width(96)
        // Every endpoint states a description below its flag: the two layouts
        // clap picks by width make one binary read as two.
        .mut_subcommands(|sub| {
            let sub = sub.max_term_width(96).next_line_help(true);
            if sub.get_name() == "document" {
                // The kind list is generated from the one registry and printed
                // one per line, so it cannot drift and cannot overflow a row.
                sub.hide_possible_values(true)
                    .after_help(format!("Document kinds:\n  {kinds}"))
            } else {
                sub
            }
        });
    let matches = match command.try_get_matches() {
        Ok(matches) => matches,
        Err(error) => return usage(error),
    };
    let cli = match <Cli as clap::FromArgMatches>::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => return usage(error),
    };
    if let Commands::External(args) = &cli.command {
        let name = args.first().map(String::as_str).unwrap_or("<unknown>");
        let message = match elsewhere(name) {
            Some((owner, replacement)) => {
                format!("`{name}` belongs to {owner}; run `{replacement}`")
            }
            None => format!(
                "unknown endpoint `{name}`; this binary owns {}",
                OWNED.join(", ")
            ),
        };
        // A refused endpoint names its owner and reaches the caller in the
        // record shape, so a script learns where the endpoint lives from the
        // same document it parses everywhere else.
        return refuse("argument_syntax", &message, name);
    }
    let command = match &cli.command {
        Commands::Analyze { .. } => "analyze",
        Commands::Compare(args) => match &args.command {
            None => "compare",
            Some(surface::CompareCommand::Readings(_)) => "compare readings",
        },
        Commands::Regions { .. } => "regions",
        Commands::Document { .. } => "document",
        Commands::Sample { .. } => "sample",
        Commands::Inspect { .. } => "inspect",
        Commands::Report { command } => match command {
            surface::ReportCommand::Comparison { .. } => "report comparison",
            surface::ReportCommand::Photos { .. } => "report photos",
        },
        Commands::External(_) => "unknown",
    };
    let result: spektrafilm_lab::Result<(serde_json::Value, i32)> = match cli.command {
        // The printed payload is the schema itself, so it is not stamped: a
        // caller may diff it against the packaged asset.
        Commands::Document { kind, check: None } => match documents::schema_for(&kind) {
            Ok(schema) => {
                emit(&schema);
                return 0;
            }
            Err(error) => Err(error),
        },
        Commands::Document {
            kind,
            check: Some(file),
        } => documents::normalize_path(&file)
            .and_then(|path| decide(&kind, &path))
            .map(|value| (value, 0)),
        Commands::Regions(args) => {
            let id = args.id.clone().unwrap_or_else(|| {
                args.out
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("regions")
                    .to_owned()
            });
            // A delivered raster has one coordinate space. Offering the other
            // six would let a caller declare geometry this binary refuses to
            // measure, so the generator writes the space its reader reads.
            regions::generate_grid(
                &id,
                args.rows,
                args.columns,
                args.origin,
                args.patch_size,
                args.gap,
                "render",
                &args.role,
                &args.out,
            )
            .map(|value| (value, 0))
        }
        Commands::Analyze(args) => cli_images::analyze_run(&args),
        Commands::Compare(args) => match args.command {
            Some(command) => surface::compare_runs(&command),
            None => image_args(args).and_then(|args| cli_images::run(&args)),
        },
        Commands::Sample { args } => surface::sample(&args),
        Commands::Inspect(args) => surface::inspect(&args),
        Commands::Report { command } => surface::report(&command),
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
