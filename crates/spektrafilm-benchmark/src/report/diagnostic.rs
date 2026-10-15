//! Image-first diagnostic report layout.
//!
//! This module only selects and presents retained comparison evidence. It does
//! not render images, recompute metrics, fit geometry, or infer exposure.

use super::display;
use super::{
    Assets, Layout, Result, Value, escape, field, folded, geometry_rectangle, get, human,
    image_html, measurements, metric_table, navigation_fragment, navigation_links, pair_case, rows,
    text, url, viewer,
};
use crate::common::Error;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const ROLE_ORDER: &[&str] = &["source", "baseline", "candidate", "simulation", "reference"];

#[derive(Clone, Debug)]
pub(crate) struct FocusSelection {
    pub(crate) ids: Vec<String>,
}

impl FocusSelection {
    pub(crate) fn new(layout: Layout, requested: &[String], comparison: &Value) -> Result<Self> {
        if layout == Layout::Standard && !requested.is_empty() {
            return Err(Error::new(
                "focus_requires_diagnostic_layout",
                "--focus-region requires --layout diagnostic; choose diagnostic or remove the focus selection",
            ));
        }
        let mut seen = BTreeSet::new();
        for id in requested {
            if !seen.insert(id) {
                return Err(Error::new(
                    "duplicate_focus_region",
                    format!("focus region '{id}' was selected more than once"),
                ));
            }
        }
        let available = available_region_ids(comparison);
        if let Some(id) = requested.iter().find(|id| !available.contains(*id)) {
            let ids = if available.is_empty() {
                "(none retained)".to_owned()
            } else {
                available.iter().cloned().collect::<Vec<_>>().join(", ")
            };
            return Err(Error::new(
                "unknown_focus_region",
                format!("unknown focus region '{id}'; available IDs: {ids}"),
            ));
        }
        Ok(Self {
            ids: requested.to_vec(),
        })
    }
}

fn available_region_ids(comparison: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for case in rows(&comparison["cases"]) {
        let views = case_views(case);
        for view in &views {
            for region in rows(&view["regions"]) {
                if let Some(id) = region
                    .get("id")
                    .or_else(|| region.get("region_id"))
                    .and_then(Value::as_str)
                {
                    ids.insert(id.to_owned());
                }
            }
            for region in rows(&view["automatic_regions"]) {
                if let Some(id) = region
                    .get("id")
                    .or_else(|| region.get("region_id"))
                    .and_then(Value::as_str)
                {
                    ids.insert(id.to_owned());
                }
            }
            for region in rows(&view["reference"]["regions"]) {
                if let Some(id) = region
                    .get("id")
                    .or_else(|| region.get("region_id"))
                    .and_then(Value::as_str)
                {
                    ids.insert(id.to_owned());
                }
            }
        }
    }
    ids
}

pub(crate) fn page(
    comparison: &Value,
    ordered: &[&Value],
    assets: &mut Assets,
    out: &Path,
    focus: &FocusSelection,
) -> Result<String> {
    let mut html = String::from(
        "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; base-uri 'none'; form-action 'none'\"><title>SpektraFilm 诊断审查</title><style>\n",
    );
    html.push_str(super::CSS);
    html.push_str("</style><body><h1>SpektraFilm 诊断审查</h1><p>先看条件与整卡，再查看指定重点、完整区域代价和保留的阶段证据。数值只来自比较快照。</p><nav class=\"case-nav\" aria-label=\"案例与种子\"><label>案例 / 种子 <select data-case-nav>");
    for (index, case) in ordered.iter().enumerate() {
        html.push_str(&format!(
            "<option value=\"case-{index}\">{} · seed {} · {}</option>",
            escape(get(case, "case_id", get(case, "id", "unavailable"))),
            field(case, "seed"),
            escape(get(case, "status", "unknown")),
        ));
    }
    html.push_str("</select></label><span class=\"small\">诊断布局按请求顺序保留焦点区域</span></nav><p data-navigation-error role=\"alert\" hidden></p><p data-copy-status role=\"status\"></p>");
    html.push_str(&format!(
        "<p class=\"small\">布局：诊断 · 焦点区域：{}</p>",
        if focus.ids.is_empty() {
            "未指定".to_owned()
        } else {
            focus
                .ids
                .iter()
                .map(|id| escape(id))
                .collect::<Vec<_>>()
                .join("、")
        }
    ));
    html.push_str("<details class=\"supporting\"><summary>比较摘要与跨种子证据</summary>");
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
        html.push_str(&folded(label, comparison.get(key).unwrap_or(&Value::Null)));
    }
    if !rows(&comparison["seed_measurements"]).is_empty() {
        html.push_str(&format!(
            "<details><summary>跨种子完整测量</summary>{}</details>",
            metric_table(&json!({"measurements":comparison["seed_measurements"]})),
        ));
    }
    html.push_str("</details>");

    for (index, case) in ordered.iter().enumerate() {
        html.push_str(&case_page(comparison, case, index, assets, out, focus)?);
    }
    Ok(html)
}

