//! `experiment run` / `experiment export`: the development loop for a film
//! profile.
//!
//! `run` renders one declared experiment through the shared `process` entry
//! and publishes the resulting native stage-evidence bundle plus the records
//! that make the run reproducible. `export` converts an existing bundle's raw
//! f64 buffers into `.npy` arrays.
//!
//! The module owns no rendering, no measurement, and no parameter language:
//! it declares what to run, validates the declaration against the benchmark
//! case that owns the region vocabulary, and hands the resolved chain to the
//! same entry the user binaries use. Export is a byte-exact conversion; it
//! never re-renders, re-derives, averages, resamples, or reorders channels.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_core::presets::PresetLibrary;

use super::cli::{ExperimentCommand, ExperimentExportArgs, ExperimentRunArgs, OutputFormatArg};
use super::report;
use crate::process::{self, OutputFormat, ProcessArgs, WorkflowOptions};
use crate::select_data_dir_impl;

/// Schema version of the experiment document.
const SCHEMA_VERSION: u32 = 1;

/// The identifier pattern shared with the benchmark documents.
const ID_PATTERN: &str = r"^[A-Za-z0-9][A-Za-z0-9_.-]*$";

/// Pixel evidence is a lab-owned, f64-only contract.
const BUNDLE_SCHEMA_VERSION: u32 = 1;

/// Run one declared experiment and publish its boundary values.
pub fn dispatch(command: &ExperimentCommand) -> Result<()> {
    match command {
        ExperimentCommand::Run(args) => run(args),
        ExperimentCommand::Export(args) => export(args),
    }
}

/// Run an experimental command and return its command result object.
pub fn dispatch_with_result(command: &ExperimentCommand) -> Result<(serde_json::Value, i32)> {
    match command {
        ExperimentCommand::Run(_) | ExperimentCommand::Export(_) => {
            dispatch(command)?;
            Ok((
                serde_json::json!({
                    "schema_version": 1,
                    "command": "experiment",
                    "execution_status": "completed",
                }),
                0,
            ))
        }
    }
}

/// Run one declared experiment and publish its boundary values.
fn run(args: &ExperimentRunArgs) -> Result<()> {
    let document_path = absolute(&args.experiment)?;
    let text = read_document(&document_path)?;
    let document: ExperimentDocument = parse_document(&text, &document_path)?;
    let document_root = document_root(&document_path)?;

    // Validate the declaration before anything is rendered: a geometry that
    // disagrees with the case is a failure, not a crop.
    let suite_path = resolve_inside(&document.case.suite, &document_root, "case.suite")?;
    let case = load_suite(&suite_path, &document.case.id)?;
    validate_grid(&document.grid, &case)?;
    let set_sources = resolve_set_sources(&document.chain.set, &document_root)?;

    // The input file's bytes are the declaration's ground truth.
    let input_path = resolve_inside(&document.input.path, &document_root, "input.path")?;
    verify_input_digest(&input_path, &document.input.sha256)?;

    let out = absolute(&args.out)?;
    if !args.force
        && report_exists(
            &out,
            &out.join(format!("output.{}", args.output_format.into_name())),
        )
    {
        bail!(
            "refusing to replace an existing report in {}; pass --force to overwrite it",
            out.display()
        );
    }

    // `--data-dir` is optional; an omitted flag still honours
    // SPEKTRAFILM_DATA_DIR and automatic discovery.
    let data = select_data_dir_impl(
        args.data_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("data")),
        args.data_dir.is_some(),
    )?;
    let bundle_dir = out.join("bundle");
    // A route the bundle cannot describe is a configuration-level failure
    // before rendering: the capture route is `rgb_in` through `rgb_out`, so a
    // route that skips the film or print boundaries would publish a bundle
    // that never carries them.
    //
    // The document's own `route` field is checked first, but a legacy params
    // file can override the route, so the *resolved* route is the authority.
    if let Some(route) = &document.chain.route {
        ensure_describable_route(route)?;
    }
    let resolved = resolve_chain(&document, &document_root, &data.path)?;
    let resolved_route = resolved
        .params
        .get("workflow")
        .and_then(|w| w.get("route"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    ensure_describable_route(resolved_route)?;
    let process_args = ProcessArgs {
        backend: Some(crate::Backend::Cpu),
        input: input_path,
        input_color_space: document.chain.input_color_space.clone(),
        input_cctf_decoding: document.chain.input_cctf_decoding,
        output: Some(out.join(format!("output.{}", args.output_format.into_name()))),
        format: Some(args.output_format.into()),
        bit_depth: None,
        jpeg_quality: None,
        jpeg_subsampling: None,
        compression: None,
        workflow: WorkflowOptions {
            saving_color_space: None,
            saving_cctf_encoding: None,
            raw_white_balance: "as-shot".to_owned(),
            raw_temperature: None,
            raw_tint: None,
            lens_correction: false,
            route: document.chain.route.clone(),
            scan_output: document.chain.scan_output.clone(),
            film_channel_swap: "none".to_owned(),
            print_channel_swap: "none".to_owned(),
        },
        film: document.chain.film.clone(),
        paper: document.chain.paper.clone(),
        preset: document.chain.preset.clone(),
        scan_film: false,
        timings: false,
        diagnostics: None,
        gpu_timings: false,
        params: document
            .chain
            .params
            .as_ref()
            .map(|path| resolve_inside(path, &document_root, "chain.params"))
            .transpose()?,
        sources: set_sources
            .iter()
            .map(|source| source.display().to_string())
            .collect(),
        dry_run: false,
        dry_run_format: None,
        seed: Some(document.seeds[0]),
        raw_out: None,
        stage_evidence: Some(bundle_dir.clone()),
        iters: 1,
        data_dir: data.path.clone(),
    };
    let command_vector = command_vector(args.output_format, &process_args, &set_sources);
    process::run_with_data_dir(process_args, data)?;

    let manifest_path = bundle_dir.join("stage-evidence.json");
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).with_context(|| {
            format!(
                "reading published bundle manifest: {}",
                manifest_path.display()
            )
        })?)
        .with_context(|| {
            format!(
                "parsing published bundle manifest: {}",
                manifest_path.display()
            )
        })?;
    if manifest["execution_status"] != "completed" {
        bail!(
            "the published bundle is not complete: {}",
            manifest["errors"]
                .as_array()
                .map(|errors| errors
                    .iter()
                    .map(|error| error.as_str().unwrap_or_default().to_owned())
                    .collect::<Vec<_>>()
                    .join("; "))
                .unwrap_or_else(|| "execution_status is not completed".to_owned())
        );
    }
    // The bundle must carry every stage the experiment declares: a route
    // that skips a boundary would leave `not_applicable` stages, which is
    // not the record a profile author asked for.
    let stages = manifest["stages"]
        .as_array()
        .ok_or_else(|| anyhow!("bundle manifest carries no stages array"))?;
    let available: Vec<&str> = stages
        .iter()
        .filter(|s| s["availability"] == "available")
        .filter_map(|s| s["id"].as_str())
        .collect();
    let expected: Vec<&str> = spektrafilm_core::stage_evidence::StageId::ORDER
        .iter()
        .map(|s| s.id())
        .collect();
    if available.len() != expected.len() || !expected.iter().all(|e| available.contains(e)) {
        bail!(
            "the published bundle does not carry every stage; available: {:?}, expected: {:?}",
            available,
            expected
        );
    }
    if let Err(error) = write_records(
        &out,
        args.force,
        &Records {
            document: &document,
            document_path: &document_path,
            suite_path: &suite_path,
            resolved,
            manifest,
            command_vector,
        },
    ) {
        // A failed record write must not leave a stale, consumable bundle
        // from an earlier successful run.
        let _ = std::fs::remove_dir_all(&bundle_dir);
        return Err(error);
    }
    Ok(())
}

