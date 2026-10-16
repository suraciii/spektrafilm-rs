//! Portable offline presentation of retained evidence; no rendering or measurement.
use crate::{
    common::{Error, Result},
    image_data::read_npy,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

use super::assets::*;
use super::escape::*;

pub(crate) fn image_html(entry: Option<&Value>, label: &str) -> String {
    match entry {
        None => format!(
            "<div class=\"unavailable\">{}: unavailable</div>",
            escape(label)
        ),
        Some(e) => format!(
            "<figure><img src=\"{}\" alt=\"{}\" loading=\"lazy\"><figcaption>{} · <a href=\"{}\">individual image</a></figcaption></figure>",
            url(e.get("fit_entry").filter(|v| !v.is_null()).unwrap_or(e)),
            escape(label),
            escape(label),
            url(e)
        ),
    }
}
pub(crate) fn viewer(
    a: Option<&Value>,
    b: Option<&Value>,
    detail: bool,
    labels: (&str, &str),
    aligned: bool,
) -> String {
    let (Some(a), Some(b)) = (a, b) else {
        return format!(
            "<div class=\"grid\">{}{}</div>",
            image_html(a, labels.0),
            image_html(b, labels.1)
        );
    };
    let left = url(a);
    let right = url(b);
    let left_fit = url(a.get("fit_entry").filter(|v| !v.is_null()).unwrap_or(a));
    let right_fit = url(b.get("fit_entry").filter(|v| !v.is_null()).unwrap_or(b));
    let attrs = |native: &str, fit: &str| format!("data-native=\"{native}\" data-fit=\"{fit}\"");
    let la = attrs(&left, &left_fit);
    let ra = attrs(&right, &right_fit);
    let ls = if detail { &left } else { &left_fit };
    let rs = if detail { &right } else { &right_fit };
    let options = [("fit", "Fit"), ("1", "100%"), ("2", "200%"), ("4", "400%")]
        .iter()
        .map(|(v, l)| {
            format!(
                "<option value=\"{v}\"{}>{l}</option>",
                if *v == if detail { "1" } else { "fit" } {
                    " selected"
                } else {
                    ""
                }
            )
        })
        .collect::<String>();
    format!(
        r#"<div class="viewer" data-mode="{mode}" data-aligned="{aligned}"><div class="controls"><label>查看 <select data-mode>{split_option}<option value="side">左右对照</option><option value="baseline">{left_label}</option><option value="candidate">{right_label}</option></select></label><button type="button" data-view-mode="baseline">A · {left_label}</button><button type="button" data-view-mode="candidate">B · {right_label}</button><button type="button" data-alternate>切换 A/B</button><label>倍率 <select data-zoom>{options}</select></label><button type="button" data-fullscreen>全屏</button><label {split_hidden}>分割 <input type="range" min="0" max="100" value="50" data-split aria-label="A/B split position"></label><span data-scale-label class="small"></span></div><div class="panes"><div class="side-pane" data-side="baseline"><p>{left_label}</p><div class="viewport" tabindex="0" aria-label="{left_label} pan area"><div class="canvas"><img src="{ls}" {la} alt="{left_label}" loading="lazy"></div></div></div><div class="side-pane" data-side="candidate"><p>{right_label}</p><div class="viewport" tabindex="0" aria-label="{right_label} pan area"><div class="canvas"><img src="{rs}" {ra} alt="{right_label}" loading="lazy"></div></div></div><div class="split-pane"><p>{left_label}在左 / {right_label}在右 · 拖动分割线</p><div class="viewport" tabindex="0" aria-label="Split pan area"><div class="canvas split"><img src="{ls}" {la} alt="{left_label}" loading="lazy"><img class="candidate" src="{rs}" {ra} alt="{right_label}" loading="lazy"><button type="button" class="split-handle" data-split-handle role="slider" aria-label="拖动分割线" aria-valuemin="0" aria-valuemax="100" aria-valuenow="50" style="left:50%">↔</button></div></div></div></div><p><a href="{left}">{left_label} image</a> · <a href="{right}">{right_label} image</a>{}</p></div>"#,
        if a["bit_depth"] == 16 {
            " · 16-bit PNG，无抖动；屏幕显示精度由浏览器与显示设备决定。"
        } else {
            ""
        },
        mode = if aligned { "split" } else { "side" },
        split_option = if aligned {
            "<option value=\"split\">分割对照</option>"
        } else {
            ""
        },
        split_hidden = if aligned { "" } else { "hidden" },
        left_label = escape(labels.0),
        right_label = escape(labels.1)
    )
}
pub(crate) fn difference_metadata(entry: Option<&Value>) -> Value {
    let mut metadata = json!({});
    for key in [
        "metric",
        "units",
        "scale",
        "clipping",
        "amplification",
        "palette",
    ] {
        metadata[key] = entry
            .and_then(|entry| entry.get(key))
            .cloned()
            .unwrap_or(json!("不可用"));
    }
    metadata
}

pub(crate) fn crop_thumbnail(
    region: &Value,
    method: &str,
    crops: [Option<&Value>; 2],
    labels: (&str, &str),
) -> String {
    let id = get(region, "id", get(region, "region_id", "不可用"));
    let mut previews = String::new();
    for (entry, label) in crops.into_iter().zip([labels.0, labels.1]) {
        if let Some(entry) = entry {
            let fit = entry
                .get("fit_entry")
                .filter(|value| !value.is_null())
                .unwrap_or(entry);
            let size = entry["dimensions"]
                .as_array()
                .filter(|size| size.len() == 2)
                .map(|size| format!("{} × {}", size[0], size[1]))
                .unwrap_or_else(|| "尺寸未记录".into());
            previews.push_str(&format!("<span class=\"crop-preview\"><img src=\"{}\" alt=\"{} crop\" loading=\"lazy\"><span class=\"crop-size\">{} · {}</span></span>", url(fit), escape(label), escape(label), escape(&size)));
        } else {
            previews.push_str(&format!(
                "<span class=\"crop-preview unavailable\">{} crop：不可用</span>",
                escape(label)
            ));
        }
    }
    let kind = if method == "自动差异" {
        "automatic"
    } else {
        "manual"
    };
    format!(
        "<button type=\"button\" class=\"crop-thumbnail\" data-crop-target=\"{}\" data-crop-kind=\"{kind}\" aria-pressed=\"false\"><span class=\"crop-kind crop-type\">{}</span><strong class=\"crop-label\">{}</strong><span class=\"crop-previews\">{previews}</span></button>",
        escape(id),
        escape(method),
        escape(get(region, "label", &human(id)))
    )
}

pub(crate) fn rectangle(region: &Value) -> Result<Option<[u32; 4]>> {
    let geometry = &region["geometry"];
    // A named geometry carries its span as its own fields; the array and
    // `rectangle` forms stay readable for retained records that use them.
    let mut named: Option<Value> = None;
    if let Some(object) = geometry.as_object()
        && let (Some(x), Some(y), Some(width), Some(height)) = (
            object.get("x").and_then(Value::as_u64),
            object.get("y").and_then(Value::as_u64),
            object.get("width").and_then(Value::as_u64),
            object.get("height").and_then(Value::as_u64),
        )
    {
        named = Some(json!([x, y, width, height]));
    }
    let rect = region
        .get("rectangle")
        .or_else(|| region.get("rect"))
        .or_else(|| {
            if geometry.is_array() {
                Some(geometry)
            } else {
                geometry.get("rectangle").or_else(|| geometry.get("rect"))
            }
        })
        .or(named.as_ref());
    let Some(rect) = rect else { return Ok(None) };
    let values = rect.as_array().filter(|v| v.len() == 4).ok_or_else(|| {
        Error::new(
            "region_bounds",
            "detail rectangle must contain four integers",
        )
    })?;
    let mut result = [0; 4];
    for (i, v) in values.iter().enumerate() {
        result[i] = v
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| {
                Error::new(
                    "region_bounds",
                    "detail rectangle must contain four nonnegative integers",
                )
            })?;
    }
    if result[2] == 0 || result[3] == 0 {
        return Err(Error::new(
            "region_bounds",
            "detail rectangle must have positive size",
        ));
    }
    Ok(Some(result))
}
pub(crate) fn metric_table(case: &Value) -> String {
    let mut body = String::new();
    for row in measurements(case) {
        if !row.is_object() {
            continue;
        }
        body.push_str("<tr>");
        for key in ["region_id", "metric", "units", "estimator", "sample_count"] {
            body.push_str(&format!("<td>{}</td>", field(row, key)));
        }
        body.push_str(&format!(
            "<td>{}</td></tr>",
            field(
                row,
                if get(row, "availability", "available") == "available" {
                    "value"
                } else {
                    "reason"
                }
            )
        ));
    }
    if body.is_empty() {
        "<div class=\"unavailable\">Metric table: unavailable</div>".into()
    } else {
        format!(
            "<div class=\"table-scroll\"><table><thead><tr><th>Region</th><th>Metric</th><th>Units</th><th>Estimator</th><th>Samples</th><th>Value/status</th></tr></thead><tbody>{body}</tbody></table></div>"
        )
    }
}
pub(crate) fn metric_summary(case: &Value, labels: (&str, &str)) -> String {
    let mut cards = String::new();
    for (metric, label, direction) in [
        (
            "delta_e00.p95",
            "色差 P95",
            format!("越小越接近{}", labels.0),
        ),
        (
            "flip.mean",
            "FLIP 平均差异",
            format!("越小越接近{}", labels.0),
        ),
        (
            "delta_l_star.mean",
            "平均亮度变化",
            format!("正值更亮；{} − {}", labels.1, labels.0),
        ),
        (
            "numeric.rmse",
            "像素 RMS 差异",
            format!("越小越接近{}", labels.0),
        ),
        (
            "reference.delta_e00.p95.simulation_distance",
            "对比图色差 P95",
            "越小越接近对比图".into(),
        ),
        (
            "reference.flip.mean.simulation_distance",
            "对比图 FLIP 平均差异",
            "越小越接近对比图".into(),
        ),
    ] {
        if let Some(row) = measurements(case).iter().find(|r| {
            r["region_id"] == "full_image"
                && r["metric"] == metric
                && get(r, "availability", "available") == "available"
                && r["value"].as_f64().is_some()
        }) {
            let value = row["value"].as_f64().unwrap();
            let rounded = if value.abs() >= 0.001 || value == 0.0 {
                format!("{value:.3}")
            } else {
                format!("{value:.2e}")
            };
            cards.push_str(&format!("<div title=\"{}\"><span>{label}</span><strong>{rounded} <small>{}</small></strong><small>{direction}</small></div>",escape(get(row,"domain","未声明域")),escape(get(row,"units",""))));
        }
    }
    if cards.is_empty() {
        cards
    } else {
        format!("<div class=\"metric-summary\" aria-label=\"全图差异摘要\">{cards}</div>")
    }
}
pub(crate) fn changed(left: &Value, right: &Value, path: &str, changes: &mut Vec<String>) {
    if let (Some(a), Some(b)) = (left.as_object(), right.as_object()) {
        let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
        for key in keys {
            changed(
                &left[key],
                &right[key],
                &if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                },
                changes,
            );
        }
    } else if left != right {
        let name = if path == "camera.exposure_compensation_ev" {
            "曝光补偿（EV）"
        } else {
            path
        };
        changes.push(format!("{name}: {} → {}", short(left), short(right)));
    }
}
pub(crate) fn short(value: &Value) -> String {
    let s = if let Some(s) = value.as_str() {
        s.to_owned()
    } else {
        value.to_string()
    };
    if s.chars().count() > 64 {
        s.chars().take(61).collect::<String>() + "…"
    } else {
        s
    }
}
pub(crate) fn change_summary(case: &Value, comparison: &Value) -> String {
    let mut changes = vec![];
    for row in rows(&comparison["condition_differences"]) {
        if !row["case_id"].is_null()
            && row["case_id"]
                != case
                    .get("case_id")
                    .or_else(|| case.get("id"))
                    .cloned()
                    .unwrap_or(Value::Null)
        {
            continue;
        }
        if row.get("seed").is_some() && row["seed"] != case["seed"] {
            continue;
        }
        let name = get(row, "field", "condition");
        if name == "resolved_parameters" {
            changed(&row["baseline"], &row["candidate"], "", &mut changes);
        } else {
            let (a, b) = if row.get("baseline").is_some() && row.get("candidate").is_some() {
                (&row["baseline"], &row["candidate"])
            } else {
                (&row["requested"], &row["observed"])
            };
            changes.push(format!("{}: {} → {}", human(name), short(a), short(b)));
        }
    }
    let mut result = if changes.is_empty() {
        "保留的比较条件未记录变化。".into()
    } else {
        changes
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ")
    };
    if changes.len() > 3 {
        result.push_str(&format!(" · 另 {} 项（见支持证据）", changes.len() - 3));
    }
    format!(
        "<p class=\"change-summary\"><strong>基准 → 候选</strong> · {}</p>",
        escape(&result)
    )
}
pub(crate) fn folded(label: &str, value: &Value) -> String {
    format!(
        "<details><summary>{}</summary><pre>{}</pre></details>",
        escape(label),
        text(value)
    )
}
pub(crate) fn retained_mask(path: &Path) -> Result<(Vec<bool>, usize, usize)> {
    if path.extension().is_some_and(|v| v == "npy") {
        let (data, shape) = read_npy(path)?;
        if shape.len() != 2 || shape[0] == 0 || shape[1] == 0 || data.iter().any(|v| !v.is_finite())
        {
            return Err(Error::new(
                "invalid_artifact",
                "retained region mask must contain finite two-dimensional samples",
            ));
        }
        Ok((
            data.into_iter().map(|v| v != 0.0).collect(),
            shape[1],
            shape[0],
        ))
    } else {
        let image = image::open(path)
            .map_err(|e| Error::new("invalid_artifact", e.to_string()))?
            .to_luma32f();
        Ok((
            image.pixels().map(|p| p[0] != 0.0).collect(),
            image.width() as usize,
            image.height() as usize,
        ))
    }
}
pub(crate) fn geometry_rectangle(
    geometry: &Value,
    assets: &mut Assets,
) -> Result<Option<[u32; 4]>> {
    if let Some(path) = geometry["path"].as_str() {
        let Some(entry) = assets
            .copy(&json!(path))?
            .filter(|e| e["availability"] == "available")
        else {
            return Ok(None);
        };
        let (mask, w, h) = retained_mask(&assets.out.join(get(&entry, "path", "")))?;
        let mut bounds = [w, h, 0, 0];
        let mut found = false;
        for (i, valid) in mask.iter().enumerate() {
            if *valid {
                found = true;
                bounds[0] = bounds[0].min(i % w);
                bounds[1] = bounds[1].min(i / w);
                bounds[2] = bounds[2].max(i % w);
                bounds[3] = bounds[3].max(i / w);
            }
        }
        if !found {
            return Err(Error::new("region_bounds", "retained region mask is empty"));
        }
        return Ok(Some([
            bounds[0] as u32,
            bounds[1] as u32,
            (bounds[2] - bounds[0] + 1) as u32,
            (bounds[3] - bounds[1] + 1) as u32,
        ]));
    }
    rectangle(&json!({"geometry":geometry}))
}

