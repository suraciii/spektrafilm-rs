//! The research command surface of the f64 reference build.
//!
//! These endpoints execute a declaration and publish retained evidence. They
//! are experimental: they MAY change or disappear between revisions, so they
//! ship with the f64 binary rather than with the stable exporter commands.

use crate::lab::cli::{CompareCommand, SpectralCommand};
use anyhow::Result;
use clap::ArgMatches;

use crate::cli::Commands;

const BINARY: &str = "spektrafilm-f64";

/// The binary and the endpoint that produced a result.
fn stamp(mut value: serde_json::Value, endpoint: &str) -> serde_json::Value {
    value["binary"] = serde_json::json!(BINARY);
    value["endpoint"] = serde_json::json!(endpoint);
    value
}

fn emit(value: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

/// The structured error the evaluation library reports.
type BenchError = spektrafilm_lab::Error;

/// The command name a result object carries.
fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Compare { .. } => "compare",
        Commands::Spectral { .. } => "spectral",
        Commands::Fit(_) => "fit",
        Commands::External(_) => "unknown",
        _ => "unknown",
    }
}

fn execute(
    command: Commands,
    matches: &clap::ArgMatches,
) -> Result<(serde_json::Value, i32), BenchError> {
    match command {
        Commands::Compare { command } => match command {
            CompareCommand::Readings(_) => Err(BenchError::new(
                "argument_syntax",
                "`compare readings` belongs to spektrafilm-lab; run `spektrafilm-lab compare readings ...`",
            )),
        },
        Commands::Spectral { command } => match command {
            SpectralCommand::Evaluate(args) => {
                let (value, status) = crate::lab::evaluate::run_with_result(&args, matches)
                    .map_err(|error| BenchError::with_message("operation_failed", error))?;
                Ok((value, status))
            }
        },
        Commands::External(_) => unreachable!("handled before dispatch"),
        Commands::Fit(args) => {
            let (value, status) = crate::fit::cmd_fit_with_result(&args, matches)
                .map_err(|error| BenchError::with_message("operation_failed", error))?;
            Ok((value, status))
        }
        _ => unreachable!("the product surface is dispatched before the research surface"),
    }
}

/// Run one research endpoint and report its exit status.
///
/// The chain options a subcommand flattens live in that subcommand's own
/// matches, so the data-directory resolution reads them from the deepest
/// subcommand rather than from the top level.
pub fn run(command: Commands, matches: &ArgMatches) -> i32 {
    if let Commands::External(args) = &command {
        let name = args.first().map(String::as_str).unwrap_or("<unknown>");
        match moved(name) {
            Some((binary, replacement)) => {
                eprintln!("argument_syntax: `{name}` belongs to {binary}; run `{replacement}`")
            }
            None => eprintln!("argument_syntax: unknown endpoint `{name}`"),
        }
        return 2;
    }
    let name = command_name(&command);
    let mut subcommand = matches.clone();
    while let Some((_, nested)) = subcommand.subcommand() {
        subcommand = nested.clone();
    }
    match execute(command, &subcommand) {
        Ok((value, status)) => {
            emit(&stamp(value, name));
            status
        }
        Err(error) => {
            eprintln!("{error}");
            emit(&stamp(
                serde_json::json!({"schema_version":1,"command":name,"execution_status":"failed","errors":[error.to_value()]}),
                name,
            ));
            exit_status_for(&error)
        }
    }
}

/// The exit status one command result reports. A rejected invocation is an
/// argument syntax error and must not share the exit status of a failure to
/// execute: the error code is the discriminator.
fn exit_status_for(error: &BenchError) -> i32 {
    if error.code == "argument_syntax" {
        2
    } else {
        1
    }
}

pub fn moved_invocation() -> Option<i32> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let first = argv.first()?;
    let owned = match first.as_str() {
        "compare" => match argv.get(1).map(String::as_str) {
            Some(sub @ "readings") => Some((("spektrafilm-lab"), format!("compare {sub}"))),
            None | Some("-h" | "--help") => Some((("spektrafilm-lab"), "compare".to_owned())),
            _ => None,
        },
        other => moved(other),
    }?;
    let (binary, replacement) = owned;
    eprintln!("argument_syntax: `{first}` belongs to {binary}; run `{replacement}`");
    Some(2)
}

pub fn moved(name: &str) -> Option<(&'static str, String)> {
    // The reading and reporting surface belongs to the lab; the measurement
    // endpoints belong here.
    let (binary, replacement) = match name {
        "schema" => (
            "spektrafilm-lab",
            "spektrafilm-lab document --kind KIND ...".to_owned(),
        ),
        "validate" => (
            "spektrafilm-lab",
            "spektrafilm-lab document --kind KIND --check FILE ...".to_owned(),
        ),
        "regions" | "analyze" | "report" | "sample" | "inspect" => {
            ("spektrafilm-lab", format!("spektrafilm-lab {name} ..."))
        }
        _ => return None,
    };
    Some((binary, replacement.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The renderer returns 2 for a rejected invocation and 1 for a failure
    /// to execute. The discriminator is the error code, so the mapping lives in
    /// one place and every argument-syntax rejection shares it.
    #[test]
    fn argument_syntax_rejection_exits_two_and_execution_failure_exits_one() {
        let syntax = spektrafilm_lab::Error::new("argument_syntax", "missing --enter");
        assert_eq!(exit_status_for(&syntax), 2);
        for code in [
            "operation_failed",
            "schema_validation",
            "invalid_path",
            "unresolved_metric",
        ] {
            let error = spektrafilm_lab::Error::new(code, "rejected elsewhere");
            assert_eq!(exit_status_for(&error), 1, "{code} must exit 1");
        }
    }
}
