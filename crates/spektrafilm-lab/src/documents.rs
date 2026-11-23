use crate::common::{Result, digest, sha256_file};

use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

mod evidence;
mod masks;

pub use crate::common::Availability;
pub use evidence::{MeasurementRecord, validate_generated_document};
pub use masks::{geometry_mask, reference_geometry_mask};

pub const ID_PATTERN: &str = r"^[A-Za-z0-9][A-Za-z0-9_.-]*$";

pub(crate) use crate::common::err;
pub(crate) fn obj(v: &Value) -> Result<&Map<String, Value>> {
    v.as_object()
        .ok_or_else(|| err("document_type", "document root must be an object"))
}
pub(crate) fn req<'a>(o: &'a Map<String, Value>, k: &str) -> Result<&'a Value> {
    o.get(k)
        .ok_or_else(|| err("missing_field", format!("missing field: {k}")))
}
pub(crate) fn text<'a>(o: &'a Map<String, Value>, k: &str) -> Result<&'a str> {
    req(o, k)?
        .as_str()
        .ok_or_else(|| err("schema_validation", format!("{k} must be a string")))
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

/// One geometry: a named half-open rectangle, or a hashed mask. Both declare
/// their coordinate space, which is what makes a region's position readable
/// without knowing which document carried it.
pub const AUTHORING_COORDINATE_SPACES: &[&str] = &["source", "render"];
pub const NATIVE_COORDINATE_SPACES: &[&str] = &[
    "source_pixels",
    "working_pixels",
    "film_frame_pixels",
    "grain_aperture_pixels",
    "output_pixels",
];

