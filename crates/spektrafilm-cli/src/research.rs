//! The research command surface of the f64 reference build.
//!
//! These endpoints execute a declaration and publish retained evidence. They
//! are experimental: they MAY change or disappear between revisions, so they
//! ship with the f64 binary rather than with the stable exporter commands.

use anyhow::Result;
use clap::ArgMatches;
use spektrafilm_benchmark::{local_input, runner, target};
use spektrafilm_cli::lab::cli::{
    CompareCommand, EvaluationRunArgs, MeasureCommand, RunCommand, RunImageArgs, RunSuiteArgs,
    SpectralCommand,
};

use crate::Commands;

fn emit(value: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

/// The structured error the evaluation library reports.
type BenchError = spektrafilm_benchmark::Error;

/// The command name a result object carries.
fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Target { .. } => "target",
        Commands::Run { .. } => "run",
        Commands::Measure { .. } => "measure",
        Commands::Compare { .. } => "compare",
        Commands::Fixtures { .. } => "fixtures",
        Commands::Spectral { .. } => "spectral",
        Commands::Experiment { .. } => "experiment",
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
        Commands::Target {
            renderer,
            data_dir,
            backend,
            out,
        } => Ok((target::create(&renderer, &data_dir, &backend, &out)?, 0)),
        Commands::Run { command } => match command {
            RunCommand::Suite(args) => execute_run_suite(args),
            RunCommand::Image(args) => execute_run_image(args),
            RunCommand::Flat(args) => execute_run_legacy(*args, matches),
        },
        Commands::Measure { command } => match command {
            MeasureCommand::Run(args) => {
                let (value, status) =
                    spektrafilm_cli::lab::measure::run::run_with_result(&args, matches)
                        .map_err(|error| BenchError::with_message("operation_failed", error))?;
                Ok((value, status))
            }
        },
        Commands::Compare { command } => match command {
            CompareCommand::Readings(_) => Err(BenchError::new(
                "argument_syntax",
                "`compare readings` belongs to spektrafilm-lab; run `spektrafilm-lab compare readings`",
            )),
            CompareCommand::Runs(_) => Err(BenchError::new(
                "argument_syntax",
                "`compare runs` belongs to spektrafilm-lab; run `spektrafilm-lab compare runs`",
            )),
            CompareCommand::Pairs(_) => Err(BenchError::new(
                "argument_syntax",
                "`compare pairs` belongs to spektrafilm-lab; run `spektrafilm-lab compare pairs`",
            )),
            CompareCommand::Flat(_) => Err(BenchError::new(
                "argument_syntax",
                "the flat compare alias is removed; run `spektrafilm-lab compare runs|images|pairs`",
            )),
        },
        Commands::Fixtures {
            renderer,
            data_dir,
            out,
            natural_input,
            download_natural,
        } => Ok((
            spektrafilm_benchmark::fixtures::create_fixtures(
                &renderer,
                &data_dir,
                &out,
                natural_input.as_deref(),
                download_natural,
            )?,
            0,
        )),
        Commands::Spectral { command } => match command {
            SpectralCommand::Evaluate(args) => {
                let (value, status) =
                    spektrafilm_cli::lab::evaluate::run_with_result(&args, matches)
                        .map_err(|error| BenchError::with_message("operation_failed", error))?;
                Ok((value, status))
            }
        },
        Commands::Experiment { command } => {
            spektrafilm_cli::lab::experiment::dispatch_with_result(&command)
                .map_err(|error| BenchError::with_message("operation_failed", error))
        }
        Commands::External(_) => unreachable!("handled before dispatch"),
        Commands::Fit(args) => {
            let (value, status) = spektrafilm_cli::fit::cmd_fit_with_result(&args, matches)
                .map_err(|error| BenchError::with_message("operation_failed", error))?;
            Ok((value, status))
        }
        _ => unreachable!("the product surface is dispatched before the research surface"),
    }
}

fn execute_run_suite(args: RunSuiteArgs) -> Result<(serde_json::Value, i32), BenchError> {
    if args.dry_run {
        runner::preflight(&args.suite, &args.target, &args.out, &args.case_ids)
    } else {
        runner::run(&args.suite, &args.target, &args.out, &args.case_ids)
    }
}

