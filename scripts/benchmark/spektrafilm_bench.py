#!/usr/bin/env python3
"""Immutable renderer evidence, appearance comparisons, and offline review."""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

from documents import (BenchmarkError, DocumentError, canonical_json, document_digest,
                       load_json_document, load_policy, load_suite, load_target,
                       protocol_digest, resolve_path, safe_artifact_path, sha256_file,
                       validate_generated_document, validate_policy_selectors,
                       validate_target_cases)
from schemas import schema_for

VERSION = "0.1.0"
ENV_KEYS = ("RAYON_NUM_THREADS", "OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS",
            "LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH", "VK_ICD_FILENAMES",
            "VK_DRIVER_FILES", "WGPU_BACKEND", "XDG_RUNTIME_DIR")


def fail(code, message, **fields):
    raise BenchmarkError(code, message, fields)


def dump(path, value):
    path.write_bytes(canonical_json(value) + b"\n")


def emit(value):
    print(json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False))


def finalized(root, name, value):
    value["id"] = document_digest(value)
    validate_generated_document("run" if name == "run.json" else "comparison", value, root)
    temporary = root / (name + ".tmp")
    dump(temporary, value)
    temporary.replace(root / name)
    return value


class Evidence:
    def __init__(self, root):
        self.root = root
        self.records = {}

    def add(self, path, role, media_type=None, **metadata):
        relative = path.relative_to(self.root).as_posix()
        safe_artifact_path(self.root, relative)
        record = {"path": relative, "sha256": sha256_file(path), "role": role,
                  "media_type": media_type or ("application/json" if path.suffix == ".json" else "application/octet-stream"), **metadata}
        previous = self.records.get(relative)
        if previous and previous["sha256"] != record["sha256"]:
            fail("artifact_changed", f"artifact changed: {relative}")
        self.records[relative] = record
        return record

    def copy(self, source, relative, role, **metadata):
        destination = self.root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        return self.add(destination, role, **metadata)

    def values(self):
        return [self.records[key] for key in sorted(self.records)]


def renderer_environment(target, case_dir):
    inherited = {key: os.environ[key] for key in ENV_KEYS if key in os.environ}
    inherited.update({key: str(value) for key, value in target.get("environment", {}).items()})
    environment = {"PATH": os.environ.get("PATH", os.defpath), "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", **inherited}
    environment["SPEKTRAFILM_CONFIG_DIR"] = str(case_dir / "config")
    environment["RUST_LOG"] = "off"
    return environment, inherited


def tree_inventory(root):
    records = []
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            fail("data_symlink", f"data dependencies must be regular files: {path}")
        if path.is_file():
            records.append({"path": path.relative_to(root).as_posix(), "sha256": sha256_file(path)})
    return {"sha256": document_digest(records), "files": records}


def file_dependencies(suite, suite_path, target, target_path):
    paths = {resolve_path(target["renderer"]["path"], target_path): "renderer"}
    for sample in suite["protocol"].get("probe", {}).get("tone_response", []):
        if isinstance(sample["geometry"], dict):
            paths[resolve_path(sample["geometry"]["path"], suite_path)] = "mask"
    for case in suite["cases"]:
        paths[resolve_path(case["input"]["path"], suite_path)] = "input"
        config = case["configuration"]
        for key in ("presets", "set_files"):
            for name in config.get(key, []):
                paths[resolve_path(name, suite_path)] = key
        if config.get("params"):
            paths[resolve_path(config["params"], suite_path)] = "params"
        for region in case["regions"]:
            if isinstance(region["geometry"], dict):
                paths[resolve_path(region["geometry"]["path"], suite_path)] = "mask"
        for sample in case.get("probe", {}).get("tone_response", []):
            if isinstance(sample["geometry"], dict):
                paths[resolve_path(sample["geometry"]["path"], suite_path)] = "mask"
        reference = case.get("reference")
        if reference:
            paths[resolve_path(reference["path"], suite_path)] = "reference"
            for key in ("transform", "aligned_derivative", "correspondence_review", "common_area_mask"):
                if key in reference:
                    paths[resolve_path(reference[key]["path"], suite_path)] = key
            if "icc" in reference["color"]:
                paths[resolve_path(reference["color"]["icc"]["path"], suite_path)] = "icc"
            for region in reference.get("regions", []):
                if isinstance(region["geometry"], dict):
                    paths[resolve_path(region["geometry"]["path"], suite_path)] = "mask"
            if "correspondence_review" in reference:
                review = load_json_document(resolve_path(reference["correspondence_review"]["path"], suite_path))
                for excluded in review.get("excluded_regions", []):
                    if isinstance(excluded["geometry"], dict):
                        paths[resolve_path(excluded["geometry"]["path"], suite_path)] = "mask"
    for changes in [target["changes"], *target.get("case_changes", {}).values()]:
        for name in changes.get("set_files", []):
            paths[resolve_path(name, target_path)] = "target_set"
    return [{"path": str(path), "role": paths[path], "sha256": sha256_file(path)} for path in sorted(paths)]


