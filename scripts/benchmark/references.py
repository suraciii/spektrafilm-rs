"""Frozen external-reference comparison; no fitting or candidate-driven processing.

Metric identity version: spektrafilm-reference-v1. Every numeric objective emits
three suffixes: baseline_distance, candidate_distance, distance_delta. The last
is candidate minus baseline; negative means closer to this named reference.
Aligned families: reference.delta_e00.{median,p95,p99,max},
reference.tone_l_star.{q05,q25,q50,q75,q95}, reference.chroma_ab.median.
Regional families: reference.patch_delta_e00, reference.distribution_l_star,
reference.patch_chroma_ab. Material: reference.material.<curve-id> (RMSE).
Tone-contaminated evidence never supplies DE00 or lightness fidelity. Chroma-only
rows are explicitly diagnostic and require a known ungraded color interpretation.
"""
from pathlib import Path
import hashlib
import json
import re

import numpy as np
from PIL import Image, ImageCms

try:
    from .image_data import ImageDataError, load_image, display_rgb, rgb_to_lab, srgb_decode
    from .metrics import ciede2000, rectangle_mask
except ImportError:
    from image_data import ImageDataError, load_image, display_rgb, rgb_to_lab, srgb_decode
    from metrics import ciede2000, rectangle_mask

METRIC_VERSION = "spektrafilm-reference-v1"
COLOR_TRANSFORM = "srgb-d65-cie1931-2deg-bounded-v1"
SIDES = ("baseline_distance", "candidate_distance", "distance_delta")


def _path(value, base):
    value = value["path"] if isinstance(value, dict) else value
    path = Path(value)
    return path if path.is_absolute() else Path(base) / path


def _json(value, base):
    with _path(value, base).open(encoding="utf-8") as stream:
        result = json.load(stream)
    if not isinstance(result, dict):
        raise ImageDataError("invalid_reference_evidence", "Reference evidence must be a JSON object")
    return result


def _mask(geometry, dimensions, base):
    if isinstance(geometry, (list, tuple)) or (isinstance(geometry, dict) and "rectangle" in geometry):
        return rectangle_mask(geometry, dimensions)
    if not isinstance(geometry, dict) or "path" not in geometry:
        raise ImageDataError("invalid_region", "Mask requires a frozen rectangle or file")
    path = _path(geometry, base)
    if path.suffix.lower() == ".npy":
        values = np.load(path, allow_pickle=False)
    else:
        with Image.open(path) as image:
            if image.mode not in ("1", "L"):
                raise ImageDataError("invalid_region", "Mask must be a single-channel binary image")
            values = np.asarray(image)
    if values.shape != (dimensions[1], dimensions[0]):
        raise ImageDataError("invalid_region", "Mask dimensions differ from declared coordinate system")
    if any(key in geometry and geometry[key] != actual for key, actual in zip(("width", "height"), dimensions)):
        raise ImageDataError("invalid_region", "Mask dimensions differ from its declaration")
    if values.dtype.kind not in "bufi" or not np.all(np.isfinite(values)):
        raise ImageDataError("invalid_region", "Mask contains invalid samples")
    if not np.all(np.isin(values, [0, 1, 255])):
        raise ImageDataError("invalid_region", "Mask must be binary; no implicit thresholding")
    return values != 0


def _read_reference(reference, base):
    """Retain native reference samples; shared color boundary honors ICC."""
    color = dict(reference.get("color", {}))
    if isinstance(color.get("icc"), dict) and color["icc"].get("path"):
        color["icc"] = {**color["icc"], "path": str(_path(color["icc"], base))}
    return load_image({"path": str(_path(reference["path"], base)), "color": color})


