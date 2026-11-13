//! Command-line surface of the instrument binary (`spektrafilm-lab`).
//!
//! One grammar covers the whole instrument: schema discovery, document
//! validation, appearance evaluation, chain measurement, region derivation,
//! fixture generation, and the experimental commands.

use std::path::PathBuf;

use clap::{Args, Subcommand};
pub use spektrafilm_benchmark::readings::{MeasureEntry, MeasureStage};

/// The chain tap one declared vocabulary name maps to. The mapping is the
/// renderer's: the reading vocabulary names what a buffer is, and this says
/// where the renderer produced it.
pub trait ChainTap: Copy {
    fn tap(self) -> Tap;
    fn position(self) -> u8 {
        self.tap().position()
    }
}

impl ChainTap for MeasureEntry {
    /// `spectra` enters at `rgb_in`; the spectral path builds its own film
    /// response instead of running the image pipeline.
    fn tap(self) -> Tap {
        match self {
            MeasureEntry::Spectra | MeasureEntry::RgbIn => Tap::RgbIn,
            MeasureEntry::LogEFilm => Tap::LogEFilm,
            MeasureEntry::CmyFilm => Tap::CmyFilm,
        }
    }
}

impl ChainTap for MeasureStage {
    fn tap(self) -> Tap {
        match self {
            MeasureStage::RgbIn => Tap::RgbIn,
            MeasureStage::LogEFilm => Tap::LogEFilm,
            MeasureStage::CmyFilm => Tap::CmyFilm,
            MeasureStage::LogEPrint => Tap::LogEPrint,
            MeasureStage::CmyPrint => Tap::CmyPrint,
            MeasureStage::RgbOut => Tap::RgbOut,
        }
    }
}
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
    pub out: Option<PathBuf>,
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

/// Execute one authored image-domain suite.
#[derive(Args)]
pub struct RunSuiteArgs {
    /// Suite document.
    #[arg(long, value_name = "FILE")]
    pub suite: PathBuf,
    /// Renderer target document.
    #[arg(long, value_name = "FILE")]
    pub target: PathBuf,
    /// New run directory; must not exist.
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
    /// Exact suite case identity; repeat to select multiple cases.
    #[arg(long = "case", value_name = "ID", action = clap::ArgAction::Append)]
    pub case_ids: Vec<String>,
    /// Resolve configurations without decoding or rendering.
    #[arg(long)]
    pub dry_run: bool,
}

