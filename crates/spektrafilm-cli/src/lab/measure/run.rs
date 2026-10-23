//! `measure run`: readings from the real chain on a declared stimulus set.
//!
//! The command resolves the chain with the shared resolver, executes the
//! core-owned pipeline path with the declared tap entry, and reads the
//! executed run's own stage capture. It reports values; it never judges them.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use spektrafilm_backend::{ComputeBackend, cpu_backend::CpuBackend};
use spektrafilm_core::runtime::Runtime;
use spektrafilm_core::stage_evidence::{self, StageCapture};
use spektrafilm_core::{spectral_eval, spectral_service};
use spektrafilm_math::image::ImageBuf;

use crate::select_data_dir;

use super::super::SCENE_RGB_SPACE;
use super::super::cli::{MeasureEntry, MeasureRunArgs, MeasureStage};
use super::super::report;
use super::stimuli::{self, Stimuli};
use super::{
    InputRequest, Offender, ResolvedChain, StageAvailability, StageReading, csv_field,
    last_read_stage, number_field, point_wise_offenders, read_stage_bundle, resolve_chain,
    resolve_range, statistics,
};

pub fn run(args: &MeasureRunArgs, matches: &clap::ArgMatches) -> Result<()> {
    run_with_result(args, matches).map(|_| ())
}

