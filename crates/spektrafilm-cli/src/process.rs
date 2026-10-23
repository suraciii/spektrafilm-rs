use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{ArgMatches, ValueEnum};
use serde_json::json;
use spektrafilm_core::ScannerProvenance;
use spektrafilm_core::image_io::{self, BitDepth, Compression, JpegSubsampling, SaveOptions};
use spektrafilm_core::params::Tap;
use spektrafilm_core::params_builder::resize_for_preview;
use spektrafilm_core::profile;
use spektrafilm_core::runtime::{DigestMode, Runtime};
use spektrafilm_math::image::ImageBuf;

use super::{
    Backend, DataSelection, DiscoveryFormat, preset, process_report, select_data_dir, telemetry,
};

#[derive(clap::Args)]
#[group(id = "Process")]
pub struct ProcessArgs {
    /// Compute backend; otherwise use SPEKTRAFILM_BACKEND/default selection.
    #[arg(long, value_enum, help_heading = "Execution and diagnostics")]
    pub(crate) backend: Option<Backend>,
    /// Input TIFF, EXR, PNG, JPEG, or camera RAW image.
    #[arg(help_heading = "Input and RAW loading")]
    pub(crate) input: PathBuf,
    /// Raster color space; discover with describe --field io.input_color_space.
    #[arg(long, help_heading = "Input and RAW loading")]
    pub(crate) input_color_space: Option<String>,
    /// Whether raster samples require transfer decoding (true or false).
    #[arg(long, action = clap::ArgAction::Set, help_heading = "Input and RAW loading")]
    pub(crate) input_cctf_decoding: Option<bool>,
    /// Destination TIFF, EXR, PNG, or JPEG image; optional only with --dry-run.
    #[arg(
        short,
        long,
        required_unless_present = "dry_run",
        help_heading = "Output"
    )]
    pub(crate) output: Option<PathBuf>,
    /// Output container; otherwise inferred from the destination extension.
    #[arg(long, value_enum, help_heading = "Output")]
    pub(crate) format: Option<OutputFormat>,
    /// Output sample depth: 8, 16, or 32 bits (JPEG/PNG require 8).
    #[arg(long, help_heading = "Output")]
    pub(crate) bit_depth: Option<u8>,
    /// JPEG quality from 1 to 100; applies only to JPEG.
    #[arg(long, help_heading = "Output")]
    pub(crate) jpeg_quality: Option<u8>,
    /// JPEG chroma subsampling; applies only to JPEG.
    #[arg(long, value_enum, help_heading = "Output")]
    pub(crate) jpeg_subsampling: Option<JpegSubsamplingArg>,
    /// TIFF/EXR compression (EXR requires zip).
    #[arg(long, value_enum, help_heading = "Output")]
    pub(crate) compression: Option<CompressionArg>,
    #[command(flatten)]
    pub(crate) workflow: WorkflowOptions,
    /// Film stock ID; discover with list-profiles. Required without --preset.
    #[arg(
        long,
        required_unless_present = "preset",
        help_heading = "Look and workflow"
    )]
    pub(crate) film: Option<String>,
    /// Print stock ID; otherwise uses film for direct scans or its target_print.
    #[arg(long, help_heading = "Look and workflow")]
    pub(crate) paper: Option<String>,
    /// Exact preset name or .toml/.json file; repeat to compose looks and sections.
    #[arg(long, action = clap::ArgAction::Append, help_heading = "Look and workflow")]
    pub(crate) preset: Vec<String>,
    /// Scan film directly, skipping printing; conflicts with --route.
    #[arg(long, conflicts_with = "route", help_heading = "Look and workflow")]
    pub(crate) scan_film: bool,
    /// Print per-stage timing information to stderr.
    #[arg(long, help_heading = "Execution and diagnostics")]
    pub(crate) timings: bool,
    /// Save a local JSON diagnostics report without replacing an existing file.
    #[arg(long, help_heading = "Execution and diagnostics")]
    pub(crate) diagnostics: Option<PathBuf>,
    /// Collect available GPU pass timings; requires --diagnostics.
    #[arg(
        long,
        requires = "diagnostics",
        help_heading = "Execution and diagnostics"
    )]
    pub(crate) gpu_timings: bool,
    /// Legacy RuntimeParams JSON snapshot used as the parameter baseline.
    #[arg(long, help_heading = "Look and workflow")]
    pub(crate) params: Option<PathBuf>,
    /// Apply a built-in feature profile; repeatable FAMILY=NAME.
    #[arg(long = "feature-profile", value_name = "FAMILY=NAME", action = clap::ArgAction::Append, help_heading = "Look and workflow")]
    pub(crate) feature_profiles: Vec<String>,
    /// Apply PATH=VALUE assignments or a sparse .toml/.json file in order.
    /// Strings need no JSON quotes; arrays use [1,2,3]; null clears optional values.
    #[arg(long = "set", value_name = "PATH=VALUE|FILE", action = clap::ArgAction::Append, help_heading = "Look and workflow")]
    pub(crate) sources: Vec<String>,
    #[arg(long, help_heading = "Execution and diagnostics")]
    pub(crate) dry_run: bool,
    /// Dry-run presentation: json or text; requires --dry-run.
    #[arg(
        long,
        value_enum,
        requires = "dry_run",
        help_heading = "Execution and diagnostics"
    )]
    pub(crate) dry_run_format: Option<DiscoveryFormat>,
    /// Deterministic stochastic seed, from 0 through 4294967295 (default: 0).
    #[arg(long, help_heading = "Execution and diagnostics")]
    pub(crate) seed: Option<u32>,
    /// Dump pre-saving output as HxWx3 channel-interleaved f64 values.
    #[arg(long, help_heading = "Output")]
    pub(crate) raw_out: Option<PathBuf>,
    /// Write a native stage-evidence bundle (terminal manifest and f64 stage
    /// buffers) into DIRECTORY; CPU reference execution only.
    #[arg(
        long,
        value_name = "DIRECTORY",
        help_heading = "Execution and diagnostics"
    )]
    pub(crate) stage_evidence: Option<PathBuf>,
    /// Run the pipeline this many times in one process (at least 1).
    #[arg(long, default_value = "1", help_heading = "Execution and diagnostics")]
    pub(crate) iters: usize,
    /// Data directory; otherwise use SPEKTRAFILM_DATA_DIR or automatic discovery.
    #[arg(
        long,
        default_value = "data",
        help_heading = "Execution and diagnostics"
    )]
    pub(crate) data_dir: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
    Jpeg,
    Png,
    Tiff,
    Exr,
}
impl OutputFormat {
    fn image_format(self) -> image_io::ImageFormat {
        match self {
            Self::Jpeg => image_io::ImageFormat::Jpeg,
            Self::Png => image_io::ImageFormat::Png,
            Self::Tiff => image_io::ImageFormat::Tiff,
            Self::Exr => image_io::ImageFormat::Exr,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        self.image_format().name()
    }
}

impl Default for ProcessArgs {
    /// The process surface's own defaults, so a caller that fills in only its
    /// own fields cannot drift from the documented baseline.
    fn default() -> Self {
        Self {
            backend: None,
            input: PathBuf::new(),
            input_color_space: None,
            input_cctf_decoding: None,
            output: None,
            format: None,
            bit_depth: None,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
            workflow: WorkflowOptions::default(),
            film: None,
            paper: None,
            preset: Vec::new(),
            scan_film: false,
            timings: false,
            diagnostics: None,
            gpu_timings: false,
            params: None,
            feature_profiles: Vec::new(),
            sources: Vec::new(),
            dry_run: false,
            dry_run_format: None,
            seed: None,
            raw_out: None,
            stage_evidence: None,
            iters: 1,
            data_dir: PathBuf::from("data"),
        }
    }
}

impl Default for WorkflowOptions {
    fn default() -> Self {
        Self {
            saving_color_space: None,
            saving_cctf_encoding: None,
            raw_white_balance: "as-shot".to_owned(),
            raw_temperature: None,
            raw_tint: None,
            lens_correction: false,
            route: None,
            scan_output: None,
            film_channel_swap: "none".to_owned(),
            print_channel_swap: "none".to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum JpegSubsamplingArg {
    #[value(name = "444")]
    Yuv444,
    #[value(name = "420")]
    Yuv420,
}

impl JpegSubsamplingArg {
    fn core(self) -> JpegSubsampling {
        match self {
            Self::Yuv444 => JpegSubsampling::Yuv444,
            Self::Yuv420 => JpegSubsampling::Yuv420,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum CompressionArg {
    Zip,
    None,
}

impl CompressionArg {
    fn core(self) -> Compression {
        match self {
            Self::Zip => Compression::Zip,
            Self::None => Compression::None,
        }
    }
}

/// Validated writer decision for one `process` invocation: container, depth,
/// codec options, and the saving space/transfer the writer describes. The
/// dry-run report, the saving telemetry and the writer all consume this one
/// decision, so a rule added for a container cannot land in only one of them.
struct OutputPlan {
    format: OutputFormat,
    depth: BitDepth,
    jpeg_quality: Option<u8>,
    jpeg_subsampling: Option<JpegSubsampling>,
    compression: Option<Compression>,
    saving_space: String,
    saving_encoded: bool,
}

impl OutputPlan {
    /// Resolve the container from the flag/extension pair and reject codec
    /// options the selected container does not carry.
    fn from_flags(
        output: &Path,
        format: Option<OutputFormat>,
        bit_depth: Option<u8>,
        jpeg_quality: Option<u8>,
        jpeg_subsampling: Option<JpegSubsamplingArg>,
        compression: Option<CompressionArg>,
    ) -> Result<Self> {
        let extension_format = image_io::ImageFormat::detect(output)
            .with_context(|| format!("detecting output format: {}", output.display()))?;
        let format = format.unwrap_or(match extension_format {
            image_io::ImageFormat::Jpeg => OutputFormat::Jpeg,
            image_io::ImageFormat::Png => OutputFormat::Png,
            image_io::ImageFormat::Tiff => OutputFormat::Tiff,
            image_io::ImageFormat::Exr => OutputFormat::Exr,
        });
        if format.image_format() != extension_format {
            bail!(
                "--format {} disagrees with output extension ({})",
                format.name(),
                extension_format.name()
            );
        }
        Self::from_format(
            format,
            bit_depth,
            jpeg_quality,
            jpeg_subsampling,
            compression,
        )
    }

    /// Resolve a declared container when dry-run has no destination path.
    fn from_format(
        format: OutputFormat,
        bit_depth: Option<u8>,
        jpeg_quality: Option<u8>,
        jpeg_subsampling: Option<JpegSubsamplingArg>,
        compression: Option<CompressionArg>,
    ) -> Result<Self> {
        let default_depth = match format {
            OutputFormat::Jpeg | OutputFormat::Png => 8,
            OutputFormat::Tiff | OutputFormat::Exr => 16,
        };
        let depth = BitDepth::try_from(bit_depth.unwrap_or(default_depth))?;
        // The container rules live in core; the saving-space rules are checked
        // once the transfer resolves, so only the container variants are read.
        let options = SaveOptions {
            depth,
            color_space: "sRGB",
            cctf_encoding: true,
            jpeg_quality,
            jpeg_subsampling: jpeg_subsampling.map(JpegSubsamplingArg::core),
            compression: compression.map(CompressionArg::core),
        };
        use spektrafilm_core::image_io::SaveOptionViolation as V;
        for violation in image_io::save_option_violations(format.image_format(), &options, false) {
            match violation {
                V::Depth if format == OutputFormat::Exr => {
                    bail!("EXR output requires 16-bit or 32-bit depth")
                }
                V::Depth => bail!("{} output requires 8-bit depth", format.name()),
                V::JpegQualityRange => bail!("JPEG quality must be in 1..=100"),
                V::JpegQualityNotApplicable => {
                    bail!("--jpeg-quality is only valid for JPEG output")
                }
                V::JpegSubsamplingNotApplicable => {
                    bail!("--jpeg-subsampling is only valid for JPEG output")
                }
                V::CompressionNotConfigurable => {
                    bail!("--compression is only valid for TIFF or EXR output")
                }
                V::RequiresEncoded | V::RequiresLinear | V::ExrColorSpace | V::ExrCompression => {}
            }
        }
        Ok(Self {
            format,
            depth,
            jpeg_quality: matches!(format, OutputFormat::Jpeg).then(|| jpeg_quality.unwrap_or(95)),
            jpeg_subsampling: matches!(format, OutputFormat::Jpeg).then(|| {
                jpeg_subsampling
                    .unwrap_or(JpegSubsamplingArg::Yuv444)
                    .core()
            }),
            compression: matches!(format, OutputFormat::Tiff | OutputFormat::Exr)
                .then(|| compression.unwrap_or(CompressionArg::Zip).core()),
            saving_space: String::new(),
            saving_encoded: false,
        })
    }

    /// Validate options that cannot be interpreted without a container.
    fn validate_unresolved(
        bit_depth: Option<u8>,
        jpeg_quality: Option<u8>,
        jpeg_subsampling: Option<JpegSubsamplingArg>,
        compression: Option<CompressionArg>,
    ) -> Result<Option<BitDepth>> {
        if jpeg_quality.is_some() {
            bail!("--jpeg-quality requires --format when --output is omitted");
        }
        if jpeg_subsampling.is_some() {
            bail!("--jpeg-subsampling requires --format when --output is omitted");
        }
        if compression.is_some() {
            bail!("--compression requires --format when --output is omitted");
        }
        bit_depth
            .map(BitDepth::try_from)
            .transpose()
            .map_err(anyhow::Error::from)
    }

    /// Adopt the saving color space/transfer this container writes. Kept
    /// separate from [`Self::validate_saving`] so the color spaces resolve —
    /// and fail — in the same order every writer flag was checked before.
    fn with_saving(
        mut self,
        workflow: &WorkflowOptions,
        output_color_space: &str,
        output_cctf_encoding: bool,
    ) -> Self {
        self.saving_space = workflow.saving_color_space.clone().unwrap_or_else(|| {
            if self.format == OutputFormat::Exr {
                "ACES2065-1".into()
            } else {
                output_color_space.to_owned()
            }
        });
        self.saving_encoded = workflow
            .saving_cctf_encoding
            .unwrap_or(self.format != OutputFormat::Exr && output_cctf_encoding);
        self
    }

    /// Reject the saving/container combinations the selected container cannot
    /// represent.
    fn validate_saving(&self) -> Result<()> {
        use spektrafilm_core::image_io::SaveOptionViolation as V;
        let violations = image_io::save_option_violations(
            self.format.image_format(),
            &self.save_options(),
            false,
        );
        for violation in violations {
            match violation {
                V::RequiresEncoded => {
                    bail!(
                        "{} output requires encoded saving output",
                        self.format.name()
                    )
                }
                V::RequiresLinear => bail!("EXR output requires linear saving output"),
                V::ExrColorSpace => bail!("EXR color space must be sRGB or ACES2065-1"),
                V::ExrCompression => bail!("EXR compression is always zip"),
                _ => {}
            }
        }
        Ok(())
    }

    /// Writer options describing the pixels the saving conversion produces.
    fn save_options(&self) -> SaveOptions<'_> {
        SaveOptions {
            depth: self.depth,
            color_space: &self.saving_space,
            cctf_encoding: self.saving_encoded,
            jpeg_quality: self.jpeg_quality,
            jpeg_subsampling: self.jpeg_subsampling,
            compression: self.compression,
        }
    }
}
#[derive(clap::Args)]
pub(crate) struct WorkflowOptions {
    /// Writer-stage color conversion; defaults to the scanner color space and
    /// ACES2065-1 for EXR. Names are shared with the scanner registry: discover
    /// them with `describe --field io.output_color_space`.
    #[arg(long, help_heading = "Output")]
    pub(crate) saving_color_space: Option<String>,
    /// Writer-stage transfer-function encoding; EXR requires false, JPEG/PNG true.
    #[arg(long, action = clap::ArgAction::Set, help_heading = "Output")]
    pub(crate) saving_cctf_encoding: Option<bool>,
    /// RAW loading white balance; custom requires --raw-temperature.
    #[arg(long, default_value = "as-shot", value_parser = ["as-shot", "daylight", "tungsten", "custom"], help_heading = "Input and RAW loading")]
    pub(crate) raw_white_balance: String,
    /// Custom RAW white-balance temperature in Kelvin.
    #[arg(
        long,
        allow_negative_numbers = true,
        help_heading = "Input and RAW loading"
    )]
    pub(crate) raw_temperature: Option<f64>,
    /// Green-channel tint multiplier for custom RAW white balance; 1 is neutral.
    /// Requires --raw-white-balance custom.
    #[arg(
        long,
        allow_negative_numbers = true,
        help_heading = "Input and RAW loading"
    )]
    pub(crate) raw_tint: Option<f64>,
    /// Apply lens correction while loading RAW input.
    #[arg(long, help_heading = "Input and RAW loading")]
    pub(crate) lens_correction: bool,
    /// Workflow route; this help lists the supported values.
    #[arg(long, help_heading = "Look and workflow")]
    pub(crate) route: Option<String>,
    /// Film scanner output mode; applied after parameter sources.
    #[arg(long, value_parser = ["direct_scan", "negative_to_positive"], help_heading = "Look and workflow")]
    pub(crate) scan_output: Option<String>,
    /// Film density channel order: none or three comma-separated indices in 0..2.
    #[arg(long, default_value = "none", help_heading = "Look and workflow")]
    pub(crate) film_channel_swap: String,
    /// Print density channel order: none or three comma-separated indices in 0..2.
    #[arg(long, default_value = "none", help_heading = "Look and workflow")]
    pub(crate) print_channel_swap: String,
}

pub fn run(mut args: ProcessArgs, command_matches: &ArgMatches) -> Result<()> {
    // Per-stage timings are read from stage functions that may run on rayon
    // workers, so install the switch here instead of mutating the process
    // environment after the pool exists.
    spektrafilm_backend::timing::set_stage_timings(args.timings);
    let data_dir = std::mem::take(&mut args.data_dir);
    let data = select_data_dir(data_dir, command_matches)?;
    run_with_data_dir(args, data)
}

/// Run the process pipeline with an already-resolved data directory.
/// Shared with the lab module, which cannot construct `ArgMatches`.
pub(crate) fn run_with_data_dir(args: ProcessArgs, data: DataSelection) -> Result<()> {
    // Protect every bundle file, including publication temporaries, from
    // invocation inputs, output/raw-out and optional diagnostics.
    let evidence_paths = args
        .stage_evidence
        .as_deref()
        .map(|directory| {
            let mut paths = vec![
                directory.join("stage-evidence.json"),
                directory.join("stage-evidence.json.tmp"),
            ];
            for stage in spektrafilm_core::stage_evidence::StageId::ORDER {
                paths.push(directory.join(format!("{}.f64", stage.id())));
                paths.push(directory.join(format!("{}.f64.tmp", stage.id())));
            }
            paths
        })
        .unwrap_or_default();
    let mut protected = vec![args.input.as_path()];
    if let Some(output) = args.output.as_deref() {
        protected.push(output);
    }
    protected.extend(args.params.as_deref());
    protected.extend(args.raw_out.as_deref());
    protected.extend(
        args.preset
            .iter()
            .filter(|selector| selector.ends_with(".toml") || selector.ends_with(".json"))
            .map(Path::new),
    );
    protected.extend(
        args.sources
            .iter()
            .filter(|source| source.ends_with(".toml") || source.ends_with(".json"))
            .map(Path::new),
    );
    let invocation_path_count = protected.len();
    protected.extend(evidence_paths.iter().map(PathBuf::as_path));
    telemetry::run(
        spektrafilm_core::telemetry::OperationKind::Process,
        args.diagnostics.as_deref(),
        args.gpu_timings,
        &protected,
        telemetry::requested_backend(args.backend),
        |invocation| {
            cmd_process(
                &args,
                &data,
                invocation,
                &protected[..invocation_path_count],
                &evidence_paths,
            )
        },
    )?;
    Ok(())
}

fn cmd_process(
    args: &ProcessArgs,
    data: &DataSelection,
    invocation: &mut telemetry::Invocation,
    protected: &[&Path],
    evidence_paths: &[PathBuf],
) -> Result<()> {
    let input = args.input.as_path();
    let output = args.output.as_deref();
    let workflow = &args.workflow;
    let format = args.format;
    let bit_depth = args.bit_depth;
    let jpeg_quality = args.jpeg_quality;
    let jpeg_subsampling = args.jpeg_subsampling;
    let compression = args.compression;
    let film_name = args.film.as_deref();
    let paper_name = args.paper.as_deref();
    let selectors = &args.preset;
    let scan_film = args.scan_film;
    let show_timings = args.timings;
    let params_file = args.params.as_deref();
    let feature_profile_values = args
        .feature_profiles
        .iter()
        .enumerate()
        .map(|(index, value)| {
            spektrafilm_core::feature_profiles::parse_selection(value, index + 1)
                .map_err(anyhow::Error::msg)
        })
        .collect::<Result<Vec<_>>>()?;
    let sources = &args.sources;
    let dry_run = args.dry_run;
    let raw_out = args.raw_out.as_deref();
    let stage_evidence = args.stage_evidence.as_deref();
    let iters = args.iters;
    let backend_choice = args.backend;
    let data_dir = &data.path;
    let total_start = Instant::now();
    use spektrafilm_core::telemetry::{IssueBoundary, IssueCategory};
    let context = invocation.operation.context();
    if iters == 0 {
        bail!("--iters must be at least 1");
    }
    let (mut plan, unresolved_depth) = match output {
        Some(output) => (
            Some(OutputPlan::from_flags(
                output,
                format,
                bit_depth,
                jpeg_quality,
                jpeg_subsampling,
                compression,
            )?),
            None,
        ),
        None => match format {
            Some(format) => (
                Some(OutputPlan::from_format(
                    format,
                    bit_depth,
                    jpeg_quality,
                    jpeg_subsampling,
                    compression,
                )?),
                None,
            ),
            None => (
                None,
                OutputPlan::validate_unresolved(
                    bit_depth,
                    jpeg_quality,
                    jpeg_subsampling,
                    compression,
                )?,
            ),
        },
    };
    invocation.boundary(IssueCategory::Input, IssueBoundary::InputLoad);
    if !input.is_file() {
        bail!("input is not a regular file: {}", input.display());
    }
    std::fs::File::open(input)
        .with_context(|| format!("input is not readable: {}", input.display()))?;
    let input_is_raw = image_io::is_raw(input);
    if !input_is_raw {
        image_io::ImageFormat::detect(input)
            .with_context(|| format!("unsupported input format: {}", input.display()))?;
    }
    let raw_options = raw_options(workflow);
    if input_is_raw {
        validate_raw_options(&raw_options)?;
    }
    invocation.boundary(IssueCategory::Configuration, IssueBoundary::RuntimePrepare);
    let library = spektrafilm_core::presets::PresetLibrary::default();
    let presets = selectors
        .iter()
        .map(|selector| preset::select(&library, selector).map(|selected| selected.preset))
        .collect::<Result<Vec<_>>>()?;
    let resolved = spektrafilm_core::process_params::resolve(
        spektrafilm_core::process_params::ProcessParamsRequest {
            film: film_name,
            paper: paper_name,
            presets: &presets,
            params_file,
            sources,
            feature_profiles: &feature_profile_values,
            route: workflow.route.as_deref(),
            scan_film,
            scan_output: workflow.scan_output.as_deref(),
            input_is_raw,
            input_color_space: args.input_color_space.as_deref(),
            input_cctf_decoding: args.input_cctf_decoding,
            require_input_declaration: std::env::var_os("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
                .as_deref()
                != Some(std::ffi::OsStr::new("1")),
            digest_mode: if std::env::var_os("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS").as_deref()
                == Some(std::ffi::OsStr::new("1"))
            {
                DigestMode::PreserveUserEdits
            } else {
                DigestMode::ApplyStockSpecifics
            },
        },
        data_dir,
    )
    .map_err(anyhow::Error::msg)?;
    let mut film = resolved.film;
    let mut print = resolved.print;
    apply_channel_swap(&mut film, &workflow.film_channel_swap)?;
    apply_channel_swap(&mut print, &workflow.print_channel_swap)?;
    let mut params = resolved.params;
    params.random_seed = u64::from(args.seed.unwrap_or(0));
    process_report::warn_edits(&resolved.edits);
    if stage_evidence.is_some() {
        // Configuration-level rejection, before dry run or rendering: the
        // capture route must not run a configuration it cannot describe.
        spektrafilm_core::stage_evidence::ensure_supported_params(&params)
            .map_err(anyhow::Error::msg)?;
    }
    let inject = params
        .taps
        .inject
        .as_deref()
        .map(Tap::parse)
        .transpose()
        .map_err(anyhow::Error::msg)
        .with_context(|| "params taps.inject")?;
    let collect = params
        .taps
        .collect
        .as_deref()
        .map(Tap::parse)
        .transpose()
        .map_err(anyhow::Error::msg)
        .with_context(|| "params taps.collect")?;

    let output_color_space = params.io.output_color_space.clone();
    let output_cctf_encoding = params.io.output_cctf_encoding;
    if let Some(resolved_plan) = plan.take() {
        let resolved_plan =
            resolved_plan.with_saving(workflow, &output_color_space, output_cctf_encoding);
        let saving_space_resolved =
            spektrafilm_math::colorspace::resolve(&resolved_plan.saving_space).map_err(|e| {
                anyhow::anyhow!(
                    "unsupported saving color space {:?}: {e}",
                    resolved_plan.saving_space
                )
            })?;
        let output_space_resolved = spektrafilm_math::colorspace::resolve(&output_color_space)
            .map_err(|e| {
                anyhow::anyhow!("unsupported output color space {output_color_space:?}: {e}")
            })?;
        let _conversion_matrix = spektrafilm_math::colorspace::conversion_matrix(
            output_space_resolved,
            saving_space_resolved,
        );
        resolved_plan.validate_saving()?;
        plan = Some(resolved_plan);
    } else if let Some(saving_space) = workflow.saving_color_space.as_deref() {
        spektrafilm_math::colorspace::resolve(saving_space)
            .map_err(|e| anyhow::anyhow!("unsupported saving color space {saving_space:?}: {e}"))?;
    }
    // Validate configuration without creating a compute device for dry runs.
    spektrafilm_core::stages::grain_v3::validate_supported(&film, &params)
        .map_err(anyhow::Error::msg)?;
    let grain_v3_active = spektrafilm_core::stages::grain_v3::active(&params);
    let grain_fotufilm = spektrafilm_core::stages::grain_fotufilm::configuration_report(&params);
    if dry_run {
        let grain_v3 = json!({
            "active": grain_v3_active,
            "effective_backend": null,
            "note": grain_v3_active.then_some(
                "Dye executor resolves when rendering; compute-device availability is unchecked."
            ),
        });
        let backend_env = std::env::var("SPEKTRAFILM_BACKEND").ok();
        let output_format = plan.as_ref().map(|plan| plan.format.name());
        let output_bit_depth = plan
            .as_ref()
            .map(|plan| plan.depth.bits())
            .or_else(|| unresolved_depth.map(|depth| depth.bits()));
        let output_color_space = plan
            .as_ref()
            .map(|plan| plan.saving_space.clone())
            .or_else(|| workflow.saving_color_space.clone());
        let output_cctf_encoding = plan
            .as_ref()
            .map(|plan| plan.saving_encoded)
            .or(workflow.saving_cctf_encoding);
        let output_compression = plan.as_ref().and_then(|plan| {
            plan.compression.map(|compression| match compression {
                Compression::Zip => "zip",
                Compression::None => "none",
            })
        });
        let output_jpeg_quality = plan.as_ref().and_then(|plan| plan.jpeg_quality);
        let output_jpeg_subsampling = plan.as_ref().and_then(|plan| {
            plan.jpeg_subsampling.map(|subsampling| match subsampling {
                JpegSubsampling::Yuv444 => "444",
                JpegSubsampling::Yuv420 => "420",
            })
        });
        let mut unchecked = vec![
            "pixel_decode",
            "compute_device",
            "metered_exposure",
            "output_geometry",
        ];
        if plan.is_none() {
            unchecked.push("output_format");
        }
        let capture_requested = params.camera.capture_illuminant.clone();
        let capture_effective = match film.info.reference_illuminant.as_str() {
            reference if capture_requested.is_none() => reference.to_owned(),
            _ => capture_requested
                .clone()
                .expect("explicit capture illuminant"),
        };
        let capture_active = matches!(
            params.workflow.route_enum(),
            spektrafilm_core::WorkflowRoute::FilmScan
                | spektrafilm_core::WorkflowRoute::FilmPrintScan
        );
        let capture_source = if capture_requested.is_none() {
            "unset"
        } else if resolved
            .edits
            .iter()
            .any(|edit| edit.path == "camera.capture_illuminant" && edit.source.kind == "set")
        {
            "set"
        } else {
            "preset"
        };
        let capture_illuminant = json!({
            "requested": capture_requested,
            "effective": capture_active.then_some(capture_effective),
            "source": capture_source,
            "profile_reference_illuminant": film.info.reference_illuminant,
            "status": if capture_active { "active" } else { "inactive" },
            "reason": (!capture_active).then_some("route computes no film exposure from scene RGB")
        });
        let scan_profile = if params.io.scan_film { &film } else { &print };
        let readout_illuminant = json!({
            "effective": scan_profile.readout_illuminant(),
            "rule": scan_profile.readout_rule(),
        });
        let feature_profile_report = serde_json::to_value(&resolved.feature_profiles)?;
        let report = json!({
            "feature_profiles": feature_profile_report,
            "data": {
                "path": data.path,
                "source": data.source,
            },
            "film_profile": resolved.film_name,
            "print_profile": resolved.print_name,
            "capture_illuminant": capture_illuminant,
            "readout_illuminant": readout_illuminant,
            "parameters": params,
            "scanner_provenance": ScannerProvenance::from_params(&params),
            "film_grain_dye": grain_v3,
            "film_grain_fotufilm": grain_fotufilm,
            "input": {
                "kind": if input_is_raw { "raw" } else { "raster" },
                "declaration": resolved.input_declaration,
                "raw_loading": input_is_raw.then(|| json!({
                    "white_balance": workflow.raw_white_balance,
                    "temperature": raw_options.temperature,
                    "tint": raw_options.tint,
                    "lens_correction": raw_options.lens_correction
                }))
            },
            "output": {
                "path": output,
                "format": output_format,
                "bit_depth": output_bit_depth,
                "color_space": output_color_space,
                "cctf_encoding": output_cctf_encoding,
                "compression": output_compression,
                "jpeg_quality": output_jpeg_quality,
                "jpeg_subsampling": output_jpeg_subsampling
            },
            "backend": {
                "requested": backend_choice.map(|b| match b { Backend::Cpu => "cpu", Backend::Gpu => "gpu" }),
                "policy": backend_choice.map(|b| match b { Backend::Cpu => "cpu", Backend::Gpu => "gpu" }).unwrap_or("auto"),
                "environment": backend_env
            }
        });
        let mut report = report;
        report["seed"] = json!(params.random_seed);
        report["edits"] = serde_json::to_value(&resolved.edits)?;
        report["unchecked"] = json!(unchecked);
        match args.dry_run_format.unwrap_or(DiscoveryFormat::Json) {
            DiscoveryFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
            DiscoveryFormat::Text => process_report::print_text(&report),
        }
        return Ok(());
    }
    if let Some(directory) = stage_evidence {
        // Prepare the bundle destination before rendering: the terminal
        // manifest is the completeness token, so a stale one from an earlier
        // run must not survive into a run that fails before publication.
        std::fs::create_dir_all(directory).with_context(|| {
            format!("creating stage-evidence directory: {}", directory.display())
        })?;
        let manifest = directory.join("stage-evidence.json");
        spektrafilm_core::telemetry::validate_report_destination(&manifest, protected)
            .with_context(|| {
                format!(
                    "stage-evidence destination conflicts: {}",
                    manifest.display()
                )
            })?;
        match std::fs::remove_file(&manifest) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "removing stale stage-evidence manifest: {}",
                        manifest.display()
                    )
                });
            }
        }
        for path in evidence_paths.iter().skip(1) {
            spektrafilm_core::telemetry::validate_report_destination(path, protected)
                .with_context(|| {
                    format!("stage-evidence destination conflicts: {}", path.display())
                })?;
        }
    }
    let backend = invocation.phase(
        "backend_init",
        IssueCategory::Backend,
        IssueBoundary::BackendInit,
        &context,
        |backend_context| {
            let backend: Box<dyn spektrafilm_backend::ComputeBackend> = match backend_choice {
                Some(Backend::Cpu) => Box::new(spektrafilm_backend::cpu_backend::CpuBackend),
                Some(Backend::Gpu) => Box::new(
                    spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit().ok_or_else(
                        || anyhow::anyhow!("GPU backend unavailable; use --backend cpu"),
                    )?,
                ),
                None => spektrafilm_gpu::select_backend(),
            };
            telemetry::observe_backend(backend.as_ref(), backend_context);
            Ok(backend)
        },
    )?;
    eprintln!("Backend: {}", backend.name());
    if stage_evidence.is_some() {
        spektrafilm_core::stage_evidence::ensure_supported_backend(backend.as_ref())
            .map_err(anyhow::Error::msg)?;
    }
    if grain_v3_active {
        eprintln!(
            "Backend (effective): {}",
            spektrafilm_core::stages::grain_v3::effective_backend_note(
                &film,
                &params,
                backend.is_gpu(),
            )
        );
    }

    let t = Instant::now();
    let (image, metadata) = invocation.phase(
        "input_load",
        IssueCategory::Input,
        IssueBoundary::InputLoad,
        &context,
        |_| {
            let (image, metadata) = if input_is_raw {
                let options = &raw_options;
                (
                    spektrafilm_raw::load(input, options)
                        .map_err(anyhow::Error::msg)?
                        .image,
                    image_io::read_metadata(input),
                )
            } else {
                let loaded = image_io::load(input)
                    .with_context(|| format!("loading image: {}", input.display()))?;
                (loaded.image, loaded.metadata)
            };
            Ok((image, metadata))
        },
    )?;
    invocation
        .operation
        .set_input_dimensions(image.width, image.height);
    // Standard setup measures the full working frame. Its preview reduction
    // runs only after the nonlinear scan and output tail.
    let image = invocation.phase(
        "input_prepare",
        IssueCategory::Input,
        IssueBoundary::InputPrepare,
        &context,
        |_| {
            let image = if params.settings.preview_mode && !params.scanner.requires_full_frame() {
                let resized = resize_for_preview(&image, params.settings.preview_max_size);
                eprintln!(
                    "Preview: {}x{} → {}x{} (max edge {})",
                    image.width,
                    image.height,
                    resized.width,
                    resized.height,
                    params.settings.preview_max_size
                );
                resized
            } else {
                image
            };
            Ok(image)
        },
    )?;
    eprintln!(
        "Image loaded: {}x{} ({} MP), {} ms",
        image.width,
        image.height,
        (image.pixel_count() as f64 / 1e6 * 10.0).round() / 10.0,
        t.elapsed().as_millis()
    );

    // Run the calibrated runtime — a missing spectral LUT is a hard error.
    let t = Instant::now();
    let runtime = invocation.phase(
        "runtime_prepare",
        IssueCategory::Configuration,
        IssueBoundary::RuntimePrepare,
        &context,
        |prepare_context| {
            Runtime::new_observed(film, print, params, data_dir, prepare_context).map_err(|e| {
                anyhow::anyhow!(
                    "spectral pipeline construction failed: {e} — the simplified no-LUT fallback \
                     was removed; check that the profiles/data directory contains the spectral \
                     LUTs (looked under {})",
                    data_dir.display()
                )
            })
        },
    )?;
    if context.enabled() {
        invocation
            .operation
            .set_render_configuration(runtime.telemetry_configuration());
    }
    let output_attempt = std::cell::Cell::new(0);
    let mut stage_capture = stage_evidence.map(spektrafilm_core::stage_evidence::StageCapture::new);
    let mut run_once = |image: ImageBuf, index: u64| -> Result<ImageBuf> {
        let (attempt_id, result) =
            telemetry::simulation_attempt(invocation, index, |simulation_context| {
                let out = if let Some(capture) =
                    stage_capture.as_mut().filter(|_| index == iters as u64)
                {
                    // Stage evidence observes the published run only: with
                    // --iters the warm-cache iterations are not the captured
                    // execution.
                    runtime
                        .process_captured(
                            image,
                            backend.as_ref(),
                            inject,
                            collect,
                            simulation_context,
                            capture,
                        )
                        .map_err(|e| anyhow::anyhow!("pipeline stage evidence: {e}"))?
                } else if inject.is_none() && collect.is_none() {
                    runtime
                        .process_observed(image, backend.as_ref(), simulation_context)
                        .map_err(anyhow::Error::msg)?
                } else {
                    runtime
                        .process_with_taps_observed(
                            image,
                            backend.as_ref(),
                            inject,
                            collect,
                            simulation_context,
                        )
                        .map_err(|e| anyhow::anyhow!("pipeline taps: {e}"))?
                };
                if runtime.params().settings.preview_mode
                    && runtime.params().scanner.requires_full_frame()
                    && collect.is_none_or(|tap| tap == Tap::RgbOut)
                {
                    Ok(resize_for_preview(
                        &out,
                        runtime.params().settings.preview_max_size,
                    ))
                } else {
                    Ok(out)
                }
            });
        if result.is_ok() {
            output_attempt.set(attempt_id);
        }
        result
    };
    let result = if iters > 1 {
        // Warm-cache bench: process the image `iters` times in the same backend
        // instance. The first iteration pays shader-compile cost; subsequent ones
        // hit the cache and reflect "GUI live preview" performance.
        let mut last = run_once(image.clone(), 1)?;
        let iter1_ms = t.elapsed().as_millis();
        eprintln!("Pipeline (iter 1): {} ms (cold)", iter1_ms);
        for i in 2..=iters {
            let ti = Instant::now();
            last = run_once(image.clone(), i as u64)?;
            eprintln!(
                "Pipeline (iter {}): {} ms (warm)",
                i,
                ti.elapsed().as_millis()
            );
        }
        last
    } else {
        let r = run_once(image, 1)?;
        eprintln!("Pipeline: {} ms", t.elapsed().as_millis());
        r
    };
    let plan = plan.context("internal error: missing output plan for rendering")?;
    let output = output.context("internal error: missing output path for rendering")?;

    // Save output
    let t = Instant::now();
    invocation.operation.set_saved_attempt(output_attempt.get());
    let options = plan.save_options();
    invocation.operation.set_saving_configuration(
        spektrafilm_core::telemetry::SavingConfiguration::from_save_options(
            plan.format.image_format(),
            &options,
        ),
    );
    let saving_image = invocation.phase(
        "saving_convert",
        IssueCategory::Writing,
        IssueBoundary::SavingConvert,
        &context,
        |_| {
            image_io::convert_image(
                &result,
                &output_color_space,
                output_cctf_encoding,
                &plan.saving_space,
                plan.saving_encoded,
            )
            .map_err(anyhow::Error::from)
        },
    )?;
    let prepared_image = invocation.phase(
        "file_write",
        IssueCategory::Writing,
        IssueBoundary::FileWrite,
        &context,
        |write_context| {
            image_io::prepare_save_observed(
                output,
                &saving_image,
                options,
                metadata.as_ref(),
                write_context,
            )
            .with_context(|| format!("saving image: {}", output.display()))
        },
    )?;
    let report = prepared_image
        .publish(true)
        .map_err(|e| anyhow::Error::msg(e.to_string()))?;
    if let Some(warning) = report.metadata_warning {
        eprintln!("Warning: {warning}");
    }
    invocation
        .operation
        .set_published_dimensions(result.width as u32, result.height as u32);
    eprintln!(
        "Saved: {} ({}x{}), {} ms",
        output.display(),
        result.width,
        result.height,
        t.elapsed().as_millis()
    );

    // Optional raw f64 dump for bit-exact parity comparison.
    if let Some(p) = raw_out {
        invocation.phase(
            "file_write",
            IssueCategory::Writing,
            IssueBoundary::FileWrite,
            &context,
            |_| {
                use std::io::Write;
                let buf: Vec<f64> = result.data.iter().map(|&v| v as f64).collect();
                let bytes = bytemuck_cast_f64_to_bytes(&buf);
                let mut f = std::fs::File::create(p)
                    .with_context(|| format!("creating raw_out file: {}", p.display()))?;
                f.write_all(bytes)?;
                eprintln!("raw f64 buffer: {} ({} values)", p.display(), buf.len());
                Ok(())
            },
        )?;
    }

    // The stage-evidence bundle publishes last: its terminal manifest is the
    // completeness token, so an output or capture failure can never leave
    // complete-looking evidence behind. An incomplete bundle is a command
    // failure, distinct from optional telemetry persistence.
    if let Some(capture) = stage_capture.as_mut() {
        invocation.phase(
            "file_write",
            IssueCategory::Writing,
            IssueBoundary::FileWrite,
            &context,
            |_| {
                capture
                    .publish()
                    .map_err(|e| anyhow::anyhow!("stage evidence: {e}"))?;
                eprintln!(
                    "stage evidence: {}",
                    capture.destination().join("stage-evidence.json").display()
                );
                Ok(())
            },
        )?;
    }

    if show_timings {
        eprintln!("Total: {} ms", total_start.elapsed().as_millis());
    }

    Ok(())
}