def render_command(case, suite_path, target, target_path, folder):
    config = case["configuration"]
    command = [str(resolve_path(target["renderer"]["path"], target_path)), "process",
               str(resolve_path(case["input"]["path"], suite_path)), "--output", str(folder / "master.tif"),
               "--backend", target["backend"], "--data-dir", str(resolve_path(target["data_dir"], target_path)),
               "--format", "tiff", "--bit-depth", "32", "--compression", "zip"]
    for field, option in (("film", "--film"), ("paper", "--paper"), ("route", "--route"), ("scan_output", "--scan-output")):
        if config.get(field):
            command += [option, config[field]]
    for value in config.get("presets", []):
        command += ["--preset", str(resolve_path(value, suite_path))]
    if config.get("params"):
        command += ["--params", str(resolve_path(config["params"], suite_path))]
    for changes, document in [(config, suite_path), (target["changes"], target_path),
                              (target.get("case_changes", {}).get(case["id"], {}), target_path)]:
        for value in changes.get("set_files", []):
            command += ["--set", str(resolve_path(value, document))]
        for value in changes.get("set", []):
            command += ["--set", value]
    if case["input"].get("kind", "raster") == "raster":
        command += ["--input-color-space", case["input"]["color_space"], "--input-cctf-decoding",
                    str(case["input"]["cctf_decoding"]).lower()]
    else:
        for key, option in (("white_balance", "--raw-white-balance"), ("temperature", "--raw-temperature"), ("tint", "--raw-tint")):
            if key in config.get("raw", {}):
                command += [option, str(config["raw"][key])]
        if config.get("raw", {}).get("lens_correction"):
            command += ["--lens-correction"]
    return command


def invoke(command, environment, folder, name, evidence):
    try:
        result = subprocess.run(command, env=environment, capture_output=True, text=True, check=False)
    except OSError as error:
        fail("renderer_start", str(error), boundary="renderer")
    logs = {}
    for stream in ("stdout", "stderr"):
        path = folder / f"{name}.{stream}"
        path.write_text(getattr(result, stream), encoding="utf-8")
        logs[stream] = evidence.add(path, "renderer_log", "text/plain")
    return result, logs


def run_case(case, seed, suite_path, target, target_path, root, evidence):
    folder = root / "cases" / case["id"] / f"seed-{seed}"
    folder.mkdir(parents=True)
    environment, conditions = renderer_environment(target, folder)
    command = render_command(case, suite_path, target, target_path, folder) + ["--seed", str(seed)]
    row = {"case_id": case["id"], "seed": seed, "status": "failed", "environment": conditions, "command": command}
    source = resolve_path(case["input"]["path"], suite_path)
    row["input"] = evidence.copy(source, f"inputs/{case['id']}{source.suffix}", "source_input",
                                 declaration={key: value for key, value in case["input"].items() if key not in ("path", "sha256")})
    try:
        result, logs = invoke(command + ["--dry-run"], environment, folder, "dry-run", evidence)
        row.update(logs)
        if result.returncode:
            fail("renderer_configuration", result.stderr[-2000:], case_id=case["id"])
        resolved = json.loads(result.stdout)
        data_dir = resolve_path(target["data_dir"], target_path)
        for field in ("film_profile", "print_profile"):
            name = resolved[field]
            if not isinstance(name, str) or not name or "\0" in name:
                fail("profile_dependency", f"renderer resolved an invalid {field}", case_id=case["id"])
            profile_path = (data_dir / "profiles" / f"{name}.json").resolve()
            if not profile_path.is_relative_to(data_dir) or not profile_path.is_file():
                fail("profile_dependency", f"resolved {field} must select a file inside target data_dir: {name}", case_id=case["id"])
        if target["changes"]["set"] or target["changes"]["set_files"] or target.get("case_changes", {}).get(case["id"]):
            case_target = {**target, "changes": {"set": [], "set_files": []}, "case_changes": {}}
            base_command = render_command(case, suite_path, case_target, target_path, folder) + ["--seed", str(seed), "--dry-run"]
            base_result, _ = invoke(base_command, environment, folder, "case-configuration", evidence)
            if base_result.returncode:
                fail("renderer_configuration", base_result.stderr[-2000:], case_id=case["id"])
            base_parameters = json.loads(base_result.stdout)["parameters"]
            for section, names in (
                ("io", ("input_color_space", "input_cctf_decoding", "crop", "crop_center", "crop_size", "upscale_factor")),
                ("settings", ("preview_mode", "preview_max_size")),
            ):
                for name in names:
                    if resolved["parameters"][section].get(name) != base_parameters[section].get(name):
                        fail("target_geometry", f"target must not change case-owned input field: {section}.{name}", case_id=case["id"])
        path = folder / "resolved.json"
        dump(path, resolved)
        row["resolved"] = evidence.add(path, "resolved_configuration", "application/json")
        if case["input"].get("kind", "raster") == "raw":
            source_command = command[:]
            if "--route" in source_command:
                source_command[source_command.index("--route") + 1] = "input"
            else:
                source_command += ["--route", "input"]
            source_command[source_command.index("--output") + 1] = str(folder / "source-preview.png")
            source_command[source_command.index("--format") + 1] = "png"
            source_command[source_command.index("--bit-depth") + 1] = "8"
            compression_index = source_command.index("--compression")
            del source_command[compression_index:compression_index + 2]
            source_command += ["--saving-color-space", "sRGB", "--saving-cctf-encoding", "true"]
            source_result, source_logs = invoke(source_command, environment, folder, "source-preview", evidence)
            if source_result.returncode:
                fail("source_review", source_result.stderr[-2000:], case_id=case["id"])
            row["source_review"] = evidence.add(folder / "source-preview.png", "source", "image/png")
        render = command + ["--raw-out", str(folder / "master.f64"), "--diagnostics", str(folder / "diagnostics.json")]
        result, logs = invoke(render, environment, folder, "render", evidence)
        row.update(logs)
        if result.returncode:
            fail("renderer_failed", result.stderr[-2000:], case_id=case["id"], returncode=result.returncode)
        diagnostics = load_json_document(folder / "diagnostics.json")
        configuration = diagnostics["configuration"]
        execution = diagnostics["execution"]
        width, height = configuration["published_dimensions"]
        params = resolved["parameters"]["io"]
        color = {"color_space": params["output_color_space"],
                 "transfer": "encoded" if params["output_cctf_encoding"] else "linear"}
        if color["color_space"].lower() == "srgb" and params["output_cctf_encoding"]:
            color["transfer"] = "srgb"
        layout = {"width": width, "height": height, "channels": 3, "dtype": "f64", "channel_order": "RGB", "endianness": sys.byteorder}
        if (folder / "master.f64").stat().st_size != width * height * 3 * 8:
            fail("raw_layout", "renderer raw byte count disagrees with published dimensions")
        metadata = {**layout, **color}
        dump(folder / "master.f64.json", metadata)
        row["raw_metadata"] = evidence.add(folder / "master.f64.json", "raw_metadata", "application/json")
        row["raw"] = evidence.add(folder / "master.f64", "native_master", layout=layout, color=color)
        row["master"] = evidence.add(folder / "master.tif", "floating_master", "image/tiff", dimensions=[width, height], color=color)
        row["diagnostics"] = evidence.add(folder / "diagnostics.json", "render_diagnostics", "application/json")
        row["execution"] = execution
        row["dimensions"] = [width, height]
        row["precision"] = {key: configuration.get(key) for key in ("host_precision", "gpu_precision", "spectral_preparation_precision")}
        row["status"] = "completed"
    except (DocumentError, ValueError, KeyError, TypeError, OSError) as error:
        row["error"] = error.as_dict() if isinstance(error, DocumentError) else {"code": "render_evidence", "message": str(error)}
    return row


