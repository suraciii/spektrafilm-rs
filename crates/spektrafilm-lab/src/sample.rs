//! `measure sample`: readings of a declared raster at declared positions.
//!
//! The source is a raster image carrying the input declaration `process`
//! requires. The command reads the declared rectangles and reports their
//! means; it never resizes, converts beyond the declared transfer decoding,
//! clamps, or repairs anything.

use std::path::{Path, PathBuf};

use crate::image_data::{self, ImageData};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use spektrafilm_math::colorspace::{self, RgbColorSpace};

use clap::Args;

use crate::plan::{self, Plan};
use crate::readings::{csv_field, number_field, sha256_file, statistics, write_measure_named};

#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("sample_source").required(true).multiple(false)
        .args(["source"])
))]
pub struct MeasureSampleArgs {
    /// Raster image to read; requires the input declaration below.
    #[arg(long, value_name = "PATH", help_heading = "Source")]
    pub source: Option<PathBuf>,
    /// Source color space of the image, as `process` declares it.
    #[arg(
        long,
        value_name = "NAME",
        requires = "source",
        help_heading = "Source"
    )]
    pub input_color_space: Option<String>,
    /// Whether the image's stored values are transfer-encoded.
    #[arg(
        long,
        value_name = "true|false",
        value_parser = clap::value_parser!(bool),
        requires = "source",
        help_heading = "Source"
    )]
    pub input_cctf_decoding: Option<bool>,
    /// Regions document: explicit windows with stable sample identities.
    #[arg(long, value_name = "PATH", help_heading = "Measurement")]
    pub regions: PathBuf,
    /// Record destination directory; existing artifacts are never replaced.
    #[arg(long, value_name = "DIR", help_heading = "Measurement")]
    pub out: PathBuf,
}

/// The one declared source of a measurement, and the declaration that
/// governs its reading.
struct Source {
    /// The source kind this record reports.
    kind: &'static str,
    /// The declared image path or bundle/run directory.
    path: PathBuf,
    /// The image file's content hash.
    sha256: String,
    /// The decoded image the rectangles are read from.
    pixels: ImageData,
    /// The declared color space of the values, when the source declares one.
    color_space: Option<String>,
    /// The resolved declared space, when the source declares one.
    space: Option<&'static RgbColorSpace>,
    /// Whether the stored values are transfer-encoded; `None` when the
    /// source declares none, so the record does not invent one.
    encoded: Option<bool>,
    /// Whether the reading applies the declared transfer decoding, so the
    /// values it stores are the linear boundary values.
    decoded: bool,
    /// The producer-declared coordinate space of the source values.
    coordinate_space: String,
    /// The reading's vocabulary, recorded verbatim from its own authority.
    domain: String,
    units: String,
    channel_order: String,
}

impl Source {
    /// The stage identity this measurement reports: a raster image is one
    /// reading, and the record names it `source`.
    fn stage_id(&self) -> &str {
        "source"
    }

    /// One stored pixel after the declared transfer decoding.
    fn value(&self, x: u32, y: u32) -> [f64; 3] {
        let stored = self.pixels.samples[y as usize * self.pixels.width + x as usize];
        let encoded = self.encoded == Some(true);
        match (self.space, encoded) {
            (Some(space), true) => [
                colorspace::cctf_decode(stored[0], space.cctf),
                colorspace::cctf_decode(stored[1], space.cctf),
                colorspace::cctf_decode(stored[2], space.cctf),
            ],
            _ => stored,
        }
    }

    /// Whether the values this reading stores are transfer-decoded: `true`
    /// once the declared decoding has been applied, otherwise the source's own
    /// declared state, and `None` when the source declares none.
    fn reading_cctf_decoding(&self) -> Option<bool> {
        if self.decoded {
            return Some(true);
        }
        self.encoded.map(|encoded| !encoded)
    }
}

/// One declared rectangle's reported value.
struct Reading {
    /// The arithmetic mean per channel over the rectangle.
    value: [f64; 3],
}

impl Reading {
    fn is_valid(&self) -> bool {
        self.value.iter().all(|value| value.is_finite())
    }
}

pub fn sample_with_result(args: &MeasureSampleArgs) -> Result<(Value, i32)> {
    let plan = plan::load(&args.regions).map_err(anyhow::Error::msg)?;
    let source = select_source(args)?;
    // Read every rectangle before writing anything: a measurement writes no
    // artifact unless it completes.
    let readings = read_readings(&plan, &source)?;
    write_report(args, &plan, &source, &readings)
}

