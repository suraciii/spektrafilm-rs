//! Fotufilm Film grain: adapted-appearance readout of the fitted crystal
//! population.
//!
//! `spektrafilm_model::grain::fotufilm` owns exposure-class fitting, bank
//! formation, aperture matching and residual sampling. This stage owns the
//! resolved-profile adaptation (curve convention, material law, authored
//! effective metadata), the immutable prepared-bank lifetime, the
//! conservative resampling of the developed film density onto the requested
//! output lattice and the CPU observation. `specs/film-grain/fotufilm/`
//! owns the product and design contract.

use crate::params::{FotufilmMaterial, GrainEngine, RuntimeParams};
use crate::profile::Profile;
use rayon::prelude::*;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};
use spektrafilm_model::grain::fotufilm as model;
use spektrafilm_model::grain::v3::ReadoutGrid;
use std::sync::{Arc, Mutex};

/// Upstream Fotufilm revision this engine adapts. The model's `PROVENANCE`
/// owns the full port statement; a unit test keeps the two in sync.
pub const SOURCE_REVISION: &str = "59d43471dbef45c85ae442e507a291fdebfb2ec5";
/// Implementation identity reported by [`configuration_report`].
const IMPLEMENTATION: &str = "rust_cpu_film";
/// The engine reports authored effective appearance, never stock calibration.
const APPEARANCE_STATUS: &str = "authored_effective_not_stock_calibration";
/// Readout method identity: complete periodic tile integration.
const READOUT_METHOD: &str = "complete_periodic_tile";
/// Execution note reported when the engine is active.
const EXECUTION_NOTE: &str =
    "Fotufilm Film prepares its crystal population and samples the grain residual on the CPU.";
/// Precision of the samples this stage reads and writes. The population and
/// the residual arithmetic are `f64`; the bank's tile light field is `f32`.
const EXECUTION_PRECISION: &str = if std::mem::size_of::<spektrafilm_math::precision::Scalar>() == 8
{
    "f64"
} else {
    "f32"
};

/// Whether an active Fotufilm readout must run.
///
/// Preview mode, LUT mode and debug stochastic deactivation already clear
/// `grain.active` before the pipeline is built (`params_builder`), so this
/// predicate is the exact bypass boundary: `amount = 0` prepares no bank and
/// changes no pixel.
pub fn active(params: &RuntimeParams) -> bool {
    super::grain_engine_active(params, GrainEngine::Fotufilm)
        && !params.debug.deactivate_stochastic_effects
        && params.film_render.grain.fotufilm.amount > 0.0
}

/// Fail before any work when Fotufilm is selected outside its supported
/// condition: a filming film profile with resolvable, finite density curves
/// and a material law the profile type supports.
pub fn validate_supported(film: &Profile, params: &RuntimeParams) -> Result<(), String> {
    if !active(params) {
        return Ok(());
    }
    let stock = film.info.stock.as_deref().unwrap_or("<unnamed>");
    if !(film.is_film() && film.is_filming()) {
        return Err(format!(
            "film grain Fotufilm: film profile '{stock}' is not a filming film \
             (support {:?}, stage {:?}), so the film-development boundary never runs it",
            film.info.support, film.info.stage
        ));
    }
    material_law(film, params.film_render.grain.fotufilm.material, stock)?;
    build_curves(film, 1.0, stock)?;
    build_curves(film, params.film_render.density_curve_gamma as f64, stock)?;
    Ok(())
}

/// Resolved material law: `(silver, reversal)`.
///
/// Auto selects dye reversal for a positive film, silver for a negative
/// monochrome film and dye negative for a negative colour film. An explicit
/// dye law is allowed on a monochrome profile (chromogenic monochrome); an
/// explicit silver law is not allowed on a colour profile.
fn material_law(
    film: &Profile,
    material: FotufilmMaterial,
    stock: &str,
) -> Result<(bool, bool), String> {
    match material {
        FotufilmMaterial::Auto => Ok(if film.is_bw() {
            (true, false)
        } else if film.is_positive() {
            (false, true)
        } else {
            (false, false)
        }),
        FotufilmMaterial::Silver if film.is_bw() => Ok((true, false)),
        FotufilmMaterial::Silver => Err(format!(
            "film grain Fotufilm: material 'silver' requires a monochrome film \
             profile, but '{stock}' has channel model {:?}; select 'auto', \
             'dye_negative' or 'dye_reversal'",
            film.info.channel_model
        )),
        FotufilmMaterial::DyeNegative => Ok((false, false)),
        FotufilmMaterial::DyeReversal => Ok((false, true)),
    }
}