/// Reject a route the executed tap range cannot describe.
///
/// `run` captures `rgb_in` through `rgb_out`, so the executed route must carry
/// both the film and the print boundaries. A route that skips one of them
/// would publish a bundle with `not_applicable` stages, which is not the
/// evidence a profile author asked for.
fn ensure_describable_route(route: &str) -> Result<()> {
    use spektrafilm_core::telemetry::WorkflowRoute;
    let parsed = WorkflowRoute::parse(route);
    if parsed.as_str().is_empty() {
        bail!(
            "unknown workflow route {route:?}; supported routes are \
             \"input > film > print > scan\", \"input > convert-film > print > scan\", \
             \"input > convert-film > scan-minus-base\""
        );
    }
    if !parsed.prints() {
        bail!(
            "route {route:?} does not execute the print development boundary; \
             the published bundle would not carry the print stages — use a print route \
             (\"input > film > print > scan\" or \"input > convert-film > print > scan\")"
        );
    }
    Ok(())
}

/// Convert one bundle's buffers into `.npy` arrays.
fn export(args: &ExperimentExportArgs) -> Result<()> {
    let bundle_dir = absolute(&args.stage_evidence)?;
    let manifest_path = bundle_dir.join("stage-evidence.json");
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading bundle manifest: {}", manifest_path.display()))?,
    )
    .with_context(|| format!("parsing bundle manifest: {}", manifest_path.display()))?;
    validate_bundle(&manifest)?;

    let available: Vec<&Value> = manifest["stages"]
        .as_array()
        .ok_or_else(|| anyhow!("bundle manifest carries no stages array"))?
        .iter()
        .filter(|stage| stage["availability"] == "available")
        .collect();
    if available.is_empty() {
        bail!("bundle carries no available stage to convert");
    }
    let selected = select_stages(&available, &args.stage)?;

    let out = absolute(&args.out)?;
    if out != bundle_dir && out.starts_with(&bundle_dir) {
        bail!(
            "export destination {} is inside the bundle directory; \
             choose a separate --out directory",
            out.display()
        );
    }
    if !args.force && out.join("export.json").exists() {
        bail!(
            "refusing to replace an existing array report in {}; pass --force to overwrite it",
            out.display()
        );
    }

    let mut converted = Vec::with_capacity(selected.len());
    let result = (|| -> Result<()> {
        std::fs::create_dir_all(&out)
            .with_context(|| format!("creating array directory: {}", out.display()))?;
        for stage in &selected {
            converted.push(convert_stage(&bundle_dir, &out, args.force, stage)?);
        }
        Ok(())
    })();
    if let Err(error) = result {
        // Remove any arrays this export already wrote, so a consumer
        // never sees partial output without an export.json.
        for array in &converted {
            let _ = std::fs::remove_file(&array.path);
        }
        let _ = std::fs::remove_file(out.join("export.json"));
        return Err(error);
    }
    let export_text = format!(
        "{}\n",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "tool": {"name": "spektrafilm-lab", "version": env!("CARGO_PKG_VERSION")},
            "manifest": manifest_path.display().to_string(),
            "stages": converted.iter().map(|array| json!({
                "id": array.id,
                "path": array.path.display().to_string(),
                "sha256": array.sha256,
                "shape": array.shape,
            })).collect::<Vec<_>>(),
        }))?
    );
    report::write_named(
        &out,
        &[("export.json", export_text.as_str())],
        "refusing to replace an existing array report",
        args.force,
    )
    .map_err(anyhow::Error::msg)
}

// ---------------------------------------------------------------------------
// Document parsing
// ---------------------------------------------------------------------------

/// One declared experiment.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExperimentDocument {
    pub(crate) schema_version: u32,
    pub(crate) id: String,
    pub(crate) input: InputRef,
    #[serde(default)]
    pub(crate) chain: ChainSpec,
    pub(crate) case: CaseRef,
    pub(crate) grid: GridSpec,
    #[serde(default = "default_seeds")]
    pub(crate) seeds: Vec<u32>,
}

fn default_seeds() -> Vec<u32> {
    vec![0]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputRef {
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
}

/// The parameter chain, using the process surface's names and precedence.
/// An omitted field takes the process default.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChainSpec {
    pub(crate) film: Option<String>,
    pub(crate) paper: Option<String>,
    #[serde(default)]
    pub(crate) preset: Vec<String>,
    pub(crate) params: Option<PathBuf>,
    #[serde(default)]
    pub(crate) set: Vec<String>,
    pub(crate) route: Option<String>,
    pub(crate) scan_output: Option<String>,
    /// Raster source primaries; required for a raster input, as on the
    /// process surface.
    pub(crate) input_color_space: Option<String>,
    /// Whether raster samples require transfer decoding.
    pub(crate) input_cctf_decoding: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseRef {
    pub(crate) id: String,
    pub(crate) suite: PathBuf,
}

/// The patch grid the case's regions must reproduce.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GridSpec {
    pub(crate) rows: u32,
    pub(crate) cols: u32,
    pub(crate) patch_px: u32,
    pub(crate) gap_px: u32,
    pub(crate) origin: [u32; 2],
    pub(crate) patch: BTreeMap<String, [u32; 2]>,
}

/// Read the document and reject duplicate keys, which `serde_json` accepts.
fn read_document(path: &Path) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading experiment: {}", path.display()))?;
    scan_duplicate_keys(&bytes).map_err(|error| anyhow!("{}: {error}", path.display()))?;
    String::from_utf8(bytes).map_err(|error| anyhow!("{}: not UTF-8: {error}", path.display()))
}

/// Parse the document into the typed declaration, rejecting unknown fields
/// and a non-object root.
fn parse_document(text: &str, path: &Path) -> Result<ExperimentDocument> {
    let root: Value = serde_json::from_str(text)
        .with_context(|| format!("parsing experiment: {}", path.display()))?;
    if !root.is_object() {
        bail!(
            "{}: experiment document root must be an object",
            path.display()
        );
    }
    let document: ExperimentDocument = serde_json::from_value(root.clone())
        .map_err(|error| anyhow!("{}: {error}", path.display()))?;
    if document.schema_version != SCHEMA_VERSION {
        bail!(
            "{}: unsupported experiment schema version {}; this command reads {SCHEMA_VERSION}",
            path.display(),
            document.schema_version
        );
    }
    if !matches_id(&document.id) {
        bail!(
            "{}: experiment id {:?} must match {ID_PATTERN}",
            path.display(),
            document.id
        );
    }
    if document.seeds.is_empty() {
        bail!("{}: seeds must declare at least one seed", path.display());
    }
    if document.grid.rows == 0 || document.grid.cols == 0 {
        bail!("{}: grid rows and cols must be positive", path.display());
    }
    if document.grid.patch.is_empty() {
        bail!(
            "{}: grid patch must name at least one region",
            path.display()
        );
    }
    for selector in &document.chain.preset {
        if selector.contains('/') || selector.contains('\\') {
            bail!(
                "{}: chain preset {:?} must be a preset name or a .toml/.json selector",
                path.display(),
                selector
            );
        }
    }
    Ok(document)
}

