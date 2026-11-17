//! `compare`: adapt one declaration into one image comparison.
//!
//! The adapter owns argument shape, pairing, and the analysis profile. It owns
//! no measurement: the engine quantifies and joins the two sides. It presents
//! nothing; a page is one reading of the record and belongs to its reader.
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
    images as engine, local_input,
};

#[derive(ClapArgs)]
pub struct Args {
    /// Baseline file, or a directory of delivered images.
    #[arg(long, value_name = "PATH")]
    pub baseline: std::path::PathBuf,
    /// Candidate file, or a directory of delivered images.
    #[arg(long, value_name = "PATH")]
    pub candidate: std::path::PathBuf,
    /// Declared color space and transfer state of both sides; the space is one
    /// the instrument reads and the state is encoded or decoded.
    #[arg(long, value_name = "SPACE:STATE")]
    pub declaration: Declaration,
    /// Declared region: name=x,y,w,h[:role]. Repeats. A role is one of skin,
    /// neutral, shadow, highlight, color, flat_field, texture, edge, other; it
    /// decides which families can be measured there.
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
    #[arg(long, value_name = "ID", value_parser = profiles())]
    pub analysis: Option<String>,
    /// Box-average retained difference maps to one sample per NxN block. It
    /// changes no measured value.
    #[arg(long, value_name = "N")]
    pub map_scale: Option<u32>,
    /// New comparison directory; must not exist.
    #[arg(long, value_name = "DIR")]
    pub out: std::path::PathBuf,
}

/// The profile ids, with one owner: the profile table.
///
/// A caller picks a named recipe; the vocabulary is the table's, not a second
/// literal that drifts from it.
fn profiles() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(analysis::PROFILES.iter().map(|profile| profile.id))
}

pub fn run(args: &Args) -> Result<(Value, i32)> {
    let map_scale = args.map_scale.unwrap_or(1);
    if map_scale == 0 {
        return Err(Error::new(
            "argument_syntax",
            "--map-scale requires a positive integer",
        ));
    }
    let Declaration {
        space: color_space,
        decoding: cctf_decoding,
    } = args.declaration;
    let profile = analysis::profile(args.analysis.as_deref().unwrap_or("lab-appearance-v1"))?;
    let protocol = analysis::protocol(profile);
    let regions = regions_of(&args.regions, &args.roi)?;
    let pairs = pairs(args)?;
    let request = engine::Request {
        baseline_id: source_id(&args.baseline),
        candidate_id: source_id(&args.candidate),
        color_space: color_space.to_owned(),
        cctf_decoding,
        pairs,
        regions,
        protocol,
        map_scale,
    };
    engine::compare(&request, &args.out)
}

/// `SPACE:encoded|decoded` carries both facts one declaration needs.
///
/// Both halves are closed vocabularies, and both are decided here: the space
/// comes from the measurement library's one list, and the transfer state has
/// two values. A typo is a usage error, so it is refused while the arguments
/// are read, before any destination exists.
#[derive(Clone, Copy, Debug)]
pub struct Declaration {
    pub space: &'static str,
    pub decoding: bool,
}

impl std::str::FromStr for Declaration {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let (space, state) = value
            .rsplit_once(':')
            .ok_or_else(|| "--declaration must be SPACE:encoded or SPACE:decoded".to_owned())?;
        let decoding = match state {
            "encoded" => true,
            "decoded" => false,
            other => {
                return Err(format!(
                    "--declaration state must be encoded or decoded, not {other}"
                ));
            }
        };
        let space = spektrafilm_benchmark::image_data::COLOR_SPACES
            .iter()
            .find(|candidate| **candidate == space)
            .ok_or_else(|| {
                format!(
                    "--declaration color space must be one of: {}",
                    spektrafilm_benchmark::image_data::COLOR_SPACES.join(", ")
                )
            })?;
        Ok(Self { space, decoding })
    }
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
        if regions.iter().any(|region: &Region| region.id == id) {
            return Err(Error::new(
                "duplicate_region_identity",
                format!("--roi declares {id} twice"),
            ));
        }
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

