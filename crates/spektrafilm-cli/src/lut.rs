use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Map, Value, json};
use spektrafilm_core::lut_baker::{BundleBuilder, BundleSpec, Headroom, Topology};
use spektrafilm_core::lut_delivery::{self, DeliveryOptions};
use spektrafilm_core::{lut_transport, profile};

#[derive(Subcommand)]
pub(crate) enum LutCommand {
    /// Bake a deterministic LUT bundle. CLI flags override a flat TOML BundleSpec.
    Build(BuildArgs),
    /// List sorted stock names or canonical color spaces and their short tags.
    List {
        #[arg(value_enum)]
        kind: ListKind,
        #[arg(long, default_value = "data", env = "SPEKTRAFILM_DATA_DIR")]
        data_dir: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum ListKind {
    Film,
    Print,
    Input,
    Output,
    Target,
}

#[derive(Args)]
pub(crate) struct BuildArgs {
    /// Load flat BundleSpec fields, including gamut-compression tables, from TOML.
    #[arg(long = "from", value_name = "FILE")]
    from_toml: Option<PathBuf>,
    /// Bundle name; computed from the stocks and color spaces when omitted.
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    film: Option<String>,
    /// Print stock; repeat this flag for multi-print bundles.
    #[arg(long = "print", value_name = "PRINT")]
    prints: Vec<String>,
    /// Input color space canonical name or registry short tag.
    #[arg(long = "input")]
    input_cs: Option<String>,
    /// Output color space canonical name or registry short tag.
    #[arg(long = "output")]
    output_cs: Option<String>,
    #[arg(long, value_parser = ["1lut", "2lut", "3lut", "4lut"])]
    topology: Option<String>,
    #[arg(long, value_name = "N")]
    resolution: Option<usize>,
    #[arg(long, value_name = "NAME")]
    target: Option<String>,
    #[arg(long, value_parser = ["directory", "zip"])]
    container: Option<String>,
    /// Place encoded source 1.0 at 0.18 * 2**STOPS; omitted uses native log/HDR
    /// headroom or four stops for encoded SDR, unless overridden by TOML.
    #[arg(long = "stops-above-gray", value_name = "STOPS")]
    stops_above_midgray: Option<f64>,
    #[arg(long)]
    qa: bool,
    #[arg(long, value_name = "I")]
    qa_print_index: Option<usize>,
    #[arg(long)]
    ocio_config: bool,
    /// Include every contiguous multi-stage sub-chain as a collapsed cube.
    #[arg(long)]
    combinations: bool,
    /// Write the bundle inside DIR/<bundle-name>/.
    #[arg(long, value_name = "DIR")]
    out: PathBuf,
    #[arg(long, default_value = "data", env = "SPEKTRAFILM_DATA_DIR")]
    data_dir: PathBuf,
}

pub(crate) fn run(command: LutCommand) -> Result<()> {
    match command {
        LutCommand::Build(mut args) => {
            args.data_dir = crate::resolve_data_dir(args.data_dir);
            build(args)
        }
        LutCommand::List { kind, data_dir } => {
            let data_dir = crate::resolve_data_dir(data_dir);
            list(kind, &data_dir)
        }
    }
}

fn build(args: BuildArgs) -> Result<()> {
    let mut fields = if let Some(path) = &args.from_toml {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading BundleSpec TOML: {}", path.display()))?;
        let toml: toml::Value = toml::from_str(&text)
            .with_context(|| format!("parsing BundleSpec TOML: {}", path.display()))?;
        match serde_json::to_value(toml)? {
            Value::Object(fields) => fields,
            _ => bail!("BundleSpec TOML must contain fields at the document root"),
        }
    } else {
        Map::new()
    };
    for (field, value) in [
        ("name", json!(args.name)),
        ("film_profile", json!(args.film)),
        ("input_color_space", json!(args.input_cs)),
        ("output_color_space", json!(args.output_cs)),
        ("topology", json!(args.topology)),
        ("resolution", json!(args.resolution)),
        ("target", json!(args.target)),
        ("container", json!(args.container)),
        ("stops_above_midgray", json!(args.stops_above_midgray)),
        ("qa_print_index", json!(args.qa_print_index)),
    ] {
        if !value.is_null() {
            fields.insert(field.into(), value);
        }
    }
    if !args.prints.is_empty() {
        fields.insert("print_profiles".into(), json!(args.prints));
    }
    for (field, enabled) in [
        ("qa", args.qa),
        ("ocio_config", args.ocio_config),
        ("include_combinations", args.combinations),
    ] {
        if enabled {
            fields.insert(field.into(), Value::Bool(true));
        }
    }
    for (field, flag) in [
        ("film_profile", "--film"),
        ("print_profiles", "--print"),
        ("input_color_space", "--input"),
        ("output_color_space", "--output"),
    ] {
        if !fields.contains_key(field) {
            bail!("missing {flag} (or `{field}` in TOML)");
        }
    }
    let mut spec: BundleSpec = serde_json::from_value(Value::Object(fields))
        .context("invalid BundleSpec fields")?;
    spec.normalize().map_err(anyhow::Error::msg)?;
    lut_delivery::validate_target(&spec)?;
    let backend = spektrafilm_gpu::select_backend();
    let bundle = BundleBuilder::new(spec)
        .build(&args.data_dir, backend.as_ref())
        .map_err(anyhow::Error::msg)?;
    let out = args.out.join(&bundle.spec.name);
    let mut meta = lut_delivery::write_bundle_files(&bundle, &out, &DeliveryOptions::default())?;
    if bundle.spec.ocio_config {
        match spektrafilm_core::lut_ocio::emit(&out, &bundle, &meta)? {
            spektrafilm_core::lut_ocio::OcioEmission::Written(artifact) => {
                lut_delivery::append_artifact(&mut meta, artifact)?;
            }
            spektrafilm_core::lut_ocio::OcioEmission::Skipped { reason } => {
                println!("[ocio] SKIP: {reason}");
            }
        }
    }
    if bundle.spec.qa {
        let qa_root = out.join("qa");
        let report = spektrafilm_core::lut_qa::run(&bundle, &args.data_dir, backend.as_ref(), &qa_root)
            .map_err(anyhow::Error::msg)?;
        for path in spektrafilm_core::lut_qa::write_report(&report, &qa_root).map_err(anyhow::Error::msg)? {
            lut_delivery::append_artifact(&mut meta, lut_delivery::ArtifactReference {
                path: format!("qa/{}", path.to_string_lossy().replace('\\', "/")),
                kind: "qa".into(),
                description: "Pinned Python 0.3.4 LUT quality assessment".into(),
            })?;
        }
        lut_delivery::append_quality_summary(&out, &report)?;
        println!("[qa] {}", if report.passed { "PASS" } else { "FAIL" });
    }
    lut_delivery::finalize_bundle(&out, &meta, bundle.spec.container == "zip")?;
    println!("[done] {}", out.display());
    Ok(())
}


fn list(kind: ListKind, data_dir: &Path) -> Result<()> {
    match kind {
        ListKind::Film | ListKind::Print => {
            let stage = if matches!(kind, ListKind::Film) { "filming" } else { "printing" };
            let directory = data_dir.join("profiles");
            let mut names = Vec::new();
            for entry in std::fs::read_dir(&directory)
                .with_context(|| format!("reading profiles: {}", directory.display()))?
            {
                let path = entry?.path();
                if path.extension().and_then(|v| v.to_str()) != Some("json") {
                    continue;
                }
                let data: Value = serde_json::from_slice(&std::fs::read(&path)?)
                    .with_context(|| format!("reading profile metadata: {}", path.display()))?;
                if data["info"]["stage"].as_str() == Some(stage) {
                    let name = data["info"]["stock"].as_str()
                        .or_else(|| path.file_stem().and_then(|v| v.to_str()))
                        .context("profile filename is not valid UTF-8")?;
                    names.push(name.to_owned());
                }
            }
            names.sort();
            for name in names {
                println!("{name}");
            }
        }
        ListKind::Input | ListKind::Output => {
            let mut entries: Vec<_> = lut_transport::registry().iter()
                .filter(|entry| if matches!(kind, ListKind::Input) { entry.input } else { entry.output })
                .collect();
            entries.sort_by_key(|entry| entry.name);
            let width = entries.iter().map(|entry| entry.name.len()).max().unwrap_or(0);
            for entry in entries {
                println!("{:<width$}  {}", entry.name, entry.short_tag);
            }
        }
        ListKind::Target => {
            let mut names: Vec<_> = lut_delivery::list_targets().iter()
                .map(|target| target.name).collect();
            names.sort();
            for name in names {
                println!("{name}");
            }
        }
    }
    Ok(())
}

/// Compatibility export uses the canonical one-stage baker with encoded
/// ProPhoto RGB input, encoded sRGB output, and the input's native linear scale.
pub(crate) fn export_lut(
    film_name: &str,
    paper_name: Option<&str>,
    resolution: u32,
    output: &Path,
    data_dir: &Path,
) -> Result<()> {
    let print = if let Some(name) = paper_name {
        name.to_owned()
    } else {
        let film = profile::load_profile_by_name(data_dir, film_name)
            .with_context(|| format!("loading film profile: {film_name}"))?;
        film.info.target_print.context("no paper specified and film has no target_print; use --paper")?
    };
    let spec: BundleSpec = serde_json::from_value(json!({
        "film_profile": film_name,
        "print_profiles": [print],
        "input_color_space": "ProPhoto RGB",
        "output_color_space": "sRGB",
        "topology": Topology::One,
        "resolution": resolution,
        "stops_above_midgray": Headroom::Native(()),
    }))?;
    let backend = spektrafilm_gpu::select_backend();
    let bundle = BundleBuilder::new(spec).build(data_dir, backend.as_ref())
        .map_err(anyhow::Error::msg)?;
    let (_, lut) = bundle.luts.first().context("canonical baker returned no LUT")?;
    lut.write_cube(output).map_err(anyhow::Error::msg)?;
    eprintln!("LUT saved: {}", output.display());
    Ok(())
}
