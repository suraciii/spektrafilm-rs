use super::DOMAIN;
use super::artifacts::{ArtifactRole, Comparison};
use super::catalog::{self, Metric};
use super::io::{err, field, mask, mean, optional_array, percentile, population, text};
use crate::common::Result;
use crate::image_data;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Authored case region pair naming two distinct existing regions.
#[derive(Debug)]
pub struct RegionPair {
    pub id: String,
    pub label: String,
    pub left: String,
    pub right: String,
}

/// Strict validation of case `region_pairs`. Suite loading and the measurement
/// path share this parse so authoring and execution cannot diverge.
pub fn validate_region_pairs(case: &Value) -> Result<Vec<RegionPair>> {
    let regions: Vec<&str> = optional_array(case, "regions")
        .iter()
        .map(|r| text(r, "id"))
        .collect::<Result<_>>()?;
    let mut pairs = Vec::new();
    let mut ids = BTreeSet::new();
    for entry in optional_array(case, "region_pairs") {
        let id = identifier_field(entry, "id")?;
        let label = entry
            .get("label")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| err("invalid_region_pairs", "region pair label must be text"))?;
        let left = identifier_field(entry, "left")?;
        let right = identifier_field(entry, "right")?;
        if !ids.insert(id.clone()) {
            return Err(err("duplicate_id", "duplicate region pair id"));
        }
        if id == "full_image" || regions.contains(&id.as_str()) {
            return Err(err(
                "invalid_region_pairs",
                "region pair id must not collide with a region id",
            ));
        }
        if left == right {
            return Err(err(
                "invalid_region_pairs",
                "region pair endpoints must be distinct",
            ));
        }
        if !regions.contains(&left.as_str()) || !regions.contains(&right.as_str()) {
            return Err(err(
                "unresolved_identifier",
                "region pair endpoints must name existing regions",
            ));
        }
        pairs.push(RegionPair {
            id,
            label: label.to_owned(),
            left,
            right,
        });
    }
    if case["reference"]["scope"].as_str() == Some("regional") {
        let corresponding: BTreeSet<&str> = optional_array(&case["reference"], "regions")
            .iter()
            .map(|r| text(r, "id"))
            .collect::<Result<_>>()?;
        for pair in &pairs {
            if !corresponding.contains(pair.left.as_str())
                || !corresponding.contains(pair.right.as_str())
            {
                return Err(err(
                    "invalid_region_pairs",
                    "region pair endpoints must name corresponding reference regions",
                ));
            }
        }
    }
    Ok(pairs)
}

fn identifier_field(entry: &Value, key: &str) -> Result<String> {
    entry
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| identifier(v))
        .map(str::to_owned)
        .ok_or_else(|| {
            err(
                "invalid_region_pairs",
                format!("region pair {key} must be a valid identifier"),
            )
        })
}

fn identifier(x: &str) -> bool {
    !x.is_empty()
        && x.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn chroma_ab(p: [f64; 3]) -> f64 {
    p[1].hypot(p[2])
}

fn chroma_distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[1] - b[1]).hypot(a[2] - b[2])
}

/// Shortest circular hue-angle difference, model minus reference, degrees in
/// (-180, 180].
fn shortest_hue_shift(model: [f64; 3], reference: [f64; 3]) -> f64 {
    let raw = model[2].atan2(model[1]).to_degrees() - reference[2].atan2(reference[1]).to_degrees();
    let wrapped = (raw + 180.).rem_euclid(360.) - 180.;
    if wrapped == -180. { 180. } else { wrapped }
}

fn side_entry(value: Option<f64>, reason: Option<&str>) -> Value {
    match value {
        Some(v) => json!({"availability":"available","value":v}),
        None => json!({"availability":"unavailable","reason":reason.unwrap_or("missing_evidence")}),
    }
}

type DiagnosticValues = (&'static Metric, Vec<Option<f64>>, Option<&'static str>);

/// Per-region evidence retained for pair measurements and population summaries.
struct RegionRecord {
    id: String,
    role: String,
    render_count: usize,
    reference_count: usize,
    model: Vec<[f64; 3]>,
    reference: [f64; 3],
    model_chroma: Vec<f64>,
    diagnostics: [DiagnosticValues; 6],
}

