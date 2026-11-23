//! Portable offline presentation of retained evidence; no rendering or measurement.
use crate::common::{Error, Result};
use serde_json::{Value, json};
pub(crate) const CSS: &str = include_str!("../../assets/report.css");
/// Bootstrap 5.3.3, unmodified. It owns the dropdown, button and focus
/// behaviour; the page owns the appearance, through Bootstrap's variables.
pub(crate) const VENDOR_CSS: &str = include_str!("../../assets/vendor/bootstrap.min.css");
pub(crate) const VENDOR_JS: &str = include_str!("../../assets/vendor/bootstrap.bundle.min.js");
pub(crate) const JS: &str = include_str!("../../assets/report.js");
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
pub(crate) fn rows(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
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
        "color_space" => "色彩空间".into(),
        "cctf_decoding" => "CCTF 解码".into(),
        "dimensions" => "尺寸".into(),
        "pixel_aligned" => "像素对齐".into(),
        "declaration" => "色彩声明".into(),
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