/// A character-class test of the anchored identifier pattern, kept inline so
/// the document contract carries no regex dependency.
fn matches_id(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Walk the raw bytes and reject an object that repeats a key. `serde_json`
/// keeps the last occurrence, so the document contract checks it directly.
fn scan_duplicate_keys(bytes: &[u8]) -> Result<(), String> {
    const MAX_DEPTH: usize = 128;
    fn whitespace(bytes: &[u8], mut index: usize) -> usize {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        index
    }
    fn string(bytes: &[u8], mut index: usize) -> Result<usize, String> {
        index += 1;
        while index < bytes.len() {
            match bytes[index] {
                b'\\' => index += 2,
                b'"' => return Ok(index + 1),
                _ => index += 1,
            }
        }
        Err("unterminated JSON string".to_owned())
    }
    fn object(bytes: &[u8], mut index: usize, depth: usize) -> Result<usize, String> {
        if depth > MAX_DEPTH {
            return Err("JSON nesting is too deep".to_owned());
        }
        index = whitespace(bytes, index + 1);
        if bytes.get(index) == Some(&b'}') {
            return Ok(index + 1);
        }
        let mut keys = BTreeSet::new();
        loop {
            if bytes.get(index) != Some(&b'"') {
                return Err("object key must be a string".to_owned());
            }
            let end = string(bytes, index)?;
            let key: String = serde_json::from_slice(&bytes[index..end])
                .map_err(|error| format!("object key: {error}"))?;
            if !keys.insert(key.clone()) {
                return Err(format!("duplicate JSON key: {key}"));
            }
            index = whitespace(bytes, end);
            if bytes.get(index) != Some(&b':') {
                return Err("object key missing colon".to_owned());
            }
            index = whitespace(bytes, scan(bytes, whitespace(bytes, index + 1), depth + 1)?);
            match bytes.get(index) {
                Some(b',') => index = whitespace(bytes, index + 1),
                Some(b'}') => return Ok(index + 1),
                _ => return Err("invalid object separator".to_owned()),
            }
        }
    }
    fn array(bytes: &[u8], mut index: usize, depth: usize) -> Result<usize, String> {
        if depth > MAX_DEPTH {
            return Err("JSON nesting is too deep".to_owned());
        }
        index = whitespace(bytes, index + 1);
        if bytes.get(index) == Some(&b']') {
            return Ok(index + 1);
        }
        loop {
            index = whitespace(bytes, scan(bytes, index, depth + 1)?);
            match bytes.get(index) {
                Some(b',') => index = whitespace(bytes, index + 1),
                Some(b']') => return Ok(index + 1),
                _ => return Err("invalid array separator".to_owned()),
            }
        }
    }
    fn scan(bytes: &[u8], index: usize, depth: usize) -> Result<usize, String> {
        let index = whitespace(bytes, index);
        if index >= bytes.len() {
            return Err("empty JSON document".to_owned());
        }
        match bytes[index] {
            b'{' => object(bytes, index, depth),
            b'[' => array(bytes, index, depth),
            b'"' => string(bytes, index),
            _ => {
                let start = index;
                let mut index = index;
                while index < bytes.len()
                    && !bytes[index].is_ascii_whitespace()
                    && !matches!(bytes[index], b',' | b']' | b'}')
                {
                    index += 1;
                }
                let token = std::str::from_utf8(&bytes[start..index])
                    .map_err(|error| format!("invalid UTF-8: {error}"))?;
                if matches!(token, "NaN" | "Infinity" | "-Infinity")
                    || token.parse::<f64>().is_ok_and(|value| !value.is_finite())
                {
                    return Err("JSON numbers must be finite".to_owned());
                }
                Ok(index)
            }
        }
    }
    let end = scan(bytes, 0, 0)?;
    if whitespace(bytes, end) != bytes.len() {
        return Err("trailing content after the JSON document".to_owned());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Grid validation
// ---------------------------------------------------------------------------

/// One case region as the suite declares it.
#[derive(Debug, Clone)]
struct CaseRegion {
    id: String,
    geometry: [u32; 4],
}

/// One suite case, as a raw record plus its parsed regions.
type SuiteCase = (Value, Vec<CaseRegion>);

/// Read a benchmark suite and parse the referenced case out of it. Only the
/// referenced case is consumed; the rest of the suite stays unread.
fn load_suite(path: &Path, case_id: &str) -> Result<SuiteCase> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading suite: {}", path.display()))?;
    scan_duplicate_keys(text.as_bytes()).map_err(|error| anyhow!("{}: {error}", path.display()))?;
    let root: Value = serde_json::from_str(&text)
        .with_context(|| format!("parsing suite: {}", path.display()))?;
    let cases = root
        .get("cases")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("{}: suite carries no cases array", path.display()))?;
    let case = cases
        .iter()
        .find(|case| case["id"].as_str() == Some(case_id))
        .ok_or_else(|| {
            let names: Vec<&str> = cases
                .iter()
                .filter_map(|case| case["id"].as_str())
                .collect();
            anyhow!(
                "case {case_id:?} is not declared by {}; declared cases are {}",
                path.display(),
                names.join(", ")
            )
        })?;
    let regions = case
        .get("regions")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("case {case_id:?} declares no regions array"))?;
    if regions.is_empty() {
        bail!("case {case_id:?} declares an empty regions array");
    }
    let mut parsed = Vec::with_capacity(regions.len());
    for region in regions {
        let id = region
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("case {case_id:?} declares a region without an id"))?
            .to_owned();
        let geometry = region
            .get("geometry")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("case region {id:?} declares no geometry array"))?;
        if geometry.len() != 4 || geometry.iter().any(|value| value.as_u64().is_none()) {
            bail!("case region {id:?} geometry must be four nonnegative integers");
        }
        parsed.push(CaseRegion {
            id: id.clone(),
            geometry: [
                u32::try_from(geometry[0].as_u64().unwrap())
                    .map_err(|_| anyhow!("case region {id:?} geometry[0] exceeds u32"))?,
                u32::try_from(geometry[1].as_u64().unwrap())
                    .map_err(|_| anyhow!("case region {id:?} geometry[1] exceeds u32"))?,
                u32::try_from(geometry[2].as_u64().unwrap())
                    .map_err(|_| anyhow!("case region {id:?} geometry[2] exceeds u32"))?,
                u32::try_from(geometry[3].as_u64().unwrap())
                    .map_err(|_| anyhow!("case region {id:?} geometry[3] exceeds u32"))?,
            ],
        });
    }
    Ok((case.clone(), parsed))
}

/// Check the declared grid reproduces the case's regions.
fn validate_grid(grid: &GridSpec, case: &SuiteCase) -> Result<()> {
    let (case_value, regions) = case;
    let case_id = case_value["id"].as_str().unwrap_or_default();
    let mut max_row = 0;
    let mut max_col = 0;
    for region in regions {
        let position = grid.patch.get(&region.id).ok_or_else(|| {
            anyhow!(
                "case region {:?} is absent from the experiment grid; \
                 patch must name every region the case declares",
                region.id
            )
        })?;
        max_row = max_row.max(position[0]);
        max_col = max_col.max(position[1]);
        let step = grid
            .patch_px
            .checked_add(grid.gap_px)
            .ok_or_else(|| anyhow!("grid patch_px + gap_px overflows u32"))?;
        let expected = [
            grid.origin[0]
                .checked_add(position[1].checked_mul(step).ok_or_else(|| {
                    anyhow!("grid origin[0] + col * (patch_px + gap_px) overflows u32")
                })?)
                .ok_or_else(|| {
                    anyhow!("grid origin[0] + col * (patch_px + gap_px) overflows u32")
                })?,
            grid.origin[1]
                .checked_add(position[0].checked_mul(step).ok_or_else(|| {
                    anyhow!("grid origin[1] + row * (patch_px + gap_px) overflows u32")
                })?)
                .ok_or_else(|| {
                    anyhow!("grid origin[1] + row * (patch_px + gap_px) overflows u32")
                })?,
            grid.patch_px,
            grid.patch_px,
        ];
        if region.geometry != expected {
            bail!(
                "case region {:?} geometry {:?} does not match the declared grid \
                 at row {} column {}: expected {:?}",
                region.id,
                region.geometry,
                position[0],
                position[1],
                expected
            );
        }
    }
    for id in grid.patch.keys() {
        if !regions.iter().any(|region| &region.id == id) {
            bail!(
                "the experiment grid names {id:?}, which case {case_id:?} does not declare as a region"
            );
        }
    }
    let expected_rows = max_row
        .checked_add(1)
        .ok_or_else(|| anyhow!("grid position overflows u32"))?;
    let expected_cols = max_col
        .checked_add(1)
        .ok_or_else(|| anyhow!("grid position overflows u32"))?;
    if grid.rows != expected_rows || grid.cols != expected_cols {
        bail!(
            "the experiment grid declares {} rows and {} columns, but the largest \
             declared position is row {} column {}",
            grid.rows,
            grid.cols,
            max_row,
            max_col
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Paths and digests
// ---------------------------------------------------------------------------

/// Absolute form of a path without touching the filesystem, so a declared
/// path stays readable even when the file it names does not exist yet.
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("path must not be empty");
    }
    if path.is_absolute() {
        Ok(lexical(path))
    } else {
        Ok(lexical(&std::env::current_dir()?.join(path)))
    }
}

/// Remove `.` and `..` components lexically. Symlinks are deliberately not
/// resolved: the containment test stays lexical so an authored parent-relative
/// path still matches, and a physically escaping link is reported by the
/// digest check, which reads the real file.
fn lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    if normalized.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        normalized
    }
}

