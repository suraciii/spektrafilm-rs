use crate::common::{Error, Result, digest, sha256_file};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

mod evidence;
mod masks;

pub use evidence::{
    Availability, MeasurementRecord, STAGE_IDS, StageBuffer, StageBundle, check_output_boundary,
    validate_generated_document, validate_stage_bundle,
};
pub use masks::{geometry_mask, reference_geometry_mask};

pub const ID_PATTERN: &str = r"^[A-Za-z0-9][A-Za-z0-9_.-]*$";

/// Resolved parameter fields owned by the case definition. Targets must not
/// override input interpretation, geometry, or preview settings.
pub const CASE_OWNED_FIELDS: &[(&str, &[&str])] = &[
    (
        "io",
        &[
            "input_color_space",
            "input_cctf_decoding",
            "crop",
            "crop_center",
            "crop_size",
            "upscale_factor",
        ],
    ),
    ("settings", &["preview_mode", "preview_max_size"]),
];
fn err(code: &str, message: impl Into<String>) -> Error {
    Error::new(code, message.into())
}
fn obj(v: &Value) -> Result<&Map<String, Value>> {
    v.as_object()
        .ok_or_else(|| err("document_type", "document root must be an object"))
}
fn req<'a>(o: &'a Map<String, Value>, k: &str) -> Result<&'a Value> {
    o.get(k)
        .ok_or_else(|| err("missing_field", format!("missing field: {k}")))
}
fn text<'a>(o: &'a Map<String, Value>, k: &str) -> Result<&'a str> {
    req(o, k)?
        .as_str()
        .ok_or_else(|| err("schema_validation", format!("{k} must be a string")))
}
fn identifier(o: &Map<String, Value>, k: &str) -> Result<()> {
    let x = text(o, k)?;
    if x.is_empty()
        || !x.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        || !x
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err(err("invalid_identifier", format!("invalid {k}")));
    }
    Ok(())
}
fn finite(v: &Value) -> Result<()> {
    match v {
        Value::Number(n) if n.as_f64().is_some_and(|x| !x.is_finite()) => {
            Err(err("nonfinite_number", "JSON numbers must be finite"))
        }
        Value::Array(a) => a.iter().try_for_each(finite),
        Value::Object(o) => o.values().try_for_each(finite),
        _ => Ok(()),
    }
}
fn hash_text(x: &str) -> bool {
    x.len() == 64
        && x.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn scan_duplicate(b: &[u8], mut i: usize) -> Result<usize> {
    fn ws(b: &[u8], mut i: usize) -> usize {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1
        }
        i
    }
    fn string(b: &[u8], mut i: usize) -> Result<usize> {
        i += 1;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 2,
                b'"' => return Ok(i + 1),
                _ => i += 1,
            }
        }
        Err(err("document_read", "unterminated JSON string"))
    }
    fn primitive(b: &[u8], start: usize, end: usize) -> Result<()> {
        let token =
            std::str::from_utf8(&b[start..end]).map_err(|e| err("document_read", e.to_string()))?;
        if matches!(token, "NaN" | "Infinity" | "-Infinity")
            || token.parse::<f64>().is_ok_and(|x| !x.is_finite())
        {
            return Err(err("nonfinite_number", "JSON numbers must be finite"));
        }
        Ok(())
    }
    i = ws(b, i);
    if i >= b.len() {
        return Err(err("document_read", "empty JSON document"));
    };
    match b[i] {
        b'{' => {
            i = ws(b, i + 1);
            let mut keys = BTreeSet::new();
            if b.get(i) == Some(&b'}') {
                return Ok(i + 1);
            }
            loop {
                if b.get(i) != Some(&b'\"') {
                    return Err(err("document_read", "object key must be a string"));
                };
                let e = string(b, i)?;
                let k: String = serde_json::from_slice(&b[i..e])
                    .map_err(|e| err("document_read", e.to_string()))?;
                if !keys.insert(k.clone()) {
                    return Err(err("duplicate_key", format!("duplicate JSON key: {k}")));
                }
                i = ws(b, e);
                if b.get(i) != Some(&b':') {
                    return Err(err("document_read", "object key missing colon"));
                };
                i = scan_duplicate(b, i + 1)?;
                i = ws(b, i);
                match b.get(i) {
                    Some(b',') => i = ws(b, i + 1),
                    Some(b'}') => return Ok(i + 1),
                    _ => return Err(err("document_read", "invalid object separator")),
                }
            }
        }
        b'[' => {
            i = ws(b, i + 1);
            if b.get(i) == Some(&b']') {
                return Ok(i + 1);
            }
            loop {
                i = scan_duplicate(b, i)?;
                i = ws(b, i);
                match b.get(i) {
                    Some(b',') => i = ws(b, i + 1),
                    Some(b']') => return Ok(i + 1),
                    _ => return Err(err("document_read", "invalid array separator")),
                }
            }
        }
        b'\"' => string(b, i),
        _ => {
            let start = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b',' | b']' | b'}')
            {
                i += 1
            }
            primitive(b, start, i)?;
            Ok(i)
        }
    }
}