/// Execute a chain measurement and return the command result object.
///
/// The result carries `schema_version`, `command`, `execution_status`, and the
/// `artifact` directory the measurement published.
pub fn run_with_result(args: &MeasureRunArgs, matches: &clap::ArgMatches) -> Result<(Value, i32)> {
    // The data directory resolves exactly as it does on the user surface:
    // argument, then `SPEKTRAFILM_DATA_DIR`, then discovery.
    let data_dir = select_data_dir(
        args.chain
            .data_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("data")),
        matches,
    )?
    .path;

    // Load the stimulus first: a measurement of nothing is an error, not an
    // empty run, and the stimulus is declared independently of the chain.
    let stimulus = stimuli::load(&args.stimuli, args.enter).map_err(anyhow::Error::msg)?;
    let stimulus_record = stimulus.manifest_record();

    // `spectra` needs the reference illuminant to photograph the samples and
    // the film profile to validate that illuminant, so it resolves first.
    let mut target_illuminant: Option<String> = None;
    if args.enter == MeasureEntry::Spectra {
        // The illuminant pair is declared in the stimulus document: the
        // document is its single authority, not a command option.
        let Stimuli::Spectra(set) = &stimulus else {
            unreachable!("the spectra entry loads a spectra set");
        };
        let reference = set
            .condition_reference
            .as_deref()
            .unwrap_or_default()
            .to_owned();
        let target = set
            .condition_target
            .as_deref()
            .unwrap_or_default()
            .to_owned();
        let chain = resolve_spectra_chain(args, &data_dir)?;
        let film = &chain.film;
        if film.info.reference_illuminant != reference {
            bail!(
                "stimulus condition reference {reference:?} does not match the profile reference \
                 illuminant {:?}; the spectral LUT is built for the profile's own reference",
                film.info.reference_illuminant
            );
        }
        for name in [&reference, &target] {
            if !spectral_service::is_supported_illuminant(name) {
                bail!(
                    "unsupported illuminant {name:?}; supported names include {}",
                    spectral_service::available_illuminants().join(", ")
                );
            }
        }
        if set.wavelengths != film.data.wavelengths {
            bail!(
                "spectra grid does not match the profile wavelength grid ({} samples vs {})",
                set.wavelengths.len(),
                film.data.wavelengths.len()
            );
        }
        target_illuminant = Some(target);
    }

    let mut chain = resolve_measurement_chain(args, &data_dir, &stimulus)?;
    let read = read_stage_ids(&args.read);

    // The measured range is declared through the same `taps` controls the user
    // binaries read: `--enter` becomes the resolved `taps.inject` and the
    // collection boundary reaches every requested reading, so the retained
    // effective document describes what actually executes.
    let (entry_tap, collect) = resolve_range(&mut chain.params, args.enter, &read)?;
    // The measurement's own seed reaches the runtime the way the user surface
    // sets it, so a stochastic stage reproduces from the recorded seed.
    chain.params.random_seed = u64::from(args.chain.seed);

    // The point-wise requirement is evaluated before executing anything: an
    // injected numeric sample has no neighborhood, so a spatial or stochastic
    // stage that executes inside the measured range would describe a different
    // signal.
    let last_read = last_read_stage(&read)?;
    let offenders = point_wise_offenders(&chain.params, args.enter, last_read);
    if !offenders.is_empty() {
        bail!("{}", offender_report(&offenders));
    }
    let identity = super::chain_identity(
        &chain,
        &data_dir,
        args.enter.name(),
        &read,
        args.chain.seed,
        stimulus_record.clone(),
    )?;

    if args.dry_run {
        return write_dry_run(args, &chain, &data_dir, &read, stimulus, identity);
    }

    let out = args.out.clone();
    precheck_destination(&out)?;

    // Assemble the interleaved sample-major stimulus: one row, one sample per
    // pixel, so the pipeline reads every sample as an independent point.
    let sample_count = stimulus.sample_count();
    let mut pixels: Vec<f64> = Vec::with_capacity(sample_count * 3);
    let invalid_samples = match &stimulus {
        Stimuli::Spectra(set) => {
            let illuminant =
                spectral_service::select_illuminant_f64(target_illuminant.as_deref().unwrap());
            let mut invalid = 0_usize;
            for index in 0..sample_count {
                let rgb = spectral_eval::reflectance_to_rgb_f64(
                    set.row(index),
                    illuminant.as_ref(),
                    SCENE_RGB_SPACE,
                )
                .map_err(anyhow::Error::msg)?;
                pixels.extend_from_slice(&rgb);
                if !set.band_valid(index) {
                    invalid += 1;
                }
            }
            invalid
        }
        Stimuli::Points(set) => {
            let mut invalid = 0_usize;
            for index in 0..sample_count {
                pixels.extend_from_slice(&set.row(index));
                if !set.row_valid(index) {
                    invalid += 1;
                }
            }
            invalid
        }
    };
    let image = ImageBuf::from_data(sample_count as u32, 1, pixels);

    // The runs record their declaration before the runtime takes ownership of
    // the resolved profiles.
    let declaration = declaration_record(args, &chain, &data_dir, &read)?;
    let command = replay_record(args, &declaration, &identity)?;

    stage_evidence::ensure_supported_params(&chain.params).map_err(anyhow::Error::msg)?;

    // The reading is the f64 reference route: stage evidence capture is a CPU
    // reference route, and a GPU backend would not be the recorded reading.
    let backend: Box<dyn ComputeBackend> = Box::new(CpuBackend);
    stage_evidence::ensure_supported_backend(backend.as_ref()).map_err(anyhow::Error::msg)?;

    let runtime = Runtime::new(chain.film, chain.print, chain.params.clone(), &data_dir)
        .map_err(anyhow::Error::msg)
        .context("constructing the measurement runtime")?;

    // Stage evidence is captured under a staging name and only named once every
    // check has passed: a measurement that fails leaves no artifact behind, so
    // the same destination can be measured again.
    let staging = staging_directory(&out);
    let outcome = (|| -> Result<Vec<Value>> {
        let mut capture = StageCapture::new(&staging);
        // The command writes no image output: the returned buffer is
        // intentionally unread; only the captured stage boundaries are the
        // reading.
        runtime
            .process_captured(
                image,
                backend.as_ref(),
                Some(entry_tap),
                Some(collect),
                &spektrafilm_backend::telemetry::ObservationContext::default(),
                &mut capture,
            )
            .map_err(anyhow::Error::msg)
            .context("executing the measurement chain")?;
        capture.publish().map_err(anyhow::Error::msg)?;

        let (readings, availability) =
            read_stage_bundle(&staging, &read).map_err(anyhow::Error::msg)?;
        for reading in &readings {
            if reading.width != sample_count as u32 || reading.height != 1 {
                bail!(
                    "stage {} measured {}x{} but the stimulus declares {} samples",
                    reading.id,
                    reading.width,
                    reading.height,
                    sample_count
                );
            }
        }

        let stages = stage_records(&readings, &availability, &read);
        let groups = group_records(&stimulus);
        let summaries = summary_records(&stimulus, &readings, &read);
        let points_csv = build_points_csv(&stimulus, &readings, &read);
        let measure_json = json!({
            "schema_version": 1,
            "kind": "run",
            "executed": true,
            "declaration": declaration,
            "command": command,
            "executor": {
                "backend": backend.name(),
                "precision": "f64",
            },
            "observer": {
                "kind": "native_stage_capture",
                "schema_version": 1,
                "directory": "evidence",
                "backend": "cpu",
                "host_precision": "f64",
                "buffer_dtype": "f64",
                "inject": entry_tap.name(),
                "collect": collect.name(),
                "stages": stage_evidence::StageId::ORDER.iter().map(|stage| stage.id()).collect::<Vec<_>>(),
            },
            "stimulus": stimulus_record,
            "identity": identity,
            "stages": stages,
            "groups": groups,
            "summaries": summaries,
        });
        let measure_text = format!("{}\n", serde_json::to_string_pretty(&measure_json)?);
        report::publish_staged(
            &out,
            &staging,
            &[("run.json", measure_text), ("readings.csv", points_csv)],
        )
        .map_err(anyhow::Error::msg)?;
        Ok(stages)
    })();
    if outcome.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    let stages = outcome?;

    eprintln!("Measurement: {}", out.display());
    eprintln!("Samples: {} ({} invalid)", sample_count, invalid_samples);
    eprintln!("Entry: {}", entry_tap.name());
    for stage in &stages {
        let id = stage.get("id").and_then(Value::as_str).unwrap_or_default();
        let availability = stage
            .get("availability")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match stage.get("reason").and_then(Value::as_str) {
            Some(reason) => eprintln!("{id}: {availability} ({reason})"),
            None => eprintln!("{id}: {availability}"),
        }
    }
    Ok((
        json!({
            "schema_version": 1,
            "command": "run",
            "execution_status": "completed",
            "artifact": out.display().to_string(),
        }),
        0,
    ))
}

