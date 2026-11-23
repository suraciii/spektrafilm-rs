use super::io::{err, field, read_json, truthy};
use crate::common::Result;
use serde_json::Value;
use std::path::Path;

#[derive(Clone, Copy)]
pub enum Interpolation {
    Nearest,
    Bilinear,
}
pub struct FrozenTransform {
    inverse: [[f64; 3]; 3],
    interpolation: Interpolation,
}
impl FrozenTransform {
    pub(super) fn parse(value: &Value) -> Result<Self> {
        if value.get("direction").and_then(Value::as_str) != Some("reference_to_render")
            || value.get("coordinate_system").and_then(Value::as_str) != Some("pixel_centers")
        {
            return Err(err(
                "invalid_correspondence",
                "Transform must declare direction, pixel centers and interpolation",
            ));
        }
        let interpolation = match value.get("interpolation").and_then(Value::as_str) {
            Some("nearest") => Interpolation::Nearest,
            Some("bilinear") => Interpolation::Bilinear,
            _ => {
                return Err(err(
                    "invalid_correspondence",
                    "Transform must declare direction, pixel centers and interpolation",
                ));
            }
        };
        let a = value
            .get("matrix")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                err(
                    "invalid_correspondence",
                    "Transform matrix must be finite and invertible 3 x 3",
                )
            })?;
        if a.len() != 3 || a.iter().any(|r| r.as_array().is_none_or(|a| a.len() != 3)) {
            return Err(err(
                "invalid_correspondence",
                "Transform matrix must be finite and invertible 3 x 3",
            ));
        }
        let m: [[f64; 3]; 3] =
            std::array::from_fn(|i| std::array::from_fn(|j| a[i][j].as_f64().unwrap_or(f64::NAN)));
        let d = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        if m.iter().flatten().any(|x| !x.is_finite()) || !d.is_finite() || d.abs() < 1e-12 {
            return Err(err(
                "invalid_correspondence",
                "Transform matrix must be finite and invertible 3 x 3",
            ));
        }
        let inverse = [
            [
                (m[1][1] * m[2][2] - m[1][2] * m[2][1]) / d,
                (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / d,
                (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / d,
            ],
            [
                (m[1][2] * m[2][0] - m[1][0] * m[2][2]) / d,
                (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / d,
                (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / d,
            ],
            [
                (m[1][0] * m[2][1] - m[1][1] * m[2][0]) / d,
                (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / d,
                (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / d,
            ],
        ];
        Ok(Self {
            inverse,
            interpolation,
        })
    }
    pub(super) fn warp(
        &self,
        values: &[[f64; 3]],
        source: [usize; 2],
        target: [usize; 2],
    ) -> Result<(Vec<[f64; 3]>, Vec<bool>)> {
        let [sw, sh] = source;
        let [w, h] = target;
        if sw == 0 || sh == 0 || values.len() != sw * sh {
            return Err(err(
                "invalid_correspondence",
                "Reference samples and dimensions differ",
            ));
        }
        let m = self.inverse;
        let mut warped = Vec::with_capacity(w * h);
        let mut support = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let px = x as f64 + 0.5;
                let py = y as f64 + 0.5;
                let q = [
                    m[0][0] * px + m[0][1] * py + m[0][2],
                    m[1][0] * px + m[1][1] * py + m[1][2],
                    m[2][0] * px + m[2][1] * py + m[2][2],
                ];
                let divisor_good = q[2].is_finite() && q[2].abs() > 1e-12;
                let divisor = if divisor_good { q[2] } else { 1. };
                let sx = q[0] / divisor - 0.5;
                let sy = q[1] / divisor - 0.5;
                let good = divisor_good
                    && sx.is_finite()
                    && sy.is_finite()
                    && sx >= 0.
                    && sy >= 0.
                    && sx <= (sw - 1) as f64
                    && sy <= (sh - 1) as f64;
                let sx = if good { sx } else { 0. };
                let sy = if good { sy } else { 0. };
                support.push(good);
                let p = match self.interpolation {
                    Interpolation::Nearest => {
                        values[(sy + 0.5).floor() as usize * sw + (sx + 0.5).floor() as usize]
                    }
                    Interpolation::Bilinear => {
                        let x0 = sx.floor() as usize;
                        let y0 = sy.floor() as usize;
                        let x1 = (x0 + 1).min(sw - 1);
                        let y1 = (y0 + 1).min(sh - 1);
                        let fx = sx - x0 as f64;
                        let fy = sy - y0 as f64;
                        std::array::from_fn(|c| {
                            (1. - fy)
                                * ((1. - fx) * values[y0 * sw + x0][c]
                                    + fx * values[y0 * sw + x1][c])
                                + fy * ((1. - fx) * values[y1 * sw + x0][c]
                                    + fx * values[y1 * sw + x1][c])
                        })
                    }
                };
                warped.push(p);
            }
        }
        Ok((warped, support))
    }
}

pub fn validate_correspondence_documents(reference: &Value, base: &Path) -> Result<()> {
    if reference["scope"] != "aligned_scene" {
        return Ok(());
    }
    if reference.get("transform").is_some_and(truthy)
        == reference.get("aligned_derivative").is_some_and(truthy)
    {
        return Err(err(
            "invalid_correspondence",
            "Provide exactly one frozen transform or aligned derivative",
        ));
    }
    if let Some(transform) = reference.get("transform") {
        FrozenTransform::parse(&read_json(transform, base)?)?;
    }
    let review = read_json(field(reference, "correspondence_review")?, base)?;
    if review["status"] != "approved"
        || review["reviewer"]
            .as_str()
            .is_none_or(|v| v.trim().is_empty())
        || !review["limitations"].is_array()
        || !review["excluded_regions"].is_array()
    {
        return Err(err(
            "invalid_correspondence",
            "Correspondence requires approved reviewer, exclusions and limitations",
        ));
    }
    for exclusion in review["excluded_regions"].as_array().unwrap() {
        if !["motion", "occlusion", "parallax", "invalid_border"]
            .contains(&exclusion["reason"].as_str().unwrap_or(""))
            || !exclusion
                .get("geometry")
                .is_some_and(|v| v.is_object() || v.is_array())
        {
            return Err(err(
                "invalid_correspondence",
                "Correspondence exclusion requires a supported reason and geometry",
            ));
        }
    }
    Ok(())
}
