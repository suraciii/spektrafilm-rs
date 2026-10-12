"""JSON Schemas for version-one benchmark document carriers."""
from copy import deepcopy


def obj(properties, required=(), *, authored=True):
    return {"type": "object", "properties": properties, "required": list(required),
            "additionalProperties": not authored}


def array(items, minimum=0, unique=False):
    return {"type": "array", "items": items, "minItems": minimum, "uniqueItems": unique}


STRING = {"type": "string", "minLength": 1}
ID = {"type": "string", "pattern": r"^[A-Za-z0-9][A-Za-z0-9_.-]*$"}
HASH = {"type": "string", "pattern": r"^[0-9a-f]{64}$"}
NUMBER = {"type": "number"}
POSITIVE = {"type": "number", "exclusiveMinimum": 0}
INTEGER = {"type": "integer", "minimum": 0}
U32 = {"type": "integer", "minimum": 0, "maximum": 4294967295}
SEED = U32
MEASUREMENTS = array({"enum": ["numeric", "color", "tone", "flip", "grain", "halation", "edge"]}, unique=True)
FILE = obj({"path": STRING, "sha256": HASH}, ("path", "sha256"))
CHANGES = obj({"set_files": array(STRING), "set": array(STRING)}, ("set_files", "set"))
RAW = obj({"white_balance": STRING, "temperature": POSITIVE, "tint": NUMBER,
           "lens_correction": {"type": "boolean"}})
CONFIGURATION = obj({"film": STRING, "paper": STRING, "presets": array(STRING), "params": STRING,
                     "set_files": array(STRING), "set": array(STRING), "route": STRING,
                     "scan_output": STRING, "raw": RAW})
RECT = {"type": "array", "items": INTEGER, "minItems": 4, "maxItems": 4}
MASK = obj({"path": STRING, "sha256": HASH, "width": {"type": "integer", "minimum": 1},
            "height": {"type": "integer", "minimum": 1}}, ("path", "sha256", "width", "height"))
GEOMETRY = {"anyOf": [RECT, MASK]}
REGION = obj({"id": ID, "label": STRING, "role": {"enum": ["skin", "neutral", "shadow", "highlight", "color", "flat_field", "edge", "texture", "other"]},
              "geometry": GEOMETRY, "measurements": MEASUREMENTS}, ("id", "label", "role", "geometry", "measurements"))
PROTOCOL = obj({
    "id": {"const": "appearance-v1"},
    "pixels_per_degree": POSITIVE,
    "region_sampling_scales": array(POSITIVE, 1, True),
    "grain": obj({
        "residual": obj({
            "method": {"enum": ["flat_mean", "gaussian_high_pass"]},
            "high_pass_radius_px": POSITIVE,
            "border_px": {"type": "integer", "minimum": 0},
        }, ("method", "high_pass_radius_px", "border_px")),
        "spectrum": obj({"window": {"enum": ["hann", "boxcar"]}}, ("window",)),
    }, ("residual", "spectrum")),
    "halation": obj({
        "center_px": {"type": "array", "items": NUMBER, "minItems": 2, "maxItems": 2}, "core_radius_px": POSITIVE,
        "background_inner_radius_px": POSITIVE, "background_outer_radius_px": POSITIVE,
        "extent_px": POSITIVE, "bin_width_px": POSITIVE,
        "domain": {"const": "linear_srgb"},
    }, ("center_px", "core_radius_px", "background_inner_radius_px", "background_outer_radius_px", "extent_px", "bin_width_px", "domain")),
    "edge": obj({
        "axis": {"enum": ["x", "y"]}, "plateau_fraction": {"const": 0.2},
        "domain": {"const": "linear_srgb"}, "input_contrast": NUMBER,
        "input_exposure_stops": NUMBER,
    }, ("axis", "plateau_fraction", "domain", "input_contrast", "input_exposure_stops")),
    "local_contrast_scales_sigma_px": array(POSITIVE),
    "probe": obj({"tone_response": array(obj({
        "exposure_stops": NUMBER, "geometry": GEOMETRY,
    }, ("exposure_stops", "geometry")))}),
    "hotspots": obj({"count": INTEGER, "min_area_px": {"type": "integer", "minimum": 1}}, ("count", "min_area_px")),
}, ("id", "pixels_per_degree", "region_sampling_scales", "grain", "hotspots"))
REFERENCE = obj({"id": ID, "path": STRING, "sha256": HASH,
                 "scope": {"enum": ["material", "aligned_scene", "regional", "visual"]},
                 "color": obj({"color_space": STRING, "cctf_decoding": {"type": "boolean"}, "icc": FILE}),
                 "provenance": obj({key: STRING for key in ("source", "attribution", "redistribution", "capture", "development", "digitization")}, ("source", "attribution", "redistribution", "capture", "development", "digitization")),
                 "processing": obj({key: STRING for key in ("grading", "histogram_matching", "compression", "sharpening", "noise_reduction", "resampling")}, ("grading", "histogram_matching", "compression", "sharpening", "noise_reduction", "resampling")),
                 "transform": FILE, "aligned_derivative": FILE, "correspondence_review": FILE,
                 "common_area_mask": FILE, "regions": array(REGION), "domain": STRING,
                 "instrument": STRING, "units": STRING, "conditions": STRING},
                ("id", "path", "sha256", "scope", "color", "provenance", "processing"))