/// The capture staging directory: a hidden sibling of the destination, so the
/// published `evidence` is a rename inside one directory.
fn staging_directory(out: &Path) -> PathBuf {
    let name = out
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "measurement".to_owned());
    out.with_file_name(format!(".{name}.staging-{}", std::process::id()))
}

/// Resolve the chain for the reference-illuminant check, which needs the film
/// profile before the declaration is known.
fn resolve_spectra_chain(args: &MeasureRunArgs, data_dir: &Path) -> Result<ResolvedChain> {
    resolve_chain(
        &args.chain,
        data_dir,
        InputRequest {
            color_space: None,
            cctf_decoding: false,
            require: false,
        },
    )
}

/// Resolve the measurement chain with the entry's input declaration, then
/// require the declaration that survived resolution to be the one the stimulus
/// declares. A `--set io.input_color_space=…` edit that changed the space would
/// otherwise silently measure a different space.
fn resolve_measurement_chain(
    args: &MeasureRunArgs,
    data_dir: &Path,
    stimulus: &Stimuli,
) -> Result<ResolvedChain> {
    let request = match (args.enter, stimulus) {
        (MeasureEntry::Spectra, Stimuli::Spectra(set)) => InputRequest {
            color_space: Some(SCENE_RGB_SPACE),
            cctf_decoding: set.cctf_decoding,
            require: true,
        },
        (MeasureEntry::RgbIn, Stimuli::Points(set)) => InputRequest {
            color_space: set.color_space.as_deref(),
            cctf_decoding: set.cctf_decoding(),
            require: true,
        },
        (MeasureEntry::RgbIn, Stimuli::Spectra(_)) => {
            bail!("--enter rgb_in reads a points manifest, not a measured-spectra set")
        }
        (MeasureEntry::Spectra, Stimuli::Points(_)) => {
            bail!("--enter spectra reads a measured-spectra manifest, not a points manifest")
        }
        (MeasureEntry::LogEFilm | MeasureEntry::CmyFilm, Stimuli::Points(_)) => InputRequest {
            color_space: None,
            cctf_decoding: false,
            require: false,
        },
        (MeasureEntry::LogEFilm | MeasureEntry::CmyFilm, Stimuli::Spectra(_)) => {
            bail!(
                "--enter {} reads a points manifest, not a measured-spectra set",
                args.enter.name()
            )
        }
    };
    let chain = resolve_chain(&args.chain, data_dir, request)?;
    match args.enter {
        MeasureEntry::Spectra => {
            let (color_space, cctf_decoding) = match stimulus {
                Stimuli::Spectra(set) => (Some(SCENE_RGB_SPACE), set.cctf_decoding),
                Stimuli::Points(_) => (None, false),
            };
            require_declaration(&chain, color_space, cctf_decoding)?;
        }
        MeasureEntry::RgbIn => {
            let (color_space, cctf_decoding) = match stimulus {
                Stimuli::Points(set) => (set.color_space.as_deref(), set.cctf_decoding()),
                Stimuli::Spectra(_) => (None, false),
            };
            require_declaration(&chain, color_space, cctf_decoding)?;
        }
        MeasureEntry::LogEFilm | MeasureEntry::CmyFilm => {
            // The values are already film-domain numbers; a transfer-encoded
            // declaration would misdescribe them.
            if chain.declaration.cctf_decoding {
                bail!(
                    "the resolved chain declares io.input_cctf_decoding=true, but the {} \
                     stimulus values are already film-domain numbers; pass \
                     --set io.input_cctf_decoding=false",
                    args.enter.name()
                );
            }
        }
    }
    Ok(chain)
}

