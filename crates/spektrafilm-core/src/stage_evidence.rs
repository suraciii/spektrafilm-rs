//! Instrument-owned native stage observations.
//!
//! One `StageCapture` records the boundary values of one executed render and
//! publishes them as a bundle: one f64 buffer per available stage plus the
//! terminal `stage-evidence.json` manifest. The capture observes the values
//! that actually flowed through the executed route — metered exposure, the
//! prepared print normalization and the named tap boundaries — and never
//! re-implements pipeline mathematics.
//!
//! Channel-axis interpretation: every film/print boundary shares one record
//! axis. `filming::develop` passes `log_raw[c]` into the density curve of
//! `density_cmy[c]` without permutation, so `log_e_film` and `cmy_film` carry
//! the same per-channel order — the cyan-/magenta-/yellow-forming record axis,
//! declared `"CMY"`. `log_e_*` buffers hold log10 record exposure (before
//! development), `cmy_*` buffers the developed record density; neither is an
//! image color space and consumers must not permute channels across stages.
//!
//! Invariants:
//! - Buffers are written as they are captured; `stage-evidence.json` is
//!   written last and atomically, so a manifest in the destination is the
//!   completeness token for the run that produced it.
//! - A stage that the route did not execute is `not_applicable` with a stable
//!   reason and carries no buffer. A required executed stage that failed to
//!   capture or write is `unavailable`, records an error, and makes the whole
//!   bundle `execution_status: "failed"`.
//! - Capture never mutates the render: it writes copies of boundary buffers
//!   and fails visibly instead of substituting values.
//!
//! The terminal CMYK ink style appearance is not a spectral stage: it runs
//! after `rgb_out` collection, before output mix and publication, on the same
//! three-channel RGB buffer. Its evidence is therefore an independent
//! `terminal_cmyk` manifest record, not a seventh [`StageId`] and not a
//! seventh tap boundary. The record names the canonical `style`/`model`/
//! `strength` and the explicit post-`rgb_out` boundary, and is published only
//! when the runtime actually executed the stage.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{Scalar, to_f64};

use crate::WorkflowRoute;
use crate::params::{RuntimeParams, Tap};

/// Manifest schema version of the stage-evidence bundle.
pub const SCHEMA_VERSION: u32 = 1;

/// The named tap boundaries the evidence contract carries, in manifest
/// order. `rgb_pre` is a pipeline boundary outside the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StageId {
    RgbIn,
    LogEFilm,
    CmyFilm,
    LogEPrint,
    CmyPrint,
    RgbOut,
}

impl StageId {
    /// Manifest order.
    pub const ORDER: [StageId; 6] = [
        Self::RgbIn,
        Self::LogEFilm,
        Self::CmyFilm,
        Self::LogEPrint,
        Self::CmyPrint,
        Self::RgbOut,
    ];

    /// Wire identity of the stage: the existing tap name.
    pub const fn id(self) -> &'static str {
        match self {
            Self::RgbIn => "rgb_in",
            Self::LogEFilm => "log_e_film",
            Self::CmyFilm => "cmy_film",
            Self::LogEPrint => "log_e_print",
            Self::CmyPrint => "cmy_print",
            Self::RgbOut => "rgb_out",
        }
    }

    /// The evidence stage a pipeline tap records, if any.
    pub const fn of_tap(tap: Tap) -> Option<StageId> {
        match tap {
            Tap::RgbIn => Some(Self::RgbIn),
            Tap::RgbPre => None,
            Tap::LogEFilm => Some(Self::LogEFilm),
            Tap::CmyFilm => Some(Self::CmyFilm),
            Tap::LogEPrint => Some(Self::LogEPrint),
            Tap::CmyPrint => Some(Self::CmyPrint),
            Tap::RgbOut => Some(Self::RgbOut),
        }
    }

    /// Position along the tap topology, so range tests speak the same
    /// numbering as [`Tap`], which owns it.
    const fn position(self) -> usize {
        self.tap().position() as usize
    }

    /// The tap this stage records.
    const fn tap(self) -> Tap {
        match self {
            Self::RgbIn => Tap::RgbIn,
            Self::LogEFilm => Tap::LogEFilm,
            Self::CmyFilm => Tap::CmyFilm,
            Self::LogEPrint => Tap::LogEPrint,
            Self::CmyPrint => Tap::CmyPrint,
            Self::RgbOut => Tap::RgbOut,
        }
    }

    const fn domain_units(self) -> (&'static str, &'static str) {
        match self {
            Self::RgbIn => ("input_rgb", "relative"),
            // Log10 record exposure of the film/print records, before
            // development: not developed density, not image CMY color.
            Self::LogEFilm | Self::LogEPrint => ("record_exposure", "log10"),
            Self::CmyFilm | Self::CmyPrint => ("record_density", "optical_density"),
            Self::RgbOut => ("output_rgb", "relative"),
        }
    }

    const fn channel_order(self) -> &'static str {
        match self {
            Self::RgbIn | Self::RgbOut => "RGB",
            // One record axis across the film/print chain (see module docs).
            Self::LogEFilm | Self::CmyFilm | Self::LogEPrint | Self::CmyPrint => "CMY",
        }
    }
}

