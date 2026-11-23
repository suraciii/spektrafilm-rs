//! The reading and reporting endpoints the lab exposes beside its own
//! measurement endpoints.
//!
//! Every one of them reads a retained record and writes another, which is the
//! lab's job: join two reading records, project a comparison's evidence, or
//! build a page. `compare` alone compares two delivered images; `compare
//! readings` joins records.

use clap::{Args, Subcommand};
use serde_json::Value;
use spektrafilm_lab::{Error, Result, inspect, photo_report, readings_compare, sample};
use std::path::PathBuf;

/// A comparison of retained records rather than delivered images.
#[derive(Subcommand)]
pub enum CompareCommand {
    /// Join two retained reading records.
    Readings(readings_compare::MeasureCompareArgs),
}

#[derive(Args)]
pub struct InspectArgs {
    /// Retained comparison directory.
    #[arg(long, value_name = "DIR")]
    pub comparison: PathBuf,
    /// Exact case identities to select; repeats.
    #[arg(long = "case", value_name = "ID")]
    pub case_ids: Vec<String>,
    /// Exact region identities to select; repeats.
    #[arg(long = "region", value_name = "ID")]
    pub region_ids: Vec<String>,
    /// Exact metric identities to select; repeats.
    #[arg(long = "metric", value_name = "ID")]
    pub metrics: Vec<String>,
    /// Which source the row was measured on; repeats.
    #[arg(long = "side", value_name = "SIDE")]
    pub sides: Vec<String>,
}

#[derive(Subcommand)]
pub enum ReportCommand {
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

/// Run one record-level endpoint and return its document and exit status.
pub fn compare_runs(args: &CompareCommand) -> Result<(Value, i32)> {
    match args {
        CompareCommand::Readings(args) => readings_compare::compare_with_result(args)
            .map_err(|error| Error::with_message("operation_failed", error)),
    }
}

pub fn sample(args: &sample::MeasureSampleArgs) -> Result<(Value, i32)> {
    sample::sample_with_result(args).map_err(|error| Error::with_message("operation_failed", error))
}

pub fn inspect(args: &InspectArgs) -> Result<(Value, i32)> {
    inspect::inspect(
        &args.comparison,
        &args.case_ids,
        &args.region_ids,
        &args.metrics,
        &args.sides,
    )
    .map(|value| (value, 0))
}

pub fn report(command: &ReportCommand) -> Result<(Value, i32)> {
    match command {
        ReportCommand::Comparison { comparison, out } => {
            spektrafilm_lab::documents::normalize_path(out)
                .and_then(|out| spektrafilm_lab::report::build_report(comparison, &out))
                .map(|value| (value, 0))
        }
        ReportCommand::Photos { manifest, out } => {
            Ok((photo_report::build_report(manifest, out)?, 0))
        }
    }
}