def _warp(values, transform, dimensions):
    """Apply only the authored reference-to-render matrix; return valid support."""
    if (transform.get("direction") != "reference_to_render"
            or transform.get("coordinate_system") != "pixel_centers"
            or transform.get("interpolation") not in ("bilinear", "nearest")):
        raise ImageDataError("invalid_correspondence", "Transform must declare direction, pixel centers and interpolation")
    matrix = np.asarray(transform.get("matrix"), dtype=np.float64)
    if matrix.shape != (3, 3) or not np.all(np.isfinite(matrix)) or abs(np.linalg.det(matrix)) < 1e-12:
        raise ImageDataError("invalid_correspondence", "Transform matrix must be finite and invertible 3 x 3")
    width, height = dimensions
    y, x = np.indices((height, width), dtype=np.float64)
    mapped = np.stack((x + .5, y + .5, np.ones_like(x)), axis=-1) @ np.linalg.inv(matrix).T
    divisor = mapped[..., 2]
    good = np.isfinite(divisor) & (np.abs(divisor) > 1e-12)
    safe = np.where(good, divisor, 1)
    sx, sy = mapped[..., 0] / safe - .5, mapped[..., 1] / safe - .5
    good &= np.isfinite(sx) & np.isfinite(sy)
    good &= (sx >= 0) & (sy >= 0) & (sx <= values.shape[1] - 1) & (sy <= values.shape[0] - 1)
    sx = np.clip(np.where(good, sx, 0), 0, values.shape[1] - 1)
    sy = np.clip(np.where(good, sy, 0), 0, values.shape[0] - 1)
    if transform["interpolation"] == "nearest":
        return values[np.floor(sy + .5).astype(int), np.floor(sx + .5).astype(int)], good
    x0, y0 = np.floor(sx).astype(int), np.floor(sy).astype(int)
    x1, y1 = np.minimum(x0 + 1, values.shape[1] - 1), np.minimum(y0 + 1, values.shape[0] - 1)
    fx, fy = (sx - x0)[..., None], (sy - y0)[..., None]
    warped = ((1 - fy) * ((1 - fx) * values[y0, x0] + fx * values[y0, x1])
              + fy * ((1 - fx) * values[y1, x0] + fx * values[y1, x1]))
    return warped, good


def _processing_eligibility(reference):
    processing = reference.get("processing", {})
    clean = ("none", "unmodified", "not_applied")
    grading = processing.get("grading", "unknown").strip().lower()
    histogram = processing.get("histogram_matching", "unknown").strip().lower()
    tone = grading in clean and histogram in clean
    chroma = grading in clean and histogram in (*clean, "brightness_only")
    return tone, chroma