fn case_page(
    comparison: &Value,
    case: &Value,
    index: usize,
    assets: &mut Assets,
    out: &Path,
    focus: &FocusSelection,
) -> Result<String> {
    let views = case_views(case);
    let default_view = default_view(comparison, &views);
    let default_pair = view_pair_id(default_view);
    let panel_id = format!("case-{index}");
    let case_id = get(case, "case_id", get(case, "id", "unavailable"));
    let mut html = format!(
        "<section id=\"{panel_id}\" class=\"case-panel diagnostic-case\" data-case-panel=\"{panel_id}\" data-diagnostic=\"true\" data-case-id=\"{}\" data-seed=\"{}\"><h2>{} <small>· seed {}</small></h2>",
        escape(case_id),
        field(case, "seed"),
        escape(&human(case_id)),
        field(case, "seed"),
    );
    let pair_ids = views.iter().map(view_pair_id).collect::<Vec<_>>();
    let pair_roles = views
        .iter()
        .map(|view| {
            let (left, right) = if view.get("pair_id").is_some() {
                let left = get(view, "left_role", "");
                let left = if left == "input" {
                    "source"
                } else if left == "reference" {
                    "reference"
                } else {
                    left
                };
                (
                    left.to_owned(),
                    get(view, "right_role", "simulation").to_owned(),
                )
            } else {
                ("baseline".to_owned(), "candidate".to_owned())
            };
            json!({"id":view_pair_id(view),"left":left,"right":right})
        })
        .collect::<Vec<_>>();
    html.push_str(&format!(
        "<div class=\"diagnostic-pair-panel\" data-pair-panel data-diagnostic-pair data-pair-id=\"{}\" data-default-pair=\"true\" data-pair-ids=\"{}\" data-pair-roles=\"{}\">",
        escape(&default_pair),
        escape(&pair_ids.join(",")),
        escape(&serde_json::to_string(&pair_roles).unwrap_or_default()),
    ));
    html.push_str(&format!(
        "<p>{}</p>",
        navigation_links(
            &navigation_fragment(case_id, &case["seed"], Some(&default_pair), None),
            "案例"
        ),
    ));
    html.push_str(&condition_summary(case, &views, assets)?);

    let mut role_entries = BTreeMap::new();
    for role in ROLE_ORDER {
        let reference = canonical_reference(&views, role, assets);
        if reference.is_null() {
            continue;
        }
        if let Some(entry) = assets.image(&reference)? {
            role_entries.insert((*role).to_owned(), display_entry(assets, &entry)?);
        }
    }
    let regions = collect_regions(&views);
    let reference = views
        .iter()
        .find_map(|view| view.get("reference").filter(|v| v.is_object()));
    let role_pairs = role_pairs(&role_entries);
    let (default_left, default_right) = default_roles(&role_entries);
    html.push_str("<section class=\"diagnostic-section diagnostic-chart\"><h3>模型 / 参考整卡</h3><div class=\"role-selectors\"><label>左图 <select data-role-select=\"left\">");
    for role in role_entries.keys() {
        html.push_str(&format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(role),
            if role == &default_left {
                " selected"
            } else {
                ""
            },
            escape(role_label(role)),
        ));
    }
    html.push_str("</select></label><label>右图 <select data-role-select=\"right\">");
    for role in role_entries.keys() {
        html.push_str(&format!(
            "<option value=\"{}\"{}>{}</option>",
            escape(role),
            if role == &default_right {
                " selected"
            } else {
                ""
            },
            escape(role_label(role)),
        ));
    }
    html.push_str("</select></label><span class=\"small\">可任意选择保留图像；未测量的视觉配对不会借用其他分数。</span></div><div class=\"diagnostic-role-viewers\">");
    for (left, right) in &role_pairs {
        let aligned = aligned_for_roles(default_view, left, right, reference, &role_entries, out);
        let panel = format!("{}|{}", left, right);
        let left_entry = role_entries.get(left).cloned();
        let right_entry = role_entries.get(right).cloned();
        let labels = (role_label(left), role_label(right));
        let visible = left == &default_left && right == &default_right;
        html.push_str(&format!(
            "<div class=\"diagnostic-role-view\" data-role-view=\"{}\"{}>{}",
            escape(&panel),
            if visible { "" } else { " hidden" },
            viewer(
                left_entry.as_ref(),
                right_entry.as_ref(),
                false,
                labels,
                aligned
            ),
        ));
        html.push_str(&format!(
            "<p class=\"diagnostic-selection-evidence\" data-selection-evidence=\"{}\">{}</p></div>",
            escape(&panel),
            selected_pair_evidence_notice(&views, left, right),
        ));
    }
    html.push_str("</div></section>");

    let focus_regions = focus
        .ids
        .iter()
        .filter_map(|id| regions.iter().find(|region| region_id(region) == *id))
        .cloned()
        .collect::<Vec<_>>();
    let focus_heading = if focus.ids.is_empty() {
        "焦点补丁对比（未指定焦点区域）"
    } else {
        "焦点补丁对比"
    };
    html.push_str(&format!(
        "<section class=\"diagnostic-section diagnostic-focus\"><h3>{focus_heading}</h3>"
    ));
    if focus_regions.is_empty() {
        html.push_str(
            "<p class=\"unavailable\">未指定 --focus-region；下方完整区域权衡仍保留全部区域。</p>",
        );
    } else {
        for region in &focus_regions {
            html.push_str(&region_detail(
                case,
                &views,
                region,
                &role_pairs,
                &role_entries,
                reference,
                assets,
                out,
                true,
                &default_pair,
            )?);
        }
    }
    html.push_str("</section>");

    html.push_str(&tradeoff_section(
        case,
        &views,
        &regions,
        reference,
        &focus_regions,
        assets,
    )?);
    let focus_ids = focus
        .ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    html.push_str(
        "<section class=\"diagnostic-section diagnostic-regions\"><h3>其他区域与完整定位</h3>",
    );
    for region in &regions {
        if focus_ids.contains(region_id(region).as_str()) {
            continue;
        }
        html.push_str(&region_detail(
            case,
            &views,
            region,
            &role_pairs,
            &role_entries,
            reference,
            assets,
            out,
            false,
            &default_pair,
        )?);
    }
    if regions.is_empty() {
        html.push_str("<p class=\"unavailable\">没有保留区域身份。</p>");
    }
    html.push_str("</section>");
    html.push_str(&source_stage_section(case, &views, assets)?);
    html.push_str("</div></section>");
    Ok(html)
}

