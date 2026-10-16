use super::{
    err, finite, identifier, load_json_document, obj, req, safe_artifact_path, text, unique,
    validate_schema,
};
use crate::common::{Result, digest, sha256_file};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Unavailable,
    NotApplicable,
}
#[derive(Debug, Deserialize)]
pub struct MeasurementRecord {
    pub case_id: String,
    pub region_id: String,
    pub metric: String,
    pub domain: String,
    pub units: String,
    pub estimator: String,
    pub sample_count: u64,
    pub availability: Availability,
    pub seed_population: Vec<u64>,
    pub artifact_refs: Vec<Value>,
    pub value: Option<f64>,
    pub reason: Option<String>,
}

/// The renderer's named stage boundaries, exactly once each per bundle.
pub const STAGE_IDS: [&str; 6] = [
    "rgb_in",
    "log_e_film",
    "cmy_film",
    "log_e_print",
    "cmy_print",
    "rgb_out",
];

/// Producer-confirmed stage semantics: channel order and the fixed
/// domain/units pair each boundary declares.
const STAGE_SEMANTICS: [(&str, &str, &str, &str); 6] = [
    ("rgb_in", "RGB", "input_rgb", "relative"),
    ("log_e_film", "CMY", "record_exposure", "log10"),
    ("cmy_film", "CMY", "record_density", "optical_density"),
    ("log_e_print", "CMY", "record_exposure", "log10"),
    ("cmy_print", "CMY", "record_density", "optical_density"),
    ("rgb_out", "RGB", "output_rgb", "relative"),
];

/// Every stage geometry names its coordinate space explicitly from this
/// vocabulary; intermediate stages never silently inherit output crop space.
const COORDINATE_SPACES: [&str; 5] = [
    "source_pixels",
    "working_pixels",
    "film_frame_pixels",
    "grain_aperture_pixels",
    "output_pixels",
];

/// One validated stage buffer of a renderer stage bundle.
#[derive(Debug)]
pub struct StageBuffer {
    pub stage_id: String,
    pub path: PathBuf,
    /// Bundle-relative path exactly as the manifest declared it.
    pub relative: String,
    /// Retained artifact metadata: layout, domain, units, geometry, color.
    pub metadata: Value,
    pub sha256: String,
}

/// A validated renderer stage bundle: the manifest document as retained
/// context, the independently checked buffers, and the honest completeness
/// state of the capture.
#[derive(Debug)]
pub struct StageBundle {
    pub context: Value,
    pub buffers: Vec<StageBuffer>,
    pub availability: &'static str,
    pub reason: Option<String>,
}

fn availability_of<'a>(section: &'a Value, name: &str) -> Result<&'a str> {
    section["availability"]
        .as_str()
        .filter(|value| matches!(*value, "available" | "unavailable" | "not_applicable"))
        .ok_or_else(|| {
            err(
                "stage_evidence",
                format!("{name} requires a valid availability"),
            )
        })
}

/// A non-available evidence record carries its stable reason and no invented
/// measurement fields.
fn require_reason_only(section: &Value, name: &str, forbidden: &[&str]) -> Result<()> {
    if section["reason"]
        .as_str()
        .is_none_or(|reason| reason.is_empty())
    {
        return Err(err(
            "stage_evidence",
            format!("{name} requires a stable reason when it is not available"),
        ));
    }
    if forbidden.iter().any(|key| section.get(*key).is_some()) {
        return Err(err(
            "stage_evidence",
            format!("{name} must not invent values when it is not available"),
        ));
    }
    Ok(())
}

fn finite_number(section: &Value, name: &str, key: &str) -> Result<f64> {
    section[key]
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| {
            err(
                "stage_evidence",
                format!("{name}.{key} must be one finite number"),
            )
        })
}

fn check_finite_samples(path: &Path, length: u64) -> Result<()> {
    use std::io::Read;
    let mut input = std::io::BufReader::new(fs::File::open(path)?);
    let mut chunk = vec![0u8; 1 << 16];
    let mut remaining = length;
    while remaining > 0 {
        let take = chunk.len().min(remaining as usize);
        input
            .read_exact(&mut chunk[..take])
            .map_err(|e| err("stage_evidence", e.to_string()))?;
        for sample in chunk[..take].as_chunks::<8>().0 {
            let value = f64::from_le_bytes(*sample);
            if !value.is_finite() {
                return Err(err(
                    "stage_evidence",
                    format!(
                        "stage buffer contains a non-finite sample: {}",
                        path.display()
                    ),
                ));
            }
        }
        remaining -= take as u64;
    }
    Ok(())
}

/// The final rgb_out boundary must correspond to the published pipeline
/// output (the raw-out the run retained) whenever it was captured.
pub fn check_output_boundary(bundle: &StageBundle, dimensions: [u64; 2]) -> Result<()> {
    if let Some(buffer) = bundle.buffers.iter().find(|b| b.stage_id == "rgb_out") {
        let layout = &buffer.metadata["layout"];
        if layout["width"] != json!(dimensions[0]) || layout["height"] != json!(dimensions[1]) {
            return Err(err(
                "stage_evidence",
                "rgb_out boundary must correspond to the published pipeline output dimensions",
            ));
        }
    }
    Ok(())
}

