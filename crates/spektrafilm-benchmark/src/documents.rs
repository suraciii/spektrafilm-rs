use crate::common::{Error, Result, digest, sha256_file};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const ID_PATTERN: &str = r"^[A-Za-z0-9][A-Za-z0-9_.-]*$";
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

pub fn load_json_document(path: &Path) -> Result<Value> {
    let b = fs::read(path)
        .map_err(|e| err("document_read", e.to_string()).at(path.display().to_string()))?;
    scan_duplicate(&b, 0)?;
    let v: Value = serde_json::from_slice(&b)
        .map_err(|e| err("document_read", e.to_string()).at(path.display().to_string()))?;
    finite(&v)?;
    if !v.is_object() {
        return Err(err("document_type", "document root must be an object"));
    }
    Ok(v)
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
    let mut normalized = PathBuf::new();
    for component in full.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            x => normalized.push(x),
        }
    }
    Ok(normalized)
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
    } else {
        let o = obj(v)?;
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

pub fn schema_for(kind: &str) -> Result<Value> {
    let raw = match kind {
        "suite" => include_str!("../assets/suite.schema.json"),
        "target" => include_str!("../assets/target.schema.json"),
        "policy" => include_str!("../assets/policy.schema.json"),
        "run" => include_str!("../assets/run.schema.json"),
        "comparison" => include_str!("../assets/comparison.schema.json"),
        _ => {
            return Err(err(
                "unknown_document_kind",
                format!("unknown document kind: {kind}"),
            ));
        }
    };
    serde_json::from_str(raw).map_err(|e| err("schema_asset", e.to_string()))
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
        let p = format!(
            "${}",
            e.instance_path
                .to_string()
                .split('/')
                .skip(1)
                .map(|part| format!("[{}]", part.replace("~1", "/").replace("~0", "~")))
                .collect::<String>()
        );
        return Err(err(code, e.to_string()).at(p));
    }
    Ok(())
}
fn validate_authored(kind: &str, v: &Value) -> Result<()> {
    finite(v)?;
    validate_schema(kind, v)?;
    Ok(())
}