def run(args):
    suite_path, target_path = Path(args.suite).resolve(), Path(args.target).resolve()
    suite, target = load_suite(suite_path), load_target(target_path)
    validate_target_cases(target, suite)
    all_ids = [case["id"] for case in suite["cases"]]
    requested = args.case or all_ids
    if len(set(requested)) != len(requested) or set(requested) - set(all_ids):
        fail("case_selection", "case selection must contain distinct known case identities")
    selected = [identity for identity in all_ids if identity in requested]
    dependencies = file_dependencies(suite, suite_path, target, target_path)
    data_root = resolve_path(target["data_dir"], target_path)
    inventory = tree_inventory(data_root)
    root = Path(args.out).absolute()
    if root.exists() or root.is_symlink():
        fail("destination_exists", str(root))
    root.mkdir(parents=True)
    evidence = Evidence(root)
    dump(root / "suite.json", suite)
    dump(root / "target.json", target)
    suite_artifact = evidence.add(root / "suite.json", "suite", "application/json", digest=document_digest(suite), id=suite["id"], source_path=str(suite_path))
    target_artifact = evidence.add(root / "target.json", "target", "application/json", digest=document_digest(target), id=target["id"], source_path=str(target_path))
    retained_dependencies = []
    for index, dependency in enumerate(dependencies):
        source = Path(dependency["path"])
        if dependency["role"] == "renderer":
            retained_dependencies.append(dependency)
        else:
            record = evidence.copy(source, f"dependencies/{index:04d}{source.suffix}", dependency["role"])
            retained_dependencies.append({**record, "source_path": dependency["path"]})
    dump(root / "data-inventory.json", inventory)
    retained_dependencies.append(evidence.add(root / "data-inventory.json", "data_inventory", "application/json"))
    rows = [run_case(case, seed, suite_path, target, target_path, root, evidence)
            for case in suite["cases"] if case["id"] in selected for seed in case["seeds"]]
    modified = [dependency["path"] for dependency in dependencies if sha256_file(dependency["path"]) != dependency["sha256"]]
    if tree_inventory(data_root) != inventory:
        modified.append(str(data_root))
    if modified:
        for row in rows:
            row.update(status="failed", error={"code": "dependencies_changed", "message": "dependencies changed during rendering", "paths": modified})
    result = {"schema_version": 1, "execution_status": "completed" if all(row["status"] == "completed" for row in rows) else "failed",
              "suite": suite_artifact, "target": target_artifact, "protocol": suite["protocol"],
              "selection": {"case_ids": selected, "full_suite": selected == all_ids},
              "dependencies": retained_dependencies, "cases": rows, "artifacts": evidence.values(),
              "environment": renderer_environment(target, root)[1]}
    finalized(root, "run.json", result)
    return {"schema_version": 1, "command": "run", "execution_status": result["execution_status"], "artifact": str(root / "run.json"), "run_id": result["id"]}, 0 if result["execution_status"] == "completed" else 1


