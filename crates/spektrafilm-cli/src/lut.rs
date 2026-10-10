use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{ArgMatches, Args, Subcommand, ValueEnum};
use serde_json::{Map, Value, json};
use spektrafilm_core::lut_baker::{BundleBuilder, BundleSpec, Topology};
use spektrafilm_core::lut_delivery::{self, DeliveryOptions};
use spektrafilm_core::params::RuntimeParams;
use spektrafilm_core::{lut_transport, profile};

#[derive(Subcommand)]
pub(crate) enum LutCommand {
    /// Bake a deterministic LUT bundle. CLI flags override a flat TOML BundleSpec.
    Build(BuildArgs),
    /// List sorted stock names or canonical color spaces and their short tags.
    List {
        /// Registry to list: stocks, transport color spaces, or delivery targets.
        #[arg(value_enum)]
        kind: ListKind,
        /// Data directory for stock listings; otherwise environment or discovery.
        #[arg(long, default_value = "data")]
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
#[command(
    after_long_help = "Examples:\n  spektrafilm lut build build/lut_bundles --film kodak_portra_400 --print kodak_portra_endura --input vlog --output srgb\n  spektrafilm lut list film\n  spektrafilm lut list input\n  spektrafilm lut list target"
)]
pub(crate) struct BuildArgs {
    /// Load flat BundleSpec fields, including gamut-compression tables, from TOML.
    #[arg(
        long = "from",
        value_name = "FILE",
        help_heading = "Bundle configuration"
    )]
    from_toml: Option<PathBuf>,
    /// Bundle name; computed from stocks and color spaces when omitted.
    #[arg(long, help_heading = "Bundle configuration")]
    name: Option<String>,
    /// Film stock ID; discover with lut list film.
    #[arg(long, help_heading = "Bundle configuration")]
    film: Option<String>,
    /// Print stock ID (lut list print); repeat for multi-print bundles.
    #[arg(
        long = "print",
        value_name = "PRINT",
        help_heading = "Bundle configuration"
    )]
    prints: Vec<String>,
    /// Input color space name or short tag; discover with lut list input.
    #[arg(long = "input", help_heading = "Bundle configuration")]
    input_cs: Option<String>,
    /// Output color space name or short tag; discover with lut list output.
    #[arg(long = "output", help_heading = "Bundle configuration")]
    output_cs: Option<String>,
    /// Number of stages in the LUT topology.
    #[arg(long, value_parser = ["1lut", "2lut", "3lut", "4lut"], help_heading = "Bundle configuration")]
    topology: Option<String>,
    /// Cube edge resolution in samples.
    #[arg(long, value_name = "N", help_heading = "Bundle configuration")]
    resolution: Option<usize>,
    /// Delivery target name; discover with lut list target.
    #[arg(long, value_name = "NAME", help_heading = "Delivery")]
    target: Option<String>,
    /// Bundle delivery container.
    #[arg(long, value_parser = ["directory", "zip"], help_heading = "Delivery")]
    container: Option<String>,
    /// Upstream input exposure stops (auto, native, null, or finite number).
    #[arg(
        long = "stops-above-midgray",
        value_name = "STOPS",
        allow_negative_numbers = true,
        help_heading = "Bundle configuration"
    )]
    stops_above_midgray: Option<String>,
    /// Legacy linear input exposure adjustment in EV.
    #[arg(
        long = "exposure-ev",
        value_name = "EV",
        allow_negative_numbers = true,
        help_heading = "Bundle configuration"
    )]
    exposure_ev: Option<f64>,
    /// Run pinned quality assessment after baking.
    #[arg(long, help_heading = "Quality assessment")]
    qa: bool,
    /// Zero-based print index to assess; otherwise assess all prints with --qa.
    #[arg(long, value_name = "I", help_heading = "Quality assessment")]
    qa_print_index: Option<usize>,
    /// Emit an OCIO configuration when the bundle supports it.
    #[arg(long, help_heading = "Delivery")]
    ocio_config: bool,
    /// Include contiguous multi-stage sub-chains as collapsed cubes.
    #[arg(long, help_heading = "Bundle configuration")]
    combinations: bool,
    /// Captured RuntimeParams JSON snapshot to use as the bake source.
    #[arg(long, value_name = "FILE", help_heading = "Bundle configuration")]
    params: Option<PathBuf>,
    /// Parent directory of the generated OUT/<bundle-name>/ bundle.
    #[arg(value_name = "OUT", help_heading = "Delivery")]
    out: PathBuf,
    /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
    #[arg(long, default_value = "data", help_heading = "Bundle configuration")]
    data_dir: PathBuf,
}

pub(crate) fn run(command: LutCommand, matches: &ArgMatches) -> Result<()> {
    let (_, matches) = matches.subcommand().expect("required lut subcommand");
    match command {
        LutCommand::Build(mut args) => {
            args.data_dir = crate::select_data_dir(args.data_dir, matches)?.path;
            build(args)
        }
        LutCommand::List { kind, data_dir } => {
            if matches!(kind, ListKind::Film | ListKind::Print)
                || crate::explicit_data_selection(matches)
            {
                list(kind, &crate::select_data_dir(data_dir, matches)?.path)
            } else {
                list(kind, &data_dir)
            }
        }
    }
}

