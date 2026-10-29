/// 3-stage simulation pipeline: Filming → Printing → Scanning.
///
/// Full calibration chain:
///   1. Load spectra LUT → compute TC LUT for film sensitivity
///   2. Process virtual gray card through filming to get midgray spectral density
///   3. Compute print exposure normalization factor from midgray spectral density
///   4. Pass all calibration data to the pipeline stages
///
/// Completed material execution shares these stage nodes with image rendering.
/// `process_stage` captures native density plus frozen interpretation and physical
/// geometry; `stage_execution::continue_stage` enters after development without
/// replaying exposure, material grain, or completed geometry. Only the artifact
/// module owns persistence; diagnostic taps never establish resumability.
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use rayon::prelude::*;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};
use spektrafilm_math::spectral::TcLut;

mod execution;
mod exposure;
mod material;
mod retained;
pub use retained::RetainedRender;
mod output_mix;
mod preflight;
mod resident;
pub(crate) mod scan_frame;
mod stages;

#[cfg(test)]
mod tests;

/// Diagnostic helper: when env var `$var` is set, dump the image's f64
/// pixel data to that path as a raw little-endian f64 blob. Used by the
/// parity harness (`scripts/parity/run_parity.py`) to bisect the
/// Python ↔ Rust drift one stage at a time without modifying call
/// sites. Silent no-op when the env var is unset.
pub(crate) fn dump_if_env(var: &str, image: &ImageBuf) {
    let Ok(path) = std::env::var(var) else {
        return;
    };
    let bytes: Vec<u8> = image
        .data
        .iter()
        .flat_map(|&v| (v as f64).to_le_bytes())
        .collect();
    match std::fs::write(&path, &bytes) {
        Ok(()) => tracing::info!(
            path = %path,
            count = image.data.len(),
            "dumped f64 buffer for {}",
            var
        ),
        Err(e) => tracing::error!("dump {var} → {path}: {e}"),
    }
}

#[cfg(test)]
use crate::enlarger;
use crate::spectral_service::select_illuminant;
use spektrafilm_backend::timing::{print_stage_timing, stage_timings_enabled};

fn record_stage_timing(
    timings: &mut Option<&mut BTreeMap<String, f64>>,
    stage: &str,
    start: Instant,
) {
    if let Some(timings) = timings.as_deref_mut() {
        *timings.entry(stage.to_string()).or_default() += start.elapsed().as_secs_f64();
    }
}

use crate::params::{RuntimeParams, Tap};
#[cfg(test)]
use crate::pipeline_calibration::{apply_base_tuning, apply_film_chemistry};
use crate::profile::Profile;

#[derive(Clone)]
pub struct Pipeline {
    pub film: Arc<Profile>,
    pub print: Arc<Profile>,
    pub params: RuntimeParams,
    spectral: Option<Arc<crate::spectral_service::SpectralPreparation>>,
    gray: Option<Arc<crate::pipeline_calibration::GrayReferences>>,
    print_calibration: Option<Arc<crate::pipeline_calibration::PrintCalibration>>,
    convert_converter: Option<Arc<crate::stages::converting::Converter>>,
    pub(crate) conversion_tones: [crate::stages::scanning::ChannelTone; 3],
    output_gamut: Arc<crate::gamut_compression::OutputGamutCompress>,
    preparation_device: Option<u64>,
    /// Data directory the spectral front-end was built from. Present on
    /// `new_with_spectral` pipelines; lets `with_params` rebuild the TC LUT
    /// when a sensitivity-affecting control changes instead of reusing a
    /// stale one.
    data_dir: Option<std::path::PathBuf>,
    /// Original film reference captured before calibration mutates the render profile.
    original_film: Arc<Profile>,
    original_print: Arc<Profile>,
    fotufilm_prepared: crate::stages::grain_fotufilm::Prepared,
    cancellation: Option<Arc<AtomicBool>>,
}

/// Fully prepared candidate; committing it performs no fallible work.
pub(crate) struct ParameterUpdate {
    pub params: RuntimeParams,
    pub film: Arc<Profile>,
    print: Arc<Profile>,
    spectral: Option<Arc<crate::spectral_service::SpectralPreparation>>,
    gray: Option<Arc<crate::pipeline_calibration::GrayReferences>>,
    print_calibration: Option<Arc<crate::pipeline_calibration::PrintCalibration>>,
    convert_converter: Option<Arc<crate::stages::converting::Converter>>,
    conversion_tones: [crate::stages::scanning::ChannelTone; 3],
    output_gamut: Arc<crate::gamut_compression::OutputGamutCompress>,
    preparation_device: Option<u64>,
}

