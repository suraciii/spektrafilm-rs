use crate::{
    Error, Result,
    common::{self, Evidence, Progress, array, text},
    documents,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const ENV_KEYS: &[&str] = &[
    "RAYON_NUM_THREADS",
    "OPENBLAS_NUM_THREADS",
    "OMP_NUM_THREADS",
    "LD_LIBRARY_PATH",
    "DYLD_LIBRARY_PATH",
    "VK_ICD_FILENAMES",
    "VK_DRIVER_FILES",
    "WGPU_BACKEND",
    "XDG_RUNTIME_DIR",
];

pub fn finalize(root: &Path, name: &str, value: &mut Value) -> Result<()> {
    value
        .as_object_mut()
        .ok_or_else(|| Error::new("document_type", "Manifest must be an object"))?
        .remove("id");
    value["id"] = json!(common::digest(value)?);
    documents::validate_generated_document(
        if name == "run.json" {
            "run"
        } else {
            "comparison"
        },
        value,
        root,
    )?;
    let temp = root.join(format!("{name}.tmp"));
    common::dump(&temp, value)?;
    fs::rename(temp, root.join(name))?;
    Ok(())
}
fn tree_inventory(root: &Path) -> Result<Value> {
    fn visit(root: &Path, dir: &Path, records: &mut Vec<Value>) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(
                    Error::new("data_symlink", "Data dependencies must be regular files")
                        .file(path.display().to_string()),
                );
            }
            if kind.is_dir() {
                visit(root, &path, records)?;
            } else if kind.is_file() {
                records.push(json!({"path":path.strip_prefix(root).map_err(|e|Error::new("invalid_path",e.to_string()))?.to_string_lossy().replace('\\',"/"),"sha256":common::sha256_file(&path)?}));
            }
        }
        Ok(())
    }
    let mut records = Vec::new();
    visit(root, root, &mut records)?;
    records.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(json!({"sha256":common::digest(&json!(records))?,"files":records}))
}
fn file_dependencies(
    suite: &Value,
    suite_path: &Path,
    target: &Value,
    target_path: &Path,
) -> Result<Vec<Value>> {
    fn dependency_path(value: &str, document: &Path) -> Result<PathBuf> {
        Ok(documents::normalize_path(document)?
            .parent()
            .unwrap_or(Path::new("/"))
            .join(value))
    }
    let mut paths = BTreeMap::<PathBuf, String>::new();
    paths.insert(
        dependency_path(text(&target["renderer"], "path")?, target_path)?,
        "renderer".into(),
    );
    fn add_geometry(
        value: &Value,
        document: &Path,
        paths: &mut BTreeMap<PathBuf, String>,
    ) -> Result<()> {
        if let Some(path) = value.get("path").and_then(Value::as_str) {
            paths.insert(dependency_path(path, document)?, "mask".into());
        }
        Ok(())
    }
    for sample in suite["protocol"]["probe"]["tone_response"]
        .as_array()
        .into_iter()
        .flatten()
    {
        add_geometry(&sample["geometry"], suite_path, &mut paths)?;
    }
    for case in array(suite, "cases")? {
        paths.insert(
            dependency_path(text(&case["input"], "path")?, suite_path)?,
            "input".into(),
        );
        if let Some(preparation) = case.get("preparation") {
            paths.insert(
                dependency_path(text(&preparation["original"], "path")?, suite_path)?,
                "preparation_original".into(),
            );
        }
        for key in ["presets", "set_files"] {
            for path in case["configuration"][key].as_array().into_iter().flatten() {
                paths.insert(
                    dependency_path(
                        path.as_str().ok_or_else(|| {
                            Error::new("invalid_path", "File path must be string")
                        })?,
                        suite_path,
                    )?,
                    key.into(),
                );
            }
        }
        if let Some(path) = case["configuration"]["params"].as_str() {
            paths.insert(dependency_path(path, suite_path)?, "params".into());
        }
        for region in array(case, "regions")? {
            add_geometry(&region["geometry"], suite_path, &mut paths)?;
        }
        for sample in case["probe"]["tone_response"]
            .as_array()
            .into_iter()
            .flatten()
        {
            add_geometry(&sample["geometry"], suite_path, &mut paths)?;
        }
        if let Some(reference) = case.get("reference") {
            paths.insert(
                dependency_path(text(reference, "path")?, suite_path)?,
                "reference".into(),
            );
            for key in [
                "transform",
                "aligned_derivative",
                "correspondence_review",
                "common_area_mask",
            ] {
                if let Some(record) = reference.get(key) {
                    paths.insert(
                        dependency_path(text(record, "path")?, suite_path)?,
                        key.into(),
                    );
                }
            }
            if let Some(icc) = reference["color"].get("icc") {
                paths.insert(
                    dependency_path(text(icc, "path")?, suite_path)?,
                    "icc".into(),
                );
            }
            for region in reference["regions"].as_array().into_iter().flatten() {
                add_geometry(&region["geometry"], suite_path, &mut paths)?;
            }
            if let Some(review) = reference.get("correspondence_review") {
                let review_path = documents::resolve_path(text(review, "path")?, suite_path)?;
                let review = documents::load_json_document(&review_path)?;
                for excluded in review["excluded_regions"].as_array().into_iter().flatten() {
                    add_geometry(&excluded["geometry"], &review_path, &mut paths)?;
                }
            }
        }
    }
    for changes in std::iter::once(&target["changes"]).chain(
        target["case_changes"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.values()),
    ) {
        for name in changes["set_files"].as_array().into_iter().flatten() {
            paths.insert(
                dependency_path(
                    name.as_str()
                        .ok_or_else(|| Error::new("invalid_path", "File path must be string"))?,
                    target_path,
                )?,
                "target_set".into(),
            );
        }
    }
    paths
        .into_iter()
        .map(|(path, role)| {
            let resolved = documents::resolve_path(&path.to_string_lossy(), suite_path)?;
            Ok(json!({"path":resolved,"source_path":path,"role":role,"sha256":common::sha256_file(&resolved)?}))
        })
        .collect()
}
fn renderer_environment(target: &Value, folder: &Path) -> (BTreeMap<String, String>, Value) {
    let mut allowed: BTreeMap<String, String> = ENV_KEYS
        .iter()
        .filter_map(|key| env::var(key).ok().map(|value| ((*key).into(), value)))
        .collect();
    if let Some(entries) = target["environment"].as_object() {
        for (key, value) in entries {
            if let Some(value) = value.as_str() {
                allowed.insert(key.clone(), value.into());
            }
        }
    }
    let conditions = json!(allowed);
    let mut values = allowed;
    values.insert("PATH".into(), env::var("PATH").unwrap_or_default());
    values.insert("LANG".into(), "C.UTF-8".into());
    values.insert("LC_ALL".into(), "C.UTF-8".into());
    values.insert(
        "SPEKTRAFILM_CONFIG_DIR".into(),
        folder.join("config").to_string_lossy().into(),
    );
    values.insert("RUST_LOG".into(), "off".into());
    // Windows process startup requires the OS directory even after environment isolation.
    #[cfg(windows)]
    for key in ["SYSTEMROOT", "WINDIR", "TEMP", "TMP"] {
        if let Ok(value) = env::var(key) {
            values.insert(key.into(), value);
        }
    }
    (values, conditions)
}
fn render_command(
    case: &Value,
    suite_path: &Path,
    target: &Value,
    target_path: &Path,
    folder: &Path,
) -> Result<Vec<String>> {
    let config = &case["configuration"];
    let mut command = vec![
        documents::resolve_path(text(&target["renderer"], "path")?, target_path)?
            .to_string_lossy()
            .into(),
        "process".into(),
        documents::resolve_path(text(&case["input"], "path")?, suite_path)?
            .to_string_lossy()
            .into(),
        "--output".into(),
        folder.join("master.tif").to_string_lossy().into(),
        "--backend".into(),
        text(target, "backend")?.into(),
        "--data-dir".into(),
        documents::resolve_path(text(target, "data_dir")?, target_path)?
            .to_string_lossy()
            .into(),
        "--format".into(),
        "tiff".into(),
        "--bit-depth".into(),
        "32".into(),
        "--compression".into(),
        "zip".into(),
    ];
    for (field, option) in [
        ("film", "--film"),
        ("paper", "--paper"),
        ("route", "--route"),
        ("scan_output", "--scan-output"),
    ] {
        if let Some(value) = config[field].as_str() {
            command.extend([option.into(), value.into()]);
        }
    }
    for value in config["presets"].as_array().into_iter().flatten() {
        command.extend([
            "--preset".into(),
            documents::resolve_path(
                value
                    .as_str()
                    .ok_or_else(|| Error::new("invalid_path", "Preset path must be string"))?,
                suite_path,
            )?
            .to_string_lossy()
            .into(),
        ]);
    }
    if let Some(value) = config["params"].as_str() {
        command.extend([
            "--params".into(),
            documents::resolve_path(value, suite_path)?
                .to_string_lossy()
                .into(),
        ]);
    }
    let case_changes = &target["case_changes"][text(case, "id")?];
    for (changes, document) in [
        (config, suite_path),
        (&target["changes"], target_path),
        (case_changes, target_path),
    ] {
        for value in changes["set_files"].as_array().into_iter().flatten() {
            command.extend([
                "--set".into(),
                documents::resolve_path(
                    value
                        .as_str()
                        .ok_or_else(|| Error::new("invalid_path", "Set file must be string"))?,
                    document,
                )?
                .to_string_lossy()
                .into(),
            ]);
        }
        for value in changes["set"].as_array().into_iter().flatten() {
            command.extend([
                "--set".into(),
                value
                    .as_str()
                    .ok_or_else(|| Error::new("inline_assignment", "Assignment must be string"))?
                    .into(),
            ]);
        }
    }
    if case["input"]["kind"].as_str().unwrap_or("raster") == "raster" {
        command.extend([
            "--input-color-space".into(),
            text(&case["input"], "color_space")?.into(),
            "--input-cctf-decoding".into(),
            case["input"]["cctf_decoding"].to_string(),
        ]);
    } else {
        for (key, option) in [
            ("white_balance", "--raw-white-balance"),
            ("temperature", "--raw-temperature"),
            ("tint", "--raw-tint"),
        ] {
            if let Some(value) = config["raw"].get(key) {
                command.extend([
                    option.into(),
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string()),
                ]);
            }
        }
        if config["raw"]["lens_correction"] == true {
            command.push("--lens-correction".into());
        }
    }
    Ok(command)
}
fn invoke(
    command: &[String],
    environment: &BTreeMap<String, String>,
    folder: &Path,
    name: &str,
    evidence: &mut Evidence,
) -> Result<(Output, Value)> {
    let result = Command::new(&command[0])
        .args(&command[1..])
        .env_clear()
        .envs(environment)
        .output()
        .map_err(|e| Error::new("renderer_start", e.to_string()).boundary("renderer"))?;
    let mut logs = json!({});
    for (stream, bytes) in [("stdout", &result.stdout), ("stderr", &result.stderr)] {
        let path = folder.join(format!("{name}.{stream}"));
        fs::write(&path, bytes)?;
        logs[stream] = evidence.add(&path, "renderer_log", "text/plain", json!({}))?;
    }
    Ok((result, logs))
}
fn renderer_success(output: &Output, code: &str, folder: &Path, name: &str) -> Result<()> {
    if !output.status.success() {
        let bytes = &output.stderr;
        let from = bytes.len().saturating_sub(2000);
        return Err(Error::new(
            code,
            format!(
                "{}; stdout: {}; stderr: {}",
                String::from_utf8_lossy(&bytes[from..]),
                folder.join(format!("{name}.stdout")).display(),
                folder.join(format!("{name}.stderr")).display()
            ),
        )
        .boundary("renderer"));
    }
    Ok(())
}
fn resolved_geometry(resolved: &Value, diagnostics: &Value) -> Value {
    let io = &resolved["parameters"]["io"];
    json!({"input_dimensions":diagnostics["configuration"]["input_dimensions"],"working_dimensions":diagnostics["configuration"]["working_dimensions"],"published_dimensions":diagnostics["configuration"]["published_dimensions"],"crop":io["crop"],"crop_center":io["crop_center"],"crop_size":io["crop_size"],"upscale_factor":io["upscale_factor"],"preview_mode":resolved["parameters"]["settings"]["preview_mode"],"preview_max_size":resolved["parameters"]["settings"]["preview_max_size"]})
}

