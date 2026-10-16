//! `spektrafilm-lab`: the research instrument at f64 reference precision.

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use spektrafilm_benchmark::{
    common, compare, documents, inspect, local_input, report, review, runner, target,
};
use spektrafilm_cli::lab::cli::{
    EvaluationCompareArgs, EvaluationInspectArgs, EvaluationRegionsArgs, EvaluationReportArgs,
    EvaluationRunArgs, ExperimentCommand, SpectralCommand,
};

#[derive(Parser)]
#[command(
    name = "spektrafilm-lab",
    about = "SpektraFilm instrument: film-profile development, calibration, and appearance evaluation at f64 reference precision",
    version,
    after_help = "Generate packaged examples with `spektrafilm-lab fixtures --renderer RENDERER --data-dir DATA --out BUNDLE`, then consume a generated suite with `spektrafilm-lab run --suite BUNDLE/complete.json --target BUNDLE/baseline.json --out RUN`."
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
                "regions", "preparation", "review", "stimuli", "material_measurement",
                "correspondence_review", "experiment", "preflight", "provenance", "report",
                "resolved", "command", "export", "evidence", "metrics"
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
                "preparation", "review", "stimuli", "material_measurement",
                "correspondence_review", "experiment", "run", "comparison"
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
    /// Render a suite, one local image, or a declared stimulus set.
    Run {
        #[command(flatten)]
        args: EvaluationRunArgs,
    },
    /// Compare retained runs, one run's evidence pairs, or two readings.
    Compare {
        #[command(flatten)]
        args: EvaluationCompareArgs,
    },
    /// Read retained measurements without recomputing or invoking a renderer.
    Inspect {
        #[command(flatten)]
        args: EvaluationInspectArgs,
    },
    /// Render one offline report from a retained comparison or a prepared image manifest.
    Report {
        #[command(flatten)]
        args: EvaluationReportArgs,
    },
    /// Derive measurement regions from one grid set of a review manifest.
    Regions {
        #[command(flatten)]
        args: EvaluationRegionsArgs,
    },
    /// Generate the packaged examples: probe assets, suites, targets, and policy.
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
    let value = if matches!(kind, "run" | "comparison") {
        let root = file.parent().ok_or_else(|| {
            spektrafilm_benchmark::Error::new("invalid_path", "Manifest has no parent")
        })?;
        let value = documents::load_json_document(file)?;
        documents::validate_generated_document(kind, &value, root)?;
        value
    } else {
        match kind {
            "suite" => documents::load_suite(file)?,
            "target" => documents::load_target(file)?,
            "policy" => documents::load_policy(file)?,
            "review" => review::validate_manifest(file)?,
            "configuration" | "reference" | "regions" | "preparation" => {
                local_input::load(kind, file)?
            }
            // The authored kinds the instrument owns directly: shape, types,
            // and the local dependencies each one names.
            "stimuli" | "experiment" | "material_measurement" | "correspondence_review" => {
                documents::validate_authored_document(kind, file)?
            }
            _ => {
                return Err(spektrafilm_benchmark::Error::new(
                    "argument_syntax",
                    "Unknown document kind",
                ));
            }
        }
    };
    let unchecked = match kind {
        "review" => serde_json::json!([
            "image_decoding",
            "observed_geometry",
            "display_color_conversion"
        ]),
        "suite"
        | "target"
        | "configuration"
        | "reference"
        | "regions"
        | "stimuli"
        | "experiment"
        | "material_measurement"
        | "correspondence_review" => serde_json::json!([
            "rendering",
            "compute_device",
            "cross_document_applicability"
        ]),
        _ => serde_json::json!([]),
    };
    let mut result = serde_json::json!({"schema_version":1,"command":"validate","execution_status":"completed","kind":kind,"digest":common::digest(&value)?,"unchecked":unchecked});
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
        LabCommands::Schema { kind } => Ok((documents::schema_for(&kind)?, 0)),
        LabCommands::Validate { kind, file } => {
            Ok((validate(&kind, &documents::normalize_path(&file)?)?, 0))
        }
        LabCommands::Target {
            renderer,
            data_dir,
            backend,
            out,
        } => Ok((target::create(&renderer, &data_dir, &backend, &out)?, 0)),
        LabCommands::Run { args } => {
            if let Some(stimuli) = args.stimuli {
                // The chain run shares the `run` command surface: rebuild the
                // measurement arguments from the merged options.
                let enter = args.enter.ok_or_else(|| {
                    BenchError::new("argument_syntax", "--stimuli requires --enter")
                })?;
                let measurement = spektrafilm_cli::lab::cli::MeasureRunArgs {
                    stimuli,
                    enter,
                    read: args.read,
                    dry_run: args.dry_run,
                    out: args.out.clone(),
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
                Ok((value, status))
            } else {
                execute_run(args)
            }
        }
        LabCommands::Compare { args } => {
            if let Some(reading_baseline) = args.reading_baseline {
                let measurement = spektrafilm_cli::lab::cli::MeasureCompareArgs {
                    baseline: reading_baseline,
                    candidate: args.reading_candidate.ok_or_else(|| {
                        BenchError::new("argument_syntax", "Missing candidate measurement")
                    })?,
                    reference: args.reference_run,
                    out: args.out,
                };
                let (value, status) =
                    spektrafilm_cli::lab::measure::compare::compare_with_result(&measurement)
                        .map_err(|error| BenchError::with_message("operation_failed", error))?;
                Ok((value, status))
            } else {
                execute_compare(args)
            }
        }
        LabCommands::Inspect { args } => Ok((
            inspect::inspect(
                &args.comparison,
                &args.case_ids,
                &args.region_ids,
                &args.metrics,
            )?,
            0,
        )),
        LabCommands::Report { args } => {
            let out = documents::normalize_path(&args.out)?;
            if let Some(comparison) = args.comparison {
                Ok((report::build_report(&comparison, &out)?, 0))
            } else {
                let manifest = args.manifest.ok_or_else(|| {
                    spektrafilm_benchmark::Error::new("argument_syntax", "Missing report source")
                })?;
                Ok((review::build_report(&manifest, &out)?, 0))
            }
        }
        LabCommands::Regions { args } => Ok((
            review::derive_regions(&args.manifest, &args.set, &args.out)?,
            0,
        )),
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

fn execute_run(args: EvaluationRunArgs) -> Result<(serde_json::Value, i32), BenchError> {
    let target = args.target.clone().ok_or_else(|| {
        spektrafilm_benchmark::Error::new("argument_syntax", "Missing target document")
    })?;
    if let Some(suite) = args.suite {
        if args.dry_run {
            runner::preflight(
                &suite,
                &target,
                &args.out,
                &args.case_ids,
                args.stage_evidence,
            )
        } else {
            runner::run(
                &suite,
                &target,
                &args.out,
                &args.case_ids,
                args.stage_evidence,
            )
        }
    } else {
        let configuration = args.configuration.ok_or_else(|| {
            spektrafilm_benchmark::Error::new("argument_syntax", "Missing configuration")
        })?;
        let adapter = local_input::LocalInput {
            input: args.input.ok_or_else(|| {
                spektrafilm_benchmark::Error::new("argument_syntax", "Missing input")
            })?,
            configuration: configuration.clone(),
            input_kind: args.input_kind.unwrap_or_else(|| "raster".into()),
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
                &configuration,
                &target,
                &args.out,
                &[],
                args.stage_evidence,
            )
        } else {
            runner::run_suite(
                adapter.suite()?,
                &configuration,
                &target,
                &args.out,
                &[],
                args.stage_evidence,
            )
        }
    }
}

