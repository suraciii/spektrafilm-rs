//! Native stage channel statistics for declared output regions.
//!
//! The renderer's opt-in stage bundle observes the boundary values that
//! actually flowed through the executed route. This producer turns those
//! retained f64 buffers into per-region channel means, minima and maxima in
//! the producer's own domain: record exposure in log10, record density in
//! optical density, and the two RGB boundaries in their declared relative
//! units. Nothing here converts to a display colour space, permutes the CMY
//! record axis, or relabels a boundary: every record carries the domain,
//! units, channel order and host precision the producer declared.
//!
//! Geometry is never guessed. A declared region is authored against the
//! delivered output raster, so it is only summarised on a stage whose grid
//! the retained evidence proves identical: the output boundary by its own
//! layout, the working grid through the retained source-alignment flag and
//! the working/published dimensions, and the source grid additionally
//! through an explicit crop-free, unit-scale, preview-free executed route.
//! Film-frame and grain-aperture grids have no declared ROI transform and
//! stay unmapped even when their dimensions happen to match.
//!
//! Each eligible buffer is decoded once per stage and every declared region
//! is summarised from the reused region population indices, so no full image
//! is materialised per region or per stage. A stage the producer claims
//! captured but whose buffer is missing, truncated or non-finite is a
//! computation failure, never a silent unavailable fallback.

use crate::common::{Error, Result, text};
use crate::documents;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

/// Stable estimator identity: an arithmetic mean of the native channel values
/// in the producer's own domain, never a colour-space conversion.
const ESTIMATOR: &str = "native_channel_arithmetic_mean";
/// Every captured stage buffer is stored as little-endian f64 samples.
const STORAGE_DTYPE: &str = "f64";
/// Declared regions are authored against the delivered output raster.
const REGION_COORDINATE_SPACE: &str = "output_pixels";
/// No declared ROI transform maps the output region onto this stage grid.
const UNMAPPED: &str = "stage_region_geometry_unmapped";
/// Bytes per little-endian f64 sample.
const SAMPLE_BYTES: usize = 8;
/// Samples per decode block. A multiple of one sample keeps every block read
/// aligned, so no partial word is carried across reads.
const BLOCK_SAMPLES: usize = 8192;

/// One flat record per run role, declared region and producer stage
/// boundary, in that nesting order.
pub(crate) fn measure(root: &Path, run_evidence: &Value, regions: &[Value]) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    for (role, payload) in run_evidence.as_object().into_iter().flatten() {
        records.extend(measure_role(root, role, payload, regions)?);
    }
    Ok(records)
}

/// Arithmetic mean, minimum and maximum of one native channel over one
/// region population.
#[derive(Clone, Copy)]
struct Stats {
    mean: f64,
    min: f64,
    max: f64,
}

/// One declared region reduced to its population on the delivered output
/// grid, reused across every stage whose grid it provably shares.
struct Population {
    region_id: String,
    /// The declared region geometry, echoed as the region ROI.
    geometry: Value,
    /// Selected pixel indices in row-major output-pixel order.
    indices: Vec<usize>,
}

/// Producer-confirmed facts about one captured stage boundary.
struct Producer {
    domain: String,
    units: String,
    channel_order: String,
    /// The producer's own geometry record.
    geometry: Value,
    host_precision: String,
    width: usize,
    height: usize,
}

/// One stage boundary's plan for the whole case: its own availability, the
/// producer facts when the boundary captured, and the per-region statistics
/// when the declared output ROI provably maps onto its grid.
struct StagePlan {
    stage_id: String,
    availability: String,
    /// Stable producer or producer-absent reason; `None` exactly when the
    /// boundary captured and its statistics were computed.
    reason: Option<String>,
    /// Present whenever the boundary itself captured, so a consumer can see
    /// which grid and domain a refused mapping came from.
    producer: Option<Producer>,
    /// One entry per declared region, in the declared region order.
    statistics: Option<Vec<[Stats; 3]>>,
    /// Retained buffer reference of the decoded stage, when decoded.
    artifact_refs: Vec<String>,
}

/// Which intermediate grids the retained evidence proves share the delivered
/// output grid. Both start unmapped: identity must be proven, never assumed.
#[derive(Clone, Copy)]
struct Identity {
    working: bool,
    source: bool,
}
impl Identity {
    /// Whether the declared output ROI may be applied to a stage grid of the
    /// named coordinate space. Dimension equality alone never qualifies.
    fn maps(&self, space: &str) -> bool {
        match space {
            "output_pixels" => true,
            "working_pixels" => self.working,
            "source_pixels" => self.source,
            // film_frame_pixels and grain_aperture_pixels have no declared
            // ROI transform, whatever their dimensions.
            _ => false,
        }
    }
}

fn measure_role(root: &Path, role: &str, payload: &Value, regions: &[Value]) -> Result<Vec<Value>> {
    // No declared region means no record at all: skip the stage list, the
    // output dimensions and every buffer decode.
    if regions.is_empty() {
        return Ok(Vec::new());
    }
    let observed = payload.get("observed").ok_or_else(|| {
        Error::new(
            "stage_evidence",
            "run evidence must name its observed sections",
        )
    })?;
    let stages = observed["stages"].as_array().ok_or_else(|| {
        Error::new(
            "stage_evidence",
            "observed stages must be an array of records",
        )
    })?;
    if stages.len() != documents::STAGE_IDS.len()
        || stages
            .iter()
            .enumerate()
            .any(|(index, stage)| stage["id"].as_str() != Some(documents::STAGE_IDS[index]))
    {
        return Err(Error::new(
            "stage_evidence",
            "observed stages must name every stage boundary exactly once in fixed order",
        ));
    }
    let input = &payload["input"];
    let output = output_dimensions(stages, input)?;
    let populations = populations(regions, output)?;
    let identity = identity_evidence(input, output);
    // Host precision is one run-level fact, not a per-stage one: an expanded
    // f32 run stays labelled f32 even though every buffer is stored as f64.
    // The run retains it from the executed diagnostics; the bundle context
    // carries the same fact when a capture was requested.
    let host_precision = payload["precision"]["host_precision"]
        .as_str()
        .or_else(|| observed["host_precision"].as_str());
    let plans = stage_plans(
        root,
        stages,
        &payload["stage_evidence"]["buffers"],
        &populations,
        identity,
        output,
        host_precision,
    )?;
    Ok(records(role, &populations, &plans))
}