fn execute_run_image(args: RunImageArgs) -> Result<(serde_json::Value, i32), BenchError> {
    let adapter = local_input::LocalInput {
        input: args.input,
        configuration: args.configuration.clone(),
        input_kind: args.input_kind,
        input_color_space: args.input_color_space,
        input_cctf_decoding: args.input_cctf_decoding,
        seeds: args.seeds,
        regions: args.regions,
        reference: args.reference,
        reference_info: args.reference_info,
        preparation: args.preparation,
    };
    if args.dry_run {
        runner::preflight_suite(
            adapter.suite()?,
            &args.configuration,
            &args.target,
            &args.out,
            &[],
        )
    } else {
        runner::run_suite(
            adapter.suite()?,
            &args.configuration,
            &args.target,
            &args.out,
            &[],
        )
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
            Some(replacement) => eprintln!(
                "argument_syntax: `{name}` belongs to spektrafilm-lab; run `{replacement}`"
            ),
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
            emit(&value);
            status
        }
        Err(error) => {
            eprintln!("{error}");
            emit(
                &serde_json::json!({"schema_version":1,"command":name,"execution_status":"failed","errors":[error.to_value()]}),
            );
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

fn execute_run_legacy(
    args: EvaluationRunArgs,
    matches: &clap::ArgMatches,
) -> Result<(serde_json::Value, i32), BenchError> {
    let out = args
        .out
        .ok_or_else(|| BenchError::new("argument_syntax", "flat run requires --out"))?;
    if args.stimuli.is_some()
        && (args.suite.is_some()
            || args.input.is_some()
            || args.configuration.is_some()
            || args.input_kind.is_some()
            || args.input_color_space.is_some()
            || args.input_cctf_decoding.is_some()
            || !args.seeds.is_empty()
            || args.regions.is_some()
            || args.reference.is_some()
            || args.reference_info.is_some()
            || args.preparation.is_some()
            || args.target.is_some()
            || !args.case_ids.is_empty())
    {
        return Err(BenchError::new(
            "argument_syntax",
            "flat run cannot combine --stimuli with image-domain options",
        ));
    }
    if args.stimuli.is_none()
        && args.suite.is_some()
        && (args.input.is_some()
            || args.configuration.is_some()
            || args.input_kind.is_some()
            || args.input_color_space.is_some()
            || args.input_cctf_decoding.is_some()
            || !args.seeds.is_empty()
            || args.regions.is_some()
            || args.reference.is_some()
            || args.reference_info.is_some()
            || args.preparation.is_some())
    {
        return Err(BenchError::new(
            "argument_syntax",
            "flat run cannot combine --suite with local-image options",
        ));
    }
    if args.stimuli.is_none() && args.suite.is_none() && args.input.is_none() {
        return Err(BenchError::new(
            "argument_syntax",
            "flat run requires --suite, --input, or --stimuli",
        ));
    }
    if args.stimuli.is_some() && args.enter.is_none() {
        return Err(BenchError::new(
            "argument_syntax",
            "--stimuli requires --enter",
        ));
    }
    if let Some(stimuli) = args.stimuli {
        let enter = args
            .enter
            .ok_or_else(|| BenchError::new("argument_syntax", "--stimuli requires --enter"))?;
        let measurement = spektrafilm_cli::lab::cli::MeasureRunArgs {
            stimuli,
            enter,
            read: args.read,
            dry_run: args.dry_run,
            out,
            chain: spektrafilm_cli::lab::cli::ChainArgs {
                preset: args.preset,
                film: args.film,
                paper: args.paper,
                sources: args.sources,
                route: args.route,
                scan_output: args.scan_output,
                seed: 0,
                data_dir: args.data_dir,
            },
        };
        let (value, status) =
            spektrafilm_cli::lab::measure::run::run_with_result(&measurement, matches)
                .map_err(|error| BenchError::with_message("operation_failed", error))?;
        return Ok((value, status));
    }
    let target = args
        .target
        .ok_or_else(|| BenchError::new("argument_syntax", "flat run requires --target"))?;
    if let Some(suite) = args.suite {
        return execute_run_suite(RunSuiteArgs {
            suite,
            target,
            out,
            case_ids: args.case_ids,
            dry_run: args.dry_run,
        });
    }
    let configuration = args
        .configuration
        .ok_or_else(|| BenchError::new("argument_syntax", "flat run requires --configuration"))?;
    execute_run_image(RunImageArgs {
        input: args
            .input
            .ok_or_else(|| BenchError::new("argument_syntax", "flat run requires --input"))?,
        configuration,
        input_kind: args.input_kind.unwrap_or_else(|| "raster".to_owned()),
        input_color_space: args.input_color_space,
        input_cctf_decoding: args.input_cctf_decoding,
        seeds: args.seeds,
        regions: args.regions,
        reference: args.reference,
        reference_info: args.reference_info,
        preparation: args.preparation,
        target,
        out,
        dry_run: args.dry_run,
    })
}

/// The deprecated flat `run` alias: `run --stimuli` and `run --suite` insert
/// their canonical leaf before clap sees the arguments.
pub fn normalize_legacy_argv() -> Vec<std::ffi::OsString> {
    let mut args: Vec<_> = std::env::args_os().collect();
    if args.len() >= 3
        && args[1].to_str() == Some("run")
        && !matches!(
            args[2].to_str(),
            Some("suite" | "image" | "flat" | "help" | "-h" | "--help" | "-V" | "--version")
        )
    {
        args.insert(2, std::ffi::OsString::from("flat"));
    }
    args
}
/// The lab endpoints, with the binary that owns them.
pub fn moved(name: &str) -> Option<String> {
    const MOVED: &[&str] = &["schema", "validate", "regions", "report", "sample"];
    MOVED
        .contains(&name)
        .then(|| format!("spektrafilm-lab {name} ..."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use clap::Parser;

    #[test]
    fn canonical_run_subcommands_keep_endpoint_options_separate() {
        assert!(
            Cli::try_parse_from([
                "spektrafilm-f64",
                "run",
                "suite",
                "--suite",
                "suite.json",
                "--target",
                "target.json",
                "--out",
                "run",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "spektrafilm-f64",
                "run",
                "image",
                "--input",
                "image.png",
                "--configuration",
                "config.json",
                "--target",
                "target.json",
                "--out",
                "run",
                "--input-cctf-decoding",
                "false",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from([
                "spektrafilm-f64",
                "run",
                "suite",
                "--suite",
                "suite.json",
                "--input",
                "image.png",
                "--target",
                "target.json",
                "--out",
                "run",
            ])
            .is_err()
        );
    }

    /// The renderer returns 2 for a rejected invocation and 1 for a failure
    /// to execute. The discriminator is the error code, so the mapping lives in
    /// one place and every argument-syntax rejection shares it.
    #[test]
    fn argument_syntax_rejection_exits_two_and_execution_failure_exits_one() {
        let syntax = spektrafilm_benchmark::Error::new("argument_syntax", "missing --enter");
        assert_eq!(exit_status_for(&syntax), 2);
        for code in [
            "operation_failed",
            "schema_validation",
            "invalid_path",
            "unresolved_metric",
        ] {
            let error = spektrafilm_benchmark::Error::new(code, "rejected elsewhere");
            assert_eq!(exit_status_for(&error), 1, "{code} must exit 1");
        }
    }

    #[test]
    fn fixtures_selectors_are_mutually_exclusive() {
        let prefix = [
            "spektrafilm-f64",
            "fixtures",
            "--renderer",
            "renderer",
            "--data-dir",
            "data",
            "--out",
            "bundle",
        ];
        assert!(
            Cli::try_parse_from(prefix.into_iter().chain([
                "--natural-input",
                "image.png",
                "--download-natural"
            ]))
            .is_err()
        );
        assert!(
            Cli::try_parse_from(prefix.into_iter().chain(["--natural-input", "image.png"])).is_ok()
        );
        assert!(Cli::try_parse_from(prefix.into_iter().chain(["--download-natural"])).is_ok());
    }
}
