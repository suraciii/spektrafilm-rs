//! `spektrafilm-bench`: the benchmark's offline evidence and review commands.
//!
//! The binary owns the argument surface only. Every product rule — strict
//! validation, mode selection, source integrity, and publication — lives in the
//! library module the subcommand dispatches to, so the same rules apply to a
//! library caller.

use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use spektrafilm_benchmark::{
    Error, Result, compare, inspect, photo_report, readings_compare, sample,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "spektrafilm-bench",
    version = concat!(env!("CARGO_PKG_VERSION"), " schema-1"),
    about = "SpektraFilm evaluation: compare completed runs, join reading records, and report.",
    after_help = "Evaluate two completed runs with `spektrafilm-bench compare runs --baseline A --candidate B --policy P.json --gate --out DIR`."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compare two completed image-domain runs.
    Compare {
        #[command(subcommand)]
        command: CompareCommand,
    },
    /// Read declared sample points from an image or a retained stage capture.
    Sample {
        #[command(flatten)]
        args: sample::MeasureSampleArgs,
    },
    /// Project a retained comparison's evidence without presenting it.
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
    /// Build one benchmark-owned offline report.
    Report {
        #[command(subcommand)]
        command: ReportCommand,
    },
}

#[derive(Subcommand)]
enum CompareCommand {
    /// Compare two completed image-domain runs.
    Runs(RunComparisonArgs),
    /// Join two retained reading records.
    Readings(readings_compare::MeasureCompareArgs),
    /// Select explicit evidence pairs from one retained run.
    Pairs(PairArgs),
}

#[derive(clap::Args)]
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

#[derive(clap::Args)]
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
    /// Build one offline photo report from a photo manifest.
    Photos {
        /// Authored photo manifest JSON.
        #[arg(long, value_name = "FILE")]
        manifest: PathBuf,
        /// New report directory; must not exist.
        #[arg(long, value_name = "DIRECTORY")]
        out: PathBuf,
    },
}

/// The command name a result object carries.
fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Compare { command } => match command {
            CompareCommand::Runs(_) => "compare runs",
            CompareCommand::Readings(_) => "compare readings",
            CompareCommand::Pairs(_) => "compare pairs",
        },
        Command::Sample { .. } => "sample",
        Command::Inspect { .. } => "inspect",
        Command::Report { command } => match command {
            ReportCommand::Comparison { .. } => "report comparison",
            ReportCommand::Photos { .. } => "report photos",
        },
    }
}

/// The structured error the benchmark library reports.
type BenchError = Error;

fn execute(command: Command) -> Result<(Value, i32)> {
    match command {
        Command::Compare { command } => match command {
            CompareCommand::Runs(args) => compare::compare(
                &args.baseline,
                &args.candidate,
                &args.out,
                args.policy.as_deref(),
                args.gate,
            ),
            CompareCommand::Readings(args) => readings_compare::compare_with_result(&args)
                .map_err(|error| Error::with_message("operation_failed", error)),
            CompareCommand::Pairs(args) => {
                compare::compare_single(&args.run, &args.against, &args.out)
            }
        },
        Command::Sample { args } => sample::sample_with_result(&args)
            .map_err(|error| Error::with_message("operation_failed", error)),
        Command::Inspect {
            comparison,
            case_ids,
            region_ids,
            metrics,
        } => inspect::inspect(&comparison, &case_ids, &region_ids, &metrics).map(|v| (v, 0)),
        Command::Report { command } => match command {
            ReportCommand::Comparison { comparison, out } => {
                spektrafilm_benchmark::documents::normalize_path(&out)
                    .and_then(|out| spektrafilm_benchmark::report::build_report(&comparison, &out))
                    .map(|value| (value, 0))
            }
            ReportCommand::Photos { manifest, out } => {
                Ok((photo_report::build_report(&manifest, &out)?, 0))
            }
        },
    }
}

/// `compare` takes a retained run record or a reading record, not an image
/// pair. The image comparison is the lab's, and clap would report it as an
/// unknown subcommand rather than as a command another binary owns. The guard
/// runs before parsing so that message stays exact.
fn reject_lab_subcommand() -> Option<i32> {
    let mut argv = std::env::args().skip(1);
    let first = argv.next()?;
    // (what the caller typed, the lab form that does it)
    let (typed, replacement) = match first.as_str() {
        "compare" => match argv.next().as_deref() {
            // The lab's image comparison takes no subcommand: this binary's
            // `compare` prefix is the only reason a caller reaches for one.
            Some(sub @ ("images" | "image" | "pair")) => {
                (format!("compare {sub}"), "compare".to_owned())
            }
            _ => return None,
        },
        "analyze" | "regions" | "schema" | "validate" => (first.clone(), first.clone()),
        _ => return None,
    };
    eprintln!(
        "argument_syntax: `{typed}` belongs to spektrafilm-lab; run `spektrafilm-lab {replacement} ...`"
    );
    Some(2)
}

