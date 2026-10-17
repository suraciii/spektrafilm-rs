//! `measure verify`: re-resolve a recorded measurement's chain and report
//! every difference from the identity it recorded.
//!
//! The command reads the retained `run.json`, rebuilds the chain
//! declaration the run recorded, and re-resolves it against the data
//! directory present now. It re-hashes the renderer executable, the selected
//! data inventory, the profiles, and every retained stage buffer, then reports
//! what changed. It renders nothing, rewrites nothing, and judges nothing:
//! a difference is evidence, so the command exits 0 whenever it could read
//! the record and re-resolve the chain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::select_data_dir;

use super::super::cli::{ChainArgs, MeasureEntry, MeasureVerifyArgs};
use super::{
    InputRequest, ResolvedChain, chain_identity, resolve_chain, resolve_range, sha256_file,
};

/// The schema version of the identity report this command prints.
const REPORT_SCHEMA_VERSION: u32 = 1;

/// The scene-linear space the `spectra` entry photographs its samples into;
/// the run resolved its input declaration with the same space.
const SCENE_RGB_SPACE: &str = "ACES2065-1";

/// The `rgb_in` entry, whose color space comes from the recorded identity
/// rather than from a fixed constant.
const ENTRY_RGB_IN: &str = "rgb_in";

/// The `spectra` entry.
const ENTRY_SPECTRA: &str = "spectra";

pub fn verify(args: &MeasureVerifyArgs, matches: &clap::ArgMatches) -> Result<()> {
    verify_with_result(args, matches).map(|_| ())
}

/// Re-resolve a recorded record and return its report as the command result.
pub fn verify_with_result(
    args: &MeasureVerifyArgs,
    matches: &clap::ArgMatches,
) -> Result<(Value, i32)> {
    let run = args.record.as_path();
    let (record, record_path) = read_run_record(run)?;

    // A comparison record carries no chain: its identity is the retained run
    // and comparison digests plus the artifacts they cite, so the verification
    // re-hashes those instead of re-resolving a chain.
    if record.get("kind").and_then(Value::as_str) == Some("comparison") {
        return verify_comparison(run, &record);
    }

    // An image-domain run record retains no chain declaration: its identity is
    // the suite, target, case configurations, authored dependencies, and every
    // artifact it published, each with the hash recorded at write time. That is
    // what this form re-hashes.
    if record
        .get("declaration")
        .and_then(Value::as_object)
        .is_none()
    {
        return verify_image_run(run, &record);
    }

    let declaration = recorded_declaration(&record, &record_path)?;

    let entry = declaration
        .get("enter")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    // Re-resolve against the directory the run recorded unless the caller
    // names another one: a bare `measure verify --run DIR` reproduces the
    // recorded chain instead of discovering a different data directory.
    let recorded = recorded_data_dir(&declaration);
    let data_dir = if crate::named_data_dir(matches) || recorded.is_empty() {
        select_data_dir(args.data_dir.clone(), matches)?.path
    } else {
        PathBuf::from(recorded)
    };

    // Re-resolve before comparing anything: without the same chain there is
    // nothing to re-verify, so a failed resolution is an error.
    let chain_args = rebuild_chain_args(&declaration, &data_dir);
    let mut chain = resolve_recorded_chain(&chain_args, &data_dir, &record, &entry)?;

    // The identity is recomputed the way the run recorded it, then compared
    // leaf by leaf. Every difference is evidence; none is a failure.
    let recorded_identity = record
        .get("identity")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let current = recompute_identity(&mut chain, &data_dir, &declaration, &recorded_identity)?;
    let identity_differences = compare_leaves(&recorded_identity, &current);

    // The retained stage buffers are re-hashed beside the identity: a reading
    // that no longer matches its own record is evidence even when the chain
    // that produced it is unchanged. The retained set is the bundle manifest's,
    // so a buffer the run captured but the request never named is verified too.
    let buffer_differences = rehash_stage_buffers(run);

    // The report is the command result: the command writes no artifact, so
    // its stdout object carries both the verification envelope and the report
    // itself.
    let result = json!({
        "schema_version": REPORT_SCHEMA_VERSION,
        "command": "verify",
        "execution_status": "completed",
        "kind": "verify",
        "artifact": run.display().to_string(),
        "re_resolved": {
            "film": chain.film_name,
            "print": chain.print_name,
            "route": chain.params.workflow.route,
            "scan_output": chain.params.scanner.scan_output,
            "seed": chain_args.seed,
        },
        "identity_differences": identity_differences,
        "buffer_differences": buffer_differences,
        "data_dir_differences": {
            "recorded": recorded_data_dir(&declaration),
            "current": data_dir.display().to_string(),
        },
    });

    for difference in &identity_differences {
        eprintln!("identity: {}", difference_line(difference));
    }
    for difference in &buffer_differences {
        eprintln!("buffer: {}", difference_line(difference));
    }
    eprintln!(
        "verify: {} identity difference(s), {} buffer difference(s)",
        identity_differences.len(),
        buffer_differences.len()
    );

    // Differences are evidence, not failure: the record was read and the
    // chain re-resolved, so the verification completed.
    Ok((result, 0))
}

