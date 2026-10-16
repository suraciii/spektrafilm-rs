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
pub struct FitArgs {
    /// Chart layout JSON: grid geometry and patch index ranges.
    #[arg(long, help_heading = "Fit data")]
    layout: PathBuf,
    /// Scene patch table JSON: what the film saw, in the declared input color
    /// space, with the `source` that produced the values.
    #[arg(long, help_heading = "Fit data")]
    scene: PathBuf,
    /// Reference patch table JSON: the developed film, linear sRGB, with the
    /// `source` that produced the values.
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
    /// Film scanner output mode; the fit requires negative_to_positive.
    #[arg(
        long,
        default_value = "negative_to_positive",
        value_parser = ["direct_scan", "negative_to_positive"],
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

fn table_provenance(path: &Path, source: &str) -> Result<Value> {
    let mut value = file_provenance(path)?;
    value["source"] = json!(source);
    Ok(value)
}

const SURFACE_PROVENANCE: &str = "Hanatos2025 adaptation surface fitted by spektrafilm-lab fit ";

/// Append the fitted surface's provenance to a profile datasource. The fit
/// replaces the whole surface, so this replaces an earlier fit's sentence,
/// which is always the last one.
fn record_surface_provenance(datasource: &mut String, scene: &str, reference: &str) {
    if let Some(start) = datasource.find(SURFACE_PROVENANCE) {
        datasource.truncate(start);
    }
    datasource.truncate(datasource.trim_end().len());
    if !datasource.is_empty() {
        datasource.push(' ');
    }
    datasource.push_str(&format!(
        "{SURFACE_PROVENANCE}{}: scene patches from {scene}; reference patches from {reference}.",
        env!("CARGO_PKG_VERSION"),
    ));
}

pub fn cmd_fit(args: &FitArgs, matches: &clap::ArgMatches) -> Result<()> {
    let data_dir = crate::select_data_dir(args.data_dir.clone(), matches)?.path;
    let data_dir = data_dir.as_path();
    let layout: ChartLayout = read_json(&args.layout, "chart layout")?;
    let scene: PatchTable = read_json(&args.scene, "scene patch table")?;
    let reference: PatchTable = read_json(&args.reference, "reference patch table")?;
    let scene_source = scene.source.clone();
    let reference_source = reference.source.clone();

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

    // Build every output and create every destination directory before the
    // first write, so a failure leaves no partial set of results behind. The
    // profile is written first because saving it validates it.
    if let Some(surface_values) = &report.surface {
        fitted_profile.data.hanatos2025_adaptation_surface_params = surface_values.clone();
        record_surface_provenance(
            &mut fitted_profile.metadata.datasource,
            &scene_source,
            &reference_source,
        );
    }
    let overlay = report.tone.as_ref().map(|fitted_tone| {
        json!({
            "scanner": {
                "conversion_brightness_ev": fitted_tone.conversion_brightness_ev,
                "conversion_balance_ev": fitted_tone.conversion_balance_ev,
            }
        })
    });

    let mut document = serde_json::to_value(&report)?;
    let object = document
        .as_object_mut()
        .expect("fit report serializes as an object");
    object.insert(
        "tool".into(),
        json!({
            "name": "spektrafilm-lab",
            "version": env!("CARGO_PKG_VERSION"),
        }),
    );
    object.insert(
        "inputs".into(),
        json!({
            "layout": file_provenance(&args.layout)?,
            "scene": table_provenance(&args.scene, &scene_source)?,
            "reference": table_provenance(&args.reference, &reference_source)?,
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

    publish_outputs(args, &report, &fitted_profile, overlay.as_ref(), &document)?;

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

/// Write every output to a staging sibling, then rename each into place. A
/// failed write or move removes this run's staged and published files, so no
/// destination keeps a partial set of the run's results.
fn publish_outputs(
    args: &FitArgs,
    report: &spektrafilm_core::fit::FitReport,
    profile: &spektrafilm_core::profile::Profile,
    overlay: Option<&Value>,
    document: &Value,
) -> Result<()> {
    enum Output<'a> {
        Profile,
        Json(&'a Value),
    }
    let mut planned = Vec::new();
    if report.surface.is_some() {
        let path = args.out_profile.as_deref().expect("checked before the fit");
        planned.push((path, Output::Profile));
    }
    if let Some(overlay) = overlay {
        let path = args.out_overlay.as_deref().expect("checked before the fit");
        planned.push((path, Output::Json(overlay)));
    }
    planned.push((args.report.as_path(), Output::Json(document)));

    let mut staged: Vec<(PathBuf, &Path)> = Vec::with_capacity(planned.len());
    let mut written = Ok(());
    for (path, output) in planned {
        let staging = match staging_path(path) {
            Ok(staging) => staging,
            Err(error) => {
                written = Err(error);
                break;
            }
        };
        staged.push((staging.clone(), path));
        written = create_parent(path).and_then(|()| match output {
            Output::Profile => spektrafilm_core::profile::save_profile_to(&staging, profile)
                .map_err(anyhow::Error::msg),
            Output::Json(value) => write_json(&staging, value),
        });
        if written.is_err() {
            break;
        }
    }
    if let Err(error) = written {
        for (staging, _) in &staged {
            let _ = std::fs::remove_file(staging);
        }
        return Err(error);
    }
    for (index, (staging, path)) in staged.iter().enumerate() {
        if let Err(error) = std::fs::rename(staging, path) {
            // Withdraw this run's published outputs and the staged rest.
            for (_, published) in &staged[..index] {
                let _ = std::fs::remove_file(published);
            }
            for (pending, _) in &staged[index..] {
                let _ = std::fs::remove_file(pending);
            }
            return Err(error).with_context(|| format!("moving {} into place", path.display()));
        }
    }
    Ok(())
}

fn staging_path(path: &Path) -> Result<PathBuf> {
    let mut name = path
        .file_name()
        .with_context(|| format!("{} names no file", path.display()))?
        .to_os_string();
    name.push(".partial");
    Ok(path.with_file_name(name))
}

fn create_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    Ok(())
}

fn write_json(path: &Path, document: &Value) -> Result<()> {
    let mut text = serde_json::to_string_pretty(document)?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_provenance_replaces_an_earlier_fit_and_keeps_the_rest() {
        let mut datasource = "Curves from the Cs41 chart.".to_string();
        record_surface_provenance(&mut datasource, "camera A", "lab scan");
        record_surface_provenance(&mut datasource, "chart reference XYZ", "lab scan");
        assert_eq!(datasource.matches(SURFACE_PROVENANCE).count(), 1);
        assert!(datasource.starts_with("Curves from the Cs41 chart. "));
        assert!(datasource.contains("scene patches from chart reference XYZ;"));
        assert!(!datasource.contains("camera A"));

        let mut empty = String::new();
        record_surface_provenance(&mut empty, "camera A", "lab scan");
        assert!(empty.starts_with(SURFACE_PROVENANCE));
    }
}
