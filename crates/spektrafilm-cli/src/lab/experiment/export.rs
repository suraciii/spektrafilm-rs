//! `experiment export`: convert an existing bundle's raw f64 buffers into
//! `.npy` arrays.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::super::cli::ExperimentExportArgs;
use super::super::report;
use super::{BUNDLE_SCHEMA_VERSION, absolute};

/// Convert one bundle's buffers into `.npy` arrays.
pub(super) fn run(args: &ExperimentExportArgs) -> Result<()> {
    let bundle_dir = absolute(&args.input)?;
    let manifest_path = bundle_dir.join("stage-evidence.json");
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading bundle manifest: {}", manifest_path.display()))?,
    )
    .with_context(|| format!("parsing bundle manifest: {}", manifest_path.display()))?;
    validate_bundle(&manifest)?;

    let available: Vec<&Value> = manifest["stages"]
        .as_array()
        .ok_or_else(|| anyhow!("bundle manifest carries no stages array"))?
        .iter()
        .filter(|stage| stage["availability"] == "available")
        .collect();
    if available.is_empty() {
        bail!("bundle carries no available stage to convert");
    }
    let selected = select_stages(&available, &args.stage)?;

    let out = absolute(&args.out)?;
    if out != bundle_dir && out.starts_with(&bundle_dir) {
        bail!(
            "export destination {} is inside the bundle directory; \
             choose a separate --out directory",
            out.display()
        );
    }
    if !args.force && out.join("export.json").exists() {
        bail!(
            "refusing to replace an existing array report in {}; pass --force to overwrite it",
            out.display()
        );
    }

    let mut converted = Vec::with_capacity(selected.len());
    let result = (|| -> Result<()> {
        std::fs::create_dir_all(&out)
            .with_context(|| format!("creating array directory: {}", out.display()))?;
        for stage in &selected {
            converted.push(convert_stage(&bundle_dir, &out, args.force, stage)?);
        }
        Ok(())
    })();
    if let Err(error) = result {
        // Remove any arrays this export already wrote, so a consumer
        // never sees partial output without an export.json.
        for array in &converted {
            let _ = std::fs::remove_file(&array.path);
        }
        let _ = std::fs::remove_file(out.join("export.json"));
        return Err(error);
    }
    let export_text = format!(
        "{}\n",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "tool": {"name": "spektrafilm-lab", "version": env!("CARGO_PKG_VERSION")},
            "manifest": manifest_path.display().to_string(),
            "stages": converted.iter().map(|array| json!({
                "id": array.id,
                "path": array.path.display().to_string(),
                "sha256": array.sha256,
                "shape": array.shape,
            })).collect::<Vec<_>>(),
        }))?
    );
    report::write_named(
        &out,
        &[("export.json", export_text.as_str())],
        "refusing to replace an existing array report",
        args.force,
    )
    .map_err(anyhow::Error::msg)
}

/// Validate a bundle against the stage-evidence contract before converting.
/// The manifest is the authority for stage identity and layout.
fn validate_bundle(manifest: &Value) -> Result<()> {
    let schema_version = manifest
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("bundle manifest carries no schema_version"))?;
    if schema_version != u64::from(BUNDLE_SCHEMA_VERSION) {
        bail!(
            "unsupported bundle schema version {schema_version}; this command reads {BUNDLE_SCHEMA_VERSION}"
        );
    }
    let status = manifest
        .get("execution_status")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("bundle manifest carries no execution_status"))?;
    if status != "completed" {
        let errors = manifest
            .get("errors")
            .and_then(Value::as_array)
            .map(|errors| {
                errors
                    .iter()
                    .map(|error| error.as_str().unwrap_or_default().to_owned())
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        if errors.is_empty() {
            bail!(
                "bundle execution_status is {status:?}; a bundle that did not complete is not convertible"
            );
        }
        bail!("bundle execution_status is {status:?}: {errors}");
    }
    let precision = manifest
        .get("host_precision")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("bundle manifest carries no host_precision"))?;
    if precision != "f64" {
        bail!(
            "bundle host_precision is {precision:?}; the lab converts f64 buffers only — \
             re-render the experiment with an f64 reference bundle"
        );
    }
    let stages = manifest
        .get("stages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("bundle manifest carries no stages array"))?;
    let mut incomplete = Vec::new();
    for stage in stages {
        let id = stage["id"].as_str().unwrap_or_default();
        match stage["availability"].as_str() {
            Some("available") => {}
            Some("unavailable") => incomplete.push(format!("{id} is unavailable")),
            Some("not_applicable") => {}
            other => bail!("bundle stage {id:?} carries an unknown availability {other:?}"),
        }
    }
    if !incomplete.is_empty() {
        bail!("bundle is incomplete: {}", incomplete.join("; "));
    }
    Ok(())
}

/// Select the requested stages from the manifest's available records, in
/// manifest order. An unknown `--stage` names the available stages.
fn select_stages<'a>(available: &[&'a Value], requested: &[String]) -> Result<Vec<&'a Value>> {
    if requested.is_empty() {
        return Ok(available.to_vec());
    }
    let names: Vec<&str> = available
        .iter()
        .map(|stage| stage["id"].as_str().unwrap_or_default())
        .collect();
    requested
        .iter()
        .map(|id| {
            available
                .iter()
                .find(|stage| stage["id"].as_str() == Some(id.as_str()))
                .copied()
                .ok_or_else(|| {
                    anyhow!(
                        "stage {id:?} is not available in this bundle; available stages are {names:?}"
                    )
                })
        })
        .collect()
}