/// The delivered output grid: the observed rgb_out layout, which the producer
/// proves equal to the published pipeline output, or the published dimensions
/// when the output boundary was never captured.
fn output_dimensions(stages: &[Value], input: &Value) -> Result<[usize; 2]> {
    for stage in stages {
        if stage["id"].as_str() == Some("rgb_out") && stage["availability"] == json!("available") {
            let layout = &stage["layout"];
            return Ok([positive(layout, "width")?, positive(layout, "height")?]);
        }
    }
    pair(&input["simulation_geometry"]["published_dimensions"]).ok_or_else(|| {
        Error::new(
            "stage_geometry",
            "delivered output dimensions are unobserved; no stage grid can be mapped",
        )
    })
}

/// Reduce every declared region to its selected pixel indices on the output
/// grid, reusing the region's declared mask exactly as the appearance
/// measurements do.
fn populations(regions: &[Value], output: [usize; 2]) -> Result<Vec<Population>> {
    let mut result = Vec::with_capacity(regions.len());
    for region in regions {
        let region_id = text(region, "id")?.to_owned();
        let geometry = &region["geometry"];
        let mask = documents::geometry_mask(geometry, output[0], output[1], Path::new("/"))?;
        let indices = mask
            .iter()
            .enumerate()
            .filter(|(_, selected)| **selected)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if indices.is_empty() {
            return Err(Error::new(
                "insufficient_samples",
                format!("region {region_id} selects no pixels on the delivered output grid"),
            ));
        }
        result.push(Population {
            region_id,
            geometry: geometry.clone(),
            indices,
        });
    }
    Ok(result)
}

/// Read the producer-confirmed identity evidence for the intermediate grids.
/// The working grid needs the retained source-alignment flag plus the
/// observed working dimensions agreeing with the delivered output grid. The
/// source grid additionally needs the executed route to declare no crop, no
/// rescale and no preview resize on both routes, and its own working grid to
/// agree — never dimension equality alone.
fn identity_evidence(input: &Value, output: [usize; 2]) -> Identity {
    let simulation = &input["simulation_geometry"];
    let source = &input["source_geometry"];
    let working = input["source_pixel_aligned"].as_bool() == Some(true)
        && pair(&simulation["working_dimensions"]).is_some_and(|dims| dims == output);
    // A route is grid-preserving only when it declares identity explicitly.
    let identity_route = |geometry: &Value| {
        geometry["crop"].as_bool() == Some(false)
            && geometry["upscale_factor"].as_f64() == Some(1.0)
            && geometry["preview_mode"].as_bool() == Some(false)
    };
    let source = working
        && identity_route(simulation)
        && identity_route(source)
        && pair(&source["working_dimensions"]).is_some_and(|dims| dims == output);
    Identity { working, source }
}

/// One plan per producer stage boundary, in the producer's fixed order.
fn stage_plans(
    root: &Path,
    stages: &[Value],
    buffers: &Value,
    populations: &[Population],
    identity: Identity,
    output: [usize; 2],
    host_precision: Option<&str>,
) -> Result<Vec<StagePlan>> {
    let mut plans = Vec::with_capacity(stages.len());
    for stage in stages {
        let stage_id = text(stage, "id")?.to_owned();
        let availability = text(stage, "availability")?.to_owned();
        if availability != "available" {
            let reason = stage
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| "stage boundary is not available".to_owned());
            plans.push(StagePlan {
                stage_id,
                availability,
                reason: Some(reason),
                producer: None,
                statistics: None,
                artifact_refs: Vec::new(),
            });
            continue;
        }
        let precision = host_precision.ok_or_else(|| {
            Error::new(
                "stage_evidence",
                "captured stage requires observed host precision",
            )
        })?;
        let producer = producer_facts(stage, &stage["layout"], precision)?;
        let grid = [producer.width, producer.height];
        let space = text(&stage["geometry"], "coordinate_space")?;
        if grid != output || !identity.maps(space) {
            plans.push(StagePlan {
                stage_id,
                availability: "unavailable".to_owned(),
                reason: Some(UNMAPPED.to_owned()),
                producer: Some(producer),
                statistics: None,
                artifact_refs: Vec::new(),
            });
            continue;
        }
        let buffer = buffers
            .as_array()
            .into_iter()
            .flatten()
            .find(|buffer| buffer["stage_id"].as_str() == Some(stage_id.as_str()))
            .ok_or_else(|| {
                Error::new(
                    "stage_buffer",
                    format!("retained stage evidence has no buffer for stage {stage_id}"),
                )
            })?;
        if buffer.get("layout") != Some(&stage["layout"]) {
            return Err(Error::new(
                "stage_buffer",
                format!("retained buffer layout differs from stage {stage_id}"),
            ));
        }
        let reference = text(buffer, "path")?.to_owned();
        let samples = read_stage_samples(&buffer_path(root, &reference)?, grid)?;
        let statistics = populations
            .iter()
            .map(|population| summarize(&samples, &population.indices))
            .collect::<Vec<_>>();
        if statistics
            .iter()
            .flatten()
            .any(|stat| !stat.mean.is_finite())
        {
            return Err(Error::new(
                "computation_failure",
                format!("stage {stage_id} produced a non-finite channel statistic"),
            ));
        }
        plans.push(StagePlan {
            stage_id,
            availability,
            reason: None,
            producer: Some(producer),
            statistics: Some(statistics),
            artifact_refs: vec![reference],
        });
    }
    Ok(plans)
}

/// The producer-confirmed facts of one captured boundary. The host precision
/// comes from the bundle context, so an expanded f32 run stays labelled f32
/// even though every buffer is stored as f64.
fn producer_facts(stage: &Value, layout: &Value, host_precision: &str) -> Result<Producer> {
    Ok(Producer {
        domain: text(stage, "domain")?.to_owned(),
        units: text(stage, "units")?.to_owned(),
        channel_order: text(layout, "channel_order")?.to_owned(),
        geometry: stage["geometry"].clone(),
        host_precision: host_precision.to_owned(),
        width: positive(layout, "width")?,
        height: positive(layout, "height")?,
    })
}