#[derive(PartialEq)]
struct OutputInputs<'a> {
    controls: &'a crate::params::OutputGamutCompressParams,
    color_space: &'a str,
    device: Option<u64>,
}

impl<'a> OutputInputs<'a> {
    fn new(params: &'a RuntimeParams, device: Option<u64>) -> Self {
        Self {
            controls: &params.io.output_gamut_compress,
            color_space: &params.io.output_color_space,
            device,
        }
    }

    fn build(
        &self,
        backend: &dyn ComputeBackend,
    ) -> Result<crate::gamut_compression::OutputGamutCompress, String> {
        crate::gamut_compression::OutputGamutCompress::build_with_backend(
            self.controls,
            self.color_space,
            backend,
        )
    }
}

impl Pipeline {
    /// Accessor for the pre-computed TC LUT (used by parity tests).
    pub fn tc_lut(&self) -> Option<&TcLut> {
        self.spectral.as_deref().and_then(|s| s.tc_lut())
    }
    fn prepared_tc_lut(&self) -> Option<&spektrafilm_backend::PreparedTable> {
        self.spectral.as_deref().and_then(|s| s.prepared_tc_lut())
    }
    fn mallett_core(&self) -> Option<&[[f64; 3]; 3]> {
        self.spectral.as_deref().and_then(|s| s.mallett_core())
    }
    fn front_illuminant(&self) -> &str {
        self.spectral
            .as_deref()
            .map_or(&self.film.info.reference_illuminant, |s| {
                &s.front_illuminant
            })
    }
    pub fn print_exposure_factor(&self) -> f64 {
        self.print_calibration
            .as_ref()
            .map_or(1.0, |p| p.exposure_factor)
    }
    pub fn print_illuminant_slice(&self) -> &[f64] {
        self.print_calibration
            .as_ref()
            .map_or(&[], |p| p.illuminant.as_slice())
    }
    pub fn preflash_raw(&self) -> [f64; 3] {
        self.print_calibration
            .as_ref()
            .map_or([0.0; 3], |p| p.preflash_raw)
    }
    fn midgray_density(&self) -> &[f64] {
        self.gray
            .as_ref()
            .map_or(&[], |g| g.midgray_density.as_slice())
    }
    fn compensated_midgray_density(&self) -> &[f64] {
        self.gray
            .as_ref()
            .map_or(&[], |g| g.compensated_midgray_density.as_slice())
    }

    /// Update parameters, deriving changed materials from their original sources.
    /// Compatible spectral tables remain owned by this pipeline during preparation.
    pub fn with_params(self, params: RuntimeParams) -> Result<Self, String> {
        self.with_params_with_backend(params, &spektrafilm_backend::cpu_backend::CpuBackend)
    }

    pub fn with_params_with_backend(
        mut self,
        params: RuntimeParams,
        backend: &dyn ComputeBackend,
    ) -> Result<Self, String> {
        let update = self.prepare_parameter_update(params, backend)?;
        self.apply_parameter_update(update);
        Ok(self)
    }

    pub(crate) fn prepare_parameter_update(
        &self,
        params: RuntimeParams,
        backend: &dyn ComputeBackend,
    ) -> Result<ParameterUpdate, String> {
        Self::prepare_state(
            &self.original_film,
            &self.original_print,
            params,
            self.data_dir.as_deref(),
            backend,
            Some(self),
        )
    }