/// Directory the document lives in, absolute.
fn document_root(path: &Path) -> Result<PathBuf> {
    Ok(absolute(path)?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/")))
}

/// Resolve a declared path against the document's directory and require the
/// result to stay inside it. An escaping path is a command failure.
fn resolve_inside(value: &Path, root: &Path, field: &str) -> Result<PathBuf> {
    let full = if value.is_absolute() {
        lexical(value)
    } else {
        lexical(&root.join(value))
    };
    if full != *root && !full.starts_with(root) {
        bail!(
            "{field} {:?} escapes the experiment document's directory tree",
            value.display()
        );
    }
    // Also check for symlink escapes: canonicalize the full path and verify
    // it still starts with the canonicalized root. A symlink inside the tree
    // pointing outside it would pass the lexical check above.
    if let Ok(canonical) = std::fs::canonicalize(&full) {
        let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        if canonical != canonical_root && !canonical.starts_with(&canonical_root) {
            bail!(
                "{field} {:?} escapes the experiment document's directory tree (via symlink)",
                value.display()
            );
        }
    }
    Ok(full)
}

/// Resolve every `--set` file source inside the document's directory tree.
/// Inline assignment sources carry no path and pass through unchanged.
fn resolve_set_sources(sources: &[String], root: &Path) -> Result<Vec<PathBuf>> {
    sources
        .iter()
        .map(|source| match file_source_path(source) {
            Some(path) => resolve_inside(Path::new(path), root, "chain.set"),
            None => Ok(PathBuf::from(source)),
        })
        .collect()
}

/// The shared classification of a `--set` source: an inline assignment
/// (`PATH=VALUE`) is never a path, and only a `.toml`/`.json` carrier is a
/// file. Mirrors the process surface's own rule.
fn file_source_path(source: &str) -> Option<&str> {
    let assignment = source
        .split_once('=')
        .is_some_and(|(path, _)| path.trim().split('.').all(is_identifier));
    (!assignment && (source.ends_with(".toml") || source.ends_with(".json"))).then_some(source)
}

/// One dot-separated parameter-path identifier.
fn is_identifier(segment: &str) -> bool {
    let mut characters = segment.chars();
    match characters.next() {
        Some(first) if first.is_ascii_lowercase() || first == '_' => {
            characters.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        }
        _ => false,
    }
}

/// Verify the declared input file exists and hashes to the declared digest.
fn verify_input_digest(path: &Path, expected: &str) -> Result<()> {
    if !path.is_file() {
        bail!("input is not a regular file: {}", path.display());
    }
    let bytes =
        std::fs::read(path).with_context(|| format!("reading input: {}", path.display()))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected {
        bail!(
            "input {} hashes to {} but the experiment declares {}",
            path.display(),
            actual,
            expected
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Chain resolution
// ---------------------------------------------------------------------------

/// The resolved chain of the executed run.
struct ResolvedChain {
    film_name: String,
    print_name: String,
    film: Value,
    print: Value,
    params: Value,
    input_declaration: Value,
    edits: Value,
}

/// Resolve the declared chain through the shared resolver, so the retained
/// records carry exactly the chain the executed run used.
fn resolve_chain(
    document: &ExperimentDocument,
    document_root: &Path,
    data_dir: &Path,
) -> Result<ResolvedChain> {
    let library = PresetLibrary::default();
    let presets = document
        .chain
        .preset
        .iter()
        .map(|selector| crate::preset::select(&library, selector).map(|selected| selected.preset))
        .collect::<Result<Vec<_>>>()?;
    let set_sources: Vec<String> = resolve_set_sources(&document.chain.set, document_root)?
        .iter()
        .map(|source| source.display().to_string())
        .collect();
    let params_file = document
        .chain
        .params
        .as_ref()
        .map(|path| resolve_inside(path, document_root, "chain.params"))
        .transpose()?;
    let input_is_raw = document
        .input
        .path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            spektrafilm_core::image_io::accepted_extensions()
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
                && !spektrafilm_core::image_io::container_extensions()
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(extension))
        });
    let resolved = spektrafilm_core::process_params::resolve(
        spektrafilm_core::process_params::ProcessParamsRequest {
            film: document.chain.film.as_deref(),
            paper: document.chain.paper.as_deref(),
            presets: &presets,
            params_file: params_file.as_deref(),
            sources: &set_sources,
            route: document.chain.route.as_deref(),
            scan_film: false,
            scan_output: document.chain.scan_output.as_deref(),
            input_is_raw,
            input_color_space: document.chain.input_color_space.as_deref(),
            input_cctf_decoding: document.chain.input_cctf_decoding,
            require_input_declaration: true,
            digest_mode: spektrafilm_core::runtime::DigestMode::ApplyStockSpecifics,
        },
        data_dir,
    )
    .map_err(anyhow::Error::msg)?;
    Ok(ResolvedChain {
        film_name: resolved.film_name,
        print_name: resolved.print_name,
        film: serde_json::to_value(&resolved.film).map_err(anyhow::Error::msg)?,
        print: serde_json::to_value(&resolved.print).map_err(anyhow::Error::msg)?,
        params: serde_json::to_value(&resolved.params).map_err(anyhow::Error::msg)?,
        input_declaration: serde_json::to_value(&resolved.input_declaration)
            .map_err(anyhow::Error::msg)?,
        edits: serde_json::to_value(&resolved.edits).map_err(anyhow::Error::msg)?,
    })
}

// ---------------------------------------------------------------------------
// Run records
// ---------------------------------------------------------------------------

/// Everything the three retained records state about the executed run.
struct Records<'a> {
    document: &'a ExperimentDocument,
    document_path: &'a Path,
    suite_path: &'a Path,
    resolved: ResolvedChain,
    manifest: Value,
    command_vector: Vec<String>,
}

/// Write `resolved.json`, `command.json`, and `report.json` beside the
/// published bundle. A failed write unlinks whatever this call created so no
/// partial record set remains.
fn write_records(out: &Path, force: bool, records: &Records<'_>) -> Result<()> {
    let Records {
        document,
        document_path,
        suite_path,
        resolved,
        manifest,
        command_vector,
    } = records;
    let resolved_text = serde_json::to_string_pretty(&json!({
        "schema_version": 1,
        "experiment": document.id,
        "profiles": {
            "film": {"name": resolved.film_name, "document": resolved.film},
            "print": {"name": resolved.print_name, "document": resolved.print},
        },
        "parameters": resolved.params,
        "input": resolved.input_declaration,
        "edits": resolved.edits,
        "seed": document.seeds[0],
    }))?;
    let command_text = serde_json::to_string_pretty(&json!({
        "argv": command_vector,
        "cwd": std::env::current_dir().ok().map(|path| path.display().to_string()),
    }))?;
    let report_text = serde_json::to_string_pretty(&json!({
        "schema_version": 1,
        "tool": {"name": "spektrafilm-lab", "version": env!("CARGO_PKG_VERSION")},
        "experiment": {
            "id": document.id,
            "document": document_path.display().to_string(),
            "seeds": document.seeds,
        },
        "input": {
            "path": document.input.path.display().to_string(),
            "sha256": document.input.sha256,
        },
        "chain": {
            "film": document.chain.film,
            "paper": document.chain.paper,
            "preset": document.chain.preset,
            "params": document.chain.params,
            "set": document.chain.set,
            "route": document.chain.route,
            "scan_output": document.chain.scan_output,
            "input_color_space": document.chain.input_color_space,
            "input_cctf_decoding": document.chain.input_cctf_decoding,
            "resolved": {
                "film": resolved.film_name,
                "print": resolved.print_name,
            },
        },
        "case": {
            "id": document.case.id,
            "suite": suite_path.display().to_string(),
            "regions": document.grid.patch.keys().collect::<Vec<_>>(),
        },
        "grid": {
            "rows": document.grid.rows,
            "cols": document.grid.cols,
            "patch_px": document.grid.patch_px,
            "gap_px": document.grid.gap_px,
            "origin": document.grid.origin,
            "patch": document.grid.patch,
        },
        "bundle": manifest,
    }))?;
    report::write_named(
        out,
        &[
            ("report.json", report_text.as_str()),
            ("resolved.json", resolved_text.as_str()),
            ("command.json", command_text.as_str()),
        ],
        "refusing to replace an existing report",
        force,
    )
    .map_err(anyhow::Error::msg)
}