/// Merge TOML and CLI overrides while leaving exposure normalization to the builder.
fn load_bundle_spec(args: &BuildArgs) -> Result<BundleSpec> {
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
    if let Some(value) = args.stops_above_midgray.as_deref()
        && value
            .parse::<f64>()
            .is_ok_and(|stops| !stops.is_finite())
    {
        bail!("--stops-above-midgray must be auto, native, null, or a finite number: {value}");
    }
    for (field, value) in [
        ("name", json!(args.name)),
        ("film_profile", json!(args.film)),
        ("input_color_space", json!(args.input_cs)),
        ("output_color_space", json!(args.output_cs)),
        ("topology", json!(args.topology)),
        ("resolution", json!(args.resolution)),
        ("target", json!(args.target)),
        ("container", json!(args.container)),
        (
            "stops_above_midgray",
            args.stops_above_midgray
                .as_deref()
                .map(|value| {
                    value.parse::<f64>().map_or_else(
                        |_| {
                            if value == "null" {
                                Value::Null
                            } else {
                                Value::String(value.into())
                            }
                        },
                        Value::from,
                    )
                })
                .unwrap_or(Value::Null),
        ),
        ("exposure_ev", json!(args.exposure_ev)),
        ("qa_print_index", json!(args.qa_print_index)),
    ] {
        if !value.is_null() {
            fields.insert(field.into(), value);
        }
    }
    if args.stops_above_midgray.as_deref() == Some("null") {
        fields.insert("stops_above_midgray".into(), Value::Null);
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
    let spec: BundleSpec =
        serde_json::from_value(Value::Object(fields)).context("invalid BundleSpec fields")?;
    lut_delivery::validate_target(&spec)?;
    Ok(spec)
}

fn build(args: BuildArgs) -> Result<()> {
    let spec = load_bundle_spec(&args)?;
    let backend = spektrafilm_gpu::select_backend();
    let builder = if let Some(path) = &args.params {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading RuntimeParams JSON: {}", path.display()))?;
        let params: RuntimeParams = serde_json::from_str(&text)
            .with_context(|| format!("parsing RuntimeParams JSON: {}", path.display()))?;
        BundleBuilder::with_params(spec, params)
    } else {
        BundleBuilder::new(spec)
    };
    if std::io::stderr().is_terminal() {
        eprintln!("[bake] Starting LUT baking");
    }
    let bundle = builder
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
                eprintln!("[ocio] SKIP: {reason}");
            }
        }
    }
    if bundle.spec.qa {
        if std::io::stderr().is_terminal() {
            eprintln!("[qa] Starting quality assessment");
        }
        let qa_root = out.join("qa");
        let report =
            spektrafilm_core::lut_qa::run(&bundle, &args.data_dir, backend.as_ref(), &qa_root)
                .map_err(anyhow::Error::msg)?;
        for path in
            spektrafilm_core::lut_qa::write_report(&report, &qa_root).map_err(anyhow::Error::msg)?
        {
            lut_delivery::append_artifact(
                &mut meta,
                lut_delivery::ArtifactReference {
                    path: format!("qa/{}", path.to_string_lossy().replace('\\', "/")),
                    kind: "qa".into(),
                    description: "Pinned Python 0.3.4 LUT quality assessment".into(),
                },
            )?;
        }
        lut_delivery::append_quality_summary(&out, &report)?;
        eprintln!("[qa] {}", if report.passed { "PASS" } else { "FAIL" });
    }
    lut_delivery::finalize_bundle(&out, &meta, bundle.spec.container == "zip")?;
    eprintln!("[done] {}", out.display());
    Ok(())
}

fn list(kind: ListKind, data_dir: &Path) -> Result<()> {
    match kind {
        ListKind::Film | ListKind::Print => {
            let stage = if matches!(kind, ListKind::Film) {
                "filming"
            } else {
                "printing"
            };
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
                    let name = data["info"]["stock"]
                        .as_str()
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
            let mut entries: Vec<_> = lut_transport::registry()
                .iter()
                .filter(|entry| {
                    if matches!(kind, ListKind::Input) {
                        entry.input
                    } else {
                        entry.output
                    }
                })
                .collect();
            entries.sort_by_key(|entry| entry.name);
            let width = entries
                .iter()
                .map(|entry| entry.name.len())
                .max()
                .unwrap_or(0);
            for entry in entries {
                println!("{:<width$}  {}", entry.name, entry.short_tag);
            }
        }
        ListKind::Target => {
            let mut names: Vec<_> = lut_delivery::list_targets()
                .iter()
                .map(|target| target.name)
                .collect();
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
        film.info
            .target_print
            .context("no paper specified and film has no target_print; use --paper")?
    };
    let spec: BundleSpec = serde_json::from_value(json!({
        "film_profile": film_name,
        "print_profiles": [print],
        "input_color_space": "ProPhoto RGB",
        "output_color_space": "sRGB",
        "topology": Topology::One,
        "resolution": resolution,
    }))?;
    let backend = spektrafilm_gpu::select_backend();
    let bundle = BundleBuilder::new(spec)
        .build(data_dir, backend.as_ref())
        .map_err(anyhow::Error::msg)?;
    let (_, lut) = bundle
        .luts
        .first()
        .context("canonical baker returned no LUT")?;
    lut.write_cube(output).map_err(anyhow::Error::msg)?;
    eprintln!("LUT saved: {}", output.display());
    Ok(())
}
