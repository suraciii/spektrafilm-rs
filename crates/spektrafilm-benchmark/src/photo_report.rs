//! Benchmark-owned offline photo review report.
//!
//! One authored photo manifest becomes one static report page plus a terminal
//! `report.json`. The manifest declares photos, optional rectangular ROIs, and
//! (for three or more photos) the comparison pairs the report may show. The
//! generator validates the declaration strictly, decodes each source exactly
//! once from an isolated snapshot with the manifest's explicit color
//! declaration, publishes one 16-bit sRGB display PNG per photo, and embeds a
//! sanitized payload for the offline viewer.
//!
//! This module performs no measurement, scoring, ranking, registration,
//! exposure or color matching. Display conversion is visualization only.

use crate::common::{self, Result, dump, sha256_file};
use crate::documents::{self, err, obj, text};
use crate::image_data;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const BODY: &str = include_str!("../assets/photo-report.html");
const CSS: &str = include_str!("../assets/photo-report.css");
const JS: &str = include_str!("../assets/photo-report.js");
const CSP: &str = "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src 'self'; connect-src 'none'; base-uri 'none'; form-action 'none'";
const WORK_DIR: &str = ".photo-report-work";
/// The closed color-space vocabulary the manifest may declare. Anything else
/// is an `unsupported_color_space` failure rather than a silent default.
const COLOR_SPACES: [&str; 8] = [
    "sRGB",
    "Display P3",
    "DCI-P3",
    "Adobe RGB (1998)",
    "ProPhoto RGB",
    "ITU-R BT.2020",
    "ACEScg",
    "ACES2065-1",
];

/// Authored photo manifest, `schema_version: 1`. `deny_unknown_fields` on every
/// record makes an unknown authored field a validation failure.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhotoManifest {
    schema_version: u8,
    title: String,
    description: Option<String>,
    photos: Vec<Photo>,
    comparisons: Option<Vec<ComparisonRequest>>,
}

/// One authored photo and its display metadata.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Photo {
    id: String,
    path: String,
    label: String,
    note: Option<String>,
    color_space: Option<String>,
    cctf_decoding: Option<bool>,
    rois: Option<Vec<PhotoRoi>>,
}

/// One authored rectangular region of interest in native pixel coordinates.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhotoRoi {
    id: String,
    label: String,
    rect: [u64; 4],
    note: Option<String>,
}

/// One authored comparison request. Two photos get one derived request in
/// manifest order; three or more use the declared requests verbatim.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComparisonRequest {
    id: Option<String>,
    left: String,
    right: String,
    label: Option<String>,
    note: Option<String>,
}

/// A resolved comparison pair with its ROI union resolved on both sides.
#[derive(Debug, Serialize)]
struct ResolvedComparison {
    id: String,
    left: String,
    right: String,
    label: Option<String>,
    note: Option<String>,
    rois: Vec<ResolvedRoi>,
}

/// One ROI identity in a pair, resolved against both photos.
#[derive(Debug, Serialize)]
struct ResolvedRoi {
    id: String,
    left: RoiSide,
    right: RoiSide,
}

/// One side of a resolved ROI: the authored rectangle on that photo, or an
/// explicit unavailable state naming the photo that lacks the ROI identity.
/// An unavailable side is never omitted and never serialized as an empty
/// rectangle.
#[derive(Debug, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum RoiSide {
    Available {
        rect: [u64; 4],
        dimensions: [usize; 2],
        display_artifact: String,
    },
    Unavailable {
        reason: String,
    },
}

/// One decoded photo with its authored, observed, and generated records.
struct DecodedPhoto {
    id: String,
    label: String,
    note: Option<String>,
    color_space: String,
    cctf_decoding: bool,
    width: usize,
    height: usize,
    rois: Vec<PhotoRoi>,
    source_path: String,
    source_sha256: String,
    display_path: String,
    display_sha256: String,
}

/// Report mode derived from the authored photo count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    SinglePhoto,
    TwoPhoto,
    RequestedComparisons,
}
impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::SinglePhoto => "single_photo",
            Mode::TwoPhoto => "two_photo",
            Mode::RequestedComparisons => "requested_comparisons",
        }
    }
}

/// Stable identities: an initial alphanumeric character followed by
/// alphanumerics, `_`, `.`, or `-`. DOM order, vector index, and generated
/// filename never become identity.
fn identifier(value: &str, kind: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if !valid {
        return Err(err(
            "invalid_identifier",
            format!("invalid {kind} id: {value}"),
        ));
    }
    Ok(())
}

/// Parse the manifest strictly and check the declaration-level rules the
/// generator owns: unique photo and per-photo ROI identities, nonempty labels,
/// the closed color-space vocabulary, and positive ROI rectangles.
fn parse_manifest(value: &Value) -> Result<PhotoManifest> {
    let manifest: PhotoManifest = serde_json::from_value(value.clone())
        .map_err(|e| err("schema_validation", e.to_string()))?;
    if manifest.schema_version != 1 {
        return Err(err(
            "unsupported_schema",
            "photo manifest requires schema_version 1",
        ));
    }
    if manifest.title.trim().is_empty() {
        return Err(err("schema_validation", "title must be nonempty"));
    }
    if manifest.photos.is_empty() {
        return Err(err(
            "invalid_document",
            "photo manifest must declare at least one photo",
        ));
    }
    let mut seen = BTreeSet::new();
    for photo in &manifest.photos {
        identifier(&photo.id, "photo")?;
        if !seen.insert(photo.id.clone()) {
            return Err(err(
                "duplicate_id",
                format!("duplicate photo id: {}", photo.id),
            ));
        }
        if photo.label.trim().is_empty() {
            return Err(err(
                "schema_validation",
                format!("photo {} label must be nonempty", photo.id),
            ));
        }
        if let Some(space) = &photo.color_space
            && !COLOR_SPACES.contains(&space.as_str())
        {
            return Err(err(
                "unsupported_color_space",
                format!(
                    "photo {} declares unsupported color_space: {space}; supported values are {}",
                    photo.id,
                    COLOR_SPACES.join(", ")
                ),
            ));
        }
        let mut roi_ids = BTreeSet::new();
        for roi in photo.rois.as_deref().unwrap_or_default() {
            identifier(&roi.id, "ROI")?;
            if !roi_ids.insert(roi.id.clone()) {
                return Err(err(
                    "duplicate_id",
                    format!("photo {} declares duplicate ROI id: {}", photo.id, roi.id),
                ));
            }
            if roi.label.trim().is_empty() {
                return Err(err(
                    "schema_validation",
                    format!("photo {} ROI {} label must be nonempty", photo.id, roi.id),
                ));
            }
            if roi.rect[2] == 0 || roi.rect[3] == 0 {
                return Err(err(
                    "schema_validation",
                    format!(
                        "photo {} ROI {} rect width and height must be positive",
                        photo.id, roi.id
                    ),
                ));
            }
        }
    }
    Ok(manifest)
}

