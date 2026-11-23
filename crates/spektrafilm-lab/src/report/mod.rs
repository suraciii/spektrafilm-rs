//! Portable offline presentation of retained evidence; no rendering or measurement.
use crate::{
    common::{self, Error, Result, create_destination, destination_exists, dump, sha256_file},
    documents::{load_json_document, safe_artifact_path, validate_generated_document},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

mod assets;
mod escape;
mod manifest;
mod render;

pub(crate) use assets::*;
pub(crate) use escape::*;
pub(crate) use manifest::DEFAULT_TITLE;
use manifest::{Case, Image, Manifest, View};
#[cfg(test)]
pub(crate) use render::rectangle;
pub(crate) use render::{geometry_rectangle, page};

/// Publish a self-contained offline report from a validated comparison directory.
///
/// Offline navigation uses `#case=<id>&seed=<seed>[&pair=<id>[&region=<id>]]`.
/// Every value is percent-encoded UTF-8 and denotes the exact retained identity.
/// The seed is its decimal representation; a region requires an explicit pair.
/// An omitted pair selects the retained default (or the first retained pair when
/// absent); version comparisons use `baseline/candidate`. Unknown identities,
/// duplicate/unknown keys, empty values and malformed encoding show an alert and
/// hide the case panels until a valid link or case selection is chosen.
///
/// The returned value is the authoritative command result, including the
/// published artifact and manifest identities.
pub fn build_report(comparison: &Path, out: &Path) -> Result<Value> {
    let root = crate::documents::normalize_path(out)?;
    let preexisting = fs::symlink_metadata(out).is_ok();
    let result = build_report_impl(comparison, out);
    if preexisting {
        result
    } else {
        result.map_err(|error| common::incomplete_destination(error, &root, "report.json"))
    }
}

fn build_report_impl(comparison: &Path, out: &Path) -> Result<Value> {
    if fs::symlink_metadata(out).is_ok() {
        return Err(destination_exists(out));
    }
    let root = comparison.canonicalize()?;
    let document = safe_artifact_path(&root, "comparison.json")?;
    let snapshot = fs::read(&document)?;
    let comparison = load_json_document(&document)?;
    if comparison["schema_version"] != 1 {
        return Err(Error::new(
            "unsupported_schema",
            "report requires comparison schema_version 1",
        ));
    }
    validate_generated_document("comparison", &comparison, &root)?;
    let cases = comparison["cases"]
        .as_array()
        .ok_or_else(|| Error::new("invalid_document", "comparison must contain a cases array"))?;
    let records = normalized_artifacts(&comparison["artifacts"])?;
    let mut assets = Assets::new(&root, out, records)?;
    create_destination(out)?;
    fs::create_dir_all(out.join("artifacts"))?;
    for record in assets.records.clone() {
        assets.copy(&record)?;
    }
    fs::write(out.join("comparison.json"), &snapshot)?;
    assets.manifest.push(json!({"path":"comparison.json","source_path":"comparison.json","sha256":format!("{:x}",Sha256::digest(&snapshot)),"role":"comparison","media_type":"application/json","availability":"available"}));
    let ordered: Vec<_> = cases.iter().collect();
    if ordered.iter().any(|c| !c.is_object()) {
        return Err(Error::new(
            "invalid_document",
            "case result must be an object",
        ));
    }
    let (manifest, sizes) = manifest_from(&comparison, &mut assets)?;
    manifest.validate()?;
    dump(&out.join("page.json"), &manifest.to_json())?;
    // The script URL carries its own hash: a stale copy in any cache is a
    // different URL, not a different behaviour on the same URL.
    let version = format!("{:x}", Sha256::digest(JS.as_bytes()));
    let page = page(&manifest, &sizes, &version[..8]);
    publish(out, &comparison, &assets, &page)
}

/// Project a comparison record into the page model.
///
/// The projection is deliberately lossy: measurements, statuses, reasons,
/// masks, coordinates and configuration stay in the record and in
/// `report.json`. What crosses into the page is a reference, its variants, and
/// the windows a person can look at.
fn manifest_from(
    comparison: &Value,
    assets: &mut Assets,
) -> Result<(Manifest, BTreeMap<String, [u32; 2]>)> {
    let mut sizes = BTreeMap::new();
    let mut cases = Vec::new();
    let mut expanded: Vec<Value> = Vec::new();
    for case in rows(&comparison["cases"]) {
        if !case.is_object() {
            continue;
        }
        expanded.push(case.clone());
    }
    for case in &expanded {
        let base = get(case, "case_id", get(case, "id", "unavailable")).to_owned();
        let (id, title) = (base.clone(), human(&base));
        let labels = ("基准", "候选");
        let mut sides = Vec::new();
        for (role, label) in [("baseline", labels.0), ("candidate", labels.1)] {
            let reference = role_reference(case, role, assets);
            let Some(mut entry) = assets.image(&reference)? else {
                continue;
            };
            if let Some(preview) = entry.get("fit_preview").cloned()
                && let Some(fit) = assets.image(&preview)?
            {
                entry["fit_entry"] = fit;
            }
            let path = get(&entry, "path", "").to_owned();
            if path.is_empty() {
                continue;
            }
            let fit = entry
                .get("fit_entry")
                .filter(|value| !value.is_null() && value["path"] != entry["path"])
                .map(|fit| get(fit, "path", "").to_owned())
                .filter(|fit| !fit.is_empty());
            if !sizes.contains_key(&path) {
                let dimensions = image::image_dimensions(assets.out.join(&path))
                    .map_err(|e| Error::new("invalid_artifact", e.to_string()))?;
                sizes.insert(path.clone(), [dimensions.0, dimensions.1]);
            }
            sides.push(Image {
                path,
                fit,
                label: label.to_owned(),
            });
        }
        let Some(reference) = sides.first().cloned() else {
            continue;
        };
        let variants = sides.get(1..).map(<[Image]>::to_vec).unwrap_or_default();
        let mut views = Vec::new();
        views.push(View {
            id: "full".into(),
            label: "全图".into(),
            crop: None,
            notes: Vec::new(),
        });
        let mut regions: Vec<&Value> = rows(&case["regions"]).iter().collect();
        regions.extend(rows(
            case.get("automatic_regions")
                .or_else(|| case.get("hotspots"))
                .unwrap_or(&Value::Null),
        ));
        regions.extend(rows(&case["detail_regions"]));
        for region in regions {
            let geometry = region.get("geometry").unwrap_or(&Value::Null);
            let Some([x, y, width, height]) = geometry_rectangle(geometry, assets)? else {
                continue;
            };
            let id = get(region, "id", get(region, "region_id", "region")).to_owned();
            // A retained mask can be narrower than its bounding rectangle; the
            // page shows the rectangle the record declares.
            views.push(View {
                id: id.clone(),
                label: get(region, "label", &id).to_owned(),
                crop: Some([x, y, width, height]),
                notes: Vec::new(),
            });
        }
        cases.push(Case {
            title,
            id,
            aligned: case["pixel_aligned"] != false,
            reference,
            variants,
            views,
            notes: Vec::new(),
        });
    }
    cases.retain(|case| !case.views.is_empty());
    Ok((
        Manifest {
            title: DEFAULT_TITLE.to_owned(),
            cases,
        },
        sizes,
    ))
}

fn publish(out: &Path, comparison: &Value, assets: &Assets, page: &str) -> Result<Value> {
    fs::write(out.join("index.html"), page)?;
    // The viewer is a separate file. A static host may pin the scripts it allows
    // by hash, and an inline script the host never hashed does not run at all;
    // a same-origin file is what `script-src 'self'` already permits.
    fs::write(out.join("viewer.js"), JS)?;
    // Bootstrap ships as files next to the page: a static host that pins scripts
    // by hash or by `'self'` still serves them, and a 300 KB library stays out of
    // every regenerated HTML file.
    fs::write(out.join("bootstrap.css"), VENDOR_CSS)?;
    fs::write(out.join("bootstrap.bundle.js"), VENDOR_JS)?;
    let mut manifest = assets.manifest.clone();
    manifest.push(json!({"path":"viewer.js","sha256":sha256_file(&out.join("viewer.js"))?,"role":"viewer_script","media_type":"text/javascript","availability":"available"}));
    manifest.push(json!({"path":"bootstrap.css","sha256":sha256_file(&out.join("bootstrap.css"))?,"role":"vendor_stylesheet","media_type":"text/css","availability":"available"}));
    manifest.push(json!({"path":"bootstrap.bundle.js","sha256":sha256_file(&out.join("bootstrap.bundle.js"))?,"role":"vendor_script","media_type":"text/javascript","availability":"available"}));
    manifest.push(json!({"path":"index.html","sha256":sha256_file(&out.join("index.html"))?,"role":"report","media_type":"text/html","availability":"available"}));
    manifest.push(json!({"path":"page.json","sha256":sha256_file(&out.join("page.json"))?,"role":"page_manifest","media_type":"application/json","availability":"available"}));
    dump(
        &out.join("report.json"),
        &json!({"schema_version":1,"command":"report","source":"comparison","execution_status":"completed","comparison_id":comparison.get("id"),"artifact":"index.html","artifacts":manifest}),
    )?;
    Ok(json!({
        "schema_version":1,
        "command":"report",
        "source":"comparison",
        "execution_status":"completed",
        "artifact":crate::documents::normalize_path(out)?.join("index.html"),
        "manifest":crate::documents::normalize_path(out)?.join("report.json"),
        "comparison_id":comparison.get("id"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redistribution_denial_never_copies_bytes() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::write(source.path().join("private.png"), b"private").unwrap();
        let mut assets = Assets::new(
            source.path(),
            out.path(),
            vec![json!({"id":"private","path":"private.png","redistribution":false})],
        )
        .unwrap();
        let entry = assets.copy(&json!("private")).unwrap().unwrap();
        assert_eq!(entry["reason"], "redistribution_not_permitted");
        assert!(!out.path().join("private.png").exists());
    }
    #[test]
    fn hash_mismatch_is_rejected_before_publication() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::write(source.path().join("data.json"), b"{}").unwrap();
        let mut assets = Assets::new(
            source.path(),
            out.path(),
            vec![json!({"path":"data.json","sha256":"incorrect"})],
        )
        .unwrap();
        assert_eq!(
            assets.copy(&json!("data.json")).unwrap_err().code,
            "artifact_hash"
        );
        assert!(!out.path().join("data.json").exists());
    }
    #[test]
    fn a_case_finds_its_own_artifacts() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::create_dir(out.path().join("artifacts")).unwrap();
        for name in ["baseline.png", "candidate.png"] {
            crate::image_data::write_png16(
                &source.path().join(name),
                &[[0.25, 0.25, 0.25]; 4],
                2,
                2,
            )
            .unwrap();
        }
        let assets = Assets::new(
            source.path(),
            out.path(),
            vec![
                json!({"id":"b","path":"baseline.png","role":"baseline","case_id":"blue","seed":0}),
                json!({"id":"c","path":"candidate.png","role":"candidate","case_id":"blue","seed":0}),
            ],
        )
        .unwrap();
        let case = json!({"case_id": "blue", "status": "completed"});
        assert!(role_reference(&case, "baseline", &assets)["path"] == json!("baseline.png"));
        assert!(role_reference(&case, "candidate", &assets)["path"] == json!("candidate.png"));
        // Another case borrows nothing: the evidence belongs to this one.
        let other = json!({"case_id": "green", "status": "completed"});
        assert!(role_reference(&other, "baseline", &assets).is_null());
    }

    #[test]
    fn corresponding_region_masks_keep_independent_crop_geometry() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::create_dir(out.path().join("artifacts")).unwrap();
        crate::image_data::write_npy(
            &source.path().join("mask.npy"),
            &[0.0, 1.0, 1.0, 0.0, 0.0, 0.0],
            &[2, 3],
        )
        .unwrap();
        let mut assets =
            Assets::new(source.path(), out.path(), vec![json!({"path":"mask.npy"})]).unwrap();
        assert_eq!(
            geometry_rectangle(&json!({"path":"mask.npy"}), &mut assets).unwrap(),
            Some([1, 0, 2, 1])
        );
        assert_eq!(
            geometry_rectangle(&json!([0, 1, 1, 1]), &mut assets).unwrap(),
            Some([0, 1, 1, 1])
        );
    }

    #[test]
    fn reference_difference_display_keeps_valid_mask_exclusions() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::create_dir(out.path().join("artifacts")).unwrap();
        crate::image_data::write_npy(&source.path().join("map.npy"), &[1.0, 2.0], &[1, 2]).unwrap();
        crate::image_data::write_npy(&source.path().join("mask.npy"), &[1.0, 0.0], &[1, 2])
            .unwrap();
        let mut assets = Assets::new(source.path(), out.path(), vec![json!({"id":"map","path":"map.npy","role":"difference_map","metric":"delta_e00","valid_mask_artifact_id":"mask"}),json!({"id":"mask","path":"mask.npy"})]).unwrap();
        let display = assets.image(&json!("map")).unwrap().unwrap();
        let pixels = image::open(out.path().join(display["path"].as_str().unwrap()))
            .unwrap()
            .to_rgba8();
        assert_eq!(pixels.get_pixel(0, 0)[3], 255);
        assert_eq!(pixels.get_pixel(1, 0)[3], 0);
    }

    #[test]
    fn malformed_rectangles_are_errors() {
        assert_eq!(
            rectangle(&json!({"geometry":[0,0,0,10]})).unwrap_err().code,
            "region_bounds"
        );
        assert!(rectangle(&json!({"geometry":[1,2,3,4]})).unwrap().is_some());
    }
}
