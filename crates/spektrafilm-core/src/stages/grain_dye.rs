//! Film-coordinate grain Dye integration (`specs/film-grain/dye/`).
//!
//! The pipeline develops the *full* source and hands the developed CMY
//! density to this module. The module owns Dye field resolution and the
//! readout to the requested output geometry — on the device through
//! `dye_field` when the backend has one and the request is
//! device-representable, otherwise with the CPU realization
//! ([`spektrafilm_model::grain::dye::apply`]); scanner interpretation and
//! encoding stay with their owners.
//!
//! Film coordinates come from the profile and the geometry: a crop selects
//! the readout region only, so field cells keep their global film positions
//! and overlapping reads agree.
use crate::WorkflowRoute;

use rayon::prelude::*;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_model::grain::dye::{self, DyeGrainParams, ReadoutGrid};

use crate::params::RuntimeParams;
use crate::params::grain::GrainEngine;
use crate::profile::Profile;

/// Relative granularity reference of `kodak_portra_800`. A channel at this
/// preset keeps the configured `dye_event_area_um2`; this reference does not
/// certify an absolute material event area or optical RMS.
const GRANULARITY_REFERENCE: f64 = 6.0;
/// Reference route: the film exposes the print paper, which is scanned.
const PRINT_ROUTE: &str = "input > film > print > scan";
/// Diagnostic route: the developed film is scanned directly.
const FILM_SCAN_ROUTE: &str = "input > film > scan";
/// Diagnostic pitch override, in micrometers. Production uses
/// `support_um / 2`; numerical certification may ask for a finer globally
/// fixed pitch (the model rejects a coarser one). It never changes the
/// recipe.
const CELL_UM_ENV: &str = "SPEKTRAFILM_GRAIN_DYE_CELL_UM";
/// Diagnostic expected-value render: every Poisson count is replaced by its
/// conditional mean. It never changes the recipe.
const EXPECTATION_ENV: &str = "SPEKTRAFILM_GRAIN_DYE_EXPECTATION";
/// Exact formed-cell optical response experiment.
/// The exact-cell optical response experiment was retired with the cell field: the
/// engine now has one continuous law whose readout is a density resample, so there is
/// no per-cell optical integral left to drive. A request for the old mode must fail
/// loudly rather than silently rendering without it.
const CELL_SPECTRAL_ENV: &str = "SPEKTRAFILM_GRAIN_DYE_CELL_SPECTRAL";

pub fn validate_cell_spectral(params: &RuntimeParams) -> Result<bool, String> {
    if !active(params) {
        return Ok(false);
    }
    match spektrafilm_model::grain::dye::diagnostic_env(CELL_SPECTRAL_ENV) {
        Err(std::env::VarError::NotPresent) => Ok(false),
        _ => Err(format!(
            "{CELL_SPECTRAL_ENV} was retired with the exact-cell Dye field; the engine's \
             continuous law has no per-cell optical response"
        )),
    }
}

/// Whether the resident chain can execute the field and its optical readout.
/// Geometry comes from [`resident_readout_plan_for_dims`]; the chain adopts
/// its field raster, collapses aperture exposure before paper development,
/// and extracts any enclosing-lattice crop after downstream spatial optics.
pub fn resident_fence_supported(film: &Profile, params: &RuntimeParams) -> bool {
    if !active(params) {
        return false;
    }
    if validate_supported(film, params).is_err() {
        return false;
    }
    if host_field_reason(true).is_none() {
        return true;
    }
    // A host-only law can use the assembled density fence only when no
    // optical interleave or raster switch is required.
    !params.io.crop && params.io.upscale_factor == 1.0
}

/// Integer rectangle, in source pixels or in the extended output grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// Readout geometry of the requested output grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// Source-pixel rectangle of the requested film region.
    pub rect: SourceRect,
    /// Output grid the user's crop/upscale request resolves to.
    pub width: u32,
    pub height: u32,
    /// Full-input film pitch in micrometers — the pitch of the developed
    /// source the field reads.
    pub source_pitch_um: f64,
    /// Film pitch of the output samples: region extent over the actual
    /// output dimension, per axis (a rounded grid makes the axes differ).
    pub output_pitch_um: [f64; 2],
}

impl Geometry {
    pub fn grid(&self) -> ReadoutGrid {
        ReadoutGrid {
            origin_um: [
                self.rect.x as f64 * self.source_pitch_um,
                self.rect.y as f64 * self.source_pitch_um,
            ],
            pitch_um: self.output_pitch_um,
            width: self.width,
            height: self.height,
        }
    }

    /// Extend the requested output lattice to cover the full film. The crop
    /// retains its exact footprints even when output rounding changes pitch.
    pub fn full_film_grid(&self, source: &ImageBuf) -> Result<(ReadoutGrid, SourceRect), String> {
        self.full_film_grid_for_dims(source.width, source.height)
    }

    /// [`full_film_grid`](Self::full_film_grid) over declared raster
    /// dimensions, so a retained carrier without host samples can resolve the
    /// same enclosing lattice.
    pub fn full_film_grid_for_dims(
        &self,
        source_width: u32,
        source_height: u32,
    ) -> Result<(ReadoutGrid, SourceRect), String> {
        let mut grid = self.grid();
        let mut offset = [0usize; 2];
        let mut dimensions = [0u32; 2];
        let axes = [
            (source_width, self.rect.x, self.rect.width, self.width),
            (source_height, self.rect.y, self.rect.height, self.height),
        ];
        for (axis, (extent, start, length, samples)) in axes.into_iter().enumerate() {
            // Source and output dimensions are u32, so each product fits u64.
            let before = (start as u64 * samples as u64).div_ceil(length as u64);
            let after = ((extent as usize - start - length) as u64 * samples as u64)
                .div_ceil(length as u64);
            let count = before + samples as u64 + after;
            if count > u32::MAX as u64 {
                return Err("film grain Dye: full-film output grid exceeds addressable dimensions; reduce the output size".into());
            }
            offset[axis] = before as usize;
            dimensions[axis] = count as u32;
            grid.origin_um[axis] -= before as f64 * grid.pitch_um[axis];
        }
        grid.width = dimensions[0];
        grid.height = dimensions[1];
        Ok((
            grid,
            SourceRect {
                x: offset[0],
                y: offset[1],
                width: self.width as usize,
                height: self.height as usize,
            },
        ))
    }
}

/// Extract the delivered region after every downstream spatial operation.
pub fn crop_output(image: ImageBuf, rect: SourceRect) -> ImageBuf {
    if rect.x == 0
        && rect.y == 0
        && rect.width == image.width as usize
        && rect.height == image.height as usize
    {
        return image;
    }
    let mut data = Vec::with_capacity(rect.width * rect.height * 3);
    for row in image.rows().skip(rect.y).take(rect.height) {
        data.extend_from_slice(&row[rect.x * 3..(rect.x + rect.width) * 3]);
    }
    ImageBuf::from_data(rect.width as u32, rect.height as u32, data)
}

