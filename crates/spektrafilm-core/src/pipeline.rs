/// 3-stage simulation pipeline: Filming → Printing → Scanning.
///
/// Full calibration chain:
///   1. Load spectra LUT → compute TC LUT for film sensitivity
///   2. Process virtual gray card through filming to get midgray spectral density
///   3. Compute print exposure normalization factor from midgray spectral density
///   4. Pass all calibration data to the pipeline stages
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use rayon::prelude::*;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};
use spektrafilm_math::spectral::TcLut;

mod resident;

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
fn stage_timings_enabled() -> bool {
    std::env::var_os("SPEKTRAFILM_STAGE_TIMINGS").is_some()
}

fn print_stage_timing(enabled: bool, stage: &str, start: Instant) {
    if enabled {
        eprintln!("stage {stage}: {} ms", start.elapsed().as_millis());
    }
}

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
use crate::stages;

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
    scanner_setup: Option<stages::scanning::ScannerSetup>,
    /// Original film reference captured before calibration mutates the render profile.
    original_film: Arc<Profile>,
    fotufilm_prepared: stages::grain_fotufilm::Prepared,
    cancellation: Option<Arc<AtomicBool>>,
}
impl Pipeline {
    pub(crate) fn with_scanner_setup(mut self, setup: stages::scanning::ScannerSetup) -> Self {
        self.scanner_setup = Some(setup);
        self
    }

    pub(crate) fn has_scanner_setup(&self) -> bool {
        self.scanner_setup.is_some()
    }

    pub(crate) fn prepare_scanner_setup(
        &self,
        full_input: &ImageBuf,
    ) -> Result<stages::scanning::ScannerSetup, String> {
        if self.params.scanner.auto_setup == crate::params::ScannerAutoSetup::Off {
            return Ok(stages::scanning::ScannerSetup::IDENTITY);
        }
        stages::grain_v3::validate_cell_spectral(&self.params)?;
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let mut measurement = self.clone();
        measurement.params.io.crop = false;
        measurement.params.io.upscale_factor = 1.0;
        measurement.params.settings.preview_mode = false;
        measurement.params.scanner.auto_setup = crate::params::ScannerAutoSetup::Off;
        measurement.scanner_setup = None;
        let color_ref = measurement.color_reference(&backend);
        let converting = self.params.workflow.route == "input > convert-film > scan";
        let density = if converting {
            stages::converting::process(full_input, &measurement.film, &measurement.params)?
        } else {
            measurement.run_from(
                full_input.clone(),
                Tap::RgbIn,
                Tap::CmyFilm,
                &backend,
                &color_ref,
                None,
                None,
            )?
        };
        stages::scanning::prepare_setup(
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
    fn effective_backend<'a>(&self, backend: &'a dyn ComputeBackend) -> &'a dyn ComputeBackend {
        if self.params.scanner.scan_output == "negative_to_positive" {
            static CPU_BACKEND: spektrafilm_gpu::cpu_backend::CpuBackend =
                spektrafilm_gpu::cpu_backend::CpuBackend;
            &CPU_BACKEND
        } else {
            backend
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
        let fotufilm_prepared = stages::grain_fotufilm::Prepared::new(
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
        let fotufilm_prepared = stages::grain_fotufilm::Prepared::new(
            &original_film,
            params.film_render.development_time,
        );
        let calibrated = crate::pipeline_calibration::build(film, print, params, data_dir)?;
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
            original_film,
            fotufilm_prepared,
            cancellation: None,
        })
    }

