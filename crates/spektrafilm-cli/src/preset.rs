use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use serde_json::json;
use spektrafilm_core::presets::{builtin_look_presets, load_selector};
use std::path::PathBuf;

use crate::resolve_data_dir;

#[derive(Subcommand)]
pub enum PresetCommand {
    List(PresetList),
    Show(PresetShow),
}

#[derive(Args)]
pub struct PresetList {
    #[arg(long, default_value = "json")]
    pub format: String,
    #[arg(long, default_value = "data", env = "SPEKTRAFILM_DATA_DIR")]
    pub data_dir: PathBuf,
}

#[derive(Args)]
pub struct PresetShow {
    pub selector: String,
    #[arg(long, default_value = "toml")]
    pub format: String,
    #[arg(long, default_value = "data", env = "SPEKTRAFILM_DATA_DIR")]
    pub data_dir: PathBuf,
}

pub fn run(command: PresetCommand) -> Result<()> {
    match command {
        PresetCommand::List(args) => {
            if args.format != "json" {
                bail!("unsupported preset list format {}; use json", args.format);
            }
            let _data_dir = resolve_data_dir(args.data_dir);
            let entries = builtin_look_presets()
                .into_iter()
                .map(|p| {
                    json!({"id": p.id, "name": p.name,
                    "film_profile": p.film_profile, "print_profile": p.print_profile})
                })
                .collect::<Vec<_>>();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        PresetCommand::Show(args) => {
            let data_dir = resolve_data_dir(args.data_dir);
            let preset =
                load_selector(&args.selector, &data_dir).map_err(|e| anyhow::anyhow!(e))?;
            // Resolve validates profile references and complete controls before export.
            preset
                .resolve(
                    &data_dir,
                    &spektrafilm_core::params::RuntimeParams::default(),
                )
                .map_err(|e| anyhow::anyhow!(e))?;
            match args.format.as_str() {
                "toml" => println!("{}", preset.to_toml().map_err(|e| anyhow::anyhow!(e))?),
                "json" => println!("{}", preset.to_json().map_err(|e| anyhow::anyhow!(e))?),
                other => bail!("unsupported preset show format {other}; use toml or json"),
            }
        }
    }
    Ok(())
}
