//! `measure compare`: deltas between two retained measurement directories.
//!
//! The command reads the records each measurement wrote, joins the readings
//! by sample identity and stage identity, and reports the candidate minus
//! the baseline per sample and per group. It reports values; it never judges
//! them, and it never refuses a pair because the two chains differ.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use spektrafilm_math::colorspace::{self, RgbColorSpace};
use spektrafilm_math::lab;

use super::super::cli::MeasureCompareArgs;
use super::super::report;
use super::{csv_field, number_field, sha256_file, statistics};

/// The comparison a summary describes.
const SIDE_CANDIDATE_MINUS_BASELINE: &str = "candidate_minus_baseline";
const SIDE_CANDIDATE_MINUS_REFERENCE: &str = "candidate_minus_reference";
const SIDE_BASELINE_MINUS_REFERENCE: &str = "baseline_minus_reference";

/// One retained measurement directory: its record, its readings, its hash.
#[derive(Clone)]
struct Side {
    /// The directory the record and the readings were read from.
    path: PathBuf,
    /// `run.json`'s content hash.
    measure_sha256: String,
    /// The parsed `run.json` document.
    document: Value,
    /// Readings keyed by `(sample_id, stage)`.
    rows: BTreeMap<(String, String), Row>,
}

/// One reading row of a retained `readings.csv`.
#[derive(Clone)]
struct Row {
    source: String,
    partition: String,
    domain: String,
    units: String,
    channel_order: String,
    values: [f64; 3],
    valid: bool,
}

/// One joined pair of readings, with the declaration the two sides agree on.
struct Pair {
    sample_id: String,
    stage: String,
    source: String,
    partition: String,
    domain: String,
    units: String,
    channel_order: String,
    /// The color space both sides declare, when they declare the same one.
    color_space: Option<String>,
    /// Whether both sides declare the stored values transfer-decoded, when
    /// they declare the same state.
    cctf_decoding: Option<bool>,
    /// The provenance labels of each side. A label that differs is reported
    /// with the other differences, and the left side's labels group the row.
    right_source: String,
    right_partition: String,
    /// Why the two stages cannot be compared in the native domain, when they
    /// cannot. Two absent declarations agree.
    incomparable: Option<String>,
    left: [f64; 3],
    right: [f64; 3],
    valid: bool,
}

/// The roles of one pairing, for the records it publishes.
///
/// `comparison` is ``{minuend}_minus_{subtrahend}``, and every number of the
/// pairing is `minuend - subtrahend`: the name and the arithmetic can never
/// drift apart.
struct Roles {
    /// The pairing's name, e.g. `candidate_minus_baseline`.
    comparison: &'static str,
    /// The side that is subtracted, e.g. `baseline`.
    subtrahend: &'static str,
    /// The side it is subtracted from, e.g. `candidate`.
    minuend: &'static str,
}

impl Pair {
    /// The channel deltas of the pair: minuend minus subtrahend, the sides
    /// in the order the pairing joined them, or `NaN` where the pair is
    /// invalid.
    fn deltas(&self) -> [f64; 3] {
        if self.valid {
            [
                self.right[0] - self.left[0],
                self.right[1] - self.left[1],
                self.right[2] - self.left[2],
            ]
        } else {
            [f64::NAN; 3]
        }
    }

    /// Whether the pair's values are stored transfer-encoded, when both sides
    /// agree on that state.
    fn encoded(&self) -> bool {
        self.cctf_decoding == Some(false)
    }

    /// The ΔE00 of the pair and whether the conversion had to bound a value,
    /// when the stage can report a distance and both readings are valid.
    ///
    /// The metric domain is the bounded display: the readings convert through
    /// linear sRGB, are encoded into the display transfer, and then meet the
    /// Lab helper that decodes and bounds to the reachable range. A value that
    /// leaves that range loses information, which the record says instead of
    /// hiding.
    fn delta_e00(&self, space: &RgbColorSpace) -> Option<(f64, bool)> {
        if !self.valid {
            return None;
        }
        let matrix = colorspace::conversion_matrix(space, &colorspace::SRGB);
        let encoded = self.encoded();
        let convert = |values: [f64; 3]| {
            colorspace::convert_rgb(values, space, encoded, &colorspace::SRGB, false, &matrix)
        };
        let left_srgb = convert(self.left);
        let right_srgb = convert(self.right);
        let clipped = left_srgb
            .iter()
            .chain(right_srgb.iter())
            .any(|value| !(0.0..=1.0).contains(value));
        let left_lab = lab::srgb_encoded_to_lab(left_srgb.map(colorspace::srgb_encode_f64));
        let right_lab = lab::srgb_encoded_to_lab(right_srgb.map(colorspace::srgb_encode_f64));
        let distance = lab::delta_e_2000(left_lab, right_lab);
        distance.is_finite().then_some((distance, clipped))
    }
}

