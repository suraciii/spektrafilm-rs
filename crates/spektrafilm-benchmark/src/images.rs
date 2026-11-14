//! `compare images`: quantify two delivered image sets.
//!
//! The comparison uses the version shape and the same metric engine as a
//! version comparison, but its two sides are decoded files instead of
//! retained runs. It publishes `baseline` and `candidate` source blocks of
//! `kind: "images"` and never fabricates a run, a target, a suite, or an
//! executed command.
//!
//! Decoding goes through [`image_data::decode_raster`], the lab reader. It
//! merges no sidecar, applies no ICC profile or orientation, and rejects a
//! container layout it cannot state.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::common::{self, Error, Evidence, Result};
use crate::compare::{publish_measurement_findings, relative_paths, retain_metrics, review_image};
use crate::documents;
use crate::image_data;
use crate::metrics;
use crate::runner;

/// One declared image pair.
pub struct Pair {
    pub case_id: String,
    pub baseline: PathBuf,
    pub candidate: PathBuf,
}

/// One `compare images` invocation.
pub struct Request {
    pub baseline_id: String,
    pub candidate_id: String,
    pub color_space: String,
    pub cctf_decoding: bool,
    pub pairs: Vec<Pair>,
    pub regions: Vec<Value>,
    pub protocol: Value,
    /// Retained difference maps are box-averaged to one sample per `N × N`
    /// block. Measurement populations are never reduced.
    pub map_scale: u32,
}

/// A declared identity made safe for a document id.
fn slug(value: &str) -> String {
    let mut out: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if !out
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        out.insert(0, 'r');
    }
    out
}

fn artifact_extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{}", value.to_ascii_lowercase()))
        .unwrap_or_default()
}

/// A pseudo-suite identity: the protocol digest covers the protocol, the
/// regions, and the selected families, exactly as it does for a run.
fn protocol_digest(request: &Request) -> Result<String> {
    let cases: Vec<Value> = request
        .pairs
        .iter()
        .map(|pair| {
            json!({
                "id": pair.case_id,
                "seeds": [0],
                "regions": request.regions,
                "measurements": request.protocol["measurements"],
            })
        })
        .collect();
    documents::protocol_digest(&json!({"protocol": request.protocol, "cases": cases}))
}

fn source_block(
    id: &str,
    request: &Request,
    role: &str,
    files: &[(String, Value)],
) -> Result<Value> {
    let declaration = json!({
        "color_space": request.color_space,
        "cctf_decoding": request.cctf_decoding,
    });
    let list: Vec<Value> = files
        .iter()
        .map(|(case_id, record)| {
            json!({
                "case_id": case_id,
                "path": record["path"],
                "sha256": record["sha256"],
                "width": record["width"],
                "height": record["height"],
                "role": role,
            })
        })
        .collect();
    Ok(json!({
        "kind": "images",
        "id": id,
        "declaration": declaration,
        "files": list,
        "declaration_sha256": common::digest(&json!({"declaration": declaration, "files": list}))?,
    }))
}

/// Validate the declared geometry before a pixel is read: a delivered raster
/// carries only render coordinates, and `full_image` is the implicit
/// population rather than an authored identity.
fn validate_regions(regions: &[Value]) -> Result<()> {
    for region in regions {
        let id = region["id"].as_str().unwrap_or_default();
        if id.is_empty() {
            return Err(Error::new(
                "invalid_region_identity",
                "Every region needs a nonempty id",
            ));
        }
        if id == "full_image" {
            return Err(Error::new(
                "invalid_region_identity",
                "`full_image` is the implicit population and is reserved",
            ));
        }
        let space = region["geometry"]["space"].as_str().unwrap_or_default();
        if space != "render" {
            return Err(Error::new(
                "unsupported_coordinate_space",
                format!(
                    "Region {id} declares {space:?} geometry; a delivered raster has only render coordinates"
                ),
            )
            .at(id));
        }
    }
    Ok(())
}

/// A rect outside the delivered image is an authoring error, not a reading.
fn check_bounds(regions: &[Value], width: usize, height: usize, case_id: &str) -> Result<()> {
    for region in regions {
        let id = region["id"].as_str().unwrap_or_default();
        let geometry = &region["geometry"];
        let number = |key: &str| geometry[key].as_u64().unwrap_or_default();
        let (x, y) = (number("x"), number("y"));
        let (w, h) = (number("width"), number("height"));
        if w == 0 || h == 0 || x + w > width as u64 || y + h > height as u64 {
            return Err(Error::new(
                "region_out_of_bounds",
                format!(
                    "Region {id} {x},{y} {w}x{h} leaves case {case_id} bounds {width}x{height}"
                ),
            )
            .at(case_id));
        }
    }
    Ok(())
}

