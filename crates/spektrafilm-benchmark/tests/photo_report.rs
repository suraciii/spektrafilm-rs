//! Real-CLI acceptance for the benchmark's offline Photo Report command.
//!
//! Every scenario drives `spektrafilm-bench report photos` as a subprocess
//! against generated tiny RGB PNG sources, then inspects the published
//! directory exactly as a reviewer opening `index.html` would: the terminal
//! `report.json`, the display images, and the page's own payload.

use serde_json::{Value, json};
use spektrafilm_benchmark::{common, image_data};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-bench"))
}

/// Write one small RGB PNG of `width * height` flat-coloured pixels.
///
/// The sources are deliberately small: the scenarios exercise the manifest,
/// mode, ROI, and publication rules, not the converter. The library's own
/// 16-bit writer is used so the test needs no image encoder of its own, and a
/// display copy that keeps its declared dimensions proves the conversion ran
/// through the real image boundary rather than copying the source bytes.
fn write_png(path: &Path, width: usize, height: usize, rgb: [u8; 3]) {
    let samples: Vec<[f64; 3]> = (0..width * height)
        .map(|_| {
            [
                rgb[0] as f64 / 255.0,
                rgb[1] as f64 / 255.0,
                rgb[2] as f64 / 255.0,
            ]
        })
        .collect();
    image_data::write_png16(path, &samples, width, height).unwrap();
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn read_json(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Recursively copy one published report directory, the way sharing a report
/// copies the whole artifact.
fn copy_report(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            copy_report(&path, &target.join(entry.file_name()));
        } else {
            fs::copy(&path, target.join(entry.file_name())).unwrap();
        }
    }
}

/// One generated workspace: the manifest, its sources, and the destination.
struct Workspace {
    root: TempDir,
}

impl Workspace {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn manifest(&self) -> PathBuf {
        self.path("manifest.json")
    }

    fn report(&self) -> PathBuf {
        self.path("report")
    }

    fn write_manifest(&self, value: &Value) -> PathBuf {
        let path = self.manifest();
        write_json(&path, value);
        path
    }

    /// Run the real command and return its output plus the parsed stdout
    /// result. A non-zero exit is not a failure of this helper: the caller
    /// asserts the status and the stable error code.
    fn run(&self, manifest: &Path, out: &Path) -> (Output, Value) {
        let output = cli()
            .arg("report")
            .arg("photos")
            .arg("--manifest")
            .arg(manifest)
            .arg("--out")
            .arg(out)
            .output()
            .unwrap();
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        (output, result)
    }

    /// Run the command and require successful publication, returning the
    /// terminal manifest.
    fn publish(&self, manifest: &Path) -> Value {
        let out = self.report();
        let (output, _) = self.run(manifest, &out);
        assert_eq!(
            output.status.code(),
            Some(0),
            "publication must succeed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        read_json(&out.join("report.json"))
    }
}

/// The published photo record for one authored identity.
fn photo<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["photos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|photo| photo["id"] == id)
        .unwrap_or_else(|| panic!("photo {id} must appear in authored order"))
}

fn roi<'a>(photo: &'a Value, id: &str) -> &'a Value {
    photo["rois"]
        .as_array()
        .unwrap()
        .iter()
        .find(|roi| roi["id"] == id)
        .unwrap_or_else(|| panic!("ROI {id} must be listed"))
}

fn comparison<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["comparisons"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pair| pair["id"] == id)
        .unwrap_or_else(|| panic!("comparison {id} must be present"))
}

/// The resolved ROI entry of one comparison, by authored ROI identity.
fn resolved<'a>(pair: &'a Value, id: &str) -> &'a Value {
    pair["rois"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap_or_else(|| panic!("ROI {id} must be resolved for the pair"))
}

/// The published display-copy path of one photo. The generator derives it from
/// a stable identity, never from the source filename.
fn display_src(photo: &Value) -> String {
    let src = photo["src"]
        .as_str()
        .expect("a photo must name its display copy")
        .to_owned();
    assert!(
        src.starts_with("images/"),
        "the display copy lives under images/: {src}"
    );
    assert!(src.ends_with(".png"), "the display copy is a PNG: {src}");
    assert!(
        !src.contains(".."),
        "the display path stays inside the report: {src}"
    );
    src
}

/// The four-element native rectangle of an available ROI side.
fn rect_of(side: &Value) -> Vec<i64> {
    let rect = side["rect"]
        .as_array()
        .unwrap_or_else(|| panic!("an available side keeps its rectangle: {side}"));
    assert_eq!(
        rect.len(),
        4,
        "a rectangle is [x, y, width, height]: {rect:?}"
    );
    rect.iter()
        .map(|value| value.as_i64().expect("rectangle coordinates are integers"))
        .collect()
}

/// One native dimension of an available ROI side.
fn dimension(side: &Value, axis: usize) -> i64 {
    let pair = side["dimensions"]
        .as_array()
        .unwrap_or_else(|| panic!("an available side records its native dimensions: {side}"));
    assert_eq!(pair.len(), 2, "dimensions are [width, height]: {pair:?}");
    pair[axis].as_i64().expect("dimensions are integers")
}

/// The published display copy an available ROI side renders from. The pairing
/// rule is the shared ROI identity only, never label, rectangle, size,
/// position, or filename matching.
fn display_artifact_of(side: &Value) -> &str {
    side["display_artifact"]
        .as_str()
        .unwrap_or_else(|| panic!("an available side names its display copy: {side}"))
}

