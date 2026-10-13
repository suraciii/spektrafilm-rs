use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub path: Option<String>,
    pub boundary: Option<String>,
}
impl Error {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path: None,
            boundary: None,
        }
    }
    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
    pub fn boundary(mut self, boundary: impl Into<String>) -> Self {
        self.boundary = Some(boundary.into());
        self
    }
    pub fn to_value(&self) -> Value {
        let mut value = json!({"code":self.code,"message":self.message});
        if let Some(path) = &self.path {
            value["path"] = json!(path);
        }
        if let Some(boundary) = &self.boundary {
            value["boundary"] = json!(boundary);
        }
        value
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::new("operation_failed", value.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::new("document_read", value.to_string())
    }
}

/// Python's finite JSON float spelling is part of the existing evidence hash contract.
fn python_float(value: f64) -> Result<String> {
    if !value.is_finite() {
        return Err(Error::new(
            "nonfinite_number",
            "JSON numbers must be finite",
        ));
    }
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format_finite(value);
    let (mantissa, exponent) = text
        .split_once('e')
        .map_or((text, 0), |(m, e)| (m, e.parse::<i32>().unwrap_or(0)));
    let negative = mantissa.starts_with('-');
    let unsigned = mantissa.trim_start_matches('-');
    let integer_digits = unsigned.split('.').next().unwrap_or(unsigned).len() as i32;
    let mut digits: String = unsigned.chars().filter(|c| *c != '.').collect();
    let leading = digits.bytes().take_while(|c| *c == b'0').count();
    if leading == digits.len() {
        return Ok(if negative { "-0.0" } else { "0.0" }.into());
    }
    digits.drain(..leading);
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let scientific_exponent = exponent + integer_digits - leading as i32 - 1;
    let sign = if negative { "-" } else { "" };
    if !(-4..16).contains(&scientific_exponent) {
        let mut body = digits[..1].to_owned();
        if digits.len() > 1 {
            body.push('.');
            body.push_str(&digits[1..]);
        }
        return Ok(format!(
            "{sign}{body}e{}{:02}",
            if scientific_exponent < 0 { "-" } else { "+" },
            scientific_exponent.abs()
        ));
    }
    let point = scientific_exponent + 1;
    let body = if point <= 0 {
        format!("0.{}{}", "0".repeat((-point) as usize), digits)
    } else if point as usize >= digits.len() {
        format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
    } else {
        format!(
            "{}.{}",
            &digits[..point as usize],
            &digits[point as usize..]
        )
    };
    Ok(format!("{sign}{body}"))
}
fn encode_json(value: &Value, out: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(v) => out.extend_from_slice(if *v { b"true" } else { b"false" }),
        Value::Number(v) => {
            let text = if v.is_f64() {
                python_float(
                    v.as_f64()
                        .ok_or_else(|| Error::new("nonfinite_number", "Invalid float"))?,
                )?
            } else {
                v.to_string()
            };
            out.extend_from_slice(text.as_bytes());
        }
        Value::String(v) => out.extend_from_slice(serde_json::to_string(v)?.as_bytes()),
        Value::Array(values) => {
            out.push(b'[');
            for (i, v) in values.iter().enumerate() {
                if i != 0 {
                    out.push(b',');
                }
                encode_json(v, out)?;
            }
            out.push(b']');
        }
        Value::Object(values) => {
            out.push(b'{');
            let mut keys: Vec<_> = values.keys().collect();
            keys.sort();
            for (i, key) in keys.into_iter().enumerate() {
                if i != 0 {
                    out.push(b',');
                }
                out.extend_from_slice(serde_json::to_string(key)?.as_bytes());
                out.push(b':');
                encode_json(&values[key], out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}
pub fn canonical_json(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    encode_json(value, &mut bytes)?;
    Ok(bytes)
}
pub fn digest(value: &Value) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(canonical_json(value)?)))
}
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut input = fs::File::open(path)
        .map_err(|e| Error::new("dependency_read", e.to_string()).at(path.display().to_string()))?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        let n = input.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn dump(path: &Path, value: &Value) -> Result<()> {
    let mut output = fs::File::create(path)?;
    output.write_all(&canonical_json(value)?)?;
    output.write_all(b"\n")?;
    Ok(())
}
pub fn destination_exists(path: &Path) -> Error {
    Error::new(
        "destination_exists",
        format!(
            "Destination already exists: {}; choose a new destination or remove the existing destination before retrying",
            path.display()
        ),
    )
    .at(path.display().to_string())
    .boundary("publication")
}

/// Report that an operation retained evidence but did not publish its terminal manifest.
///
/// A failed terminal manifest is a valid, published record and is deliberately not
/// described as incomplete. This helper is only used after a destination has been
/// created and an operation fails before its terminal manifest is written.
pub fn incomplete_destination(mut error: Error, root: &Path, manifest: &str) -> Error {
    if error.code == "destination_exists" {
        return error;
    }
    let terminal = root.join(manifest);
    if root.is_dir() && !terminal.is_file() {
        error.message = format!(
            "{}; destination {} is incomplete evidence because {} was not published; partial evidence was retained. Remove it or choose a new destination before retrying",
            error.message,
            root.display(),
            manifest,
        );
    }
    error
}

