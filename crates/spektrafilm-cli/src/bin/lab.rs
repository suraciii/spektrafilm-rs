//! `spektrafilm-lab`: the research instrument at f64 reference precision.

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use spektrafilm_benchmark::{
    common, compare, documents, inspect, local_input, regions, report, runner, target,
};
use spektrafilm_cli::lab::cli::{
    CompareCommand, EvaluationCompareArgs, EvaluationInspectArgs, EvaluationRunArgs,
    ExperimentCommand, MeasureCommand, RegionsCommand, ReportCommand, RunCommand, RunImageArgs,
    RunSuiteArgs, SpectralCommand,
};

#[derive(Parser)]
#[command(
    name = "spektrafilm-lab",
    about = "SpektraFilm instrument: film-profile development, calibration, and appearance evaluation at f64 reference precision",
    // Version must identify the instrument implementation and the document
    // versions it reads and writes, so the instrument's own version is not the
    // only number a caller can rely on.
    version = concat!(env!("CARGO_PKG_VERSION"), " (instrument documents schema_version 1)"),
    after_help = "Generate packaged examples with `spektrafilm-lab fixtures --renderer RENDERER --data-dir DATA --out BUNDLE`, then consume a generated suite with `spektrafilm-lab run suite --suite BUNDLE/complete.json --target BUNDLE/baseline.json --out RUN`."
)]
struct LabCli {
    #[command(subcommand)]
    command: LabCommands,
}

#[derive(Subcommand)]
// Clap subcommands are built once at startup, outside hot paths.
#[allow(clippy::large_enum_variant)]
enum LabCommands {
    /// Print the authoritative schema for an authored or retained document.
    Schema {
        /// Document kind.
        #[arg(
            long,
            value_name = "KIND",
            value_parser = [
                "suite", "target", "policy", "run", "comparison", "configuration", "reference",
                "regions", "preparation", "stimuli", "material_measurement", "correspondence_review",
                "experiment", "preflight", "provenance", "report", "resolved", "command", "export",
                "evidence", "metrics", "review"
            ]
        )]
        kind: String,
    },
    /// Validate document shape and available local dependencies without rendering.
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
        file: std::path::PathBuf,
    },
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
    /// Read retained measurements without recomputing or invoking a renderer.
    Inspect {
        #[command(flatten)]
        args: EvaluationInspectArgs,
    },
    /// Build an offline report from one retained comparison.
    Report {
        #[command(subcommand)]
        command: ReportCommand,
    },
    /// Generate standalone measurement regions.
    Regions {
        #[command(subcommand)]
        command: RegionsCommand,
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
}

fn emit(value: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}

/// The structured error the evaluation library reports.
type BenchError = spektrafilm_benchmark::Error;

/// The command name a result object carries.
fn command_name(command: &LabCommands) -> &'static str {
    match command {
        LabCommands::Schema { .. } => "schema",
        LabCommands::Validate { .. } => "validate",
        LabCommands::Target { .. } => "target",
        LabCommands::Run { .. } => "run",
        LabCommands::Measure { .. } => "measure",
        LabCommands::Compare { .. } => "compare",
        LabCommands::Inspect { .. } => "inspect",
        LabCommands::Report { .. } => "report",
        LabCommands::Regions { .. } => "regions",
        LabCommands::Fixtures { .. } => "fixtures",
        LabCommands::Sample { .. } => "sample",
        LabCommands::Verify { .. } => "verify",
        LabCommands::Spectral { .. } => "spectral",
        LabCommands::Experiment { .. } => "experiment",
        LabCommands::Fit(_) => "fit",
    }
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
        "suite"
        | "target"
        | "configuration"
        | "reference"
        | "regions"
        | "stimuli"
        | "experiment"
        | "material_measurement"
        | "correspondence_review" => {
            serde_json::json!([
                "rendering",
                "compute_device",
                "cross_document_applicability"
            ])
        }
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
        _ => serde_json::json!([]),
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