// Keep regional arrays and frozen comparison controls explicit at this boundary.
#[allow(clippy::too_many_arguments)]
pub fn regional(
    c: &mut Comparison,
    ref_lab: &[[f64; 3]],
    ref_encoded: &[[f64; 3]],
    reference_dims: [usize; 2],
    labs: &[Vec<[f64; 3]>],
    encoded: &[Vec<[f64; 3]>],
    dims: [usize; 2],
    case: &Value,
    base: &Path,
    tone: bool,
    chroma: bool,
) -> Result<()> {
    let reference = c.reference;
    let corresponding = optional_array(reference, "regions");
    if corresponding.is_empty() {
        return Err(err(
            "missing_reference_regions",
            "Regional scope requires fixed corresponding regions",
        ));
    }
    let mut seen = BTreeSet::new();
    for r in corresponding {
        if !seen.insert(text(r, "id")?) {
            return Err(err(
                "invalid_reference_regions",
                "Reference region identities must be unique",
            ));
        }
    }
    let pairs = validate_region_pairs(case)?;
    let floor = catalog::hue_floor(&c.protocol)?;
    if let Some(f) = floor {
        c.review["hue_chroma_floor"] = json!(f);
    }
    let side_names: Vec<&str> = if c.single {
        vec!["simulation"]
    } else {
        vec!["baseline", "candidate", "distance_delta"]
    };
    let captures: Vec<&str> = if c.single {
        vec!["simulation"]
    } else {
        vec!["baseline", "candidate"]
    };
    c.limitation("Corresponding region statistics do not establish pixel correspondence or full-scene fidelity.");
    if floor.is_some() {
        c.limitation("Hue diagnostics apply only where both chromas meet the authored hue_chroma_floor; equal-L and pair relations keep separate identities from ordinary DE00.");
    }
    c.review["regions"] = json!([]);
    c.review["region_pairs"] = json!([]);
    c.review["population_summaries"] = json!([]);
    let mut records: Vec<RegionRecord> = Vec::new();
    for region in corresponding {
        let id = text(region, "id")?;
        let render = optional_array(case, "regions")
            .iter()
            .rev()
            .find(|r| r.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| {
                err(
                    "invalid_reference_regions",
                    "Reference region has no matching render region",
                )
            })?;
        let render_mask = mask(field(render, "geometry")?, dims, base)?;
        let render_pop = population(&render_mask);
        let ref_mask = mask(field(region, "geometry")?, reference_dims, base)?;
        let ref_pop = population(&ref_mask);
        if render_pop.is_empty() || ref_pop.is_empty() {
            return Err(err(
                "empty_reference_population",
                "Corresponding region has no eligible samples",
            ));
        }
        let populations: Vec<_> = labs
            .iter()
            .map(|lab| (&lab[..], &render_pop))
            .chain(std::iter::once((ref_lab, &ref_pop)))
            .collect();
        let centers: Vec<[f64; 3]> = populations
            .iter()
            .map(|population| {
                std::array::from_fn(|ch| {
                    percentile(
                        population.1.iter().map(|i| population.0[*i][ch]).collect(),
                        0.5,
                    )
                })
            })
            .collect();
        let quantiles: Vec<Vec<f64>> = populations
            .iter()
            .map(|population| {
                (0..=100)
                    .map(|q| {
                        percentile(
                            population.1.iter().map(|i| population.0[*i][0]).collect(),
                            q as f64 / 100.,
                        )
                    })
                    .collect()
            })
            .collect();
        let reference_index = labs.len();
        let reference_center = centers[reference_index];
        let de: Vec<Option<f64>> = (0..labs.len())
            .map(|s| Some(image_data::delta_e_2000(centers[s], reference_center)))
            .collect();
        let equal_l: Vec<Option<f64>> = (0..labs.len())
            .map(|s| {
                Some(image_data::delta_e_2000(
                    [reference_center[0], centers[s][1], centers[s][2]],
                    reference_center,
                ))
            })
            .collect();
        let ld: Vec<Option<f64>> = (0..labs.len())
            .map(|s| {
                Some(mean(
                    &quantiles[s]
                        .iter()
                        .zip(&quantiles[reference_index])
                        .map(|(a, b)| (a - b).abs())
                        .collect::<Vec<_>>(),
                ))
            })
            .collect();
        let ch: Vec<Option<f64>> = (0..labs.len())
            .map(|s| Some(chroma_distance(centers[s], reference_center)))
            .collect();
        let reference_chroma = chroma_ab(reference_center);
        let model_chroma: Vec<f64> = (0..labs.len()).map(|s| chroma_ab(centers[s])).collect();
        let (hue_shift, hue_reason): (Vec<Option<f64>>, Option<&'static str>) = match floor {
            None => (vec![None; labs.len()], Some("hue_diagnostics_not_selected")),
            Some(_) if !chroma => (
                vec![None; labs.len()],
                Some("unknown_or_graded_color_reference"),
            ),
            Some(f) => {
                let per_side: Vec<Option<f64>> = (0..labs.len())
                    .map(|s| {
                        (reference_chroma >= f && model_chroma[s] >= f)
                            .then(|| shortest_hue_shift(centers[s], reference_center))
                    })
                    .collect();
                let reason = per_side
                    .iter()
                    .any(Option::is_none)
                    .then_some("below_chroma_floor");
                (per_side, reason)
            }
        };
        let hue_error: Vec<Option<f64>> = hue_shift.iter().map(|v| v.map(f64::abs)).collect();
        let tone_reason = (!tone).then_some("tone_contaminated_reference");
        let chroma_reason = (!chroma).then_some("unknown_or_graded_color_reference");
        let missing = vec![None; labs.len()];
        let diagnostics = [
            (
                &catalog::PATCH_DE00,
                if tone { de } else { missing.clone() },
                tone_reason,
            ),
            (
                &catalog::PATCH_DE00_EQUAL_L,
                if chroma { equal_l } else { missing.clone() },
                chroma_reason,
            ),
            (
                &catalog::DISTRIBUTION_L,
                if tone { ld } else { missing.clone() },
                tone_reason,
            ),
            (
                &catalog::PATCH_CHROMA_AB,
                if chroma { ch } else { missing.clone() },
                chroma_reason,
            ),
            (&catalog::PATCH_HUE_SHIFT, hue_shift, hue_reason),
            (&catalog::PATCH_HUE_ERROR, hue_error, hue_reason),
        ];
        let role = render
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("other")
            .to_owned();
        let label = render
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_owned();
        let render_geometry = field(render, "geometry")?;
        let reference_geometry = field(region, "geometry")?;
        let readouts = if optional_array(render, "measurements")
            .iter()
            .any(|family| family == "color")
        {
            // The readouts reuse the regional centres and populations directly, so
            // no second sort of the same samples happens here. The reference is
            // the displacement source: every model side yields one
            // reference-to-model displacement.
            let mut readout_sides: Vec<crate::color_readouts::Side<'_>> =
                vec![crate::color_readouts::Side {
                    role: "reference",
                    geometry: reference_geometry,
                    lab: ref_lab,
                    encoded: ref_encoded,
                    linear: None,
                    population: &ref_pop,
                    median: Some(centers[reference_index]),
                }];
            readout_sides.extend((0..labs.len()).map(|s| crate::color_readouts::Side {
                role: captures[s],
                geometry: render_geometry,
                lab: &labs[s],
                encoded: &encoded[s],
                linear: None,
                population: &render_pop,
                median: Some(centers[s]),
            }));
            crate::color_readouts::region_readouts(
                id,
                "native_image_pixels",
                &readout_sides,
                &crate::color_readouts::Gating {
                    tone,
                    chroma,
                    hue_floor: floor,
                },
                Some((tone, chroma)),
            )?
        } else {
            crate::color_readouts::unavailable_pair(id, "color_family_not_selected")
        };
        c.readouts(readouts);
        let mut payload = json!({"region_id":id,"label":label,"role":role,"population":catalog::population_label(&role),"render_geometry":render["geometry"],"reference_geometry":region["geometry"],"render_sample_count":render_pop.len(),"reference_sample_count":ref_pop.len(),"lab_median":{"reference":centers[reference_index]},"lightness_quantile_probabilities":(0..=100).map(|q|q as f64/100.).collect::<Vec<_>>(),"lightness_quantiles":{"reference":quantiles[reference_index]},"domain":DOMAIN,"units":"L*,a*,b*"});
        for s in 0..labs.len() {
            payload["lab_median"][captures[s]] = json!(centers[s]);
            payload["lightness_quantiles"][captures[s]] = json!(quantiles[s]);
        }
        let mut diagnostics_payload = Map::new();
        for (metric, values, reason) in &diagnostics {
            let mut sides = Map::new();
            for (s, capture) in captures.iter().enumerate() {
                let mut entry = side_entry(values[s], *reason);
                if catalog::hue_metric(metric.id) {
                    entry["chroma_ab"] = json!(model_chroma[s]);
                }
                sides.insert((*capture).to_owned(), entry);
            }
            if !c.single {
                let delta = match (values[0], values[1]) {
                    (Some(a), Some(b)) => Some(b - a),
                    _ => None,
                };
                sides.insert("distance_delta".to_owned(), side_entry(delta, *reason));
            }
            let mut entry = json!({"role":metric.role,"domain":DOMAIN,"units":metric.units,"sample_count":render_pop.len(),"sides":Value::Object(sides)});
            if catalog::hue_metric(metric.id) {
                if let Some(f) = floor {
                    entry["hue_chroma_floor"] = json!(f);
                }
                entry["reference_chroma_ab"] = json!(reference_chroma);
            }
            diagnostics_payload.insert(metric.id.to_owned(), entry);
        }
        payload["diagnostics"] = Value::Object(diagnostics_payload);
        let evidence = c.json_artifact(ArtifactRole::RegionalStatistics, &payload)?;
        c.review["regions"].as_array_mut().unwrap().push(payload);
        c.review_ref(&evidence);
        let refs = [evidence];
        for (metric, values, reason) in &diagnostics {
            c.distances(
                id,
                metric.id,
                DOMAIN,
                metric.units,
                metric.estimator,
                render_pop.len(),
                Some(values.clone()),
                *reason,
                &refs,
            );
        }
        records.push(RegionRecord {
            id: id.to_owned(),
            role,
            render_count: render_pop.len(),
            reference_count: ref_pop.len(),
            model: centers[..labs.len()].to_vec(),
            reference: reference_center,
            model_chroma,
            diagnostics,
        });
    }
    pair_diagnostics(c, &records, &pairs, &captures, tone)?;
    population_summaries(c, &records, &side_names)?;
    Ok(())
}