pub struct Progress {
    operation: &'static str,
    started: std::time::Instant,
    completed: usize,
    failed: usize,
    summarized: bool,
}

impl Progress {
    pub fn new(operation: &'static str) -> Self {
        Self {
            operation,
            started: std::time::Instant::now(),
            completed: 0,
            failed: 0,
            summarized: false,
        }
    }

    pub fn start(&self, case_id: &str, seed: u64) -> std::time::Instant {
        eprintln!("Starting {} {} seed {}", self.operation, case_id, seed);
        std::time::Instant::now()
    }

    pub fn finish(&mut self, case_id: &str, seed: u64, status: &str, started: std::time::Instant) {
        if matches!(status, "completed" | "validated") {
            self.completed += 1;
        } else {
            self.failed += 1;
        }
        eprintln!(
            "Completed {} {} seed {}: status={} elapsed_seconds={:.3}",
            self.operation,
            case_id,
            seed,
            status,
            started.elapsed().as_secs_f64()
        );
    }

    pub fn summary_rows(&mut self, rows: &[Value]) {
        self.completed = rows
            .iter()
            .filter(|row| matches!(row["status"].as_str(), Some("completed" | "validated")))
            .count();
        self.failed = rows
            .iter()
            .filter(|row| !matches!(row["status"].as_str(), Some("completed" | "validated")))
            .count();
        self.summary();
    }

    pub fn summary(&mut self) {
        if self.summarized {
            return;
        }
        self.summarized = true;
        eprintln!(
            "{} summary: completed={} failed={} elapsed_seconds={:.3}",
            self.operation,
            self.completed,
            self.failed,
            self.started.elapsed().as_secs_f64()
        );
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if !self.summarized && (self.completed != 0 || self.failed != 0) {
            self.summary();
        }
    }
}

pub fn create_destination(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        return Err(destination_exists(path));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(path).map_err(|e| {
        Error::new("publication_failure", e.to_string()).at(path.display().to_string())
    })?;
    Ok(())
}

pub struct Evidence {
    pub root: PathBuf,
    pub records: BTreeMap<String, Value>,
}
impl Evidence {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            records: BTreeMap::new(),
        }
    }
    pub fn add(
        &mut self,
        path: &Path,
        role: &str,
        media_type: &str,
        metadata: Value,
    ) -> Result<Value> {
        let canonical_root = self.root.canonicalize()?;
        let canonical_path = path.canonicalize()?;
        let relative = canonical_path
            .strip_prefix(&canonical_root)
            .map_err(|_| Error::new("unsafe_artifact_path", "Artifact is outside destination"))?
            .to_str()
            .ok_or_else(|| Error::new("invalid_path", "Artifact path must be UTF-8"))?
            .replace('\\', "/");
        crate::documents::safe_artifact_path(&self.root, &relative)?;
        let hash = sha256_file(path)?;
        if self
            .records
            .get(&relative)
            .is_some_and(|previous| previous["sha256"] != hash)
        {
            return Err(Error::new(
                "artifact_changed",
                format!("artifact changed: {relative}"),
            ));
        }
        let mut record = metadata.as_object().cloned().unwrap_or_default();
        record.insert("path".into(), json!(relative));
        record.insert("sha256".into(), json!(hash));
        record.insert("role".into(), json!(role));
        record.insert("media_type".into(), json!(media_type));
        let record = Value::Object(record);
        self.records.insert(relative, record.clone());
        Ok(record)
    }
    pub fn copy(
        &mut self,
        source: &Path,
        relative: &str,
        role: &str,
        metadata: Value,
    ) -> Result<Value> {
        let destination = crate::documents::safe_artifact_path(&self.root, relative)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let expected = sha256_file(source)?;
        fs::copy(source, &destination)?;
        if sha256_file(&destination)? != expected {
            return Err(Error::new(
                "artifact_changed",
                "Source changed while retaining evidence",
            )
            .at(source.display().to_string()));
        }
        let media_type = metadata
            .get("media_type")
            .and_then(Value::as_str)
            .unwrap_or_else(|| match destination.extension().and_then(|s| s.to_str()) {
                Some("json") => "application/json",
                Some("png") => "image/png",
                Some("tif" | "tiff") => "image/tiff",
                _ => "application/octet-stream",
            })
            .to_owned();
        self.add(&destination, role, &media_type, metadata)
    }
    pub fn values(&self) -> Vec<Value> {
        self.records.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_float_spellings_preserve_content_hashes() {
        let samples = [
            (1e-7, "1e-07"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (-0.0, "-0.0"),
            (1.25, "1.25"),
        ];
        for (value, expected) in samples {
            assert_eq!(python_float(value).unwrap(), expected);
        }
        assert_eq!(
            canonical_json(&json!({"z":1.0,"a":[1e-7,1]})).unwrap(),
            br#"{"a":[1e-07,1],"z":1.0}"#
        );
    }
}
