"""Build a portable offline review from retained comparison evidence only."""
from __future__ import annotations

import hashlib
import html
import json
from pathlib import Path, PurePosixPath
from urllib.parse import quote

try:
    from .report_assets import CSS, JS
    from .documents import DocumentError, validate_generated_document, safe_artifact_path, load_json_document
except ImportError:
    from report_assets import CSS, JS
    from documents import DocumentError, validate_generated_document, safe_artifact_path, load_json_document

def _error(code, message):
    return DocumentError(code, message)


def _text(value):
    if isinstance(value, (dict, list)):
        value = json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False)
    return html.escape(str(value), quote=True)


def _json(path):
    return load_json_document(path)


def _hash(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _safe_path(root, value):
    return safe_artifact_path(root, value)


class _Artifacts:
    def __init__(self, root, out, artifacts):
        self.root, self.out = root, out
        if isinstance(artifacts, dict):
            artifacts = [{"id": key, **value} if isinstance(value, dict) else {"id": key, "path": value}
                         for key, value in artifacts.items()]
        if not isinstance(artifacts, list):
            raise _error("invalid_document", "artifacts must be an array or object")
        self.records = artifacts
        self.index = {}
        self.manifest = []
        self.copied = {}
        for artifact in artifacts:
            if not isinstance(artifact, dict) or "path" not in artifact:
                raise _error("invalid_document", "artifact must identify a path")
            path = artifact["path"]
            _safe_path(root, path)
            existing = self.index.get(path)
            if existing is not None:
                if artifact.get("sha256") and existing.get("sha256") and artifact["sha256"] != existing["sha256"]:
                    raise _error("artifact_hash", f"conflicting hashes for artifact path: {path}")
                if "id" in artifact:
                    artifact_id = str(artifact["id"])
                    if artifact_id in self.index and self.index[artifact_id] is not existing:
                        raise _error("invalid_document", f"duplicate artifact id: {artifact_id}")
                    self.index[artifact_id] = existing
                continue
            self.index[path] = artifact
            self.records.append(artifact) if artifact not in self.records else None
            if "id" in artifact:
                artifact_id = str(artifact["id"])
                if artifact_id in self.index:
                    raise _error("invalid_document", f"duplicate artifact id: {artifact_id}")
                self.index[artifact_id] = artifact

    def record(self, reference):
        if isinstance(reference, str):
            return self.index.get(reference, {"path": reference})
        if isinstance(reference, dict):
            if "path" in reference:
                return {**self.index.get(reference["path"], {}), **reference}
            if "id" in reference:
                return self.index.get(str(reference["id"]))
        return None

    def copy(self, reference):
        record = self.record(reference)
        if not record:
            return None
        source_path = record["path"]
        source = _safe_path(self.root, source_path)
        if source_path in self.copied:
            return self.copied[source_path]
        entry = {**record, "source_path": source_path}
        if "redistribution" in record and record["redistribution"] not in (True, "permitted", "allowed", "CC0", "public_domain"):
            entry.update(path=None, availability="unavailable", reason="redistribution_not_permitted")
        elif not source.is_file():
            entry.update(path=None, availability="unavailable", reason="missing_artifact")
        else:
            digest = _hash(source)
            if record.get("sha256") and record["sha256"] != digest:
                raise _error("artifact_hash", f"artifact hash mismatch: {source_path}")
            destination = source_path
            if destination == "comparison.json":
                destination = f"evidence/{source_path}"
            destination_path = self.out / destination
            destination_path.parent.mkdir(parents=True, exist_ok=True)
            data = source.read_bytes()
            if hashlib.sha256(data).hexdigest() != digest:
                raise _error("artifact_hash", f"artifact changed while copying: {source_path}")
            destination_path.write_bytes(data)
            entry.update(id=entry.get("id", f"artifact-{len(self.manifest):06d}"), path=destination,
                         sha256=digest, availability="available")
        self.manifest.append(entry)
        self.copied[source_path] = entry
        return entry

    def image(self, reference):
        entry = self.copy(reference)
        if not entry or entry["availability"] != "available":
            return None
        suffix = Path(entry["path"]).suffix
        if suffix == ".npy" and entry.get("role") == "difference_map":
            import numpy as np
            from PIL import Image
            values = np.load(self.out / entry["path"], allow_pickle=False)
            if values.ndim != 2 or not np.issubdtype(values.dtype, np.number) or not np.all(np.isfinite(values)):
                raise _error("invalid_artifact", "difference map must contain finite numeric two-dimensional samples")
            signed = entry.get("metric") not in ("delta_e00", "flip")
            low, high = (-10.0, 10.0) if signed else (0.0, 1.0 if entry.get("metric") == "flip" else 10.0)
            bounded = np.clip(values, low, high)
            if signed:
                intensity = np.abs(bounded) / high
                rgb = np.ones((*values.shape, 3), dtype=float)
                rgb[..., 0] -= intensity * (bounded < 0)
                rgb[..., 2] -= intensity * (bounded > 0)
                rgb[..., 1] -= intensity
            else:
                intensity = bounded / high
                rgb = np.stack((intensity, intensity, intensity), axis=-1)
            path = f"artifacts/{len(self.manifest):06d}-map.png"
            Image.fromarray(np.rint(rgb * 255).astype(np.uint8)).save(self.out / path)
            display = {**entry, "path": path, "sha256": _hash(self.out / path), "media_type": "image/png",
                       "source_sha256": entry["sha256"], "role": "difference_display", "scale": [low, high],
                       "clipping": [low, high], "amplification": 1,
                       "palette": "negative blue / zero white / positive red" if signed else f"black zero / white {high:g}"}
            self.manifest.append(display)
            return display
        if suffix not in (".png", ".jpg", ".jpeg", ".gif", ".webp"):
            return None
        return entry

    def crop(self, entry, rect):
        if not entry:
            return None
        from PIL import Image
        with Image.open(self.out / entry["path"]) as image:
            x, y, width, height = rect
            if x + width > image.width or y + height > image.height:
                raise _error("region_bounds", f"region {rect} outside retained full image")
            path = f"artifacts/{len(self.manifest):06d}-detail.png"
            if entry.get("bit_depth") == 16:
                import png16
                png16.crop(self.out / entry["path"], self.out / path, rect)
            else:
                image.crop((x, y, x + width, y + height)).save(self.out / path, format="PNG")
            result = {"path": path, "sha256": _hash(self.out / path), "media_type": "image/png",
                      "role": "detail", "availability": "available", "dimensions": [width, height],
                      "source_path": entry["source_path"], "source_sha256": entry["sha256"],
                      "rectangle": rect, "method": "crop_completed_full_image", "resampling": "none",
                      "bit_depth": entry.get("bit_depth", 8)}
            self.manifest.append(result)
            return result

    def plot(self, reference, label):
        """Render retained numeric evidence as a presentation SVG without recomputing it."""
        entry = self.copy(reference)
        if not entry or entry.get("availability") != "available":
            return None
        path = self.out / entry["path"]
        try:
            if path.suffix == ".npy":
                import numpy as np
                raw = np.load(path, allow_pickle=False)
                if raw.ndim == 1:
                    series = [(list(range(raw.size)), raw.tolist(), label)]
                elif raw.ndim == 2 and raw.shape[1] >= 2:
                    columns = entry.get("columns", [])
                    series = [(raw[:, 0].tolist(), raw[:, i].tolist(), columns[i] if i < len(columns) else f"{label} {i}") for i in range(1, raw.shape[1]) if i >= len(columns) or columns[i] not in ("count", "pixel_count", "bin_count")]
                else:
                    return None
            elif path.suffix == ".json":
                value = _json(path)
                if isinstance(value, dict):
                    if entry.get("role") == "reference_regional_statistics":
                        x = value["lightness_quantile_probabilities"]
                        series = [(x, curve, side) for side, curve in value["lightness_quantiles"].items()]
                    elif entry.get("role") == "reference_material_curve":
                        x = value["coordinates"]
                        series = [(x, value[side], side) for side in ("baseline", "candidate", "reference")]
                    else:
                        x = value.get("x", value.get("coordinates", value.get("exposure", [])))
                        keys = [k for k in value if k not in ("x", "coordinates", "exposure", "units", "x_units", "y_units")]
                        series = [(x, value[k], str(k)) for k in keys if isinstance(value[k], list)]
                elif isinstance(value, list) and value and all(isinstance(row, list) and len(row) >= 2 for row in value):
                    series = [([row[0] for row in value], [row[1] for row in value], label)]
                else:
                    return None
            else:
                return None
            series = [(list(map(float, xs)), list(map(float, ys)), name) for xs, ys, name in series]
            points = [(x, y) for xs, ys, _ in series for x, y in zip(xs, ys) if __import__("math").isfinite(x) and __import__("math").isfinite(y)]
            if not points:
                return None
            xmin, xmax = min(x for x, _ in points), max(x for x, _ in points)
            ymin, ymax = min(y for _, y in points), max(y for _, y in points)
            if xmin == xmax: xmax = xmin + 1
            if ymin == ymax: ymax = ymin + 1
            colors = ("#8dccff", "#ffca69", "#ff8d8d", "#9ee493")
            paths = []
            for index, (xs, ys, name) in enumerate(series):
                coords = [(40 + 520 * (x - xmin) / (xmax - xmin), 220 - 180 * (y - ymin) / (ymax - ymin)) for x, y in zip(xs, ys) if xmin <= x <= xmax and ymin <= y <= ymax]
                if coords:
                    paths.append(f'<polyline fill="none" stroke="{colors[index % len(colors)]}" points="{" ".join(f"{x:.2f},{y:.2f}" for x,y in coords)}"/><text x="45" y="{35 + index*16}" fill="{colors[index % len(colors)]}">{html.escape(name)}</text>')
            columns = entry.get("columns", [])
            units = f'{columns[0] if columns else entry.get("x_units", "sample coordinate")} / {entry.get("units", entry.get("domain", "declared response"))}'
            return f'<svg class="evidence-plot" viewBox="0 0 600 260" role="img" aria-label="{_text(label)}"><rect x="0" y="0" width="600" height="260" fill="#090b10"/><path d="M40 220H560M40 20V220" stroke="#8794aa"/><text x="45" y="250" fill="#b9c6d8">{_text(units)} · x [{xmin:g},{xmax:g}] · y [{ymin:g},{ymax:g}]</text>{"".join(paths)}</svg>'
        except (ValueError, TypeError, OSError, OverflowError):
            return None
def _url(entry):
    return html.escape(quote(entry["path"], safe="/"), quote=True)


def _image(entry, label):
    if not entry:
        return f'<div class="unavailable">{_text(label)}: unavailable</div>'
    return f'<figure><img src="{_url(entry.get("fit_entry") or entry)}" alt="{_text(label)}" loading="lazy"><figcaption>{_text(label)} · <a href="{_url(entry)}">individual image</a></figcaption></figure>'


def _viewer(baseline, candidate, detail=False):
    if not baseline or not candidate:
        return '<div class="grid">' + _image(baseline, "Baseline") + _image(candidate, "Candidate") + '</div>'
    left, right = _url(baseline), _url(candidate)
    left_fit = _url(baseline.get("fit_entry") or baseline)
    right_fit = _url(candidate.get("fit_entry") or candidate)
    left_attrs = f'data-native="{left}" data-fit="{left_fit}"'
    right_attrs = f'data-native="{right}" data-fit="{right_fit}"'
    default = '1' if detail else 'fit'
    loading = ' loading="lazy"'
    options = ''.join(f'<option value="{value}"{" selected" if value == default else ""}>{label}</option>'
                      for value, label in [('fit', 'Fit'), ('1', '100%'), ('2', '200%'), ('4', '400%')])
    return f'''<div class="viewer" data-mode="split"><div class="controls">
<label>查看 <select data-mode><option value="split">分割对照</option><option value="side">左右对照</option><option value="baseline">基准</option><option value="candidate">候选</option></select></label>
<button type="button" data-alternate>切换 A/B</button><label>倍率 <select data-zoom>{options}</select></label>
<label>分割 <input type="range" min="0" max="100" value="50" data-split aria-label="A/B split position"></label>
<span data-scale-label class="small"></span></div><div class="panes">
<div class="side-pane"><p>基准 Baseline</p><div class="viewport" tabindex="0" aria-label="Baseline pan area"><div class="canvas"><img src="{left if detail else left_fit}" {left_attrs} alt="Baseline"{loading}></div></div></div>
<div class="side-pane"><p>候选 Candidate</p><div class="viewport" tabindex="0" aria-label="Candidate pan area"><div class="canvas"><img src="{right if detail else right_fit}" {right_attrs} alt="Candidate"{loading}></div></div></div>
<div class="split-pane"><p>基准在左 / 候选在右 · 拖动分割线</p><div class="viewport" tabindex="0" aria-label="Split pan area"><div class="canvas split"><img src="{left if detail else left_fit}" {left_attrs} alt="Baseline" loading="lazy"><img class="candidate" src="{right if detail else right_fit}" {right_attrs} alt="Candidate" loading="lazy"><button type="button" class="split-handle" data-split-handle role="slider" aria-label="拖动分割线" aria-valuemin="0" aria-valuemax="100" aria-valuenow="50" style="left:50%">↔</button></div></div></div>
</div><p><a href="{left}">Baseline image</a> · <a href="{right}">Candidate image</a>{' · 16-bit PNG，无抖动；屏幕显示精度由浏览器与显示设备决定。' if baseline.get('bit_depth') == 16 else ''}</p></div>'''


def _rectangle(region):
    geometry = region.get("geometry", {})
    if isinstance(geometry, list):
        rect = geometry
    elif isinstance(geometry, dict):
        rect = region.get("rectangle", region.get("rect", geometry.get("rectangle", geometry.get("rect"))))
    else:
        rect = region.get("rectangle", region.get("rect"))
    if rect is None:
        return None
    if not isinstance(rect, (list, tuple)) or len(rect) != 4 or any(type(v) is not int for v in rect):
        raise _error("region_bounds", "detail rectangle must contain four integers")
    rect = list(rect)
    if rect[0] < 0 or rect[1] < 0 or rect[2] <= 0 or rect[3] <= 0:
        raise _error("region_bounds", "detail rectangle must have nonnegative origin and positive size")
    return rect


def _locator(entry, rect):
    if not entry or not rect:
        return '<div class="unavailable">Full-image locator: unavailable</div>'
    from PIL import Image
    with Image.open(entry["local_path"]) as image:
        width, height = image.size
    x, y, w, h = rect
    return f'<div class="locator"><img src="{_url(entry)}" alt="Full-image region locator"><span class="rectangle" style="left:{100*x/width}%;top:{100*y/height}%;width:{100*w/width}%;height:{100*h/height}%"></span></div>'


def _role_reference(case, role, assets):
    names = {"baseline": ("baseline_review", "baseline_image", "baseline"),
             "candidate": ("candidate_review", "candidate_image", "candidate"),
             "source": ("source_review", "source_image", "source"),
             "reference": ("reference_review", "reference_image", "reference"),
             "difference": ("difference_map", "difference_image", "difference", "delta_e00_map")}[role]
    if isinstance(case.get("review"), dict):
        value = case["review"].get(role)
        if isinstance(value, dict):
            value = value.get("display_artifact_id", value.get("artifact_id", value.get("path", value)))
        if value:
            return value
    case_artifacts = case.get("artifacts", {})
    for name in names:
        value = case_artifacts.get(name) if isinstance(case_artifacts, dict) else None
        value = value or case.get(name)
        if value and assets.record(value):
            return value
    records = case_artifacts if isinstance(case_artifacts, list) else []
    records = [assets.record(record) for record in records]
    records += [record for record in assets.records if record.get("case_id") == case.get("case_id", case.get("id"))
                and ("seed" not in record or record["seed"] == case.get("seed"))]
    for record in records:
        if record and record.get("role") in names:
            return record
    return None


def _metric_table(case):
    rows = case.get("metrics", case.get("measurements", []))
    if isinstance(rows, dict):
        rows = rows.get("measurements", [])
    if not isinstance(rows, list):
        return '<div class="unavailable">Metric table: unavailable</div>'
    body = []
    for row in rows:
        if not isinstance(row, dict):
            continue
        value = row.get("value", "unavailable") if row.get("availability", "available") == "available" else row.get("reason", "unavailable")
        body.append('<tr>' + ''.join(f'<td>{_text(row.get(key, "unavailable"))}</td>' for key in ("region_id", "metric", "units", "estimator", "sample_count")) + f'<td>{_text(value)}</td></tr>')
    if not body:
        return '<div class="unavailable">Metric table: unavailable</div>'
    return '<table><thead><tr><th>Region</th><th>Metric</th><th>Units</th><th>Estimator</th><th>Samples</th><th>Value/status</th></tr></thead><tbody>' + ''.join(body) + '</tbody></table>'


def _evidence_block(case):
    blocks = []
    for key, label in (("curves", "Retained response curves"), ("spectra", "Retained spectra")):
        if case.get(key):
            blocks.append(f'<h4>{_text(label)}</h4><pre>{_text(case[key])}</pre>')
    return ''.join(blocks)

def _human_name(identity):
    names = {"color_chart_30mp": "30 MP 色彩测试图", "color_chart": "色彩测试图",
             "kodak_dc50_scene": "自然场景 · Kodak DC50", "exposure_response": "灰阶响应",
             "grain_fields": "平场颗粒", "halation_point": "点光源与光晕", "edge_response": "边缘响应",
             "skin": "肤色", "shadow": "阴影", "texture": "纹理", "neutral": "中性灰"}
    return names.get(str(identity), str(identity).replace("_", " ").replace("-", " ").title())


def _short_value(value):
    if isinstance(value, float):
        return f"{value:.4g}"
    if isinstance(value, (dict, list)):
        text = json.dumps(value, ensure_ascii=False, allow_nan=False)
    else:
        text = str(value)
    return text if len(text) <= 64 else text[:61] + "…"


def _change_summary(case, comparison):
    changes = []

    def changed(left, right, path=""):
        if isinstance(left, dict) and isinstance(right, dict):
            for key in sorted(set(left) | set(right)):
                changed(left.get(key), right.get(key), f"{path}.{key}" if path else key)
        elif left != right:
            name = {"camera.exposure_compensation_ev": "曝光补偿（EV）"}.get(path, path)
            changes.append(f"{name}: {_short_value(left)} → {_short_value(right)}")

    for row in comparison.get("condition_differences", []):
        if row.get("case_id") not in (None, case.get("case_id", case.get("id"))):
            continue
        if "seed" in row and row["seed"] != case.get("seed"):
            continue
        field = row.get("field", "condition")
        if field == "resolved_parameters":
            changed(row.get("baseline"), row.get("candidate"))
        elif "baseline" in row and "candidate" in row:
            changes.append(f"{_human_name(field)}: {_short_value(row['baseline'])} → {_short_value(row['candidate'])}")
        else:
            changes.append(f"{_human_name(field)}: {_short_value(row.get('requested'))} → {_short_value(row.get('observed'))}")
    text = " · ".join(changes[:3]) if changes else "保留的比较条件未记录变化。"
    if len(changes) > 3:
        text += f" · 另 {len(changes) - 3} 项（见支持证据）"
    return '<p class="change-summary"><strong>基准 → 候选</strong> · ' + _text(text) + '</p>'


def _metric_summary(case):
    rows = case.get("metrics", case.get("measurements", []))
    if isinstance(rows, dict):
        rows = rows.get("measurements", [])
    if not isinstance(rows, list):
        return ''
    choices = [("delta_e00.p95", "色差 P95", "越小越接近基准"),
               ("flip.mean", "FLIP 平均差异", "越小越接近基准"),
               ("delta_l_star.mean", "平均亮度变化", "正值更亮；候选 − 基准"),
               ("numeric.rmse", "像素 RMS 差异", "越小越接近基准")]
    cards = []
    for metric, label, direction in choices:
        row = next((r for r in rows if isinstance(r, dict) and r.get("region_id") == "full_image"
                    and r.get("metric") == metric and r.get("availability", "available") == "available"
                    and type(r.get("value")) in (int, float)), None)
        if row:
            value = row["value"]
            rounded = f"{value:.3f}" if abs(value) >= .001 or value == 0 else f"{value:.2e}"
            cards.append(f'<div title="{_text(row.get("domain", "未声明域"))}"><span>{label}</span><strong>{rounded} <small>{_text(row.get("units", ""))}</small></strong><small>{direction}</small></div>')
    return '<div class="metric-summary" aria-label="全图差异摘要">' + ''.join(cards) + '</div>' if cards else ''


def build_report(comparison_dir, out_dir):
    """Publish index.html and report.json; refuse any existing destination.

    Review images and maps are copied, and lossless region crops are extracted
    from those completed images. Measurements and hotspot selection stay intact.
    """
    root, out = Path(comparison_dir).resolve(), Path(out_dir)
    if out.exists() or out.is_symlink():
        raise _error("destination_exists", f"report destination already exists: {out}")
    document_path = _safe_path(root, "comparison.json")
    comparison = _json(document_path)
    if not isinstance(comparison, dict) or comparison.get("schema_version") != 1:
        raise _error("unsupported_schema", "report requires comparison schema_version 1")
    validate_generated_document("comparison", comparison, root)
    if not isinstance(comparison.get("cases"), list):
        raise _error("invalid_document", "comparison must contain a cases array")
    out.mkdir(parents=True, exist_ok=False)
    retained = comparison.get("artifacts", [])
    if isinstance(retained, dict):
        retained = [{"id": key, **value} if isinstance(value, dict) else {"id": key, "path": value}
                    for key, value in retained.items()]
    if not isinstance(retained, list):
        raise _error("invalid_document", "artifacts must be an array or object")
    retained = list(retained)
    unique = {}
    for record in retained:
        if not isinstance(record, dict) or "path" not in record:
            raise _error("invalid_document", "artifact must identify a path")
        key = record["path"]
        prior = unique.get(key)
        if prior and prior.get("sha256") and record.get("sha256") and prior["sha256"] != record["sha256"]:
            raise _error("artifact_hash", f"conflicting duplicate artifact: {key}")
        unique.setdefault(key, record)
    retained = list(unique.values())
    (out / "artifacts").mkdir()
    assets = _Artifacts(root, out, retained)
    for record in assets.records:
        assets.copy(record)
    snapshot = document_path.read_bytes()
    (out / "comparison.json").write_bytes(snapshot)
    assets.manifest.append({"path": "comparison.json", "source_path": "comparison.json",
                            "sha256": hashlib.sha256(snapshot).hexdigest(), "role": "comparison",
                            "media_type": "application/json", "availability": "available"})
    cases = []
    for case in comparison["cases"]:
        if not isinstance(case, dict):
            raise _error("invalid_document", "case result must be an object")
        case_id = case.get("case_id", case.get("id", "unavailable"))
        seed = case.get("seed", "unavailable")
        cases.append((case_id, seed, case))
    ordered = sorted(cases, key=lambda item: (0 if item[0] == "color_chart_30mp" else 1, str(item[0]), str(item[1])))
    first_index = 0 if ordered else None
    parts = ['<!doctype html><html lang="zh-CN"><meta charset="utf-8">',
             '<meta name="viewport" content="width=device-width,initial-scale=1">',
             '<meta http-equiv="Content-Security-Policy" content="default-src \'none\'; img-src \'self\'; style-src \'unsafe-inline\'; script-src \'unsafe-inline\'; connect-src \'none\'; base-uri \'none\'; form-action \'none\'">',
             '<title>SpektraFilm benchmark review</title><style>' + CSS + '</style><body>',
             '<h1>SpektraFilm 人工审查</h1>',
             '<p>选择场景查看 A/B；细节、差异图和完整数据按需展开。</p>',
             '<nav class="case-nav" aria-label="案例与种子"><label>案例 / 种子 <select data-case-nav>']
    for index, (case_id, seed, case) in enumerate(ordered):
        status = case.get("status", "unknown")
        parts.append(f'<option value="case-{index}">{_text(_human_name(case_id))} · seed {_text(seed)} · {_text(status)}</option>')
    parts.append('</select></label><span class="small">所有案例和失败状态均可访问</span></nav>')
    summary = comparison.get("summary", {})
    parts.append('<p class="small">' + ' · '.join(f'{label}：{_text(summary.get(key, "未记录"))}' for key, label in (("selected_cases", "案例"), ("completed_results", "完成"), ("failed_results", "失败"))) + '</p>')
    parts.append('<details class="supporting"><summary>比较摘要与跨种子证据</summary><p>选中案例后，打开下方支持证据查看原始来源、协议、规则、聚合和 findings。</p>')
    for key, label in (("summary", "汇总"), ("selection", "选择"), ("protocol", "协议"), ("condition_differences", "条件变化"), ("rules", "规则"), ("baseline", "基准来源"), ("candidate", "候选来源")):
        parts.append(f'<details><summary>{label}</summary><pre>{_text(comparison.get(key, "不可用"))}</pre></details>')
    if comparison.get("seed_measurements"):
        parts.append('<details><summary>跨种子完整测量</summary>' + _metric_table({"measurements": comparison["seed_measurements"]}) + '</details>')
    parts.append('</details>')
    for index, (case_id, seed, case) in enumerate(ordered):
        images = {role: assets.image(_role_reference(case, role, assets)) for role in ("source", "baseline", "candidate", "reference", "difference")}
        baseline, candidate = images["baseline"], images["candidate"]
        for image in images.values():
            if image and image.get("fit_preview"):
                image["fit_entry"] = assets.image(image["fit_preview"])
        status = case.get("status", "unavailable")
        hidden = '' if index == first_index else ' hidden'
        parts.append(f'<section id="case-{index}" class="case-panel" data-case-panel="case-{index}"{hidden}><h2>{_text(_human_name(case_id))} <small>· seed {_text(seed)}</small></h2><p class="status">状态：{_text(status)}{(" · " + _text(case.get("reason"))) if case.get("reason") else ""}</p>')
        parts.append(_change_summary(case, comparison))
        if case.get("pixel_aligned", True) and baseline and candidate:
            from PIL import Image
            with Image.open(out / baseline["path"]) as a, Image.open(out / candidate["path"]) as b:
                aligned = a.size == b.size
            parts.append(_viewer(baseline, candidate) if aligned else '<p class="unavailable">图像尺寸不同，仅提供并列检查，未声明共同观察尺度。</p><div class="grid">' + _image(baseline, "基准") + _image(candidate, "候选") + '</div>')
        else:
            parts.append('<p class="unavailable">同步 A/B 不可用：证据缺失或图像未对齐。</p>')
            parts.append('<div class="grid">' + _image(baseline, "基准") + _image(candidate, "候选") + '</div>')
        parts.append(_metric_summary(case))
        regions = [(region, "固定语义区域") for region in case.get("regions", [])]
        regions += [(region, "自动差异区域") for region in case.get("automatic_regions", case.get("hotspots", []))]
        regions += [(region, region.get("selection_method", region.get("method", "保留的细节区域"))) for region in case.get("detail_regions", [])]
        if regions:
            parts.append('<details><summary>细节区域（坐标、定位和 A/B 裁剪）</summary>')
        for region, method in regions:
            mask_entry = None
            if isinstance(region.get("geometry"), dict) and "path" in region["geometry"]:
                mask_path = region["geometry"]["path"]
                if case.get("status") != "completed" and mask_path not in assets.index:
                    parts.append(f'<p class="unavailable">{_text(region.get("label", region.get("id")))}：渲染失败，区域掩膜证据不可用。</p>')
                    continue
                import numpy as np
                from PIL import Image
                mask_entry = assets.copy(mask_path)
                if not mask_entry or mask_entry.get("availability") != "available":
                    parts.append(f'<p class="unavailable">{_text(region.get("label", region.get("id")))}：区域掩膜证据不可用。</p>')
                    continue
                path = out / mask_entry["path"]
                if path.suffix == ".npy": mask = np.load(path, allow_pickle=False)
                else:
                    with Image.open(path) as image: mask = np.asarray(image)
                y, x = np.nonzero(mask)
                if not x.size: raise _error("region_bounds", "retained region mask is empty")
                region = {**region, "rectangle": [int(x.min()), int(y.min()), int(x.max()-x.min()+1), int(y.max()-y.min()+1)]}
            rect = _rectangle(region)
            region_id = region.get("id", region.get("region_id", "不可用"))
            parts.append(f'<details><summary>{_text(region.get("label", _human_name(region_id)))}</summary><p>坐标 [x, y, width, height]：{_text(rect or "不可用")} · 选择方式：{_text(method)}</p>')
            parts.append('<details><summary>选择依据与区域元数据</summary><pre>' + _text(region) + '</pre></details>')
            if mask_entry: parts.append(f'<p>测量使用保留掩膜（含排除像素）：<a href="{_url(mask_entry)}">区域掩膜</a></p>')
            loc = {**baseline, "local_path": out / baseline["path"]} if baseline else None
            parts.append(_locator(loc, rect))
            crops = []
            for role, image in (("baseline", baseline), ("candidate", candidate)):
                crop = assets.image(_role_reference(region, role, assets))
                if crop and Path(crop["path"]).suffix in (".jpg", ".jpeg"): crop = None
                crops.append(crop or (assets.crop(image, rect) if rect else None))
            parts.append(_viewer(*crops, detail=True))
            findings = [f for f in comparison.get("findings", []) if f.get("case_id") == case_id and f.get("region_id") == region_id]
            parts.append('<details><summary>该区域 findings</summary><pre>' + _text(findings) + '</pre></details></details>')
        if regions: parts.append('</details>')
        parts.append('<details><summary>来源输入与参考图</summary><div class="grid">' + ''.join(_image(images[role], {"source": "来源输入", "reference": "参考图"}[role]) for role in ("source", "reference") if images[role]) + '</div>' + ('<pre>' + _text(case.get("reference", "参考范围和来源限制不可用")) + '</pre>' if images["reference"] or case.get("reference") else '') + '</details>')
        parts.append('<details><summary>差异图与显示元数据</summary>' + _image(images["difference"], "差异图") + '<pre>' + _text({key: (images["difference"] or {}).get(key, "不可用") for key in ("metric", "units", "scale", "clipping", "amplification", "palette")}) + '</pre></details>')
        evidence_plots = []
        plot_roles = {"tone_response", "grain_radial_spectrum", "halation_profile", "halation_edge_spill_profile", "edge_response", "edge_sfr", "reference_material_curve", "reference_regional_statistics"}
        for artifact in case.get("artifacts", []):
            if artifact.get("role") not in plot_roles:
                continue
            plot = assets.plot(artifact, artifact.get("id", artifact["role"]))
            if plot:
                evidence_plots.append(f'<details><summary>{_text(_human_name(artifact.get("id", artifact["role"])))}</summary>{plot}<a href="{_url(assets.copy(artifact))}">保留的数值样本</a></details>')
        if evidence_plots:
            parts.append('<details><summary>曲线与频谱证据</summary>' + ''.join(evidence_plots) + '</details>')
        for role in ("baseline", "candidate"):
            resolved = assets.copy(case.get(role + "_resolved"))
            parts.append(f'<details><summary>{"基准" if role == "baseline" else "候选"}执行与渲染配置</summary><pre>{_text(case.get(role + "_execution", "不可用"))}</pre>' + (f'<a href="{_url(resolved)}">已解析配置</a>' if resolved else '') + '</details>')
        parts.append('<details><summary>完整测量（原样保留）</summary>' + _metric_table(case) + _evidence_block(case) + '</details>')
        parts.append('</section>')
    parts.append('<details class="supporting"><summary>全部 findings（原样保留）</summary><pre>' + _text(comparison.get("findings", [])) + '</pre></details>')
    parts.append('<p><a href="comparison.json">比较快照</a> · <a href="report.json">相对 artifact manifest</a></p><script>' + JS + '</script></body></html>')
    (out / "index.html").write_text('\n'.join(parts), encoding="utf-8")
    assets.manifest.append({"path": "index.html", "sha256": _hash(out / "index.html"), "role": "report", "media_type": "text/html", "availability": "available"})
    manifest = {"schema_version": 1, "command": "report", "execution_status": "completed",
                "comparison_id": comparison.get("id"), "artifact": "index.html", "artifacts": assets.manifest}
    (out / "report.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2, allow_nan=False) + '\n', encoding="utf-8")
    return manifest