def compare_reference(baseline, candidate, case, protocol, out_dir, suite_base):
    """Return scalars, findings, immutable local artifacts and reference_review.

    File hashes/dependency identity are checked by the orchestrator. This boundary
    checks geometry, correspondence semantics, matching material conditions and
    finite arithmetic. It neither uploads evidence nor asserts redistribution rights.
    """
    reference = case.get("reference")
    result = {"measurements": [], "findings": [], "artifacts": [],
              "execution_status": "completed", "metric_version": METRIC_VERSION,
              "reference_review": None}
    if reference is None:
        return result
    protocol = protocol if isinstance(protocol, dict) else {"id": protocol}
    case_id = case["id"]
    seeds = protocol.get("seed_population", case.get("seeds", []))
    identity_prefix = case_id + ":reference:" + hashlib.sha256(json.dumps(seeds, separators=(",", ":")).encode()).hexdigest()[:12]
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    review = {"id": reference["id"], "scope": reference["scope"],
              "source_sha256": reference.get("sha256"), "provenance": reference.get("provenance", {}),
              "processing": reference.get("processing", {}), "color": reference.get("color", {}),
              "limitations": ["Distance describes the selected reference, not physical film accuracy.",
                             "Local research access does not grant redistribution of originals or derivatives."],
              "geometry": "side_by_side", "artifact_refs": [],
              "split": case.get("split"), "split_group": case.get("split_group"),
              "color_transform": COLOR_TRANSFORM}
    result["reference_review"] = review
    sequence = 0

    def artifact(values, role, *, units=None, domain=None, coordinates=None, json_value=None, display=False):
        nonlocal sequence
        sequence += 1
        name = f"reference-{sequence:03d}-{role}"
        path = out / (name + (".json" if json_value is not None else ".png" if display else ".npy"))
        if json_value is not None:
            path.write_text(json.dumps(json_value, indent=2, allow_nan=False) + "\n", encoding="utf-8")
        elif display:
            import png16
            png16.save(path, np.rint(np.clip(values, 0, 1) * 65535).astype(np.uint16))
        else:
            np.save(path, np.asarray(values), allow_pickle=False)
        identity = f"{identity_prefix}:{sequence}"
        row = {"id": identity, "path": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
               "media_type": "application/json" if json_value is not None else "image/png" if display else "application/x-npy",
               "role": role, "reference_id": reference["id"]}
        row["provenance"] = reference.get("provenance", {})
        row["redistribution"] = reference.get("provenance", {}).get("redistribution", "unknown")
        if values is not None:
            row["layout"] = {"dtype": str(np.asarray(values).dtype), "axis_order": "height,width,channel" if np.asarray(values).ndim == 3 else "height,width"}
            if np.asarray(values).ndim >= 2:
                row["coordinates"] = {"system": "pixel_centers", "x": {"start": .5, "step": 1, "count": int(np.asarray(values).shape[1])},
                                      "y": {"start": .5, "step": 1, "count": int(np.asarray(values).shape[0])}}
        if values is not None:
            row["dimensions"] = list(np.asarray(values).shape)
            if display:
                row.update(width=int(np.asarray(values).shape[1]), height=int(np.asarray(values).shape[0]),
                           bit_depth=16, quantization="16-bit rounded; no dither")
        if units is not None:
            row["units"] = units
        if domain is not None:
            row["domain"] = domain
        if coordinates is not None:
            row["coordinates"] = coordinates
        result["artifacts"].append(row)
        return identity

    def finding(region, metric, reason, *, failure=False, evidence=None):
        result["findings"].append({"id": f"{identity_prefix}:{region}:{metric}:{reason}", "case_id": case_id,
                                  "region_id": region, "metric": metric, "decision": "unavailable",
                                  "code": reason, "evidence": evidence or []})
        if failure:
            result["execution_status"] = "failed"

    def distances(region, metric, domain, units, estimator, count, values=None, reason=None, refs=None):
        if values is not None and (len(values) != 2 or not np.all(np.isfinite(values))):
            reason = "computation_failure"
            finding(region, metric, reason, failure=True, evidence=refs)
        expanded = None if reason or values is None else [float(values[0]), float(values[1]), float(values[1] - values[0])]
        if expanded is not None and not np.all(np.isfinite(expanded)):
            reason, expanded = "computation_failure", None
            finding(region, metric, reason, failure=True, evidence=refs)
        for side, index in zip(SIDES, range(3)):
            row = {"case_id": case_id, "seed_population": seeds, "region_id": region,
                   "reference_id": reference["id"], "metric": metric + "." + side,
                   "metric_version": METRIC_VERSION, "protocol_id": protocol.get("id"),
                   "domain": domain, "units": units, "estimator": estimator,
                   "sample_count": int(count), "artifact_refs": refs or [],
                   "availability": "available" if expanded is not None else "unavailable"}
            if expanded is not None:
                row["value"] = expanded[index]
            else:
                row["reason"] = reason or "missing_evidence"
            result["measurements"].append(row)
        if expanded is not None:
            result["findings"].append({"id": f"{identity_prefix}:{region}:{metric}:measured", "case_id": case_id,
                                      "region_id": region, "metric": metric + ".distance_delta",
                                      "decision": "measured", "evidence": refs or [],
                                      "baseline_distance": expanded[0], "candidate_distance": expanded[1],
                                      "distance_delta": expanded[2],
                                      "interpretation": "Negative distance_delta means closer to this reference; no acceptance threshold is inferred."})
        else:
            finding(region, metric, reason or "missing_evidence", evidence=refs)

    def image_specs(region, reason, *, regional=False):
        if regional:
            specs = [("reference.patch_delta_e00", "delta E 00"),
                     ("reference.distribution_l_star", "L*"), ("reference.patch_chroma_ab", "Lab a*/b* distance")]
        else:
            specs = [("reference.delta_e00." + stat, "delta E 00") for stat in ("median", "p95", "p99", "max")]
            specs += [("reference.tone_l_star.q" + str(q).zfill(2), "L*") for q in (5, 25, 50, 75, 95)]
            specs += [("reference.chroma_ab.median", "Lab a*/b* distance")]
        for metric, units in specs:
            distances(region, metric, "bounded_srgb_d65_lab", units, "frozen reference population", 0, reason=reason)

    try:
        if reference["scope"] == "material":
            _material(reference, baseline, candidate, case, suite_base, distances, artifact, review, finding)
            return result
        source_reference = _read_reference(reference, suite_base)
        native = source_reference.samples
        if not np.all(np.isfinite(native)):
            raise ImageDataError("nonfinite_samples", "Reference contains nonfinite samples")
        master_id = artifact(native, "reference_native", domain=source_reference.color_space or "unknown")
        review.update(master_artifact_id=master_id, native_dimensions=source_reference.dimensions,
                      decode_metadata=source_reference.metadata)
        review["artifact_refs"].append(master_id)
        color_reason = None
        try:
            encoded = display_rgb(source_reference)
            if not np.all(np.isfinite(encoded)):
                raise ImageDataError("computation_failure", "Reference color conversion produced nonfinite samples")
            review["display_clipped_sample_fraction"] = float(np.mean((encoded < 0) | (encoded > 1)))
            review["color_available"] = True
        except ImageDataError as exc:
            color_reason = exc.code
            encoded = native
            review["color_available"] = False
            review["limitations"].append("Preview uses uninterpreted native RGB; color fidelity is unavailable.")
        preview = out / "reference-display.png"
        import png16
        png16.save(preview, np.rint(np.clip(encoded, 0, 1) * 65535).astype(np.uint16))
        display_id = f"{identity_prefix}:display"
        result["artifacts"].append({"id": display_id, "path": preview.name,
                                    "sha256": hashlib.sha256(preview.read_bytes()).hexdigest(),
                                    "media_type": "image/png", "role": "reference_display", "bit_depth": 16,
                                    "quantization": "16-bit rounded; no dither",
                                    "width": source_reference.dimensions[0], "height": source_reference.dimensions[1],
                                    "redistribution": reference["provenance"].get("redistribution", "unknown"),
                                    "provenance": reference["provenance"],
                                    "reference_id": reference["id"], "color_available": not bool(color_reason)})
        review["display_artifact_id"] = display_id
        review["artifact_refs"].append(display_id)
        if reference["scope"] == "visual":
            review["limitations"].append("Visual reference only; no numerical fidelity or exact scene correspondence claim.")
            return result
        images = [load_image(baseline), load_image(candidate)]
        if images[0].dimensions != images[1].dimensions:
            raise ImageDataError("dimension_mismatch", "Baseline and candidate geometry differs")
        if not all(np.all(np.isfinite(image.samples)) for image in images):
            raise ImageDataError("nonfinite_samples", "Rendered evidence contains nonfinite samples")
        if color_reason:
            raise ImageDataError(color_reason, "Reference color interpretation is unavailable")
        labs = []
        for image in images:
            rendered = display_rgb(image)
            if not np.all(np.isfinite(rendered)):
                raise ImageDataError("computation_failure", "Rendered display conversion contains nonfinite samples")
            labs.append(rgb_to_lab(np.clip(rendered, 0, 1)))
        tone, chroma = _processing_eligibility(reference)
        review.update(tone_target_eligible=tone, color_diagnostic_eligible=chroma)
        if not tone:
            review["limitations"].append("Grading/histogram history prevents an unmodified tone or full DE00 fidelity objective.")
        if reference["scope"] == "aligned_scene":
            _aligned(reference, source_reference, encoded, labs, images[0].dimensions, case, suite_base,
                     tone, chroma, distances, artifact, review)
        elif reference["scope"] == "regional":
            _regional(reference, rgb_to_lab(np.clip(encoded, 0, 1)), labs, images[0].dimensions,
                      case, suite_base, tone, chroma, distances, artifact, review)
        else:
            raise ImageDataError("unsupported_reference_scope", "Unknown reference scope")
    except (ImageDataError, OSError, ValueError, TypeError, KeyError, np.linalg.LinAlgError, ImageCms.PyCMSError) as exc:
        reason = exc.code if isinstance(exc, ImageDataError) else "invalid_reference_evidence"
        failure = reason in ("nonfinite_samples", "computation_failure")
        finding("full_image", "reference", reason, failure=failure)
        review["unavailable_reason"] = reason
        review["limitations"].append(str(exc))
        existing = {row["region_id"] for row in result["measurements"]}
        if reference["scope"] == "aligned_scene":
            for region in ["full_image", *(entry["id"] for entry in case.get("regions", []))]:
                if region not in existing:
                    image_specs(region, reason)
        elif reference["scope"] == "regional":
            for region in [entry["id"] for entry in reference.get("regions", [])] or ["full_image"]:
                if region not in existing:
                    image_specs(region, reason, regional=True)
        elif reference["scope"] == "material":
            for curve in case.get("probe_response", {}).get("curves", []) or [{"id": "response"}]:
                metric = "reference.material." + curve["id"]
                if not any(row["metric"].startswith(metric + ".") for row in result["measurements"]):
                    distances("full_image", metric, reference.get("domain", "unknown"), reference.get("units", "unknown"),
                              "matched coordinate RMSE", 0, reason=reason)
    return result


