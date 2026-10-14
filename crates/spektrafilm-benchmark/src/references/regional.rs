use super::DOMAIN;
use super::artifacts::{ArtifactRole, Comparison};
use super::io::{err, field, mask, mean, optional_array, percentile, population, text};
use crate::common::Result;
use crate::image_data;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

// Keep regional arrays and frozen comparison controls explicit at this boundary.
#[allow(clippy::too_many_arguments)]
pub fn regional(
    c: &mut Comparison,
    ref_lab: &[[f64; 3]],
    reference_dims: [usize; 2],
    labs: &[Vec<[f64; 3]>],
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
    c.limitation("Corresponding region statistics do not establish pixel correspondence or full-scene fidelity.");
    c.review["regions"] = json!([]);
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
        let render_pop = population(&mask(field(render, "geometry")?, dims, base)?);
        let ref_pop = population(&mask(field(region, "geometry")?, reference_dims, base)?);
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
        let mut payload = json!({"region_id":id,"render_geometry":render["geometry"],"reference_geometry":region["geometry"],"render_sample_count":render_pop.len(),"reference_sample_count":ref_pop.len(),"lab_median":{"reference":centers[reference_index]},"lightness_quantile_probabilities":(0..=100).map(|q|q as f64/100.).collect::<Vec<_>>(),"lightness_quantiles":{"reference":quantiles[reference_index]},"domain":DOMAIN,"units":"L*,a*,b*"});
        for s in 0..labs.len() {
            let role = if c.single {
                "simulation"
            } else if s == 0 {
                "baseline"
            } else {
                "candidate"
            };
            payload["lab_median"][role] = json!(centers[s]);
            payload["lightness_quantiles"][role] = json!(quantiles[s]);
        }
        let evidence = c.json_artifact(ArtifactRole::RegionalStatistics, &payload)?;
        c.review["regions"].as_array_mut().unwrap().push(payload);
        c.review_ref(&evidence);
        let refs = [evidence];
        let de: Vec<_> = (0..labs.len())
            .map(|s| image_data::delta_e_2000(centers[s], centers[reference_index]))
            .collect();
        let ld: Vec<_> = (0..labs.len())
            .map(|s| {
                mean(
                    &quantiles[s]
                        .iter()
                        .zip(&quantiles[reference_index])
                        .map(|(a, b)| (a - b).abs())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let ch: Vec<_> = (0..labs.len())
            .map(|s| {
                ((centers[s][1] - centers[reference_index][1]).powi(2)
                    + (centers[s][2] - centers[reference_index][2]).powi(2))
                .sqrt()
            })
            .collect();
        c.distances(
            id,
            "reference.patch_delta_e00",
            DOMAIN,
            "delta E 00",
            "DE00 between independent Lab channel medians; robust interior patch",
            render_pop.len(),
            tone.then_some(de),
            (!tone).then_some("tone_contaminated_reference"),
            &refs,
        );
        c.distances(id,"reference.distribution_l_star",DOMAIN,"L*","mean absolute distance of 101 independent marginal lightness quantiles; linear quantiles",render_pop.len(),tone.then_some(ld),(!tone).then_some("tone_contaminated_reference"),&refs);
        c.distances(
            id,
            "reference.patch_chroma_ab",
            DOMAIN,
            "Lab a*/b* distance",
            "distance between independent median a*/b*; excludes lightness; diagnostic only",
            render_pop.len(),
            chroma.then_some(ch),
            (!chroma).then_some("unknown_or_graded_color_reference"),
            &refs,
        );
    }
    Ok(())
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