/// Build the model's per-record curves from a resolved film profile.
///
/// The developed density image is produced from minimum-normalized curves
/// (`stages::filming`), so the model receives the same convention: each
/// record is normalized by its own curve minimum and its grain-local D-min
/// is zero. Positive profiles store falling curves while the model fits
/// rising ones, so their exposure axis is mirrored. `gamma` selects the
/// exposure sampling the developed image used (`log_exposure / gamma`).
/// Real measured curves carry small non-monotonicities; the model's density
/// inversion requires a monotone axis, so each record is projected onto its
/// running maximum (a no-op on already monotone data).
///
/// A monochrome profile carries one emulsion value per sample (the
/// development-time family is collapsed by `resolve_for_render` and the
/// channel axis is degenerate), so every record receives that single
/// curve. The model's monochrome path reads record 1 for levels and
/// residual and mirrors its anchor measurements into records 0 and 2;
/// zero curves in the unused records would fail validation and fit
/// nothing.
fn build_curves(film: &Profile, gamma: f64, stock: &str) -> Result<[model::Curve; 3], String> {
    let samples = film.density_curves_f64();
    let monochrome = film.is_bw();
    let mut exposure = film.log_exposure_f64();
    if samples.len() < 2 || exposure.len() != samples.len() {
        return Err(format!(
            "film grain Fotufilm: film profile '{stock}' has {} density-curve \
             samples over {} log-exposure knots; the model needs matching, \
             non-degenerate curves",
            samples.len(),
            exposure.len()
        ));
    }
    if !gamma.is_finite() || gamma <= 0.0 {
        return Err(format!(
            "film grain Fotufilm: film_render.density_curve_gamma {gamma} must be \
             finite and positive"
        ));
    }
    let mut per_record: [Vec<f64>; 3] = std::array::from_fn(|_| Vec::with_capacity(samples.len()));
    let authored_records = if monochrome { 1 } else { 3 };
    // `density_curves_f64` pads a short row with zeros, which would hide a
    // malformed profile, so the record count is checked on the profile data.
    for (index, row) in film.data.density_curves.iter().enumerate() {
        if row.len() < authored_records {
            return Err(format!(
                "film grain Fotufilm: film profile '{stock}' row {index} has {} density-curve records; expected at least {authored_records}",
                row.len()
            ));
        }
    }
    for row in &samples {
        for (record, values) in per_record.iter_mut().take(authored_records).enumerate() {
            if !row[record].is_finite() {
                return Err(format!(
                    "film grain Fotufilm: film profile '{stock}' has a non-finite density-curve sample in record {record}"
                ));
            }
            values.push(row[record]);
        }
    }
    if monochrome {
        per_record[1] = per_record[0].clone();
        per_record[2] = per_record[0].clone();
    }
    if film.is_positive() {
        exposure.reverse();
        for values in &mut per_record {
            values.reverse();
        }
        for value in &mut exposure {
            *value = -*value;
        }
    }
    if (gamma - 1.0).abs() > 1e-12 {
        for value in &mut exposure {
            *value /= gamma;
        }
    }
    if exposure.windows(2).any(|pair| pair[1] <= pair[0]) {
        return Err(format!(
            "film grain Fotufilm: film profile '{stock}' has a non-ascending \
             log-exposure axis; the model's curve inversion is undefined"
        ));
    }
    let curves = std::array::from_fn(|record| {
        let values = &mut per_record[record];
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let mut running = -f64::INFINITY;
        for value in values.iter_mut() {
            *value -= min;
            running = running.max(*value);
            *value = running;
        }
        model::Curve {
            log_exposure: exposure.clone(),
            density: values.clone(),
            d_min: 0.0,
            d_max: running,
        }
    });
    for (record, curve) in curves.iter().enumerate() {
        curve.validate().map_err(|error| {
            format!("film grain Fotufilm: film profile '{stock}' record {record}: {error}")
        })?;
    }
    Ok(curves)
}

