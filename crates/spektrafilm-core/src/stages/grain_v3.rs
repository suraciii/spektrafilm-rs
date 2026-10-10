//! Film-coordinate grain V3 integration (`specs/film-grain/v3/`).
//!
//! The pipeline develops the *full* source and hands the developed CMY
//! density to this module. The module owns V3 field resolution and the
//! readout to the requested output geometry
//! ([`spektrafilm_model::grain::v3::apply`]); scanner interpretation and
//! encoding stay with their owners.
//!
//! Film coordinates come from the profile and the geometry: a crop selects
//! the readout region only, so field cells keep their global film positions
//! and overlapping reads agree.

use spektrafilm_math::image::ImageBuf;
use spektrafilm_model::grain::v3::{self, GrainV3Params, SourceRect};

use crate::params::RuntimeParams;
use crate::params::grain::GrainEngine;
use crate::profile::Profile;

/// Stock of the first supported condition (spec §Initial supported
/// condition). Other stocks need the same validation before they are added.
const SUPPORTED_STOCK: &str = "kodak_portra_800";
/// Supported camera long edge, in millimetres (35 mm full-film area).
const SUPPORTED_FILM_FORMAT_MM: f32 = 36.0;
/// Supported route: develop the film and scan it directly.
const SUPPORTED_ROUTE: &str = "input > film > scan";
/// Diagnostic pitch override, in micrometers. Production uses
/// `support_um / 2`; numerical certification may ask for a finer globally
/// fixed pitch (the model rejects a coarser one). It never changes the
/// recipe.
const CELL_UM_ENV: &str = "SPEKTRAFILM_GRAIN_V3_CELL_UM";
/// Diagnostic expected-value render: every Poisson count is replaced by its
/// conditional mean. It never changes the recipe.
const EXPECTATION_ENV: &str = "SPEKTRAFILM_GRAIN_V3_EXPECTATION";

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

/// Whether an active V3 field must run.
///
/// Preview mode, LUT mode and stochastic deactivation already clear
/// `grain.active` at digest time. Spatial deactivation bypasses V3 here:
/// the field is spatial by construction, and the LUT bake needs a
/// pointwise chain. These modes must not resolve or allocate the field.
pub fn active(params: &RuntimeParams) -> bool {
    let grain = &params.film_render.grain;
    grain.active
        && matches!(grain.engine, GrainEngine::V3)
        && !params.debug.deactivate_spatial_effects
}

