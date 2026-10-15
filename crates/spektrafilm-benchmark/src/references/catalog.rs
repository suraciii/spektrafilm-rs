//! Canonical identity registry for reference diagnostics.
//!
//! Measurement production and policy metric selection share this list; policy
//! validation must never grow a second hand-written metric list. Every
//! diagnostic row and review payload takes its identity, role, units and
//! estimator text from the entries below.

use serde_json::Value;

use super::io::err;
use crate::common::Result;

/// Identity and frozen description of one reference diagnostic metric.
pub struct Metric {
    pub id: &'static str,
    pub role: &'static str,
    pub units: &'static str,
    pub estimator: &'static str,
}

/// Side name of the retained reference relation anchor row.
pub const RELATION_SIDE: &str = "reference_relation";

pub const PATCH_DE00: Metric = Metric {
    id: "reference.patch_delta_e00",
    role: "patch_color_difference",
    units: "delta E 00",
    estimator: "DE00 between independent Lab channel medians; robust interior patch",
};
pub const PATCH_DE00_EQUAL_L: Metric = Metric {
    id: "reference.patch_delta_e00_equal_l",
    role: "equal_lightness_chromatic_diagnostic",
    units: "delta E 00",
    estimator: "DE00 between independent Lab channel medians with model L* set to reference L* inside the metric only; images are never modified; robust interior patch; diagnostic only",
};
pub const DISTRIBUTION_L: Metric = Metric {
    id: "reference.distribution_l_star",
    role: "lightness_distribution_diagnostic",
    units: "L*",
    estimator: "mean absolute distance of 101 independent marginal lightness quantiles; linear quantiles",
};
pub const PATCH_CHROMA_AB: Metric = Metric {
    id: "reference.patch_chroma_ab",
    role: "chroma_distance_diagnostic",
    units: "Lab a*/b* distance",
    estimator: "distance between independent median a*/b*; excludes lightness; diagnostic only",
};
pub const PATCH_HUE_SHIFT: Metric = Metric {
    id: "reference.patch_hue_shift",
    role: "signed_hue_shift_diagnostic",
    units: "degrees",
    estimator: "shortest circular hue-angle difference of independent Lab channel medians, model minus reference, degrees in (-180,180]; both chromas must meet the authored hue_chroma_floor; the version distance_delta side is the arithmetic candidate-minus-baseline difference of the reference-relative signed shifts, not the shortest candidate-to-baseline hue change, so a wrap-branch difference can approach 360 degrees",
};
pub const PATCH_HUE_ERROR: Metric = Metric {
    id: "reference.patch_hue_error",
    role: "absolute_hue_error_diagnostic",
    units: "degrees",
    estimator: "absolute shortest circular hue-angle difference of independent Lab channel medians, degrees in [0,180]; both chromas must meet the authored hue_chroma_floor",
};
pub const PAIR_SEPARATION: Metric = Metric {
    id: "reference.pair_separation_delta_e00",
    role: "pair_separation",
    units: "delta E 00",
    estimator: "DE00 between the pair endpoints' independent Lab channel medians within one capture; robust interior patches",
};
pub const PAIR_SEPARATION_CHANGE: Metric = Metric {
    id: "reference.pair_separation_change",
    role: "signed_separation_change",
    units: "delta E 00",
    estimator: "signed model pair separation minus reference pair separation; negative means the model pair is closer than the reference relation; a close pair alone does not imply correct endpoint colors",
};
pub const PAIR_SEPARATION_DEPARTURE: Metric = Metric {
    id: "reference.pair_separation_departure",
    role: "absolute_relation_departure",
    units: "delta E 00",
    estimator: "absolute value of model pair separation minus reference pair separation; a close pair alone does not imply correct endpoint colors",
};

/// Regional diagnostics measured per case region from robust Lab medians.
pub const REGIONAL_METRICS: &[&Metric] = &[
    &PATCH_DE00,
    &PATCH_DE00_EQUAL_L,
    &DISTRIBUTION_L,
    &PATCH_CHROMA_AB,
    &PATCH_HUE_SHIFT,
    &PATCH_HUE_ERROR,
];
/// Region-pair diagnostics measured per authored region pair.
pub const PAIR_METRICS: &[&Metric] = &[
    &PAIR_SEPARATION,
    &PAIR_SEPARATION_CHANGE,
    &PAIR_SEPARATION_DEPARTURE,
];

