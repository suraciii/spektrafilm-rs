//! `spektrafilm-experiment`: the research renderer at f64 reference precision.

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use spektrafilm_benchmark::{local_input, runner, target};
use spektrafilm_cli::lab::cli::{
    CompareCommand, EvaluationRunArgs, ExperimentCommand, MeasureCommand, RunCommand, RunImageArgs,
    RunSuiteArgs, SpectralCommand,
};

#[derive(Parser)]
#[command(
    name = "spektrafilm-experiment",
    about = "SpektraFilm research renderer: execute a declaration and publish retained evidence at f64 reference precision",
    // Version must identify the instrument implementation and the document
    // versions it reads and writes, so the instrument's own version is not the
    // only number a caller can rely on.
    version = concat!(env!("CARGO_PKG_VERSION"), " (research documents schema_version 1)"),
    after_help = "Generate packaged examples with `spektrafilm-experiment fixtures --renderer RENDERER --data-dir DATA --out BUNDLE`, then consume a generated suite with `spektrafilm-experiment run suite --suite BUNDLE/complete.json --target BUNDLE/baseline.json --out RUN`."
)]
struct LabCli {
    #[command(subcommand)]
    command: LabCommands,
}

#[derive(Subcommand)]
// Clap subcommands are built once at startup, outside hot paths.
#[allow(clippy::large_enum_variant)]
enum LabCommands {
    /// Pin a renderer executable in an existing target document.
    Target {
        /// Renderer executable.
        #[arg(long, value_name = "FILE")]
        renderer: std::path::PathBuf,
        /// Explicit profile/data directory.
        #[arg(long, value_name = "DIR")]
        data_dir: std::path::PathBuf,
        /// Requested compute backend; availability is checked during rendering.
        #[arg(long, value_name = "BACKEND", value_parser = ["cpu", "gpu"])]
        backend: String,
        /// New target JSON file; must not exist.
        #[arg(long, value_name = "FILE")]
        out: std::path::PathBuf,
    },
    /// Execute an image-domain endpoint, or accept the deprecated flat run alias.
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
    /// Execute one declared chain measurement.
    Measure {
        #[command(subcommand)]
        command: MeasureCommand,
    },
    /// Compare retained records through one explicit endpoint, or accept the deprecated flat alias.
    Compare {
        #[command(subcommand)]
        command: CompareCommand,
    },
    /// Generate the packaged examples: probe assets, suites, targets, and policy.
    ///
    /// Consume a generated suite with `spektrafilm-lab run suite --suite BUNDLE/complete.json --target BUNDLE/baseline.json --out RUN`.
    Fixtures {
        /// Renderer executable the generated targets pin.
        #[arg(long, value_name = "FILE")]
        renderer: std::path::PathBuf,
        /// Explicit profile/data directory.
        #[arg(long, value_name = "DIR")]
        data_dir: std::path::PathBuf,
        /// New bundle directory; must not exist.
        #[arg(long, value_name = "DIR")]
        out: std::path::PathBuf,
        /// Prepared local natural image.
        #[arg(long, value_name = "FILE", conflicts_with = "download_natural")]
        natural_input: Option<std::path::PathBuf>,
        /// Download the pinned natural source instead of reading a local file.
        #[arg(long)]
        download_natural: bool,
    },
    /// Read declared sample points from an image or a retained stage capture.
    Sample {
        #[command(flatten)]
        args: spektrafilm_cli::lab::cli::MeasureSampleArgs,
    },
    /// Re-resolve a recorded record and report identity differences.
    Verify {
        #[command(flatten)]
        args: spektrafilm_cli::lab::cli::MeasureVerifyArgs,
    },
    /// Spectral diagnostics.
    Spectral {
        #[command(subcommand)]
        command: SpectralCommand,
    },
    /// The development loop for a film profile.
    Experiment {
        #[command(subcommand)]
        command: ExperimentCommand,
    },
    /// Fit profile and look parameters against reference patch data.
    Fit(spektrafilm_cli::fit::FitArgs),
    /// A lab endpoint that belongs to `spektrafilm-lab`.
    #[command(external_subcommand)]
    External(Vec<String>),
}

fn emit(value: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

/// The structured error the evaluation library reports.
type BenchError = spektrafilm_benchmark::Error;

/// The command name a result object carries.
fn command_name(command: &LabCommands) -> &'static str {
    match command {
        LabCommands::Target { .. } => "target",
        LabCommands::Run { .. } => "run",
        LabCommands::Measure { .. } => "measure",
        LabCommands::Compare { .. } => "compare",
        LabCommands::Fixtures { .. } => "fixtures",
        LabCommands::Sample { .. } => "sample",
        LabCommands::Verify { .. } => "verify",
        LabCommands::Spectral { .. } => "spectral",
        LabCommands::Experiment { .. } => "experiment",
        LabCommands::Fit(_) => "fit",
        LabCommands::External(_) => "unknown",
    }
}

