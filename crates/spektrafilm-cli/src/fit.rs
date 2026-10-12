//! `fit` command: derive profile and look values from reference patch data.

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_core::fit::{ChartLayout, FitGroup, FitOptions, FitRequest, PatchTable};
use spektrafilm_core::presets::PresetLibrary;
use spektrafilm_core::process_params::{self, ProcessParamsRequest};
use spektrafilm_core::runtime::DigestMode;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum FitGroupArg {
    Surface,
    Tone,
}

impl From<FitGroupArg> for FitGroup {
    fn from(value: FitGroupArg) -> Self {
        match value {
            FitGroupArg::Surface => FitGroup::Surface,
            FitGroupArg::Tone => FitGroup::Tone,
        }
    }
}

#[derive(Args)]
pub(crate) struct FitArgs {
    /// Chart layout JSON: grid geometry and patch index ranges.
    #[arg(long, help_heading = "Fit data")]
    layout: PathBuf,
    /// Scene patch table JSON: what the film saw, in the declared input color space.
    #[arg(long, help_heading = "Fit data")]
    scene: PathBuf,
    /// Reference patch table JSON: the developed film, linear sRGB.
    #[arg(long, help_heading = "Fit data")]
    reference: PathBuf,
    /// Parameter groups to fit, comma-separated: surface, tone.
    #[arg(
        long = "fit",
        value_enum,
        value_delimiter = ',',
        default_value = "surface",
        help_heading = "Fit data"
    )]
    groups: Vec<FitGroupArg>,
    /// Leading surface terms per layer to fit.
    #[arg(long, default_value_t = 9, help_heading = "Fit data")]
    surface_terms: usize,
    /// Ridge weight on the fitted surface coefficients.
    #[arg(long, default_value_t = 1.0, help_heading = "Fit data")]
    lambda: f64,
    /// Maximum accepted parameter vectors.
    #[arg(long, default_value_t = 15, help_heading = "Fit data")]
    max_iters: usize,
    /// Cross-validate the color patches: fit the even and odd halves
    /// separately and report the held-out medians. The written profile and
    /// overlay still come from the fit over every patch.
    #[arg(long, help_heading = "Fit data")]
    cv: bool,
    /// Fit report JSON destination; written only after the fit succeeds.
    #[arg(long, help_heading = "Fit output")]
    report: PathBuf,
    /// Profile copy with the fitted surface; required when fitting the surface.
    #[arg(long, help_heading = "Fit output")]
    out_profile: Option<PathBuf>,
    /// Sparse parameter overlay with the fitted tone; required when fitting tone.
    #[arg(long, help_heading = "Fit output")]
    out_overlay: Option<PathBuf>,
    /// Exact preset name or .toml/.json file; repeat to compose looks and sections.
    #[arg(long, action = clap::ArgAction::Append, help_heading = "Look and workflow")]
    preset: Vec<String>,
    /// Film stock ID; required without --preset.
    #[arg(long, help_heading = "Look and workflow")]
    film: Option<String>,
    /// Print stock ID; otherwise uses the film for direct scans or its target_print.
    #[arg(long, help_heading = "Look and workflow")]
    paper: Option<String>,
    /// Workflow route; the fit needs a scan route.
    #[arg(long, help_heading = "Look and workflow")]
    route: Option<String>,
    /// Film scanner output mode; the fit requires positive_scan.
    #[arg(
        long,
        default_value = "positive_scan",
        value_parser = ["direct_scan", "positive_scan"],
        help_heading = "Look and workflow"
    )]
    scan_output: String,
    /// Raster color space of the scene patch table; discover with describe --field io.input_color_space.
    #[arg(long, help_heading = "Input declaration")]
    input_color_space: Option<String>,
    /// Whether the scene table requires transfer decoding (true or false).
    #[arg(long, action = clap::ArgAction::Set, help_heading = "Input declaration")]
    input_cctf_decoding: Option<bool>,
    /// Apply PATH=VALUE assignments or a sparse .toml/.json file in order.
    #[arg(long = "set", value_name = "PATH=VALUE|FILE", action = clap::ArgAction::Append, help_heading = "Look and workflow")]
    sources: Vec<String>,
    /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,
}

fn read_json<T: DeserializeOwned>(path: &Path, what: &str) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {what}: {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {what}: {}", path.display()))
}

fn file_provenance(path: &Path) -> Result<Value> {
    let bytes = std::fs::read(path).with_context(|| format!("hashing {}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(json!({
        "path": path.display().to_string(),
        "sha256": format!("{digest:x}"),
    }))
}