/// Authored effective population metadata and sampling controls.
fn metadata_and_look(
    film: &Profile,
    params: &RuntimeParams,
) -> Result<(model::Metadata, model::Look), String> {
    let grain = &params.film_render.grain.fotufilm;
    let stock = film.info.stock.as_deref().unwrap_or("<unnamed>");
    let (silver, reversal) = material_law(film, grain.material, stock)?;
    let metadata = model::Metadata {
        sigma48: std::array::from_fn(|record| grain.rms_density * grain.layer_weights[record]),
        anchor_net_density: [1.0; 3],
        cloud_radius_mm: grain.clump_radius_um / 1000.0,
        layer_size_ratio: grain.layer_size_ratio,
        radius_span: grain.radius_span,
        sublayer_shares: grain.sublayer_shares,
        coating_density_scale: grain.coating_density_scale,
        fog_density: grain.fog_density,
        reversal,
        silver,
        monochrome: film.is_bw(),
    };
    metadata
        .validate()
        .map_err(|error| format!("film grain Fotufilm: {error}"))?;
    let look = model::Look {
        amount: grain.amount,
        size: grain.size,
        color: grain.color,
        softness: grain.softness,
        layers: grain.layers,
    };
    Ok((metadata, look))
}

/// Population/bank identity: every numerical input except Amount, Size,
/// Softness, Color, Layers, seed and raster size.
#[derive(Clone, PartialEq)]
struct Identity {
    reference: [model::Curve; 3],
    current: [model::Curve; 3],
    metadata: model::Metadata,
}

/// Immutable prepared-bank lifetime shared by pipeline clones.
///
/// Preparation is lazy and identity-keyed: an Amount-only edit reuses the
/// completed bank, while a population/metadata/curve change prepares a new
/// one. A canceled preparation is never published.
#[derive(Clone)]
pub struct Prepared {
    inner: Arc<Inner>,
}

struct Inner {
    /// Loaded film profile before calibration mutates it.
    reference: Profile,
    cache: Mutex<Option<(Identity, Arc<model::Bank>)>>,
}

impl Prepared {
    /// Capture the reference film profile before calibration mutates it.
    pub fn new(film: &Profile, _development_time: Option<f64>) -> Self {
        Self {
            inner: Arc::new(Inner {
                reference: film.clone(),
                cache: Mutex::new(None),
            }),
        }
    }

    /// Read the requested film region out of the full developed source.
    ///
    /// `source` is the full-source developed CMY density at `source_pitch_um`.
    /// The model runs on the CPU, so the observation records a CPU executor
    /// with its reason whatever backend the surrounding stages use.
    #[allow(clippy::too_many_arguments)]
    pub fn readout(
        &self,
        source: &ImageBuf,
        film: &Profile,
        params: &RuntimeParams,
        source_pitch_um: f64,
        grid: ReadoutGrid,
        backend: &dyn spektrafilm_gpu::ComputeBackend,
        cancel: &(dyn Fn() -> Result<(), String> + Sync),
    ) -> Result<ImageBuf, String> {
        let mut observation = super::StageObservation::cpu(
            backend,
            "grain_fotufilm",
            spektrafilm_gpu::telemetry::CpuReason::FotufilmFilmCpu,
        );
        let result = self.readout_inner(source, film, params, source_pitch_um, grid, cancel);
        observation.set_complete(result.is_ok());
        result
    }