pub fn load_json_value(path: &Path) -> Result<Value> {
    let b = fs::read(path)
        .map_err(|e| err("document_read", e.to_string()).file(path.display().to_string()))?;
    scan_duplicate(&b, 0)?;
    let v: Value = serde_json::from_slice(&b)
        .map_err(|e| err("document_read", e.to_string()).file(path.display().to_string()))?;
    finite(&v)?;
    Ok(v)
}
pub fn load_json_document(path: &Path) -> Result<Value> {
    let v = load_json_value(path)?;
    if !v.is_object() {
        return Err(err("document_type", "document root must be an object"));
    }
    Ok(v)
}

/// Validate one authored document the instrument owns directly.
///
/// The shape comes from the kind's schema; the local dependencies the
/// document names are then read and hashed, so a declared file that is
/// missing or unreadable fails validation instead of an execution.
pub fn validate_authored_document(kind: &str, path: &Path) -> Result<Value> {
    let value = load_json_document(path)?;
    validate_schema(kind, &value).map_err(|e| e.file(path.display().to_string()))?;
    validate_local_dependencies(kind, &value, path)?;
    Ok(value)
}
/// Absolute form of a path: relative paths resolve against the process working
/// directory, then `.` and `..` components are removed lexically.
///
/// Symlinks are deliberately not resolved here. Callers that need physical
/// identity canonicalize explicitly, and dependency lookup keys must stay
/// lexical so a parent-relative authored path still matches the retained
/// source the runner recorded for it.
pub fn normalize_path(path: &Path) -> Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}
pub fn resolve_path(value: &str, document: &Path) -> Result<PathBuf> {
    if value.is_empty() || value.contains('\0') {
        return Err(err(
            "invalid_path",
            "path must be a nonempty string without NUL",
        ));
    }
    let base = if document.is_absolute() {
        document.to_path_buf()
    } else {
        std::env::current_dir()?.join(document)
    };
    let full = if Path::new(value).is_absolute() {
        PathBuf::from(value)
    } else {
        base.parent().unwrap_or(Path::new("/")).join(value)
    };
    if let Ok(p) = full.canonicalize() {
        return Ok(p);
    }
    normalize_path(&full)
}
pub fn safe_artifact_path(root: &Path, value: &str) -> Result<PathBuf> {
    if value.is_empty()
        || value.contains('\0')
        || value.contains('\\')
        || value.contains(':')
        || value == "."
        || value == ".."
    {
        return Err(err(
            "unsafe_artifact_path",
            "artifact requires a relative POSIX path",
        ));
    }
    let p = Path::new(value);
    if p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(err(
            "unsafe_artifact_path",
            "artifact contains absolute root or traversal",
        ));
    }
    let root = root.canonicalize()?;
    let out = root.join(p);
    let mut cur = root.clone();
    for c in p.components() {
        cur.push(c);
        if cur.is_symlink() {
            return Err(err(
                "unsafe_artifact_path",
                "artifact symlinks are not supported",
            ));
        }
    }
    if out.exists() && !out.canonicalize()?.starts_with(&root) {
        return Err(err("unsafe_artifact_path", "artifact escapes directory"));
    }
    Ok(out)
}
fn unique(a: &[Value], field: &str) -> Result<()> {
    let mut s = BTreeSet::new();
    for x in a {
        let v = obj(x)?
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| err("schema_validation", format!("{field} must be string")))?;
        if !s.insert(v) {
            return Err(err("duplicate_id", format!("duplicate {field}")));
        }
    }
    Ok(())
}
fn file_record(v: &Value, doc: &Path) -> Result<()> {
    let o = obj(v)?;
    let p = text(o, "path")?;
    let h = text(o, "sha256")?;
    if !hash_text(h) {
        return Err(err(
            "invalid_hash",
            "SHA-256 requires 64 lowercase hexadecimal digits",
        ));
    }
    if sha256_file(&resolve_path(p, doc)?)? != h {
        return Err(err("hash_mismatch", format!("hash mismatch for {p}")));
    }
    Ok(())
}
/// One geometry: a named half-open rectangle, or a hashed mask. Both declare
/// their coordinate space, which is what makes a region's position readable
/// without knowing which document carried it.
fn geometry(v: &Value, doc: &Path) -> Result<()> {
    let o = obj(v)?;
    if let Some(space) = o.get("space").and_then(Value::as_str)
        && !matches!(space, "source" | "render")
    {
        return Err(err(
            "invalid_bounds",
            "geometry space must be `source` or `render`",
        ));
    }
    if let Some(a) = v.as_array() {
        if a.len() != 4 || a.iter().any(|x| x.as_u64().is_none()) {
            return Err(err(
                "invalid_bounds",
                "rectangle requires four nonnegative integers",
            ));
        };
        if a[2].as_u64() == Some(0) || a[3].as_u64() == Some(0) {
            return Err(err(
                "invalid_bounds",
                "region width and height must be positive",
            ));
        };
        Ok(())
    } else if o.contains_key("path") {
        let p = text(o, "path")?;
        file_record(v, doc)?;
        if !matches!(
            Path::new(p)
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.to_ascii_lowercase())
                .as_deref(),
            Some("png") | Some("tif") | Some("tiff") | Some("npy") | Some("pgm")
        ) {
            return Err(err("invalid_mask", "mask must use PNG, TIFF, NPY, or PGM"));
        }
        if o.get("width").and_then(Value::as_u64).unwrap_or(0) == 0
            || o.get("height").and_then(Value::as_u64).unwrap_or(0) == 0
        {
            return Err(err("invalid_mask", "mask dimensions must be positive"));
        }
        Ok(())
    } else {
        // A named rectangle: the schema requires the four fields and a
        // positive span, and this confirms the span so a zero-width rectangle
        // cannot pass on shape alone.
        for key in ["x", "y", "width", "height"] {
            if o.get(key).and_then(Value::as_u64).is_none() {
                return Err(err("invalid_bounds", format!("geometry requires {key}")));
            }
        }
        if o["width"].as_u64() == Some(0) || o["height"].as_u64() == Some(0) {
            return Err(err(
                "invalid_bounds",
                "region width and height must be positive",
            ));
        }
        Ok(())
    }
}
fn inline_assignment(source: &str) -> bool {
    let mut start = 0;
    let mut depth = 0i32;
    let (mut quoted, mut escaped) = (false, false);
    let mut equals = None;
    let extended = format!("{source},");
    for (i, c) in extended.char_indices() {
        if quoted {
            if escaped {
                escaped = false
            } else if c == '\\' {
                escaped = true
            } else if c == '"' {
                quoted = false
            }
        } else if c == '"' {
            quoted = true
        } else if c == '[' {
            depth += 1
        } else if c == ']' {
            depth -= 1;
            if depth < 0 {
                return false;
            }
        } else if c == '=' && depth == 0 && equals.is_none() {
            equals = Some(i)
        } else if c == ',' && depth == 0 {
            let Some(eq) = equals else { return false };
            let key = extended[start..eq].trim();
            if !key.split('.').all(|k| {
                let mut b = k.bytes();
                b.next().is_some_and(|x| x.is_ascii_lowercase())
                    && b.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == b'_')
            }) || extended[eq + 1..i].trim().is_empty()
            {
                return false;
            }
            start = i + 1;
            equals = None
        }
    }
    !quoted && depth == 0
}
fn config(v: &Value, doc: &Path) -> Result<()> {
    let o = obj(v)?;
    if let Some(a) = o.get("set") {
        for x in a
            .as_array()
            .ok_or_else(|| err("schema_validation", "set must be array"))?
        {
            if !x.as_str().is_some_and(inline_assignment) {
                return Err(err("inline_assignment", "invalid renderer assignment"));
            }
        }
    }
    for k in ["presets", "set_files"] {
        if let Some(a) = o.get(k) {
            for x in a
                .as_array()
                .ok_or_else(|| err("schema_validation", "file carrier must be array"))?
            {
                sha256_file(&resolve_path(
                    x.as_str()
                        .ok_or_else(|| err("schema_validation", "path must be string"))?,
                    doc,
                )?)?;
            }
        }
    }
    if let Some(p) = o.get("params") {
        sha256_file(&resolve_path(
            p.as_str()
                .ok_or_else(|| err("schema_validation", "params must be string"))?,
            doc,
        )?)?;
    }
    Ok(())
}