/// Resolve one retained buffer reference inside the comparison destination.
fn buffer_path(root: &Path, reference: &str) -> Result<PathBuf> {
    documents::safe_artifact_path(root, reference)
}

/// Decode one little-endian f64 stage buffer in a single pass and refuse a
/// truncated or over-long file: a claimed available stage that cannot be
/// decoded is a computation failure.
fn read_stage_samples(path: &Path, grid: [usize; 2]) -> Result<Vec<f64>> {
    let count = grid[0]
        .checked_mul(grid[1])
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| Error::new("stage_buffer", "stage buffer length overflow"))?;
    let file = fs::File::open(path).map_err(|error| {
        Error::new(
            "stage_buffer",
            format!("stage buffer {} cannot be read: {error}", path.display()),
        )
    })?;
    let mut reader = BufReader::with_capacity(1 << 16, file);
    let mut samples = Vec::with_capacity(count);
    let mut block = vec![0u8; BLOCK_SAMPLES * SAMPLE_BYTES];
    let mut remaining = count;
    while remaining > 0 {
        let words = remaining.min(BLOCK_SAMPLES);
        let bytes = words * SAMPLE_BYTES;
        reader.read_exact(&mut block[..bytes]).map_err(|error| {
            Error::new(
                "stage_buffer",
                format!("stage buffer {} is truncated: {error}", path.display()),
            )
        })?;
        for word in block[..bytes].as_chunks::<8>().0 {
            samples.push(f64::from_le_bytes(*word));
        }
        remaining -= words;
    }
    let mut trailing = [0u8; 1];
    if reader.read(&mut trailing).map_err(|error| {
        Error::new(
            "stage_buffer",
            format!("stage buffer {} cannot be read: {error}", path.display()),
        )
    })? != 0
    {
        return Err(Error::new(
            "stage_buffer",
            format!(
                "stage buffer {} is longer than its declared layout",
                path.display()
            ),
        ));
    }
    if samples.iter().any(|value| !value.is_finite()) {
        return Err(Error::new(
            "nonfinite_samples",
            format!(
                "stage buffer {} contains a non-finite sample",
                path.display()
            ),
        ));
    }
    Ok(samples)
}

/// Per-channel arithmetic mean, minimum and maximum over one region
/// population. Channels are never mixed: channel `c` only ever reads sample
/// `pixel * 3 + c`.
fn summarize(samples: &[f64], indices: &[usize]) -> [Stats; 3] {
    let mut sums = [0.0f64; 3];
    let mut mins = [f64::INFINITY; 3];
    let mut maxs = [f64::NEG_INFINITY; 3];
    for &pixel in indices {
        let base = pixel * 3;
        for (channel, sum) in sums.iter_mut().enumerate() {
            let value = samples[base + channel];
            *sum += value;
            mins[channel] = mins[channel].min(value);
            maxs[channel] = maxs[channel].max(value);
        }
    }
    let count = indices.len() as f64;
    std::array::from_fn(|channel| Stats {
        mean: sums[channel] / count,
        min: mins[channel],
        max: maxs[channel],
    })
}

/// Flatten the plans into one record per region and stage. Available records
/// carry the producer facts and the channel statistics; refused and skipped
/// boundaries carry their stable reason and no channel values.
fn records(role: &str, populations: &[Population], plans: &[StagePlan]) -> Vec<Value> {
    let mut result = Vec::with_capacity(populations.len() * plans.len());
    for (position, population) in populations.iter().enumerate() {
        for plan in plans {
            // One statistics entry per declared region, in declared order.
            let statistics = plan.statistics.as_ref().and_then(|s| s.get(position));
            let sample_count = statistics.map_or(0, |_| population.indices.len());
            let mut readout = json!({
                "run_role": role,
                "region_id": population.region_id,
                "stage_id": plan.stage_id,
                "estimator": ESTIMATOR,
                "availability": plan.availability,
                "region_geometry": population.geometry,
                "region_coordinate_space": REGION_COORDINATE_SPACE,
                "sample_count": sample_count,
                "artifact_refs": plan.artifact_refs,
            });
            if let Some(reason) = &plan.reason {
                readout["reason"] = json!(reason);
            }
            if let Some(producer) = &plan.producer {
                readout["stage_geometry"] = producer.geometry.clone();
                readout["domain"] = json!(producer.domain);
                readout["units"] = json!(producer.units);
                readout["channel_order"] = json!(producer.channel_order);
                readout["host_precision"] = json!(producer.host_precision);
                readout["storage_dtype"] = json!(STORAGE_DTYPE);
            }
            if let Some(stats) = statistics {
                readout["channel_mean"] = channel_vector(stats, |stat| stat.mean);
                readout["channel_min"] = channel_vector(stats, |stat| stat.min);
                readout["channel_max"] = channel_vector(stats, |stat| stat.max);
            }
            result.push(readout);
        }
    }
    result
}

fn channel_vector(stats: &[Stats; 3], select: impl Fn(&Stats) -> f64) -> Value {
    json!([select(&stats[0]), select(&stats[1]), select(&stats[2])])
}

/// A positive integer layout field.
fn positive(section: &Value, key: &str) -> Result<usize> {
    section[key]
        .as_u64()
        .filter(|value| *value > 0)
        .map(|value| value as usize)
        .ok_or_else(|| {
            Error::new(
                "stage_evidence",
                format!("stage layout {key} must be a positive integer"),
            )
        })
}

