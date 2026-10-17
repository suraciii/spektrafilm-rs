//! Offline visual review for explicitly prepared image sets.
//!
//! The authored manifest declares sets of prepared images — a shared patch
//! grid or explicit per-image region rectangles — together with every raster
//! path and an explicit color source (`color_space` + `cctf_decoding`). This
//! module validates the declaration and its local dependencies, converts each
//! source exactly once into a passive 16-bit sRGB display PNG, and publishes a
//! self-contained `index.html` with a terminal `report.json` manifest.
//! Display conversion is visualization only: there is no simulation,
//! normalization, remeasurement or scoring, and the original rasters are never
//! modified, so out-of-range samples and source metadata survive untouched in
//! the declared sources.

use crate::common::{self, Error, Result, create_destination, dump, sha256_file};
use crate::{documents, image_data};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const BODY: &str = include_str!("../assets/review.html");
const CSS: &str = include_str!("../assets/review.css");
const JS: &str = include_str!("../assets/review.js");
const CSP: &str = "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src 'self'; connect-src 'none'; base-uri 'none'; form-action 'none'";
const WORK_DIR: &str = ".review-work";

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
fn count(o: &Map<String, Value>, k: &str) -> Result<u64> {
    req(o, k)?
        .as_u64()
        .ok_or_else(|| err("invalid_bounds", format!("{k} must be a 64-bit integer")))
}
fn pair(o: &Map<String, Value>, k: &str) -> Result<[u64; 2]> {
    let a = req(o, k)?
        .as_array()
        .ok_or_else(|| err("schema_validation", format!("{k} must be array")))?;
    if a.len() != 2 {
        return Err(err(
            "schema_validation",
            format!("{k} must hold two values"),
        ));
    }
    let mut out = [0; 2];
    for (slot, value) in out.iter_mut().zip(a) {
        *slot = value
            .as_u64()
            .ok_or_else(|| err("invalid_bounds", format!("{k} must hold 64-bit integers")))?;
    }
    Ok(out)
}
/// Parse one declared region rectangle `[x, y, width, height]`. Width and
/// height must be positive so every window is a nonempty half-open rectangle.
fn rect(v: &Value) -> Result<[u64; 4]> {
    let a = v
        .as_array()
        .ok_or_else(|| err("invalid_bounds", "region rectangle must be an array"))?;
    if a.len() != 4 {
        return Err(err(
            "invalid_bounds",
            "region rectangle must hold four integers",
        ));
    }
    let mut out = [0; 4];
    for (slot, value) in out.iter_mut().zip(a) {
        *slot = value.as_u64().ok_or_else(|| {
            err(
                "invalid_bounds",
                "region rectangle must hold 64-bit integers",
            )
        })?;
    }
    if out[2] == 0 || out[3] == 0 {
        return Err(err(
            "invalid_bounds",
            "region width and height must be positive",
        ));
    }
    Ok(out)
}
/// Half-open rectangle intersection; saturated ends keep hostile declarations
/// from wrapping.
fn overlap(a: [u64; 4], b: [u64; 4]) -> bool {
    a[0] < b[0].saturating_add(b[2])
        && b[0] < a[0].saturating_add(a[2])
        && a[1] < b[1].saturating_add(b[3])
        && b[1] < a[1].saturating_add(a[3])
}

/// Spreadsheet-style column labels: `A` through `Z`, then `AA` and subsequent
/// names, matching the review page's patch labels.
fn column_label(column: u64) -> String {
    let mut label = String::new();
    let mut n = column;
    while n > 0 {
        let rest = (n - 1) % 26;
        label.insert(0, char::from(b'A' + rest as u8));
        n = (n - 1) / 26;
    }
    label
}

/// Declared patch window geometry. Windows are laid out row-major from the
/// origin with the declared cell size and gap; all arithmetic stays checked
/// because oversized declarations must fail instead of wrapping.
struct Grid {
    rows: u64,
    columns: u64,
    origin: [u64; 2],
    patch_size: [u64; 2],
    gap: [u64; 2],
}

impl Grid {
    fn parse(v: &Value) -> Result<Self> {
        let o = obj(v)?;
        let grid = Self {
            rows: count(o, "rows")?,
            columns: count(o, "columns")?,
            origin: pair(o, "origin")?,
            patch_size: pair(o, "patch_size")?,
            gap: pair(o, "gap")?,
        };
        grid.extent()?;
        Ok(grid)
    }
    fn span(count: u64, patch: u64, gap: u64) -> Option<u64> {
        count
            .checked_mul(patch)?
            .checked_add(count.checked_sub(1)?.checked_mul(gap)?)
    }
    fn position(origin: u64, index: u64, size: u64, gap: u64) -> Option<u64> {
        index
            .checked_mul(size.checked_add(gap)?)?
            .checked_add(origin)
    }
    /// Row-major patch windows with spreadsheet-style labels and checked
    /// arithmetic; the review page derives the same windows from the same
    /// declaration.
    fn windows(&self) -> Result<Vec<(String, [u64; 4])>> {
        self.extent()?;
        let overflow = || {
            err(
                "invalid_bounds",
                "declared grid windows overflow 64-bit geometry",
            )
        };
        let mut windows = Vec::new();
        for row in 1..=self.rows {
            for column in 1..=self.columns {
                let x = Self::position(self.origin[0], column - 1, self.patch_size[0], self.gap[0])
                    .ok_or_else(overflow)?;
                let y = Self::position(self.origin[1], row - 1, self.patch_size[1], self.gap[1])
                    .ok_or_else(overflow)?;
                windows.push((
                    format!("{}{row}", column_label(column)),
                    [x, y, self.patch_size[0], self.patch_size[1]],
                ));
            }
        }
        Ok(windows)
    }
    /// `[width, height]` spanned by every declared patch window including the
    /// origin offset; `None` means the declared grid overflows 64-bit geometry.
    fn extent(&self) -> Result<[u64; 2]> {
        let width = Self::span(self.columns, self.patch_size[0], self.gap[0])
            .and_then(|v| v.checked_add(self.origin[0]));
        let height = Self::span(self.rows, self.patch_size[1], self.gap[1])
            .and_then(|v| v.checked_add(self.origin[1]));
        match (width, height) {
            (Some(width), Some(height)) => Ok([width, height]),
            _ => Err(err(
                "invalid_bounds",
                "declared grid windows overflow 64-bit geometry",
            )),
        }
    }
}

