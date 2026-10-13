use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use spektrafilm_benchmark::{Error, Result, common, compare, documents, report, runner};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name="spektrafilm-bench",version=concat!(env!("CARGO_PKG_VERSION"), " schema-1"),about="Render immutable evidence, compare appearance, and build an offline review.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Schema {
        #[arg(long, value_parser=["suite","target","policy","run","comparison"])]
        kind: String,
    },
    Validate {
        #[arg(long, value_parser=["suite","target","policy","run","comparison"])]
        kind: String,
        #[arg(long)]
        file: PathBuf,
    },
    Run {
        #[arg(long)]
        suite: PathBuf,
        #[arg(long)]
        target: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long = "case")]
        case_ids: Vec<String>,
    },
    Compare {
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        candidate: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        policy: Option<PathBuf>,
        #[arg(long, requires = "policy")]
        gate: bool,
    },
    Report {
        #[arg(long)]
        comparison: PathBuf,
        #[arg(long)]
        out: PathBuf,
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
            _ => return Err(Error::new("argument_syntax", "Unknown document kind")),
        }
    };
    let mut result = json!({"schema_version":1,"command":"validate","execution_status":"completed","kind":kind,"digest":common::digest(&value)?,"unchecked":if kind=="suite"||kind=="target"{json!(["rendering","compute_device"])}else{json!([])}});
    if kind == "suite" {
        result["protocol_digest"] = json!(documents::protocol_digest(&value)?);
    }
    Ok(result)
}
fn execute(command: Command) -> Result<(Value, i32)> {
    match command {
        Command::Schema { kind } => Ok((documents::schema_for(&kind)?, 0)),
        Command::Validate { kind, file } => Ok((validate(&kind, &file.canonicalize()?)?, 0)),
        Command::Run {
            suite,
            target,
            out,
            case_ids,
        } => runner::run(&suite, &target, &out, &case_ids),
        Command::Compare {
            baseline,
            candidate,
            out,
            policy,
            gate,
        } => compare::compare(&baseline, &candidate, &out, policy.as_deref(), gate),
        Command::Report { comparison, out } => {
            let out = runner::absolute(&out)?;
            report::build_report(&comparison, &out)?;
            Ok((
                json!({"schema_version":1,"command":"report","execution_status":"completed","artifact":out.join("index.html")}),
                0,
            ))
        }
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