/// Validate one renderer stage bundle directory as an external CLI consumer:
/// `stage-evidence.json` written last, with bundle-relative buffer paths. Every
/// available buffer must be a regular file inside the bundle with matching
/// hash, the byte length its declared layout implies, and only finite f64
/// samples. Route-skipped stages stay `not_applicable`; a required executed
/// stage whose capture failed makes the whole capture incomplete evidence.
pub fn validate_stage_bundle(bundle_dir: &Path) -> Result<StageBundle> {
    let manifest_path = bundle_dir.join("stage-evidence.json");
    if !manifest_path
        .symlink_metadata()
        .map(|m| m.file_type().is_file())
        .unwrap_or(false)
    {
        return Err(err(
            "stage_evidence",
            "stage-evidence.json must be a regular file",
        ));
    }
    let document = load_json_document(&manifest_path)?;
    let root = obj(&document)?;
    if root.get("schema_version") != Some(&json!(1)) {
        return Err(err("stage_evidence", "bundle schema_version must be 1"));
    }
    if !matches!(
        root.get("execution_status").and_then(Value::as_str),
        Some("completed" | "failed")
    ) {
        return Err(err(
            "stage_evidence",
            "bundle execution_status must be completed or failed",
        ));
    }
    if !matches!(
        root.get("host_precision").and_then(Value::as_str),
        Some("f32" | "f64")
    ) {
        return Err(err(
            "stage_evidence",
            "bundle host_precision must be f32 or f64",
        ));
    }
    for key in ["workflow_route", "scan_output"] {
        if req(root, key)?
            .as_str()
            .is_none_or(|value| value.is_empty())
        {
            return Err(err(
                "stage_evidence",
                format!("bundle {key} must name the executed route"),
            ));
        }
    }
    if root.get("normalization_mode") != Some(&json!("profile_prepared")) {
        return Err(err(
            "stage_evidence",
            "bundle normalization_mode must be profile_prepared",
        ));
    }
    let errors = root["errors"]
        .as_array()
        .filter(|rows| {
            rows.iter()
                .all(|row| row.as_str().is_some_and(|s| !s.is_empty()))
        })
        .ok_or_else(|| {
            err(
                "stage_evidence",
                "bundle errors must be an array of strings",
            )
        })?;

    // Exposure: observed values come from the actual exposure application.
    let exposure = obj(req(root, "exposure")?)?;
    let exposure_availability = availability_of(&document["exposure"], "exposure")?;
    let mut incomplete: Vec<&str> = Vec::new();
    let mut metered_dimensions: Option<(u64, u64)> = None;
    let mut rgb_in_dimensions: Option<(u64, u64)> = None;
    if exposure_availability == "available" {
        let auto_enabled = exposure
            .get("auto_enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| err("stage_evidence", "exposure.auto_enabled must be boolean"))?;
        if exposure["method"]
            .as_str()
            .is_none_or(|value| value.is_empty())
        {
            return Err(err("stage_evidence", "exposure.method must be named"));
        }
        let dimensions = exposure["source_dimensions"]
            .as_array()
            .filter(|values| {
                values.len() == 2 && values.iter().all(|v| v.as_u64().is_some_and(|d| d > 0))
            })
            .ok_or_else(|| {
                err(
                    "stage_evidence",
                    "exposure.source_dimensions must be two positive dimensions",
                )
            })?;
        metered_dimensions = Some((
            dimensions[0].as_u64().unwrap(),
            dimensions[1].as_u64().unwrap(),
        ));
        let metered = finite_number(&document["exposure"], "exposure", "metered_ev")?;
        for key in ["compensation_ev", "applied_ev", "gain"] {
            finite_number(&document["exposure"], "exposure", key)?;
        }
        // A disabled automatic exposure identifies its zero metering
        // contribution explicitly instead of claiming measured exposure.
        match exposure["metered_ev_source"].as_str() {
            Some("disabled_zero") if !auto_enabled && metered == 0.0 => {}
            Some("measured") if auto_enabled => {}
            _ => {
                return Err(err(
                    "stage_evidence",
                    "exposure metered_ev_source must identify measured metering or the explicit disabled zero",
                ));
            }
        }
    } else {
        require_reason_only(
            &document["exposure"],
            "exposure",
            &[
                "auto_enabled",
                "method",
                "source_dimensions",
                "metered_ev",
                "metered_ev_source",
                "compensation_ev",
                "applied_ev",
                "gain",
            ],
        )?;
        if !matches!(exposure_availability, "not_applicable") {
            incomplete.push("exposure");
        }
    }

    // Print normalization: the actual prepared print exposure factor.
    let print = obj(req(root, "print_normalization")?)?;
    let print_availability =
        availability_of(&document["print_normalization"], "print_normalization")?;
    if print_availability == "available" {
        finite_number(
            &document["print_normalization"],
            "print_normalization",
            "value",
        )?;
        if print.get("mode") != Some(&json!("profile_prepared")) {
            return Err(err(
                "stage_evidence",
                "print_normalization.value must declare profile_prepared mode",
            ));
        }
    } else {
        require_reason_only(
            &document["print_normalization"],
            "print_normalization",
            &["value", "mode"],
        )?;
        if !matches!(print_availability, "not_applicable") {
            incomplete.push("print_normalization");
        }
    }

    // Stages: the six named boundaries in fixed order, each exactly once.
    let stages = root["stages"].as_array().ok_or_else(|| {
        err(
            "stage_evidence",
            "bundle stages must be an array of records",
        )
    })?;
    if stages.len() != STAGE_IDS.len()
        || stages
            .iter()
            .enumerate()
            .any(|(index, record)| record["id"] != json!(STAGE_IDS[index]))
    {
        return Err(err(
            "stage_evidence",
            "bundle stages must name every stage boundary exactly once in fixed order",
        ));
    }
    let mut buffers = Vec::new();
    for stage in stages {
        let record = obj(stage)?;
        let id = text(record, "id")?;
        let (_, channel_order, domain, units) = STAGE_SEMANTICS
            .iter()
            .find(|(name, ..)| *name == id)
            .expect("stage id checked above");
        let availability = availability_of(stage, id)?;
        if availability != "available" {
            require_reason_only(
                stage,
                id,
                &[
                    "path", "sha256", "layout", "domain", "units", "geometry", "color",
                ],
            )?;
            // Reasons are stable codes, not prose.
            let reason = record["reason"].as_str().unwrap();
            if reason.is_empty()
                || !reason
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                return Err(err(
                    "stage_evidence",
                    format!("{id} requires a stable snake_case reason code"),
                ));
            }
            if !matches!(availability, "not_applicable") {
                incomplete.push(id);
            }
            continue;
        }
        let relative = text(record, "path")?.to_owned();
        let layout = obj(req(record, "layout")?)?;
        let width = layout["width"]
            .as_u64()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                err(
                    "stage_evidence",
                    format!("{id} layout width must be positive"),
                )
            })?;
        let height = layout["height"]
            .as_u64()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                err(
                    "stage_evidence",
                    format!("{id} layout height must be positive"),
                )
            })?;
        if layout["channels"] != json!(3)
            || layout["dtype"] != json!("f64")
            || layout["endianness"] != json!("little")
            || layout["channel_order"] != json!(*channel_order)
        {
            return Err(err(
                "stage_evidence",
                format!("{id} layout must be 3-channel little-endian f64 in {channel_order} order"),
            ));
        }
        if *channel_order == "RGB" {
            let color = obj(req(record, "color")?)?;
            if color["color_space"]
                .as_str()
                .is_none_or(|space| space.is_empty())
                || color["cctf_decoding"].as_bool().is_none()
            {
                return Err(err(
                    "stage_evidence",
                    format!("{id} must carry its color interpretation"),
                ));
            }
        } else if record.get("color").is_some() {
            return Err(err(
                "stage_evidence",
                format!("{id} carries density values, not a color interpretation"),
            ));
        }
        if record["domain"] != json!(*domain) || record["units"] != json!(*units) {
            return Err(err(
                "stage_evidence",
                format!("{id} must declare domain {domain} and units {units}"),
            ));
        }
        let geometry = obj(req(record, "geometry")?)?;
        if geometry["coordinate_space"]
            .as_str()
            .is_none_or(|space| !COORDINATE_SPACES.contains(&space))
        {
            return Err(err(
                "stage_evidence",
                format!("{id} geometry must name one explicit coordinate space"),
            ));
        }
        let declared = text(record, "sha256")?;
        if declared.len() != 64 || !declared.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err(
                "stage_evidence",
                format!("{id} buffer hash is invalid"),
            ));
        }
        let path = safe_artifact_path(bundle_dir, &relative)?;
        if !path
            .symlink_metadata()
            .map(|m| m.file_type().is_file())
            .unwrap_or(false)
        {
            return Err(err(
                "stage_evidence",
                format!("{id} buffer must be a regular file"),
            ));
        }
        let length = width
            .checked_mul(height)
            .and_then(|value| value.checked_mul(24))
            .ok_or_else(|| err("stage_evidence", format!("{id} buffer length overflow")))?;
        if fs::metadata(&path)?.len() != length {
            return Err(err(
                "stage_evidence",
                format!("{id} buffer byte count disagrees with its declared layout"),
            ));
        }
        let actual = sha256_file(&path)?;
        if actual != declared {
            return Err(err("stage_evidence", format!("{id} buffer hash mismatch")));
        }
        check_finite_samples(&path, length)?;
        if id == "rgb_in" {
            // Metering runs on the pipeline input before crop and resize, so
            // the metered dimensions are exactly the rgb_in dimensions.
            rgb_in_dimensions = Some((width, height));
        }
        let mut metadata = json!({"layout":layout,"domain":record["domain"],"units":record["units"],"geometry":record["geometry"]});
        if let Some(color) = record.get("color") {
            metadata["color"] = color.clone();
        }
        buffers.push(StageBuffer {
            stage_id: id.to_owned(),
            path,
            relative,
            metadata,
            sha256: actual,
        });
    }
    // Producer identity: exposure metering observes exactly the rgb_in
    // boundary, so its declared source dimensions must agree.
    if let (Some(metered), Some(rgb_in)) = (metered_dimensions, rgb_in_dimensions)
        && metered != rgb_in
    {
        return Err(err(
            "stage_evidence",
            "exposure metering dimensions must be the rgb_in boundary dimensions",
        ));
    }
    let complete = incomplete.is_empty()
        && errors.is_empty()
        && document["execution_status"] == json!("completed");
    let reason = if complete {
        None
    } else {
        let mut names = incomplete.clone();
        if !errors.is_empty() {
            names.push("errors");
        }
        if document["execution_status"] != json!("completed") {
            names.push("execution_status");
        }
        Some(format!("incomplete stage capture: {}", names.join(", ")))
    };
    Ok(StageBundle {
        context: Value::Object(
            [
                "host_precision",
                "workflow_route",
                "scan_output",
                "normalization_mode",
                "exposure",
                "print_normalization",
                "stages",
                "errors",
            ]
            .into_iter()
            .map(|key| (key.to_owned(), root[key].clone()))
            .collect(),
        ),
        buffers,
        availability: if complete { "available" } else { "unavailable" },
        reason,
    })
}
/// An artifact reference must resolve in the manifest's index and agree with
/// its recorded hash.
fn indexed_reference(index: &std::collections::BTreeMap<&str, &Value>, item: &Value) -> Result<()> {
    let p = item
        .as_str()
        .or_else(|| item.get("path").and_then(Value::as_str))
        .ok_or_else(|| {
            err(
                "unindexed_artifact",
                "artifact reference must resolve in index",
            )
        })?;
    let a = index.get(p).ok_or_else(|| {
        err(
            "unindexed_artifact",
            format!("artifact reference absent: {p}"),
        )
    })?;
    if item.is_object() && item.get("sha256") != a.get("sha256") {
        return Err(err(
            "artifact_identity",
            "artifact reference hash differs from index",
        ));
    }
    Ok(())
}

