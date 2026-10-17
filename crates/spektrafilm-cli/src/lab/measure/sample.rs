//! `measure sample`: readings of a declared source at declared positions.
//!
//! The source is either a raster image carrying the input declaration
//! `process` requires, or one stage of a retained stage-evidence bundle whose
//! own manifest declares the reading. The command reads the declared
//! rectangles and reports their means; it never resizes, converts beyond the
//! declared transfer decoding, clamps, or repairs anything.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use spektrafilm_core::image_io;
use spektrafilm_math::colorspace::{self, RgbColorSpace};
use spektrafilm_math::image::ImageBuf;

use super::super::cli::MeasureSampleArgs;
use super::super::report;
use super::plan::{self, Plan};
use super::{StageReading, csv_field, number_field, read_stage_bundle, sha256_file, statistics};

/// The pixels a measurement reads, and the authority that declares them.
enum Pixels {
    /// A raster image loaded with the loader `process` uses.
    Image(ImageBuf),
    /// One stage buffer read back from a published evidence bundle.
    Evidence(StageReading),
}

impl Pixels {
    fn dimensions(&self) -> [u32; 2] {
        match self {
            Pixels::Image(image) => [image.width, image.height],
            Pixels::Evidence(reading) => [reading.width, reading.height],
        }
    }

    /// One stored pixel, before the declared transfer decoding.
    fn pixel(&self, x: u32, y: u32) -> [f64; 3] {
        match self {
            Pixels::Image(image) => {
                let base = (y as usize * image.width as usize + x as usize) * 3;
                [image.data[base], image.data[base + 1], image.data[base + 2]]
            }
            Pixels::Evidence(reading) => {
                let base = (y as usize * reading.width as usize + x as usize) * 3;
                [
                    reading.values[base],
                    reading.values[base + 1],
                    reading.values[base + 2],
                ]
            }
        }
    }
}

/// The one declared source of a measurement, and the declaration that
/// governs its reading.
struct Source {
    /// `image` or `evidence`.
    kind: &'static str,
    /// The declared image path or bundle directory.
    path: PathBuf,
    /// The image file's content hash, or the stage buffer's hash.
    sha256: String,
    /// The pixels the rectangles are read from.
    pixels: Pixels,
    /// The declared color space of the values, when the source declares one.
    color_space: Option<String>,
    /// The resolved declared space, when the source declares one.
    space: Option<&'static RgbColorSpace>,
    /// Whether the stored values are transfer-encoded; `None` when the
    /// source declares none, so the record does not invent one.
    encoded: Option<bool>,
    /// The stage id, for an evidence source.
    stage: Option<String>,
    /// Whether the reading applies the declared transfer decoding, so the
    /// values it stores are the linear boundary values.
    decoded: bool,
    /// The reading's vocabulary, recorded verbatim from its own authority.
    domain: String,
    units: String,
    channel_order: String,
}

impl Source {
    /// The stage identity this measurement reports: a retained stage carries
    /// its own id, a raster image is its own single reading.
    fn stage_id(&self) -> &str {
        self.stage.as_deref().unwrap_or("source")
    }