/// Verify a comparison record: re-hash the retained run and comparison
/// identities and the evidence the comparison cites. A comparison computes no
/// chain, so there is nothing to re-resolve.
fn verify_comparison(run: &Path, record: &Value) -> Result<(Value, i32)> {
    // A reading comparison names its sides rather than carrying one identity,
    // so the report identifies the pair it re-hashed.
    let recorded = record
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            ["baseline", "candidate", "reference"]
                .into_iter()
                .filter_map(|key| record.get(key))
                .filter_map(|side| side.get("path"))
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" vs ")
        });
    let mut differences = Vec::new();
    for key in ["baseline", "candidate", "reference"] {
        let Some(side) = record.get(key).filter(|side| side.is_object()) else {
            continue;
        };
        let path = side.get("path").and_then(Value::as_str).unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        let declared = side
            .get("measure_sha256")
            .and_then(Value::as_str)
            .unwrap_or("");
        // `path` is the side's measurement directory and the hash names that
        // directory's `run.json`, which is what the comparison recorded.
        let actual = sha256_file(&Path::new(path).join("run.json")).unwrap_or_default();
        if actual != declared {
            differences.push(json!({
                "field": format!("{key}.measure_sha256"),
                "path": path,
                "recorded": declared,
                "current": actual,
            }));
        }
    }
    let result = json!({
        "schema_version": REPORT_SCHEMA_VERSION,
        "command": "verify",
        "execution_status": "completed",
        "kind": "verify",
        "artifact": run.display().to_string(),
        "comparison": recorded,
        "identity_differences": differences,
        "buffer_differences": [],
    });
    for difference in &differences {
        eprintln!("identity: {}", difference_line(difference));
    }
    eprintln!(
        "verify: {} identity difference(s), 0 buffer difference(s)",
        differences.len()
    );
    Ok((result, 0))
}