def load_run(root):
    value = validate_generated_document("run", load_json_document(root / "run.json"), root)
    suite = load_json_document(root / value["suite"]["path"])
    if document_digest(suite) != value["suite"]["digest"]:
        fail("suite_identity", "suite snapshot digest mismatch")
    expected = {(case["id"], seed) for case in suite["cases"] if case["id"] in value["selection"]["case_ids"] for seed in case["seeds"]}
    actual = {(row["case_id"], row["seed"]) for row in value["cases"]}
    if actual != expected:
        fail("run_selection", "run cases or seeds do not match suite selection")
    return value, suite


def raw_image(root, row):
    from image_data import load_image
    raw = row["raw"]
    return load_image({"path": str(safe_artifact_path(root, raw["path"])), **raw["layout"], **raw["color"]})


def review_image(image, destination, evidence, role, **metadata):
    import numpy as np
    from PIL import Image
    from image_data import display_rgb
    import png16
    samples = display_rgb(image)
    if not np.all(np.isfinite(samples)):
        fail("nonfinite_review", "review image has nonfinite display samples")
    overview = np.clip(samples, 0, 1)
    width, height = image.dimensions
    scale = min(1., 1600 / max(width, height))
    size = (max(1, round(width * scale)), max(1, round(height * scale)))
    channels = [np.asarray(Image.fromarray(overview[..., channel].astype(np.float32), mode="F").resize(size, Image.Resampling.LANCZOS), dtype=np.float64) for channel in range(3)]
    codes = np.clip(np.stack(channels, axis=-1), 0, 1)
    fit_path = destination.with_name(destination.stem + "-overview.png")
    png16.save(fit_path, np.rint(codes * 65535).astype(np.uint16))
    fit = evidence.add(fit_path, role + "_overview", "image/png", dimensions=list(size),
                       source_dimensions=image.dimensions, resampling="encoded sRGB float Lanczos to longest edge <=1600 before quantization",
                       quantization="16-bit rounded; no dither", bit_depth=16,
                       purpose="fit display only; never a measurement or native detail source", **metadata)
    png16.save(destination, np.rint(np.clip(samples, 0, 1) * 65535).astype(np.uint16))
    return evidence.add(destination, role, "image/png", dimensions=image.dimensions, bit_depth=16,
                        quantization="16-bit rounded; no dither", color={"color_space": "sRGB", "transfer": "srgb"}, fit_preview=fit["path"], **metadata)


def retain_metrics(measured, folder, evidence, case_id, seed):
    refs = {}
    for artifact in measured.get("artifacts", []):
        source = safe_artifact_path(folder, artifact["path"])
        metadata = {key: value for key, value in artifact.items() if key not in ("id", "path", "sha256", "media_type", "role", "availability", "case_id", "seed")}
        identity = f"{case_id}/seed-{seed}/{artifact.get('id', artifact['path'])}"
        record = evidence.add(source, artifact["role"], artifact.get("media_type"), **metadata, id=identity, case_id=case_id, seed=seed)
        refs[artifact.get("id", artifact["path"])] = record["path"]
    for measurement in measured.get("measurements", []):
        measurement["artifact_refs"] = [refs.get(ref, ref) for ref in measurement.get("artifact_refs", [])]
        measurement["case_id"] = case_id
    for finding in measured.get("findings", []):
        finding["evidence"] = [refs.get(ref, ref) for ref in finding.get("evidence", [])]
    if measured.get("reference_review"):
        for key, value in list(measured["reference_review"].items()):
            if key.endswith("_artifact_id"):
                measured["reference_review"][key] = refs.get(value, value)
            elif key == "artifact_refs":
                measured["reference_review"][key] = [refs.get(ref, ref) for ref in value]
    measured["artifacts"] = [evidence.records[path] for path in refs.values()]
    return measured