/// Region role names accepted by case and region records. The regions schema
/// is the single authority for the closed set, and the suite schema's case and
/// reference regions carry the same enumeration.
pub fn region_roles() -> Result<Vec<String>> {
    enum_names(
        &schema_for("regions")?["properties"]["regions"]["items"]["properties"]["role"]["enum"],
    )
}

/// Measurement family names accepted by case and region records.
pub fn measurement_families() -> Result<Vec<String>> {
    enum_names(
        &schema_for("regions")?["properties"]["regions"]["items"]["properties"]["measurements"]["items"]
            ["enum"],
    )
}

fn enum_names(value: &Value) -> Result<Vec<String>> {
    value
        .as_array()
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .ok_or_else(|| err("schema_asset", "enum schema is missing its names"))
}

pub fn schema_for(kind: &str) -> Result<Value> {
    // `metrics` is not a document: it is the closed vocabulary of metric
    // identities a policy may name. The emitted object is the vocabulary, not
    // a JSON Schema.
    if kind == "metrics" {
        return Ok(metrics_vocabulary());
    }
    if matches!(kind, "configuration" | "reference" | "preparation") {
        let suite = schema_for("suite")?;
        let mut schema = suite["properties"]["cases"]["items"]["properties"][kind].clone();
        if kind == "reference" {
            let properties = schema["properties"].as_object_mut().unwrap();
            properties.remove("path");
            properties.remove("sha256");
            schema["required"]
                .as_array_mut()
                .unwrap()
                .retain(|v| v != "path" && v != "sha256");
        }
        schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
        return Ok(schema);
    }
    // A regions document is its own kind: the same region records a suite case
    // embeds, in the document envelope that carries the schema version and the
    // identity. `schema --kind regions` emits the document.
    if kind == "regions" {
        let raw = include_str!("../assets/regions.schema.json");
        let mut schema: Value =
            serde_json::from_str(raw).map_err(|e| err("schema_asset", e.to_string()))?;
        schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
        return Ok(schema);
    }
    let raw = match kind {
        "suite" => include_str!("../assets/suite.schema.json"),
        "target" => include_str!("../assets/target.schema.json"),
        "policy" => include_str!("../assets/policy.schema.json"),
        "run" => include_str!("../assets/run.schema.json"),
        "comparison" => include_str!("../assets/comparison.schema.json"),
        "review" => include_str!("../assets/review.schema.json"),
        "regions" => include_str!("../assets/regions.schema.json"),
        "stimuli" => include_str!("../assets/stimuli.schema.json"),
        "experiment" => include_str!("../assets/experiment.schema.json"),
        "preflight" => include_str!("../assets/preflight.schema.json"),
        "provenance" => include_str!("../assets/provenance.schema.json"),
        "report" => include_str!("../assets/report.schema.json"),
        "resolved" => include_str!("../assets/resolved.schema.json"),
        "command" => include_str!("../assets/command.schema.json"),
        "export" => include_str!("../assets/export.schema.json"),
        "evidence" => include_str!("../assets/evidence.schema.json"),
        "material_measurement" => include_str!("../assets/material_measurement.schema.json"),
        "correspondence_review" => include_str!("../assets/correspondence_review.schema.json"),
        _ => {
            return Err(err(
                "unknown_document_kind",
                format!("unknown document kind: {kind}"),
            ));
        }
    };
    serde_json::from_str(raw).map_err(|e| err("schema_asset", e.to_string()))
}