fn unique_ids(items: &[Value], kind: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for item in items {
        let id = text(obj(item)?, "id")?;
        if !seen.insert(id) {
            return Err(err("duplicate_id", format!("duplicate {kind} id: {id}")));
        }
    }
    Ok(())
}

/// Resolve a self-describing raster without consulting adjacent metadata.
/// Generation decodes an isolated snapshot so only the manifest can declare
/// its color interpretation. Missing sources fail without decoding anything.
fn source_path(image: &Map<String, Value>, manifest: &Path) -> Result<PathBuf> {
    let path = documents::resolve_path(text(image, "path")?, manifest)?;
    let shown = path.display().to_string();
    if matches!(
        path.extension()
            .and_then(|x| x.to_str())
            .map(|x| x.to_ascii_lowercase())
            .as_deref(),
        Some("raw") | Some("bin") | Some("f32") | Some("f64")
    ) {
        return Err(err(
            "source_declaration",
            "review sources must be self-describing rasters (TIFF, PNG, NPY, JPEG, WebP); raw sidecar carriers are not supported",
        )
        .file(shown));
    }
    let metadata = fs::metadata(&path)
        .map_err(|e| err("dependency_read", e.to_string()).file(shown.clone()))?;
    if !metadata.is_file() {
        return Err(err("dependency_read", "review source must be a regular file").file(shown));
    }
    Ok(path)
}

/// Check the optional measurement intent of one set: only grid sets may declare
/// it, its role and measurement families must be known names, and a declared
/// sample window must fit inside the declared patch window.
fn validate_measurement(o: &Map<String, Value>, grid: &Option<Grid>, set_id: &str) -> Result<()> {
    let Some(measurement) = o.get("measurement") else {
        return Ok(());
    };
    let Some(grid) = grid else {
        return Err(err(
            "measurement_requires_grid",
            format!(
                "set {set_id} declares explicit regions and a measurement object; measurement intent requires a grid set"
            ),
        ));
    };
    let measurement = obj(measurement)?;
    if let Some(role) = measurement.get("role").and_then(Value::as_str) {
        let roles = documents::region_roles()?;
        if !roles.iter().any(|name| name == role) {
            return Err(err(
                "invalid_role",
                format!(
                    "unknown region role: {role}; available: {}",
                    roles.join(", ")
                ),
            ));
        }
    }
    if let Some(choices) = measurement.get("measurements").and_then(Value::as_array) {
        let families = documents::measurement_families()?;
        for choice in choices.iter().filter_map(Value::as_str) {
            if !families.iter().any(|name| name == choice) {
                return Err(err(
                    "invalid_measurements",
                    format!(
                        "unknown measurement family: {choice}; available: {}",
                        families.join(", ")
                    ),
                ));
            }
        }
    }
    if measurement.get("sample").is_some() {
        let sample = pair(measurement, "sample")?;
        if sample[0] > grid.patch_size[0] || sample[1] > grid.patch_size[1] {
            return Err(err(
                "invalid_sample",
                format!(
                    "sample window {}×{} does not fit the declared patch window {}×{}",
                    sample[0], sample[1], grid.patch_size[0], grid.patch_size[1]
                ),
            ));
        }
    }
    Ok(())
}