/// Re-hash an image-domain run record's retained identity.
///
/// The record names what it read and what it wrote: the suite and target
/// snapshots, the authored dependencies, and every published artifact, each
/// with the hash taken when it was written. Verification re-hashes each one in
/// place and reports the difference, so a record whose inputs or outputs moved
/// says so instead of silently passing. The renderer executable is named by the
/// environment the record retained and is re-hashed the same way when it is
/// still where the record says it is.
fn verify_image_run(run: &Path, record: &Value) -> Result<(Value, i32)> {
    let mut differences = Vec::new();
    let mut checked = 0_usize;
    // A record that names a file relative to its own root resolves against that
    // root; one that names an absolute path is used as recorded.
    let resolve = |path: &str| -> PathBuf {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            run.join(candidate)
        }
    };
    let mut check = |path: &str, recorded: &str, label: &str| {
        if path.is_empty() {
            return;
        }
        checked += 1;
        let actual = sha256_file(&resolve(path)).unwrap_or_default();
        if actual != recorded {
            differences.push(json!({
                "field": label,
                "path": path,
                "recorded": recorded,
                "current": actual,
            }));
        }
    };
    for key in ["suite", "target"] {
        if let Some(entry) = record.get(key).filter(|entry| entry.is_object()) {
            check(
                entry["path"].as_str().unwrap_or_default(),
                entry["sha256"].as_str().unwrap_or_default(),
                key,
            );
        }
    }
    for entry in record["dependencies"].as_array().into_iter().flatten() {
        check(
            entry["path"].as_str().unwrap_or_default(),
            entry["sha256"].as_str().unwrap_or_default(),
            "dependency",
        );
    }
    for entry in record["artifacts"].as_array().into_iter().flatten() {
        check(
            entry["path"].as_str().unwrap_or_default(),
            entry["sha256"].as_str().unwrap_or_default(),
            "artifact",
        );
    }
    let result = json!({
        "schema_version": REPORT_SCHEMA_VERSION,
        "command": "verify",
        "execution_status": "completed",
        "kind": "verify",
        "artifact": run.display().to_string(),
        "form": "image_run",
        "checked": checked,
        "identity_differences": differences,
        "buffer_differences": [],
    });
    for difference in &differences {
        eprintln!("identity: {}", difference_line(difference));
    }
    eprintln!(
        "verify: {} identity difference(s) of {} retained hash(es), 0 buffer difference(s)",
        differences.len(),
        checked
    );
    Ok((result, 0))
}

/// Read the retained record a directory holds and require it to be one this
/// command can re-resolve.
fn read_run_record(run: &Path) -> Result<(Value, PathBuf)> {
    // A measurement directory holds `run.json`; a comparison directory holds
    // `comparison.json`. The record names its own kind, so the directory's
    // single record file is found rather than assumed.
    let path = ["run.json", "comparison.json"]
        .into_iter()
        .map(|name| run.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            anyhow!(
                "{} holds no run.json or comparison.json to verify",
                run.display()
            )
        })?;
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let record: Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let version = record.get("schema_version").and_then(Value::as_u64);
    if version != Some(1) {
        bail!(
            "{} has schema_version {}, expected 1",
            path.display(),
            record
                .get("schema_version")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_owned())
        );
    }
    match record.get("kind").and_then(Value::as_str) {
        // A record names its own kind, so the command reads it without being
        // told which one it is. A `run` record carries a chain to re-resolve;
        // a `comparison` record carries retained identities to re-hash; a
        // `sample` record reads one source and carries no chain, so there is
        // no chain identity to re-resolve.
        Some("run") | Some("comparison") => Ok((record, path)),
        Some("sample") => bail!(
            "{} is a sample record; it reads one source and carries no chain, \
             so there is no chain identity to re-resolve",
            path.display()
        ),
        Some(other) => bail!(
            "{} has kind {:?}, expected a `run` or `comparison` record",
            path.display(),
            other
        ),
        None => bail!(
            "{} carries no kind, expected a `run` or `comparison` record",
            path.display()
        ),
    }
}

/// The declaration the run recorded; a record without one cannot be verified.
fn recorded_declaration(record: &Value, path: &Path) -> Result<Value> {
    record
        .get("declaration")
        .filter(|declaration| declaration.is_object())
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "{} carries no declaration object; only a measurement that recorded its \
                 declared chain can be re-resolved",
                path.display()
            )
        })
}