    fn prepare_state(
        original_film: &Arc<Profile>,
        original_print: &Arc<Profile>,
        mut params: RuntimeParams,
        data_dir: Option<&Path>,
        backend: &dyn ComputeBackend,
        previous: Option<&Pipeline>,
    ) -> Result<ParameterUpdate, String> {
        use crate::pipeline_calibration::{
            FilmMaterialInputs, GrayInputs, PrintInputs, PrintMaterialInputs,
        };
        let cold_tones = if previous.is_none() {
            Some(crate::stages::scanning::style::resolve(
                &params.scanner.conversion_style,
                data_dir,
            )?)
        } else {
            None
        };
        if previous.is_none() {
            params.validate()?;
        }
        crate::params_builder::normalize_runtime_topology(original_film, &mut params);
        if previous.is_some() {
            params.validate()?;
        }
        params.validate_color()?;
        let film_inputs = FilmMaterialInputs::new(original_film, &params);
        let film = if let Some(old) = previous
            .filter(|old| film_inputs == FilmMaterialInputs::new(original_film, &old.params))
        {
            if params.scanner.scan_output == "negative_to_positive"
                && params.workflow.route_enum().artifact_output().is_none()
            {
                if old.params.scanner.scan_output == "negative_to_positive"
                    && old.params.workflow.route_enum().artifact_output().is_none()
                {
                    crate::pipeline_calibration::validate_scan_output_route_detailed(&params)
                        .map_err(|error| error.to_string())?;
                } else {
                    crate::pipeline_calibration::validate_scan_output(
                        &params,
                        &film_inputs.resolve(),
                    )?;
                }
            } else {
                crate::pipeline_calibration::validate_scan_output(&params, &old.film)?;
            }
            Arc::clone(&old.film)
        } else {
            let resolved = film_inputs.resolve();
            crate::pipeline_calibration::validate_scan_output(&params, &resolved)?;
            Arc::new(film_inputs.build(resolved)?)
        };
        let conversion_tones = if let Some(tones) = cold_tones {
            tones
        } else if let Some(old) = previous
            .filter(|old| old.params.scanner.conversion_style == params.scanner.conversion_style)
        {
            old.conversion_tones
        } else {
            crate::stages::scanning::style::resolve(&params.scanner.conversion_style, data_dir)?
        };
        let print_inputs = PrintMaterialInputs::new(original_print, &params);
        let print = if let Some(old) = previous
            .filter(|old| print_inputs == PrintMaterialInputs::new(original_print, &old.params))
        {
            Arc::clone(&old.print)
        } else if let Some(inputs) = &print_inputs {
            Arc::new(inputs.build())
        } else {
            Arc::clone(original_print)
        };
        let prepare_print = print_inputs.is_some();
        if prepare_print {
            crate::pipeline_calibration::validate_print_morph(
                &print,
                &params.print_render.density_curves_morph,
            )?;
        }
        let neutral_inputs =
            crate::pipeline_calibration::NeutralInputs::new(&film, &print, &params, prepare_print);
        let neutral = if let Some(old) = previous.filter(|old| {
            old.print_calibration.is_some()
                && neutral_inputs
                    == crate::pipeline_calibration::NeutralInputs::new(
                        &old.film,
                        &old.print,
                        &old.params,
                        prepare_print,
                    )
        }) {
            old.print_calibration
                .as_ref()
                .expect("checked above")
                .neutral_cmy
        } else {
            neutral_inputs.build(data_dir)?
        };
        params.enlarger.c_filter_neutral = neutral[0] as f32;
        params.enlarger.m_filter_neutral = neutral[1] as f32;
        params.enlarger.y_filter_neutral = neutral[2] as f32;
        let spectral = if params.workflow.route_enum() == crate::WorkflowRoute::Input {
            None
        } else {
            let inputs =
                crate::spectral_service::SpectralInputs::new(original_film, &params, backend);
            if let Some(old) = previous.filter(|old| {
                old.spectral
                    .as_ref()
                    .is_some_and(|s| s.compatible_with(backend))
                    && inputs
                        == crate::spectral_service::SpectralInputs::new(
                            original_film,
                            &old.params,
                            backend,
                        )
            }) {
                old.spectral.clone()
            } else if let Some(data_dir) = data_dir {
                Some(Arc::new(
                    crate::spectral_service::build_filming_upsampler_with_backend(
                        inputs, data_dir, backend,
                    )?
                    .preparation,
                ))
            } else if !backend.is_gpu()
                && previous.is_some_and(|old| {
                    old.spectral.is_none()
                        && inputs
                            == crate::spectral_service::SpectralInputs::new(
                                original_film,
                                &old.params,
                                backend,
                            )
                })
            {
                None
            } else {
                return Err("pipeline lacks data_dir for spectral preparation".to_owned());
            }
        };
        let gray = if let Some(front) = spectral.as_deref() {
            let inputs = GrayInputs::new(&film, front, &params);
            if let Some(old) =
                previous.filter(|old| match (old.spectral.as_deref(), old.gray.as_ref()) {
                    (Some(front), Some(_)) => {
                        inputs == GrayInputs::new(&old.film, front, &old.params)
                    }
                    _ => false,
                })
            {
                old.gray.clone()
            } else {
                Some(Arc::new(inputs.build()))
            }
        } else {
            None
        };
        let print_calibration =
            if prepare_print && params.workflow.route_enum() != crate::WorkflowRoute::Input {
                if let Some(gray) = gray.as_deref() {
                    let inputs = PrintInputs::new(&film, &print, gray, &params, neutral);
                    if let Some(old) = previous.filter(|old| {
                        match (old.gray.as_deref(), old.print_calibration.as_deref()) {
                            (Some(gray), Some(calibration)) => {
                                inputs
                                    == PrintInputs::new(
                                        &old.film,
                                        &old.print,
                                        gray,
                                        &old.params,
                                        calibration.neutral_cmy,
                                    )
                            }
                            _ => false,
                        }
                    }) {
                        old.print_calibration.clone()
                    } else {
                        Some(Arc::new(inputs.build()))
                    }
                } else {
                    previous.and_then(|old| old.print_calibration.clone())
                }
            } else {
                None
            };
        let previous_converter = previous.and_then(|old| {
            old.convert_converter
                .as_ref()
                .map(|c| (&*old.film, &old.params, c))
        });
        let convert_converter =
            crate::stages::converting::prepare_converter(&film, &params, previous_converter)?;
        let device = backend.preparation_device_id();
        let output_inputs = OutputInputs::new(&params, device);
        let output_gamut = if let Some(old) = previous
            .filter(|old| output_inputs == OutputInputs::new(&old.params, old.preparation_device))
        {
            Arc::clone(&old.output_gamut)
        } else {
            Arc::new(output_inputs.build(backend)?)
        };
        Ok(ParameterUpdate {
            params,
            film,
            print,
            spectral,
            gray,
            print_calibration,
            convert_converter,
            conversion_tones,
            output_gamut,
            preparation_device: device,
        })
    }