fn execute(
    command: LabCommands,
    matches: &clap::ArgMatches,
) -> Result<(serde_json::Value, i32), BenchError> {
    match command {
        LabCommands::Schema { kind } => {
            if kind == "review" {
                Err(BenchError::new(
                    "unsupported_document_kind",
                    "review documents are benchmark-owned and unsupported by spektrafilm-lab",
                ))
            } else {
                Ok((documents::schema_for(&kind)?, 0))
            }
        }
        LabCommands::Validate { kind, file } => {
            if kind == "review" {
                Err(BenchError::new(
                    "unsupported_document_kind",
                    "review documents are benchmark-owned and unsupported by spektrafilm-lab",
                ))
            } else {
                Ok((validate(&kind, &documents::normalize_path(&file)?)?, 0))
            }
        }
        LabCommands::Target {
            renderer,
            data_dir,
            backend,
            out,
        } => Ok((target::create(&renderer, &data_dir, &backend, &out)?, 0)),
        LabCommands::Run { command } => match command {
            RunCommand::Suite(args) => execute_run_suite(args),
            RunCommand::Image(args) => execute_run_image(args),
            RunCommand::Flat(args) => execute_run_legacy(args, matches),
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
            CompareCommand::Runs(args) => compare::compare(
                &args.baseline,
                &args.candidate,
                &args.out,
                args.policy.as_deref(),
                args.gate,
            ),
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
            CompareCommand::Pairs(args) => {
                compare::compare_single(&args.run, &args.against, &args.out)
            }
            CompareCommand::Flat(args) => execute_compare_legacy(args),
        },
        LabCommands::Inspect { args } => Ok((
            inspect::inspect(
                &args.comparison,
                &args.case_ids,
                &args.region_ids,
                &args.metrics,
            )?,
            0,
        )),
        LabCommands::Report { command } => match command {
            ReportCommand::Comparison(args) => {
                let out = documents::normalize_path(&args.out)?;
                Ok((report::build_report(&args.comparison, &out)?, 0))
            }
        },
        LabCommands::Regions { command } => match command {
            RegionsCommand::Grid(args) => Ok((
                regions::generate_grid(
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
                )?,
                0,
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
        LabCommands::Fit(args) => {
            let (value, status) = spektrafilm_cli::fit::cmd_fit_with_result(&args, matches)
                .map_err(|error| BenchError::with_message("operation_failed", error))?;
            Ok((value, status))
        }
    }
}

fn execute_run_suite(args: RunSuiteArgs) -> Result<(serde_json::Value, i32), BenchError> {
    if args.dry_run {
        runner::preflight(
            &args.suite,
            &args.target,
            &args.out,
            &args.case_ids,
            args.stage_evidence,
        )
    } else {
        runner::run(
            &args.suite,
            &args.target,
            &args.out,
            &args.case_ids,
            args.stage_evidence,
        )
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
            args.stage_evidence,
        )
    } else {
        runner::run_suite(
            adapter.suite()?,
            &args.configuration,
            &args.target,
            &args.out,
            &[],
            args.stage_evidence,
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
            || !args.case_ids.is_empty()
            || args.stage_evidence)
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
            stage_evidence: args.stage_evidence,
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
        stage_evidence: args.stage_evidence,
    })
}
fn execute_compare_legacy(
    args: EvaluationCompareArgs,
) -> Result<(serde_json::Value, i32), BenchError> {
    let out = args
        .out
        .ok_or_else(|| BenchError::new("argument_syntax", "flat compare requires --out"))?;
    if args.run.is_some()
        && (args.baseline.is_some()
            || args.candidate.is_some()
            || args.reference_run.is_some()
            || args.policy.is_some()
            || args.gate)
    {
        return Err(BenchError::new(
            "argument_syntax",
            "flat compare cannot combine --run with two-sided comparison options",
        ));
    }
    if args.run.is_some() && args.against.is_none() {
        return Err(BenchError::new(
            "argument_syntax",
            "flat --run requires --against",
        ));
    }
    if args.run.is_none() && args.against.is_some() {
        return Err(BenchError::new(
            "argument_syntax",
            "flat --against requires --run",
        ));
    }
    if let Some(run) = args.run {
        let against = args
            .against
            .ok_or_else(|| BenchError::new("argument_syntax", "flat --run requires --against"))?;
        return compare::compare_single(&run, &against, &out);
    }
    if args.reference_run.is_some() && args.policy.is_some() {
        return Err(BenchError::new(
            "argument_syntax",
            "flat compare cannot combine --reference-run with --policy",
        ));
    }
    if args.reference_run.is_some() {
        let measurement = spektrafilm_cli::lab::cli::MeasureCompareArgs {
            baseline: args.baseline.ok_or_else(|| {
                BenchError::new("argument_syntax", "flat compare requires --baseline")
            })?,
            candidate: args.candidate.ok_or_else(|| {
                BenchError::new("argument_syntax", "flat compare requires --candidate")
            })?,
            reference: args.reference_run,
            out,
        };
        return spektrafilm_cli::lab::measure::compare::compare_with_result(&measurement)
            .map_err(|error| BenchError::with_message("operation_failed", error));
    }
    let baseline = args
        .baseline
        .ok_or_else(|| BenchError::new("argument_syntax", "flat compare requires --baseline"))?;
    let candidate = args
        .candidate
        .ok_or_else(|| BenchError::new("argument_syntax", "flat compare requires --candidate"))?;
    compare::compare(
        &baseline,
        &candidate,
        &out,
        args.policy.as_deref(),
        args.gate,
    )
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

fn run() -> i32 {
    let matches = LabCli::command().get_matches_from(normalize_legacy_argv());
    let Ok(cli) = LabCli::from_arg_matches(&matches) else {
        eprintln!("argument syntax error: the command could not be parsed");
        return 2;
    };
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

    /// The instrument returns 2 for a rejected invocation and 1 for a failure
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
    fn canonical_compare_subcommands_keep_sources_separate() {
        assert!(
            LabCli::try_parse_from([
                "lab",
                "compare",
                "runs",
                "--baseline",
                "baseline",
                "--candidate",
                "candidate",
                "--out",
                "comparison",
            ])
            .is_ok()
        );
        assert!(
            LabCli::try_parse_from([
                "lab",
                "compare",
                "readings",
                "--baseline",
                "baseline",
                "--candidate",
                "candidate",
                "--reference",
                "reference",
                "--out",
                "comparison",
            ])
            .is_ok()
        );
        assert!(
            LabCli::try_parse_from([
                "lab",
                "compare",
                "pairs",
                "--run",
                "run",
                "--against",
                "both",
                "--out",
                "comparison",
            ])
            .is_ok()
        );
        assert!(
            LabCli::try_parse_from([
                "lab",
                "compare",
                "runs",
                "--baseline",
                "baseline",
                "--candidate",
                "candidate",
                "--reference-run",
                "reference",
                "--out",
                "comparison",
            ])
            .is_err()
        );
    }

    #[test]
    fn report_sources_require_comparison_subcommand_and_reject_manifest() {
        assert!(LabCli::try_parse_from(["lab", "report"]).is_err());
        assert!(LabCli::try_parse_from(["lab", "report", "comparison"]).is_err());
        assert!(
            LabCli::try_parse_from([
                "lab",
                "report",
                "comparison",
                "--comparison",
                "comparison",
                "--out",
                "report"
            ])
            .is_ok()
        );
        assert!(LabCli::try_parse_from(["lab", "report", "--manifest", "manifest.json"]).is_err());
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
