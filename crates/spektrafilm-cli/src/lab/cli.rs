//! Command-line surface of the instrument binary (`spektrafilm-lab`).
//!
//! One grammar covers the whole instrument: schema discovery, document
//! validation, appearance evaluation, chain measurement, region derivation,
//! fixture generation, and the experimental commands.

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use spektrafilm_core::params::Tap;

/// Spectral diagnostics.
#[derive(Subcommand)]
pub enum SpectralCommand {
    /// Report the roundtrip error of the product path on measured spectra.
    Evaluate(EvaluateArgs),
}

/// Appearance evaluation: document discovery, evidence rendering, comparison,
/// and offline reporting.
#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("run_source").required(true).multiple(false)
        .args(["suite", "input", "stimuli"])
))]
// The suite-only options must not combine with the single-input form; the
// single-input options must not combine with the suite form.
#[command(group(
    clap::ArgGroup::new("suite_only").multiple(true).args(["case_ids"]).conflicts_with("input")
))]
#[command(group(
    clap::ArgGroup::new("input_only").multiple(true)
        .args(["configuration", "input_kind", "input_color_space", "input_cctf_decoding",
               "seeds", "regions", "reference", "reference_info", "preparation"])
        .conflicts_with("suite")
))]
pub struct EvaluationRunArgs {
    /// Suite document.
    #[arg(long, value_name = "FILE", help_heading = "Image-domain run")]
    pub suite: Option<PathBuf>,
    /// Prepared local image.
    #[arg(
        long,
        value_name = "FILE",
        requires = "configuration",
        help_heading = "Image-domain run"
    )]
    pub input: Option<PathBuf>,
    /// Existing case configuration object.
    #[arg(
        long,
        value_name = "FILE",
        requires = "input",
        help_heading = "Image-domain run"
    )]
    pub configuration: Option<PathBuf>,
    /// Explicit source kind (default raster).
    #[arg(long, value_name = "KIND", value_parser = ["raster", "raw"], requires = "input", help_heading = "Image-domain run")]
    pub input_kind: Option<String>,
    /// Raster source color space.
    #[arg(
        long,
        value_name = "NAME",
        requires = "input",
        help_heading = "Image-domain run"
    )]
    pub input_color_space: Option<String>,
    /// Whether to decode the raster transfer function.
    #[arg(long, value_name = "true|false", value_parser = clap::value_parser!(bool), requires = "input", help_heading = "Image-domain run")]
    pub input_cctf_decoding: Option<bool>,
    /// Distinct UINT32 seed; repeat to choose a population (default 0).
    #[arg(
        long = "seed",
        value_name = "UINT32",
        requires = "input",
        help_heading = "Image-domain run"
    )]
    pub seeds: Vec<u32>,
    /// Existing fixed region array in render coordinates.
    #[arg(
        long,
        value_name = "FILE",
        requires = "input",
        help_heading = "Image-domain run"
    )]
    pub regions: Option<PathBuf>,
    /// Reference image.
    #[arg(long, value_name = "FILE", requires_all = ["input", "reference_info"], help_heading = "Image-domain run")]
    pub reference: Option<PathBuf>,
    /// Existing reference object without path and sha256.
    #[arg(long, value_name = "FILE", requires_all = ["input", "reference"], help_heading = "Image-domain run")]
    pub reference_info: Option<PathBuf>,
    /// Passive preparation provenance object.
    #[arg(
        long,
        value_name = "FILE",
        requires = "input",
        help_heading = "Image-domain run"
    )]
    pub preparation: Option<PathBuf>,
    /// Renderer target document.
    #[arg(
        long,
        value_name = "FILE",
        required_unless_present = "stimuli",
        conflicts_with = "stimuli",
        help_heading = "Image-domain run"
    )]
    pub target: Option<PathBuf>,
    /// New run directory; must not exist.
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
    /// Exact suite case identity; repeat in suite mode.
    #[arg(
        long = "case",
        value_name = "ID",
        requires = "suite",
        help_heading = "Image-domain run"
    )]
    pub case_ids: Vec<String>,
    /// Resolve all selected configurations without decoding or rendering; retain preflight.json.
    #[arg(long)]
    pub dry_run: bool,
    /// Retain native CPU stage buffers, applied exposure, and prepared print normalization.
    #[arg(long, conflicts_with = "stimuli", help_heading = "Image-domain run")]
    pub stage_evidence: bool,

    // ---- Chain run: one declared stimulus set executed through the chain ----
    /// Stimulus document: a measured-spectra manifest for `spectra`, a points
    /// manifest otherwise. Arrays live beside the manifest as .npy/.npz.
    #[arg(long, value_name = "PATH", help_heading = "Chain run")]
    pub stimuli: Option<PathBuf>,
    /// Chain entry boundary; it declares how the stimulus values are read.
    #[arg(long, value_enum, requires = "stimuli", help_heading = "Chain run")]
    pub enter: Option<MeasureEntry>,
    /// Stages to read; omit to read every available stage.
    #[arg(long, value_enum, value_delimiter = ',', action = clap::ArgAction::Append, requires = "stimuli", help_heading = "Chain run")]
    pub read: Vec<MeasureStage>,
    /// Exact preset name or .toml/.json file; supplies the film and print profiles.
    #[arg(long, action = clap::ArgAction::Append, requires = "stimuli", help_heading = "Chain")]
    pub preset: Vec<String>,
    /// Film profile name in the data directory, without a preset.
    #[arg(
        long,
        value_name = "STOCK",
        requires = "stimuli",
        help_heading = "Chain"
    )]
    pub film: Option<String>,
    /// Print profile name; defaults as in the process surface.
    #[arg(
        long,
        value_name = "STOCK",
        requires = "stimuli",
        help_heading = "Chain"
    )]
    pub paper: Option<String>,
    /// Apply PATH=VALUE assignments or a sparse .toml/.json file in order.
    #[arg(long = "set", value_name = "PATH=VALUE|FILE", action = clap::ArgAction::Append, requires = "stimuli", help_heading = "Chain")]
    pub sources: Vec<String>,
    /// Workflow route; the resolved route bounds the executed stages.
    #[arg(
        long,
        value_name = "ROUTE",
        requires = "stimuli",
        help_heading = "Chain"
    )]
    pub route: Option<String>,
    /// Film scanner output mode.
    #[arg(long, value_parser = ["direct_scan", "negative_to_positive"], requires = "stimuli", help_heading = "Chain")]
    pub scan_output: Option<String>,
    /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
    #[arg(long, value_name = "DIR", help_heading = "Chain")]
    pub data_dir: Option<PathBuf>,
}

