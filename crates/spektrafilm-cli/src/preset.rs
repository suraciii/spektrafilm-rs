use anyhow::Result;
use clap::{ArgMatches, Args, Subcommand, ValueEnum};
use serde_json::json;
use spektrafilm_core::presets::{PresetGroup, PresetLibrary, PresetSelection};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum PresetCommand {
    /// List presets (default: built-in). The library is CONFIG_DIR/presets, where
    /// CONFIG_DIR is SPEKTRAFILM_CONFIG_DIR or the platform SpektraFilm config directory.
    /// Bare names resolve My Presets first, then Built-in.
    List(PresetList),
    /// Validate and export an exact preset name or a .toml/.json file.
    /// Output carriers: TOML (default) or JSON.
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

#[derive(Clone, Copy, ValueEnum)]
pub enum ListSource {
    BuiltIn,
    User,
    All,
}

#[derive(Args)]
pub struct PresetList {
    /// Listing output format.
    #[arg(long, default_value = "json", value_enum)]
    pub format: ListFormat,
    /// Select groups to list; no other directories are searched.
    #[arg(long, default_value = "built-in", value_enum)]
    pub source: ListSource,
    /// Validate an explicit data directory; listing otherwise needs no data.
    #[arg(long, default_value = "data")]
    pub data_dir: PathBuf,
}

#[derive(Args)]
pub struct PresetShow {
    /// Exact name (optionally user: or built-in:) or .toml/.json file path.
    pub selector: String,
    /// Complete document carrier: TOML or JSON.
    #[arg(long, default_value = "toml", value_enum)]
    pub format: ShowFormat,
    /// Data directory for look profile validation; otherwise environment or discovery.
    #[arg(long, default_value = "data")]
    pub data_dir: PathBuf,
}

pub fn select(library: &PresetLibrary, selector: &str) -> Result<PresetSelection> {
    let selected = library
        .resolve_selector(selector)
        .map_err(anyhow::Error::msg)?;
    if selected.shadowed_builtin {
        eprintln!(
            "preset {:?} resolved to My Presets; shadows Built-in preset {:?}",
            selected.preset.name, selected.preset.name
        );
    }
    Ok(selected)
}

pub fn run(command: PresetCommand, matches: &ArgMatches) -> Result<()> {
    let (_, matches) = matches.subcommand().expect("required preset subcommand");
    let library = PresetLibrary::default();
    match command {
        PresetCommand::List(args) => {
            if crate::explicit_data_selection(matches) {
                crate::select_data_dir(args.data_dir, matches)?;
            }
            let mut records = Vec::new();
            for entry in library.scan().map_err(anyhow::Error::msg)? {
                if !matches!(args.source, ListSource::All)
                    && !matches!(
                        (args.source, entry.group),
                        (ListSource::BuiltIn, PresetGroup::BuiltIn)
                            | (ListSource::User, PresetGroup::User)
                    )
                {
                    continue;
                }
                if let Some(fault) = &entry.fault {
                    eprintln!(
                        "{}: {fault}",
                        entry
                            .path
                            .as_deref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| entry.name.clone())
                    );
                }
                let kind = if entry.preset.is_some() {
                    Some("look")
                } else {
                    None
                };
                let mut record = json!({"name": entry.name, "kind": kind, "group": entry.group});
                if kind == Some("look") {
                    record["film_profile"] = json!(entry.film_profile);
                    record["print_profile"] = json!(entry.print_profile);
                }
                if let Some(path) = entry.path {
                    record["path"] = json!(path);
                }
                if let Some(fault) = entry.fault {
                    record["fault"] = json!(fault);
                }
                records.push(record);
            }
            match args.format {
                ListFormat::Json => println!("{}", serde_json::to_string_pretty(&records)?),
                ListFormat::Text => {
                    for record in records {
                        let fields = record.as_object().expect("preset record object");
                        println!(
                            "{}",
                            fields
                                .iter()
                                .map(|(key, value)| format!("{key}: {value}"))
                                .collect::<Vec<_>>()
                                .join("; ")
                        );
                    }
                }
            }
        }
        PresetCommand::Show(args) => {
            let selected = select(&library, &args.selector)?;
            let preset = selected.preset;
            preset.validate().map_err(anyhow::Error::msg)?;
            let data = crate::select_data_dir(args.data_dir, matches)?;
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