fn pair_diagnostics(
    c: &mut Comparison,
    records: &[RegionRecord],
    pairs: &[RegionPair],
    captures: &[&str],
    tone: bool,
) -> Result<()> {
    for pair in pairs {
        let endpoint = |id: &str| {
            records.iter().find(|r| r.id == id).ok_or_else(|| {
                err(
                    "invalid_region_pairs",
                    "region pair endpoints must name corresponding reference regions",
                )
            })
        };
        let left = endpoint(&pair.left)?;
        let right = endpoint(&pair.right)?;
        let count = left.render_count.min(right.render_count);
        let reference_separation = image_data::delta_e_2000(left.reference, right.reference);
        let separations: Vec<Option<f64>> = captures
            .iter()
            .enumerate()
            .map(|(s, _)| Some(image_data::delta_e_2000(left.model[s], right.model[s])))
            .collect();
        let change: Vec<Option<f64>> = separations
            .iter()
            .map(|v| v.map(|s| s - reference_separation))
            .collect();
        let departure: Vec<Option<f64>> = change.iter().map(|v| v.map(f64::abs)).collect();
        let tone_reason = (!tone).then_some("tone_contaminated_reference");
        let missing = vec![None; captures.len()];
        let diagnostics = [
            (
                &catalog::PAIR_SEPARATION,
                if tone {
                    separations.clone()
                } else {
                    missing.clone()
                },
                tone_reason,
            ),
            (
                &catalog::PAIR_SEPARATION_CHANGE,
                if tone { change } else { missing.clone() },
                tone_reason,
            ),
            (
                &catalog::PAIR_SEPARATION_DEPARTURE,
                if tone { departure } else { missing.clone() },
                tone_reason,
            ),
        ];
        let delta = |values: &[Option<f64>]| match (values.first(), values.get(1)) {
            (Some(Some(a)), Some(Some(b))) => Some(b - a),
            _ => None,
        };
        let mut diagnostics_payload = Map::new();
        for (metric, values, reason) in &diagnostics {
            let mut sides = Map::new();
            if metric.id == catalog::PAIR_SEPARATION.id {
                sides.insert(
                    catalog::RELATION_SIDE.to_owned(),
                    side_entry(Some(reference_separation), None),
                );
            }
            for (s, capture) in captures.iter().enumerate() {
                let mut entry = side_entry(values[s], *reason);
                if metric.id == catalog::PAIR_SEPARATION.id {
                    entry["chroma_ab_left"] = json!(left.model_chroma[s]);
                    entry["chroma_ab_right"] = json!(right.model_chroma[s]);
                }
                sides.insert((*capture).to_owned(), entry);
            }
            if captures.len() > 1 {
                sides.insert(
                    "distance_delta".to_owned(),
                    side_entry(delta(values), *reason),
                );
            }
            let entry = json!({"role":metric.role,"domain":DOMAIN,"units":metric.units,"sample_count":count,"sides":Value::Object(sides)});
            diagnostics_payload.insert(metric.id.to_owned(), entry);
        }
        let payload = json!({"id":pair.id,"label":pair.label,"role":"region_pair","left":pair.left,"right":pair.right,"left_role":left.role,"right_role":right.role,"sample_counts":{"left":{"render":left.render_count,"reference":left.reference_count},"right":{"render":right.render_count,"reference":right.reference_count}},"domain":DOMAIN,"units":catalog::PAIR_SEPARATION.units,"diagnostics":Value::Object(diagnostics_payload)});
        let evidence = c.json_artifact(ArtifactRole::RegionPairStatistics, &payload)?;
        c.review["region_pairs"]
            .as_array_mut()
            .unwrap()
            .push(payload);
        c.review_ref(&evidence);
        let refs = [evidence];
        c.relation_row(
            &pair.id,
            catalog::PAIR_SEPARATION.id,
            DOMAIN,
            catalog::PAIR_SEPARATION.units,
            catalog::PAIR_SEPARATION.estimator,
            count,
            Some(reference_separation),
            None,
            &refs,
        );
        for (metric, values, reason) in &diagnostics {
            c.distances(
                &pair.id,
                metric.id,
                DOMAIN,
                metric.units,
                metric.estimator,
                count,
                Some(values.clone()),
                *reason,
                &refs,
            );
        }
    }
    Ok(())
}

