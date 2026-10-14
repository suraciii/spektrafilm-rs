//! Portable offline presentation of retained evidence; no rendering or measurement.
use crate::{
    common::{self, Error, Result, create_destination, destination_exists, dump, sha256_file},
    documents::{load_json_document, safe_artifact_path, validate_generated_document},
    image_data::read_npy,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

mod assets;
mod escape;
mod html;

pub use assets::*;
pub use escape::*;
pub use html::*;

/// Publish a self-contained offline report from a validated comparison directory.
///
/// Offline navigation uses `#case=<id>&seed=<seed>[&pair=<id>[&region=<id>]]`.
/// Every value is percent-encoded UTF-8 and denotes the exact retained identity.
/// The seed is its decimal representation; a region requires an explicit pair.
/// An omitted pair selects the retained default (or the first retained pair when
/// absent); version comparisons use `baseline/candidate`. Unknown identities,
/// duplicate/unknown keys, empty values and malformed encoding show an alert and
/// hide the case panels until a valid link or case selection is chosen.
pub fn build_report(comparison: &Path, out: &Path) -> Result<()> {
    let root = crate::documents::normalize_path(out)?;
    let preexisting = fs::symlink_metadata(out).is_ok();
    let result = build_report_impl(comparison, out);
    if preexisting {
        result
    } else {
        result.map_err(|error| common::incomplete_destination(error, &root, "report.json"))
    }
}

fn build_report_impl(comparison: &Path, out: &Path) -> Result<()> {
    if fs::symlink_metadata(out).is_ok() {
        return Err(destination_exists(out));
    }
    let root = comparison.canonicalize()?;
    let document = safe_artifact_path(&root, "comparison.json")?;
    let snapshot = fs::read(&document)?;
    let comparison = load_json_document(&document)?;
    if comparison["schema_version"] != 1 {
        return Err(Error::new(
            "unsupported_schema",
            "report requires comparison schema_version 1",
        ));
    }
    validate_generated_document("comparison", &comparison, &root)?;
    let cases = comparison["cases"]
        .as_array()
        .ok_or_else(|| Error::new("invalid_document", "comparison must contain a cases array"))?;
    let records = normalized_artifacts(&comparison["artifacts"])?;
    let mut assets = Assets::new(&root, out, records)?;
    create_destination(out)?;
    fs::create_dir_all(out.join("artifacts"))?;
    for record in assets.records.clone() {
        assets.copy(&record)?;
    }
    fs::write(out.join("comparison.json"), &snapshot)?;
    assets.manifest.push(json!({"path":"comparison.json","source_path":"comparison.json","sha256":format!("{:x}",Sha256::digest(&snapshot)),"role":"comparison","media_type":"application/json","availability":"available"}));
    let ordered: Vec<_> = cases.iter().collect();
    if ordered.iter().any(|c| !c.is_object()) {
        return Err(Error::new(
            "invalid_document",
            "case result must be an object",
        ));
    }
    let mut html = format!(
        "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; base-uri 'none'; form-action 'none'\"><title>SpektraFilm benchmark review</title><style>{CSS}</style><body><h1>SpektraFilm 人工审查</h1><p>选择场景查看 A/B；细节、差异图和完整数据按需展开。</p><nav class=\"case-nav\" aria-label=\"案例与种子\"><label>案例 / 种子 <select data-case-nav>"
    );
    for (index, case) in ordered.iter().enumerate() {
        html.push_str(&format!(
            "<option value=\"case-{index}\">{} · seed {} · {}</option>",
            escape(&human(get(case, "case_id", get(case, "id", "unavailable")))),
            field(case, "seed"),
            escape(get(case, "status", "unknown"))
        ));
    }
    html.push_str("</select></label><span class=\"small\">所有案例和失败状态均可访问</span></nav><p data-navigation-error role=\"alert\" hidden></p><p data-copy-status role=\"status\"></p><p class=\"small\">");
    html.push_str(
        &[
            ("selected_cases", "案例"),
            ("completed_results", "完成"),
            ("failed_results", "失败"),
        ]
        .iter()
        .map(|(key, label)| {
            format!(
                "{label}：{}",
                comparison["summary"].get(key).map(text).unwrap_or_else(|| {
                    if comparison["mode"] != "single_run" {
                        return "未记录".into();
                    }
                    match *key {
                        "selected_cases" => cases
                            .iter()
                            .filter_map(|case| case["case_id"].as_str())
                            .collect::<BTreeSet<_>>()
                            .len()
                            .to_string(),
                        "completed_results" => cases
                            .iter()
                            .filter(|case| case["status"] == "completed")
                            .count()
                            .to_string(),
                        "failed_results" => cases
                            .iter()
                            .filter(|case| case["status"] == "failed")
                            .count()
                            .to_string(),
                        _ => "未记录".into(),
                    }
                })
            )
        })
        .collect::<Vec<_>>()
        .join(" · "),
    );
    html.push_str("</p><details class=\"supporting\"><summary>比较摘要与跨种子证据</summary><p>选中案例后，打开下方支持证据查看原始来源、协议、规则、聚合和 findings。</p>");
    for (key, label) in [
        ("summary", "汇总"),
        ("selection", "选择"),
        ("protocol", "协议"),
        ("condition_differences", "条件变化"),
        ("rules", "规则"),
        ("baseline", "基准来源"),
        ("candidate", "候选来源"),
        ("run", "运行来源"),
    ] {
        if comparison["mode"] == "single_run" && ["baseline", "candidate"].contains(&key) {
            continue;
        }
        html.push_str(&folded(
            label,
            comparison.get(key).unwrap_or(&json!("不可用")),
        ));
    }
    if !rows(&comparison["seed_measurements"]).is_empty() {
        html.push_str(&format!(
            "<details><summary>跨种子完整测量</summary>{}</details>",
            metric_table(&json!({"measurements":comparison["seed_measurements"]}))
        ));
    }
    html.push_str("</details>");
    for (index, case) in ordered.iter().enumerate() {
        let id = get(case, "case_id", get(case, "id", "unavailable"));
        html.push_str(&format!("<section id=\"case-{index}\" class=\"case-panel\" data-case-panel=\"case-{index}\" data-case-id=\"{}\" data-seed=\"{}\"{}><h2>{} <small>· seed {}</small></h2>{}", escape(id), field(case, "seed"), if index == 0 { "" } else { " hidden" }, escape(&human(id)), field(case, "seed"), navigation_links(&navigation_fragment(id, &case["seed"], None, None), "案例")));
        let pairs = rows(&case["pairs"]);
        let views: Vec<Value> = if pairs.is_empty() {
            vec![(**case).clone()]
        } else {
            pairs.iter().map(|pair| pair_case(case, pair)).collect()
        };
        let default = get(&comparison, "default_pair", "input");
        let default = if pairs.iter().any(|pair| pair["id"] == default) {
            default
        } else {
            pairs
                .first()
                .map(|pair| get(pair, "id", ""))
                .unwrap_or("baseline/candidate")
        };
        if !pairs.is_empty() {
            html.push_str("<nav class=\"pair-nav\" aria-label=\"比较对象\"><label>比较对象 <select data-pair-nav>");
            for (pair_index, pair) in pairs.iter().enumerate() {
                html.push_str(&format!(
                    "<option value=\"pair-{index}-{pair_index}\"{}>{} / 模拟</option>",
                    if pair["id"] == default {
                        " selected"
                    } else {
                        ""
                    },
                    if pair["left_role"] == "input" {
                        "输入"
                    } else {
                        "对比图"
                    }
                ));
            }
            html.push_str("</select></label></nav>");
        }
        for (pair_index, case) in views.iter().enumerate() {
            let labels = if case.get("pair_id").is_some() {
                (
                    if case["left_role"] == "input" {
                        "输入"
                    } else {
                        "对比图"
                    },
                    "模拟",
                )
            } else {
                ("基准", "候选")
            };
            let pair_id = get(case, "pair_id", "baseline/candidate");
            html.push_str(&format!("<div class=\"image-review\" data-pair-panel=\"pair-{index}-{pair_index}\" data-pair-id=\"{}\" data-default-pair=\"{}\"{}>", escape(pair_id), pair_id == default, if pair_id == default { "" } else { " hidden" }));
            let mut images = BTreeMap::new();
            for role in ["source", "baseline", "candidate", "reference", "difference"] {
                let reference = role_reference(case, role, &assets);
                let mut image = assets.image(&reference)?;
                if let Some(entry) = image.as_mut()
                    && let Some(preview) = entry.get("fit_preview").cloned()
                    && let Some(fit) = assets.image(&preview)?
                {
                    entry["fit_entry"] = fit;
                }
                images.insert(role, image);
            }
            let a = images["baseline"].as_ref();
            let b = images["candidate"].as_ref();
            html.push_str("<div class=\"review-actions\"><button type=\"button\" data-review-reset aria-pressed=\"true\">全图</button>");
            html.push_str(&format!("<button type=\"button\" data-difference-toggle aria-pressed=\"false\"{}>差异图</button></div><div data-review-full>", if images["difference"].is_some() { "" } else { " disabled" }));
            if case["pixel_aligned"] != false
                && let (Some(a), Some(b)) = (a, b)
            {
                let da = image::image_dimensions(out.join(get(a, "path", "")))
                    .map_err(|e| Error::new("invalid_artifact", e.to_string()))?;
                let db = image::image_dimensions(out.join(get(b, "path", "")))
                    .map_err(|e| Error::new("invalid_artifact", e.to_string()))?;
                if da == db {
                    html.push_str(&viewer(Some(a), Some(b), false, labels, true));
                } else {
                    html.push_str("<p class=\"unavailable\">图像尺寸不同，仅提供并列检查，未声明共同观察尺度。</p><div class=\"grid\">");
                    html.push_str(&viewer(Some(a), Some(b), false, labels, false));
                    html.push_str("</div>");
                }
            } else {
                html.push_str(
                    "<p class=\"unavailable\">同步 A/B 不可用：证据缺失或图像未对齐。</p>",
                );
                html.push_str(&viewer(a, b, false, labels, false));
            }
            html.push_str("</div><div data-review-difference hidden>");
            html.push_str(&image_html(images["difference"].as_ref(), "差异图"));
            html.push_str(&format!(
                "<pre>{}</pre></div>",
                text(&difference_metadata(images["difference"].as_ref()))
            ));
            let mut regions: Vec<(Value, String)> = rows(&case["regions"])
                .iter()
                .cloned()
                .map(|v| (v, "指定 crop".into()))
                .collect();
            regions.extend(
                rows(
                    case.get("automatic_regions")
                        .or_else(|| case.get("hotspots"))
                        .unwrap_or(&Value::Null),
                )
                .iter()
                .cloned()
                .map(|v| (v, "自动差异".into())),
            );
            regions.extend(rows(&case["detail_regions"]).iter().cloned().map(|v| {
                let method =
                    get(&v, "selection_method", get(&v, "method", "保留的细节区域")).to_owned();
                (v, method)
            }));
            let mut thumbnails = String::new();
            let region_start = html.len();
            for (mut region, method) in regions.iter().cloned() {
                let region_id = get(&region, "id", get(&region, "region_id", "不可用")).to_owned();
                html.push_str(&format!(
                    "<details data-region-id=\"{}\" data-review-region hidden><summary>{}</summary>{}",
                    escape(&region_id),
                    escape(get(&region, "label", &human(&region_id))),
                    navigation_links(
                        &navigation_fragment(id, &case["seed"], Some(pair_id), Some(&region_id)),
                        "区域"
                    )
                ));
                let region_viewer_start = html.len();
                let mut mask_entry = None;
                if let Some(path) = region["geometry"]["path"].as_str() {
                    if case["status"] != "completed" && !assets.index.contains_key(path) {
                        html.push_str(&format!(
                            "<p class=\"unavailable\">{}：渲染失败，区域掩膜证据不可用。</p>",
                            field(&region, "label")
                        ));
                        html.push_str("</details>");
                        thumbnails.push_str(&crop_thumbnail(
                            &region,
                            &method,
                            [None, None],
                            labels,
                        ));
                        continue;
                    }
                    let entry = assets.copy(&json!(path))?;
                    if entry
                        .as_ref()
                        .is_none_or(|e| e["availability"] != "available")
                    {
                        html.push_str(&format!(
                            "<p class=\"unavailable\">{}：区域掩膜证据不可用。</p>",
                            field(&region, "label")
                        ));
                        html.push_str("</details>");
                        thumbnails.push_str(&crop_thumbnail(
                            &region,
                            &method,
                            [None, None],
                            labels,
                        ));
                        continue;
                    }
                    let entry = entry.unwrap();
                    let (mask, w, h) = retained_mask(&out.join(get(&entry, "path", "")))?;
                    let mut xmin = w;
                    let mut ymin = h;
                    let mut xmax = 0;
                    let mut ymax = 0;
                    let mut count = 0;
                    for (i, v) in mask.iter().enumerate() {
                        if *v {
                            xmin = xmin.min(i % w);
                            ymin = ymin.min(i / w);
                            xmax = xmax.max(i % w);
                            ymax = ymax.max(i / w);
                            count += 1;
                        }
                    }
                    if count == 0 {
                        return Err(Error::new("region_bounds", "retained region mask is empty"));
                    }
                    region["rectangle"] = json!([xmin, ymin, xmax - xmin + 1, ymax - ymin + 1]);
                    mask_entry = Some(entry);
                }
                let rect = rectangle(&region)?;
                html.push_str(&format!(
                    "<p>坐标 [x, y, width, height]：{} · 选择方式：{}</p>",
                    rect.map(|v| text(&json!(v)))
                        .unwrap_or_else(|| "不可用".into()),
                    escape(get(
                        &region,
                        "selection_method",
                        get(&region, "method", &method)
                    ))
                ));
                html.push_str(&folded("选择依据与区域元数据", &region));
                if let Some(entry) = mask_entry {
                    html.push_str(&format!(
                        "<p>测量使用保留掩膜（含排除像素）：<a href=\"{}\">区域掩膜</a></p>",
                        url(&entry)
                    ));
                }
                for (image, geometry_key, label) in [
                    (a, "left_geometry", labels.0),
                    (b, "right_geometry", labels.1),
                ] {
                    let locator_rect = if let Some(geometry) = region.get(geometry_key) {
                        geometry_rectangle(geometry, &mut assets)?
                    } else if case["pixel_aligned"] != false {
                        rect
                    } else {
                        None
                    };
                    if let (Some(image), Some([x, y, w, h])) = (image, locator_rect) {
                        let (width, height) =
                            image::image_dimensions(out.join(get(image, "path", "")))
                                .map_err(|e| Error::new("invalid_artifact", e.to_string()))?;
                        html.push_str(&format!("<figure class=\"locator\" data-region-locator><div class=\"locator-image\"><img src=\"{}\" alt=\"{}区域定位\"><span class=\"rectangle\" style=\"left:{}%;top:{}%;width:{}%;height:{}%\"></span></div><figcaption>{} · {}</figcaption></figure>",url(image),escape(label),100.0*x as f64/width as f64,100.0*y as f64/height as f64,100.0*w as f64/width as f64,100.0*h as f64/height as f64,escape(label),text(&json!([x,y,w,h]))));
                    } else {
                        html.push_str(&format!(
                            "<div class=\"unavailable\">{}区域定位：不可用</div>",
                            escape(label)
                        ));
                    }
                }
                let mut crops = vec![];
                for (role, image) in [("baseline", a), ("candidate", b)] {
                    if case.get("pair_id").is_some() {
                        region["pair_id"] = case["pair_id"].clone();
                    }
                    let reference = role_reference(&region, role, &assets);
                    let retained = assets.image(&reference)?.filter(|e| {
                        !matches!(
                            Path::new(get(e, "path", ""))
                                .extension()
                                .and_then(|v| v.to_str()),
                            Some("jpg" | "jpeg")
                        )
                    });
                    crops.push(if retained.is_some() {
                        retained
                    } else {
                        let geometry_key = if role == "baseline" {
                            "left_geometry"
                        } else {
                            "right_geometry"
                        };
                        let crop_rect = if let Some(geometry) = region.get(geometry_key) {
                            geometry_rectangle(geometry, &mut assets)?
                        } else if case["pixel_aligned"] != false {
                            rect
                        } else {
                            None
                        };
                        assets.crop(image, crop_rect)?
                    });
                }
                for entry in crops.iter_mut().flatten() {
                    if let Ok((width, height)) =
                        image::image_dimensions(out.join(get(entry, "path", "")))
                    {
                        entry["dimensions"] = json!([width, height]);
                    }
                    if let Some(preview) = entry.get("fit_preview").cloned()
                        && let Some(fit) = assets.image(&preview)?
                    {
                        entry["fit_entry"] = fit;
                    }
                }
                thumbnails.push_str(&crop_thumbnail(
                    &region,
                    &method,
                    [crops[0].as_ref(), crops[1].as_ref()],
                    labels,
                ));
                let crop_aligned = case["pixel_aligned"] != false
                    && crops[0]
                        .as_ref()
                        .zip(crops[1].as_ref())
                        .is_some_and(|(left, right)| {
                            left["dimensions"].is_array()
                                && left["dimensions"] == right["dimensions"]
                        });
                html.insert_str(
                    region_viewer_start,
                    &viewer(
                        crops[0].as_ref(),
                        crops[1].as_ref(),
                        true,
                        labels,
                        crop_aligned,
                    ),
                );
                let findings: Vec<_> = rows(&comparison["findings"])
                    .iter()
                    .filter(|f| {
                        f["case_id"] == id
                            && f["region_id"] == region_id
                            && (case.get("pair_id").is_none() || f["pair_id"] == case["pair_id"])
                    })
                    .cloned()
                    .collect();
                html.push_str(&folded("该区域 findings", &json!(findings)));
                html.push_str("</details>");
            }
            let region_panels = html.split_off(region_start);
            html.push_str(&region_panels);
            if regions.is_empty() {
                html.push_str(
                    "<p class=\"crop-empty\" data-crop-empty>没有可查看的 crop；当前显示全图。</p>",
                );
            } else {
                html.push_str("<div class=\"crop-tabs\" role=\"group\" aria-label=\"crop 类型\"><button type=\"button\" data-crop-filter=\"all\" aria-pressed=\"true\">全部 crop</button><button type=\"button\" data-crop-filter=\"manual\" aria-pressed=\"false\">指定 crop</button><button type=\"button\" data-crop-filter=\"automatic\" aria-pressed=\"false\">自动差异</button></div>");
                html.push_str(&format!("<div class=\"crop-strip\" aria-label=\"指定 crop 与自动差异区域\">{thumbnails}</div>"));
            }
            html.push_str(&metric_summary(case, labels));
            html.push_str(&format!(
                "{}<p class=\"status\">状态：{} {}</p>",
                navigation_links(
                    &navigation_fragment(id, &case["seed"], Some(pair_id), None),
                    "比较"
                ),
                escape(get(case, "status", "unavailable")),
                case.get("reason").map(text).unwrap_or_default()
            ));
            if case.get("pair_id").is_none() {
                html.push_str(&change_summary(case, &comparison));
            } else {
                html.push_str(&folded("比较角色、对应关系与适用范围", case));
            }
            html.push_str("<details><summary>来源输入与参考图</summary><div class=\"grid\">");
            for (role, label) in [("source", "来源输入"), ("reference", "参考图")] {
                html.push_str(&image_html(images[role].as_ref(), label));
            }
            html.push_str("</div>");
            if images["reference"].is_some() || case.get("reference").is_some() {
                html.push_str(&format!(
                    "<pre>{}</pre>",
                    case.get("reference")
                        .map(text)
                        .unwrap_or_else(|| "参考范围和来源限制不可用".into())
                ));
            }
            html.push_str("</details><details><summary>差异图与显示元数据</summary>");
            html.push_str(&image_html(images["difference"].as_ref(), "差异图"));
            let metadata = difference_metadata(images["difference"].as_ref());
            html.push_str(&format!("<pre>{}</pre></details>", text(&metadata)));
            let plot_roles = [
                "tone_response",
                "grain_radial_spectrum",
                "halation_profile",
                "halation_edge_spill_profile",
                "edge_response",
                "edge_sfr",
                "reference_material_curve",
                "reference_regional_statistics",
            ];
            let mut plots = String::new();
            for artifact in rows(&case["artifacts"]) {
                if plot_roles.contains(&get(artifact, "role", "")) {
                    let label = get(artifact, "id", get(artifact, "role", ""));
                    if let Some(plot) = assets.plot(artifact, label)?
                        && let Some(entry) = assets.copy(artifact)?
                    {
                        plots.push_str(&format!("<details><summary>{}</summary>{plot}<a href=\"{}\">保留的数值样本</a></details>",escape(&human(label)),url(&entry)));
                    }
                }
            }
            if !plots.is_empty() {
                html.push_str(&format!(
                    "<details><summary>曲线与频谱证据</summary>{plots}</details>"
                ));
            }
            for (role, label) in if case.get("pair_id").is_some() {
                [("left", labels.0), ("right", labels.1)]
            } else {
                [("baseline", labels.0), ("candidate", labels.1)]
            } {
                let entry =
                    assets.copy(case.get(format!("{role}_resolved")).unwrap_or(&Value::Null))?;
                html.push_str(&format!(
                    "<details><summary>{label}执行与渲染配置</summary><pre>{}</pre>",
                    case.get(format!("{role}_execution"))
                        .map(text)
                        .unwrap_or_else(|| "不可用".into())
                ));
                if let Some(entry) = entry.filter(|e| e["availability"] == "available") {
                    html.push_str(&format!("<a href=\"{}\">已解析配置</a>", url(&entry)));
                }
                html.push_str("</details>");
            }
            html.push_str("<details><summary>完整测量（原样保留）</summary>");
            html.push_str(&metric_table(case));
            for (key, label) in [
                ("curves", "Retained response curves"),
                ("spectra", "Retained spectra"),
            ] {
                if let Some(value) = case.get(key) {
                    html.push_str(&format!("<h4>{label}</h4><pre>{}</pre>", text(value)));
                }
            }
            html.push_str("</details></div>");
        }
        for (key, label) in [
            ("source_dimensions", "原始输入尺寸"),
            ("source_geometry", "输入解码与模拟几何"),
            ("preparation", "输入准备来源与操作记录"),
            ("input", "输入来源与文件身份"),
        ] {
            if let Some(value) = case.get(key).filter(|value| !value.is_null()) {
                html.push_str(&folded(label, value));
            }
        }
        html.push_str("</section>");
    }
    let omissions: Vec<_> = assets
        .manifest
        .iter()
        .filter(|entry| entry["availability"] == "unavailable")
        .cloned()
        .collect();
    if !omissions.is_empty() {
        html.push_str(&folded(
            "不可用与禁止再分发的证据（原样保留）",
            &json!(omissions),
        ));
    }
    html.push_str(&folded(
        "全部 findings（原样保留）",
        comparison.get("findings").unwrap_or(&json!([])),
    ));
    html.push_str(&format!("<p><a href=\"comparison.json\">比较快照</a> · <a href=\"report.json\">相对 artifact manifest</a></p><script>{JS}</script></body></html>"));
    fs::write(out.join("index.html"), html)?;
    assets.manifest.push(json!({"path":"index.html","sha256":sha256_file(&out.join("index.html"))?,"role":"report","media_type":"text/html","availability":"available"}));
    dump(
        &out.join("report.json"),
        &json!({"schema_version":1,"command":"report","execution_status":"completed","comparison_id":comparison.get("id"),"artifact":"index.html","artifacts":assets.manifest}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_fragments_encode_exact_identities() {
        assert_eq!(
            navigation_fragment(
                "色 / &%?#",
                &json!(17),
                Some("reference/a&b"),
                Some("patch + β")
            ),
            "#case=%E8%89%B2%20%2F%20%26%25%3F%23&seed=17&pair=reference%2Fa%26b&region=patch%20%2B%20%CE%B2"
        );
        assert_eq!(
            navigation_fragment("case", &json!(0), None, None),
            "#case=case&seed=0"
        );
    }
    #[test]
    fn redistribution_denial_never_copies_bytes() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::write(source.path().join("private.png"), b"private").unwrap();
        let mut assets = Assets::new(
            source.path(),
            out.path(),
            vec![json!({"id":"private","path":"private.png","redistribution":false})],
        )
        .unwrap();
        let entry = assets.copy(&json!("private")).unwrap().unwrap();
        assert_eq!(entry["reason"], "redistribution_not_permitted");
        assert!(!out.path().join("private.png").exists());
    }
    #[test]
    fn hash_mismatch_is_rejected_before_publication() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::write(source.path().join("data.json"), b"{}").unwrap();
        let mut assets = Assets::new(
            source.path(),
            out.path(),
            vec![json!({"path":"data.json","sha256":"incorrect"})],
        )
        .unwrap();
        assert_eq!(
            assets.copy(&json!("data.json")).unwrap_err().code,
            "artifact_hash"
        );
        assert!(!out.path().join("data.json").exists());
    }
    #[test]
    fn png16_crops_preserve_every_integer_code() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::create_dir(out.path().join("artifacts")).unwrap();
        let codes = [
            [1, 257, 65535],
            [65534, 32769, 17],
            [19, 20, 21],
            [22, 23, 24],
        ];
        crate::image_data::write_png16_codes(&source.path().join("full.png"), &codes, 2, 2)
            .unwrap();
        let mut assets = Assets::new(
            source.path(),
            out.path(),
            vec![json!({"id":"full","path":"full.png","bit_depth":16})],
        )
        .unwrap();
        let full = assets.image(&json!("full")).unwrap().unwrap();
        let crop = assets
            .crop(Some(&full), Some([1, 0, 1, 2]))
            .unwrap()
            .unwrap();
        let image = image::open(out.path().join(crop["path"].as_str().unwrap()))
            .unwrap()
            .to_rgb16();
        assert_eq!(image.get_pixel(0, 0).0, codes[1]);
        assert_eq!(image.get_pixel(0, 1).0, codes[3]);
    }
    #[test]
    fn single_pair_missing_assets_never_fall_back_to_other_pair() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::write(source.path().join("wrong.png"), b"another pair").unwrap();
        let mut assets = Assets::new(
            source.path(),
            out.path(),
            vec![
                json!({"id":"wrong","path":"wrong.png","role":"baseline_review","case_id":"local"}),
            ],
        )
        .unwrap();
        let case = pair_case(
            &json!({"case_id":"local","seed":0,"baseline_review":"wrong"}),
            &json!({"id":"reference","review":{"left":"missing.png"}}),
        );
        let reference = role_reference(&case, "baseline", &assets);
        assert_eq!(reference, "missing.png");
        assert!(assets.image(&reference).unwrap().is_none());
        assert_eq!(assets.manifest[0]["reason"], "missing_artifact");
        assert!(role_reference(&case, "candidate", &assets).is_null());
        assert!(role_reference(&case, "difference", &assets).is_null());
    }

    #[test]
    fn corresponding_region_masks_keep_independent_crop_geometry() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::create_dir(out.path().join("artifacts")).unwrap();
        crate::image_data::write_npy(
            &source.path().join("mask.npy"),
            &[0.0, 1.0, 1.0, 0.0, 0.0, 0.0],
            &[2, 3],
        )
        .unwrap();
        let mut assets =
            Assets::new(source.path(), out.path(), vec![json!({"path":"mask.npy"})]).unwrap();
        assert_eq!(
            geometry_rectangle(&json!({"path":"mask.npy"}), &mut assets).unwrap(),
            Some([1, 0, 2, 1])
        );
        assert_eq!(
            geometry_rectangle(&json!([0, 1, 1, 1]), &mut assets).unwrap(),
            Some([0, 1, 1, 1])
        );
    }

    #[test]
    fn reference_difference_display_keeps_valid_mask_exclusions() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::create_dir(out.path().join("artifacts")).unwrap();
        crate::image_data::write_npy(&source.path().join("map.npy"), &[1.0, 2.0], &[1, 2]).unwrap();
        crate::image_data::write_npy(&source.path().join("mask.npy"), &[1.0, 0.0], &[1, 2])
            .unwrap();
        let mut assets = Assets::new(source.path(), out.path(), vec![json!({"id":"map","path":"map.npy","role":"difference_map","metric":"delta_e00","valid_mask_artifact_id":"mask"}),json!({"id":"mask","path":"mask.npy"})]).unwrap();
        let display = assets.image(&json!("map")).unwrap().unwrap();
        let pixels = image::open(out.path().join(display["path"].as_str().unwrap()))
            .unwrap()
            .to_rgba8();
        assert_eq!(pixels.get_pixel(0, 0)[3], 255);
        assert_eq!(pixels.get_pixel(1, 0)[3], 0);
    }

    #[test]
    fn malformed_rectangles_are_errors() {
        assert_eq!(
            rectangle(&json!({"geometry":[0,0,0,10]})).unwrap_err().code,
            "region_bounds"
        );
        assert!(rectangle(&json!({"geometry":[1,2,3,4]})).unwrap().is_some());
    }
}