    /// One stored pixel after the declared transfer decoding.
    fn value(&self, x: u32, y: u32) -> [f64; 3] {
        let stored = self.pixels.pixel(x, y);
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

pub fn sample(args: &MeasureSampleArgs) -> Result<()> {
    sample_with_result(args).map(|_| ())
}

/// Read the declared samples and return the command result object.
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
    match (&args.source, &args.from_evidence) {
        (None, None) => bail!(
            "a source is required: pass --source IMAGE with --input-color-space and \
             --input-cctf-decoding, or --from-evidence DIRECTORY with --stage STAGE"
        ),
        (Some(_), Some(_)) => bail!(
            "--source conflicts with --from-evidence: a measurement reads one source, \
             either an image (--source IMAGE) or a stage bundle (--from-evidence DIRECTORY)"
        ),
        (Some(path), None) => image_source(args, path),
        (None, Some(directory)) => evidence_source(args, directory),
    }
}

/// A raster image source carrying the input declaration `process` requires.
fn image_source(args: &MeasureSampleArgs, path: &Path) -> Result<Source> {
    if args.stage.is_some() {
        bail!("--stage reads a stage bundle; an image source declares its own reading");
    }
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
    let loaded =
        image_io::load(path).map_err(|e| anyhow!("loading image {}: {e}", path.display()))?;
    let sha256 = sha256_file(path)?;
    Ok(Source {
        kind: "image",
        path: path.to_path_buf(),
        sha256,
        pixels: Pixels::Image(loaded.image),
        color_space: Some(color_space.to_owned()),
        space: Some(space),
        // The flag carries `process`'s own meaning: true declares that the
        // stored values are transfer-encoded, so the reading decodes them.
        encoded: Some(cctf_decoding),
        stage: None,
        // A raster carries no stage vocabulary of its own; its reading lives
        // in the declared source raster's coordinate space.
        domain: "source_rgb".to_owned(),
        units: "relative".to_owned(),
        channel_order: "RGB".to_owned(),
        decoded: cctf_decoding,
    })
}

/// One stage of a retained evidence bundle, whose manifest governs the
/// reading: its coordinate space, channel order, units, and color.
fn evidence_source(args: &MeasureSampleArgs, directory: &Path) -> Result<Source> {
    let stage = args
        .stage
        .ok_or_else(|| anyhow!("--from-evidence requires --stage STAGE"))?;
    if args.input_color_space.is_some() {
        bail!(
            "--from-evidence declares the reading from the bundle manifest; \
             --input-color-space would be a second authority for the same declaration"
        );
    }
    if args.input_cctf_decoding.is_some() {
        bail!(
            "--from-evidence declares the reading from the bundle manifest; \
             --input-cctf-decoding would be a second authority for the same declaration"
        );
    }
    let id = stage.id().to_owned();
    let (mut readings, availability) = read_stage_bundle(directory, std::slice::from_ref(&id))
        .map_err(|e| anyhow!("reading stage evidence in {}: {e}", directory.display()))?;
    let reading = match readings.len() {
        0 => {
            let entry = availability.iter().find(|entry| entry.id == id);
            let availability = entry
                .map(|entry| entry.availability.as_str())
                .unwrap_or("unknown");
            let reason = entry
                .and_then(|entry| entry.reason.clone())
                .unwrap_or_else(|| "not_present".to_owned());
            bail!(
                "stage {id} of {} is not available ({availability}): {reason}",
                directory.display()
            );
        }
        // Exactly one available reading is required; a second would be a
        // second authority for the same reading.
        1 => readings.pop().ok_or_else(|| {
            anyhow!(
                "stage {id} of {} declares no readable buffer",
                directory.display()
            )
        })?,
        _ => bail!(
            "stage {id} of {} declares more than one available buffer",
            directory.display()
        ),
    };
    let encoded = reading.cctf_decoding.map(|decoding| !decoding);
    // The manifest's own color space governs the decoding; a stage that
    // declares neither has no decoding to apply.
    let space = match reading.color_space.as_deref() {
        Some(name) => Some(
            colorspace::resolve(name)
                .map_err(|e| anyhow!("stage {id} color space {name:?}: {e}"))?,
        ),
        None => {
            if encoded == Some(true) {
                bail!(
                    "stage {id} of {} declares encoded transfer without a color space",
                    directory.display()
                );
            }
            None
        }
    };
    let sha256 = reading.sha256.clone();
    let color_space = reading.color_space.clone();
    let stage_id = reading.id.clone();
    let domain = reading.domain.clone();
    let units = reading.units.clone();
    let channel_order = reading.channel_order.clone();
    Ok(Source {
        kind: "evidence",
        path: directory.to_path_buf(),
        sha256,
        pixels: Pixels::Evidence(reading),
        color_space,
        space,
        encoded,
        stage: Some(stage_id),
        decoded: encoded == Some(true),
        domain,
        units,
        channel_order,
    })
}

/// Read every declared rectangle as the arithmetic mean per channel. A window
/// that leaves the source bounds is an error naming the sample and the
/// bounds; it is never clamped, padded, extrapolated, or repaired.
fn read_readings(plan: &Plan, source: &Source) -> Result<Vec<Reading>> {
    let [width, height] = source.pixels.dimensions();
    let mut readings = Vec::with_capacity(plan.samples.len());
    for sample in &plan.samples {
        if u64::from(sample.x) + u64::from(sample.width) > u64::from(width)
            || u64::from(sample.y) + u64::from(sample.height) > u64::from(height)
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
                "measurements": sample.measurements,
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

/// The single stage record of this measurement: the source's own vocabulary,
/// recorded verbatim and never restated from another source. Its
/// `cctf_decoding` describes the values the reading stores, so a decoded
/// reading reports `false` even when the source declared `true`.
fn stage_record(source: &Source) -> Value {
    json!({
        "id": source.stage_id(),
        "domain": source.domain,
        "units": source.units,
        "channel_order": source.channel_order,
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
            "color_space": source.color_space,
            // The source block reports the source's own declaration verbatim.
            "cctf_decoding": source.encoded.map(|encoded| !encoded),
            "stage": source.stage,
        },
        "regions": plan_record(plan),
        "stages": [stage_record(source)],
        "groups": group_records(plan),
        "summaries": summary_records(plan, source, readings),
    });
    let measure_text = format!("{}\n", serde_json::to_string_pretty(&measure_json)?);
    let points_text = build_points_csv(plan, source, readings);
    report::write_measure_named(
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