    fn readout_inner(
        &self,
        source: &ImageBuf,
        film: &Profile,
        params: &RuntimeParams,
        source_pitch_um: f64,
        grid: ReadoutGrid,
        cancel: &(dyn Fn() -> Result<(), String> + Sync),
    ) -> Result<ImageBuf, String> {
        cancel()?;
        let stock = film.info.stock.as_deref().unwrap_or("<unnamed>");
        let resolved_reference = crate::profile::resolve_for_render(
            self.inner.reference.clone(),
            params.film_render.development_time,
        );
        let reference = build_curves(&resolved_reference, 1.0, stock)?;
        let current = build_curves(film, params.film_render.density_curve_gamma as f64, stock)?;
        let (metadata, look) = metadata_and_look(film, params)?;
        let identity = Identity {
            reference,
            current,
            metadata,
        };
        let bank = self.bank(&identity, cancel)?;
        let input = area_resample(source, source_pitch_um, &grid)?;
        let pitch_mm = [grid.pitch_um[0] / 1000.0, grid.pitch_um[1] / 1000.0];
        let geometry = model::Geometry {
            source_pitch_mm: [source_pitch_um / 1000.0; 2],
            origin_mm: [grid.origin_um[0] / 1000.0, grid.origin_um[1] / 1000.0],
            output_pitch_mm: pitch_mm,
            // One output sample integrates its own film cell; Size and
            // Softness rescale that aperture inside the model.
            footprint_mm: pitch_mm,
        };
        tracing::info!(
            stock,
            source = format_args!("{}x{}", source.width, source.height),
            output = format_args!("{}x{}", grid.width, grid.height),
            origin_um = ?grid.origin_um,
            output_pitch_um = ?grid.pitch_um,
            amount = look.amount,
            size = look.size,
            softness = look.softness,
            "film grain Fotufilm: readout on CPU"
        );
        bank.transform(&input, geometry, look, params.random_seed, cancel)
    }

    /// The immutable bank for `identity`, preparing it on first use.
    fn bank(
        &self,
        identity: &Identity,
        cancel: &(dyn Fn() -> Result<(), String> + Sync),
    ) -> Result<Arc<model::Bank>, String> {
        if let Ok(cache) = self.inner.cache.lock()
            && let Some((cached, bank)) = cache.as_ref()
            && cached == identity
        {
            return Ok(bank.clone());
        }
        cancel()?;
        let population = model::Population::prepare(
            model::PopulationInput {
                curves: identity.current.clone(),
                reference_curves: Some(identity.reference.clone()),
                metadata: identity.metadata.clone(),
                reference_metadata: Some(identity.metadata.clone()),
            },
            cancel,
        )
        .map_err(|error| format!("film grain Fotufilm: {error}"))?;
        let diagnostics = population.diagnostics();
        tracing::info!(
            provenance = diagnostics.provenance,
            fit_error = ?diagnostics.records.each_ref().map(|record| record.fit_error),
            developability = ?diagnostics.records.each_ref().map(|record| record.developability),
            anchor_target = ?diagnostics.anchor_target,
            anchor_measured = ?diagnostics.anchor_measured,
            anchor_log_error = ?diagnostics.anchor_log_error,
            anchor_passes = ?diagnostics.anchor_passes,
            "film grain Fotufilm: population prepared"
        );
        let bank = population
            .build_bank(cancel)
            .map_err(|error| format!("film grain Fotufilm: {error}"))?;
        if let Ok(mut cache) = self.inner.cache.lock() {
            *cache = Some((identity.clone(), bank.clone()));
        }
        Ok(bank)
    }
    #[cfg(test)]
    pub(crate) fn bank_identity(&self) -> Option<usize> {
        self.inner
            .cache
            .lock()
            .ok()
            .and_then(|cache| cache.as_ref().map(|(_, bank)| Arc::as_ptr(bank) as usize))
    }
}

/// One output sample's coverage of the source lattice along one axis.
struct Span {
    /// `(source index, weight)` with weights summing to one. The outermost
    /// source cells extend to infinity, so a lattice that reaches beyond the
    /// developed source reproduces its edge density.
    weights: Vec<(usize, f64)>,
}