def _aligned(reference, source, encoded, labs, dimensions, case, base, tone, chroma, distances, artifact, review):
    if bool(reference.get("transform")) == bool(reference.get("aligned_derivative")):
        raise ImageDataError("invalid_correspondence", "Provide exactly one frozen transform or aligned derivative")
    if not reference.get("correspondence_review") or not reference.get("common_area_mask"):
        raise ImageDataError("invalid_correspondence", "Pixel comparison requires reviewed correspondence and common-area mask")
    record = _json(reference["correspondence_review"], base)
    if record.get("status") != "approved" or not isinstance(record.get("reviewer"), str) or not record["reviewer"].strip():
        raise ImageDataError("invalid_correspondence", "Correspondence must be approved by an identified reviewer")
    if not isinstance(record.get("excluded_regions"), list) or not isinstance(record.get("limitations"), list):
        raise ImageDataError("invalid_correspondence", "Review must record exclusions and limitations explicitly")
    if reference.get("aligned_derivative"):
        declaration = dict(reference)
        declaration["path"] = reference["aligned_derivative"]["path"]
        aligned = _read_reference(declaration, base)
        if aligned.dimensions != dimensions:
            raise ImageDataError("invalid_correspondence", "Aligned derivative has incorrect geometry")
        encoded = display_rgb(aligned)
        review["aligned_display_clipped_sample_fraction"] = float(np.mean((encoded < 0) | (encoded > 1)))
        valid_support = np.ones((dimensions[1], dimensions[0]), dtype=bool)
    else:
        encoded, valid_support = _warp(encoded, _json(reference["transform"], base), dimensions)
    if not np.all(np.isfinite(encoded)):
        raise ImageDataError("computation_failure", "Aligned reference has nonfinite samples")
    valid = _mask(reference["common_area_mask"], dimensions, base)
    if np.any(valid & ~valid_support):
        raise ImageDataError("invalid_correspondence", "Common-area mask includes unsupported transform borders")
    for exclusion in record["excluded_regions"]:
        if exclusion.get("reason") not in ("motion", "occlusion", "parallax", "invalid_border"):
            raise ImageDataError("invalid_correspondence", "Unsupported correspondence exclusion reason")
        excluded = _mask(exclusion["geometry"], dimensions, base)
        if np.any(valid & excluded):
            raise ImageDataError("invalid_correspondence", "Common-area mask retains reviewed invalid correspondence")
    if not np.any(valid):
        raise ImageDataError("invalid_correspondence", "Common-area mask has no eligible pixels")
    ref_lab = rgb_to_lab(np.clip(encoded, 0, 1))
    aligned_display_id = artifact(encoded, "reference_aligned_display", display=True, domain="bounded_encoded_srgb")
    review["display_artifact_id"] = aligned_display_id
    review["artifact_refs"].append(aligned_display_id)
    reference_id = artifact(encoded, "reference_aligned", domain="unclipped_encoded_srgb")
    valid_id = artifact(valid, "reference_common_area_mask", domain="render_pixel_coordinates")
    review.update(geometry="pixel_aligned", aligned_artifact_id=reference_id,
                  common_area_artifact_id=valid_id, correspondence_review=record)
    review["artifact_refs"].extend([reference_id, valid_id])
    review["limitations"].extend(record["limitations"])
    maps = [(ciede2000(lab, ref_lab), np.linalg.norm(lab[..., 1:] - ref_lab[..., 1:], axis=-1)) for lab in labs]
    if any(not np.all(np.isfinite(value)) for pair in maps for value in pair):
        raise ImageDataError("computation_failure", "Reference distance maps contain nonfinite samples")
    map_ids = [artifact(np.stack([pair[0], pair[1], lab[..., 0], ref_lab[..., 0]], axis=-1),
                        "reference_" + side + "_distance_maps", domain="bounded_srgb_d65_lab",
                        units="DE00; Lab a*/b* distance; rendered L*; reference L*")
               for side, pair, lab in zip(("baseline", "candidate"), maps, labs)]
    for region in [{"id": "full_image"}, *case.get("regions", [])]:
        population = valid if region["id"] == "full_image" else valid & _mask(region["geometry"], dimensions, base)
        count = int(np.count_nonzero(population))
        refs = [reference_id, valid_id, *map_ids]
        missing = "empty_reference_population" if not count else None
        for name, q in (("median", 50), ("p95", 95), ("p99", 99), ("max", 100)):
            values = [np.percentile(pair[0][population], q, method="linear") for pair in maps] if count and tone else None
            distances(region["id"], "reference.delta_e00." + name, "bounded_srgb_d65_lab", "delta E 00",
                      "pixel quantile method=linear; common-area mask", count, values,
                      missing or (None if tone else "tone_contaminated_reference"), refs)
        for q in (5, 25, 50, 75, 95):
            values = [abs(np.percentile(lab[..., 0][population], q, method="linear")
                          - np.percentile(ref_lab[..., 0][population], q, method="linear")) for lab in labs] if count and tone else None
            distances(region["id"], f"reference.tone_l_star.q{q:02d}", "bounded_srgb_d65_lab", "L*",
                      "absolute difference of fixed-population lightness quantiles", count, values,
                      missing or (None if tone else "tone_contaminated_reference"), refs)
        values = [np.median(pair[1][population]) for pair in maps] if count and chroma else None
        distances(region["id"], "reference.chroma_ab.median", "bounded_srgb_d65_lab", "Lab a*/b* distance",
                  "median pixel chromatic distance; excludes lightness; diagnostic only", count, values,
                  missing or (None if chroma else "unknown_or_graded_color_reference"), refs)


