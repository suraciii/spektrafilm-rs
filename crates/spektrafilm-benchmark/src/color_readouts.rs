//! Comparison-owned regional color readouts.
//!
//! Every declared region receives two separately named diagnostic centers:
//! the componentwise median of the bounded display Lab population that the
//! ordinary patch diagnostics already use, and the arithmetic mean of the
//! unclipped linear sRGB population converted once to Lab D65. Neither
//! estimator replaces an existing scalar metric or a reference eligibility
//! decision; they retain additional coordinates with signed, individually
//! gated to-minus-from displacements.

use crate::common::Result;
use crate::image_data;
use crate::metrics::median;
use serde_json::{Map, Value, json};
use spektrafilm_math::colorspace::SRGB_TO_XYZ_F64;
use spektrafilm_math::lab::{delta_e_2000, xyz_to_lab};

/// Stable identity of this readout payload shape.
pub(crate) const READOUT_VERSION: &str = "color-region-v1";
pub(crate) const WHITE_POINT: &str = "D65";
pub(crate) const OBSERVER: &str = "CIE1931_2deg";

/// Componentwise median of the bounded display Lab population.
const MEDIAN_ESTIMATOR: &str = "bounded_lab_channel_median";
const MEDIAN_DOMAIN: &str = "bounded_srgb_d65_lab";
/// Arithmetic mean of the unclipped linear sRGB population, then Lab D65.
const MEAN_ESTIMATOR: &str = "unclipped_linear_srgb_mean";
const MEAN_DOMAIN: &str = "unclipped_linear_srgb_mean_lab";

const TONE_REASON: &str = "tone_contaminated_reference";
const CHROMA_REASON: &str = "unknown_or_graded_color_reference";
const NO_FLOOR_REASON: &str = "hue_diagnostics_not_selected";
const BELOW_FLOOR_REASON: &str = "below_chroma_floor";
const EMPTY_REASON: &str = "empty_reference_population";
const COMPUTATION_REASON: &str = "computation_failure";

/// Chroma radius of a Lab point, `hypot(a*, b*)`.
pub(crate) fn chroma_ab(p: [f64; 3]) -> f64 {
    p[1].hypot(p[2])
}

/// Hue angle `atan2(b*, a*)` in degrees, in (-180, 180].
pub(crate) fn hue_angle(p: [f64; 3]) -> f64 {
    p[2].atan2(p[1]).to_degrees()
}

/// Shortest circular hue-angle difference, model minus reference, degrees in
/// (-180, 180].
pub(crate) fn shortest_hue_shift(model: [f64; 3], reference: [f64; 3]) -> f64 {
    let raw = hue_angle(model) - hue_angle(reference);
    let wrapped = (raw + 180.).rem_euclid(360.) - 180.;
    if wrapped == -180. { 180. } else { wrapped }
}

#[derive(Clone, Copy)]
enum Estimator {
    MedianLab,
    LinearMean,
}
impl Estimator {
    const ALL: [Self; 2] = [Self::MedianLab, Self::LinearMean];
    fn id(self) -> &'static str {
        match self {
            Self::MedianLab => MEDIAN_ESTIMATOR,
            Self::LinearMean => MEAN_ESTIMATOR,
        }
    }
    fn domain(self) -> &'static str {
        match self {
            Self::MedianLab => MEDIAN_DOMAIN,
            Self::LinearMean => MEAN_DOMAIN,
        }
    }
}

/// Retained eligibility of the comparison the readout belongs to. Ordinary
/// Lab vectors, DE76, DE00 and lightness displacement require tone; chroma
/// distance, equal-L DE00 and hue require chroma, and hue additionally the
/// authored floor.
#[derive(Clone, Copy)]
pub(crate) struct Gating {
    pub tone: bool,
    pub chroma: bool,
    pub hue_floor: Option<f64>,
}

/// One side of a regional readout: its population, its already computed
/// bounded display Lab raster, its unclipped display sRGB raster, and — when
/// the caller already computed the componentwise median — that center.
pub(crate) struct Side<'a> {
    pub role: &'a str,
    pub geometry: &'a Value,
    pub lab: &'a [[f64; 3]],
    pub encoded: &'a [[f64; 3]],
    pub linear: Option<&'a [[f64; 3]]>,
    pub population: &'a [usize],
    pub median: Option<[f64; 3]>,
}

