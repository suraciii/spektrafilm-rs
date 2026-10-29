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
    pub film: Profile,
    pub print: Profile,
    pub params: RuntimeParams,
    tc_lut: Option<TcLut>,
    prepared_tc_lut: Option<spektrafilm_backend::PreparedTable>,
    /// Mallett2019 reflectance-basis core matrix (linear-sRGB → raw), when that
    /// upsampler is selected instead of the hanatos2025 tc LUT. Mutually
    /// exclusive with `tc_lut`; neither is prepared for the input-only route.
    mallett_core: Option<[[f64; 3]; 3]>,
    convert_converter: Option<crate::stages::converting::Converter>,
    /// Illuminant used by the RGB→tc projection before TC LUT lookup.
    /// Most methods use the film reference illuminant; arctic2026alpha02
    /// recovers reflectance in D65 coordinates and relights separately.
    front_illuminant: String,
    /// Print exposure normalization factor (1/geomean of midgray raw through enlarger).
    print_exposure_factor: f64,
    /// Filtered enlarger illuminant for printing stage (f64 for Python parity).
    print_illuminant: Vec<f64>,
    /// Constant preflash raw 3-vector added to the print exposure before the
    /// inner log10. `[0; 3]` unless the enlarger preflash is active.
    preflash_raw: [f64; 3],
    midgray_density: Vec<f64>,
    compensated_midgray_density: Vec<f64>,
    /// Resolved positive-conversion tone triple for the negative-to-positive
    /// interpretation (`scanner.conversion_style`).
    pub(crate) conversion_tones: [crate::stages::scanning::ChannelTone; 3],
    /// Output gamut compressor (built once; identity unless oklch is enabled).
    output_gamut: crate::gamut_compression::OutputGamutCompress,
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
    pub material: Option<crate::pipeline_calibration::MaterialCalibration>,
    convert_converter: Option<crate::stages::converting::Converter>,
    conversion_tones: [crate::stages::scanning::ChannelTone; 3],
    output_gamut: crate::gamut_compression::OutputGamutCompress,
}

impl Pipeline {
    /// Accessor for the pre-computed TC LUT (used by parity tests).
    pub fn tc_lut(&self) -> Option<&TcLut> {
        self.tc_lut.as_ref()
    }
    /// Accessor for the print exposure factor (parity tests).
    pub fn print_exposure_factor(&self) -> f64 {
        self.print_exposure_factor
    }
    /// Accessor for the filtered print illuminant (parity tests).
    pub fn print_illuminant_slice(&self) -> &[f64] {
        &self.print_illuminant
    }
    /// Accessor for the constant preflash raw vector (parity tests / debug).
    pub fn preflash_raw(&self) -> [f64; 3] {
        self.preflash_raw
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
        mut params: RuntimeParams,
        backend: &dyn ComputeBackend,
    ) -> Result<ParameterUpdate, String> {
        crate::params_builder::normalize_runtime_topology(&self.original_film, &mut params);
        params.validate()?;
        params.validate_color()?;
        crate::pipeline_calibration::validate_scan_output(&params, &self.film)?;
        let preparation_changed = match (&self.prepared_tc_lut, backend.preparation_device_id()) {
            (Some(table), Some(device)) => table.values.backend_id() != device,
            (Some(_), None) => true,
            (None, Some(_)) => self.tc_lut.is_some(),
            (None, None) => false,
        };
        let route_changed = self.params.workflow.route_enum() != params.workflow.route_enum();
        let prepares_film = params.workflow.route_enum() != crate::WorkflowRoute::Input;
        let spectral_changed = route_changed
            || (prepares_film
                && (crate::pipeline_calibration::spectral_changed(&self.params, &params)
                    || preparation_changed));
        let recalibrate = spectral_changed
            || crate::pipeline_calibration::material_changed(&self.params, &params)
            || (prepares_film
                && crate::pipeline_calibration::calibration_changed(&self.params, &params));
        if !recalibrate
            && params.workflow.route_enum() != crate::WorkflowRoute::InputFilm
            && let Some(model) = self.print.data.density_curves_model.as_ref()
        {
            crate::print_morph::morph_density_curves(
                &self.print.log_exposure_f64(),
                model,
                &params.print_render.density_curves_morph,
                self.print.is_positive(),
            )
            .map_err(|error| format!("invalid print density-curve model: {error}"))?;
        }
        let conversion_tones = if recalibrate
            || params.scanner.conversion_style != self.params.scanner.conversion_style
        {
            crate::stages::scanning::style::resolve(
                &params.scanner.conversion_style,
                self.data_dir.as_deref(),
            )?
        } else {
            self.conversion_tones
        };
        if recalibrate {
            let data_dir = self
                .data_dir
                .as_deref()
                .ok_or_else(|| "pipeline lacks data_dir for calibration rebuild".to_owned())?;
            let retained = (!spectral_changed).then_some(
                crate::pipeline_calibration::SpectralPreparationRef {
                    tc_lut: self.tc_lut.as_ref(),
                    prepared_tc_lut: self.prepared_tc_lut.as_ref(),
                    mallett_core: self.mallett_core.as_ref(),
                    front_illuminant: &self.front_illuminant,
                },
            );
            let calibrated = crate::pipeline_calibration::build_with_frontend(
                (*self.original_film).clone(),
                (*self.original_print).clone(),
                params,
                data_dir,
                backend,
                retained,
            )?;
            return Ok(ParameterUpdate {
                params: calibrated.params,
                material: Some(calibrated.material),
                convert_converter: None,
                conversion_tones,
                output_gamut: calibrated.output_gamut,
            });
        }
        let convert_converter = if params.workflow.route_enum().is_convert_film()
            && crate::stages::converting::converter_changed(&self.params, &params)
        {
            crate::stages::converting::resolve_converter(&self.film, &params)?
        } else {
            None
        };
        let output_gamut = crate::gamut_compression::OutputGamutCompress::build_with_backend(
            &params.io.output_gamut_compress,
            &params.io.output_color_space,
            backend,
        )?;
        Ok(ParameterUpdate {
            params,
            material: None,
            convert_converter,
            conversion_tones,
            output_gamut,
        })
    }