/// The reason an unavailable side carries. The side stays an object with an
/// explicit availability and reason, and never serializes a rectangle.
fn unavailable_reason(side: &Value) -> String {
    assert!(
        side.is_object(),
        "an unavailable side stays an object with a reason: {side}"
    );
    assert_eq!(
        side["availability"], "unavailable",
        "the side is explicitly unavailable: {side}"
    );
    for key in ["rect", "dimensions", "display_artifact"] {
        assert!(
            side.get(key).is_none(),
            "an unavailable side must not serialize {key}: {side}"
        );
    }
    let reason = side["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("an unavailable side names its reason: {side}"));
    assert!(
        !reason.is_empty(),
        "an unavailable side names a nonempty reason: {side}"
    );
    reason.to_owned()
}

/// A photo manifest entry with the required authored fields only.
fn photo_entry(id: &str, path: &str, label: &str) -> Value {
    json!({"id": id, "path": path, "label": label})
}

/// One authored manifest with the required top-level fields.
fn manifest_with(photos: Vec<Value>, comparisons: Option<Value>) -> Value {
    let mut manifest = json!({
        "schema_version": 1,
        "title": "800T outdoor comparison",
        "photos": photos,
    });
    if let Some(comparisons) = comparisons {
        manifest["comparisons"] = comparisons;
    }
    manifest
}

/// Assert the published artifacts a reviewer needs, with their hashes intact.
fn assert_published_artifacts(report: &Value, out: &Path, expected_images: usize) {
    assert!(
        out.join("index.html").is_file(),
        "index.html must be published"
    );
    assert!(
        out.join("viewer.js").is_file(),
        "viewer.js must be published"
    );
    let images = fs::read_dir(out.join("images"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "png")
        })
        .count();
    assert_eq!(
        images, expected_images,
        "one display copy per photo, never one per pair"
    );
    let mut display_paths = Vec::new();
    for photo in report["photos"].as_array().unwrap() {
        let src = display_src(photo);
        let display = out.join(&src);
        assert!(display.is_file(), "display copy {src} must be published");
        let observed = common::sha256_file(&display).unwrap();
        assert_eq!(
            photo["display"]["sha256"].as_str(),
            Some(observed.as_str()),
            "the display hash must describe the published bytes"
        );
        display_paths.push(src);
    }
    let unique: std::collections::BTreeSet<&String> = display_paths.iter().collect();
    assert_eq!(
        unique.len(),
        display_paths.len(),
        "each photo keeps its own display copy: {display_paths:?}"
    );
    for artifact in report["artifacts"]
        .as_array()
        .expect("report.json names its published artifacts")
    {
        let path = artifact["path"]
            .as_str()
            .expect("an artifact names its published path");
        let published = out.join(path);
        assert!(published.is_file(), "artifact {path} must be published");
        assert_eq!(
            artifact["sha256"].as_str(),
            Some(common::sha256_file(&published).unwrap().as_str()),
            "artifact hash must describe the published bytes"
        );
    }
}

/// Acceptance 1: one photo, no ROI, opens a single-photo page with no
/// comparison controls.
#[test]
fn single_photo_without_roi_publishes_a_single_photo_report() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 6, 4, [10, 200, 90]);
    let manifest = work.write_manifest(&manifest_with(
        vec![photo_entry("scan", "scan.png", "Original scan")],
        None,
    ));
    let out = work.report();
    let (output, result) = work.run(&manifest, &out);
    assert_eq!(output.status.code(), Some(0));
    let report = read_json(&out.join("report.json"));

    assert_eq!(report["mode"], "single_photo");
    assert_eq!(result["mode"], "single_photo");
    assert_eq!(result["execution_status"], "completed");
    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["command"], "report photos");
    assert!(result["source"].is_string());
    assert_eq!(
        result["artifact"].as_str(),
        Some(out.join("index.html").to_str().unwrap())
    );
    assert_eq!(
        result["manifest"].as_str(),
        Some(out.join("report.json").to_str().unwrap())
    );
    assert_eq!(report["title"], "800T outdoor comparison");
    assert_eq!(report["photos"].as_array().unwrap().len(), 1);
    assert!(
        report["comparisons"].as_array().unwrap().is_empty(),
        "a single photo is never compared with anything"
    );

    let scan = photo(&report, "scan");
    assert_eq!(scan["label"], "Original scan");
    assert_eq!(scan["width"], 6);
    assert_eq!(scan["height"], 4);
    assert!(scan["rois"].as_array().unwrap().is_empty());
    let src = display_src(scan);
    assert_ne!(
        Path::new(&src).file_name(),
        Path::new("scan.png").file_name(),
        "the display path is not the source filename"
    );

    // Source provenance and the fixed display default are both retained.
    assert_eq!(
        scan["source"]["sha256"].as_str(),
        Some(
            common::sha256_file(&work.path("scan.png"))
                .unwrap()
                .as_str()
        )
    );
    assert_eq!(scan["source"]["path"], "scan.png");
    assert_eq!(scan["color"]["color_space"], "sRGB");
    assert_eq!(scan["color"]["cctf_decoding"], true);
    assert_eq!(scan["display"]["color_space"], "sRGB");
    assert_eq!(scan["display"]["bit_depth"], 16);

    let html = fs::read_to_string(out.join("index.html")).unwrap();
    assert!(
        html.contains("photo-report-data"),
        "the page embeds its payload for direct file opening"
    );
    assert!(
        html.contains("viewer.js"),
        "the page loads its local viewer script"
    );
    assert!(
        !html.contains("http://") && !html.contains("https://"),
        "the page must not reference a network origin"
    );
    assert_published_artifacts(&report, &out, 1);
}

/// Acceptance 2: one photo with two ROIs opens the full photo and each declared
/// ROI detail.
#[test]
fn single_photo_with_rois_keeps_every_declared_roi() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 8, 6, [12, 20, 240]);
    let manifest = work.write_manifest(&manifest_with(
        vec![json!({
            "id": "scan",
            "path": "scan.png",
            "label": "Original scan",
            "rois": [
                {"id": "face", "label": "Face", "rect": [0, 0, 4, 3]},
                {"id": "sky", "label": "Sky", "rect": [2, 1, 6, 5], "note": "Upper right."}
            ]
        })],
        None,
    ));
    let report = work.publish(&manifest);
    let scan = photo(&report, "scan");
    assert_eq!(scan["rois"].as_array().unwrap().len(), 2);
    assert_eq!(roi(scan, "face")["rect"], json!([0, 0, 4, 3]));
    assert_eq!(roi(scan, "sky")["label"], "Sky");
    assert_eq!(roi(scan, "sky")["note"], "Upper right.");
    assert_published_artifacts(&report, &work.report(), 1);
}