/// Validate the retained copy of one stage bundle against its indexed
/// references: the manifest and every relative buffer dependency must be
/// indexed, the bundle must be complete producer evidence, and the retained
/// context must be exactly what the bundle manifest says. Used by run rows and
/// comparison run-evidence payloads so a recomputed document digest cannot
/// smuggle malformed context or bundle dependencies past validation.
fn validate_retained_bundle(
    root: &Path,
    index: &std::collections::BTreeMap<&str, &Value>,
    references: &Value,
    context: Option<&Value>,
) -> Result<StageBundle> {
    let manifest_record = req(obj(references)?, "manifest")?;
    let manifest = obj(manifest_record)?;
    indexed_reference(index, manifest_record)?;
    let manifest_path = safe_artifact_path(root, text(manifest, "path")?)?;
    let bundle_dir = manifest_path
        .parent()
        .ok_or_else(|| err("stage_evidence", "bundle manifest must name its directory"))?;
    let bundle = validate_stage_bundle(bundle_dir)?;
    if bundle.availability != "available" {
        return Err(err(
            "stage_evidence",
            bundle
                .reason
                .unwrap_or_else(|| "incomplete stage capture".into()),
        ));
    }
    if context != Some(&bundle.context) {
        return Err(err(
            "stage_evidence",
            "retained stage context differs from the bundle manifest",
        ));
    }
    let buffers = references["buffers"]
        .as_array()
        .ok_or_else(|| err("stage_evidence", "stage_evidence.buffers must be an array"))?;
    if buffers.len() != bundle.buffers.len() {
        return Err(err(
            "stage_evidence",
            "retained stage buffers must cover the bundle exactly",
        ));
    }
    let mut stage_ids = BTreeSet::new();
    for buffer in buffers {
        indexed_reference(index, buffer)?;
        let buffer = obj(buffer)?;
        let stage_id = text(buffer, "stage_id")?;
        if !stage_ids.insert(stage_id) {
            return Err(err(
                "stage_evidence",
                format!("retained stage buffers repeat stage: {stage_id}"),
            ));
        }
        let expected = bundle
            .buffers
            .iter()
            .find(|candidate| candidate.stage_id == stage_id)
            .ok_or_else(|| {
                err(
                    "stage_evidence",
                    format!("retained buffer names an unknown stage: {stage_id}"),
                )
            })?;
        let relative = Path::new(text(manifest, "path")?)
            .parent()
            .unwrap_or(Path::new(""))
            .join(&expected.relative)
            .to_string_lossy()
            .replace('\\', "/");
        if buffer["path"] != json!(relative)
            || buffer["sha256"] != json!(expected.sha256)
            || buffer.get("layout") != Some(&expected.metadata["layout"])
        {
            return Err(err(
                "stage_evidence",
                format!("retained buffer record differs from the bundle for {stage_id}"),
            ));
        }
    }
    Ok(bundle)
}