pub fn load_suite(path: &Path) -> Result<Value> {
    let v = load_json_document(path)?;
    validate_authored("suite", &v)?;
    let cases = v["cases"].as_array().unwrap();
    unique(cases, "id")?;
    for x in v["protocol"]["probe"]["tone_response"]
        .as_array()
        .into_iter()
        .flatten()
    {
        geometry(&x["geometry"], path)?
    }
    for c in cases {
        let parsed: CaseDocument = serde_json::from_value(c.clone())?;
        file_record(&c["input"], path)?;
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
                    let review = load_json_document(&resolve_path(
                        r["correspondence_review"]["path"].as_str().unwrap(),
                        path,
                    )?)?;
                    for x in review["excluded_regions"].as_array().into_iter().flatten() {
                        geometry(&x["geometry"], path)?
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
    Ok(v)
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
            for key in [
                "io.input_color_space",
                "io.input_cctf_decoding",
                "io.crop",
                "io.crop_center",
                "io.crop_size",
                "io.upscale_factor",
            ] {
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
pub struct RegionDocument {
    pub id: String,
    pub label: String,
    pub role: String,
    pub geometry: Value,
    pub measurements: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseDocument {
    pub id: String,
    pub kind: String,
    pub input: Value,
    pub configuration: Value,
    pub seeds: Vec<u64>,
    pub regions: Vec<RegionDocument>,
    pub measurements: Vec<String>,
}
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
        {
            return Err(err(
                "unresolved_identifier",
                "policy selector does not resolve",
            ));
        }
    }
    Ok(())
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
    let reference = |item: &Value| -> Result<()> {
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
    };
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
                        reference(req(ro, k)?)?
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
pub fn geometry_mask(g: &Value, width: usize, height: usize, base: &Path) -> Result<Vec<bool>> {
    let count = width
        .checked_mul(height)
        .ok_or_else(|| err("invalid_mask", "image size overflow"))?;
    let g = g.get("mask").or_else(|| g.get("rectangle")).unwrap_or(g);
    if let Some(a) = g.as_array() {
        if a.len() != 4 || a.iter().any(|x| x.as_u64().is_none()) {
            return Err(err("invalid_bounds", "rectangle requires four integers"));
        }
        let x = a[0].as_u64().unwrap() as usize;
        let y = a[1].as_u64().unwrap() as usize;
        let w = a[2].as_u64().unwrap() as usize;
        let h = a[3].as_u64().unwrap() as usize;
        if w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|n| n > width)
            || y.checked_add(h).is_none_or(|n| n > height)
        {
            return Err(err("invalid_bounds", "geometry exceeds image"));
        }
        let mut m = vec![false; count];
        for yy in y..y + h {
            for xx in x..x + w {
                m[yy * width + xx] = true
            }
        }
        return Ok(m);
    }
    let o = obj(g)?;
    file_record(g, base)?;
    if o.get("width").and_then(Value::as_u64) != Some(width as u64)
        || o.get("height").and_then(Value::as_u64) != Some(height as u64)
    {
        return Err(err(
            "invalid_region",
            "declared mask dimensions differ from master",
        ));
    }
    let p = resolve_path(text(o, "path")?, base)?;
    let ext = p
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let values = match ext.as_str() {
        "npy" => {
            let (v, shape) = crate::image_data::read_npy(&p)?;
            if shape != [height, width] {
                return Err(err("invalid_region", "mask dimensions differ from master"));
            }
            v
        }
        "tif" | "tiff" => {
            let mut decoder =
                tiff::decoder::Decoder::new(std::io::BufReader::new(fs::File::open(&p)?))
                    .map_err(|e| err("invalid_region", e.to_string()))?;
            if decoder
                .dimensions()
                .map_err(|e| err("invalid_region", e.to_string()))?
                != (width as u32, height as u32)
                || !matches!(
                    decoder
                        .colortype()
                        .map_err(|e| err("invalid_region", e.to_string()))?,
                    tiff::ColorType::Gray(_)
                )
            {
                return Err(err(
                    "invalid_region",
                    "mask must be single-channel master-sized array",
                ));
            }
            use tiff::decoder::DecodingResult::*;
            match decoder
                .read_image()
                .map_err(|e| err("invalid_region", e.to_string()))?
            {
                U8(v) => v.into_iter().map(|x| x as f64).collect(),
                U16(v) => v.into_iter().map(|x| x as f64).collect(),
                U32(v) => v.into_iter().map(|x| x as f64).collect(),
                U64(v) => v.into_iter().map(|x| x as f64).collect(),
                I8(v) => v.into_iter().map(|x| x as f64).collect(),
                I16(v) => v.into_iter().map(|x| x as f64).collect(),
                I32(v) => v.into_iter().map(|x| x as f64).collect(),
                I64(v) => v.into_iter().map(|x| x as f64).collect(),
                F32(v) => v.into_iter().map(|x| x as f64).collect(),
                F64(v) => v,
                _ => return Err(err("invalid_region", "unsupported TIFF mask samples")),
            }
        }
        "png" => {
            let im = image::open(&p).map_err(|e| err("invalid_region", e.to_string()))?;
            if im.width() != width as u32 || im.height() != height as u32 {
                return Err(err("invalid_region", "mask dimensions differ from master"));
            }
            match im {
                image::DynamicImage::ImageLuma8(v) => {
                    v.into_raw().into_iter().map(f64::from).collect()
                }
                image::DynamicImage::ImageLuma16(v) => {
                    v.into_raw().into_iter().map(f64::from).collect()
                }
                _ => {
                    return Err(err(
                        "invalid_region",
                        "mask must be single-channel master-sized array",
                    ));
                }
            }
        }
        _ => return Err(err("invalid_region", "mask must be NPY, PNG, or TIFF")),
    };
    if values.len() != count || values.iter().any(|x| !x.is_finite()) {
        return Err(err(
            "invalid_region",
            "mask must be finite single-channel master-sized array",
        ));
    }
    let mask = values.into_iter().map(|x| x != 0.0).collect::<Vec<_>>();
    if !mask.iter().any(|x| *x) {
        return Err(err("invalid_region", "mask has no selected pixels"));
    }
    Ok(mask)
}
pub fn reference_geometry_mask(
    g: &Value,
    width: usize,
    height: usize,
    base: &Path,
) -> Result<Vec<bool>> {
    let g = g.get("rectangle").unwrap_or(g);
    if g.is_array() {
        return geometry_mask(g, width, height, base);
    }
    let o = obj(g).map_err(|_| err("invalid_region", "mask requires frozen rectangle or file"))?;
    let name = text(o, "path")?;
    let p = if Path::new(name).is_absolute() {
        PathBuf::from(name)
    } else {
        base.join(name)
    };
    let (v, shape) = if p
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| x.eq_ignore_ascii_case("npy"))
    {
        crate::image_data::read_npy(&p)?
    } else {
        let im = image::open(&p).map_err(|e| err("invalid_region", e.to_string()))?;
        let dims = vec![im.height() as usize, im.width() as usize];
        match im {
            image::DynamicImage::ImageLuma8(v) => {
                (v.into_raw().into_iter().map(f64::from).collect(), dims)
            }
            _ => {
                return Err(err(
                    "invalid_region",
                    "mask must be single-channel binary image",
                ));
            }
        }
    };
    if shape != [height, width]
        || ["width", "height"]
            .iter()
            .zip([width, height])
            .any(|(k, n)| o.get(*k).is_some_and(|x| x.as_u64() != Some(n as u64)))
    {
        return Err(err(
            "invalid_region",
            "mask dimensions differ from declared coordinate system",
        ));
    }
    if v.iter().any(|x| !x.is_finite()) {
        return Err(err("invalid_region", "mask contains invalid samples"));
    }
    if v.iter().any(|x| *x != 0.0 && *x != 1.0 && *x != 255.0) {
        return Err(err(
            "invalid_region",
            "mask must be binary; no implicit thresholding",
        ));
    }
    Ok(v.into_iter().map(|x| x != 0.0).collect())
}