fn case_views(case: &Value) -> Vec<Value> {
    let pairs = rows(&case["pairs"]);
    if pairs.is_empty() {
        vec![case.clone()]
    } else {
        pairs.iter().map(|pair| pair_case(case, pair)).collect()
    }
}

fn default_view<'a>(comparison: &Value, views: &'a [Value]) -> &'a Value {
    let requested = comparison
        .get("default_pair")
        .and_then(Value::as_str)
        .unwrap_or("reference");
    views
        .iter()
        .find(|view| view_pair_id(view) == requested)
        .or_else(|| views.first())
        .unwrap_or(&views[0])
}

fn view_pair_id(view: &Value) -> String {
    get(view, "pair_id", get(view, "id", "baseline/candidate")).to_owned()
}

fn canonical_reference(views: &[Value], role: &str, assets: &Assets) -> Value {
    for view in views {
        let reference = if view.get("pair_id").is_some() {
            match role {
                "simulation" => super::role_reference(view, "candidate", assets),
                "source" => super::role_reference(view, "source", assets),
                "reference" => super::role_reference(view, "reference", assets),
                _ => Value::Null,
            }
        } else {
            super::role_reference(view, role, assets)
        };
        if !reference.is_null() && assets.record(&reference).is_some() {
            return reference;
        }
    }
    Value::Null
}

fn role_pairs(entries: &BTreeMap<String, Value>) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for left in entries.keys() {
        for right in entries.keys() {
            if left != right {
                pairs.push((left.clone(), right.clone()));
            }
        }
    }
    pairs
}

fn default_roles(entries: &BTreeMap<String, Value>) -> (String, String) {
    if entries.contains_key("candidate") && entries.contains_key("reference") {
        return ("candidate".into(), "reference".into());
    }
    if entries.contains_key("simulation") && entries.contains_key("reference") {
        return ("simulation".into(), "reference".into());
    }
    if entries.contains_key("baseline") && entries.contains_key("candidate") {
        return ("baseline".into(), "candidate".into());
    }
    if entries.contains_key("source") && entries.contains_key("simulation") {
        return ("source".into(), "simulation".into());
    }
    let mut roles = entries.keys();
    (
        roles.next().cloned().unwrap_or_default(),
        roles.next().cloned().unwrap_or_default(),
    )
}

fn role_label(role: &str) -> &'static str {
    match role {
        "source" => "来源输入",
        "baseline" => "基准模拟",
        "candidate" => "候选模拟",
        "simulation" => "模拟",
        "reference" => "参考图 / 真实扫描",
        _ => "图像",
    }
}

fn display_entry(assets: &mut Assets, entry: &Value) -> Result<Value> {
    let mut display = entry.clone();
    if let Some(jpeg) = display::jpeg_display_copy(assets, entry)? {
        display["fit_entry"] = jpeg;
    } else if let Some(preview) = entry.get("fit_preview").cloned()
        && let Some(fit) = assets.image(&preview)?
    {
        display["fit_entry"] = fit;
    }
    Ok(display)
}

