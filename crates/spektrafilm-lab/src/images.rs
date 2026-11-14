//! `compare images`: adapt one declaration into one image comparison.
//!
//! The adapter owns argument shape, pairing, and the analysis profile. It owns
//! no measurement: the engine quantifies and joins the two sides, and the
//! report builder presents the retained comparison.
//!
//! A delivered image has no executed command. This adapter therefore writes no
//! run record, no target, and no argv; its comparison names the files by digest
//! and by the declaration the caller made.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use serde_json::Value;
use spektrafilm_benchmark::{Error, Result};
use spektrafilm_benchmark::{
    analysis::{self, Region},
    documents, images as engine, local_input, report,
};

#[derive(ClapArgs)]
pub struct Args {
    /// Baseline file, or a directory of delivered images.
    #[arg(long, value_name = "FILE_OR_DIR")]
    pub baseline: std::path::PathBuf,
    /// Candidate file, or a directory of delivered images.
    #[arg(long, value_name = "FILE_OR_DIR")]
    pub candidate: std::path::PathBuf,
    /// Declared color space and transfer state of both sides.
    #[arg(long, value_name = "SPACE:encoded|decoded")]
    pub declaration: String,
    /// Declared region: name=x,y,w,h[:role]. Repeats.
    #[arg(
        long = "roi",
        value_name = "ID=X,Y,W,H[:ROLE]",
        conflicts_with = "regions"
    )]
    pub roi: Vec<String>,
    /// Authored regions document in render coordinates.
    #[arg(long, value_name = "FILE")]
    pub regions: Option<std::path::PathBuf>,
    /// Built-in analysis profile.
    #[arg(long, value_name = "ID", conflicts_with = "protocol")]
    pub analysis: Option<String>,
    /// Authored protocol document.
    #[arg(long, value_name = "FILE")]
    pub protocol: Option<std::path::PathBuf>,
    /// Box-average retained difference maps to one sample per NxN block.
    #[arg(long, value_name = "N")]
    pub map_scale: Option<u32>,
    /// Build the offline report inside the comparison directory.
    #[arg(long)]
    pub report: bool,
    /// Case identity for the single-pair form.
    #[arg(long, value_name = "ID")]
    pub case_id: Option<String>,
    /// New comparison directory; must not exist.
    #[arg(long, value_name = "DIR")]
    pub out: std::path::PathBuf,
}

pub fn run(args: &Args) -> Result<(Value, i32)> {
    let map_scale = args.map_scale.unwrap_or(1);
    if map_scale == 0 {
        return Err(Error::new(
            "argument_syntax",
            "--map-scale requires a positive integer",
        ));
    }
    let (color_space, cctf_decoding) = declaration(&args.declaration)?;
    let profile = analysis::profile(args.analysis.as_deref().unwrap_or("lab-appearance-v1"))?;
    let protocol = match &args.protocol {
        Some(path) => documents::load_json_document(path)
            .map_err(|error| error.at(path.display().to_string()))?,
        None => analysis::protocol(profile),
    };
    let regions = match (&args.regions, args.roi.is_empty()) {
        (Some(path), _) => {
            let document = local_input::load("regions", path)
                .map_err(|error| error.at(path.display().to_string()))?;
            document["regions"].as_array().cloned().ok_or_else(|| {
                Error::new(
                    "invalid_document",
                    "regions document carries no regions array",
                )
            })?
        }
        (None, false) => analysis::regions(profile, &declared_regions(&args.roi)?),
        (None, true) => Vec::new(),
    };
    let pairs = pairs(args)?;
    let request = engine::Request {
        baseline_id: source_id(&args.baseline),
        candidate_id: source_id(&args.candidate),
        color_space,
        cctf_decoding,
        pairs,
        regions,
        protocol,
        map_scale,
    };
    let (result, status) = engine::compare(&request, &args.out)?;
    if !args.report {
        return Ok((result, status));
    }
    let destination = args.out.join("report");
    let built = match report::build_report(&args.out, &destination) {
        Ok(value) => value,
        Err(error) => {
            std::fs::remove_dir_all(&destination).ok();
            return Err(error);
        }
    };
    let mut result = result;
    result["report"] = built;
    Ok((result, status))
}