fn hue_gate(gating: &Gating, chroma: f64) -> Option<&'static str> {
    match gating.hue_floor {
        None => Some(NO_FLOOR_REASON),
        Some(_) if !gating.chroma => Some(CHROMA_REASON),
        Some(floor) if chroma < floor => Some(BELOW_FLOOR_REASON),
        Some(_) => None,
    }
}

fn observation(value: Option<f64>, reason: Option<&str>) -> Value {
    match value {
        Some(v) if v.is_finite() => json!({"availability":"available","value":v}),
        _ => json!({"availability":"unavailable","reason":reason.unwrap_or(COMPUTATION_REASON)}),
    }
}

fn vector_observation(value: [f64; 3], reason: Option<&str>) -> Value {
    match reason {
        Some(_) => {
            json!({"availability":"unavailable","reason":reason.unwrap_or(COMPUTATION_REASON)})
        }
        None if value.iter().all(|v| v.is_finite()) => {
            json!({"availability":"available","value":[value[0],value[1],value[2]]})
        }
        None => json!({"availability":"unavailable","reason":COMPUTATION_REASON}),
    }
}

fn median_center(side: &Side<'_>, population: &[usize]) -> Option<[f64; 3]> {
    if let Some(center) = side.median {
        return Some(center);
    }
    if population.is_empty() {
        return None;
    }
    let mut columns: [Vec<f64>; 3] =
        std::array::from_fn(|c| population.iter().map(|&i| side.lab[i][c]).collect());
    let mut center = [0.0f64; 3];
    for (channel, values) in columns.iter_mut().enumerate() {
        center[channel] = median(values)?;
    }
    Some(center)
}

/// Arithmetic mean of the unclipped linear sRGB population, then one shared
/// XYZ -> Lab D65 conversion. Never clamps and never routes through the
/// clamping encoded-to-Lab helper.
fn mean_center(side: &Side<'_>, population: &[usize]) -> Option<([f64; 3], [f64; 3])> {
    if population.is_empty() {
        return None;
    }
    let mut sum = [0.0f64; 3];
    for &i in population {
        let pixel = side.linear.map_or_else(
            || side.encoded[i].map(image_data::srgb_decode),
            |linear| linear[i],
        );
        for (channel, total) in sum.iter_mut().enumerate() {
            *total += pixel[channel];
        }
    }
    let linear = sum.map(|total| total / population.len() as f64);
    let xyz: [f64; 3] = std::array::from_fn(|row| {
        SRGB_TO_XYZ_F64[row]
            .iter()
            .zip(linear)
            .map(|(m, v)| m * v)
            .sum::<f64>()
    });
    Some((xyz_to_lab(xyz), linear))
}

fn center(
    estimator: Estimator,
    side: &Side<'_>,
    population: &[usize],
) -> Result<([f64; 3], Option<[f64; 3]>), &'static str> {
    let (lab, linear) = match estimator {
        Estimator::MedianLab => (median_center(side, population), None),
        Estimator::LinearMean => match mean_center(side, population) {
            Some((lab, linear)) => (Some(lab), Some(linear)),
            None => (None, None),
        },
    };
    let lab = lab.ok_or(EMPTY_REASON)?;
    if !lab.iter().all(|v| v.is_finite())
        || linear.is_some_and(|l| !l.iter().all(|v| v.is_finite()))
    {
        return Err(COMPUTATION_REASON);
    }
    Ok((lab, linear))
}

fn side_payload(
    side: &Side<'_>,
    lab: [f64; 3],
    linear: Option<[f64; 3]>,
    count: usize,
    coordinate_space: &str,
    gating: &Gating,
) -> Value {
    let chroma = chroma_ab(lab);
    let reason = hue_gate(gating, chroma);
    let mut payload = json!({
        "lab": [lab[0], lab[1], lab[2]],
        "lch": {
            "l_star": lab[0],
            "c_star": chroma,
            "h_degrees": observation(reason.is_none().then(|| hue_angle(lab)), reason),
        },
        "geometry": side.geometry.clone(),
        "coordinate_space": coordinate_space,
        "sample_count": count,
    });
    if let Some(linear) = linear {
        payload["mean_linear_srgb"] = json!([linear[0], linear[1], linear[2]]);
    }
    payload
}

