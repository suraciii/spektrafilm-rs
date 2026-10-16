//! `spektrafilm-lab`: development command surface at f64 reference precision.

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use spektrafilm_cli::fit;
use spektrafilm_cli::lab::cli::{MeasureCommand, SpectralCommand};

#[derive(Parser)]
#[command(
    name = "spektrafilm-lab",
    about = "SpektraFilm development commands (f64 reference precision)",
    version
)]
struct LabCli {
    #[command(subcommand)]
    command: LabCommands,
}

#[derive(Subcommand)]
// Clap subcommands are built once at startup, outside hot paths.
#[allow(clippy::large_enum_variant)]
enum LabCommands {
    /// Fit profile and look parameters against reference patch data.
    Fit(fit::FitArgs),
    /// Spectral diagnostics.
    Spectral {
        #[command(subcommand)]
        command: SpectralCommand,
    },
    /// Development measurement of the executed chain.
    Measure {
        #[command(subcommand)]
        command: MeasureCommand,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let matches = LabCli::command().get_matches();
    let cli = LabCli::from_arg_matches(&matches)?;
    match cli.command {
        LabCommands::Fit(args) => {
            let (_, sub_matches) = matches.subcommand().expect("required subcommand");
            fit::cmd_fit(&args, sub_matches)?;
        }
        LabCommands::Spectral { command } => {
            let (_, sub_matches) = matches.subcommand().expect("required subcommand");
            let (_, evaluate_matches) = sub_matches.subcommand().expect("required subcommand");
            match command {
                SpectralCommand::Evaluate(args) => {
                    spektrafilm_cli::lab::evaluate::run(&args, evaluate_matches)?;
                }
            }
        }
        LabCommands::Measure { command } => {
            let (_, sub_matches) = matches.subcommand().expect("required subcommand");
            let (_, measure_matches) = sub_matches.subcommand().expect("required subcommand");
            match command {
                MeasureCommand::Run(args) => {
                    spektrafilm_cli::lab::measure::run::run(&args, measure_matches)?;
                }
                MeasureCommand::Sample(args) => {
                    spektrafilm_cli::lab::measure::sample::sample(&args)?;
                }
                MeasureCommand::Compare(args) => {
                    spektrafilm_cli::lab::measure::compare::compare(&args)?;
                }
                MeasureCommand::Verify(args) => {
                    spektrafilm_cli::lab::measure::verify::verify(&args, measure_matches)?;
                }
            }
        }
    }
    Ok(())
}