/// A supported raster the manifest may name: self-describing containers that
/// `image_data::load_image` decodes without a sidecar. Raw float carriers are
/// rejected because the file itself does not carry their interpretation.
fn supported_source(path: &Path, shown: &str) -> Result<()> {
    let carrier = path
        .extension()
        .and_then(|x| x.to_str())
        .map(|x| x.to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(carrier.as_str(), "raw" | "bin" | "f32" | "f64") {
        return Err(err(
            "source_declaration",
            "photo sources must be self-describing rasters (TIFF, PNG, NPY, JPEG, WebP); raw sidecar carriers are not supported",
        )
        .file(shown));
    }
    let metadata = fs::metadata(path)
        .map_err(|e| err("dependency_read", e.to_string()).file(shown.to_owned()))?;
    if !metadata.is_file() {
        return Err(
            err("dependency_read", "photo source must be a regular file").file(shown.to_owned()),
        );
    }
    Ok(())
}

/// Resolve one authored path relative to the manifest, rejecting absolute and
/// parent-traversing paths, and confirm it names a supported regular raster.
fn source_path(photo: &Map<String, Value>, manifest: &Path, id: &str) -> Result<PathBuf> {
    let value = text(photo, "path").map_err(|_| {
        err(
            "schema_validation",
            format!("photo {id} path must be a string"),
        )
    })?;
    let path = documents::resolve_relative_asset_path(value, manifest)
        .map_err(|e| e.file(format!("{id}: path")))?;
    let shown = path.display().to_string();
    supported_source(&path, &shown)?;
    Ok(path)
}

/// Mode selection and the pair list the report may show.
///
/// One photo is a single-photo report; two photos form one fixed pair in
/// manifest order; three or more require a nonempty explicit `comparisons`
/// array and show only those declared pairs. No pair is ever inferred.
fn resolve_mode(manifest: &PhotoManifest) -> Result<(Mode, Vec<ComparisonRequest>)> {
    match manifest.photos.len() {
        1 => {
            if manifest.comparisons.is_some() {
                return Err(err(
                    "invalid_comparison",
                    "a single-photo manifest must not declare comparisons",
                ));
            }
            Ok((Mode::SinglePhoto, vec![]))
        }
        2 => {
            if manifest.comparisons.is_some() {
                return Err(err(
                    "invalid_comparison",
                    "a two-photo manifest must not declare comparisons; the pair is fixed in manifest order",
                ));
            }
            let left = manifest.photos[0].id.clone();
            let right = manifest.photos[1].id.clone();
            Ok((
                Mode::TwoPhoto,
                vec![ComparisonRequest {
                    id: Some(comparison_id(&left, &right)),
                    left,
                    right,
                    label: None,
                    note: None,
                }],
            ))
        }
        _ => {
            let comparisons = manifest.comparisons.as_ref().ok_or_else(|| {
                err(
                    "missing_comparisons",
                    format!(
                        "{} photos require a nonempty comparisons array",
                        manifest.photos.len()
                    ),
                )
            })?;
            if comparisons.is_empty() {
                return Err(err(
                    "missing_comparisons",
                    "comparisons must declare at least one pair",
                ));
            }
            let known: BTreeSet<&str> = manifest.photos.iter().map(|p| p.id.as_str()).collect();
            let mut seen = BTreeSet::new();
            let mut resolved = Vec::new();
            for (index, request) in comparisons.iter().enumerate() {
                if !known.contains(request.left.as_str()) {
                    return Err(err(
                        "unknown_photo",
                        format!(
                            "comparison {index} names unknown left photo: {}",
                            request.left
                        ),
                    ));
                }
                if !known.contains(request.right.as_str()) {
                    return Err(err(
                        "unknown_photo",
                        format!(
                            "comparison {index} names unknown right photo: {}",
                            request.right
                        ),
                    ));
                }
                if request.left == request.right {
                    return Err(err(
                        "self_comparison",
                        format!(
                            "comparison {index} compares photo {} with itself",
                            request.left
                        ),
                    ));
                }
                let id = match &request.id {
                    Some(id) => {
                        identifier(id, "comparison")?;
                        id.clone()
                    }
                    None => comparison_id(&request.left, &request.right),
                };
                if !seen.insert(id.clone()) {
                    return Err(err(
                        "duplicate_id",
                        format!("duplicate comparison id: {id}"),
                    ));
                }
                resolved.push(ComparisonRequest {
                    id: Some(id),
                    left: request.left.clone(),
                    right: request.right.clone(),
                    label: request.label.clone(),
                    note: request.note.clone(),
                });
            }
            Ok((Mode::RequestedComparisons, resolved))
        }
    }
}

/// Stable identity for a comparison whose `id` the manifest omits: the ordered
/// pair, joined without changing left/right order.
fn comparison_id(left: &str, right: &str) -> String {
    format!("{left}__{right}")
}

/// Resolve the ROI union of one pair: the left photo's ROI order, then the
/// right photo's ROI IDs not already listed. Identity is the only pairing
/// authority; coordinates, labels, and dimensions are never compared.
fn resolve_pair(request: &ComparisonRequest, photos: &[DecodedPhoto]) -> ResolvedComparison {
    let left = photos.iter().find(|p| p.id == request.left);
    let right = photos.iter().find(|p| p.id == request.right);
    let (Some(left), Some(right)) = (left, right) else {
        return ResolvedComparison {
            id: request.id.clone().unwrap_or_default(),
            left: request.left.clone(),
            right: request.right.clone(),
            label: request.label.clone(),
            note: request.note.clone(),
            rois: vec![],
        };
    };
    let mut order: Vec<&str> = left.rois.iter().map(|r| r.id.as_str()).collect();
    for roi in &right.rois {
        if !order.contains(&roi.id.as_str()) {
            order.push(roi.id.as_str());
        }
    }
    let side = |roi: Option<&PhotoRoi>, photo: &DecodedPhoto| match roi {
        Some(roi) => RoiSide::Available {
            rect: roi.rect,
            dimensions: [roi.rect[2] as usize, roi.rect[3] as usize],
            display_artifact: photo.display_path.clone(),
        },
        None => RoiSide::Unavailable {
            reason: format!("photo {} does not declare ROI", photo.id),
        },
    };
    let rois = order
        .into_iter()
        .map(|id| ResolvedRoi {
            id: id.to_owned(),
            left: side(left.rois.iter().find(|r| r.id == id), left),
            right: side(right.rois.iter().find(|r| r.id == id), right),
        })
        .collect();
    ResolvedComparison {
        id: request.id.clone().unwrap_or_default(),
        left: request.left.clone(),
        right: request.right.clone(),
        label: request.label.clone(),
        note: request.note.clone(),
        rois,
    }
}

/// The isolated snapshot name for one photo, derived from its authored
/// position so a snapshot can never collide with a neighboring source file.
fn snapshot_path(work: &Path, index: usize, source: &Path) -> PathBuf {
    match source.extension().and_then(|x| x.to_str()) {
        Some(ext) if !ext.is_empty() => work.join(format!("{index:04}.{ext}")),
        _ => work.join(format!("{index:04}")),
    }
}

/// Snapshot, decode, and convert one photo. The snapshot is an isolated copy so
/// decoding can never consume a neighboring sidecar, and its hash is compared
/// with the authored path's hash before anything is published.
fn decode_photo(
    photo: &Photo,
    source: &Path,
    snapshot: &Path,
    display_relative: &str,
    display_path: &Path,
) -> Result<DecodedPhoto> {
    fs::copy(source, snapshot)?;
    let source_sha256 = sha256_file(snapshot)?;
    if sha256_file(source)? != source_sha256 {
        return Err(err(
            "hash_mismatch",
            format!("photo {} source changed while being read", photo.id),
        ));
    }
    let color_space = photo
        .color_space
        .clone()
        .unwrap_or_else(|| "sRGB".to_owned());
    let cctf_decoding = photo.cctf_decoding.unwrap_or(true);
    let image = image_data::load_image(&json!({
        "path": snapshot.display().to_string(),
        "color_space": color_space,
        "cctf_decoding": cctf_decoding,
    }))
    .map_err(|mut error| {
        error.message = format!("photo {}: {}", photo.id, error.message);
        error.file(source.display().to_string())
    })?;
    let display = image_data::display_rgb(&image).map_err(|mut error| {
        error.message = format!("photo {}: {}", photo.id, error.message);
        error.file(source.display().to_string())
    })?;
    image_data::write_png16(display_path, &display, image.width, image.height)?;
    Ok(DecodedPhoto {
        id: photo.id.clone(),
        label: photo.label.clone(),
        note: photo.note.clone(),
        color_space,
        cctf_decoding,
        width: image.width,
        height: image.height,
        rois: photo.rois.clone().unwrap_or_default(),
        source_path: photo.path.clone(),
        source_sha256,
        display_path: display_relative.to_owned(),
        display_sha256: sha256_file(display_path)?,
    })
}

/// Each ROI must lie fully inside its decoded photo; the failure names the
/// photo and the ROI.
fn validate_roi_bounds(photo: &DecodedPhoto) -> Result<()> {
    for roi in &photo.rois {
        if roi.rect[0].saturating_add(roi.rect[2]) > photo.width as u64
            || roi.rect[1].saturating_add(roi.rect[3]) > photo.height as u64
        {
            return Err(err(
                "invalid_bounds",
                format!(
                    "photo {} ROI {} leaves the decoded image ({}x{})",
                    photo.id, roi.id, photo.width, photo.height
                ),
            )
            .boundary(photo.id.clone()));
        }
    }
    Ok(())
}

/// Compare every authored path against the hash observed from its snapshot. A
/// source that changed during generation fails before publication can claim a
/// completed report.
fn verify_source_hashes(sources: &[PathBuf], observed: &[String]) -> Result<()> {
    for (source, expected) in sources.iter().zip(observed) {
        if sha256_file(source)? != *expected {
            return Err(err(
                "hash_mismatch",
                format!(
                    "photo source changed during generation: {}",
                    source.display()
                ),
            ));
        }
    }
    Ok(())
}

/// The sanitized payload shared by `report.json` and the embedded page data.
/// Authored text stays plain text; the viewer renders it, never executes it.
fn payload(
    manifest: &PhotoManifest,
    mode: Mode,
    photos: &[DecodedPhoto],
    comparisons: &[ResolvedComparison],
    manifest_path: &Path,
    manifest_sha256: &str,
) -> Value {
    let photo_records: Vec<Value> = photos
        .iter()
        .map(|photo| {
            json!({
                "id": photo.id,
                "label": photo.label,
                "note": photo.note,
                "src": photo.display_path,
                "width": photo.width,
                "height": photo.height,
                "rois": photo.rois.iter().map(|roi| json!({
                    "id": roi.id,
                    "label": roi.label,
                    "rect": roi.rect,
                    "note": roi.note,
                })).collect::<Vec<Value>>(),
                "source": {"path": photo.source_path, "sha256": photo.source_sha256},
                "color": {
                    "color_space": photo.color_space,
                    "cctf_decoding": photo.cctf_decoding,
                },
                "display": {
                    "path": photo.display_path,
                    "sha256": photo.display_sha256,
                    "carrier": "png",
                    "bit_depth": 16,
                    "color_space": "sRGB",
                    "transfer": "srgb",
                    "resampling": "none",
                    "quantization": "16-bit rounded; no dither",
                },
            })
        })
        .collect();
    let comparison_records: Vec<Value> = comparisons
        .iter()
        .map(|comparison| {
            json!({
                "id": comparison.id,
                "left": comparison.left,
                "right": comparison.right,
                "label": comparison.label,
                "note": comparison.note,
                "rois": comparison.rois.iter().map(|roi| json!({
                    "id": roi.id,
                    "left": roi.left,
                    "right": roi.right,
                })).collect::<Vec<Value>>(),
            })
        })
        .collect();
    json!({
        "schema_version": 1,
        "command": "report photos",
        "source": "photo_manifest",
        "mode": mode.name(),
        "title": manifest.title,
        "description": manifest.description,
        "photos": photo_records,
        "comparisons": comparison_records,
        "source_manifest": {
            "path": manifest_path.display().to_string(),
            "sha256": manifest_sha256,
        },
    })
}

/// Escape authored text for HTML text and attribute positions. The embedded
/// payload is JSON-escaped separately and suppresses `<`.
fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Escape the embedded JSON payload so authored text cannot terminate the
/// `<script type="application/json">` element. `<` and `>` are suppressed as
/// `\u003c`/`\u003e`, which is valid JSON escaping and keeps the payload
/// parseable by `JSON.parse`.
fn escape_payload(data: &str) -> String {
    data.replace('<', "\\u003c").replace('>', "\\u003e")
}

/// The static shell around the viewer assets: inlined CSS, the authored title as
/// text, the embedded payload, and the local viewer script.
fn html_shell(title: &str, embedded: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\"><title>{title}</title><style>{CSS}</style></head><body>{BODY}<script type=\"application/json\" id=\"photo-report-data\">{embedded}</script><script src=\"viewer.js\"></script></body></html>",
    )
}

