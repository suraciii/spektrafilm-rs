use super::DOMAIN;
use super::artifacts::{ArtifactRole, Comparison};
use super::io::{
    clipped_lab, clipping_fraction, err, field, finite_rgb, mask, mean, optional_array, percentile,
    population, read_json, read_reference, text, truthy,
};
use super::transform::FrozenTransform;
use crate::common::Result;
use crate::image_data::{self, ImageData};
use serde_json::{Value, json};
use std::path::Path;

// Keep source/reference arrays and frozen comparison controls explicit at this boundary.
#[allow(clippy::too_many_arguments)]
pub fn aligned(
    c: &mut Comparison,
    source: &ImageData,
    encoded: &[[f64; 3]],
    labs: &[Vec<[f64; 3]>],
    rendered: &[Vec<[f64; 3]>],
    dims: [usize; 2],
    case: &Value,
    base: &Path,
    tone: bool,
    chroma: bool,
) -> Result<()> {
    let reference = c.reference;
    let has_transform = reference.get("transform").is_some_and(truthy);
    let has_derivative = reference.get("aligned_derivative").is_some_and(truthy);
    if has_transform == has_derivative {
        return Err(err(
            "invalid_correspondence",
            "Provide exactly one frozen transform or aligned derivative",
        ));
    }
    if !reference.get("correspondence_review").is_some_and(truthy)
        || !reference.get("common_area_mask").is_some_and(truthy)
    {
        return Err(err(
            "invalid_correspondence",
            "Pixel comparison requires reviewed correspondence and common-area mask",
        ));
    }
    let record = read_json(&reference["correspondence_review"], base)?;
    if record.get("status").and_then(Value::as_str) != Some("approved")
        || record
            .get("reviewer")
            .and_then(Value::as_str)
            .is_none_or(|x| x.trim().is_empty())
    {
        return Err(err(
            "invalid_correspondence",
            "Correspondence must be approved by an identified reviewer",
        ));
    }
    let excluded = record
        .get("excluded_regions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            err(
                "invalid_correspondence",
                "Review must record exclusions and limitations explicitly",
            )
        })?;
    let limitations = record
        .get("limitations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            err(
                "invalid_correspondence",
                "Review must record exclusions and limitations explicitly",
            )
        })?;
    let (encoded, support) = if has_derivative {
        let mut declaration = reference.clone();
        declaration["path"] = field(&reference["aligned_derivative"], "path")?.clone();
        let aligned = read_reference(&declaration, base)?;
        if aligned.dimensions() != dims {
            return Err(err(
                "invalid_correspondence",
                "Aligned derivative has incorrect geometry",
            ));
        }
        let rgb = image_data::display_rgb(&aligned)?;
        c.review["aligned_display_clipped_sample_fraction"] = json!(clipping_fraction(&rgb));
        (rgb, vec![true; dims[0] * dims[1]])
    } else {
        FrozenTransform::parse(&read_json(&reference["transform"], base)?)?.warp(
            encoded,
            source.dimensions(),
            dims,
        )?
    };
    finite_rgb(
        &encoded,
        "computation_failure",
        "Aligned reference has nonfinite samples",
    )?;
    let valid = mask(&reference["common_area_mask"], dims, base)?;
    if valid.iter().zip(&support).any(|(v, s)| *v && !*s) {
        return Err(err(
            "invalid_correspondence",
            "Common-area mask includes unsupported transform borders",
        ));
    }
    for exclusion in excluded {
        if !["motion", "occlusion", "parallax", "invalid_border"].contains(
            &exclusion
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ) {
            return Err(err(
                "invalid_correspondence",
                "Unsupported correspondence exclusion reason",
            ));
        }
        let excluded = mask(field(exclusion, "geometry")?, dims, base)?;
        if valid.iter().zip(excluded).any(|(v, e)| *v && e) {
            return Err(err(
                "invalid_correspondence",
                "Common-area mask retains reviewed invalid correspondence",
            ));
        }
    }
    if !valid.iter().any(|v| *v) {
        return Err(err(
            "invalid_correspondence",
            "Common-area mask has no eligible pixels",
        ));
    }
    let ref_lab = clipped_lab(&encoded);
    let display_id = c.rgb_artifact(
        ArtifactRole::AlignedDisplay,
        &encoded,
        dims,
        "bounded_encoded_srgb",
        true,
    )?;
    c.review["display_artifact_id"] = json!(display_id);
    c.review_ref(&display_id);
    let reference_id = c.rgb_artifact(
        ArtifactRole::Aligned,
        &encoded,
        dims,
        "unclipped_encoded_srgb",
        false,
    )?;
    let valid_id = c.mask_artifact(&valid, dims)?;
    c.review["geometry"] = json!("pixel_aligned");
    c.review["aligned_artifact_id"] = json!(reference_id);
    c.review["common_area_artifact_id"] = json!(valid_id);
    c.review["correspondence_review"] = record.clone();
    c.review_ref(&reference_id);
    c.review_ref(&valid_id);
    c.review["limitations"]
        .as_array_mut()
        .unwrap()
        .extend(limitations.iter().cloned());
    let maps: Vec<(Vec<f64>, Vec<f64>)> = labs
        .iter()
        .map(|lab| {
            (
                lab.iter()
                    .zip(&ref_lab)
                    .map(|(a, b)| image_data::delta_e_2000(*a, *b))
                    .collect(),
                lab.iter()
                    .zip(&ref_lab)
                    .map(|(a, b)| ((a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt())
                    .collect(),
            )
        })
        .collect();
    if maps
        .iter()
        .flat_map(|(a, b)| a.iter().chain(b))
        .any(|x| !x.is_finite())
    {
        return Err(err(
            "computation_failure",
            "Reference distance maps contain nonfinite samples",
        ));
    }
    let mut refs = vec![reference_id, valid_id];
    for s in 0..labs.len() {
        let role = if c.single || s == 1 {
            ArtifactRole::CandidateMaps
        } else {
            ArtifactRole::BaselineMaps
        };
        let mut v = Vec::with_capacity(dims[0] * dims[1] * 4);
        for (i, lab) in labs[s].iter().enumerate() {
            v.extend_from_slice(&[maps[s].0[i], maps[s].1[i], lab[0], ref_lab[i][0]]);
        }
        refs.push(c.numeric_artifact(
            role,
            &v,
            &[dims[1], dims[0], 4],
            DOMAIN,
            Some("DE00; Lab a*/b* distance; rendered L*; reference L*"),
        )?);
    }
    if c.single {
        let (id, path) = c.next_artifact(ArtifactRole::CandidateMaps, "npy");
        image_data::write_npy(&path, &maps[0].0, &[dims[1], dims[0]])?;
        let mut row = c.artifact_row(&id, &path, "difference_map", "application/x-npy")?;
        row["metric"] = json!("delta_e00");
        row["dimensions"] = json!(dims);
        row["valid_mask_artifact_id"] = json!(refs[1]);
        row["availability"] = json!(if tone { "available" } else { "unavailable" });
        if !tone {
            row["reason"] = json!("tone_contaminated_reference");
        }
        c.body.artifacts.push(row);
        c.review["difference_artifact_id"] = json!(id);
        c.review_ref(&id);
        if tone {
            let ranked: Vec<_> = maps[0]
                .0
                .iter()
                .zip(&valid)
                .map(|(value, valid)| if *valid { *value } else { 0. })
                .collect();
            let mut hotspots = crate::metrics::select_hotspots(
                &ranked,
                dims[0],
                dims[1],
                &c.protocol["hotspots"],
                "reference.delta_e00",
                std::slice::from_ref(&id),
            );
            hotspots.retain(|region| {
                let rectangle: Vec<_> = region["rectangle"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as usize)
                    .collect();
                (rectangle[1]..rectangle[1] + rectangle[3]).all(|y| {
                    (rectangle[0]..rectangle[0] + rectangle[2]).all(|x| valid[y * dims[0] + x])
                })
            });
            c.review["automatic_regions"] = json!(hotspots);
        }
        let clean = ["none", "unmodified", "not_applied"];
        let flip_allowed = tone
            && ["compression", "sharpening", "noise_reduction", "resampling"]
                .iter()
                .all(|key| {
                    reference["processing"][key]
                        .as_str()
                        .is_some_and(|v| clean.contains(&v.trim().to_lowercase().as_str()))
                })
            && valid.iter().all(|v| *v);
        let reason = if flip_allowed {
            None
        } else {
            Some("reference_processing_or_filter_support_unavailable")
        };
        let mut flip_refs = refs.clone();
        let flip = if flip_allowed {
            let map = image_data::official_flip(
                &encoded,
                &rendered[0],
                dims[0],
                dims[1],
                c.protocol["pixels_per_degree"].as_f64().unwrap_or(60.),
            )?;
            let (id, path) = c.next_artifact(ArtifactRole::CandidateMaps, "npy");
            image_data::write_npy(&path, &map, &[dims[1], dims[0]])?;
            let mut row = c.artifact_row(&id, &path, "difference_map", "application/x-npy")?;
            row["metric"] = json!("flip");
            row["dimensions"] = json!(dims);
            c.body.artifacts.push(row);
            flip_refs.push(id);
            Some(map)
        } else {
            None
        };
        for (name, q) in [("mean", None), ("p95", Some(0.95)), ("max", Some(1.))] {
            let value = flip
                .as_ref()
                .map(|map| vec![q.map_or_else(|| mean(map), |q| percentile(map.clone(), q))]);
            c.distances(
                "full_image",
                &format!("reference.flip.{name}"),
                "bounded_encoded_srgb",
                "FLIP error",
                "official LDR-FLIP; complete valid filter support",
                if flip_allowed { valid.len() } else { 0 },
                value,
                reason,
                &flip_refs,
            );
        }
    }
    let full = json!({"id":"full_image"});
    for region in std::iter::once(&full).chain(optional_array(case, "regions")) {
        let id = text(region, "id")?;
        let region_mask = if id == "full_image" {
            valid.clone()
        } else {
            valid
                .iter()
                .zip(mask(field(region, "geometry")?, dims, base)?)
                .map(|(a, b)| *a && b)
                .collect()
        };
        let pop = population(&region_mask);
        let count = pop.len();
        let missing = (count == 0).then_some("empty_reference_population");
        for (name, q) in [("median", 0.5), ("p95", 0.95), ("p99", 0.99), ("max", 1.)] {
            let values = (count > 0 && tone).then(|| {
                maps.iter()
                    .map(|map| percentile(pop.iter().map(|i| map.0[*i]).collect(), q))
                    .collect()
            });
            c.distances(
                id,
                &format!("reference.delta_e00.{name}"),
                DOMAIN,
                "delta E 00",
                "pixel quantile method=linear; common-area mask",
                count,
                values,
                missing.or((!tone).then_some("tone_contaminated_reference")),
                &refs,
            );
        }
        for q in [5, 25, 50, 75, 95] {
            let values = (count > 0 && tone).then(|| {
                let ref_q = percentile(
                    pop.iter().map(|i| ref_lab[*i][0]).collect(),
                    q as f64 / 100.,
                );
                labs.iter()
                    .map(|lab| {
                        (percentile(pop.iter().map(|i| lab[*i][0]).collect(), q as f64 / 100.)
                            - ref_q)
                            .abs()
                    })
                    .collect()
            });
            c.distances(
                id,
                &format!("reference.tone_l_star.q{q:02}"),
                DOMAIN,
                "L*",
                "absolute difference of fixed-population lightness quantiles",
                count,
                values,
                missing.or((!tone).then_some("tone_contaminated_reference")),
                &refs,
            );
        }
        let values = (count > 0 && chroma).then(|| {
            maps.iter()
                .map(|map| percentile(pop.iter().map(|i| map.1[*i]).collect(), 0.5))
                .collect()
        });
        c.distances(
            id,
            "reference.chroma_ab.median",
            DOMAIN,
            "Lab a*/b* distance",
            "median pixel chromatic distance; excludes lightness; diagnostic only",
            count,
            values,
            missing.or((!chroma).then_some("unknown_or_graded_color_reference")),
            &refs,
        );
    }
    Ok(())
}