pub(crate) fn pair_case(case: &Value, pair: &Value) -> Value {
    let mut view = pair.clone();
    for key in [
        "case_id",
        "seed",
        "source_dimensions",
        "source_geometry",
        "preparation",
        "input",
    ] {
        view[key] = case[key].clone();
    }
    if let Some(stages) = case.get("stage_readouts") {
        view["stage_readouts"] = stages.clone();
    }
    view["pair_id"] = pair["id"].clone();
    view
}

// Fragment identities are retained case/seed/pair/region values, never display ordinals.
pub(crate) fn fragment_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(encoded, "%{byte:02X}").unwrap();
        }
    }
    encoded
}

pub(crate) fn navigation_fragment(
    case_id: &str,
    seed: &Value,
    pair: Option<&str>,
    region: Option<&str>,
) -> String {
    let mut fragment = format!(
        "#case={}&seed={}",
        fragment_component(case_id),
        fragment_component(&text(seed))
    );
    if let Some(pair) = pair {
        fragment.push_str(&format!("&pair={}", fragment_component(pair)));
    }
    if let Some(region) = region {
        fragment.push_str(&format!("&region={}", fragment_component(region)));
    }
    fragment
}

pub(crate) fn navigation_links(fragment: &str, label: &str) -> String {
    format!(
        "<span class=\"navigation-links\"><a href=\"{}\">打开{label}链接</a> <button type=\"button\" data-copy-link=\"{}\">复制{label}链接</button></span>",
        escape(fragment),
        escape(fragment)
    )
}