fn aligned_for_roles(
    view: &Value,
    left: &str,
    right: &str,
    reference: Option<&Value>,
    entries: &BTreeMap<String, Value>,
    out: &Path,
) -> bool {
    let declared = if left == "source" || right == "source" {
        view["pixel_aligned"] == true
    } else if left == "reference" || right == "reference" {
        reference
            .and_then(|r| r.get("geometry"))
            .and_then(Value::as_str)
            .is_some_and(|g| g == "pixel_aligned")
    } else {
        true
    };
    if !declared {
        return false;
    }
    let dimensions = |role: &str| {
        entries
            .get(role)
            .and_then(|entry| image::image_dimensions(out.join(get(entry, "path", ""))).ok())
    };
    dimensions(left) == dimensions(right)
}

fn collect_regions(views: &[Value]) -> Vec<Value> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for view in views {
        for region in rows(&view["regions"]) {
            let id = region_id(region);
            if !id.is_empty() && seen.insert(id) {
                result.push(region.clone());
            }
        }
        for region in rows(&view["automatic_regions"]) {
            let id = region_id(region);
            if !id.is_empty() && seen.insert(id) {
                result.push(region.clone());
            }
        }
        for region in rows(&view["reference"]["regions"]) {
            let id = region_id(region);
            if !id.is_empty() && seen.insert(id) {
                result.push(region.clone());
            }
        }
    }
    result
}

fn region_id(region: &Value) -> String {
    get(region, "id", get(region, "region_id", "")).to_owned()
}

fn condition_summary(case: &Value, views: &[Value], assets: &Assets) -> Result<String> {
    let mut roles = Vec::new();
    for role in ROLE_ORDER {
        let reference = canonical_reference(views, role, assets);
        if !reference.is_null() {
            let identity = get(&reference, "id", get(&reference, "path", "unavailable"));
            roles.push(format!("{} ({})", role_label(role), human(identity)));
        }
    }
    let reference = views
        .iter()
        .find_map(|view| view.get("reference").filter(|v| v.is_object()));
    let eligibility = reference
        .map(|r| {
            format!(
                "参考范围：{} · tone 数值：{} · color 数值：{}{}",
                get(r, "scope", "未声明"),
                if r["tone_target_eligible"] == false {
                    "不可用"
                } else {
                    "按保留证据"
                },
                if r["color_diagnostic_eligible"] == false {
                    "不可用"
                } else {
                    "按保留证据"
                },
                r.get("unavailable_reason")
                    .map(|v| format!(" · {}", text(v)))
                    .unwrap_or_default(),
            )
        })
        .unwrap_or_else(|| "参考范围：不可用；当前仅作视觉检查".into());
    Ok(format!(
        "<section class=\"diagnostic-section diagnostic-summary\"><h3>条件与角色摘要</h3><p>案例 <strong>{}</strong> · seed {} · 状态 {} · 对齐：{}。</p><p>保留角色：{}</p><p>{}</p>{}</section>",
        escape(get(case, "case_id", get(case, "id", "unavailable"))),
        field(case, "seed"),
        escape(get(case, "status", "unknown")),
        if views.iter().any(|v| v["pixel_aligned"] == true) {
            "存在声明的像素对齐"
        } else {
            "区域对应或未声明"
        },
        if roles.is_empty() {
            "不可用".into()
        } else {
            roles.join(" · ")
        },
        escape(&eligibility),
        folded(
            "完整条件、角色与参考限制",
            &json!({"case":case,"views":views,"reference":reference})
        ),
    ))
}

fn focus_region_fragment(case: &Value, pair_id: &str, id: &str) -> String {
    navigation_links(
        &navigation_fragment(
            get(case, "case_id", get(case, "id", "unavailable")),
            &case["seed"],
            Some(pair_id),
            Some(id),
        ),
        "区域",
    )
}