/// The closed metric-identity vocabulary a policy may name.
///
/// The measurement protocol owns the identities; this vocabulary states the
/// families, the per-family identities, the seed aggregates, and the domains
/// so a policy author can read them from `schema` alone.
fn metrics_vocabulary() -> Value {
    json!({
        "kind": "metrics",
        "metric_version": crate::metrics::METRIC_VERSION,
        "color_transform": crate::metrics::COLOR_TRANSFORM,
        "families": {
            "numeric": ["max_abs_difference", "rmse"],
            "color": ["delta_e00", "delta_e00_p95"],
            "tone": ["mtf50", "tone_delta_e00"],
            "flip": ["flip_mean"],
            "grain": ["grain_residual_rms", "grain_spectrum_rmse"],
            "halation": ["halation_radius_px", "halation_energy_ratio"],
            "edge": ["edge_mtf50", "edge_shift_px"],
        },
        "seed_aggregates": ["seed_mean", "seed_std"],
        "domains": ["scalar", "curve", "map", "spectrum"],
        "estimators": ["mean", "median", "p95"],
        "notes": "A metric identity is `<family>.<name>` optionally suffixed with `.seed_mean` or `.seed_std`. The protocol resolves each family to explicit identities for the selected case and region; a policy must name an identity the selected case and region can produce."
    })
}
fn validate_schema(kind: &str, v: &Value) -> Result<()> {
    let schema = schema_for(kind)?;
    let compiled =
        jsonschema::validator_for(&schema).map_err(|e| err("schema_asset", e.to_string()))?;
    let mut errors = compiled.iter_errors(v).collect::<Vec<_>>();
    errors.sort_by_key(|e| e.instance_path.to_string());
    if let Some(e) = errors.first() {
        let code = match &e.kind {
            jsonschema::error::ValidationErrorKind::AdditionalProperties { .. } => "unknown_field",
            jsonschema::error::ValidationErrorKind::Required { .. } => "missing_field",
            jsonschema::error::ValidationErrorKind::UniqueItems => "duplicate_value",
            jsonschema::error::ValidationErrorKind::Minimum { .. }
            | jsonschema::error::ValidationErrorKind::Maximum { .. }
            | jsonschema::error::ValidationErrorKind::MinItems { .. } => "invalid_bounds",
            jsonschema::error::ValidationErrorKind::Pattern { .. } => "invalid_identifier",
            _ => "schema_validation",
        };
        // The instance path is already a JSON Pointer; `$` is a private
        // syntax, so the error carries the pointer itself.
        return Err(err(code, e.to_string()).pointer(format!(
            "/{}",
            e.instance_path.to_string().trim_start_matches('/')
        )));
    }
    Ok(())
}
pub fn validate_local_dependencies(kind: &str, value: &Value, document: &Path) -> Result<()> {
    match kind {
        "configuration" => config(value, document)?,
        // A regions document carries its regions under `regions`; a suite case
        // embeds the same records as its own array. Both shapes reach this one
        // validator, so one region shape has one authority.
        "regions" => {
            let regions = if let Some(array) = value.get("regions").and_then(Value::as_array) {
                array
            } else {
                value
                    .as_array()
                    .ok_or_else(|| err("schema_validation", "regions must be an array"))?
            };
            unique(regions, "id")?;
            for region in regions {
                if region["id"] == "full_image" {
                    return Err(err("reserved_region", "full_image is reserved"));
                }
                geometry(&region["geometry"], document)?;
            }
        }
        "preparation" => file_record(&value["original"], document)?,
        "reference" => {
            for key in [
                "transform",
                "aligned_derivative",
                "correspondence_review",
                "common_area_mask",
            ] {
                if let Some(record) = value.get(key) {
                    file_record(record, document)?;
                }
            }
            if let Some(icc) = value["color"].get("icc") {
                file_record(icc, document)?;
            }
            for region in value["regions"].as_array().into_iter().flatten() {
                geometry(&region["geometry"], document)?;
            }
            if value["scope"] == "aligned_scene"
                && (value.get("correspondence_review").is_none()
                    || value.get("common_area_mask").is_none()
                    || (value.get("transform").is_none()
                        && value.get("aligned_derivative").is_none()))
            {
                return Err(err(
                    "missing_correspondence",
                    "aligned scene requires transform/derivative, review, and mask",
                ));
            }
            if value["scope"] == "aligned_scene" {
                crate::references::validate_correspondence_documents(value, document)?;
                let review_path = resolve_path(
                    value["correspondence_review"]["path"].as_str().unwrap(),
                    document,
                )?;
                let review = load_json_document(&review_path)?;
                for region in review["excluded_regions"].as_array().into_iter().flatten() {
                    geometry(&region["geometry"], &review_path)?;
                }
            }
            if value["scope"] == "regional"
                && value["regions"].as_array().is_none_or(|v| v.is_empty())
            {
                return Err(err(
                    "missing_regions",
                    "regional reference requires corresponding regions",
                ));
            }
            if value["scope"] == "material" {
                for key in ["domain", "units", "instrument", "conditions"] {
                    if value.get(key).is_none() {
                        return Err(err(
                            "material_conditions",
                            "material reference requires measurement conditions",
                        ));
                    }
                }
            }
            if value["color"].get("icc").is_none()
                && (value["color"].get("color_space").is_none()
                    || value["color"].get("cctf_decoding").is_none())
            {
                return Err(err(
                    "reference_color",
                    "reference requires explicit color or hashed ICC",
                ));
            }
        }
        // A stimulus document names the arrays it carries; those files are its
        // dependencies, and they are read beside the document.
        "stimuli" => {
            for key in ["points", "reflectances"] {
                if let Some(record) = value.get(key) {
                    file_record(record, document)?;
                }
            }
        }
        // An experiment document names the suite, the input, and every
        // parameter carrier its chain declares.
        "experiment" => {
            file_record(&value["input"], document)?;
            for key in ["transform", "aligned_derivative"] {
                if let Some(record) = value.get(key) {
                    file_record(record, document)?;
                }
            }
        }
        "material_measurement" => {
            for key in ["source", "measurement"] {
                if let Some(record) = value.get(key) {
                    file_record(record, document)?;
                }
            }
        }
        "correspondence_review" => {
            for key in ["left", "right", "transform"] {
                if let Some(record) = value.get(key) {
                    file_record(record, document)?;
                }
            }
        }
        _ => return Err(err("unknown_document_kind", kind)),
    }
    Ok(())
}

