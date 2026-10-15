//! Offline visual report for explicitly prepared standardized chart images.
//!
//! The authored manifest declares every raster path together with an explicit
//! color source (`color_space` + `cctf_decoding`). This module validates the
//! declaration and its local dependencies, converts each source exactly once
//! into a passive 16-bit sRGB display PNG, and publishes a self-contained
//! `index.html` with a terminal `report.json` manifest. Display conversion is
//! visualization only: there is no simulation, normalization, remeasurement or
//! scoring, and the original rasters are never modified, so out-of-range
//! samples and source metadata survive untouched in the declared sources.

use crate::common::{self, Error, Result, create_destination, dump, sha256_file};
use crate::{documents, image_data};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const BODY: &str = include_str!("../assets/chart-report.html");
const CSS: &str = include_str!("../assets/chart-report.css");
const JS: &str = include_str!("../assets/chart-report.js");
const CSP: &str = "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src 'self'; connect-src 'none'; base-uri 'none'; form-action 'none'";
const WORK_DIR: &str = ".chart-report-work";

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
            "chart-report sources must be self-describing rasters (TIFF, PNG, NPY, JPEG, WebP); raw sidecar carriers are not supported",
        )
        .at(shown));
    }
    let metadata =
        fs::metadata(&path).map_err(|e| err("dependency_read", e.to_string()).at(shown.clone()))?;
    if !metadata.is_file() {
        return Err(err(
            "dependency_read",
            "chart-report source must be a regular file",
        )
        .at(shown));
    }
    Ok(path)
}

fn validate_manifest_value(value: &Value, manifest: &Path) -> Result<()> {
    documents::validate_shape("chart-report", value)?;
    let charts = value["charts"].as_array().unwrap();
    let images = value["images"].as_array().unwrap();
    unique_ids(charts, "chart")?;
    unique_ids(images, "image")?;
    let chart_ids: BTreeSet<&str> = charts
        .iter()
        .map(|chart| chart["id"].as_str().unwrap())
        .collect();
    let image_ids: BTreeSet<&str> = images
        .iter()
        .map(|image| image["id"].as_str().unwrap())
        .collect();
    for chart in charts {
        Grid::parse(&chart["grid"])?;
    }
    for image in images {
        let o = obj(image)?;
        let chart = text(o, "chart")?;
        if !chart_ids.contains(chart) {
            return Err(err(
                "unknown_chart",
                format!("image references unknown chart: {chart}"),
            ));
        }
        source_path(o, manifest)?;
    }
    let default_pair = obj(&value["default_pair"])?;
    for side in ["left", "right"] {
        let id = text(default_pair, side)?;
        if !image_ids.contains(id) {
            return Err(err(
                "unknown_image",
                format!("default_pair.{side} references unknown image: {id}"),
            ));
        }
    }
    Ok(())
}

/// Validate an authored chart manifest and its local dependencies without
/// decoding any image. The returned value is the authored document.
pub fn validate_manifest(path: &Path) -> Result<Value> {
    let value = documents::load_json_document(path)?;
    validate_manifest_value(&value, path)?;
    Ok(value)
}

fn effective_pair(
    value: &Value,
    left: Option<&str>,
    right: Option<&str>,
) -> Result<(String, String)> {
    match (left, right) {
        (None, None) => {
            let default_pair = obj(&value["default_pair"])?;
            Ok((
                text(default_pair, "left")?.to_owned(),
                text(default_pair, "right")?.to_owned(),
            ))
        }
        (Some(left), Some(right)) => {
            let image_ids: BTreeSet<&str> = value["images"]
                .as_array()
                .unwrap()
                .iter()
                .map(|image| image["id"].as_str().unwrap())
                .collect();
            for (side, id) in [("left", left), ("right", right)] {
                if !image_ids.contains(id) {
                    return Err(err(
                        "unknown_image",
                        format!("--{side} references unknown image: {id}"),
                    ));
                }
            }
            Ok((left.to_owned(), right.to_owned()))
        }
        _ => Err(err(
            "invalid_selector",
            "--left and --right must be provided together",
        )),
    }
}

fn snapshot_path(work: &Path, index: usize, source: &Path) -> PathBuf {
    match source.extension().and_then(|x| x.to_str()) {
        Some(ext) if !ext.is_empty() => work.join(format!("{index:04}.{ext}")),
        _ => work.join(format!("{index:04}")),
    }
}

/// Build the offline visual report and return the standard command result.
///
/// Manifest and dependency validation precede destination creation; decoding
/// and observed geometry are checked during generation. Later failures retain
/// incomplete evidence. The manifest and its sources are never modified.
pub fn build_report(
    manifest: &Path,
    out: &Path,
    left: Option<&str>,
    right: Option<&str>,
) -> Result<Value> {
    let root = documents::normalize_path(out)?;
    if fs::symlink_metadata(out).is_ok() {
        return Err(common::destination_exists(out));
    }
    build_report_impl(manifest, out, left, right, &root)
        .map_err(|error| common::incomplete_destination(error, &root, "report.json"))
}