// Keep case identity, frozen geometry and retained image roles explicit here.
#[allow(clippy::too_many_arguments)]
fn region_detail(
    case: &Value,
    views: &[Value],
    region: &Value,
    role_pairs: &[(String, String)],
    role_entries: &BTreeMap<String, Value>,
    reference: Option<&Value>,
    assets: &mut Assets,
    out: &Path,
    focus: bool,
    pair_id: &str,
) -> Result<String> {
    let id = region_id(region);
    let label = get(region, "label", &id);
    let mut html = format!(
        "<details class=\"diagnostic-region\" data-review-region data-region-id=\"{}\"{}><summary>{} <small>{}</small></summary>{}",
        escape(&id),
        if focus { " open" } else { "" },
        escape(label),
        escape(&id),
        focus_region_fragment(case, pair_id, &id),
    );
    html.push_str("<div class=\"diagnostic-region-variants\">");
    for (left, right) in role_pairs {
        let key = format!("{}|{}", left, right);
        let visible = left == "candidate" && right == "reference"
            || left == "simulation" && right == "reference";
        html.push_str(&format!(
            "<div class=\"diagnostic-region-variant\" data-role-region=\"{}\"{}>",
            escape(&key),
            if visible { "" } else { " hidden" },
        ));
        let left_crop = crop_for_role(
            views,
            region,
            left,
            reference,
            role_entries.get(left),
            assets,
            out,
        )?;
        let right_crop = crop_for_role(
            views,
            region,
            right,
            reference,
            role_entries.get(right),
            assets,
            out,
        )?;
        html.push_str(&format!(
            "<div class=\"grid\"><div><p><strong>{}</strong> · 坐标 {}</p>{}</div><div><p><strong>{}</strong> · 坐标 {}</p>{}</div></div>",
            escape(role_label(left)),
            coordinate_text(&left_crop),
            image_html(left_crop.as_ref(), role_label(left)),
            escape(role_label(right)),
            coordinate_text(&right_crop),
            image_html(right_crop.as_ref(), role_label(right)),
        ));
        html.push_str(&selected_region_evidence(
            views, reference, &id, left, right,
        ));
        html.push_str("</div>");
    }
    if role_pairs.is_empty() {
        html.push_str("<p class=\"unavailable\">没有两个可用的图像角色。</p>");
    }
    html.push_str("</div></details>");
    Ok(html)
}

fn crop_for_role(
    views: &[Value],
    region: &Value,
    role: &str,
    reference: Option<&Value>,
    entry: Option<&Value>,
    assets: &mut Assets,
    out: &Path,
) -> Result<Option<Value>> {
    let Some(entry) = entry else { return Ok(None) };
    let mut rect = None;
    for view in views {
        let geometry = role_geometry(view, region, role, reference);
        if let Some(geometry) = geometry {
            rect = geometry_rectangle(&geometry, assets)?;
            if rect.is_some() {
                break;
            }
        }
    }
    let Some(crop) = assets.crop(Some(entry), rect)? else {
        return Ok(None);
    };
    let mut crop = crop;
    if let Ok((width, height)) = image::image_dimensions(out.join(get(&crop, "path", ""))) {
        crop["dimensions"] = json!([width, height]);
    }
    Ok(Some(crop))
}

fn role_geometry(
    view: &Value,
    region: &Value,
    role: &str,
    reference: Option<&Value>,
) -> Option<Value> {
    if role == "reference" {
        if let Some(refs) = reference.and_then(|r| r["regions"].as_array())
            && let Some(item) = refs
                .iter()
                .find(|item| region_id(item) == region_id(region))
        {
            if item.get("reference_geometry").is_some() {
                return Some(item["reference_geometry"].clone());
            }
            if item.get("geometry").is_some()
                && reference.is_some_and(|r| r["geometry"] == "pixel_aligned")
            {
                return Some(item["geometry"].clone());
            }
        }
        if reference.is_some_and(|r| r["geometry"] == "pixel_aligned") {
            return region.get("geometry").cloned();
        }
        return None;
    }
    if role == "source" {
        if view["pixel_aligned"] == true {
            return region
                .get("left_geometry")
                .or_else(|| region.get("geometry"))
                .cloned();
        }
        return region.get("source_geometry").cloned();
    }
    if role == "simulation" {
        return region
            .get("right_geometry")
            .or_else(|| region.get("geometry"))
            .cloned();
    }
    region
        .get("geometry")
        .or_else(|| region.get("right_geometry"))
        .cloned()
}

fn coordinate_text(crop: &Option<Value>) -> String {
    crop.as_ref()
        .and_then(|c| c.get("rectangle"))
        .map(text)
        .unwrap_or_else(|| "不可用".into())
}

fn selected_pair_evidence_notice(views: &[Value], left: &str, right: &str) -> String {
    if measured_identity_exists(views, left, right) {
        format!(
            "当前视觉配对：{} ↔ {}；下方重点区域只显示该身份的保留数值。",
            role_label(left),
            role_label(right)
        )
    } else {
        format!(
            "当前视觉配对：{} ↔ {}；该任意视觉配对未测量，数值证据不可用。",
            role_label(left),
            role_label(right)
        )
    }
}

