use crate::color_readouts;
use crate::common::{Result, err};
use crate::image_data::{self, ImageData};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

mod evidence;
mod families;
mod mask;
mod math;

pub use evidence::{ArtifactRole, Evidence};
pub use families::{Map, edge, grain, halation, health, numeric, summary, tone};
pub use mask::geometry_mask;
pub use math::{count, luminance, mean, median, number, quantile, selected, spelling};

pub const METRIC_VERSION: &str = "spektrafilm-appearance-spatial-v3";
pub const COLOR_TRANSFORM: &str = "colour-science-CAT02-srgb-D65-2deg-v1";
fn populations(regions: &[Value], protocol: &Value) -> Vec<Value> {
    let mut result = vec![
        json!({"id":"full_image","measurements":protocol.get("measurements").cloned().unwrap_or(json!(["numeric","color","tone"]))}),
    ];
    result.extend_from_slice(regions);
    result
}
fn compare_inner(
    e: &mut Evidence,
    a: &ImageData,
    b: &ImageData,
    populations: &[Value],
    protocol: &Value,
) -> Result<()> {
    e.extra.insert("numeric_health".into(),json!({"baseline":health(&a.samples,a.dimensions()),"candidate":health(&b.samples,b.dimensions())}));
    if a.dimensions() != b.dimensions() {
        return Err(err(
            "dimension_mismatch",
            "Masters differ in dimensions; no resize performed",
        ));
    }
    if a.samples
        .iter()
        .chain(&b.samples)
        .any(|p| p.iter().any(|x| !x.is_finite()))
    {
        return Err(err(
            "nonfinite_samples",
            "Native master contains nonfinite samples",
        ));
    }
    let (w, h) = (a.width, a.height);
    let encoded = [image_data::display_rgb(a)?, image_data::display_rgb(b)?];
    if encoded
        .iter()
        .flatten()
        .any(|p| p.iter().any(|x| !x.is_finite()))
    {
        return Err(err(
            "computation_failure",
            "Display transform produced nonfinite values",
        ));
    }
    let mut display_health = json!({});
    for (side, image) in ["baseline", "candidate"].into_iter().zip(&encoded) {
        let below = image.iter().flatten().filter(|x| **x < 0.0).count();
        let above = image.iter().flatten().filter(|x| **x > 1.0).count();
        let total = image.len() * 3;
        display_health[side] = json!({"below_zero_fraction":below as f64/total as f64,"above_one_fraction":above as f64/total as f64,"clipped_sample_fraction":(below+above) as f64/total as f64,"sample_count":total,"domain":"unclipped_encoded_srgb"});
    }
    e.extra.insert("display_health".into(), display_health);
    let labs = std::array::from_fn::<_, 2, _>(|side| {
        let mut lab = Vec::with_capacity(w * h);
        for chunk in encoded[side].chunks(65536) {
            let bounded: Vec<[f64; 3]> =
                chunk.iter().map(|p| p.map(|x| x.clamp(0.0, 1.0))).collect();
            lab.extend(image_data::rgb_to_lab(&bounded));
        }
        lab
    });
    let mut maps = Vec::new();
    for (name, units, component) in [
        ("delta_e00", "delta E 00", None),
        ("delta_l_star", "L*", Some(0)),
        ("delta_a_star", "a*", Some(1)),
        ("delta_b_star", "b*", Some(2)),
        ("delta_c_star", "C*", Some(3)),
    ] {
        let values: Vec<f64> = labs[0]
            .iter()
            .zip(&labs[1])
            .map(|(a, b)| match component {
                None => image_data::delta_e_2000(*a, *b),
                Some(3) => b[1].hypot(b[2]) - a[1].hypot(a[2]),
                Some(c) => b[c] - a[c],
            })
            .collect();
        let artifact = e.artifact(
            name,
            &values,
            &[h, w],
            ArtifactRole::DifferenceMap,
            json!({"metric":name,"domain":"bounded_srgb_d65_lab","units":units,"dimensions":[w,h]}),
        )?;
        maps.push(Map {
            name,
            units,
            values,
            artifact,
        });
    }
    let has_family = |family: &str| {
        populations.iter().any(|p| {
            p["measurements"]
                .as_array()
                .is_some_and(|x| x.iter().any(|x| x.as_str() == Some(family)))
        })
    };
    if has_family("flip") {
        let ppd = number(protocol, "pixels_per_degree")?;
        let values = image_data::official_flip(&encoded[0], &encoded[1], w, h, ppd)?;
        let artifact=e.artifact("flip",&values,&[h,w],ArtifactRole::DifferenceMap,json!({"metric":"flip","domain":"bounded_encoded_srgb","units":"FLIP error","pixels_per_degree":protocol["pixels_per_degree"],"implementation":"NVlabs flip-evaluator 1.7","output_size":[w,h]}))?;
        maps.push(Map {
            name: "flip",
            units: "FLIP error",
            values,
            artifact,
        });
        e.extra.insert("flip_parameters".into(), json!({"ppd":ppd}));
    }
    let linear = if ["tone", "grain", "halation", "edge"]
        .iter()
        .any(|f| has_family(f))
    {
        Some(std::array::from_fn::<_, 2, _>(|side| {
            encoded[side]
                .iter()
                .map(|p| p.map(image_data::srgb_decode))
                .collect::<Vec<_>>()
        }))
    } else {
        None
    };
    // Regional color readouts reuse the same bounded Lab population and the
    // same unclipped encoded raster the scalar families already built.
    let mut color_readouts = Vec::new();
    let hue_floor = crate::references::hue_floor(protocol)?;
    let refs: Vec<String> = maps.iter().map(|m| m.artifact.clone()).collect();
    let mut seen = HashSet::new();
    for population in populations {
        let region = population["id"].as_str().unwrap_or("");
        let mut active = "region";
        let execution = (|| -> Result<()> {
            if !seen.insert(region) {
                return Err(err(
                    "invalid_region",
                    "Duplicate or reserved full_image region",
                ));
            }
            let mask = if region == "full_image" {
                vec![true; w * h]
            } else {
                geometry_mask(&population["geometry"], w, h)?
            };
            let n = count(&mask);
            if n == 0 {
                return Err(err("insufficient_samples", "Region has no selected pixels"));
            }
            let geometry = if region == "full_image" {
                json!({"rectangle":[0,0,w,h]})
            } else {
                population["geometry"].clone()
            };
            if population["measurements"]
                .as_array()
                .is_some_and(|families| families.iter().any(|family| family == "color"))
            {
                if region != "full_image" {
                    let indices: Vec<usize> = mask
                        .iter()
                        .enumerate()
                        .filter_map(|(i, selected)| selected.then_some(i))
                        .collect();
                    color_readouts.extend(color_readouts::region_readouts(
                        region,
                        "native_render_pixels",
                        &[
                            color_readouts::Side {
                                role: "baseline",
                                geometry: &geometry,
                                lab: &labs[0],
                                encoded: &encoded[0],
                                linear: linear.as_ref().map(|images| images[0].as_slice()),
                                population: &indices,
                                median: None,
                            },
                            color_readouts::Side {
                                role: "candidate",
                                geometry: &geometry,
                                lab: &labs[1],
                                encoded: &encoded[1],
                                linear: linear.as_ref().map(|images| images[1].as_slice()),
                                population: &indices,
                                median: None,
                            },
                        ],
                        &color_readouts::Gating {
                            tone: true,
                            chroma: true,
                            hue_floor,
                        },
                        None,
                    )?);
                }
            } else if region != "full_image" {
                color_readouts.extend(color_readouts::unavailable_pair(
                    region,
                    "color_family_not_selected",
                ));
            }
            for family in population["measurements"].as_array().into_iter().flatten() {
                active = family.as_str().unwrap_or("");
                match active {
                    "numeric" => numeric(e, region, &mask, a, b),
                    "color" => {
                        summary(
                            e,
                            region,
                            "delta_e00",
                            selected(&maps[0].values, &mask),
                            std::slice::from_ref(&maps[0].artifact),
                            "bounded_srgb_d65_lab",
                            "delta E 00",
                            false,
                        );
                        for m in &maps[1..5] {
                            let mut values = selected(&m.values, &mask);
                            let average = mean(&values);
                            let middle = median(&mut values);
                            for (label, value) in [("mean", average), ("median", middle)] {
                                e.scalar(
                                    region,
                                    &format!("{}.{label}", m.name),
                                    value,
                                    n,
                                    "bounded_srgb_d65_lab",
                                    m.units,
                                    &format!("signed pixel {label} candidate minus baseline"),
                                    std::slice::from_ref(&m.artifact),
                                    None,
                                );
                            }
                        }
                        for threshold in protocol["delta_e_thresholds"]
                            .as_array()
                            .into_iter()
                            .flatten()
                        {
                            let t = threshold.as_f64().ok_or_else(|| {
                                err("invalid_protocol", "Threshold must be numeric")
                            })?;
                            let label = spelling(threshold);
                            let above = maps[0]
                                .values
                                .iter()
                                .zip(&mask)
                                .filter(|(x, m)| **m && **x > t)
                                .count();
                            e.scalar(
                                region,
                                &format!("delta_e00.fraction_above_{label}"),
                                Some(above as f64 / n as f64),
                                n,
                                "bounded_srgb_d65_lab",
                                "fraction",
                                &format!(
                                    "pixel count above threshold={label} / valid selected pixels"
                                ),
                                std::slice::from_ref(&maps[0].artifact),
                                None,
                            );
                        }
                    }
                    "flip" => {
                        let map = maps.last().unwrap();
                        summary(
                            e,
                            region,
                            "flip",
                            selected(&map.values, &mask),
                            std::slice::from_ref(&map.artifact),
                            "bounded_encoded_srgb",
                            "FLIP error",
                            true,
                        );
                    }
                    "tone" => tone(
                        e,
                        region,
                        &mask,
                        &labs,
                        linear.as_ref().unwrap(),
                        protocol,
                        w,
                        h,
                        std::slice::from_ref(&maps[1].artifact),
                    )?,
                    "grain" => grain(
                        e,
                        region,
                        &mask,
                        linear.as_ref().unwrap(),
                        w,
                        h,
                        &protocol["grain"],
                        population["role"].as_str() == Some("flat_field")
                            || protocol["probe"]["kind"].as_str() == Some("flat_field"),
                    )?,
                    "halation" => halation(
                        e,
                        region,
                        &mask,
                        linear.as_ref().unwrap(),
                        w,
                        h,
                        &protocol["halation"],
                    )?,
                    "edge" => edge(
                        e,
                        region,
                        &mask,
                        linear.as_ref().unwrap(),
                        w,
                        h,
                        &protocol["edge"],
                    )?,
                    _ => return Err(err("invalid_protocol", format!("Unknown family: {active}"))),
                }
            }
            Ok(())
        })();
        if let Err(error) = execution {
            e.scalar(
                region,
                active,
                None,
                0,
                "declared_measurement",
                "unavailable",
                "selected family execution",
                &[],
                Some(&error.code),
            );
            e.failure(
                region,
                active,
                &error,
                &refs,
                ![
                    "missing_probe_geometry",
                    "insufficient_samples",
                    "zero_edge_contrast",
                ]
                .contains(&error.code.as_str()),
            );
        }
    }
    drop(encoded);
    let ranking = maps.last().filter(|m| m.name == "flip").unwrap_or(&maps[0]);
    e.extra.insert(
        "automatic_regions".into(),
        json!(select_hotspots(
            &ranking.values,
            w,
            h,
            &protocol["hotspots"],
            ranking.name,
            std::slice::from_ref(&ranking.artifact)
        )),
    );
    e.extra
        .insert("region_readouts".into(), json!(color_readouts));
    Ok(())
}
pub fn compare_images(
    a: &ImageData,
    b: &ImageData,
    regions: &[Value],
    protocol: &Value,
    out: &Path,
) -> Result<Value> {
    let mut evidence = Evidence::new(protocol, out);
    let populations = populations(regions, protocol);
    if let Err(error) = compare_inner(&mut evidence, a, b, &populations, protocol) {
        evidence.failure("full_image", "image", &error, &[], true);
        let reason = error.code.clone();
        let mut unavailable = Vec::new();
        for population in populations {
            for family in population["measurements"].as_array().into_iter().flatten() {
                evidence.scalar(
                    population["id"].as_str().unwrap_or(""),
                    family.as_str().unwrap_or(""),
                    None,
                    0,
                    "declared_measurement",
                    "unavailable",
                    "selected family boundary failure",
                    &[],
                    Some(&reason),
                );
            }
        }
        for region in regions {
            unavailable.extend(color_readouts::unavailable_pair(
                region["id"].as_str().unwrap_or(""),
                &reason,
            ));
        }
        evidence
            .extra
            .insert("region_readouts".into(), json!(unavailable));
    }
    Ok(evidence.finish())
}
pub fn select_hotspots(
    values: &[f64],
    width: usize,
    height: usize,
    settings: &Value,
    metric: &str,
    refs: &[String],
) -> Vec<Value> {
    if values.len() != width * height || values.iter().any(|x| !x.is_finite()) {
        return Vec::new();
    }
    let requested = settings.get("count").and_then(Value::as_u64).unwrap_or(0) as usize;
    let minimum = settings
        .get("min_area_px")
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;
    if requested == 0 || minimum == 0 {
        return Vec::new();
    }
    let side = (minimum as f64).sqrt().ceil() as usize;
    let mut rectangles = Vec::new();
    for y in (0..height).step_by(side) {
        for x in (0..width).step_by(side) {
            let w = side.min(width - x);
            let h = side.min(height - y);
            if w * h >= minimum {
                rectangles.push([x, y, w, h]);
            }
        }
    }
    let rank = |rect: [usize; 4]| {
        let mut sum = 0.0;
        for y in rect[1]..rect[1] + rect[3] {
            sum += values[y * width + rect[0]..y * width + rect[0] + rect[2]]
                .iter()
                .sum::<f64>();
        }
        (-sum / (rect[2] * rect[3]) as f64, rect[1], rect[0])
    };
    rectangles.sort_by(|a, b| rank(*a).partial_cmp(&rank(*b)).unwrap());
    rectangles.truncate(requested);
    loop {
        let mut pair = None;
        'search: for i in 0..rectangles.len() {
            for j in i + 1..rectangles.len() {
                let a = rectangles[i];
                let b = rectangles[j];
                if (a[0] <= b[0] + b[2]
                    && b[0] <= a[0] + a[2]
                    && a[1] < b[1] + b[3]
                    && b[1] < a[1] + a[3])
                    || (a[1] <= b[1] + b[3]
                        && b[1] <= a[1] + a[3]
                        && a[0] < b[0] + b[2]
                        && b[0] < a[0] + a[2])
                {
                    pair = Some((i, j));
                    break 'search;
                }
            }
        }
        let Some((i, j)) = pair else {
            break;
        };
        let a = rectangles[i];
        let b = rectangles.remove(j);
        let x = a[0].min(b[0]);
        let y = a[1].min(b[1]);
        rectangles[i] = [
            x,
            y,
            (a[0] + a[2]).max(b[0] + b[2]) - x,
            (a[1] + a[3]).max(b[1] + b[3]) - y,
        ];
    }
    rectangles.sort_by(|a, b| rank(*a).partial_cmp(&rank(*b)).unwrap());
    rectangles.into_iter().enumerate().map(|(i,rectangle)|json!({"id":format!("automatic_{:03}",i+1),"geometry":rectangle,"rectangle":rectangle,"ranking_metric":metric,"ranking_value":-rank(rectangle).0,"selection_method":"rank square tiles by mean descending, ties y/x; select count; merge touching or overlapping rectangles to fixed point; rerank merged rectangle means","artifact_refs":refs})).collect()
}
pub fn aggregate_seed_measurements(results: &[Value]) -> Vec<Value> {
    let mut expected: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut groups: BTreeMap<Vec<String>, Vec<Value>> = BTreeMap::new();
    for result in results {
        if let Some(seed) = result.get("seed") {
            expected
                .entry(result["case_id"].as_str().unwrap_or("").into())
                .or_default()
                .push(seed.clone());
        }
        for row in result["measurements"].as_array().into_iter().flatten() {
            let key = [
                "case_id",
                "region_id",
                "metric",
                "domain",
                "units",
                "estimator",
            ]
            .map(|k| row[k].as_str().unwrap_or("").to_owned())
            .to_vec();
            groups.entry(key).or_default().push(row.clone());
        }
    }
    let mut output = Vec::new();
    for (key, rows) in groups {
        let seeds: Vec<Value> = rows
            .iter()
            .flat_map(|r| {
                r["seed_population"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .cloned()
            })
            .collect();
        let wanted = expected.get(&key[0]).unwrap_or(&seeds);
        let set = |v: &[Value]| v.iter().map(Value::to_string).collect::<HashSet<_>>();
        let complete = set(&seeds) == set(wanted);
        let available = complete && rows.iter().all(|r| r["availability"] == "available");
        let values: Vec<f64> = rows
            .iter()
            .filter(|r| r["availability"] == "available")
            .filter_map(|r| r["value"].as_f64())
            .collect();
        let mut refs = Vec::new();
        let mut seen = HashSet::new();
        for r in &rows {
            for reference in r["artifact_refs"].as_array().into_iter().flatten() {
                if seen.insert(reference.to_string()) {
                    refs.push(reference.clone());
                }
            }
        }
        for suffix in ["seed_mean", "seed_std"] {
            let mut row = rows[0].clone();
            let object = row.as_object_mut().unwrap();
            object.remove("value");
            object.remove("reason");
            row["metric"] = json!(format!("{}.{suffix}", key[2]));
            row["seed_population"] = json!(seeds);
            row["sample_count"] = json!(rows.len());
            row["seed_count"] = json!(rows.len());
            row["estimator"] = json!(if suffix == "seed_mean" {
                "equal-weight per-seed arithmetic mean"
            } else {
                "cross-seed sample standard deviation ddof=1"
            });
            row["artifact_refs"] = json!(refs);
            row["per_seed_measurements"] = json!(rows);
            row["expected_seed_population"] = json!(wanted);
            let reason = if !complete {
                Some("missing_seed_measurement")
            } else if !available {
                Some("unavailable_seed_measurement")
            } else if suffix == "seed_std" && rows.len() < 2 {
                Some("insufficient_seeds")
            } else {
                None
            };
            if let Some(reason) = reason {
                row["availability"] = json!("unavailable");
                row["reason"] = json!(reason);
            } else {
                let average = mean(&values).unwrap();
                let value = if suffix == "seed_mean" {
                    average
                } else {
                    (values.iter().map(|v| (v - average).powi(2)).sum::<f64>()
                        / (values.len() - 1) as f64)
                        .sqrt()
                };
                row["availability"] = json!("available");
                row["value"] = json!(value);
            }
            output.push(row);
        }
    }
    output
}

fn metric_definitions(case: &Value, region_id: &str, protocol: &Value) -> HashSet<String> {
    let mut definitions = HashSet::new();
    let families = if region_id == "full_image" {
        case.get("measurements")
            .or_else(|| protocol.get("measurements"))
    } else {
        case["regions"]
            .as_array()
            .and_then(|regions| regions.iter().find(|r| r["id"].as_str() == Some(region_id)))
            .and_then(|r| r.get("measurements"))
    };
    let default_families = json!(["numeric", "color", "tone"]);
    let chosen: HashSet<&str> = families
        .or_else(|| (region_id == "full_image").then_some(&default_families))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if chosen.contains("numeric") {
        for name in ["max_abs_difference", "rmse"] {
            definitions.insert(format!("numeric.{name}"));
        }
        for side in ["baseline", "candidate"] {
            for name in [
                "nonfinite_sample_count",
                "nonfinite_pixel_count",
                "below_zero_fraction",
                "above_one_fraction",
            ] {
                definitions.insert(format!("numeric.{side}.{name}"));
            }
        }
    }
    if chosen.contains("color") {
        for stat in ["median", "p95", "p99", "max"] {
            definitions.insert(format!("delta_e00.{stat}"));
        }
        for name in [
            "delta_l_star",
            "delta_a_star",
            "delta_b_star",
            "delta_c_star",
        ] {
            for stat in ["mean", "median"] {
                definitions.insert(format!("{name}.{stat}"));
            }
        }
        for threshold in protocol["delta_e_thresholds"]
            .as_array()
            .into_iter()
            .flatten()
        {
            definitions.insert(format!("delta_e00.fraction_above_{}", spelling(threshold)));
        }
    }
    if chosen.contains("flip") {
        for stat in ["mean", "p95", "p99", "max"] {
            definitions.insert(format!("flip.{stat}"));
        }
    }
    for side in ["baseline", "candidate", "delta"] {
        if chosen.contains("tone") {
            for q in [5, 25, 50, 75, 95] {
                definitions.insert(format!("tone.l_star.{side}.q{q:02}"));
            }
            for scale in protocol["region_sampling_scales"]
                .as_array()
                .into_iter()
                .flatten()
            {
                definitions.insert(format!(
                    "tone.local_contrast.sigma_{}.{side}.rms",
                    spelling(scale)
                ));
            }
            if region_id == "full_image" {
                let points = case["probe"]
                    .get("tone_response")
                    .filter(|value| !value.is_null())
                    .unwrap_or(&protocol["probe"]["tone_response"]);
                let n = points.as_array().map_or(0, Vec::len);
                for interval in 0..n.saturating_sub(1) {
                    for label in ["linear_luminance", "l_star"] {
                        definitions.insert(format!(
                            "tone.response.interval_{interval}.{label}.{side}.slope"
                        ));
                    }
                }
            }
        }
        if chosen.contains("grain") {
            let flat = case.get("probe").unwrap_or(&protocol["probe"])["kind"].as_str()
                == Some("flat_field")
                || case["regions"].as_array().into_iter().flatten().any(|r| {
                    r["id"].as_str() == Some(region_id) && r["role"].as_str() == Some("flat_field")
                });
            let requested = protocol["grain"]["residual"]["method"]
                .as_str()
                .unwrap_or(if flat {
                    "flat_mean"
                } else {
                    "gaussian_high_pass"
                });
            let method = if requested == "flat_mean" && !flat {
                "gaussian_high_pass"
            } else {
                requested
            };
            for channel in ["R", "G", "B", "luminance", "chromatic"] {
                definitions.insert(format!("grain.{method}.{side}.rms.{channel}"));
            }
            for pair in ["RG", "RB", "GB"] {
                for name in ["covariance", "correlation"] {
                    definitions.insert(format!("grain.{method}.{side}.{name}.{pair}"));
                }
            }
            for name in ["psd.luminance_total", "frequency_centroid_cycles_per_pixel"] {
                definitions.insert(format!("grain.{method}.{side}.{name}"));
            }
        }
        if chosen.contains("halation") {
            definitions.insert(format!("halation.{side}.core_clipped_fraction"));
            definitions.insert(format!("halation.{side}.red_minus_green.integrated_spill"));
            for channel in ["R", "G", "B"] {
                for name in [
                    "peak_amplitude",
                    "peak_radius_px",
                    "half_height_width_px",
                    "integrated_spill",
                ] {
                    definitions.insert(format!("halation.{side}.{channel}.{name}"));
                }
            }
        }
        if chosen.contains("edge") {
            for name in [
                "overshoot",
                "undershoot",
                "mtf50_cycles_per_pixel",
                "plateau_contrast",
            ] {
                definitions.insert(format!("edge.{side}.{name}"));
            }
        }
    }
    definitions
}
pub fn metric_supported_with_protocol(
    metric: &str,
    case: &Value,
    region_id: &str,
    protocol: &Value,
) -> bool {
    let metric = metric
        .strip_suffix(".seed_mean")
        .or_else(|| metric.strip_suffix(".seed_std"))
        .unwrap_or(metric);
    if metric.starts_with("reference.") {
        let Some(base) = crate::references::metric_base_id(metric) else {
            return false;
        };
        return crate::references::metric_supported(base, case, region_id, protocol);
    }
    metric_definitions(case, region_id, protocol).contains(metric)
}

#[cfg(test)]
mod tests {
    use super::families::mtf50;
    use super::*;
    use std::{fs, path::PathBuf};

    #[test]
    fn policy_reference_relation_only_accepts_the_measured_pair_anchor() {
        let case = json!({
            "reference":{"scope":"regional","regions":[{"id":"a"},{"id":"b"}]},
            "region_pairs":[{"id":"ab","left":"a","right":"b"}]
        });
        let protocol = json!({"color":{"hue_chroma_floor":2.0}});
        assert!(metric_supported_with_protocol(
            "reference.pair_separation_delta_e00.reference_relation",
            &case,
            "ab",
            &protocol
        ));
        for (base, region) in [
            ("reference.patch_delta_e00", "a"),
            ("reference.patch_hue_shift", "a"),
            ("reference.pair_separation_change", "ab"),
            ("reference.pair_separation_departure", "ab"),
        ] {
            assert!(
                !metric_supported_with_protocol(
                    &format!("{base}.reference_relation"),
                    &case,
                    region,
                    &protocol
                ),
                "unmeasured relation must not become a valid policy selector: {base}"
            );
            for side in [
                "baseline_distance",
                "candidate_distance",
                "distance_delta",
                "simulation_distance",
            ] {
                assert!(metric_supported_with_protocol(
                    &format!("{base}.{side}"),
                    &case,
                    region,
                    &protocol
                ));
            }
        }
    }
    fn image(value: f64, width: usize, height: usize) -> ImageData {
        ImageData {
            samples: vec![[value; 3]; width * height],
            width,
            height,
            color_space: "sRGB".into(),
            transfer: "encoded".into(),
            metadata: json!({}),
            path: None,
        }
    }
    fn output() -> PathBuf {
        std::env::temp_dir().join(format!(
            "benchmark-metric-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn structured_boundary_failure() {
        let a = image(0.4, 4, 4);
        let b = image(0.4, 5, 4);
        let result = compare_images(
            &a,
            &b,
            &[],
            &json!({"measurements":["numeric","color"]}),
            &output(),
        )
        .unwrap();
        assert_eq!(result["execution_status"], "failed");
        assert_eq!(result["measurements"][0]["reason"], "dimension_mismatch");
        assert_eq!(result["region_readouts"], json!([]));
        let mut b = a.clone();
        b.samples[3][1] = f64::NAN;
        let result =
            compare_images(&a, &b, &[], &json!({"measurements":["numeric"]}), &output()).unwrap();
        assert_eq!(
            result["numeric_health"]["candidate"]["nonfinite_pixel_count"],
            1
        );
        assert_eq!(result["measurements"][0]["reason"], "nonfinite_samples");
    }
    #[test]
    fn identical_and_signed_official_flip() {
        let a = image(0.4, 16, 16);
        let path = output();
        let protocol = json!({"measurements":["numeric","color","flip"],"pixels_per_degree":67});
        let result = compare_images(&a, &a, &[], &protocol, &path).unwrap();
        assert_eq!(result["execution_status"], "completed");
        for row in result["measurements"].as_array().unwrap() {
            if row["metric"].as_str().unwrap().starts_with("flip.") {
                assert_eq!(row["value"], 0.0);
            }
        }
        fs::remove_dir_all(path).unwrap();
        let path = output();
        let result = compare_images(&a, &image(0.5, 16, 16), &[], &protocol, &path).unwrap();
        let rows = result["measurements"].as_array().unwrap();
        assert!(
            rows.iter()
                .find(|r| r["metric"] == "delta_l_star.mean")
                .unwrap()["value"]
                .as_f64()
                .unwrap()
                > 0.0
        );
        assert!(
            rows.iter().find(|r| r["metric"] == "flip.mean").unwrap()["value"]
                .as_f64()
                .unwrap()
                > 0.0
        );
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn flat_zero_variance_retains_radial_spectrum() {
        let a = image(0.0, 8, 8);
        let path = output();
        let result=compare_images(&a,&a,&[],&json!({"measurements":["grain"],"probe":{"kind":"flat_field"},"grain":{"residual":{"method":"flat_mean"},"spectrum":{"window":"boxcar"}}}),&path).unwrap();
        let row = result["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["metric"] == "grain.flat_mean.baseline.correlation.RG")
            .unwrap();
        assert_eq!(row["reason"], "zero_variance");
        assert!(
            result["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"] == "grain.full_image.baseline.radial_psd_artifact")
        );
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn missing_seed_precedes_insufficient_seeds() {
        let row = json!({"case_id":"a","region_id":"full_image","metric":"numeric.rmse","domain":"native","units":"sample","estimator":"rmse","availability":"available","value":1.0,"seed_population":[1],"artifact_refs":[]});
        let result = aggregate_seed_measurements(&[
            json!({"case_id":"a","seed":1,"measurements":[row]}),
            json!({"case_id":"a","seed":2,"measurements":[]}),
        ]);
        assert!(
            result
                .iter()
                .all(|r| r["reason"] == "missing_seed_measurement")
        );
    }
    #[test]
    fn hotspot_ties_and_touching_merge() {
        let values = vec![1.0; 16];
        let regions = select_hotspots(
            &values,
            4,
            4,
            &json!({"count":2,"min_area_px":4}),
            "flip",
            &[],
        );
        assert_eq!(regions[0]["rectangle"], json!([0, 0, 4, 2]));
        assert!(
            select_hotspots(
                &values,
                4,
                4,
                &json!({"count":3,"min_area_px":17}),
                "flip",
                &[]
            )
            .is_empty()
        );
        assert_eq!(mtf50(&[1.0, 0.75, 0.25], 8), Some(0.1875));
        assert_eq!(mtf50(&[1.0, 0.7], 8), None);
    }
    /// Declared regions only: an explicit `full_image` region is rejected by
    /// the population builder, so the retained readouts must never include an
    /// implicit full-image row.
    #[test]
    fn declared_regions_retain_two_estimator_readouts_with_signed_axes() {
        let path = output();
        // One of four selected pixels is brighter; the median stays fixed.
        let mut samples = vec![[0.5; 3]; 16];
        samples[1] = [0.9; 3];
        let b = ImageData {
            samples,
            width: 4,
            height: 4,
            color_space: "sRGB".into(),
            transfer: "encoded".into(),
            metadata: json!({}),
            path: None,
        };
        let result = compare_images(
            &image(0.5, 4, 4),
            &b,
            &[json!({"id":"patch","geometry":{"rectangle":[0,0,2,2]},"measurements":["color"]})],
            &json!({"measurements":["numeric","color"],"color":{"hue_chroma_floor":0.5}}),
            &path,
        )
        .unwrap();
        fs::remove_dir_all(path).unwrap();
        let readouts = result["region_readouts"].as_array().unwrap();
        assert_eq!(readouts.len(), 2, "one row per estimator");
        let median = readouts
            .iter()
            .find(|r| r["estimator"] == "bounded_lab_channel_median")
            .unwrap();
        let mean = readouts
            .iter()
            .find(|r| r["estimator"] == "unclipped_linear_srgb_mean")
            .unwrap();
        assert_eq!(median["domain"], "bounded_srgb_d65_lab");
        assert_eq!(mean["domain"], "unclipped_linear_srgb_mean_lab");
        assert!(median["sides"]["baseline"]["lch"]["h_degrees"]["availability"] == "unavailable");
        assert!(mean["sides"]["candidate"]["lch"]["h_degrees"]["availability"] == "unavailable");
        let displacement = &median["displacements"][0];
        assert_eq!(displacement["delta_lab"]["value"], json!([0., 0., 0.]));
        assert!(
            mean["displacements"][0]["delta_l_star"]["value"]
                .as_f64()
                .unwrap()
                > 0.
        );
        // The two estimators are genuinely different populations.
        let median_center = median["sides"]["candidate"]["lab"][0].as_f64().unwrap();
        let mean_center = mean["sides"]["candidate"]["lab"][0].as_f64().unwrap();
        assert!(
            (median_center - mean_center).abs() > 1e-6,
            "median {median_center} and linear mean {mean_center} must stay distinct"
        );
        assert!(mean["sides"]["candidate"]["mean_linear_srgb"].is_array());
        assert!(
            median["sides"]["candidate"]
                .get("mean_linear_srgb")
                .is_none()
        );
    }
    #[test]
    fn unselected_color_family_reports_reason_instead_of_coordinates() {
        let path = output();
        let result = compare_images(
            &image(0.5, 4, 4),
            &image(0.5, 4, 4),
            &[json!({"id":"patch","geometry":{"rectangle":[0,0,2,2]},"measurements":["numeric"]})],
            &json!({"measurements":["numeric", "color"],"color":{"hue_chroma_floor":0.5}}),
            &path,
        )
        .unwrap();
        fs::remove_dir_all(path).unwrap();
        let readouts = result["region_readouts"].as_array().unwrap();
        assert_eq!(readouts.len(), 2);
        assert!(readouts.iter().all(|r| r["availability"] == "unavailable"
            && r["reason"] == "color_family_not_selected"
            && r.get("sides").is_none()
            && r.get("displacements").is_none()));
    }
}