def retained_case(case, run, run_root, comparison_root, evidence):
    """Resolve only immutable retained dependencies, never original source files."""
    clone = json.loads(json.dumps(case))
    protocol_probe = run["protocol"].get("probe", {})
    if "tone_response" in protocol_probe and "tone_response" not in clone.get("probe", {}):
        clone.setdefault("probe", {})["tone_response"] = json.loads(json.dumps(protocol_probe["tone_response"]))
    authored = Path(run["suite"]["source_path"])
    dependencies = {row["source_path"]: row for row in run["dependencies"] if "source_path" in row}

    def visit(item):
        if isinstance(item, dict):
            if "path" in item:
                original = str(resolve_path(item["path"], authored))
                dependency = dependencies.get(original)
                if dependency is None:
                    fail("missing_dependency", f"no retained source for {original}")
                destination = f"retained/{case['id']}/{dependency['sha256']}{Path(original).suffix}"
                target = comparison_root / destination
                if not target.exists():
                    evidence.copy(safe_artifact_path(run_root, dependency["path"]), destination, dependency["role"])
                if key_context[0] == "reference":
                    record = evidence.records[destination]
                    record.update(redistribution=clone["reference"]["provenance"]["redistribution"], provenance=clone["reference"]["provenance"])
                item["path"] = str(target)
            for child in item.values():
                visit(child)
        elif isinstance(item, list):
            for child in item:
                visit(child)

    key_context = [None]
    for key in ("input", "reference", "regions", "probe_response", "probe"):
        if key in clone:
            key_context[0] = key
            visit(clone[key])
    if clone.get("reference", {}).get("correspondence_review"):
        retained = clone["reference"]["correspondence_review"]
        review = load_json_document(retained["path"])
        key_context[0] = "reference"
        visit(review)
        rewritten = comparison_root / "retained" / case["id"] / "resolved-correspondence-review.json"
        dump(rewritten, review)
        record = evidence.add(rewritten, "resolved_correspondence_review", "application/json")
        retained.update(path=str(rewritten), sha256=record["sha256"])
    return clone


def probe_protocol(suite_protocol, case, seed):
    result = {**suite_protocol, "case_id": case["id"], "seed_population": [seed], "measurements": case["measurements"]}
    probes = case.get("probe", {})
    for key in ("halation", "edge"):
        if key in probes:
            result[key] = probes[key]
    if "tone_response" in probes:
        result["probe"] = {"tone_response": probes["tone_response"]}
    return result


def rule_result(rule, measurements):
    selector = rule["selector"]
    rows = [row for row in measurements if row["case_id"] == selector["case_id"] and row["region_id"] == selector["region_id"] and row["metric"] == rule["metric"]]
    values = [row["value"] for row in rows if row["availability"] == "available"]
    def satisfied(value):
        return (("min" not in rule or value >= rule["min"]) and ("max" not in rule or value <= rule["max"]) and
                (rule.get("direction") != "increase" or value > 0) and (rule.get("direction") != "decrease" or value < 0) and
                (rule.get("direction") != "unchanged" or value == 0))
    if not any(not satisfied(value) for value in values) and (not rows or any(row["availability"] != "available" for row in rows)):
        return {"rule_id": rule["id"], "status": "unavailable", "required": rule["required"], "reason": "missing_or_unavailable_measurement", "values": values}
    return {"rule_id": rule["id"], "status": "passed" if all(satisfied(value) for value in values) else "failed",
            "required": rule["required"], "values": values, "population": "all selected seed measurements"}


def condition_differences(baseline, candidate, left_root, right_root):
    targets = [load_json_document(root / run["target"]["path"]) for run, root in ((baseline, left_root), (candidate, right_root))]
    result = []

    def difference(field, left, right, declared=False, **context):
        if left != right:
            result.append({"field": field, "baseline": left, "candidate": right, "declared": declared, **context})

    def dependencies(run):
        return {row.get("source_path", row["path"]): {"role": row["role"], "sha256": row["sha256"]} for row in run["dependencies"]}

    explicit_renderer = targets[0]["renderer"]["sha256"] != targets[1]["renderer"]["sha256"]
    explicit_data = targets[0]["data_dir"] != targets[1]["data_dir"]
    explicit_changes = any(targets[0].get(key) != targets[1].get(key) for key in ("changes", "case_changes"))
    difference("renderer", targets[0]["renderer"]["sha256"], targets[1]["renderer"]["sha256"], explicit_renderer)
    inventories = [load_json_document(root / next(row["path"] for row in run["dependencies"] if row["role"] == "data_inventory"))["sha256"] for run, root in ((baseline, left_root), (candidate, right_root))]
    difference("data_inventory", *inventories, explicit_data)
    deps = [dependencies(run) for run in (baseline, candidate)]
    for source in sorted(set(deps[0]) | set(deps[1])):
        left, right = deps[0].get(source), deps[1].get(source)
        if (left or right)["role"] not in ("renderer", "data_inventory"):
            difference("dependency", left, right, explicit_changes and (left is None or right is None), source_path=source)
    difference("environment", baseline.get("environment"), candidate.get("environment"), targets[0].get("environment") != targets[1].get("environment"))
    left_rows = {(row["case_id"], row["seed"]): row for row in baseline["cases"]}
    right_rows = {(row["case_id"], row["seed"]): row for row in candidate["cases"]}
    for key in sorted(left_rows):
        rows = (left_rows[key], right_rows[key])
        if any(row["status"] != "completed" for row in rows):
            continue
        context = {"case_id": key[0], "seed": key[1]}
        for side, row, target in zip(("baseline", "candidate"), rows, targets):
            actual = row.get("execution", {}).get("backend_selected")
            expected_backend = "wgpu" if target["backend"] == "gpu" else "cpu"
            if actual != expected_backend or (target["backend"] == "gpu" and row.get("execution", {}).get("path") == "cpu"):
                result.append({"field": "backend_route", "side": side, "requested": target["backend"], "observed": actual, "declared": False, **context})
            if target["backend"] == "gpu" and (row.get("execution", {}).get("adapter") or {}).get("device_type") == "cpu":
                result.append({"field": "gpu_adapter", "side": side, "requested": "gpu", "observed": row["execution"]["adapter"], "declared": False, **context})
        difference("execution", rows[0].get("execution"), rows[1].get("execution"), targets[0]["backend"] != targets[1]["backend"], **context)
        difference("precision", rows[0].get("precision"), rows[1].get("precision"), explicit_renderer or targets[0]["backend"] != targets[1]["backend"], **context)
        parameters = [load_json_document(root / row["resolved"]["path"])["parameters"] for root, row in zip((left_root, right_root), rows)]
        difference("resolved_parameters", *parameters, explicit_changes or explicit_renderer or explicit_data, **context)
        for section, names in (
            ("io", ("input_color_space", "input_cctf_decoding", "crop", "crop_center", "crop_size", "upscale_factor")),
            ("settings", ("preview_mode", "preview_max_size")),
        ):
            for name in names:
                if parameters[0][section].get(name) != parameters[1][section].get(name):
                    fail("incompatible_geometry", f"target changed case-owned input field: {section}.{name}", **context)
        difference("dimensions", rows[0].get("dimensions"), rows[1].get("dimensions"), False, **context)
    return result