/// Acceptance 3: two photos without ROIs open one full-image comparison in
/// manifest order, with no `comparisons` field required.
#[test]
fn two_photos_without_rois_fix_one_pair_in_authored_order() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 5, 5, [255, 0, 0]);
    write_png(&work.path("sim.png"), 5, 5, [0, 0, 255]);
    let manifest = work.write_manifest(&manifest_with(
        vec![
            photo_entry("scan", "scan.png", "Original scan"),
            photo_entry("sim", "sim.png", "Simulation output"),
        ],
        None,
    ));
    let report = work.publish(&manifest);
    assert_eq!(report["mode"], "two_photo");
    let pairs = report["comparisons"].as_array().unwrap();
    assert_eq!(pairs.len(), 1, "exactly one fixed pair is derived");
    assert_eq!(
        pairs[0]["id"], "scan__sim",
        "the derived pair identity is stable and follows the authored order"
    );
    assert_eq!(pairs[0]["left"], "scan");
    assert_eq!(pairs[0]["right"], "sim");
    assert!(
        pairs[0]["rois"].as_array().unwrap().is_empty(),
        "no ROI is declared, so no ROI is resolved"
    );
    assert_published_artifacts(&report, &work.report(), 2);
}

/// Acceptance 4: two photos with a shared ROI open the full-image comparison
/// and the corresponding two-sided ROI comparison.
#[test]
fn two_photos_with_a_shared_roi_resolve_both_sides() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 8, 8, [200, 200, 200]);
    write_png(&work.path("sim.png"), 8, 8, [40, 40, 40]);
    let manifest = work.write_manifest(&manifest_with(
        vec![
            json!({
                "id": "scan", "path": "scan.png", "label": "Original scan",
                "rois": [{"id": "face", "label": "Face", "rect": [1, 1, 4, 4]}]
            }),
            json!({
                "id": "sim", "path": "sim.png", "label": "Simulation output",
                "rois": [{"id": "face", "label": "Face", "rect": [2, 2, 3, 3]}]
            }),
        ],
        None,
    ));
    let report = work.publish(&manifest);
    let pair = &report["comparisons"].as_array().unwrap()[0];
    let face = resolved(pair, "face");
    assert_eq!(rect_of(&face["left"]), vec![1, 1, 4, 4]);
    assert_eq!(rect_of(&face["right"]), vec![2, 2, 3, 3]);
    assert_eq!(dimension(&face["left"], 0), 4);
    assert_eq!(dimension(&face["right"], 0), 3);
    assert_eq!(face["left"]["availability"], "available");
    assert_eq!(face["right"]["availability"], "available");
    assert_published_artifacts(&report, &work.report(), 2);
}

/// Acceptance 5: a one-sided ROI shows the available crop and an explicit
/// unavailable counterpart that stays an object with a reason.
#[test]
fn two_photos_with_a_one_sided_roi_keep_an_explicit_unavailable_side() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 8, 8, [10, 10, 10]);
    write_png(&work.path("sim.png"), 8, 8, [245, 245, 245]);
    let manifest = work.write_manifest(&manifest_with(
        vec![
            json!({
                "id": "scan", "path": "scan.png", "label": "Original scan",
                "rois": [{"id": "glint", "label": "Glint", "rect": [0, 0, 3, 3]}]
            }),
            photo_entry("sim", "sim.png", "Simulation output"),
        ],
        None,
    ));
    let report = work.publish(&manifest);
    let pair = &report["comparisons"].as_array().unwrap()[0];
    let glint = resolved(pair, "glint");
    assert_eq!(rect_of(&glint["left"]), vec![0, 0, 3, 3]);
    assert!(
        !unavailable_reason(&glint["right"]).is_empty(),
        "the missing side is an explicit unavailable state with a reason"
    );
    assert_published_artifacts(&report, &work.report(), 2);
}

/// Acceptance 6 and 8: three photos with two declared pairs show exactly those
/// pairs, and no undeclared pair is ever created.
#[test]
fn three_photos_generate_only_declared_pairs() {
    let work = Workspace::new();
    for name in ["a.png", "b.png", "c.png"] {
        write_png(&work.path(name), 4, 4, [128, 64, 32]);
    }
    let manifest = work.write_manifest(&manifest_with(
        vec![
            photo_entry("a", "a.png", "Photo A"),
            photo_entry("b", "b.png", "Photo B"),
            photo_entry("c", "c.png", "Photo C"),
        ],
        Some(json!([
            {"left": "a", "right": "b"},
            {"left": "b", "right": "c"}
        ])),
    ));
    let out = work.report();
    let (output, result) = work.run(&manifest, &out);
    assert_eq!(output.status.code(), Some(0));
    let report = read_json(&out.join("report.json"));

    assert_eq!(report["mode"], "requested_comparisons");
    assert_eq!(result["mode"], "requested_comparisons");
    let pairs = report["comparisons"].as_array().unwrap();
    assert_eq!(pairs.len(), 2, "only the declared pairs are generated");
    assert_eq!(pairs[0]["left"], "a");
    assert_eq!(pairs[0]["right"], "b");
    assert_eq!(pairs[1]["left"], "b");
    assert_eq!(pairs[1]["right"], "c");
    for pair in pairs {
        let left = pair["left"].as_str().unwrap();
        let right = pair["right"].as_str().unwrap();
        assert_ne!(left, right, "a photo is never compared with itself");
        assert!(photo(&report, left)["id"].is_string());
        assert!(photo(&report, right)["id"].is_string());
    }
    assert!(
        pairs
            .iter()
            .all(|pair| pair["left"] != "a" || pair["right"] != "c"),
        "the undeclared a/c pair must not exist"
    );
    assert_published_artifacts(&report, &out, 3);
}