fn axis_spans(
    origin_um: f64,
    pitch_um: f64,
    out_len: usize,
    source_pitch_um: f64,
    source_len: usize,
) -> Result<Vec<Span>, String> {
    let mut spans = Vec::with_capacity(out_len);
    let last = source_len as i64 - 1;
    for sample in 0..out_len {
        let lo = origin_um + sample as f64 * pitch_um;
        let hi = lo + pitch_um;
        let mut weights: Vec<(usize, f64)> = Vec::new();
        let first = (lo / source_pitch_um).floor() as i64 - 1;
        let end = (hi / source_pitch_um).ceil() as i64 + 1;
        for index in first..=end {
            let clamped = index.clamp(0, last) as usize;
            let cell_lo = if index <= 0 {
                f64::NEG_INFINITY
            } else {
                index as f64 * source_pitch_um
            };
            let cell_hi = if index >= last {
                f64::INFINITY
            } else {
                (index as f64 + 1.0) * source_pitch_um
            };
            let overlap = hi.min(cell_hi) - lo.max(cell_lo);
            if overlap <= 0.0 {
                continue;
            }
            if let Some(entry) = weights.last_mut()
                && entry.0 == clamped
            {
                entry.1 += overlap;
            } else {
                weights.push((clamped, overlap));
            }
        }
        let total = weights.iter().map(|entry| entry.1).sum::<f64>();
        if weights.is_empty() || total <= 0.0 {
            return Err(
                "film grain Fotufilm: requested lattice has no coverage of the \
                 developed source"
                    .to_owned(),
            );
        }
        for entry in &mut weights {
            entry.1 /= total;
        }
        spans.push(Span { weights });
    }
    Ok(spans)
}

/// Conservatively area-resample the developed source density onto the
/// requested output lattice.
///
/// The reconstruction is piecewise constant per source pixel: each output
/// sample averages the source cells its film-space cell overlaps. Upsampling
/// therefore repeats the covering source value; the model's grain is added
/// after this resampling, never resampled itself.
fn area_resample(
    source: &ImageBuf,
    source_pitch_um: f64,
    grid: &ReadoutGrid,
) -> Result<ImageBuf, String> {
    if !source_pitch_um.is_finite() || source_pitch_um <= 0.0 {
        return Err(format!(
            "film grain Fotufilm: source pitch {source_pitch_um} must be finite and positive"
        ));
    }
    if source.width == 0 || source.height == 0 || grid.width == 0 || grid.height == 0 {
        return Err("film grain Fotufilm: empty source or output lattice".to_owned());
    }
    if grid
        .pitch_um
        .iter()
        .any(|pitch| !pitch.is_finite() || *pitch <= 0.0)
    {
        return Err(format!(
            "film grain Fotufilm: output pitch {:?} must be finite and positive",
            grid.pitch_um
        ));
    }
    let output_bytes = (grid.width as u64)
        .checked_mul(grid.height as u64)
        .and_then(|pixels| pixels.checked_mul(3))
        .and_then(|samples| {
            samples.checked_mul(std::mem::size_of::<spektrafilm_math::precision::Scalar>() as u64)
        });
    if output_bytes.is_none_or(|bytes| bytes > isize::MAX as u64) {
        return Err("film grain Fotufilm: output storage exceeds the addressable allocation size; reduce the output dimensions".to_owned());
    }
    let x_spans = axis_spans(
        grid.origin_um[0],
        grid.pitch_um[0],
        grid.width as usize,
        source_pitch_um,
        source.width as usize,
    )?;
    let y_spans = axis_spans(
        grid.origin_um[1],
        grid.pitch_um[1],
        grid.height as usize,
        source_pitch_um,
        source.height as usize,
    )?;
    let source_width = source.width as usize;
    let output_width = grid.width as usize;
    let mut output = ImageBuf::new(grid.width, grid.height);
    output
        .data
        .par_chunks_mut(output_width * 3)
        .enumerate()
        .for_each(|(row, destination)| {
            for (column, x_span) in x_spans.iter().enumerate() {
                let mut accumulated = [0.0_f64; 3];
                for (source_row, y_weight) in &y_spans[row].weights {
                    let base = source_row * source_width * 3;
                    for (source_column, x_weight) in &x_span.weights {
                        let index = base + source_column * 3;
                        let weight = x_weight * y_weight;
                        for (channel, total) in accumulated.iter_mut().enumerate() {
                            *total += weight * to_f64(source.data[index + channel]);
                        }
                    }
                }
                for channel in 0..3 {
                    destination[column * 3 + channel] = from_f64(accumulated[channel]);
                }
            }
        });
    Ok(output)
}