/// The one declared source of this measurement. Two authorities for one
/// declaration is an error.
fn select_source(args: &MeasureSampleArgs) -> Result<Source> {
    match &args.source {
        None => bail!(
            "a source is required: pass --source IMAGE with --input-color-space and \
             --input-cctf-decoding"
        ),
        Some(path) => image_source(args, path),
    }
}

/// A raster image source carrying the input declaration `process` requires.
fn image_source(args: &MeasureSampleArgs, path: &Path) -> Result<Source> {
    let color_space = args
        .input_color_space
        .as_deref()
        .ok_or_else(|| anyhow!("--source requires --input-color-space, as process does"))?;
    let cctf_decoding = args
        .input_cctf_decoding
        .ok_or_else(|| anyhow!("--source requires --input-cctf-decoding, as process does"))?;
    // An unknown declared space is an error naming it, never a pass-through.
    let space = colorspace::resolve(color_space)
        .map_err(|e| anyhow!("unsupported input color space {color_space:?}: {e}"))?;
    let loaded = image_data::decode_raster(path, color_space, cctf_decoding)
        .map_err(|e| anyhow!("loading image {}: {e}", path.display()))?;
    let sha256 = sha256_file(path)?;
    Ok(Source {
        kind: "image",
        path: path.to_path_buf(),
        sha256,
        pixels: loaded,
        color_space: Some(color_space.to_owned()),
        space: Some(space),
        // The flag carries `process`'s own meaning: true declares that the
        // stored values are transfer-encoded, so the reading decodes them.
        encoded: Some(cctf_decoding),
        // A raster carries no stage vocabulary of its own; its reading lives
        // in the declared source raster's coordinate space.
        decoded: cctf_decoding,
        coordinate_space: "source_pixels".to_owned(),
        domain: "source_rgb".to_owned(),
        units: "relative".to_owned(),
        channel_order: "RGB".to_owned(),
    })
}

/// Read every declared rectangle as the arithmetic mean per channel. A window
/// that leaves the source bounds is an error naming the sample and the
/// bounds; it is never clamped, padded, extrapolated, or repaired.
fn read_readings(plan: &Plan, source: &Source) -> Result<Vec<Reading>> {
    let [width, height] = source.pixels.dimensions();
    let (width, height) = (width as u64, height as u64);
    let expected_space = source.coordinate_space.as_str();
    for sample in &plan.samples {
        let compatible = match sample.space.as_str() {
            "source" => expected_space == "source_pixels",
            "render" => expected_space == "output_pixels",
            native => native == expected_space,
        };
        if !compatible {
            bail!(
                "region {:?} coordinate space {:?} is incompatible with source coordinate space {:?}",
                sample.id,
                sample.space,
                expected_space
            );
        }
    }
    let mut readings = Vec::with_capacity(plan.samples.len());
    for sample in &plan.samples {
        if u64::from(sample.x) + u64::from(sample.width) > width
            || u64::from(sample.y) + u64::from(sample.height) > height
        {
            bail!(
                "sample {:?} window (x {}, y {}, width {}, height {}) leaves the source \
                 bounds {width}x{height}",
                sample.id,
                sample.x,
                sample.y,
                sample.width,
                sample.height
            );
        }
        let mut sums = [0.0_f64; 3];
        for row in sample.y..sample.y + sample.height {
            for column in sample.x..sample.x + sample.width {
                let value = source.value(column, row);
                for channel in 0..3 {
                    sums[channel] += value[channel];
                }
            }
        }
        let pixels = u64::from(sample.width) * u64::from(sample.height);
        readings.push(Reading {
            value: [
                sums[0] / pixels as f64,
                sums[1] / pixels as f64,
                sums[2] / pixels as f64,
            ],
        });
    }
    Ok(readings)
}

/// Every (source, partition) the plan declares, in a stable order.
fn group_keys(plan: &Plan) -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = Vec::new();
    for sample in &plan.samples {
        let key = (sample.source.clone(), sample.partition.clone());
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.sort();
    keys
}

/// One group entry per (source, partition) with its sample count.
fn group_records(plan: &Plan) -> Vec<Value> {
    group_keys(plan)
        .into_iter()
        .map(|(source, partition)| {
            let samples = plan
                .samples
                .iter()
                .filter(|sample| sample.source == source && sample.partition == partition)
                .count();
            json!({"source": source, "partition": partition, "samples": samples})
        })
        .collect()
}