pub fn validate_generated_document(kind: &str, v: &Value, root: &Path) -> Result<()> {
    finite(v)?;
    validate_schema(kind, v)?;
    let o = obj(v)?;
    let body = Value::Object(
        o.iter()
            .filter(|(k, _)| k.as_str() != "id")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    if text(o, "id")? != digest(&body)? {
        return Err(err(
            "manifest_identity",
            "manifest identity does not match contents",
        ));
    }
    let mut index = std::collections::BTreeMap::new();
    for a in v["artifacts"].as_array().unwrap() {
        let ao = obj(a)?;
        let p = text(ao, "path")?;
        if index.insert(p, a).is_some() {
            return Err(err(
                "duplicate_artifact",
                "artifact index paths must be unique",
            ));
        }
        let file = safe_artifact_path(root, p)?;
        if !file.is_file() {
            return Err(err("missing_artifact", "artifact must be a regular file"));
        }
        if sha256_file(&file)? != text(ao, "sha256")? {
            return Err(err("hash_mismatch", "artifact hash mismatch"));
        }
    }
    let reference = |item: &Value| indexed_reference(&index, item);
    let measurements = |rows: &Value| -> Result<()> {
        for row in rows.as_array().into_iter().flatten() {
            let r = obj(row)?;
            if [
                "case_id",
                "region_id",
                "metric",
                "domain",
                "units",
                "estimator",
                "sample_count",
                "availability",
                "seed_population",
                "artifact_refs",
            ]
            .iter()
            .any(|k| !r.contains_key(*k))
                || row["sample_count"].as_u64().is_none()
            {
                return Err(err(
                    "measurement_fields",
                    "measurement requires identity, population and estimator",
                ));
            }
            if !matches!(
                row["availability"].as_str(),
                Some("available" | "unavailable" | "not_applicable")
            ) {
                return Err(err(
                    "measurement_availability",
                    "invalid measurement availability",
                ));
            }
            let parsed: MeasurementRecord = serde_json::from_value(row.clone())
                .map_err(|e| err("measurement_fields", e.to_string()))?;
            match parsed.availability {
                Availability::Available => {
                    if parsed.value.is_none_or(|x| !x.is_finite()) || row.get("reason").is_some() {
                        return Err(err(
                            "measurement_value",
                            "available measurement requires one finite value",
                        ));
                    }
                }
                Availability::Unavailable | Availability::NotApplicable => {
                    if row.get("value").is_some()
                        || parsed.reason.as_deref().is_none_or(str::is_empty)
                    {
                        return Err(err(
                            "measurement_availability",
                            "unavailable measurement requires reason and no value",
                        ));
                    }
                }
            }
            for a in &parsed.artifact_refs {
                reference(a)?
            }
        }
        Ok(())
    };
    let readouts = |owner: &Value, case: &Value| -> Result<()> {
        for readout in owner["region_readouts"].as_array().into_iter().flatten() {
            for artifact in readout["artifact_refs"].as_array().into_iter().flatten() {
                reference(artifact)?;
            }
            if let Some(mask) = readout.get("common_area_artifact_id") {
                reference(mask)?;
            }
            if owner.get("left_role").is_some() && readout["pair_id"] != owner["id"] {
                return Err(err(
                    "readout_identity",
                    "Color readout differs from its comparison pair",
                ));
            }
            let expected_domain = if readout["estimator"] == "bounded_lab_channel_median" {
                "bounded_srgb_d65_lab"
            } else {
                "unclipped_linear_srgb_mean_lab"
            };
            if readout["domain"] != expected_domain {
                return Err(err(
                    "readout_domain",
                    "Color readout domain differs from its estimator",
                ));
            }
            if readout["availability"] != "available" {
                continue;
            }
            let sides = readout["sides"].as_object().unwrap();
            let reference_review = &owner["reference"];
            let is_reference = readout["pair_id"] == "reference";
            if is_reference
                && (reference_review["scope"] == "visual"
                    || reference_review["color_available"] == false
                    || (reference_review["scope"] == "aligned_scene"
                        && reference_review["geometry"] != "pixel_aligned"))
            {
                return Err(err(
                    "readout_applicability",
                    "Color readout conflicts with retained reference qualification",
                ));
            }
            for displacement in readout["displacements"].as_array().unwrap() {
                let from = displacement["from_role"].as_str().unwrap();
                let to = displacement["to_role"].as_str().unwrap();
                if from == to
                    || !sides.contains_key(from)
                    || !sides.contains_key(to)
                    || (is_reference && from != "reference")
                    || (readout["pair_id"] == "input" && (from != "input" || to != "simulation"))
                    || (readout["pair_id"] == "baseline/candidate"
                        && (from != "baseline" || to != "candidate"))
                {
                    return Err(err(
                        "readout_identity",
                        "Color displacement must name the retained comparison direction",
                    ));
                }
                for (eligible, keys) in [
                    (
                        "tone_target_eligible",
                        &["delta_lab", "delta_l_star", "delta_e76", "delta_e00"][..],
                    ),
                    (
                        "color_diagnostic_eligible",
                        &["delta_c_star", "delta_h_degrees", "equal_l_delta_e00"][..],
                    ),
                ] {
                    if is_reference
                        && reference_review[eligible] != true
                        && keys
                            .iter()
                            .any(|key| displacement[*key]["availability"] == "available")
                    {
                        return Err(err(
                            "readout_applicability",
                            "Color displacement exceeds retained reference eligibility",
                        ));
                    }
                }
            }
        }
        for readout in owner["stage_readouts"].as_array().into_iter().flatten() {
            for artifact in readout["artifact_refs"].as_array().into_iter().flatten() {
                reference(artifact)?;
            }
            let role = readout["run_role"].as_str().unwrap();
            let payload = &case["run_evidence"][role];
            if !payload.is_object() {
                return Err(err(
                    "readout_identity",
                    "Stage readout has no retained run role",
                ));
            }
            if readout["availability"] != "available" {
                continue;
            }
            let stage = payload["observed"]["stages"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|stage| stage["id"] == readout["stage_id"]);
            if stage.is_none_or(|stage| {
                stage["availability"] != "available"
                    || stage["domain"] != readout["domain"]
                    || stage["units"] != readout["units"]
                    || stage["layout"]["channel_order"] != readout["channel_order"]
                    || stage["geometry"] != readout["stage_geometry"]
            }) || payload["precision"]["host_precision"] != readout["host_precision"]
            {
                return Err(err(
                    "readout_domain",
                    "Stage readout differs from its producer's observed boundary",
                ));
            }
            let buffer = payload["stage_evidence"]["buffers"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|buffer| buffer["stage_id"] == readout["stage_id"]);
            if buffer.is_none_or(|buffer| {
                !readout["artifact_refs"]
                    .as_array()
                    .unwrap()
                    .contains(&buffer["path"])
            }) {
                return Err(err(
                    "readout_evidence",
                    "Available stage readout requires its retained native buffer",
                ));
            }
            for channel in 0..3 {
                let min = readout["channel_min"][channel].as_f64().unwrap();
                let max = readout["channel_max"][channel].as_f64().unwrap();
                if min > max {
                    return Err(err(
                        "readout_value",
                        "Stage minimum must not exceed its maximum",
                    ));
                }
            }
        }
        Ok(())
    };
    if kind == "run" {
        reference(&v["suite"])?;
        reference(&v["target"])?;
        let suite = load_json_document(&safe_artifact_path(
            root,
            v["suite"]["path"].as_str().unwrap(),
        )?)?;
        validate_schema("suite", &suite)?;
        if digest(&suite)? != v["suite"]["digest"].as_str().unwrap_or("")
            || suite["protocol"] != v["protocol"]
        {
            return Err(err(
                "suite_identity",
                "suite snapshot and protocol mismatch",
            ));
        }
        let selected = v["selection"]["case_ids"]
            .as_array()
            .ok_or_else(|| err("run_selection", "selection case_ids must be array"))?;
        let all = suite["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].clone())
            .collect::<Vec<_>>();
        let set = selected.iter().map(|x| x.as_str()).collect::<BTreeSet<_>>();
        if selected.is_empty()
            || set.len() != selected.len()
            || selected.iter().any(|x| !all.contains(x))
            || v["selection"]["full_suite"].as_bool() != Some(selected == &all)
        {
            return Err(err(
                "run_selection",
                "selection must identify distinct suite cases and full-suite state",
            ));
        }
        let mut expected = BTreeSet::new();
        for c in suite["cases"].as_array().unwrap() {
            if selected.contains(&c["id"]) {
                for seed in c["seeds"].as_array().unwrap() {
                    expected.insert((
                        c["id"].as_str().unwrap().to_string(),
                        seed.as_u64().unwrap(),
                    ));
                }
            }
        }
        let mut observed = BTreeSet::new();
        for row in v["cases"].as_array().unwrap() {
            let ro = obj(row)?;
            identifier(ro, "case_id")?;
            let seed = row["seed"]
                .as_u64()
                .filter(|x| *x <= u32::MAX as u64)
                .ok_or_else(|| err("run_case", "invalid case seed"))?;
            if !observed.insert((text(ro, "case_id")?.to_string(), seed)) {
                return Err(err("run_selection", "duplicate rendered case/seed"));
            }
            match row["status"].as_str() {
                Some("completed") => {
                    for k in [
                        "input",
                        "raw",
                        "raw_metadata",
                        "master",
                        "diagnostics",
                        "resolved",
                        "stdout",
                        "stderr",
                    ] {
                        let record = obj(req(ro, k)?)?;
                        text(record, "path")?;
                        reference(req(ro, k)?)?
                    }
                    // Completed rows always retain the original existing source
                    // evidence; a manifest that lost it must not revalidate.
                    for key in [
                        "source_raw",
                        "source_raw_metadata",
                        "source_resolved",
                        "source_diagnostics",
                        "source_master",
                    ] {
                        let record = row.get(key).ok_or_else(|| {
                            err(
                                "source_evidence",
                                format!("completed run row requires retained {key}"),
                            )
                        })?;
                        text(obj(record)?, "path")?;
                        reference(record)?;
                    }
                    for key in ["command", "source_command"] {
                        let argv = row[key].as_array().filter(|argv| {
                            !argv.is_empty()
                                && argv
                                    .iter()
                                    .all(|arg| arg.as_str().is_some_and(|arg| !arg.is_empty()))
                        });
                        if argv.is_none() {
                            return Err(err(
                                "run_command",
                                "retained command must record the executed argument vector",
                            ));
                        }
                    }
                    {
                        let source = &row["source_raw"];
                        let layout = &source["layout"];
                        let width = layout["width"]
                            .as_u64()
                            .filter(|v| *v > 0)
                            .ok_or_else(|| err("raw_layout", "Invalid source width"))?;
                        let height = layout["height"]
                            .as_u64()
                            .filter(|v| *v > 0)
                            .ok_or_else(|| err("raw_layout", "Invalid source height"))?;
                        let bytes = width
                            .checked_mul(height)
                            .and_then(|v| v.checked_mul(24))
                            .ok_or_else(|| err("raw_layout", "Source dimensions overflow"))?;
                        if source["color"]
                            != json!({"color_space":"ACES2065-1","cctf_decoding":false})
                            || layout["channels"] != 3
                            || layout["dtype"] != "f64"
                            || layout["channel_order"] != "RGB"
                            || fs::metadata(safe_artifact_path(
                                root,
                                source["path"].as_str().unwrap(),
                            )?)?
                            .len()
                                != bytes
                        {
                            return Err(err(
                                "raw_layout",
                                "Retained source layout, color, or byte count is invalid",
                            ));
                        }
                        let metadata = load_json_document(&safe_artifact_path(
                            root,
                            row["source_raw_metadata"]["path"].as_str().unwrap(),
                        )?)?;
                        let mut expected = layout.clone();
                        expected
                            .as_object_mut()
                            .unwrap()
                            .extend(source["color"].as_object().unwrap().clone());
                        let diagnostics = load_json_document(&safe_artifact_path(
                            root,
                            row["source_diagnostics"]["path"].as_str().unwrap(),
                        )?)?;
                        if metadata != expected
                            || diagnostics["configuration"]["input_dimensions"]
                                != row["source_dimensions"]
                            || diagnostics["configuration"]["published_dimensions"]
                                != json!([width, height])
                            || row["source_geometry"]["published_dimensions"]
                                != json!([width, height])
                            || row["simulation_geometry"]["published_dimensions"]
                                != row["dimensions"]
                        {
                            return Err(err(
                                "source_geometry",
                                "Source evidence metadata and observed geometry disagree",
                            ));
                        }
                        let case = suite["cases"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|c| c["id"] == row["case_id"])
                            .ok_or_else(|| err("run_selection", "Unknown retained case"))?;
                        if case.get("preparation") != row.get("preparation") {
                            return Err(err(
                                "preparation_identity",
                                "Run preparation differs from suite provenance",
                            ));
                        }
                    }
                    let layout = &row["raw"]["layout"];
                    let w = layout["width"]
                        .as_u64()
                        .ok_or_else(|| err("raw_layout", "invalid width"))?;
                    let h = layout["height"]
                        .as_u64()
                        .ok_or_else(|| err("raw_layout", "invalid height"))?;
                    let bytes = w
                        .checked_mul(h)
                        .and_then(|x| x.checked_mul(24))
                        .ok_or_else(|| err("raw_layout", "raw byte count overflow"))?;
                    if fs::metadata(safe_artifact_path(
                        root,
                        row["raw"]["path"].as_str().unwrap(),
                    )?)?
                    .len()
                        != bytes
                    {
                        return Err(err("raw_layout", "raw byte count differs from dimensions"));
                    }
                    let d = load_json_document(&safe_artifact_path(
                        root,
                        row["diagnostics"]["path"].as_str().unwrap(),
                    )?)?;
                    if d["configuration"]["published_dimensions"] != row["dimensions"] {
                        return Err(err(
                            "render_geometry",
                            "diagnostics and retained dimensions disagree",
                        ));
                    }
                    if let Some(stage) = row.get("stage_evidence") {
                        let record = obj(stage)?;
                        let requested = record
                            .get("requested")
                            .and_then(Value::as_bool)
                            .ok_or_else(|| {
                                err("stage_evidence", "stage_evidence.requested must be boolean")
                            })?;
                        if !requested {
                            if record.get("manifest").is_some() || record.get("buffers").is_some() {
                                return Err(err(
                                    "stage_evidence",
                                    "uncaptured stage evidence must not claim bundle artifacts",
                                ));
                            }
                            if !matches!(
                                record["availability"].as_str(),
                                Some("not_applicable" | "unavailable")
                            ) || record["reason"]
                                .as_str()
                                .is_none_or(|reason| reason.is_empty())
                            {
                                return Err(err(
                                    "stage_evidence",
                                    "uncaptured stage evidence must name its unavailability explicitly",
                                ));
                            }
                            continue;
                        }
                        if record["availability"] != json!("available") {
                            return Err(err(
                                "stage_evidence",
                                "a completed render cannot retain incomplete stage capture as evidence",
                            ));
                        }
                        let bundle =
                            validate_retained_bundle(root, &index, stage, record.get("context"))?;
                        // Producer semantics: every captured boundary declares
                        // its own geometry and the run observes the matching
                        // dimensions; RGB boundaries state the declared input
                        // and output color interpretation. Stage capture is
                        // rejected under preview bounding, so the decoded
                        // source space always is the retained original input.
                        let resolved = load_json_document(&safe_artifact_path(
                            root,
                            row["resolved"]["path"].as_str().unwrap(),
                        )?)?;
                        let io = &resolved["parameters"]["io"];
                        for buffer in record["buffers"].as_array().into_iter().flatten() {
                            let layout = &buffer["layout"];
                            let dims = json!([layout["width"], layout["height"]]);
                            let space = buffer["geometry"]["coordinate_space"].as_str();
                            let expected = match space {
                                Some("source_pixels") => Some(&row["source_dimensions"]),
                                Some("working_pixels") => {
                                    Some(&row["simulation_geometry"]["working_dimensions"])
                                }
                                Some("output_pixels") => Some(&row["dimensions"]),
                                // Film-frame and supersampled aperture grids
                                // have no matching run-level dimensions.
                                _ => None,
                            };
                            if let Some(expected) = expected
                                && expected.is_array()
                                && *expected != dims
                            {
                                return Err(err(
                                    "stage_evidence",
                                    format!(
                                        "stage {} geometry disagrees with the run's observed {} dimensions",
                                        buffer["stage_id"],
                                        space.unwrap_or("coordinate space")
                                    ),
                                ));
                            }
                            let id = buffer["stage_id"].as_str().unwrap_or_default();
                            let declaration = match id {
                                "rgb_in" => Some((
                                    io["input_color_space"].clone(),
                                    io["input_cctf_decoding"] == json!(true),
                                )),
                                "rgb_out" => Some((
                                    io["output_color_space"].clone(),
                                    io["output_cctf_encoding"] == json!(true),
                                )),
                                _ => None,
                            };
                            if let Some((color_space, encoded)) = declaration {
                                let expected =
                                    json!({"color_space":color_space,"cctf_decoding":!encoded});
                                if buffer["color"] != expected {
                                    return Err(err(
                                        "stage_evidence",
                                        format!(
                                            "stage {id} color interpretation differs from the declared io configuration"
                                        ),
                                    ));
                                }
                            }
                        }
                        if record["context"]["exposure"]["availability"] == json!("available")
                            && record["context"]["exposure"]["source_dimensions"]
                                != row["source_dimensions"]
                        {
                            return Err(err(
                                "stage_evidence",
                                "exposure metering dimensions must be the retained full-source dimensions",
                            ));
                        }
                        let dimensions = row["dimensions"]
                            .as_array()
                            .filter(|values| values.len() == 2)
                            .ok_or_else(|| err("stage_evidence", "missing output dimensions"))?;
                        check_output_boundary(
                            &bundle,
                            [
                                dimensions[0].as_u64().unwrap_or_default(),
                                dimensions[1].as_u64().unwrap_or_default(),
                            ],
                        )?;
                    }
                }
                Some("failed") => {
                    if row
                        .get("error")
                        .is_none_or(|x| !x.is_object() || x.as_object().unwrap().is_empty())
                    {
                        return Err(err(
                            "missing_failure",
                            "failed render requires structured error",
                        ));
                    }
                    if v["execution_status"] == "completed" {
                        return Err(err("run_status", "completed run contains failed cases"));
                    }
                }
                _ => return Err(err("run_status", "invalid run case status")),
            }
        }
        if observed != expected {
            return Err(err(
                "run_selection",
                "rendered case/seed population differs from suite",
            ));
        }
        for x in v["dependencies"].as_array().unwrap() {
            if x["role"] != "renderer" {
                reference(x)?
            }
        }
    } else if kind == "comparison" {
        if let Some(p) = v.get("policy") {
            reference(p)?
        } else if v["evaluation_status"] != "not_evaluated" {
            return Err(err(
                "evaluation_status",
                "comparison without policy must remain not_evaluated",
            ));
        }
        for case in v["cases"].as_array().unwrap() {
            measurements(&case["measurements"])?;
            readouts(case, case)?;
            // Per-role run evidence: identity consistency, indexed references,
            // and the copied stage bundle exactly as the producer wrote it.
            if let Some(run_evidence) = case.get("run_evidence") {
                for (role, payload) in obj(run_evidence)? {
                    let payload = obj(payload)?;
                    if payload.get("role") != Some(&json!(role.as_str()))
                        || payload.get("case_id") != Some(&case["case_id"])
                        || payload.get("seed") != Some(&case["seed"])
                    {
                        return Err(err(
                            "run_evidence_identity",
                            "run evidence identity differs from its case",
                        ));
                    }
                    for key in ["resolved_configuration", "diagnostics"] {
                        text(obj(req(payload, key)?)?, "path")?;
                        reference(req(payload, key)?)?;
                    }
                    let stage = obj(req(payload, "stage_evidence")?)?;
                    let requested =
                        stage
                            .get("requested")
                            .and_then(Value::as_bool)
                            .ok_or_else(|| {
                                err("stage_evidence", "stage_evidence.requested must be boolean")
                            })?;
                    if requested {
                        validate_retained_bundle(
                            root,
                            &index,
                            &payload["stage_evidence"],
                            payload.get("observed"),
                        )?;
                        continue;
                    }
                    if stage.get("manifest").is_some() || stage.get("buffers").is_some() {
                        return Err(err(
                            "stage_evidence",
                            "uncaptured stage evidence must not claim bundle artifacts",
                        ));
                    }
                    let observed = payload
                        .get("observed")
                        .and_then(Value::as_object)
                        .ok_or_else(|| {
                            err(
                                "stage_evidence",
                                "run evidence must name unobserved sections",
                            )
                        })?;
                    for key in ["exposure", "print_normalization"] {
                        let section = obj(observed.get(key).ok_or_else(|| {
                            err(
                                "stage_evidence",
                                "run evidence must name unobserved sections",
                            )
                        })?)?;
                        if section["availability"] != json!("unavailable")
                            || section["reason"]
                                .as_str()
                                .is_none_or(|reason| reason.is_empty())
                        {
                            return Err(err(
                                "stage_evidence",
                                "uncaptured exposure and normalization must be named unobserved",
                            ));
                        }
                    }
                    let stages = observed["stages"].as_array().ok_or_else(|| {
                        err(
                            "stage_evidence",
                            "uncaptured stages must be named explicitly",
                        )
                    })?;
                    if stages.len() != STAGE_IDS.len()
                        || stages.iter().enumerate().any(|(index, record)| {
                            record["id"] != json!(STAGE_IDS[index])
                                || record["availability"] != json!("unavailable")
                                || record["reason"]
                                    .as_str()
                                    .is_none_or(|reason| reason.is_empty())
                        })
                    {
                        return Err(err(
                            "stage_evidence",
                            "uncaptured stages must be named unobserved explicitly",
                        ));
                    }
                }
            }
            if v["mode"] == "single_run" {
                let pairs = case["pairs"].as_array().ok_or_else(|| {
                    err(
                        "comparison_pair",
                        "Single-run case requires comparison pairs",
                    )
                })?;
                unique(pairs, "id")?;
                if !pairs.iter().any(|pair| pair["id"] == v["default_pair"]) {
                    return Err(err("comparison_pair", "Default pair is absent from case"));
                }
                for pair in pairs {
                    if pair["left_role"] != pair["id"] || pair["right_role"] != "simulation" {
                        return Err(err(
                            "comparison_pair",
                            "Pair roles must identify input/reference and simulation",
                        ));
                    }
                    if case["status"] == "completed" && pair["status"] != "completed" {
                        return Err(err(
                            "comparison_status",
                            "Completed case contains an incomplete pair",
                        ));
                    }
                    if pair["id"] == "reference" && pair["status"] == "completed" {
                        let scope = pair["reference"]["scope"].as_str().ok_or_else(|| {
                            err("comparison_pair", "Reference pair requires retained scope")
                        })?;
                        let aligned = scope == "aligned_scene"
                            && pair["reference"]["geometry"] == "pixel_aligned";
                        if pair["pixel_aligned"] != aligned {
                            return Err(err(
                                "comparison_correspondence",
                                "Pair alignment conflicts with retained reference correspondence",
                            ));
                        }
                        if pair["measurements"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|row| {
                                row["availability"] == "available"
                                    && (scope == "visual"
                                        || (scope == "aligned_scene" && !aligned)
                                        || pair["reference"]["color_available"] == false)
                            })
                        {
                            return Err(err(
                                "measurement_applicability",
                                "Available reference score conflicts with retained scope or interpretation",
                            ));
                        }
                    }
                    measurements(&pair["measurements"])?;
                    readouts(pair, case)?;
                    for measurement in pair["measurements"].as_array().into_iter().flatten() {
                        if measurement["case_id"] != case["case_id"]
                            || measurement["pair_id"] != pair["id"]
                        {
                            return Err(err(
                                "comparison_pair",
                                "Measurement identity differs from its pair",
                            ));
                        }
                    }
                    for item in pair["review"]
                        .as_object()
                        .into_iter()
                        .flat_map(|map| map.values())
                    {
                        reference(item)?;
                    }
                    for item in pair["artifacts"].as_array().into_iter().flatten() {
                        reference(item)?;
                    }
                    if let Some(item) = pair.get("measurement_artifact") {
                        reference(item)?;
                    }
                    for key in ["left_master", "right_master", "right_resolved"] {
                        if let Some(item) = pair.get(key) {
                            reference(item)?;
                        }
                    }
                }
            }
            for x in case["review"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
            {
                reference(x)?
            }
            for x in case["artifacts"].as_array().into_iter().flatten() {
                reference(x)?
            }
            if v["execution_status"] == "completed" && case["status"] == "failed" {
                return Err(err(
                    "comparison_status",
                    "completed comparison contains failed measurements",
                ));
            }
        }
        measurements(&v["seed_measurements"])?;
        unique(v["findings"].as_array().unwrap(), "id")?;
        for finding in v["findings"].as_array().unwrap() {
            let f = obj(finding)?;
            for k in [
                "id",
                "case_id",
                "region_id",
                "metric",
                "decision",
                "evidence",
            ] {
                req(f, k)?;
            }
            let e = finding["evidence"]
                .as_array()
                .filter(|x| !x.is_empty())
                .ok_or_else(|| err("finding_evidence", "finding requires inspectable evidence"))?;
            for x in e {
                reference(x)?
            }
        }
        if v["evaluation_status"] == "passed" && v["execution_status"] != "completed" {
            return Err(err("evaluation_status", "failed execution cannot qualify"));
        }
    }
    Ok(())
}