/// Dry-run and report provenance of the engine, its readout method and its
/// runtime activation.
pub fn configuration_report(params: &RuntimeParams) -> serde_json::Value {
    let active = active(params);
    serde_json::json!({
        "active": active,
        "engine": "fotufilm",
        "implementation": IMPLEMENTATION,
        "effective_backend": if active { Some("cpu") } else { None },
        "note": if active { Some(EXECUTION_NOTE) } else { None },
        "appearance_status": APPEARANCE_STATUS,
        "execution_precision": EXECUTION_PRECISION,
        "readout": READOUT_METHOD,
        // A dry run reports and prepares nothing; a render reports its own
        // prepared bank through the observation record and diagnostics log.
        "prepared": false,
        "source_revision": SOURCE_REVISION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{GrainEngine, RuntimeParams};

    fn profile(json: &str) -> Profile {
        serde_json::from_str(json).expect("profile json")
    }

    /// Four samples over three records; `values` are per record. The profile
    /// JSON stores one row per exposure sample: one value per record for
    /// colour stocks, the single emulsion value for monochrome stocks.
    fn film_profile(kind: &str, channel_model: &str, records: [[f64; 4]; 3]) -> Profile {
        let monochrome = channel_model == "bw";
        let samples = (0..4)
            .map(|sample| {
                if monochrome {
                    format!("[{}]", records[0][sample])
                } else {
                    format!(
                        "[{},{},{}]",
                        records[0][sample], records[1][sample], records[2][sample]
                    )
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        profile(&format!(
            r#"{{"metadata":{{}},
                "info":{{"stock":"test","type":"{kind}","channel_model":"{channel_model}"}},
                "data":{{
                    "log_exposure":[-2.0,-0.5,1.0,2.5],
                    "density_curves":[{samples}]
                }}}}"#
        ))
    }

    fn negative_colour() -> Profile {
        film_profile(
            "negative",
            "color",
            [
                [0.2, 0.9, 2.1, 2.4],
                [0.3, 1.0, 2.2, 2.5],
                [0.4, 1.1, 2.3, 2.6],
            ],
        )
    }

    fn positive_colour() -> Profile {
        film_profile(
            "positive",
            "color",
            [
                [2.4, 2.0, 0.7, 0.2],
                [2.5, 2.1, 0.8, 0.25],
                [2.6, 2.2, 0.9, 0.3],
            ],
        )
    }

    fn monochrome_negative() -> Profile {
        film_profile(
            "negative",
            "bw",
            [
                [0.3, 1.0, 2.2, 2.5],
                [0.3, 1.0, 2.2, 2.5],
                [0.3, 1.0, 2.2, 2.5],
            ],
        )
    }

    /// A silver reversal (positive) monochrome stock: one falling curve.
    fn monochrome_positive() -> Profile {
        film_profile("positive", "bw", [[2.6, 2.1, 0.6, 0.2]; 3])
    }

    fn fotufilm_params(material: FotufilmMaterial) -> RuntimeParams {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Fotufilm;
        params.film_render.grain.fotufilm.material = material;
        params
    }
    #[test]
    fn debug_stochastic_deactivation_makes_fotufilm_inactive() {
        let mut params = fotufilm_params(FotufilmMaterial::Auto);
        params.debug.deactivate_stochastic_effects = true;
        assert!(!active(&params));
    }

    #[test]
    fn short_density_curve_rows_return_actionable_error() {
        let mut film = negative_colour();
        film.data.density_curves[0].truncate(2);
        let error = build_curves(&film, 1.0, "test").unwrap_err();
        assert!(error.contains("density-curve records"), "{error}");
        assert!(error.contains("expected at least 3"), "{error}");
    }

    #[test]
    fn source_revision_matches_model_provenance() {
        assert!(
            model::PROVENANCE.contains(SOURCE_REVISION),
            "{}",
            model::PROVENANCE
        );
    }

    #[test]
    fn zero_amount_and_inactive_grain_are_exact_bypasses() {
        let mut params = fotufilm_params(FotufilmMaterial::Auto);
        assert!(active(&params));
        params.film_render.grain.fotufilm.amount = 0.0;
        assert!(!active(&params));
        assert!(validate_supported(&negative_colour(), &params).is_ok());
        params.film_render.grain.fotufilm.amount = 1.0;
        params.film_render.grain.active = false;
        assert!(!active(&params));
    }

    #[test]
    fn curves_are_normalized_monotone_and_min_zeroed() {
        let curves = build_curves(&negative_colour(), 1.0, "test").expect("curves");
        for curve in &curves {
            assert_eq!(curve.d_min, 0.0);
            assert!(
                curve.density.windows(2).all(|pair| pair[1] >= pair[0]),
                "density must be non-decreasing"
            );
            assert!(
                curve.log_exposure.windows(2).all(|pair| pair[1] > pair[0]),
                "exposure must ascend"
            );
            assert!(curve.d_max > 0.0);
        }
        assert_eq!(curves[0].density[0], 0.0);
    }

    #[test]
    fn positive_films_mirror_into_rising_model_curves() {
        let curves = build_curves(&positive_colour(), 1.0, "test").expect("curves");
        let curve = &curves[0];
        assert!(curve.density.windows(2).all(|pair| pair[1] >= pair[0]));
        assert_eq!(curve.log_exposure[0], -2.5);
        assert_eq!(curve.log_exposure[3], 2.0);
        assert!(curve.density[0] <= curve.density[3]);
    }

    #[test]
    fn measured_dips_project_onto_the_running_maximum() {
        let film = film_profile(
            "negative",
            "color",
            [
                [0.2, 0.8, 0.75, 2.4],
                [0.3, 0.9, 0.85, 2.5],
                [0.4, 1.0, 0.95, 2.6],
            ],
        );
        let curves = build_curves(&film, 1.0, "test").expect("curves");
        assert!(curves[0].density.windows(2).all(|pair| pair[1] >= pair[0]));
        assert_eq!(curves[0].density[1], curves[0].density[2]);
    }

    #[test]
    fn monochrome_records_share_the_single_emulsion_curve() {
        let curves = build_curves(&monochrome_negative(), 1.0, "test").expect("curves");
        assert_eq!(curves[0].density, curves[1].density);
        assert_eq!(curves[1].density, curves[2].density);
        assert!(curves[1].d_max > 0.0);
        let silver_positive = build_curves(&monochrome_positive(), 1.0, "test").expect("curves");
        assert!(
            silver_positive[1]
                .density
                .windows(2)
                .all(|pair| pair[1] >= pair[0])
        );
        assert!(silver_positive[1].d_max > 0.0);
    }

    #[test]
    fn gamma_edits_resample_the_exposure_axis() {
        let wide = build_curves(&negative_colour(), 1.0, "test").expect("curves");
        let edited = build_curves(&negative_colour(), 2.0, "test").expect("curves");
        assert_eq!(edited[0].log_exposure[1], wide[0].log_exposure[1] / 2.0);
    }

    #[test]
    fn material_auto_and_explicit_laws_follow_the_profile_type() {
        let auto = FotufilmMaterial::Auto;
        assert_eq!(
            material_law(&negative_colour(), auto, "test").unwrap(),
            (false, false)
        );
        assert_eq!(
            material_law(&positive_colour(), auto, "test").unwrap(),
            (false, true)
        );
        assert_eq!(
            material_law(&monochrome_negative(), auto, "test").unwrap(),
            (true, false)
        );
        assert_eq!(
            material_law(&monochrome_positive(), auto, "test").unwrap(),
            (true, false),
            "a monochrome positive still develops silver"
        );
        assert_eq!(
            material_law(
                &monochrome_negative(),
                FotufilmMaterial::DyeNegative,
                "test"
            )
            .unwrap(),
            (false, false)
        );
        assert!(material_law(&negative_colour(), FotufilmMaterial::Silver, "test").is_err());
    }

    #[test]
    fn unsupported_stock_is_rejected_before_any_work() {
        let params = fotufilm_params(FotufilmMaterial::Auto);
        let mut paper = negative_colour();
        paper.info.support = "paper".into();
        paper.info.stage = "printing".into();
        let error = validate_supported(&paper, &params).unwrap_err();
        assert!(error.contains("not a filming film"), "{error}");
    }

    #[test]
    fn invalid_curves_are_rejected_with_the_stock_name() {
        let params = fotufilm_params(FotufilmMaterial::Auto);
        let mut film = negative_colour();
        film.data.density_curves.truncate(1);
        let error = validate_supported(&film, &params).unwrap_err();
        assert!(error.contains("test"), "{error}");
        assert!(error.contains("density-curve samples"), "{error}");
    }

    #[test]
    fn area_resample_is_identity_when_the_lattice_matches_the_source() {
        let mut source = ImageBuf::new(2, 2);
        source.set(0, 0, [from_f64(0.1), from_f64(0.2), from_f64(0.3)]);
        source.set(1, 0, [from_f64(0.4), from_f64(0.5), from_f64(0.6)]);
        source.set(0, 1, [from_f64(0.7), from_f64(0.8), from_f64(0.9)]);
        source.set(1, 1, [from_f64(1.0), from_f64(1.1), from_f64(1.2)]);
        let grid = ReadoutGrid {
            origin_um: [0.0, 0.0],
            pitch_um: [10.0, 10.0],
            width: 2,
            height: 2,
        };
        let out = area_resample(&source, 10.0, &grid).expect("resample");
        for (index, expected) in [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.1, 1.2]
            .iter()
            .enumerate()
        {
            // The native-precision build stores samples as f32.
            assert!(
                (to_f64(out.data[index]) - expected).abs() < 1e-6,
                "index {index}: {}",
                to_f64(out.data[index])
            );
        }
    }

    #[test]
    fn area_resample_averages_the_covered_source_cells() {
        let mut source = ImageBuf::new(4, 1);
        for (column, value) in [0.0, 1.0, 2.0, 3.0].iter().enumerate() {
            source.set(column as u32, 0, [from_f64(*value); 3]);
        }
        let grid = ReadoutGrid {
            origin_um: [0.0, 0.0],
            pitch_um: [40.0, 40.0],
            width: 1,
            height: 1,
        };
        let out = area_resample(&source, 10.0, &grid).expect("resample");
        assert!((to_f64(out.data[0]) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn area_resample_reproduces_the_edge_density_outside_the_source() {
        let mut source = ImageBuf::new(2, 1);
        source.set(0, 0, [from_f64(0.25); 3]);
        source.set(1, 0, [from_f64(0.75); 3]);
        let grid = ReadoutGrid {
            origin_um: [-10.0, 0.0],
            pitch_um: [10.0, 10.0],
            width: 4,
            height: 1,
        };
        let out = area_resample(&source, 10.0, &grid).expect("resample");
        // Columns cover [-10, 0], [0, 10], [10, 20] and [20, 30]; the last two
        // read the extended edge cell of the second source pixel.
        let expected = [0.25, 0.25, 0.75, 0.75];
        for (column, value) in expected.iter().enumerate() {
            assert!(
                (to_f64(out.data[column * 3]) - value).abs() < 1e-6,
                "column {column}"
            );
        }
    }

    #[test]
    fn configuration_report_identifies_the_cpu_port() {
        let params = fotufilm_params(FotufilmMaterial::Auto);
        let report = configuration_report(&params);
        assert_eq!(report["active"], true);
        assert_eq!(report["engine"], "fotufilm");
        assert_eq!(report["implementation"], IMPLEMENTATION);
        assert_eq!(report["appearance_status"], APPEARANCE_STATUS);
        assert_eq!(report["effective_backend"], "cpu");
        assert_eq!(report["execution_precision"], EXECUTION_PRECISION);
        assert!(report["note"].as_str().unwrap().contains("CPU"));
        assert_eq!(report["readout"], READOUT_METHOD);
        assert_eq!(report["prepared"], false);
        assert_eq!(report["source_revision"], SOURCE_REVISION);

        let mut inactive = fotufilm_params(FotufilmMaterial::Auto);
        inactive.film_render.grain.fotufilm.amount = 0.0;
        let report = configuration_report(&inactive);
        assert_eq!(report["active"], false);
        assert_eq!(report["effective_backend"], serde_json::Value::Null);
        assert_eq!(report["note"], serde_json::Value::Null);
        // Provenance stays reportable while the engine is inactive.
        assert_eq!(report["source_revision"], SOURCE_REVISION);
    }
}
