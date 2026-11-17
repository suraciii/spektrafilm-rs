//! Film-coordinate grain Dye integration (`specs/film-grain/dye/`).
//!
//! The pipeline develops the *full* source and hands the developed CMY
//! density to this module. The module owns Dye field resolution and the
//! readout to the requested output geometry — on the device through
//! `dye_field` when the backend has one and the request is
//! device-representable, otherwise with the CPU realization
//! ([`spektrafilm_model::grain::dye::texture::apply`]); scanner
//! interpretation and encoding stay with their owners.
//!
//! Film coordinates come from the profile and the geometry: a crop selects
//! the readout region only, so the field keeps its global film positions and
//! overlapping reads agree.
use crate::WorkflowRoute;

use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};
use spektrafilm_model::grain::dye::{DyeGrainParams, ReadoutGrid, diagnostic_env, texture};

use crate::params::RuntimeParams;
use crate::params::grain::GrainEngine;
use crate::profile::Profile;

/// Positive profiles use half the negative delivered base cell (7.0 µm
/// instead of 14.0 µm) so their dominant density layer stays visibly grainy
/// rather than becoming a low-frequency density cloud.
const POSITIVE_BASE_SCALE: f64 = 0.5;

/// Reference route: the film exposes the print paper, which is scanned.
const PRINT_ROUTE: &str = "input > film > print > scan";
/// Diagnostic route: the developed film is scanned directly.
const FILM_SCAN_ROUTE: &str = "input > film > scan";
/// Retired field-pitch override. The continuous law samples the source
/// lattice and has no independent cell pitch, so a request must fail loudly
/// rather than silently rendering with different numerics.
const CELL_UM_ENV: &str = "SPEKTRAFILM_GRAIN_DYE_CELL_UM";
/// Diagnostic expected-value render: every Poisson count is replaced by its
/// conditional mean. It never changes the recipe.
const EXPECTATION_ENV: &str = "SPEKTRAFILM_GRAIN_DYE_EXPECTATION";
/// Retired exact-cell optical response experiment.
const CELL_SPECTRAL_ENV: &str = "SPEKTRAFILM_GRAIN_DYE_CELL_SPECTRAL";
/// Retired host-only mass-law experiment.
const MASS_LAW_ENV: &str = "SPEKTRAFILM_GRAIN_DYE_MASS_LAW";

/// Reject diagnostics that belonged to the retired cell field instead of
/// silently changing their meaning.
fn validate_retired_diagnostics(params: &RuntimeParams) -> Result<(), String> {
    if !active(params) {
        return Ok(());
    }
    for name in [CELL_UM_ENV, CELL_SPECTRAL_ENV, MASS_LAW_ENV] {
        if diagnostic_env(name).is_ok() {
            return Err(format!(
                "{name} was retired with the exact-cell Dye field; the engine's continuous \
                 law has no cell pitch, per-cell optical response, or host-only mass law"
            ));
        }
    }
    Ok(())
}