/// One summary per (source, partition, channel) over the finite values.
fn summary_records(plan: &Plan, source: &Source, readings: &[Reading]) -> Vec<Value> {
    let stage = source.stage_id();
    let mut records = Vec::new();
    for (group_source, partition) in group_keys(plan) {
        for channel in 0..3 {
            let stats = statistics(
                plan.samples
                    .iter()
                    .zip(readings)
                    .filter(|(sample, _)| {
                        sample.source == group_source && sample.partition == partition
                    })
                    .map(|(_, reading)| reading.value[channel]),
            );
            records.push(json!({
                "source": group_source,
                "partition": partition,
                "stage": stage,
                "channel": channel,
                "count": stats.count,
                "mean": number_or_null(stats.mean),
                "median": number_or_null(stats.median),
                "p95": number_or_null(stats.p95),
                    "min": number_or_null(stats.min),
                    "max": number_or_null(stats.max),
            }));
        }
    }
    records
}

/// A summary statistic, or `null` when it has no finite value to report.
fn number_or_null(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        Value::Null
    }
}

/// The plan record the measurement carries.
fn plan_record(plan: &Plan) -> Value {
    json!({
        "kind": "regions",
        "path": plan.manifest_path.display().to_string(),
        "sha256": plan.sha256,
        "id": plan.id,
        "name": plan.name,
        "regions": plan
            .samples
            .iter()
            .map(|sample| json!({
                "id": sample.id,
                "label": sample.label,
                "source": sample.source,
                "partition": sample.partition,
                "role": sample.role,
                "geometry": {
                    "x": sample.x,
                    "y": sample.y,
                    "width": sample.width,
                    "height": sample.height,
                    "space": sample.space,
                },
            }))
            .collect::<Vec<Value>>(),
    })
}

/// The single stage record of this measurement: the source's vocabulary,
/// including its producer-declared coordinate space.
fn stage_record(source: &Source) -> Value {
    json!({
        "id": source.stage_id(),
        "domain": source.domain,
        "units": source.units,
        "channel_order": source.channel_order,
        "coordinate_space": source.coordinate_space,
        "color_space": source.color_space,
        "cctf_decoding": source.reading_cctf_decoding(),
        "sha256": source.sha256,
        "availability": "available",
        "reason": Value::Null,
    })
}

/// One row per sample; a non-finite channel writes an empty field and marks
/// the sample invalid.
fn build_points_csv(plan: &Plan, source: &Source, readings: &[Reading]) -> String {
    let mut out = String::from(
        "sample_id,source,partition,stage,domain,units,channel_order,c0,c1,c2,valid\n",
    );
    for (sample, reading) in plan.samples.iter().zip(readings) {
        let mut fields = vec![
            csv_field(&sample.id),
            csv_field(&sample.source),
            csv_field(&sample.partition),
            csv_field(source.stage_id()),
            csv_field(&source.domain),
            csv_field(&source.units),
            csv_field(&source.channel_order),
        ];
        for value in reading.value {
            fields.push(number_field(value));
        }
        fields.push(if reading.is_valid() { "true" } else { "false" }.to_owned());
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}

fn write_report(
    args: &MeasureSampleArgs,
    plan: &Plan,
    source: &Source,
    readings: &[Reading],
) -> Result<(Value, i32)> {
    let measure_json = json!({
        "schema_version": 1,
        "kind": "sample",
        // A sample reads a declared source; it never executes a chain.
        "executed": false,
        "source": {
            "kind": source.kind,
            "path": source.path.display().to_string(),
            "sha256": source.sha256,
            "dimensions": source.pixels.dimensions(),
            "coordinate_space": source.coordinate_space,
            "color_space": source.color_space,
            "stage": "source",
            // The source block reports the source's own declaration verbatim.
            "cctf_decoding": source.encoded.map(|encoded| !encoded),
        },
        "regions": plan_record(plan),
        "stages": [stage_record(source)],
        "groups": group_records(plan),
        "summaries": summary_records(plan, source, readings),
    });
    let measure_text = format!("{}\n", serde_json::to_string_pretty(&measure_json)?);
    let points_text = build_points_csv(plan, source, readings);
    write_measure_named(
        &args.out,
        &[("run.json", measure_text), ("readings.csv", points_text)],
    )
    .map_err(anyhow::Error::msg)?;

    eprintln!(
        "Source {} ({}): {} samples → {}",
        source.path.display(),
        source.kind,
        plan.samples.len(),
        args.out.display()
    );
    let invalid: Vec<&str> = plan
        .samples
        .iter()
        .zip(readings)
        .filter(|(_, reading)| !reading.is_valid())
        .map(|(sample, _)| sample.id.as_str())
        .collect();
    if !invalid.is_empty() {
        eprintln!("Invalid samples: {}", invalid.join(", "));
    }
    Ok((
        json!({
            "schema_version": 1,
            "command": "sample",
            "execution_status": "completed",
            "artifact": args.out.display().to_string(),
        }),
        0,
    ))
}