/// The full `process` argument vector the executed run used, including the
/// `--stage-evidence` destination and every `--set` source, so a retained run
/// is reproducible.
fn command_vector(
    output_format: OutputFormatArg,
    args: &ProcessArgs,
    set_sources: &[PathBuf],
) -> Vec<String> {
    let mut argv = vec![
        "process".to_owned(),
        args.input.display().to_string(),
        "--output".to_owned(),
        args.output
            .as_ref()
            .expect("output is set")
            .display()
            .to_string(),
        "--format".to_owned(),
        output_format.into_name().to_owned(),
        "--backend".to_owned(),
        "cpu".to_owned(),
        "--stage-evidence".to_owned(),
        args.stage_evidence
            .as_ref()
            .expect("stage evidence is set")
            .display()
            .to_string(),
        "--seed".to_owned(),
        args.seed.expect("seed is set").to_string(),
        "--iters".to_owned(),
        args.iters.to_string(),
        // --experiment is not a process flag; the document is recorded in report.json.
        // It is omitted so the vector is a replayable process invocation.
    ];
    if let Some(film) = &args.film {
        argv.push("--film".to_owned());
        argv.push(film.clone());
    }
    if let Some(paper) = &args.paper {
        argv.push("--paper".to_owned());
        argv.push(paper.clone());
    }
    for selector in &args.preset {
        argv.push("--preset".to_owned());
        argv.push(selector.clone());
    }
    if let Some(params) = &args.params {
        argv.push("--params".to_owned());
        argv.push(params.display().to_string());
    }
    for source in set_sources {
        argv.push("--set".to_owned());
        argv.push(source.display().to_string());
    }
    if let Some(route) = &args.workflow.route {
        argv.push("--route".to_owned());
        argv.push(route.clone());
    }
    if let Some(scan_output) = &args.workflow.scan_output {
        argv.push("--scan-output".to_owned());
        argv.push(scan_output.clone());
    }
    if let Some(color_space) = &args.input_color_space {
        argv.push("--input-color-space".to_owned());
        argv.push(color_space.clone());
    }
    if let Some(decoding) = args.input_cctf_decoding {
        argv.push("--input-cctf-decoding".to_owned());
        argv.push(decoding.to_string());
    }
    argv
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Validate a bundle against the stage-evidence contract before converting.
/// The manifest is the authority for stage identity and layout.
fn validate_bundle(manifest: &Value) -> Result<()> {
    let schema_version = manifest
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("bundle manifest carries no schema_version"))?;
    if schema_version != u64::from(BUNDLE_SCHEMA_VERSION) {
        bail!(
            "unsupported bundle schema version {schema_version}; this command reads {BUNDLE_SCHEMA_VERSION}"
        );
    }
    let status = manifest
        .get("execution_status")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("bundle manifest carries no execution_status"))?;
    if status != "completed" {
        let errors = manifest
            .get("errors")
            .and_then(Value::as_array)
            .map(|errors| {
                errors
                    .iter()
                    .map(|error| error.as_str().unwrap_or_default().to_owned())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        if errors.is_empty() {
            bail!(
                "bundle execution_status is {status:?}; a bundle that did not complete is not convertible"
            );
        }
        bail!("bundle execution_status is {status:?}: {errors}");
    }
    let precision = manifest
        .get("host_precision")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("bundle manifest carries no host_precision"))?;
    if precision != "f64" {
        bail!(
            "bundle host_precision is {precision:?}; the lab converts f64 buffers only — \
             re-render the experiment with an f64 reference bundle"
        );
    }
    let stages = manifest
        .get("stages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("bundle manifest carries no stages array"))?;
    let mut incomplete = Vec::new();
    for stage in stages {
        let id = stage["id"].as_str().unwrap_or_default();
        match stage["availability"].as_str() {
            Some("available") => {}
            Some("unavailable") => incomplete.push(format!("{id} is unavailable")),
            Some("not_applicable") => {}
            other => bail!("bundle stage {id:?} carries an unknown availability {other:?}"),
        }
    }
    if !incomplete.is_empty() {
        bail!("bundle is incomplete: {}", incomplete.join("; "));
    }
    Ok(())
}

/// Select the requested stages from the manifest's available records, in
/// manifest order. An unknown `--stage` names the available stages.
fn select_stages<'a>(available: &[&'a Value], requested: &[String]) -> Result<Vec<&'a Value>> {
    if requested.is_empty() {
        return Ok(available.to_vec());
    }
    let names: Vec<&str> = available
        .iter()
        .map(|stage| stage["id"].as_str().unwrap_or_default())
        .collect();
    requested
        .iter()
        .map(|id| {
            available
                .iter()
                .find(|stage| stage["id"].as_str() == Some(id.as_str()))
                .copied()
                .ok_or_else(|| {
                    anyhow!(
                        "stage {id:?} is not available in this bundle; available stages are {names:?}"
                    )
                })
        })
        .collect()
}

/// One converted array, as recorded in `export.json`.
#[derive(Debug)]
struct ConvertedArray {
    id: String,
    path: PathBuf,
    sha256: String,
    shape: Vec<usize>,
}

/// Convert one available stage record into an array, validating the buffer
/// against the manifest's own layout record, then writing it and hashing the
/// file that was written.
fn convert_stage(
    bundle_dir: &Path,
    out: &Path,
    force: bool,
    stage: &Value,
) -> Result<ConvertedArray> {
    let id = stage["id"]
        .as_str()
        .ok_or_else(|| anyhow!("bundle stage record carries no id"))?
        .to_owned();
    let relative = stage["path"]
        .as_str()
        .ok_or_else(|| anyhow!("bundle stage {id:?} carries no buffer path"))?;
    if relative != format!("{id}.f64") {
        bail!(
            "bundle stage {id:?} names buffer {relative:?}; a stage's buffer is its own <id>.f64 file"
        );
    }
    let buffer_path = bundle_dir.join(relative);
    let bytes = std::fs::read(&buffer_path)
        .with_context(|| format!("reading bundle buffer: {}", buffer_path.display()))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    let declared = stage["sha256"]
        .as_str()
        .ok_or_else(|| anyhow!("bundle stage {id:?} carries no sha256"))?;
    if actual != declared {
        bail!(
            "bundle stage {id:?} buffer {} hashes to {} but the manifest declares {}",
            buffer_path.display(),
            actual,
            declared
        );
    }
    let layout = stage
        .get("layout")
        .ok_or_else(|| anyhow!("bundle stage {id:?} carries no layout"))?;
    let width = layout["width"]
        .as_u64()
        .ok_or_else(|| anyhow!("bundle stage {id:?} layout carries no width"))?;
    let height = layout["height"]
        .as_u64()
        .ok_or_else(|| anyhow!("bundle stage {id:?} layout carries no height"))?;
    let channels = layout["channels"]
        .as_u64()
        .ok_or_else(|| anyhow!("bundle stage {id:?} layout carries no channels"))?;
    if layout["dtype"].as_str() != Some("f64") {
        bail!(
            "bundle stage {id:?} layout dtype is {:?}; the lab converts f64 buffers",
            layout["dtype"]
        );
    }
    if layout["endianness"].as_str() != Some("little") {
        bail!(
            "bundle stage {id:?} layout endianness is {:?}; the lab converts little-endian buffers",
            layout["endianness"]
        );
    }
    let declared_bytes = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(channels))
        .and_then(|samples| samples.checked_mul(8))
        .ok_or_else(|| anyhow!("bundle stage {id:?} layout overflows"))?;
    if bytes.len() as u64 != declared_bytes {
        bail!(
            "bundle stage {id:?} buffer is {} bytes but its layout declares {declared_bytes} \
             ({width}x{height}x{channels} f64 samples)",
            bytes.len()
        );
    }
    // The values are the buffer's bytes exactly: no scaling, no channel
    // permutation, no clipping.
    let samples = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect::<Vec<f64>>();
    if !samples.iter().all(|sample| sample.is_finite()) {
        bail!("bundle stage {id:?} carries a non-finite sample; export is a lossless conversion");
    }
    let shape = vec![height as usize, width as usize, channels as usize];
    let path = out.join(format!("{id}.npy"));
    if !force && path.exists() {
        bail!(
            "refusing to replace an existing array in {}; pass --force to overwrite it",
            out.display()
        );
    }
    // Write to a temporary file then rename, so a mid-sequence failure
    // never leaves a partial array that looks complete.
    let tmp_path = out.join(format!("{id}.npy.tmp"));
    spektrafilm_math::npy::save_npy_f64(&tmp_path, &samples, &shape)
        .map_err(|error| anyhow!("writing {}: {error}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &path)
        .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()))?;
    let written =
        std::fs::read(&path).with_context(|| format!("reading back {}", path.display()))?;
    Ok(ConvertedArray {
        id,
        path,
        sha256: format!("{:x}", Sha256::digest(&written)),
        shape,
    })
}