fn bytemuck_cast_f64_to_bytes(v: &[f64]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

fn raw_options(workflow: &WorkflowOptions) -> spektrafilm_raw::RawOptions {
    spektrafilm_raw::RawOptions {
        white_balance: match workflow.raw_white_balance.as_str() {
            "daylight" => spektrafilm_raw::WhiteBalance::Daylight,
            "tungsten" => spektrafilm_raw::WhiteBalance::Tungsten,
            "custom" => spektrafilm_raw::WhiteBalance::Custom,
            _ => spektrafilm_raw::WhiteBalance::AsShot,
        },
        temperature: workflow.raw_temperature,
        tint: workflow.raw_tint,
        lens_correction: workflow.lens_correction,
    }
}

fn validate_raw_options(options: &spektrafilm_raw::RawOptions) -> Result<()> {
    options.validate().map_err(anyhow::Error::from)
}

fn apply_channel_swap(profile: &mut profile::Profile, selection: &str) -> Result<()> {
    if selection == "none" {
        return Ok(());
    }
    let order: Vec<usize> = selection
        .split(',')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()
        .with_context(|| "channel swap must contain three indices such as 2,1,0")?;
    if order.len() != 3 || order.iter().any(|&channel| channel > 2) {
        bail!("channel swap must contain three indices in 0..2");
    }
    for row in &mut profile.data.channel_density {
        if row.len() >= 3 {
            let original = [row[0], row[1], row[2]];
            for channel in 0..3 {
                row[channel] = original[order[channel]];
            }
        }
    }
    Ok(())
}