/// Metrics summarized across whole-chart populations; keeps signed hue shift
/// (directional) out of cross-region cost summaries.
pub const SUMMARIZED_METRICS: &[&Metric] = &[
    &PATCH_DE00,
    &PATCH_DE00_EQUAL_L,
    &DISTRIBUTION_L,
    &PATCH_CHROMA_AB,
    &PATCH_HUE_ERROR,
];

pub fn regional_metric(id: &str) -> Option<&'static Metric> {
    REGIONAL_METRICS.iter().copied().find(|m| m.id == id)
}
pub fn pair_metric(id: &str) -> Option<&'static Metric> {
    PAIR_METRICS.iter().copied().find(|m| m.id == id)
}

/// Whether the metric requires both chromas to meet the authored floor.
pub fn hue_metric(id: &str) -> bool {
    id == PATCH_HUE_SHIFT.id || id == PATCH_HUE_ERROR.id
}

const SIDES: [&str; 4] = [
    "baseline_distance",
    "candidate_distance",
    "distance_delta",
    "simulation_distance",
];

/// Strip a supported side suffix, retaining the metric base. Only the pair
/// separation metric emits a reference-relation anchor.
pub fn base_id(metric: &str) -> Option<&str> {
    let (base, side) = metric.rsplit_once('.')?;
    (SIDES.contains(&side) || (side == RELATION_SIDE && base == PAIR_SEPARATION.id)).then_some(base)
}

const ALIGNED_METRICS: [&str; 10] = [
    "reference.delta_e00.median",
    "reference.delta_e00.p95",
    "reference.delta_e00.p99",
    "reference.delta_e00.max",
    "reference.tone_l_star.q05",
    "reference.tone_l_star.q25",
    "reference.tone_l_star.q50",
    "reference.tone_l_star.q75",
    "reference.tone_l_star.q95",
    "reference.chroma_ab.median",
];

/// Authorised hue floor of the protocol, or `None` when hue diagnostics are
/// not selected. A present floor must be finite and strictly positive.
pub fn hue_floor(protocol: &Value) -> Result<Option<f64>> {
    let Some(color) = protocol.get("color").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let floor = color
        .get("hue_chroma_floor")
        .and_then(Value::as_f64)
        .filter(|f| f.is_finite() && *f > 0.)
        .ok_or_else(|| {
            err(
                "invalid_protocol",
                "color.hue_chroma_floor must be a finite positive number",
            )
        })?;
    Ok(Some(floor))
}

/// Whether the named reference metric base can be produced for this case,
/// region and protocol selection. This is the single policy-selection check;
/// it reads the same definitions the measurement code emits.
pub fn metric_supported(base: &str, case: &Value, region_id: &str, protocol: &Value) -> bool {
    if let Some(curve) = base.strip_prefix("reference.material.") {
        return case["reference"]["scope"].as_str() == Some("material")
            && region_id == "full_image"
            && case["probe_response"]["curves"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|row| row["id"].as_str() == Some(curve));
    }
    if base.starts_with("reference.pair_") {
        return pair_metric(base).is_some()
            && case["reference"]["scope"].as_str() == Some("regional")
            && case
                .get("region_pairs")
                .and_then(Value::as_array)
                .is_some_and(|pairs| pairs.iter().any(|p| p["id"].as_str() == Some(region_id)));
    }
    if let Some(metric) = regional_metric(base) {
        return case["reference"]["scope"].as_str() == Some("regional")
            && case["reference"]["regions"]
                .as_array()
                .is_some_and(|regions| {
                    regions.iter().any(|r| r["id"].as_str() == Some(region_id))
                })
            && (!hue_metric(metric.id) || hue_floor(protocol).unwrap_or(None).is_some());
    }
    case["reference"]["scope"].as_str() == Some("aligned_scene") && ALIGNED_METRICS.contains(&base)
}

/// Summary population label of an authored region role. Roles outside the
/// chart populations keep their own role string as the population label.
pub fn population_label(role: &str) -> &str {
    match role {
        "color" | "skin" => "color",
        "neutral" => "neutral",
        "shadow" | "highlight" => "endpoint",
        other => other,
    }
}