/// Run one image comparison and publish its directory.
pub fn compare(request: &Request, out: &Path) -> Result<(Value, i32)> {
    let root = documents::normalize_path(out)?;
    let preexisting = std::fs::symlink_metadata(out).is_ok();
    let result = compare_impl(request, out);
    if preexisting {
        result
    } else {
        result.map_err(|error| common::incomplete_destination(error, &root, "comparison.json"))
    }
}

fn compare_impl(request: &Request, out: &Path) -> Result<(Value, i32)> {
    if request.pairs.is_empty() {
        return Err(Error::new("missing_image", "No image pair declared"));
    }
    validate_regions(&request.regions)?;
    let root = documents::normalize_path(out)?;
    common::create_destination(&root)?;
    let mut evidence = Evidence::new(&root);
    common::dump(&root.join("protocol.json"), &request.protocol)?;
    evidence.add(
        &root.join("protocol.json"),
        "protocol",
        "application/json",
        json!({"id": request.protocol["id"]}),
    )?;
    if !request.regions.is_empty() {
        let regions = json!({
            "schema_version": 1,
            "id": slug(&format!("{}-{}", request.baseline_id, request.candidate_id)),
            "regions": request.regions,
        });
        common::dump(&root.join("regions.json"), &regions)?;
        evidence.add(
            &root.join("regions.json"),
            "regions",
            "application/json",
            json!({"id": regions["id"]}),
        )?;
    }
    let mut cases = Vec::new();
    let mut findings = Vec::new();
    let mut differences = Vec::new();
    let mut baseline_files = Vec::new();
    let mut candidate_files = Vec::new();
    for (field, value) in [
        ("color_space", json!(request.color_space)),
        ("cctf_decoding", json!(request.cctf_decoding)),
    ] {
        differences.push(json!({
            "field": field,
            "baseline": value,
            "candidate": value,
            "declared": true,
        }));
    }
    for pair in &request.pairs {
        let folder = root.join("cases").join(&pair.case_id);
        std::fs::create_dir_all(&folder)?;
        let sides = [("baseline", &pair.baseline), ("candidate", &pair.candidate)];
        let mut images = Vec::new();
        let mut records = Vec::new();
        for (role, path) in sides {
            let image =
                image_data::decode_raster(path, &request.color_space, request.cctf_decoding)?
                    .into_linear()?;
            let source = evidence.copy(
                path,
                &format!(
                    "cases/{}/{}-source{}",
                    pair.case_id,
                    role,
                    artifact_extension(path)
                ),
                "source_image",
                json!({
                    "case_id": pair.case_id,
                    "role": role,
                    "color_space": request.color_space,
                    "cctf_decoding": request.cctf_decoding,
                    "width": image.width,
                    "height": image.height,
                    "dimensions": [image.width, image.height],
                }),
            )?;
            records.push((pair.case_id.clone(), source.clone()));
            images.push((role, image, source));
        }
        let (baseline_role, baseline, baseline_source) = &images[0];
        let (candidate_role, candidate, candidate_source) = &images[1];
        debug_assert_eq!(*baseline_role, "baseline");
        debug_assert_eq!(*candidate_role, "candidate");
        if baseline.dimensions() != candidate.dimensions() {
            return Err(Error::new(
                "dimension_mismatch",
                format!(
                    "Pair {} differs in dimensions: {}x{} and {}x{}",
                    pair.case_id,
                    baseline.width,
                    baseline.height,
                    candidate.width,
                    candidate.height
                ),
            )
            .at(&pair.case_id));
        }
        check_bounds(
            &request.regions,
            baseline.width,
            baseline.height,
            &pair.case_id,
        )?;
        differences.push(json!({
            "field": "dimensions",
            "case_id": pair.case_id,
            "baseline": baseline.dimensions(),
            "candidate": candidate.dimensions(),
            "declared": true,
        }));
        baseline_files.push((pair.case_id.clone(), baseline_source.clone()));
        candidate_files.push((pair.case_id.clone(), candidate_source.clone()));
        let protocol = {
            let mut protocol = request.protocol.clone();
            protocol["case_id"] = json!(pair.case_id);
            protocol
        };
        let measured = metrics::compare_images(
            baseline,
            candidate,
            &request.regions,
            &protocol,
            &folder,
            request.map_scale,
        )?;
        let measured = retain_metrics(measured, &folder, &mut evidence, &pair.case_id, 0)?;
        let mut record = json!({
            "case_id": pair.case_id,
            "status": measured["execution_status"],
            "measurements": measured["measurements"],
            "regions": request.regions,
            "region_readouts": measured["region_readouts"],
            "automatic_regions": measured["automatic_regions"],
            "artifacts": measured["artifacts"],
        });
        // One display copy per side, retained for inspection. The copy is an
        // artifact; the record does not present it, so nothing here describes
        // how it looks.
        review_image(
            baseline,
            &folder.join("baseline.png"),
            &mut evidence,
            "baseline",
            &pair.case_id,
            0,
        )?;
        review_image(
            candidate,
            &folder.join("candidate.png"),
            &mut evidence,
            "candidate",
            &pair.case_id,
            0,
        )?;
        record["baseline_source"] = baseline_source.clone();
        record["candidate_source"] = candidate_source.clone();
        if let Some(readouts) = record["region_readouts"].as_array_mut() {
            for readout in readouts {
                readout["pair_id"] = json!("baseline/candidate");
            }
        }
        relative_paths(&mut record["region_readouts"], &root);
        let measurement_path = folder.join("measurements.json");
        common::dump(
            &measurement_path,
            &json!({
                "measurements": record["measurements"],
                "region_readouts": record["region_readouts"],
            }),
        )?;
        let measurement_evidence = evidence.add(
            &measurement_path,
            "measurements",
            "application/json",
            json!({}),
        )?;
        if let Some(rows) = measured["findings"].as_array().cloned() {
            let mut rows = rows;
            findings.extend(publish_measurement_findings(
                &mut rows,
                &[json!(pair.case_id), json!(0)],
                &measurement_evidence["path"],
            )?);
        }
        if record["status"] == "failed" {
            // A failed population records why; that reason is the code a
            // caller has to act on, so it replaces the generic failure.
            let reason = measured["measurements"]
                .as_array()
                .and_then(|rows| rows.iter().find_map(|row| row["reason"].as_str()))
                .unwrap_or("measurement_failure")
                .to_owned();
            return Err(Error::new(
                reason,
                format!("Measurements failed for case {}", pair.case_id),
            )
            .at(&pair.case_id));
        }
        cases.push(record);
    }
    let baseline = source_block(&request.baseline_id, request, "baseline", &baseline_files)?;
    let candidate = source_block(
        &request.candidate_id,
        request,
        "candidate",
        &candidate_files,
    )?;
    differences.push(json!({
        "field": "analysis",
        "baseline": request.protocol["id"],
        "candidate": request.protocol["id"],
        "declared": true,
    }));
    differences.push(json!({
        "field": "map_scale",
        "baseline": request.map_scale,
        "candidate": request.map_scale,
        "declared": true,
    }));
    let case_ids: Vec<Value> = request
        .pairs
        .iter()
        .map(|pair| json!(pair.case_id))
        .collect();
    let digest = protocol_digest(request)?;
    let mut manifest = json!({
        "schema_version": 1,
        "kind": "comparison",
        "id": format!("{}-{}", request.baseline_id, request.candidate_id),
        "execution_status": "completed",
        "baseline": baseline,
        "candidate": candidate,
        "protocol": request.protocol,
        "analysis": {"id": request.protocol["id"]},
        "map_scale": request.map_scale,
        "protocol_digest": digest,
        "selection": {"case_ids": case_ids, "full_suite": true},
        "condition_differences": differences,
        "cases": cases,
        "artifacts": evidence.values(),
        "summary": {
            "selected_cases": request.pairs.len(),
            "completed_cases": request.pairs.len(),
            "completed_results": request.pairs.len(),
            "failed_results": 0,
            "required_rules": 0,
            "unavailable_rules": 0,
            "protected_failures": [],
        },
    });
    runner::finalize(&root, "comparison.json", &mut manifest)?;
    let mut result = json!({
        "schema_version": 1,
        "command": "compare",
        "execution_status": "completed",
        "evaluation_status": "not_evaluated",
        "artifact": root.join("comparison.json").to_string_lossy().replace('\\', "/"),
        "comparison_id": manifest["id"],
        "protocol_digest": manifest["protocol_digest"],
        "cases": request.pairs.len(),
        "regions": request.regions.len() + 1,
    });
    result["summary"] = manifest["summary"].clone();
    Ok((result, 0))
}