def compare(args):
    from metrics import compare_images
    left_root, right_root = Path(args.baseline).resolve(), Path(args.candidate).resolve()
    baseline, suite = load_run(left_root)
    candidate, other_suite = load_run(right_root)
    if document_digest(suite) != document_digest(other_suite) or baseline["selection"] != candidate["selection"]:
        fail("incompatible_runs", "runs must have identical suite snapshots and selections")
    policy = load_policy(Path(args.policy).resolve()) if args.policy else None
    if policy:
        validate_policy_selectors(policy, suite)
    root = Path(args.out).absolute()
    if root.exists() or root.is_symlink():
        fail("destination_exists", str(root))
    root.mkdir(parents=True)
    evidence = Evidence(root)
    case_defs = {case["id"]: case for case in suite["cases"]}
    left_rows = {(row["case_id"], row["seed"]): row for row in baseline["cases"]}
    right_rows = {(row["case_id"], row["seed"]): row for row in candidate["cases"]}
    cases, findings = [], []
    differences = condition_differences(baseline, candidate, left_root, right_root)
    for case_id, seed in sorted(left_rows):
        left, right = left_rows[(case_id, seed)], right_rows[(case_id, seed)]
        folder = root / "cases" / case_id / f"seed-{seed}"
        folder.mkdir(parents=True)
        record = {"case_id": case_id, "seed": seed, "status": "failed", "measurements": [], "regions": case_defs[case_id]["regions"], "review": {}}
        if left["status"] != "completed" or right["status"] != "completed":
            record["reason"] = "failed_render"
            failure_path = folder / "render-failure.json"
            dump(failure_path, {"baseline": left, "candidate": right})
            failure = evidence.add(failure_path, "render_failure", "application/json")
            for role, row, run_root in (("baseline", left, left_root), ("candidate", right, right_root)):
                for stream in ("stdout", "stderr"):
                    if stream in row:
                        evidence.copy(safe_artifact_path(run_root, row[stream]["path"]), f"cases/{case_id}/seed-{seed}/{role}.{stream}", "renderer_log", media_type="text/plain")
            findings.append({"id": document_digest([case_id, seed, "failed_render"]), "case_id": case_id, "region_id": "full_image", "metric": "execution", "decision": "failed", "evidence": [failure["path"]]})
            cases.append(record)
            continue
        images = (raw_image(left_root, left), raw_image(right_root, right))
        for role, image, run_root, row in zip(("baseline", "candidate"), images, (left_root, right_root), (left, right)):
            try:
                record["review"][role] = review_image(image, folder / f"{role}.png", evidence, role, case_id=case_id, seed=seed)
            except (DocumentError, ValueError) as error:
                record.setdefault("review_unavailable", {})[role] = str(error)
            record[role + "_master"] = evidence.copy(safe_artifact_path(run_root, row["master"]["path"]), f"cases/{case_id}/seed-{seed}/{role}-master.tif", role + "_master", color=row["master"].get("color"))
            record[role + "_resolved"] = evidence.copy(safe_artifact_path(run_root, row["resolved"]["path"]), f"cases/{case_id}/seed-{seed}/{role}-resolved.json", "resolved_configuration")
            record[role + "_diagnostics"] = evidence.copy(safe_artifact_path(run_root, row["diagnostics"]["path"]), f"cases/{case_id}/seed-{seed}/{role}-diagnostics.json", "render_diagnostics")
            record[role + "_execution"] = {"execution": row.get("execution"), "precision": row.get("precision"), "dimensions": row.get("dimensions")}
        case = retained_case(case_defs[case_id], baseline, left_root, root, evidence)
        record["regions"] = json.loads(json.dumps(case["regions"]))
        for region in record["regions"]:
            if isinstance(region["geometry"], dict):
                region["geometry"]["path"] = Path(region["geometry"]["path"]).relative_to(root).as_posix()
        from image_data import load_image
        source_decl = case["input"]
        if source_decl.get("kind", "raster") == "raster":
            source_image = load_image({"path": source_decl["path"], "color_space": source_decl["color_space"], "cctf_decoding": source_decl["cctf_decoding"]})
            record["review"]["source"] = review_image(source_image, folder / "source.png", evidence, "source", case_id=case_id, seed=seed)
        elif left.get("source_review"):
            record["review"]["source"] = evidence.copy(safe_artifact_path(left_root, left["source_review"]["path"]), f"cases/{case_id}/seed-{seed}/source.png", "source", media_type="image/png")
        protocol = probe_protocol(suite["protocol"], case, seed)
        measured = retain_metrics(compare_images(*images, case["regions"], protocol, folder), folder, evidence, case_id, seed)
        record.update(measured)
        if case.get("reference"):
            from references import compare_reference
            reference_folder = folder / "reference"
            reference = retain_metrics(compare_reference(*images, case, protocol, reference_folder, root), reference_folder, evidence, case_id, seed)
            record["measurements"].extend(reference["measurements"])
            record["findings"].extend(reference["findings"])
            record["artifacts"].extend(reference["artifacts"])
            record["reference"] = reference.get("reference_review")
            if reference["execution_status"] == "failed":
                record["execution_status"] = "failed"
            display_path = (reference.get("reference_review") or {}).get("display_artifact_id")
            for artifact in reference["artifacts"]:
                if artifact["path"] == display_path:
                    record["review"]["reference"] = artifact
        record["status"] = "completed" if record["execution_status"] == "completed" else "failed"
        error_path = folder / "measurements.json"
        dump(error_path, {"measurements": record["measurements"], "findings": record["findings"]})
        measurement_evidence = evidence.add(error_path, "measurements", "application/json")
        for finding in record["findings"]:
            finding["id"] = document_digest([case_id, seed, finding["id"]])
            finding["evidence"] = finding.get("evidence") or [measurement_evidence["path"]]
            findings.append(finding)
        for row in record["measurements"]:
            if row["availability"] == "available" and row["metric"].startswith(("delta_", "numeric.rmse", "numeric.max_abs", "tone.l_star.delta")) and row["value"] != 0:
                refs = row["artifact_refs"] or [measurement_evidence["path"]]
                findings.append({"id": document_digest([case_id, seed, row["region_id"], row["metric"]]), "case_id": case_id, "region_id": row["region_id"], "metric": row["metric"], "decision": "changed", "evidence": refs})
                continue
            if row["availability"] == "available" and ".delta." in row["metric"] and row["value"] != 0:
                findings.append({"id": document_digest([case_id, seed, row["region_id"], row["metric"]]), "case_id": case_id, "region_id": row["region_id"], "metric": row["metric"], "decision": "changed", "evidence": row["artifact_refs"] or [measurement_evidence["path"]]})
        cases.append(record)
    from metrics import aggregate_seed_measurements
    seed_measurements = aggregate_seed_measurements(cases)
    measurements = [row for case in cases for row in case["measurements"]] + seed_measurements
    rules = [rule_result(rule, measurements) for rule in policy["rules"]] if policy else []
    if policy:
        for rule, decision in zip(policy["rules"], rules):
            selected_rows = [row for row in measurements if row["case_id"] == rule["selector"]["case_id"] and row["region_id"] == rule["selector"]["region_id"] and row["metric"] == rule["metric"]]
            refs = list(dict.fromkeys(ref for row in selected_rows for ref in row["artifact_refs"])) or ["policy.json"]
            findings.append({"id": document_digest(["policy", rule["id"]]), **rule["selector"], "metric": rule["metric"], "rule_id": rule["id"], "decision": decision["status"], "evidence": refs})
    status = "completed" if all(case["status"] == "completed" for case in cases) else "failed"
    if not policy:
        evaluation = "not_evaluated"
    elif any(row["status"] == "failed" for row in rules):
        evaluation = "failed"
    elif status == "failed" or policy["calibration"]["status"] != "calibrated" or any(row["required"] and row["status"] == "unavailable" for row in rules):
        evaluation = "insufficient_evidence"
    elif any(not row["declared"] for row in differences) or not baseline["selection"]["full_suite"]:
        evaluation = "review_required"
    else:
        evaluation = "passed"
    result = {"schema_version": 1, "execution_status": status, "evaluation_status": evaluation,
              "baseline": {"id": baseline["id"], "manifest_sha256": sha256_file(left_root / "run.json")},
              "candidate": {"id": candidate["id"], "manifest_sha256": sha256_file(right_root / "run.json")},
              "protocol": suite["protocol"], "protocol_digest": protocol_digest(suite), "selection": baseline["selection"], "condition_differences": differences,
              "cases": cases, "seed_measurements": seed_measurements, "findings": findings, "rules": rules, "artifacts": evidence.values(),
              "summary": {"selected_cases": len(baseline["selection"]["case_ids"]), "completed_cases": len({case["case_id"] for case in cases if case["status"] == "completed"}), "completed_results": sum(case["status"] == "completed" for case in cases),
                          "failed_results": sum(case["status"] == "failed" for case in cases), "required_rules": sum(rule["required"] for rule in rules),
                          "unavailable_rules": sum(rule["status"] == "unavailable" for rule in rules), "protected_failures": [rule for rule in rules if rule["status"] == "failed"]}}
    if policy:
        retained_policy = json.loads(json.dumps(policy))
        for index, item in enumerate(retained_policy["calibration"]["evidence"]):
            record = evidence.copy(resolve_path(item["path"], Path(args.policy)), f"calibration/{index:04d}{Path(item['path']).suffix}", "calibration_evidence")
            item["path"] = record["path"]
        dump(root / "policy.json", retained_policy)
        result["policy"] = evidence.add(root / "policy.json", "policy", "application/json", digest=document_digest(retained_policy))
        result["artifacts"] = evidence.values()
    finalized(root, "comparison.json", result)
    code = int(status == "failed" or (args.gate and evaluation != "passed"))
    return {"schema_version": 1, "command": "compare", "execution_status": status, "evaluation_status": evaluation,
            "artifact": str(root / "comparison.json"), "comparison_id": result["id"]}, code