#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("compare_source").required(true).multiple(false)
        .args(["baseline", "run"])
))]
pub struct EvaluationCompareArgs {
    /// Baseline run directory.
    #[arg(
        long,
        value_name = "DIR",
        requires = "candidate",
        conflicts_with = "run",
        help_heading = "Image-domain comparison"
    )]
    pub baseline: Option<PathBuf>,
    /// Candidate run directory.
    #[arg(
        long,
        value_name = "DIR",
        requires = "baseline",
        conflicts_with = "run",
        help_heading = "Image-domain comparison"
    )]
    pub candidate: Option<PathBuf>,
    /// Reference run directory; a per-sample distance needs one.
    #[arg(
        long,
        value_name = "DIR",
        requires = "baseline",
        conflicts_with = "run",
        help_heading = "Image-domain comparison"
    )]
    pub reference_run: Option<PathBuf>,
    /// Single run directory.
    #[arg(
        long,
        value_name = "DIR",
        requires = "against",
        conflicts_with = "baseline",
        help_heading = "Image-domain comparison"
    )]
    pub run: Option<PathBuf>,
    /// Evidence pairs to compare.
    #[arg(long, value_name = "PAIR", value_parser = ["input", "reference", "both"], requires = "run", help_heading = "Image-domain comparison")]
    pub against: Option<String>,
    /// New comparison directory; must not exist.
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
    /// Version-comparison policy document.
    #[arg(
        long,
        value_name = "FILE",
        conflicts_with = "run",
        help_heading = "Image-domain comparison"
    )]
    pub policy: Option<PathBuf>,
    /// Fail command when policy does not pass.
    #[arg(
        long,
        requires = "policy",
        conflicts_with = "run",
        help_heading = "Image-domain comparison"
    )]
    pub gate: bool,
}