fn build_report_impl(
    manifest: &Path,
    out: &Path,
    left: Option<&str>,
    right: Option<&str>,
    root: &Path,
) -> Result<Value> {
    let manifest_path = documents::normalize_path(manifest)?;
    let manifest_sha256 = sha256_file(&manifest_path)?;
    let authored = validate_manifest(manifest)?;
    if sha256_file(&manifest_path)? != manifest_sha256 {
        return Err(err(
            "hash_mismatch",
            "chart manifest changed while being read",
        ));
    }
    let (left, right) = effective_pair(&authored, left, right)?;
    create_destination(out)?;
    let work = out.join(WORK_DIR);
    fs::create_dir_all(&work)?;
    fs::create_dir_all(out.join("images"))?;

    let charts = authored["charts"].as_array().unwrap();
    let images = authored["images"].as_array().unwrap();
    let mut grids = BTreeMap::new();
    for chart in charts {
        let o = obj(chart)?;
        grids.insert(text(o, "id")?, Grid::parse(&chart["grid"])?);
    }

    let mut chart_layouts: BTreeMap<&str, [usize; 2]> = BTreeMap::new();
    let mut artifacts = Vec::new();
    let mut rendered = Vec::new();
    for (index, value) in images.iter().enumerate() {
        let o = obj(value)?;
        let id = text(o, "id")?;
        let chart = text(o, "chart")?;
        let grid = grids.get(chart).unwrap();
        let source = source_path(o, &manifest_path)?;
        let source_error = |mut error: Error| {
            error.message = format!("image {id}: {}", error.message);
            error.at(source.display().to_string())
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
        let layout = [image.width, image.height];
        if let Some(expected) = chart_layouts.get(chart) {
            if *expected != layout {
                return Err(err(
                    "invalid_image_layout",
                    format!("images sharing chart {chart} must have identical dimensions"),
                )
                .at(id));
            }
        } else {
            chart_layouts.insert(chart, layout);
        }
        let extent = grid.extent()?;
        if extent[0] > image.width as u64 || extent[1] > image.height as u64 {
            return Err(err(
                "invalid_bounds",
                format!("grid windows for chart {chart} leave the decoded image"),
            )
            .at(id));
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
            "chart": chart,
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
    let chart_count = charts.len();
    let payload = json!({
        "schema_version": 1,
        "title": text(obj(&authored)?, "title")?,
        "description": authored["description"].as_str().unwrap_or(""),
        "charts": charts,
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
        "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\"><title>SpektraFilm 色卡视觉报告</title><style>{CSS}</style>{BODY}<script type=\"application/json\" id=\"chart-report-data\">{embedded}</script><script src=\"viewer.js\"></script></body></html>",
    );
    fs::write(out.join("index.html"), html)?;
    if sha256_file(&manifest_path)? != manifest_sha256 {
        return Err(err(
            "hash_mismatch",
            "chart manifest changed during publication",
        ));
    }
    dump(&out.join("report.json"), &payload)?;
    Ok(json!({
        "schema_version": 1,
        "command": "chart-report",
        "execution_status": "completed",
        "artifact": root.join("index.html"),
        "manifest": root.join("report.json"),
        "image_count": image_count,
        "chart_count": chart_count,
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

    fn chart(id: &str, rows: u64, columns: u64, patch: [u64; 2], gap: [u64; 2]) -> String {
        format!(
            r#"{{"id":"{id}","label":"图 {id}","grid":{{"rows":{rows},"columns":{columns},"origin":[0,0],"patch_size":[{},{}],"gap":[{},{}]}}}}"#,
            patch[0], patch[1], gap[0], gap[1]
        )
    }

    fn image(id: &str, chart: &str, path: &str, space: &str) -> String {
        format!(
            r#"{{"id":"{id}","chart":"{chart}","path":"{path}","label":"输入 {id}","color_space":"{space}","cctf_decoding":false}}"#
        )
    }

    fn document(charts: &[String], images: &[String], left: &str, right: &str) -> String {
        format!(
            r#"{{"schema_version":1,"title":"色卡检查","charts":[{}],"images":[{}],"default_pair":{{"left":"{left}","right":"{right}"}}}}"#,
            charts.join(","),
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
                &[chart("c", 1, 1, [8, 8], [0, 0])],
                &[
                    image("valid", "c", "valid.png", "sRGB"),
                    image("broken", "c", "broken.png", "sRGB"),
                ],
                "valid",
                "broken",
            ),
        );
        let out = dir.path().join("out");
        let error = build_report(&authored, &out, None, None).unwrap_err();
        assert_eq!(error.path.as_deref(), broken.to_str());
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
                &[chart("c", 4294967296, 1, [8, 4294967296], [0, 0])],
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
                &[chart("c", 1, 3, [8, 8], [0, 0])],
                &[
                    image("a", "c", "a.png", "sRGB"),
                    image("b", "c", "a.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let out = dir.path().join("out");
        let error = build_report(&escaping, &out, None, None).unwrap_err();
        assert_eq!(error.code, "invalid_bounds");
        assert!(!out.join("report.json").exists());
    }

    #[test]
    fn defaults_and_overrides_reference_images() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        png(&dir.path().join("b.png"), 8, 8);
        let charts = [chart("c", 1, 1, [8, 8], [0, 0])];
        let images = [
            image("a", "c", "a.png", "sRGB"),
            image("b", "c", "b.png", "sRGB"),
        ];
        let unknown_default = manifest(dir.path(), &document(&charts, &images, "a", "missing"));
        let error = validate_manifest(&unknown_default).unwrap_err();
        assert_eq!(error.code, "unknown_image");

        let valid = manifest(dir.path(), &document(&charts, &images, "a", "b"));
        let unpaired = dir.path().join("unpaired");
        let error = build_report(&valid, &unpaired, Some("b"), None).unwrap_err();
        assert_eq!(error.code, "invalid_selector");
        assert!(!unpaired.exists());

        let unknown_override = dir.path().join("unknown-override");
        let error =
            build_report(&valid, &unknown_override, Some("a"), Some("missing")).unwrap_err();
        assert_eq!(error.code, "unknown_image");
        assert!(!unknown_override.exists());
    }

    #[test]
    fn same_chart_requires_identical_layouts() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        png(&dir.path().join("b.png"), 4, 8);
        let shared = manifest(
            dir.path(),
            &document(
                &[chart("c", 1, 1, [8, 8], [0, 0])],
                &[
                    image("a", "c", "a.png", "sRGB"),
                    image("b", "c", "b.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let out = dir.path().join("out");
        let error = build_report(&shared, &out, None, None).unwrap_err();
        assert_eq!(error.code, "invalid_image_layout");
        assert!(!out.join("report.json").exists());

        let distinct = manifest(
            dir.path(),
            &document(
                &[
                    chart("c1", 1, 1, [8, 8], [0, 0]),
                    chart("c2", 2, 1, [4, 4], [0, 0]),
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
        build_report(&distinct, &out, None, None).unwrap();
        let payload: Value =
            serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        assert_eq!(payload["images"][0]["width"], 8);
        assert_eq!(payload["images"][1]["width"], 4);
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
                    chart("c1", 1, 1, [1, 1], [0, 0]),
                    chart("c2", 1, 1, [1, 1], [0, 0]),
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
        build_report(&manifest, &out, None, None).unwrap();
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
            r#"{{"schema_version":1,"title":"{hostile}","description":"a<b","charts":[{}],"images":[{{"id":"a","chart":"c","path":"a.png","label":"{hostile}","description":"x<y","color_space":"sRGB","cctf_decoding":false}},{{"id":"b","chart":"c","path":"a.png","label":"右","color_space":"sRGB","cctf_decoding":false}}],"default_pair":{{"left":"a","right":"b"}}}}"#,
            chart("c", 1, 1, [8, 8], [0, 0]),
        );
        let manifest = manifest(dir.path(), &body);
        let out = dir.path().join("out");
        build_report(&manifest, &out, None, None).unwrap();

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
                &[chart("c", 1, 1, [8, 8], [0, 0])],
                &[
                    image("a", "c", "a.png", "sRGB"),
                    image("b", "c", "a.png", "sRGB"),
                ],
                "a",
                "b",
            ),
        );
        let clean = dir.path().join("clean");
        build_report(&manifest, &clean, None, None).unwrap();
        fs::write(
            dir.path().join("a.png.json"),
            r#"{"color_space":"Display P3","transfer":"srgb","icc":{"path":"missing.icc"}}"#,
        )
        .unwrap();
        let with_sidecar = dir.path().join("with-sidecar");
        build_report(&manifest, &with_sidecar, None, None).unwrap();
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
                &[chart("c", 1, 1, [8, 8], [0, 0])],
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
        let error = build_report(&manifest, &out, None, None).unwrap_err();
        assert_eq!(error.code, "destination_exists");
        assert_eq!(fs::read_to_string(out.join("keep.txt")).unwrap(), "keep");
        assert!(!out.join("report.json").exists());

        #[cfg(unix)]
        {
            let target = dir.path().join("target");
            fs::create_dir(&target).unwrap();
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            let error = build_report(&manifest, &link, None, None).unwrap_err();
            assert_eq!(error.code, "destination_exists");
            assert!(!target.join("report.json").exists());
        }
    }
}
