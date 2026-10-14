//! Portable offline presentation of retained evidence; no rendering or measurement.
use crate::common::{Error, Result};
use serde_json::{Value, json};
pub(crate) const CSS: &str = include_str!("../../assets/report.css");
pub(crate) const JS: &str = include_str!("../../assets/report.js");
pub(crate) fn text(value: &Value) -> String {
    let value = match value {
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string_pretty(value).unwrap_or_default()
        }
        _ => value.to_string(),
    };
    escape(&value)
}
pub(crate) fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
pub(crate) fn get<'a>(value: &'a Value, key: &str, default: &'a str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or(default)
}
pub(crate) fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .map(text)
        .unwrap_or_else(|| "unavailable".into())
}
pub(crate) fn rows(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub(crate) fn measurements(case: &Value) -> &[Value] {
    let value = case.get("metrics").or_else(|| case.get("measurements"));
    match value {
        Some(Value::Object(map)) => map.get("measurements").map(rows).unwrap_or(&[]),
        Some(v) => rows(v),
        None => &[],
    }
}
pub(crate) fn human(id: &str) -> String {
    match id {
        "color_chart_30mp" => "30 MP 色彩测试图".into(),
        "color_chart" => "色彩测试图".into(),
        "kodak_dc50_scene" => "自然场景 · Kodak DC50".into(),
        "exposure_response" => "灰阶响应".into(),
        "grain_fields" => "平场颗粒".into(),
        "halation_point" => "点光源与光晕".into(),
        "edge_response" => "边缘响应".into(),
        "skin" => "肤色".into(),
        "shadow" => "阴影".into(),
        "texture" => "纹理".into(),
        "neutral" => "中性灰".into(),
        _ => id
            .replace(['_', '-'], " ")
            .split_whitespace()
            .map(|p| {
                let mut c = p.chars();
                c.next().map_or(String::new(), |v| {
                    v.to_uppercase().collect::<String>() + c.as_str()
                })
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}
pub(crate) fn url(entry: &Value) -> String {
    let mut encoded = String::new();
    for b in get(entry, "path", "").bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    escape(&encoded)
}
pub(crate) fn normalized_artifacts(value: &Value) -> Result<Vec<Value>> {
    match value {
        Value::Array(v) => Ok(v.clone()),
        Value::Object(v) => Ok(v
            .iter()
            .map(|(id, v)| {
                let mut r = if v.is_object() {
                    v.clone()
                } else {
                    json!({"path":v})
                };
                r["id"] = json!(id);
                r
            })
            .collect()),
        Value::Null => Ok(vec![]),
        _ => Err(Error::new(
            "invalid_document",
            "artifacts must be an array or object",
        )),
    }
}