/// Source-evidence route command for one simulation command: the same
/// configuration with the input route and linear ACES2065-1 RGB taps.
/// Neither ordinary simulation nor source-only execution captures native stages.
fn source_render_command(command: &[String], folder: &Path) -> Result<Vec<String>> {
    let mut source_command = command.to_vec();
    if let Some(i) = source_command.iter().position(|s| s == "--route") {
        source_command[i + 1] = "input".into();
    } else {
        source_command.extend(["--route".into(), "input".into()]);
    }
    let output_index = source_command
        .iter()
        .position(|s| s == "--output")
        .ok_or_else(|| Error::new("source_evidence", "Missing output option"))?;
    source_command[output_index + 1] = folder.join("source.tif").to_string_lossy().into();
    source_command.extend([
        "--set".into(), "io.output_color_space=ACES2065-1,io.output_cctf_encoding=false,cmyk.active=false,film_render.grain.active=false,taps.inject=rgb_in,taps.collect=rgb_out".into(),
        "--raw-out".into(), folder.join("source.f64").to_string_lossy().into(),
        "--diagnostics".into(), folder.join("source-diagnostics.json").to_string_lossy().into(),
    ]);
    Ok(source_command)
}

/// The single source-configuration resolver for execution and preflight: the
/// source route is dry-run from its own command, so both observe exactly the
/// configuration the source execution uses. The dry-run never decodes pixels
/// and never claims exposure or stage observation.
fn resolve_source_configuration(
    source_command: &[String],
    environment: &BTreeMap<String, String>,
    folder: &Path,
    evidence: &mut Evidence,
) -> Result<(Value, Value)> {
    let mut source_dry = source_command.to_vec();
    for option in ["--raw-out", "--diagnostics"] {
        if let Some(index) = source_dry.iter().position(|value| value == option) {
            source_dry.drain(index..index + 2);
        }
    }
    source_dry.push("--dry-run".into());
    let (result, _) = invoke(&source_dry, environment, folder, "source-dry-run", evidence)?;
    renderer_success(&result, "source_configuration", folder, "source-dry-run")?;
    let source_resolved: Value = serde_json::from_slice(&result.stdout)?;
    if source_resolved["parameters"]["workflow"]["route"] != "input"
        || source_resolved["parameters"]["io"]["output_color_space"] != "ACES2065-1"
        || source_resolved["parameters"]["io"]["output_cctf_encoding"] != false
        || source_resolved["parameters"]["cmyk"]["active"] != false
        || source_resolved["parameters"]["taps"]["inject"] != "rgb_in"
        || source_resolved["parameters"]["taps"]["collect"] != "rgb_out"
    {
        return Err(Error::new(
            "source_configuration",
            "Source route must resolve to input with linear ACES2065-1, CMYK disabled, and RGB input/output taps",
        ));
    }
    common::dump(&folder.join("source-resolved.json"), &source_resolved)?;
    let record = evidence.add(
        &folder.join("source-resolved.json"),
        "source_configuration",
        "application/json",
        json!({}),
    )?;
    Ok((source_resolved, record))
}