/// The pixel grid a captured buffer lives on, named explicitly so
/// intermediate stages never inherit the delivered crop silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoordinateSpace {
    /// The full-source input raster, before crop and rescale.
    Source,
    /// The prepared working grid, before any grain-owned crop and rescale.
    Working,
    /// The full-film frame grid used while a delivered-region crop is still
    /// pending (spatial effects need the pixels around the crop).
    FilmFrame,
    /// The supersampled grain-aperture grid before its exposure collapse.
    GrainAperture,
    /// The delivered pipeline output grid.
    Output,
}

impl CoordinateSpace {
    const fn name(self) -> &'static str {
        match self {
            Self::Source => "source_pixels",
            Self::Working => "working_pixels",
            Self::FilmFrame => "film_frame_pixels",
            Self::GrainAperture => "grain_aperture_pixels",
            Self::Output => "output_pixels",
        }
    }
}

/// What the actual full-source exposure application did. The camera exposure
/// application reports its own metering; `exposure_compensation_ev` is applied
/// later at the film exposure and is reported from where it is applied.
#[derive(Debug, Clone)]
pub(crate) struct ExposureSource {
    pub(crate) auto_enabled: bool,
    pub(crate) method: String,
    pub(crate) source_dimensions: [u32; 2],
    pub(crate) metered_ev: f64,
    /// `"measured"` or `"disabled_zero"`: disabled automatic exposure is an
    /// explicitly identified zero metering contribution, never a claim of a
    /// measured physical exposure.
    pub(crate) metered_ev_source: &'static str,
}