fn displacement_payload(
    from_role: &str,
    to_role: &str,
    from: [f64; 3],
    to: [f64; 3],
    gating: &Gating,
) -> Value {
    let tone_reason = (!gating.tone).then_some(TONE_REASON);
    let chroma_reason = (!gating.chroma).then_some(CHROMA_REASON);
    let hue_reason = hue_gate(gating, chroma_ab(from)).or_else(|| hue_gate(gating, chroma_ab(to)));
    let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    json!({
        "from_role": from_role,
        "to_role": to_role,
        "delta_lab": vector_observation(delta, tone_reason),
        "delta_l_star": observation(tone_reason.is_none().then(|| delta[0]), tone_reason),
        "delta_c_star": observation(
            chroma_reason.is_none().then(|| chroma_ab(to) - chroma_ab(from)),
            chroma_reason,
        ),
        "delta_h_degrees": observation(
            hue_reason.is_none().then(|| shortest_hue_shift(to, from)),
            hue_reason,
        ),
        "delta_e76": observation(
            tone_reason
                .is_none()
                .then(|| delta[0].hypot(delta[1]).hypot(delta[2])),
            tone_reason,
        ),
        "delta_e00": observation(tone_reason.is_none().then(|| delta_e_2000(from, to)), tone_reason),
        "equal_l_delta_e00": observation(
            chroma_reason.is_none().then(|| delta_e_2000([from[0], to[1], to[2]], from)),
            chroma_reason,
        ),
    })
}

fn unavailable(region_id: &str, estimator: Estimator, reason: &str) -> Value {
    json!({
        "region_id": region_id,
        "estimator": estimator.id(),
        "domain": estimator.domain(),
        "readout_version": READOUT_VERSION,
        "white_point": WHITE_POINT,
        "observer": OBSERVER,
        "availability": "unavailable",
        "reason": reason,
        "artifact_refs": [],
    })
}

/// Both estimator records for a region that cannot produce coordinates, for
/// example because the colour family was not selected or the reference scope
/// does not admit a colour population.
pub(crate) fn unavailable_pair(region_id: &str, reason: &str) -> Vec<Value> {
    Estimator::ALL
        .into_iter()
        .map(|estimator| unavailable(region_id, estimator, reason))
        .collect()
}