fn validate_manifest_value(value: &Value, manifest: &Path) -> Result<()> {
    documents::validate_shape("review", value)?;
    let sets = value["sets"].as_array().unwrap();
    let images = value["images"].as_array().unwrap();
    unique_ids(sets, "set")?;
    unique_ids(images, "image")?;
    let set_ids: BTreeSet<&str> = sets.iter().map(|set| set["id"].as_str().unwrap()).collect();
    let mut set_of_image: BTreeMap<&str, &str> = BTreeMap::new();
    for image in images {
        let o = obj(image)?;
        let set = text(o, "set")?;
        if !set_ids.contains(set) {
            return Err(err(
                "unknown_set",
                format!("image references unknown set: {set}"),
            ));
        }
        set_of_image.insert(text(o, "id")?, set);
        source_path(o, manifest)?;
    }
    for set in sets {
        let o = obj(set)?;
        let set_id = text(o, "id")?;
        let grid = o.get("grid").map(Grid::parse).transpose()?;
        validate_measurement(o, &grid, set_id)?;
        let Some(regions) = o.get("regions").and_then(Value::as_array) else {
            continue;
        };
        unique_ids(regions, "region")?;
        let mut by_image: BTreeMap<&str, Vec<(&str, [u64; 4])>> = BTreeMap::new();
        for region in regions {
            let region = obj(region)?;
            let region_id = text(region, "id")?;
            let geometry = obj(req(region, "geometry")?)?;
            for (image_id, rectangle) in geometry {
                if set_of_image.get(image_id.as_str()) != Some(&set_id) {
                    return Err(err(
                        "unresolved_region_key",
                        format!(
                            "region {region_id} references an image outside set {set_id}: {image_id}"
                        ),
                    ));
                }
                by_image
                    .entry(image_id)
                    .or_default()
                    .push((region_id, rect(rectangle)?));
            }
        }
        for (image_id, rectangles) in &by_image {
            for (index, (left_id, left)) in rectangles.iter().enumerate() {
                for (right_id, right) in &rectangles[index + 1..] {
                    if overlap(*left, *right) {
                        return Err(err(
                            "overlapping_regions",
                            format!("regions {left_id} and {right_id} overlap on image {image_id}"),
                        ));
                    }
                }
            }
        }
    }
    let default_pair = obj(&value["default_pair"])?;
    for side in ["left", "right"] {
        let id = text(default_pair, side)?;
        if !set_of_image.contains_key(id) {
            return Err(err(
                "unknown_image",
                format!("default_pair.{side} references unknown image: {id}"),
            ));
        }
    }
    Ok(())
}

/// Validate an authored review manifest and its local dependencies without
/// decoding any image. The returned value is the authored document.
pub fn validate_manifest(path: &Path) -> Result<Value> {
    let value = documents::load_json_document(path)?;
    validate_manifest_value(&value, path)?;
    Ok(value)
}

fn snapshot_path(work: &Path, index: usize, source: &Path) -> PathBuf {
    match source.extension().and_then(|x| x.to_str()) {
        Some(ext) if !ext.is_empty() => work.join(format!("{index:04}.{ext}")),
        _ => work.join(format!("{index:04}")),
    }
}

/// Build the offline visual review and return the standard command result.
///
/// Manifest and dependency validation precede destination creation; decoding
/// and observed geometry are checked during generation. Later failures retain
/// incomplete evidence. The manifest and its sources are never modified.
pub fn build_report(manifest: &Path, out: &Path) -> Result<Value> {
    let root = documents::normalize_path(out)?;
    if fs::symlink_metadata(out).is_ok() {
        return Err(common::destination_exists(out));
    }
    build_report_impl(manifest, out, &root)
        .map_err(|error| common::incomplete_destination(error, &root, "report.json"))
}

/// Rectangles declared per (set, image), keyed by the set's region ids.
type RegionBounds<'a> = BTreeMap<(&'a str, &'a str), Vec<(&'a str, [u64; 4])>>;

