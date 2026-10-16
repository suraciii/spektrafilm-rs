//! Command-line surface of the lab binary's spectral group.

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// Spectral diagnostics.
#[derive(Subcommand)]
pub enum SpectralCommand {
    /// Report the roundtrip error of the product path on measured spectra.
    Evaluate(EvaluateArgs),
}

#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("chain").required(true).multiple(false).args(["preset", "profile"])
))]
pub struct EvaluateArgs {
    /// Measured-spectra set JSON manifest; arrays live beside it as .npz/.npy.
    #[arg(long, value_name = "PATH", help_heading = "Evaluation")]
    pub spectra: PathBuf,
    /// Illuminant pair REFERENCE=TARGET: the film reference illuminant and
    /// the scene illuminant that produced the input RGB.
    #[arg(long, value_name = "REFERENCE=TARGET", help_heading = "Evaluation")]
    pub condition: String,
    /// Report destination directory; an existing report is never replaced.
    #[arg(long, value_name = "DIR", help_heading = "Evaluation")]
    pub out: PathBuf,
    /// Exact preset name or .toml/.json file; supplies the film profile.
    #[arg(long, action = clap::ArgAction::Append, help_heading = "Chain")]
    pub preset: Vec<String>,
    /// Film profile JSON path to evaluate.
    #[arg(long, value_name = "PATH", help_heading = "Chain")]
    pub profile: Option<PathBuf>,
    /// Apply PATH=VALUE assignments or a sparse .toml/.json file in order.
    #[arg(long = "set", value_name = "PATH=VALUE|FILE", action = clap::ArgAction::Append, help_heading = "Chain")]
    pub sources: Vec<String>,
    /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
    #[arg(long, default_value = "data", help_heading = "Chain")]
    pub data_dir: PathBuf,
}