fn selected_region_evidence(
    views: &[Value],
    reference: Option<&Value>,
    region_id: &str,
    left: &str,
    right: &str,
) -> String {
    let mut rows_out = Vec::new();
    for view in views {
        for row in measurements(view) {
            if get(row, "region_id", "") == region_id && row_applies(row, view, left, right) {
                rows_out.push(row.clone());
            }
        }
    }
    let mut html = String::new();
    let model = if left == "reference" {
        right
    } else if right == "reference" {
        left
    } else {
        ""
    };
    if !model.is_empty()
        && let Some(reference) = reference
    {
        html.push_str(&reference_region_diagnostics(reference, region_id, model));
    }
    if rows_out.is_empty() && html.is_empty() {
        html.push_str(
            "<p class=\"unavailable\">该视觉配对的此区域没有保留数值证据；未借用其他配对分数。</p>",
        );
    } else if !rows_out.is_empty() {
        html.push_str(&metric_table(&json!({"measurements":rows_out})));
    }
    html
}

fn reference_region_diagnostics(reference: &Value, region_id: &str, model: &str) -> String {
    let Some(region) = rows(&reference["regions"])
        .iter()
        .find(|region| get(region, "region_id", get(region, "id", "")) == region_id)
    else {
        return String::new();
    };
    let Some(diagnostics) = region["diagnostics"]
        .as_object()
        .filter(|map| !map.is_empty())
    else {
        return String::new();
    };
    let mut html = format!(
        "<details class=\"diagnostic-numeric\"><summary>参考区域诊断 · {}</summary><table><thead><tr><th>指标身份</th><th>域 / 单位</th><th>状态</th><th>值</th></tr></thead><tbody>",
        escape(model)
    );
    for (base, diagnostic) in diagnostics {
        let side = diagnostic["sides"].get(model).unwrap_or(&Value::Null);
        let availability = get(side, "availability", "unavailable");
        let value = if availability == "available" {
            field(side, "value")
        } else {
            field(side, "reason")
        };
        html.push_str(&format!(
            "<tr><td>{}</td><td>{} / {}</td><td>{}</td><td>{}</td></tr>",
            escape(base),
            escape(get(diagnostic, "domain", "unavailable")),
            escape(get(diagnostic, "units", "unavailable")),
            escape(availability),
            value,
        ));
    }
    html.push_str("</tbody></table></details>");
    html
}

fn row_applies(row: &Value, view: &Value, left: &str, right: &str) -> bool {
    let identity = if row.get("pair_id").is_some() {
        let pair_left = get(view, "left_role", "");
        let pair_right = get(view, "right_role", "simulation");
        let pair_left = if pair_left == "input" {
            "source"
        } else if pair_left == "reference" {
            "reference"
        } else {
            pair_left
        };
        (pair_left, pair_right)
    } else {
        let metric = get(row, "metric", "");
        let side = metric.rsplit('.').next().unwrap_or("");
        match side {
            "baseline_distance" => ("baseline", "reference"),
            "candidate_distance" => ("candidate", "reference"),
            "simulation_distance" => ("simulation", "reference"),
            "distance_delta" => ("baseline", "candidate"),
            _ => ("baseline", "candidate"),
        }
    };
    (identity.0 == left && identity.1 == right) || (identity.0 == right && identity.1 == left)
}

fn measured_identity_exists(views: &[Value], left: &str, right: &str) -> bool {
    views.iter().any(|view| {
        measurements(view)
            .iter()
            .any(|row| row_applies(row, view, left, right))
    })
}