/// Acceptance 7: three photos without `comparisons` fail before publication.
#[test]
fn three_photos_without_comparisons_fail_before_publication() {
    let work = Workspace::new();
    for name in ["a.png", "b.png", "c.png"] {
        write_png(&work.path(name), 4, 4, [1, 2, 3]);
    }
    let manifest = work.write_manifest(&manifest_with(
        vec![
            photo_entry("a", "a.png", "Photo A"),
            photo_entry("b", "b.png", "Photo B"),
            photo_entry("c", "c.png", "Photo C"),
        ],
        None,
    ));
    let (output, result) = work.run(&manifest, &work.report());
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(result["execution_status"], "failed");
    assert!(
        result["errors"][0]["code"]
            .as_str()
            .is_some_and(|code| !code.is_empty()),
        "a stable error code must identify the failure: {result}"
    );
    assert!(!work.report().exists(), "nothing may be published");
}

/// An empty `comparisons` array is not an explicit pair list.
#[test]
fn empty_comparisons_are_rejected() {
    let work = Workspace::new();
    for name in ["a.png", "b.png", "c.png"] {
        write_png(&work.path(name), 4, 4, [9, 9, 9]);
    }
    let manifest = work.write_manifest(&manifest_with(
        vec![
            photo_entry("a", "a.png", "Photo A"),
            photo_entry("b", "b.png", "Photo B"),
            photo_entry("c", "c.png", "Photo C"),
        ],
        Some(json!([])),
    ));
    let (output, result) = work.run(&manifest, &work.report());
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(result["execution_status"], "failed");
    assert!(!work.report().exists());
}

/// The small input form stays unambiguous: one or two photos must not provide
/// `comparisons`.
#[test]
fn comparisons_are_rejected_for_one_and_two_photos() {
    for count in [1usize, 2] {
        let work = Workspace::new();
        let mut photos = Vec::new();
        for index in 0..count {
            let name = format!("p{index}.png");
            write_png(&work.path(&name), 4, 4, [index as u8, 0, 0]);
            photos.push(photo_entry(&format!("p{index}"), &name, "Photo"));
        }
        let manifest = work.write_manifest(&manifest_with(
            photos,
            Some(json!([{"left": "p0", "right": format!("p{}", count - 1)}])),
        ));
        let (output, result) = work.run(&manifest, &work.report());
        assert_eq!(
            output.status.code(),
            Some(1),
            "{count} photos must not accept comparisons: {result}"
        );
        assert!(!work.report().exists());
    }
}

/// A manifest that names an unknown photo, or compares a photo with itself,
/// fails before publication.
#[test]
fn unknown_and_self_comparisons_are_rejected() {
    let work = Workspace::new();
    for name in ["a.png", "b.png", "c.png"] {
        write_png(&work.path(name), 4, 4, [7, 7, 7]);
    }
    let photos = vec![
        photo_entry("a", "a.png", "Photo A"),
        photo_entry("b", "b.png", "Photo B"),
        photo_entry("c", "c.png", "Photo C"),
    ];
    for comparisons in [
        json!([{"left": "a", "right": "missing"}]),
        json!([{"left": "a", "right": "a"}]),
    ] {
        let manifest = work.write_manifest(&manifest_with(photos.clone(), Some(comparisons)));
        let (output, result) = work.run(&manifest, &work.report());
        assert_eq!(output.status.code(), Some(1), "{result}");
        assert!(!work.report().exists());
    }
}

/// Acceptance 10: an ROI outside the decoded bounds fails before publication.
#[test]
fn roi_outside_decoded_bounds_fails_before_publication() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [30, 60, 90]);
    let manifest = work.write_manifest(&manifest_with(
        vec![json!({
            "id": "scan", "path": "scan.png", "label": "Original scan",
            "rois": [{"id": "overflow", "label": "Overflow", "rect": [2, 2, 4, 4]}]
        })],
        None,
    ));
    let (output, result) = work.run(&manifest, &work.report());
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(result["execution_status"], "failed");
    assert!(!work.report().exists(), "an invalid ROI never publishes");
}

/// An ROI rectangle with a zero width or height is rejected.
#[test]
fn empty_roi_rectangles_are_rejected() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [30, 60, 90]);
    for rect in [vec![0, 0, 0, 2], vec![0, 0, 2, 0], vec![0, 0, 4]] {
        let manifest = work.write_manifest(&manifest_with(
            vec![json!({
                "id": "scan", "path": "scan.png", "label": "Scan",
                "rois": [{"id": "face", "label": "Face", "rect": rect}]
            })],
            None,
        ));
        let (output, result) = work.run(&manifest, &work.report());
        assert_eq!(output.status.code(), Some(1), "rect {rect:?}: {result}");
        assert!(!work.report().exists());
    }
}

/// Unsafe authored paths — absolute and parent-traversing — are rejected.
#[test]
fn unsafe_photo_paths_are_rejected() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [0, 0, 0]);
    for path in [
        work.path("scan.png").display().to_string(),
        "../scan.png".to_owned(),
        "nested/../../scan.png".to_owned(),
    ] {
        let manifest = work.write_manifest(&manifest_with(
            vec![photo_entry("scan", &path, "Original scan")],
            None,
        ));
        let (output, result) = work.run(&manifest, &work.report());
        assert_eq!(output.status.code(), Some(1), "path {path}: {result}");
        assert!(!work.report().exists());
    }
}

