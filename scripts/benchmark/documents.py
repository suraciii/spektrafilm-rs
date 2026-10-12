"""Strict authored documents and self-contained, hashed benchmark evidence."""
from __future__ import annotations

import hashlib
import json
import math
import os
import re
from pathlib import Path, PurePosixPath

from jsonschema import Draft202012Validator
from schemas import SCHEMAS

ID_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*\Z")
HASH_PATTERN = re.compile(r"[0-9a-f]{64}\Z")


class DocumentError(ValueError):
    def __init__(self, code, message, path=None):
        super().__init__(message)
        self.code, self.message, self.path = code, message, str(path) if path is not None else None

    def as_dict(self):
        return {"code": self.code, "message": self.message, **({"path": self.path} if self.path else {})}


class BenchmarkError(DocumentError):
    def __init__(self, code, message, fields=None):
        super().__init__(code, message)
        self.fields = dict(fields or {})

    def as_dict(self):
        return {**super().as_dict(), **self.fields}


def error(code, message, path=None):
    raise DocumentError(code, message, path)


def finite(value):
    if isinstance(value, float) and not math.isfinite(value):
        error("nonfinite_number", "JSON numbers must be finite")
    if isinstance(value, dict):
        for item in value.values():
            finite(item)
    elif isinstance(value, list):
        for item in value:
            finite(item)


def pairs(items):
    result = {}
    for key, value in items:
        if key in result:
            error("duplicate_key", f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_json_document(path):
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"), object_pairs_hook=pairs,
                           parse_constant=lambda token: error("nonfinite_number", f"nonfinite JSON number: {token}"))
    except DocumentError:
        raise
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        error("document_read", str(exc), path)
    finite(value)
    if not isinstance(value, dict):
        error("document_type", "document root must be an object", path)
    return value