def parser():
    root = argparse.ArgumentParser(prog="spektrafilm-bench", description="Render immutable evidence, compare appearance, and build an offline review.")
    root.add_argument("--version", action="version", version=f"spektrafilm-bench {VERSION} schema-1")
    sub = root.add_subparsers(dest="command", required=True)
    for name in ("schema", "validate"):
        command = sub.add_parser(name, help="Discover a schema" if name == "schema" else "Validate a document and its dependencies")
        command.add_argument("--kind", required=True, choices=("suite", "target", "policy", "run", "comparison"), help="Document kind")
        if name == "validate":
            command.add_argument("--file", required=True, help="JSON document path")
    command = sub.add_parser("run", help="Render all selected cases using one target")
    for name in ("suite", "target", "out"):
        command.add_argument("--" + name, required=True, help={"suite": "Suite JSON", "target": "Renderer target JSON", "out": "New run directory"}[name])
    command.add_argument("--case", action="append", help="Exact case identity; repeat to select cases")
    command = sub.add_parser("compare", help="Compare retained runs without rendering")
    for name in ("baseline", "candidate", "out"):
        command.add_argument("--" + name, required=True, help="New comparison directory" if name == "out" else name.title() + " run directory")
    command.add_argument("--policy", help="Versioned acceptance policy JSON")
    command.add_argument("--gate", action="store_true", help="Return failure for any non-pass evaluation; requires --policy")
    command = sub.add_parser("report", help="Build a portable offline review from retained measurements")
    command.add_argument("--comparison", required=True, help="Comparison directory")
    command.add_argument("--out", required=True, help="New report directory")
    return root