/// Execute one local image through the image-domain runner.
#[derive(Args)]
pub struct RunImageArgs {
    /// Prepared local image.
    #[arg(long, value_name = "FILE")]
    pub input: PathBuf,
    /// Existing case configuration object.
    #[arg(long, value_name = "FILE")]
    pub configuration: PathBuf,
    /// Explicit source kind.
    #[arg(long, value_name = "KIND", value_parser = ["raster", "raw"], default_value = "raster")]
    pub input_kind: String,
    /// Raster source color space.
    #[arg(long, value_name = "NAME")]
    pub input_color_space: Option<String>,
    /// Whether to decode the raster transfer function.
    #[arg(long, value_name = "true|false", value_parser = clap::value_parser!(bool))]
    pub input_cctf_decoding: Option<bool>,
    /// Distinct UINT32 seed; repeat to choose a population.
    #[arg(long = "seed", value_name = "UINT32", action = clap::ArgAction::Append)]
    pub seeds: Vec<u32>,
    /// Existing fixed region array in render coordinates.
    #[arg(long, value_name = "FILE")]
    pub regions: Option<PathBuf>,
    /// Reference image.
    #[arg(long, value_name = "FILE")]
    pub reference: Option<PathBuf>,
    /// Existing reference object without path and sha256.
    #[arg(long, value_name = "FILE")]
    pub reference_info: Option<PathBuf>,
    /// Passive preparation provenance object.
    #[arg(long, value_name = "FILE")]
    pub preparation: Option<PathBuf>,
    /// Renderer target document.
    #[arg(long, value_name = "FILE")]
    pub target: PathBuf,
    /// New run directory; must not exist.
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
    /// Resolve configurations without decoding or rendering.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Subcommand)]
pub enum RunCommand {
    /// Execute one authored image-domain suite.
    Suite(RunSuiteArgs),
    /// Execute one local image through the image-domain runner.
    Image(RunImageArgs),
    /// Compatibility alias target inserted by the argv normalizer.
    #[command(hide = true)]
    Flat(Box<EvaluationRunArgs>),
}

#[derive(Subcommand)]
pub enum MeasureCommand {
    /// Execute one declared point-wise stimulus chain.
    Run(MeasureRunArgs),
}

#[derive(Args)]
/// Compare two completed image-domain runs.
pub struct CompareRunsArgs {
    /// Baseline run directory.
    #[arg(long, value_name = "DIR")]
    pub baseline: PathBuf,
    /// Candidate run directory.
    #[arg(long, value_name = "DIR")]
    pub candidate: PathBuf,
    /// New comparison directory.
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
    /// Version-comparison policy document.
    #[arg(long, value_name = "FILE")]
    pub policy: Option<PathBuf>,
    /// Fail when policy does not pass.
    #[arg(long, requires = "policy")]
    pub gate: bool,
}

/// Compare two retained reading records.
#[derive(Args)]
pub struct CompareReadingsArgs {
    /// Baseline reading directory.
    #[arg(long, value_name = "DIR")]
    pub baseline: PathBuf,
    /// Candidate reading directory.
    #[arg(long, value_name = "DIR")]
    pub candidate: PathBuf,
    /// Reference reading directory.
    #[arg(long, value_name = "DIR")]
    pub reference: Option<PathBuf>,
    /// New comparison directory.
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
}

/// Select explicit evidence pairs from one retained run.
#[derive(Args)]
pub struct ComparePairsArgs {
    /// Retained run directory.
    #[arg(long, value_name = "DIR")]
    pub run: PathBuf,
    /// Evidence pair to select.
    #[arg(long, value_name = "PAIR", value_parser = ["input", "reference", "both"])]
    pub against: String,
    /// New comparison directory.
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
}

#[derive(Subcommand)]
pub enum CompareCommand {
    /// Compare two completed image-domain runs.
    Runs(CompareRunsArgs),
    /// Compare two retained reading records.
    Readings(CompareReadingsArgs),
    /// Select explicit evidence pairs from one retained run.
    Pairs(ComparePairsArgs),
    /// Compatibility alias target inserted by the argv normalizer.
    #[command(hide = true)]
    Flat(EvaluationCompareArgs),
}
#[derive(Args)]
pub struct EvaluationCompareArgs {
    /// Baseline retained image-domain run or reading directory.
    #[arg(long, value_name = "DIR")]
    pub baseline: Option<PathBuf>,
    /// Candidate retained image-domain run or reading directory.
    #[arg(long, value_name = "DIR")]
    pub candidate: Option<PathBuf>,
    /// Reference retained reading directory.
    #[arg(long, value_name = "DIR")]
    pub reference_run: Option<PathBuf>,
    /// Single retained run directory.
    #[arg(long, value_name = "DIR")]
    pub run: Option<PathBuf>,
    /// Evidence pair to select.
    #[arg(long, value_name = "PAIR", value_parser = ["input", "reference", "both"])]
    pub against: Option<String>,
    /// New comparison directory.
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
    /// Version-comparison policy document.
    #[arg(long, value_name = "FILE")]
    pub policy: Option<PathBuf>,
    /// Fail when policy does not pass.
    #[arg(long)]
    pub gate: bool,
}
/// A stage a measurement can read.
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
    /// Native observation bundle directory holding the terminal manifest.
    #[arg(value_name = "INPUT", help_heading = "Export")]
    pub input: PathBuf,
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