/// The distance decision for one stage, taken from the stage records both
/// sides declare for it.
enum Distance {
    /// ΔE00 is computable in this space.
    Available { space: &'static RgbColorSpace },
    /// ΔE00 is not computable, with a stable reason.
    Unavailable { reason: String },
}

impl Distance {
    /// Decide whether one pair of stages can report a distance. Each pairing
    /// decides for itself: a decision taken from one pair must never be spent
    /// on another.
    fn decide(left: &Side, right: &Side, roles: &Roles, stage: &str) -> Distance {
        let Some(left_space) = stage_color_space(&left.document, stage) else {
            return Distance::Unavailable {
                reason: "no declared color space".to_owned(),
            };
        };
        let Some(right_space) = stage_color_space(&right.document, stage) else {
            return Distance::Unavailable {
                reason: "no declared color space".to_owned(),
            };
        };
        let left_decoding = stage_transfer(&left.document, stage);
        let right_decoding = stage_transfer(&right.document, stage);
        if left_space != right_space || left_decoding != right_decoding {
            return Distance::Unavailable {
                reason: interpretation_reason(
                    roles,
                    &left_space,
                    &transfer_word(left_decoding),
                    &right_space,
                    &transfer_word(right_decoding),
                ),
            };
        }
        match colorspace::resolve(&left_space) {
            Ok(space) => Distance::Available { space },
            Err(error) => Distance::Unavailable {
                reason: format!("unresolved color space: {left_space} ({error})"),
            },
        }
    }
}

/// Why two sides cannot be compared in one metric, naming both roles.
fn interpretation_reason(
    roles: &Roles,
    left_space: &str,
    left_transfer: &str,
    right_space: &str,
    right_transfer: &str,
) -> String {
    format!(
        "color interpretation differs: {} {left_space}/{left_transfer} vs {} \
         {right_space}/{right_transfer}",
        roles.subtrahend, roles.minuend
    )
}

/// The word for one side's declared transfer state, so a disagreement names
/// both states in the same vocabulary the records carry.
fn transfer_word(decoding: Option<bool>) -> String {
    match decoding {
        Some(true) => "decoded".to_owned(),
        Some(false) => "encoded".to_owned(),
        None => "none".to_owned(),
    }
}

pub fn compare(args: &MeasureCompareArgs) -> Result<()> {
    compare_with_result(args).map(|_| ())
}

/// Compare two retained readings and return the command result object.
pub fn compare_with_result(args: &MeasureCompareArgs) -> Result<(Value, i32)> {
    let baseline = load_side(&args.baseline, "baseline")?;
    let candidate = load_side(&args.candidate, "candidate")?;
    let reference = match &args.reference {
        Some(path) => Some(load_side(path, "reference")?),
        None => None,
    };

    // Each pairing decides its own availability from its own two sides. A
    // reference is a third retained reading with its own role, never a
    // relabelled copy of the reported pair.
    let mut pairings = vec![Pairing::join(
        &baseline,
        &candidate,
        &ROLES_BASELINE_CANDIDATE,
    )?];
    if let Some(reference) = reference.as_ref() {
        pairings.push(Pairing::join(
            reference,
            &candidate,
            &ROLES_CANDIDATE_REFERENCE,
        )?);
        pairings.push(Pairing::join(
            reference,
            &baseline,
            &ROLES_BASELINE_REFERENCE,
        )?);
    }
    let reported = &pairings[0];
    if reported.pairs.is_empty() {
        bail!("the baseline and candidate share no (sample_id, stage) reading");
    }
    if let Some(reference) = reference.as_ref() {
        // A reference that joins nothing is reported, not fatal: its numbers
        // are simply absent, with the reason.
        if pairings[1].pairs.is_empty() || pairings[2].pairs.is_empty() {
            eprintln!(
                "Reference: {} shares no (sample_id, stage) reading with the compared sides",
                reference.path.display()
            );
        }
    }

    let mut summaries = Vec::new();
    let mut delta_e00_groups = Vec::new();
    let mut unavailable = Vec::new();
    for pairing in &pairings {
        summaries.extend(channel_summaries(pairing));
        delta_e00_groups.extend(delta_e00_summaries(pairing));
        unavailable.extend(pairing.unavailable_records());
    }
    unavailable.sort_by_key(|record| {
        (
            record["pair"].as_str().unwrap_or_default().to_owned(),
            record["stage"].as_str().unwrap_or_default().to_owned(),
            record["metric"].as_str().unwrap_or_default().to_owned(),
            record["reason"].as_str().unwrap_or_default().to_owned(),
        )
    });
    unavailable.dedup();

    // A reading comparison is still a `comparison` record: the registry has one
    // comparison kind, and `inspect`, `report`, and `verify` read that kind. The
    // required fields therefore carry what a reading comparison actually
    // decided — the joined samples are its `cases`, the identity and
    // availability reports are its `findings` — so the record validates and its
    // consumers can read it. `protocol`, `selection`, and
    // `condition_differences` describe a suite's declared conditions, which a
    // chain measurement has none of, so they stay empty rather than inventing a
    // suite the two sides never declared.
    //
    // The identity is the digest of everything the document says except its own
    // `id`, which is the convention every generated document follows. A manifest
    // never indexes itself, so `comparison.json` is absent from the artifact
    // index: it is not final until this digest exists. `deltas.csv` is written
    // first so its hash can be recorded.
    let deltas = build_deltas_csv(&pairings);
    report::write_measure_named(&args.out, &[("deltas.csv", deltas)])
        .map_err(anyhow::Error::msg)?;
    let body = json!({
        "schema_version": 1,
        "kind": "comparison",
        "execution_status": "completed",
        // No policy is selected for a reading comparison: it reports deltas and
        // availability and applies no rule.
        "evaluation_status": "not_evaluated",
        "protocol": json!({}),
        "selection": json!({}),
        "condition_differences": [],
        "cases": reading_cases(reported),
        // A finding is a policy decision that needs inspectable evidence
        // (`id`, `case_id`, `region_id`, `metric`, `decision`, `evidence`). A
        // reading comparison selects no policy and decides nothing, so it
        // raises no finding; the identity differences and unavailable
        // measurements it does report are first-class fields below.
        "findings": [],
        "summary": reading_summary(reported, &pairings),
        "artifacts": artifact_index(&args.out),
        "baseline": side_record(&baseline),
        "candidate": side_record(&candidate),
        "reference": match &reference {
            Some(reference) => side_record(reference),
            None => Value::Null,
        },
        "identity_differences": identity_differences(&pairings),
        "unmatched": unmatched_records(&baseline, &candidate, &reference),
        "stages": stage_records(reported),
        "summaries": summaries,
        "delta_e00_summaries": delta_e00_groups,
        "unavailable": unavailable,
    });
    let mut document = body;
    document["id"] = json!(spektrafilm_benchmark::common::digest(&document)?);

    let compare_text = format!("{}\n", serde_json::to_string_pretty(&document)?);
    report::write_measure_named(&args.out, &[("comparison.json", compare_text)])
        .map_err(anyhow::Error::msg)?;

    print_summary(&args.out, reported, &document);
    Ok((
        json!({
            "schema_version": 1,
            "command": "compare",
            "execution_status": "completed",
            "artifact": args.out.display().to_string(),
        }),
        0,
    ))
}

/// One pairing of two sides, with the distance decisions it made for itself.
struct Pairing {
    roles: &'static Roles,
    left: Side,
    right: Side,
    pairs: Vec<Pair>,
    /// One distance decision per stage of this pairing, from this pairing's
    /// own two sides.
    distances: BTreeMap<String, Distance>,
}

/// The three pairings a comparison reports.
static ROLES_BASELINE_CANDIDATE: Roles = Roles {
    comparison: SIDE_CANDIDATE_MINUS_BASELINE,
    subtrahend: "baseline",
    minuend: "candidate",
};
static ROLES_CANDIDATE_REFERENCE: Roles = Roles {
    comparison: SIDE_CANDIDATE_MINUS_REFERENCE,
    subtrahend: "reference",
    minuend: "candidate",
};
static ROLES_BASELINE_REFERENCE: Roles = Roles {
    comparison: SIDE_BASELINE_MINUS_REFERENCE,
    subtrahend: "reference",
    minuend: "baseline",
};

impl Pairing {
    /// Join the two sides a comparison always has.
    fn join(left: &Side, right: &Side, roles: &'static Roles) -> Result<Pairing> {
        let pairs = join_sides(left, right, roles)?;
        let distances = decide_distances(left, right, roles, &pairs);
        Ok(Pairing {
            roles,
            left: left.clone(),
            right: right.clone(),
            pairs,
            distances,
        })
    }