def main(argv=None):
    argument_parser = parser()
    args = argument_parser.parse_args(argv)
    if args.command == "compare" and args.gate and not args.policy:
        argument_parser.error("--gate requires --policy")
    try:
        if args.command == "schema":
            emit(schema_for(args.kind))
            return 0
        if args.command == "validate":
            path = Path(args.file).resolve()
            if args.kind in ("run", "comparison"):
                value = validate_generated_document(args.kind, load_json_document(path), path.parent)
            else:
                value = {"suite": load_suite, "target": load_target, "policy": load_policy}[args.kind](path)
            result, code = {"schema_version": 1, "command": "validate", "execution_status": "completed", "kind": args.kind,
                            "digest": document_digest(value), "unchecked": ["rendering", "compute_device"] if args.kind in ("suite", "target") else []}, 0
            if args.kind == "suite":
                result["protocol_digest"] = protocol_digest(value)
        elif args.command == "run":
            result, code = run(args)
        elif args.command == "compare":
            result, code = compare(args)
        else:
            from report import build_report
            build_report(Path(args.comparison).resolve(), Path(args.out).absolute())
            result, code = {"schema_version": 1, "command": "report", "execution_status": "completed", "artifact": str(Path(args.out).absolute() / "index.html")}, 0
        emit(result)
        return code
    except (DocumentError, OSError, ValueError, KeyError, TypeError) as error:
        entry = error.as_dict() if isinstance(error, DocumentError) else {"code": "operation_failed", "message": str(error)}
        print(f"{entry['code']}: {entry['message']}", file=sys.stderr)
        emit({"schema_version": 1, "command": args.command, "execution_status": "failed", "errors": [entry]})
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
