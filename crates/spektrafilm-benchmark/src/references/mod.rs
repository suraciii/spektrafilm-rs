mod aligned;
mod artifacts;
mod catalog;
mod io;
mod material;
mod regional;
mod transform;

pub use artifacts::Comparison;
pub use catalog::{base_id as metric_base_id, hue_floor, metric_supported};
pub use regional::validate_region_pairs;
pub use transform::validate_correspondence_documents;

use aligned::*;
use artifacts::*;
use io::*;
use material::*;
use regional::*;
#[cfg(test)]
use transform::*;

use crate::common::Result;
use crate::image_data::{self, ImageData};
#[cfg(test)]
use crate::{common, documents};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

pub const METRIC_VERSION: &str = "spektrafilm-reference-v1";
const DOMAIN: &str = "bounded_srgb_d65_lab";
const COLOR_TRANSFORM: &str = "srgb-d65-cie1931-2deg-bounded-v1";
const SIDES: [&str; 3] = ["baseline_distance", "candidate_distance", "distance_delta"];

pub fn compare_scope(
    c: &mut Comparison,
    images: &[&ImageData],
    case: &Value,
    base: &Path,
) -> Result<()> {
    let reference = c.reference;
    let scope = text(reference, "scope")?;
    if scope == "material" {
        return material(c, images, case, base);
    }
    let source = read_reference(reference, base)?;
    finite_rgb(
        &source.samples,
        "nonfinite_samples",
        "Reference contains nonfinite samples",
    )?;
    let master = c.rgb_artifact(
        ArtifactRole::Native,
        &source.samples,
        source.dimensions(),
        if source.color_space.is_empty() {
            "unknown"
        } else {
            &source.color_space
        },
        false,
    )?;
    c.review["master_artifact_id"] = json!(master);
    c.review["native_dimensions"] = json!(source.dimensions());
    c.review["decode_metadata"] = source.metadata.clone();
    c.review_ref(&master);
    let converted = image_data::display_rgb(&source).and_then(|v| {
        finite_rgb(
            &v,
            "computation_failure",
            "Reference color conversion produced nonfinite samples",
        )?;
        Ok(v)
    });
    let (encoded, color_reason) = match converted {
        Ok(v) => {
            c.review["display_clipped_sample_fraction"] = json!(clipping_fraction(&v));
            c.review["color_available"] = json!(true);
            (v, None)
        }
        Err(e) => {
            c.review["color_available"] = json!(false);
            c.limitation("Preview uses uninterpreted native RGB; color fidelity is unavailable.");
            (source.samples.clone(), Some(e.code))
        }
    };
    let display = c.preview(&encoded, source.dimensions(), color_reason.is_none())?;
    c.review["display_artifact_id"] = json!(display);
    c.review["native_display_artifact_id"] = json!(display);
    c.review_ref(&display);
    if scope == "visual" {
        c.review["tone_target_eligible"] = json!(false);
        c.review["color_diagnostic_eligible"] = json!(false);
        let mut regions = Vec::new();
        for region in optional_array(reference, "regions") {
            let geometry = field(region, "geometry")?;
            let pixels = mask(geometry, source.dimensions(), base)?;
            if !pixels.iter().any(|selected| *selected) {
                return Err(err(
                    "empty_reference_population",
                    "Visual reference region has no eligible pixels",
                ));
            }
            regions.push(json!({
                "region_id": text(region, "id")?,
                "label": region.get("label"),
                "role": region.get("role"),
                "reference_geometry": geometry,
            }));
        }
        c.review["regions"] = json!(regions);
        c.limitation(
            "Visual reference only; no numerical fidelity or exact scene correspondence claim.",
        );
        for region in optional_array(reference, "regions") {
            c.readouts(crate::color_readouts::unavailable_pair(
                text(region, "id")?,
                "visual_only_reference",
            ));
        }
        if c.single {
            c.distances(
                "full_image",
                "reference.visual",
                "visual_review",
                "not applicable",
                "visual reference; numerical distance prohibited",
                0,
                None,
                Some("visual_only_reference"),
                &[display],
            );
        }
        return Ok(());
    }
    for image in images {
        if image.dimensions() != images[0].dimensions() {
            return Err(err("dimension_mismatch", "Rendered geometry differs"));
        }
        finite_rgb(
            &image.samples,
            "nonfinite_samples",
            "Rendered evidence contains nonfinite samples",
        )?;
    }
    if let Some(reason) = color_reason {
        return Err(err(
            &reason,
            "Reference color interpretation is unavailable",
        ));
    }
    let mut rendered = Vec::with_capacity(images.len());
    let mut labs = Vec::with_capacity(images.len());
    for image in images {
        let rgb = image_data::display_rgb(image)?;
        finite_rgb(
            &rgb,
            "computation_failure",
            "Rendered display conversion contains nonfinite samples",
        )?;
        labs.push(clipped_lab(&rgb));
        rendered.push(rgb);
    }
    let (tone, chroma) = processing_eligibility(reference)?;
    c.review["tone_target_eligible"] = json!(tone);
    c.review["color_diagnostic_eligible"] = json!(chroma);
    if !tone {
        c.limitation("Grading/histogram history prevents an unmodified tone or full DE00 fidelity objective.");
    }
    match scope {
        "aligned_scene" => aligned(
            c,
            &source,
            &encoded,
            &labs,
            &rendered,
            images[0].dimensions(),
            case,
            base,
            tone,
            chroma,
        ),
        "regional" => regional(
            c,
            &clipped_lab(&encoded),
            &encoded,
            source.dimensions(),
            &labs,
            &rendered,
            images[0].dimensions(),
            case,
            base,
            tone,
            chroma,
        ),
        _ => Err(err(
            "unsupported_reference_scope",
            "Unknown reference scope",
        )),
    }
}
pub fn image_specs(c: &mut Comparison, region: &str, reason: &str, regional: bool) {
    if regional {
        for metric in catalog::REGIONAL_METRICS {
            c.distances(
                region,
                metric.id,
                DOMAIN,
                metric.units,
                metric.estimator,
                0,
                None,
                Some(reason),
                &[],
            );
        }
        return;
    }
    let mut specs: Vec<(String, &str)> = ["median", "p95", "p99", "max"]
        .iter()
        .map(|s| (format!("reference.delta_e00.{s}"), "delta E 00"))
        .collect();
    specs.extend(
        [5, 25, 50, 75, 95]
            .iter()
            .map(|q| (format!("reference.tone_l_star.q{q:02}"), "L*")),
    );
    specs.push(("reference.chroma_ab.median".into(), "Lab a*/b* distance"));
    for (metric, units) in specs {
        c.distances(
            region,
            &metric,
            DOMAIN,
            units,
            "frozen reference population",
            0,
            None,
            Some(reason),
            &[],
        );
    }
}