    pub(crate) fn apply_parameter_update(&mut self, update: ParameterUpdate) {
        self.params = update.params;
        self.film = update.film;
        self.print = update.print;
        self.spectral = update.spectral;
        self.gray = update.gray;
        self.print_calibration = update.print_calibration;
        self.convert_converter = update.convert_converter;
        self.conversion_tones = update.conversion_tones;
        self.output_gamut = update.output_gamut;
        self.preparation_device = update.preparation_device;
    }
    #[cfg(test)]
    pub(crate) fn fotufilm_bank_identity(&self) -> Option<usize> {
        self.fotufilm_prepared.bank_identity()
    }
    pub fn with_cancellation(mut self, cancellation: Option<Arc<AtomicBool>>) -> Self {
        self.cancellation = cancellation;
        self
    }

    fn check_cancelled(&self) -> Result<(), String> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(|token| token.load(Ordering::Relaxed))
        {
            Err("operation cancelled".to_owned())
        } else {
            Ok(())
        }
    }
}

impl Pipeline {
    /// Create pipeline without spectral LUT — **test-only**.
    ///
    /// Production callers must use [`Pipeline::new_with_spectral`]: the
    /// spectral LUT is the calibrated front end of the 0.3.4 pipeline, and
    /// silently degrading to the identity front end changes consumer-visible
    /// output. Kept for the parity/unit tests that exercise the stage chain
    /// without paying LUT construction.
    #[cfg(test)]
    pub fn new(film: Profile, print: Profile, params: RuntimeParams) -> Self {
        let original_print = Arc::new(print.clone());
        let original_film = Arc::new(film.clone());
        let fotufilm_prepared = crate::stages::grain_fotufilm::Prepared::new(
            &original_film,
            params.film_render.development_time,
        );
        params.validate().expect("invalid runtime parameters");
        params
            .validate_color()
            .expect("invalid colour configuration");
        // B&W profiles must be resolved on every construction path —
        // R/G/B channels.
        let mut film =
            crate::profile::resolve_for_render(film, params.film_render.development_time);
        let mut params = params;
        crate::params_builder::normalize_runtime_topology(&film, &mut params);
        crate::pipeline_calibration::validate_scan_output(&params, &film)
            .expect("invalid scan output mode");
        let mut print =
            crate::profile::resolve_for_render(print, params.print_render.development_time);
        apply_base_tuning(&mut film, &params.film_render.base, None);
        apply_film_chemistry(&mut film, &params.film_render.chemistry)
            .expect("validated film chemistry parameters");
        apply_base_tuning(
            &mut print,
            &params.film_render.base,
            Some(&params.print_render.base),
        );
        let print_illuminant = enlarger::enlarger_filtered_illuminant_f64(
            &params.enlarger.illuminant,
            params.enlarger.c_filter_neutral as f64,
            (params.enlarger.m_filter_neutral + params.enlarger.m_filter_shift) as f64,
            (params.enlarger.y_filter_neutral + params.enlarger.y_filter_shift) as f64,
        );
        let conversion_tones =
            crate::stages::scanning::style::resolve(&params.scanner.conversion_style, None)
                .expect("validated scanner conversion style");
        let output_gamut = crate::gamut_compression::OutputGamutCompress::build(
            &params.io.output_gamut_compress,
            &params.io.output_color_space,
        )
        .expect("validated output gamut configuration");
        let convert_converter = crate::stages::converting::prepare_converter(&film, &params, None)
            .expect("validated film conversion");
        let neutral_cmy = [
            params.enlarger.c_filter_neutral as f64,
            params.enlarger.m_filter_neutral as f64,
            params.enlarger.y_filter_neutral as f64,
        ];
        Self {
            film: Arc::new(film),
            print: Arc::new(print),
            params,
            spectral: None,
            gray: None,
            print_calibration: Some(Arc::new(crate::pipeline_calibration::PrintCalibration {
                exposure_factor: 1.0,
                illuminant: print_illuminant,
                preflash_raw: [0.0; 3],
                neutral_cmy,
            })),
            convert_converter,
            conversion_tones,
            output_gamut: Arc::new(output_gamut),
            preparation_device: None,
            data_dir: None,
            original_film,
            original_print,
            fotufilm_prepared,
            cancellation: None,
        }
    }