fn check_configuration(
    case: &Value,
    seed: u64,
    context: &mut RenderContext,
    folder: &Path,
    row: &mut Value,
) -> Result<Value> {
    let suite_path = context.suite_path;
    let target = context.target;
    let target_path = context.target_path;
    let evidence = &mut *context.evidence;
    let (environment, _) = renderer_environment(target, folder);
    let mut dry = render_command(case, suite_path, target, target_path, folder)?;
    dry.extend(["--seed".into(), seed.to_string(), "--dry-run".into()]);
    row["configuration_command"] = json!(dry);
    let (result, logs) = invoke(&dry, &environment, folder, "dry-run", evidence)?;
    row["stdout"] = logs["stdout"].clone();
    row["stderr"] = logs["stderr"].clone();
    renderer_success(&result, "renderer_configuration", folder, "dry-run")?;
    let resolved: Value = serde_json::from_slice(&result.stdout).map_err(|error| {
        Error::new(
            "renderer_configuration",
            format!(
                "Invalid renderer JSON: {error}; stdout: {}; stderr: {}",
                folder.join("dry-run.stdout").display(),
                folder.join("dry-run.stderr").display()
            ),
        )
        .boundary("renderer")
    })?;
    let path = folder.join("resolved.json");
    common::dump(&path, &resolved)?;
    row["resolved"] = evidence.add(
        &path,
        "resolved_configuration",
        "application/json",
        json!({}),
    )?;
    let data_dir = documents::resolve_path(text(target, "data_dir")?, target_path)?;
    for field in ["film_profile", "print_profile"] {
        let name = text(&resolved, field)?;
        let profile = data_dir.join("profiles").join(format!("{name}.json"));
        let canonical = profile.canonicalize().map_err(|_| {
            Error::new(
                "profile_dependency",
                format!("Resolved {field} must select a file inside target data_dir: {name}"),
            )
        })?;
        if !canonical.starts_with(&data_dir) || !canonical.is_file() {
            return Err(Error::new(
                "profile_dependency",
                format!("Resolved {field} escapes target data_dir: {name}"),
            ));
        }
    }
    let case_id = text(case, "id")?;
    let changes = &target["changes"];
    if changes["set"].as_array().is_some_and(|v| !v.is_empty())
        || changes["set_files"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        || target["case_changes"].get(case_id).is_some()
    {
        let mut case_target = target.clone();
        case_target["changes"] = json!({"set":[],"set_files":[]});
        case_target["case_changes"] = json!({});
        let mut base = render_command(case, suite_path, &case_target, target_path, folder)?;
        base.extend(["--seed".into(), seed.to_string(), "--dry-run".into()]);
        let (result, logs) = invoke(&base, &environment, folder, "case-configuration", evidence)?;
        row["case_configuration_logs"] = logs;
        renderer_success(
            &result,
            "renderer_configuration",
            folder,
            "case-configuration",
        )?;
        let base: Value = serde_json::from_slice(&result.stdout)?;
        for (section, names) in documents::CASE_OWNED_FIELDS {
            for name in *names {
                if resolved["parameters"][section][name] != base["parameters"][section][name] {
                    return Err(Error::new(
                        "target_geometry",
                        format!("Target must not change case-owned input field: {section}.{name}"),
                    ));
                }
            }
        }
    }
    Ok(resolved)
}

#[allow(clippy::too_many_arguments)]
fn run_case(
    case: &Value,
    seed: u64,
    suite_path: &Path,
    target: &Value,
    target_path: &Path,
    root: &Path,
    evidence: &mut Evidence,
) -> Result<Value> {
    let case_id = text(case, "id")?;
    let folder = root
        .join("cases")
        .join(case_id)
        .join(format!("seed-{seed}"));
    fs::create_dir_all(&folder)?;
    let (environment, conditions) = renderer_environment(target, &folder);
    let mut command = render_command(case, suite_path, target, target_path, &folder)?;
    command.extend(["--seed".into(), seed.to_string()]);
    let source_command = source_render_command(&command, &folder)?;
    // The retained command is the actual executed simulation argv, including
    // the evidence sinks, not the base argv they were appended to.
    let mut render = command;
    render.extend([
        "--raw-out".into(),
        folder.join("master.f64").to_string_lossy().into(),
        "--diagnostics".into(),
        folder.join("diagnostics.json").to_string_lossy().into(),
    ]);
    let mut row = json!({"case_id":case_id,"seed":seed,"status":"failed","environment":conditions,"command":render,"source_command":source_command});
    let attempt = (|| -> Result<()> {
        let source = documents::resolve_path(text(&case["input"], "path")?, suite_path)?;
        let mut declaration = case["input"].clone();
        let obj = declaration
            .as_object_mut()
            .ok_or_else(|| Error::new("source_declaration", "Input must be object"))?;
        obj.remove("path");
        obj.remove("sha256");
        let extension = source.extension().and_then(|s| s.to_str()).unwrap_or("bin");
        row["input"] = evidence.copy(
            &source,
            &format!("inputs/{case_id}.{extension}"),
            "source_input",
            json!({"declaration":declaration}),
        )?;
        let mut context = RenderContext {
            suite_path,
            target,
            target_path,
            root,
            evidence: &mut *evidence,
        };
        let resolved = check_configuration(case, seed, &mut context, &folder, &mut row)?;
        let (source_resolved, source_record) = resolve_source_configuration(
            &source_command,
            &environment,
            &folder,
            &mut *context.evidence,
        )?;
        row["source_resolved"] = source_record;
        let (result, _) = invoke(
            &source_command,
            &environment,
            &folder,
            "source-render",
            evidence,
        )?;
        renderer_success(&result, "source_evidence", &folder, "source-render")?;
        let source_diagnostics =
            documents::load_json_document(&folder.join("source-diagnostics.json"))?;
        let source_dimensions = &source_diagnostics["configuration"]["published_dimensions"];
        let source_width = source_dimensions[0]
            .as_u64()
            .ok_or_else(|| Error::new("source_geometry", "Missing source width"))?;
        let source_height = source_dimensions[1]
            .as_u64()
            .ok_or_else(|| Error::new("source_geometry", "Missing source height"))?;
        let source_layout = json!({"width":source_width,"height":source_height,"channels":3,"dtype":"f64","channel_order":"RGB","endianness":if cfg!(target_endian="little"){ "little" }else{ "big" }});
        let source_color = json!({"color_space":"ACES2065-1","cctf_decoding":false});
        let expected = source_width
            .checked_mul(source_height)
            .and_then(|n| n.checked_mul(24))
            .ok_or_else(|| Error::new("raw_layout", "Source dimensions overflow"))?;
        if source_width == 0
            || source_height == 0
            || fs::metadata(folder.join("source.f64"))?.len() != expected
        {
            return Err(Error::new(
                "raw_layout",
                "Source byte count disagrees with observed dimensions",
            ));
        }
        let mut source_metadata = source_layout.clone();
        source_metadata
            .as_object_mut()
            .unwrap()
            .extend(source_color.as_object().unwrap().clone());
        common::dump(&folder.join("source.f64.json"), &source_metadata)?;
        row["source_raw_metadata"] = evidence.add(
            &folder.join("source.f64.json"),
            "source_raw_metadata",
            "application/json",
            json!({}),
        )?;
        row["source_raw"] = evidence.add(
            &folder.join("source.f64"),
            "source_native_master",
            "application/octet-stream",
            json!({"layout":source_layout,"color":source_color}),
        )?;
        row["source_master"] = evidence.add(
            &folder.join("source.tif"),
            "source_floating_master",
            "image/tiff",
            json!({"dimensions":source_dimensions,"color":source_color}),
        )?;
        row["source_diagnostics"] = evidence.add(
            &folder.join("source-diagnostics.json"),
            "source_diagnostics",
            "application/json",
            json!({}),
        )?;
        row["source_dimensions"] = source_diagnostics["configuration"]["input_dimensions"].clone();
        row["source_geometry"] = resolved_geometry(&source_resolved, &source_diagnostics);
        if let Some(preparation) = case.get("preparation") {
            row["preparation"] = preparation.clone();
        }
        let (result, logs) = invoke(&render, &environment, &folder, "render", evidence)?;
        row["stdout"] = logs["stdout"].clone();
        row["stderr"] = logs["stderr"].clone();
        renderer_success(&result, "renderer_failed", &folder, "render")?;
        let diagnostics = documents::load_json_document(&folder.join("diagnostics.json"))?;
        let configuration = &diagnostics["configuration"];
        let dimensions = configuration["published_dimensions"]
            .as_array()
            .ok_or_else(|| Error::new("render_geometry", "Missing published dimensions"))?;
        let width = dimensions
            .first()
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::new("render_geometry", "Invalid width"))?;
        let height = dimensions
            .get(1)
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::new("render_geometry", "Invalid height"))?;
        let io = &resolved["parameters"]["io"];
        let space = text(io, "output_color_space")?;
        let encoded = io["output_cctf_encoding"]
            .as_bool()
            .ok_or_else(|| Error::new("source_declaration", "Invalid transfer declaration"))?;
        let color = json!({"color_space":space,"cctf_decoding":!encoded});
        let layout = json!({"width":width,"height":height,"channels":3,"dtype":"f64","channel_order":"RGB","endianness":if cfg!(target_endian="little"){"little"}else{"big"}});
        let expected = width
            .checked_mul(height)
            .and_then(|v| v.checked_mul(24))
            .ok_or_else(|| Error::new("raw_layout", "Dimensions overflow"))?;
        if fs::metadata(folder.join("master.f64"))?.len() != expected {
            return Err(Error::new(
                "raw_layout",
                "Renderer raw byte count disagrees with published dimensions",
            ));
        }
        let mut metadata = layout.clone();
        metadata
            .as_object_mut()
            .unwrap()
            .extend(color.as_object().unwrap().clone());
        common::dump(&folder.join("master.f64.json"), &metadata)?;
        row["raw_metadata"] = evidence.add(
            &folder.join("master.f64.json"),
            "raw_metadata",
            "application/json",
            json!({}),
        )?;
        row["raw"] = evidence.add(
            &folder.join("master.f64"),
            "native_master",
            "application/octet-stream",
            json!({"layout":layout,"color":color}),
        )?;
        row["master"] = evidence.add(
            &folder.join("master.tif"),
            "floating_master",
            "image/tiff",
            json!({"dimensions":[width,height],"color":color}),
        )?;
        row["diagnostics"] = evidence.add(
            &folder.join("diagnostics.json"),
            "render_diagnostics",
            "application/json",
            json!({}),
        )?;
        row["execution"] = diagnostics["execution"].clone();
        row["dimensions"] = json!([width, height]);
        row["simulation_geometry"] = resolved_geometry(&resolved, &diagnostics);
        let grain = &resolved["parameters"]["film_render"]["grain"];
        let dye = grain["active"] == true
            && grain["engine"] == "dye"
            && resolved["parameters"]["debug"]["deactivate_spatial_effects"] != true;
        let fotufilm = grain["active"] == true
            && grain["engine"] == "fotufilm"
            && resolved["parameters"]["debug"]["deactivate_stochastic_effects"] != true
            && grain["fotufilm"]["amount"]
                .as_f64()
                .is_some_and(|v| v > 0.0);
        row["source_pixel_aligned"] =
            json!(!dye && !fotufilm && row["source_geometry"] == row["simulation_geometry"]);
        row["precision"] = json!({"host_precision":configuration["host_precision"],"gpu_precision":configuration["gpu_precision"],"spectral_preparation_precision":configuration["spectral_preparation_precision"]});
        row["stage_evidence"] = json!({
            "requested": false,
            "availability": "not_applicable",
            "reason": "ordinary runs do not collect native observations",
            "context": unobserved_stage_context("ordinary runs do not collect native observations"),
        });
        row["status"] = json!("completed");
        Ok(())
    })();
    if let Err(error) = attempt {
        row["error"] = error.boundary(case_id).to_value();
    }
    Ok(row)
}