fn execute_compare(args: EvaluationCompareArgs) -> Result<(serde_json::Value, i32), BenchError> {
    if let Some(run) = args.run {
        compare::compare_single(
            &run,
            args.against.as_deref().ok_or_else(|| {
                spektrafilm_benchmark::Error::new("argument_syntax", "Missing against")
            })?,
            &args.out,
        )
    } else {
        compare::compare(
            &args.baseline.ok_or_else(|| {
                spektrafilm_benchmark::Error::new("argument_syntax", "Missing baseline")
            })?,
            &args.candidate.ok_or_else(|| {
                spektrafilm_benchmark::Error::new("argument_syntax", "Missing candidate")
            })?,
            &args.out,
            args.policy.as_deref(),
            args.gate,
        )
    }
}

fn run() -> i32 {
    let matches = LabCli::command().get_matches();
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
            1
        }
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
    fn run_modes_require_exactly_one_input_and_reject_inapplicable_options() {
        let prefix = ["lab", "run", "--target", "target.json", "--out", "run"];
        for suffix in [
            vec![],
            vec![
                "--suite",
                "suite.json",
                "--input",
                "image.png",
                "--configuration",
                "config.json",
            ],
            vec!["--suite", "suite.json", "--seed", "7"],
            vec!["--suite", "suite.json", "--regions", "regions.json"],
            vec!["--input", "image.png"],
            vec![
                "--input",
                "image.png",
                "--configuration",
                "config.json",
                "--case",
                "image",
            ],
            vec![
                "--input",
                "image.png",
                "--configuration",
                "config.json",
                "--reference",
                "ref.png",
            ],
        ] {
            assert!(LabCli::try_parse_from(prefix.into_iter().chain(suffix)).is_err());
        }
        assert!(
            LabCli::try_parse_from(prefix.into_iter().chain([
                "--input",
                "image.png",
                "--configuration",
                "config.json",
                "--input-cctf-decoding",
                "false"
            ]))
            .is_ok()
        );
    }

    #[test]
    fn comparison_modes_reject_mixed_run_version_and_policy_arguments() {
        let prefix = ["lab", "compare", "--out", "comparison"];
        for suffix in [
            vec![],
            vec!["--run", "run"],
            vec!["--against", "input"],
            vec!["--baseline", "baseline"],
            vec![
                "--run",
                "run",
                "--against",
                "both",
                "--candidate",
                "candidate",
            ],
            vec![
                "--run",
                "run",
                "--against",
                "input",
                "--policy",
                "policy.json",
            ],
            vec!["--run", "run", "--against", "input", "--gate"],
        ] {
            assert!(LabCli::try_parse_from(prefix.into_iter().chain(suffix)).is_err());
        }
        assert!(
            LabCli::try_parse_from(
                prefix
                    .into_iter()
                    .chain(["--run", "run", "--against", "both"])
            )
            .is_ok()
        );
        assert!(
            LabCli::try_parse_from(prefix.into_iter().chain([
                "--baseline",
                "baseline",
                "--candidate",
                "candidate"
            ]))
            .is_ok()
        );
    }

    #[test]
    fn report_sources_require_exactly_one_input() {
        let prefix = ["lab", "report", "--out", "report"];
        for suffix in [
            vec![],
            vec!["--comparison", "comparison", "--manifest", "manifest.json"],
        ] {
            assert!(LabCli::try_parse_from(prefix.into_iter().chain(suffix)).is_err());
        }
        assert!(
            LabCli::try_parse_from(prefix.into_iter().chain(["--comparison", "comparison"]))
                .is_ok()
        );
        assert!(
            LabCli::try_parse_from(prefix.into_iter().chain(["--manifest", "manifest.json"]))
                .is_ok()
        );
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