/// A tone-response probe needs at least two strictly increasing exposure
/// coordinates: the response curve is sampled at those coordinates, so a
/// probe that cannot order them cannot produce a curve.
fn validate_tone_probe(points: &Value) -> Result<()> {
    let points = points.as_array().map(Vec::as_slice).unwrap_or_default();
    if points.is_empty() {
        return Ok(());
    }
    let mut previous: Option<f64> = None;
    for point in points {
        let exposure = point
            .get("exposure_stops")
            .and_then(Value::as_f64)
            .ok_or_else(|| {
                err(
                    "invalid_protocol",
                    "tone response point requires a finite exposure_stops",
                )
            })?;
        if !exposure.is_finite() {
            return Err(err(
                "invalid_protocol",
                "tone response exposure_stops must be finite",
            ));
        }
        if let Some(previous) = previous
            && exposure <= previous
        {
            return Err(err(
                "invalid_protocol",
                "tone response exposures must be strictly increasing with at least two points",
            ));
        }
        previous = Some(exposure);
    }
    if points.len() < 2 {
        return Err(err(
            "invalid_protocol",
            "tone response exposures must be strictly increasing with at least two points",
        ));
    }
    Ok(())
}

pub(crate) fn validate_shape(kind: &str, value: &Value) -> Result<()> {
    validate_authored(kind, value)
}