#[derive(Args)]
pub struct EvaluationInspectArgs {
    /// Comparison directory.
    #[arg(long, value_name = "DIR")]
    pub comparison: PathBuf,
    /// Exact case identity; repeat to select multiple.
    #[arg(long = "case", value_name = "ID")]
    pub case_ids: Vec<String>,
    /// Exact region identity; repeat to select multiple.
    #[arg(long = "region", value_name = "ID")]
    pub region_ids: Vec<String>,
    /// Exact metric identity; repeat to select multiple.
    #[arg(long = "metric", value_name = "ID")]
    pub metrics: Vec<String>,
}

#[derive(Args)]
pub struct EvaluationTargetArgs {
    /// Renderer executable.
    #[arg(long, value_name = "FILE")]
    pub renderer: PathBuf,
    /// Explicit profile/data directory.
    #[arg(long, value_name = "DIR")]
    pub data_dir: PathBuf,
    /// Requested compute backend; availability is checked during rendering.
    #[arg(long, value_name = "BACKEND", value_parser = ["cpu", "gpu"])]
    pub backend: String,
    /// New target JSON file; must not exist.
    #[arg(long, value_name = "FILE")]
    pub out: PathBuf,
}

#[derive(Subcommand)]
pub enum ReportCommand {
    /// Build an offline report from one retained comparison.
    Comparison(ReportComparisonArgs),
}

#[derive(Args)]
pub struct ReportComparisonArgs {
    /// Retained comparison directory.
    #[arg(long, value_name = "DIR")]
    pub comparison: PathBuf,
    /// New report directory; must not exist.
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
}

#[derive(Subcommand)]
pub enum RegionsCommand {
    /// Expand one finite grid into an ordinary regions document.
    Grid(RegionsGridArgs),
}

#[derive(Args)]
pub struct RegionsGridArgs {
    /// Regions document identity.
    #[arg(long, value_name = "ID")]
    pub id: String,
    /// Number of grid rows.
    #[arg(long, value_name = "N")]
    pub rows: u64,
    /// Number of grid columns.
    #[arg(long, value_name = "N")]
    pub columns: u64,
    /// Top-left origin as X,Y.
    #[arg(long, value_name = "X,Y", value_parser = parse_pair)]
    pub origin: [u64; 2],
    /// Patch size as WIDTHxHEIGHT.
    #[arg(long, value_name = "WIDTHxHEIGHT", value_parser = parse_size)]
    pub patch_size: [u64; 2],
    /// Gap as X,Y.
    #[arg(long, value_name = "X,Y", value_parser = parse_pair)]
    pub gap: [u64; 2],
    /// Centered sample size as WIDTHxHEIGHT.
    #[arg(long, value_name = "WIDTHxHEIGHT", value_parser = parse_size)]
    pub sample: Option<[u64; 2]>,
    /// Coordinate space for generated rectangles.
    #[arg(long, value_name = "SPACE", default_value = "source")]
    pub space: String,
    #[arg(long, value_name = "ROLE", default_value = "other")]
    pub role: String,
    /// Measurement families, separated by commas.
    #[arg(long, value_name = "FAMILY,...", value_delimiter = ',', action = clap::ArgAction::Append)]
    pub measurements: Vec<String>,
    /// New regions document JSON file; must not exist.
    #[arg(long, value_name = "FILE")]
    pub out: PathBuf,
}
fn parse_pair(value: &str) -> Result<[u64; 2], String> {
    let mut parts = value.split(',');
    let first = parts
        .next()
        .ok_or_else(|| "expected X,Y".to_owned())?
        .parse::<u64>()
        .map_err(|_| format!("invalid pair {value:?}; expected X,Y"))?;
    let second = parts
        .next()
        .ok_or_else(|| "expected X,Y".to_owned())?
        .parse::<u64>()
        .map_err(|_| format!("invalid pair {value:?}; expected X,Y"))?;
    if parts.next().is_some() {
        return Err(format!("invalid pair {value:?}; expected X,Y"));
    }
    Ok([first, second])
}