def canonical_json(value):
    finite(value)
    return json.dumps(value, sort_keys=True, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")


def document_digest(value):
    return hashlib.sha256(canonical_json(value)).hexdigest()


def protocol_digest(suite):
    from importlib.metadata import version
    from metrics import METRIC_VERSION, COLOR_TRANSFORM
    from references import METRIC_VERSION as reference_version

    def identity(value):
        if isinstance(value, dict):
            return {key: identity(item) for key, item in value.items() if key != "path"}
        if isinstance(value, list):
            return [identity(item) for item in value]
        return value

    cases = [{key: identity(case[key]) for key in ("id", "input", "seeds", "regions", "measurements", "probe", "probe_response", "reference") if key in case} for case in suite["cases"]]
    icc_identity = None
    if any("icc" in case.get("reference", {}).get("color", {}) for case in suite["cases"]):
        from icc import library
        icc_identity = {"lcms2": library().cmsGetEncodedCMMversion(), "intent": "relative_colorimetric", "black_point_compensation": False, "format": "RGB_DBL"}
    return document_digest({"protocol": suite["protocol"], "cases": cases,
                            "metric_version": METRIC_VERSION, "reference_version": reference_version,
                            "color_transform": COLOR_TRANSFORM, "icc_adapter": icc_identity,
                            "dependencies": {name: version(name) for name in ("numpy", "scipy", "colour-science", "flip-evaluator", "Pillow", "tifffile")}})


def sha256_file(path):
    digest = hashlib.sha256()
    try:
        with Path(path).open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(block)
    except OSError as exc:
        error("dependency_read", str(exc), path)
    return digest.hexdigest()


def verify_sha256(path, expected):
    if not isinstance(expected, str) or not HASH_PATTERN.fullmatch(expected):
        error("invalid_hash", "SHA-256 requires 64 lowercase hexadecimal digits", path)
    observed = sha256_file(path)
    if observed != expected:
        error("hash_mismatch", f"expected {expected}; observed {observed}", path)
    return observed


def resolve_path(value, document_path):
    if not isinstance(value, str) or not value or "\0" in value:
        error("invalid_path", "path must be a nonempty string without NUL")
    return (Path(document_path).absolute().parent / value).resolve()


def safe_artifact_path(root, value, *, must_exist=True):
    if not isinstance(value, str) or not value or "\0" in value or "\\" in value or ":" in value:
        error("unsafe_artifact_path", "artifact requires a relative POSIX path", value)
    relative = PurePosixPath(value)
    if relative.is_absolute() or ".." in relative.parts or value in (".", ".."):
        error("unsafe_artifact_path", "artifact contains absolute root or traversal", value)
    root = Path(root).resolve()
    result = root
    for part in relative.parts:
        result /= part
        if result.is_symlink():
            error("unsafe_artifact_path", "artifact symlinks are not supported", value)
    if not result.resolve().is_relative_to(root):
        error("unsafe_artifact_path", "artifact escapes directory", value)
    if must_exist and not result.is_file():
        error("missing_artifact", "artifact must be a regular file", value)
    return result


def validate_schema(kind, value):
    finite(value)
    errors = sorted(Draft202012Validator(SCHEMAS[kind]).iter_errors(value), key=lambda item: str(list(item.path)))
    if errors:
        first = errors[0]
        code = {"additionalProperties": "unknown_field", "required": "missing_field", "uniqueItems": "duplicate_value",
                "minimum": "invalid_bounds", "maximum": "invalid_bounds", "minItems": "invalid_bounds",
                "pattern": "invalid_identifier"}.get(first.validator, "schema_validation")
        error(code, first.message, "$" + "".join(f"[{key}]" for key in first.path))


def unique(items, key="id"):
    values = [item[key] for item in items]
    if len(values) != len(set(values)):
        error("duplicate_id", f"duplicate {key}")


def dependency(record, document):
    return verify_sha256(resolve_path(record["path"], document), record["sha256"])


def geometry(value, document):
    if isinstance(value, list):
        if value[2] <= 0 or value[3] <= 0:
            error("invalid_bounds", "region width and height must be positive")
    else:
        dependency(value, document)
        if Path(value["path"]).suffix.lower() not in (".png", ".tif", ".tiff", ".npy", ".pgm"):
            error("invalid_mask", "mask must use a lossless binary image or NPY")


def config_files(configuration, document):
    for key in ("presets", "set_files"):
        for path in configuration.get(key, []):
            sha256_file(resolve_path(path, document))
    if configuration.get("params"):
        sha256_file(resolve_path(configuration["params"], document))


def load_suite(path):
    value = load_json_document(path)
    validate_schema("suite", value)
    unique(value["cases"])
    for case in value["cases"]:
        declaration = case["input"]
        dependency(declaration, path)
        kind = declaration.get("kind", "raster")
        if kind == "raw":
            if declaration.get("adapter") != "spektrafilm-raw" or "color_space" in declaration or "cctf_decoding" in declaration:
                error("source_declaration", "RAW requires spektrafilm-raw adapter and no raster declaration", case["id"])
        elif "adapter" in declaration or not {"color_space", "cctf_decoding"} <= declaration.keys():
            error("source_declaration", "raster requires explicit color and transfer declaration", case["id"])
        if case["configuration"].get("raw") and kind != "raw":
            error("inapplicable_field", "RAW controls require RAW source", case["id"])
        config_files(case["configuration"], path)
        unique(case["regions"])
        for region in case["regions"]:
            if region["id"] == "full_image":
                error("reserved_region", "full_image is reserved for whole-image measurements")
            geometry(region["geometry"], path)
        probe = case.get("probe", {})
        for sample in probe.get("tone_response", []):
            geometry(sample["geometry"], path)
        if "reference" in case:
            reference = case["reference"]
            dependency(reference, path)
            for key in ("transform", "aligned_derivative", "correspondence_review", "common_area_mask"):
                if key in reference:
                    dependency(reference[key], path)
            if reference["scope"] == "aligned_scene":
                if not {"correspondence_review", "common_area_mask"} <= reference.keys() or not ("transform" in reference or "aligned_derivative" in reference):
                    error("missing_correspondence", "aligned scene requires transform/derivative, review, and mask")
                review = load_json_document(resolve_path(reference["correspondence_review"]["path"], path))
                for excluded in review.get("excluded_regions", []):
                    geometry(excluded["geometry"], path)
            elif reference["scope"] == "regional" and not reference.get("regions"):
                error("missing_regions", "regional reference requires corresponding regions")
            elif reference["scope"] == "material" and not {"domain", "units", "instrument", "conditions"} <= reference.keys():
                error("material_conditions", "material reference requires declared measurement conditions")
            if "icc" in reference["color"]:
                dependency(reference["color"]["icc"], path)
            elif not {"color_space", "cctf_decoding"} <= reference["color"].keys():
                error("reference_color", "reference requires explicit color or hashed ICC")
            for region in reference.get("regions", []):
                geometry(region["geometry"], path)
        if "probe_response" in case:
            allowed = {region["id"] for region in case["regions"]} | {"full_image"}
            for curve in case["probe_response"]["curves"]:
                samples = curve["samples"]
                coordinates = [sample["coordinate"] for sample in samples]
                if any(b <= a for a, b in zip(coordinates, coordinates[1:])) or any(sample["region_id"] not in allowed for sample in samples):
                    error("probe_coordinates", "probe coordinates must increase and regions must resolve")
    return value


def load_target(path):
    value = load_json_document(path)
    validate_schema("target", value)
    dependency(value["renderer"], path)
    if not os.access(resolve_path(value["renderer"]["path"], path), os.X_OK):
        error("renderer_executable", "renderer must be executable")
    if not resolve_path(value["data_dir"], path).is_dir():
        error("data_directory", "data directory must exist")
    for changes in [value["changes"], *value.get("case_changes", {}).values()]:
        config_files(changes, path)
        for assignment in changes.get("set", []):
            for forbidden in ("io.input_color_space", "io.input_cctf_decoding", "io.crop", "io.crop_center", "io.crop_size", "io.upscale_factor"):
                if re.search(r"(?:^|,)\s*" + re.escape(forbidden) + r"\s*=", assignment):
                    error("target_geometry", "target must not modify input interpretation or geometry")
    return value


def validate_target_cases(target, suite):
    unknown = set(target.get("case_changes", {})) - {case["id"] for case in suite["cases"]}
    if unknown:
        error("unresolved_identifier", f"unknown target case: {sorted(unknown)[0]}")


def load_policy(path):
    value = load_json_document(path)
    validate_schema("policy", value)
    unique(value["rules"])
    calibration = value["calibration"]
    if calibration["status"] == "calibrated" and (not calibration["evidence"] or not {"cases", "labels", "observation_scale", "scope"} <= calibration.keys()):
        error("missing_calibration", "calibrated policy requires reviewed evidence and population")
    for item in calibration["evidence"]:
        dependency(item, path)
    for rule in value["rules"]:
        if "min" in rule and "max" in rule and rule["min"] > rule["max"]:
            error("invalid_bounds", "rule min exceeds max")
        if not ("min" in rule or "max" in rule or (rule["purpose"] == "expect" and "direction" in rule)):
            error("missing_constraint", "rule requires an interval or expected direction")
        if "direction" in rule and rule["purpose"] != "expect":
            error("inapplicable_field", "direction belongs to expected-change rules")
    return value


def validate_policy_selectors(policy, suite):
    if policy["protocol_digest"] != protocol_digest(suite):
        error("protocol_mismatch", "policy must bind to comparison protocol")
    cases = {case["id"]: {region["id"] for region in case["regions"]} | {"full_image"} for case in suite["cases"]}
    for rule in policy["rules"]:
        selection = rule["selector"]
        if selection["case_id"] not in cases or selection["region_id"] not in cases[selection["case_id"]]:
            error("unresolved_identifier", "policy selector does not resolve", rule["id"])


def validate_generated_document(kind, value, root):
    validate_schema(kind, value)
    expected = document_digest({key: item for key, item in value.items() if key != "id"})
    if value["id"] != expected:
        error("manifest_identity", "manifest identity does not match its contents")
    indexed = {}
    for artifact in value["artifacts"]:
        path = artifact["path"]
        if path in indexed:
            error("duplicate_artifact", "artifact index paths must be unique", path)
        indexed[path] = artifact
        verify_sha256(safe_artifact_path(root, path), artifact["sha256"])

    def reference(item):
        if isinstance(item, str):
            if item not in indexed:
                error("unindexed_artifact", "artifact reference must resolve in index", item)
            return
        if not isinstance(item, dict) or item.get("path") not in indexed:
            error("unindexed_artifact", "artifact record must resolve in index")
        if item.get("sha256") != indexed[item["path"]]["sha256"]:
            error("artifact_identity", "artifact reference hash differs from index", item["path"])

    def measurements(rows):
        for row in rows:
            required = {"case_id", "region_id", "metric", "domain", "units", "estimator", "sample_count", "availability", "seed_population", "artifact_refs"}
            if not required <= row.keys() or type(row["sample_count"]) is not int or row["sample_count"] < 0:
                error("measurement_fields", "measurement requires identity, population and estimator")
            if row["availability"] == "available":
                if type(row.get("value")) not in (int, float) or not math.isfinite(row["value"]) or "reason" in row:
                    error("measurement_value", "available measurement requires one finite value")
            elif row["availability"] in ("unavailable", "not_applicable"):
                if "value" in row or not isinstance(row.get("reason"), str) or not row["reason"]:
                    error("measurement_availability", "unavailable measurement requires reason and no value")
            else:
                error("measurement_availability", "invalid measurement availability")
            for artifact in row["artifact_refs"]:
                reference(artifact)

    if kind == "run":
        reference(value["suite"])
        reference(value["target"])
        suite = load_json_document(safe_artifact_path(root, value["suite"]["path"]))
        if document_digest(suite) != value["suite"]["digest"] or suite["protocol"] != value["protocol"]:
            error("suite_identity", "suite snapshot and protocol mismatch")
        selected = value["selection"]["case_ids"]
        all_ids = [case["id"] for case in suite["cases"]]
        if len(selected) != len(set(selected)) or set(selected) - set(all_ids) or not selected:
            error("run_selection", "selection must be nonempty distinct suite cases")
        if value["selection"]["full_suite"] != (selected == all_ids):
            error("run_selection", "full-suite flag disagrees with selection")
        expected_rows = {(case["id"], seed) for case in suite["cases"] if case["id"] in selected for seed in case["seeds"]}
        observed = []
        for row in value["cases"]:
            if not ID_PATTERN.fullmatch(row.get("case_id", "")) or type(row.get("seed")) is not int or not 0 <= row["seed"] <= 4294967295:
                error("run_case", "invalid case identity or seed")
            observed.append((row["case_id"], row["seed"]))
            if row["status"] not in ("completed", "failed"):
                error("run_status", "invalid run case status")
            if row["status"] == "completed":
                for name in ("input", "raw", "raw_metadata", "master", "diagnostics", "resolved", "stdout", "stderr"):
                    if name not in row:
                        error("missing_evidence", f"completed case requires {name}")
                    reference(row[name])
                layout = row["raw"]["layout"]
                byte_count = layout["width"] * layout["height"] * 3 * 8
                if safe_artifact_path(root, row["raw"]["path"]).stat().st_size != byte_count:
                    error("raw_layout", "raw byte count differs from dimensions")
                diagnostics = load_json_document(safe_artifact_path(root, row["diagnostics"]["path"]))
                if diagnostics["configuration"]["published_dimensions"] != row["dimensions"]:
                    error("render_geometry", "diagnostics and retained dimensions disagree")
            elif not row.get("error"):
                error("missing_failure", "failed render requires structured error")
        if set(observed) != expected_rows or len(observed) != len(expected_rows):
            error("run_selection", "rendered case/seed population differs from suite")
        if value["execution_status"] == "completed" and any(row["status"] != "completed" for row in value["cases"]):
            error("run_status", "completed run contains failed cases")
        for item in value["dependencies"]:
            if item.get("role") != "renderer":
                reference(item)
    elif kind == "comparison":
        if "policy" in value:
            reference(value["policy"])
        elif value["evaluation_status"] != "not_evaluated":
            error("evaluation_status", "comparison without policy must remain not_evaluated")
        for case in value["cases"]:
            measurements(case.get("measurements", []))
            for item in case.get("review", {}).values():
                reference(item)
            for item in case.get("artifacts", []):
                reference(item)
        measurements(value.get("seed_measurements", []))
        unique(value["findings"])
        for finding in value["findings"]:
            if not {"id", "case_id", "region_id", "metric", "decision", "evidence"} <= finding.keys() or not finding["evidence"]:
                error("finding_evidence", "finding requires identity and inspectable evidence")
            for item in finding["evidence"]:
                reference(item)
        if value["execution_status"] == "completed" and any(case["status"] == "failed" for case in value["cases"]):
            error("comparison_status", "completed comparison contains failed measurements")
        if value["evaluation_status"] == "passed" and value["execution_status"] != "completed":
            error("evaluation_status", "failed execution cannot qualify")
    return value
