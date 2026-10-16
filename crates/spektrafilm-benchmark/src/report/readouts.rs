//! Presentation of retained regional values; no color or image computation.
use super::escape::{escape, field, get, rows};
use super::html::folded;
use serde_json::Value;
use std::fmt::Write;

fn number(value: &Value) -> String {
    value.as_f64().map_or_else(
        || "不可用".into(),
        |v| {
            if v != 0.0 && v.abs() < 0.0001 {
                format!("{v:.3e}")
            } else {
                format!("{v:.4}")
            }
        },
    )
}

fn vector(value: &Value) -> String {
    rows(value)
        .iter()
        .map(number)
        .collect::<Vec<_>>()
        .join(", ")
}

fn observation(value: &Value) -> String {
    if value["availability"] == "available" {
        if value["value"].is_array() {
            vector(&value["value"])
        } else {
            number(&value["value"])
        }
    } else {
        format!("不可用：{}", escape(get(value, "reason", "not_retained")))
    }
}

fn role(value: &str) -> &str {
    match value {
        "baseline" => "基准",
        "candidate" => "候选",
        "input" => "输入",
        "simulation" => "模拟",
        "reference" => "参考",
        other => other,
    }
}

pub(super) fn region(case: &Value, region_id: &str) -> String {
    let mut html = String::from(
        "<details data-region-readouts><summary>色块坐标与偏移</summary><p class=\"small\">偏移 = 终点 − 起点；+L* 更亮，+a* 偏红、−a* 偏绿，+b* 偏黄、−b* 偏蓝。ΔC* 为彩度变化；Δh 为最短色相角变化（度）。D65 / CIE 1931 2°。</p>",
    );
    let mut found = false;
    for readout in rows(&case["region_readouts"])
        .iter()
        .filter(|r| r["region_id"] == region_id)
    {
        found = true;
        let estimator = match get(readout, "estimator", "") {
            "bounded_lab_channel_median" => "显示域 Lab 逐通道中位数",
            "unclipped_linear_srgb_mean" => "未裁切线性 sRGB 均值 → Lab",
            other => other,
        };
        let _ = write!(
            html,
            "<h4>{} · {}</h4><p class=\"small\">{}</p>",
            escape(estimator),
            field(readout, "pair_id"),
            field(readout, "domain")
        );
        if readout["availability"] != "available" {
            let _ = write!(
                html,
                "<p class=\"unavailable\">不可用：{}</p>",
                field(readout, "reason")
            );
            continue;
        }
        html.push_str("<div class=\"table-scroll\"><table><thead><tr><th>图像角色</th><th>L*</th><th>a*</th><th>b*</th><th>C*</th><th>h（°）</th><th>像素数</th></tr></thead><tbody>");
        for (name, side) in readout["sides"].as_object().into_iter().flatten() {
            let _ = write!(
                html,
                "<tr><th>{}</th><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(role(name)),
                number(&side["lab"][0]),
                number(&side["lab"][1]),
                number(&side["lab"][2]),
                number(&side["lch"]["c_star"]),
                observation(&side["lch"]["h_degrees"]),
                field(side, "sample_count")
            );
        }
        html.push_str("</tbody></table></div><div class=\"table-scroll\"><table><thead><tr><th>起点 → 终点</th><th>Δ[L*, a*, b*]</th><th>ΔC*</th><th>Δh（°）</th><th>ΔE76</th><th>ΔE00</th><th>等 L* ΔE00</th></tr></thead><tbody>");
        for displacement in rows(&readout["displacements"]) {
            let _ = write!(
                html,
                "<tr><th>{} → {}</th>",
                escape(role(get(displacement, "from_role", ""))),
                escape(role(get(displacement, "to_role", "")))
            );
            for key in [
                "delta_lab",
                "delta_c_star",
                "delta_h_degrees",
                "delta_e76",
                "delta_e00",
                "equal_l_delta_e00",
            ] {
                let _ = write!(html, "<td>{}</td>", observation(&displacement[key]));
            }
            html.push_str("</tr>");
        }
        html.push_str("</tbody></table></div>");
        html.push_str(&folded("完整精度、采样范围与证据", readout));
    }
    if !found {
        html.push_str("<p class=\"unavailable\">未保留该区域的色彩读数；报告不会重新测量。</p>");
    }
    html.push_str("</details><details data-stage-readouts><summary>内部阶段：原生通道统计</summary><p class=\"small\">通道顺序和单位以各阶段为准；CMY 层曝光／密度不是 Lab。均值不作曝光补偿；f64 存储不代表 f64 计算。</p><div class=\"table-scroll\"><table><thead><tr><th>运行 / 阶段</th><th>域 / 单位</th><th>通道</th><th>均值</th><th>最小值</th><th>最大值</th><th>像素 / 计算精度</th></tr></thead><tbody>");
    let mut stages_found = false;
    for readout in rows(&case["stage_readouts"])
        .iter()
        .filter(|r| r["region_id"] == region_id)
    {
        stages_found = true;
        let _ = write!(
            html,
            "<tr><th>{} / {}</th><td>{} / {}</td><td>{}</td>",
            escape(role(get(readout, "run_role", ""))),
            field(readout, "stage_id"),
            field(readout, "domain"),
            field(readout, "units"),
            field(readout, "channel_order")
        );
        if readout["availability"] == "available" {
            for key in ["channel_mean", "channel_min", "channel_max"] {
                let _ = write!(html, "<td>{}</td>", vector(&readout[key]));
            }
        } else {
            let _ = write!(
                html,
                "<td colspan=\"3\">{}：{}</td>",
                field(readout, "availability"),
                field(readout, "reason")
            );
        }
        let _ = write!(
            html,
            "<td>{} / {}</td></tr>",
            field(readout, "sample_count"),
            field(readout, "host_precision")
        );
    }
    html.push_str("</tbody></table></div>");
    if !stages_found {
        html.push_str("<p class=\"unavailable\">未保留该区域的阶段读数；报告不会重新测量。</p>");
    }
    for readout in rows(&case["stage_readouts"])
        .iter()
        .filter(|r| r["region_id"] == region_id)
    {
        html.push_str(&folded(
            &format!(
                "{} / {}：完整精度与坐标证据",
                get(readout, "run_role", ""),
                get(readout, "stage_id", "")
            ),
            readout,
        ));
    }
    html.push_str("</details>");
    html
}