fn parse_size(value: &str) -> Result<[u64; 2], String> {
    let mut parts = value.split('x');
    let first = parts
        .next()
        .ok_or_else(|| "expected WIDTHxHEIGHT".to_owned())?
        .parse::<u64>()
        .map_err(|_| format!("invalid size {value:?}; expected WIDTHxHEIGHT"))?;
    let second = parts
        .next()
        .ok_or_else(|| "expected WIDTHxHEIGHT".to_owned())?
        .parse::<u64>()
        .map_err(|_| format!("invalid size {value:?}; expected WIDTHxHEIGHT"))?;
    if parts.next().is_some() {
        return Err(format!("invalid size {value:?}; expected WIDTHxHEIGHT"));
    }
    Ok([first, second])
}

/// The chain boundary a measurement enters at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum MeasureEntry {
    /// Measured reflectance spectra.
    #[value(name = "spectra")]
    Spectra,
    /// Scene RGB after input decoding.
    #[value(name = "rgb_in")]
    RgbIn,
    /// Film raw log exposure (CMY).
    #[value(name = "log_e_film")]
    LogEFilm,
    /// Film density (CMY).
    #[value(name = "cmy_film")]
    CmyFilm,
}

impl MeasureEntry {
    /// Every entry in chain order.
    pub const ALL: [MeasureEntry; 4] = [
        MeasureEntry::Spectra,
        MeasureEntry::RgbIn,
        MeasureEntry::LogEFilm,
        MeasureEntry::CmyFilm,
    ];

    /// The declared name of the entry, as the command surface spells it.
    pub const fn name(self) -> &'static str {
        match self {
            MeasureEntry::Spectra => "spectra",
            MeasureEntry::RgbIn => "rgb_in",
            MeasureEntry::LogEFilm => "log_e_film",
            MeasureEntry::CmyFilm => "cmy_film",
        }
    }

    /// The canonical tap the entry injects at: the shared `taps.inject`
    /// control. `spectra` enters at `rgb_in`; the spectral path builds its own
    /// film response instead of running the image pipeline.
    pub const fn tap(self) -> Tap {
        match self {
            MeasureEntry::Spectra | MeasureEntry::RgbIn => Tap::RgbIn,
            MeasureEntry::LogEFilm => Tap::LogEFilm,
            MeasureEntry::CmyFilm => Tap::CmyFilm,
        }
    }

    /// The stimulus domain the entry requires.
    pub const fn domain(self) -> &'static str {
        match self {
            MeasureEntry::Spectra => "reflectance",
            MeasureEntry::RgbIn => "scene_rgb",
            MeasureEntry::LogEFilm => "film_log_e",
            MeasureEntry::CmyFilm => "film_density",
        }
    }

    /// The entry's position on the chain's tap scale: 0 is `rgb_in`. A stage
    /// the entry is at or before executes; an earlier stage does not.
    pub const fn position(self) -> u8 {
        self.tap().position()
    }
}