/// Whether a run destination already holds a record this run would replace.
fn report_exists(out: &Path, output_image: &Path) -> bool {
    ["report.json", "resolved.json", "command.json"]
        .iter()
        .any(|name| out.join(name).exists())
        || output_image.exists()
}

impl From<OutputFormatArg> for OutputFormat {
    fn from(format: OutputFormatArg) -> Self {
        match format {
            OutputFormatArg::Jpeg => Self::Jpeg,
            OutputFormatArg::Png => Self::Png,
            OutputFormatArg::Tiff => Self::Tiff,
            OutputFormatArg::Exr => Self::Exr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory unique per call site, so tests never collide.
    fn scratch() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "spektrafilm-lab-experiment-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn digest_of(text: &str) -> String {
        format!("{:x}", Sha256::digest(text.as_bytes()))
    }

    fn document_text(patch: &str, rows: u32, cols: u32) -> String {
        format!(
            r#"{{
                "schema_version": 1,
                "id": "experiment",
                "input": {{"path": "input.png", "sha256": "{}"}},
                "case": {{"id": "case", "suite": "suite.json"}},
                "grid": {{
                    "rows": {rows}, "cols": {cols},
                    "patch_px": 4, "gap_px": 2, "origin": [0, 0],
                    "patch": {patch}
                }},
                "seeds": [0]
            }}"#,
            "0".repeat(64)
        )
    }

    fn suite_text(regions: &[(&str, [u32; 4])]) -> String {
        let regions: Vec<Value> = regions
            .iter()
            .map(|(id, geometry)| {
                json!({
                    "id": id,
                    "label": id,
                    "role": "color",
                    "geometry": geometry,
                    "measurements": [],
                })
            })
            .collect();
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "id": "suite",
            "protocol": {"id": "appearance-v1"},
            "cases": [{
                "id": "case",
                "kind": "probe",
                "input": {"path": "input.png", "sha256": "0".repeat(64)},
                "configuration": {},
                "seeds": [0],
                "regions": regions,
                "measurements": [],
            }],
        }))
        .unwrap()
    }

    fn load_case(regions: &[(&str, [u32; 4])]) -> SuiteCase {
        let dir = scratch();
        let suite = dir.join("suite.json");
        std::fs::write(&suite, suite_text(regions)).unwrap();
        load_suite(&suite, "case").unwrap()
    }

    fn grid(patch: &[(&str, [u32; 2])], rows: u32, cols: u32) -> GridSpec {
        GridSpec {
            rows,
            cols,
            patch_px: 4,
            gap_px: 2,
            origin: [0, 0],
            patch: patch
                .iter()
                .map(|(id, position)| ((*id).to_owned(), *position))
                .collect(),
        }
    }

    fn parse(text: &str) -> Result<ExperimentDocument> {
        let dir = scratch();
        let path = dir.join("experiment.json");
        std::fs::write(&path, text).unwrap();
        parse_document(text, &path)
    }

    // -- document parsing --------------------------------------------------

    #[test]
    fn parses_a_valid_document() {
        let document = parse(&document_text(
            r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0], "d": [1, 1]}"#,
            2,
            2,
        ))
        .unwrap();
        assert_eq!(document.schema_version, 1);
        assert_eq!(document.id, "experiment");
        assert_eq!(document.seeds, vec![0]);
        assert_eq!(document.grid.patch.len(), 4);
        assert!(document.chain.film.is_none());
        assert!(document.chain.set.is_empty());
    }

    #[test]
    fn rejects_a_duplicate_key() {
        let text = document_text(
            r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0], "d": [1, 1]}"#,
            2,
            2,
        )
        .replace(r#""seeds": [0]"#, r#""seeds": [0], "seeds": [1]"#);
        let error = scan_duplicate_keys(text.as_bytes()).unwrap_err();
        assert!(error.contains("duplicate JSON key: seeds"), "{error}");
        let dir = scratch();
        let path = dir.join("experiment.json");
        std::fs::write(&path, &text).unwrap();
        let error = read_document(&path).unwrap_err().to_string();
        assert!(error.contains("duplicate JSON key: seeds"), "{error}");
    }

    #[test]
    fn rejects_a_duplicate_key_inside_a_nested_object() {
        let text = document_text(
            r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0], "d": [1, 1]}"#,
            2,
            2,
        )
        .replace(
            r#""input": {"path": "input.png", "sha256": ""#,
            r#""input": {"path": "input.png", "path": "other.png", "sha256": ""#,
        );
        let error = scan_duplicate_keys(text.as_bytes()).unwrap_err();
        assert!(error.contains("duplicate JSON key: path"), "{error}");
    }

    #[test]
    fn rejects_an_unknown_field() {
        let text = document_text(
            r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0], "d": [1, 1]}"#,
            2,
            2,
        )
        .replace(r#""seeds": [0]"#, r#""seeds": [0], "unknown": 1"#);
        let error = parse(&text).unwrap_err().to_string();
        assert!(error.contains("unknown field"), "{error}");
    }

    #[test]
    fn rejects_an_unsupported_schema_version() {
        let text = document_text(
            r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0], "d": [1, 1]}"#,
            2,
            2,
        )
        .replace(r#""schema_version": 1"#, r#""schema_version": 2"#);
        let error = parse(&text).unwrap_err().to_string();
        assert!(
            error.contains("unsupported experiment schema version 2"),
            "{error}"
        );
    }

    #[test]
    fn rejects_an_invalid_id() {
        let text = document_text(
            r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0], "d": [1, 1]}"#,
            2,
            2,
        )
        .replace(r#""id": "experiment""#, r#""id": "-bad id""#);
        let error = parse(&text).unwrap_err().to_string();
        assert!(error.contains("must match"), "{error}");
    }

    #[test]
    fn rejects_an_empty_seeds_array() {
        let text = document_text(
            r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0], "d": [1, 1]}"#,
            2,
            2,
        )
        .replace(r#""seeds": [0]"#, r#""seeds": []"#);
        let error = parse(&text).unwrap_err().to_string();
        assert!(error.contains("at least one seed"), "{error}");
    }

    #[test]
    fn rejects_a_zero_grid_extent() {
        let text = document_text(r#"{"a": [0, 0]}"#, 0, 1);
        let error = parse(&text).unwrap_err().to_string();
        assert!(error.contains("rows and cols must be positive"), "{error}");
    }

    #[test]
    fn rejects_a_non_finite_number() {
        let text = document_text(
            r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0], "d": [1, 1]}"#,
            2,
            2,
        )
        .replace(r#""gap_px": 2"#, r#""gap_px": NaN"#);
        let error = scan_duplicate_keys(text.as_bytes()).unwrap_err();
        assert!(error.contains("finite"), "{error}");
    }

    // -- grid validation ---------------------------------------------------

    #[test]
    fn accepts_a_matching_grid() {
        let regions = [
            ("a", [0, 0, 4, 4]),
            ("b", [6, 0, 4, 4]),
            ("c", [0, 6, 4, 4]),
            ("d", [6, 6, 4, 4]),
        ];
        let case = load_case(&regions);
        let declared = grid(
            &[("a", [0, 0]), ("b", [0, 1]), ("c", [1, 0]), ("d", [1, 1])],
            2,
            2,
        );
        validate_grid(&declared, &case).unwrap();
    }

    #[test]
    fn accepts_a_grid_with_a_nonzero_origin() {
        let regions = [
            ("a", [10, 20, 4, 4]),
            ("b", [16, 20, 4, 4]),
            ("c", [10, 26, 4, 4]),
            ("d", [16, 26, 4, 4]),
        ];
        let case = load_case(&regions);
        let mut declared = grid(
            &[("a", [0, 0]), ("b", [0, 1]), ("c", [1, 0]), ("d", [1, 1])],
            2,
            2,
        );
        declared.origin = [10, 20];
        validate_grid(&declared, &case).unwrap();
    }

    #[test]
    fn rejects_a_region_absent_from_patch() {
        let regions = [
            ("a", [0, 0, 4, 4]),
            ("b", [6, 0, 4, 4]),
            ("c", [0, 6, 4, 4]),
            ("d", [6, 6, 4, 4]),
        ];
        let case = load_case(&regions);
        let declared = grid(&[("a", [0, 0]), ("b", [0, 1]), ("c", [1, 0])], 2, 2);
        let error = validate_grid(&declared, &case).unwrap_err().to_string();
        assert!(error.contains("absent from the experiment grid"), "{error}");
        assert!(error.contains("\"d\""), "{error}");
    }

    #[test]
    fn rejects_a_patch_entry_the_case_does_not_declare() {
        let regions = [("a", [0, 0, 4, 4]), ("b", [6, 0, 4, 4])];
        let case = load_case(&regions);
        let declared = grid(&[("a", [0, 0]), ("b", [0, 1]), ("ghost", [1, 0])], 2, 2);
        let error = validate_grid(&declared, &case).unwrap_err().to_string();
        assert!(error.contains("does not declare"), "{error}");
        assert!(error.contains("ghost"), "{error}");
    }

    #[test]
    fn rejects_a_geometry_that_does_not_match_the_grid() {
        let regions = [("a", [1, 0, 4, 4]), ("b", [6, 0, 4, 4])];
        let case = load_case(&regions);
        let declared = grid(&[("a", [0, 0]), ("b", [0, 1])], 1, 2);
        let error = validate_grid(&declared, &case).unwrap_err().to_string();
        assert!(
            error.contains("does not match the declared grid"),
            "{error}"
        );
        assert!(error.contains("[1, 0, 4, 4]"), "{error}");
    }

    #[test]
    fn rejects_a_declared_extent_that_does_not_match_the_grid() {
        let regions = [("a", [0, 0, 4, 4]), ("b", [6, 0, 4, 4])];
        let case = load_case(&regions);
        let declared = grid(&[("a", [0, 0]), ("b", [0, 1])], 3, 2);
        let error = validate_grid(&declared, &case).unwrap_err().to_string();
        assert!(error.contains("largest declared position"), "{error}");
    }

    #[test]
    fn rejects_a_case_the_suite_does_not_declare() {
        let dir = scratch();
        let suite = dir.join("suite.json");
        std::fs::write(&suite, suite_text(&[("a", [0, 0, 4, 4])])).unwrap();
        let error = load_suite(&suite, "absent").unwrap_err().to_string();
        assert!(error.contains("is not declared by"), "{error}");
    }

    // -- paths and digests -------------------------------------------------

    #[test]
    fn resolves_paths_inside_the_document_tree() {
        let root = scratch();
        let inside = resolve_inside(Path::new("nested/input.png"), &root, "input.path").unwrap();
        assert_eq!(inside, root.join("nested/input.png"));
        let same = resolve_inside(Path::new("."), &root, "case.suite").unwrap();
        assert_eq!(same, root);
    }

    #[test]
    fn rejects_a_path_that_escapes_the_document_tree() {
        let root = scratch();
        let error = resolve_inside(Path::new("../outside.png"), &root, "input.path").unwrap_err();
        assert!(error.to_string().contains("escapes"), "{error}");
        let error = resolve_inside(Path::new("/etc/passwd"), &root, "input.path").unwrap_err();
        assert!(error.to_string().contains("escapes"), "{error}");
        let error =
            resolve_inside(Path::new("a/../../outside.png"), &root, "input.path").unwrap_err();
        assert!(error.to_string().contains("escapes"), "{error}");
    }

    #[test]
    fn resolves_set_file_sources_inside_the_tree() {
        let root = scratch();
        let sources = [
            "grain.toml".to_owned(),
            "grain.engine=procedural".to_owned(),
        ];
        let resolved = resolve_set_sources(&sources, &root).unwrap();
        assert_eq!(resolved[0], root.join("grain.toml"));
        assert_eq!(resolved[1], PathBuf::from("grain.engine=procedural"));
        let error = resolve_set_sources(&["../outside.toml".to_owned()], &root).unwrap_err();
        assert!(error.to_string().contains("chain.set"), "{error}");
    }

    #[test]
    fn classifies_set_sources() {
        assert_eq!(file_source_path("grain.toml"), Some("grain.toml"));
        assert_eq!(
            file_source_path("nested/grain.json"),
            Some("nested/grain.json")
        );
        assert_eq!(file_source_path("grain.engine=procedural"), None);
        assert_eq!(file_source_path("grain.engine=x.toml"), None);
        assert_eq!(file_source_path("grain"), None);
    }

    #[test]
    fn verifies_an_input_digest() {
        let dir = scratch();
        let path = dir.join("input.png");
        std::fs::write(&path, "pixels").unwrap();
        verify_input_digest(&path, &digest_of("pixels")).unwrap();
        let error = verify_input_digest(&path, &digest_of("other")).unwrap_err();
        assert!(error.to_string().contains("hashes to"), "{error}");
        let error =
            verify_input_digest(&dir.join("missing.png"), &digest_of("pixels")).unwrap_err();
        assert!(error.to_string().contains("not a regular file"), "{error}");
    }

    // -- export ------------------------------------------------------------

    fn bundle_manifest(precision: &str, stages: Value) -> Value {
        json!({
            "schema_version": 1,
            "execution_status": "completed",
            "host_precision": precision,
            "stages": stages,
        })
    }

    fn available_stage(id: &str, sha256: &str, width: u64, height: u64) -> Value {
        json!({
            "availability": "available",
            "id": id,
            "path": format!("{id}.f64"),
            "sha256": sha256,
            "layout": {
                "width": width,
                "height": height,
                "channels": 3,
                "dtype": "f64",
                "endianness": "little",
                "channel_order": if id == "rgb_in" || id == "rgb_out" { "RGB" } else { "CMY" },
            },
        })
    }

    fn buffer_bytes(samples: &[f64]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(samples.len() * 8);
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn accepts_a_completed_f64_bundle() {
        let manifest = bundle_manifest(
            "f64",
            json!([available_stage("rgb_in", &"0".repeat(64), 2, 2)]),
        );
        validate_bundle(&manifest).unwrap();
    }

    #[test]
    fn rejects_an_f32_bundle() {
        let manifest = bundle_manifest(
            "f32",
            json!([available_stage("rgb_in", &"0".repeat(64), 2, 2)]),
        );
        let error = validate_bundle(&manifest).unwrap_err().to_string();
        assert!(error.contains("f64"), "{error}");
        assert!(error.contains("f32"), "{error}");
    }

    #[test]
    fn rejects_a_bundle_with_an_unavailable_stage() {
        let manifest = bundle_manifest(
            "f64",
            json!([{
                "availability": "unavailable",
                "id": "cmy_film",
                "reason": "buffer_write_failed",
            }]),
        );
        let error = validate_bundle(&manifest).unwrap_err().to_string();
        assert!(error.contains("incomplete"), "{error}");
        assert!(error.contains("cmy_film"), "{error}");
    }

    #[test]
    fn rejects_an_uncompleted_bundle() {
        let mut manifest = bundle_manifest(
            "f64",
            json!([available_stage("rgb_in", &"0".repeat(64), 2, 2)]),
        );
        manifest["execution_status"] = json!("failed");
        manifest["errors"] = json!(["rgb_out: not captured"]);
        let error = validate_bundle(&manifest).unwrap_err().to_string();
        assert!(error.contains("not captured"), "{error}");
    }

    #[test]
    fn rejects_an_unknown_requested_stage() {
        let manifest = bundle_manifest(
            "f64",
            json!([
                available_stage("rgb_in", &"0".repeat(64), 2, 2),
                available_stage("rgb_out", &"0".repeat(64), 2, 2),
            ]),
        );
        let available: Vec<&Value> = manifest["stages"].as_array().unwrap().iter().collect();
        let error = select_stages(&available, &["cmy_film".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("rgb_in"), "{error}");
        assert!(error.to_string().contains("rgb_out"), "{error}");
    }

    #[test]
    fn selects_requested_stages_in_manifest_order() {
        let manifest = bundle_manifest(
            "f64",
            json!([
                available_stage("rgb_in", &"0".repeat(64), 2, 2),
                available_stage("rgb_out", &"0".repeat(64), 2, 2),
            ]),
        );
        let available: Vec<&Value> = manifest["stages"].as_array().unwrap().iter().collect();
        let selected = select_stages(&available, &["rgb_out".to_owned()]).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0]["id"], "rgb_out");
    }

    #[test]
    fn converts_a_buffer_byte_for_byte() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        let out = dir.join("arrays");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let samples: Vec<f64> = (0..12).map(|index| index as f64 * 0.5).collect();
        let bytes = buffer_bytes(&samples);
        std::fs::write(bundle.join("rgb_in.f64"), &bytes).unwrap();
        let stage = available_stage("rgb_in", &format!("{:x}", Sha256::digest(&bytes)), 2, 2);
        let converted = convert_stage(&bundle, &out, false, &stage).unwrap();
        assert_eq!(converted.id, "rgb_in");
        assert_eq!(converted.shape, vec![2, 2, 3]);
        assert_eq!(converted.path, out.join("rgb_in.npy"));
        // The payload is the buffer's bytes exactly, and the recorded digest
        // is the written array's.
        let written = std::fs::read(&converted.path).unwrap();
        assert!(written.ends_with(&bytes));
        assert_eq!(converted.sha256, format!("{:x}", Sha256::digest(&written)));
    }

    #[test]
    fn refuses_to_replace_an_existing_array_without_force() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        let out = dir.join("arrays");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let bytes = buffer_bytes(&[0.0; 12]);
        std::fs::write(bundle.join("rgb_in.f64"), &bytes).unwrap();
        let stage = available_stage("rgb_in", &format!("{:x}", Sha256::digest(&bytes)), 2, 2);
        std::fs::write(out.join("rgb_in.npy"), b"SENTINEL").unwrap();
        let error = convert_stage(&bundle, &out, false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("refusing to replace"), "{error}");
        assert_eq!(std::fs::read(out.join("rgb_in.npy")).unwrap(), b"SENTINEL");
        convert_stage(&bundle, &out, true, &stage).unwrap();
        assert_ne!(std::fs::read(out.join("rgb_in.npy")).unwrap(), b"SENTINEL");
    }

    #[test]
    fn rejects_a_buffer_whose_digest_does_not_match() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("rgb_in.f64"), [0u8; 96]).unwrap();
        let stage = available_stage("rgb_in", &"1".repeat(64), 2, 2);
        let error = convert_stage(&bundle, &dir.join("arrays"), false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("hashes to"), "{error}");
    }

    #[test]
    fn rejects_a_buffer_whose_length_does_not_match_the_layout() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let bytes = [0u8; 48];
        std::fs::write(bundle.join("rgb_in.f64"), bytes).unwrap();
        let stage = available_stage("rgb_in", &format!("{:x}", Sha256::digest(bytes)), 2, 2);
        let error = convert_stage(&bundle, &dir.join("arrays"), false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("layout declares"), "{error}");
    }

    #[test]
    fn rejects_a_missing_buffer() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let stage = available_stage("rgb_in", &"0".repeat(64), 2, 2);
        let error = convert_stage(&bundle, &dir.join("arrays"), false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("reading bundle buffer"), "{error}");
    }

    #[test]
    fn rejects_a_non_finite_sample() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let mut samples = vec![0.0f64; 12];
        samples[5] = f64::NAN;
        let bytes = buffer_bytes(&samples);
        std::fs::write(bundle.join("rgb_in.f64"), &bytes).unwrap();
        let stage = available_stage("rgb_in", &format!("{:x}", Sha256::digest(&bytes)), 2, 2);
        let error = convert_stage(&bundle, &dir.join("arrays"), false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("non-finite"), "{error}");
    }

    #[test]
    fn rejects_a_stage_record_that_names_another_stage_buffer() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let bytes = [0u8; 96];
        std::fs::write(bundle.join("rgb_out.f64"), bytes).unwrap();
        let mut stage = available_stage("rgb_in", &format!("{:x}", Sha256::digest(bytes)), 2, 2);
        stage["path"] = json!("rgb_out.f64");
        let error = convert_stage(&bundle, &dir.join("arrays"), false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("<id>.f64"), "{error}");
    }

    #[test]
    fn refuses_to_replace_an_existing_report() {
        let dir = scratch();
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let output_image = out.join("output.tiff");
        assert!(!report_exists(&out, &output_image));
        // The output image is a record this run would replace.
        std::fs::write(&output_image, "{}").unwrap();
        assert!(report_exists(&out, &output_image));
        let _ = std::fs::remove_file(&output_image);
        for name in ["report.json", "resolved.json", "command.json"] {
            let _ = std::fs::remove_file(out.join(name));
            std::fs::write(out.join(name), "{}").unwrap();
            assert!(
                report_exists(&out, &output_image),
                "{name} must be a record"
            );
            let _ = std::fs::remove_file(out.join(name));
        }
    }

    #[test]
    fn accepts_a_route_the_bundle_can_describe() {
        for route in [
            "input > film > print > scan",
            "input > convert-film > print > scan",
        ] {
            ensure_describable_route(route).unwrap();
        }
    }

    #[test]
    fn rejects_a_route_outside_the_executed_tap_range() {
        // Routes that skip the print development boundary publish a bundle
        // that never carries the print stages.
        for route in [
            "input",
            "input > film > scan",
            "input > convert-film > scan",
            "input > convert-film > scan-minus-base",
            "not a route",
        ] {
            let error = ensure_describable_route(route).unwrap_err().to_string();
            assert!(error.contains(route), "{route}: {error}");
        }
    }

    #[test]
    fn builds_a_reproducible_command_vector() {
        let dir = scratch();
        let _document = dir.join("experiment.json");
        let set_source = dir.join("grain.toml");
        let mut args = ProcessArgs {
            input: dir.join("input.png"),
            output: Some(dir.join("output.tiff")),
            stage_evidence: Some(dir.join("bundle")),
            seed: Some(0),
            ..ProcessArgs::default()
        };
        args.film = Some("kodak_portra_400".to_owned());
        args.workflow.route = Some("input > film > scan".to_owned());
        args.sources = vec![set_source.display().to_string()];
        let argv = command_vector(
            OutputFormatArg::Tiff,
            &args,
            std::slice::from_ref(&set_source),
        );
        assert_eq!(argv[0], "process");
        assert!(argv.contains(&"cpu".to_owned()));
        assert!(argv.contains(&"--stage-evidence".to_owned()));
        assert!(!argv.contains(&"--experiment".to_owned()));
        assert!(argv.contains(&"--set".to_owned()));
        assert!(argv.contains(&"--route".to_owned()));
        assert!(argv.contains(&"kodak_portra_400".to_owned()));
        assert!(argv.contains(&dir.join("bundle").display().to_string()));
        assert!(argv.contains(&dir.join("grain.toml").display().to_string()));
        assert!(argv.contains(&"--format".to_owned()));
        assert!(argv.contains(&"tiff".to_owned()));
    }
}