/// Explicit naming of exposure and stage boundaries that no capture observed.
/// Unobserved never masquerades as measured: no values are invented here.
fn unobserved_stage_context(reason: &str) -> Value {
    json!({
        "exposure": {"availability":"unavailable","reason":format!("{reason}; exposure unobserved")},
        "print_normalization": {"availability":"unavailable","reason":format!("{reason}; print normalization unobserved")},
        "terminal_cmyk": {"availability":"not_applicable","reason":"not_recorded_on_route"},
        "stages": documents::STAGE_IDS.iter().map(|id| json!({"id":id,"availability":"unavailable","reason":format!("{reason}; stage {id} unobserved")})).collect::<Vec<_>>(),
    })
}

/// Retain one validated run row into the comparison's evidence and return its
/// per-case `run_evidence` role payload: run/target/input identities, the
/// resolved-configuration and diagnostics references, the passive input
/// preparation declaration, the actual executed argv, environment and data
/// inventory identity, declared versus observed exposure/normalization, and
/// the stage bundle references. The stage bundle manifest and all of its
/// relative buffer dependencies are copied together as immutable indexed
/// artifacts. The run document was validated at load time, so this helper
/// only copies and hashes the artifacts the payload references.
pub fn retain_run_evidence(
    evidence: &mut Evidence,
    run_root: &Path,
    run: &Value,
    row: &Value,
    definition: &Value,
    role: &str,
) -> Result<Value> {
    let case_id = text(row, "case_id")?;
    let seed = row["seed"]
        .as_u64()
        .ok_or_else(|| Error::new("run_case", "invalid case seed"))?;
    let folder = format!("cases/{case_id}/seed-{seed}");
    let resolved_source = documents::safe_artifact_path(run_root, text(&row["resolved"], "path")?)?;
    let resolved_configuration = evidence.copy(
        &resolved_source,
        &format!("{folder}/{role}-resolved.json"),
        "resolved_configuration",
        json!({}),
    )?;
    let diagnostics = evidence.copy(
        &documents::safe_artifact_path(run_root, text(&row["diagnostics"], "path")?)?,
        &format!("{folder}/{role}-diagnostics.json"),
        "render_diagnostics",
        json!({}),
    )?;
    // Declared conditions come from the retained resolved configuration;
    // observed conditions come from the capture, or are named unobserved.
    let resolved_document = documents::load_json_document(&resolved_source)?;
    let parameters = &resolved_document["parameters"];
    let declared = json!({
        "exposure": {
            "auto_enabled": parameters["camera"]["auto_exposure"],
            "method": parameters["camera"]["auto_exposure_method"],
            "compensation_ev": parameters["camera"]["exposure_compensation_ev"],
        },
        "print_normalization": {
            "mode": "profile_prepared",
            "print_exposure": parameters["enlarger"]["print_exposure"],
            "print_exposure_mode": parameters["enlarger"]["print_exposure_mode"],
            "print_exposure_ev_tracking": parameters["settings"]["print_exposure_ev_tracking"],
        },
    });
    let mut stage_evidence = json!({
        "requested": false,
        "availability": "not_applicable",
        "reason": "stage capture not requested",
    });
    let mut observed = unobserved_stage_context("stage capture not requested");
    if let Some(stage) = row.get("stage_evidence").filter(|value| value.is_object()) {
        observed = stage["context"].clone();
        stage_evidence = json!({
            "requested": stage["requested"],
            "availability": stage["availability"],
        });
        if let Some(reason) = stage.get("reason") {
            stage_evidence["reason"] = reason.clone();
        }
        if stage["requested"] == json!(true) {
            let manifest_path = text(&stage["manifest"], "path")?;
            let bundle_dir = Path::new(manifest_path).parent().ok_or_else(|| {
                Error::new("stage_evidence", "bundle manifest must name its directory")
            })?;
            let destination = format!("{folder}/{role}-stage-evidence");
            let manifest = evidence.copy(
                &documents::safe_artifact_path(run_root, manifest_path)?,
                &format!("{destination}/stage-evidence.json"),
                "stage_evidence_manifest",
                json!({"case_id":case_id,"seed":seed,"role":role}),
            )?;
            let mut buffers = Vec::new();
            for record in stage["buffers"].as_array().into_iter().flatten() {
                let path = text(record, "path")?;
                let relative = Path::new(path)
                    .strip_prefix(bundle_dir)
                    .map_err(|_| {
                        Error::new(
                            "stage_evidence",
                            "buffer must live inside the bundle directory",
                        )
                    })?
                    .to_string_lossy()
                    .replace('\\', "/");
                buffers.push(evidence.copy(
                    &documents::safe_artifact_path(run_root, path)?,
                    &format!("{destination}/{relative}"),
                    "stage_evidence_buffer",
                    json!({"case_id":case_id,"seed":seed,"role":role,"stage_id":record["stage_id"],"layout":record["layout"]}),
                )?);
            }
            stage_evidence["manifest"] = manifest;
            stage_evidence["buffers"] = json!(buffers);
        }
    }
    let inventory = array(run, "dependencies")?
        .iter()
        .find(|record| record["role"] == json!("data_inventory"))
        .map(|record| record["sha256"].clone())
        .unwrap_or(Value::Null);
    let input = json!({
        "declaration": definition["input"],
        "source_dimensions": row["source_dimensions"],
        "source_geometry": row["source_geometry"],
        "simulation_geometry": row["simulation_geometry"],
        "source_pixel_aligned": row["source_pixel_aligned"],
        "source_raw": {"sha256": row["source_raw"]["sha256"], "layout": row["source_raw"]["layout"], "color": row["source_raw"]["color"]},
        "source_raw_metadata": {"sha256": row["source_raw_metadata"]["sha256"]},
        "source_master": {"sha256": row["source_master"]["sha256"], "dimensions": row["source_master"]["dimensions"], "color": row["source_master"]["color"]},
    });
    let target = documents::load_json_document(&documents::safe_artifact_path(
        run_root,
        text(&run["target"], "path")?,
    )?)?;
    let mut payload = json!({
        "role": role,
        "case_id": case_id,
        "seed": seed,
        "run": {"id": run["id"], "schema_version": run["schema_version"], "execution_status": run["execution_status"]},
        "target": {"id": target["id"], "digest": common::digest(&target)?},
        "input": input,
        "command": row["command"],
        "source_command": row["source_command"],
        "environment": row["environment"],
        "precision": row["precision"],
        "execution": row["execution"],
        "data_inventory": {"sha256": inventory},
        "resolved_configuration": resolved_configuration,
        "diagnostics": diagnostics,
        "declared": declared,
        "observed": observed,
        "stage_evidence": stage_evidence,
    });
    if let Some(preparation) = definition.get("preparation") {
        payload["preparation"] = preparation.clone();
    }
    Ok(payload)
}
struct RenderContext<'a> {
    suite_path: &'a Path,
    target: &'a Value,
    target_path: &'a Path,
    root: &'a Path,
    evidence: &'a mut Evidence,
}
fn render_cases<'a>(
    suite: &'a Value,
    selected: &[String],
    context: &mut RenderContext,
    rows: &mut Vec<Value>,
    progress: &mut Progress<'a>,
) -> Result<()> {
    for case in array(suite, "cases")? {
        let case_id = text(case, "id")?;
        if !selected.contains(&case_id.to_owned()) {
            continue;
        }
        for seed in array(case, "seeds")? {
            let seed = seed
                .as_u64()
                .ok_or_else(|| Error::new("invalid_seed", "Seed must be unsigned"))?;
            let started = progress.start(case_id, seed);
            let row = match run_case(
                case,
                seed,
                context.suite_path,
                context.target,
                context.target_path,
                context.root,
                context.evidence,
            ) {
                Ok(row) => row,
                Err(error) => {
                    json!({"case_id":case_id,"seed":seed,"status":"failed","error":error.boundary(case_id).to_value()})
                }
            };
            let status = row["status"].as_str().unwrap_or("failed").to_owned();
            progress.finish(case_id, seed, &status, started);
            rows.push(row);
        }
    }
    Ok(())
}
/// Validate every selected case and seed through renderer dry-run without decoding or rendering.
/// Requires a new output directory and retains configuration evidence in `preflight.json`.
pub fn preflight(
    suite_path: &Path,
    target_path: &Path,
    out: &Path,
    selections: &[String],
) -> Result<(Value, i32)> {
    let suite_path = suite_path.canonicalize()?;
    let suite = documents::load_suite(&suite_path)?;
    preflight_suite(suite, &suite_path, target_path, out, selections)
}