    /// Topological tap order of the active chain — the Rust spelling of
    /// upstream `_build_topology`. `io.scan_film` short-circuits the print
    /// stages exactly like the Python node list.
    fn tap_order(&self) -> &'static [Tap] {
        if self.params.io.scan_film {
            &[
                Tap::RgbIn,
                Tap::RgbPre,
                Tap::LogEFilm,
                Tap::CmyFilm,
                Tap::RgbOut,
            ]
        } else {
            &[
                Tap::RgbIn,
                Tap::RgbPre,
                Tap::LogEFilm,
                Tap::CmyFilm,
                Tap::LogEPrint,
                Tap::CmyPrint,
                Tap::RgbOut,
            ]
        }
    }

    /// One topology node: transform the image at tap `at` into its value at
    /// the next tap in [`Pipeline::tap_order`]. `RgbOut` is terminal.
    #[allow(clippy::too_many_arguments)]
    fn fire_node(
        &self,
        at: Tap,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        color_ref: &crate::color_reference::ColorReference,
        pixel_size_um: f64,
        ae_ev: f64,
        stage_timings: &mut Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        let print_timings = stage_timings_enabled();
        let name = match at {
            Tap::RgbPre => "filming_expose",
            Tap::LogEFilm => "filming_develop",
            Tap::CmyFilm if self.params.io.scan_film => "scanning",
            Tap::CmyFilm | Tap::LogEPrint => "printing",
            Tap::CmyPrint => "scanning",
            Tap::RgbIn | Tap::RgbOut => "unreachable_tap",
        };
        let mut observation = stages::StageObservation::cpu(
            backend,
            name,
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        let backend = observation.backend(backend);
        match at {
            Tap::RgbIn => unreachable!("rgb_in preprocessing is handled by run_from"),
            Tap::RgbPre => {
                let t = Instant::now();
                let out = stages::filming::expose(
                    &image,
                    &self.film,
                    &self.params,
                    backend,
                    self.tc_lut.as_ref(),
                    self.mallett_core.as_ref(),
                    &select_illuminant(&self.front_illuminant),
                    color_ref.filming_exposure_correction,
                    pixel_size_um,
                    ae_ev,
                );
                print_stage_timing(print_timings, "filming_expose", t);
                record_stage_timing(stage_timings, "filming_expose", t);
                dump_if_env("SPEKTRAFILM_DUMP_FILM_LOG_RAW", &out);
                Ok(out)
            }
            Tap::LogEFilm => {
                let t = Instant::now();
                let out = stages::filming::develop(
                    &image,
                    &self.film,
                    &self.params,
                    backend,
                    pixel_size_um,
                );
                print_stage_timing(print_timings, "filming_develop", t);
                record_stage_timing(stage_timings, "filming_develop", t);
                tracing::info!("pipeline: filming complete");
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &out);
                Ok(out)
            }
            Tap::CmyFilm => {
                if self.params.io.scan_film {
                    let t = Instant::now();
                    let out = stages::scanning::process(
                        &image,
                        &self.film,
                        &self.scan_params(),
                        backend,
                        color_ref,
                        &self.output_gamut,
                    )?;
                    print_stage_timing(print_timings, "scanning", t);
                    record_stage_timing(stage_timings, "scanning", t);
                    tracing::info!("pipeline: scanning complete (film scan)");
                    Ok(out)
                } else {
                    let t = Instant::now();
                    let out = stages::printing::expose_calibrated(
                        &image,
                        &self.film,
                        &self.print,
                        &self.params,
                        backend,
                        &self.print_illuminant,
                        self.print_exposure_factor,
                        self.preflash_raw,
                        color_ref.printing_exposure_correction,
                        pixel_size_um,
                    );
                    print_stage_timing(print_timings, "printing", t);
                    record_stage_timing(stage_timings, "printing", t);
                    dump_if_env("SPEKTRAFILM_DUMP_PRINT_LOG_RAW", &out);
                    Ok(out)
                }
            }
            Tap::LogEPrint => {
                let t = Instant::now();
                let result = stages::printing::develop(&image, &self.print, &self.params, backend);
                observation.set_complete(result.is_ok());
                let out = result?;
                print_stage_timing(print_timings, "printing_develop", t);
                record_stage_timing(stage_timings, "printing_develop", t);
                dump_if_env("SPEKTRAFILM_DUMP_PRINT_DENSITY", &out);
                Ok(out)
            }
            Tap::CmyPrint => {
                let t = Instant::now();
                let out = stages::scanning::process(
                    &image,
                    &self.print,
                    &self.params,
                    backend,
                    color_ref,
                    &self.output_gamut,
                )?;
                print_stage_timing(print_timings, "scanning", t);
                record_stage_timing(stage_timings, "scanning", t);
                tracing::info!("pipeline: printing complete");
                Ok(out)
            }
            Tap::RgbOut => unreachable!("RgbOut is terminal — run_from stops before it"),
        }
    }
    fn validate_grain_v3_taps(
        cell_spectral: bool,
        aperture_factor: Option<u32>,
        inject: Tap,
        collect: Tap,
        scan_film: bool,
    ) -> Result<(), String> {
        if !cell_spectral {
            return Ok(());
        }
        if aperture_factor.is_some() {
            return Err("SPEKTRAFILM_GRAIN_V3_CELL_SPECTRAL conflicts with SPEKTRAFILM_GRAIN_V3_APERTURE_FACTOR".to_owned());
        }
        let supported_collect = collect == Tap::RgbOut
            || (!scan_film && matches!(collect, Tap::LogEPrint | Tap::CmyPrint));
        if !matches!(inject, Tap::RgbIn | Tap::RgbPre | Tap::LogEFilm) || !supported_collect {
            return Err("SPEKTRAFILM_GRAIN_V3_CELL_SPECTRAL requires injection at or before log_e_film and optical collection (print log_e_print/cmy_print/rgb_out, film rgb_out); cmy_film is a density diagnostic".to_owned());
        }
        Ok(())
    }

    /// Run the topology from the value at `inject` to the value at
    /// `collect` (both inclusive bounds in [`Pipeline::tap_order`]).
    /// Mirrors upstream `run_topology`: unreachable pairs error with the
    /// upstream message, `inject == collect` returns the input unchanged.
    #[allow(clippy::too_many_arguments)]
    fn run_from(
        &self,
        image: ImageBuf,
        inject: Tap,
        collect: Tap,
        backend: &dyn ComputeBackend,
        color_ref: &crate::color_reference::ColorReference,
        physical_context: Option<(f64, f64)>,
        stage_timings: Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        self.check_cancelled()?;
        stages::grain_fotufilm::validate_supported(&self.film, &self.params)?;
        let order = self.tap_order();
        let Some(ip) = order.iter().position(|t| *t == inject) else {
            return Err(format!(
                "tap {inject:?} is not part of this topology (scan_film = {})",
                self.params.io.scan_film
            ));
        };
        let Some(cp) = order.iter().position(|t| *t == collect) else {
            return Err(format!(
                "tap {collect:?} is not part of this topology (scan_film = {})",
                self.params.io.scan_film
            ));
        };
        if cp < ip {
            return Err(format!(
                "no node path reaches tap {:?} from {:?}",
                collect, inject
            ));
        }
        let aperture_factor = stages::grain_v3::validate_aperture(&self.params)?;
        let cell_spectral = stages::grain_v3::validate_cell_spectral(&self.params)?;
        Self::validate_grain_v3_taps(
            cell_spectral,
            aperture_factor,
            inject,
            collect,
            self.params.io.scan_film,
        )?;
        if cell_spectral && color_ref.has_remap() {
            return Err(
                "SPEKTRAFILM_GRAIN_V3_CELL_SPECTRAL does not support scanner luminance remap"
                    .to_owned(),
            );
        }
        if aperture_factor.is_some()
            && (collect != Tap::RgbOut
                || ip > order.iter().position(|t| *t == Tap::LogEFilm).unwrap())
        {
            return Err("SPEKTRAFILM_GRAIN_V3_APERTURE_FACTOR requires injection at or before log_e_film and collect=rgb_out".to_owned());
        }
        let (mut cur, mut pixel_size_um, ae_ev) = if inject == Tap::RgbIn && cp > ip {
            stages::grain_v3::validate_supported(&self.film, &self.params)?;
            let image = self.expose_full_input(image, backend);
            let (working, pitch) = self.rescale_working(&image, backend)?;
            let working = match working {
                std::borrow::Cow::Owned(working) => working,
                std::borrow::Cow::Borrowed(_) => image,
            };
            // The exposure multiply already ran, so the stage sees EV 0 —
            // matching upstream's prepared-input contract.
            (working, pitch, 0.0)
        } else {
            let (pitch, ae_ev) = physical_context.unwrap_or_else(|| {
                (
                    self.params.camera.film_format_mm as f64 * 1000.0
                        / image.width.max(image.height).max(1) as f64,
                    0.0,
                )
            });
            (image, pitch, ae_ev)
        };
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(cur.width, cur.height);
        }
        let mut stage_timings = stage_timings;
        let mut aperture_output_pitch_um = None;
        let mut i = ip;
        if inject == Tap::RgbIn && cp > ip {
            i += 1;
        }
        let mut output_crop = None;
        while i < cp {
            self.check_cancelled()?;
            cur = self.fire_node(
                order[i],
                cur,
                backend,
                color_ref,
                pixel_size_um,
                ae_ev,
                &mut stage_timings,
            )?;
            if order[i] == Tap::LogEFilm && stages::grain_v3::active(&self.params) {
                let t = Instant::now();
                let geometry = stages::grain_v3::geometry(&cur, &self.params, pixel_size_um)?;
                let glare = &self.params.print_render.glare;
                let scan_spatial = collect == Tap::RgbOut
                    && (self.params.scanner.lens_blur > 0.0
                        || self.params.scanner.unsharp_mask.iter().all(|&v| v > 0.0)
                        || (!self.params.io.scan_film && glare.active && glare.percent > 0.0));
                let print_spatial = !self.params.io.scan_film
                    && collect != Tap::CmyFilm
                    && self.params.enlarger.diffusion_filter.active;
                let grid = if let Some(factor) = aperture_factor {
                    stages::grain_v3::aperture_grid(&geometry, factor)?
                } else if self.params.io.crop && (scan_spatial || print_spatial) {
                    let (grid, crop) = geometry.full_film_grid(&cur)?;
                    output_crop = Some(crop);
                    grid
                } else {
                    geometry.grid()
                };
                // The existing scalar optical controls use the actual pitch
                // on the full source's long axis. Rounding may make the other
                // axis differ; the field itself retains both exact pitches.
                let long_axis = usize::from(cur.height > cur.width);
                let requested_pitch_um = geometry.output_pitch_um[long_axis];
                if cell_spectral {
                    cur = stages::grain_v3::readout_with_response(
                        &cur,
                        &self.film,
                        &self.params,
                        pixel_size_um,
                        grid,
                        backend,
                        |cells, backend| {
                            if self.params.io.scan_film {
                                stages::scanning::capture_linear(
                                    cells,
                                    &self.film,
                                    &self.params,
                                    backend,
                                    color_ref,
                                )
                            } else {
                                stages::printing::expose_linear_calibrated(
                                    cells,
                                    &self.film,
                                    &self.print,
                                    &self.params,
                                    backend,
                                    &self.print_illuminant,
                                    self.print_exposure_factor,
                                    self.preflash_raw,
                                    color_ref.printing_exposure_correction,
                                )
                            }
                        },
                    )?;
                    pixel_size_um = requested_pitch_um;
                    if self.params.io.scan_film {
                        let observation = stages::StageObservation::new(backend, "scanning");
                        cur = stages::scanning::finish_capture(
                            cur,
                            &self.film,
                            &self.params,
                            observation.backend(backend),
                            color_ref,
                            &self.output_gamut,
                        )?;
                        i = order.iter().position(|tap| *tap == Tap::RgbOut).unwrap();
                    } else {
                        cur.data.par_iter_mut().for_each(|value| {
                            *value = from_f64(to_f64(*value).log10());
                        });
                        dump_if_env("SPEKTRAFILM_DUMP_PRINT_LOG_RAW", &cur);
                        i = order.iter().position(|tap| *tap == Tap::LogEPrint).unwrap();
                    }
                    print_stage_timing(stage_timings_enabled(), "grain_v3_cell_spectral", t);
                    record_stage_timing(&mut stage_timings, "grain_v3_cell_spectral", t);
                    continue;
                }
                cur = stages::grain_v3::readout(
                    &cur,
                    &self.film,
                    &self.params,
                    pixel_size_um,
                    grid,
                    backend,
                )?;
                pixel_size_um = if let Some(factor) = aperture_factor {
                    aperture_output_pitch_um = Some(requested_pitch_um);
                    requested_pitch_um / factor as f64
                } else {
                    requested_pitch_um
                };
                print_stage_timing(stage_timings_enabled(), "grain_v3_readout", t);
                record_stage_timing(&mut stage_timings, "grain_v3_readout", t);
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &cur);
            }
            if order[i] == Tap::LogEFilm && stages::grain_fotufilm::active(&self.params) {
                let t = Instant::now();
                let geometry = stages::grain_v3::geometry(&cur, &self.params, pixel_size_um)?;
                let glare = &self.params.print_render.glare;
                let scan_spatial = collect == Tap::RgbOut
                    && (self.params.scanner.lens_blur > 0.0
                        || self.params.scanner.unsharp_mask.iter().all(|&v| v > 0.0)
                        || (!self.params.io.scan_film && glare.active && glare.percent > 0.0));
                let print_spatial = !self.params.io.scan_film
                    && collect != Tap::CmyFilm
                    && self.params.enlarger.diffusion_filter.active;
                let grid = if self.params.io.crop && (scan_spatial || print_spatial) {
                    let (grid, crop) = geometry.full_film_grid(&cur)?;
                    output_crop = Some(crop);
                    grid
                } else {
                    geometry.grid()
                };
                let long_axis = usize::from(cur.height > cur.width);
                cur = self.fotufilm_prepared.readout(
                    &cur,
                    &self.film,
                    &self.params,
                    pixel_size_um,
                    grid,
                    backend,
                    &|| self.check_cancelled(),
                )?;
                pixel_size_um = geometry.output_pitch_um[long_axis];
                record_stage_timing(&mut stage_timings, "grain_fotufilm_readout", t);
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &cur);
            }
            if order[i] == Tap::CmyFilm
                && let Some(factor) = aperture_factor
            {
                let t = Instant::now();
                cur = stages::grain_v3::collapse_log_exposure(&cur, factor)?;
                pixel_size_um = aperture_output_pitch_um.ok_or_else(|| {
                    "film grain Dye aperture: missing requested output pitch".to_owned()
                })?;
                print_stage_timing(stage_timings_enabled(), "grain_v3_aperture", t);
                record_stage_timing(&mut stage_timings, "grain_v3_aperture", t);
                tracing::info!(factor, "film grain Dye: collapsed linear print exposure");
            }
            i += 1;
        }
        Ok(match output_crop {
            Some(rect) => stages::grain_v3::crop_output(cur, rect),
            None => cur,
        })
    }

    /// Process an image with explicit pipeline taps — the Rust spelling of
    /// upstream `SimulationPipeline.process(image, inject, collect)`.
    ///
    /// Omitted endpoints use persistent `params.taps`, then `rgb_in` /
    /// `rgb_out`. Injecting at `log_e_film` or later bypasses the camera
    /// (and film) stages upstream of the injection point; collecting at
    /// `cmy_film` returns film densities instead of final RGB. Invalid or
    /// unreachable pairs fail with the upstream error text.
    ///
    /// Nondefault endpoints use per-stage boundaries. The normal end-to-end
    /// pair retains the fused GPU-resident path and full-input preparation.
    pub fn process_with_taps(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps_timed(image, backend, inject, collect, None)
    }

    /// Process while collecting per-node wall-clock timings.
    pub fn process_with_timings(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        timings: &mut BTreeMap<String, f64>,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps_timed(image, backend, None, None, Some(timings))
    }

    pub fn process_with_taps_timed(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
        mut timings: Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        let inject = match inject {
            Some(tap) => tap,
            None => self
                .params
                .taps
                .inject
                .as_deref()
                .map(Tap::parse)
                .transpose()?
                .unwrap_or(Tap::RgbIn),
        };
        if self.params.scanner.auto_setup == crate::params::ScannerAutoSetup::PerFrame
            && self.scanner_setup.is_none()
        {
            if inject != Tap::RgbIn {
                return Err("per_frame scanner setup requires the full rgb_in input".into());
            }
            let setup = self.prepare_scanner_setup(&image)?;
            let mut prepared = self.clone();
            prepared.scanner_setup = Some(setup);
            return prepared.process_with_taps_timed(
                image,
                backend,
                Some(inject),
                collect,
                timings,
            );
        }
        let cpu_fallback = if self.params.scanner.scan_output == "negative_to_positive" {
            backend
                .observation_context()
                .filter(|context| context.enabled())
                .map(|context| {
                    if backend.is_gpu() {
                        context.decline_resident(
                            spektrafilm_gpu::telemetry::ResidentDeclineReason::NegativeToPositiveOutput,
                        );
                    }
                    spektrafilm_gpu::bind_cpu_fallback(
                        context.clone(),
                        spektrafilm_gpu::telemetry::CpuReason::NegativeToPositiveOutput,
                    )
                })
        } else {
            None
        };
        let backend = cpu_fallback
            .as_deref()
            .unwrap_or_else(|| self.effective_backend(backend));
        let collect = match collect {
            Some(tap) => tap,
            None => self
                .params
                .taps
                .collect
                .as_deref()
                .map(Tap::parse)
                .transpose()?
                .unwrap_or(Tap::RgbOut),
        };
        if stages::grain_v3::active(&self.params) {
            let cell_spectral = stages::grain_v3::validate_cell_spectral(&self.params)?;
            let aperture_factor = stages::grain_v3::validate_aperture(&self.params)?;
            if cell_spectral {
                stages::grain_v3::validate_supported(&self.film, &self.params)?;
            }
            Self::validate_grain_v3_taps(
                cell_spectral,
                aperture_factor,
                inject,
                collect,
                self.params.io.scan_film,
            )?;
        }
        if let Some(context) = backend.observation_context()
            && (inject != Tap::RgbIn || collect != Tap::RgbOut)
        {
            context.decline_resident(
                spektrafilm_gpu::telemetry::ResidentDeclineReason::DiagnosticTapRoute,
            );
            context.set_path(if backend.is_gpu() {
                spektrafilm_gpu::telemetry::ExecutionPath::PerStage
            } else {
                spektrafilm_gpu::telemetry::ExecutionPath::Cpu
            });
        }
        if inject == Tap::RgbIn && collect == Tap::RgbOut {
            return self.process_full(image, backend, timings);
        }
        tracing::info!(
            inject = inject.name(),
            collect = collect.name(),
            "pipeline: start"
        );
        let t = Instant::now();
        let color_ref = self.color_reference(backend);
        record_stage_timing(&mut timings, "color_reference", t);
        self.run_from(image, inject, collect, backend, &color_ref, None, timings)
    }

    pub fn process(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps(image, backend, None, None)
    }

    fn process_full(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        mut timings: Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        // Active Dye outside its supported condition fails here, before any
        // route work — including the passthrough and convert routes, which
        // never reach a grain stage.
        stages::grain_v3::validate_supported(&self.film, &self.params)?;
        stages::grain_fotufilm::validate_supported(&self.film, &self.params)?;
        self.check_cancelled()?;
        if self.params.workflow.route == "input" {
            let _observation = stages::StageObservation::cpu(
                backend,
                "post_scan",
                spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
            );
            if let Some(context) = backend.observation_context() {
                context.set_path(spektrafilm_gpu::telemetry::ExecutionPath::Cpu);
                context.set_working_dimensions(image.width, image.height);
                context.decline_resident(
                    spektrafilm_gpu::telemetry::ResidentDeclineReason::WorkflowRoute,
                );
            }
            use spektrafilm_math::colorspace::{conversion_matrix, convert_rgb, resolve};
            let source = resolve(&self.params.io.input_color_space)?;
            let destination = resolve(&self.params.io.output_color_space)?;
            let matrix = conversion_matrix(source, destination);
            let mut output = image;
            output.data.par_chunks_exact_mut(3).for_each(|pixel| {
                let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
                let rgb = convert_rgb(
                    rgb,
                    source,
                    self.params.io.input_cctf_decoding,
                    destination,
                    self.params.io.output_cctf_encoding,
                    &matrix,
                );
                for channel in 0..3 {
                    pixel[channel] = from_f64(rgb[channel]);
                }
            });
            // Finished RGB accepts the optional magazine finishing exactly like
            // a scanned base workflow: after output color management, before
            // the caller's image writer.
            crate::magazine_print_color::apply(
                &mut output,
                &self.params.magazine_print_color,
                destination,
                self.params.io.output_cctf_encoding,
            );
            return Ok(output);
        }
        let image = self.expose_full_input(image, backend);
        let t = Instant::now();
        let (working, pixel_size_um) = self.rescale_working(&image, backend)?;
        print_stage_timing(stage_timings_enabled(), "resize", t);
        record_stage_timing(&mut timings, "resize", t);
        let working = match working {
            std::borrow::Cow::Owned(working) => working,
            std::borrow::Cow::Borrowed(_) => image,
        };
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(working.width, working.height);
        }
        if self
            .params
            .workflow
            .route
            .starts_with("input > convert-film")
        {
            return self.process_convert(working, pixel_size_um, backend);
        }
        self.run(working, pixel_size_um, 0.0, backend, timings)
    }

    fn process_convert(
        &self,
        image: ImageBuf,
        pixel_size_um: f64,
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        if let Some(context) = backend.observation_context() {
            context.set_path(if backend.is_gpu() {
                spektrafilm_gpu::telemetry::ExecutionPath::PerStage
            } else {
                spektrafilm_gpu::telemetry::ExecutionPath::Cpu
            });
            context
                .decline_resident(spektrafilm_gpu::telemetry::ResidentDeclineReason::WorkflowRoute);
        }
        let route = self.params.workflow.route.as_str();
        let print_after_convert = route == "input > convert-film > print > scan";
        let scan_minus_base = route == "input > convert-film > scan-minus-base";
        let mut converting = stages::StageObservation::cpu(
            backend,
            "converting",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        let converted = stages::converting::process(&image, &self.film, &self.params);
        converting.set_complete(converted.is_ok());
        let film_density = converted?;
        drop(converting);
        let color_ref = self.color_reference(backend);
        if print_after_convert {
            let mut printing = stages::StageObservation::new(backend, "printing");
            let printing_backend = printing.backend(backend);
            let print_density = stages::printing::expose_calibrated(
                &film_density,
                &self.film,
                &self.print,
                &self.params,
                printing_backend,
                &self.print_illuminant,
                self.print_exposure_factor,
                self.preflash_raw,
                color_ref.printing_exposure_correction,
                pixel_size_um,
            );
            let developed = stages::printing::develop(
                &print_density,
                &self.print,
                &self.params,
                printing_backend,
            );
            printing.set_complete(developed.is_ok());
            let print_density = developed?;
            drop(printing);
            let scanning = stages::StageObservation::new(backend, "scanning");
            let backend = scanning.backend(backend);
            Ok(stages::scanning::process(
                &print_density,
                &self.print,
                &self.params,
                backend,
                &color_ref,
                &self.output_gamut,
            )?)
        } else {
            let mut scan_params = self.scan_params().into_owned();
            scan_params.io.scan_film = true;
            let scanning = stages::StageObservation::new(backend, "scanning");
            let backend = scanning.backend(backend);
            Ok(stages::scanning::scan_with_options(
                &film_density,
                &self.film,
                &scan_params,
                backend,
                &color_ref,
                &self.output_gamut,
                Some(&self.params.film_render.convert.scan_illuminant),
                !scan_minus_base,
            )?)
        }
    }

    /// Shared CPU/GPU chain from the prepared working image (crop +
    /// upscale already applied). `pixel_size_um` is the pitch derived
    /// from the full input; `ae_ev` the EV metered on the full input.
    fn run(
        &self,
        image: ImageBuf,
        pixel_size_um: f64,
        ae_ev: f64,
        backend: &dyn ComputeBackend,
        mut timings: Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        let stage_timings = stage_timings_enabled();
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(image.width, image.height);
        }
        // Scanner B&W/slide exposure correction (no-op unless scanner
        // white/black correction is on for a slide or print scan). Computed
        // up front so both the GPU-resident and per-stage paths share it.
        let t = Instant::now();
        let color_ref = self.color_reference(backend);
        print_stage_timing(stage_timings, "color_reference", t);
        record_stage_timing(&mut timings, "color_reference", t);

        // GPU fast path: dispatch the whole filming→printing→scanning chain
        // (or filming→scanning when scan_film) as a single GPU command
        // buffer — one upload + one readback total. WGSL runs the
        // extended resident chain, including camera/scanner lens blur and
        // highlight boost. Unsupported effects select the faithful per-stage
        // path; backends without resident support return `None`. Active Dye
        // selects a per-stage path through `resident_decision`.
        if (self.tc_lut.is_some() || self.mallett_core.is_some())
            && let Some(out) =
                self.try_gpu_resident(&image, backend, &color_ref, pixel_size_um, ae_ev)
        {
            tracing::info!("pipeline: gpu-resident fast path complete");
            let _post = stages::StageObservation::cpu(
                backend,
                "post_scan",
                spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
            );
            return Ok(self.apply_post_scan(out));
        }
        if let Some(context) = backend.observation_context() {
            context.set_path(if backend.is_gpu() {
                spektrafilm_gpu::telemetry::ExecutionPath::PerStage
            } else {
                spektrafilm_gpu::telemetry::ExecutionPath::Cpu
            });
        }

        // Per-stage path — the same topology `process_with_taps` walks, from
        // the rescaled working image (rgb_pre) to rgb_out.
        self.run_from(
            image,
            Tap::RgbPre,
            Tap::RgbOut,
            backend,
            &color_ref,
            Some((pixel_size_um, ae_ev)),
            timings,
        )
    }

    /// Run only the GPU-resident path from a borrowed image. This is used by
    /// the live GUI, so a full-resolution preview does not need to
    /// deep-clone the loaded image just to satisfy `process(ImageBuf)`.
    ///
    /// Working geometry (crop + upscale, `crop_and_rescale`) is applied
    /// here from the borrow; with neither active the image stays
    /// zero-copy. Auto-exposure is metered on the full input exactly as
    /// [`Self::process`] does.
    ///
    /// `Ok(None)` means the backend cannot run the resident chain and the
    /// caller should fall back to [`Self::process`]; `Err` carries an
    /// invalid-geometry error that must fail the render.
    pub fn process_resident_borrowed(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<Option<ImageBuf>, String> {
        let inject = self
            .params
            .taps
            .inject
            .as_deref()
            .map(Tap::parse)
            .transpose()?
            .unwrap_or(Tap::RgbIn);
        let collect = self
            .params
            .taps
            .collect
            .as_deref()
            .map(Tap::parse)
            .transpose()?
            .unwrap_or(Tap::RgbOut);
        if inject != Tap::RgbIn || collect != Tap::RgbOut {
            if let Some(context) = backend.observation_context() {
                context.decline_resident(
                    spektrafilm_gpu::telemetry::ResidentDeclineReason::DiagnosticTapRoute,
                );
            }
            return Ok(None);
        }
        if self.tc_lut.is_none() && self.mallett_core.is_none() {
            if let Some(context) = backend.observation_context() {
                context.decline_resident(
                    spektrafilm_gpu::telemetry::ResidentDeclineReason::MissingResidentFrontPass,
                );
            }
            return Ok(None);
        }
        tracing::info!(
            backend = backend.name(),
            "pipeline: borrowed resident start"
        );
        let metering = stages::StageObservation::cpu(
            backend,
            "metering",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        let ae_ev = self.meter_autoexposure(image);
        // Clone only when the exposure multiply actually needs owned pixels;
        // a full-resolution preview must not deep-clone a no-op exposure.
        let exposed;
        let image = if self.params.camera.auto_exposure {
            exposed = self.apply_autoexposure(image.clone(), ae_ev);
            &exposed
        } else {
            image
        };
        drop(metering);
        let (working, pixel_size_um) = self.rescale_working(image, backend)?;
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(working.width, working.height);
        }
        let color_ref = self.color_reference(backend);
        let out = match self.try_gpu_resident(&working, backend, &color_ref, pixel_size_um, 0.0) {
            Some(out) => out,
            None => return Ok(None),
        };
        let _post = stages::StageObservation::cpu(
            backend,
            "post_scan",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        Ok(Some(self.apply_post_scan(out)))
    }
    fn apply_autoexposure(&self, mut image: ImageBuf, ev: f64) -> ImageBuf {
        if self.params.camera.auto_exposure {
            let scale = from_f64(2.0f64.powf(ev));
            image.data.par_iter_mut().for_each(|value| *value *= scale);
        }
        image
    }

    /// Meter the auto-exposure EV on the given (full, pre-geometry)
    /// image when auto-exposure is enabled.
    fn meter_autoexposure(&self, image: &ImageBuf) -> f64 {
        if !self.params.camera.auto_exposure {
            return 0.0;
        }
        stages::filming::meter_autoexposure_ev(image, &self.params)
    }

    /// Metering plus the exposure multiply, under the `metering` observation
    /// scope. Both owned-image entry points apply the metered EV immediately,
    /// so the EV itself stays internal.
    fn expose_full_input(&self, image: ImageBuf, backend: &dyn ComputeBackend) -> ImageBuf {
        if self
            .params
            .workflow
            .route
            .starts_with("input > convert-film")
        {
            return image;
        }
        let metering = stages::StageObservation::cpu(
            backend,
            "metering",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        let ae_ev = self.meter_autoexposure(&image);
        let image = self.apply_autoexposure(image, ae_ev);
        drop(metering);
        image
    }

    /// Crop + rescale under the `geometry` observation scope. Callers resolve
    /// the returned borrow against the image they still hold, so the
    /// no-geometry case stays zero-copy.
    fn rescale_working<'a>(
        &self,
        image: &'a ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<(std::borrow::Cow<'a, ImageBuf>, f64), String> {
        let mut geometry = stages::StageObservation::cpu(
            backend,
            "geometry",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        // Dye develops the full source; its field readout owns the crop and
        // output grid so that overlapping reads keep the same film coordinates.
        let resized = if stages::grain_v3::active(&self.params)
            || stages::grain_fotufilm::active(&self.params)
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
        let _observation = stages::StageObservation::cpu(
            backend,
            "color_reference",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
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

#[cfg(test)]
mod spectral_invalidation_tests {
    use super::*;

    fn data_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    }

    #[test]
    fn film_chemistry_rebuilds_total_and_grain_layers_from_the_fitted_model() {
        let mut film =
            crate::profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
        let fitted = film.data.density_curves.clone();
        let fitted_layers = film.data.density_curves_layers.clone();
        for active in [false, true] {
            film.data
                .density_curves
                .iter_mut()
                .flatten()
                .for_each(|v| *v = 0.5);
            film.data
                .density_curves_layers
                .iter_mut()
                .flatten()
                .flatten()
                .for_each(|v| *v = 0.1);
            apply_film_chemistry(
                &mut film,
                &crate::params::PrintCurvesMorphParams {
                    active,
                    ..Default::default()
                },
            )
            .unwrap();
            for (got, want) in film
                .data
                .density_curves
                .iter()
                .flatten()
                .zip(fitted.iter().flatten())
            {
                assert!((got - want).abs() < 1e-12);
            }
            for (got, want) in film
                .data
                .density_curves_layers
                .iter()
                .flatten()
                .flatten()
                .zip(fitted_layers.iter().flatten().flatten())
            {
                assert!((got - want).abs() < 1e-12);
            }
        }
        apply_film_chemistry(
            &mut film,
            &crate::params::PrintCurvesMorphParams {
                gamma_factor: 1.1,
                gamma_factor_fast: 0.9,
                gamma_factor_slow: 1.2,
                developer_exhaustion: 0.3,
                ..Default::default()
            },
        )
        .unwrap();
        // Pinned 28bf883e apply_print_curves_morph_with_layers, Portra 400 sample 128.
        let expected = [
            [0.5911804126249434, 0.5581332496586162, 0.6521190325040326],
            [0.4509958575813556, 0.4580461526112701, 0.5776553146563388],
            [
                0.017750303385221305,
                0.026894456204068887,
                0.034051265456119285,
            ],
        ];
        for channel in 0..3 {
            for (layer, expected_layer) in expected.iter().enumerate() {
                assert!(
                    (film.data.density_curves_layers[128][layer][channel]
                        - expected_layer[channel])
                        .abs()
                        < 1e-9
                );
            }
            let sum: f64 = expected.iter().map(|layer| layer[channel]).sum();
            assert!((film.data.density_curves[128][channel] - sum).abs() < 1e-9);
        }
    }

    /// Keep one prepared fixture per test process. `Pipeline::clone` shares the
    /// immutable Fotufilm preparation through `Prepared`'s `Arc`; each caller
    /// still receives an independent parameter set, while repeated render tests
    /// avoid rebuilding the same population and bank.
    fn build() -> Pipeline {
        static SHARED: std::sync::OnceLock<Pipeline> = std::sync::OnceLock::new();
        SHARED
            .get_or_init(|| {
                let dir = data_dir();
                let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
                let print =
                    crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
                Pipeline::new_with_spectral(film, print, RuntimeParams::default(), &dir).unwrap()
            })
            .clone()
    }
    #[test]
    fn fotufilm_zero_amount_bypasses_geometry_and_cancelled_render_fails() {
        let mut pipeline = build();
        pipeline.params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        pipeline.params.film_render.grain.active = true;
        pipeline.params.film_render.grain.fotufilm.amount = 0.0;
        pipeline.params.io.upscale_factor = 0.5;
        let image = ImageBuf::new(8, 6);
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let (working, _) = pipeline.rescale_working(&image, &backend).unwrap();
        assert_eq!((working.width, working.height), (4, 3));
        pipeline.params.film_render.grain.fotufilm.amount = 1.0;
        let (working, _) = pipeline.rescale_working(&image, &backend).unwrap();
        assert_eq!((working.width, working.height), (8, 6));
        let cancelled = pipeline.with_cancellation(Some(Arc::new(AtomicBool::new(true))));
        assert!(
            cancelled
                .process(image, &backend)
                .unwrap_err()
                .contains("cancelled")
        );
    }
    #[test]
    fn fotufilm_reference_survives_amount_and_calibration_edits() {
        let base = build();
        let original = base.original_film.clone();
        let mut params = base.params.clone();
        params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        params.film_render.grain.fotufilm.amount = 0.25;
        let edited = base.with_params(params).unwrap();
        assert!(Arc::ptr_eq(&original, &edited.original_film));
        let mut params = edited.params.clone();
        params.camera.exposure_compensation_ev += 1.0;
        let recalibrated = edited.with_params(params).unwrap();
        assert!(Arc::ptr_eq(&original, &recalibrated.original_film));
    }

    /// A monochrome stock must reach the silver path: the pipeline renders
    /// `kodak_doublex` (five development-time columns, one emulsion value per
    /// sample) with grain on and off, and the grain must cover the frame,
    /// stay bounded in density and be seed-deterministic. This exercises the
    /// single-curve record adaptation and the resolved reference on a real
    /// profile; the CLI cannot reach it, because the loaded monochrome
    /// profiles carry no Hanatos2025 adaptation window and the spectral front
    /// end rejects them on every revision.
    #[test]
    fn fotufilm_grain_reaches_a_monochrome_film() {
        let dir = data_dir();
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let mut image = ImageBuf::new(32, 24);
        for y in 0..24 {
            for x in 0..32 {
                let exposure = if x < 16 { -1.7 } else { -0.6 };
                image.set(x, y, [from_f64(exposure + y as f64 * 0.002); 3]);
            }
        }
        let mut params = RuntimeParams::default();
        params.camera.film_format_mm = 36.0;
        params.film_render.development_time = Some(6.5);
        params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        params.film_render.grain.active = true;
        params.film_render.grain.fotufilm.amount = 1.0;
        let pipeline = Pipeline::new(
            crate::profile::load_profile_by_name(&dir, "kodak_doublex").unwrap(),
            crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
            params.clone(),
        );
        let grained = pipeline
            .process_with_taps(
                image.clone(),
                &backend,
                Some(Tap::LogEFilm),
                Some(Tap::CmyFilm),
            )
            .unwrap();
        let repeat = pipeline
            .process_with_taps(
                image.clone(),
                &backend,
                Some(Tap::LogEFilm),
                Some(Tap::CmyFilm),
            )
            .unwrap();
        assert_eq!(grained.data, repeat.data, "grain readout must be seeded");
        let mut bypass = params;
        bypass.film_render.grain.active = false;
        let plain = Pipeline::new(
            crate::profile::load_profile_by_name(&dir, "kodak_doublex").unwrap(),
            crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
            bypass,
        )
        .process_with_taps(image, &backend, Some(Tap::LogEFilm), Some(Tap::CmyFilm))
        .unwrap();
        assert_eq!((grained.width, grained.height), (32, 24));
        let mut changed = 0_usize;
        let mut worst = 0.0_f64;
        let (grained_pixels, _) = grained.data.as_chunks::<3>();
        let (plain_pixels, _) = plain.data.as_chunks::<3>();
        for (pixel, plain_pixel) in grained_pixels.iter().zip(plain_pixels.iter()) {
            assert!(pixel.iter().all(|value| value.is_finite()));
            let delta = (0..3)
                .map(|channel| (to_f64(pixel[channel]) - to_f64(plain_pixel[channel])).abs())
                .fold(0.0_f64, f64::max);
            worst = worst.max(delta);
            if delta > 1e-6 {
                changed += 1;
            }
        }
        let pixels = grained.data.len() / 3;
        assert!(
            changed * 2 > pixels,
            "grain must reach most pixels, changed {changed} of {pixels}"
        );
        assert!(
            worst < 1.0,
            "silver grain must stay within a density unit, worst {worst}"
        );
    }

    #[test]
    fn injected_film_density_does_not_generate_fotufilm_grain() {
        let mut pipeline = build();
        pipeline.params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        pipeline.params.film_render.grain.active = true;
        pipeline.params.io.scan_film = true;
        pipeline.params.workflow.route = "input > film > scan".to_owned();
        let image = ImageBuf::new(4, 3);
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let active = pipeline
            .process_with_taps(
                image.clone(),
                &backend,
                Some(Tap::CmyFilm),
                Some(Tap::RgbOut),
            )
            .unwrap();
        pipeline.params.film_render.grain.active = false;
        let disabled = pipeline
            .process_with_taps(image, &backend, Some(Tap::CmyFilm), Some(Tap::RgbOut))
            .unwrap();
        assert_eq!(active.data, disabled.data);
    }

    fn lut_data(pipeline: &Pipeline) -> Vec<f64> {
        pipeline
            .tc_lut()
            .expect("spectral pipeline has a LUT")
            .data
            .clone()
    }

    /// Changing any sensitivity-affecting spectral control must rebuild the
    /// TC LUT (and its calibration) instead of reusing the cached one — the
    /// Rust counterpart of the `SpectralLUTService` invalidation keyed on
    /// `Hanatos2025SensitivityAdaptation` + filtered sensitivity.
    #[test]
    fn with_params_rebuilds_lut_on_spectral_change() {
        let base = build();
        let base_lut = lut_data(&base);

        let change = |mutate: fn(&mut RuntimeParams)| {
            let mut params = RuntimeParams::default();
            mutate(&mut params);
            params
        };

        let cases: Vec<(&str, RuntimeParams)> = vec![
            (
                "spectral_gaussian_blur",
                change(|p| p.settings.spectral_gaussian_blur = 8.0),
            ),
            (
                "apply_hanatos2025_adaptation_window",
                change(|p| p.settings.apply_hanatos2025_adaptation_window = false),
            ),
            (
                "apply_hanatos2025_adaptation_surface",
                change(|p| p.settings.apply_hanatos2025_adaptation_surface = true),
            ),
            (
                "camera.filter_uv",
                change(|p| p.camera.filter_uv = [0.5, 415.0, 9.0]),
            ),
            (
                "camera.filter_ir",
                change(|p| p.camera.filter_ir = [0.7, 670.0, 18.0]),
            ),
        ];

        for (name, params) in cases {
            let rebuilt = base.clone().with_params(params).unwrap();
            let lut = lut_data(&rebuilt);
            assert!(
                base_lut.iter().zip(&lut).any(|(a, b)| a != b),
                "{name}: TC LUT was reused after the spectral control changed"
            );
        }
    }

    /// Non-spectral changes keep the calibrated LUT — `with_params` must not
    /// rebuild (and must not corrupt) the front-end for ordinary tweaks.
    #[test]
    fn with_params_reuses_lut_on_non_spectral_change() {
        let base = build();
        let base_lut = lut_data(&base);

        let mut params = RuntimeParams::default();
        params.scanner.lens_blur = 0.7;
        params.film_render.halation.boost_ev = 0.5;
        let tweaked = base.clone().with_params(params).unwrap();
        assert_eq!(
            lut_data(&tweaked),
            base_lut,
            "non-spectral change must not rebuild the TC LUT"
        );
    }
    #[test]
    fn with_params_rejects_failed_calibration_rebuild() {
        let base = build();
        let mut params = base.params.clone();
        params.settings.rgb_to_raw_method = "unsupported-test-method".into();
        let error = match base.with_params(params) {
            Ok(_) => panic!("invalid rebuild must not keep stale calibration"),
            Err(error) => error,
        };
        assert!(error.contains("unsupported method"), "{error}");
    }
    #[test]
    fn with_params_rejects_invalid_print_morph_update() {
        let base = build();
        let mut params = base.params.clone();
        params.print_render.density_curves_morph.active = true;
        params.print_render.density_curves_morph.gamma_factor = 0.0;
        assert!(base.with_params(params).is_err());
    }

    #[test]
    fn stock_edits_survive_construction_updates_and_spectral_rebuild() {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "fujifilm_velvia_100").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let seeded = crate::params_builder::digest_params(
            RuntimeParams::default(),
            &film,
            &print,
            None,
            true,
        );
        let mut edited = seeded;
        edited.film_render.halation.halation_strength = [0.3, 0.2, 0.1];
        edited.film_render.halation.halation_first_sigma_um = [13.0, 17.0, 23.0];
        edited.film_render.dir_couplers.gamma_samelayer_rgb = [0.9, 0.8, 0.7];
        edited.film_render.dir_couplers.gamma_interlayer_r_to_gb = [0.4, 0.3];
        let params = crate::params_builder::digest_params(edited, &film, &print, None, false);
        let mut pipeline = Pipeline::new_with_spectral(film, print, params.clone(), &dir).unwrap();
        assert_eq!(
            pipeline.params.film_render.halation.halation_strength,
            [0.3, 0.2, 0.1]
        );
        assert_eq!(
            pipeline.params.film_render.halation.halation_first_sigma_um,
            [13.0, 17.0, 23.0]
        );
        assert_eq!(
            pipeline.params.film_render.dir_couplers.gamma_samelayer_rgb,
            [0.9, 0.8, 0.7]
        );
        for rebuild in [false, true] {
            let mut updated = params.clone();
            if rebuild {
                updated.settings.spectral_gaussian_blur = 8.0;
            }
            pipeline = pipeline.with_params(updated).unwrap();
            assert_eq!(
                pipeline.params.film_render.halation.halation_strength,
                [0.3, 0.2, 0.1]
            );
            assert_eq!(
                pipeline.params.film_render.halation.halation_first_sigma_um,
                [13.0, 17.0, 23.0]
            );
            assert_eq!(
                pipeline.params.film_render.dir_couplers.gamma_samelayer_rgb,
                [0.9, 0.8, 0.7]
            );
            assert_eq!(
                pipeline
                    .params
                    .film_render
                    .dir_couplers
                    .gamma_interlayer_r_to_gb,
                [0.4, 0.3]
            );
        }
        let mut deactivated = params;
        deactivated.debug.deactivate_spatial_effects = true;
        let deactivated = crate::params_builder::digest_params(
            deactivated,
            &pipeline.film,
            &pipeline.print,
            None,
            false,
        );
        let pipeline = pipeline.with_params(deactivated).unwrap();
        assert_eq!(
            pipeline.params.film_render.halation.halation_first_sigma_um,
            [0.0; 3]
        );
    }

    #[test]
    fn with_params_recalibrates_print_exposure_for_camera_ev() {
        let dir = data_dir();
        let base = build();
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let image = ImageBuf::from_data(8, 8, vec![from_f64(0.184); 8 * 8 * 3]);

        for ev in [-2.0f32, -1.0, 0.0, 1.0, 2.0] {
            let mut params = base.params.clone();
            params.camera.exposure_compensation_ev = ev;
            let updated = base.clone().with_params(params.clone()).unwrap();
            let fresh =
                Pipeline::new_with_spectral(base.film.clone(), base.print.clone(), params, &dir)
                    .unwrap();

            assert!(
                (updated.print_exposure_factor() - fresh.print_exposure_factor()).abs() < 1e-12,
                "EV {ev}: stale print exposure factor"
            );
            let updated_output = updated.process(image.clone(), &backend).unwrap();
            let fresh_output = fresh.process(image.clone(), &backend).unwrap();
            assert_eq!(updated_output.data.len(), fresh_output.data.len());
            for (actual, expected) in updated_output.data.iter().zip(&fresh_output.data) {
                assert!(
                    (*actual as f64 - *expected as f64).abs() < 1e-5,
                    "EV {ev}: output mismatch ({actual} vs {expected})"
                );
            }
        }
    }

    #[test]
    fn input_gamut_active_toggle_matches_fresh_lut() {
        let base = build();
        let mut updated = base.clone();
        for active in [false, true] {
            let mut params = base.params.clone();
            params.io.input_gamut_compress.active = active;
            let rebuilt = updated.with_params(params.clone()).unwrap();
            let fresh = Pipeline::new_with_spectral(
                base.film.clone(),
                base.print.clone(),
                params,
                &data_dir(),
            )
            .unwrap();
            assert_eq!(lut_data(&rebuilt), lut_data(&fresh));
            assert_eq!(
                rebuilt.print_exposure_factor(),
                fresh.print_exposure_factor()
            );
            if !active {
                assert_ne!(lut_data(&rebuilt), lut_data(&base));
            }
            updated = rebuilt;
        }
    }

    #[test]
    fn persistent_taps_and_explicit_precedence_follow_topology() {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.debug.lut_mode = true;
        params.io.crop = true;
        params.io.crop_size = [0.5, 0.5];
        params.taps.inject = Some("log_e_film".into());
        params.taps.collect = Some("cmy_film".into());
        let params = crate::params_builder::digest_params(params, &film, &print, None, true);
        let pipeline = Pipeline::new(film, print, params);
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let image = ImageBuf::from_data(8, 8, vec![from_f64(-1.0); 8 * 8 * 3]);
        let expected = pipeline
            .process_with_taps(
                image.clone(),
                &backend,
                Some(Tap::LogEFilm),
                Some(Tap::CmyFilm),
            )
            .unwrap();
        assert_eq!(
            pipeline.process(image.clone(), &backend).unwrap().data,
            expected.data
        );
        assert_eq!(
            pipeline
                .process_with_taps(image.clone(), &backend, None, None)
                .unwrap()
                .data,
            expected.data
        );
        let unchanged = pipeline
            .process_with_taps(image.clone(), &backend, None, Some(Tap::LogEFilm))
            .unwrap();
        assert_eq!(unchanged.data, image.data);
        let unchanged = pipeline
            .process_with_taps(image.clone(), &backend, Some(Tap::CmyFilm), None)
            .unwrap();
        assert_eq!(unchanged.data, image.data);
        assert_eq!((expected.width, expected.height), (8, 8));
        assert!(
            pipeline
                .process_resident_borrowed(&image, &backend)
                .unwrap()
                .is_none()
        );
        let image = ImageBuf::from_data(8, 8, vec![from_f64(0.184); 8 * 8 * 3]);
        let normal = pipeline
            .process_with_taps(image.clone(), &backend, Some(Tap::RgbIn), Some(Tap::RgbOut))
            .unwrap();
        // `digest_params(..., lut_mode = true)` disables crop so a LUT bake
        // remains a static point transform.
        assert_eq!((normal.width, normal.height), (8, 8));
        let mut params = pipeline.params.clone();
        params.taps = Default::default();
        let default = pipeline
            .with_params(params)
            .unwrap()
            .process(image, &backend)
            .unwrap();
        assert_eq!(normal.data, default.data);
    }
    #[test]
    fn v3_final_crop_preserves_scanner_and_print_boundaries() {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_800").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let mut image = ImageBuf::new(96, 64);
        for y in 0..64 {
            for x in 0..96 {
                let exposure = if x < 48 { -1.7 } else { -0.6 };
                image.set(x, y, [from_f64(exposure + y as f64 * 0.002); 3]);
            }
        }
        for (scan_film, output) in [
            (false, "direct_scan"),
            (true, "direct_scan"),
            (true, "negative_to_positive"),
        ] {
            for scale in [1.0, 1.5] {
                let mut params = RuntimeParams::default();
                params.camera.film_format_mm = 36.0;
                params.film_render.grain.engine = crate::params::GrainEngine::Dye;
                params.film_render.dir_couplers.active = false;
                params.film_render.halation.active = false;
                params.workflow.route = if scan_film {
                    "input > film > scan"
                } else {
                    "input > film > print > scan"
                }
                .into();
                params.io.scan_film = scan_film;
                params.io.upscale_factor = scale;
                params.scanner.scan_output = output.into();
                params.scanner.lens_blur = if scale == 1.0 { 1.5 } else { 3.0 };
                params.scanner.unsharp_mask = [0.7, 0.7];
                params.enlarger.diffusion_filter.active = !scan_film;
                let full_pipeline = Pipeline::new(film.clone(), print.clone(), params.clone());
                let color_ref = full_pipeline.color_reference(&backend);
                // Inject a small developed-exposure field at photographic pitch.
                let render = |pipeline: &Pipeline| {
                    pipeline
                        .run_from(
                            image.clone(),
                            Tap::LogEFilm,
                            Tap::RgbOut,
                            &backend,
                            &color_ref,
                            Some((3.1, 0.0)),
                            None,
                        )
                        .unwrap()
                };
                let full = render(&full_pipeline);
                params.io.crop = true;
                params.io.crop_center = [0.5, 0.5];
                params.io.crop_size = [0.5, 0.5];
                let cropped = render(&Pipeline::new(film.clone(), print.clone(), params));
                let expected = crate::resizing::crop_image(&full, [0.5, 0.5], [0.5, 0.5]);
                let expected = expected.unwrap();
                assert_eq!(
                    (cropped.width, cropped.height),
                    (expected.width, expected.height)
                );
                let worst = cropped
                    .data
                    .iter()
                    .zip(expected.data.iter())
                    .map(|(&a, &b)| (a as f64 - b as f64).abs())
                    .fold(0.0f64, f64::max);
                assert!(
                    worst <= 1e-5,
                    "{scan_film} {output} scale {scale}: crop edge error {worst}"
                );
            }
        }
    }
    #[test]
    fn fotufilm_dispatch_matrix_follows_development_boundary() {
        #[derive(Clone, Copy)]
        struct Case {
            name: &'static str,
            route: &'static str,
            scan_film: bool,
            inject: Tap,
            collect: Tap,
            develops: bool,
        }

        let cases = [
            Case {
                name: "film-scan full",
                route: "input > film > scan",
                scan_film: true,
                inject: Tap::RgbIn,
                collect: Tap::RgbOut,
                develops: true,
            },
            Case {
                name: "film-scan collect film",
                route: "input > film > scan",
                scan_film: true,
                inject: Tap::RgbIn,
                collect: Tap::CmyFilm,
                develops: true,
            },
            Case {
                name: "film-scan developed injection",
                route: "input > film > scan",
                scan_film: true,
                inject: Tap::CmyFilm,
                collect: Tap::RgbOut,
                develops: false,
            },
            Case {
                name: "film-scan film-log injection",
                route: "input > film > scan",
                scan_film: true,
                inject: Tap::LogEFilm,
                collect: Tap::CmyFilm,
                develops: true,
            },
            Case {
                name: "film-scan passthrough",
                route: "input > film > scan",
                scan_film: true,
                inject: Tap::CmyFilm,
                collect: Tap::CmyFilm,
                develops: false,
            },
            Case {
                name: "film-print full",
                route: "input > film > print > scan",
                scan_film: false,
                inject: Tap::RgbIn,
                collect: Tap::RgbOut,
                develops: true,
            },
            Case {
                name: "film-print collect film",
                route: "input > film > print > scan",
                scan_film: false,
                inject: Tap::RgbIn,
                collect: Tap::CmyFilm,
                develops: true,
            },
            Case {
                name: "film-print collect print log",
                route: "input > film > print > scan",
                scan_film: false,
                inject: Tap::RgbIn,
                collect: Tap::LogEPrint,
                develops: true,
            },
            Case {
                name: "film-print collect print density",
                route: "input > film > print > scan",
                scan_film: false,
                inject: Tap::LogEPrint,
                collect: Tap::CmyPrint,
                develops: false,
            },
            Case {
                name: "film-print print-paper segment",
                route: "input > film > print > scan",
                scan_film: false,
                inject: Tap::CmyFilm,
                collect: Tap::CmyPrint,
                develops: false,
            },
            Case {
                name: "film-print later injection",
                route: "input > film > print > scan",
                scan_film: false,
                inject: Tap::CmyPrint,
                collect: Tap::RgbOut,
                develops: false,
            },
            Case {
                name: "passthrough",
                route: "input",
                scan_film: false,
                inject: Tap::RgbIn,
                collect: Tap::RgbIn,
                develops: false,
            },
            Case {
                name: "convert-film print segment",
                route: "input > convert-film > print > scan",
                scan_film: false,
                inject: Tap::CmyFilm,
                collect: Tap::RgbOut,
                develops: false,
            },
            Case {
                name: "convert-film scan segment",
                route: "input > convert-film > scan",
                scan_film: true,
                inject: Tap::CmyFilm,
                collect: Tap::RgbOut,
                develops: false,
            },
        ];

        // The matrix changes only routing controls. Rebuild parameter views
        // from one prepared pipeline so every case exercises its own route
        // without repeating the same Fotufilm population preparation.
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let image = ImageBuf::from_data(
            8,
            6,
            (0..8 * 6 * 3)
                .map(|i| from_f64(-1.6 + (i % 17) as f64 * 0.11))
                .collect(),
        );
        let base = Pipeline::new(film, print, RuntimeParams::default());

        for case in cases {
            let mut params = RuntimeParams::default();
            params.workflow.route = case.route.into();
            params.io.scan_film = case.scan_film;
            params.film_render.grain.active = true;
            params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
            params.film_render.grain.fotufilm.amount = 1.0;
            params.random_seed = 17;
            let active = base.clone().with_params(params.clone()).unwrap();
            params.film_render.grain.active = false;
            let disabled = base.clone().with_params(params).unwrap();

            let active_context = spektrafilm_gpu::telemetry::ObservationContext::new(
                spektrafilm_gpu::telemetry::CollectionMode::Summary,
            );
            let active_backend = spektrafilm_gpu::bind_backend(&backend, active_context.clone());
            let active_image = active
                .process_with_taps(
                    image.clone(),
                    active_backend.as_ref(),
                    Some(case.inject),
                    Some(case.collect),
                )
                .unwrap_or_else(|error| panic!("{}: {error}", case.name));
            let active_grain_stages = active_context
                .snapshot()
                .observations
                .iter()
                .filter(|observation| observation.name == "grain_fotufilm")
                .count();
            assert_eq!(
                active_grain_stages,
                if case.develops { 1 } else { 0 },
                "{}",
                case.name
            );
            let disabled_image = disabled
                .process_with_taps(
                    image.clone(),
                    &backend,
                    Some(case.inject),
                    Some(case.collect),
                )
                .unwrap_or_else(|error| panic!("{} disabled: {error}", case.name));
            if case.develops {
                assert_ne!(active_image.data, disabled_image.data, "{}", case.name);
            } else {
                assert_eq!(active_image.data, disabled_image.data, "{}", case.name);
            }
        }
    }

    #[test]
    fn fotufilm_dispatch_with_crop_and_optics_runs_once() {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > print > scan".into();
        params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        params.film_render.grain.active = true;
        params.io.crop = true;
        params.io.crop_size = [0.5, 0.5];
        params.enlarger.lens_blur = 1.25;
        params.scanner.lens_blur = 1.0;
        params.random_seed = 41;
        let pipeline = Pipeline::new(film.clone(), print.clone(), params.clone());
        params.film_render.grain.active = false;
        let disabled = Pipeline::new(film, print, params);
        let image = ImageBuf::from_data(12, 8, vec![from_f64(-0.8); 12 * 8 * 3]);
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let mut timings = std::collections::BTreeMap::new();
        let active = pipeline
            .process_with_taps_timed(
                image.clone(),
                &backend,
                Some(Tap::RgbIn),
                Some(Tap::RgbOut),
                Some(&mut timings),
            )
            .unwrap();
        assert!(timings.contains_key("grain_fotufilm_readout"));
        let quiet = disabled
            .process_with_taps(image, &backend, Some(Tap::RgbIn), Some(Tap::RgbOut))
            .unwrap();
        assert_ne!(active.data, quiet.data);
        assert!(active.width < 12 && active.height < 8);
    }

    #[test]
    fn fotufilm_lut_mode_matches_grain_disabled_and_prepares_no_bank() {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.debug.lut_mode = true;
        params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        params.film_render.grain.active = true;
        params.film_render.grain.fotufilm.amount = 1.0;
        let digested =
            crate::params_builder::digest_params(params.clone(), &film, &print, None, true);
        assert!(!stages::grain_fotufilm::active(&digested));
        let active = Pipeline::new(film.clone(), print.clone(), digested);
        params.film_render.grain.active = false;
        let disabled_params =
            crate::params_builder::digest_params(params, &film, &print, None, true);
        let disabled = Pipeline::new(film, print, disabled_params);
        let image = ImageBuf::from_data(4, 4, vec![from_f64(0.18); 4 * 4 * 3]);
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        assert_eq!(
            active.process(image.clone(), &backend).unwrap().data,
            disabled.process(image, &backend).unwrap().data
        );
    }

    #[test]
    fn fotufilm_seed_and_amount_determinism() {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        // Seed and amount are readout controls; keep the prepared pipeline
        // shared while each parameter view remains independently rebuilt.
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        params.film_render.grain.active = true;
        params.film_render.grain.fotufilm.amount = 1.0;
        params.random_seed = 91;
        let image = ImageBuf::from_data(
            8,
            6,
            (0..8 * 6 * 3)
                .map(|i| from_f64(-1.1 + (i % 13) as f64 * 0.15))
                .collect(),
        );
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let base = Pipeline::new(film, print, params.clone());
        let render = |p: Pipeline| p.process(image.clone(), &backend).unwrap().data;
        let first = render(base.clone());
        let repeat = render(base.clone());
        assert_eq!(first, repeat);
        params.random_seed += 1;
        let changed = render(base.clone().with_params(params.clone()).unwrap());
        assert_ne!(first, changed);
        params.random_seed = 91;
        params.film_render.grain.fotufilm.amount = 0.65;
        let amount_first = render(base.clone().with_params(params.clone()).unwrap());
        let amount_repeat = render(base.with_params(params).unwrap());
        assert_eq!(amount_first, amount_repeat);
    }
}