    /// One distance decision per stage, taken from this pairing's own sides.
    fn colorimetric(&self, stage: &str) -> Option<&RgbColorSpace> {
        match self.distances.get(stage) {
            Some(Distance::Available { space }) => Some(space),
            _ => None,
        }
    }

    /// Every metric this pairing could not compute, each with its reason.
    fn unavailable_records(&self) -> Vec<Value> {
        let mut records = Vec::new();
        for (stage, decision) in &self.distances {
            if let Distance::Unavailable { reason } = decision {
                records.push(json!({
                    "pair": self.roles.comparison,
                    "stage": stage,
                    "metric": "delta_e_2000",
                    "reason": reason,
                }));
            }
            // A stage whose interpretation is eligible but whose readings are
            // all invalid says so instead of disappearing: its distance group
            // is empty, and an empty group without a reason would be a missing
            // measurement rather than an unavailable one.
            if self
                .pairs
                .iter()
                .filter(|pair| &pair.stage == stage)
                .all(|pair| !pair.valid)
            {
                records.push(json!({
                    "pair": self.roles.comparison,
                    "stage": stage,
                    "metric": "delta_e_2000",
                    "reason": "all readings invalid",
                }));
            }
        }
        for pair in &self.pairs {
            let Some(reason) = &pair.incomparable else {
                continue;
            };
            records.push(json!({
                "pair": self.roles.comparison,
                "stage": pair.stage,
                "metric": "channel_delta",
                "reason": reason,
            }));
        }
        records
    }
}

/// Read one retained measurement directory. A missing record or reading file
/// is an error naming the directory.
fn load_side(directory: &Path, label: &str) -> Result<Side> {
    let measure = directory.join("run.json");
    let points = directory.join("readings.csv");
    if !measure.is_file() {
        bail!("{label} directory {} has no run.json", directory.display());
    }
    if !points.is_file() {
        bail!(
            "{label} directory {} has no readings.csv",
            directory.display()
        );
    }
    let bytes =
        std::fs::read(&measure).with_context(|| format!("reading {}", measure.display()))?;
    let document: Value =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", measure.display()))?;
    let schema = document
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if schema != 1 {
        bail!(
            "{} declares schema_version {}, expected 1",
            measure.display(),
            schema
        );
    }
    let kind = document
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind != "run" && kind != "sample" {
        bail!(
            "{} declares kind {:?}, expected \"run\" or \"sample\"",
            measure.display(),
            kind
        );
    }
    let rows =
        read_points(&points).map_err(|error| anyhow!("reading {}: {error}", points.display()))?;
    Ok(Side {
        path: directory.to_path_buf(),
        measure_sha256: sha256_file(&measure)?,
        document,
        rows,
    })
}

/// Split one CSV document into records of fields.
///
/// This is the inverse of [`csv_field`], which the measurement writes with:
/// a field may be quoted, a quoted field may carry commas, line breaks, and
/// doubled quotes, and a record ends at an unquoted line break. Anything else
/// is a malformed record and is refused with its line number rather than
/// split into the wrong columns.
fn parse_csv_records(text: &str) -> Result<Vec<(usize, Vec<String>)>, String> {
    let mut records = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut line = 1_usize;
    let mut record_line = 1_usize;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        match (in_quotes, character) {
            (true, '"') => match chars.peek() {
                Some('"') => {
                    chars.next();
                    field.push('"');
                }
                _ => in_quotes = false,
            },
            (true, other) => {
                if other == '\n' {
                    line += 1;
                }
                field.push(other);
            }
            (false, '"') if field.is_empty() => {
                in_quotes = true;
                quoted = true;
            }
            (false, ',') => {
                fields.push(std::mem::take(&mut field));
                quoted = false;
            }
            (false, '\r') => {}
            (false, '\n') => {
                line += 1;
                fields.push(std::mem::take(&mut field));
                quoted = false;
                if !(fields.len() == 1 && fields[0].is_empty()) {
                    records.push((record_line, std::mem::take(&mut fields)));
                }
                record_line = line;
            }
            (false, other) => {
                if quoted {
                    return Err(format!(
                        "line {line} has text after a closing quote; a field must be quoted whole"
                    ));
                }
                field.push(other);
            }
        }
    }
    if in_quotes {
        return Err(format!(
            "line {record_line} opens a quote that never closes"
        ));
    }
    if !field.is_empty() || !fields.is_empty() {
        fields.push(field);
        if !(fields.len() == 1 && fields[0].is_empty()) {
            records.push((record_line, fields));
        }
    }
    Ok(records)
}