/// Preflight an in-memory suite using `suite_path` as its dependency resolution anchor.
/// Every seed is checked independently; the retained manifest cannot serve as a completed run.
pub fn preflight_suite(
    suite: Value,
    suite_path: &Path,
    target_path: &Path,
    out: &Path,
    selections: &[String],
) -> Result<(Value, i32)> {
    execute_suite(suite, suite_path, target_path, out, selections, true)
}

fn preflight_cases<'a>(
    suite: &'a Value,
    selected: &[String],
    context: &mut RenderContext,
    rows: &mut Vec<Value>,
    progress: &mut Progress<'a>,
) -> Result<()> {
    for case in array(suite, "cases")? {
        let case_id = text(case, "id")?;
        if !selected.iter().any(|id| id == case_id) {
            continue;
        }
        for seed in array(case, "seeds")? {
            let seed = seed
                .as_u64()
                .ok_or_else(|| Error::new("invalid_seed", "Seed must be unsigned"))?;
            let started = progress.start(case_id, seed);
            let folder = context
                .root
                .join("cases")
                .join(case_id)
                .join(format!("seed-{seed}"));
            let mut row = json!({"case_id":case_id,"seed":seed,"status":"failed","environment":renderer_environment(context.target,&folder).1});
            let result = (|| -> Result<()> {
                fs::create_dir_all(&folder)?;
                let (environment, _) = renderer_environment(context.target, &folder);
                check_configuration(case, seed, context, &folder, &mut row)?;
                // The source route preflight resolves exactly the source
                // configuration the execution will use, through the same
                // resolver; no pixels are decoded, so exposure, stages and
                // source geometry stay explicitly unobserved.
                let mut command = render_command(
                    case,
                    context.suite_path,
                    context.target,
                    context.target_path,
                    &folder,
                )?;
                command.extend(["--seed".into(), seed.to_string()]);
                let source_command = source_render_command(&command, &folder)?;
                let (_, record) = resolve_source_configuration(
                    &source_command,
                    &environment,
                    &folder,
                    &mut *context.evidence,
                )?;
                row["source_command"] = json!(source_command);
                row["source_resolved"] = record;
                row["stage_evidence"] = json!({
                    "requested": false,
                    "availability": "not_applicable",
                    "reason": "ordinary runs do not collect native observations",
                    "context": unobserved_stage_context("dry-run does not render"),
                });
                row["status"] = json!("validated");
                Ok(())
            })();
            if let Err(error) = result {
                row["error"] = error.boundary(case_id).to_value();
            }
            let status = row["status"].as_str().unwrap_or("failed").to_owned();
            progress.finish(case_id, seed, &status, started);
            rows.push(row);
        }
    }
    Ok(())
}