    pub(crate) fn apply_parameter_update(&mut self, update: ParameterUpdate) {
        if let Some(material) = update.material {
            self.film = material.film;
            self.print = material.print;
            self.convert_converter = material.convert_converter;
            self.print_exposure_factor = material.print_exposure_factor;
            self.print_illuminant = material.print_illuminant;
            self.preflash_raw = material.preflash_raw;
            self.midgray_density = material.midgray_density;
            self.compensated_midgray_density = material.compensated_midgray_density;
            if let Some(spectral) = material.spectral {
                self.tc_lut = spectral.tc_lut;
                self.prepared_tc_lut = spectral.prepared_tc_lut;
                self.mallett_core = spectral.mallett_core;
                self.front_illuminant = spectral.front_illuminant;
            }
        }
        if let Some(converter) = update.convert_converter {
            self.convert_converter = Some(converter);
        }
        self.params = update.params;
        self.output_gamut = update.output_gamut;
        self.conversion_tones = update.conversion_tones;
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
        let front_illuminant = film.info.reference_illuminant.clone();
        let convert_converter = crate::stages::converting::resolve_converter(&film, &params)
            .expect("validated film conversion");
        Self {
            film,
            print,
            params,
            tc_lut: None,
            prepared_tc_lut: None,
            mallett_core: None,
            convert_converter,
            front_illuminant,
            print_exposure_factor: 1.0,
            print_illuminant,
            preflash_raw: [0.0; 3],
            midgray_density: Vec::new(),
            compensated_midgray_density: Vec::new(),
            conversion_tones,
            output_gamut,
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
        let original_print = Arc::new(print.clone());
        let original_film = Arc::new(film.clone());
        let fotufilm_prepared = crate::stages::grain_fotufilm::Prepared::new(
            &original_film,
            params.film_render.development_time,
        );
        let conversion_tones = crate::stages::scanning::style::resolve(
            &params.scanner.conversion_style,
            Some(data_dir),
        )?;
        let calibrated = crate::pipeline_calibration::build_with_backend(
            film, print, params, data_dir, backend,
        )?;
        let material = calibrated.material;
        let spectral = material
            .spectral
            .expect("cold preparation owns its front end");
        Ok(Self {
            film: material.film,
            print: material.print,
            params: calibrated.params,
            tc_lut: spectral.tc_lut,
            prepared_tc_lut: spectral.prepared_tc_lut,
            mallett_core: spectral.mallett_core,
            convert_converter: material.convert_converter,
            front_illuminant: spectral.front_illuminant,
            print_exposure_factor: material.print_exposure_factor,
            print_illuminant: material.print_illuminant,
            preflash_raw: material.preflash_raw,
            midgray_density: material.midgray_density,
            compensated_midgray_density: material.compensated_midgray_density,
            conversion_tones,
            output_gamut: calibrated.output_gamut,
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