/// Unavailable region-pair rows for a failed reference evaluation, so selected
/// pair metrics stay visible with their reason instead of silently vanishing.
pub fn pair_specs(c: &mut Comparison, pair: &str, reason: &str) {
    c.relation_row(
        pair,
        catalog::PAIR_SEPARATION.id,
        DOMAIN,
        catalog::PAIR_SEPARATION.units,
        catalog::PAIR_SEPARATION.estimator,
        0,
        None,
        Some(reason),
        &[],
    );
    for metric in catalog::PAIR_METRICS {
        c.distances(
            pair,
            metric.id,
            DOMAIN,
            metric.units,
            metric.estimator,
            0,
            None,
            Some(reason),
            &[],
        );
    }
}
pub fn compare_reference(
    baseline: &ImageData,
    candidate: &ImageData,
    case: &Value,
    protocol: &Value,
    out_dir: &Path,
    suite_base: &Path,
) -> Result<Value> {
    compare_reference_population(
        &[baseline, candidate],
        case,
        protocol,
        out_dir,
        suite_base,
        false,
    )
}

pub fn compare_reference_single(
    simulation: &ImageData,
    case: &Value,
    protocol: &Value,
    out_dir: &Path,
    suite_base: &Path,
) -> Result<Value> {
    compare_reference_population(&[simulation], case, protocol, out_dir, suite_base, true)
}