/// `SPACE:encoded|decoded` carries both facts one declaration needs.
fn declaration(value: &str) -> Result<(String, bool)> {
    let (space, state) = value.rsplit_once(':').ok_or_else(|| {
        Error::new(
            "argument_syntax",
            "--declaration must be SPACE:encoded or SPACE:decoded",
        )
    })?;
    let decoding = match state {
        "encoded" => true,
        "decoded" => false,
        other => {
            return Err(Error::new(
                "argument_syntax",
                format!("--declaration state must be encoded or decoded, not {other}"),
            ));
        }
    };
    if space.is_empty() {
        return Err(Error::new(
            "argument_syntax",
            "--declaration must name a colour space",
        ));
    }
    Ok((space.to_owned(), decoding))
}

/// Parse `ID=X,Y,W,H[:ROLE]` declarations in order.
fn declared_regions(values: &[String]) -> Result<Vec<Region>> {
    let mut regions = Vec::new();
    for value in values {
        let (id, rest) = value.split_once('=').ok_or_else(|| {
            Error::new(
                "argument_syntax",
                format!("--roi must be ID=X,Y,W,H[:ROLE], not {value}"),
            )
        })?;
        analysis::validate_identity(id)?;
        let (numbers, role) = match rest.split_once(':') {
            Some((numbers, role)) => (numbers, role),
            None => (rest, "other"),
        };
        analysis::validate_role(role)?;
        let fields: Vec<&str> = numbers.split(',').collect();
        if fields.len() != 4 {
            return Err(Error::new(
                "argument_syntax",
                format!("--roi {id} must carry x,y,w,h, not {numbers}"),
            ));
        }
        let parse = |field: &str, name: &str| -> Result<u64> {
            field.trim().parse::<u64>().map_err(|_| {
                Error::new(
                    "argument_syntax",
                    format!("--roi {id} has a non-integer {name}: {field}"),
                )
            })
        };
        let region = Region {
            id: id.to_owned(),
            x: parse(fields[0], "x")?,
            y: parse(fields[1], "y")?,
            width: parse(fields[2], "width")?,
            height: parse(fields[3], "height")?,
            role: role.to_owned(),
        };
        if region.width == 0 || region.height == 0 {
            return Err(Error::new(
                "argument_syntax",
                format!("--roi {id} must have a positive width and height"),
            ));
        }
        regions.push(region);
    }
    Ok(regions)
}

/// One side's own name: the file stem, or the directory name.
fn source_id(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .and_then(|value| value.to_str())
        .unwrap_or("source")
        .to_owned()
}

fn case_identity(stem: &str) -> Result<String> {
    let valid = stem
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && stem
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '.' | '-'));
    if !valid {
        return Err(Error::new(
            "invalid_case_identity",
            format!("image name does not match ^[A-Za-z0-9][A-Za-z0-9_.-]*$: {stem}"),
        ));
    }
    Ok(stem.to_owned())
}

fn listing(directory: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut files = BTreeMap::new();
    let entries = std::fs::read_dir(directory).map_err(|error| {
        Error::new(
            "invalid_path",
            format!("reading {}: {error}", directory.display()),
        )
    })?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                Error::new(
                    "invalid_case_identity",
                    format!("image name is not UTF-8: {}", path.display()),
                )
            })?
            .to_owned();
        files.insert(name, path);
    }
    Ok(files)
}

