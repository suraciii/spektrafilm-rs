use anyhow::Result;
use clap::{ArgMatches, Args, Subcommand, ValueEnum};
use serde_json::json;
use spektrafilm_core::presets::{builtin_look_presets, load_selector};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum PresetCommand {
    /// List built-in presets only; does not search user libraries.
    List(PresetList),
    /// Validate and export a built-in preset or .toml/.json preset file.
    Show(PresetShow),
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ListFormat {
    Json,
    Text,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ShowFormat {
    Toml,
    Json,
}

#[derive(Args)]
pub struct PresetList {
    /// Listing output format.
    #[arg(long, default_value = "json", value_enum)]
    pub format: ListFormat,
    /// Validate an explicit data directory; otherwise built-ins need no data.
    #[arg(long, default_value = "data")]
    pub data_dir: PathBuf,
}

#[derive(Args)]
pub struct PresetShow {
    /// Built-in ID (preset list) or .toml/.json preset file path.
    pub selector: String,
    /// Complete preset output format.
    #[arg(long, default_value = "toml", value_enum)]
    pub format: ShowFormat,
    /// Data directory for profile validation; otherwise environment or discovery.
    #[arg(long, default_value = "data")]
    pub data_dir: PathBuf,
}

pub fn run(command: PresetCommand, matches: &ArgMatches) -> Result<()> {
    let (_, matches) = matches.subcommand().expect("required preset subcommand");
    match command {
        PresetCommand::List(args) => {
            if crate::explicit_data_selection(matches) {
                crate::select_data_dir(args.data_dir, matches)?;
            }
            let entries = builtin_look_presets()
                .into_iter()
                .map(|p| {
                    json!({"id": p.id, "name": p.name,
                    "film_profile": p.film_profile, "print_profile": p.print_profile})
                })
                .collect::<Vec<_>>();
            match args.format {
                ListFormat::Json => println!("{}", serde_json::to_string_pretty(&entries)?),
                ListFormat::Text => {
                    for entry in &entries {
                        crate::print_metadata_record(entry);
                    }
                }
            }
        }
        PresetCommand::Show(args) => {
            let data = crate::select_data_dir(args.data_dir, matches)?;
            let preset = load_selector(&args.selector, &data.path).map_err(anyhow::Error::msg)?;
            // Resolve validates profile references and complete controls before export.
            preset
                .resolve(
                    &data.path,
                    &spektrafilm_core::params::RuntimeParams::default(),
                )
                .map_err(anyhow::Error::msg)?;
            match args.format {
                ShowFormat::Toml => println!("{}", preset.to_toml().map_err(anyhow::Error::msg)?),
                ShowFormat::Json => println!("{}", preset.to_json().map_err(anyhow::Error::msg)?),
            }
        }
    }
    Ok(())
}