/// Retained centers and signed displacements for one declared region.
///
/// `sides[0]` is the displacement source; every other side yields one
/// source-to-side displacement. Sides that already carry a componentwise
/// median reuse it instead of sorting the population again.
pub(crate) fn region_readouts(
    region_id: &str,
    coordinate_space: &str,
    sides: &[Side<'_>],
    gating: &Gating,
    eligibility: Option<(bool, bool)>,
) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    for estimator in Estimator::ALL {
        let mut centers = Vec::with_capacity(sides.len());
        let mut linear = Vec::with_capacity(sides.len());
        let mut failure = None;
        for side in sides {
            match center(estimator, side, side.population) {
                Ok((lab, mean)) => {
                    centers.push(lab);
                    linear.push(mean);
                }
                Err(reason) => {
                    failure = Some(reason);
                    break;
                }
            }
        }
        if let Some(reason) = failure {
            if reason == COMPUTATION_REASON {
                return Err(crate::common::Error::new(
                    reason,
                    "Regional color center is nonfinite",
                ));
            }
            rows.push(unavailable(region_id, estimator, reason));
            continue;
        }
        let mut side_map = Map::new();
        for (index, side) in sides.iter().enumerate() {
            side_map.insert(
                side.role.to_owned(),
                side_payload(
                    side,
                    centers[index],
                    linear[index],
                    side.population.len(),
                    coordinate_space,
                    gating,
                ),
            );
        }
        let displacements: Vec<Value> = sides
            .iter()
            .enumerate()
            .skip(1)
            .map(|(index, side)| {
                displacement_payload(sides[0].role, side.role, centers[0], centers[index], gating)
            })
            .collect();
        if displacements.iter().any(|displacement| {
            displacement
                .as_object()
                .unwrap()
                .values()
                .any(|value| value["reason"] == COMPUTATION_REASON)
        }) {
            return Err(crate::common::Error::new(
                COMPUTATION_REASON,
                "Regional color displacement is nonfinite",
            ));
        }
        let mut record = json!({
            "region_id": region_id,
            "estimator": estimator.id(),
            "domain": estimator.domain(),
            "readout_version": READOUT_VERSION,
            "white_point": WHITE_POINT,
            "observer": OBSERVER,
            "availability": "available",
            "sides": Value::Object(side_map),
            "displacements": displacements,
            "artifact_refs": [],
        });
        if let Some((tone, chroma)) = eligibility {
            record["eligibility"] = json!({"tone": tone, "chroma": chroma});
        }
        rows.push(record);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(width: usize) -> Value {
        json!([0, 0, width, 1])
    }

    fn side<'a>(
        role: &'a str,
        geometry: &'a Value,
        lab: &'a [[f64; 3]],
        encoded: &'a [[f64; 3]],
        population: &'a [usize],
    ) -> Side<'a> {
        Side {
            role,
            geometry,
            lab,
            encoded,
            linear: None,
            population,
            median: None,
        }
    }

    fn readout<'a>(rows: &'a [Value], estimator: &str) -> &'a Value {
        rows.iter()
            .find(|row| row["estimator"] == estimator)
            .expect("estimator readout")
    }

    fn available(observation: &Value) -> f64 {
        assert_eq!(observation["availability"], "available");
        observation["value"].as_f64().expect("finite value")
    }

    #[test]
    fn mean_center_uses_unclipped_linear_mean_before_one_lab_conversion() {
        let geometry = geometry(2);
        let lab = image_data::rgb_to_lab(&[[0.; 3], [1.; 3]]);
        let encoded = [[0.; 3], [1.; 3]];
        let mask = [0, 1];
        let rows = region_readouts(
            "patch",
            "native_render_pixels",
            &[side("baseline", &geometry, &lab, &encoded, &mask)],
            &Gating {
                tone: true,
                chroma: true,
                hue_floor: None,
            },
            None,
        )
        .unwrap();
        let mean = readout(&rows, MEAN_ESTIMATOR);
        assert_eq!(
            mean["sides"]["baseline"]["mean_linear_srgb"],
            json!([0.5, 0.5, 0.5])
        );
        // Mean of the linear population, then one conversion: never the mean of
        // two already converted colors, which would sit at L* = 50.
        let expected_l = 116.0 * 0.5_f64.cbrt() - 16.0;
        assert!((mean["sides"]["baseline"]["lab"][0].as_f64().unwrap() - expected_l).abs() < 1e-3);
        let median_l = readout(&rows, MEDIAN_ESTIMATOR)["sides"]["baseline"]["lab"][0]
            .as_f64()
            .unwrap();
        assert!((median_l - 50.0).abs() < 1e-3);
    }

    #[test]
    fn out_of_range_encoded_samples_clip_for_the_median_but_not_for_the_mean() {
        let geometry = geometry(1);
        let lab = [[0.; 3]];
        let encoded = [[-1.; 3]];
        let mask = [0];
        let rows = region_readouts(
            "patch",
            "native_render_pixels",
            &[side("baseline", &geometry, &lab, &encoded, &mask)],
            &Gating {
                tone: true,
                chroma: true,
                hue_floor: None,
            },
            None,
        )
        .unwrap();
        let median = readout(&rows, MEDIAN_ESTIMATOR)["sides"]["baseline"]["lab"][0]
            .as_f64()
            .unwrap();
        let mean = readout(&rows, MEAN_ESTIMATOR)["sides"]["baseline"]["lab"][0]
            .as_f64()
            .unwrap();
        assert_eq!(median, 0.);
        assert!(mean < 0., "{mean}");
        assert!(
            readout(&rows, MEAN_ESTIMATOR)["sides"]["baseline"]["mean_linear_srgb"][0]
                .as_f64()
                .unwrap()
                < 0.
        );
    }

    #[test]
    fn signed_displacement_axes_follow_to_minus_from() {
        let geometry = geometry(1);
        let baseline_lab = [[50., 10., 0.]];
        let candidate_lab = [[60., 0., 20.]];
        let encoded = [[0.5; 3], [0.5; 3]];
        let mask = [0];
        let rows = region_readouts(
            "patch",
            "native_render_pixels",
            &[
                side("baseline", &geometry, &baseline_lab, &encoded, &mask),
                side("candidate", &geometry, &candidate_lab, &encoded, &mask),
            ],
            &Gating {
                tone: true,
                chroma: true,
                hue_floor: Some(2.0),
            },
            None,
        )
        .unwrap();
        let displacement = &readout(&rows, MEDIAN_ESTIMATOR)["displacements"][0];
        assert_eq!(displacement["from_role"], "baseline");
        assert_eq!(displacement["to_role"], "candidate");
        assert_eq!(displacement["delta_lab"]["value"], json!([10., -10., 20.]));
        assert_eq!(available(&displacement["delta_l_star"]), 10.);
        assert_eq!(available(&displacement["delta_c_star"]), 10.);
        // 90 degrees counter-clockwise from hue 0 to hue 90.
        assert!((available(&displacement["delta_h_degrees"]) - 90.).abs() < 1e-9);
        assert!((available(&displacement["delta_e76"]) - 600_f64.sqrt()).abs() < 1e-9);
        assert!(available(&displacement["delta_e00"]) > 0.);
        assert!(available(&displacement["equal_l_delta_e00"]) > 0.);
        // Equal lightness inside the metric only: the retained side keeps L*.
        assert_eq!(
            readout(&rows, MEDIAN_ESTIMATOR)["sides"]["candidate"]["lab"],
            json!([60., 0., 20.])
        );
    }

    #[test]
    fn hue_requires_the_authored_floor_and_wraps_the_shortest_interval() {
        let geometry = geometry(1);
        let at = |degrees: f64| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            [50., 10. * cos, 10. * sin]
        };
        let reference_lab = [at(179.5)];
        let model_lab = [at(-179.5)];
        let encoded = [[0.5; 3], [0.5; 3]];
        let mask = [0];
        let sides = [
            side("reference", &geometry, &reference_lab, &encoded, &mask),
            side("simulation", &geometry, &model_lab, &encoded, &mask),
        ];
        let unfloored = region_readouts(
            "patch",
            "render_pixel_coordinates",
            &sides,
            &Gating {
                tone: true,
                chroma: true,
                hue_floor: None,
            },
            None,
        )
        .unwrap();
        let observation =
            &readout(&unfloored, MEDIAN_ESTIMATOR)["displacements"][0]["delta_h_degrees"];
        assert_eq!(observation["availability"], "unavailable");
        assert_eq!(observation["reason"], NO_FLOOR_REASON);
        assert_eq!(
            readout(&unfloored, MEDIAN_ESTIMATOR)["sides"]["reference"]["lch"]["h_degrees"]["reason"],
            NO_FLOOR_REASON
        );
        let floored = region_readouts(
            "patch",
            "render_pixel_coordinates",
            &sides,
            &Gating {
                tone: true,
                chroma: true,
                hue_floor: Some(2.),
            },
            None,
        )
        .unwrap();
        let shift =
            readout(&floored, MEDIAN_ESTIMATOR)["displacements"][0]["delta_h_degrees"].clone();
        assert!((available(&shift) - 1.).abs() < 1e-9, "{shift}");
        assert!(
            (available(
                &readout(&floored, MEDIAN_ESTIMATOR)["sides"]["reference"]["lch"]["h_degrees"]
            ) - 179.5)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn low_chroma_sides_and_displacements_lose_hue_without_losing_coordinates() {
        let geometry = geometry(1);
        let reference_lab = [[50., 1., 0.]];
        let model_lab = [[50., 1., 0.]];
        let encoded = [[0.5; 3], [0.5; 3]];
        let mask = [0];
        let rows = region_readouts(
            "patch",
            "render_pixel_coordinates",
            &[
                side("reference", &geometry, &reference_lab, &encoded, &mask),
                side("simulation", &geometry, &model_lab, &encoded, &mask),
            ],
            &Gating {
                tone: true,
                chroma: true,
                hue_floor: Some(2.),
            },
            Some((true, true)),
        )
        .unwrap();
        let record = readout(&rows, MEDIAN_ESTIMATOR);
        for role in ["reference", "simulation"] {
            let hue = &record["sides"][role]["lch"]["h_degrees"];
            assert_eq!(hue["availability"], "unavailable");
            assert_eq!(hue["reason"], BELOW_FLOOR_REASON);
            assert_eq!(record["sides"][role]["lch"]["c_star"], 1.);
            assert_eq!(record["sides"][role]["sample_count"], 1);
            assert_eq!(
                record["sides"][role]["coordinate_space"],
                "render_pixel_coordinates"
            );
            assert_eq!(record["sides"][role]["geometry"], geometry);
        }
        let displacement = &record["displacements"][0];
        assert_eq!(
            displacement["delta_h_degrees"]["reason"],
            BELOW_FLOOR_REASON
        );
        assert_eq!(
            displacement["equal_l_delta_e00"]["availability"],
            "available"
        );
        assert_eq!(displacement["delta_e00"]["availability"], "available");
        assert_eq!(record["eligibility"], json!({"tone": true, "chroma": true}));
    }

    #[test]
    fn eligibility_gates_displacements_but_not_retained_coordinates() {
        let geometry = geometry(1);
        let reference_lab = [[50., 10., 5.]];
        let model_lab = [[60., 12., 6.]];
        let encoded = [[0.5; 3], [0.5; 3]];
        let mask = [0];
        let rows = region_readouts(
            "patch",
            "render_pixel_coordinates",
            &[
                side("reference", &geometry, &reference_lab, &encoded, &mask),
                side("simulation", &geometry, &model_lab, &encoded, &mask),
            ],
            &Gating {
                tone: false,
                chroma: false,
                hue_floor: Some(2.),
            },
            Some((false, false)),
        )
        .unwrap();
        let record = readout(&rows, MEAN_ESTIMATOR);
        assert_eq!(record["availability"], "available");
        assert_eq!(
            record["eligibility"],
            json!({"tone": false, "chroma": false})
        );
        assert!(record["sides"]["reference"]["lab"][0].is_number());
        assert_eq!(
            record["sides"]["reference"]["lch"]["h_degrees"]["reason"],
            CHROMA_REASON
        );
        let displacement = &record["displacements"][0];
        for key in ["delta_lab", "delta_l_star", "delta_e76", "delta_e00"] {
            assert_eq!(displacement[key]["availability"], "unavailable", "{key}");
            assert_eq!(displacement[key]["reason"], TONE_REASON, "{key}");
            assert!(displacement[key].get("value").is_none(), "{key}");
        }
        for key in ["delta_c_star", "equal_l_delta_e00", "delta_h_degrees"] {
            assert_eq!(displacement[key]["availability"], "unavailable", "{key}");
            assert_eq!(displacement[key]["reason"], CHROMA_REASON, "{key}");
            assert!(displacement[key].get("value").is_none(), "{key}");
        }
    }

    #[test]
    fn empty_populations_and_unselected_regions_report_reasons_without_coordinates() {
        let geometry = geometry(2);
        let lab = [[0.; 3], [0.; 3]];
        let encoded = [[0.5; 3], [0.5; 3]];
        let mask = [];
        let rows = region_readouts(
            "patch",
            "native_render_pixels",
            &[side("baseline", &geometry, &lab, &encoded, &mask)],
            &Gating {
                tone: true,
                chroma: true,
                hue_floor: None,
            },
            None,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row["availability"], "unavailable");
            assert_eq!(row["reason"], EMPTY_REASON);
            assert!(row.get("sides").is_none());
            assert!(row.get("displacements").is_none());
        }
        let rows = unavailable_pair("other", "color_family_not_selected");
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row["availability"], "unavailable");
            assert_eq!(row["reason"], "color_family_not_selected");
            assert!(row.get("sides").is_none());
        }
    }
}
