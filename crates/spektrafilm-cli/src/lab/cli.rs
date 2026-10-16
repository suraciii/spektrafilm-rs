//! Command-line surface of the lab binary's spectral group and measurement group.

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use spektrafilm_core::params::Tap;

/// Spectral diagnostics.
#[derive(Subcommand)]
pub enum SpectralCommand {
    /// Report the roundtrip error of the product path on measured spectra.
    Evaluate(EvaluateArgs),
}

/// Development measurement of the executed chain.
#[derive(Subcommand)]
pub enum MeasureCommand {
    /// Measure the real chain on a declared stimulus set.
    Run(MeasureRunArgs),
    /// Read declared sample points from an image or a retained stage capture.
    Sample(MeasureSampleArgs),
    /// Compare two retained measurements, sample by sample.
    Compare(MeasureCompareArgs),
    /// Re-resolve a recorded measurement's chain and report identity differences.
    Verify(MeasureVerifyArgs),
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
    /// Deterministic stochastic seed recorded with the measurement.
    #[arg(long, default_value_t = 0, help_heading = "Chain")]
    pub seed: u32,
    /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
    #[arg(long, default_value = "data", help_heading = "Chain")]
    pub data_dir: PathBuf,
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
    /// Illuminant pair REFERENCE=TARGET for the `spectra` entry.
    #[arg(long, value_name = "REFERENCE=TARGET", help_heading = "Stimulus")]
    pub condition: Option<String>,
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
pub struct MeasureSampleArgs {
    /// Raster image to read; requires the input declaration below.
    #[arg(long, value_name = "PATH", help_heading = "Source")]
    pub source: Option<PathBuf>,
    /// Retained stage-capture bundle to read instead of an image.
    #[arg(long, value_name = "DIR", help_heading = "Source")]
    pub from_evidence: Option<PathBuf>,
    /// Stage to read from the bundle; its declaration governs the reading.
    #[arg(long, value_enum, value_name = "STAGE", help_heading = "Source")]
    pub stage: Option<MeasureStage>,
    /// Source color space of the image, as `process` declares it.
    #[arg(long, value_name = "NAME", help_heading = "Source")]
    pub input_color_space: Option<String>,
    /// Whether the image's stored values are transfer-encoded.
    #[arg(long, value_name = "true|false", help_heading = "Source")]
    pub input_cctf_decoding: Option<bool>,
    /// Sample plan: explicit windows with stable sample identities.
    #[arg(long, value_name = "PATH", help_heading = "Measurement")]
    pub plan: PathBuf,
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
    /// Measurement directory to re-resolve.
    #[arg(long, value_name = "DIR", help_heading = "Verification")]
    pub run: PathBuf,
    /// Data directory to re-resolve against; defaults to the directory the run recorded.
    #[arg(long, default_value = "data", help_heading = "Verification")]
    pub data_dir: PathBuf,
}

#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("chain").required(true).multiple(false).args(["preset", "profile"])
))]
pub struct EvaluateArgs {
    /// Measured-spectra set JSON manifest; arrays live beside it as .npz/.npy.
    #[arg(long, value_name = "PATH", help_heading = "Evaluation")]
    pub spectra: PathBuf,
    /// Illuminant pair REFERENCE=TARGET: the film reference illuminant and
    /// the scene illuminant that produced the input RGB.
    #[arg(long, value_name = "REFERENCE=TARGET", help_heading = "Evaluation")]
    pub condition: String,
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