fn population_summaries(
    c: &mut Comparison,
    records: &[RegionRecord],
    side_names: &[&str],
) -> Result<()> {
    let mut populations: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, record) in records.iter().enumerate() {
        populations
            .entry(catalog::population_label(&record.role).to_owned())
            .or_default()
            .push(i);
    }
    let mut summaries = Vec::new();
    for (population, members) in &populations {
        let region_ids: Vec<&str> = members.iter().map(|&i| records[i].id.as_str()).collect();
        let mut roles: Vec<&str> = members.iter().map(|&i| records[i].role.as_str()).collect();
        roles.sort_unstable();
        roles.dedup();
        for metric in catalog::SUMMARIZED_METRICS {
            let mut per_capture = Map::new();
            for (side_index, side) in side_names.iter().enumerate() {
                let mut entries = Vec::new();
                let mut reasons = BTreeSet::new();
                for &i in members {
                    let diagnostic = records[i]
                        .diagnostics
                        .iter()
                        .find(|d| d.0.id == metric.id)
                        .expect("regional diagnostics cover summarized metrics");
                    let value = if *side == "distance_delta" {
                        match (diagnostic.1.first(), diagnostic.1.get(1)) {
                            (Some(Some(a)), Some(Some(b))) => Some(b - a),
                            _ => None,
                        }
                    } else {
                        diagnostic.1.get(side_index).copied().flatten()
                    };
                    if value.is_none() {
                        reasons.insert(diagnostic.2.unwrap_or("missing_evidence"));
                    }
                    entries.push((records[i].id.as_str(), value));
                }
                per_capture.insert(
                    (*side).to_owned(),
                    summary_side(&entries, &reasons, *side == "distance_delta"),
                );
            }
            summaries.push(json!({"population":population,"roles":roles,"region_ids":region_ids,"metric":metric.id,"domain":DOMAIN,"units":metric.units,"per_capture":Value::Object(per_capture)}));
        }
    }
    if !summaries.is_empty() {
        c.limitation("Population summaries and worst-regression indices describe retained measurement costs only; they are not acceptance scores, vetoes, or material adoption claims.");
    }
    c.review["population_summaries"] = json!(summaries);
    Ok(())
}