/// Parse a retained `readings.csv` into rows keyed by `(sample_id, stage)`.
/// An empty numeric field is a value that is not finite.
fn read_points(path: &Path) -> Result<BTreeMap<(String, String), Row>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut records = parse_csv_records(&text)?.into_iter();
    let (_, header) = records
        .next()
        .ok_or_else(|| format!("{} is empty", path.display()))?;
    let expected = [
        "sample_id",
        "source",
        "partition",
        "stage",
        "domain",
        "units",
        "channel_order",
        "c0",
        "c1",
        "c2",
        "valid",
    ];
    if header.len() != expected.len()
        || header
            .iter()
            .zip(expected)
            .any(|(got, want)| !got.eq_ignore_ascii_case(want))
    {
        return Err(format!(
            "{} has an unexpected header; expected {}",
            path.display(),
            expected.join(",")
        ));
    }
    let column = |name: &str| -> usize {
        header
            .iter()
            .position(|column| column.eq_ignore_ascii_case(name))
            .unwrap_or(usize::MAX)
    };
    let mut rows = BTreeMap::new();
    for (line, fields) in records {
        if fields.len() != header.len() {
            return Err(format!(
                "{} line {line} has {} fields, expected {}",
                path.display(),
                fields.len(),
                header.len()
            ));
        }
        let number = |name: &str| -> Result<f64, String> {
            let text = &fields[column(name)];
            if text.is_empty() {
                return Ok(f64::NAN);
            }
            text.parse::<f64>().map_err(|_| {
                format!(
                    "{} line {line} has a non-numeric {name}: {text}",
                    path.display()
                )
            })
        };
        let values = [number("c0")?, number("c1")?, number("c2")?];
        let valid =
            fields[column("valid")] == "true" && values.iter().all(|value| value.is_finite());
        let row = Row {
            source: fields[column("source")].clone(),
            partition: fields[column("partition")].clone(),
            domain: fields[column("domain")].clone(),
            units: fields[column("units")].clone(),
            channel_order: fields[column("channel_order")].clone(),
            values,
            valid,
        };
        let sample_id = fields[column("sample_id")].clone();
        let stage = fields[column("stage")].clone();
        if rows
            .insert((sample_id.clone(), stage.clone()), row)
            .is_some()
        {
            return Err(format!(
                "{} carries sample {sample_id} stage {stage} more than once",
                path.display()
            ));
        }
    }
    if rows.is_empty() {
        return Err(format!("{} carries no reading row", path.display()));
    }
    Ok(rows)
}