/// Duplicate authored identities are rejected: the manifest must not name one
/// photo or one ROI twice.
#[test]
fn duplicate_photo_and_roi_ids_are_rejected() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [0, 0, 0]);
    write_png(&work.path("copy.png"), 4, 4, [0, 0, 0]);
    let manifest = work.write_manifest(&manifest_with(
        vec![
            photo_entry("scan", "scan.png", "First"),
            photo_entry("scan", "copy.png", "Second"),
        ],
        None,
    ));
    let (output, result) = work.run(&manifest, &work.report());
    assert_eq!(output.status.code(), Some(1), "{result}");
    assert!(!work.report().exists());

    let duplicate_roi = work.write_manifest(&manifest_with(
        vec![json!({
            "id": "scan", "path": "scan.png", "label": "Original scan",
            "rois": [
                {"id": "face", "label": "Face", "rect": [0, 0, 2, 2]},
                {"id": "face", "label": "Face again", "rect": [1, 1, 2, 2]}
            ]
        })],
        None,
    ));
    let (output, result) = work.run(&duplicate_roi, &work.report());
    assert_eq!(output.status.code(), Some(1), "{result}");
    assert!(!work.report().exists());
}

/// Strict JSON: duplicate keys and unknown authored fields are rejected.
#[test]
fn duplicate_keys_and_unknown_fields_are_rejected() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [0, 0, 0]);
    let duplicate = work.path("duplicate.json");
    fs::write(
        &duplicate,
        br#"{"schema_version":1,"title":"T","title":"T again","photos":[{"id":"scan","path":"scan.png","label":"Scan"}]}"#,
    )
    .unwrap();
    let (output, result) = work.run(&duplicate, &work.report());
    assert_eq!(output.status.code(), Some(1), "{result}");
    assert!(!work.report().exists());

    let unknown = work.write_manifest(&json!({
        "schema_version": 1,
        "title": "T",
        "unexpected": true,
        "photos": [{"id": "scan", "path": "scan.png", "label": "Scan"}]
    }));
    let (output, result) = work.run(&unknown, &work.report());
    assert_eq!(output.status.code(), Some(1), "{result}");
    assert!(!work.report().exists());
}

/// An unsupported color declaration, or a non-boolean transfer declaration, is
/// rejected: the closed vocabulary is the product contract.
#[test]
fn unsupported_color_declarations_are_rejected() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [0, 0, 0]);
    for color_space in ["Rec. 709", "XYZ", "ACEScc", "romm rgb"] {
        let manifest = work.write_manifest(&manifest_with(
            vec![json!({
                "id": "scan", "path": "scan.png", "label": "Scan",
                "color_space": color_space
            })],
            None,
        ));
        let (output, result) = work.run(&manifest, &work.report());
        assert_eq!(output.status.code(), Some(1), "{color_space}: {result}");
        assert!(!work.report().exists());
    }
    let manifest = work.write_manifest(&manifest_with(
        vec![json!({
            "id": "scan", "path": "scan.png", "label": "Scan",
            "cctf_decoding": "yes"
        })],
        None,
    ));
    let (output, result) = work.run(&manifest, &work.report());
    assert_eq!(output.status.code(), Some(1), "{result}");
    assert!(!work.report().exists());
}

/// Every supported explicit declaration is accepted and retained.
#[test]
fn supported_color_declarations_are_retained() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [16, 32, 64]);
    for (index, color_space) in [
        "sRGB",
        "Display P3",
        "DCI-P3",
        "Adobe RGB (1998)",
        "ProPhoto RGB",
        "ITU-R BT.2020",
        "ACEScg",
        "ACES2065-1",
    ]
    .iter()
    .enumerate()
    {
        let manifest = work.write_manifest(&manifest_with(
            vec![json!({
                "id": "scan", "path": "scan.png", "label": "Scan",
                "color_space": color_space,
                "cctf_decoding": index % 2 == 0
            })],
            None,
        ));
        let out = work.path(&format!("report-{index}"));
        let (output, _) = work.run(&manifest, &out);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{color_space} must be supported: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = read_json(&out.join("report.json"));
        let scan = photo(&report, "scan");
        assert_eq!(scan["color"]["color_space"], *color_space);
        assert_eq!(scan["color"]["cctf_decoding"], index % 2 == 0);
        assert_eq!(scan["display"]["color_space"], "sRGB");
    }
}

/// A neighboring sidecar must not override the explicit declaration.
#[test]
fn a_sidecar_cannot_override_the_declared_color_source() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [80, 120, 160]);
    fs::write(
        work.path("scan.png.json"),
        br#"{"color_space":"ACEScg","cctf_decoding":false,"icc":"override"}"#,
    )
    .unwrap();
    let manifest = work.write_manifest(&manifest_with(
        vec![json!({
            "id": "scan", "path": "scan.png", "label": "Scan",
            "color_space": "Display P3",
            "cctf_decoding": true
        })],
        None,
    ));
    let (output, _) = work.run(&manifest, &work.report());
    assert_eq!(
        output.status.code(),
        Some(0),
        "the sidecar must not be consumed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = read_json(&work.report().join("report.json"));
    assert_eq!(photo(&report, "scan")["color"]["color_space"], "Display P3");
    assert_eq!(photo(&report, "scan")["color"]["cctf_decoding"], true);
}

/// Acceptance 11: the report opens after the original photo directory is
/// removed, using only the bundled display artifacts and `report.json`.
#[test]
fn the_report_survives_removal_of_the_source_directory() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 6, 6, [100, 150, 200]);
    let manifest = work.write_manifest(&manifest_with(
        vec![json!({
            "id": "scan", "path": "scan.png", "label": "Original scan",
            "rois": [{"id": "face", "label": "Face", "rect": [1, 1, 3, 3]}]
        })],
        None,
    ));
    work.publish(&manifest);

    // The complete output directory is the shareable artifact; copy it into a
    // second workspace, then destroy the workspace that held the sources.
    let carried = Workspace::new();
    let shared = carried.path("shared-report");
    copy_report(&work.report(), &shared);
    let report = read_json(&shared.join("report.json"));
    let shared_bytes = fs::read(shared.join(display_src(photo(&report, "scan")))).unwrap();
    drop(work);

    assert_eq!(report["mode"], "single_photo");
    assert!(shared.join("index.html").is_file());
    assert!(shared.join("viewer.js").is_file());
    assert_eq!(photo(&report, "scan")["width"], 6);
    assert_eq!(
        photo(&report, "scan")["source"]["sha256"]
            .as_str()
            .unwrap()
            .len(),
        64,
        "the source identity survives without the source"
    );
    assert_eq!(
        roi(photo(&report, "scan"), "face")["rect"],
        json!([1, 1, 3, 3])
    );
    assert!(shared_bytes.len() > 8);
}