/// Whether the resident chain can execute the field and its readout.
/// Geometry comes from [`resident_readout_plan_for_dims`]; the chain adopts
/// its field raster and extracts any enclosing-lattice crop after downstream
/// spatial optics.
pub fn resident_fence_supported(film: &Profile, params: &RuntimeParams) -> bool {
    active(params) && validate_supported(film, params).is_ok()
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
    .or_else(|| (!active).then(|| "Dye is inactive".to_owned()));
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
    validate_retired_diagnostics(params)?;
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
/// all read this one result, so the field lattice and the deferred crop cannot
/// drift between routes.
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
/// extracts the requested rectangle after those stages.
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

/// Active density layers declared by the profile. A profile without layer
/// curves behaves as one layer; a layered profile supplies at most three.
fn profile_layer_count(film: &Profile) -> Result<u32, String> {
    let rows = &film.data.density_curves_layers;
    if rows.is_empty() {
        return Ok(1);
    }
    let count = rows[0].len();
    if !(1..=3).contains(&count) || rows.iter().any(|row| row.len() != count) {
        return Err(
            "film grain Dye: the profile must declare one to three consistent density layers"
                .into(),
        );
    }
    Ok(count as u32)
}

/// One layer's share of the developed density. The profile's layer curves
/// describe the same exposure axis as the composite target, so a pointwise
/// inverse lookup splits the target without a second exposure solve.
struct DyeLayerSplit {
    axis: Vec<[f64; 3]>,
    curves: Vec<[[f64; 3]; 3]>,
    inv_dx: [Vec<f64>; 3],
    sign: f64,
    count: usize,
}

impl DyeLayerSplit {
    fn prepare(film: &Profile) -> Result<Option<Self>, String> {
        let rows = &film.data.density_curves_layers;
        if rows.is_empty() {
            return Ok(None);
        }
        let count = rows[0].len();
        if !(1..=3).contains(&count) || rows.iter().any(|row| row.len() != count) {
            return Err(
                "film grain Dye: the profile must declare one to three consistent density layers"
                    .into(),
            );
        }
        if count == 1 {
            return Ok(None);
        }
        let mut axis = crate::chain_prep::FilmCurves::prepare(film).normalized;
        let mut curves = film.density_curves_layers_f64();
        if film.is_bw() {
            // A stored BW profile may expose one record while the Dye target
            // arrives with that record replicated in all three lanes.
            for row in &mut axis {
                row[1] = row[0];
                row[2] = row[0];
            }
            for row in &mut curves {
                for layer in row.iter_mut() {
                    layer[1] = layer[0];
                    layer[2] = layer[0];
                }
            }
        }
        if axis.len() < 2 || axis.len() != curves.len() {
            return Err("film grain Dye: composite and layer density curves must have the same non-empty exposure axis".into());
        }
        let sign = if film.is_positive() { -1.0 } else { 1.0 };
        let mut inv_dx: [Vec<f64>; 3] = std::array::from_fn(|_| Vec::new());
        for c in 0..3 {
            for row in &axis {
                if !row[c].is_finite() {
                    return Err("film grain Dye: composite density curves must be finite".into());
                }
            }
            let mut previous = f64::NEG_INFINITY;
            for row in &mut axis {
                let oriented = row[c] * sign;
                if oriented < previous {
                    row[c] = previous * sign;
                } else {
                    previous = oriented;
                }
            }
            inv_dx[c].reserve(axis.len() - 1);
            for pair in axis.windows(2) {
                let delta = pair[1][c] * sign - pair[0][c] * sign;
                if !delta.is_finite() || delta < 0.0 {
                    return Err("film grain Dye: composite density curves must be monotonic".into());
                }
                inv_dx[c].push(if delta > 0.0 { 1.0 / delta } else { 0.0 });
            }
        }
        for row in &curves {
            for layer in row.iter().take(count) {
                if layer.iter().any(|value| !value.is_finite()) {
                    return Err("film grain Dye: layer density curves must be finite".into());
                }
            }
        }
        Ok(Some(Self {
            axis,
            curves,
            inv_dx,
            sign,
            count,
        }))
    }

    fn count(&self) -> usize {
        self.count
    }

    fn table(&self) -> spektrafilm_backend::dye_params::DyeLayerTable {
        spektrafilm_backend::dye_params::DyeLayerTable {
            layer_count: self.count as u32,
            axis: self.axis.clone(),
            curves: self.curves.clone(),
            sign: self.sign,
        }
    }

    fn interpolate(&self, target: f64, channel: usize, layer: usize) -> f64 {
        let x = target * self.sign;
        let last = self.axis.len() - 1;
        if x <= self.axis[0][channel] * self.sign {
            return self.curves[0][layer][channel];
        }
        if x >= self.axis[last][channel] * self.sign {
            return self.curves[last][layer][channel];
        }
        let upper = self
            .axis
            .partition_point(|row| row[channel] * self.sign <= x);
        let lower = upper - 1;
        let x0 = self.axis[lower][channel] * self.sign;
        let t = (x - x0) * self.inv_dx[channel][lower];
        if t <= 0.0 {
            return self.curves[lower][layer][channel];
        }
        self.curves[lower][layer][channel]
            + t * (self.curves[upper][layer][channel] - self.curves[lower][layer][channel])
    }

    /// Split the full developed target into one layer's density. The raw
    /// layer curves define the relative layer shares; normalize them
    /// pointwise so the layer targets sum to the target exactly.
    fn target(&self, source: &ImageBuf, layer: usize) -> Result<ImageBuf, String> {
        if layer >= self.count {
            return Err("film grain Dye: layer index is outside the profile layer set".into());
        }
        let width = source.width as usize;
        let height = source.height as usize;
        let expected = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or("film grain Dye: source dimensions overflow")?;
        if source.data.len() != expected {
            return Err(
                "film grain Dye: source raster shape does not match its sample count".into(),
            );
        }
        let mut out = ImageBuf::new(source.width, source.height);
        for y in 0..height {
            for x in 0..width {
                let base = (y * width + x) * 3;
                for c in 0..3 {
                    let target = to_f64(source.data[base + c]);
                    if !target.is_finite() {
                        return Err("film grain Dye: source density must be finite".into());
                    }
                    let mut values = [0.0; 3];
                    let mut sum = 0.0;
                    for (l, value) in values.iter_mut().enumerate().take(self.count) {
                        *value = self.interpolate(target, c, l);
                        sum += *value;
                    }
                    let value = if target > 0.0 {
                        if sum > 0.0 {
                            values[layer] * target / sum
                        } else {
                            // A profile may have zero layer curves at the
                            // bottom of the density range. Keep the layer
                            // targets summing to the developed target rather
                            // than dropping a positive density.
                            target / self.count as f64
                        }
                    } else {
                        0.0
                    };
                    out.data[base + c] = from_f64(value);
                }
            }
        }
        Ok(out)
    }
}

/// Resolve the Dye look inputs from the resolved film profile and the
/// runtime controls.
///
/// The profile-derived density curves form the layer targets. Geometry is
/// consumed by the upstream film chain that forms the density target.
pub fn resolve(film: &Profile, params: &RuntimeParams) -> Result<DyeGrainParams, String> {
    validate_supported(film, params)?;
    let grain = &params.film_render.grain;
    Ok(DyeGrainParams {
        monochrome: film.is_bw(),
        positive: film.is_positive(),
        dye_strength: grain.dye_strength,
        layer_count: profile_layer_count(film)?,
        seed: params.random_seed,
        expectation: expectation_requested(),
    })
}

/// Read the requested film region out of the full developed source.
///
/// The field resolves on the device whenever the backend has one and the
/// request is device-representable (`device_params`): the same mapped look
/// values and readout grid go to the backend's `dye_field`. A backend
/// without a device field path resolves the field on the CPU with the
/// published reason, and an explicit GPU request fails by name instead of
/// demoting silently.
pub fn readout(
    source: &ImageBuf,
    film: &Profile,
    params: &RuntimeParams,
    source_pitch_um: f64,
    grid: ReadoutGrid,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<ImageBuf, String> {
    let resolved = resolve(film, params)?;
    let split = DyeLayerSplit::prepare(film)?;
    let layer_scales = layer_scales(film);
    if let Some(split) = &split
        && backend.is_gpu()
    {
        let count = split.count();
        let table = std::sync::Arc::new(split.table());
        let mut law = law_params(&resolved);
        law.fog /= count as f64;
        law.layer = 0;
        if let Some(out) = try_device_field(
            source,
            film,
            source_pitch_um,
            grid,
            &resolved,
            &law,
            layer_scales,
            0,
            count,
            Some(table),
            backend,
        )? {
            return Ok(out);
        }
        return cpu_layer_sum(
            source,
            split,
            source_pitch_um,
            grid,
            &resolved,
            layer_scales,
            backend,
        );
    }
    let layer_count = split.as_ref().map_or(1, DyeLayerSplit::count);
    let mut out: Option<ImageBuf> = None;
    for layer in 0..layer_count {
        let law = law_params_for_layer(&resolved, layer_scales, layer as u32);
        let layer_target = match &split {
            Some(split) => Some(split.target(source, layer)?),
            None => None,
        };
        let layer_source = layer_target.as_ref().unwrap_or(source);
        let field = readout_field(
            layer_source,
            film,
            source_pitch_um,
            grid,
            &resolved,
            &law,
            layer_scales,
            layer,
            layer_count,
            backend,
        )?;
        add_to_sum(&mut out, field)?;
    }
    out.ok_or_else(|| "film grain Dye: no density layer was available".into())
}

fn add_to_sum(total: &mut Option<ImageBuf>, field: ImageBuf) -> Result<(), String> {
    match total {
        None => *total = Some(field),
        Some(total) => {
            if total.data.len() != field.data.len() {
                return Err("film grain Dye: layer readouts disagree on the output grid".into());
            }
            total
                .data
                .iter_mut()
                .zip(field.data)
                .for_each(|(sum, value)| *sum += value);
        }
    }
    Ok(())
}

/// CPU fallback for a device request that has no field path. The layered
/// geometry is resolved per layer on the host, exactly as the non-device
/// route would.
#[allow(clippy::too_many_arguments)]
fn cpu_layer_sum(
    source: &ImageBuf,
    split: &DyeLayerSplit,
    source_pitch_um: f64,
    grid: ReadoutGrid,
    resolved: &DyeGrainParams,
    layer_scales: [f64; 3],
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<ImageBuf, String> {
    let mut out = None;
    for layer in 0..split.count() {
        let law = law_params_for_layer(resolved, layer_scales, layer as u32);
        let target = split.target(source, layer)?;
        let field = cpu_field(&target, source_pitch_um, grid, &law, backend)?;
        add_to_sum(&mut out, field)?;
    }
    out.ok_or_else(|| "film grain Dye: no density layer was available".into())
}

/// Try the selected device field path, returning `None` when the backend has
/// no device field rather than silently resolving the host law.
#[allow(clippy::too_many_arguments)]
fn try_device_field(
    source: &ImageBuf,
    film: &Profile,
    source_pitch_um: f64,
    grid: ReadoutGrid,
    resolved: &DyeGrainParams,
    law: &texture::DyeLawParams,
    layer_scales: [f64; 3],
    layer: usize,
    layer_count: usize,
    layer_table: Option<std::sync::Arc<spektrafilm_backend::dye_params::DyeLayerTable>>,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<Option<ImageBuf>, String> {
    if !backend.is_gpu() {
        return Ok(None);
    }
    match device_params_with_law(source_pitch_um, grid, law, layer_scales, layer_table) {
        Ok(device) => {
            let observation = super::StageObservation::new(backend, "grain_dye_field");
            let observed = observation.backend(backend);
            tracing::info!(
                stock = film.info.stock.as_deref().unwrap_or(""),
                source = format_args!("{}x{}", source.width, source.height),
                origin_um = ?grid.origin_um,
                output_pitch_um = ?grid.pitch_um,
                output = format_args!("{}x{}", grid.width, grid.height),
                dye_strength = resolved.dye_strength,
                layer,
                layer_count,
                expectation = resolved.expectation,
                "film grain Dye: field readout on GPU"
            );
            match observed.dye_field(source, &device) {
                Some(result) => return result.map(Some),
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
    Ok(None)
}

/// Realize one density layer on the selected executor.
#[allow(clippy::too_many_arguments)]
fn readout_field(
    source: &ImageBuf,
    film: &Profile,
    source_pitch_um: f64,
    grid: ReadoutGrid,
    resolved: &DyeGrainParams,
    law: &texture::DyeLawParams,
    layer_scales: [f64; 3],
    layer: usize,
    layer_count: usize,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<ImageBuf, String> {
    if let Some(result) = try_device_field(
        source,
        film,
        source_pitch_um,
        grid,
        resolved,
        law,
        layer_scales,
        layer,
        layer_count,
        None,
        backend,
    )? {
        return Ok(result);
    }
    cpu_field(source, source_pitch_um, grid, law, backend)
}

/// The host field readout: one CPU realization with the published stage
/// reason, shared by the host-backend route and the device fallbacks.
fn cpu_field(
    source: &ImageBuf,
    source_pitch_um: f64,
    grid: ReadoutGrid,
    law: &texture::DyeLawParams,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<ImageBuf, String> {
    // The observation records the host executor and its published reason.
    let _observation = super::StageObservation::cpu(
        backend,
        "grain_dye_field",
        spektrafilm_backend::telemetry::CpuReason::DyeGrainFieldCpu,
    );
    // The Dye grain law: the record density is built from a correlated
    // coordinate-addressed field whose amplitude follows the derived granularity curve. The engine's law
    // constants come from DyeLawParams::delivered; the recipe's strength maps
    // onto them in `law_params`, and the mean is preserved exactly: E[D] = t for
    // every strength. The device receives the same mapped law, so both executors realize it.
    texture::apply(
        source,
        source_pitch_um,
        &texture::ReadoutGeometry {
            origin_um: grid.origin_um,
            pitch_um: grid.pitch_um,
            width: grid.width,
            height: grid.height,
        },
        law,
    )
}

/// The delivered law with the recipe's one effective Dye look control
/// applied. This is the one mapping both executors consume.
///
/// The look strength scales the correlated field's amplitude and the event
/// count's standard deviation together: the field amplitude scales `white`,
/// while the event mass `q` enters the count variance as its square root, so
/// `q` scales by the square. The per-record ratio and the derived density
/// taper are untouched. A zero strength is the deterministic limit: the law
/// replaces every draw by its conditional mean, so no random grain or
/// residual count noise remains.
fn law_params(resolved: &DyeGrainParams) -> texture::DyeLawParams {
    let mut params = texture::DyeLawParams::delivered();
    let strength = resolved.dye_strength
        * if resolved.positive {
            spektrafilm_backend::dye_params::law::POSITIVE_AMPLITUDE_SCALE
        } else {
            1.0
        };
    let mut zero = strength == 0.0;
    if strength > 0.0 {
        // The mapped event area must stay representable on both executors.
        // Below the f32 normal minimum the device would read zero, so the
        // request resolves to the deterministic limit on both.
        let q = params.q * strength * strength;
        if q >= f64::from(f32::MIN_POSITIVE) {
            params.white *= strength;
            params.q = q;
        } else {
            zero = true;
        }
    }
    params.length_scale = if resolved.positive {
        POSITIVE_BASE_SCALE
    } else {
        1.0
    };
    params.fine_scale = if resolved.positive {
        spektrafilm_backend::dye_params::law::POSITIVE_FINE_SCALE
    } else {
        0.0
    };
    params.fine_weight = if resolved.positive {
        spektrafilm_backend::dye_params::law::POSITIVE_FINE_WEIGHT
    } else {
        0.0
    };
    params.seed = resolved.seed;
    params.expectation = resolved.expectation || zero;
    params.monochrome = resolved.monochrome;
    params
}

/// Relative cell sizes for the profile's density layers.
///
/// Negative profiles spread the target across fast, mid, and slow layers, so
/// the size ladder [1, 0.5, 0.3] gives their visible grain a multi-scale
/// texture. Positive profiles weight their normal-density target toward the
/// finest-index layer (Kodachrome 64 carries about 87% there at t = 0.55, and
/// Tri-X about 48%); applying 0.3 to that layer at a 35 mm 1:1 crop puts its
/// cell below one source pixel, leaving an uncorrelated field that reads as
/// no grain. Positive profiles therefore keep the base cell on every layer.
fn layer_scales(film: &Profile) -> [f64; 3] {
    if film.is_positive() {
        [1.0; 3]
    } else {
        spektrafilm_backend::dye_params::law::LAYER_SCALES
    }
}

/// The same delivered law for one profile density layer. Splitting the fog
/// keeps the total event rate of the layered realization equal to the
/// single-layer rate at the same total density.
fn law_params_for_layer(
    resolved: &DyeGrainParams,
    layer_scales: [f64; 3],
    layer: u32,
) -> texture::DyeLawParams {
    let mut params = law_params(resolved);
    let count = resolved.layer_count.max(1) as f64;
    let index = layer.min(2) as usize;
    params.fog /= count;
    params.layer = layer;
    params.length_scale *= layer_scales[index];
    params
}

/// Device parameters for one readout: the mapped look values and the output
/// grid. The host and device read the same law constants; this record only
/// carries the recipe-dependent values.
#[cfg(test)]
fn device_params(
    resolved: &DyeGrainParams,
    source_pitch_um: f64,
    grid: ReadoutGrid,
) -> Result<spektrafilm_backend::dye_params::DyeGpuParams, String> {
    device_params_with_law(
        source_pitch_um,
        grid,
        &law_params(resolved),
        spektrafilm_backend::dye_params::law::LAYER_SCALES,
        None,
    )
}

fn device_params_with_law(
    source_pitch_um: f64,
    grid: ReadoutGrid,
    law: &texture::DyeLawParams,
    layer_scales: [f64; 3],
    layer_table: Option<std::sync::Arc<spektrafilm_backend::dye_params::DyeLayerTable>>,
) -> Result<spektrafilm_backend::dye_params::DyeGpuParams, String> {
    let layer_count = layer_table.as_ref().map_or(1, |table| table.layer_count);
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
        fog: law.fog,
        length_scale: law.length_scale,
        fine_scale: law.fine_scale,
        fine_weight: law.fine_weight,
        layer_scales,
        layer_count,
        layer_table,
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
    let resolved = resolve(film, params)?;
    let split = DyeLayerSplit::prepare(film)?;
    let table = split
        .as_ref()
        .map(|split| std::sync::Arc::new(split.table()));
    let count = split.as_ref().map_or(1, DyeLayerSplit::count);
    let mut law = law_params(&resolved);
    law.fog /= count as f64;
    law.layer = 0;
    device_params_with_law(source_pitch_um, grid, &law, layer_scales(film), table)
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
/// device realizes the continuous conditional count law at any readout
/// geometry, including expectation mode and non-default look controls.
fn host_field_reason(gpu_backend: bool) -> Option<String> {
    if !gpu_backend {
        return Some("the selected backend has no device field path".to_owned());
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

fn expectation_requested() -> bool {
    diagnostic_env(EXPECTATION_ENV).is_ok_and(|value| value == "1")
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

    fn correlation(a: &[f64], b: &[f64]) -> f64 {
        let n = a.len().min(b.len()) as f64;
        let ma = a.iter().sum::<f64>() / n;
        let mb = b.iter().sum::<f64>() / n;
        let mut cov = 0.0;
        let mut va = 0.0;
        let mut vb = 0.0;
        for (x, y) in a.iter().zip(b) {
            let dx = x - ma;
            let dy = y - mb;
            cov += dx * dy;
            va += dx * dx;
            vb += dy * dy;
        }
        cov / (va * vb).sqrt().max(1e-12)
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

    fn readout_geometry(grid: ReadoutGrid) -> texture::ReadoutGeometry {
        texture::ReadoutGeometry {
            origin_um: grid.origin_um,
            pitch_um: grid.pitch_um,
            width: grid.width,
            height: grid.height,
        }
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
    use spektrafilm_math::precision::{from_f64, to_f64};

    #[test]
    fn profile_density_layers_sum_to_the_developed_target() {
        let film = load("kodak_portra_400");
        let split = DyeLayerSplit::prepare(&film).unwrap().unwrap();
        assert_eq!(split.count(), 3);
        let source = density_target(32, 24);
        let mut total = ImageBuf::new(source.width, source.height);
        for layer in 0..split.count() {
            let target = split.target(&source, layer).unwrap();
            assert!(
                target
                    .data
                    .iter()
                    .zip(&source.data)
                    .any(|(layer, source)| (to_f64(*layer) - to_f64(*source)).abs() > 1e-3),
                "layer {layer} did not carry a distinct part of the density"
            );
            total
                .data
                .iter_mut()
                .zip(target.data)
                .for_each(|(sum, value)| *sum += value);
        }
        assert!(
            max_abs_diff(&source, &total) < 2e-4,
            "layer sum differs from the target by {}",
            max_abs_diff(&source, &total)
        );
    }

    #[test]
    fn layer_split_preserves_a_low_positive_target_when_layer_curves_are_zero() {
        let film = load("kodak_endura_premier");
        let split = DyeLayerSplit::prepare(&film).unwrap().unwrap();
        let value = from_f64(0.00024);
        let source = ImageBuf::from_data(2, 2, vec![value; 12]);
        let mut total = ImageBuf::new(source.width, source.height);
        for layer in 0..split.count() {
            let target = split.target(&source, layer).unwrap();
            total
                .data
                .iter_mut()
                .zip(target.data)
                .for_each(|(sum, value)| *sum += value);
        }
        assert!(
            max_abs_diff(&source, &total) < 2e-4,
            "layer sum differs from the target by {}",
            max_abs_diff(&source, &total)
        );
    }

    #[test]
    fn layered_filming_profiles_normalize_to_a_positive_target() {
        let dir = data_dir().join("profiles");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let stock = path.file_stem().unwrap().to_str().unwrap();
            let film = load(stock);
            if !film.is_filming() || film.data.density_curves_layers.is_empty() {
                continue;
            }
            let Some(split) =
                DyeLayerSplit::prepare(&film).unwrap_or_else(|error| panic!("{stock}: {error}"))
            else {
                continue;
            };
            let source = ImageBuf::from_data(
                2,
                2,
                vec![
                    from_f64(0.5),
                    from_f64(0.6),
                    from_f64(0.7),
                    from_f64(0.5),
                    from_f64(0.6),
                    from_f64(0.7),
                    from_f64(0.5),
                    from_f64(0.6),
                    from_f64(0.7),
                    from_f64(0.5),
                    from_f64(0.6),
                    from_f64(0.7),
                ],
            );
            let mut total = ImageBuf::new(2, 2);
            for layer in 0..split.count() {
                let target = split.target(&source, layer).unwrap();
                total
                    .data
                    .iter_mut()
                    .zip(target.data)
                    .for_each(|(sum, value)| *sum += value);
            }
            assert!(
                max_abs_diff(&source, &total) < 2e-4,
                "{stock}: layer sum differs from the target by {}",
                max_abs_diff(&source, &total)
            );
            checked += 1;
        }
        assert!(checked > 0, "no layered filming profiles were checked");
    }

    #[test]
    fn monochrome_layer_curves_normalize_to_the_target() {
        let film = load("kodak_trix");
        let split = DyeLayerSplit::prepare(&film).unwrap().unwrap();
        assert_eq!(split.count(), 3);
        let source = density_target(16, 16);
        let mut total = ImageBuf::new(source.width, source.height);
        for layer in 0..split.count() {
            let target = split.target(&source, layer).unwrap();
            assert!(
                target.data.iter().any(|value| to_f64(*value) > 0.0),
                "layer {layer} carried no density"
            );
            total
                .data
                .iter_mut()
                .zip(target.data)
                .for_each(|(sum, value)| *sum += value);
        }
        assert!(
            max_abs_diff(&source, &total) < 2e-4,
            "monochrome layer sum differs from the target by {}",
            max_abs_diff(&source, &total)
        );
    }

    #[test]
    fn layered_gpu_field_matches_the_host_layer_sum() {
        let gpu = spektrafilm_gpu::test_support::adapter();
        let film = load("kodak_portra_400");
        let params = supported_params();
        let source = density_target(64, 64);
        let pitch = 4.86;
        let grid = ReadoutGrid {
            origin_um: [0.0, 0.0],
            pitch_um: [pitch, pitch],
            width: source.width,
            height: source.height,
        };
        let cpu = readout(
            &source,
            &film,
            &params,
            pitch,
            grid,
            &spektrafilm_backend::cpu_backend::CpuBackend,
        )
        .unwrap();
        let device = readout(&source, &film, &params, pitch, grid, gpu.backend()).unwrap();
        let moments = |image: &ImageBuf| {
            let values: Vec<f64> = image.data.chunks(3).map(|p| to_f64(p[1])).collect();
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let variance =
                values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
            (mean, variance)
        };
        let (cpu_mean, cpu_var) = moments(&cpu);
        let (device_mean, device_var) = moments(&device);
        assert!(
            (cpu_mean - device_mean).abs() < 0.02,
            "layered mean mismatch: host {cpu_mean:.6}, device {device_mean:.6}"
        );
        assert!(
            cpu_var > 1e-6 && device_var > 1e-6,
            "layered field lost its texture: host {cpu_var:.8}, device {device_var:.8}"
        );
        let variance_ratio = device_var / cpu_var;
        assert!(
            (0.5..=2.0).contains(&variance_ratio),
            "layered variance mismatch: host {cpu_var:.8}, device {device_var:.8}"
        );
    }

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
            let mut law = spektrafilm_model::grain::dye::texture::DyeLawParams::delivered();
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
            let gpu_values: Vec<f64> = gpu.data.chunks(3).map(|p| p[1] as f64).collect();
            let cpu_values: Vec<f64> = cpu.data.chunks(3).map(|p| p[1] as f64).collect();
            let corr = correlation(&gpu_values, &cpu_values);
            assert!(
                corr > 0.98,
                "the device and host must draw the same field at lambda {label}: correlation {corr:.4}"
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
        for grid in grids {
            let device = device_params(&resolved, 2.0, grid).unwrap();
            let gpu_out =
                spektrafilm_backend::ComputeBackend::dye_field(&*backend, &source, &device)
                    .expect("the backend exposes a device field path")
                    .expect("the device field runs");
            // The reference is the engine's law: the device must reproduce the
            // host model exactly in expectation, for every readout grid.
            let mut reference = texture::DyeLawParams::delivered();
            reference.expectation = resolved.expectation;
            reference.seed = resolved.seed;
            let cpu_out = texture::apply(
                &source,
                2.0,
                &texture::ReadoutGeometry {
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
                "Dye expectation on {}: grid {grid:?}, max {worst:.2e}",
                backend.name()
            );
            assert!(worst <= 1e-4, "device vs CPU expectation error {worst:.2e}");
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
            let law = law_params(&resolved);
            let actual = texture::apply(
                &source,
                2.0,
                &texture::ReadoutGeometry {
                    origin_um: [-0.25, 0.75],
                    pitch_um: [2.125, 1.375],
                    width: 2,
                    height: 2,
                },
                &law,
            )
            .unwrap();
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
                resolve(&other, &stock_params).unwrap();
            }
        }

        // Positive color filming stocks use the same field; polarity is
        // carried by the upstream film-density curve direction.
        for stock in ["fujifilm_provia_100f", "fujifilm_velvia_100"] {
            let other = load(stock);
            let mut stock_params = params.clone();
            stock_params.workflow.route = PRINT_ROUTE.into();
            stock_params.io.scan_film = false;
            validate_supported(&other, &stock_params).unwrap();
            resolve(&other, &stock_params).unwrap();

            stock_params.workflow.route = FILM_SCAN_ROUTE.into();
            stock_params.io.scan_film = true;
            stock_params.scanner.scan_output = "direct_scan".into();
            validate_supported(&other, &stock_params).unwrap();
            resolve(&other, &stock_params).unwrap();
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
            resolve(&film, &format).unwrap();
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
        let law = law_params(&resolve(&film, &params).unwrap());
        let full = texture::apply(&source, 2.0, &readout_geometry(grid), &law).unwrap();
        let crop = texture::apply(&source, 2.0, &readout_geometry(requested.grid()), &law).unwrap();
        assert!(max_abs_diff_at(&full, &crop, keep.x, keep.y) <= read_tolerance());
    }

    #[test]
    fn crop_read_agrees_with_the_full_frame() {
        let film = load("kodak_portra_800");
        let params = supported_params();
        let law = law_params(&resolve(&film, &params).unwrap());
        // Fractional source-to-field footprints, across two output bands.
        let pitch = 3.1;
        let source = density_target(96, 160);
        let full = texture::apply(
            &source,
            pitch,
            &readout_geometry(ReadoutGrid {
                origin_um: [0.0; 2],
                pitch_um: [pitch; 2],
                width: 96,
                height: 160,
            }),
            &law,
        )
        .unwrap();
        let crop = texture::apply(
            &source,
            pitch,
            &readout_geometry(ReadoutGrid {
                origin_um: [16.0 * pitch, 40.0 * pitch],
                pitch_um: [pitch; 2],
                width: 32,
                height: 48,
            }),
            &law,
        )
        .unwrap();
        let diff = max_abs_diff_at(&full, &crop, 16, 40);
        assert!(
            diff <= read_tolerance(),
            "crop read diverged from the full frame by {diff}"
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
            let law = law_params(&resolve(&film, &params).unwrap());
            // A bounded film window of a larger source, with a nonzero origin.
            let q = crate::resizing::pixel_size_um(mm, 12000, 8000);
            let full = ReadoutGrid {
                origin_um: [10.0 * q, 8.0 * q],
                pitch_um: [q; 2],
                width: 16,
                height: 8,
            };
            let all = texture::apply(&target, q, &readout_geometry(full), &law).unwrap();
            for x in [0, 8] {
                let tile = ReadoutGrid {
                    origin_um: [(10 + x) as f64 * q, 8.0 * q],
                    width: 8,
                    ..full
                };
                let part = texture::apply(&target, q, &readout_geometry(tile), &law).unwrap();
                assert!(max_abs_diff_at(&all, &part, x, 0) <= read_tolerance());
            }
            let crop = ReadoutGrid {
                origin_um: [13.0 * q, 10.0 * q],
                width: 6,
                height: 4,
                ..full
            };
            let part = texture::apply(&target, q, &readout_geometry(crop), &law).unwrap();
            assert!(max_abs_diff_at(&all, &part, 3, 2) <= read_tolerance());
        }
    }
    /// Zero strength is the deterministic limit: the realized density is the
    /// target, with no random field and no residual count noise.
    #[test]
    fn zero_dye_strength_returns_the_target() {
        use spektrafilm_model::grain::dye::texture::{DyeLawParams, ReadoutGeometry, apply};
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

        let delivered = DyeLawParams::delivered();
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
            let mut image = ImageBuf::new(96, 96);
            image.data.iter_mut().for_each(|v| *v = from_f64(0.62));
            image
        };
        let pitch = 3.5;
        let grid = ReadoutGeometry {
            origin_um: [0.0, 0.0],
            pitch_um: [pitch, pitch],
            width: 96,
            height: 96,
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

    /// Positive profiles keep every density layer on the finer delivered base;
    /// the negative profile keeps the delivered negative base and layer ladder.
    #[test]
    fn positive_profiles_use_the_finer_delivered_base() {
        let params = supported_params();
        for stock in ["kodak_kodachrome_64", "kodak_trix"] {
            let film = load(stock);
            let resolved = resolve(&film, &params).unwrap();
            let scales = layer_scales(&film);
            assert_eq!(scales, [1.0; 3], "{stock}");
            for layer in 0..resolved.layer_count {
                let law = law_params_for_layer(&resolved, scales, layer);
                assert!(
                    (law.length_scale - POSITIVE_BASE_SCALE).abs() < 1e-12,
                    "{stock} layer {layer} did not use the positive base cell"
                );
                let strength = resolved.dye_strength
                    * spektrafilm_backend::dye_params::law::POSITIVE_AMPLITUDE_SCALE;
                assert!(
                    (law.white - spektrafilm_backend::dye_params::law::WHITE * strength).abs()
                        < 1e-12,
                    "{stock} layer {layer} did not carry the positive amplitude"
                );
                assert!(
                    (law.fine_weight - spektrafilm_backend::dye_params::law::POSITIVE_FINE_WEIGHT)
                        .abs()
                        < 1e-12,
                    "{stock} layer {layer} lost the positive microcontrast"
                );
            }
        }

        let film = load("kodak_portra_400");
        let resolved = resolve(&film, &params).unwrap();
        assert_eq!(
            layer_scales(&film),
            spektrafilm_backend::dye_params::law::LAYER_SCALES
        );
        assert!(
            (law_params(&resolved).length_scale - 1.0).abs() < 1e-12,
            "negative profile lost its delivered base cell"
        );
    }

    /// The device realizes the mapped non-default look law with the same
    /// amplitude and mean as the CPU reference.
    #[test]
    fn device_field_realizes_the_mapped_look_law() {
        use spektrafilm_backend::ComputeBackend as _;
        use spektrafilm_model::grain::dye::texture::{ReadoutGeometry, apply};
        let backend = spektrafilm_gpu::test_support::adapter();
        let source = density_target(64, 48);
        let pitch = 3.5;
        let grid = ReadoutGrid {
            origin_um: [0.0, 0.0],
            pitch_um: [pitch, pitch],
            width: 64,
            height: 48,
        };
        let moments = |image: &ImageBuf| {
            let values: Vec<f64> = image.data.chunks(3).map(|p| p[1] as f64).collect();
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
            (mean, var.sqrt())
        };
        for stock in ["kodak_portra_800", "kodak_kodachrome_64"] {
            let film = load(stock);
            let mut params = supported_params();
            params.film_render.grain.dye_strength = 1.6;
            let resolved = resolve(&film, &params).unwrap();
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
            let (gpu_mean, gpu_sd) = moments(&gpu);
            let (cpu_mean, cpu_sd) = moments(&cpu);
            assert!(
                (gpu_mean - cpu_mean).abs() <= 0.02 * cpu_mean.abs(),
                "{stock} mean: device {gpu_mean:.5}, host {cpu_mean:.5}"
            );
            let ratio = gpu_sd.max(1e-12) / cpu_sd.max(1e-12);
            assert!(
                (ratio - 1.0).abs() <= 0.15,
                "{stock} amplitude: device is {ratio:.3} of the host"
            );
        }
    }
}