/// A stage a measurement can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum MeasureStage {
    /// Scene RGB as the chain reads it after input decoding.
    #[value(name = "rgb_in")]
    RgbIn,
    /// Film raw log exposure.
    #[value(name = "log_e_film")]
    LogEFilm,
    /// Film density.
    #[value(name = "cmy_film")]
    CmyFilm,
    /// Print raw log exposure.
    #[value(name = "log_e_print")]
    LogEPrint,
    /// Print density.
    #[value(name = "cmy_print")]
    CmyPrint,
    /// Final pipeline output before saving.
    #[value(name = "rgb_out")]
    RgbOut,
}

impl MeasureStage {
    /// Every readable stage in chain order.
    pub const ALL: [MeasureStage; 6] = [
        MeasureStage::RgbIn,
        MeasureStage::LogEFilm,
        MeasureStage::CmyFilm,
        MeasureStage::LogEPrint,
        MeasureStage::CmyPrint,
        MeasureStage::RgbOut,
    ];

    /// The canonical tap this stage reads.
    pub const fn tap(self) -> Tap {
        match self {
            MeasureStage::RgbIn => Tap::RgbIn,
            MeasureStage::LogEFilm => Tap::LogEFilm,
            MeasureStage::CmyFilm => Tap::CmyFilm,
            MeasureStage::LogEPrint => Tap::LogEPrint,
            MeasureStage::CmyPrint => Tap::CmyPrint,
            MeasureStage::RgbOut => Tap::RgbOut,
        }
    }

    /// The stage's position on the chain's tap scale: 0 is `rgb_in`. An
    /// operation that changes values from this stage on is inside a range
    /// whose entry is before it and whose last reading is at or after it.
    pub const fn position(self) -> u8 {
        self.tap().position()
    }

    /// The stage-evidence identifier this stage reads.
    pub const fn id(self) -> &'static str {
        match self {
            MeasureStage::RgbIn => "rgb_in",
            MeasureStage::LogEFilm => "log_e_film",
            MeasureStage::CmyFilm => "cmy_film",
            MeasureStage::LogEPrint => "log_e_print",
            MeasureStage::CmyPrint => "cmy_print",
            MeasureStage::RgbOut => "rgb_out",
        }
    }
}

/// The chain options a measurement shares with `process`.
#[derive(Args)]
pub struct ChainArgs {
    /// Exact preset name or .toml/.json file; supplies the film and print profiles.
    #[arg(long, action = clap::ArgAction::Append, help_heading = "Chain")]
    pub preset: Vec<String>,
    /// Film profile name in the data directory, without a preset.
    #[arg(long, value_name = "STOCK", help_heading = "Chain")]
    pub film: Option<String>,
    /// Print profile name; defaults as in the process surface.
    #[arg(long, value_name = "STOCK", help_heading = "Chain")]
    pub paper: Option<String>,
    /// Apply PATH=VALUE assignments or a sparse .toml/.json file in order.
    #[arg(long = "set", value_name = "PATH=VALUE|FILE", action = clap::ArgAction::Append, help_heading = "Chain")]
    pub sources: Vec<String>,
    /// Workflow route; the resolved route bounds the executed stages.
    #[arg(long, value_name = "ROUTE", help_heading = "Chain")]
    pub route: Option<String>,
    /// Film scanner output mode.
    #[arg(long, value_parser = ["direct_scan", "negative_to_positive"], help_heading = "Chain")]
    pub scan_output: Option<String>,
    /// Deterministic stochastic seed recorded with the measurement. A chain
    /// run resolves its own seed from the parameter surface, so the instrument
    /// records this one in the measurement's identity instead of accepting it
    /// as a command option. It is not a command-line argument.
    #[arg(skip)]
    pub seed: u32,
    /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
    #[arg(long, value_name = "DIR", help_heading = "Chain")]
    pub data_dir: Option<PathBuf>,
}

