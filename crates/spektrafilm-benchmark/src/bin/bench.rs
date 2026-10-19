//! `spektrafilm-bench`: the benchmark's offline evidence and review commands.
//!
//! The binary owns the argument surface only. Every product rule — strict
//! validation, mode selection, source integrity, and publication — lives in the
//! library module the subcommand dispatches to, so the same rules apply to a
//! library caller.

use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use spektrafilm_benchmark::{Error, Result, photo_report};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "spektrafilm-bench",
    version = concat!(env!("CARGO_PKG_VERSION"), " schema-1"),
    about = "Build benchmark-owned offline visual reports from authored manifests.",
    after_help = "Publish one offline photo report with `spektrafilm-bench report photos --manifest MANIFEST.json --out REPORT`."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build one benchmark-owned offline report from an authored manifest.
    Report {
        #[command(subcommand)]
        command: ReportCommand,
    },
}

#[derive(Subcommand)]
enum ReportCommand {
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
fn command_name() -> &'static str {
    "report photos"
}

/// The structured error the benchmark library reports.
type BenchError = Error;

fn execute(command: Command) -> Result<(Value, i32)> {
    match command {
        Command::Report { command } => match command {
            ReportCommand::Photos { manifest, out } => {
                Ok((photo_report::build_report(&manifest, &out)?, 0))
            }
        },
    }
}

fn run() -> i32 {
    let cli = Cli::parse();
    match execute(cli.command) {
        Ok((value, status)) => {
            emit(&value);
            status
        }
        Err(error) => {
            eprintln!("{error}");
            emit(&json!({
                "schema_version": 1,
                "command": command_name(),
                "execution_status": "failed",
                "errors": [error.to_value()]
            }));
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
        assert_eq!(command_name(), "report photos");
    }
}