/// The generator never modifies the manifest or the sources, and the published
/// source hash describes the bytes it read.
#[test]
fn generation_never_modifies_the_manifest_or_the_sources() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [11, 22, 33]);
    let manifest = work.write_manifest(&manifest_with(
        vec![json!({
            "id": "scan", "path": "scan.png", "label": "Scan",
            "rois": [{"id": "face", "label": "Face", "rect": [0, 0, 2, 2]}]
        })],
        None,
    ));
    let manifest_before = fs::read(&manifest).unwrap();
    let source_before = fs::read(work.path("scan.png")).unwrap();
    let report = work.publish(&manifest);
    assert_eq!(fs::read(&manifest).unwrap(), manifest_before);
    assert_eq!(fs::read(work.path("scan.png")).unwrap(), source_before);
    assert_eq!(
        photo(&report, "scan")["source"]["sha256"],
        common::sha256_file(&work.path("scan.png")).unwrap()
    );
    assert_eq!(
        report["source_manifest"]["sha256"].as_str(),
        Some(common::sha256_file(&manifest).unwrap().as_str()),
        "the manifest identity is retained"
    );
}

/// A rewritten source publishes its new identity: the hash is read live, never
/// cached from an earlier run.
#[test]
fn a_rewritten_source_publishes_its_new_identity() {
    let work = Workspace::new();
    let source = work.path("scan.png");
    write_png(&source, 4, 4, [10, 10, 10]);
    let manifest = work.write_manifest(&manifest_with(
        vec![photo_entry("scan", "scan.png", "Original scan")],
        None,
    ));
    let first = work.path("first");
    let (output, _) = work.run(&manifest, &first);
    assert_eq!(output.status.code(), Some(0));
    let first_hash = photo(&read_json(&first.join("report.json")), "scan")["source"]["sha256"]
        .as_str()
        .unwrap()
        .to_owned();

    write_png(&source, 4, 4, [240, 240, 240]);
    let second = work.path("second");
    let (output, _) = work.run(&manifest, &second);
    assert_eq!(output.status.code(), Some(0));
    let report = read_json(&second.join("report.json"));
    let second_hash = photo(&report, "scan")["source"]["sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(first_hash, second_hash, "the source hash is read live");
    assert_eq!(
        second_hash,
        common::sha256_file(&source).unwrap(),
        "the published hash describes the current source bytes"
    );
    let display = second.join(display_src(photo(&report, "scan")));
    assert_eq!(
        photo(&report, "scan")["display"]["sha256"].as_str(),
        Some(common::sha256_file(&display).unwrap().as_str())
    );
}

/// An existing destination is rejected without modification.
#[test]
fn an_existing_destination_is_rejected_without_modification() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [10, 20, 30]);
    let manifest = work.write_manifest(&manifest_with(
        vec![photo_entry("scan", "scan.png", "Original scan")],
        None,
    ));
    let out = work.report();
    fs::create_dir_all(&out).unwrap();
    let sentinel = out.join("keep.txt");
    fs::write(&sentinel, b"untouched").unwrap();
    let (output, result) = work.run(&manifest, &out);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(result["execution_status"], "failed");
    assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");
    assert!(!out.join("index.html").exists(), "nothing may be published");
}

/// A missing or unreadable source fails before publication.
#[test]
fn a_missing_source_fails_before_publication() {
    let work = Workspace::new();
    let manifest = work.write_manifest(&manifest_with(
        vec![photo_entry("scan", "absent.png", "Original scan")],
        None,
    ));
    let (output, result) = work.run(&manifest, &work.report());
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(result["execution_status"], "failed");
    assert!(!work.report().exists());
}

/// A rejected invocation is an argument syntax error and exits 2.
#[test]
fn argument_syntax_errors_exit_two() {
    for args in [
        vec!["report", "photos"],
        vec!["report", "photos", "--manifest"],
        vec!["report", "photos", "--out", "report"],
        vec![
            "report",
            "photos",
            "--manifest",
            "m.json",
            "--out",
            "r",
            "--extra",
            "x",
        ],
        vec!["report"],
        vec!["photos", "--manifest", "m.json", "--out", "r"],
        Vec::<&str>::new(),
    ] {
        let output = cli().args(&args).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "clap must reject {args:?} as a syntax error"
        );
        assert!(
            output.stdout.is_empty(),
            "a rejected invocation emits no result on stdout"
        );
    }
}

/// Acceptance 13: the published report carries no metric, score, ranking, or
/// registration language, and the page loads no network origin.
#[test]
fn the_report_carries_no_metric_or_network_dependency() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [10, 10, 10]);
    write_png(&work.path("sim.png"), 4, 4, [240, 240, 240]);
    let manifest = work.write_manifest(&manifest_with(
        vec![
            json!({
                "id": "scan", "path": "scan.png", "label": "Original scan",
                "rois": [{"id": "face", "label": "Face", "rect": [0, 0, 2, 2]}]
            }),
            json!({
                "id": "sim", "path": "sim.png", "label": "Simulation output",
                "rois": [{"id": "face", "label": "Face", "rect": [1, 1, 2, 2]}]
            }),
        ],
        None,
    ));
    work.publish(&manifest);
    let html = fs::read_to_string(work.report().join("index.html"))
        .unwrap()
        .to_ascii_lowercase();
    let terminal = fs::read_to_string(work.report().join("report.json"))
        .unwrap()
        .to_ascii_lowercase();
    for needle in [
        "delta_e",
        "rmse",
        "flip",
        "mtf",
        "score",
        "ranking",
        "recommend",
        "acceptance",
        "registration",
        "http://",
        "https://",
        "//cdn",
        "fetch(",
        "xmlhttprequest",
    ] {
        assert!(!html.contains(needle), "the page must not contain {needle}");
        assert!(
            !terminal.contains(needle),
            "report.json must not contain {needle}"
        );
    }
}