/// The declared regions, from a document or from the command line.
///
/// Both measurement endpoints resolve geometry through this, so a region
/// document and a `--roi` produce the same regions, and neither endpoint can
/// drift in which of the two wins.
fn regions_of(document: &Option<PathBuf>, roi: &[String]) -> Result<Vec<Value>> {
    match (document, roi.is_empty()) {
        (Some(path), _) => {
            let value = local_input::load("regions", path)
                .map_err(|error| error.at(path.display().to_string()))?;
            value["regions"].as_array().cloned().ok_or_else(|| {
                Error::new(
                    "invalid_document",
                    "regions document carries no regions array",
                )
            })
        }
        (None, false) => Ok(analysis::regions(&declared_regions(roi)?)),
        (None, true) => Ok(Vec::new()),
    }
}

/// One side's own name: the file stem, or the directory name.
fn source_id(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .and_then(|value| value.to_str())
        .unwrap_or("source")
        .to_owned()
}

/// The case identity a file name carries: its stem, validated.
///
/// Both endpoints name the cases of one directory through this, so a record
/// written by `analyze` and a record written by `compare` over the same
/// directory name the same cases.
fn case_of(name: &str) -> Result<String> {
    case_identity(
        Path::new(name)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(name),
    )
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

/// The case identity of every file in a directory, keyed by file name.
///
/// Two names that reduce to one identity are one case, so the directory is
/// refused rather than measured twice. Both endpoints name the cases of one
/// directory through this, so a record from either names them the same way.
fn case_ids(files: &BTreeMap<String, PathBuf>) -> Result<BTreeMap<String, String>> {
    let mut ids = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for name in files.keys() {
        let case_id = case_of(name)?;
        if !seen.insert(case_id.clone()) {
            return Err(Error::new(
                "duplicate_case_identity",
                format!("two files reduce to {case_id}"),
            ));
        }
        ids.insert(name.clone(), case_id);
    }
    Ok(ids)
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
            let ids = case_ids(&left)?;
            let mut result = Vec::new();
            for (name, path) in &left {
                result.push(engine::Pair {
                    case_id: ids[name].clone(),
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
        (false, false) => Ok(vec![engine::Pair {
            case_id: case_identity(&source_id(&args.baseline))?,
            baseline: args.baseline.clone(),
            candidate: args.candidate.clone(),
        }]),
        _ => Err(Error::new(
            "argument_syntax",
            "--baseline and --candidate must both be files or both be directories",
        )),
    }
}

#[derive(ClapArgs)]
pub struct AnalyzeArgs {
    /// One delivered image, or a directory of delivered images.
    #[arg(long, value_name = "PATH")]
    pub source: std::path::PathBuf,
    /// Declared color space and transfer state of the source; the space is one
    /// the instrument reads and the state is encoded or decoded.
    #[arg(long, value_name = "SPACE:STATE")]
    pub declaration: Declaration,
    /// Declared region: name=x,y,w,h[:role]. Repeats. A role is one of skin,
    /// neutral, shadow, highlight, color, flat_field, texture, edge, other; it
    /// decides which families can be measured there.
    #[arg(
        long = "roi",
        value_name = "ID=X,Y,W,H[:ROLE]",
        conflicts_with = "regions"
    )]
    pub roi: Vec<String>,
    /// Authored regions document in render coordinates.
    #[arg(long, value_name = "FILE")]
    pub regions: Option<std::path::PathBuf>,
    /// Built-in analysis profile. It materializes the protocol the record
    /// retains, so a profile id is all a caller states.
    #[arg(long, value_name = "ID", value_parser = profiles())]
    pub analysis: Option<String>,
    /// New analysis directory; must not exist.
    #[arg(long, value_name = "DIR")]
    pub out: std::path::PathBuf,
}

pub fn analyze_run(args: &AnalyzeArgs) -> Result<(Value, i32)> {
    let Declaration {
        space: color_space,
        decoding: cctf_decoding,
    } = args.declaration;
    let profile = analysis::profile(args.analysis.as_deref().unwrap_or("lab-analysis-v1"))?;
    let protocol = analysis::protocol(profile);
    let regions = regions_of(&args.regions, &args.roi)?;
    let files = analysis_files(&args.source)?;
    let request = engine::AnalysisRequest {
        source_id: source_id(&args.source),
        color_space: color_space.to_owned(),
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
    let ids = case_ids(&listed)?;
    let mut files = Vec::new();
    for (name, path) in listed {
        files.push(engine::Pair {
            case_id: ids[&name].clone(),
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
