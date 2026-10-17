/// 3-stage simulation pipeline: Filming → Printing → Scanning.
///
/// Full calibration chain:
///   1. Load spectra LUT → compute TC LUT for film sensitivity
///   2. Process virtual gray card through filming to get midgray spectral density
///   3. Compute print exposure normalization factor from midgray spectral density
///   4. Pass all calibration data to the pipeline stages
use crate::WorkflowRoute;
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
mod output_mix;
mod resident;
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
    /// Mallett2019 reflectance-basis core matrix (linear-sRGB → raw), when that
    /// upsampler is selected instead of the hanatos2025 tc LUT. Mutually
    /// exclusive with `tc_lut`.
    mallett_core: Option<[[f64; 3]; 3]>,
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
    /// Output gamut compressor (built once; identity unless oklch is enabled).
    output_gamut: crate::gamut_compression::OutputGamutCompress,
    /// Data directory the spectral front-end was built from. Present on
    /// `new_with_spectral` pipelines; lets `with_params` rebuild the TC LUT
    /// when a sensitivity-affecting control changes instead of reusing a
    /// stale one.
    data_dir: Option<std::path::PathBuf>,
    scanner_setup: Option<crate::stages::scanning::ScannerSetup>,
    digital_reference: Option<Arc<crate::digital_reference::DigitalReference>>,
    /// Original film reference captured before calibration mutates the render profile.
    original_film: Arc<Profile>,
    fotufilm_prepared: crate::stages::grain_fotufilm::Prepared,
    cancellation: Option<Arc<AtomicBool>>,
}
impl Pipeline {
    pub(crate) fn with_scanner_setup(
        mut self,
        setup: crate::stages::scanning::ScannerSetup,
    ) -> Self {
        self.scanner_setup = Some(setup);
        self
    }

    pub(crate) fn has_scanner_setup(&self) -> bool {
        self.scanner_setup.is_some()
    }
    pub(crate) fn reference_provenance(
        &self,
    ) -> Option<&crate::digital_reference::ReferenceProvenance> {
        self.digital_reference
            .as_deref()
            .map(|reference| &reference.provenance)
    }

    pub(crate) fn prepare_scanner_setup(
        &self,
        full_input: &ImageBuf,
    ) -> Result<crate::stages::scanning::ScannerSetup, String> {
        if self.params.scanner.auto_setup == crate::params::ScannerAutoSetup::Off {
            return Ok(crate::stages::scanning::ScannerSetup::IDENTITY);
        }
        crate::stages::grain_v3::validate_cell_spectral(&self.params)?;
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
        let mut measurement = self.clone();
        measurement.params.io.crop = false;
        measurement.params.io.upscale_factor = 1.0;
        measurement.params.settings.preview_mode = false;
        measurement.params.scanner.auto_setup = crate::params::ScannerAutoSetup::Off;
        measurement.scanner_setup = None;
        let color_ref = measurement.color_reference(&backend);
        let converting = self.params.workflow.route_enum() == WorkflowRoute::ConvertFilmScan;
        let density = if converting {
            crate::stages::converting::process(full_input, &measurement.film, &measurement.params)?
        } else {
            measurement.run_from(
                full_input.clone(),
                Tap::RgbIn,
                Tap::CmyFilm,
                &backend,
                &color_ref,
                None,
                None,
                None,
            )?
        };
        crate::stages::scanning::prepare_setup(
            &density,
            &measurement.film,
            &measurement.params,
            &color_ref,
            converting.then_some(self.params.film_render.convert.scan_illuminant.as_str()),
        )
    }