/// Fail before any work when V3 is selected outside its supported
/// condition. The restriction is this implementation's support, not a
/// permanent exclusion: expanding it requires the same validation for the
/// additional condition.
pub fn validate_supported(film: &Profile, params: &RuntimeParams) -> Result<(), String> {
    if !active(params) {
        return Ok(());
    }
    let stock = film.info.stock.as_deref().unwrap_or("");
    let format_mm = params.camera.film_format_mm;
    let route = params.workflow.route.as_str();
    if stock == SUPPORTED_STOCK
        && format_mm == SUPPORTED_FILM_FORMAT_MM
        && route == SUPPORTED_ROUTE
        && params.io.scan_film
    {
        return Ok(());
    }
    Err(format!(
        "film grain V3: unsupported condition — this implementation supports stock \
         {SUPPORTED_STOCK:?} at a {SUPPORTED_FILM_FORMAT_MM} mm long edge on the \
         `{SUPPORTED_ROUTE}` route with io.scan_film = true, but got stock {stock:?}, \
         camera.film_format_mm {format_mm}, workflow.route {route:?}, io.scan_film {}; \
         select the supported film, format and route, or set \
         film_render.grain.engine to v1/v2",
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
    let (x, y, width, height) =
        crate::resizing::requested_crop_rect(source.width, source.height, &params.io)?;
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

/// Resolve the V3 model inputs from the resolved film profile and the
/// runtime controls. `resolve` owns the profile-derived inputs (layer
/// maxima, composite curves, layer curves, film polarity) so every caller
/// — render, expectation diagnostic, certification — feeds the model the
/// same data.
pub fn resolve(film: &Profile, params: &RuntimeParams) -> Result<GrainV3Params, String> {
    validate_supported(film, params)?;
    let grain = &params.film_render.grain;
    let curves = crate::chain_prep::FilmCurves::prepare(film);
    let layer_curves = film.density_curves_layers_f64();
    if layer_curves.is_empty() || curves.normalized.is_empty() {
        return Err(format!(
            "film grain V3: film profile '{}' has no density curves \
             (density_curves / density_curves_layers), so the field cannot be \
             resolved",
            film.info.stock.as_deref().unwrap_or("<unnamed>")
        ));
    }
    let layer_max = spektrafilm_model::density_curves::density_max_layers_f64(&layer_curves);
    if layer_max.iter().flatten().any(|&max| max <= 0.0) {
        return Err(format!(
            "film grain V3: film profile '{}' yields a non-positive per-layer density \
             maximum (empty or all-zero density_curves_layers), so no field can be \
             realized",
            film.info.stock.as_deref().unwrap_or("<unnamed>")
        ));
    }
    let support_um = grain.v3_dye_support_um as f64;
    Ok(GrainV3Params {
        support_um,
        cell_um: cell_um_override(),
        particle_area_um2: grain.particle_area_um2,
        particle_scale: grain.particle_scale,
        particle_scale_sublayers: grain.particle_scale_sublayers,
        density_min: grain.density_min,
        uniformity: grain.uniformity,
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
/// The field is realized on the CPU: V3 disables the fused WGPU/resident
/// chain (see [`crate::pipeline::Pipeline::process`]), so the reported
/// backend for an active V3 render is CPU regardless of the selected
/// backend of the surrounding stages.
pub fn readout(
    source: &ImageBuf,
    film: &Profile,
    params: &RuntimeParams,
    source_pitch_um: f64,
    backend: &dyn spektrafilm_gpu::ComputeBackend,
) -> Result<ImageBuf, String> {
    let readout_geometry = geometry(source, params, source_pitch_um)?;
    let resolved = resolve(film, params)?;
    // The field resolves on the CPU whatever backend the surrounding stages
    // use; the observation records that executor and its reason.
    let _observation = super::StageObservation::cpu(
        backend,
        "grain_v3_field",
        spektrafilm_gpu::telemetry::CpuReason::GrainV3FieldCpu,
    );
    tracing::info!(
        stock = film.info.stock.as_deref().unwrap_or(""),
        source = format_args!("{}x{}", source.width, source.height),
        region = format_args!(
            "{}x{}+({},{})",
            readout_geometry.rect.width,
            readout_geometry.rect.height,
            readout_geometry.rect.x,
            readout_geometry.rect.y
        ),
        output = format_args!("{}x{}", readout_geometry.width, readout_geometry.height),
        support_um = resolved.support_um,
        cell_um = resolved.cell_um.unwrap_or(resolved.support_um / 2.0),
        expectation = resolved.expectation,
        "film grain V3: field readout on CPU"
    );
    v3::apply(
        source,
        readout_geometry.source_pitch_um,
        readout_geometry.rect,
        readout_geometry.width,
        readout_geometry.height,
        &resolved,
    )
}

/// Effective-backend note for user-visible backend/capability output.
pub fn effective_backend_note() -> &'static str {
    "film grain V3: field readout on CPU (resident WGPU chain disabled)"
}

fn cell_um_override() -> Option<f64> {
    std::env::var(CELL_UM_ENV)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn expectation_requested() -> bool {
    std::env::var(EXPECTATION_ENV).is_ok_and(|value| value == "1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::RuntimeParams;
    use spektrafilm_math::precision::from_f64;

    fn data_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    }

    fn load(stock: &str) -> Profile {
        crate::profile::load_profile_by_name(&data_dir(), stock).unwrap()
    }

    fn supported_params() -> RuntimeParams {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.camera.film_format_mm = 36.0;
        params.film_render.grain.engine = GrainEngine::V3;
        params.film_render.grain.active = true;
        params
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

    #[test]
    fn unsupported_conditions_fail_with_actionable_errors() {
        let film = load("kodak_portra_800");
        let params = supported_params();
        validate_supported(&film, &params).unwrap();
        resolve(&film, &params).unwrap();

        let other_stock = load("kodak_portra_400");
        let error = validate_supported(&other_stock, &params).unwrap_err();
        assert!(
            error.contains("unsupported condition") && error.contains("kodak_portra_400"),
            "{error}"
        );

        let mut format = supported_params();
        format.camera.film_format_mm = 24.0;
        let error = validate_supported(&film, &format).unwrap_err();
        assert!(error.contains("film_format_mm 24"), "{error}");

        for route in [
            "input > film > print > scan",
            "input > convert-film > scan",
            "input > convert-film > scan-minus-base",
            "input",
        ] {
            let mut routed = supported_params();
            routed.workflow.route = route.into();
            routed.io.scan_film = route == "input > film > scan";
            let error = validate_supported(&film, &routed).unwrap_err();
            assert!(error.contains(route), "route {route} not named in {error}");
            assert!(
                resolve(&film, &routed).is_err(),
                "resolve must enforce the support gate"
            );
        }

        // The print chain may not run under a scan route either.
        let mut no_scan = supported_params();
        no_scan.io.scan_film = false;
        assert!(validate_supported(&film, &no_scan).is_err());

        // Inactive V3 is not gated: it never renders, so nothing to reject.
        let mut inactive = supported_params();
        inactive.film_render.grain.active = false;
        inactive.workflow.route = "input > film > print > scan".into();
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

        // Spatial deactivation alone bypasses V3: the field is spatial by
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
    fn crop_read_agrees_with_the_full_frame() {
        let film = load("kodak_portra_800");
        let params = supported_params();
        let resolved = resolve(&film, &params).unwrap();
        // 3.5 µm source pitch against a 4 µm field pitch: fractional
        // footprints, and the 160-row frame spans two output bands.
        let pitch = 3.5;
        let source = density_target(96, 160);
        let full = v3::apply(
            &source,
            pitch,
            SourceRect {
                x: 0,
                y: 0,
                width: 96,
                height: 160,
            },
            96,
            160,
            &resolved,
        )
        .unwrap();
        let crop = v3::apply(
            &source,
            pitch,
            SourceRect {
                x: 16,
                y: 40,
                width: 32,
                height: 48,
            },
            32,
            48,
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
        let rect = SourceRect {
            x: 0,
            y: 0,
            width: 64,
            height: 48,
        };
        let pitch = 3.5;
        let source = density_target(64, 48);
        let coarser = resolve(&film, &params).unwrap();

        // Expected-value render: the support is fixed, only the statistics
        // scale with the event area.
        let mut expected_a = coarser.clone();
        expected_a.expectation = true;
        let mut expected_b = expected_a.clone();
        expected_b.particle_area_um2 = expected_a.particle_area_um2 * 10.0;
        let a = v3::apply(&source, pitch, rect, 64, 48, &expected_a).unwrap();
        let b = v3::apply(&source, pitch, rect, 64, 48, &expected_b).unwrap();
        let mean_diff = max_abs_diff(&a, &b);
        assert!(
            mean_diff <= 1e-5,
            "the dye support must not follow the event area (diff {mean_diff})"
        );

        // Realized field: the same support, different statistical strength.
        let mut realized_b = coarser.clone();
        realized_b.particle_area_um2 *= 10.0;
        let ra = v3::apply(&source, pitch, rect, 64, 48, &coarser).unwrap();
        let rb = v3::apply(&source, pitch, rect, 64, 48, &realized_b).unwrap();
        let noise_diff = max_abs_diff(&ra, &rb);
        assert!(
            noise_diff > 1e-4,
            "event area must control statistical strength (diff {noise_diff})"
        );
    }
}