/// Fail when a `--set` edit moved the declaration away from the one the
/// stimulus declares, naming the edit that did it.
fn require_declaration(
    chain: &ResolvedChain,
    color_space: Option<&str>,
    cctf_decoding: bool,
) -> Result<()> {
    let declaration = &chain.declaration;
    let color_space_mismatch = match color_space {
        Some(expected) => declaration.color_space != expected,
        None => false,
    };
    if !color_space_mismatch && declaration.cctf_decoding == cctf_decoding {
        return Ok(());
    }
    let space_edit = chain
        .requested_edit("io.input_color_space")
        .and_then(Value::as_str)
        .map(|requested| format!("--set io.input_color_space={requested}"))
        .unwrap_or_else(|| "the resolved chain".to_owned());
    let decoding_edit = chain
        .requested_edit("io.input_cctf_decoding")
        .map(|requested| format!("--set io.input_cctf_decoding={requested}"))
        .unwrap_or_else(|| "the resolved chain".to_owned());
    let mut causes = Vec::new();
    if color_space_mismatch {
        causes.push(format!(
            "the chain reads the stimulus in {:?} but the entry declares {expected:?} ({space_edit})",
            declaration.color_space,
            expected = color_space.unwrap_or_default()
        ));
    }
    if declaration.cctf_decoding != cctf_decoding {
        causes.push(format!(
            "the chain declares io.input_cctf_decoding={} but the entry requires {cctf_decoding} \
             ({decoding_edit})",
            declaration.cctf_decoding
        ));
    }
    bail!(
        "the measurement's input declaration was changed: {}; pass the declared values instead",
        causes.join("; ")
    );
}

/// The declaration the caller made, so `measure verify` can rebuild this exact
/// invocation and re-resolve it. It records the requested chain options, not
/// the resolved profile names: a resolved film name beside `--preset` would
/// not reproduce the resolution.
fn declaration_record(
    args: &MeasureRunArgs,
    chain: &ResolvedChain,
    data_dir: &Path,
    read: &[String],
) -> Result<Value> {
    Ok(json!({
        "preset": chain.presets,
        "film": args.chain.film,
        "paper": args.chain.paper,
        "sources": chain.sources,
        "route": args.chain.route,
        "scan_output": args.chain.scan_output,
        "seed": args.chain.seed,
        "data_dir": std::fs::canonicalize(data_dir)?,
        "enter": args.enter.name(),
        "read": read,
        "stimuli": std::fs::canonicalize(&args.stimuli)?,
        "resolved": {
            "film": chain.film_name,
            "print": chain.print_name,
        },
    }))
}