fn execute(
    command: LabCommands,
    matches: &clap::ArgMatches,
) -> Result<(serde_json::Value, i32), BenchError> {
    match command {
        LabCommands::Target {
            renderer,
            data_dir,
            backend,
            out,
        } => Ok((target::create(&renderer, &data_dir, &backend, &out)?, 0)),
        LabCommands::Run { command } => match command {
            RunCommand::Suite(args) => execute_run_suite(args),
            RunCommand::Image(args) => execute_run_image(args),
            RunCommand::Flat(args) => execute_run_legacy(*args, matches),
        },
        LabCommands::Measure { command } => match command {
            MeasureCommand::Run(args) => {
                let (value, status) =
                    spektrafilm_cli::lab::measure::run::run_with_result(&args, matches)
                        .map_err(|error| BenchError::with_message("operation_failed", error))?;
                Ok((value, status))
            }
        },
        LabCommands::Compare { command } => match command {
            CompareCommand::Readings(args) => {
                let measurement = spektrafilm_cli::lab::cli::MeasureCompareArgs {
                    baseline: args.baseline,
                    candidate: args.candidate,
                    reference: args.reference,
                    out: args.out,
                };
                spektrafilm_cli::lab::measure::compare::compare_with_result(&measurement)
                    .map_err(|error| BenchError::with_message("operation_failed", error))
            }
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
        LabCommands::Fixtures {
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
        LabCommands::Sample { args } => {
            let (value, status) = spektrafilm_cli::lab::measure::sample::sample_with_result(&args)
                .map_err(|error| BenchError::with_message("operation_failed", error))?;
            Ok((value, status))
        }
        LabCommands::Verify { args } => {
            let (value, status) =
                spektrafilm_cli::lab::measure::verify::verify_with_result(&args, matches)
                    .map_err(|error| BenchError::with_message("operation_failed", error))?;
            Ok((value, status))
        }
        LabCommands::Spectral { command } => match command {
            SpectralCommand::Evaluate(args) => {
                let (value, status) =
                    spektrafilm_cli::lab::evaluate::run_with_result(&args, matches)
                        .map_err(|error| BenchError::with_message("operation_failed", error))?;
                Ok((value, status))
            }
        },
        LabCommands::Experiment { command } => {
            spektrafilm_cli::lab::experiment::dispatch_with_result(&command)
                .map_err(|error| BenchError::with_message("operation_failed", error))
        }
        LabCommands::External(_) => unreachable!("handled before dispatch"),
        LabCommands::Fit(args) => {
            let (value, status) = spektrafilm_cli::fit::cmd_fit_with_result(&args, matches)
                .map_err(|error| BenchError::with_message("operation_failed", error))?;
            Ok((value, status))
        }
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
fn normalize_legacy_argv() -> Vec<std::ffi::OsString> {
    let mut args: Vec<_> = std::env::args_os().collect();
    if args.len() >= 3 {
        let alias = match (args[1].to_str(), args[2].to_str()) {
            (Some("run"), Some(value)) => !matches!(
                value,
                "suite" | "image" | "flat" | "help" | "-h" | "--help" | "-V" | "--version"
            ),
            (Some("compare"), Some(value)) => !matches!(
                value,
                "runs"
                    | "readings"
                    | "pairs"
                    | "images"
                    | "flat"
                    | "help"
                    | "-h"
                    | "--help"
                    | "-V"
                    | "--version"
            ),
            _ => false,
        };
        if alias {
            args.insert(2, std::ffi::OsString::from("flat"));
        }
    }
    args
}

/// Lab endpoints, with the binary that owns them.
fn moved(name: &str) -> Option<String> {
    const MOVED: &[&str] = &["schema", "validate", "regions", "inspect", "report"];
    MOVED
        .contains(&name)
        .then(|| format!("spektrafilm-lab {name} ..."))
}

fn run() -> i32 {
    let matches = LabCli::command().get_matches_from(normalize_legacy_argv());
    let Ok(cli) = LabCli::from_arg_matches(&matches) else {
        eprintln!("argument syntax error: the command could not be parsed");
        return 2;
    };
    if let LabCommands::External(args) = &cli.command {
        let name = args.first().map(String::as_str).unwrap_or("<unknown>");
        match moved(name) {
            Some(replacement) => eprintln!(
                "argument_syntax: `{name}` belongs to spektrafilm-lab; run `{replacement}`"
            ),
            None => eprintln!("argument_syntax: unknown endpoint `{name}`"),
        }
        return 2;
    }
    let name = command_name(&cli.command);
    // The chain options a subcommand flattens live in that subcommand's own
    // matches, so the data-directory resolution reads them from there rather
    // than from the top level. A grouped command (`spectral evaluate`) nests
    // one level deeper, so the descent follows every subcommand hop.
    let mut subcommand = matches.subcommand().map(|(_, sub)| sub.to_owned());
    while let Some(current) = subcommand.as_ref() {
        match current.subcommand() {
            Some((_, nested)) => subcommand = Some(nested.to_owned()),
            None => break,
        }
    }
    let subcommand = subcommand.unwrap_or_default();
    match execute(cli.command, &subcommand) {
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
/// execute: the error code is the discriminator, so the mapping is stated once
/// here rather than repeated at every construction site.
fn exit_status_for(error: &BenchError) -> i32 {
    if error.code == "argument_syntax" {
        2
    } else {
        1
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    std::process::exit(run())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_run_subcommands_keep_endpoint_options_separate() {
        assert!(
            LabCli::try_parse_from([
                "lab",
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
            LabCli::try_parse_from([
                "lab",
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
            LabCli::try_parse_from([
                "lab",
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
            "lab",
            "fixtures",
            "--renderer",
            "renderer",
            "--data-dir",
            "data",
            "--out",
            "bundle",
        ];
        assert!(
            LabCli::try_parse_from(prefix.into_iter().chain([
                "--natural-input",
                "image.png",
                "--download-natural"
            ]))
            .is_err()
        );
        assert!(
            LabCli::try_parse_from(prefix.into_iter().chain(["--natural-input", "image.png"]))
                .is_ok()
        );
        assert!(LabCli::try_parse_from(prefix.into_iter().chain(["--download-natural"])).is_ok());
    }
}