fn preflight_errors(rows: &[Value], root: &Path) -> Result<Vec<Value>> {
    let mut groups: Vec<(Value, String, Value)> = Vec::new();
    for row in rows {
        let Some(error) = row.get("error") else {
            continue;
        };
        // Compare complete captured diagnostics, not path-bearing summaries. Every seed
        // has already been checked, so grouping never implies unchecked seed validity.
        let logs = if row["case_configuration_logs"].is_object() {
            &row["case_configuration_logs"]
        } else {
            row
        };
        let diagnostic = if error["code"] == "renderer_configuration" {
            let mut streams = Vec::new();
            for stream in ["stdout", "stderr"] {
                if let Some(path) = logs[stream]["path"].as_str() {
                    streams.push(
                        json!({"stream":stream,"sha256":common::sha256_file(&root.join(path))?}),
                    );
                }
            }
            if streams.is_empty() {
                error["message"].to_string()
            } else {
                serde_json::to_string(&streams)?
            }
        } else {
            error["message"].to_string()
        };
        if let Some((_, _, group)) = groups.iter_mut().find(|(identity, message, _)| {
            *identity == json!([row["case_id"], error["code"]]) && *message == diagnostic
        }) {
            group["affected_seeds"]
                .as_array_mut()
                .expect("seed array")
                .push(row["seed"].clone());
            group["evidence"].as_array_mut().expect("evidence array").push(json!({"seed":row["seed"],"stdout":row["stdout"],"stderr":row["stderr"],"case_configuration_logs":row["case_configuration_logs"]}));
        } else {
            groups.push((json!([row["case_id"],error["code"]]),diagnostic,json!({"case_id":row["case_id"],"affected_seeds":[row["seed"]],"error":error,"evidence":[{"seed":row["seed"],"stdout":row["stdout"],"stderr":row["stderr"],"case_configuration_logs":row["case_configuration_logs"]}]})));
        }
    }
    Ok(groups.into_iter().map(|(_, _, group)| group).collect())
}