/// Replay the same declaration that `measure verify` re-resolves, retaining
/// the dependency identities already computed by the measurement owner.
fn replay_record(args: &MeasureRunArgs, declaration: &Value, identity: &Value) -> Result<Value> {
    let cwd = std::env::current_dir()?;
    let out = if args.out.is_absolute() {
        args.out.clone()
    } else {
        cwd.join(&args.out)
    };
    let text = |field: &str| declaration[field].as_str().unwrap_or_default().to_owned();
    let mut argv = vec![
        "measure".to_owned(),
        "run".to_owned(),
        "--stimuli".to_owned(),
        text("stimuli"),
        "--enter".to_owned(),
        text("enter"),
        "--out".to_owned(),
        out.display().to_string(),
        "--data-dir".to_owned(),
        text("data_dir"),
    ];
    for field in ["film", "paper", "route", "scan_output"] {
        if let Some(value) = declaration[field].as_str() {
            argv.extend([format!("--{}", field.replace('_', "-")), value.to_owned()]);
        }
    }
    for (field, option) in [
        ("preset", "--preset"),
        ("sources", "--set"),
        ("read", "--read"),
    ] {
        for value in declaration[field].as_array().into_iter().flatten() {
            if let Some(value) = value.as_str() {
                argv.extend([option.to_owned(), value.to_owned()]);
            }
        }
    }
    Ok(json!({
        "argv": argv,
        "cwd": cwd,
        "executable": identity["renderer"],
        "seed": declaration["seed"],
        "dependencies": {
            "parameters": identity["parameters"]["files"],
            "stimulus": identity["stimulus"],
            "data_dir": identity["data_dir"],
            "film_profile": identity["film_profile"],
            "print_profile": identity["print_profile"],
        },
        "observer": {"record": "run.json", "field": "observer"},
    }))
}

/// Every active point-wise offender, one per line, with the exact `--set` edit
/// that disables it.
fn offender_report(offenders: &[Offender]) -> String {
    let mut lines = vec![
        "the resolved chain is not point-wise, so a reading would describe a \
         different signal than the measured one:"
            .to_owned(),
    ];
    for offender in offenders {
        lines.push(format!(
            "{}={}  ->  pass --set {}={}",
            offender.path, offender.value, offender.path, offender.disable
        ));
    }
    lines.join("\n")
}

/// The requested stages, or every stage when none is requested; duplicates
/// collapse, and chain order decides the rest.
fn read_stage_ids(requested: &[MeasureStage]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    if requested.is_empty() {
        for stage in MeasureStage::ALL {
            ids.push(stage.id().to_owned());
        }
        return ids;
    }
    for stage in MeasureStage::ALL {
        let id = stage.id();
        if requested.iter().any(|requested| requested.id() == id)
            && !ids.iter().any(|existing| existing == id)
        {
            ids.push(id.to_owned());
        }
    }
    ids
}

/// One stage record per declared stage: the reading's own fields when it
/// captured, the availability record when it did not.
fn stage_records(
    readings: &[StageReading],
    availability: &[StageAvailability],
    read: &[String],
) -> Vec<Value> {
    let mut records = Vec::with_capacity(read.len());
    for id in read {
        if let Some(reading) = readings.iter().find(|reading| &reading.id == id) {
            records.push(json!({
                "id": reading.id,
                "domain": reading.domain,
                "units": reading.units,
                "channel_order": reading.channel_order,
                "color_space": reading.color_space,
                "cctf_decoding": reading.cctf_decoding,
                "sha256": reading.sha256,
                "availability": "available",
                "reason": Value::Null,
            }));
        } else if let Some(entry) = availability.iter().find(|entry| &entry.id == id) {
            records.push(json!({
                "id": entry.id,
                "domain": Value::Null,
                "units": Value::Null,
                "channel_order": Value::Null,
                "color_space": Value::Null,
                "cctf_decoding": Value::Null,
                "sha256": Value::Null,
                "availability": entry.availability,
                "reason": entry.reason,
            }));
        }
    }
    records
}

