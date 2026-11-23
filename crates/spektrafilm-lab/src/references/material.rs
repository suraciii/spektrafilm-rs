use super::DOMAIN;
use super::artifacts::{ArtifactRole, Comparison};
use super::io::{
    clipped_lab, err, field, finite_rgb, mask, optional_array, population, read_json, text, truthy,
};
use super::regional::{ProbeDomain, ProbeEstimator};
use crate::common::Result;
use crate::image_data::{self, ImageData};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

pub fn unique_curves(curves: &[Value]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for curve in curves {
        if !ids.insert(text(curve, "id")?) {
            return Err(err(
                "invalid_material_measurement",
                "Curve identities must be unique",
            ));
        }
    }
    Ok(())
}
pub fn material_arrays(
    c: &mut Comparison,
    estimator: &ProbeEstimator,
    images: &[&ImageData],
) -> Result<Vec<Vec<[f64; 3]>>> {
    let reference = c.reference;
    let domain = text(reference, "domain")?;
    let units = text(reference, "units")?;
    match estimator.domain {
        ProbeDomain::Native => {
            if domain == "native_rgb" && units == "native channel value" {
                if images.iter().any(|image| {
                    (image.color_space.as_str(), image.transfer.as_str())
                        != (images[0].color_space.as_str(), images[0].transfer.as_str())
                }) {
                    return Err(err(
                        "incompatible_material_conditions",
                        "Native RGB probe declarations differ",
                    ));
                }
                c.limitation("Native RGB response is a rendered diagnostic, not density, spectral response, or physical sensitometry.");
            } else {
                for image in images {
                    let declaration = image.metadata.get("material_measurement");
                    if ["domain", "units", "instrument", "conditions"]
                        .iter()
                        .any(|f| declaration.and_then(|d| d.get(*f)) != reference.get(*f))
                    {
                        return Err(err(
                            "incompatible_material_conditions",
                            "Native probe lacks matching measurement-domain/instrument declaration; RGB is not a density proxy",
                        ));
                    }
                }
            }
            Ok(images.iter().map(|image| image.samples.clone()).collect())
        }
        ProbeDomain::Lab => {
            if domain != DOMAIN || !["L*", "a*", "b*"].contains(&units) {
                return Err(err(
                    "incompatible_material_conditions",
                    "Lab response requires declared D65 Lab domain and channel units",
                ));
            }
            images
                .iter()
                .map(|image| Ok(clipped_lab(&image_data::display_rgb(image)?)))
                .collect()
        }
        ProbeDomain::Luminance => {
            if domain != "linear_srgb_relative_luminance" || units != "relative luminance" {
                return Err(err(
                    "incompatible_material_conditions",
                    "Luminance response must declare relative linear sRGB luminance",
                ));
            }
            let convert = |image: &ImageData| -> Result<Vec<[f64; 3]>> {
                Ok(image_data::display_rgb(image)?
                    .iter()
                    .map(|p| {
                        [
                            image_data::srgb_decode(p[0]) * 0.2126
                                + image_data::srgb_decode(p[1]) * 0.7152
                                + image_data::srgb_decode(p[2]) * 0.0722,
                            0.,
                            0.,
                        ]
                    })
                    .collect())
            };
            images.iter().map(|image| convert(image)).collect()
        }
    }
}
pub fn numeric_curve(v: Option<&Value>) -> Option<Vec<f64>> {
    v?.as_array()?.iter().map(Value::as_f64).collect()
}
pub fn material(
    c: &mut Comparison,
    images: &[&ImageData],
    case: &Value,
    base: &Path,
) -> Result<()> {
    let reference = c.reference;
    let source = read_json(field(reference, "path")?, base)?;
    let source_id = c.json_artifact(ArtifactRole::MaterialSource, &source)?;
    c.review_ref(&source_id);
    c.review["master_artifact_id"] = json!(source_id);
    let response = case
        .get("probe_response")
        .filter(|v| v.is_object())
        .ok_or_else(|| {
            err(
                "missing_probe_response",
                "Material evidence requires a declared matching model probe",
            )
        })?;
    if source.get("schema_version") != Some(&json!(1)) {
        return Err(err(
            "invalid_material_measurement",
            "Material measurement must use schema_version 1",
        ));
    }
    for f in ["domain", "units", "instrument", "conditions"] {
        let value = reference.get(f).filter(|v| truthy(v)).ok_or_else(|| {
            err(
                "incompatible_material_conditions",
                format!("Source and model response must agree on {f}"),
            )
        })?;
        if source.get(f) != Some(value) || response.get(f) != Some(value) {
            return Err(err(
                "incompatible_material_conditions",
                format!("Source and model response must agree on {f}"),
            ));
        }
        if value
            .as_str()
            .is_some_and(|s| s.to_lowercase() == "unknown")
        {
            return Err(err(
                "incompatible_material_conditions",
                "Material measurement conditions must be known",
            ));
        }
    }
    let coordinate = source
        .get("coordinate")
        .filter(|v| v.is_object())
        .ok_or_else(|| {
            err(
                "incompatible_material_coordinates",
                "Material coordinate identity and units must match",
            )
        })?;
    if !coordinate.get("name").is_some_and(truthy)
        || !coordinate.get("units").is_some_and(truthy)
        || response.get("coordinate") != Some(coordinate)
    {
        return Err(err(
            "incompatible_material_coordinates",
            "Material coordinate identity and units must match",
        ));
    }
    let source_curves = optional_array(&source, "curves");
    let response_curves = optional_array(response, "curves");
    if source_curves.is_empty() || response_curves.is_empty() {
        return Err(err(
            "missing_material_curves",
            "Source and model response require curves",
        ));
    }
    unique_curves(source_curves)?;
    unique_curves(response_curves)?;
    for image in images {
        if image.dimensions() != images[0].dimensions() {
            return Err(err("dimension_mismatch", "Probe dimensions differ"));
        }
        finite_rgb(
            &image.samples,
            "nonfinite_samples",
            "Material probe contains nonfinite samples",
        )?;
    }
    let estimator = ProbeEstimator::parse(
        response
            .get("estimator")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    let arrays = material_arrays(c, &estimator, images)?;
    for a in &arrays {
        finite_rgb(
            a,
            "computation_failure",
            "Material domain conversion is nonfinite",
        )?;
    }
    for f in ["domain", "units", "instrument", "conditions"] {
        c.review[f] = reference[f].clone();
    }
    c.review["geometry"] = json!("measurement_curves");
    c.review["coordinate"] = coordinate.clone();
    c.limitation("This objective compares declared model probe response to measured curves under matching conditions.");
    for curve in response_curves {
        let identity = text(curve, "id")?;
        if identity.is_empty()
            || !identity
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(err(
                "invalid_material_measurement",
                "Curve identity must be a stable metric token",
            ));
        }
        let expected = source_curves
            .iter()
            .find(|v| v.get("id").and_then(Value::as_str) == Some(identity))
            .ok_or_else(|| {
                err(
                    "incompatible_material_curves",
                    "Source/model curve identity and channel must match",
                )
            })?;
        let channel = curve
            .get("channel")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                err(
                    "incompatible_material_curves",
                    "Source/model curve identity and channel must match",
                )
            })?;
        let ch = estimator.channel(channel).ok_or_else(|| {
            err(
                "incompatible_material_curves",
                "Source/model curve identity and channel must match",
            )
        })?;
        if expected.get("channel") != curve.get("channel") {
            return Err(err(
                "incompatible_material_curves",
                "Source/model curve identity and channel must match",
            ));
        }
        if matches!(estimator.domain, ProbeDomain::Lab)
            && format!("{channel}*") != text(reference, "units")?
        {
            return Err(err(
                "incompatible_material_conditions",
                "Lab response channel does not match declared units",
            ));
        }
        let coordinates = numeric_curve(expected.get("coordinates")).ok_or_else(|| {
            err(
                "invalid_material_measurement",
                "Curve requires finite, strictly increasing coordinates and matching values",
            )
        })?;
        let values = numeric_curve(expected.get("values")).ok_or_else(|| {
            err(
                "invalid_material_measurement",
                "Curve requires finite, strictly increasing coordinates and matching values",
            )
        })?;
        if coordinates.len() < 2
            || coordinates.len() != values.len()
            || coordinates.iter().chain(&values).any(|x| !x.is_finite())
            || coordinates.windows(2).any(|w| w[1] <= w[0])
        {
            return Err(err(
                "invalid_material_measurement",
                "Curve requires finite, strictly increasing coordinates and matching values",
            ));
        }
        let samples = optional_array(curve, "samples");
        let declared: Option<Vec<f64>> = samples
            .iter()
            .map(|s| s.get("coordinate").and_then(Value::as_f64))
            .collect();
        if declared.as_ref() != Some(&coordinates) {
            return Err(err(
                "incompatible_material_coordinates",
                "Probe coordinates must exactly match frozen source coordinates",
            ));
        }
        let mut measured = vec![Vec::new(); images.len()];
        let mut pixel_counts = Vec::new();
        let mut regions = Vec::new();
        for sample in samples {
            let region_id = text(sample, "region_id")?;
            let pop = if region_id == "full_image" {
                (0..images[0].samples.len()).collect()
            } else {
                let region = optional_array(case, "regions")
                    .iter()
                    .rev()
                    .find(|r| r.get("id").and_then(Value::as_str) == Some(region_id))
                    .ok_or_else(|| {
                        err(
                            "invalid_probe_response",
                            "Probe sample names a missing region",
                        )
                    })?;
                population(&mask(
                    field(region, "geometry")?,
                    images[0].dimensions(),
                    base,
                )?)
            };
            if pop.is_empty() {
                return Err(err(
                    "empty_reference_population",
                    "Probe region contains no samples",
                ));
            }
            pixel_counts.push(pop.len());
            regions.push(region_id);
            c.readouts(crate::color_readouts::unavailable_pair(
                region_id,
                "reference_scope_not_color_comparable",
            ));
            for s in 0..images.len() {
                measured[s].push(estimator.reduce(pop.iter().map(|i| arrays[s][*i][ch]).collect()));
            }
        }
        let residual: Vec<Vec<f64>> = measured
            .iter()
            .map(|samples| samples.iter().zip(&values).map(|(a, b)| a - b).collect())
            .collect();
        if measured
            .iter()
            .chain(&residual)
            .flatten()
            .any(|x| !x.is_finite())
        {
            return Err(err(
                "computation_failure",
                "Material response or residual estimation is nonfinite",
            ));
        }
        let mut payload = json!({"curve_id":identity,"coordinates":coordinates,"coordinate":coordinate,"reference":values,"domain":reference["domain"],"units":reference["units"],"instrument":reference["instrument"],"conditions":reference["conditions"],"estimator":estimator.name,"region_pixel_counts":pixel_counts,"regions":regions});
        for s in 0..images.len() {
            let role = if c.single {
                "simulation"
            } else if s == 0 {
                "baseline"
            } else {
                "candidate"
            };
            payload[role] = json!(measured[s]);
            payload[format!("{role}_residual")] = json!(residual[s]);
        }
        let evidence = c.json_artifact(ArtifactRole::MaterialCurve, &payload)?;
        c.review_ref(&evidence);
        let rmse: Vec<_> = residual
            .iter()
            .map(|samples| {
                (samples.iter().map(|x| x * x).sum::<f64>() / values.len() as f64).sqrt()
            })
            .collect();
        c.distances(
            "full_image",
            &format!("reference.material.{identity}"),
            text(reference, "domain")?,
            text(reference, "units")?,
            &format!(
                "RMSE over matched source coordinates; probe {}",
                estimator.name
            ),
            coordinates.len(),
            Some(rmse.into_iter().map(Some).collect()),
            None,
            &[evidence],
        );
    }
    Ok(())
}