/// Hash and verify every published artifact; a missing or altered file fails
/// before the command can claim completion.
fn verify_artifacts(out: &Path, artifacts: &[Value]) -> Result<()> {
    for artifact in artifacts {
        let relative = artifact["path"]
            .as_str()
            .ok_or_else(|| err("invalid_document", "artifact must identify a path"))?;
        let expected = artifact["sha256"]
            .as_str()
            .ok_or_else(|| err("invalid_document", "artifact must identify a sha256"))?;
        let actual = sha256_file(&out.join(relative))?;
        if actual != expected {
            return Err(err(
                "artifact_hash",
                format!("artifact {relative} changed after publication"),
            )
            .at(out.join(relative).display().to_string()));
        }
    }
    Ok(())
}

/// Build the offline photo report and return the standard command result.
///
/// Manifest validation and source resolution precede destination creation. All
/// photos are decoded and all ROI bounds are checked in a private sibling
/// directory first, so validation failures do not leave a report directory.
/// The manifest and its sources are never modified.
pub fn build_report(manifest: &Path, out: &Path) -> Result<Value> {
    let root = documents::normalize_path(out)?;
    if fs::symlink_metadata(&root).is_ok() {
        return Err(common::destination_exists(&root));
    }
    build_report_impl(manifest, &root)
        .map_err(|error| common::incomplete_destination(error, &root, "report.json"))
}

