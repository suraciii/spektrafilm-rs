use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use spektrafilm_benchmark::{
    Error, Result, common, compare, documents, inspect, local_input, report, review, runner, target,
};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name="spektrafilm-bench",version=concat!(env!("CARGO_PKG_VERSION"), " schema-1"),about="Render immutable evidence, compare appearance, and build an offline review.", after_help="Generate packaged examples with `spektrafilm-bench-fixtures --help`, then consume a generated suite with `spektrafilm-bench run --suite BUNDLE/complete.json --target BUNDLE/baseline.json --out RUN`.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Print the authoritative schema for an authored or retained document.
    Schema {
        #[arg(long, help="Document kind", value_parser=["suite","target","policy","run","comparison","configuration","reference","regions","preparation","review"])]
        kind: String,
    },
    /// Validate document shape and available local dependencies without rendering.
    Validate {
        #[arg(long, help="Document kind", value_parser=["suite","target","policy","run","comparison","configuration","reference","regions","preparation","review"])]
        kind: String,
        #[arg(long, help = "JSON document to validate")]
        file: PathBuf,
    },
    /// Render a suite or one explicitly declared local image.
    Run {
        #[arg(
            long,
            help = "Suite document",
            required_unless_present = "input",
            conflicts_with = "input"
        )]
        suite: Option<PathBuf>,
        #[arg(
            long,
            help = "Prepared local image",
            required_unless_present = "suite",
            requires = "configuration"
        )]
        input: Option<PathBuf>,
        #[arg(
            long,
            help = "Existing case configuration object",
            requires = "input",
            conflicts_with = "suite"
        )]
        configuration: Option<PathBuf>,
        #[arg(long, help="Explicit source kind (default raster)", value_parser=["raster","raw"], requires="input", conflicts_with="suite")]
        input_kind: Option<String>,
        #[arg(
            long,
            help = "Raster source color space",
            requires = "input",
            conflicts_with = "suite"
        )]
        input_color_space: Option<String>,
        #[arg(long, help="Whether to decode the raster transfer function", value_parser=clap::value_parser!(bool), requires="input", conflicts_with="suite")]
        input_cctf_decoding: Option<bool>,
        #[arg(
            long = "seed",
            help = "Distinct UINT32 seed; repeat to choose a population (default 0)",
            requires = "input",
            conflicts_with = "suite"
        )]
        seeds: Vec<u32>,
        #[arg(
            long,
            help = "Existing fixed region array in render coordinates",
            requires = "input",
            conflicts_with = "suite"
        )]
        regions: Option<PathBuf>,
        #[arg(long, help="Reference image", requires_all=["input","reference_info"], conflicts_with="suite")]
        reference: Option<PathBuf>,
        #[arg(long, help="Existing reference object without path and sha256", requires_all=["input","reference"], conflicts_with="suite")]
        reference_info: Option<PathBuf>,
        #[arg(
            long,
            help = "Passive preparation provenance object",
            requires = "input",
            conflicts_with = "suite"
        )]
        preparation: Option<PathBuf>,
        #[arg(long, help = "Renderer target document")]
        target: PathBuf,
        #[arg(long, help = "New run directory; must not exist")]
        out: PathBuf,
        #[arg(
            long = "case",
            help = "Exact suite case identity; repeat in suite mode",
            requires = "suite",
            conflicts_with = "input"
        )]
        case_ids: Vec<String>,
        #[arg(
            long,
            help = "Resolve all selected configurations without decoding or rendering; retain preflight.json"
        )]
        dry_run: bool,
        #[arg(
            long,
            help = "Retain native CPU stage buffers, applied exposure, and prepared print normalization"
        )]
        stage_evidence: bool,
    },
    /// Compare retained runs or input/reference against one run's simulation.
    Compare {
        #[arg(
            long,
            help = "Baseline run directory",
            required_unless_present = "run",
            requires = "candidate",
            conflicts_with = "run"
        )]
        baseline: Option<PathBuf>,
        #[arg(
            long,
            help = "Candidate run directory",
            required_unless_present = "run",
            requires = "baseline",
            conflicts_with = "run"
        )]
        candidate: Option<PathBuf>,
        #[arg(long, help="Single run directory", requires="against", conflicts_with_all=["baseline","candidate","policy","gate"])]
        run: Option<PathBuf>,
        #[arg(long, help="Evidence pairs to compare", value_parser=["input","reference","both"], requires="run")]
        against: Option<String>,
        #[arg(long, help = "New comparison directory; must not exist")]
        out: PathBuf,
        #[arg(
            long,
            help = "Version-comparison policy document",
            conflicts_with = "run"
        )]
        policy: Option<PathBuf>,
        #[arg(
            long,
            help = "Fail command when policy does not pass",
            requires = "policy",
            conflicts_with = "run"
        )]
        gate: bool,
    },
    /// Read retained measurements without recomputing or invoking a renderer.
    Inspect {
        #[arg(long, help = "Comparison directory")]
        comparison: PathBuf,
        #[arg(long = "case", help = "Exact case identity; repeat to select multiple")]
        case_ids: Vec<String>,
        #[arg(
            long = "region",
            help = "Exact region identity; repeat to select multiple"
        )]
        region_ids: Vec<String>,
        #[arg(
            long = "metric",
            help = "Exact metric identity; repeat to select multiple"
        )]
        metrics: Vec<String>,
    },
    /// Pin a renderer executable in an existing target document.
    Target {
        #[arg(long, help = "Renderer executable")]
        renderer: PathBuf,
        #[arg(long, help = "Explicit profile/data directory")]
        data_dir: PathBuf,
        #[arg(long, help = "Requested compute backend; availability is checked during rendering", value_parser = ["cpu", "gpu"])]
        backend: String,
        #[arg(long, help = "New target JSON file; must not exist")]
        out: PathBuf,
    },
    /// Publish an offline review of retained comparison evidence.
    Report {
        #[arg(long, help = "Comparison directory")]
        comparison: PathBuf,
        #[arg(long, help = "New report directory; must not exist")]
        out: PathBuf,
        #[arg(long, help = "Review layout", value_parser = ["standard", "diagnostic"], default_value = "standard")]
        layout: String,
        #[arg(
            long = "focus-region",
            help = "Exact region to feature in the diagnostic layout; repeat for multiple regions"
        )]
        focus_regions: Vec<String>,
    },
    /// Build an offline visual comparison of explicitly prepared chart images.
    Review {
        #[arg(
            long,
            help = "Chart manifest with image declarations, grids, and captions"
        )]
        manifest: PathBuf,
        #[arg(long, help = "New report directory; must not exist")]
        out: PathBuf,
        #[arg(long, help = "Override the default left image ID", requires = "right")]
        left: Option<String>,
        #[arg(long, help = "Override the default right image ID", requires = "left")]
        right: Option<String>,
    },
    /// Derive measurement regions from one grid set of a review manifest.
    Regions {
        #[arg(long, help = "Review manifest with the declared grid set")]
        review: PathBuf,
        #[arg(long, help = "Grid set identity to derive")]
        set: String,
        #[arg(long, help = "New region array JSON file; must not exist")]
        out: PathBuf,
        #[arg(
            long,
            help = "Sample window WIDTHxHEIGHT centered inside each patch window"
        )]
        sample: Option<String>,
        #[arg(long, help = "Region role", default_value = "other")]
        role: String,
        #[arg(long, help = "Measurement families, comma separated (default none)")]
        measurements: Option<String>,
    },
}
fn emit(value: &Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}
fn validate(kind: &str, file: &Path) -> Result<Value> {
    let value = if matches!(kind, "run" | "comparison") {
        let root = file
            .parent()
            .ok_or_else(|| Error::new("invalid_path", "Manifest has no parent"))?;
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
            _ => return Err(Error::new("argument_syntax", "Unknown document kind")),
        }
    };
    let unchecked = match kind {
        "review" => json!([
            "image_decoding",
            "observed_geometry",
            "display_color_conversion"
        ]),
        "suite" | "target" | "configuration" | "reference" | "regions" => {
            json!([
                "rendering",
                "compute_device",
                "cross_document_applicability"
            ])
        }
        _ => json!([]),
    };
    let mut result = json!({"schema_version":1,"command":"validate","execution_status":"completed","kind":kind,"digest":common::digest(&value)?,"unchecked":unchecked});
    if kind == "suite" {
        result["protocol_digest"] = json!(documents::protocol_digest(&value)?);
    }
    Ok(result)
}
fn measurement_selection(value: Option<&str>) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mut selected = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() || selected.iter().any(|existing| existing == part) {
            return Err(Error::new(
                "invalid_measurements",
                format!("measurement families must be distinct nonempty names: {value}"),
            ));
        }
        selected.push(part.to_owned());
    }
    Ok(selected)
}
fn execute(command: Command) -> Result<(Value, i32)> {
    match command {
        Command::Schema { kind } => Ok((documents::schema_for(&kind)?, 0)),
        Command::Validate { kind, file } => {
            Ok((validate(&kind, &documents::normalize_path(&file)?)?, 0))
        }
        Command::Run {
            suite,
            input,
            configuration,
            input_kind,
            input_color_space,
            input_cctf_decoding,
            seeds,
            regions,
            reference,
            reference_info,
            preparation,
            target,
            out,
            case_ids,
            dry_run,
            stage_evidence,
        } => {
            if let Some(suite) = suite {
                if dry_run {
                    runner::preflight(&suite, &target, &out, &case_ids, stage_evidence)
                } else {
                    runner::run(&suite, &target, &out, &case_ids, stage_evidence)
                }
            } else {
                let configuration = configuration
                    .ok_or_else(|| Error::new("argument_syntax", "Missing configuration"))?;
                let adapter = local_input::LocalInput {
                    input: input.ok_or_else(|| Error::new("argument_syntax", "Missing input"))?,
                    configuration: configuration.clone(),
                    input_kind: input_kind.unwrap_or_else(|| "raster".into()),
                    input_color_space,
                    input_cctf_decoding,
                    seeds,
                    regions,
                    reference,
                    reference_info,
                    preparation,
                };
                if dry_run {
                    runner::preflight_suite(
                        adapter.suite()?,
                        &configuration,
                        &target,
                        &out,
                        &[],
                        stage_evidence,
                    )
                } else {
                    runner::run_suite(
                        adapter.suite()?,
                        &configuration,
                        &target,
                        &out,
                        &[],
                        stage_evidence,
                    )
                }
            }
        }
        Command::Compare {
            baseline,
            candidate,
            run,
            against,
            out,
            policy,
            gate,
        } => {
            if let Some(run) = run {
                compare::compare_single(
                    &run,
                    against
                        .as_deref()
                        .ok_or_else(|| Error::new("argument_syntax", "Missing against"))?,
                    &out,
                )
            } else {
                compare::compare(
                    &baseline.ok_or_else(|| Error::new("argument_syntax", "Missing baseline"))?,
                    &candidate.ok_or_else(|| Error::new("argument_syntax", "Missing candidate"))?,
                    &out,
                    policy.as_deref(),
                    gate,
                )
            }
        }
        Command::Inspect {
            comparison,
            case_ids,
            region_ids,
            metrics,
        } => Ok((
            inspect::inspect(&comparison, &case_ids, &region_ids, &metrics)?,
            0,
        )),
        Command::Target {
            renderer,
            data_dir,
            backend,
            out,
        } => Ok((target::create(&renderer, &data_dir, &backend, &out)?, 0)),
        Command::Report {
            comparison,
            out,
            layout,
            focus_regions,
        } => {
            let layout = if layout == "diagnostic" {
                report::Layout::Diagnostic
            } else {
                report::Layout::Standard
            };
            let out = documents::normalize_path(&out)?;
            Ok((
                report::build_report(&comparison, &out, layout, &focus_regions)?,
                0,
            ))
        }
        Command::Review {
            manifest,
            out,
            left,
            right,
        } => Ok((
            review::build_report(&manifest, &out, left.as_deref(), right.as_deref())?,
            0,
        )),
        Command::Regions {
            review,
            set,
            out,
            sample,
            role,
            measurements,
        } => Ok((
            review::derive_regions(
                &review,
                &set,
                &out,
                sample.as_deref(),
                &role,
                &measurement_selection(measurements.as_deref())?,
            )?,
            0,
        )),
    }
}
fn run() -> i32 {
    let cli = Cli::parse();
    let name = match &cli.command {
        Command::Schema { .. } => "schema",
        Command::Validate { .. } => "validate",
        Command::Run { .. } => "run",
        Command::Compare { .. } => "compare",
        Command::Report { .. } => "report",
        Command::Review { .. } => "review",
        Command::Regions { .. } => "regions",
        Command::Inspect { .. } => "inspect",
        Command::Target { .. } => "target",
    };
    match execute(cli.command) {
        Ok((value, status)) => {
            emit(&value);
            status
        }
        Err(error) => {
            eprintln!("{error}");
            emit(
                &json!({"schema_version":1,"command":name,"execution_status":"failed","errors":[error.to_value()]}),
            );
            1
        }
    }
}
fn main() {
    std::process::exit(run())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_modes_require_exactly_one_input_and_reject_inapplicable_options() {
        let prefix = ["bench", "run", "--target", "target.json", "--out", "run"];
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
            assert!(Cli::try_parse_from(prefix.into_iter().chain(suffix)).is_err());
        }
        assert!(
            Cli::try_parse_from(prefix.into_iter().chain([
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
        let prefix = ["bench", "compare", "--out", "comparison"];
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
            assert!(Cli::try_parse_from(prefix.into_iter().chain(suffix)).is_err());
        }
        assert!(
            Cli::try_parse_from(
                prefix
                    .into_iter()
                    .chain(["--run", "run", "--against", "both"])
            )
            .is_ok()
        );
        assert!(
            Cli::try_parse_from(prefix.into_iter().chain([
                "--baseline",
                "baseline",
                "--candidate",
                "candidate"
            ]))
            .is_ok()
        );
    }
}