fn summary_side(
    entries: &[(&str, Option<f64>)],
    reasons: &BTreeSet<&str>,
    regression: bool,
) -> Value {
    let values: Vec<f64> = entries.iter().filter_map(|(_, v)| *v).collect();
    let unavailable_count = entries.len() - values.len();
    let mut side = json!({"region_count":values.len(),"unavailable_count":unavailable_count});
    if unavailable_count > 0 {
        side["unavailable_reason"] = json!(if reasons.len() == 1 {
            *reasons.iter().next().unwrap()
        } else {
            "missing_evidence"
        });
    }
    if !values.is_empty() {
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let worst = entries
            .iter()
            .filter_map(|(id, v)| v.map(|v| (*id, v)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
            .unwrap_or("");
        side["max"] = json!(max);
        side["p95"] = json!(percentile(values.clone(), 0.95));
        side["median"] = json!(percentile(values, 0.5));
        if regression {
            side["regression_count"] = json!(
                entries
                    .iter()
                    .filter(|(_, v)| v.is_some_and(|v| v > 0.))
                    .count()
            );
            side["worst_regression_region_id"] = json!((max > 0.).then_some(worst));
        } else {
            side["worst_region_id"] = json!(worst);
        }
    }
    side
}

#[derive(Clone, Copy)]
pub enum ProbeDomain {
    Native,
    Lab,
    Luminance,
}
#[derive(Clone, Copy)]
pub enum Reduction {
    Mean,
    Median,
}
pub struct ProbeEstimator {
    pub(super) name: String,
    pub(super) domain: ProbeDomain,
    reduction: Reduction,
}
impl ProbeEstimator {
    pub(super) fn parse(name: &str) -> Result<Self> {
        let (domain, reduction) = match name {
            "native_channel_mean" => (ProbeDomain::Native, Reduction::Mean),
            "native_channel_median" => (ProbeDomain::Native, Reduction::Median),
            "lab_channel_mean" => (ProbeDomain::Lab, Reduction::Mean),
            "lab_channel_median" => (ProbeDomain::Lab, Reduction::Median),
            "linear_luminance_mean" => (ProbeDomain::Luminance, Reduction::Mean),
            "linear_luminance_median" => (ProbeDomain::Luminance, Reduction::Median),
            _ => {
                return Err(err(
                    "invalid_probe_response",
                    "Unknown model response estimator",
                ));
            }
        };
        Ok(Self {
            name: name.into(),
            domain,
            reduction,
        })
    }
    pub(super) fn channel(&self, name: &str) -> Option<usize> {
        match self.domain {
            ProbeDomain::Native => match name {
                "R" => Some(0),
                "G" => Some(1),
                "B" => Some(2),
                _ => None,
            },
            ProbeDomain::Lab => match name {
                "L" => Some(0),
                "a" => Some(1),
                "b" => Some(2),
                _ => None,
            },
            ProbeDomain::Luminance => (name == "Y").then_some(0),
        }
    }
    pub(super) fn reduce(&self, values: Vec<f64>) -> f64 {
        match self.reduction {
            Reduction::Mean => mean(&values),
            Reduction::Median => percentile(values, 0.5),
        }
    }
}