/// Join two sides on `(sample_id, stage)`.
///
/// A shared key whose two readings disagree on the domain, the units, the
/// channel order, or the sample's own labels is an incompatible identity set:
/// the comparison refuses it and names the mismatch instead of silently
/// truncating the join or reading one side's labels as both. A key only one
/// side carries is reported in `unmatched` by its caller.
fn join_sides(left: &Side, right: &Side, roles: &Roles) -> Result<Vec<Pair>> {
    let mut pairs = Vec::new();
    for (key, left_row) in &left.rows {
        let Some(right_row) = right.rows.get(key) else {
            continue;
        };
        for (field, left_value, right_value) in [
            ("domain", &left_row.domain, &right_row.domain),
            ("units", &left_row.units, &right_row.units),
            (
                "channel_order",
                &left_row.channel_order,
                &right_row.channel_order,
            ),
        ] {
            if left_value != right_value {
                bail!(
                    "sample {} stage {} declares {field} {} on the {} side and {} on the {} \
                     side; the two readings do not describe the same quantity",
                    key.0,
                    key.1,
                    left_value,
                    roles.subtrahend,
                    right_value,
                    roles.minuend
                );
            }
        }
        // The declaration a pair is grouped and measured under is the one both
        // sides agree on. A disagreement is not fatal here: the pairing still
        // reports the raw values, and says which metric it could not compute.
        let left_space = stage_color_space(&left.document, &key.1);
        let right_space = stage_color_space(&right.document, &key.1);
        let left_decoding = stage_transfer(&left.document, &key.1);
        let right_decoding = stage_transfer(&right.document, &key.1);
        let agrees = left_space == right_space && left_decoding == right_decoding;
        let color_space = if agrees { left_space.clone() } else { None };
        let cctf_decoding = if agrees { left_decoding } else { None };
        let incomparable = if agrees {
            None
        } else {
            Some(interpretation_reason(
                roles,
                left_space.as_deref().unwrap_or("none"),
                &transfer_word(left_decoding),
                right_space.as_deref().unwrap_or("none"),
                &transfer_word(right_decoding),
            ))
        };
        pairs.push(Pair {
            sample_id: key.0.clone(),
            stage: key.1.clone(),
            source: left_row.source.clone(),
            partition: left_row.partition.clone(),
            domain: left_row.domain.clone(),
            units: left_row.units.clone(),
            channel_order: left_row.channel_order.clone(),
            color_space,
            cctf_decoding,
            right_source: right_row.source.clone(),
            right_partition: right_row.partition.clone(),
            incomparable,
            left: left_row.values,
            right: right_row.values,
            valid: left_row.valid && right_row.valid,
        });
    }
    Ok(pairs)
}

/// One distance decision per stage of a pairing, taken from that pairing's own
/// two sides.
fn decide_distances(
    left: &Side,
    right: &Side,
    roles: &Roles,
    pairs: &[Pair],
) -> BTreeMap<String, Distance> {
    let mut distances: BTreeMap<String, Distance> = BTreeMap::new();
    for pair in pairs {
        if distances.contains_key(&pair.stage) {
            continue;
        }
        distances.insert(
            pair.stage.clone(),
            Distance::decide(left, right, roles, &pair.stage),
        );
    }
    distances
}

/// Every key one side of a pair carries alone, as `unmatched` records.
fn unmatched_of(left: &Side, right: &Side, left_name: &str, right_name: &str) -> Vec<Value> {
    let mut unmatched = Vec::new();
    for key in left.rows.keys() {
        if !right.rows.contains_key(key) {
            unmatched.push(json!({
                "sample_id": key.0,
                "stage": key.1,
                "side": left_name,
            }));
        }
    }
    for key in right.rows.keys() {
        if !left.rows.contains_key(key) {
            unmatched.push(json!({
                "sample_id": key.0,
                "stage": key.1,
                "side": right_name,
            }));
        }
    }
    unmatched
}

/// Every unmatched key of the baseline, candidate, and reference joins, in
/// one list.
fn unmatched_records(baseline: &Side, candidate: &Side, reference: &Option<Side>) -> Vec<Value> {
    let mut unmatched = unmatched_of(baseline, candidate, "baseline", "candidate");
    if let Some(reference) = reference {
        unmatched.extend(unmatched_of(candidate, reference, "candidate", "reference"));
        unmatched.extend(unmatched_of(baseline, reference, "baseline", "reference"));
    }
    unmatched.sort_by_key(|record| {
        (
            record["sample_id"].as_str().unwrap_or_default().to_owned(),
            record["stage"].as_str().unwrap_or_default().to_owned(),
            record["side"].as_str().unwrap_or_default().to_owned(),
        )
    });
    unmatched.dedup_by(|left, right| left == right);
    unmatched
}

