//! Shared command plumbing for the SpektraFilm binaries.
//!
//! The user binaries (`spektrafilm`, `spektrafilm-f64`) and the development
//! `spektrafilm-lab` binary share these modules; only the lab binary is
//! gated on the `precision-f64` feature. Each binary's own `main` owns its
//! command surface.

pub mod contract;
#[cfg(feature = "precision-f64")]
pub mod fit;
#[cfg(feature = "precision-f64")]
pub mod lab;
pub mod lut;
pub mod preset;
pub mod process;
mod process_report;
pub mod runner;
mod telemetry;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;

/// Compute backend selection shared by the user binaries.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Backend {
    Cpu,
    Gpu,
}

/// Discovery output format shared by the user binaries.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum DiscoveryFormat {
    Json,
    Text,
}

pub struct DataSelection {
    pub path: PathBuf,
    pub source: &'static str,
}

pub(crate) fn explicit_data_selection(matches: &clap::ArgMatches) -> bool {
    matches.value_source("data_dir") == Some(clap::parser::ValueSource::CommandLine)
        || std::env::var_os("SPEKTRAFILM_DATA_DIR").is_some()
}

pub fn select_data_dir(argument: PathBuf, matches: &clap::ArgMatches) -> Result<DataSelection> {
    let explicit = matches.value_source("data_dir") == Some(clap::parser::ValueSource::CommandLine);
    select_data_dir_impl(argument, explicit)
}

/// Resolve the data directory without requiring `ArgMatches`.
/// Shared with the lab module, which cannot construct `ArgMatches`.
pub(crate) fn select_data_dir_impl(argument: PathBuf, explicit: bool) -> Result<DataSelection> {
    let selection = if explicit {
        Some((argument, "argument", "--data-dir"))
    } else if let Some(value) = std::env::var_os("SPEKTRAFILM_DATA_DIR") {
        if value.is_empty() {
            bail!("SPEKTRAFILM_DATA_DIR is empty; set it to a data directory or supply --data-dir");
        }
        Some((PathBuf::from(value), "environment", "SPEKTRAFILM_DATA_DIR"))
    } else {
        None
    };
    if let Some((path, source, option)) = selection {
        if !path.is_dir() {
            bail!(
                "{option} selected {} which is not an existing directory; supply a valid data directory",
                path.display()
            );
        }
        return Ok(DataSelection {
            path: fs::canonicalize(&path)
                .with_context(|| format!("resolving data directory: {}", path.display()))?,
            source,
        });
    }
    let executable = std::env::current_exe().context("locating executable for data discovery")?;
    let directory = executable
        .parent()
        .context("executable has no parent directory")?;
    let candidates = [
        directory.join("data"),
        directory.join("../share/data"),
        directory.join("../Resources/data"),
        std::env::current_dir()?.join("data"),
    ];
    for path in &candidates {
        if path.is_dir() {
            return Ok(DataSelection {
                path: fs::canonicalize(path)
                    .with_context(|| format!("resolving data directory: {}", path.display()))?,
                source: "automatic",
            });
        }
    }
    bail!(
        "no data directory found; tried {}; set --data-dir or SPEKTRAFILM_DATA_DIR",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
}