fn validate_authored(kind: &str, v: &Value) -> Result<()> {
    finite(v)?;
    validate_schema(kind, v)?;
    Ok(())
}

pub fn load_suite(path: &Path) -> Result<Value> {
    let v = load_json_document(path)?;
    validate_suite(&v, path)?;
    Ok(v)
}

pub fn validate_suite(v: &Value, path: &Path) -> Result<()> {
    validate_authored("suite", v).map_err(|e| e.file(path.display().to_string()))?;
    let cases = v["cases"].as_array().unwrap();
    unique(cases, "id")?;
    for x in v["protocol"]["probe"]["tone_response"]
        .as_array()
        .into_iter()
        .flatten()
    {
        geometry(&x["geometry"], path)?
    }
    // A tone probe whose exposure coordinates are not at least two strictly
    // increasing values cannot produce a response curve, so it fails
    // validation instead of a later measurement.
    validate_tone_probe(&v["protocol"]["probe"]["tone_response"])?;
    for c in cases {
        let parsed: CaseDocument = serde_json::from_value(c.clone())?;
        file_record(&c["input"], path)?;
        if let Some(preparation) = c.get("preparation") {
            file_record(&preparation["original"], path)?;
        }
        let input = obj(&c["input"])?;
        let raw = input.get("kind").and_then(Value::as_str) == Some("raw");
        if raw {
            if input.get("adapter").and_then(Value::as_str) != Some("spektrafilm-raw")
                || input.contains_key("color_space")
                || input.contains_key("cctf_decoding")
            {
                return Err(err(
                    "source_declaration",
                    "RAW requires adapter and no raster declaration",
                ));
            }
        } else if input.contains_key("adapter")
            || !input.contains_key("color_space")
            || !input.contains_key("cctf_decoding")
        {
            return Err(err(
                "source_declaration",
                "raster requires explicit color and transfer declaration",
            ));
        }
        if !raw
            && c["configuration"]
                .get("raw")
                .is_some_and(|x| x.as_object().is_some_and(|x| !x.is_empty()))
        {
            return Err(err("inapplicable_field", "RAW controls require RAW source"));
        }
        config(&parsed.configuration, path)?;
        unique(c["regions"].as_array().unwrap(), "id")?;
        for r in &parsed.regions {
            if r.id == "full_image" {
                return Err(err("reserved_region", "full_image is reserved"));
            }
            geometry(&r.geometry, path)?
        }
        crate::references::validate_region_pairs(c)?;
        for x in c["probe"]["tone_response"].as_array().into_iter().flatten() {
            geometry(&x["geometry"], path)?
        }
        if let Some(r) = c.get("reference") {
            file_record(r, path)?;
            for k in [
                "transform",
                "aligned_derivative",
                "correspondence_review",
                "common_area_mask",
            ] {
                if let Some(x) = r.get(k) {
                    file_record(x, path)?
                }
            }
            match r["scope"].as_str().unwrap() {
                "aligned_scene" => {
                    if r.get("correspondence_review").is_none()
                        || r.get("common_area_mask").is_none()
                        || (r.get("transform").is_none() && r.get("aligned_derivative").is_none())
                    {
                        return Err(err(
                            "missing_correspondence",
                            "aligned scene requires transform/derivative, review, and mask",
                        ));
                    }
                    crate::references::validate_correspondence_documents(r, path)?;
                    let review_path =
                        resolve_path(r["correspondence_review"]["path"].as_str().unwrap(), path)?;
                    let review = load_json_document(&review_path)?;
                    for x in review["excluded_regions"].as_array().into_iter().flatten() {
                        geometry(&x["geometry"], &review_path)?
                    }
                }
                "regional" => {
                    if r["regions"].as_array().is_none_or(|a| a.is_empty()) {
                        return Err(err(
                            "missing_regions",
                            "regional reference requires corresponding regions",
                        ));
                    }
                }
                "material" => {
                    for k in ["domain", "units", "instrument", "conditions"] {
                        if r.get(k).is_none() {
                            return Err(err(
                                "material_conditions",
                                "material reference requires measurement conditions",
                            ));
                        }
                    }
                }
                _ => {}
            }
            let color = obj(&r["color"])?;
            if let Some(icc) = color.get("icc") {
                file_record(icc, path)?
            } else if !color.contains_key("color_space") || !color.contains_key("cctf_decoding") {
                return Err(err(
                    "reference_color",
                    "reference requires explicit color or hashed ICC",
                ));
            }
            for x in r["regions"].as_array().into_iter().flatten() {
                geometry(&x["geometry"], path)?
            }
        }
        if let Some(p) = c.get("probe_response") {
            for curve in p["curves"].as_array().unwrap() {
                let mut previous = None;
                for sample in curve["samples"].as_array().unwrap() {
                    let coordinate = sample["coordinate"].as_f64().unwrap();
                    let region = sample["region_id"].as_str().unwrap();
                    if previous.is_some_and(|p| coordinate <= p)
                        || (region != "full_image"
                            && !parsed.regions.iter().any(|r| r.id == region))
                    {
                        return Err(err(
                            "probe_coordinates",
                            "probe coordinates must increase and regions must resolve",
                        ));
                    }
                    previous = Some(coordinate)
                }
            }
        }
    }
    Ok(())
}
pub fn load_target(path: &Path) -> Result<Value> {
    let v = load_json_document(path)?;
    validate_authored("target", &v)?;
    file_record(&v["renderer"], path)?;
    let renderer = resolve_path(v["renderer"]["path"].as_str().unwrap(), path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&renderer)?.permissions().mode() & 0o111 == 0 {
            return Err(err("renderer_executable", "renderer must be executable"));
        }
    }
    if !resolve_path(v["data_dir"].as_str().unwrap(), path)?.is_dir() {
        return Err(err("data_directory", "data directory must exist"));
    }
    for changes in std::iter::once(&v["changes"]).chain(
        v["case_changes"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.values()),
    ) {
        config(changes, path)?;
        for assignment in changes["set"].as_array().unwrap() {
            let source = assignment.as_str().unwrap();
            for (section, names) in CASE_OWNED_FIELDS {
                // Preview settings are compared against the case-only dry
                // resolution by the runner; this static guard covers the input
                // interpretation and geometry keys.
                if *section != "io" {
                    continue;
                }
                for name in *names {
                    let key = format!("{section}.{name}");
                    if source
                        .split(',')
                        .any(|x| x.split_once('=').is_some_and(|(p, _)| p.trim() == key))
                    {
                        return Err(err(
                            "target_geometry",
                            "target must not modify input interpretation or geometry",
                        ));
                    }
                }
            }
        }
    }
    Ok(v)
}
pub fn load_policy(path: &Path) -> Result<Value> {
    let v = load_json_document(path)?;
    validate_authored("policy", &v)?;
    let o = obj(&v)?;
    let cal = obj(req(o, "calibration")?)?;
    if cal.get("status").and_then(Value::as_str) == Some("calibrated")
        && (cal
            .get("evidence")
            .and_then(Value::as_array)
            .is_none_or(|a| a.is_empty())
            || ["cases", "labels", "observation_scale", "scope"]
                .iter()
                .any(|k| !cal.contains_key(*k)))
    {
        return Err(err(
            "missing_calibration",
            "calibrated policy requires reviewed evidence and population",
        ));
    }
    for x in cal["evidence"].as_array().unwrap() {
        file_record(x, path)?
    }
    let rules = o["rules"].as_array().unwrap();
    unique(rules, "id")?;
    for x in rules {
        let r = obj(x)?;
        if let (Some(a), Some(b)) = (
            r.get("min").and_then(Value::as_f64),
            r.get("max").and_then(Value::as_f64),
        ) && a > b
        {
            return Err(err("invalid_bounds", "rule min exceeds max"));
        }
        if !r.contains_key("min")
            && !r.contains_key("max")
            && !(r.get("purpose").and_then(Value::as_str) == Some("expect")
                && r.contains_key("direction"))
        {
            return Err(err(
                "missing_constraint",
                "rule requires an interval or expected direction",
            ));
        }
        if r.contains_key("direction") && r.get("purpose").and_then(Value::as_str) != Some("expect")
        {
            return Err(err(
                "inapplicable_field",
                "direction belongs to expected-change rules",
            ));
        }
    }
    Ok(v)
}
pub fn validate_target_cases(target: &Value, suite: &Value) -> Result<()> {
    let allowed = suite
        .get("cases")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("id").and_then(Value::as_str))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    if let Some(m) = target.get("case_changes").and_then(Value::as_object)
        && let Some(k) = m.keys().find(|k| !allowed.contains(k.as_str()))
    {
        return Err(err(
            "unresolved_identifier",
            format!("unknown target case: {k}"),
        ));
    }
    Ok(())
}
pub fn protocol_digest(suite: &Value) -> Result<String> {
    fn identity(v: &Value) -> Value {
        match v {
            Value::Object(o) => Value::Object(
                o.iter()
                    .filter(|(k, _)| k.as_str() != "path")
                    .map(|(k, v)| (k.clone(), identity(v)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(identity).collect()),
            x => x.clone(),
        }
    }
    let o = obj(suite)?;
    let authored_cases = req(o, "cases")?
        .as_array()
        .ok_or_else(|| err("schema_validation", "suite cases must be array"))?;
    let mut cases = Vec::new();
    for c in authored_cases {
        let co = obj(c)?;
        cases.push(Value::Object(
            [
                "id",
                "input",
                "seeds",
                "regions",
                "region_pairs",
                "measurements",
                "probe",
                "probe_response",
                "reference",
            ]
            .iter()
            .filter_map(|k| co.get(*k).map(|v| ((*k).to_string(), identity(v))))
            .collect(),
        ));
    }
    let icc = if authored_cases
        .iter()
        .any(|c| c["reference"]["color"].get("icc").is_some())
    {
        json!({"lcms2":crate::icc::icc_version()?,"intent":"relative_colorimetric","black_point_compensation":false,"format":"RGB_DBL"})
    } else {
        Value::Null
    };
    digest(
        &json!({"protocol":req(o,"protocol")?,"cases":cases,"metric_version":crate::metrics::METRIC_VERSION,"reference_version":crate::references::METRIC_VERSION,"color_transform":crate::metrics::COLOR_TRANSFORM,"native_identity":crate::image_data::native_identity(),"icc_adapter":icc,"implementation":{"language":"Rust","package_version":env!("CARGO_PKG_VERSION"),"source_revision":env!("SPEKTRAFILM_BENCHMARK_REVISION"),"arithmetic":"IEEE-754 binary64; official FLIP binary32"}}),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegionDocument {
    pub id: String,
    pub label: String,
    pub role: String,
    pub geometry: Value,
    pub measurements: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaseDocument {
    pub id: String,
    pub kind: String,
    pub input: Value,
    pub configuration: Value,
    pub seeds: Vec<u64>,
    pub regions: Vec<RegionDocument>,
    pub measurements: Vec<String>,
}

pub fn validate_policy_selectors(policy: &Value, suite: &Value) -> Result<()> {
    if policy.get("protocol_digest").and_then(Value::as_str)
        != Some(protocol_digest(suite)?.as_str())
    {
        return Err(err(
            "protocol_mismatch",
            "policy must bind to comparison protocol",
        ));
    }
    let cases = suite["cases"].as_array().cloned().unwrap_or_default();
    for r in policy["rules"].as_array().cloned().unwrap_or_default() {
        let s = &r["selector"];
        let c = s["case_id"].as_str().unwrap_or("");
        let reg = s["region_id"].as_str().unwrap_or("");
        let Some(case) = cases.iter().find(|x| x["id"].as_str() == Some(c)) else {
            return Err(err(
                "unresolved_identifier",
                "policy selector does not resolve",
            ));
        };
        if reg != "full_image"
            && !case["regions"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x["id"].as_str() == Some(reg)))
            && !case["region_pairs"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x["id"].as_str() == Some(reg)))
        {
            return Err(err(
                "unresolved_identifier",
                "policy selector does not resolve",
            ));
        }
        // The rule's metric identity is decidable from the document: the
        // measurement definitions the protocol resolves for this case and
        // region decide whether the rule can ever produce a measurement. The
        // same definitions decide it at compare time, so the check is not a
        // second registry.
        let metric = r["metric"].as_str().unwrap_or("");
        if !crate::metrics::metric_supported_with_protocol(metric, case, reg, &suite["protocol"]) {
            return Err(err(
                "unresolved_metric",
                format!("Metric is unknown or unselected: {metric}"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suite(color: Option<Value>, pairs: Value) -> Value {
        let mut protocol = json!({"id":"appearance-v1","pixels_per_degree":60.,"region_sampling_scales":[1.],"grain":{"residual":{"method":"flat_mean","high_pass_radius_px":1.,"border_px":0},"spectrum":{"window":"boxcar"}},"hotspots":{"count":0,"min_area_px":1}});
        if let Some(color) = color {
            protocol["color"] = color;
        }
        json!({"schema_version":1,"id":"t","protocol":protocol,"cases":[{"id":"c","kind":"probe","input":{"path":"x.png","sha256":"0000000000000000000000000000000000000000000000000000000000000000"},"configuration":{},"seeds":[0],"regions":[{"id":"a","label":"A","role":"color","geometry":{"x":0,"y":0,"width":1,"height":1,"space":"render"},"measurements":[]},{"id":"b","label":"B","role":"neutral","geometry":{"x":1,"y":0,"width":1,"height":1,"space":"render"},"measurements":[]}],"region_pairs":pairs,"measurements":[]}]})
    }

    #[test]
    fn suite_schema_bounds_the_hue_floor_and_pair_documents() {
        assert!(
            validate_shape(
                "suite",
                &suite(
                    Some(json!({"hue_chroma_floor": 2.})),
                    json!([{"id":"ab","label":"A/B","left":"a","right":"b"}])
                )
            )
            .is_ok()
        );
        assert!(validate_shape("suite", &suite(None, json!([]))).is_ok());
        for invalid in [
            json!({"hue_chroma_floor": 0.}),
            json!({"hue_chroma_floor": -1.}),
            json!({"hue_chroma_floor": "2"}),
            json!({}),
        ] {
            assert!(validate_shape("suite", &suite(Some(invalid), json!([]))).is_err());
        }
        for invalid in [
            json!([{"id":"ab","label":"A/B","left":"a"}]),
            json!([{"id":"ab","label":"A/B","left":"a","right":"b","extra":1}]),
            json!([{"id":"a b","label":"A/B","left":"a","right":"b"}]),
            json!([{"id":"ab","label":"","left":"a","right":"b"}]),
        ] {
            assert!(validate_shape("suite", &suite(None, invalid)).is_err());
        }
    }
}