fn build_report_impl(manifest: &Path, out: &Path, root: &Path) -> Result<Value> {
    let manifest_path = documents::normalize_path(manifest)?;
    let manifest_sha256 = sha256_file(&manifest_path)?;
    let authored = validate_manifest(manifest)?;
    if sha256_file(&manifest_path)? != manifest_sha256 {
        return Err(err(
            "hash_mismatch",
            "review manifest changed while being read",
        ));
    }
    let default_pair = obj(&authored["default_pair"])?;
    let (left, right) = (
        text(default_pair, "left")?.to_owned(),
        text(default_pair, "right")?.to_owned(),
    );
    create_destination(out)?;
    let work = out.join(WORK_DIR);
    fs::create_dir_all(&work)?;
    fs::create_dir_all(out.join("images"))?;

    let sets = authored["sets"].as_array().unwrap();
    let images = authored["images"].as_array().unwrap();
    let mut grids = BTreeMap::new();
    let mut region_bounds: RegionBounds = BTreeMap::new();
    for set in sets {
        let o = obj(set)?;
        let set_id = text(o, "id")?;
        if let Some(grid) = o.get("grid") {
            grids.insert(set_id, Grid::parse(grid)?);
        }
        for region in o
            .get("regions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let region = obj(region)?;
            let region_id = text(region, "id")?;
            let geometry = obj(req(region, "geometry")?)?;
            for (image_id, rectangle) in geometry {
                region_bounds
                    .entry((set_id, image_id.as_str()))
                    .or_default()
                    .push((region_id, rect(rectangle)?));
            }
        }
    }

    let mut grid_layouts: BTreeMap<&str, [usize; 2]> = BTreeMap::new();
    let mut artifacts = Vec::new();
    let mut rendered = Vec::new();
    for (index, value) in images.iter().enumerate() {
        let o = obj(value)?;
        let id = text(o, "id")?;
        let set = text(o, "set")?;
        let source = source_path(o, &manifest_path)?;
        let source_error = |mut error: Error| {
            error.message = format!("image {id}: {}", error.message);
            error.file(source.display().to_string())
        };
        // Load only an isolated snapshot with the declared color source so a
        // sidecar appearing next to the source can never be consumed here.
        let snapshot = snapshot_path(&work, index, &source);
        fs::copy(&source, &snapshot)?;
        let source_sha256 = sha256_file(&snapshot)?;
        let image = image_data::load_image(&json!({
            "path": snapshot.display().to_string(),
            "color_space": text(o, "color_space")?,
            "cctf_decoding": o["cctf_decoding"].clone(),
        }))
        .map_err(source_error)?;
        if let Some(grid) = grids.get(set) {
            let layout = [image.width, image.height];
            if let Some(expected) = grid_layouts.get(set) {
                if *expected != layout {
                    return Err(err(
                        "invalid_image_layout",
                        format!("images sharing grid set {set} must have identical dimensions"),
                    )
                    .boundary(id));
                }
            } else {
                grid_layouts.insert(set, layout);
            }
            let extent = grid.extent()?;
            if extent[0] > image.width as u64 || extent[1] > image.height as u64 {
                return Err(err(
                    "invalid_bounds",
                    format!("grid windows for set {set} leave the decoded image"),
                )
                .boundary(id));
            }
        } else if let Some(regions) = region_bounds.get(&(set, id)) {
            for (region_id, rectangle) in regions {
                if rectangle[0].saturating_add(rectangle[2]) > image.width as u64
                    || rectangle[1].saturating_add(rectangle[3]) > image.height as u64
                {
                    return Err(err(
                        "invalid_bounds",
                        format!("region {region_id} of set {set} leaves the decoded image"),
                    )
                    .boundary(id));
                }
            }
        }
        let display = image_data::display_rgb(&image).map_err(source_error)?;
        let relative = format!("images/{index:04}.png");
        image_data::write_png16(&out.join(&relative), &display, image.width, image.height)?;
        artifacts.push(json!({
            "path": relative,
            "sha256": sha256_file(&out.join(&relative))?,
            "media_type": "image/png",
            "role": "display_image",
            "availability": "available",
            "dimensions": [image.width, image.height],
        }));
        rendered.push(json!({
            "id": id,
            "set": set,
            "label": text(o, "label")?,
            "description": o.get("description").and_then(Value::as_str).unwrap_or(""),
            "color_space": text(o, "color_space")?,
            "cctf_decoding": o["cctf_decoding"].clone(),
            "width": image.width,
            "height": image.height,
            "src": relative,
            "source": {"path": source.display().to_string(), "sha256": source_sha256},
            "display": {"carrier": "png", "bit_depth": 16, "color_space": "sRGB", "transfer": "srgb"},
        }));
    }
    fs::remove_dir_all(&work)?;
    fs::write(out.join("viewer.js"), JS)?;
    artifacts.push(json!({
        "path": "viewer.js",
        "sha256": sha256_file(&out.join("viewer.js"))?,
        "media_type": "text/javascript",
        "role": "viewer_script",
        "availability": "available",
    }));

    let image_count = rendered.len();
    let set_count = sets.len();
    let payload = json!({
        "schema_version": 1,
        "title": text(obj(&authored)?, "title")?,
        "description": authored["description"].as_str().unwrap_or(""),
        "sets": sets,
        "images": rendered,
        "default_pair": {"left": left, "right": right},
        "source_manifest": {
            "path": manifest_path.display().to_string(),
            "sha256": manifest_sha256,
        },
        "artifacts": artifacts,
    });
    let data = String::from_utf8(common::canonical_json(&payload)?)
        .map_err(|e| err("document_read", e.to_string()))?;
    let embedded = data.replace('<', "\\u003c");
    let html = format!(
        "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\"><title>SpektraFilm 图像对照检查报告</title><style>{CSS}</style>{BODY}<script type=\"application/json\" id=\"review-data\">{embedded}</script><script src=\"viewer.js\"></script></body></html>",
    );
    fs::write(out.join("index.html"), html)?;
    if sha256_file(&manifest_path)? != manifest_sha256 {
        return Err(err(
            "hash_mismatch",
            "review manifest changed during publication",
        ));
    }
    dump(&out.join("report.json"), &payload)?;
    Ok(json!({
        "schema_version": 1,
        "command": "report",
        "source": "manifest",
        "execution_status": "completed",
        "artifact": root.join("index.html"),
        "manifest": root.join("report.json"),
        "image_count": image_count,
        "set_count": set_count,
    }))
}