pub fn compare_reference_population(
    images: &[&ImageData],
    case: &Value,
    protocol: &Value,
    out_dir: &Path,
    suite_base: &Path,
    single: bool,
) -> Result<Value> {
    let Some(reference) = case.get("reference").filter(|v| !v.is_null()) else {
        return Ok(
            json!({"measurements":[],"findings":[],"artifacts":[],"execution_status":"completed","metric_version":METRIC_VERSION,"reference_review":null}),
        );
    };
    let protocol = if protocol.is_object() {
        protocol.clone()
    } else {
        json!({"id":protocol})
    };
    let mut c = Comparison::new(reference, case, &protocol, out_dir)?;
    c.single = single;
    fs::create_dir_all(out_dir)?;
    if let Err(error) = compare_scope(&mut c, images, case, suite_base) {
        // Python distinguishes typed image errors from generic I/O/document failures.
        let reason = if [
            "operation_failed",
            "document_read",
            "document_type",
            "image_read_failed",
        ]
        .contains(&error.code.as_str())
        {
            "invalid_reference_evidence".to_owned()
        } else {
            error.code.clone()
        };
        c.finding(
            "full_image",
            "reference",
            &reason,
            matches!(reason.as_str(), "nonfinite_samples" | "computation_failure"),
            &[],
        );
        c.review["unavailable_reason"] = json!(reason);
        c.limitation(error.message);
        let existing: BTreeSet<String> = c
            .body
            .measurements
            .iter()
            .map(|m| m.shared.region_id.clone())
            .collect();
        // Every declared region keeps a readout row with the same explicit
        // reason the scalar diagnostics report, so no comparison silently
        // loses a region it used to measure.
        let readout_regions = optional_array(case, "regions")
            .iter()
            .filter_map(|region| region["id"].as_str());
        for id in readout_regions {
            if !c
                .readouts
                .iter()
                .any(|r| r["region_id"].as_str() == Some(id))
            {
                c.readouts(crate::color_readouts::unavailable_pair(id, &reason));
            }
        }
        match reference.get("scope").and_then(Value::as_str).unwrap_or("") {
            "aligned_scene" => {
                for id in std::iter::once("full_image").chain(
                    optional_array(case, "regions")
                        .iter()
                        .filter_map(|r| r.get("id").and_then(Value::as_str)),
                ) {
                    if !existing.contains(id) {
                        image_specs(&mut c, id, &reason, false);
                    }
                }
            }
            "regional" => {
                let ids: Vec<&str> = optional_array(reference, "regions")
                    .iter()
                    .filter_map(|r| r.get("id").and_then(Value::as_str))
                    .collect();
                if ids.is_empty() {
                    image_specs(&mut c, "full_image", &reason, true);
                } else {
                    for id in ids {
                        if !existing.contains(id) {
                            image_specs(&mut c, id, &reason, true);
                        }
                    }
                }
                for id in optional_array(case, "region_pairs")
                    .iter()
                    .filter_map(|r| r.get("id").and_then(Value::as_str))
                {
                    if !existing.contains(id) {
                        pair_specs(&mut c, id, &reason);
                    }
                }
            }
            "material" => {
                let mut ids: Vec<&str> = case
                    .get("probe_response")
                    .map(|p| {
                        optional_array(p, "curves")
                            .iter()
                            .filter_map(|r| r.get("id").and_then(Value::as_str))
                            .collect()
                    })
                    .unwrap_or_default();
                if ids.is_empty() {
                    ids.push("response");
                }
                for id in ids {
                    let metric = format!("reference.material.{id}");
                    if !c
                        .body
                        .measurements
                        .iter()
                        .any(|m| m.shared.metric.starts_with(&format!("{metric}.")))
                    {
                        c.distances(
                            "full_image",
                            &metric,
                            reference
                                .get("domain")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown"),
                            reference
                                .get("units")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown"),
                            "matched coordinate RMSE",
                            0,
                            None,
                            Some(&reason),
                            &[],
                        );
                    }
                }
            }
            _ => {}
        }
    }
    Ok(c.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn transform(matrix: Value, interpolation: &str) -> FrozenTransform {
        FrozenTransform::parse(&json!({"matrix":matrix,"direction":"reference_to_render","coordinate_system":"pixel_centers","interpolation":interpolation})).unwrap()
    }
    #[test]
    fn rectangular_identity_retains_geometry_and_pixels() {
        let pixels: Vec<[f64; 3]> = (0..6).map(|i| [i as f64; 3]).collect();
        let t = transform(json!([[1, 0, 0], [0, 1, 0], [0, 0, 1]]), "nearest");
        let (v, s) = t.warp(&pixels, [3, 2], [3, 2]).unwrap();
        assert_eq!(v, pixels);
        assert_eq!(s, vec![true; 6]);
    }
    #[test]
    fn translation_support_uses_pixel_centers_and_retains_border_sample() {
        let pixels = vec![[1.; 3], [2.; 3], [3.; 3]];
        let t = transform(json!([[1, 0, 1], [0, 1, 0], [0, 0, 1]]), "nearest");
        let (v, s) = t.warp(&pixels, [3, 1], [3, 1]).unwrap();
        assert_eq!(s, vec![false, true, true]);
        assert_eq!(v, vec![[1.; 3], [1.; 3], [2.; 3]]);
    }
    #[test]
    fn bilinear_half_pixel_is_interpolated_in_rgb_before_lab() {
        let t = transform(json!([[1, 0, -0.5], [0, 1, 0], [0, 0, 1]]), "bilinear");
        let (v, s) = t.warp(&[[0.; 3], [1.; 3]], [2, 1], [2, 1]).unwrap();
        assert_eq!(v[0], [0.5; 3]);
        assert_eq!(s, vec![true, false]);
    }
    #[test]
    fn singular_and_undeclared_transforms_are_rejected() {
        assert!(FrozenTransform::parse(&json!({"matrix":[[1,0,0],[0,0,0],[0,0,1]],"direction":"reference_to_render","coordinate_system":"pixel_centers","interpolation":"nearest"})).is_err());
        assert!(FrozenTransform::parse(&json!({"matrix":[[1,0,0],[0,1,0],[0,0,1]]})).is_err());
    }
    #[test]
    fn brightness_only_retains_diagnostic_chroma_but_blocks_tone() {
        assert_eq!(
            processing_eligibility(
                &json!({"processing":{"grading":" NONE ","histogram_matching":"brightness_only"}})
            )
            .unwrap(),
            (false, true)
        );
        assert_eq!(
            processing_eligibility(
                &json!({"processing":{"grading":"unknown","histogram_matching":"none"}})
            )
            .unwrap(),
            (false, false)
        );
        assert_eq!(
            processing_eligibility(
                &json!({"processing":{"grading":"unmodified","histogram_matching":"not_applied"}})
            )
            .unwrap(),
            (true, true)
        );
    }
    #[test]
    fn material_estimators_reduce_population_and_enforce_channel_identity() {
        let med = ProbeEstimator::parse("native_channel_median").unwrap();
        assert_eq!(med.reduce(vec![0., 1., 1., 10.]), 1.);
        assert_eq!(med.channel("R"), Some(0));
        assert_eq!(med.channel("Y"), None);
        let mean = ProbeEstimator::parse("linear_luminance_mean").unwrap();
        assert_eq!(mean.reduce(vec![1., 3.]), 2.);
        assert_eq!(mean.channel("Y"), Some(0));
        assert!(ProbeEstimator::parse("density_from_rgb").is_err());
    }
    #[test]
    fn overflow_distance_fails_and_never_serializes_a_numeric_value() {
        let reference = json!({"id":"r","scope":"material"});
        let case = json!({"id":"c","seeds":[0]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), Path::new(".")).unwrap();
        c.distances(
            "full_image",
            "reference.material.a",
            "native_rgb",
            "native channel value",
            "test",
            2,
            Some(vec![Some(-f64::MAX), Some(f64::MAX)]),
            None,
            &[],
        );
        let out = c.finish();
        assert_eq!(out["execution_status"], "failed");
        for row in out["measurements"].as_array().unwrap() {
            assert_eq!(row["availability"], "unavailable");
            assert!(row.get("value").is_none());
            assert_eq!(row["reason"], "computation_failure");
        }
    }
    fn image(values: Vec<[f64; 3]>, width: usize, height: usize) -> ImageData {
        ImageData {
            samples: values,
            width,
            height,
            color_space: "sRGB".into(),
            transfer: "encoded".into(),
            metadata: json!({}),
            path: None,
        }
    }
    #[test]
    fn material_loads_json_before_raster_and_retains_signed_rmse_curve() {
        let temp = tempfile::tempdir().unwrap();
        let coordinate = json!({"name":"exposure","units":"stops"});
        let conditions = json!({"temperature_c":20});
        let source = json!({"schema_version":1,"domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions,"coordinate":coordinate,"curves":[{"id":"red","channel":"R","coordinates":[0.,1.],"values":[0.,1.]}]});
        common::dump(&temp.path().join("material.json"), &source).unwrap();
        let reference = json!({"id":"measured","scope":"material","path":"material.json","domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions});
        let response = json!({"domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions,"coordinate":coordinate,"estimator":"native_channel_mean","curves":[{"id":"red","channel":"R","samples":[{"coordinate":0.,"region_id":"left"},{"coordinate":1.,"region_id":"right"}]}]});
        let case = json!({"id":"material","seeds":[0],"reference":reference,"probe_response":response,"regions":[{"id":"left","geometry":[0,0,1,1]},{"id":"right","geometry":[1,0,1,1]}]});
        let baseline = image(vec![[0.2; 3], [0.8; 3]], 2, 1);
        let candidate = image(vec![[0.; 3], [1.; 3]], 2, 1);
        let output = temp.path().join("output");
        let result = compare_reference(
            &baseline,
            &candidate,
            &case,
            &json!({"id":"protocol"}),
            &output,
            temp.path(),
        )
        .unwrap();
        assert_eq!(result["execution_status"], "completed");
        assert_eq!(result["measurements"].as_array().unwrap().len(), 3);
        assert!((result["measurements"][0]["value"].as_f64().unwrap() - 0.2).abs() < 1e-15);
        assert_eq!(result["measurements"][1]["value"], 0.);
        assert!(result["measurements"][2]["value"].as_f64().unwrap() < 0.);
        let artifact = result["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["role"] == "reference_material_curve")
            .unwrap();
        let curve: Value = serde_json::from_slice(
            &fs::read(output.join(artifact["path"].as_str().unwrap())).unwrap(),
        )
        .unwrap();
        assert_eq!(curve["region_pixel_counts"], json!([1, 1]));
        assert_eq!(curve["candidate_residual"], json!([0., 0.]));
    }
    #[test]
    fn regional_reference_has_independent_raster_population_size() {
        let temp = tempfile::tempdir().unwrap();
        let reference =
            json!({"id":"r","scope":"regional","regions":[{"id":"patch","geometry":[0,0,3,1]}]});
        let case = json!({"id":"c","regions":[{"id":"patch","geometry":[0,0,2,1]}]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), temp.path()).unwrap();
        regional(
            &mut c,
            &[[10., 0., 0.], [20., 0., 0.], [30., 0., 0.]],
            &[[10., 0., 0.], [20., 0., 0.], [30., 0., 0.]],
            [3, 1],
            &[vec![[20., 0., 0.]; 2], vec![[20., 0., 0.]; 2]],
            &[vec![[20., 0., 0.]; 2], vec![[20., 0., 0.]; 2]],
            [2, 1],
            &case,
            temp.path(),
            true,
            true,
        )
        .unwrap();
        assert_eq!(c.review["regions"][0]["render_sample_count"], 2);
        assert_eq!(c.review["regions"][0]["reference_sample_count"], 3);
        assert_eq!(c.body.measurements.len(), 18);
    }

    #[test]
    fn single_regional_distance_uses_simulation_without_baseline_fields() {
        let temp = tempfile::tempdir().unwrap();
        let reference =
            json!({"id":"r","scope":"regional","regions":[{"id":"patch","geometry":[0,0,3,1]}]});
        let case = json!({"id":"c","seeds":[7],"regions":[{"id":"patch","geometry":[0,0,2,1]}]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), temp.path()).unwrap();
        c.single = true;
        regional(
            &mut c,
            &[[10., 0., 0.], [20., 0., 0.], [30., 0., 0.]],
            &[[10., 0., 0.], [20., 0., 0.], [30., 0., 0.]],
            [3, 1],
            &[vec![[35., 0., 0.]; 2]],
            &[vec![[35., 0., 0.]; 2]],
            [2, 1],
            &case,
            temp.path(),
            true,
            true,
        )
        .unwrap();
        let out = c.finish();
        assert_eq!(out["measurements"].as_array().unwrap().len(), 6);
        assert!(out["measurements"].as_array().unwrap().iter().all(|m| {
            m["metric"]
                .as_str()
                .unwrap()
                .ends_with(".simulation_distance")
        }));
        assert!(out["measurements"][0]["value"].as_f64().unwrap() > 0.);
        let statistics = &out["reference_review"]["regions"][0];
        assert_eq!(statistics["lab_median"]["simulation"], json!([35., 0., 0.]));
        assert!(statistics["lab_median"].get("baseline").is_none());
        assert!(
            out["findings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f.get("distance_delta").is_none() && f.get("baseline_distance").is_none())
        );
    }

    #[test]
    fn single_processing_limits_never_emit_numeric_unavailable_distance() {
        let temp = tempfile::tempdir().unwrap();
        let reference =
            json!({"id":"r","scope":"regional","regions":[{"id":"patch","geometry":[0,0,1,1]}]});
        let case = json!({"id":"c","regions":[{"id":"patch","geometry":[0,0,1,1]}]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), temp.path()).unwrap();
        c.single = true;
        regional(
            &mut c,
            &[[20., 0., 0.]],
            &[[20., 0., 0.]],
            [1, 1],
            &[vec![[35., 0., 0.]]],
            &[vec![[35., 0., 0.]]],
            [1, 1],
            &case,
            temp.path(),
            false,
            false,
        )
        .unwrap();
        let out = c.finish();
        assert!(
            out["measurements"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["availability"] == "unavailable"
                    && m.get("value").is_none()
                    && m["reason"].is_string())
        );
    }
    #[test]
    fn single_visual_retains_native_review_without_numeric_score() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("reference.png");
        image_data::write_png16(&path, &[[0.2; 3]; 6], 3, 2).unwrap();
        let case = json!({"id":"c","seeds":[0],"regions":[{"id":"patch","geometry":[0,0,1,2]}],"reference":{"id":"r","path":path,"sha256":common::sha256_file(&path).unwrap(),"scope":"visual","color":{"color_space":"sRGB","cctf_decoding":true},"regions":[{"id":"patch","geometry":[2,0,1,2]}]}});
        let result = compare_reference_single(
            &image(vec![[0.8; 3]; 4], 2, 2),
            &case,
            &json!({"id":"p"}),
            &temp.path().join("out"),
            temp.path(),
        )
        .unwrap();
        assert_eq!(result["execution_status"], "completed");
        assert_eq!(
            result["reference_review"]["native_dimensions"],
            json!([3, 2])
        );
        assert!(result["reference_review"]["native_display_artifact_id"].is_string());
        assert_eq!(result["measurements"][0]["reason"], "visual_only_reference");
        assert!(result["measurements"][0].get("value").is_none());
        assert_eq!(
            result["reference_review"]["regions"][0]["reference_geometry"],
            json!([2, 0, 1, 2])
        );
        assert_eq!(result["reference_review"]["tone_target_eligible"], false);
        assert_eq!(
            result["reference_review"]["color_diagnostic_eligible"],
            false
        );
        assert!(
            result["reference_review"]["regions"][0]
                .get("diagnostics")
                .is_none()
        );
    }

    #[test]
    fn single_aligned_respects_mask_and_reports_actual_distance_and_flip_limits() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("reference.png");
        image_data::write_png16(&path, &[[0.2; 3]; 4], 2, 2).unwrap();
        common::dump(
            &temp.path().join("review.json"),
            &json!({"status":"approved","reviewer":"test","excluded_regions":[],"limitations":[]}),
        )
        .unwrap();
        let case = json!({"id":"c","seeds":[0],"regions":[{"id":"patch","geometry":[0,0,2,2],"measurements":["color"]}],"reference":{"id":"r","path":path,"scope":"aligned_scene","color":{"color_space":"sRGB","cctf_decoding":true},"processing":{"grading":"none","histogram_matching":"none","compression":"none","sharpening":"none","noise_reduction":"none","resampling":"none"},"aligned_derivative":{"path":path},"correspondence_review":{"path":"review.json"},"common_area_mask":[0,0,1,2]}});
        let result = compare_reference_single(
            &image(vec![[0.8; 3], [0.0; 3], [0.8; 3], [0.0; 3]], 2, 2),
            &case,
            &json!({"id":"p","pixels_per_degree":60,"hotspots":{"count":1,"min_area_px":1}}),
            &temp.path().join("out"),
            temp.path(),
        )
        .unwrap();
        assert_eq!(result["reference_review"]["geometry"], "pixel_aligned");
        let measurements = result["measurements"].as_array().unwrap();
        let distance = measurements
            .iter()
            .find(|m| m["metric"] == "reference.delta_e00.median.simulation_distance")
            .unwrap();
        assert_eq!(distance["sample_count"], 2);
        assert!(distance["value"].as_f64().unwrap() > 0.);
        let flip = measurements
            .iter()
            .find(|m| m["metric"] == "reference.flip.mean.simulation_distance")
            .unwrap();
        assert_eq!(flip["availability"], "unavailable");
        assert!(flip.get("value").is_none());
        assert!(
            result["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["role"] == "reference_common_area_mask")
        );
        assert!(
            measurements
                .iter()
                .all(|m| !m["metric"].as_str().unwrap().contains("baseline"))
        );
        for readout in result["region_readouts"].as_array().unwrap() {
            assert_eq!(readout["sides"]["simulation"]["sample_count"], 2);
            // Excluded black pixels must not dilute either center estimator.
            assert!(
                (readout["sides"]["simulation"]["lab"][0].as_f64().unwrap() - 82.04578).abs()
                    < 0.001
            );
            assert_eq!(
                readout["common_area_artifact_id"],
                result["reference_review"]["common_area_artifact_id"]
            );
        }
    }

    #[test]
    fn single_material_curve_retains_real_simulation_residual_only() {
        let temp = tempfile::tempdir().unwrap();
        let coordinate = json!({"name":"exposure","units":"stops"});
        let conditions = json!({"temperature_c":20});
        common::dump(&temp.path().join("material.json"),&json!({"schema_version":1,"domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions,"coordinate":coordinate,"curves":[{"id":"red","channel":"R","coordinates":[0.,1.],"values":[0.,1.]}]})).unwrap();
        let reference = json!({"id":"r","path":"material.json","scope":"material","domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions});
        let case = json!({"id":"c","seeds":[0],"reference":reference,"regions":[{"id":"left","geometry":[0,0,1,1]},{"id":"right","geometry":[1,0,1,1]}],"probe_response":{"domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions,"coordinate":coordinate,"estimator":"native_channel_mean","curves":[{"id":"red","channel":"R","samples":[{"coordinate":0.,"region_id":"left"},{"coordinate":1.,"region_id":"right"}]}]}});
        let output = temp.path().join("out");
        let result = compare_reference_single(
            &image(vec![[0.2; 3], [0.8; 3]], 2, 1),
            &case,
            &json!({"id":"p"}),
            &output,
            temp.path(),
        )
        .unwrap();
        assert!(result["measurements"][0]["value"].as_f64().unwrap() > 0.);
        let artifact = result["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["role"] == "reference_material_curve")
            .unwrap();
        let curve = documents::load_json_document(&output.join(artifact["path"].as_str().unwrap()))
            .unwrap();
        assert_eq!(curve["simulation"], json!([0.2, 0.8]));
        assert!(curve.get("baseline").is_none());
        assert!(curve.get("simulation_residual").is_some());
    }

    #[test]
    fn hue_diagnostics_respect_the_authored_chroma_floor() {
        let temp = tempfile::tempdir().unwrap();
        let reference = json!({"id":"r","scope":"regional","regions":[{"id":"low","geometry":[0,0,1,1]},{"id":"high","geometry":[1,0,1,1]}]});
        let case = json!({"id":"c","seeds":[0],"regions":[{"id":"low","label":"low","role":"color","geometry":[0,0,1,1],"measurements":[]},{"id":"high","label":"high","role":"color","geometry":[1,0,1,1],"measurements":[]}]});
        let mut c = Comparison::new(
            &reference,
            &case,
            &json!({"id":"p","color":{"hue_chroma_floor":2.}}),
            temp.path(),
        )
        .unwrap();
        c.single = true;
        regional(
            &mut c,
            &[[50., 0., 0.], [50., 10., 0.]],
            &[[50., 0., 0.], [50., 10., 0.]],
            [2, 1],
            &[vec![[50., 1., 0.], [50., 12., 2.]]],
            &[vec![[50., 1., 0.], [50., 12., 2.]]],
            [2, 1],
            &case,
            temp.path(),
            true,
            true,
        )
        .unwrap();
        let out = c.finish();
        let rows = out["measurements"].as_array().unwrap();
        let row = |region: &str, metric: &str| {
            rows.iter()
                .find(|m| m["metric"] == metric && m["region_id"] == region)
                .unwrap()
        };
        let low = row("low", "reference.patch_hue_shift.simulation_distance");
        assert_eq!(low["availability"], "unavailable");
        assert_eq!(low["reason"], "below_chroma_floor");
        assert!(low.get("value").is_none());
        let high = row("high", "reference.patch_hue_shift.simulation_distance");
        let expected = 2f64.atan2(12.).to_degrees();
        assert!(high["availability"] == "available");
        assert!((high["value"].as_f64().unwrap() - expected).abs() < 1e-9);
        let error = row("high", "reference.patch_hue_error.simulation_distance");
        assert!((error["value"].as_f64().unwrap() - expected).abs() < 1e-9);
        assert!(
            row(
                "low",
                "reference.patch_delta_e00_equal_l.simulation_distance"
            )["value"]
                .as_f64()
                .is_some()
        );
        let diagnostics = &out["reference_review"]["regions"][1]["diagnostics"];
        assert_eq!(
            diagnostics["reference.patch_hue_shift"]["hue_chroma_floor"],
            2.
        );
        assert_eq!(
            diagnostics["reference.patch_hue_shift"]["sides"]["simulation"]["chroma_ab"],
            12f64.hypot(2.)
        );
    }

    #[test]
    fn signed_hue_shift_wraps_to_the_shortest_direction() {
        let temp = tempfile::tempdir().unwrap();
        let reference = json!({"id":"r","scope":"regional","regions":[{"id":"ccw","geometry":[0,0,1,1]},{"id":"cw","geometry":[1,0,1,1]}]});
        let case = json!({"id":"c","seeds":[0],"regions":[{"id":"ccw","label":"ccw","role":"color","geometry":[0,0,1,1],"measurements":[]},{"id":"cw","label":"cw","role":"color","geometry":[1,0,1,1],"measurements":[]}]});
        let mut c = Comparison::new(
            &reference,
            &case,
            &json!({"id":"p","color":{"hue_chroma_floor":2.}}),
            temp.path(),
        )
        .unwrap();
        c.single = true;
        let at = |degrees: f64| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            [50., 10. * cos, 10. * sin]
        };
        // Reference 179.5°, model -179.5°: raw difference -359° must wrap to +1°.
        regional(
            &mut c,
            &[at(179.5), at(10.)],
            &[at(179.5), at(10.)],
            [2, 1],
            &[vec![at(-179.5), at(9.)]],
            &[vec![at(-179.5), at(9.)]],
            [2, 1],
            &case,
            temp.path(),
            true,
            true,
        )
        .unwrap();
        let out = c.finish();
        let rows = out["measurements"].as_array().unwrap();
        let value = |region: &str, metric: &str| {
            rows.iter()
                .find(|m| m["metric"] == metric && m["region_id"] == region)
                .unwrap()["value"]
                .as_f64()
                .unwrap()
        };
        let ccw = value("ccw", "reference.patch_hue_shift.simulation_distance");
        assert!((ccw - 1.).abs() < 1e-9, "{ccw}");
        let cw = value("cw", "reference.patch_hue_shift.simulation_distance");
        assert!((cw + 1.).abs() < 1e-9, "{cw}");
        assert!((value("cw", "reference.patch_hue_error.simulation_distance") - 1.).abs() < 1e-9);
    }

    #[test]
    fn equal_l_diagnostic_differs_from_full_de00_without_touching_samples() {
        let temp = tempfile::tempdir().unwrap();
        let reference = json!({"id":"r","scope":"regional","regions":[{"id":"flat","geometry":[0,0,1,1]},{"id":"chroma","geometry":[1,0,1,1]}]});
        let case = json!({"id":"c","seeds":[0],"regions":[{"id":"flat","label":"flat","role":"color","geometry":[0,0,1,1],"measurements":[]},{"id":"chroma","label":"chroma","role":"color","geometry":[1,0,1,1],"measurements":[]}]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), temp.path()).unwrap();
        c.single = true;
        regional(
            &mut c,
            &[[50., 5., 5.], [50., 5., 5.]],
            &[[50., 5., 5.], [50., 5., 5.]],
            [2, 1],
            &[vec![[70., 5., 5.], [70., 15., 5.]]],
            &[vec![[70., 5., 5.], [70., 15., 5.]]],
            [2, 1],
            &case,
            temp.path(),
            true,
            true,
        )
        .unwrap();
        let out = c.finish();
        let rows = out["measurements"].as_array().unwrap();
        let value = |region: &str, metric: &str| {
            rows.iter()
                .find(|m| m["metric"] == metric && m["region_id"] == region)
                .unwrap()["value"]
                .as_f64()
                .unwrap()
        };
        let full = value("flat", "reference.patch_delta_e00.simulation_distance");
        let equal_l = value(
            "flat",
            "reference.patch_delta_e00_equal_l.simulation_distance",
        );
        assert!(full > 1.);
        assert!(equal_l.abs() < 1e-12, "{equal_l}");
        let chroma_full = value("chroma", "reference.patch_delta_e00.simulation_distance");
        let chroma_equal = value(
            "chroma",
            "reference.patch_delta_e00_equal_l.simulation_distance",
        );
        assert!(chroma_equal > 1.);
        assert!((chroma_equal - chroma_full).abs() > 1.);
        // The equal-L substitution stays inside the metric; samples keep model L*.
        assert_eq!(
            out["reference_review"]["regions"][1]["lab_median"]["simulation"],
            json!([70., 15., 5.])
        );
    }

    #[test]
    fn pair_relation_convergence_does_not_imply_endpoint_accuracy() {
        let temp = tempfile::tempdir().unwrap();
        let reference = json!({"id":"r","scope":"regional","regions":[{"id":"l","geometry":[0,0,1,1]},{"id":"r2","geometry":[1,0,1,1]}]});
        let case = json!({"id":"c","seeds":[0],"regions":[{"id":"l","label":"l","role":"color","geometry":[0,0,1,1],"measurements":[]},{"id":"r2","label":"r2","role":"color","geometry":[1,0,1,1],"measurements":[]}],"region_pairs":[{"id":"lr","label":"l/r2","left":"l","right":"r2"}]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), temp.path()).unwrap();
        c.single = true;
        // The model swaps the two endpoint colors: the separation relation is
        // preserved exactly while both endpoints stay individually wrong.
        regional(
            &mut c,
            &[[50., 10., 5.], [50., -10., -5.]],
            &[[50., 10., 5.], [50., -10., -5.]],
            [2, 1],
            &[vec![[50., -10., -5.], [50., 10., 5.]]],
            &[vec![[50., -10., -5.], [50., 10., 5.]]],
            [2, 1],
            &case,
            temp.path(),
            true,
            true,
        )
        .unwrap();
        let out = c.finish();
        let rows = out["measurements"].as_array().unwrap();
        let value = |region: &str, metric: &str| {
            rows.iter()
                .find(|m| m["metric"] == metric && m["region_id"] == region)
                .unwrap()["value"]
                .as_f64()
                .unwrap()
        };
        let relation = value(
            "lr",
            "reference.pair_separation_delta_e00.reference_relation",
        );
        let separation = value(
            "lr",
            "reference.pair_separation_delta_e00.simulation_distance",
        );
        assert!(relation > 1.);
        assert!((separation - relation).abs() < 1e-9);
        assert!(value("lr", "reference.pair_separation_change.simulation_distance").abs() < 1e-9);
        assert!(
            value(
                "lr",
                "reference.pair_separation_departure.simulation_distance"
            )
            .abs()
                < 1e-9
        );
        assert!(value("l", "reference.patch_delta_e00.simulation_distance") > 1.);
        assert!(value("r2", "reference.patch_delta_e00.simulation_distance") > 1.);
    }

    #[test]
    fn region_pair_identities_are_strictly_validated() {
        let regions = json!([{"id":"a","label":"A","role":"color","geometry":[0,0,1,1],"measurements":[]},{"id":"b","label":"B","role":"color","geometry":[1,0,1,1],"measurements":[]}]);
        let mut case = json!({"id":"c","regions":regions});
        case["region_pairs"] = json!([{"id":"ab","label":"A/B","left":"a","right":"b"}]);
        assert!(validate_region_pairs(&case).is_ok());
        let code = |case: &mut Value, pairs: Value| {
            case["region_pairs"] = pairs;
            validate_region_pairs(case).unwrap_err().code
        };
        assert_eq!(
            code(
                &mut case,
                json!([{"id":"ab","label":"x","left":"a","right":"b"},{"id":"ab","label":"y","left":"b","right":"a"}])
            ),
            "duplicate_id"
        );
        assert_eq!(
            code(
                &mut case,
                json!([{"id":"ab","label":"x","left":"a","right":"zz"}])
            ),
            "unresolved_identifier"
        );
        assert_eq!(
            code(
                &mut case,
                json!([{"id":"ab","label":"x","left":"a","right":"a"}])
            ),
            "invalid_region_pairs"
        );
        assert_eq!(
            code(
                &mut case,
                json!([{"id":"a","label":"x","left":"a","right":"b"}])
            ),
            "invalid_region_pairs"
        );
        assert_eq!(
            code(
                &mut case,
                json!([{"id":"full_image","label":"x","left":"a","right":"b"}])
            ),
            "invalid_region_pairs"
        );
        assert_eq!(
            code(
                &mut case,
                json!([{"id":"a b","label":"x","left":"a","right":"b"}])
            ),
            "invalid_region_pairs"
        );
        case["reference"] =
            json!({"id":"r","scope":"regional","regions":[{"id":"a","geometry":[0,0,1,1]}]});
        assert_eq!(
            code(
                &mut case,
                json!([{"id":"ab","label":"x","left":"a","right":"b"}])
            ),
            "invalid_region_pairs"
        );
    }

    #[test]
    fn population_summaries_label_populations_and_retain_worst_regressions() {
        let temp = tempfile::tempdir().unwrap();
        let reference = json!({"id":"r","scope":"regional","regions":[{"id":"a","geometry":[0,0,1,1]},{"id":"b","geometry":[1,0,1,1]},{"id":"n","geometry":[2,0,1,1]}]});
        let case = json!({"id":"c","seeds":[0],"regions":[{"id":"a","label":"A","role":"color","geometry":[0,0,1,1],"measurements":[]},{"id":"b","label":"B","role":"color","geometry":[1,0,1,1],"measurements":[]},{"id":"n","label":"N","role":"neutral","geometry":[2,0,1,1],"measurements":[]}]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), temp.path()).unwrap();
        let reference_lab = [[50., 10., 5.], [50., -10., -5.], [50., 0., 0.]];
        regional(
            &mut c,
            &reference_lab,
            &reference_lab,
            [3, 1],
            &[
                reference_lab.to_vec(),
                vec![[40., 30., 25.], reference_lab[1], reference_lab[2]],
            ],
            &[
                reference_lab.to_vec(),
                vec![[40., 30., 25.], reference_lab[1], reference_lab[2]],
            ],
            [3, 1],
            &case,
            temp.path(),
            true,
            true,
        )
        .unwrap();
        let out = c.finish();
        let summaries = out["reference_review"]["population_summaries"]
            .as_array()
            .unwrap();
        let entry = |population: &str, metric: &str| {
            summaries
                .iter()
                .find(|s| s["population"] == population && s["metric"] == metric)
                .unwrap()
        };
        let color = entry("color", "reference.patch_delta_e00");
        assert_eq!(color["region_ids"], json!(["a", "b"]));
        assert_eq!(color["roles"], json!(["color"]));
        assert_eq!(color["per_capture"]["baseline"]["region_count"], 2);
        assert_eq!(color["per_capture"]["candidate"]["worst_region_id"], "a");
        assert_eq!(
            color["per_capture"]["distance_delta"]["regression_count"],
            1
        );
        assert_eq!(
            color["per_capture"]["distance_delta"]["worst_regression_region_id"],
            "a"
        );
        assert_eq!(
            entry("neutral", "reference.patch_delta_e00")["region_ids"],
            json!(["n"])
        );
        let hue = entry("color", "reference.patch_hue_error");
        assert_eq!(hue["per_capture"]["baseline"]["region_count"], 0);
        assert_eq!(hue["per_capture"]["baseline"]["unavailable_count"], 2);
        assert_eq!(
            hue["per_capture"]["baseline"]["unavailable_reason"],
            "hue_diagnostics_not_selected"
        );
    }

    #[test]
    fn hue_floor_selection_is_explicit_and_strict() {
        assert!(hue_floor(&json!({})).unwrap().is_none());
        assert!(hue_floor(&json!({"color":{"hue_chroma_floor":2.}})).unwrap() == Some(2.));
        for invalid in [
            json!({"color":{"hue_chroma_floor":0.}}),
            json!({"color":{"hue_chroma_floor":-1.}}),
            json!({"color":{"hue_chroma_floor":"2"}}),
        ] {
            assert_eq!(hue_floor(&invalid).unwrap_err().code, "invalid_protocol");
        }
        let temp = tempfile::tempdir().unwrap();
        let reference =
            json!({"id":"r","scope":"regional","regions":[{"id":"p","geometry":[0,0,1,1]}]});
        let case = json!({"id":"c","seeds":[0],"regions":[{"id":"p","label":"p","role":"color","geometry":[0,0,1,1],"measurements":[]}]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), temp.path()).unwrap();
        c.single = true;
        regional(
            &mut c,
            &[[50., 10., 5.]],
            &[[50., 10., 5.]],
            [1, 1],
            &[vec![[52., 11., 6.]]],
            &[vec![[52., 11., 6.]]],
            [1, 1],
            &case,
            temp.path(),
            true,
            true,
        )
        .unwrap();
        let out = c.finish();
        let rows = out["measurements"].as_array().unwrap();
        let row = rows
            .iter()
            .find(|m| m["metric"] == "reference.patch_hue_shift.simulation_distance")
            .unwrap();
        assert_eq!(row["reason"], "hue_diagnostics_not_selected");
        assert!(row.get("value").is_none());
    }
    /// A visual reference must never acquire coordinates through the new mean
    /// estimator: it reports the same reason the scalar distance reports.
    #[test]
    fn visual_reference_readouts_report_the_same_reason_without_coordinates() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("reference.png");
        image_data::write_png16(&path, &[[0.2; 3]; 6], 3, 2).unwrap();
        let case = json!({"id":"c","seeds":[0],"regions":[{"id":"patch","geometry":[0,0,1,2]}],"reference":{"id":"r","path":path,"sha256":common::sha256_file(&path).unwrap(),"scope":"visual","color":{"color_space":"sRGB","cctf_decoding":true},"regions":[{"id":"patch","geometry":[2,0,1,2]}]}});
        let result = compare_reference_single(
            &image(vec![[0.8; 3]; 4], 2, 2),
            &case,
            &json!({"id":"p"}),
            &temp.path().join("out"),
            temp.path(),
        )
        .unwrap();
        let readouts = result["region_readouts"].as_array().unwrap();
        assert_eq!(readouts.len(), 2);
        assert!(readouts.iter().all(|r| r["availability"] == "unavailable"
            && r["reason"] == "visual_only_reference"
            && r.get("sides").is_none()
            && r.get("displacements").is_none()));
    }
    #[test]
    fn regional_color_centers_preserve_processing_eligibility() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("reference.png");
        image_data::write_png16(&path, &[[0.2; 3]; 2], 2, 1).unwrap();
        let mut case = json!({"id":"c","seeds":[0],"regions":[{"id":"patch","geometry":[0,0,2,1],"measurements":["color"]}],
            "reference":{"id":"r","path":path,"scope":"regional","color":{"color_space":"sRGB","cctf_decoding":true},
            "regions":[{"id":"patch","geometry":[0,0,2,1]}],"processing":{"grading":"none","histogram_matching":"none"}}});
        let model = image(vec![[0.6; 3]; 2], 2, 1);
        for (history, tone, chroma) in [
            ("none", true, true),
            ("brightness_only", false, true),
            ("unknown", false, false),
        ] {
            case["reference"]["processing"]["histogram_matching"] = json!(history);
            let out = compare_reference_single(
                &model,
                &case,
                &json!({"id":"p","color":{"hue_chroma_floor":5.}}),
                &temp.path().join(history),
                temp.path(),
            )
            .unwrap();
            let readouts = out["region_readouts"].as_array().unwrap();
            assert_eq!(readouts.len(), 2);
            for readout in readouts {
                assert!(
                    (readout["sides"]["reference"]["lab"][0].as_f64().unwrap() - 21.24673).abs()
                        < 0.001
                );
                assert!(
                    (readout["sides"]["simulation"]["lab"][0].as_f64().unwrap() - 63.22259).abs()
                        < 0.001
                );
                let displacement = &readout["displacements"][0];
                assert_eq!(displacement["from_role"], "reference");
                assert_eq!(displacement["to_role"], "simulation");
                if tone {
                    assert!(
                        (displacement["delta_l_star"]["value"].as_f64().unwrap() - 41.97586).abs()
                            < 0.002
                    );
                } else {
                    for key in ["delta_lab", "delta_l_star", "delta_e76", "delta_e00"] {
                        assert_eq!(displacement[key]["reason"], "tone_contaminated_reference");
                        assert!(displacement[key].get("value").is_none());
                    }
                }
                if chroma {
                    assert!(displacement["equal_l_delta_e00"]["value"].as_f64().unwrap() < 0.01);
                    assert_eq!(
                        displacement["delta_h_degrees"]["reason"],
                        "below_chroma_floor"
                    );
                } else {
                    for key in ["delta_c_star", "equal_l_delta_e00", "delta_h_degrees"] {
                        assert_eq!(
                            displacement[key]["reason"],
                            "unknown_or_graded_color_reference"
                        );
                        assert!(displacement[key].get("value").is_none());
                    }
                }
            }
        }
    }
}
