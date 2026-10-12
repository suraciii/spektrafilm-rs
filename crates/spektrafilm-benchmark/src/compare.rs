use crate::{
    Error, Result,
    common::{self, Evidence},
    documents,
    image_data::{self, ImageData},
    metrics, references,
    runner::{self, array, text},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

fn load_run(root: &Path) -> Result<(Value, Value)> {
    let value = documents::load_json_document(&root.join("run.json"))?;
    documents::validate_generated_document("run", &value, root)?;
    let suite = documents::load_json_document(&documents::safe_artifact_path(
        root,
        text(&value["suite"], "path")?,
    )?)?;
    Ok((value, suite))
}
fn raw_image(root: &Path, row: &Value) -> Result<ImageData> {
    let mut declaration = row["raw"]["layout"].clone();
    let map = declaration
        .as_object_mut()
        .ok_or_else(|| Error::new("raw_layout", "Missing raw layout"))?;
    map.extend(
        row["raw"]["color"]
            .as_object()
            .ok_or_else(|| Error::new("source_declaration", "Missing native color declaration"))?
            .clone(),
    );
    map.insert(
        "path".into(),
        json!(documents::safe_artifact_path(
            root,
            text(&row["raw"], "path")?
        )?),
    );
    image_data::load_image(&declaration)
}
fn review_image(
    image: &ImageData,
    destination: &Path,
    evidence: &mut Evidence,
    role: &str,
    case_id: &str,
    seed: u64,
) -> Result<Value> {
    let samples = image_data::display_rgb(image)?;
    if samples.iter().flatten().any(|v| !v.is_finite()) {
        return Err(Error::new(
            "nonfinite_review",
            "Review image has nonfinite display samples",
        ));
    }
    image_data::write_png16(destination, &samples, image.width, image.height)?;
    let (fit, w, h) = image_data::resize_display(&samples, image.width, image.height, 1600)?;
    let fit_path = destination.with_file_name(format!(
        "{}-fit.png",
        destination
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(role)
    ));
    image_data::write_png16(&fit_path, &fit, w, h)?;
    let fit_record=evidence.add(&fit_path,"fit_preview","image/png",json!({"width":w,"height":h,"dimensions":[w,h],"source_dimensions":image.dimensions(),"bit_depth":16,"resampling":"Lanczos in encoded sRGB before quantization","quantization":"16-bit rounded; no dither","color":{"color_space":"sRGB","transfer":"srgb"},"case_id":case_id,"seed":seed}))?;
    evidence.add(destination,role,"image/png",json!({"width":image.width,"height":image.height,"dimensions":image.dimensions(),"source_dimensions":image.dimensions(),"bit_depth":16,"resampling":"none","quantization":"16-bit rounded; no dither","color":{"color_space":"sRGB","transfer":"srgb"},"fit_preview":fit_record["path"],"case_id":case_id,"seed":seed}))
}
fn retain_metrics(
    mut measured: Value,
    folder: &Path,
    evidence: &mut Evidence,
    case_id: &str,
    seed: u64,
) -> Result<Value> {
    let mut refs = BTreeMap::<String, String>::new();
    let mut records = Vec::new();
    for artifact in measured["artifacts"].as_array().into_iter().flatten() {
        let mut metadata = artifact.clone();
        let obj = metadata
            .as_object_mut()
            .ok_or_else(|| Error::new("artifact_identity", "Artifact must be object"))?;
        for key in [
            "id",
            "path",
            "sha256",
            "media_type",
            "role",
            "availability",
            "case_id",
            "seed",
        ] {
            obj.remove(key);
        }
        let id = artifact["id"].as_str().unwrap_or(text(artifact, "path")?);
        metadata["id"] = json!(format!("{case_id}/seed-{seed}/{id}"));
        metadata["case_id"] = json!(case_id);
        metadata["seed"] = json!(seed);
        let record = evidence.add(
            &documents::safe_artifact_path(folder, text(artifact, "path")?)?,
            text(artifact, "role")?,
            artifact["media_type"]
                .as_str()
                .unwrap_or("application/octet-stream"),
            metadata,
        )?;
        refs.insert(id.into(), text(&record, "path")?.into());
        records.push(record);
    }
    fn rewrite_list(value: &mut Value, refs: &BTreeMap<String, String>) {
        if let Some(items) = value.as_array_mut() {
            for item in items {
                if let Some(path) = item.as_str().and_then(|s| refs.get(s)) {
                    *item = json!(path);
                }
            }
        }
    }
    if let Some(rows) = measured["measurements"].as_array_mut() {
        for row in rows {
            rewrite_list(&mut row["artifact_refs"], &refs);
            row["case_id"] = json!(case_id);
        }
    }
    if let Some(rows) = measured["findings"].as_array_mut() {
        for row in rows {
            rewrite_list(&mut row["evidence"], &refs);
        }
    }
    if let Some(regions) = measured["automatic_regions"].as_array_mut() {
        for region in regions {
            rewrite_list(&mut region["artifact_refs"], &refs);
        }
    }
    if let Some(review) = measured["reference_review"].as_object_mut() {
        for (key, value) in review {
            if key.ends_with("_artifact_id") {
                if let Some(path) = value.as_str().and_then(|s| refs.get(s)) {
                    *value = json!(path);
                }
            } else if key == "artifact_refs" {
                rewrite_list(value, &refs);
            }
        }
    }
    measured["artifacts"] = json!(records);
    Ok(measured)
}
fn retained_case(
    case: &Value,
    run: &Value,
    run_root: &Path,
    root: &Path,
    evidence: &mut Evidence,
) -> Result<Value> {
    let mut clone = case.clone();
    clone["run_protocol"] = run["protocol"].clone();
    if clone["probe"]["tone_response"].is_null()
        && let Some(tone) = run["protocol"]["probe"].get("tone_response")
    {
        if clone.get("probe").is_none() {
            clone["probe"] = json!({});
        }
        clone["probe"]["tone_response"] = tone.clone();
    }
    let authored = Path::new(text(&run["suite"], "source_path")?);
    let dependencies: BTreeMap<_, _> = array(run, "dependencies")?
        .iter()
        .filter_map(|row| row["source_path"].as_str().map(|s| (s, row)))
        .collect();
    let case_id = text(case, "id")?;
    let provenance = case["reference"]["provenance"].clone();
    struct Context<'a> {
        authored: &'a Path,
        dependencies: &'a BTreeMap<&'a str, &'a Value>,
        run_root: &'a Path,
        root: &'a Path,
        case_id: &'a str,
        provenance: &'a Value,
    }
    fn visit(
        item: &mut Value,
        context: &Context<'_>,
        evidence: &mut Evidence,
        reference: bool,
    ) -> Result<()> {
        match item {
            Value::Object(map) => {
                if let Some(path) = map.get("path").and_then(Value::as_str) {
                    // Resolve lexically; original files are deliberately not required by compare.
                    let original = lexical_absolute(
                        &context
                            .authored
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join(path),
                    );
                    let dependency = context
                        .dependencies
                        .get(original.to_string_lossy().as_ref())
                        .ok_or_else(|| {
                            Error::new(
                                "missing_dependency",
                                format!("No retained source for {}", original.display()),
                            )
                        })?;
                    let extension = original
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| format!(".{s}"))
                        .unwrap_or_default();
                    let relative = format!(
                        "retained/{}/{}{extension}",
                        context.case_id,
                        text(dependency, "sha256")?
                    );
                    let destination = context.root.join(&relative);
                    if !destination.exists() {
                        evidence.copy(
                            &documents::safe_artifact_path(
                                context.run_root,
                                text(dependency, "path")?,
                            )?,
                            &relative,
                            text(dependency, "role")?,
                            json!({}),
                        )?;
                    }
                    if reference {
                        let record = evidence.records.get_mut(&relative).ok_or_else(|| {
                            Error::new("missing_artifact", "Retained artifact absent")
                        })?;
                        record["redistribution"] = context.provenance["redistribution"].clone();
                        record["provenance"] = context.provenance.clone();
                    }
                    map.insert("path".into(), json!(destination));
                }
                for child in map.values_mut() {
                    visit(child, context, evidence, reference)?;
                }
            }
            Value::Array(items) => {
                for item in items {
                    visit(item, context, evidence, reference)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let context = Context {
        authored,
        dependencies: &dependencies,
        run_root,
        root,
        case_id,
        provenance: &provenance,
    };
    if let Some(protocol) = clone.get_mut("run_protocol") {
        visit(protocol, &context, evidence, false)?;
    }
    for key in ["input", "reference", "regions", "probe_response", "probe"] {
        if let Some(value) = clone.get_mut(key) {
            visit(value, &context, evidence, key == "reference")?;
        }
    }
    if let Some(retained) = clone["reference"].get("correspondence_review") {
        let mut review = documents::load_json_document(Path::new(text(retained, "path")?))?;
        visit(&mut review, &context, evidence, true)?;
        let path = root
            .join("retained")
            .join(case_id)
            .join("resolved-correspondence-review.json");
        common::dump(&path, &review)?;
        let record = evidence.add(
            &path,
            "resolved_correspondence_review",
            "application/json",
            json!({}),
        )?;
        clone["reference"]["correspondence_review"]["path"] = json!(path);
        clone["reference"]["correspondence_review"]["sha256"] = record["sha256"].clone();
    }
    Ok(clone)
}
fn lexical_absolute(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
fn probe_protocol(protocol: &Value, case: &Value, seed: u64) -> Value {
    let mut result = protocol.clone();
    result["case_id"] = case["id"].clone();
    result["seed_population"] = json!([seed]);
    result["measurements"] = case["measurements"].clone();
    if let Some(retained_protocol) = case.get("run_protocol") {
        for key in ["probe", "halation", "edge"] {
            if let Some(probe) = retained_protocol.get(key) {
                result[key] = probe.clone();
            }
        }
    }
    for key in ["halation", "edge"] {
        if let Some(probe) = case["probe"].get(key) {
            result[key] = probe.clone();
        }
    }
    if let Some(tone) = case["probe"].get("tone_response") {
        result["probe"] = json!({"tone_response":tone});
    }
    result
}
fn rule_result(rule: &Value, measurements: &[Value]) -> Value {
    let rows: Vec<_> = measurements
        .iter()
        .filter(|row| {
            row["case_id"] == rule["selector"]["case_id"]
                && row["region_id"] == rule["selector"]["region_id"]
                && row["metric"] == rule["metric"]
        })
        .collect();
    let values: Vec<_> = rows
        .iter()
        .filter(|row| row["availability"] == "available")
        .filter_map(|row| row["value"].as_f64())
        .collect();
    let satisfied = |value: f64| {
        rule["min"].as_f64().is_none_or(|min| value >= min)
            && rule["max"].as_f64().is_none_or(|max| value <= max)
            && match rule["direction"].as_str() {
                Some("increase") => value > 0.0,
                Some("decrease") => value < 0.0,
                Some("unchanged") => value == 0.0,
                _ => true,
            }
    };
    if values.iter().all(|v| satisfied(*v))
        && (rows.is_empty() || rows.iter().any(|row| row["availability"] != "available"))
    {
        json!({"rule_id":rule["id"],"status":"unavailable","required":rule["required"],"reason":"missing_or_unavailable_measurement","values":values})
    } else {
        json!({"rule_id":rule["id"],"status":if values.iter().all(|v|satisfied(*v)){"passed"}else{"failed"},"required":rule["required"],"values":values,"population":"all selected seed measurements"})
    }
}
fn condition_differences(
    baseline: &Value,
    candidate: &Value,
    left_root: &Path,
    right_root: &Path,
) -> Result<Vec<Value>> {
    let targets = [
        documents::load_json_document(&documents::safe_artifact_path(
            left_root,
            text(&baseline["target"], "path")?,
        )?)?,
        documents::load_json_document(&documents::safe_artifact_path(
            right_root,
            text(&candidate["target"], "path")?,
        )?)?,
    ];
    let mut result = Vec::new();
    fn difference(
        result: &mut Vec<Value>,
        field: &str,
        left: &Value,
        right: &Value,
        declared: bool,
        context: &Value,
    ) {
        if left != right {
            let mut value =
                json!({"field":field,"baseline":left,"candidate":right,"declared":declared});
            if let Some(context) = context.as_object() {
                value.as_object_mut().unwrap().extend(context.clone());
            }
            result.push(value);
        }
    }
    let renderer = targets[0]["renderer"]["sha256"] != targets[1]["renderer"]["sha256"];
    let data = targets[0]["data_dir"] != targets[1]["data_dir"];
    let changes = targets[0]["changes"] != targets[1]["changes"]
        || targets[0]["case_changes"] != targets[1]["case_changes"];
    difference(
        &mut result,
        "renderer",
        &targets[0]["renderer"]["sha256"],
        &targets[1]["renderer"]["sha256"],
        renderer,
        &Value::Null,
    );
    let inventory = |run: &Value, root: &Path| -> Result<Value> {
        let record = array(run, "dependencies")?
            .iter()
            .find(|r| r["role"] == "data_inventory")
            .ok_or_else(|| Error::new("missing_dependency", "Missing data inventory"))?;
        Ok(
            documents::load_json_document(&documents::safe_artifact_path(
                root,
                text(record, "path")?,
            )?)?["sha256"]
                .clone(),
        )
    };
    difference(
        &mut result,
        "data_inventory",
        &inventory(baseline, left_root)?,
        &inventory(candidate, right_root)?,
        data,
        &Value::Null,
    );
    let deps = |run: &Value| -> Result<BTreeMap<String, Value>> {
        array(run, "dependencies")?
            .iter()
            .map(|row| {
                Ok((
                    row["source_path"]
                        .as_str()
                        .unwrap_or(text(row, "path")?)
                        .into(),
                    json!({"role":row["role"],"sha256":row["sha256"]}),
                ))
            })
            .collect()
    };
    let left_deps = deps(baseline)?;
    let right_deps = deps(candidate)?;
    for source in left_deps
        .keys()
        .chain(right_deps.keys())
        .collect::<BTreeSet<_>>()
    {
        let left = left_deps.get(source).unwrap_or(&Value::Null);
        let right = right_deps.get(source).unwrap_or(&Value::Null);
        let role = left.get("role").or_else(|| right.get("role"));
        if role != Some(&json!("renderer")) && role != Some(&json!("data_inventory")) {
            difference(
                &mut result,
                "dependency",
                left,
                right,
                changes && (left.is_null() || right.is_null()),
                &json!({"source_path":source}),
            );
        }
    }
    difference(
        &mut result,
        "environment",
        &baseline["environment"],
        &candidate["environment"],
        targets[0]["environment"] != targets[1]["environment"],
        &Value::Null,
    );
    let right_rows: BTreeMap<_, _> = array(candidate, "cases")?
        .iter()
        .map(|r| {
            (
                (
                    r["case_id"].as_str().unwrap_or(""),
                    r["seed"].as_u64().unwrap_or(0),
                ),
                r,
            )
        })
        .collect();
    for left in array(baseline, "cases")? {
        let key = (
            text(left, "case_id")?,
            left["seed"]
                .as_u64()
                .ok_or_else(|| Error::new("invalid_seed", "Missing seed"))?,
        );
        let right = right_rows
            .get(&key)
            .ok_or_else(|| Error::new("incompatible_runs", "Candidate case missing"))?;
        if left["status"] != "completed" || right["status"] != "completed" {
            continue;
        }
        let context = json!({"case_id":key.0,"seed":key.1});
        for (side, row, target) in [
            ("baseline", left, &targets[0]),
            ("candidate", *right, &targets[1]),
        ] {
            let requested = text(target, "backend")?;
            let expected = if requested == "gpu" { "wgpu" } else { "cpu" };
            if row["execution"]["backend_selected"] != expected
                || (requested == "gpu" && row["execution"]["path"] == "cpu")
            {
                result.push(json!({"field":"backend_route","side":side,"requested":requested,"observed":row["execution"]["backend_selected"],"declared":false,"case_id":key.0,"seed":key.1}));
            }
            if requested == "gpu" && row["execution"]["adapter"]["device_type"] == "cpu" {
                result.push(json!({"field":"gpu_adapter","side":side,"requested":"gpu","observed":row["execution"]["adapter"],"declared":false,"case_id":key.0,"seed":key.1}));
            }
        }
        let backend = targets[0]["backend"] != targets[1]["backend"];
        difference(
            &mut result,
            "execution",
            &left["execution"],
            &right["execution"],
            backend,
            &context,
        );
        difference(
            &mut result,
            "precision",
            &left["precision"],
            &right["precision"],
            renderer || backend,
            &context,
        );
        let left_parameters = documents::load_json_document(&documents::safe_artifact_path(
            left_root,
            text(&left["resolved"], "path")?,
        )?)?["parameters"]
            .clone();
        let right_parameters = documents::load_json_document(&documents::safe_artifact_path(
            right_root,
            text(&right["resolved"], "path")?,
        )?)?["parameters"]
            .clone();
        difference(
            &mut result,
            "resolved_parameters",
            &left_parameters,
            &right_parameters,
            changes || renderer || data,
            &context,
        );
        for (section, names) in [
            (
                "io",
                &[
                    "input_color_space",
                    "input_cctf_decoding",
                    "crop",
                    "crop_center",
                    "crop_size",
                    "upscale_factor",
                ][..],
            ),
            ("settings", &["preview_mode", "preview_max_size"][..]),
        ] {
            for name in names {
                if left_parameters[section][name] != right_parameters[section][name] {
                    return Err(Error::new(
                        "incompatible_geometry",
                        format!("Target changed case-owned field: {section}.{name}"),
                    ));
                }
            }
        }
        difference(
            &mut result,
            "dimensions",
            &left["dimensions"],
            &right["dimensions"],
            false,
            &context,
        );
    }
    Ok(result)
}

pub fn compare(
    baseline_path: &Path,
    candidate_path: &Path,
    out: &Path,
    policy_path: Option<&Path>,
    gate: bool,
) -> Result<(Value, i32)> {
    let left_root = baseline_path.canonicalize()?;
    let right_root = candidate_path.canonicalize()?;
    let (baseline, suite) = load_run(&left_root)?;
    let (candidate, other_suite) = load_run(&right_root)?;
    if common::digest(&suite)? != common::digest(&other_suite)?
        || baseline["selection"] != candidate["selection"]
    {
        return Err(Error::new(
            "incompatible_runs",
            "Runs must have identical suite snapshots and selections",
        ));
    }
    let policy = policy_path.map(documents::load_policy).transpose()?;
    if let Some(policy) = &policy {
        documents::validate_policy_selectors(policy, &suite)?;
        for rule in array(policy, "rules")? {
            let case = array(&suite, "cases")?
                .iter()
                .find(|case| case["id"] == rule["selector"]["case_id"])
                .ok_or_else(|| Error::new("unresolved_identifier", "Unknown policy case"))?;
            if !metrics::metric_supported_with_protocol(
                text(rule, "metric")?,
                case,
                text(&rule["selector"], "region_id")?,
                &suite["protocol"],
            ) {
                return Err(Error::new(
                    "unresolved_metric",
                    format!("Metric is unknown or unselected: {}", text(rule, "metric")?),
                )
                .at(text(rule, "id")?));
            }
        }
    }
    let differences = condition_differences(&baseline, &candidate, &left_root, &right_root)?;
    let root = runner::absolute(out)?;
    common::create_destination(&root)?;
    let mut evidence = Evidence::new(&root);
    let defs: BTreeMap<_, _> = array(&suite, "cases")?
        .iter()
        .map(|c| (c["id"].as_str().unwrap_or(""), c))
        .collect();
    let left_rows: BTreeMap<_, _> = array(&baseline, "cases")?
        .iter()
        .map(|r| {
            (
                (
                    r["case_id"].as_str().unwrap_or(""),
                    r["seed"].as_u64().unwrap_or(0),
                ),
                r,
            )
        })
        .collect();
    let right_rows: BTreeMap<_, _> = array(&candidate, "cases")?
        .iter()
        .map(|r| {
            (
                (
                    r["case_id"].as_str().unwrap_or(""),
                    r["seed"].as_u64().unwrap_or(0),
                ),
                r,
            )
        })
        .collect();
    let mut cases = Vec::new();
    let mut findings = Vec::new();
    for ((case_id, seed), left) in left_rows {
        let right = right_rows
            .get(&(case_id, seed))
            .ok_or_else(|| Error::new("incompatible_runs", "Candidate case missing"))?;
        let definition = defs
            .get(case_id)
            .ok_or_else(|| Error::new("run_selection", "Unknown case"))?;
        let folder = root
            .join("cases")
            .join(case_id)
            .join(format!("seed-{seed}"));
        fs::create_dir_all(&folder)?;
        let mut record = json!({"case_id":case_id,"seed":seed,"status":"failed","measurements":[],"regions":definition["regions"],"review":{}});
        if left["status"] != "completed" || right["status"] != "completed" {
            record["reason"] = json!("failed_render");
            let path = folder.join("render-failure.json");
            common::dump(&path, &json!({"baseline":left,"candidate":right}))?;
            let failure = evidence.add(&path, "render_failure", "application/json", json!({}))?;
            for (role, row, run_root) in [
                ("baseline", left, &left_root),
                ("candidate", *right, &right_root),
            ] {
                for stream in ["stdout", "stderr"] {
                    if let Some(log) = row.get(stream) {
                        evidence.copy(
                            &documents::safe_artifact_path(run_root, text(log, "path")?)?,
                            &format!("cases/{case_id}/seed-{seed}/{role}.{stream}"),
                            "renderer_log",
                            json!({"media_type":"text/plain"}),
                        )?;
                    }
                }
            }
            findings.push(json!({"id":common::digest(&json!([case_id,seed,"failed_render"]))?,"case_id":case_id,"region_id":"full_image","metric":"execution","decision":"failed","evidence":[failure["path"]]}));
            cases.push(record);
            continue;
        }
        eprintln!("Comparing {case_id} seed {seed}");
        let images = [raw_image(&left_root, left)?, raw_image(&right_root, right)?];
        for (i, role, run_root, row) in [
            (0, "baseline", &left_root, left),
            (1, "candidate", &right_root, *right),
        ] {
            match review_image(
                &images[i],
                &folder.join(format!("{role}.png")),
                &mut evidence,
                role,
                case_id,
                seed,
            ) {
                Ok(review) => record["review"][role] = review,
                Err(error) => {
                    if record.get("review_unavailable").is_none() {
                        record["review_unavailable"] = json!({});
                    }
                    record["review_unavailable"][role] = json!(error.to_string());
                }
            }
            record[format!("{role}_master")] = evidence.copy(
                &documents::safe_artifact_path(run_root, text(&row["master"], "path")?)?,
                &format!("cases/{case_id}/seed-{seed}/{role}-master.tif"),
                &format!("{role}_master"),
                json!({"color":row["master"]["color"]}),
            )?;
            record[format!("{role}_resolved")] = evidence.copy(
                &documents::safe_artifact_path(run_root, text(&row["resolved"], "path")?)?,
                &format!("cases/{case_id}/seed-{seed}/{role}-resolved.json"),
                "resolved_configuration",
                json!({}),
            )?;
            record[format!("{role}_diagnostics")] = evidence.copy(
                &documents::safe_artifact_path(run_root, text(&row["diagnostics"], "path")?)?,
                &format!("cases/{case_id}/seed-{seed}/{role}-diagnostics.json"),
                "render_diagnostics",
                json!({}),
            )?;
            record[format!("{role}_execution")] = json!({"execution":row["execution"],"precision":row["precision"],"dimensions":row["dimensions"]});
        }
        let case = retained_case(definition, &baseline, &left_root, &root, &mut evidence)?;
        record["regions"] = case["regions"].clone();
        if let Some(regions) = record["regions"].as_array_mut() {
            for region in regions {
                if let Some(path) = region["geometry"]["path"].as_str() {
                    region["geometry"]["path"] = json!(
                        Path::new(path)
                            .strip_prefix(&root)
                            .map_err(|e| Error::new("unsafe_artifact_path", e.to_string()))?
                            .to_string_lossy()
                            .replace('\\', "/")
                    );
                }
            }
        }
        if case["input"]["kind"].as_str().unwrap_or("raster") == "raster" {
            let source = image_data::load_image(&case["input"])?;
            record["review"]["source"] = review_image(
                &source,
                &folder.join("source.png"),
                &mut evidence,
                "source",
                case_id,
                seed,
            )?;
        } else if let Some(source) = left.get("source_review") {
            record["review"]["source"] = evidence.copy(
                &documents::safe_artifact_path(&left_root, text(source, "path")?)?,
                &format!("cases/{case_id}/seed-{seed}/source.png"),
                "source",
                json!({"media_type":"image/png"}),
            )?;
        }
        let protocol = probe_protocol(&suite["protocol"], &case, seed);
        let measured = retain_metrics(
            metrics::compare_images(
                &images[0],
                &images[1],
                array(&case, "regions")?,
                &protocol,
                &folder,
            )?,
            &folder,
            &mut evidence,
            case_id,
            seed,
        )?;
        record.as_object_mut().unwrap().extend(
            measured
                .as_object()
                .ok_or_else(|| Error::new("computation_failure", "Measurements must be object"))?
                .clone(),
        );
        if case.get("reference").is_some() {
            let ref_folder = folder.join("reference");
            let reference = retain_metrics(
                references::compare_reference(
                    &images[0],
                    &images[1],
                    &case,
                    &protocol,
                    &ref_folder,
                    &root,
                )?,
                &ref_folder,
                &mut evidence,
                case_id,
                seed,
            )?;
            for key in ["measurements", "findings", "artifacts"] {
                record[key]
                    .as_array_mut()
                    .ok_or_else(|| {
                        Error::new("computation_failure", "Missing reference output arrays")
                    })?
                    .extend(reference[key].as_array().cloned().unwrap_or_default());
            }
            record["reference"] = reference["reference_review"].clone();
            if reference["execution_status"] == "failed" {
                record["execution_status"] = json!("failed");
            }
            if let Some(display) = reference["reference_review"]["display_artifact_id"].as_str()
                && let Some(artifact) = reference["artifacts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|a| a["path"] == display)
            {
                record["review"]["reference"] = artifact.clone();
            }
        }
        record["status"] = json!(if record["execution_status"] == "completed" {
            "completed"
        } else {
            "failed"
        });
        let path = folder.join("measurements.json");
        common::dump(
            &path,
            &json!({"measurements":record["measurements"],"findings":record["findings"]}),
        )?;
        let measurement_evidence =
            evidence.add(&path, "measurements", "application/json", json!({}))?;
        if let Some(rows) = record["findings"].as_array_mut() {
            for finding in rows {
                finding["id"] = json!(common::digest(&json!([case_id, seed, finding["id"]]))?);
                if finding["evidence"].as_array().is_none_or(|v| v.is_empty()) {
                    finding["evidence"] = json!([measurement_evidence["path"]]);
                }
                findings.push(finding.clone());
            }
        }
        for row in array(&record, "measurements")? {
            let metric = text(row, "metric")?;
            if row["availability"] == "available"
                && row["value"].as_f64().is_some_and(|v| v != 0.0)
                && (metric.starts_with("delta_")
                    || metric.starts_with("numeric.rmse")
                    || metric.starts_with("numeric.max_abs")
                    || metric.starts_with("tone.l_star.delta")
                    || metric.contains(".delta."))
            {
                let refs = if row["artifact_refs"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty())
                {
                    row["artifact_refs"].clone()
                } else {
                    json!([measurement_evidence["path"]])
                };
                findings.push(json!({"id":common::digest(&json!([case_id,seed,row["region_id"],metric]))?,"case_id":case_id,"region_id":row["region_id"],"metric":metric,"decision":"changed","evidence":refs}));
            }
        }
        cases.push(record);
    }
    let seed_measurements = metrics::aggregate_seed_measurements(&cases);
    let mut measurements: Vec<Value> = cases
        .iter()
        .flat_map(|case| case["measurements"].as_array().cloned().unwrap_or_default())
        .collect();
    measurements.extend(seed_measurements.clone());
    let mut rules = Vec::new();
    if let Some(policy) = &policy {
        for rule in array(policy, "rules")? {
            let decision = rule_result(rule, &measurements);
            let refs: BTreeSet<_> = measurements
                .iter()
                .filter(|row| {
                    row["case_id"] == rule["selector"]["case_id"]
                        && row["region_id"] == rule["selector"]["region_id"]
                        && row["metric"] == rule["metric"]
                })
                .flat_map(|row| row["artifact_refs"].as_array().into_iter().flatten())
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            let refs = if refs.is_empty() {
                vec!["policy.json".into()]
            } else {
                refs.into_iter().collect()
            };
            findings.push(json!({"id":common::digest(&json!(["policy",rule["id"]]))?,"case_id":rule["selector"]["case_id"],"region_id":rule["selector"]["region_id"],"metric":rule["metric"],"rule_id":rule["id"],"decision":decision["status"],"evidence":refs}));
            rules.push(decision);
        }
    }
    let completed = cases.iter().all(|case| case["status"] == "completed");
    let evaluation = match &policy {
        None => "not_evaluated",
        Some(_) if rules.iter().any(|r| r["status"] == "failed") => "failed",
        Some(policy)
            if !completed
                || policy["calibration"]["status"] != "calibrated"
                || rules
                    .iter()
                    .any(|r| r["required"] == true && r["status"] == "unavailable") =>
        {
            "insufficient_evidence"
        }
        Some(_)
            if differences.iter().any(|d| d["declared"] != true)
                || baseline["selection"]["full_suite"] != true =>
        {
            "review_required"
        }
        Some(_) => "passed",
    };
    let completed_cases: BTreeSet<_> = cases
        .iter()
        .filter(|c| c["status"] == "completed")
        .filter_map(|c| c["case_id"].as_str())
        .collect();
    let mut manifest = json!({"schema_version":1,"execution_status":if completed{"completed"}else{"failed"},"evaluation_status":evaluation,"baseline":{"id":baseline["id"],"manifest_sha256":common::sha256_file(&left_root.join("run.json"))?},"candidate":{"id":candidate["id"],"manifest_sha256":common::sha256_file(&right_root.join("run.json"))?},"protocol":suite["protocol"],"protocol_digest":documents::protocol_digest(&suite)?,"selection":baseline["selection"],"condition_differences":differences,"cases":cases,"seed_measurements":seed_measurements,"findings":findings,"rules":rules,"artifacts":evidence.values(),"summary":{"selected_cases":baseline["selection"]["case_ids"].as_array().map_or(0,Vec::len),"completed_cases":completed_cases.len(),"completed_results":cases.iter().filter(|c|c["status"]=="completed").count(),"failed_results":cases.iter().filter(|c|c["status"]!="completed").count(),"required_rules":rules.iter().filter(|r|r["required"]==true).count(),"unavailable_rules":rules.iter().filter(|r|r["status"]=="unavailable").count(),"protected_failures":rules.iter().filter(|r|r["status"]=="failed").collect::<Vec<_>>()}});
    if let Some(policy) = policy {
        let policy_path =
            policy_path.ok_or_else(|| Error::new("missing_policy", "Missing policy file"))?;
        let mut retained = policy.clone();
        for (i, item) in retained["calibration"]["evidence"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let source = documents::resolve_path(text(item, "path")?, policy_path)?;
            let ext = source
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| format!(".{s}"))
                .unwrap_or_default();
            let record = evidence.copy(
                &source,
                &format!("calibration/{i:04}{ext}"),
                "calibration_evidence",
                json!({}),
            )?;
            item["path"] = record["path"].clone();
        }
        common::dump(&root.join("policy.json"), &retained)?;
        manifest["policy"] = evidence.add(
            &root.join("policy.json"),
            "policy",
            "application/json",
            json!({"digest":common::digest(&retained)?}),
        )?;
        manifest["artifacts"] = json!(evidence.values());
    }
    runner::finalize(&root, "comparison.json", &mut manifest)?;
    let mut result = json!({"schema_version":1,"command":"compare","execution_status":manifest["execution_status"],"evaluation_status":evaluation,"artifact":root.join("comparison.json"),"comparison_id":manifest["id"]});
    if !completed {
        result["errors"] = json!([{"code":"measurement_failure","boundary":"comparison","message":"One or more selected results failed; inspect comparison cases and findings"}]);
    }
    Ok((
        result,
        i32::from(!completed || (gate && evaluation != "passed")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_missing_seed_cannot_mask_observed_violation() {
        let rule = json!({"id":"protected","selector":{"case_id":"chart","region_id":"skin"},"metric":"delta_e00.p95","max":2.0,"required":true});
        let values = vec![
            json!({"case_id":"chart","region_id":"skin","metric":"delta_e00.p95","availability":"available","value":3.0}),
            json!({"case_id":"chart","region_id":"skin","metric":"delta_e00.p95","availability":"unavailable","reason":"failed_render"}),
        ];
        assert_eq!(rule_result(&rule, &values)["status"], "failed");
    }
    #[test]
    fn policy_limits_are_inclusive_without_migration_tolerance() {
        let rule = json!({"id":"protected","selector":{"case_id":"chart","region_id":"skin"},"metric":"delta_e00.p95","max":2.0,"required":true});
        for (value, status) in [
            (2.0, "passed"),
            (f64::from_bits(2.0f64.to_bits() + 1), "failed"),
        ] {
            let rows = vec![
                json!({"case_id":"chart","region_id":"skin","metric":"delta_e00.p95","availability":"available","value":value}),
            ];
            assert_eq!(rule_result(&rule, &rows)["status"], status);
        }
    }
}