fn run() -> i32 {
    if let Some(status) = reject_lab_subcommand() {
        return status;
    }
    let cli = Cli::parse();
    let name = command_name(&cli.command);
    match execute(cli.command) {
        Ok((value, status)) => {
            emit(&stamp(value, name));
            status
        }
        Err(error) => {
            eprintln!("{error}");
            emit(&stamp(
                json!({
                    "schema_version": 1,
                    "command": name,
                    "execution_status": "failed",
                    "errors": [error.to_value()]
                }),
                name,
            ));
            exit_status_for(&error)
        }
    }
}

/// The exit status one command result reports. A rejected invocation is an
/// argument syntax error and must not share the exit status of a failure to
/// execute: the error code is the discriminator, so the mapping is stated once
/// here rather than repeated at every construction site.
fn exit_status_for(error: &BenchError) -> i32 {
    if error.code == "argument_syntax" {
        2
    } else {
        1
    }
}

const BINARY: &str = "spektrafilm-bench";

/// The binary and the endpoint that produced a result.
fn stamp(mut value: Value, endpoint: &str) -> Value {
    value["binary"] = json!(BINARY);
    value["endpoint"] = json!(endpoint);
    value
}

fn emit(value: &Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

fn main() {
    std::process::exit(run());
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// The product boundary is the exact documented command form; a manifest
    /// argument or a report destination is never optional, and no other report
    /// source is registered.
    #[test]
    fn report_photos_requires_manifest_and_out_and_rejects_other_sources() {
        let photos = ["bench", "report", "photos", "--manifest", "m.json"];
        assert!(Cli::try_parse_from(photos).is_err());
        assert!(
            Cli::try_parse_from([
                "bench",
                "report",
                "photos",
                "--manifest",
                "m.json",
                "--out",
                "report"
            ])
            .is_ok()
        );
        for rejected in [
            vec!["bench", "report", "photos"],
            vec!["bench", "report"],
            vec!["bench", "report", "comparison", "--comparison", "c"],
            vec!["bench", "photos", "--manifest", "m.json", "--out", "r"],
            vec![
                "bench",
                "report",
                "photos",
                "--comparison",
                "c",
                "--out",
                "r",
            ],
        ] {
            assert!(
                Cli::try_parse_from(rejected).is_err(),
                "the benchmark photo report command form must stay exact"
            );
        }
    }

    /// The exit status separates a rejected invocation from a failure to
    /// execute; the discriminator is the stable error code.
    #[test]
    fn argument_syntax_rejection_exits_two_and_execution_failure_exits_one() {
        let syntax = BenchError::new("argument_syntax", "missing --out");
        assert_eq!(exit_status_for(&syntax), 2);
        for code in [
            "hash_mismatch",
            "invalid_path",
            "schema_validation",
            "unsupported_color_transform",
            "destination_exists",
            "publication_failure",
        ] {
            let error = BenchError::new(code, "rejected elsewhere");
            assert_eq!(exit_status_for(&error), 1, "{code} must exit 1");
        }
    }

    /// Clap rejects an unknown flag before the library runs, so a rejected
    /// invocation must not reach the generator at all.
    #[test]
    fn unknown_arguments_are_rejected_before_dispatch() {
        assert!(
            Cli::try_parse_from([
                "bench",
                "report",
                "photos",
                "--manifest",
                "m.json",
                "--out",
                "report",
                "--compare",
                "all"
            ])
            .is_err()
        );
        assert_eq!(Cli::command().get_name(), "spektrafilm-bench");
    }

    /// The dispatch path reaches the real generator: a manifest that fails
    /// validation surfaces as a generator error (exit 1), never as a syntax
    /// rejection (exit 2), so the two boundaries stay distinguishable.
    #[test]
    fn execute_dispatches_the_photo_report_builder() {
        let scratch = tempfile::tempdir().unwrap();
        let error = execute(Command::Report {
            command: ReportCommand::Photos {
                manifest: scratch.path().join("absent.json"),
                out: scratch.path().join("report"),
            },
        })
        .unwrap_err();
        assert_ne!(
            error.code, "argument_syntax",
            "a generator failure is not an invocation rejection"
        );
        assert_eq!(exit_status_for(&error), 1);
        assert_eq!(
            command_name(&Command::Report {
                command: ReportCommand::Photos {
                    manifest: scratch.path().join("absent.json"),
                    out: scratch.path().join("report"),
                },
            }),
            "report photos"
        );
    }
}