COORDINATE = obj({"name": STRING, "units": STRING}, ("name", "units"))
REFERENCE_CURVE = obj({"id": ID, "channel": {"enum": ["R", "G", "B", "L", "a", "b", "Y"]},
                       "coordinates": array(NUMBER, 1), "values": array(NUMBER, 1)}, ("id", "channel", "coordinates", "values"))
REFERENCE_EVALUATION = obj({"schema_version": {"const": 1}, "domain": STRING, "units": STRING,
                            "instrument": STRING, "conditions": STRING, "coordinate": COORDINATE,
                            "curves": array(REFERENCE_CURVE, 1)},
                           ("schema_version", "domain", "units", "instrument", "conditions", "coordinate", "curves"))
PROBE_RESPONSE = obj({"domain": STRING, "units": STRING, "instrument": STRING, "conditions": STRING,
                      "coordinate": COORDINATE,
                      "estimator": {"enum": ["native_channel_mean", "native_channel_median", "lab_channel_mean", "lab_channel_median", "linear_luminance_mean", "linear_luminance_median"]},
                      "curves": array(obj({"id": ID, "channel": {"enum": ["R", "G", "B", "L", "a", "b", "Y"]},
                                           "samples": array(obj({"coordinate": NUMBER, "region_id": ID}, ("coordinate", "region_id")), 1)}, ("id", "channel", "samples")), 1)},
                     ("domain", "units", "instrument", "conditions", "coordinate", "estimator", "curves"))
CASE = obj({"id": ID, "kind": {"enum": ["probe", "scene"]},
            "input": obj({"path": STRING, "sha256": HASH, "kind": {"enum": ["raster", "raw"]}, "adapter": STRING,
                          "color_space": STRING, "cctf_decoding": {"type": "boolean"}}, ("path", "sha256")),
            "configuration": CONFIGURATION, "seeds": array(SEED, 1, True),
            "regions": array(REGION), "measurements": MEASUREMENTS, "reference": REFERENCE,
            "probe_response": PROBE_RESPONSE, "probe": obj({"halation": PROTOCOL["properties"]["halation"], "edge": PROTOCOL["properties"]["edge"], "tone_response": PROTOCOL["properties"]["probe"]["properties"]["tone_response"]}), "split": {"enum": ["calibration", "evaluation"]}, "split_group": ID},
           ("id", "kind", "input", "configuration", "seeds", "regions", "measurements"))