pub(crate) fn cmd_fit(args: &FitArgs, matches: &clap::ArgMatches) -> Result<()> {
    let data_dir = crate::select_data_dir(args.data_dir.clone(), matches)?.path;
    let data_dir = data_dir.as_path();
    let layout: ChartLayout = read_json(&args.layout, "chart layout")?;
    let scene: PatchTable = read_json(&args.scene, "scene patch table")?;
    let reference: PatchTable = read_json(&args.reference, "reference patch table")?;

    let groups: Vec<FitGroup> = args
        .groups
        .iter()
        .map(|group| FitGroup::from(*group))
        .collect();
    let surface = groups.contains(&FitGroup::Surface);
    let tone = groups.contains(&FitGroup::Tone);
    if surface && args.out_profile.is_none() {
        bail!("--out-profile is required when the fit includes the surface group");
    }
    if tone && args.out_overlay.is_none() {
        bail!("--out-overlay is required when the fit includes the tone group");
    }

    let library = PresetLibrary::default();
    let presets = args
        .preset
        .iter()
        .map(|selector| crate::preset::select(&library, selector).map(|selected| selected.preset))
        .collect::<Result<Vec<_>>>()?;
    let resolved = process_params::resolve(
        ProcessParamsRequest {
            film: args.film.as_deref(),
            paper: args.paper.as_deref(),
            presets: &presets,
            params_file: None,
            sources: &args.sources,
            route: args.route.as_deref(),
            scan_film: false,
            scan_output: Some(args.scan_output.as_str()),
            input_is_raw: false,
            input_color_space: args.input_color_space.as_deref(),
            input_cctf_decoding: args.input_cctf_decoding,
            require_input_declaration: true,
            digest_mode: DigestMode::ApplyStockSpecifics,
        },
        data_dir,
    )
    .map_err(anyhow::Error::msg)
    .context("resolving the fit chain configuration")?;

    let mut fitted_profile = resolved.film.clone();
    let report = spektrafilm_core::fit::fit(FitRequest {
        params: resolved.params.clone(),
        film: resolved.film,
        print: resolved.print,
        data_dir: data_dir.to_path_buf(),
        layout,
        scene,
        reference,
        options: FitOptions {
            groups: groups.clone(),
            surface_terms: args.surface_terms,
            lambda: args.lambda,
            max_iters: args.max_iters,
            cv: args.cv,
        },
    })
    .map_err(anyhow::Error::msg)?;

    if let Some(surface_values) = &report.surface {
        fitted_profile.data.hanatos2025_adaptation_surface_params = surface_values.clone();
        let path = args.out_profile.as_ref().expect("checked before the fit");
        spektrafilm_core::profile::save_profile_to(path, &fitted_profile)
            .map_err(anyhow::Error::msg)?;
    }
    if let Some(fitted_tone) = &report.tone {
        let path = args.out_overlay.as_ref().expect("checked before the fit");
        let overlay = json!({
            "scanner": {
                "positive_density_ev": fitted_tone.positive_density_ev,
                "positive_balance_ev": fitted_tone.positive_balance_ev,
            }
        });
        write_json(path, &overlay)?;
    }

    let mut document = serde_json::to_value(&report)?;
    let object = document
        .as_object_mut()
        .expect("fit report serializes as an object");
    object.insert(
        "tool".into(),
        json!({
            "name": "spektrafilm-f64",
            "version": env!("CARGO_PKG_VERSION"),
        }),
    );
    object.insert(
        "inputs".into(),
        json!({
            "layout": file_provenance(&args.layout)?,
            "scene": file_provenance(&args.scene)?,
            "reference": file_provenance(&args.reference)?,
        }),
    );
    object.insert(
        "chain".into(),
        json!({
            "presets": args.preset,
            "film": resolved.film_name,
            "print": resolved.print_name,
            "route": resolved.params.workflow.route,
            "scan_output": resolved.params.scanner.scan_output,
            "input_color_space": resolved.input_declaration.color_space,
            "input_cctf_decoding": resolved.input_declaration.cctf_decoding,
        }),
    );
    let mut outputs = Map::new();
    if let Some(path) = &args.out_profile {
        outputs.insert("profile".into(), json!(path.display().to_string()));
    }
    if let Some(path) = &args.out_overlay {
        outputs.insert("overlay".into(), json!(path.display().to_string()));
    }
    object.insert("outputs".into(), Value::Object(outputs));
    write_json(&args.report, &document)?;

    eprintln!(
        "Fit {:?}: color ΔE00 {} → {}, gray ΔE00 {} → {}, {} iterations, {} evaluations",
        report.groups,
        format_de(report.base_color_de_median),
        format_de(report.color_de_median),
        format_de(report.base_gray_de_median),
        format_de(report.gray_de_median),
        report.iterations,
        report.evaluations,
    );
    eprintln!("Report: {}", args.report.display());
    for (index, fold) in report.cv.iter().flatten().enumerate() {
        eprintln!(
            "  fold {} (held out {} color patches): train ΔE00 {} → held-out {} (unfitted {})",
            index + 1,
            fold.held_out.len(),
            format_de(fold.train_de_median),
            format_de(fold.test_de_median),
            format_de(fold.test_base_de_median),
        );
    }
    Ok(())
}

fn format_de(value: f64) -> String {
    format!("{value:.3}")
}

fn write_json(path: &Path, document: &Value) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut text = serde_json::to_string_pretty(document)?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