#[derive(Args)]
pub struct MeasureRunArgs {
    /// Stimulus document: a measured-spectra manifest for `spectra`, a points
    /// manifest otherwise. Arrays live beside the manifest as .npy/.npz.
    #[arg(long, value_name = "PATH", help_heading = "Stimulus")]
    pub stimuli: PathBuf,
    /// Chain entry boundary; it declares how the stimulus values are read.
    #[arg(long, value_enum, help_heading = "Stimulus")]
    pub enter: MeasureEntry,
    /// Stages to read; omit to read every available stage.
    #[arg(long, value_enum, value_delimiter = ',', action = clap::ArgAction::Append, help_heading = "Measurement")]
    pub read: Vec<MeasureStage>,
    /// Resolve the chain and write the identity record; never execute it.
    #[arg(long, help_heading = "Measurement")]
    pub dry_run: bool,
    /// Record destination directory; existing artifacts are never replaced.
    #[arg(long, value_name = "DIR", help_heading = "Measurement")]
    pub out: PathBuf,
    #[command(flatten)]
    pub chain: ChainArgs,
}

#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("sample_source").required(true).multiple(false)
        .args(["source", "from_evidence", "from_run"])
))]
pub struct MeasureSampleArgs {
    /// Raster image to read; requires the input declaration below.
    #[arg(long, value_name = "PATH", help_heading = "Source")]
    pub source: Option<PathBuf>,
    /// Retained stage-capture bundle to read instead of an image.
    #[arg(
        long,
        value_name = "DIR",
        conflicts_with_all = ["source", "from_run"],
        help_heading = "Source"
    )]
    pub from_evidence: Option<PathBuf>,
    /// Retained image-domain run directory or RUN.JSON whose selected case owns the stage capture.
    #[arg(
        long,
        value_name = "DIR|RUN.JSON",
        conflicts_with_all = ["source", "from_evidence"],
        help_heading = "Source"
    )]
    pub from_run: Option<PathBuf>,
    /// Exact case identity in a retained image-domain run.
    #[arg(
        long,
        value_name = "ID",
        required_unless_present_any = ["source", "from_evidence"],
        requires = "from_run",
        help_heading = "Source"
    )]
    pub case: Option<String>,
    #[arg(
        long,
        value_name = "UINT32",
        required_unless_present_any = ["source", "from_evidence"],
        requires = "from_run",
        help_heading = "Source"
    )]
    pub seed: Option<u32>,
    /// Stage to read from a retained source; its declaration governs the reading.
    #[arg(
        long,
        value_enum,
        value_name = "STAGE",
        required_unless_present = "source",
        conflicts_with = "source",
        help_heading = "Source"
    )]
    pub stage: Option<MeasureStage>,
    /// Source color space of the image, as `process` declares it.
    #[arg(
        long,
        value_name = "NAME",
        requires = "source",
        required_unless_present_any = ["from_evidence", "from_run"],
        help_heading = "Source"
    )]
    pub input_color_space: Option<String>,
    /// Whether the image's stored values are transfer-encoded.
    #[arg(
        long,
        value_name = "true|false",
        value_parser = clap::value_parser!(bool),
        requires = "source",
        required_unless_present_any = ["from_evidence", "from_run"],
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

#[derive(Args)]
pub struct MeasureCompareArgs {
    /// Baseline measurement directory.
    #[arg(long, value_name = "DIR", help_heading = "Comparison")]
    pub baseline: PathBuf,
    /// Candidate measurement directory.
    #[arg(long, value_name = "DIR", help_heading = "Comparison")]
    pub candidate: PathBuf,
    /// Reference measurement directory; per-sample distance needs one.
    #[arg(long, value_name = "DIR", help_heading = "Comparison")]
    pub reference: Option<PathBuf>,
    /// Comparison destination directory; existing artifacts are never replaced.
    #[arg(long, value_name = "DIR", help_heading = "Comparison")]
    pub out: PathBuf,
}