/// Decoded dimensions of every image in one set. Each source is loaded from an
/// isolated snapshot so sidecars next to it can never be consumed here.
fn set_dimensions(
    authored: &Value,
    set_id: &str,
    manifest: &Path,
) -> Result<Vec<(String, [usize; 2])>> {
    let images = authored["images"].as_array().unwrap();
    let work =
        std::env::temp_dir().join(format!("spektrafilm-review-regions-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work)?;
    let result = (|| -> Result<Vec<(String, [usize; 2])>> {
        let mut dimensions = Vec::new();
        for (index, value) in images.iter().enumerate() {
            let o = obj(value)?;
            if text(o, "set")? != set_id {
                continue;
            }
            let id = text(o, "id")?.to_owned();
            let source = source_path(o, manifest)?;
            let snapshot = snapshot_path(&work, index, &source);
            fs::copy(&source, &snapshot)?;
            let image = image_data::load_image(&json!({
                "path": snapshot.display().to_string(),
                "color_space": text(o, "color_space")?,
                "cctf_decoding": o["cctf_decoding"].clone(),
            }))
            .map_err(|mut error: Error| {
                error.message = format!("image {id}: {}", error.message);
                error.file(source.display().to_string())
            })?;
            dimensions.push((id, [image.width, image.height]));
        }
        Ok(dimensions)
    })();
    let _ = fs::remove_dir_all(&work);
    result
}

/// The set's declared measurement intent with its documented defaults: the
/// whole patch window, role `other`, and no measurement families.
fn measurement_intent(set: &Value) -> Result<(Option<[u64; 2]>, String, Vec<String>)> {
    let Some(measurement) = set.get("measurement") else {
        return Ok((None, "other".to_owned(), Vec::new()));
    };
    let measurement = obj(measurement)?;
    let sample = match measurement.get("sample") {
        Some(_) => Some(pair(measurement, "sample")?),
        None => None,
    };
    let role = measurement
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("other")
        .to_owned();
    let measurements = measurement
        .get("measurements")
        .and_then(Value::as_array)
        .map(|choices| {
            choices
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok((sample, role, measurements))
}

/// Derive a measurement region array from one grid set of a review manifest.
///
/// The command reads the manifest, the set's declared measurement intent, and
/// the set's decoded image dimensions; it never renders and never computes
/// measurements. Windows reuse the review page's declared arithmetic, keep the
/// patch names, and are validated as the existing `regions` kind before the
/// destination is created.
pub fn derive_regions(manifest: &Path, set_id: &str, out: &Path) -> Result<Value> {
    let root = documents::normalize_path(out)?;
    if fs::symlink_metadata(out).is_ok() {
        return Err(common::destination_exists(out));
    }
    let manifest_path = documents::normalize_path(manifest)?;
    let authored = validate_manifest(&manifest_path)?;
    let sets = authored["sets"].as_array().unwrap();
    let set = sets
        .iter()
        .find(|set| set["id"].as_str() == Some(set_id))
        .ok_or_else(|| {
            let available: Vec<&str> = sets.iter().filter_map(|set| set["id"].as_str()).collect();
            err(
                "unknown_set",
                format!("unknown set: {set_id}; available: {}", available.join(", ")),
            )
        })?;
    let Some(grid) = set.get("grid") else {
        return Err(err(
            "missing_grid",
            format!("set {set_id} declares no grid; regions are derived from grid sets only"),
        ));
    };
    let windows = Grid::parse(grid)?.windows()?;
    let (sample, role, measurements) = measurement_intent(set)?;
    let dimensions = set_dimensions(&authored, set_id, &manifest_path)?;
    let mut regions = Vec::new();
    for (label, patch) in windows {
        let window = match sample {
            None => patch,
            Some([width, height]) => [
                patch[0] + (patch[2] - width) / 2,
                patch[1] + (patch[3] - height) / 2,
                width,
                height,
            ],
        };
        for (image_id, [width, height]) in &dimensions {
            if window[0].saturating_add(window[2]) > *width as u64
                || window[1].saturating_add(window[3]) > *height as u64
            {
                return Err(err(
                    "invalid_bounds",
                    format!("window {label} leaves the decoded image {image_id}"),
                ));
            }
        }
        regions.push(json!({
            "id": label,
            "label": label,
            "role": &role,
            "geometry": {
                "x": window[0],
                "y": window[1],
                "width": window[2],
                "height": window[3],
                // The grid's arithmetic is in decoded image pixels
                // (specs/instrument/spec.md: the review grid's own contract
                // states "All geometry uses decoded image pixels"), and the
                // document this derivation writes must be usable as a sampling
                // `regions` value, which reads the source image. The space is
                // therefore `source`, never `render`.
                "space": "source",
            },
            "measurements": &measurements,
        }));
    }
    // The derived file is a regions document, so it carries the same envelope
    // a case's `regions` element and a standalone document carry: one kind,
    // one schema version, one identity, one nonempty regions array.
    let value = json!({
        "schema_version": 1,
        "id": format!("{set_id}-regions"),
        "regions": regions,
    });
    documents::validate_shape("regions", &value)?;
    documents::validate_local_dependencies("regions", &value, &root)?;
    let bytes = serde_json::to_vec_pretty(&value)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                common::destination_exists(out)
            } else {
                error.into()
            }
        })?;
    let persisted = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        let stored = documents::load_json_value(out)?;
        documents::validate_shape("regions", &stored)?;
        Ok(())
    })();
    if let Err(error) = persisted {
        drop(file);
        fs::remove_file(out)?;
        return Err(error);
    }
    let region_count = value["regions"].as_array().map(Vec::len).unwrap_or(0);
    Ok(json!({
        "schema_version": 1,
        "command": "regions",
        "execution_status": "completed",
        "artifact": root,
        "set": set_id,
        "region_count": region_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("manifest.json");
        fs::write(&path, body).unwrap();
        path
    }

    fn grid_set(id: &str, rows: u64, columns: u64, patch: [u64; 2], gap: [u64; 2]) -> String {
        format!(
            r#"{{"id":"{id}","label":"集合 {id}","grid":{{"rows":{rows},"columns":{columns},"origin":[0,0],"patch_size":[{},{}],"gap":[{},{}]}}}}"#,
            patch[0], patch[1], gap[0], gap[1]
        )
    }

    fn region_set(id: &str, regions: &str) -> String {
        format!(r#"{{"id":"{id}","label":"集合 {id}","regions":[{regions}]}}"#)
    }

    fn region(id: &str, image: &str, rectangle: [u64; 4]) -> String {
        format!(
            r#"{{"id":"{id}","label":"区域 {id}","geometry":{{"{image}":[{},{},{},{}]}}}}"#,
            rectangle[0], rectangle[1], rectangle[2], rectangle[3]
        )
    }

    fn image(id: &str, set: &str, path: &str, space: &str) -> String {
        format!(
            r#"{{"id":"{id}","set":"{set}","path":"{path}","label":"输入 {id}","color_space":"{space}","cctf_decoding":false}}"#
        )
    }

    fn document(sets: &[String], images: &[String], left: &str, right: &str) -> String {
        format!(
            r#"{{"schema_version":1,"title":"图像对照","sets":[{}],"images":[{}],"default_pair":{{"left":"{left}","right":"{right}"}}}}"#,
            sets.join(","),
            images.join(",")
        )
    }

    fn png(path: &Path, width: usize, height: usize) {
        image_data::write_png16_codes(path, &vec![[0x8000; 3]; width * height], width, height)
            .unwrap();
    }

    #[test]
    fn failed_decode_identifies_the_authored_source() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("valid.png"), 8, 8);
        let broken = dir.path().join("broken.png");
        fs::write(&broken, b"not a PNG").unwrap();
        let authored = manifest(
            dir.path(),
            &document(
                &[grid_set("c", 1, 1, [8, 8], [0, 0])],
                &[
                    image("valid", "c", "valid.png", "sRGB"),
                    image("broken", "c", "broken.png", "sRGB"),
                ],
                "valid",
                "broken",
            ),
        );
        let out = dir.path().join("out");
        let error = build_report(&authored, &out).unwrap_err();
        // `resolve_path` canonicalizes the source, so on macOS the temp dir
        // arrives as `/private/var/...` rather than the `/var/...` symlink.
        let expected = broken.canonicalize().unwrap();
        assert_eq!(error.to_value()["file"], json!(expected.to_str()));
        assert!(error.message.contains("broken"));
        assert!(!out.join("report.json").exists());
    }

    #[test]
    fn grid_arithmetic_and_window_bounds_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 16, 8);
        let overflowing = manifest(
            dir.path(),
            &document(
                &[grid_set("c", 4294967296, 1, [8, 4294967296], [0, 0])],
                &[
                    image("a", "c", "a.png", "sRGB"),
                    image("b", "c", "a.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let error = validate_manifest(&overflowing).unwrap_err();
        assert_eq!(error.code, "invalid_bounds");

        let escaping = manifest(
            dir.path(),
            &document(
                &[grid_set("c", 1, 3, [8, 8], [0, 0])],
                &[
                    image("a", "c", "a.png", "sRGB"),
                    image("b", "c", "a.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let out = dir.path().join("out");
        let error = build_report(&escaping, &out).unwrap_err();
        assert_eq!(error.code, "invalid_bounds");
        assert!(!out.join("report.json").exists());
    }

    #[test]
    fn measurement_intent_requires_a_grid_and_known_names() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 16, 8);
        let images = [
            image("a", "c", "a.png", "sRGB"),
            image("b", "c", "a.png", "sRGB"),
        ];
        let grid = |measurement: &str| {
            format!(
                r#"{{"id":"c","label":"集合 c","grid":{{"rows":1,"columns":2,"origin":[0,0],"patch_size":[16,8],"gap":[0,0]}},"measurement":{measurement}}}"#
            )
        };
        let declared = manifest(
            dir.path(),
            &document(
                &[grid(
                    r#"{"sample":[8,8],"role":"color","measurements":["color","tone"]}"#,
                )],
                &images,
                "a",
                "b",
            ),
        );
        validate_manifest(&declared).unwrap();

        for (measurement, expected) in [
            (r#"{"sample":[32,8]}"#, "invalid_sample"),
            (r#"{"sample":[8,0]}"#, "invalid_bounds"),
            (r#"{"role":"bogus"}"#, "invalid_role"),
            (r#"{"measurements":["bogus"]}"#, "invalid_measurements"),
        ] {
            let path = manifest(
                dir.path(),
                &document(&[grid(measurement)], &images, "a", "b"),
            );
            let error = validate_manifest(&path).unwrap_err();
            assert_eq!(error.code, expected, "{measurement}");
        }

        let regions = r#"{"id":"c","label":"集合 c","regions":[{"id":"r1","label":"区域","geometry":{"a":[0,0,4,4]}}],"measurement":{"role":"other"}}"#.to_owned();
        let path = manifest(dir.path(), &document(&[regions], &images, "a", "b"));
        let error = validate_manifest(&path).unwrap_err();
        assert_eq!(error.code, "measurement_requires_grid");
    }

    #[test]
    fn default_pair_must_reference_declared_images() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        png(&dir.path().join("b.png"), 8, 8);
        let sets = [grid_set("c", 1, 1, [8, 8], [0, 0])];
        let images = [
            image("a", "c", "a.png", "sRGB"),
            image("b", "c", "b.png", "sRGB"),
        ];
        let unknown_default = manifest(dir.path(), &document(&sets, &images, "a", "missing"));
        let error = validate_manifest(&unknown_default).unwrap_err();
        assert_eq!(error.code, "unknown_image");

        let valid = manifest(dir.path(), &document(&sets, &images, "a", "b"));
        let out = dir.path().join("out");
        let result = build_report(&valid, &out).unwrap();
        assert_eq!(result["command"], "report");
        assert_eq!(result["source"], "manifest");
        assert!(out.join("index.html").is_file());
    }

    #[test]
    fn grid_sets_require_identical_layouts_but_regions_sets_do_not() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        png(&dir.path().join("b.png"), 4, 8);
        let shared = manifest(
            dir.path(),
            &document(
                &[grid_set("c", 1, 1, [8, 8], [0, 0])],
                &[
                    image("a", "c", "a.png", "sRGB"),
                    image("b", "c", "b.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let out = dir.path().join("out");
        let error = build_report(&shared, &out).unwrap_err();
        assert_eq!(error.code, "invalid_image_layout");
        assert!(!out.join("report.json").exists());

        let distinct = manifest(
            dir.path(),
            &document(
                &[
                    grid_set("c1", 1, 1, [8, 8], [0, 0]),
                    grid_set("c2", 2, 1, [4, 4], [0, 0]),
                ],
                &[
                    image("a", "c1", "a.png", "sRGB"),
                    image("b", "c2", "b.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let out = dir.path().join("distinct");
        build_report(&distinct, &out).unwrap();
        let payload: Value =
            serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        assert_eq!(payload["images"][0]["width"], 8);
        assert_eq!(payload["images"][1]["width"], 4);
        assert_eq!(payload["sets"][0]["id"], "c1");
        assert_eq!(payload["images"][0]["set"], "c1");

        let regions = manifest(
            dir.path(),
            &document(
                &[region_set(
                    "r",
                    &format!(
                        "{},{}",
                        region("a", "a", [0, 0, 4, 4]),
                        region("b", "b", [0, 0, 4, 4])
                    ),
                )],
                &[
                    image("a", "r", "a.png", "sRGB"),
                    image("b", "r", "b.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let out = dir.path().join("regions");
        build_report(&regions, &out).unwrap();
        let payload: Value =
            serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        assert_eq!(payload["images"][0]["width"], 8);
        assert_eq!(payload["images"][1]["width"], 4);
    }

    #[test]
    fn regions_sets_validate_geometry_and_identities() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        png(&dir.path().join("b.png"), 8, 8);
        let images = [
            image("a", "r", "a.png", "sRGB"),
            image("b", "r", "b.png", "sRGB"),
        ];

        // A region may cover a subset of its set's images, and region
        // identities are scoped to their set.
        let subset = manifest(
            dir.path(),
            &document(
                &[
                    region_set("r", &region("only-a", "a", [0, 0, 4, 4])),
                    region_set("r2", &region("only-a", "b", [0, 0, 4, 4])),
                ],
                &[
                    image("a", "r", "a.png", "sRGB"),
                    image("b", "r2", "b.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        validate_manifest(&subset).unwrap();

        let duplicate = manifest(
            dir.path(),
            &document(
                &[region_set(
                    "r",
                    &format!(
                        "{},{}",
                        region("same", "a", [0, 0, 4, 4]),
                        region("same", "b", [0, 0, 4, 4])
                    ),
                )],
                &images,
                "a",
                "b",
            ),
        );
        assert_eq!(
            validate_manifest(&duplicate).unwrap_err().code,
            "duplicate_id"
        );

        let unresolved = manifest(
            dir.path(),
            &document(
                &[region_set("r", &region("a", "other", [0, 0, 4, 4]))],
                &images,
                "a",
                "b",
            ),
        );
        let error = validate_manifest(&unresolved).unwrap_err();
        assert_eq!(error.code, "unresolved_region_key");
        assert!(error.message.contains("other"));

        let overlapping = manifest(
            dir.path(),
            &document(
                &[region_set(
                    "r",
                    &format!(
                        "{},{}",
                        region("a", "a", [0, 0, 4, 4]),
                        region("b", "a", [3, 3, 4, 4])
                    ),
                )],
                &images,
                "a",
                "b",
            ),
        );
        assert_eq!(
            validate_manifest(&overlapping).unwrap_err().code,
            "overlapping_regions"
        );

        let flat = manifest(
            dir.path(),
            &document(
                &[region_set("r", &region("a", "a", [0, 0, 0, 4]))],
                &images,
                "a",
                "b",
            ),
        );
        assert_eq!(validate_manifest(&flat).unwrap_err().code, "invalid_bounds");

        for set in [
            format!(
                r#"{{"id":"r","label":"两种形态","grid":{{"rows":1,"columns":1,"origin":[0,0],"patch_size":[4,4],"gap":[0,0]}},"regions":[{region}]}}"#,
                region = region("a", "a", [0, 0, 4, 4])
            ),
            r#"{"id":"r","label":"没有形态"}"#.to_owned(),
        ] {
            let malformed = manifest(dir.path(), &document(&[set], &images, "a", "b"));
            assert_eq!(
                validate_manifest(&malformed).unwrap_err().code,
                "schema_validation"
            );
        }
    }

    #[test]
    fn regions_bounds_are_checked_during_generation() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        png(&dir.path().join("b.png"), 8, 8);
        let escaping = manifest(
            dir.path(),
            &document(
                &[region_set("r", &region("a", "a", [4, 0, 5, 4]))],
                &[
                    image("a", "r", "a.png", "sRGB"),
                    image("b", "r", "b.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        validate_manifest(&escaping).unwrap();
        let out = dir.path().join("out");
        let error = build_report(&escaping, &out).unwrap_err();
        assert_eq!(error.code, "invalid_bounds");
        assert!(error.message.contains("region a"));
        assert!(!out.join("report.json").exists());
    }

    #[test]
    fn chart_report_v1_manifests_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let v1 = format!(
            r#"{{"schema_version":1,"title":"旧清单","charts":[{}],"images":[{{"id":"a","chart":"c","path":"a.png","label":"输入 a","color_space":"sRGB","cctf_decoding":false}}],"default_pair":{{"left":"a","right":"a"}}}}"#,
            grid_set("c", 1, 1, [8, 8], [0, 0]),
        );
        let authored = manifest(dir.path(), &v1);
        let error = validate_manifest(&authored).unwrap_err();
        assert!(
            matches!(error.code.as_str(), "missing_field" | "unknown_field"),
            "unexpected code: {}",
            error.code
        );
    }

    #[test]
    fn source_identity_and_original_rasters_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let out_of_range = dir.path().join("wide.npy");
        image_data::write_npy(&out_of_range, &[-0.25, 0.5, 2.0, 1.0, 1.0, 1.0], &[2, 1, 3])
            .unwrap();
        let manifest = manifest(
            dir.path(),
            &document(
                &[
                    grid_set("c1", 1, 1, [1, 1], [0, 0]),
                    grid_set("c2", 1, 1, [1, 1], [0, 0]),
                ],
                &[
                    image("a", "c1", "a.png", "sRGB"),
                    image("b", "c2", "wide.npy", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let before = sha256_file(&out_of_range).unwrap();
        let out = dir.path().join("out");
        build_report(&manifest, &out).unwrap();
        assert_eq!(before, sha256_file(&out_of_range).unwrap());

        let payload: Value =
            serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        let wide = &payload["images"][1];
        let source = out_of_range.canonicalize().unwrap();
        assert_eq!(wide["source"]["path"], source.display().to_string());
        assert_eq!(wide["source"]["sha256"], before);
        assert_eq!(wide["width"], 1);
        assert_eq!(wide["height"], 2);
    }

    #[test]
    fn markup_metadata_cannot_break_out_of_embedded_data() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let hostile = "</script><img src=x onerror=alert(1)>";
        let body = format!(
            r#"{{"schema_version":1,"title":"{hostile}","description":"a<b","sets":[{}],"images":[{{"id":"a","set":"c","path":"a.png","label":"{hostile}","description":"x<y","color_space":"sRGB","cctf_decoding":false}},{{"id":"b","set":"c","path":"a.png","label":"右","color_space":"sRGB","cctf_decoding":false}}],"default_pair":{{"left":"a","right":"b"}}}}"#,
            grid_set("c", 1, 1, [8, 8], [0, 0]),
        );
        let manifest = manifest(dir.path(), &body);
        let out = dir.path().join("out");
        build_report(&manifest, &out).unwrap();

        let html = fs::read_to_string(out.join("index.html")).unwrap();
        assert!(!html.contains(hostile));
        assert!(html.contains(r#"\u003c/script>"#));
        let payload: Value =
            serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        assert_eq!(payload["title"], hostile);
        assert_eq!(payload["images"][0]["label"], hostile);
    }

    #[test]
    fn adjacent_metadata_cannot_override_declared_color() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let manifest = manifest(
            dir.path(),
            &document(
                &[grid_set("c", 1, 1, [8, 8], [0, 0])],
                &[
                    image("a", "c", "a.png", "sRGB"),
                    image("b", "c", "a.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let clean = dir.path().join("clean");
        build_report(&manifest, &clean).unwrap();
        fs::write(
            dir.path().join("a.png.json"),
            r#"{"color_space":"Display P3","transfer":"srgb","icc":{"path":"missing.icc"}}"#,
        )
        .unwrap();
        let with_sidecar = dir.path().join("with-sidecar");
        build_report(&manifest, &with_sidecar).unwrap();
        assert_eq!(
            fs::read(clean.join("images/0000.png")).unwrap(),
            fs::read(with_sidecar.join("images/0000.png")).unwrap()
        );
    }

    #[test]
    fn existing_destinations_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let manifest = manifest(
            dir.path(),
            &document(
                &[grid_set("c", 1, 1, [8, 8], [0, 0])],
                &[
                    image("a", "c", "a.png", "sRGB"),
                    image("b", "c", "a.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let out = dir.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("keep.txt"), "keep").unwrap();
        let error = build_report(&manifest, &out).unwrap_err();
        assert_eq!(error.code, "destination_exists");
        assert_eq!(fs::read_to_string(out.join("keep.txt")).unwrap(), "keep");
        assert!(!out.join("report.json").exists());

        #[cfg(unix)]
        {
            let target = dir.path().join("target");
            fs::create_dir(&target).unwrap();
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            let error = build_report(&manifest, &link).unwrap_err();
            assert_eq!(error.code, "destination_exists");
            assert!(!target.join("report.json").exists());
        }
    }
}