def _regional(reference, ref_lab, labs, dimensions, case, base, tone, chroma, distances, artifact, review):
    regions = {region["id"]: region for region in case.get("regions", [])}
    corresponding = reference.get("regions", [])
    if not corresponding:
        raise ImageDataError("missing_reference_regions", "Regional scope requires fixed corresponding regions")
    if len({row["id"] for row in corresponding}) != len(corresponding):
        raise ImageDataError("invalid_reference_regions", "Reference region identities must be unique")
    review["limitations"].append("Corresponding region statistics do not establish pixel correspondence or full-scene fidelity.")
    review["regions"] = []
    for region in corresponding:
        identity = region["id"]
        if identity not in regions:
            raise ImageDataError("invalid_reference_regions", "Reference region has no matching render region")
        render_mask = _mask(regions[identity]["geometry"], dimensions, base)
        ref_mask = _mask(region["geometry"], [ref_lab.shape[1], ref_lab.shape[0]], base)
        counts = [int(np.count_nonzero(render_mask)), int(np.count_nonzero(ref_mask))]
        if not all(counts):
            raise ImageDataError("empty_reference_population", "Corresponding region has no eligible samples")
        render_values, reference_values = [lab[render_mask] for lab in labs], ref_lab[ref_mask]
        # Independent populations: robust patch centers and marginal quantile
        # distances, never pairwise pixel subtraction or implicit resampling.
        centers = [np.median(values, axis=0) for values in [*render_values, reference_values]]
        quantiles = [np.percentile(values[:, 0], np.arange(101), method="linear") for values in [*render_values, reference_values]]
        payload = {"region_id": identity, "render_geometry": regions[identity]["geometry"],
                   "reference_geometry": region["geometry"], "render_sample_count": counts[0], "reference_sample_count": counts[1],
                   "lab_median": {side: center.tolist() for side, center in zip(("baseline", "candidate", "reference"), centers)},
                   "lightness_quantile_probabilities": (np.arange(101) / 100).tolist(),
                   "lightness_quantiles": {side: curve.tolist() for side, curve in zip(("baseline", "candidate", "reference"), quantiles)},
                   "domain": "bounded_srgb_d65_lab", "units": "L*,a*,b*"}
        evidence = artifact(None, "reference_regional_statistics", json_value=payload)
        review["regions"].append(payload)
        review["artifact_refs"].append(evidence)
        for metric, units, estimator, eligible, values in (
            ("reference.patch_delta_e00", "delta E 00", "DE00 between independent Lab channel medians; robust interior patch", tone,
             [float(ciede2000(center, centers[2])) for center in centers[:2]]),
            ("reference.distribution_l_star", "L*", "mean absolute distance of 101 independent marginal lightness quantiles; linear quantiles", tone,
             [float(np.mean(np.abs(curve - quantiles[2]))) for curve in quantiles[:2]]),
            ("reference.patch_chroma_ab", "Lab a*/b* distance", "distance between independent median a*/b*; excludes lightness; diagnostic only", chroma,
             [float(np.linalg.norm(center[1:] - centers[2][1:])) for center in centers[:2]])):
            distances(identity, metric, "bounded_srgb_d65_lab", units, estimator, counts[0], values if eligible else None,
                      None if eligible else ("tone_contaminated_reference" if tone is False and metric != "reference.patch_chroma_ab" else "unknown_or_graded_color_reference"), [evidence])