#[derive(Args)]
pub struct MeasureVerifyArgs {
    /// Record directory to re-resolve.
    #[arg(long, value_name = "DIR", help_heading = "Verification")]
    pub record: PathBuf,
    /// Data directory to re-resolve against; defaults to the directory the run recorded.
    #[arg(long, default_value = "data", help_heading = "Verification")]
    pub data_dir: PathBuf,
}

/// The development loop for a film profile: render one declared experiment,
/// then convert its boundary values for a measurement tool.
#[derive(Subcommand)]
pub enum ExperimentCommand {
    /// Render one declared experiment and retain its boundary values.
    Run(ExperimentRunArgs),
    /// Convert a bundle's buffers into .npy arrays.
    Export(ExperimentExportArgs),
}

#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("chain").required(true).multiple(false).args(["preset", "profile"])
))]
pub struct EvaluateArgs {
    /// Measured-spectra set JSON manifest; arrays live beside it as .npz/.npy.
    #[arg(long, value_name = "PATH", help_heading = "Evaluation")]
    pub spectra: PathBuf,
    /// Report destination directory; an existing report is never replaced.
    #[arg(long, value_name = "DIR", help_heading = "Evaluation")]
    pub out: PathBuf,
    /// Exact preset name or .toml/.json file; supplies the film profile.
    #[arg(long, action = clap::ArgAction::Append, help_heading = "Chain")]
    pub preset: Vec<String>,
    /// Film profile JSON path to evaluate.
    #[arg(long, value_name = "PATH", help_heading = "Chain")]
    pub profile: Option<PathBuf>,
    /// Apply PATH=VALUE assignments or a sparse .toml/.json file in order.
    #[arg(long = "set", value_name = "PATH=VALUE|FILE", action = clap::ArgAction::Append, help_heading = "Chain")]
    pub sources: Vec<String>,
    /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
    #[arg(long, default_value = "data", help_heading = "Chain")]
    pub data_dir: PathBuf,
}

#[derive(Args)]
pub struct ExperimentRunArgs {
    /// Experiment document declaring the case, grid, chain, and input.
    #[arg(long, value_name = "PATH", help_heading = "Experiment")]
    pub experiment: PathBuf,
    /// Report destination; an existing report is never replaced without --force.
    #[arg(long, value_name = "DIR", help_heading = "Experiment")]
    pub out: PathBuf,
    /// Image container written beside the bundle.
    #[arg(long, value_enum, default_value = "tiff", help_heading = "Experiment")]
    pub output_format: OutputFormatArg,
    /// Replace an existing report in the destination.
    #[arg(long, help_heading = "Experiment")]
    pub force: bool,
    /// The `process` parameter surface. Each option overrides the
    /// experiment document's own declaration of the same name, and a `--set`
    /// source applies after the document's, so the command line is the last
    /// word on the chain it executes.
    #[command(flatten)]
    pub chain: ChainArgs,
}

#[derive(Args)]
pub struct ExperimentExportArgs {
    /// Stage-evidence bundle directory holding the terminal manifest.
    #[arg(long, value_name = "DIR", help_heading = "Export")]
    pub stage_evidence: PathBuf,
    /// Array destination; an existing array is never replaced without --force.
    #[arg(long, value_name = "DIR", help_heading = "Export")]
    pub out: PathBuf,
    /// Stage to convert; repeat to convert several. The default is every
    /// available stage in manifest order.
    #[arg(long, value_name = "ID", action = clap::ArgAction::Append, help_heading = "Export")]
    pub stage: Vec<String>,
    /// Replace an existing array in the destination.
    #[arg(long, help_heading = "Export")]
    pub force: bool,
}

/// Image container for the experiment's rendered output, named as on the
/// process surface.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum OutputFormatArg {
    Jpeg,
    Png,
    Tiff,
    Exr,
}

impl OutputFormatArg {
    /// The process surface's spelling of the container.
    pub fn into_name(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Exr => "exr",
        }
    }
}