    fn scan_params(&self) -> std::borrow::Cow<'_, RuntimeParams> {
        match self.scanner_setup {
            Some(setup) => {
                let mut params = self.params.clone();
                setup.apply(&mut params);
                std::borrow::Cow::Owned(params)
            }
            None => std::borrow::Cow::Borrowed(&self.params),
        }
    }
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

    /// Return a copy of this calibrated pipeline with updated runtime params.
    ///
    /// When a sensitivity-affecting (spectral) control changes, the whole
    /// spectral front-end is rebuilt from `data_dir` — mirroring the cache
    /// invalidation of Python's `SpectralLUTService`, where
    /// `set_hanatos2025_adaptation` clears the filming tc_lut whenever the
    /// adaptation state changes and `get_filming_tc_lut` re-keys on the
    /// (camera-filtered) sensitivity. Reusing the stale LUT would silently
    /// ignore the new UV/IR filters, blur and surface settings.
    ///
    /// Calibration-affecting runtime controls are detected and rebuilt from
    /// the spectral source data; rebuild and validation failures are returned
    /// instead of retaining stale calibration.
    pub fn with_params(mut self, params: RuntimeParams) -> Result<Self, String> {
        let mut params = params;
        crate::params_builder::normalize_runtime_topology(&self.film, &mut params);
        params.validate()?;
        params.validate_color()?;
        crate::pipeline_calibration::validate_scan_output(&params, &self.film)?;
        if let Some(model) = self.print.data.density_curves_model.as_ref() {
            crate::print_morph::morph_density_curves(
                &self.print.log_exposure_f64(),
                model,
                &params.print_render.density_curves_morph,
                self.print.is_positive(),
            )
            .map_err(|error| format!("invalid print density-curve model: {error}"))?;
        }
        let spectral_changed = crate::pipeline_calibration::spectral_changed(&self.params, &params);
        let calibration_changed =
            crate::pipeline_calibration::calibration_changed(&self.params, &params);
        if self.params.scanner.negative_conversion_method
            != params.scanner.negative_conversion_method
        {
            self.digital_reference = if params.scanner.negative_conversion_method
                == crate::params::NegativeConversionMethod::DigitalReference
            {
                Some(Arc::new(
                    crate::digital_reference::DigitalReference::prepare(
                        &self.film,
                        self.data_dir
                            .as_deref()
                            .ok_or("Digital Reference requires a runtime data directory")?,
                    )?,
                ))
            } else {
                None
            };
        }
        if spectral_changed || calibration_changed {
            return self.rebuild_spectral(params);
        }
        self.output_gamut = crate::gamut_compression::OutputGamutCompress::build(
            &params.io.output_gamut_compress,
            &params.io.output_color_space,
        )?;
        if self.params != params {
            self.scanner_setup = None;
        }
        self.params = params;
        Ok(self)
    }
    /// Rebuild the spectral pipeline unconditionally, carrying the prepared
    /// grain cache and the bound cancellation signal across the rebuild.
    ///
    /// Film-curve controls such as `film_render.density_curve_gamma`,
    /// `film_render.development_time` and `film_render.chemistry` are absent
    /// from the spectral and calibration control keys, so a caller that must
    /// refresh the resolved film cannot rely on `with_params` alone.
    pub(crate) fn rebuild_spectral(self, params: RuntimeParams) -> Result<Self, String> {
        let data_dir = self
            .data_dir
            .clone()
            .ok_or_else(|| "pipeline lacks data_dir for calibration rebuild".to_owned())?;
        let mut rebuilt =
            Self::new_with_spectral(self.film.clone(), self.print.clone(), params, &data_dir)?;
        rebuilt.original_film = self.original_film.clone();
        rebuilt.fotufilm_prepared = self.fotufilm_prepared.clone();
        rebuilt.cancellation = self.cancellation.clone();
        Ok(rebuilt)
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
        let original_film = Arc::new(film.clone());
        let fotufilm_prepared = crate::stages::grain_fotufilm::Prepared::new(
            &original_film,
            params.film_render.development_time,
        );
        params
            .validate_color()
            .expect("invalid colour configuration");
        // B&W profiles must be resolved on every construction path —
        // an unresolved family would read development-time columns as
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
        let output_gamut = crate::gamut_compression::OutputGamutCompress::build(
            &params.io.output_gamut_compress,
            &params.io.output_color_space,
        )
        .expect("validated output gamut configuration");
        let front_illuminant = film.info.reference_illuminant.clone();
        Self {
            film,
            print,
            params,
            tc_lut: None,
            mallett_core: None,
            front_illuminant,
            print_exposure_factor: 1.0,
            print_illuminant,
            preflash_raw: [0.0; 3],
            output_gamut,
            data_dir: None,
            scanner_setup: None,
            digital_reference: None,
            original_film,
            fotufilm_prepared,
            cancellation: None,
        }
    }

    /// Create pipeline with full spectral calibration.
    pub fn new_with_spectral(
        film: Profile,
        print: Profile,
        params: RuntimeParams,
        data_dir: &Path,
    ) -> Result<Self, String> {
        let original_film = Arc::new(film.clone());
        let fotufilm_prepared = crate::stages::grain_fotufilm::Prepared::new(
            &original_film,
            params.film_render.development_time,
        );
        let calibrated = crate::pipeline_calibration::build(film, print, params, data_dir)?;
        let digital_reference = if calibrated.params.scanner.negative_conversion_method
            == crate::params::NegativeConversionMethod::DigitalReference
        {
            Some(Arc::new(
                crate::digital_reference::DigitalReference::prepare(&calibrated.film, data_dir)?,
            ))
        } else {
            None
        };
        Ok(Self {
            film: calibrated.film,
            print: calibrated.print,
            params: calibrated.params,
            tc_lut: calibrated.tc_lut,
            mallett_core: calibrated.mallett_core,
            front_illuminant: calibrated.front_illuminant,
            print_exposure_factor: calibrated.print_exposure_factor,
            print_illuminant: calibrated.print_illuminant,
            preflash_raw: calibrated.preflash_raw,
            output_gamut: calibrated.output_gamut,
            data_dir: calibrated.data_dir,
            scanner_setup: None,
            digital_reference,
            original_film,
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
        // Dye develops the full source; its field readout owns the crop and
        // output grid so that overlapping reads keep the same film coordinates.
        let resized = if crate::stages::grain_v3::active(&self.params)
            || crate::stages::grain_fotufilm::active(&self.params)
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

    /// Colour reference for one render — scanner/exposure corrections shared
    /// by the resident and per-stage routes, computed under the
    /// `color_reference` observation scope.
    fn color_reference(
        &self,
        backend: &dyn ComputeBackend,
    ) -> crate::color_reference::ColorReference {
        let _observation = crate::stages::StageObservation::cpu(
            backend,
            "color_reference",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        crate::color_reference::ColorReference::compute(
            &self.film,
            &self.print,
            &self.params,
            &self.print_illuminant,
            self.print_exposure_factor,
            self.preflash_raw,
        )
    }
}