/// One converted array, as recorded in `export.json`.
#[derive(Debug)]
struct ConvertedArray {
    id: String,
    path: PathBuf,
    sha256: String,
    shape: Vec<usize>,
}

/// Convert one available stage record into an array, validating the buffer
/// against the manifest's own layout record, then writing it and hashing the
/// file that was written.
fn convert_stage(
    bundle_dir: &Path,
    out: &Path,
    force: bool,
    stage: &Value,
) -> Result<ConvertedArray> {
    let id = stage["id"]
        .as_str()
        .ok_or_else(|| anyhow!("bundle stage record carries no id"))?
        .to_owned();
    let relative = stage["path"]
        .as_str()
        .ok_or_else(|| anyhow!("bundle stage {id:?} carries no buffer path"))?;
    if relative != format!("{id}.f64") {
        bail!(
            "bundle stage {id:?} names buffer {relative:?}; a stage's buffer is its own <id>.f64 file"
        );
    }
    let buffer_path = bundle_dir.join(relative);
    let bytes = std::fs::read(&buffer_path)
        .with_context(|| format!("reading bundle buffer: {}", buffer_path.display()))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    let declared = stage["sha256"]
        .as_str()
        .ok_or_else(|| anyhow!("bundle stage {id:?} carries no sha256"))?;
    if actual != declared {
        bail!(
            "bundle stage {id:?} buffer {} hashes to {} but the manifest declares {}",
            buffer_path.display(),
            actual,
            declared
        );
    }
    let layout = stage
        .get("layout")
        .ok_or_else(|| anyhow!("bundle stage {id:?} carries no layout"))?;
    let width = layout["width"]
        .as_u64()
        .ok_or_else(|| anyhow!("bundle stage {id:?} layout carries no width"))?;
    let height = layout["height"]
        .as_u64()
        .ok_or_else(|| anyhow!("bundle stage {id:?} layout carries no height"))?;
    let channels = layout["channels"]
        .as_u64()
        .ok_or_else(|| anyhow!("bundle stage {id:?} layout carries no channels"))?;
    if layout["dtype"].as_str() != Some("f64") {
        bail!(
            "bundle stage {id:?} layout dtype is {:?}; the lab converts f64 buffers",
            layout["dtype"]
        );
    }
    if layout["endianness"].as_str() != Some("little") {
        bail!(
            "bundle stage {id:?} layout endianness is {:?}; the lab converts little-endian buffers",
            layout["endianness"]
        );
    }
    let declared_bytes = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(channels))
        .and_then(|samples| samples.checked_mul(8))
        .ok_or_else(|| anyhow!("bundle stage {id:?} layout overflows"))?;
    if bytes.len() as u64 != declared_bytes {
        bail!(
            "bundle stage {id:?} buffer is {} bytes but its layout declares {declared_bytes} \
             ({width}x{height}x{channels} f64 samples)",
            bytes.len()
        );
    }
    // The values are the buffer's bytes exactly: no scaling, no channel
    // permutation, no clipping.
    let samples = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect::<Vec<f64>>();
    if !samples.iter().all(|sample| sample.is_finite()) {
        bail!("bundle stage {id:?} carries a non-finite sample; export is a lossless conversion");
    }
    let shape = vec![height as usize, width as usize, channels as usize];
    let path = out.join(format!("{id}.npy"));
    if !force && path.exists() {
        bail!(
            "refusing to replace an existing array in {}; pass --force to overwrite it",
            out.display()
        );
    }
    // Write to a temporary file then rename, so a mid-sequence failure
    // never leaves a partial array that looks complete.
    let tmp_path = out.join(format!("{id}.npy.tmp"));
    spektrafilm_math::npy::save_npy_f64(&tmp_path, &samples, &shape)
        .map_err(|error| anyhow!("writing {}: {error}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &path)
        .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()))?;
    let written =
        std::fs::read(&path).with_context(|| format!("reading back {}", path.display()))?;
    Ok(ConvertedArray {
        id,
        path,
        sha256: format!("{:x}", Sha256::digest(&written)),
        shape,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::scratch;
    use super::*;

    fn bundle_manifest(precision: &str, stages: Value) -> Value {
        json!({
            "schema_version": 1,
            "execution_status": "completed",
            "host_precision": precision,
            "stages": stages,
        })
    }

    fn available_stage(id: &str, sha256: &str, width: u64, height: u64) -> Value {
        json!({
            "availability": "available",
            "id": id,
            "path": format!("{id}.f64"),
            "sha256": sha256,
            "layout": {
                "width": width,
                "height": height,
                "channels": 3,
                "dtype": "f64",
                "endianness": "little",
                "channel_order": if id == "rgb_in" || id == "rgb_out" { "RGB" } else { "CMY" },
            },
        })
    }

    fn buffer_bytes(samples: &[f64]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(samples.len() * 8);
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn accepts_a_completed_f64_bundle() {
        let manifest = bundle_manifest(
            "f64",
            json!([available_stage("rgb_in", &"0".repeat(64), 2, 2)]),
        );
        validate_bundle(&manifest).unwrap();
    }

    #[test]
    fn rejects_an_f32_bundle() {
        let manifest = bundle_manifest(
            "f32",
            json!([available_stage("rgb_in", &"0".repeat(64), 2, 2)]),
        );
        let error = validate_bundle(&manifest).unwrap_err().to_string();
        assert!(error.contains("f64"), "{error}");
        assert!(error.contains("f32"), "{error}");
    }

    #[test]
    fn rejects_a_bundle_with_an_unavailable_stage() {
        let manifest = bundle_manifest(
            "f64",
            json!([{
                "availability": "unavailable",
                "id": "cmy_film",
                "reason": "buffer_write_failed",
            }]),
        );
        let error = validate_bundle(&manifest).unwrap_err().to_string();
        assert!(error.contains("incomplete"), "{error}");
        assert!(error.contains("cmy_film"), "{error}");
    }

    #[test]
    fn rejects_an_uncompleted_bundle() {
        let mut manifest = bundle_manifest(
            "f64",
            json!([available_stage("rgb_in", &"0".repeat(64), 2, 2)]),
        );
        manifest["execution_status"] = json!("failed");
        manifest["errors"] = json!(["rgb_out: not captured"]);
        let error = validate_bundle(&manifest).unwrap_err().to_string();
        assert!(error.contains("not captured"), "{error}");
    }

    #[test]
    fn rejects_an_unknown_requested_stage() {
        let manifest = bundle_manifest(
            "f64",
            json!([
                available_stage("rgb_in", &"0".repeat(64), 2, 2),
                available_stage("rgb_out", &"0".repeat(64), 2, 2),
            ]),
        );
        let available: Vec<&Value> = manifest["stages"].as_array().unwrap().iter().collect();
        let error = select_stages(&available, &["cmy_film".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("rgb_in"), "{error}");
        assert!(error.to_string().contains("rgb_out"), "{error}");
    }

    #[test]
    fn selects_requested_stages_in_manifest_order() {
        let manifest = bundle_manifest(
            "f64",
            json!([
                available_stage("rgb_in", &"0".repeat(64), 2, 2),
                available_stage("rgb_out", &"0".repeat(64), 2, 2),
            ]),
        );
        let available: Vec<&Value> = manifest["stages"].as_array().unwrap().iter().collect();
        let selected = select_stages(&available, &["rgb_out".to_owned()]).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0]["id"], "rgb_out");
    }

    #[test]
    fn converts_a_buffer_byte_for_byte() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        let out = dir.join("arrays");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let samples: Vec<f64> = (0..12).map(|index| index as f64 * 0.5).collect();
        let bytes = buffer_bytes(&samples);
        std::fs::write(bundle.join("rgb_in.f64"), &bytes).unwrap();
        let stage = available_stage("rgb_in", &format!("{:x}", Sha256::digest(&bytes)), 2, 2);
        let converted = convert_stage(&bundle, &out, false, &stage).unwrap();
        assert_eq!(converted.id, "rgb_in");
        assert_eq!(converted.shape, vec![2, 2, 3]);
        assert_eq!(converted.path, out.join("rgb_in.npy"));
        // The payload is the buffer's bytes exactly, and the recorded digest
        // is the written array's.
        let written = std::fs::read(&converted.path).unwrap();
        assert!(written.ends_with(&bytes));
        assert_eq!(converted.sha256, format!("{:x}", Sha256::digest(&written)));
    }

    #[test]
    fn refuses_to_replace_an_existing_array_without_force() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        let out = dir.join("arrays");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let bytes = buffer_bytes(&[0.0; 12]);
        std::fs::write(bundle.join("rgb_in.f64"), &bytes).unwrap();
        let stage = available_stage("rgb_in", &format!("{:x}", Sha256::digest(&bytes)), 2, 2);
        std::fs::write(out.join("rgb_in.npy"), b"SENTINEL").unwrap();
        let error = convert_stage(&bundle, &out, false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("refusing to replace"), "{error}");
        assert_eq!(std::fs::read(out.join("rgb_in.npy")).unwrap(), b"SENTINEL");
        convert_stage(&bundle, &out, true, &stage).unwrap();
        assert_ne!(std::fs::read(out.join("rgb_in.npy")).unwrap(), b"SENTINEL");
    }

    #[test]
    fn rejects_a_buffer_whose_digest_does_not_match() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("rgb_in.f64"), [0u8; 96]).unwrap();
        let stage = available_stage("rgb_in", &"1".repeat(64), 2, 2);
        let error = convert_stage(&bundle, &dir.join("arrays"), false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("hashes to"), "{error}");
    }

    #[test]
    fn rejects_a_buffer_whose_length_does_not_match_the_layout() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let bytes = [0u8; 48];
        std::fs::write(bundle.join("rgb_in.f64"), bytes).unwrap();
        let stage = available_stage("rgb_in", &format!("{:x}", Sha256::digest(bytes)), 2, 2);
        let error = convert_stage(&bundle, &dir.join("arrays"), false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("layout declares"), "{error}");
    }

    #[test]
    fn rejects_a_missing_buffer() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let stage = available_stage("rgb_in", &"0".repeat(64), 2, 2);
        let error = convert_stage(&bundle, &dir.join("arrays"), false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("reading bundle buffer"), "{error}");
    }

    #[test]
    fn rejects_a_non_finite_sample() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let mut samples = vec![0.0f64; 12];
        samples[5] = f64::NAN;
        let bytes = buffer_bytes(&samples);
        std::fs::write(bundle.join("rgb_in.f64"), &bytes).unwrap();
        let stage = available_stage("rgb_in", &format!("{:x}", Sha256::digest(&bytes)), 2, 2);
        let error = convert_stage(&bundle, &dir.join("arrays"), false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("non-finite"), "{error}");
    }

    #[test]
    fn rejects_a_stage_record_that_names_another_stage_buffer() {
        let dir = scratch();
        let bundle = dir.join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        let bytes = [0u8; 96];
        std::fs::write(bundle.join("rgb_out.f64"), bytes).unwrap();
        let mut stage = available_stage("rgb_in", &format!("{:x}", Sha256::digest(bytes)), 2, 2);
        stage["path"] = json!("rgb_out.f64");
        let error = convert_stage(&bundle, &dir.join("arrays"), false, &stage)
            .unwrap_err()
            .to_string();
        assert!(error.contains("<id>.f64"), "{error}");
    }
}