def _material(reference, baseline, candidate, case, base, distances, artifact, review, finding):
    source = _json(reference["path"], base)
    source_id = artifact(None, "reference_material_source", json_value=source)
    review["artifact_refs"].append(source_id)
    review["master_artifact_id"] = source_id
    response = case.get("probe_response")
    if not isinstance(response, dict):
        raise ImageDataError("missing_probe_response", "Material evidence requires a declared matching model probe")
    if source.get("schema_version") != 1:
        raise ImageDataError("invalid_material_measurement", "Material measurement must use schema_version 1")
    for field in ("domain", "units", "instrument", "conditions"):
        if not reference.get(field) or source.get(field) != reference[field] or response.get(field) != reference[field]:
            raise ImageDataError("incompatible_material_conditions", "Source and model response must agree on " + field)
        if str(reference[field]).lower() == "unknown":
            raise ImageDataError("incompatible_material_conditions", "Material measurement conditions must be known")
    coordinate = source.get("coordinate")
    if (not isinstance(coordinate, dict) or not coordinate.get("name") or not coordinate.get("units")
            or response.get("coordinate") != coordinate):
        raise ImageDataError("incompatible_material_coordinates", "Material coordinate identity and units must match")
    source_curves = source.get("curves", [])
    response_curves = response.get("curves", [])
    if not source_curves or not response_curves:
        raise ImageDataError("missing_material_curves", "Source and model response require curves")
    if len({curve["id"] for curve in source_curves}) != len(source_curves) or len({curve["id"] for curve in response_curves}) != len(response_curves):
        raise ImageDataError("invalid_material_measurement", "Curve identities must be unique")
    images = [load_image(baseline), load_image(candidate)]
    if images[0].dimensions != images[1].dimensions:
        raise ImageDataError("dimension_mismatch", "Probe dimensions differ")
    if not all(np.all(np.isfinite(image.samples)) for image in images):
        raise ImageDataError("nonfinite_samples", "Material probe contains nonfinite samples")
    estimator = response.get("estimator", "")
    allowed = ("native_channel_mean", "native_channel_median", "lab_channel_mean", "lab_channel_median",
               "linear_luminance_mean", "linear_luminance_median")
    if estimator not in allowed:
        raise ImageDataError("invalid_probe_response", "Unknown model response estimator")
    if estimator.startswith("native_channel"):
        if reference["domain"] == "native_rgb" and reference["units"] == "native channel value":
            if (images[0].color_space, images[0].transfer) != (images[1].color_space, images[1].transfer):
                raise ImageDataError("incompatible_material_conditions", "Native RGB probe declarations differ")
            review["limitations"].append("Native RGB response is a rendered diagnostic, not density, spectral response, or physical sensitometry.")
        else:
            for image in images:
                declaration = image.metadata.get("material_measurement", {})
                if any(declaration.get(field) != reference[field] for field in ("domain", "units", "instrument", "conditions")):
                    raise ImageDataError("incompatible_material_conditions", "Native probe lacks matching measurement-domain/instrument declaration; RGB is not a density proxy")
        arrays = [image.samples for image in images]
        channels = {"R": 0, "G": 1, "B": 2}
    elif estimator.startswith("lab_channel"):
        if reference["domain"] != "bounded_srgb_d65_lab" or reference["units"] not in ("L*", "a*", "b*"):
            raise ImageDataError("incompatible_material_conditions", "Lab response requires declared D65 Lab domain and channel units")
        arrays = [rgb_to_lab(np.clip(display_rgb(image), 0, 1)) for image in images]
        channels = {"L": 0, "a": 1, "b": 2}
    else:
        if reference["domain"] != "linear_srgb_relative_luminance" or reference["units"] != "relative luminance":
            raise ImageDataError("incompatible_material_conditions", "Luminance response must declare relative linear sRGB luminance")
        arrays = [(srgb_decode(display_rgb(image)) @ np.array([.2126, .7152, .0722]))[..., None] for image in images]
        channels = {"Y": 0}
    if not all(np.all(np.isfinite(array)) for array in arrays):
        raise ImageDataError("computation_failure", "Material domain conversion is nonfinite")
    region_map = {region["id"]: region for region in case.get("regions", [])}
    source_map = {curve["id"]: curve for curve in source_curves}
    review.update(geometry="measurement_curves", domain=reference["domain"], units=reference["units"],
                  instrument=reference["instrument"], conditions=reference["conditions"], coordinate=coordinate)
    review["limitations"].append("This objective compares declared model probe response to measured curves under matching conditions.")
    reduce = np.median if estimator.endswith("median") else np.mean
    for curve in response_curves:
        identity = curve["id"]
        if not isinstance(identity, str) or not re.fullmatch(r"[A-Za-z0-9_-]+", identity):
            raise ImageDataError("invalid_material_measurement", "Curve identity must be a stable metric token")
        expected = source_map.get(identity)
        if expected is None or expected.get("channel") != curve.get("channel") or curve["channel"] not in channels:
            raise ImageDataError("incompatible_material_curves", "Source/model curve identity and channel must match")
        if estimator.startswith("lab_channel") and curve["channel"] + "*" != reference["units"]:
            raise ImageDataError("incompatible_material_conditions", "Lab response channel does not match declared units")
        coordinates = np.asarray(expected.get("coordinates"), dtype=np.float64)
        values = np.asarray(expected.get("values"), dtype=np.float64)
        samples = curve.get("samples", [])
        if (coordinates.ndim != 1 or coordinates.size < 2 or values.shape != coordinates.shape
                or not np.all(np.isfinite(coordinates)) or not np.all(np.isfinite(values))
                or not np.all(np.diff(coordinates) > 0)):
            raise ImageDataError("invalid_material_measurement", "Curve requires finite, strictly increasing coordinates and matching values")
        declared = np.asarray([sample["coordinate"] for sample in samples], dtype=np.float64)
        if declared.shape != coordinates.shape or not np.array_equal(declared, coordinates):
            raise ImageDataError("incompatible_material_coordinates", "Probe coordinates must exactly match frozen source coordinates")
        measured = [[], []]
        pixel_counts = []
        for sample in samples:
            region_id = sample["region_id"]
            if region_id == "full_image":
                mask = np.ones((images[0].dimensions[1], images[0].dimensions[0]), dtype=bool)
            elif region_id in region_map:
                mask = _mask(region_map[region_id]["geometry"], images[0].dimensions, base)
            else:
                raise ImageDataError("invalid_probe_response", "Probe sample names a missing region")
            count = int(np.count_nonzero(mask))
            if not count:
                raise ImageDataError("empty_reference_population", "Probe region contains no samples")
            pixel_counts.append(count)
            for index, array in enumerate(arrays):
                measured[index].append(float(reduce(array[..., channels[curve["channel"]]][mask])))
        measured = np.asarray(measured)
        residual = measured - values
        if not np.all(np.isfinite(measured)) or not np.all(np.isfinite(residual)):
            raise ImageDataError("computation_failure", "Material response or residual estimation is nonfinite")
        payload = {"curve_id": identity, "coordinates": coordinates.tolist(), "coordinate": coordinate,
                   "reference": values.tolist(), "baseline": measured[0].tolist(), "candidate": measured[1].tolist(),
                   "baseline_residual": residual[0].tolist(), "candidate_residual": residual[1].tolist(),
                   "domain": reference["domain"], "units": reference["units"], "instrument": reference["instrument"],
                   "conditions": reference["conditions"], "estimator": estimator, "region_pixel_counts": pixel_counts,
                   "regions": [sample["region_id"] for sample in samples]}
        evidence = artifact(None, "reference_material_curve", json_value=payload)
        review["artifact_refs"].append(evidence)
        distances("full_image", "reference.material." + identity, reference["domain"], reference["units"],
                  "RMSE over matched source coordinates; probe " + estimator, coordinates.size,
                  [np.sqrt(np.mean((side - values) ** 2)) for side in measured], refs=[evidence])