/// Resolve an authored asset path relative to a document without allowing an
/// absolute path, a parent component, or a symlink to escape the document
/// directory.
///
/// The lexical check keeps a safe authored relative path usable, so
/// `link/photo.png` where `link` is a real directory still resolves normally.
/// Physical confinement is then confirmed by comparing the resolved path's
/// canonical (symlink-free) prefix with the document directory's canonical
/// prefix, so a symlink anywhere along the authored path cannot point outside
/// the manifest directory. A missing final component still resolves: the
/// caller's own read of the path reports the dependency.
pub fn resolve_relative_asset_path(value: &str, document: &Path) -> Result<PathBuf> {
    if value.is_empty() || value.contains('\0') {
        return Err(err(
            "invalid_path",
            "asset path must be a nonempty relative path without NUL",
        ));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(err(
            "invalid_path",
            "asset path must stay below the manifest directory",
        ));
    }
    let base = document
        .parent()
        .ok_or_else(|| err("invalid_path", "manifest has no parent directory"))?;
    let joined = base.join(path);
    let boundary = match base.canonicalize() {
        Ok(boundary) => boundary,
        // The manifest directory itself is not resolvable; the caller's read
        // of the asset reports the dependency failure.
        Err(_) => return Ok(joined),
    };
    let confined = match joined.canonicalize() {
        Ok(physical) => physical.starts_with(&boundary),
        // A component on the path is missing or is a symlink loop. Walk the
        // existing prefix so an escaping symlink is still reported instead of
        // surfacing later as a confusing read failure.
        Err(_) => {
            let mut existing = boundary.clone();
            let mut confined = true;
            for component in path.components() {
                existing.push(component.as_os_str());
                let Ok(physical) = existing.canonicalize() else {
                    break;
                };
                if !physical.starts_with(&boundary) {
                    confined = false;
                    break;
                }
                existing = physical;
            }
            confined
        }
    };
    if !confined {
        return Err(err(
            "invalid_path",
            "asset path must stay below the manifest directory",
        ));
    }
    Ok(joined)
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

fn geometry(v: &Value, doc: &Path) -> Result<()> {
    let o = obj(v)?;
    if let Some(space) = o.get("space").and_then(Value::as_str)
        && !AUTHORING_COORDINATE_SPACES.contains(&space)
        && !NATIVE_COORDINATE_SPACES.contains(&space)
    {
        return Err(err(
            "invalid_bounds",
            "geometry space is not a supported coordinate space",
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

/// Region role names accepted by case and region records. The regions schema
/// is the single authority for the closed set, and the suite schema's case and
/// reference regions carry the same enumeration.
pub fn region_roles() -> Result<Vec<String>> {
    enum_names(
        &schema_for("regions")?["properties"]["regions"]["items"]["properties"]["role"]["enum"],
    )
}

/// Measurement family names a document may declare.
///
/// The vocabulary belongs to the measurement library, not to a profile: a
/// profile selects from it, and a lab profile that selects none of a family does
/// not make that family invalid elsewhere.
pub fn measurement_families() -> Result<Vec<String>> {
    Ok(crate::metrics::FAMILIES
        .iter()
        .map(|family| (*family).to_owned())
        .collect())
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

/// Every document kind this project names, and whether `validate` can decide it.
///
/// One list. `schema` prints a kind a caller may only inspect; `validate`
/// checks a kind it has a rule for. Both read this table, so the two never drift
/// apart and neither hides a kind the other accepts.
pub const DOCUMENT_KINDS: &[(&str, bool)] = &[
    ("regions", true),
    ("comparison", true),
    ("analysis", true),
    ("metrics", false),
    ("report", false),
];

/// The kinds `schema` prints.
pub fn schema_kinds() -> Vec<&'static str> {
    DOCUMENT_KINDS.iter().map(|(kind, _)| *kind).collect()
}

/// The kinds `validate` decides.
pub fn schema_for(kind: &str) -> Result<Value> {
    // `metrics` is not a document: it is the closed vocabulary of metric
    // identities a policy may name. The emitted object is the vocabulary, not
    // a JSON Schema.
    if kind == "metrics" {
        return Ok(metrics_vocabulary());
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
        "comparison" => include_str!("../assets/comparison.schema.json"),
        "analysis" => include_str!("../assets/analysis.schema.json"),
        "report" => include_str!("../assets/report.schema.json"),
        _ => {
            return Err(err(
                "unknown_document_kind",
                format!("unknown document kind: {kind}"),
            ));
        }
    };
    serde_json::from_str(raw).map_err(|e| err("schema_asset", e.to_string()))
}

/// The closed metric-identity vocabulary a record may name.
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
        "notes": "A metric identity is `<family>.<name>` optionally suffixed with `.seed_mean` or `.seed_std`. The protocol resolves each family to explicit identities for the selected case and region."
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
/// Load the one regions document shape a measurement reads.
///
/// A regions document carries its regions under `regions`; a suite case used
/// to embed the same records as a bare array. One shape now, one authority.
pub fn load_regions(path: &Path) -> Result<Value> {
    let value = load_json_value(path)?;
    validate_shape("regions", &value)?;
    validate_local_dependencies("regions", &value, path)?;
    Ok(value)
}

pub fn validate_local_dependencies(kind: &str, value: &Value, document: &Path) -> Result<()> {
    match kind {
        "regions" => {
            let regions = value
                .get("regions")
                .and_then(Value::as_array)
                .ok_or_else(|| err("schema_validation", "regions must be an array"))?;
            unique(regions, "id")?;
            for region in regions {
                if region["id"] == "full_image" {
                    return Err(err("invalid_region_identity", "full_image is reserved"));
                }
                geometry(&region["geometry"], document)?;
            }
        }
        "stimuli" => {
            for key in ["points", "reflectances"] {
                if let Some(record) = value.get(key) {
                    file_record(record, document)?;
                }
            }
        }
        _ => return Err(err("unknown_document_kind", kind)),
    }
    Ok(())
}

pub fn validate_shape(kind: &str, value: &Value) -> Result<()> {
    validate_authored(kind, value)
}

fn validate_authored(kind: &str, v: &Value) -> Result<()> {
    finite(v)?;
    validate_schema(kind, v)?;
    Ok(())
}

pub fn protocol_digest(record: &Value) -> Result<String> {
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
    let o = obj(record)?;
    let authored_cases = req(o, "cases")?
        .as_array()
        .ok_or_else(|| err("schema_validation", "record cases must be array"))?;
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