/// Two non-negative integer dimensions.
fn pair(value: &Value) -> Option<[usize; 2]> {
    let array = value.as_array()?;
    if array.len() != 2 {
        return None;
    }
    Some([array[0].as_u64()? as usize, array[1].as_u64()? as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{common::sha256_file, image_data::write_npy};

    /// Producer-confirmed stage semantics, restated so the tests can build
    /// bundles the way the renderer writes them.
    fn semantics(id: &str) -> (&'static str, &'static str, &'static str) {
        match id {
            "rgb_in" => ("RGB", "input_rgb", "relative"),
            "log_e_film" => ("CMY", "record_exposure", "log10"),
            "cmy_film" => ("CMY", "record_density", "optical_density"),
            "log_e_print" => ("CMY", "record_exposure", "log10"),
            "cmy_print" => ("CMY", "record_density", "optical_density"),
            _ => ("RGB", "output_rgb", "relative"),
        }
    }

    struct Stage<'a> {
        id: &'a str,
        availability: &'a str,
        reason: Option<&'a str>,
        space: &'a str,
        width: usize,
        height: usize,
        samples: Option<Vec<f64>>,
    }

    fn captured(id: &'static str, space: &'static str, samples: Vec<f64>) -> Stage<'static> {
        Stage {
            id,
            availability: "available",
            reason: None,
            space,
            width: 4,
            height: 4,
            samples: Some(samples),
        }
    }

    fn skipped(id: &'static str, reason: &'static str) -> Stage<'static> {
        Stage {
            id,
            availability: "not_applicable",
            reason: Some(reason),
            space: "output_pixels",
            width: 4,
            height: 4,
            samples: None,
        }
    }

    fn failed(id: &'static str, reason: &'static str) -> Stage<'static> {
        Stage {
            id,
            availability: "unavailable",
            reason: Some(reason),
            space: "output_pixels",
            width: 4,
            height: 4,
            samples: None,
        }
    }

    /// Grid samples where pixel `p` channel `c` holds `(p + 1) * (c + 1)`, so
    /// every channel and every subset is independently identifiable.
    fn grid(pixels: usize) -> Vec<f64> {
        (0..pixels)
            .flat_map(|pixel| {
                [0, 1, 2].map(|channel| (pixel as f64 + 1.0) * (channel as f64 + 1.0))
            })
            .collect()
    }

    /// The executed-route geometry declaration the runner retains for one
    /// route. The source route decodes the source raster without the
    /// simulation's grain-owned crop and rescale.
    fn geometry(working: [u64; 2], crop: bool, upscale: f64, preview: bool) -> Value {
        json!({
            "input_dimensions": [4, 4],
            "working_dimensions": working,
            "published_dimensions": [4, 4],
            "crop": crop,
            "crop_center": [0.5, 0.5],
            "crop_size": [4, 4],
            "upscale_factor": upscale,
            "preview_mode": preview,
            "preview_max_size": 2048,
        })
    }

    /// The retained input evidence: both executed geometry declarations and
    /// the producer-confirmed source-alignment flag.
    fn input(source_pixel_aligned: bool, simulation: Value) -> Value {
        json!({
            "source_pixel_aligned": source_pixel_aligned,
            "source_geometry": geometry([4, 4], false, 1.0, false),
            "simulation_geometry": simulation,
            "source_dimensions": [4, 4],
        })
    }

    /// A route that the producer confirms delivers the source grid unchanged.
    fn aligned() -> Value {
        input(true, geometry([4, 4], false, 1.0, false))
    }

    /// Write the stage buffers and build the retained per-role payload the
    /// runner hands to the comparison.
    fn payload(
        directory: &Path,
        host_precision: &str,
        input: Value,
        stages: &[Stage<'_>],
    ) -> Value {
        let mut buffers = Vec::new();
        let mut observed_stages = Vec::new();
        for stage in stages {
            let (order, domain, units) = semantics(stage.id);
            let mut entry = json!({
                "id": stage.id,
                "availability": stage.availability,
            });
            if let Some(reason) = stage.reason {
                entry["reason"] = json!(reason);
            }
            if let Some(samples) = &stage.samples {
                let reference = format!("{}.f64", stage.id);
                let path = directory.join(&reference);
                let bytes = samples
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect::<Vec<u8>>();
                fs::write(&path, bytes).unwrap();
                let layout = json!({
                    "width": stage.width,
                    "height": stage.height,
                    "channels": 3,
                    "dtype": "f64",
                    "endianness": "little",
                    "channel_order": order,
                });
                entry["path"] = json!(reference);
                entry["sha256"] = json!(sha256_file(&path).unwrap());
                entry["layout"] = layout.clone();
                entry["domain"] = json!(domain);
                entry["units"] = json!(units);
                entry["geometry"] = json!({"coordinate_space": stage.space});
                buffers.push(json!({
                    "path": reference,
                    "sha256": entry["sha256"].clone(),
                    "role": "stage_evidence_buffer",
                    "media_type": "application/octet-stream",
                    "stage_id": stage.id,
                    "layout": layout,
                }));
            }
            observed_stages.push(entry);
        }
        json!({
            "role": "baseline",
            "input": input,
            "precision": json!({"host_precision": host_precision, "gpu_precision": "f32", "spectral_preparation_precision": "f64"}),
            "observed": {
                "host_precision": host_precision,
                "workflow_route": "print",
                "scan_output": "negative_to_positive",
                "normalization_mode": "profile_prepared",
                "exposure": {"availability": "available"},
                "print_normalization": {"availability": "available"},
                "stages": observed_stages,
                "errors": [],
            },
            "stage_evidence": {
                "requested": true,
                "availability": "available",
                "buffers": buffers,
            },
        })
    }

    fn run_evidence(payload: Value) -> Value {
        json!({"baseline": payload})
    }

    fn region(id: &str, geometry: Value) -> Value {
        json!({"id": id, "geometry": geometry})
    }

    fn readout<'a>(records: &'a [Value], region_id: &str, stage_id: &str) -> &'a Value {
        records
            .iter()
            .find(|readout| readout["region_id"] == region_id && readout["stage_id"] == stage_id)
            .unwrap_or_else(|| panic!("no record for {region_id}/{stage_id}"))
    }

    /// Every captured stage boundary of a route that executes the film chain
    /// on the working grid, with the two RGB boundaries at their own grids.
    fn print_route(cmy_samples: Vec<f64>) -> [Stage<'static>; 6] {
        [
            failed("rgb_in", "outside_executed_tap_range"),
            skipped("log_e_film", "fused_into_grain_readout"),
            captured("cmy_film", "working_pixels", cmy_samples),
            skipped("log_e_print", "no_print_stage_on_route"),
            skipped("cmy_print", "no_print_stage_on_route"),
            captured("rgb_out", "output_pixels", vec![0.25; 48]),
        ]
    }

    #[test]
    fn cmy_region_statistics_report_the_true_subset() {
        let directory = tempfile::tempdir().unwrap();
        let evidence = payload(directory.path(), "f64", aligned(), &print_route(grid(16)));
        let regions = [
            region("patch", json!([1, 1, 2, 2])),
            region("whole", json!([0, 0, 4, 4])),
        ];
        let records = measure(directory.path(), &run_evidence(evidence), &regions).unwrap();

        let density = readout(&records, "patch", "cmy_film");
        assert_eq!(density["availability"], json!("available"));
        assert_eq!(density["domain"], json!("record_density"));
        assert_eq!(density["units"], json!("optical_density"));
        assert_eq!(density["channel_order"], json!("CMY"));
        assert_eq!(density["host_precision"], json!("f64"));
        assert_eq!(density["storage_dtype"], json!("f64"));
        assert_eq!(density["region_coordinate_space"], json!("output_pixels"));
        assert_eq!(density["region_geometry"], json!([1, 1, 2, 2]));
        assert_eq!(
            density["stage_geometry"],
            json!({"coordinate_space": "working_pixels"})
        );
        assert_eq!(density["sample_count"], json!(4));
        assert_eq!(density["artifact_refs"].as_array().unwrap().len(), 1);
        // Pixels 5, 6, 9 and 10 hold (p + 1) * (c + 1).
        assert_eq!(density["channel_mean"], json!([8.5, 17.0, 25.5]));
        assert_eq!(density["channel_min"], json!([6.0, 12.0, 18.0]));
        assert_eq!(density["channel_max"], json!([11.0, 22.0, 33.0]));

        // The same grid over the whole delivered image reports the whole
        // population, so the region readout is a true subset.
        let whole = readout(&records, "whole", "cmy_film");
        assert_eq!(whole["sample_count"], json!(16));
        assert_eq!(whole["channel_mean"], json!([8.5, 17.0, 25.5]));
        assert_eq!(whole["channel_min"], json!([1.0, 2.0, 3.0]));
        assert_eq!(whole["channel_max"], json!([16.0, 32.0, 48.0]));
        assert_ne!(whole["channel_min"], density["channel_min"]);
        assert_ne!(whole["channel_max"], density["channel_max"]);

        let output = readout(&records, "patch", "rgb_out");
        assert_eq!(output["availability"], json!("available"));
        assert_eq!(output["domain"], json!("output_rgb"));
        assert_eq!(output["units"], json!("relative"));
        assert_eq!(output["channel_order"], json!("RGB"));
        assert_eq!(
            output["stage_geometry"],
            json!({"coordinate_space": "output_pixels"})
        );
        assert_eq!(output["channel_mean"], json!([0.25, 0.25, 0.25]));
        assert_eq!(output["sample_count"], json!(4));

        // A refused boundary keeps its producer reason and no channel values.
        let exposure = readout(&records, "patch", "log_e_film");
        assert_eq!(exposure["availability"], json!("not_applicable"));
        assert_eq!(exposure["reason"], json!("fused_into_grain_readout"));
        assert_eq!(exposure["sample_count"], json!(0));
        assert!(exposure.get("channel_mean").is_none());
        assert!(exposure.get("domain").is_none());
        assert!(exposure.get("host_precision").is_none());
        assert!(exposure.get("storage_dtype").is_none());
        assert_eq!(exposure["artifact_refs"], json!([]));

        let rgb_in = readout(&records, "patch", "rgb_in");
        assert_eq!(rgb_in["availability"], json!("unavailable"));
        assert_eq!(rgb_in["reason"], json!("outside_executed_tap_range"));
        assert!(rgb_in.get("stage_geometry").is_none());
    }

    #[test]
    fn channel_statistics_do_not_mix_channels() {
        let directory = tempfile::tempdir().unwrap();
        // One repeating pixel whose three channels are distinct and out of
        // order, so any channel mixing would reorder or smear them.
        let samples = [0.5, 2.0, -1.0].repeat(16);
        let evidence = payload(directory.path(), "f64", aligned(), &print_route(samples));
        let records = measure(
            directory.path(),
            &run_evidence(evidence),
            &[region("single", json!([2, 3, 1, 1]))],
        )
        .unwrap();
        let output = readout(&records, "single", "cmy_film");
        assert_eq!(output["availability"], json!("available"));
        assert_eq!(output["sample_count"], json!(1));
        assert_eq!(output["channel_mean"], json!([0.5, 2.0, -1.0]));
        assert_eq!(output["channel_min"], json!([0.5, 2.0, -1.0]));
        assert_eq!(output["channel_max"], json!([0.5, 2.0, -1.0]));
    }

    #[test]
    fn masked_region_statistics_skip_unselected_pixels() {
        let directory = tempfile::tempdir().unwrap();
        // Only the first and the last pixel of the 4x4 grid are selected.
        let mut values = vec![0.0; 16];
        values[0] = 1.0;
        values[15] = 1.0;
        let mask = directory.path().join("mask.npy");
        write_npy(&mask, &values, &[4, 4]).unwrap();
        let evidence = payload(directory.path(), "f64", aligned(), &print_route(grid(16)));
        let geometry = json!({
            "path": mask.to_string_lossy(),
            "sha256": sha256_file(&mask).unwrap(),
            "width": 4,
            "height": 4,
        });
        let records = measure(
            directory.path(),
            &run_evidence(evidence),
            &[region("holes", geometry.clone())],
        )
        .unwrap();
        let density = readout(&records, "holes", "cmy_film");
        assert_eq!(density["availability"], json!("available"));
        assert_eq!(density["sample_count"], json!(2));
        // Pixels 0 and 15 hold (p + 1) * (c + 1); the fourteen unselected
        // pixels never contribute.
        assert_eq!(density["channel_mean"], json!([8.5, 17.0, 25.5]));
        assert_eq!(density["channel_min"], json!([1.0, 2.0, 3.0]));
        assert_eq!(density["channel_max"], json!([16.0, 32.0, 48.0]));
        assert_eq!(density["region_geometry"], geometry);
    }

    #[test]
    fn source_grid_needs_more_than_dimension_equality() {
        let directory = tempfile::tempdir().unwrap();
        let stages = [
            captured("rgb_in", "source_pixels", grid(16)),
            skipped("log_e_film", "no_film_stage_on_route"),
            skipped("cmy_film", "no_film_stage_on_route"),
            skipped("log_e_print", "no_print_stage_on_route"),
            skipped("cmy_print", "no_print_stage_on_route"),
            captured("rgb_out", "output_pixels", vec![0.5; 48]),
        ];
        // An executed crop on a dimension-equal grid must not map.
        let cropped = input(true, geometry([4, 4], true, 1.0, false));
        let records = measure(
            directory.path(),
            &run_evidence(payload(directory.path(), "f64", cropped, &stages)),
            &[region("patch", json!([0, 0, 1, 1]))],
        )
        .unwrap();
        let source = readout(&records, "patch", "rgb_in");
        assert_eq!(source["availability"], json!("unavailable"));
        assert_eq!(source["reason"], json!("stage_region_geometry_unmapped"));
        assert!(source.get("channel_mean").is_none());
        // The producer facts stay visible for a refused mapping.
        assert_eq!(source["domain"], json!("input_rgb"));
        assert_eq!(source["units"], json!("relative"));
        assert_eq!(source["channel_order"], json!("RGB"));
        assert_eq!(source["host_precision"], json!("f64"));
        assert_eq!(
            source["stage_geometry"],
            json!({"coordinate_space": "source_pixels"})
        );
        assert_eq!(source["artifact_refs"], json!([]));

        // The same grid on an explicitly identity route maps.
        let records = measure(
            directory.path(),
            &run_evidence(payload(directory.path(), "f64", aligned(), &stages)),
            &[region("patch", json!([0, 0, 1, 1]))],
        )
        .unwrap();
        let source = readout(&records, "patch", "rgb_in");
        assert_eq!(source["availability"], json!("available"));
        assert_eq!(source["sample_count"], json!(1));
        assert_eq!(source["channel_mean"], json!([1.0, 2.0, 3.0]));

        // A non-unit upscale and a preview resize each refuse the mapping
        // alone, without any crop.
        for simulation in [
            geometry([4, 4], false, 2.0, false),
            geometry([4, 4], false, 1.0, true),
        ] {
            let records = measure(
                directory.path(),
                &run_evidence(payload(
                    directory.path(),
                    "f64",
                    input(true, simulation),
                    &stages,
                )),
                &[region("patch", json!([0, 0, 1, 1]))],
            )
            .unwrap();
            let source = readout(&records, "patch", "rgb_in");
            assert_eq!(source["availability"], json!("unavailable"));
            assert_eq!(source["reason"], json!("stage_region_geometry_unmapped"));
        }
    }

    #[test]
    fn working_grid_needs_the_retained_source_alignment_flag() {
        let directory = tempfile::tempdir().unwrap();
        let stages = [
            failed("rgb_in", "outside_executed_tap_range"),
            skipped("log_e_film", "no_film_stage_on_route"),
            captured("cmy_film", "working_pixels", grid(16)),
            skipped("log_e_print", "no_print_stage_on_route"),
            skipped("cmy_print", "no_print_stage_on_route"),
            captured("rgb_out", "output_pixels", vec![0.5; 48]),
        ];
        // Matching dimensions without the retained alignment flag never map.
        let records = measure(
            directory.path(),
            &run_evidence(payload(
                directory.path(),
                "f64",
                input(false, geometry([4, 4], false, 1.0, false)),
                &stages,
            )),
            &[region("patch", json!([0, 0, 1, 1]))],
        )
        .unwrap();
        assert_eq!(
            readout(&records, "patch", "cmy_film")["reason"],
            json!("stage_region_geometry_unmapped")
        );

        // A working grid that differs from the delivered output never maps.
        let records = measure(
            directory.path(),
            &run_evidence(payload(
                directory.path(),
                "f64",
                input(true, geometry([5, 4], false, 1.0, false)),
                &stages,
            )),
            &[region("patch", json!([0, 0, 1, 1]))],
        )
        .unwrap();
        assert_eq!(
            readout(&records, "patch", "cmy_film")["reason"],
            json!("stage_region_geometry_unmapped")
        );
    }

    #[test]
    fn intermediate_grids_without_declared_roi_transform_stay_unmapped() {
        let directory = tempfile::tempdir().unwrap();
        let stages = [
            failed("rgb_in", "outside_executed_tap_range"),
            captured("log_e_film", "film_frame_pixels", grid(16)),
            captured("cmy_film", "film_frame_pixels", grid(16)),
            skipped("log_e_print", "no_print_stage_on_route"),
            captured("cmy_print", "grain_aperture_pixels", grid(16)),
            captured("rgb_out", "output_pixels", vec![0.5; 48]),
        ];
        let records = measure(
            directory.path(),
            &run_evidence(payload(directory.path(), "f64", aligned(), &stages)),
            &[region("patch", json!([1, 1, 2, 2]))],
        )
        .unwrap();
        for stage_id in ["log_e_film", "cmy_film", "cmy_print"] {
            let stage = readout(&records, "patch", stage_id);
            assert_eq!(stage["availability"], json!("unavailable"), "{stage_id}");
            assert_eq!(
                stage["reason"],
                json!("stage_region_geometry_unmapped"),
                "{stage_id}"
            );
            assert_eq!(stage["sample_count"], json!(0), "{stage_id}");
            for key in ["channel_mean", "channel_min", "channel_max"] {
                assert!(stage.get(key).is_none(), "{stage_id}.{key}");
            }
            assert_eq!(stage["artifact_refs"], json!([]), "{stage_id}");
        }
        // The record axis stays CMY and the domains stay record-based.
        assert_eq!(
            readout(&records, "patch", "log_e_film")["domain"],
            json!("record_exposure")
        );
        assert_eq!(
            readout(&records, "patch", "log_e_film")["units"],
            json!("log10")
        );
        assert_eq!(
            readout(&records, "patch", "cmy_film")["units"],
            json!("optical_density")
        );
        assert_eq!(
            readout(&records, "patch", "cmy_print")["channel_order"],
            json!("CMY")
        );
        // The delivered output grid still maps.
        assert_eq!(
            readout(&records, "patch", "rgb_out")["availability"],
            json!("available")
        );
    }

    #[test]
    fn missing_capture_reports_every_boundary_unavailable() {
        let directory = tempfile::tempdir().unwrap();
        let stages = documents::STAGE_IDS
            .iter()
            .map(|id| {
                json!({
                    "id": id,
                    "availability": "unavailable",
                    "reason": format!("stage capture not requested; stage {id} unobserved"),
                })
            })
            .collect::<Vec<_>>();
        let evidence = json!({
            "role": "baseline",
            "input": aligned(),
            "observed": {
                "exposure": {"availability": "unavailable", "reason": "stage capture not requested"},
                "print_normalization": {"availability": "unavailable", "reason": "stage capture not requested"},
                "stages": stages,
            },
            "stage_evidence": {
                "requested": false,
                "availability": "not_applicable",
                "reason": "stage capture not requested",
            },
        });
        let regions = [
            region("first", json!([0, 0, 1, 1])),
            region("second", json!([2, 2, 2, 2])),
        ];
        let records = measure(directory.path(), &run_evidence(evidence), &regions).unwrap();
        // Two declared regions times six named boundaries.
        assert_eq!(records.len(), 12);
        for readout in &records {
            assert_eq!(readout["availability"], json!("unavailable"));
            assert!(
                readout["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.starts_with("stage capture not requested"))
            );
            assert_eq!(readout["sample_count"], json!(0));
            assert_eq!(readout["artifact_refs"], json!([]));
            for key in [
                "stage_geometry",
                "domain",
                "units",
                "channel_order",
                "host_precision",
                "storage_dtype",
                "channel_mean",
                "channel_min",
                "channel_max",
            ] {
                assert!(readout.get(key).is_none(), "{key} must stay unobserved");
            }
        }
        assert_eq!(records[0]["region_id"], json!("first"));
        assert_eq!(records[5]["region_id"], json!("first"));
        assert_eq!(records[6]["region_id"], json!("second"));
        assert_eq!(records[0]["stage_id"], json!("rgb_in"));
        assert_eq!(records[5]["stage_id"], json!("rgb_out"));
    }

    #[test]
    fn route_skipped_boundary_keeps_not_applicable_and_producer_reason() {
        let directory = tempfile::tempdir().unwrap();
        let evidence = payload(directory.path(), "f64", aligned(), &print_route(grid(16)));
        let records = measure(
            directory.path(),
            &run_evidence(evidence),
            &[region("patch", json!([0, 0, 2, 2]))],
        )
        .unwrap();
        let skipped_stage = readout(&records, "patch", "log_e_film");
        assert_eq!(skipped_stage["availability"], json!("not_applicable"));
        assert_eq!(skipped_stage["reason"], json!("fused_into_grain_readout"));
        assert!(skipped_stage.get("channel_mean").is_none());
        assert!(skipped_stage.get("domain").is_none());
        // A route-skipped boundary invents no buffer reference.
        assert_eq!(skipped_stage["artifact_refs"], json!([]));
        assert_eq!(
            readout(&records, "patch", "rgb_out")["availability"],
            json!("available")
        );
    }

    #[test]
    fn nonfinite_available_samples_fail_the_readout() {
        let directory = tempfile::tempdir().unwrap();
        let mut samples = grid(16);
        // A non-finite sample inside the declared region population.
        samples[5 * 3] = f64::NAN;
        let evidence = payload(directory.path(), "f64", aligned(), &print_route(samples));
        let error = measure(
            directory.path(),
            &run_evidence(evidence),
            &[region("patch", json!([1, 1, 2, 2]))],
        )
        .unwrap_err();
        assert_eq!(error.code, "nonfinite_samples");

        // A truncated buffer on a claimed available stage fails as well.
        let directory = tempfile::tempdir().unwrap();
        let evidence = payload(directory.path(), "f64", aligned(), &print_route(grid(16)));
        let path = directory.path().join("cmy_film.f64");
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() - 8]).unwrap();
        let error = measure(
            directory.path(),
            &run_evidence(evidence),
            &[region("patch", json!([1, 1, 2, 2]))],
        )
        .unwrap_err();
        assert_eq!(error.code, "stage_buffer");
    }

    #[test]
    fn missing_buffer_on_a_claimed_available_stage_fails() {
        let directory = tempfile::tempdir().unwrap();
        let mut evidence = payload(directory.path(), "f64", aligned(), &print_route(grid(16)));
        evidence["stage_evidence"]["buffers"]
            .as_array_mut()
            .unwrap()
            .retain(|buffer| buffer["stage_id"] != "cmy_film");
        let error = measure(
            directory.path(),
            &run_evidence(evidence),
            &[region("patch", json!([1, 1, 2, 2]))],
        )
        .unwrap_err();
        assert_eq!(error.code, "stage_buffer");
        assert!(error.message.contains("no buffer"), "{}", error.message);
    }

    #[test]
    fn expanded_f32_run_stays_labelled_f32() {
        let directory = tempfile::tempdir().unwrap();
        let evidence = payload(directory.path(), "f32", aligned(), &print_route(grid(16)));
        let records = measure(
            directory.path(),
            &run_evidence(evidence),
            &[region("patch", json!([0, 0, 4, 4]))],
        )
        .unwrap();
        let density = readout(&records, "patch", "cmy_film");
        assert_eq!(density["availability"], json!("available"));
        assert_eq!(density["host_precision"], json!("f32"));
        assert_eq!(density["storage_dtype"], json!("f64"));
        assert_eq!(density["sample_count"], json!(16));
        assert_eq!(density["channel_mean"], json!([8.5, 17.0, 25.5]));
        assert_eq!(
            readout(&records, "patch", "rgb_out")["host_precision"],
            json!("f32")
        );
    }

    #[test]
    fn every_declared_region_is_reported_for_every_stage() {
        let directory = tempfile::tempdir().unwrap();
        let stages = [
            captured("rgb_in", "source_pixels", grid(16)),
            skipped("log_e_film", "no_film_stage_on_route"),
            captured("cmy_film", "working_pixels", grid(16)),
            skipped("log_e_print", "no_print_stage_on_route"),
            skipped("cmy_print", "no_print_stage_on_route"),
            captured("rgb_out", "output_pixels", vec![0.5; 48]),
        ];
        let regions = [
            region("left", json!([0, 0, 1, 1])),
            region("right", json!([3, 3, 1, 1])),
        ];
        let records = measure(
            directory.path(),
            &run_evidence(payload(directory.path(), "f64", aligned(), &stages)),
            &regions,
        )
        .unwrap();
        assert_eq!(records.len(), 12);
        assert_eq!(records[0]["region_id"], json!("left"));
        assert_eq!(records[0]["stage_id"], json!("rgb_in"));
        assert_eq!(records[5]["stage_id"], json!("rgb_out"));
        assert_eq!(records[6]["region_id"], json!("right"));
        // The first and last pixels of the grid hold different values.
        assert_eq!(
            readout(&records, "left", "cmy_film")["channel_mean"],
            json!([1.0, 2.0, 3.0])
        );
        assert_eq!(
            readout(&records, "right", "cmy_film")["channel_mean"],
            json!([16.0, 32.0, 48.0])
        );
        assert_eq!(
            readout(&records, "left", "cmy_film")["sample_count"],
            json!(1)
        );
        assert_eq!(
            readout(&records, "right", "cmy_film")["sample_count"],
            json!(1)
        );
    }

    #[test]
    fn multiple_run_roles_are_reported_independently() {
        let directory = tempfile::tempdir().unwrap();
        let print = |samples: Vec<f64>| {
            [
                failed("rgb_in", "outside_executed_tap_range"),
                skipped("log_e_film", "no_film_stage_on_route"),
                captured("cmy_film", "working_pixels", samples),
                skipped("log_e_print", "no_print_stage_on_route"),
                skipped("cmy_print", "no_print_stage_on_route"),
                captured("rgb_out", "output_pixels", vec![0.5; 48]),
            ]
        };
        let baseline_dir = directory.path().join("baseline");
        fs::create_dir_all(&baseline_dir).unwrap();
        let baseline = payload(&baseline_dir, "f64", aligned(), &print(grid(16)));
        let candidate_dir = directory.path().join("candidate");
        fs::create_dir_all(&candidate_dir).unwrap();
        let mut candidate_samples = grid(16);
        candidate_samples[0] = 100.0;
        let candidate = payload(&candidate_dir, "f64", aligned(), &print(candidate_samples));
        let mut evidence = json!({"baseline": baseline, "candidate": candidate});
        for role in ["baseline", "candidate"] {
            for buffer in evidence[role]["stage_evidence"]["buffers"]
                .as_array_mut()
                .unwrap()
            {
                buffer["path"] = json!(format!("{role}/{}", buffer["path"].as_str().unwrap()));
            }
        }
        let records = measure(
            directory.path(),
            &evidence,
            &[region("patch", json!([0, 0, 4, 4]))],
        )
        .unwrap();
        assert_eq!(records.len(), 12);
        assert_eq!(records[0]["run_role"], json!("baseline"));
        assert_eq!(records[6]["run_role"], json!("candidate"));
        let record = |role: &str| {
            records
                .iter()
                .find(|row| row["run_role"] == role && row["stage_id"] == "cmy_film")
                .unwrap()
        };
        assert_eq!(record("baseline")["channel_mean"], json!([8.5, 17.0, 25.5]));
        assert_eq!(
            record("candidate")["channel_mean"],
            json!([14.6875, 17.0, 25.5])
        );
        assert_eq!(
            record("candidate")["channel_max"],
            json!([100.0, 32.0, 48.0])
        );
    }

    #[test]
    fn stage_boundaries_must_name_every_stage_exactly_once() {
        let directory = tempfile::tempdir().unwrap();
        let evidence = json!({
            "role": "baseline",
            "input": aligned(),
            "precision": json!({"host_precision": "f64"}),
            "observed": {"stages": [{"id": "rgb_in", "availability": "not_applicable", "reason": "x"}]},
            "stage_evidence": {"requested": true, "availability": "available", "buffers": []},
        });
        let error = measure(
            directory.path(),
            &run_evidence(evidence),
            &[region("patch", json!([0, 0, 1, 1]))],
        )
        .unwrap_err();
        assert_eq!(error.code, "stage_evidence");
    }

    #[test]
    fn no_declared_regions_produce_no_records_without_decoding_buffers() {
        let directory = tempfile::tempdir().unwrap();
        // A malformed payload: without the early return, reading the stage
        // list or the output dimensions would already fail here.
        let evidence = json!({
            "role": "baseline",
            "input": aligned(),
            "observed": {"stages": "not an array"},
            "stage_evidence": {"requested": true, "availability": "available", "buffers": []},
        });
        let records = measure(directory.path(), &run_evidence(evidence), &[]).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn each_declared_region_gets_its_own_statistics() {
        let directory = tempfile::tempdir().unwrap();
        // Distinct per-region populations, so any shared statistics entry
        // would produce identical values for both regions.
        let evidence = payload(directory.path(), "f64", aligned(), &print_route(grid(16)));
        let regions = [
            region("first", json!([0, 0, 1, 1])),
            region("middle", json!([1, 1, 2, 2])),
            region("last", json!([3, 3, 1, 1])),
        ];
        let records = measure(directory.path(), &run_evidence(evidence), &regions).unwrap();
        assert_eq!(records.len(), 18);
        // Every region maps to a different pixel population of the grid.
        assert_eq!(
            readout(&records, "first", "cmy_film")["channel_mean"],
            json!([1.0, 2.0, 3.0])
        );
        assert_eq!(
            readout(&records, "middle", "cmy_film")["channel_mean"],
            json!([8.5, 17.0, 25.5])
        );
        assert_eq!(
            readout(&records, "last", "cmy_film")["channel_mean"],
            json!([16.0, 32.0, 48.0])
        );
        assert_eq!(
            readout(&records, "first", "cmy_film")["sample_count"],
            json!(1)
        );
        assert_eq!(
            readout(&records, "middle", "cmy_film")["sample_count"],
            json!(4)
        );
        assert_eq!(
            readout(&records, "last", "cmy_film")["sample_count"],
            json!(1)
        );
        // The min/max vectors follow their own region, not a neighbour's.
        assert_eq!(
            readout(&records, "middle", "cmy_film")["channel_min"],
            json!([6.0, 12.0, 18.0])
        );
        assert_eq!(
            readout(&records, "middle", "cmy_film")["channel_max"],
            json!([11.0, 22.0, 33.0])
        );
        // Record order is region-major, stage-minor, for every region.
        assert_eq!(records[0]["region_id"], json!("first"));
        assert_eq!(records[6]["region_id"], json!("middle"));
        assert_eq!(records[12]["region_id"], json!("last"));
        assert_eq!(records[17]["stage_id"], json!("rgb_out"));
    }
}