/// One group entry per (source, partition) with its sample count.
fn group_records(stimulus: &Stimuli) -> Vec<Value> {
    let mut keys: Vec<(String, String)> = Vec::new();
    for label in stimulus.labels() {
        let key = (label.source.clone(), label.partition.clone());
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.sort();
    keys.into_iter()
        .map(|(source, partition)| {
            let samples = stimulus
                .labels()
                .iter()
                .filter(|label| label.source == source && label.partition == partition)
                .count();
            json!({"source": source, "partition": partition, "samples": samples})
        })
        .collect()
}

/// One summary per (source, partition, stage, channel) over the finite values
/// of that stage's available readings.
fn summary_records(stimulus: &Stimuli, readings: &[StageReading], read: &[String]) -> Vec<Value> {
    let labels = stimulus.labels();
    let mut groups: Vec<(String, String)> = Vec::new();
    for label in labels {
        let key = (label.source.clone(), label.partition.clone());
        if !groups.contains(&key) {
            groups.push(key);
        }
    }
    groups.sort();
    let mut records = Vec::new();
    for (source, partition) in groups {
        let members: Vec<usize> = labels
            .iter()
            .enumerate()
            .filter(|(_, label)| label.source == source && label.partition == partition)
            .map(|(index, _)| index)
            .collect();
        for id in read {
            let Some(reading) = readings.iter().find(|reading| &reading.id == id) else {
                continue;
            };
            for channel in 0..3 {
                let stats = statistics(members.iter().map(|&index| reading.sample(index)[channel]));
                records.push(json!({
                    "source": source,
                    "partition": partition,
                    "stage": id,
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

/// One row per sample per available stage; a non-finite channel marks the
/// sample invalid and writes an empty field.
fn build_points_csv(stimulus: &Stimuli, readings: &[StageReading], read: &[String]) -> String {
    let labels = stimulus.labels();
    let mut out = String::from(
        "sample_id,source,partition,stage,domain,units,channel_order,c0,c1,c2,valid\n",
    );
    for (index, label) in labels.iter().enumerate() {
        for id in read {
            let Some(reading) = readings.iter().find(|reading| &reading.id == id) else {
                continue;
            };
            let values = reading.sample(index);
            let valid = values.iter().all(|value| value.is_finite());
            let mut fields = vec![
                csv_field(&label.id),
                csv_field(&label.source),
                csv_field(&label.partition),
                csv_field(id),
                csv_field(&reading.domain),
                csv_field(&reading.units),
                csv_field(&reading.channel_order),
            ];
            for value in values {
                fields.push(number_field(value));
            }
            fields.push(if valid { "true" } else { "false" }.to_owned());
            out.push_str(&fields.join(","));
            out.push('\n');
        }
    }
    out
}

/// Refuse to run into a destination that already claims a measurement, before
/// any execution: a run writes no artifact unless the measurement completes,
/// and it never replaces one.
fn precheck_destination(out: &Path) -> Result<()> {
    for name in ["run.json", "readings.csv"] {
        let path = out.join(name);
        if path.exists() {
            bail!("refusing to replace an existing {}", path.display());
        }
    }
    let evidence = out.join("evidence");
    if evidence.exists() {
        bail!("refusing to replace an existing {}", evidence.display());
    }
    Ok(())
}

/// Resolve, load, guard, and write the identity record only: no execution, no
/// evidence directory, no `readings.csv`.
fn write_dry_run(
    args: &MeasureRunArgs,
    chain: &ResolvedChain,
    data_dir: &Path,
    read: &[String],
    stimulus: Stimuli,
    identity: Value,
) -> Result<(Value, i32)> {
    let out = args.out.clone();
    precheck_destination(&out)?;
    let stages: Vec<Value> = read
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "domain": Value::Null,
                "units": Value::Null,
                "channel_order": Value::Null,
                "color_space": Value::Null,
                "cctf_decoding": Value::Null,
                "sha256": Value::Null,
                "availability": "not_executed",
                "reason": "dry_run",
            })
        })
        .collect();
    let measure_json = json!({
        "schema_version": 1,
        "kind": "run",
        "executed": false,
        "declaration": declaration_record(args, chain, data_dir, read)?,
        "stimulus": stimulus.manifest_record(),
        "identity": identity,
        "stages": stages,
        "groups": group_records(&stimulus),
        "summaries": [],
    });
    let measure_text = format!("{}\n", serde_json::to_string_pretty(&measure_json)?);
    report::write_measure_named(&out, &[("run.json", measure_text)]).map_err(anyhow::Error::msg)?;
    eprintln!("Measurement (dry run): {}", out.display());
    eprintln!("Samples: {}", stimulus.sample_count());
    eprintln!("Entry: {}", args.enter.name());
    Ok((
        json!({
            "schema_version": 1,
            "command": "run",
            "execution_status": "completed",
            "artifact": out.display().to_string(),
        }),
        0,
    ))
}