    /// Create the CPU reference pipeline with full spectral calibration.
    /// For selected-GPU processing use [`Self::new_with_spectral_backend`], or
    /// migrate with [`Self::with_params_with_backend`] before processing.
    pub fn new_with_spectral(
        film: Profile,
        print: Profile,
        params: RuntimeParams,
        data_dir: &Path,
    ) -> Result<Self, String> {
        Self::new_with_spectral_backend(
            film,
            print,
            params,
            data_dir,
            &spektrafilm_backend::cpu_backend::CpuBackend,
        )
    }

    pub fn new_with_spectral_backend(
        film: Profile,
        print: Profile,
        params: RuntimeParams,
        data_dir: &Path,
        backend: &dyn ComputeBackend,
    ) -> Result<Self, String> {
        let original_print = Arc::new(print);
        let original_film = Arc::new(film);
        let fotufilm_prepared = crate::stages::grain_fotufilm::Prepared::new(
            &original_film,
            params.film_render.development_time,
        );
        let prepared = Self::prepare_state(
            &original_film,
            &original_print,
            params,
            Some(data_dir),
            backend,
            None,
        )?;
        Ok(Self {
            film: prepared.film,
            print: prepared.print,
            params: prepared.params,
            spectral: prepared.spectral,
            gray: prepared.gray,
            print_calibration: prepared.print_calibration,
            convert_converter: prepared.convert_converter,
            conversion_tones: prepared.conversion_tones,
            output_gamut: prepared.output_gamut,
            preparation_device: prepared.preparation_device,
            data_dir: Some(data_dir.to_owned()),
            original_film,
            original_print,
            fotufilm_prepared,
            cancellation: None,
        })
    }

    /// Crop + rescale under the `geometry` observation scope. Callers resolve
    /// the returned borrow against the image they still hold, so the
    /// no-geometry case stays zero-copy.
    fn rescale_working<'a>(
        &self,
        image: &'a ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<(std::borrow::Cow<'a, ImageBuf>, f64), String> {
        let mut geometry = crate::stages::StageObservation::cpu(
            backend,
            "geometry",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        // Only routes forming film defer crop/upscale to an optical field.
        // Inverse scan recovers existing density and uses ordinary geometry.
        let resized = if self.params.workflow.route_enum().exposes_film()
            && (crate::stages::grain_v3::active(&self.params)
                || crate::stages::grain_fotufilm::active(&self.params))
        {
            Ok((
                std::borrow::Cow::Borrowed(image),
                crate::resizing::pixel_size_um(
                    self.params.camera.film_format_mm,
                    image.width,
                    image.height,
                ),
            ))
        } else {
            crate::resizing::crop_and_rescale(
                image,
                &self.params.io,
                self.params.camera.film_format_mm,
            )
        };
        geometry.set_complete(resized.is_ok());
        drop(geometry);
        resized
    }
}