enum Boundary {
    /// Captured and written to `path`.
    Available {
        path: String,
        sha256: String,
        width: u32,
        height: u32,
        space: CoordinateSpace,
    },
    /// The executed route never produced this boundary.
    Skipped(&'static str),
    /// A required executed boundary failed to capture or write.
    Unavailable(&'static str),
}

#[derive(Debug, Clone)]
struct Statics {
    route: WorkflowRoute,
    route_text: String,
    scan_output: String,
    print_exposure_factor: f64,
    input_color_space: String,
    input_cctf_decoding: bool,
    output_color_space: String,
    output_cctf_encoding: bool,
}

/// The canonical fixed model of the terminal CMYK ink style appearance.
pub const CMYK_MODEL: &str = "cmyk_ink_v1";
/// The canonical style name of the terminal CMYK ink appearance.
pub const CMYK_STYLE: &str = "ink";
/// The explicit post-`rgb_out` boundary the appearance runs on.
pub const CMYK_BOUNDARY: &str = "post_rgb_out";

/// In-memory recorder for one captured render; buffers are written as the
/// boundaries are reached and [`StageCapture::publish`] writes the terminal
/// manifest.
pub struct StageCapture {
    destination: PathBuf,
    statics: Option<Statics>,
    inject: Tap,
    collect: Tap,
    boundaries: BTreeMap<StageId, Boundary>,
    exposure_source: Option<ExposureSource>,
    compensation_applied: Option<f64>,
    terminal_cmyk: Option<TerminalCmyk>,
    errors: Vec<String>,
}

/// The terminal CMYK ink style appearance as actually executed, or as
/// explicitly not executed. Recorded after `rgb_out` collection; never a
/// spectral stage and never carries a buffer. A skipped record keeps its
/// identity through the stable reason code and carries no strength claim.
#[derive(Debug, Clone)]
struct TerminalCmyk {
    executed: bool,
    strength: f64,
    output_color_space: String,
    output_cctf_encoding: bool,
    reason: &'static str,
}

impl StageCapture {
    /// Capture into `destination`; the directory is created on first write.
    pub fn new(destination: impl Into<PathBuf>) -> Self {
        Self {
            destination: destination.into(),
            statics: None,
            inject: Tap::RgbIn,
            collect: Tap::RgbOut,
            boundaries: BTreeMap::new(),
            exposure_source: None,
            compensation_applied: None,
            terminal_cmyk: None,
            errors: Vec::new(),
        }
    }

    /// The absolute bundle directory this capture publishes into.
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    /// Start one execution: record the static preparation facts and the
    /// value entering the executed range. Idempotent across a repeated call
    /// for the same execution (the per-frame scanner preparation re-enters
    /// the pipeline entry).
    pub(crate) fn begin(
        &mut self,
        params: &RuntimeParams,
        print_exposure_factor: f64,
        inject: Tap,
        collect: Tap,
        input: &ImageBuf,
    ) {
        self.statics = Some(Statics {
            route: params.workflow.route_enum(),
            route_text: params.workflow.route.clone(),
            scan_output: params.scanner.scan_output.clone(),
            print_exposure_factor,
            input_color_space: params.io.input_color_space.clone(),
            input_cctf_decoding: params.io.input_cctf_decoding,
            output_color_space: params.io.output_color_space.clone(),
            output_cctf_encoding: params.io.output_cctf_encoding,
        });
        self.inject = inject;
        self.collect = collect;
        self.boundaries.clear();
        self.exposure_source = None;
        self.compensation_applied = None;
        self.terminal_cmyk = None;
        self.errors.clear();
        if let Some(id) = StageId::of_tap(inject) {
            let space = match inject {
                Tap::RgbIn => CoordinateSpace::Source,
                Tap::RgbOut => CoordinateSpace::Output,
                _ => CoordinateSpace::Working,
            };
            self.record_boundary(id, input, space);
        }
    }

    /// Write the boundary value as it flowed through the executed chain.
    pub(crate) fn record_boundary(
        &mut self,
        id: StageId,
        image: &ImageBuf,
        space: CoordinateSpace,
    ) {
        let filename = format!("{}.f64", id.id());
        if let Err(error) = std::fs::create_dir_all(&self.destination) {
            self.errors.push(format!(
                "{}: buffer write failed: create {}: {error}",
                id.id(),
                self.destination.display()
            ));
            self.boundaries
                .insert(id, Boundary::Unavailable("buffer_write_failed"));
            return;
        }
        match write_buffer(&self.destination.join(&filename), image) {
            Ok(sha256) => {
                self.boundaries.insert(
                    id,
                    Boundary::Available {
                        path: filename,
                        sha256,
                        width: image.width,
                        height: image.height,
                        space,
                    },
                );
            }
            Err(error) => {
                self.errors
                    .push(format!("{}: buffer write failed: {error}", id.id()));
                self.boundaries
                    .insert(id, Boundary::Unavailable("buffer_write_failed"));
            }
        }
    }

    /// Record that the executed route does not produce this boundary.
    pub(crate) fn record_skip(&mut self, id: StageId, reason: &'static str) {
        self.boundaries
            .entry(id)
            .or_insert(Boundary::Skipped(reason));
    }

    /// Record the full-source exposure application actually performed.
    pub(crate) fn record_exposure_source(&mut self, source: ExposureSource) {
        self.exposure_source = Some(source);
    }

    /// Record the exposure compensation the executed render actually
    /// multiplied into the film exposure.
    pub(crate) fn record_compensation(&mut self, ev: f64) {
        self.compensation_applied = Some(ev);
    }

    /// Record the terminal CMYK ink style appearance from the executed
    /// render's actual parameters. The appearance is applied only when the
    /// canonical group is active with positive strength; disabled and
    /// strength-zero runs are explicitly `not_applicable` with a stable
    /// reason, never a strength claim. The record reflects the terminal
    /// RGB appearance, never a spectral stage.
    pub(crate) fn record_terminal_cmyk_params(&mut self, params: &RuntimeParams) {
        let strength = params.cmyk.strength.clamp(0.0, 1.0);
        let executed = params.cmyk.active && params.cmyk.strength > 0.0;
        let reason = if !params.cmyk.active {
            "cmyk_not_active"
        } else {
            "cmyk_strength_zero"
        };
        self.terminal_cmyk = Some(TerminalCmyk {
            executed,
            strength,
            output_color_space: params.io.output_color_space.clone(),
            output_cctf_encoding: params.io.output_cctf_encoding,
            reason,
        });
    }

    /// Write the terminal manifest atomically and report whether the bundle
    /// is complete. The manifest is written even for an incomplete bundle so
    /// failures stay visible; an `Err` means the evidence must not be taken
    /// as complete.
    pub fn publish(&mut self) -> Result<(), String> {
        let statics = self
            .statics
            .as_ref()
            .ok_or("stage evidence was not captured: no execution was recorded")?;
        let manifest = self.assemble(statics);
        let complete = manifest.execution_status == "completed";
        let json = serde_json::to_string_pretty(&manifest)
            .map_err(|error| format!("serialize stage-evidence manifest: {error}"))?;
        let path = self.destination.join("stage-evidence.json");
        let tmp = self.destination.join("stage-evidence.json.tmp");
        std::fs::create_dir_all(&self.destination)
            .map_err(|error| format!("create {}: {error}", self.destination.display()))?;
        write_atomic(&tmp, &path, |file| {
            file.write_all(json.as_bytes())
                .map_err(|error| error.to_string())
        })
        .map_err(|error| format!("write {}: {error}", path.display()))?;
        if complete {
            Ok(())
        } else {
            Err(format!(
                "stage evidence is not complete: {}",
                manifest.errors.join("; ")
            ))
        }
    }

    fn assemble(&self, statics: &Statics) -> Manifest {
        let mut errors = self.errors.clone();
        let mut stages = Vec::with_capacity(StageId::ORDER.len());
        for id in StageId::ORDER {
            let record = match self.boundaries.get(&id) {
                Some(Boundary::Available {
                    path,
                    sha256,
                    width,
                    height,
                    space,
                }) => {
                    let (domain, units) = id.domain_units();
                    let color = match id {
                        StageId::RgbIn => Some(Color {
                            color_space: statics.input_color_space.clone(),
                            cctf_decoding: statics.input_cctf_decoding,
                        }),
                        StageId::RgbOut => Some(Color {
                            color_space: statics.output_color_space.clone(),
                            cctf_decoding: !statics.output_cctf_encoding,
                        }),
                        _ => None,
                    };
                    StageRecord::Available {
                        id: id.id(),
                        path: path.clone(),
                        sha256: sha256.clone(),
                        layout: Layout {
                            width: *width,
                            height: *height,
                            channels: 3,
                            dtype: "f64",
                            endianness: "little",
                            channel_order: id.channel_order(),
                        },
                        domain,
                        units,
                        geometry: Geometry {
                            coordinate_space: space.name(),
                        },
                        color,
                    }
                }
                Some(Boundary::Skipped(reason)) => StageRecord::NotApplicable {
                    id: id.id(),
                    reason: (*reason).to_string(),
                },
                Some(Boundary::Unavailable(reason)) => StageRecord::Unavailable {
                    id: id.id(),
                    reason: (*reason).to_string(),
                },
                None => {
                    if self.in_executed_range(id) {
                        errors.push(format!("{}: executed stage was not captured", id.id()));
                        StageRecord::Unavailable {
                            id: id.id(),
                            reason: "not_captured".to_string(),
                        }
                    } else {
                        StageRecord::NotApplicable {
                            id: id.id(),
                            reason: "outside_executed_tap_range".to_string(),
                        }
                    }
                }
            };
            stages.push(record);
        }
        let execution_status = if stages
            .iter()
            .any(|record| matches!(record, StageRecord::Unavailable { .. }))
            || !errors.is_empty()
        {
            "failed"
        } else {
            "completed"
        };
        let exposure = match &self.exposure_source {
            Some(source) => {
                let compensation_ev = self.compensation_applied.unwrap_or(0.0);
                let applied_ev = source.metered_ev + compensation_ev;
                ExposureRecord::Available {
                    auto_enabled: source.auto_enabled,
                    method: source.method.clone(),
                    source_dimensions: source.source_dimensions,
                    metered_ev: source.metered_ev,
                    metered_ev_source: source.metered_ev_source,
                    compensation_ev,
                    applied_ev,
                    gain: applied_ev.exp2(),
                }
            }
            None => ExposureRecord::NotApplicable {
                reason: match statics.route {
                    WorkflowRoute::Input => "no_exposure_application_on_route",
                    route if route.is_convert_film() => "convert_route_uses_convert_exposure",
                    _ => "not_executed_on_tap_route",
                }
                .to_string(),
            },
        };
        let print_normalization = if statics.route == WorkflowRoute::Input {
            PrintNormalizationRecord::NotApplicable {
                reason: "no_print_normalization_on_input_route".to_string(),
            }
        } else {
            PrintNormalizationRecord::Available {
                value: statics.print_exposure_factor,
                mode: "profile_prepared",
            }
        };
        Manifest {
            schema_version: SCHEMA_VERSION,
            execution_status,
            host_precision: host_precision(),
            workflow_route: statics.route_text.clone(),
            scan_output: statics.scan_output.clone(),
            normalization_mode: "profile_prepared",
            exposure,
            print_normalization,
            terminal_cmyk: match &self.terminal_cmyk {
                Some(record) if record.executed => TerminalCmykRecord::Available {
                    style: CMYK_STYLE,
                    model: CMYK_MODEL,
                    strength: record.strength.clamp(0.0, 1.0),
                    boundary: CMYK_BOUNDARY,
                    color_space: record.output_color_space.clone(),
                    cctf_encoding: record.output_cctf_encoding,
                },
                Some(record) => TerminalCmykRecord::NotApplicable {
                    reason: record.reason.to_string(),
                },
                None => TerminalCmykRecord::NotApplicable {
                    reason: "not_recorded_on_route".to_string(),
                },
            },
            stages,
            errors,
        }
    }

    /// Whether the executed range covers the stage: the input and collected
    /// boundaries are inclusive tap positions. Route-skipped stages never
    /// reach this test; anything else inside the range must be captured.
    fn in_executed_range(&self, id: StageId) -> bool {
        (self.inject.position() as usize) <= id.position()
            && id.position() <= (self.collect.position() as usize)
    }
}

/// Declared host storage precision: buffers always store f64, widened from
/// the host scalar without claiming f64 computation for f32 storage.
fn host_precision() -> &'static str {
    if std::mem::size_of::<Scalar>() == 8 {
        "f64"
    } else {
        "f32"
    }
}

/// Reject execution modes this capture route cannot represent faithfully
/// instead of fabricating boundary buffers from a fused or non-reference
/// route.
pub fn ensure_supported_backend(
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<(), String> {
    if backend.is_gpu() {
        return Err(format!(
            "stage evidence capture supports the CPU reference execution route only; \
             the selected backend {} is a GPU backend — pass --backend cpu \
             (spektrafilm-f64 --backend cpu for f64 reference evidence)",
            backend.name()
        ));
    }
    Ok(())
}

/// Reject configurations whose rendering the v1 manifest cannot describe
/// without a false metering claim. Preview bounding resizes the decoded
/// source before the pipeline, so the metering source and the original
/// decoded geometry would need two identities the contract does not carry.
pub fn ensure_supported_params(params: &RuntimeParams) -> Result<(), String> {
    if params.settings.preview_mode {
        return Err(
            "stage evidence capture does not support preview mode: preview bounding \
             resizes the decoded source before metering and the bundle would not \
             represent the full-original geometry — render at full resolution \
             (settings.preview_mode=false) for native observations"
                .to_string(),
        );
    }
    Ok(())
}

fn write_buffer(path: &Path, image: &ImageBuf) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let tmp = path.with_extension("f64.tmp");
    write_atomic(&tmp, path, |file| {
        let mut bytes = [0u8; 64 * 1024];
        for samples in image.data.chunks(bytes.len() / 8) {
            let chunk = &mut bytes[..samples.len() * 8];
            for (sample, encoded) in samples.iter().zip(chunk.as_chunks_mut::<8>().0) {
                encoded.copy_from_slice(&to_f64(*sample).to_le_bytes());
            }
            hasher.update(&*chunk);
            file.write_all(chunk).map_err(|error| error.to_string())?;
        }
        Ok(())
    })?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Publish one file as a complete unit: full write to a sibling temporary
/// file, then an atomic rename over the destination.
fn write_atomic(
    tmp: &Path,
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), String>,
) -> Result<(), String> {
    let result = (|| {
        let mut file = std::fs::File::create(tmp)
            .map_err(|error| format!("create {}: {error}", tmp.display()))?;
        write(&mut file).map_err(|error| format!("write {}: {error}", tmp.display()))?;
        file.sync_all()
            .map_err(|error| format!("flush {}: {error}", tmp.display()))?;
        std::fs::rename(tmp, path).map_err(|error| format!("publish {}: {error}", path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

#[derive(Serialize)]
struct Manifest {
    schema_version: u32,
    execution_status: &'static str,
    host_precision: &'static str,
    workflow_route: String,
    scan_output: String,
    normalization_mode: &'static str,
    exposure: ExposureRecord,
    print_normalization: PrintNormalizationRecord,
    terminal_cmyk: TerminalCmykRecord,
    stages: Vec<StageRecord>,
    errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum ExposureRecord {
    Available {
        auto_enabled: bool,
        method: String,
        source_dimensions: [u32; 2],
        metered_ev: f64,
        metered_ev_source: &'static str,
        compensation_ev: f64,
        applied_ev: f64,
        gain: f64,
    },
    NotApplicable {
        reason: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum PrintNormalizationRecord {
    Available { value: f64, mode: &'static str },
    NotApplicable { reason: String },
}

/// The terminal CMYK ink style appearance as the executed render applied it,
/// after `rgb_out` collection and before output mix. It is an appearance on
/// the same three-channel RGB buffer, not a spectral stage: no buffer, no
/// seventh [`StageId`], no CMY channel claim.
#[derive(Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum TerminalCmykRecord {
    Available {
        style: &'static str,
        model: &'static str,
        strength: f64,
        boundary: &'static str,
        color_space: String,
        cctf_encoding: bool,
    },
    NotApplicable {
        reason: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum StageRecord {
    Available {
        id: &'static str,
        path: String,
        sha256: String,
        layout: Layout,
        domain: &'static str,
        units: &'static str,
        geometry: Geometry,
        #[serde(skip_serializing_if = "Option::is_none")]
        color: Option<Color>,
    },
    Unavailable {
        id: &'static str,
        reason: String,
    },
    NotApplicable {
        id: &'static str,
        reason: String,
    },
}

#[derive(Serialize)]
struct Layout {
    width: u32,
    height: u32,
    channels: u8,
    dtype: &'static str,
    endianness: &'static str,
    channel_order: &'static str,
}

#[derive(Serialize)]
struct Geometry {
    coordinate_space: &'static str,
}

#[derive(Serialize)]
struct Color {
    color_space: String,
    cctf_decoding: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_at(dir: &Path) -> StageCapture {
        let mut capture = StageCapture::new(dir);
        capture.begin(
            &capture_params(),
            2.5,
            Tap::RgbIn,
            Tap::RgbOut,
            &image(4, 3),
        );
        capture
    }

    fn capture_params() -> RuntimeParams {
        let mut params = RuntimeParams::default();
        params.io.input_color_space = "sRGB".into();
        params.io.input_cctf_decoding = false;
        params.io.output_color_space = "sRGB".into();
        params.io.output_cctf_encoding = false;
        params
    }

    #[test]
    fn capture_rejects_preview_mode_with_an_actionable_reason() {
        // Preview bounding resizes the decoded source before metering, so the
        // bundle would claim a geometry the contract cannot describe. The
        // request must fail naming the setting and the way out, not silently
        // render at the preview geometry.
        let mut params = capture_params();
        params.settings.preview_mode = true;
        let error = ensure_supported_params(&params).unwrap_err();
        assert!(
            error.contains("preview mode") && error.contains("settings.preview_mode=false"),
            "the rejection must name the setting and how to proceed: {error}"
        );
    }
    fn image(width: u32, height: u32) -> ImageBuf {
        ImageBuf::from_data(
            width,
            height,
            (0..width * height * 3)
                .map(|i| from_scalar(i as f64 * 0.01))
                .collect(),
        )
    }

    fn from_scalar(value: f64) -> Scalar {
        value as Scalar
    }

    fn manifest(dir: &Path) -> serde_json::Value {
        let text = std::fs::read_to_string(dir.join("stage-evidence.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spektrafilm-stage-evidence-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn manifest_publishes_relative_buffers_with_matching_hashes() {
        let dir = temp_dir("publish");
        let mut capture = capture_at(&dir);
        capture.record_exposure_source(ExposureSource {
            auto_enabled: true,
            method: "center_weighted".into(),
            source_dimensions: [4, 3],
            metered_ev: 1.5,
            metered_ev_source: "measured",
        });
        capture.record_compensation(0.25);
        capture.record_boundary(StageId::LogEFilm, &image(2, 2), CoordinateSpace::Working);
        capture.record_skip(StageId::CmyFilm, "no_film_stage_on_route");
        capture.record_skip(StageId::LogEPrint, "no_print_stage_on_route");
        capture.record_skip(StageId::CmyPrint, "no_print_stage_on_route");
        capture.record_boundary(StageId::RgbOut, &image(5, 4), CoordinateSpace::Output);
        capture.publish().unwrap();

        let manifest = manifest(&dir);
        assert_eq!(manifest["schema_version"], SCHEMA_VERSION);
        assert_eq!(manifest["execution_status"], "completed");
        assert_eq!(manifest["normalization_mode"], "profile_prepared");
        assert_eq!(manifest["host_precision"], host_precision());
        let stages = manifest["stages"].as_array().unwrap();
        assert_eq!(stages.len(), 6);
        assert_eq!(stages[0]["id"], "rgb_in");
        assert_eq!(stages[5]["id"], "rgb_out");
        assert_eq!(stages[1]["layout"]["channel_order"], "CMY");
        assert!(stages[1].get("color").is_none());
        assert_eq!(stages[0]["layout"]["channel_order"], "RGB");
        assert_eq!(stages[0]["color"]["cctf_decoding"], false);
        assert_eq!(stages[2]["availability"], "not_applicable");
        assert_eq!(stages[2].get("path"), None);
        assert_eq!(stages[2].get("layout"), None);
        assert_eq!(stages[2]["reason"], "no_film_stage_on_route");

        let path = stages[0]["path"].as_str().unwrap();
        assert!(!Path::new(path).is_absolute());
        let bytes = std::fs::read(dir.join(path)).unwrap();
        assert_eq!(bytes.len(), 4 * 3 * 3 * 8);
        assert_eq!(stages[0]["layout"]["width"], 4);
        assert_eq!(stages[0]["sha256"], sha256_hex(&bytes));

        let exposure = &manifest["exposure"];
        assert_eq!(exposure["availability"], "available");
        assert_eq!(exposure["metered_ev_source"], "measured");
        assert_eq!(exposure["compensation_ev"], 0.25);
        assert_eq!(exposure["applied_ev"], 1.75);
        assert_eq!(exposure["source_dimensions"], serde_json::json!([4, 3]));
        assert_eq!(manifest["print_normalization"]["value"], 2.5);
    }

    fn publish_captured(dir: &Path, record: Option<TerminalCmyk>) -> serde_json::Value {
        let mut capture = StageCapture::new(dir);
        capture.begin(
            &capture_params(),
            1.0,
            Tap::RgbIn,
            Tap::RgbOut,
            &image(2, 2),
        );
        capture.record_boundary(StageId::LogEFilm, &image(2, 2), CoordinateSpace::Working);
        capture.record_boundary(StageId::CmyFilm, &image(2, 2), CoordinateSpace::Working);
        capture.record_boundary(StageId::LogEPrint, &image(2, 2), CoordinateSpace::Working);
        capture.record_boundary(StageId::CmyPrint, &image(2, 2), CoordinateSpace::Working);
        capture.record_boundary(StageId::RgbOut, &image(2, 2), CoordinateSpace::Output);
        if let Some(record) = record {
            capture.terminal_cmyk = Some(record);
        }
        capture.publish().unwrap();
        manifest(dir)
    }

    fn terminal_cmyk(executed: bool, strength: f64, reason: &'static str) -> TerminalCmyk {
        TerminalCmyk {
            executed,
            strength,
            output_color_space: "sRGB".into(),
            output_cctf_encoding: true,
            reason,
        }
    }

    #[test]
    fn executed_terminal_cmyk_carries_canonical_identity_and_no_spectral_stage() {
        // The appearance is a terminal RGB record, never a seventh StageId:
        // the stage list stays at the six named boundaries.
        let dir = temp_dir("terminal-cmyk-executed");
        let manifest = publish_captured(&dir, Some(terminal_cmyk(true, 0.75, "cmyk_not_active")));
        let terminal = &manifest["terminal_cmyk"];
        assert_eq!(terminal["availability"], "available");
        assert_eq!(terminal["style"], "ink");
        assert_eq!(terminal["model"], "cmyk_ink_v1");
        assert_eq!(terminal["strength"], 0.75);
        assert_eq!(terminal["boundary"], "post_rgb_out");
        assert_eq!(terminal["color_space"], "sRGB");
        assert_eq!(terminal["cctf_encoding"], true);
        // No buffer path is ever attached to the terminal appearance.
        assert!(terminal.get("path").is_none());
        assert!(terminal.get("layout").is_none());
        assert!(terminal.get("sha256").is_none());
        let stages = manifest["stages"].as_array().unwrap();
        assert_eq!(stages.len(), 6);
    }

    #[test]
    fn strength_is_clamped_into_the_declared_unit_interval() {
        let dir = temp_dir("terminal-cmyk-clamped");
        let manifest = publish_captured(&dir, Some(terminal_cmyk(true, 1.5, "cmyk_not_active")));
        assert_eq!(manifest["terminal_cmyk"]["strength"], 1.0);
        let manifest = publish_captured(&dir, Some(terminal_cmyk(true, -1.0, "cmyk_not_active")));
        assert_eq!(manifest["terminal_cmyk"]["strength"], 0.0);
    }

    #[test]
    fn inactive_terminal_cmyk_is_not_applicable_with_a_stable_reason() {
        let dir = temp_dir("terminal-cmyk-disabled");
        let manifest = publish_captured(&dir, Some(terminal_cmyk(false, 1.0, "cmyk_not_active")));
        let terminal = &manifest["terminal_cmyk"];
        assert_eq!(terminal["availability"], "not_applicable");
        assert_eq!(terminal["reason"], "cmyk_not_active");
        // A skipped record never claims a strength or a color interpretation.
        assert!(terminal.get("strength").is_none());
        assert!(terminal.get("style").is_none());
        assert!(terminal.get("model").is_none());
        assert!(terminal.get("boundary").is_none());
        assert_eq!(manifest["execution_status"], "completed");
    }

    #[test]
    fn strength_zero_terminal_cmyk_is_not_applicable_with_a_stable_reason() {
        let dir = temp_dir("terminal-cmyk-zero");
        let manifest =
            publish_captured(&dir, Some(terminal_cmyk(false, 0.0, "cmyk_strength_zero")));
        let terminal = &manifest["terminal_cmyk"];
        assert_eq!(terminal["availability"], "not_applicable");
        assert_eq!(terminal["reason"], "cmyk_strength_zero");
        assert!(terminal.get("strength").is_none());
        assert_eq!(manifest["execution_status"], "completed");
    }

    #[test]
    fn missing_terminal_cmyk_record_stays_not_applicable_without_failing_the_bundle() {
        let dir = temp_dir("terminal-cmyk-missing");
        let manifest = publish_captured(&dir, None);
        let terminal = &manifest["terminal_cmyk"];
        assert_eq!(terminal["availability"], "not_applicable");
        assert_eq!(terminal["reason"], "not_recorded_on_route");
        assert_eq!(manifest["execution_status"], "completed");
        assert!(manifest["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn disabled_auto_exposure_identifies_zero_metering_contribution() {
        let dir = temp_dir("disabled-metering");
        let mut capture = capture_at(&dir);
        capture.record_exposure_source(ExposureSource {
            auto_enabled: false,
            method: "center_weighted".into(),
            source_dimensions: [4, 3],
            metered_ev: 0.0,
            metered_ev_source: "disabled_zero",
        });
        // The exposure facts publish even though this test leaves the film
        // stages uncaptured.
        let _ = capture.publish();
        let exposure = &manifest(&dir)["exposure"];
        assert_eq!(exposure["auto_enabled"], false);
        assert_eq!(exposure["metered_ev"], 0.0);
        assert_eq!(exposure["metered_ev_source"], "disabled_zero");
        assert_eq!(exposure["applied_ev"], 0.0);
        assert_eq!(exposure["gain"], 1.0);
    }

    #[test]
    fn missing_required_stage_fails_the_bundle_visibly() {
        let dir = temp_dir("missing-stage");
        let mut capture = capture_at(&dir);
        // rgb_in came from `begin`; rgb_out is inside the executed range but
        // never captured.
        let error = capture.publish().unwrap_err();
        assert!(error.contains("rgb_out"), "{error}");
        let manifest = manifest(&dir);
        assert_eq!(manifest["execution_status"], "failed");
        let stages = manifest["stages"].as_array().unwrap();
        assert_eq!(stages[5]["availability"], "unavailable");
        assert_eq!(stages[5]["reason"], "not_captured");
        assert_eq!(stages[5].get("path"), None);
        assert!(!manifest["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn stages_outside_the_executed_tap_range_are_not_applicable() {
        let dir = temp_dir("tap-range");
        let mut capture = StageCapture::new(&dir);
        let params = RuntimeParams::default();
        capture.begin(&params, 1.0, Tap::LogEFilm, Tap::CmyFilm, &image(2, 2));
        capture.record_boundary(StageId::CmyFilm, &image(2, 2), CoordinateSpace::Working);
        capture.publish().unwrap();
        let stages = manifest(&dir)["stages"].as_array().unwrap().clone();
        assert_eq!(stages[0]["availability"], "not_applicable");
        assert_eq!(stages[0]["reason"], "outside_executed_tap_range");
        assert_eq!(stages[3]["reason"], "outside_executed_tap_range");
        assert_eq!(stages[5]["reason"], "outside_executed_tap_range");
        assert_eq!(stages[1]["availability"], "available");
        assert_eq!(stages[2]["availability"], "available");
    }

    #[test]
    fn buffer_write_failure_is_visible_and_not_complete_evidence() {
        let dir = temp_dir("write-failure");
        // A directory where the buffer file must go makes publication of that
        // buffer fail without touching the render.
        std::fs::create_dir(dir.join("log_e_film.f64")).unwrap();
        let mut capture = capture_at(&dir);
        capture.record_boundary(StageId::LogEFilm, &image(2, 2), CoordinateSpace::Working);
        capture.record_boundary(StageId::RgbOut, &image(2, 2), CoordinateSpace::Output);
        let error = capture.publish().unwrap_err();
        assert!(error.contains("log_e_film"), "{error}");
        let manifest = manifest(&dir);
        assert_eq!(manifest["execution_status"], "failed");
        let stages = manifest["stages"].as_array().unwrap();
        assert_eq!(stages[1]["availability"], "unavailable");
        assert_eq!(stages[1]["reason"], "buffer_write_failed");
    }

    #[test]
    fn manifest_publication_is_terminal_and_atomic() {
        let dir = temp_dir("atomic");
        let mut capture = capture_at(&dir);
        capture.record_boundary(StageId::RgbOut, &image(2, 2), CoordinateSpace::Output);
        assert!(!dir.join("stage-evidence.json").exists());
        let _ = capture.publish();
        assert!(dir.join("stage-evidence.json").is_file());
        assert!(!dir.join("stage-evidence.json.tmp").exists());
    }

    #[test]
    fn input_route_marks_film_and_print_stages_not_applicable() {
        let dir = temp_dir("input-route");
        let mut capture = StageCapture::new(&dir);
        let mut params = capture_params();
        params.workflow.route = "input".into();
        capture.begin(&params, 2.5, Tap::RgbIn, Tap::RgbOut, &image(4, 3));
        capture.record_skip(StageId::LogEFilm, "no_film_stage_on_route");
        capture.record_skip(StageId::CmyFilm, "no_film_stage_on_route");
        capture.record_skip(StageId::LogEPrint, "no_print_stage_on_route");
        capture.record_skip(StageId::CmyPrint, "no_print_stage_on_route");
        capture.record_boundary(StageId::RgbOut, &image(2, 2), CoordinateSpace::Output);
        capture.publish().unwrap();
        let manifest = manifest(&dir);
        assert_eq!(manifest["execution_status"], "completed");
        assert_eq!(manifest["exposure"]["availability"], "not_applicable");
        assert_eq!(
            manifest["exposure"]["reason"],
            "no_exposure_application_on_route"
        );
        assert_eq!(
            manifest["print_normalization"]["reason"],
            "no_print_normalization_on_input_route"
        );
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }
}