/// The data directory the run recorded, as text.
fn recorded_data_dir(declaration: &Value) -> String {
    declaration
        .get("data_dir")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Rebuild the requested chain options exactly as the caller passed them. The
/// profile names the record carries beside them are the resolution's own
/// result: feeding those back would re-resolve a different chain than the one
/// the run recorded, so only the requested options are used.
fn rebuild_chain_args(declaration: &Value, data_dir: &Path) -> ChainArgs {
    ChainArgs {
        preset: string_array(declaration, "preset"),
        film: string_field(declaration, "film"),
        paper: string_field(declaration, "paper"),
        sources: string_array(declaration, "sources"),
        route: string_field(declaration, "route"),
        scan_output: string_field(declaration, "scan_output"),
        seed: seed_of(declaration),
        data_dir: Some(data_dir.to_path_buf()),
    }
}

/// One JSON string field, or nothing when the record carries no such value.
fn string_field(declaration: &Value, key: &str) -> Option<String> {
    declaration
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// One JSON array of strings. A non-string element is dropped rather than
/// coerced: the declaration vocabulary is exact.
fn string_array(declaration: &Value, key: &str) -> Vec<String> {
    declaration
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The recorded seed, or 0 when the record carries none.
fn seed_of(declaration: &Value) -> u32 {
    declaration.get("seed").and_then(Value::as_u64).unwrap_or(0) as u32
}

/// Re-resolve the recorded declaration with the input declaration the entry
/// declares: `spectra` and `rgb_in` carry a color space and linear values, the
/// film-domain boundaries carry neither.
fn resolve_recorded_chain(
    chain_args: &ChainArgs,
    data_dir: &Path,
    record: &Value,
    entry: &str,
) -> Result<ResolvedChain> {
    // `rgb_in` declares the color space and the transfer state its stimulus
    // carried: a run that declared encoded values must re-resolve with
    // decoding enabled, or the parameters would not be the recorded ones.
    let declared = record
        .get("identity")
        .and_then(|identity| identity.get("input_declaration"));
    let (color_space, cctf_decoding) = if entry == ENTRY_RGB_IN {
        (
            declared
                .and_then(|declaration| declaration.get("color_space"))
                .and_then(Value::as_str),
            declared
                .and_then(|declaration| declaration.get("cctf_decoding"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )
    } else {
        (None, false)
    };
    let input = match entry {
        // The `spectra` entry photographs its samples into the scene-linear
        // reference space, which is what the run declared.
        ENTRY_SPECTRA => InputRequest {
            color_space: Some(SCENE_RGB_SPACE),
            cctf_decoding: false,
            require: true,
        },
        ENTRY_RGB_IN => InputRequest {
            color_space,
            cctf_decoding,
            require: true,
        },
        _ => InputRequest {
            color_space: None,
            cctf_decoding: false,
            require: false,
        },
    };
    resolve_chain(chain_args, data_dir, input).map_err(|error| {
        anyhow!(
            "{error}; the recorded declaration no longer resolves, so the run cannot be \
             re-verified against the data directory now"
        )
    })
}

/// Recompute the identity the run recorded, from the records present now. The
/// stimulus record is carried over unchanged: it describes the run's own
/// declared input, which this command never re-reads.
fn recompute_identity(
    chain: &mut ResolvedChain,
    data_dir: &Path,
    declaration: &Value,
    recorded_identity: &Value,
) -> Result<Value> {
    let stimulus = recorded_identity
        .get("stimulus")
        .cloned()
        .unwrap_or(Value::Null);
    let entry = string_field(declaration, "enter").unwrap_or_default();
    let read = string_array(declaration, "read");
    // The range the run executed is derived the same way the run derived it:
    // the recorded entry becomes `taps.inject` and the collection boundary
    // reaches every recorded reading, so the re-resolved parameters are the
    // ones the identity was taken from.
    let entry = entry_enum(&entry, declaration)?;
    resolve_range(&mut chain.params, entry, &read)?;
    chain_identity(
        chain,
        data_dir,
        entry.name(),
        &read,
        seed_of(declaration),
        stimulus,
    )
}

/// The entry boundary a declaration names.
fn entry_enum(entry: &str, declaration: &Value) -> Result<MeasureEntry> {
    MeasureEntry::ALL
        .iter()
        .copied()
        .find(|candidate| candidate.name() == entry)
        .ok_or_else(|| {
            anyhow!("the recorded declaration names an unknown entry {entry:?}: {declaration}")
        })
}

/// Flatten one JSON value into `path -> leaf` pairs. Object keys join with
/// `.` and array indices with `[]`, so an identity leaf keeps its address.
fn flatten(value: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(child, &path, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                flatten(child, &format!("{prefix}[{index}]"), out);
            }
        }
        leaf => out.push((prefix.to_owned(), leaf.clone())),
    }
}

/// Every leaf where the recorded and the recomputed identity disagree, with
/// the recorded value first and `null` where one side carries no leaf.
fn compare_leaves(recorded: &Value, current: &Value) -> Vec<Value> {
    let mut recorded_leaves = Vec::new();
    let mut current_leaves = Vec::new();
    flatten(recorded, "", &mut recorded_leaves);
    flatten(current, "", &mut current_leaves);
    let current_by_path: BTreeMap<&str, &Value> = current_leaves
        .iter()
        .map(|(path, value)| (path.as_str(), value))
        .collect();
    let recorded_paths: Vec<&str> = recorded_leaves
        .iter()
        .map(|(path, _)| path.as_str())
        .collect();

    let mut differences = Vec::new();
    for (path, recorded_value) in &recorded_leaves {
        let current_value = current_by_path.get(path.as_str()).copied();
        if current_value != Some(recorded_value) {
            differences.push(json!({
                "path": path,
                "recorded": recorded_value,
                "current": current_value.cloned().unwrap_or(Value::Null),
            }));
        }
    }
    for (path, current_value) in &current_leaves {
        if !recorded_paths.contains(&path.as_str()) {
            differences.push(json!({
                "path": path,
                "recorded": Value::Null,
                "current": current_value,
            }));
        }
    }
    differences
}

/// Re-hash every retained stage buffer the bundle manifest names as available.
/// A buffer that is missing or unreadable is reported with `current: null`: an
/// absent buffer is a difference, not a broken verification.
fn rehash_stage_buffers(run: &Path) -> Vec<Value> {
    let evidence = run.join("evidence");
    let mut differences = Vec::new();
    for entry in bundle_stage_entries(&evidence) {
        // The manifest's own digest is the record the run retained; the buffer
        // it names is re-hashed beside it.
        let (id, name, recorded) = entry;
        let path = evidence.join(&name);
        let current = std::fs::File::open(&path)
            .ok()
            .and_then(|_| sha256_file(&path).ok());
        if current.as_deref() != Some(recorded.as_str()) {
            differences.push(json!({
                "stage": id,
                "recorded": recorded,
                "current": current,
                "path": path.display().to_string(),
            }));
        }
    }
    differences
}

/// Every stage entry the retained stage-evidence manifest records as
/// available: its stage identifier, the bundle-relative buffer file name, and
/// the digest the manifest recorded. The buffer file name is kept as recorded,
/// so a bundle that names its buffers another way is still re-hashed.
fn bundle_stage_entries(evidence: &Path) -> Vec<(String, String, String)> {
    let mut entries = Vec::new();
    let manifest_path = evidence.join("stage-evidence.json");
    let Ok(text) = std::fs::read_to_string(&manifest_path) else {
        return entries;
    };
    let Ok(manifest) = serde_json::from_str::<Value>(&text) else {
        return entries;
    };
    for stage in manifest
        .get("stages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        if stage.get("availability").and_then(Value::as_str) != Some("available") {
            continue;
        }
        if let (Some(id), Some(name), Some(recorded)) = (
            stage.get("id").and_then(Value::as_str),
            stage.get("path").and_then(Value::as_str),
            stage.get("sha256").and_then(Value::as_str),
        ) {
            entries.push((id.to_owned(), name.to_owned(), recorded.to_owned()));
        }
    }
    entries
}

/// One difference as a single stderr line.
fn difference_line(difference: &Value) -> String {
    let recorded = compact(difference.get("recorded").unwrap_or(&Value::Null));
    let current = compact(difference.get("current").unwrap_or(&Value::Null));
    let label = difference
        .get("stage")
        .and_then(Value::as_str)
        .map(|stage| format!("stage {stage}"))
        .unwrap_or_else(|| {
            difference
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        });
    format!("{label}: recorded {recorded}, current {current}")
}

/// One JSON value as compact text, for a single-line summary.
fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}