fn pairs(args: &Args) -> Result<Vec<engine::Pair>> {
    match (args.baseline.is_dir(), args.candidate.is_dir()) {
        (true, true) => {
            if args.case_id.is_some() {
                return Err(Error::new(
                    "argument_syntax",
                    "--case-id applies only to the single-pair form",
                ));
            }
            let left = listing(&args.baseline)?;
            let right = listing(&args.candidate)?;
            let names: BTreeSet<&String> = left.keys().collect();
            let other: BTreeSet<&String> = right.keys().collect();
            if names != other {
                let only_left: Vec<&str> = names
                    .difference(&other)
                    .map(|value| value.as_str())
                    .collect();
                let only_right: Vec<&str> = other
                    .difference(&names)
                    .map(|value| value.as_str())
                    .collect();
                return Err(Error::new(
                    "incompatible_image_sets",
                    format!(
                        "only in baseline [{}], only in candidate [{}]",
                        only_left.join(", "),
                        only_right.join(", ")
                    ),
                ));
            }
            let mut seen = BTreeSet::new();
            let mut result = Vec::new();
            for (name, path) in &left {
                let stem = Path::new(name)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or(name);
                let case_id = case_identity(stem)?;
                if !seen.insert(case_id.clone()) {
                    return Err(Error::new(
                        "duplicate_case_identity",
                        format!("two files reduce to {case_id}"),
                    ));
                }
                result.push(engine::Pair {
                    case_id,
                    baseline: path.clone(),
                    candidate: right[name].clone(),
                });
            }
            if result.is_empty() {
                return Err(Error::new(
                    "incompatible_image_sets",
                    format!("no image pairs in {}", args.baseline.display()),
                ));
            }
            Ok(result)
        }
        (false, false) => {
            let stem = args
                .case_id
                .clone()
                .unwrap_or_else(|| source_id(&args.baseline));
            Ok(vec![engine::Pair {
                case_id: case_identity(&stem)?,
                baseline: args.baseline.clone(),
                candidate: args.candidate.clone(),
            }])
        }
        _ => Err(Error::new(
            "argument_syntax",
            "--baseline and --candidate must both be files or both be directories",
        )),
    }
}

#[derive(ClapArgs)]
pub struct AnalyzeArgs {
    /// One delivered image, or a directory of delivered images.
    #[arg(long, value_name = "FILE_OR_DIR")]
    pub source: std::path::PathBuf,
    /// Declared color space and transfer state of the source.
    #[arg(long, value_name = "SPACE:encoded|decoded")]
    pub declaration: String,
    /// Declared region: name=x,y,w,h[:role]. Repeats.
    #[arg(
        long = "roi",
        value_name = "ID=X,Y,W,H[:ROLE]",
        conflicts_with = "regions"
    )]
    pub roi: Vec<String>,
    /// Authored regions document in render coordinates.
    #[arg(long, value_name = "FILE")]
    pub regions: Option<std::path::PathBuf>,
    /// Built-in analysis profile.
    #[arg(long, value_name = "ID", conflicts_with = "protocol")]
    pub analysis: Option<String>,
    /// Authored protocol document.
    #[arg(long, value_name = "FILE")]
    pub protocol: Option<std::path::PathBuf>,
    /// New analysis directory; must not exist.
    #[arg(long, value_name = "DIR")]
    pub out: std::path::PathBuf,
}

pub fn analyze_run(args: &AnalyzeArgs) -> Result<(Value, i32)> {
    let (color_space, cctf_decoding) = declaration(&args.declaration)?;
    let profile = analysis::profile(args.analysis.as_deref().unwrap_or("lab-analysis-v1"))?;
    let protocol = match &args.protocol {
        Some(path) => documents::load_json_document(path)
            .map_err(|error| error.at(path.display().to_string()))?,
        None => analysis::protocol(profile),
    };
    let regions = match (&args.regions, args.roi.is_empty()) {
        (Some(path), _) => {
            let document = local_input::load("regions", path)
                .map_err(|error| error.at(path.display().to_string()))?;
            document["regions"].as_array().cloned().ok_or_else(|| {
                Error::new(
                    "invalid_document",
                    "regions document carries no regions array",
                )
            })?
        }
        (None, false) => analysis::regions(profile, &declared_regions(&args.roi)?),
        (None, true) => Vec::new(),
    };
    let files = analysis_files(&args.source)?;
    let request = engine::AnalysisRequest {
        source_id: source_id(&args.source),
        color_space,
        cctf_decoding,
        files,
        regions,
        protocol,
    };
    engine::analyze(&request, &args.out)
}

/// One source is one file, or a directory of files that pair with nothing.
fn analysis_files(source: &Path) -> Result<Vec<engine::Pair>> {
    if !source.is_dir() {
        let stem = source
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("source");
        let case_id = case_identity(stem)?;
        return Ok(vec![engine::Pair {
            case_id,
            baseline: source.to_path_buf(),
            candidate: source.to_path_buf(),
        }]);
    }
    let listed = listing(source)?;
    let mut files = Vec::new();
    for (stem, path) in listed {
        files.push(engine::Pair {
            case_id: case_identity(&stem)?,
            baseline: path.clone(),
            candidate: path,
        });
    }
    if files.is_empty() {
        return Err(Error::new(
            "missing_image",
            format!("no delivered images in {}", source.display()),
        ));
    }
    Ok(files)
}