pub fn run(
    suite_path: &Path,
    target_path: &Path,
    out: &Path,
    selections: &[String],
) -> Result<(Value, i32)> {
    let suite_path = suite_path.canonicalize()?;
    let target_path = target_path.canonicalize()?;
    let suite = documents::load_suite(&suite_path)?;
    run_suite(suite, &suite_path, &target_path, out, selections)
}

pub fn run_suite(
    suite: Value,
    suite_path: &Path,
    target_path: &Path,
    out: &Path,
    selections: &[String],
) -> Result<(Value, i32)> {
    execute_suite(suite, suite_path, target_path, out, selections, false)
}

fn execute_suite(
    suite: Value,
    suite_path: &Path,
    target_path: &Path,
    out: &Path,
    selections: &[String],
    dry_run: bool,
) -> Result<(Value, i32)> {
    let suite_path = documents::normalize_path(suite_path)?;
    documents::validate_suite(&suite, &suite_path)?;
    let target_path = target_path.canonicalize()?;
    let target = documents::load_target(&target_path)?;
    documents::validate_target_cases(&target, &suite)?;
    let all_ids: Vec<String> = array(&suite, "cases")?
        .iter()
        .map(|c| text(c, "id").map(str::to_owned))
        .collect::<Result<_>>()?;
    let requested = if selections.is_empty() {
        &all_ids
    } else {
        selections
    };
    let mut unique = std::collections::BTreeSet::new();
    let mut unknown = Vec::new();
    let mut duplicate = Vec::new();
    for id in requested {
        if !all_ids.contains(id) {
            unknown.push(id);
        }
        if !unique.insert(id) {
            duplicate.push(id);
        }
    }
    if !unknown.is_empty() || !duplicate.is_empty() {
        return Err(Error::new(
            "case_selection",
            format!(
                "Unknown case IDs: {unknown:?}; duplicate case IDs: {duplicate:?}; available case IDs: {all_ids:?}"
            ),
        ));
    }
    let selected: Vec<_> = all_ids
        .iter()
        .filter(|id| requested.contains(id))
        .cloned()
        .collect();
    let dependencies = file_dependencies(&suite, &suite_path, &target, &target_path)?;
    let data_root = documents::resolve_path(text(&target, "data_dir")?, &target_path)?;
    let root = documents::normalize_path(out)?;
    common::create_destination(&root)?;
    let result = (|| -> Result<(Value, i32)> {
        let mut evidence = Evidence::new(&root);
        let inventory =
            tree_inventory(&data_root).map_err(|e| e.file(data_root.display().to_string()));
        common::dump(&root.join("suite.json"), &suite)?;
        common::dump(&root.join("target.json"), &target)?;
        let suite_record = evidence.add(
            &root.join("suite.json"),
            "suite",
            "application/json",
            json!({"digest":common::digest(&suite)?,"id":suite["id"],"source_path":suite_path}),
        )?;
        let target_record = evidence.add(
            &root.join("target.json"),
            "target",
            "application/json",
            json!({"digest":common::digest(&target)?,"id":target["id"],"source_path":target_path}),
        )?;
        let mut retained = Vec::new();
        for (i, dependency) in dependencies.iter().enumerate() {
            if dependency["role"] == "renderer" {
                retained.push(dependency.clone());
            } else {
                let source = Path::new(text(dependency, "path")?);
                let ext = source
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| format!(".{s}"))
                    .unwrap_or_default();
                let mut record = evidence.copy(
                    source,
                    &format!("dependencies/{i:04}{ext}"),
                    text(dependency, "role")?,
                    json!({}),
                )?;
                record["source_path"] = dependency["source_path"].clone();
                if dependency["role"] == "correspondence_review" {
                    record["resolved_source_path"] = dependency["path"].clone();
                }
                retained.push(record);
            }
        }
        if let Ok(inventory) = &inventory {
            common::dump(&root.join("data-inventory.json"), inventory)?;
            retained.push(evidence.add(
                &root.join("data-inventory.json"),
                "data_inventory",
                "application/json",
                json!({}),
            )?);
        }
        let mut rows = Vec::new();
        let mut progress = Progress::new("run");
        let mut context = RenderContext {
            suite_path: &suite_path,
            target: &target,
            target_path: &target_path,
            root: &root,
            evidence: &mut evidence,
        };
        let processing = if dry_run {
            preflight_cases(&suite, &selected, &mut context, &mut rows, &mut progress)
        } else {
            render_cases(&suite, &selected, &mut context, &mut rows, &mut progress)
        };
        if let Err(error) = processing {
            progress.summary_rows(&rows);
            return Err(error);
        }
        if let Err(error) = &inventory {
            for row in &mut rows {
                if row["status"] == "completed" || row["status"] == "validated" {
                    row["status"] = json!("failed");
                    row["error"] = error.to_value();
                }
            }
        }
        let mut changed = Vec::new();
        if let Ok(inventory) = &inventory {
            for dependency in &dependencies {
                if common::sha256_file(Path::new(text(dependency, "path")?))?
                    != text(dependency, "sha256")?
                {
                    changed.push(dependency["path"].clone());
                }
            }
            if &tree_inventory(&data_root)? != inventory {
                changed.push(json!(data_root));
            }
        }
        if !changed.is_empty() {
            for row in &mut rows {
                row["status"] = json!("failed");
                row["error"] = json!({"code":"dependencies_changed","message":"Dependencies changed during rendering","paths":changed});
            }
        }
        progress.summary_rows(&rows);
        if dry_run {
            let valid = rows.iter().all(|row| row["status"] == "validated");
            let errors = preflight_errors(&rows, &root)?;
            let unchecked = json!([
                {"check":"pixel_decode","status":"unchecked","reason":"Dry-run does not decode images"},
                {"check":"gpu_execution","status":"unchecked","reason":"Dry-run does not initialize or execute a GPU"},
                {"check":"output_write","status":"unchecked","reason":"Dry-run does not render or publish images"},
                {"check":"observed_output_geometry","status":"unchecked","reason":"Dry-run does not decode or render images to observe output dimensions"},
                {"check":"numerical_output","status":"unchecked","reason":"Dry-run produces no numerical image evidence"},
                {"check":"exposure_observation","status":"unchecked","reason":"Dry-run never applies exposure, so metered and applied exposure stay unobserved"},
                {"check":"print_normalization_observation","status":"unchecked","reason":"Dry-run never prepares print normalization, so its value stays unobserved"},
                {"check":"stage_evidence","status":"unchecked","reason":"Ordinary runs do not collect native observations; stage boundaries stay unobserved"}
            ]);
            let mut manifest = json!({"schema_version":1,"kind":"preflight","mode":"dry_run","execution_status":if valid{"validated"}else{"failed"},"suite":suite_record,"target":target_record,"protocol":suite["protocol"],"selection":{"case_ids":selected,"full_suite":selected==all_ids},"dependencies":retained,"cases":rows,"artifacts":evidence.values(),"environment":renderer_environment(&target,&root).1,"errors":errors,"unchecked":unchecked});
            manifest["id"] = json!(common::digest(&manifest)?);
            common::dump(&root.join("preflight.json.tmp"), &manifest)?;
            fs::rename(root.join("preflight.json.tmp"), root.join("preflight.json"))?;
            return Ok((
                json!({"schema_version":1,"command":"run","mode":"dry_run","execution_status":manifest["execution_status"],"artifact":root.join("preflight.json"),"preflight_id":manifest["id"],"errors":manifest["errors"],"unchecked":manifest["unchecked"]}),
                if valid { 0 } else { 1 },
            ));
        }
        let completed = rows.iter().all(|r| r["status"] == "completed");
        let errors:Vec<_>=rows.iter().filter_map(|r|r.get("error").map(|e|json!({"case_id":r["case_id"],"seed":r["seed"],"boundary":"renderer","error":e}))).collect();
        let mut manifest = json!({"schema_version":1,"kind":"run","execution_status":if completed{"completed"}else{"failed"},"suite":suite_record,"target":target_record,"protocol":suite["protocol"],"selection":{"case_ids":selected,"full_suite":selected==all_ids},"dependencies":retained,"cases":rows,"artifacts":evidence.values(),"environment":renderer_environment(&target,&root).1});
        finalize(&root, "run.json", &mut manifest)?;
        let mut result = json!({"schema_version":1,"command":"run","execution_status":manifest["execution_status"],"artifact":root.join("run.json"),"run_id":manifest["id"]});
        if !completed {
            result["errors"] = json!(errors);
        }
        Ok((result, if completed { 0 } else { 1 }))
    })();
    result.map_err(|error| {
        common::incomplete_destination(
            error,
            &root,
            if dry_run {
                "preflight.json"
            } else {
                "run.json"
            },
        )
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn symlink_parent_dependencies_keep_distinct_files_and_lookup_identities() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("actual/subdir")).unwrap();
        std::os::unix::fs::symlink(root.join("actual/subdir"), root.join("linked-dir")).unwrap();
        fs::write(root.join("actual/input.png"), b"resolved pixels").unwrap();
        fs::write(root.join("input.png"), b"lexically adjacent pixels").unwrap();
        fs::write(root.join("renderer"), b"renderer identity").unwrap();
        let suite = json!({"cases":[{"input":{"path":"linked-dir/../input.png"},"configuration":{},"regions":[]},{"input":{"path":"input.png"},"configuration":{},"regions":[]}]});
        let target = json!({"renderer":{"path":"renderer"},"changes":{}});
        let dependencies = file_dependencies(
            &suite,
            &root.join("suite.json"),
            &target,
            &root.join("target.json"),
        )
        .unwrap();
        let linked = dependencies
            .iter()
            .find(|row| row["source_path"] == json!(root.join("linked-dir/../input.png")))
            .unwrap();
        let local = dependencies
            .iter()
            .find(|row| row["source_path"] == json!(root.join("input.png")))
            .unwrap();
        assert_eq!(
            linked["path"],
            json!(root.join("actual/input.png").canonicalize().unwrap())
        );
        assert_eq!(
            linked["sha256"],
            json!(common::sha256_file(&root.join("actual/input.png")).unwrap())
        );
        assert_ne!(linked["sha256"], local["sha256"]);
    }
}