fn staging_directory(root: &Path) -> Result<PathBuf> {
    let parent = root
        .parent()
        .ok_or_else(|| err("publication_failure", "report destination has no parent"))?;
    fs::create_dir_all(parent)?;
    for attempt in 0..100u32 {
        let candidate = parent.join(format!(".photo-report-{}-{attempt}", std::process::id()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(err("publication_failure", error.to_string())),
        }
    }
    Err(err(
        "publication_failure",
        "could not allocate a private Photo Report staging directory",
    ))
}

fn publish_staging(stage: &Path, root: &Path) -> Result<()> {
    common::create_destination(root)?;
    for name in ["images", "viewer.js", "index.html", "report.json"] {
        fs::rename(stage.join(name), root.join(name)).map_err(|error| {
            err(
                "publication_failure",
                format!("could not publish {name}: {error}"),
            )
        })?;
    }
    Ok(())
}

fn build_report_impl(manifest: &Path, root: &Path) -> Result<Value> {
    let manifest_path = documents::normalize_path(manifest)?;
    let manifest_sha256 = sha256_file(&manifest_path)?;
    let authored = documents::load_json_document(&manifest_path)?;
    if sha256_file(&manifest_path)? != manifest_sha256 {
        return Err(err(
            "hash_mismatch",
            "photo manifest changed while being read",
        ));
    }
    let parsed = parse_manifest(&authored)?;
    let (mode, requests) = resolve_mode(&parsed)?;
    let photo_values: Vec<Map<String, Value>> = authored["photos"]
        .as_array()
        .ok_or_else(|| err("invalid_document", "photos must be an array"))?
        .iter()
        .map(|photo| obj(photo).cloned())
        .collect::<Result<_>>()?;

    // Resolve all source dependencies before allocating a staging directory.
    let sources: Vec<PathBuf> = parsed
        .photos
        .iter()
        .zip(&photo_values)
        .map(|(photo, value)| source_path(value, &manifest_path, &photo.id))
        .collect::<Result<_>>()?;

    let stage = staging_directory(root)?;
    let result = (|| -> Result<Value> {
        let work = stage.join(WORK_DIR);
        fs::create_dir_all(&work)?;
        fs::create_dir_all(stage.join("images"))?;
        let mut decoded = Vec::new();
        let mut artifacts: Vec<Value> = Vec::new();
        for (index, (photo, source)) in parsed.photos.iter().zip(&sources).enumerate() {
            let relative = format!("images/{index:04}.png");
            let photo = decode_photo(
                photo,
                source,
                &snapshot_path(&work, index, source),
                &relative,
                &stage.join(&relative),
            )?;
            validate_roi_bounds(&photo)?;
            artifacts.push(json!({
                "path": relative,
                "sha256": photo.display_sha256,
                "media_type": "image/png",
                "role": "display_image",
                "availability": "available",
                "dimensions": [photo.width, photo.height],
            }));
            decoded.push(photo);
        }
        let comparisons: Vec<ResolvedComparison> = requests
            .iter()
            .map(|request| resolve_pair(request, &decoded))
            .collect();
        if sha256_file(&manifest_path)? != manifest_sha256 {
            return Err(err(
                "hash_mismatch",
                "photo manifest changed during generation",
            ));
        }
        let observed: Vec<String> = decoded.iter().map(|p| p.source_sha256.clone()).collect();
        verify_source_hashes(&sources, &observed)?;

        fs::remove_dir_all(&work)?;
        fs::write(stage.join("viewer.js"), JS)?;
        artifacts.push(json!({
            "path": "viewer.js",
            "sha256": sha256_file(&stage.join("viewer.js"))?,
            "media_type": "text/javascript",
            "role": "viewer_script",
            "availability": "available",
        }));
        let mut payload = payload(
            &parsed,
            mode,
            &decoded,
            &comparisons,
            &manifest_path,
            &manifest_sha256,
        );
        payload["artifacts"] = json!(artifacts);
        let data = String::from_utf8(common::canonical_json(&payload)?)
            .map_err(|e| err("document_read", e.to_string()))?;
        fs::write(
            stage.join("index.html"),
            html_shell(&escape_text(&parsed.title), &escape_payload(&data)),
        )?;
        artifacts.push(json!({
            "path": "index.html",
            "sha256": sha256_file(&stage.join("index.html"))?,
            "media_type": "text/html",
            "role": "report_page",
            "availability": "available",
        }));
        payload["artifacts"] = json!(artifacts);
        dump(&stage.join("report.json"), &payload)?;
        verify_artifacts(&stage, &artifacts)?;
        publish_staging(&stage, root)?;
        Ok(json!({
            "schema_version": 1,
            "command": "report photos",
            "source": "photo_manifest",
            "mode": mode.name(),
            "execution_status": "completed",
            "artifact": root.join("index.html"),
            "manifest": root.join("report.json"),
            "photo_count": decoded.len(),
            "comparison_count": comparisons.len(),
        }))
    })();
    let _ = fs::remove_dir_all(&stage);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(path: &Path, width: usize, height: usize) {
        image_data::write_png16_codes(path, &vec![[0x8000; 3]; width * height], width, height)
            .unwrap();
    }

    fn photo(id: &str, path: &str, label: &str) -> String {
        format!(r#"{{"id":"{id}","path":"{path}","label":"{label}"}}"#)
    }

    fn photo_with_rois(id: &str, path: &str, label: &str, rois: &[String]) -> String {
        format!(
            r#"{{"id":"{id}","path":"{path}","label":"{label}","rois":[{}]}}"#,
            rois.join(",")
        )
    }

    fn roi(id: &str, rect: [u64; 4]) -> String {
        format!(
            r#"{{"id":"{id}","label":"区域 {id}","rect":[{},{},{},{}]}}"#,
            rect[0], rect[1], rect[2], rect[3]
        )
    }

    fn manifest(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("manifest.json");
        fs::write(&path, body).unwrap();
        path
    }

    fn single(body: &str) -> String {
        format!(r#"{{"schema_version":1,"title":"标题","photos":[{body}]}}"#)
    }

    fn pair(left: &str, right: &str) -> String {
        format!(r#"{{"schema_version":1,"title":"对照","photos":[{left},{right}]}}"#)
    }

    fn three(a: &str, b: &str, c: &str, comparisons: &str) -> String {
        format!(
            r#"{{"schema_version":1,"title":"多图","photos":[{a},{b},{c}],"comparisons":{comparisons}}}"#
        )
    }

    fn report_of(out: &Path) -> Value {
        serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap()
    }

    #[test]
    fn single_photo_report_shows_the_photo_and_declared_rois() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 16, 12);
        let manifest = manifest(
            dir.path(),
            &single(&photo_with_rois(
                "scan",
                "a.png",
                "扫描",
                &[roi("face", [2, 3, 4, 5]), roi("edge", [0, 0, 16, 12])],
            )),
        );
        let out = dir.path().join("out");
        let result = build_report(&manifest, &out).unwrap();
        assert_eq!(result["command"], "report photos");
        assert_eq!(result["source"], "photo_manifest");
        assert_eq!(result["mode"], "single_photo");
        assert_eq!(result["photo_count"], 1);
        assert_eq!(result["comparison_count"], 0);
        assert!(out.join("index.html").is_file());
        assert!(out.join("report.json").is_file());
        assert!(out.join("viewer.js").is_file());
        assert!(out.join("images/0000.png").is_file());
        assert!(!out.join(".photo-report-work").exists());

        let report = report_of(&out);
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["mode"], "single_photo");
        assert_eq!(report["title"], "标题");
        assert_eq!(report["description"], Value::Null);
        assert_eq!(report["photos"][0]["id"], "scan");
        assert_eq!(report["photos"][0]["src"], "images/0000.png");
        assert_eq!(report["photos"][0]["width"], 16);
        assert_eq!(report["photos"][0]["height"], 12);
        assert_eq!(report["photos"][0]["color"]["color_space"], "sRGB");
        assert_eq!(report["photos"][0]["color"]["cctf_decoding"], true);
        assert_eq!(report["photos"][0]["display"]["bit_depth"], 16);
        assert_eq!(report["photos"][0]["display"]["carrier"], "png");
        assert_eq!(report["photos"][0]["display"]["resampling"], "none");
        assert_eq!(report["photos"][0]["source"]["path"], "a.png");
        assert_eq!(report["photos"][0]["rois"][0]["rect"], json!([2, 3, 4, 5]));
        assert_eq!(report["photos"][0]["rois"][1]["id"], "edge");
        assert_eq!(report["comparisons"].as_array().unwrap().len(), 0);
        assert!(report["source_manifest"]["sha256"].is_string());
        let roles: Vec<&str> = report["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["display_image", "viewer_script", "report_page"]);

        let html = fs::read_to_string(out.join("index.html")).unwrap();
        assert!(html.contains("photo-report-data"));
        assert!(html.contains("src=\"viewer.js\""));
        assert!(html.contains("<title>标题</title>"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn two_photos_form_one_fixed_pair_in_manifest_order() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 12, 8);
        png(&dir.path().join("b.png"), 10, 10);
        let manifest = manifest(
            dir.path(),
            &pair(
                &photo_with_rois("left", "a.png", "左", &[roi("shared", [0, 0, 4, 4])]),
                &photo_with_rois("right", "b.png", "右", &[roi("shared", [1, 1, 3, 3])]),
            ),
        );
        let out = dir.path().join("out");
        let result = build_report(&manifest, &out).unwrap();
        assert_eq!(result["mode"], "two_photo");
        assert_eq!(result["comparison_count"], 1);

        let report = report_of(&out);
        let comparison = &report["comparisons"][0];
        assert_eq!(comparison["id"], "left__right");
        assert_eq!(comparison["left"], "left");
        assert_eq!(comparison["right"], "right");
        assert_eq!(comparison["label"], Value::Null);
        let sides = &comparison["rois"][0];
        assert_eq!(sides["id"], "shared");
        assert_eq!(sides["left"]["availability"], "available");
        assert_eq!(sides["left"]["dimensions"], json!([4, 4]));
        assert_eq!(sides["left"]["display_artifact"], "images/0000.png");
        assert_eq!(sides["right"]["rect"], json!([1, 1, 3, 3]));
        assert_eq!(sides["right"]["dimensions"], json!([3, 3]));
        assert_eq!(sides["right"]["display_artifact"], "images/0001.png");
    }

    #[test]
    fn a_one_sided_roi_reports_an_explicit_unavailable_side() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 12, 8);
        png(&dir.path().join("b.png"), 12, 8);
        let manifest = manifest(
            dir.path(),
            &pair(
                &photo_with_rois("left", "a.png", "左", &[roi("shared", [0, 0, 4, 4])]),
                &photo("right", "b.png", "右"),
            ),
        );
        let out = dir.path().join("out");
        build_report(&manifest, &out).unwrap();
        let sides = &report_of(&out)["comparisons"][0]["rois"][0];
        assert_eq!(sides["left"]["availability"], "available");
        assert_eq!(sides["right"]["availability"], "unavailable");
        assert!(sides["right"]["reason"].is_string());
        assert!(sides["right"]["rect"].is_null());
        assert!(sides["right"]["display_artifact"].is_null());
    }

    #[test]
    fn the_roi_union_follows_left_order_then_right_only_ids() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 12, 8);
        png(&dir.path().join("b.png"), 12, 8);
        let manifest = manifest(
            dir.path(),
            &pair(
                &photo_with_rois(
                    "left",
                    "a.png",
                    "左",
                    &[roi("only-left", [0, 0, 2, 2]), roi("shared", [0, 0, 4, 4])],
                ),
                &photo_with_rois(
                    "right",
                    "b.png",
                    "右",
                    &[roi("shared", [0, 0, 4, 4]), roi("only-right", [2, 2, 4, 4])],
                ),
            ),
        );
        let out = dir.path().join("out");
        build_report(&manifest, &out).unwrap();
        let rois = report_of(&out)["comparisons"][0]["rois"]
            .as_array()
            .unwrap()
            .clone();
        let ids: Vec<&str> = rois.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["only-left", "shared", "only-right"]);
        assert_eq!(rois[0]["left"]["availability"], "available");
        assert_eq!(rois[0]["right"]["availability"], "unavailable");
        assert_eq!(rois[2]["left"]["availability"], "unavailable");
        assert_eq!(rois[2]["right"]["availability"], "available");
    }

    #[test]
    fn three_photos_show_only_declared_pairs() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.png", "b.png", "c.png"] {
            png(&dir.path().join(name), 8, 8);
        }
        let manifest = manifest(
            dir.path(),
            &three(
                &photo("a", "a.png", "A"),
                &photo("b", "b.png", "B"),
                &photo("c", "c.png", "C"),
                r#"[{"left":"a","right":"b"},{"left":"b","right":"c","id":"second"}]"#,
            ),
        );
        let out = dir.path().join("out");
        let result = build_report(&manifest, &out).unwrap();
        assert_eq!(result["mode"], "requested_comparisons");
        let report = report_of(&out);
        let comparisons = report["comparisons"].as_array().unwrap();
        assert_eq!(comparisons.len(), 2);
        assert_eq!(comparisons[0]["id"], "a__b");
        assert_eq!(comparisons[0]["left"], "a");
        assert_eq!(comparisons[0]["right"], "b");
        assert_eq!(comparisons[1]["id"], "second");
        assert_eq!(comparisons[1]["left"], "b");
        assert_eq!(comparisons[1]["right"], "c");
    }

    #[test]
    fn three_photos_without_comparisons_fail_before_publication() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.png", "b.png", "c.png"] {
            png(&dir.path().join(name), 8, 8);
        }
        let manifest = manifest(
            dir.path(),
            &format!(
                r#"{{"schema_version":1,"title":"多图","photos":[{},{},{}]}}"#,
                photo("a", "a.png", "A"),
                photo("b", "b.png", "B"),
                photo("c", "c.png", "C")
            ),
        );
        let out = dir.path().join("out");
        let error = build_report(&manifest, &out).unwrap_err();
        assert_eq!(error.code, "missing_comparisons");
        assert!(!out.exists());
    }

    #[test]
    fn small_inputs_may_not_declare_comparisons() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        png(&dir.path().join("b.png"), 8, 8);
        let one = manifest(
            dir.path(),
            r#"{"schema_version":1,"title":"单图","photos":[{"id":"a","path":"a.png","label":"A"}],"comparisons":[]}"#,
        );
        let out = dir.path().join("out-one");
        assert_eq!(
            build_report(&one, &out).unwrap_err().code,
            "invalid_comparison"
        );
        assert!(!out.exists());

        let two = manifest(
            dir.path(),
            r#"{"schema_version":1,"title":"对照","photos":[{"id":"a","path":"a.png","label":"A"},{"id":"b","path":"b.png","label":"B"}],"comparisons":[{"left":"a","right":"b"}]}"#,
        );
        let out = dir.path().join("out-two");
        assert_eq!(
            build_report(&two, &out).unwrap_err().code,
            "invalid_comparison"
        );
        assert!(!out.exists());
    }

    #[test]
    fn empty_self_unknown_and_duplicate_pairs_fail() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.png", "b.png", "c.png"] {
            png(&dir.path().join(name), 8, 8);
        }
        let cases = [
            ("[]", "missing_comparisons"),
            (r#"[{"left":"a","right":"a"}]"#, "self_comparison"),
            (r#"[{"left":"a","right":"missing"}]"#, "unknown_photo"),
            (r#"[{"left":"missing","right":"b"}]"#, "unknown_photo"),
            (
                r#"[{"id":"p","left":"a","right":"b"},{"id":"p","left":"b","right":"c"}]"#,
                "duplicate_id",
            ),
        ];
        for (comparisons, code) in cases {
            let manifest = manifest(
                dir.path(),
                &three(
                    &photo("a", "a.png", "A"),
                    &photo("b", "b.png", "B"),
                    &photo("c", "c.png", "C"),
                    comparisons,
                ),
            );
            let out = dir.path().join(format!("out-{code}-{}", comparisons.len()));
            assert_eq!(build_report(&manifest, &out).unwrap_err().code, code);
        }
    }

    #[test]
    fn roi_outside_decoded_bounds_fails_and_names_an_incomplete_destination() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let manifest = manifest(
            dir.path(),
            &single(&photo_with_rois(
                "scan",
                "a.png",
                "扫描",
                &[roi("outside", [6, 6, 8, 8])],
            )),
        );
        let out = dir.path().join("out");
        let error = build_report(&manifest, &out).unwrap_err();
        assert_eq!(error.code, "invalid_bounds");
        assert!(error.message.contains("scan"));
        assert!(error.message.contains("outside"));
        assert!(!out.join("report.json").exists());
        assert!(!out.join("index.html").exists());
    }

    #[test]
    fn roi_rectangles_must_be_positive_and_unique_per_photo() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let zero = manifest(
            dir.path(),
            &single(&photo_with_rois(
                "scan",
                "a.png",
                "扫描",
                &[roi("thin", [0, 0, 0, 4])],
            )),
        );
        assert_eq!(
            build_report(&zero, &dir.path().join("out-zero"))
                .unwrap_err()
                .code,
            "schema_validation"
        );

        let duplicate = manifest(
            dir.path(),
            &single(&photo_with_rois(
                "scan",
                "a.png",
                "扫描",
                &[roi("face", [0, 0, 2, 2]), roi("face", [0, 0, 3, 3])],
            )),
        );
        assert_eq!(
            build_report(&duplicate, &dir.path().join("out-duplicate"))
                .unwrap_err()
                .code,
            "duplicate_id"
        );
    }

    #[test]
    fn unsafe_and_unsupported_declarations_fail() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let cases = [
            (
                r#"{"id":"a","path":"/etc/passwd","label":"A"}"#,
                "invalid_path",
            ),
            (
                r#"{"id":"a","path":"../a.png","label":"A"}"#,
                "invalid_path",
            ),
            (
                r#"{"id":"a","path":"missing.png","label":"A"}"#,
                "dependency_read",
            ),
            (
                r#"{"id":"a","path":"a.png","label":"A","color_space":"Rec. 709"}"#,
                "unsupported_color_space",
            ),
            (
                r#"{"id":"a","path":"a.png","label":""}"#,
                "schema_validation",
            ),
        ];
        for (index, (photo, code)) in cases.iter().enumerate() {
            let manifest = manifest(dir.path(), &single(photo));
            let out = dir.path().join(format!("out-case-{index}"));
            assert_eq!(build_report(&manifest, &out).unwrap_err().code, *code);
            assert!(!out.exists());
        }
    }

    #[test]
    fn strict_json_rejects_unknown_fields_duplicate_keys_and_bad_schema() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let cases = [
            (
                r#"{"id":"a","path":"a.png","label":"A","score":1}"#,
                "schema_validation",
            ),
            (
                r#"{"id":"a","path":"a.png","label":"A","label":"B"}"#,
                "duplicate_key",
            ),
            (
                r#"{"id":"a","path":"a.png","label":"A"},{"id":"a","path":"a.png","label":"B"}"#,
                "duplicate_id",
            ),
        ];
        for (index, (photo, code)) in cases.iter().enumerate() {
            let manifest = manifest(dir.path(), &single(photo));
            let out = dir.path().join(format!("out-strict-{index}"));
            assert_eq!(build_report(&manifest, &out).unwrap_err().code, *code);
        }

        let version = manifest(
            dir.path(),
            r#"{"schema_version":2,"title":"标题","photos":[{"id":"a","path":"a.png","label":"A"}]}"#,
        );
        assert_eq!(
            build_report(&version, &dir.path().join("out-version"))
                .unwrap_err()
                .code,
            "unsupported_schema"
        );

        let none = manifest(
            dir.path(),
            r#"{"schema_version":1,"title":"标题","photos":[]}"#,
        );
        assert_eq!(
            build_report(&none, &dir.path().join("out-none"))
                .unwrap_err()
                .code,
            "invalid_document"
        );
    }

    #[test]
    fn hostile_authored_text_cannot_execute_in_the_page() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        // A title and label that try to close the data element and open a
        // script must survive as plain text only.
        let manifest = manifest(
            dir.path(),
            &format!(
                r#"{{"schema_version":1,"title":"</script><script>alert(1)</script>","description":"<img src=x onerror=alert(2)>","photos":[{photo}]}}"#,
                photo = r#"{"id":"a","path":"a.png","label":"<b>bold</b>","note":"<script>alert(3)</script>"}"#
            ),
        );
        let out = dir.path().join("out");
        build_report(&manifest, &out).unwrap();
        let html = fs::read_to_string(out.join("index.html")).unwrap();
        assert!(!html.contains("<script>alert"));
        // The embedded payload escapes every `<` so the data element cannot be
        // terminated by authored text.
        let embedded = html
            .split("id=\"photo-report-data\">")
            .nth(1)
            .and_then(|rest| rest.split("</script>").next())
            .unwrap();
        assert!(!embedded.contains('<'));
        assert!(embedded.contains("\\u003c"));
        // Everything outside the data element is markup plus escaped text: no
        // authored tag or attribute can reach an element position.
        let markup = html.replace(embedded, "");
        assert!(!markup.contains("onerror=alert"));
        assert!(!markup.contains("<b>bold</b>"));
        // The escaped title survives as text; labels live in the payload, where
        // JSON string escaping plus `<`/`>` suppression keeps them inert.
        assert!(markup.contains("&lt;script&gt;"));
        assert!(embedded.contains("\\u003cb\\u003ebold\\u003c/b\\u003e"));
        // The suppressed escapes are valid JSON, so the viewer can still parse.
        let restored = embedded.replace("\\u003c", "<").replace("\\u003e", ">");
        let parsed: Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(parsed["photos"][0]["label"], "<b>bold</b>");
    }

    #[test]
    fn explicit_non_srgb_and_linear_declarations_are_retained() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let p3 = manifest(
            dir.path(),
            &single(
                r#"{"id":"a","path":"a.png","label":"A","color_space":"Display P3","cctf_decoding":true}"#,
            ),
        );
        let out = dir.path().join("out");
        build_report(&p3, &out).unwrap();
        let report = report_of(&out);
        assert_eq!(report["photos"][0]["color"]["color_space"], "Display P3");
        assert_eq!(report["photos"][0]["color"]["cctf_decoding"], true);
        assert_eq!(report["photos"][0]["display"]["color_space"], "sRGB");

        let linear = manifest(
            dir.path(),
            &single(
                r#"{"id":"a","path":"a.png","label":"A","color_space":"ACEScg","cctf_decoding":false}"#,
            ),
        );
        let out = dir.path().join("out-linear");
        build_report(&linear, &out).unwrap();
        let report = report_of(&out);
        assert_eq!(report["photos"][0]["color"]["color_space"], "ACEScg");
        assert_eq!(report["photos"][0]["color"]["cctf_decoding"], false);
    }

    #[test]
    fn an_existing_destination_is_rejected_without_modification() {
        let dir = tempfile::tempdir().unwrap();
        png(&dir.path().join("a.png"), 8, 8);
        let manifest = manifest(dir.path(), &single(&photo("a", "a.png", "A")));
        let out = dir.path().join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("keep.txt"), b"keep").unwrap();
        let error = build_report(&manifest, &out).unwrap_err();
        assert_eq!(error.code, "destination_exists");
        assert_eq!(fs::read(out.join("keep.txt")).unwrap(), b"keep");
        assert!(!out.join("index.html").exists());
    }

    #[test]
    fn a_source_changed_after_decoding_fails_the_integrity_check() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.png");
        png(&source, 8, 8);
        let snapshot = dir.path().join("snapshot.png");
        fs::copy(&source, &snapshot).unwrap();
        let observed = vec![sha256_file(&snapshot).unwrap()];
        verify_source_hashes(std::slice::from_ref(&source), &observed).unwrap();
        png(&source, 4, 4);
        let error = verify_source_hashes(std::slice::from_ref(&source), &observed).unwrap_err();
        assert_eq!(error.code, "hash_mismatch");
    }

    #[test]
    fn hosted_text_is_escaped_in_the_page_shell() {
        let title = "<script>alert(1)</script>";
        let html = html_shell(&escape_text(title), "{}");
        assert!(!html.contains("<script>alert(1)"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("<title>&lt;script&gt;"));
        // The only real script elements are the data element and the viewer.
        assert_eq!(html.matches("<script").count(), 2);
    }
}