/// The color space one side's stage record declares, when it declares one.
fn stage_color_space(document: &Value, stage: &str) -> Option<String> {
    stage_record(document, stage)?
        .get("color_space")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The transfer state one side's stage record declares, when it declares one.
fn stage_transfer(document: &Value, stage: &str) -> Option<bool> {
    stage_record(document, stage)?
        .get("cctf_decoding")
        .and_then(Value::as_bool)
}

/// The stage record of one side's `run.json`.
fn stage_record<'a>(document: &'a Value, stage: &str) -> Option<&'a Value> {
    document
        .get("stages")
        .and_then(Value::as_array)?
        .iter()
        .find(|record| record.get("id").and_then(Value::as_str) == Some(stage))
}

/// One stage record of the comparison: the stage's declaration, and whether
/// the reported pairing can compute a distance in it.
fn stage_records(pairing: &Pairing) -> Vec<Value> {
    let mut records: Vec<Value> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for pair in &pairing.pairs {
        if seen.contains(&pair.stage.as_str()) {
            continue;
        }
        seen.push(&pair.stage);
        records.push(json!({
            "id": pair.stage,
            "domain": pair.domain,
            "units": pair.units,
            "channel_order": pair.channel_order,
            "color_space": pair.color_space,
            "cctf_decoding": pair.cctf_decoding,
            "comparable": pair.incomparable.is_none(),
            "colorimetric": pairing.colorimetric(&pair.stage).is_some(),
        }));
    }
    records
}

/// Every (stage, source, partition) group of one side's pairs, in a stable
/// order.
fn group_keys(pairs: &[Pair]) -> Vec<(String, String, String)> {
    let mut groups: Vec<(String, String, String)> = Vec::new();
    for pair in pairs {
        let key = (
            pair.stage.clone(),
            pair.source.clone(),
            pair.partition.clone(),
        );
        if !groups.contains(&key) {
            groups.push(key);
        }
    }
    groups.sort();
    groups
}

/// A summary statistic, or `null` when it has no finite value to report.
fn number_or_null(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        Value::Null
    }
}