CALIBRATION = obj({"status": {"enum": ["calibrated", "exploratory"]}, "evidence": array(FILE),
                   "cases": array(ID, unique=True), "labels": array(STRING),
                   "observation_scale": POSITIVE, "scope": STRING}, ("status", "evidence"))
RULE = obj({"id": ID, "purpose": {"enum": ["protect", "expect", "reference"]},
            "selector": obj({"case_id": ID, "region_id": ID}, ("case_id", "region_id")),
            "metric": STRING, "required": {"type": "boolean"}, "min": NUMBER, "max": NUMBER,
            "direction": {"enum": ["increase", "decrease", "unchanged"]}},
           ("id", "purpose", "selector", "metric", "required"))
ARTIFACT = obj({"path": STRING, "sha256": HASH, "media_type": STRING, "role": STRING,
                "width": {"type": "integer", "minimum": 1}, "height": {"type": "integer", "minimum": 1},
                "dimensions": array({"type": "integer", "minimum": 1}, 1), "layout": {"type": "object"}},
               ("path", "sha256", "media_type", "role"), authored=False)
SCHEMAS = {
    "suite": obj({"schema_version": {"const": 1}, "id": ID, "protocol": PROTOCOL, "cases": array(CASE, 1)}, ("schema_version", "id", "protocol", "cases")),
    "target": obj({"schema_version": {"const": 1}, "id": ID, "renderer": FILE, "data_dir": STRING,
                   "backend": {"enum": ["cpu", "gpu"]}, "changes": CHANGES,
                   "case_changes": {"type": "object", "additionalProperties": CHANGES},
                   "environment": {"type": "object", "properties": {key: STRING for key in ("RAYON_NUM_THREADS", "OPENBLAS_NUM_THREADS", "OMP_NUM_THREADS")}, "additionalProperties": False}},
                  ("schema_version", "id", "renderer", "data_dir", "backend", "changes")),
    "policy": obj({"schema_version": {"const": 1}, "id": ID, "protocol_digest": HASH,
                   "calibration": CALIBRATION, "rules": array(RULE)}, ("schema_version", "id", "protocol_digest", "calibration", "rules")),
}
for kind in ("run", "comparison"):
    properties = {"schema_version": {"const": 1}, "id": STRING, "execution_status": {"enum": ["completed", "failed"]}}
    if kind == "run":
        properties.update({"suite": {"type": "object"}, "target": {"type": "object"}, "protocol": {"type": "object"},
                           "selection": {"type": "object"}, "dependencies": {"type": "array"},
                           "cases": array({"type": "object"}), "artifacts": array(ARTIFACT)})
        required = ("schema_version", "id", "execution_status", "suite", "target", "protocol", "selection", "dependencies", "cases", "artifacts")
    else:
        properties.update({"evaluation_status": {"enum": ["not_evaluated", "passed", "review_required", "failed", "insufficient_evidence"]},
                           "baseline": {"type": "object"}, "candidate": {"type": "object"}, "protocol": {"type": "object"},
                           "selection": {"type": "object"}, "condition_differences": {"type": "array"}, "cases": array({"type": "object"}),
                           "findings": array({"type": "object"}), "summary": {"type": "object"}, "artifacts": array(ARTIFACT), "policy": {"type": "object"}})
        required = ("schema_version", "id", "execution_status", "evaluation_status", "baseline", "candidate", "protocol", "selection", "condition_differences", "cases", "findings", "summary", "artifacts")
    SCHEMAS[kind] = obj(properties, required, authored=False)


def schema_for(kind):
    if kind not in SCHEMAS:
        raise ValueError(f"unknown document kind: {kind}")
    result = deepcopy(SCHEMAS[kind])
    result["$schema"] = "https://json-schema.org/draft/2020-12/schema"
    result["title"] = f"SpektraFilm benchmark {kind} version 1"
    return result


get_schema = schema_for