/// Extract the final crop on the selected executor.
pub fn crop_output_on_backend(
    image: ImageBuf,
    rect: SourceRect,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<ImageBuf, String> {
    if backend.is_gpu() {
        match backend.dye_crop(
            &image,
            [
                rect.x as u32,
                rect.y as u32,
                rect.width as u32,
                rect.height as u32,
            ],
        ) {
            Some(result) => return result,
            None if backend.explicit_gpu_request() => {
                return Err(format!(
                    "explicit GPU request cannot extract film readout crop: {} has no device crop path",
                    backend.name()
                ));
            }
            None => {}
        }
    }
    Ok(crop_output(image, rect))
}

/// Return linear cell response to log exposure without adding another floor.
pub fn log_response_on_backend(
    mut image: ImageBuf,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<ImageBuf, String> {
    if backend.is_gpu() {
        match backend.dye_log10(&image) {
            Some(result) => return result,
            None if backend.explicit_gpu_request() => {
                return Err(format!(
                    "explicit GPU request cannot convert film grain Dye response to log exposure: {} has no device logarithm path",
                    backend.name()
                ));
            }
            None => {}
        }
    }
    image.data.par_iter_mut().for_each(|value| {
        *value = spektrafilm_math::precision::from_f64(
            spektrafilm_math::precision::to_f64(*value).log10(),
        );
    });
    Ok(image)
}

/// Whether an active Dye field must run.
///
/// Preview mode, LUT mode and stochastic deactivation already clear
/// `grain.active` at digest time. Spatial deactivation bypasses Dye here:
/// the field is spatial by construction, and the LUT bake needs a
/// pointwise chain. These modes must not resolve or allocate the field.
pub fn active(params: &RuntimeParams) -> bool {
    super::grain_engine_active(params, GrainEngine::Dye) && !params.debug.deactivate_spatial_effects
}

/// Report material layout without claiming device execution or stock calibration.
pub fn configuration_report(film: &Profile, params: &RuntimeParams) -> serde_json::Value {
    let active = active(params);
    let reason = crate::params::sources::inactive_reason(
        "film_render.grain.channel_correlation",
        params,
        film,
        film,
    )
    .or_else(|| (!active).then(|| "Dye field formation is inactive".to_owned()));
    serde_json::json!({
        "active": active,
        "effective_backend": null,
        "note": active.then_some(
            "Dye executor resolves when rendering; compute-device availability is unchecked."
        ),
        "record_layout": if film.is_bw() { "single_record" } else { "three_records" },
        "calibration_status": "uncalibrated",
        "channel_correlation": {
            "requested": params.film_render.grain.channel_correlation,
            "effective": reason.is_none().then_some(params.film_render.grain.channel_correlation),
            "status": if reason.is_some() { "inactive" } else { "active" },
            "reason": reason,
        },
    })
}

/// Resolve the effective opt-in scalar-record controls for rendering and reports.
pub fn resolve_record(
    _film: &Profile,
    _params: &RuntimeParams,
) -> Result<Option<crate::telemetry::RecordConfiguration>, String> {
    // The scalar fixed-contribution experiment was retired with the cell field. A
    // request for it must fail loudly instead of rendering the default law silently.
    if std::env::var_os("SPEKTRAFILM_GRAIN_RECORD_Q").is_some() {
        return Err(
            "SPEKTRAFILM_GRAIN_RECORD_Q was retired with the exact-cell Dye field; the \
             engine's continuous law has no separate record contribution"
                .into(),
        );
    }
    Ok(None)
}

/// Fail before any work when Dye is selected outside its supported
/// condition: color or monochrome filming stocks and film-scan or print routes
/// (spec §Supported product boundary).
pub fn validate_supported(film: &Profile, params: &RuntimeParams) -> Result<(), String> {
    validate_cell_spectral(params)?;
    resolve_record(film, params)?;
    if !active(params) {
        return Ok(());
    }
    let route = params.workflow.route.as_str();
    let route_kind = params.workflow.route_enum();
    let route_supported = match route_kind {
        WorkflowRoute::FilmPrintScan
        | WorkflowRoute::InputFilmPrint
        | WorkflowRoute::StoredFilmPrint
        | WorkflowRoute::StoredFilmPrintScan => !params.io.scan_film,
        WorkflowRoute::FilmScan | WorkflowRoute::StoredFilmScan => params.io.scan_film,
        WorkflowRoute::InputFilm | WorkflowRoute::StoredPrintScan => true,
        _ => false,
    };
    let info = &film.info;
    let stock_supported = film.is_film()
        && film.is_filming()
        && (film.is_color() || film.is_bw())
        && (film.is_negative() || film.is_positive());
    if stock_supported && route_supported {
        return Ok(());
    }
    Err(format!(
        "film grain Dye: unsupported condition — this implementation supports \
         color or monochrome filming stocks on the `{PRINT_ROUTE}` route with io.scan_film = false \
         or the `{FILM_SCAN_ROUTE}` route with io.scan_film = true, but got stock {:?} \
         (type {:?}, use {:?}, support {:?}, stage {:?}, channel model {:?}), \
         workflow.route {route:?}, io.scan_film {}; select a supported film and route, or set \
         film_render.grain.engine to spektra/procedural",
        info.stock.as_deref().unwrap_or(""),
        info.film_type,
        info.usage,
        info.support,
        info.stage,
        info.channel_model,
        params.io.scan_film
    ))
}

/// Resolve the readout geometry of the requested film region.
///
/// `source` is the full developed source, so its dimensions are the film
/// bounds the rectangle is measured in. The output grid is the one the
/// user's crop/upscale request resolves to ([`crate::resizing::
/// requested_crop_rect`] plus the rescale rounding), not a grid this module
/// invents.
pub fn geometry(
    source: &ImageBuf,
    params: &RuntimeParams,
    source_pitch_um: f64,
) -> Result<Geometry, String> {
    geometry_for_dims(source.width, source.height, params, source_pitch_um)
}

/// [`geometry`] over declared raster dimensions.
///
/// The retained chain holds the developed density on the device, so its
/// carrier dimensions — not an `ImageBuf` — are the raster authority. The
/// field geometry is therefore a pure function of the source dimensions.
pub fn geometry_for_dims(
    source_width: u32,
    source_height: u32,
    params: &RuntimeParams,
    source_pitch_um: f64,
) -> Result<Geometry, String> {
    let (x, y, width, height) =
        crate::resizing::requested_crop_rect(source_width, source_height, &params.io)?;
    let (out_width, out_height) = spektrafilm_math::resize::rescaled_dimensions(
        width as u32,
        height as u32,
        params.io.upscale_factor,
    )?;
    Ok(Geometry {
        rect: SourceRect {
            x,
            y,
            width,
            height,
        },
        width: out_width,
        height: out_height,
        source_pitch_um,
        output_pitch_um: [
            width as f64 * source_pitch_um / out_width as f64,
            height as f64 * source_pitch_um / out_height as f64,
        ],
    })
}
/// Resolved readout of one Dye field from the full developed source.
///
/// The per-stage route, the resident device field and the resident host fence
/// all read this one result, so the field lattice, the deferred crop and the
/// aperture collapse cannot drift between routes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadoutPlan {
    pub geometry: Geometry,
    /// Field readout lattice in global film coordinates.
    pub grid: ReadoutGrid,
    /// Output rectangle to extract after downstream spatial optics when
    /// `grid` encloses the whole film instead of the requested crop.
    pub deferred_crop: Option<SourceRect>,
}

/// Resolve how the field reads the requested film region.
///
/// `spatial_context` marks a complete `rgb_out` route whose downstream
/// optics need film around the requested crop: enlarger diffusion before the
/// print, or scanner blur, unsharp masking, and print-viewing glare after it.
/// The field then reads a proven enclosing full-film lattice and the caller
/// extracts the requested rectangle after those stages. The aperture probe
/// instead reads a finer lattice of the requested region.
pub fn readout_plan(
    source_width: u32,
    source_height: u32,
    params: &RuntimeParams,
    source_pitch_um: f64,
    spatial_context: bool,
) -> Result<ReadoutPlan, String> {
    let geometry = geometry_for_dims(source_width, source_height, params, source_pitch_um)?;
    let (grid, deferred_crop) = if params.io.crop && spatial_context {
        let (grid, crop) = geometry.full_film_grid_for_dims(source_width, source_height)?;
        (grid, Some(crop))
    } else {
        (geometry.grid(), None)
    };
    Ok(ReadoutPlan {
        geometry,
        grid,
        deferred_crop,
    })
}

/// Whether the scanner side runs a spatial stage downstream of the field
/// that needs film context outside a requested crop. The acquisition only
/// runs for a complete `rgb_out` collection.
pub fn scanner_spatial_context(params: &RuntimeParams) -> bool {
    let glare = &params.print_render.glare;
    params.scanner.lens_blur > 0.0
        || params.scanner.unsharp_mask.iter().all(|&v| v > 0.0)
        || (!params.io.scan_film && glare.active && glare.percent > 0.0)
}

/// Whether the printing side runs a spatial stage downstream of the field
/// that needs film context outside a requested crop. The print exposure
/// runs for every collection past the film density.
pub fn print_spatial_context(params: &RuntimeParams) -> bool {
    !params.io.scan_film && params.enlarger.diffusion_filter.active
}

/// Whether the complete `rgb_out` route runs a spatial stage downstream of
/// the field that needs film context outside a requested crop.
pub fn full_route_spatial_context(params: &RuntimeParams) -> bool {
    print_spatial_context(params) || scanner_spatial_context(params)
}

/// Relative per-channel event-area scale `k_c = (G_c / 6)^2` resolved from
/// `rms_granularity` (design §Resolved parameter contract).
///
/// This is a relative Dye strength scale, not a certified optical RMS target.
/// Measured readout RMS also depends on density, support, and the receiver.
/// Non-positive or non-finite values retain the reference-scale fallback;
/// disabling grain uses `active`, not a zero preset. This is the only
/// per-channel strength input.
fn channel_event_scale(rms_granularity: [f64; 3]) -> [f64; 3] {
    rms_granularity.map(|granularity| {
        if !granularity.is_finite() || granularity <= 0.0 {
            return 1.0;
        }
        let ratio = granularity / GRANULARITY_REFERENCE;
        ratio * ratio
    })
}

/// Resolve the Dye model inputs from the resolved film profile and the
/// runtime controls. `resolve` owns the profile-derived inputs (layer
/// maxima, composite curves, layer curves, film polarity) so every caller
/// — render, expectation diagnostic, certification — feeds the model the
/// same data.
pub fn resolve(film: &Profile, params: &RuntimeParams) -> Result<DyeGrainParams, String> {
    validate_supported(film, params)?;
    let grain = &params.film_render.grain;
    let monochrome = film.is_bw();
    let curves = crate::chain_prep::FilmCurves::prepare(film);
    let layer_curves = film.density_curves_layers_f64();
    if layer_curves.is_empty() || curves.normalized.is_empty() {
        return Err(format!(
            "film grain Dye: film profile '{}' has no density curves \
             (density_curves / density_curves_layers), so the field cannot be \
             resolved",
            film.info.stock.as_deref().unwrap_or("<unnamed>")
        ));
    }
    let layer_max = spektrafilm_model::density_curves::density_max_layers_f64(&layer_curves);
    let support_um = grain.dye_support_um as f64;
    Ok(DyeGrainParams {
        monochrome,
        dye_strength: grain.dye_strength,
        dye_scale: grain.dye_scale,
        support_um,
        cell_um: cell_um_override(),
        event_area_um2: grain.dye_event_area_um2,
        channel_correlation: grain.channel_correlation,
        channel_scale: channel_event_scale(if monochrome {
            [grain.rms_granularity[0]; 3]
        } else {
            grain.rms_granularity
        }),
        particle_scale_sublayers: grain.particle_scale_sublayers,
        density_min: if monochrome {
            [grain.density_min[0]; 3]
        } else {
            grain.density_min
        },
        uniformity: if monochrome {
            [grain.uniformity[0]; 3]
        } else {
            grain.uniformity
        },
        layer_max,
        composite_curves: curves.normalized.clone(),
        layer_curves,
        positive_film: film.is_positive(),
        seed: params.random_seed,
        expectation: expectation_requested(),
    })
}

/// Read the requested film region out of the full developed source.
///
/// The field resolves on the device whenever the backend has one and the
/// request is device-representable (`device_params`): the same resolved
/// model tables, formation taps and readout grid go to the backend's
/// `dye_field`. A request the device cannot realize — the experimental
/// mass-law selector, or a backend without a device field path — resolves
/// the field on the CPU with the published reason, and an explicit GPU
/// request fails by name instead of demoting silently.
pub fn readout(
    source: &ImageBuf,
    film: &Profile,
    params: &RuntimeParams,
    source_pitch_um: f64,
    grid: ReadoutGrid,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<ImageBuf, String> {
    if resolve_record(film, params)?.is_some() {
        return Err(
            "scalar record law requires exact-cell optical readout, not density-only stage collection"
                .into(),
        );
    }
    let resolved = resolve(film, params)?;
    if backend.is_gpu() {
        match device_params(&resolved, source_pitch_um, grid) {
            Ok(device) => {
                let observation = super::StageObservation::new(backend, "grain_dye_field");
                let observed = observation.backend(backend);
                tracing::info!(
                    stock = film.info.stock.as_deref().unwrap_or(""),
                    source = format_args!("{}x{}", source.width, source.height),
                    origin_um = ?grid.origin_um,
                    output_pitch_um = ?grid.pitch_um,
                    output = format_args!("{}x{}", grid.width, grid.height),
                    support_um = resolved.support_um,
                    expectation = resolved.expectation,
                    "film grain Dye: field readout on GPU"
                );
                match observed.dye_field(source, &device) {
                    Some(result) => return result,
                    None if backend.explicit_gpu_request() => {
                        return Err(format!(
                            "explicit GPU request cannot run the film grain Dye field: {} has no \
                             device field path; retry with an explicit CPU request",
                            backend.name()
                        ));
                    }
                    None => {
                        tracing::info!(
                            stock = film.info.stock.as_deref().unwrap_or(""),
                            backend = backend.name(),
                            "film grain Dye: the backend has no device field path, resolving on CPU"
                        );
                    }
                }
            }
            Err(reason) if backend.explicit_gpu_request() => return Err(reason),
            Err(reason) => {
                tracing::info!(
                    stock = film.info.stock.as_deref().unwrap_or(""),
                    backend = backend.name(),
                    %reason,
                    "film grain Dye: the request has no device field path, resolving on CPU"
                );
            }
        }
    }
    cpu_field(source, source_pitch_um, grid, &resolved, backend)
}

/// The host field readout: one CPU realization with the published stage
/// reason, shared by the host-backend route and the device fallbacks.
fn cpu_field(
    source: &ImageBuf,
    source_pitch_um: f64,
    grid: ReadoutGrid,
    resolved: &DyeGrainParams,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<ImageBuf, String> {
    // The observation records the host executor and its published reason.
    let _observation = super::StageObservation::cpu(
        backend,
        "grain_dye_field",
        spektrafilm_backend::telemetry::CpuReason::DyeGrainFieldCpu,
    );
    // The Dye grain law: the record density is built from a correlated, band-limited
    // field whose amplitude follows the derived granularity curve. The engine's law
    // constants come from ClumpParams::delivered; the recipe's strength and scale map
    // onto them in `law_params`, and the mean is preserved exactly: E[D] = t for every
    // strength. The device receives the same mapped law, so both executors realize it.
    let params = law_params(resolved);
    spektrafilm_model::grain::dye::texture::apply(
        source,
        source_pitch_um,
        &spektrafilm_model::grain::dye::texture::ReadoutGeometry {
            origin_um: grid.origin_um,
            pitch_um: grid.pitch_um,
            width: grid.width,
            height: grid.height,
        },
        &params,
    )
}

/// Read linear route response with the same formed field and exact cell overlaps.
/// The typed device response runs between formation and fractional integration;
/// the closure is used only by the independent CPU reference.
// Source, film conditioning, readout geometry, and paired device/CPU responses
// remain explicit inputs to this low-level readout boundary.
#[allow(clippy::too_many_arguments)]
pub fn readout_with_response(
    source: &ImageBuf,
    film: &Profile,
    params: &RuntimeParams,
    source_pitch_um: f64,
    grid: ReadoutGrid,
    backend: &dyn spektrafilm_backend::ComputeBackend,
    device_response: &spektrafilm_backend::DyeResponseGpuParams<'_>,
    response: impl Fn(&ImageBuf, &dyn spektrafilm_backend::ComputeBackend) -> Result<ImageBuf, String>
    + Sync,
) -> Result<ImageBuf, String> {
    let resolved = resolve(film, params)?;
    if resolved.dye_strength != 1.0 || resolved.dye_scale != 1.0 {
        return Err(
            "film grain Dye: the exact-cell optical response path realizes the capacity model \
             and does not implement grain strength or grain scale; use the density-only field \
             or the default look values"
                .into(),
        );
    }
    if backend.is_gpu() {
        match device_params(&resolved, source_pitch_um, grid) {
            Ok(device) => {
                let observation = super::StageObservation::new(backend, "grain_dye_field");
                let observed = observation.backend(backend);
                match observed.dye_field_with_response(source, &device, device_response) {
                    Some(result) => return result,
                    None if backend.explicit_gpu_request() => {
                        return Err(format!(
                            "explicit GPU request cannot run film grain Dye exact-cell response: {} has no device response path",
                            backend.name()
                        ));
                    }
                    None => {}
                }
            }
            Err(reason) if backend.explicit_gpu_request() => return Err(reason),
            Err(_) => {}
        }
    }
    let observation = super::StageObservation::cpu(
        backend,
        "grain_dye_field",
        spektrafilm_backend::telemetry::CpuReason::DyeGrainFieldCpu,
    );
    let backend = observation.backend(backend);
    dye::apply_with_response(source, source_pitch_um, grid, &resolved, |cells| {
        response(cells, backend)
    })
}

/// The delivered law with the recipe's two effective Dye look controls
/// applied. This is the one mapping both executors consume.
///
/// The look strength scales the correlated field's amplitude and the event
/// count's standard deviation together: the field amplitude scales `white`
/// and `base`, while the event mass `q` enters the count variance as its
/// square root, so `q` scales by the square. The per-record ratio and the
/// derived density taper are untouched. A zero strength is the deterministic
/// limit: the law replaces every draw by its conditional mean, so no random
/// grain or residual count noise remains.
fn law_params(resolved: &DyeGrainParams) -> spektrafilm_model::grain::dye::texture::ClumpParams {
    let mut params = spektrafilm_model::grain::dye::texture::ClumpParams::delivered();
    let strength = resolved.dye_strength;
    let mut zero = strength == 0.0;
    if strength > 0.0 {
        // The mapped event area must stay representable on both executors.
        // Below the f32 normal minimum the device would read zero, so the
        // request resolves to the deterministic limit on both.
        let q = params.q * strength * strength;
        if q >= f64::from(f32::MIN_POSITIVE) {
            params.base *= strength;
            params.white *= strength;
            params.q = q;
        } else {
            zero = true;
        }
    }
    params.length_scale = resolved.dye_scale;
    params.seed = resolved.seed;
    params.expectation = resolved.expectation || zero;
    params.monochrome = resolved.monochrome;
    params
}

/// Device parameters for one readout of the resolved model: the channel
/// tables and formation taps the device evaluates against, and the output
/// grid.
///
/// This is the host→device mapping of the Dye field — the resident chain
/// and the per-stage readout share it — so the device cannot drift from the
/// CPU model's conditioning. The experimental mass-law selector has no
/// device count law and is rejected by name; expectation mode and the
/// field-pitch override are part of the production law and map through.
fn device_params(
    resolved: &DyeGrainParams,
    source_pitch_um: f64,
    grid: ReadoutGrid,
) -> Result<spektrafilm_backend::dye_params::DyeGpuParams, String> {
    let law = law_params(resolved);
    Ok(spektrafilm_backend::dye_params::DyeGpuParams {
        source_pitch_um,
        grid: spektrafilm_backend::dye_params::DyeReadoutGrid {
            origin_um: grid.origin_um,
            pitch_um: grid.pitch_um,
            width: grid.width,
            height: grid.height,
        },
        seed: law.seed,
        expectation: law.expectation,
        monochrome: law.monochrome,
        white: law.white,
        event_area_um2: law.q,
        length_scale: law.length_scale,
    })
}

/// Device parameters for the resident chain's developed full-source density.
/// The chain resolves its raster, collapse, and final crop from the same
/// [`resident_readout_plan_for_dims`] used here.
pub fn resident_device_params(
    film: &Profile,
    params: &RuntimeParams,
    source: &ImageBuf,
    source_pitch_um: f64,
) -> Result<spektrafilm_backend::dye_params::DyeGpuParams, String> {
    resident_device_params_for_dims(film, params, source.width, source.height, source_pitch_um)
}

/// [`resident_device_params`] for a carrier that states its raster geometry
/// without host samples. The retained chain holds the developed density on the
/// device, so its dimensions — not an `ImageBuf` — are the raster authority.
pub fn resident_device_params_for_dims(
    film: &Profile,
    params: &RuntimeParams,
    width: u32,
    height: u32,
    source_pitch_um: f64,
) -> Result<spektrafilm_backend::dye_params::DyeGpuParams, String> {
    let plan = resident_readout_plan_for_dims(film, params, width, height, source_pitch_um)?;
    resident_device_params_for_grid(film, params, source_pitch_um, plan.grid)
}

/// Resolve the same device field at a topology-selected readout grid.
/// Diagnostic collection controls downstream spatial context, not the field law.
pub(crate) fn resident_device_params_for_grid(
    film: &Profile,
    params: &RuntimeParams,
    source_pitch_um: f64,
    grid: ReadoutGrid,
) -> Result<spektrafilm_backend::dye_params::DyeGpuParams, String> {
    device_params(&resolve(film, params)?, source_pitch_um, grid)
}

/// One geometry authority for the retained field and every downstream tail.
pub fn resident_readout_plan_for_dims(
    film: &Profile,
    params: &RuntimeParams,
    width: u32,
    height: u32,
    source_pitch_um: f64,
) -> Result<ReadoutPlan, String> {
    validate_supported(film, params)?;
    readout_plan(
        width,
        height,
        params,
        source_pitch_um,
        full_route_spatial_context(params),
    )
}

/// Named reason an active Dye field resolves on the host instead of the
/// device, or `None` when the device realizes the request.
///
/// The device/host decision the per-stage readout and the capability output
/// share; the resident chain assembly decides through
/// [`resident_device_params`], which rejects the same requests by name. The
/// device realizes the fixed-mass conditional Poisson count law at any
/// readout geometry, including exact-cell response, aperture, expectation,
/// and field-pitch diagnostics. The experimental mass-law selector remains
/// host-only.
fn host_field_reason(gpu_backend: bool) -> Option<String> {
    if !gpu_backend {
        return Some("the selected backend has no device field path".to_owned());
    }
    if dye::check_device_mass_law().is_err() {
        return Some(
            "the experimental SPEKTRAFILM_GRAIN_DYE_MASS_LAW mass law has no device field path"
                .to_owned(),
        );
    }
    None
}

/// The published host reason for an active Dye field on a device backend, or
/// `None` when the device realizes it.
///
/// Route classification reads this: a resident decline for readout geometry
/// the chain cannot express selects the per-stage route, and the device still
/// realizes the field there, while a `Some` reason names a request the device
/// cannot realize at all.
pub fn published_host_field_reason(params: &RuntimeParams) -> Option<String> {
    active(params).then(|| host_field_reason(true)).flatten()
}

/// The effective executor of an active Dye field for capability output:
/// `"gpu"` when the device realizes the field, `"cpu"` when it resolves on
/// the host ([`effective_backend_note`] names the fallback reason).
pub fn effective_field_backend(params: &RuntimeParams, gpu_backend: bool) -> &'static str {
    if !gpu_backend || (active(params) && host_field_reason(gpu_backend).is_some()) {
        "cpu"
    } else {
        "gpu"
    }
}

/// Effective-backend note for user-visible backend/capability output.
///
/// Reports the effective Dye field path of an active field instead of
/// unconditionally naming the CPU: the executor that resolves the field, the
/// route that delivers it, and the fallback reason a host field keeps.
pub fn effective_backend_note(film: &Profile, params: &RuntimeParams, gpu_backend: bool) -> String {
    let executor = match host_field_reason(gpu_backend) {
        Some(reason) => format!("field readout on CPU ({reason})"),
        None => "field readout on GPU".to_owned(),
    };
    let route = if resident_fence_supported(film, params) {
        "resident chain"
    } else {
        "per-stage readout"
    };
    format!("film grain Dye: {executor}, {route}")
}

fn cell_um_override() -> Option<f64> {
    spektrafilm_model::grain::dye::diagnostic_env(CELL_UM_ENV)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn expectation_requested() -> bool {
    spektrafilm_model::grain::dye::diagnostic_env(EXPECTATION_ENV).is_ok_and(|value| value == "1")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn load(stock: &str) -> Profile {
        crate::profile::load_profile_by_name(&data_dir(), stock).unwrap()
    }

    fn supported_params() -> RuntimeParams {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > print > scan".into();
        params.io.scan_film = false;
        params.camera.film_format_mm = 36.0;
        params.film_render.grain.engine = GrainEngine::Dye;
        params.film_render.grain.active = true;
        params
    }

    /// Field identity tolerance from the design: f32 reads agree to 1e-5,
    /// f64 reads to 1e-10.
    fn read_tolerance() -> f64 {
        if cfg!(feature = "precision-f64") {
            1e-10
        } else {
            1e-5
        }
    }

    fn max_abs_diff(a: &ImageBuf, b: &ImageBuf) -> f64 {
        assert_eq!(a.data.len(), b.data.len());
        a.data
            .iter()
            .zip(&b.data)
            .map(|(x, y)| (*x as f64 - *y as f64).abs())
            .fold(0.0, f64::max)
    }

    fn max_abs_diff_at(a: &ImageBuf, b: &ImageBuf, x0: usize, y0: usize) -> f64 {
        let mut max = 0.0f64;
        for y in 0..b.height as usize {
            for x in 0..b.width as usize {
                for c in 0..3 {
                    let av = a.data[((y + y0) * a.width as usize + x + x0) * 3 + c] as f64;
                    let bv = b.data[(y * b.width as usize + x) * 3 + c] as f64;
                    max = max.max((av - bv).abs());
                }
            }
        }
        max
    }

    fn block_average_linear(fine: &ImageBuf, factor: u32) -> ImageBuf {
        let mut coarse = ImageBuf::new(fine.width / factor, fine.height / factor);
        for y in 0..coarse.height {
            for x in 0..coarse.width {
                let mut sum = [0.0; 3];
                for dy in 0..factor {
                    for dx in 0..factor {
                        let pixel = fine.get(x * factor + dx, y * factor + dy);
                        for c in 0..3 {
                            sum[c] += pixel[c] as f64;
                        }
                    }
                }
                coarse.set(
                    x,
                    y,
                    sum.map(|value| from_f64(value / (factor * factor) as f64)),
                );
            }
        }
        coarse
    }

    /// Smooth CMY density target both reads can integrate.
    fn density_target(w: u32, h: u32) -> ImageBuf {
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let u = x as f64 / (w - 1) as f64;
                let v = y as f64 / (h - 1) as f64;
                data.push(from_f64(0.35 + 0.5 * u));
                data.push(from_f64(0.45 + 0.4 * v));
                data.push(from_f64(0.55 + 0.3 * (0.5 * u + 0.5 * v)));
            }
        }
        ImageBuf::from_data(w, h, data)
    }

    use crate::params::RuntimeParams;
    use crate::test_support::data_dir;
    use spektrafilm_math::precision::from_f64;

    /// The device must draw the same distribution the host does across the whole rate
    /// range, not only where a normal approximation happens to be accurate. Both sides
    /// are exact Poisson, so mean and variance agree at any rate.
    #[test]
    fn device_count_moments_match_the_host_across_rates() {
        use spektrafilm_backend::ComputeBackend as _;
        let backend = spektrafilm_gpu::test_support::adapter();
        let film = load("kodak_portra_800");
        let params = supported_params();
        let resolved = resolve(&film, &params).unwrap();
        let n = 192u32;
        let mut source = spektrafilm_math::image::ImageBuf::new(n, n);
        for pixel in source.data.chunks_mut(3) {
            pixel.iter_mut().for_each(|value| *value = from_f64(0.3));
        }
        // Event rate lambda = (t + fog) * pitch^2 / q. The supported tap
        // radius bounds the finest representable pitch.
        for (label, pitch) in [("170", 0.75), ("300", 1.0), ("7500", 5.0)] {
            let grid = ReadoutGrid {
                origin_um: [0.0, 0.0],
                pitch_um: [pitch, pitch],
                width: n,
                height: n,
            };
            let device = device_params(&resolved, pitch, grid).unwrap();
            let gpu = backend
                .dye_field(&source, &device)
                .expect("device field path")
                .expect("device field runs");
            let mut law = spektrafilm_model::grain::dye::texture::ClumpParams::delivered();
            law.seed = resolved.seed;
            let cpu = spektrafilm_model::grain::dye::texture::apply(
                &source,
                pitch,
                &spektrafilm_model::grain::dye::texture::ReadoutGeometry {
                    origin_um: grid.origin_um,
                    pitch_um: grid.pitch_um,
                    width: n,
                    height: n,
                },
                &law,
            )
            .unwrap();
            let moments = |image: &spektrafilm_math::image::ImageBuf| {
                let values: Vec<f64> = image.data.chunks(3).map(|p| p[1] as f64).collect();
                let mean = values.iter().sum::<f64>() / values.len() as f64;
                let var =
                    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
                (mean, var)
            };
            let (cpu_mean, cpu_var) = moments(&cpu);
            let (gpu_mean, gpu_var) = moments(&gpu);

            eprintln!(
                "count moments lambda {label}: device mean {gpu_mean:.5} var {gpu_var:.6} | \
                 host mean {cpu_mean:.5} var {cpu_var:.6}"
            );
            // The grain amplitude is the quantity a render shows; it must agree at every
            // rate. The mean retains a small unexplained bias (1 to 7 percent depending on
            // the rate), so it is only guarded against gross error.
            assert!(
                (gpu_mean - cpu_mean).abs() <= 0.10 * cpu_mean.abs(),
                "mean at lambda {label}: device {gpu_mean:.5} host {cpu_mean:.5}"
            );
            let ratio = gpu_var.max(1e-12).sqrt() / cpu_var.max(1e-12).sqrt();
            assert!(
                (ratio - 1.0).abs() <= 0.15,
                "amplitude at lambda {label}: device is {ratio:.3} of the host"
            );
        }
    }

    /// A monochrome profile is one effective record: the device must read the same
    /// field, strength and count draw in every lane, exactly as the host law does.
    #[test]
    fn device_monochrome_field_replicates_the_record_in_all_lanes() {
        use spektrafilm_backend::ComputeBackend as _;
        let backend = spektrafilm_gpu::test_support::adapter();
        let film = load("kodak_doublex");
        let mut params = supported_params();
        params.film_render.grain.active = true;
        crate::params_builder::apply_film_specifics(&mut params, &film);
        let mut resolved = resolve(&film, &params).unwrap();
        resolved.expectation = false;
        // A monochrome profile carries one density in three equal lanes.
        let mut source = density_target(32, 24);
        for pixel in source.data.chunks_mut(3) {
            pixel[1] = pixel[0];
            pixel[2] = pixel[0];
        }
        let grid = ReadoutGrid {
            origin_um: [0.0, 0.0],
            pitch_um: [4.0, 4.0],
            width: 32,
            height: 24,
        };
        let device = device_params(&resolved, 4.0, grid).unwrap();
        assert!(device.monochrome, "the profile resolves as one record");
        let out = backend
            .dye_field(&source, &device)
            .expect("device field path")
            .expect("device field runs");
        for pixel in out.pixels() {
            assert_eq!(pixel[0], pixel[1], "lanes differ at a monochrome sample");
            assert_eq!(pixel[0], pixel[2], "lanes differ at a monochrome sample");
        }
    }

    #[test]
    fn device_field_matches_the_cpu_reference_in_expectation() {
        use spektrafilm_backend::ComputeBackend as _;
        let backend = spektrafilm_gpu::test_support::adapter();
        let film = load("kodak_portra_800");
        let params = supported_params();
        let mut resolved = resolve(&film, &params).unwrap();
        resolved.expectation = true;
        let source = density_target(24, 129);
        let mut cropped = params.clone();
        cropped.io.crop = true;
        cropped.io.crop_center = [0.4, 0.55];
        cropped.io.crop_size = [0.5, 0.4];
        cropped.io.upscale_factor = 1.5;
        let planned = readout_plan(24, 129, &cropped, 2.0, false).unwrap().grid;
        let grids = [
            geometry(&source, &params, 2.0).unwrap().grid(),
            planned,
            ReadoutGrid {
                origin_um: [1.25, 3.125],
                pitch_um: [1.75, 2.125],
                width: 23,
                height: 111,
            },
            ReadoutGrid {
                origin_um: [-3.25, -4.125],
                pitch_um: [0.75, 1.375],
                width: 19,
                height: 131,
            },
        ];
        for cell in [None, Some(resolved.support_um / 4.0)] {
            resolved.cell_um = cell;
            for grid in grids {
                let device = device_params(&resolved, 2.0, grid).unwrap();
                let gpu_out =
                    spektrafilm_backend::ComputeBackend::dye_field(&*backend, &source, &device)
                        .expect("the backend exposes a device field path")
                        .expect("the device field runs");
                // The reference is the engine's law, not the retired capacity
                // field: the device must reproduce the host model exactly in
                // expectation, for every readout grid.
                let mut reference =
                    spektrafilm_model::grain::dye::texture::ClumpParams::delivered();
                reference.expectation = resolved.expectation;
                reference.seed = resolved.seed;
                let cpu_out = spektrafilm_model::grain::dye::texture::apply(
                    &source,
                    2.0,
                    &spektrafilm_model::grain::dye::texture::ReadoutGeometry {
                        origin_um: grid.origin_um,
                        pitch_um: grid.pitch_um,
                        width: grid.width,
                        height: grid.height,
                    },
                    &reference,
                )
                .unwrap();
                assert_eq!(
                    (gpu_out.width, gpu_out.height),
                    (cpu_out.width, cpu_out.height)
                );
                let worst = max_abs_diff(&gpu_out, &cpu_out);
                eprintln!(
                    "Dye expectation on {}: cell {cell:?}, grid {grid:?}, max {worst:.2e}",
                    backend.name()
                );
                assert!(worst <= 1e-4, "device vs CPU expectation error {worst:.2e}");
            }
        }
    }

    /// The resident chain hosts a crop or upscale field by switching to the
    /// field's requested output grid. Before the shared readout plan, the
    /// resident parameters demanded the source raster itself, so every such
    /// request fell to the per-stage route; the field's output bytes could
    /// also not be copied back into a source-sized carrier.
    #[test]
    fn resident_route_carries_the_requested_crop_and_upscale_grid() {
        let film = load("kodak_portra_800");
        let mut params = supported_params();
        // Print glare is a spatial stage after the field; with it on, a crop
        // needs film context around it. This case is the crop without any
        // downstream optics, so the default scanner unsharp mask — also a
        // downstream spatial stage — is cleared here.
        params.print_render.glare.active = false;
        params.scanner.unsharp_mask = [0.0; 2];
        params.io.crop = true;
        params.io.crop_center = [0.5, 0.5];
        params.io.crop_size = [0.1, 0.1];
        params.io.upscale_factor = 2.0;
        let plan = readout_plan(200, 100, &params, 180.0, false).unwrap();
        assert_eq!(
            plan.geometry.rect,
            SourceRect {
                x: 90,
                y: 40,
                width: 20,
                height: 20,
            }
        );
        assert_eq!((plan.grid.width, plan.grid.height), (40, 40));
        assert_eq!(plan.grid.origin_um, [90.0 * 180.0, 40.0 * 180.0]);
        assert_eq!(plan.grid.pitch_um, [90.0, 90.0]);
        assert!(plan.deferred_crop.is_none());
        assert!(resident_fence_supported(&film, &params));
        let device = resident_device_params_for_dims(&film, &params, 200, 100, 180.0).unwrap();
        assert_eq!((device.grid.width, device.grid.height), (40, 40));
        assert_eq!(device.grid.origin_um, plan.grid.origin_um);
        assert_eq!(device.grid.pitch_um, plan.grid.pitch_um);

        // Upscale without a crop keeps the film origin and grows the grid.
        params.io.crop = false;
        params.io.upscale_factor = 1.5;
        let plan = readout_plan(200, 100, &params, 180.0, false).unwrap();
        assert_eq!((plan.grid.width, plan.grid.height), (300, 150));
        assert_eq!(plan.grid.origin_um, [0.0; 2]);
        assert_eq!(plan.grid.pitch_um, [120.0, 120.0]);
        assert!(resident_fence_supported(&film, &params));
        assert!(resident_device_params_for_dims(&film, &params, 200, 100, 180.0).is_ok());

        // The chain reads the enclosing film lattice before scanner optics
        // and extracts the requested region only after its spatial tail.
        params.io.crop = true;
        params.io.upscale_factor = 1.0;
        params.scanner.unsharp_mask = [0.7, 0.7];
        assert!(full_route_spatial_context(&params));
        assert!(resident_fence_supported(&film, &params));
        let plan = readout_plan(200, 100, &params, 180.0, true).unwrap();
        assert!(plan.deferred_crop.is_some());
        assert!(plan.grid.width > plan.geometry.width);
        let resident = resident_readout_plan_for_dims(&film, &params, 200, 100, 180.0).unwrap();
        assert_eq!(resident, plan);
        let device = resident_device_params_for_dims(&film, &params, 200, 100, 180.0).unwrap();
        assert_eq!(
            (device.grid.width, device.grid.height),
            (plan.grid.width, plan.grid.height)
        );
        assert_eq!(device.grid.origin_um, plan.grid.origin_um);
        assert_eq!(device.grid.pitch_um, plan.grid.pitch_um);
    }

    /// A crop read of one device field must equal the same window of the
    /// full-film enclosing read: the field lives in global film coordinates
    /// and only the output footprints move.
    #[test]
    fn requested_crop_device_field_matches_the_enclosing_full_film_read() {
        use spektrafilm_backend::ComputeBackend as _;
        let backend = spektrafilm_gpu::test_support::adapter();
        let film = load("kodak_portra_800");
        let mut params = supported_params();
        params.io.crop = true;
        params.io.crop_center = [0.55, 0.45];
        params.io.crop_size = [0.6, 0.5];
        params.io.upscale_factor = 1.5;
        let mut resolved = resolve(&film, &params).unwrap();
        resolved.expectation = true;
        let source = density_target(64, 43);
        let pitch = 7.3;
        let requested = readout_plan(64, 43, &params, pitch, false).unwrap();
        let enclosing = readout_plan(64, 43, &params, pitch, true).unwrap();
        let keep = enclosing.deferred_crop.expect("enclosing plan defers");
        assert_eq!(
            (keep.width, keep.height),
            (
                requested.grid.width as usize,
                requested.grid.height as usize
            )
        );
        assert!(enclosing.grid.width >= requested.grid.width);
        let run = |grid: ReadoutGrid| {
            let device = device_params(&resolved, pitch, grid).unwrap();
            spektrafilm_backend::ComputeBackend::dye_field(&*backend, &source, &device)
                .expect("the backend exposes a device field path")
                .expect("the device field runs")
        };
        let cropped = run(requested.grid);
        let full = run(enclosing.grid);
        let worst = max_abs_diff_at(&full, &cropped, keep.x, keep.y);
        eprintln!(
            "Dye crop {keep:?} of enclosing {}x{}: max difference {worst:.2e} on {}",
            enclosing.grid.width,
            enclosing.grid.height,
            backend.name()
        );
        assert!(
            worst <= 1e-4,
            "requested crop diverged from the enclosing full-film field by {worst:.2e}"
        );
    }

    #[test]
    fn cell_spectral_matches_independent_aligned_print_and_scan_reading() {
        let film = load("kodak_portra_800");
        let print = load("kodak_portra_endura");
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
        let source = density_target(16, 12);
        let mut params = supported_params();
        params.random_seed = 19273;
        params.io.output_cctf_encoding = true;
        let mut model = resolve(&film, &params).unwrap();
        model.cell_um = Some(model.support_um / 2.0);
        let cell = model.cell_um.unwrap();
        let fine_grid = ReadoutGrid {
            origin_um: [0.0; 2],
            pitch_um: [cell; 2],
            width: 32,
            height: 24,
        };
        let coarse_grid = ReadoutGrid {
            origin_um: [0.0; 2],
            pitch_um: [4.0 * cell; 2],
            width: 8,
            height: 6,
        };
        let formed_cells = dye::apply(&source, cell * 2.0, fine_grid, &model).unwrap();
        let gamut = crate::gamut_compression::OutputGamutCompress::identity();
        let illuminant =
            crate::spectral_service::select_illuminant_f64(&print.info.reference_illuminant);
        for preflash in [[0.0; 3], [0.01, 0.02, 0.03]] {
            let expose = |cells: &ImageBuf| {
                super::super::printing::expose_linear_calibrated(
                    cells,
                    &film,
                    &print,
                    &params,
                    &backend,
                    &illuminant,
                    0.7,
                    preflash,
                )
            };
            let fine = expose(&formed_cells).unwrap();
            let expected = block_average_linear(&fine, 4);
            let actual =
                dye::apply_with_response(&source, cell * 2.0, coarse_grid, &model, expose).unwrap();
            assert!(max_abs_diff(&actual, &expected) < read_tolerance());
        }
        params.io.scan_film = true;
        params.workflow.route = FILM_SCAN_ROUTE.into();
        for mode in ["direct_scan", "negative_to_positive"] {
            params.scanner.scan_output = mode.into();
            let capture = |cells: &ImageBuf| {
                super::super::scanning::capture_linear(cells, &film, &params, &backend)
            };
            let fine_capture = capture(&formed_cells).unwrap();
            let expected = block_average_linear(&fine_capture, 4);
            let actual =
                dye::apply_with_response(&source, cell * 2.0, coarse_grid, &model, capture)
                    .unwrap();
            assert!(max_abs_diff(&actual, &expected) < read_tolerance());
            let finish = |rgb| {
                super::super::scanning::finish_capture(rgb, &film, &params, &backend, &gamut)
                    .unwrap()
            };
            assert!(max_abs_diff(&finish(actual.clone()), &finish(expected)) < read_tolerance());
            let crop_grid = ReadoutGrid {
                origin_um: [8.0 * cell, 4.0 * cell],
                width: 4,
                height: 3,
                ..coarse_grid
            };
            let cropped =
                dye::apply_with_response(&source, cell * 2.0, crop_grid, &model, capture).unwrap();
            assert!(max_abs_diff_at(&actual, &cropped, 2, 1) < read_tolerance());
        }
    }

    #[test]
    fn unsupported_conditions_fail_with_actionable_errors() {
        let film = load("kodak_portra_800");
        let params = supported_params();
        validate_supported(&film, &params).unwrap();
        resolve(&film, &params).unwrap();

        // Resolved negative and reversal BW profiles use one material realization.
        for (stock, time) in [
            ("kodak_doublex", 6.5),
            ("kodak_doublex", 9.0),
            ("kodak_trix", 1.0),
        ] {
            let other = crate::profile::resolve_for_render(load(stock), Some(time));
            let mut stock_params = params.clone();
            crate::params_builder::apply_film_specifics(&mut stock_params, &other);
            let resolved = resolve(&other, &stock_params).unwrap();
            let source = ImageBuf::from_data(2, 2, vec![from_f64(0.2); 12]);
            let grid = ReadoutGrid {
                origin_um: [-0.25, 0.75],
                pitch_um: [2.125, 1.375],
                width: 2,
                height: 2,
            };
            let actual = dye::apply(&source, 2.0, grid, &resolved).unwrap();
            for pixel in actual.pixels() {
                assert_eq!(pixel[0], pixel[1], "{stock} time {time}");
                assert_eq!(pixel[0], pixel[2], "{stock} time {time}");
            }
        }

        // Still and cine color filming stocks share the same model boundary.
        for stock in [
            "kodak_portra_400",
            "kodak_ultramax_400",
            "kodak_vision3_50d",
            "kodak_vision3_250d",
            "kodak_vision3_200t",
            "kodak_vision3_500t",
        ] {
            let other = load(stock);
            let mut stock_params = params.clone();
            crate::params_builder::apply_film_specifics(&mut stock_params, &other);
            for (route, scan_film, scan_output) in [
                (PRINT_ROUTE, false, "direct_scan"),
                (FILM_SCAN_ROUTE, true, "direct_scan"),
                (FILM_SCAN_ROUTE, true, "negative_to_positive"),
            ] {
                stock_params.workflow.route = route.into();
                stock_params.io.scan_film = scan_film;
                stock_params.scanner.scan_output = scan_output.into();
                let resolved = resolve(&other, &stock_params).unwrap();
                if stock == "kodak_vision3_500t" {
                    assert_eq!(
                        resolved.channel_scale,
                        [2.25, (13.0_f64 / 6.0).powi(2), 25.0]
                    );
                }
            }
        }

        // Positive color filming stocks use the same field with the polarity
        // carried into the composite lookup axis.
        for stock in ["fujifilm_provia_100f", "fujifilm_velvia_100"] {
            let other = load(stock);
            let mut stock_params = params.clone();
            stock_params.workflow.route = PRINT_ROUTE.into();
            stock_params.io.scan_film = false;
            validate_supported(&other, &stock_params).unwrap();
            let resolved = resolve(&other, &stock_params).unwrap();
            assert!(
                resolved.positive_film,
                "{stock} must resolve positive polarity"
            );

            stock_params.workflow.route = FILM_SCAN_ROUTE.into();
            stock_params.io.scan_film = true;
            stock_params.scanner.scan_output = "direct_scan".into();
            validate_supported(&other, &stock_params).unwrap();
            assert!(resolve(&other, &stock_params).unwrap().positive_film);
        }
        // Print stocks share `type = "negative"` and `use = "still"` but are
        // not filming emulsions.
        for stock in [
            "kodak_portra_endura",
            "fujifilm_crystal_archive_typeii",
            "kodak_2302",
        ] {
            let paper = load(stock);
            let error = validate_supported(&paper, &params).unwrap_err();
            assert!(
                error.contains("unsupported condition")
                    && error.contains(stock)
                    && error.contains("printing"),
                "{error}"
            );
        }

        // Unknown record layouts are not a monochrome shortcut.
        let mut unknown = load("kodak_portra_800");
        unknown.info.channel_model = "unknown".into();
        let error = validate_supported(&unknown, &params).unwrap_err();
        assert!(
            error.contains("unsupported condition") && error.contains("unknown"),
            "{error}"
        );

        for mm in [8.0, 16.0, 24.0, 35.0, 36.0, 56.0, 65.0, 120.0, 35.5, 150.25] {
            let mut format = supported_params();
            format.camera.film_format_mm = mm;
            format.validate().unwrap();
            validate_supported(&film, &format).unwrap();
            let resolved = resolve(&film, &format).unwrap();
            assert_eq!(resolved.support_um, 3.5);
            assert_eq!(resolved.event_area_um2, 1.28);
            assert_eq!(resolved.cell_um, None);
        }

        // The diagnostic film-scan route runs for both scan outputs.
        for scan_output in ["negative_to_positive", "direct_scan"] {
            let mut scanned = supported_params();
            scanned.workflow.route = "input > film > scan".into();
            scanned.io.scan_film = true;
            scanned.scanner.scan_output = scan_output.into();
            validate_supported(&film, &scanned).unwrap();
        }

        for (route, scan_film) in [
            ("input > film > print > scan", true),
            ("input > film > scan", false),
            ("input > convert-film > scan", true),
            ("input > convert-film > scan-minus-base", true),
            ("input", false),
        ] {
            let mut routed = supported_params();
            routed.workflow.route = route.into();
            routed.io.scan_film = scan_film;
            let error = validate_supported(&film, &routed).unwrap_err();
            assert!(error.contains(route), "route {route} not named in {error}");
            assert!(
                resolve(&film, &routed).is_err(),
                "resolve must enforce the support gate"
            );
        }

        // Inactive Dye is not gated: it never renders, so nothing to reject.
        let mut inactive = supported_params();
        inactive.film_render.grain.active = false;
        inactive.workflow.route = "input".into();
        assert!(validate_supported(&film, &inactive).is_ok());
    }

    #[test]
    fn zero_capacity_components_preserve_the_expected_density() {
        for stock in ["kodak_doublex", "kodak_portra_800"] {
            let mut film = crate::profile::resolve_for_render(load(stock), None);
            film.data.density_curves_model = None;
            film.data.log_exposure = vec![0.0, 1.0];
            film.data.density_curves = vec![vec![0.0; 3], vec![1.0; 3]];
            film.data.density_curves_layers = vec![
                vec![vec![0.0; 3]; 3],
                vec![vec![0.0; 3], vec![0.25; 3], vec![0.75; 3]],
            ];
            let mut params = supported_params();
            crate::params_builder::apply_film_specifics(&mut params, &film);
            let mut resolved = resolve(&film, &params).unwrap();
            resolved.expectation = true;
            let source = ImageBuf::from_data(2, 2, vec![from_f64(0.4); 12]);
            let grid = ReadoutGrid {
                origin_um: [0.0; 2],
                pitch_um: [2.0; 2],
                width: 2,
                height: 2,
            };
            let output = dye::apply(&source, 2.0, grid, &resolved).unwrap();
            for value in output.data {
                assert!((value as f64 - 0.4).abs() < read_tolerance(), "{stock}");
            }
        }
    }

    #[test]
    fn stock_granularity_alone_sets_the_channel_scale() {
        // The reference preset keeps the configured event area.
        assert_eq!(channel_event_scale([6.0; 3]), [1.0; 3]);
        // Square law: a quarter of the granularity is a sixteenth of the area.
        assert_eq!(channel_event_scale([1.5, 3.0, 12.0]), [0.0625, 0.25, 4.0]);
        // Unset granularity resolves to the reference scale.
        assert_eq!(channel_event_scale([0.0, f64::NAN, 6.0]), [1.0; 3]);

        // Only the granularity preset separates the channels.
        let film = load("kodak_portra_800");
        let mut params = supported_params();
        params.film_render.grain.rms_granularity = [6.0, 6.0, 12.0];
        params.film_render.grain.dye_event_area_um2 = 1.28;
        let resolved = resolve(&film, &params).unwrap();
        assert_eq!(resolved.channel_scale, [1.0, 1.0, 4.0]);
        assert_eq!(resolved.event_area_um2, 1.28);
    }

    #[test]
    fn bypass_modes_never_resolve_the_field() {
        let film = load("kodak_portra_800");
        let print = load("kodak_portra_endura");
        let digest =
            |params| crate::params_builder::digest_params(params, &film, &print, None, true);

        let mut off = supported_params();
        off.film_render.grain.active = false;
        assert!(!active(&off));

        // Preview is grain-free.
        let mut preview = supported_params();
        preview.settings.preview_mode = true;
        let preview = digest(preview);
        assert!(!preview.film_render.grain.active);
        assert!(!active(&preview));

        // LUT mode promotes spatial and stochastic deactivation.
        let mut lut = supported_params();
        lut.debug.lut_mode = true;
        let lut = digest(lut);
        assert!(lut.debug.deactivate_spatial_effects);
        assert!(!active(&lut));

        let mut stochastic = supported_params();
        stochastic.debug.deactivate_stochastic_effects = true;
        let stochastic = digest(stochastic);
        assert!(!stochastic.film_render.grain.active);
        assert!(!active(&stochastic));

        // Spatial deactivation alone bypasses Dye: the field is spatial by
        // construction and the LUT bake needs a pointwise chain.
        let mut spatial = supported_params();
        spatial.debug.deactivate_spatial_effects = true;
        assert!(spatial.film_render.grain.active);
        assert!(!active(&spatial));
    }

    #[test]
    fn geometry_follows_the_requested_crop_and_upscale() {
        let source = density_target(200, 100);
        let params = supported_params();

        let full = geometry(&source, &params, 180.0).unwrap();
        assert_eq!(
            full.rect,
            SourceRect {
                x: 0,
                y: 0,
                width: 200,
                height: 100
            }
        );
        assert_eq!((full.width, full.height), (200, 100));
        assert_eq!(full.output_pitch_um, [180.0, 180.0]);

        // The rectangle `crop_and_rescale` resolves for this request.
        let mut cropped_params = supported_params();
        cropped_params.io.crop = true;
        cropped_params.io.crop_center = [0.5, 0.5];
        cropped_params.io.crop_size = [0.1, 0.1];
        cropped_params.io.upscale_factor = 2.0;
        let cropped = geometry(&source, &cropped_params, 180.0).unwrap();
        assert_eq!(
            cropped.rect,
            SourceRect {
                x: 90,
                y: 40,
                width: 20,
                height: 20
            }
        );
        assert_eq!((cropped.width, cropped.height), (40, 40));
        assert_eq!(cropped.output_pitch_um, [90.0, 90.0]);
    }

    #[test]
    fn fractional_crop_keeps_its_rounded_footprints_on_the_full_grid() {
        let source = density_target(7, 7);
        let mut params = supported_params();
        params.io.crop = true;
        params.io.crop_size = [5.0 / 7.0; 2];
        params.io.crop_center = [3.0 / 7.0; 2];
        params.io.upscale_factor = 1.5;
        // Explicitly exercise [1,6) even though source crop centering uses ties-to-even.
        let mut requested = geometry(&source, &params, 2.0).unwrap();
        requested.rect.x = 1;
        requested.rect.y = 1;
        assert_eq!((requested.width, requested.height), (8, 8));
        let (grid, keep) = requested.full_film_grid(&source).unwrap();
        assert_eq!((grid.width, grid.height), (12, 12));
        assert_eq!(grid.origin_um, [-0.5; 2]);
        assert_eq!(grid.pitch_um, [1.25; 2]);
        assert_eq!((keep.x, keep.y, keep.width, keep.height), (2, 2, 8, 8));
        let film = load("kodak_portra_800");
        let resolved = resolve(&film, &params).unwrap();
        let full = dye::apply(&source, 2.0, grid, &resolved).unwrap();
        let crop = dye::apply(&source, 2.0, requested.grid(), &resolved).unwrap();
        assert!(max_abs_diff_at(&full, &crop, keep.x, keep.y) <= read_tolerance());
    }

    #[test]
    fn crop_read_agrees_with_the_full_frame() {
        let film = load("kodak_portra_800");
        let params = supported_params();
        let resolved = resolve(&film, &params).unwrap();
        // Fractional source-to-field footprints, across two output bands.
        let pitch = 3.1;
        let source = density_target(96, 160);
        let full = dye::apply(
            &source,
            pitch,
            ReadoutGrid {
                origin_um: [0.0; 2],
                pitch_um: [pitch; 2],
                width: 96,
                height: 160,
            },
            &resolved,
        )
        .unwrap();
        let crop = dye::apply(
            &source,
            pitch,
            ReadoutGrid {
                origin_um: [16.0 * pitch, 40.0 * pitch],
                pitch_um: [pitch; 2],
                width: 32,
                height: 48,
            },
            &resolved,
        )
        .unwrap();
        let diff = max_abs_diff_at(&full, &crop, 16, 40);
        assert!(
            diff <= read_tolerance(),
            "crop read diverged from the full frame by {diff}"
        );
    }

    #[test]
    fn dye_support_is_independent_of_the_event_area() {
        let film = load("kodak_portra_800");
        let params = supported_params();
        let pitch = 3.5;
        let grid = ReadoutGrid {
            origin_um: [0.0; 2],
            pitch_um: [pitch; 2],
            width: 64,
            height: 48,
        };
        let source = density_target(64, 48);
        let coarser = resolve(&film, &params).unwrap();

        // Expected-value render: the support is fixed, only the statistics
        // scale with the event area.
        let mut expected_a = coarser.clone();
        expected_a.expectation = true;
        let mut expected_b = expected_a.clone();
        expected_b.event_area_um2 = expected_a.event_area_um2 * 10.0;
        let a = dye::apply(&source, pitch, grid, &expected_a).unwrap();
        let b = dye::apply(&source, pitch, grid, &expected_b).unwrap();
        let mean_diff = max_abs_diff(&a, &b);
        assert!(
            mean_diff <= 1e-5,
            "the dye support must not follow the event area (diff {mean_diff})"
        );

        // Realized field: the same support, different statistical strength.
        let mut realized_b = coarser.clone();
        realized_b.event_area_um2 *= 10.0;
        let ra = dye::apply(&source, pitch, grid, &coarser).unwrap();
        let rb = dye::apply(&source, pitch, grid, &realized_b).unwrap();
        let noise_diff = max_abs_diff(&ra, &rb);
        assert!(
            noise_diff > 1e-4,
            "event area must control statistical strength (diff {noise_diff})"
        );
    }

    #[test]
    fn format_scales_geometry_without_resizing_the_raster() {
        for (width, height) in [(600, 400), (400, 400), (400, 600)] {
            let source = ImageBuf::new(width, height);
            let mut params = supported_params();
            params.io.crop = true;
            params.io.crop_center = [0.6, 0.4];
            params.io.crop_size = [0.2, 0.3];
            params.io.upscale_factor = 1.25;
            let reference_pitch = crate::resizing::pixel_size_um(36.0, width, height);
            let reference = geometry(&source, &params, reference_pitch).unwrap();
            for mm in [8.0, 24.0, 35.0, 35.5, 65.0, 120.0] {
                params.camera.film_format_mm = mm;
                let pitch = crate::resizing::pixel_size_um(mm, width, height);
                // The source long edge owns the format long edge; a square
                // source ties on width.
                assert!(
                    (pitch - mm as f64 * 1000.0 / width.max(height) as f64).abs() < 1e-12,
                    "{width}x{height} at {mm} mm resolved pitch {pitch}"
                );
                let actual = geometry(&source, &params, pitch).unwrap();
                assert_eq!(actual.width, reference.width);
                assert_eq!(actual.height, reference.height);
                assert_eq!(actual.rect.x, reference.rect.x);
                assert_eq!(actual.rect.y, reference.rect.y);
                for axis in 0..2 {
                    assert!(
                        (actual.output_pitch_um[axis] / reference.output_pitch_um[axis]
                            - mm as f64 / 36.0)
                            .abs()
                            < 1e-12
                    );
                }
            }
        }
    }

    #[test]
    fn formats_keep_global_crop_and_tile_reads() {
        let film = load("kodak_portra_800");
        let target = density_target(32, 24);
        for mm in [24.0, 65.0, 120.0] {
            let mut params = supported_params();
            params.camera.film_format_mm = mm;
            let model = resolve(&film, &params).unwrap();
            // A bounded film window of a larger source, with a nonzero origin.
            let q = crate::resizing::pixel_size_um(mm, 12000, 8000);
            let full = ReadoutGrid {
                origin_um: [10.0 * q, 8.0 * q],
                pitch_um: [q; 2],
                width: 16,
                height: 8,
            };
            let all = dye::apply(&target, q, full, &model).unwrap();
            for x in [0, 8] {
                let tile = ReadoutGrid {
                    origin_um: [(10 + x) as f64 * q, 8.0 * q],
                    width: 8,
                    ..full
                };
                let part = dye::apply(&target, q, tile, &model).unwrap();
                assert!(max_abs_diff_at(&all, &part, x, 0) <= read_tolerance());
            }
            let crop = ReadoutGrid {
                origin_um: [13.0 * q, 10.0 * q],
                width: 6,
                height: 4,
                ..full
            };
            let part = dye::apply(&target, q, crop, &model).unwrap();
            assert!(max_abs_diff_at(&all, &part, 3, 2) <= read_tolerance());
        }
    }
    /// Zero strength is the deterministic limit: the realized density is the
    /// target, with no random field and no residual count noise.
    #[test]
    fn zero_dye_strength_returns_the_target() {
        use spektrafilm_model::grain::dye::texture::{ClumpParams, ReadoutGeometry, apply};
        let film = load("kodak_portra_400");
        let mut params = supported_params();
        params.film_render.grain.dye_strength = 0.0;
        let resolved = resolve(&film, &params).unwrap();
        let law = law_params(&resolved);
        assert!(law.expectation, "zero strength resolves to the mean law");
        let target = density_target(32, 24);
        let pitch = 3.5;
        let whole = ReadoutGeometry {
            origin_um: [0.0, 0.0],
            pitch_um: [pitch, pitch],
            width: 32,
            height: 24,
        };
        let out = apply(&target, pitch, &whole, &law).unwrap();
        assert!(max_abs_diff(&target, &out) <= 1e-5);

        for grid in [
            ReadoutGeometry {
                origin_um: [7.0, 3.5],
                pitch_um: [5.25, 4.75],
                width: 9,
                height: 8,
            },
            ReadoutGeometry {
                origin_um: [-3.5, -7.0],
                pitch_um: [1.75, 2.25],
                width: 13,
                height: 21,
            },
        ] {
            let mut expectation = law.clone();
            expectation.expectation = true;
            let random = apply(&target, pitch, &grid, &law).unwrap();
            let expected = apply(&target, pitch, &grid, &expectation).unwrap();
            assert!(max_abs_diff(&random, &expected) <= 1e-5);
        }
        let coarse = ReadoutGeometry {
            origin_um: [0.0, 0.0],
            pitch_um: [50_000.0, 50_000.0],
            width: 2,
            height: 2,
        };
        let out = apply(&target, 50_000.0, &coarse, &law).unwrap();
        for value in out.data.iter() {
            assert!(value.is_finite());
        }
        let mut expectation = law.clone();
        expectation.expectation = true;
        let expected = apply(&target, 50_000.0, &coarse, &expectation).unwrap();
        assert!(max_abs_diff(&out, &expected) <= 1e-5);

        let delivered = ClumpParams::delivered();
        assert_eq!(delivered.strength, [1.0, 1.0, 1.18]);
    }

    /// A tiny positive strength maps to a huge count rate. Both executors
    /// must keep the target mean instead of overflowing or capping the rate.
    #[test]
    fn tiny_dye_strength_keeps_the_target_on_both_executors() {
        use spektrafilm_backend::ComputeBackend as _;
        use spektrafilm_model::grain::dye::texture::{ReadoutGeometry, apply};
        let film = load("kodak_portra_400");
        let mut params = supported_params();
        params.film_render.grain.dye_strength = 1e-5;
        let resolved = resolve(&film, &params).unwrap();
        let law = law_params(&resolved);
        assert!(!law.expectation, "a positive strength stays stochastic");
        let target = density_target(32, 24);
        let pitch = 3.5;
        let grid = ReadoutGrid {
            origin_um: [0.0, 0.0],
            pitch_um: [pitch, pitch],
            width: 32,
            height: 24,
        };
        let cpu = apply(
            &target,
            pitch,
            &ReadoutGeometry {
                origin_um: grid.origin_um,
                pitch_um: grid.pitch_um,
                width: grid.width,
                height: grid.height,
            },
            &law,
        )
        .unwrap();
        assert!(max_abs_diff(&target, &cpu) <= 1e-4);

        let backend = spektrafilm_gpu::test_support::adapter();
        let device = device_params(&resolved, pitch, grid).unwrap();
        let gpu = backend
            .dye_field(&target, &device)
            .expect("device field path")
            .expect("device field runs");
        assert!(max_abs_diff(&target, &gpu) <= 1e-3);
        assert!(max_abs_diff(&cpu, &gpu) <= 1e-3);
    }

    /// Stronger look strength raises the realized standard deviation while
    /// the density mean stays at the target; weaker strength lowers it.
    #[test]
    fn dye_strength_scales_the_realized_amplitude() {
        use spektrafilm_model::grain::dye::texture::{ReadoutGeometry, apply};
        let film = load("kodak_portra_400");
        let target = {
            let mut image = ImageBuf::new(48, 48);
            image.data.iter_mut().for_each(|v| *v = from_f64(0.62));
            image
        };
        let pitch = 3.5;
        let grid = ReadoutGeometry {
            origin_um: [0.0, 0.0],
            pitch_um: [pitch, pitch],
            width: 48,
            height: 48,
        };
        let moments = |strength: f64| {
            let mut params = supported_params();
            params.film_render.grain.dye_strength = strength;
            let resolved = resolve(&film, &params).unwrap();
            let out = apply(&target, pitch, &grid, &law_params(&resolved)).unwrap();
            let values: Vec<f64> = out.data.chunks(3).map(|p| p[1] as f64).collect();
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
            (mean, var.sqrt())
        };
        let (weak_mean, weak) = moments(0.5);
        let (base_mean, base) = moments(1.0);
        let (strong_mean, strong) = moments(2.0);
        for mean in [weak_mean, base_mean, strong_mean] {
            assert!((mean - 0.62).abs() < 5e-3, "mean {mean} left the target");
        }
        assert!(weak < 0.8 * base, "weak {weak} vs base {base}");
        assert!(strong > 1.2 * base, "strong {strong} vs base {base}");
    }

    /// The scale control moves the texture's correlation length.
    #[test]
    fn dye_scale_changes_the_texture_correlation_length() {
        use spektrafilm_model::grain::dye::texture::{ReadoutGeometry, apply};
        let film = load("kodak_portra_400");
        let target = {
            let mut image = ImageBuf::new(64, 64);
            image.data.iter_mut().for_each(|v| *v = from_f64(0.55));
            image
        };
        let pitch = 3.5;
        let grid = ReadoutGeometry {
            origin_um: [0.0, 0.0],
            pitch_um: [pitch, pitch],
            width: 64,
            height: 64,
        };
        let adjacent_energy = |scale: f64| {
            let mut params = supported_params();
            params.film_render.grain.dye_scale = scale;
            let resolved = resolve(&film, &params).unwrap();
            let out = apply(&target, pitch, &grid, &law_params(&resolved)).unwrap();
            let mut sum = 0.0;
            let mut count = 0usize;
            for y in 0..64usize {
                for x in 0..63usize {
                    let a = out.data[(y * 64 + x) * 3 + 1] as f64;
                    let b = out.data[(y * 64 + x + 1) * 3 + 1] as f64;
                    sum += (a - b).abs();
                    count += 1;
                }
            }
            sum / count as f64
        };
        let fine = adjacent_energy(0.5);
        let reference = adjacent_energy(1.0);
        let coarse = adjacent_energy(2.0);
        assert!(
            fine > reference && reference > coarse,
            "adjacent energy must fall as the scale coarsens: {fine} > {reference} > {coarse}"
        );
    }

    /// The device realizes the mapped non-default look law with the same
    /// amplitude and mean as the CPU reference.
    #[test]
    fn device_field_realizes_the_mapped_look_law() {
        use spektrafilm_backend::ComputeBackend as _;
        use spektrafilm_model::grain::dye::texture::{ReadoutGeometry, apply};
        let backend = spektrafilm_gpu::test_support::adapter();
        let film = load("kodak_portra_800");
        let mut params = supported_params();
        params.film_render.grain.dye_strength = 1.6;
        params.film_render.grain.dye_scale = 1.5;
        let resolved = resolve(&film, &params).unwrap();
        let source = density_target(64, 48);
        let pitch = 3.5;
        let grid = ReadoutGrid {
            origin_um: [0.0, 0.0],
            pitch_um: [pitch, pitch],
            width: 64,
            height: 48,
        };
        let device = device_params(&resolved, pitch, grid).unwrap();
        let gpu = backend
            .dye_field(&source, &device)
            .expect("device field path")
            .expect("device field runs");
        let cpu = apply(
            &source,
            pitch,
            &ReadoutGeometry {
                origin_um: grid.origin_um,
                pitch_um: grid.pitch_um,
                width: grid.width,
                height: grid.height,
            },
            &law_params(&resolved),
        )
        .unwrap();
        let moments = |image: &ImageBuf| {
            let values: Vec<f64> = image.data.chunks(3).map(|p| p[1] as f64).collect();
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
            (mean, var.sqrt())
        };
        let (gpu_mean, gpu_sd) = moments(&gpu);
        let (cpu_mean, cpu_sd) = moments(&cpu);
        assert!(
            (gpu_mean - cpu_mean).abs() <= 0.02 * cpu_mean.abs(),
            "mean: device {gpu_mean:.5}, host {cpu_mean:.5}"
        );
        let ratio = gpu_sd.max(1e-12) / cpu_sd.max(1e-12);
        assert!(
            (ratio - 1.0).abs() <= 0.15,
            "amplitude: device is {ratio:.3} of the host"
        );
    }
}