fn tradeoff_section(
    case: &Value,
    views: &[Value],
    regions: &[Value],
    reference: Option<&Value>,
    focus_regions: &[Value],
    _assets: &mut Assets,
) -> Result<String> {
    let mut html = String::from(
        "<section class=\"diagnostic-section diagnostic-tradeoffs\"><h3>完整区域权衡与最坏代价</h3><p>所有数值按实际测量身份列出；基准、候选、模拟、参考及区域关系不互相替代。</p>",
    );
    html.push_str(&worst_costs(reference, views));
    let mut all_rows = Vec::new();
    for view in views {
        all_rows.extend(measurements(view).iter().cloned());
    }
    if all_rows.is_empty() {
        html.push_str("<p class=\"unavailable\">没有保留区域测量；当前仅可视觉检查。</p>");
    } else {
        html.push_str("<details><summary>全部区域测量（含裁切与色调）</summary><div class=\"table-scroll\"><table class=\"tradeoff-table\"><thead><tr><th>区域</th><th>群体</th><th>指标身份</th><th>测量配对</th><th>状态</th><th>值 / 原因</th></tr></thead><tbody>");
        for row in &all_rows {
            let id = get(row, "region_id", "unavailable");
            let region = regions.iter().find(|region| region_id(region) == id);
            let population = region
                .map(|region| get(region, "population", get(region, "role", "unavailable")))
                .unwrap_or("unavailable");
            let identity = row_identity_label(row, views);
            let status = get(row, "availability", "available");
            let value = if status == "available" {
                field(row, "value")
            } else {
                field(row, "reason")
            };
            html.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(id),
                escape(population),
                escape(get(row, "metric", "unavailable")),
                escape(&identity),
                escape(status),
                value,
            ));
        }
        html.push_str("</tbody></table></div></details>");
    }
    if let Some(reference) = reference {
        html.push_str(&population_summary_table(reference));
        html.push_str(&region_pair_table(reference));
    }
    let focus_ids = focus_regions.iter().map(region_id).collect::<Vec<_>>();
    if !focus_ids.is_empty() {
        html.push_str(&format!(
            "<p class=\"small\">焦点区域已置于本节之前：{}</p>",
            focus_ids
                .iter()
                .map(|id| escape(id))
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    html.push_str("</section>");
    let _ = case;
    Ok(html)
}

fn row_identity_label(row: &Value, views: &[Value]) -> String {
    if let Some(pair_id) = row.get("pair_id").and_then(Value::as_str)
        && let Some(view) = views.iter().find(|view| view_pair_id(view) == pair_id)
    {
        let left = get(view, "left_role", "");
        let left = if left == "input" {
            "source"
        } else if left == "reference" {
            "reference"
        } else {
            left
        };
        return format!(
            "{} ↔ {}",
            role_label(left),
            role_label(get(view, "right_role", "simulation"))
        );
    }
    let metric = get(row, "metric", "");
    match metric.rsplit('.').next().unwrap_or("") {
        "baseline_distance" => "基准 ↔ 参考".into(),
        "candidate_distance" => "候选 ↔ 参考".into(),
        "simulation_distance" => "模拟 ↔ 参考".into(),
        "distance_delta" => "基准 ↔ 候选".into(),
        _ => "基准 ↔ 候选".into(),
    }
}

fn worst_costs(reference: Option<&Value>, views: &[Value]) -> String {
    let Some(reference) = reference else {
        return String::new();
    };
    let mut html = String::from("<div class=\"worst-costs\"><h4>最坏代价与尾部</h4>");
    for summary in rows(&reference["population_summaries"]) {
        let population = get(summary, "population", "unavailable");
        let metric = get(summary, "metric", "unavailable");
        for (capture, stats) in summary["per_capture"].as_object().into_iter().flatten() {
            let worst = stats
                .get("worst_regression_region_id")
                .or_else(|| stats.get("worst_region_id"))
                .and_then(Value::as_str)
                .unwrap_or("unavailable");
            html.push_str(&format!(
                "<p><strong>{}</strong> · {} · {}：最坏区域 {} · median {} · P95 {} · max {}</p>",
                escape(population),
                escape(metric),
                escape(capture),
                escape(worst),
                field(stats, "median"),
                field(stats, "p95"),
                field(stats, "max"),
            ));
        }
    }
    if html == "<div class=\"worst-costs\"><h4>最坏代价与尾部</h4>" {
        let mut values = BTreeMap::<(&str, &str), (&Value, f64)>::new();
        for view in views {
            for row in measurements(view) {
                if get(row, "availability", "available") != "available" {
                    continue;
                }
                if let Some(value) = row["value"].as_f64() {
                    let metric = get(row, "metric", "unavailable");
                    let key = (metric, get(row, "pair_id", ""));
                    if values.get(&key).is_none_or(|(_, prior)| value > *prior) {
                        values.insert(key, (row, value));
                    }
                }
            }
        }
        for ((metric, _), (row, value)) in values {
            html.push_str(&format!(
                "<p><strong>{}</strong> · {}：最坏区域 {} · {value:.6}</p>",
                escape(metric),
                escape(&row_identity_label(row, views)),
                escape(get(row, "region_id", "unavailable"))
            ));
        }
    }
    html.push_str("</div>");
    html
}

fn population_summary_table(reference: &Value) -> String {
    let summaries = rows(&reference["population_summaries"]);
    if summaries.is_empty() {
        return String::new();
    }
    let mut html = String::from(
        "<details class=\"population-summaries\"><summary>群体摘要（无普遍分数或否决）</summary><div class=\"table-scroll\"><table><thead><tr><th>群体</th><th>指标</th><th>区域</th><th>采集</th><th>median</th><th>P95</th><th>max</th><th>最坏</th></tr></thead><tbody>",
    );
    for summary in summaries {
        for (capture, stats) in summary["per_capture"].as_object().into_iter().flatten() {
            let worst = stats
                .get("worst_regression_region_id")
                .or_else(|| stats.get("worst_region_id"));
            html.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>", escape(get(summary,"population","unavailable")), escape(get(summary,"metric","unavailable")), field(summary,"region_ids"), escape(capture), field(stats,"median"), field(stats,"p95"), field(stats,"max"), worst.map(text).unwrap_or_else(||"unavailable".into())));
        }
    }
    html.push_str("</tbody></table></div></details>");
    html
}

fn region_pair_table(reference: &Value) -> String {
    let pairs = rows(&reference["region_pairs"]);
    if pairs.is_empty() {
        return String::new();
    }
    let mut html = String::from(
        "<details class=\"region-pairs\"><summary>区域配对关系</summary><div class=\"table-scroll\"><table><thead><tr><th>身份</th><th>端点</th><th>指标</th><th>侧</th><th>状态</th><th>值 / 原因</th></tr></thead><tbody>",
    );
    for pair in pairs {
        for (base, diag) in pair["diagnostics"].as_object().into_iter().flatten() {
            for (side, value) in diag["sides"].as_object().into_iter().flatten() {
                let status = get(value, "availability", "unavailable");
                let display = if status == "available" {
                    field(value, "value")
                } else {
                    field(value, "reason")
                };
                html.push_str(&format!("<tr><td>{}</td><td>{} ↔ {}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>", escape(get(pair,"id","unavailable")), escape(get(pair,"left","unavailable")), escape(get(pair,"right","unavailable")), escape(base), escape(side), escape(status), display));
            }
        }
    }
    html.push_str("</tbody></table></div></details>");
    html
}

fn source_stage_section(case: &Value, views: &[Value], assets: &mut Assets) -> Result<String> {
    let mut html = String::from(
        "<section class=\"diagnostic-section diagnostic-provenance\"><h3>来源、几何与阶段证据</h3>",
    );
    html.push_str(&folded("来源输入与几何（观察记录）", &json!({"input":case.get("input"),"source_dimensions":case.get("source_dimensions"),"source_geometry":case.get("source_geometry"),"preparation":case.get("preparation")})));
    let run = case.get("run_evidence").cloned().unwrap_or(Value::Null);
    if run.is_null() {
        html.push_str("<p class=\"unavailable\">未保留 run_evidence；报告仍可从比较快照生成。</p>");
    } else {
        for (role, evidence) in run.as_object().into_iter().flatten() {
            html.push_str(&format!(
                "<details><summary>{} 的运行证据</summary>",
                escape(role)
            ));
            html.push_str(&folded("原样运行证据", evidence));
            html.push_str(&stage_table(evidence, assets)?);
            html.push_str("<p class=\"small\">原生阶段元数据是实际执行的观察证据，不构成物理成因论断。</p></details>");
        }
    }
    for view in views {
        if view.get("left_execution").is_some() || view.get("right_execution").is_some() {
            html.push_str(&folded("比较角色执行与配置", &json!({"pair":view.get("pair_id"),"left":view.get("left_execution"),"right":view.get("right_execution"),"baseline":view.get("baseline_execution"),"candidate":view.get("candidate_execution")})));
        }
    }
    html.push_str("</section>");
    Ok(html)
}

fn stage_table(evidence: &Value, assets: &mut Assets) -> Result<String> {
    let stages = evidence
        .get("observed")
        .and_then(|observed| observed.get("stages"))
        .map(rows)
        .unwrap_or_default();
    let stage_evidence = evidence.get("stage_evidence");
    let buffers = stage_evidence
        .map(|value| rows(&value["buffers"]))
        .unwrap_or_default();
    if stages.is_empty() {
        let availability = stage_evidence
            .map(|value| get(value, "availability", "unavailable"))
            .unwrap_or("unavailable");
        let reason = stage_evidence
            .map(|value| get(value, "reason", "native stage observations unavailable"))
            .unwrap_or("native stage observations unavailable");
        return Ok(format!(
            "<p class=\"unavailable\">阶段观察不可用（{}：{}）；不可从此页推断物理原因。</p>",
            escape(availability),
            escape(reason),
        ));
    }
    let mut html = String::from(
        "<div class=\"table-scroll\"><table><thead><tr><th>阶段</th><th>状态</th><th>域 / 单位</th><th>几何</th><th>artifact</th></tr></thead><tbody>",
    );
    for stage in stages {
        let stage_id = get(stage, "id", "unavailable");
        let mut link = "不可用".to_owned();
        let stage_path = stage.get("path").and_then(Value::as_str);
        let buffer = buffers.iter().find(|buffer| {
            get(buffer, "stage_id", "") == stage_id
                || stage_path.is_some_and(|path| get(buffer, "bundle_path", "") == path)
        });
        if let Some(buffer) = buffer
            && let Some(entry) = assets.copy(buffer)?
            && entry["availability"] == "available"
        {
            link = format!(
                "<a href=\"{}\">{}</a>",
                url(&entry),
                escape(get(&entry, "path", "artifact")),
            );
        }
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{} / {}</td><td>{}</td><td>{}</td></tr>",
            escape(stage_id),
            escape(get(stage, "availability", "unavailable")),
            escape(get(stage, "domain", "unavailable")),
            escape(get(stage, "units", "unavailable")),
            field(stage, "geometry"),
            link,
        ));
    }
    html.push_str("</tbody></table></div>");
    Ok(html)
}