/// Acceptance 14: hostile authored text is carried as data, never as markup,
/// and cannot terminate the embedded payload element.
#[test]
fn hostile_authored_text_is_not_executable() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [10, 10, 10]);
    let hostile = "</script><script>alert(1)</script>";
    let manifest = work.write_manifest(&json!({
        "schema_version": 1,
        "title": hostile,
        "description": "<img src=x onerror=alert(2)>",
        "photos": [{
            "id": "scan", "path": "scan.png", "label": hostile,
            "note": "<svg onload=alert(3)>",
            "rois": [{"id": "face", "label": hostile, "rect": [0, 0, 2, 2], "note": hostile}]
        }]
    }));
    let (output, _) = work.run(&manifest, &work.report());
    assert_eq!(
        output.status.code(),
        Some(0),
        "hostile text is authored data, not a rejected manifest: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let html = fs::read_to_string(work.report().join("index.html")).unwrap();
    let embedded = html
        .split("photo-report-data\">")
        .nth(1)
        .expect("the page embeds its payload")
        .split("</script>")
        .next()
        .unwrap();
    let payload: Value = serde_json::from_str(embedded).unwrap();
    assert_eq!(payload["title"], hostile);
    assert_eq!(payload["photos"][0]["label"], hostile);
    assert_eq!(payload["photos"][0]["rois"][0]["label"], hostile);
    assert!(
        !embedded.contains("</script"),
        "an embedded payload must not be able to terminate its element"
    );
    assert!(
        embedded.contains("\\u003c"),
        "the payload escapes < so no authored text can close the element"
    );
    let outside = format!(
        "{}{}",
        html.split("photo-report-data\">").next().unwrap(),
        html.split("</script>").last().unwrap()
    );
    assert!(
        !outside.contains("<script>alert"),
        "hostile text must not appear as executable markup"
    );
}

/// A manifest path that does not exist is a failure with a stable code.
#[test]
fn a_missing_manifest_is_a_failure() {
    let work = Workspace::new();
    let (output, result) = work.run(&work.path("absent.json"), &work.report());
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(result["execution_status"], "failed");
    assert!(result["errors"][0]["code"].as_str().is_some());
    assert!(!work.report().exists());
}

/// Zero photos, a bad schema version, and an empty title are rejected.
#[test]
fn required_authored_fields_are_enforced() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [1, 1, 1]);
    for manifest in [
        json!({"schema_version": 1, "title": "T", "photos": []}),
        json!({"schema_version": 2, "title": "T", "photos": [photo_entry("scan", "scan.png", "Scan")]}),
        json!({"schema_version": 1, "title": "", "photos": [photo_entry("scan", "scan.png", "Scan")]}),
        json!({"schema_version": 1, "photos": [photo_entry("scan", "scan.png", "Scan")]}),
    ] {
        let path = work.write_manifest(&manifest);
        let (output, result) = work.run(&path, &work.report());
        assert_eq!(output.status.code(), Some(1), "{manifest}: {result}");
        assert!(!work.report().exists());
    }
}

/// One deterministic run: the same manifest and sources publish the same
/// display bytes, so a shared report is reproducible.
#[test]
fn generation_is_deterministic() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 6, 4, [12, 34, 56]);
    let manifest = work.write_manifest(&manifest_with(
        vec![json!({
            "id": "scan", "path": "scan.png", "label": "Scan",
            "rois": [{"id": "face", "label": "Face", "rect": [1, 1, 3, 2]}]
        })],
        None,
    ));
    let first = work.path("first");
    let second = work.path("second");
    let (output, _) = work.run(&manifest, &first);
    assert_eq!(output.status.code(), Some(0));
    let (output, _) = work.run(&manifest, &second);
    assert_eq!(output.status.code(), Some(0));
    let a = read_json(&first.join("report.json"));
    let b = read_json(&second.join("report.json"));
    assert_eq!(a["mode"], b["mode"]);
    assert_eq!(
        display_src(photo(&a, "scan")),
        display_src(photo(&b, "scan"))
    );
    assert_eq!(a["photos"], b["photos"]);
    assert_eq!(a["comparisons"], b["comparisons"]);
    let left = first.join(display_src(photo(&a, "scan")));
    let right = second.join(display_src(photo(&b, "scan")));
    assert_eq!(
        fs::read(&left).unwrap(),
        fs::read(&right).unwrap(),
        "the display copy is a deterministic function of the declared source"
    );
}

/// The ROI union order follows the left photo's authored order, then the
/// right-only identities.
#[test]
fn the_roi_union_follows_left_order_then_right_only_ids() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 8, 8, [1, 2, 3]);
    write_png(&work.path("sim.png"), 8, 8, [4, 5, 6]);
    let manifest = work.write_manifest(&manifest_with(
        vec![
            json!({
                "id": "scan", "path": "scan.png", "label": "Scan",
                "rois": [
                    {"id": "second", "label": "Second", "rect": [0, 0, 2, 2]},
                    {"id": "first", "label": "First", "rect": [2, 2, 2, 2]}
                ]
            }),
            json!({
                "id": "sim", "path": "sim.png", "label": "Sim",
                "rois": [
                    {"id": "first", "label": "First", "rect": [0, 0, 3, 3]},
                    {"id": "right-only", "label": "Right only", "rect": [4, 4, 3, 3]}
                ]
            }),
        ],
        None,
    ));
    let report = work.publish(&manifest);
    let pair = &report["comparisons"].as_array().unwrap()[0];
    let ids: Vec<&str> = pair["rois"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["second", "first", "right-only"],
        "left authored order first, then the right-only identities"
    );
    assert!(!unavailable_reason(&resolved(pair, "right-only")["left"]).is_empty());
    assert_eq!(
        rect_of(&resolved(pair, "right-only")["right"]),
        vec![4, 4, 3, 3]
    );
    assert_eq!(
        comparison(&report, pair["id"].as_str().unwrap())["left"],
        "scan"
    );
}