/// Per (stage, source, partition, channel) summaries of a pairing's finite
/// channel deltas.
fn channel_summaries(pairing: &Pairing) -> Vec<Value> {
    let mut records = Vec::new();
    for (stage, source, partition) in group_keys(&pairing.pairs) {
        let members: Vec<&Pair> = pairing
            .pairs
            .iter()
            .filter(|pair| {
                pair.stage == stage && pair.source == source && pair.partition == partition
            })
            .filter(|pair| pair.incomparable.is_none())
            .collect();
        for channel in 0..3 {
            let stats = statistics(
                members
                    .iter()
                    .filter(|pair| pair.valid)
                    .map(|pair| pair.deltas()[channel]),
            );
            records.push(json!({
                "side": pairing.roles.comparison,
                "stage": stage,
                "source": source,
                "partition": partition,
                "channel": channel,
                "metric": "channel_delta",
                "metric_domain": "native",
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

/// Per (stage, source, partition) summaries of a pairing's finite ΔE00, with
/// the number of rows whose conversion had to bound a value.
fn delta_e00_summaries(pairing: &Pairing) -> Vec<Value> {
    let mut records = Vec::new();
    for (stage, source, partition) in group_keys(&pairing.pairs) {
        let Some(space) = pairing.colorimetric(&stage) else {
            continue;
        };
        let members: Vec<&Pair> = pairing
            .pairs
            .iter()
            .filter(|pair| {
                pair.stage == stage && pair.source == source && pair.partition == partition
            })
            .collect();
        let measured: Vec<(f64, bool)> = members
            .iter()
            .filter_map(|pair| pair.delta_e00(space))
            .collect();
        let stats = statistics(measured.iter().map(|(distance, _)| *distance));
        if stats.count == 0 {
            continue;
        }
        records.push(json!({
            "side": pairing.roles.comparison,
            "stage": stage,
            "source": source,
            "partition": partition,
            "metric": "delta_e_2000",
            "metric_domain": DELTA_E00_DOMAIN,
            "count": stats.count,
            "mean": number_or_null(stats.mean),
            "median": number_or_null(stats.median),
            "p95": number_or_null(stats.p95),
            "min": number_or_null(stats.min),
            "max": number_or_null(stats.max),
            "clipped": measured.iter().filter(|(_, clipped)| *clipped).count(),
        }));
    }
    records
}

/// The domain the color distance is computed in: bounded display — the
/// readings pass through linear sRGB into the display's encoded range before
/// they meet the Lab metric.
const DELTA_E00_DOMAIN: &str = "sRGB-encoded D65 (bounded display)";

/// One `deltas.csv` row per joined pair of every pairing: the minuend side
/// minus the subtrahend side, with the comparison that produced it. The
/// reference pairings are retained here too, so the per-sample view is not
/// narrower than the summaries.
fn build_deltas_csv(pairings: &[Pairing]) -> String {
    let mut out = String::from(
        "pair,sample_id,source,partition,stage,domain,units,b0,b1,b2,c0,c1,c2,d0,d1,d2,d_e00,\
         valid,clipped\n",
    );
    for pairing in pairings {
        for pair in &pairing.pairs {
            let deltas = pair.deltas();
            let mut fields = vec![
                csv_field(pairing.roles.comparison),
                csv_field(&pair.sample_id),
                csv_field(&pair.source),
                csv_field(&pair.partition),
                csv_field(&pair.stage),
                csv_field(&pair.domain),
                csv_field(&pair.units),
            ];
            // A pair whose two sides declare different color interpretations
            // reports no channel deltas here either: the record marks the
            // metric unavailable, and a consumer that aggregates this file
            // must not mix it with comparable pairs.
            let comparable = pair.incomparable.is_none();
            for value in pair.left {
                fields.push(number_field(value));
            }
            for value in pair.right {
                fields.push(number_field(value));
            }
            for value in deltas {
                fields.push(if comparable {
                    number_field(value)
                } else {
                    String::new()
                });
            }
            let distance = pairing
                .colorimetric(&pair.stage)
                .and_then(|space| pair.delta_e00(space));
            fields.push(match distance {
                Some((value, _)) => number_field(value),
                None => String::new(),
            });
            fields.push(
                if pair.valid && comparable {
                    "true"
                } else {
                    "false"
                }
                .to_owned(),
            );
            fields.push(match distance {
                Some((_, clipped)) => clipped.to_string(),
                None => String::new(),
            });
            out.push_str(&fields.join(","));
            out.push('\n');
        }
    }
    out
}

/// One compared side: its directory, its record hash.
/// The artifact index of the two files the comparison published, each with the
/// hash of the bytes actually written, so a consumer can verify what it reads.
fn artifact_index(out: &Path) -> Value {
    let entries = [
        ("comparison.json", "application/json", "comparison"),
        ("deltas.csv", "text/csv", "deltas"),
    ];
    json!(
        entries
            .iter()
            .filter_map(|(name, media_type, role)| {
                let digest = sha256_file(&out.join(name)).ok()?;
                Some(json!({
                    "path": name,
                    "sha256": digest,
                    "media_type": media_type,
                    "role": role,
                }))
            })
            .collect::<Vec<_>>()
    )
}

/// The joined samples of one pairing, as the comparison's `cases`.
///
/// A reading comparison has no suite cases, so its case population is the
/// samples the pairing actually joined. Each entry names the sample and stage
/// it covers and the comparison that measured it, which is what a consumer
/// needs to locate the row in `deltas.csv`.
fn reading_cases(pairing: &Pairing) -> Value {
    let mut cases: Vec<Value> = pairing
        .pairs
        .iter()
        .map(|pair| {
            json!({
                "case_id": pair.sample_id,
                "stage": pair.stage,
                "comparison": pairing.roles.comparison,
                "domain": pair.domain,
                "units": pair.units,
                "valid": pair.valid,
            })
        })
        .collect();
    cases.sort_by(|a, b| {
        (
            a["case_id"].as_str().unwrap_or_default(),
            a["stage"].as_str().unwrap_or_default(),
        )
            .cmp(&(
                b["case_id"].as_str().unwrap_or_default(),
                b["stage"].as_str().unwrap_or_default(),
            ))
    });
    json!(cases)
}

/// The comparison's `summary`: what it measured, over how many samples, and
/// what it could not measure. Every count is derived from the reported
/// pairing's own rows, so a summary cannot claim a population the pairing
/// did not join.
fn reading_summary(reported: &Pairing, pairings: &[Pairing]) -> Value {
    let total = reported.pairs.len();
    let valid = reported.pairs.iter().filter(|pair| pair.valid).count();
    let unavailable: usize = pairings
        .iter()
        .map(|pairing| pairing.unavailable_records().len())
        .sum();
    json!({
        "comparison": reported.roles.comparison,
        "samples": total,
        "valid_samples": valid,
        "invalid_samples": total - valid,
        "unavailable_measurements": unavailable,
        "stages": stage_records(reported).len(),
    })
}

fn side_record(side: &Side) -> Value {
    json!({
        "path": side.path.display().to_string(),
        "measure_sha256": side.measure_sha256,
    })
}

/// Flatten one JSON value into `path -> leaf` pairs. Object keys join with
/// `.` and array indices with `[]`, so a leaf keeps its address.
fn flatten(value: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(child, &path, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                flatten(child, &format!("{prefix}[{index}]"), out);
            }
        }
        leaf => out.push((prefix.to_owned(), leaf.clone())),
    }
}

/// The records a measurement may carry, flattened under their own names. A
/// sampled measurement records its `source`; an executed run records its
/// `declaration` and its `identity`. Every record both sides carry is
/// compared, so a difference is never missed for being in another record.
const RECORDS: [&str; 4] = ["identity", "declaration", "source", "stages"];

/// Every leaf where two sides' records disagree, one record per leaf per
/// pairing. Every difference is evidence, never an error, and it is reported
/// under the roles it was taken from: a reference difference is not relabelled
/// as the candidate's.
fn identity_differences(pairings: &[Pairing]) -> Vec<Value> {
    let mut differences = Vec::new();
    for pairing in pairings {
        differences.extend(differences_of(pairing));
    }
    differences.sort_by_key(|difference| {
        (
            difference["comparison"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            difference["path"].as_str().unwrap_or_default().to_owned(),
        )
    });
    // Only identical entries collapse, and only within one pairing.
    differences.dedup();
    differences
}

/// One difference, reported under the roles it was taken from: the record
/// names the pairing and carries each side's value under that side's own name,
/// so a reference's difference can never be read as the candidate's.
fn role_difference(pairing: &Pairing, path: String, subtrahend: Value, minuend: Value) -> Value {
    let mut record = serde_json::Map::new();
    record.insert("comparison".to_owned(), json!(pairing.roles.comparison));
    record.insert("path".to_owned(), json!(path));
    record.insert(pairing.roles.subtrahend.to_owned(), subtrahend);
    record.insert(pairing.roles.minuend.to_owned(), minuend);
    Value::Object(record)
}

/// The differing leaves of the records one pairing's two sides both carry,
/// plus the provenance labels of its readings: a label difference is a
/// difference of the record, not of the measured quantity, so it is reported
/// instead of deciding which label wins.
fn differences_of(pairing: &Pairing) -> Vec<Value> {
    let mut differences = Vec::new();
    for pair in &pairing.pairs {
        for (field, left, right) in [
            ("source", &pair.source, &pair.right_source),
            ("partition", &pair.partition, &pair.right_partition),
        ] {
            if left != right {
                differences.push(role_difference(
                    pairing,
                    format!("reading[{}|{}].{field}", pair.sample_id, pair.stage),
                    json!(left),
                    json!(right),
                ));
            }
        }
    }
    let left = &pairing.left;
    let right = &pairing.right;
    for name in RECORDS {
        let Some(left_record) = left.document.get(name) else {
            continue;
        };
        let Some(right_record) = right.document.get(name) else {
            continue;
        };
        let mut left_leaves = Vec::new();
        let mut right_leaves = Vec::new();
        flatten(left_record, name, &mut left_leaves);
        flatten(right_record, name, &mut right_leaves);
        let right_by_path: BTreeMap<&str, &Value> = right_leaves
            .iter()
            .map(|(path, value)| (path.as_str(), value))
            .collect();
        let left_paths: Vec<&str> = left_leaves.iter().map(|(path, _)| path.as_str()).collect();
        for (path, left_value) in &left_leaves {
            let right_value = right_by_path.get(path.as_str()).copied();
            if right_value != Some(left_value) {
                differences.push(role_difference(
                    pairing,
                    path.clone(),
                    left_value.clone(),
                    right_value.cloned().unwrap_or(Value::Null),
                ));
            }
        }
        for (path, right_value) in &right_leaves {
            if !left_paths.contains(&path.as_str()) {
                differences.push(role_difference(
                    pairing,
                    path.clone(),
                    Value::Null,
                    right_value.clone(),
                ));
            }
        }
    }
    differences
}

/// What the comparison found, on stderr: the matched keys, the identity
/// drift, the unmatched keys, and every metric that could not be computed.
fn print_summary(out: &Path, pairing: &Pairing, document: &Value) {
    let pairs = &pairing.pairs;
    let mut stages: Vec<&str> = pairs.iter().map(|pair| pair.stage.as_str()).collect();
    stages.sort_unstable();
    stages.dedup();
    let mut samples: Vec<&str> = pairs.iter().map(|pair| pair.sample_id.as_str()).collect();
    samples.sort_unstable();
    samples.dedup();
    eprintln!(
        "Comparison: {} samples × {} stages → {}",
        samples.len(),
        stages.len(),
        out.display()
    );
    let differences = document["identity_differences"]
        .as_array()
        .map_or(0, Vec::len);
    eprintln!("Identity differences: {differences}");
    let unmatched = document["unmatched"].as_array().map_or(0, Vec::len);
    eprintln!("Unmatched keys: {unmatched}");
    for entry in document["unavailable"].as_array().into_iter().flatten() {
        eprintln!(
            "Unavailable {} on {}: {}",
            entry["metric"].as_str().unwrap_or_default(),
            entry["pair"].as_str().unwrap_or_default(),
            entry["reason"].as_str().unwrap_or_default()
        );
    }
    let colorimetric: Vec<&str> = stages
        .iter()
        .copied()
        .filter(|stage| pairing.colorimetric(stage).is_some())
        .collect();
    if !colorimetric.is_empty() {
        eprintln!("Colorimetric stages: {}", colorimetric.join(", "));
    }
    eprintln!("Report: {}", out.join("comparison.json").display());
    eprintln!("Deltas: {}", out.join("deltas.csv").display());
}