/// Acceptance 9: a shared ROI with different coordinates stays a two-sided view
/// with both native rectangles retained.
#[test]
fn a_shared_roi_with_different_rectangles_keeps_both_native_rectangles() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 16, 12, [200, 100, 50]);
    write_png(&work.path("sim.png"), 12, 16, [50, 100, 200]);
    let manifest = work.write_manifest(&manifest_with(
        vec![
            json!({
                "id": "scan", "path": "scan.png", "label": "Scan",
                "rois": [{"id": "face", "label": "Face", "rect": [1, 1, 10, 8]}]
            }),
            json!({
                "id": "sim", "path": "sim.png", "label": "Sim",
                "rois": [{"id": "face", "label": "Face", "rect": [3, 2, 6, 5]}]
            }),
        ],
        None,
    ));
    let report = work.publish(&manifest);
    let scan = photo(&report, "scan");
    let sim = photo(&report, "sim");
    let face = resolved(&report["comparisons"].as_array().unwrap()[0], "face");
    assert_eq!(rect_of(&face["left"]), vec![1, 1, 10, 8]);
    assert_eq!(rect_of(&face["right"]), vec![3, 2, 6, 5]);
    assert_eq!(dimension(&face["left"], 0), 10);
    assert_eq!(dimension(&face["left"], 1), 8);
    assert_eq!(dimension(&face["right"], 0), 6);
    assert_eq!(dimension(&face["right"], 1), 5);
    assert_eq!(display_artifact_of(&face["left"]), display_src(scan));
    assert_eq!(display_artifact_of(&face["right"]), display_src(sim));
    assert_eq!(
        face["left"]["availability"], "available",
        "a shared ROI is available on both sides"
    );
}

/// Labels, notes, and paths are plain text: an authored label that names a
/// role stays the author's words, and the report never invents a baseline or
/// candidate judgment of its own.
#[test]
fn the_report_keeps_the_authored_pair_fixed_and_ordered() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [0, 0, 0]);
    write_png(&work.path("sim.png"), 4, 4, [255, 255, 255]);
    let manifest = work.write_manifest(&manifest_with(
        vec![
            json!({"id": "scan", "path": "scan.png", "label": "Baseline scan", "note": "First."}),
            photo_entry("sim", "sim.png", "Simulation output"),
        ],
        None,
    ));
    let report = work.publish(&manifest);
    let pair = &report["comparisons"].as_array().unwrap()[0];
    assert_eq!(pair["left"], "scan", "the pair stays in authored order");
    assert_eq!(pair["right"], "sim");
    assert_eq!(photo(&report, "scan")["label"], "Baseline scan");
    assert_eq!(photo(&report, "scan")["note"], "First.");
    assert_eq!(
        report["photos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|photo| photo["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["scan", "sim"],
        "photos stay in authored order"
    );
}

/// A comparison entry may carry an optional unique identity, label, and note.
#[test]
fn declared_comparison_metadata_is_retained() {
    let work = Workspace::new();
    for name in ["a.png", "b.png", "c.png"] {
        write_png(&work.path(name), 4, 4, [3, 3, 3]);
    }
    let manifest = work.write_manifest(&manifest_with(
        vec![
            photo_entry("a", "a.png", "Photo A"),
            photo_entry("b", "b.png", "Photo B"),
            photo_entry("c", "c.png", "Photo C"),
        ],
        Some(json!([
            {"id": "first-pair", "left": "a", "right": "b", "label": "First", "note": "Scan versus simulation."},
            {"id": "second-pair", "left": "b", "right": "c"}
        ])),
    ));
    let report = work.publish(&manifest);
    let first = comparison(&report, "first-pair");
    assert_eq!(first["left"], "a");
    assert_eq!(first["right"], "b");
    assert_eq!(first["label"], "First");
    assert_eq!(first["note"], "Scan versus simulation.");
    let second = comparison(&report, "second-pair");
    assert_eq!(second["left"], "b");
    assert_eq!(second["right"], "c");
    assert_eq!(report["comparisons"].as_array().unwrap().len(), 2);
}

/// Duplicate comparison identities are rejected.
#[test]
fn duplicate_comparison_ids_are_rejected() {
    let work = Workspace::new();
    for name in ["a.png", "b.png", "c.png"] {
        write_png(&work.path(name), 4, 4, [3, 3, 3]);
    }
    let manifest = work.write_manifest(&manifest_with(
        vec![
            photo_entry("a", "a.png", "Photo A"),
            photo_entry("b", "b.png", "Photo B"),
            photo_entry("c", "c.png", "Photo C"),
        ],
        Some(json!([
            {"id": "pair", "left": "a", "right": "b"},
            {"id": "pair", "left": "b", "right": "c"}
        ])),
    ));
    let (output, result) = work.run(&manifest, &work.report());
    assert_eq!(output.status.code(), Some(1), "{result}");
    assert!(!work.report().exists());
}

/// The result on stdout is exactly one machine-readable JSON object.
#[test]
fn a_successful_result_is_one_machine_json_object_on_stdout() {
    let work = Workspace::new();
    write_png(&work.path("scan.png"), 4, 4, [5, 6, 7]);
    let manifest = work.write_manifest(&manifest_with(
        vec![photo_entry("scan", "scan.png", "Original scan")],
        None,
    ));
    let out = work.report();
    let (output, result) = work.run(&manifest, &out);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["command"], "report photos");
    assert_eq!(result["mode"], "single_photo");
    assert_eq!(result["execution_status"], "completed");
    assert_eq!(
        result["artifact"].as_str(),
        Some(out.join("index.html").to_str().unwrap())
    );
    assert_eq!(
        result["manifest"].as_str(),
        Some(out.join("report.json").to_str().unwrap())
    );
    // One JSON document on stdout: a trailing second document would not parse.
    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parsed["command"], "report photos");
}
