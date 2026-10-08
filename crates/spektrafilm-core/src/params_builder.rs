//! Runtime parameter digestion — mirrors Python
//! `spektrafilm/runtime/params_builder.py` (upstream 0.3.4).
//!
//! [`digest_params`] folds the user-facing parameter state into the static
//! form the pipeline consumes: database neutral print filters, preview
//! deactivation, stock-specific overrides, then the debug / lut-mode
//! switches — exactly the upstream order, so the precedence is upstream's:
//!
//! 1. `apply_database_neutral_print_filters`
//! 2. `settings.preview_mode` deactivation
//! 3. stock-specific overrides (film: DIR couplers, halation preset,
//!    velvia/provia; print: none in 0.3.4)
//! 4. `debug.lut_mode` promotion
//! 5. `debug.deactivate_spatial_effects`
//! 6. `debug.deactivate_stochastic_effects`
//!
//! In the pipeline params should be static and not be changed;
//! `settings` and `debug` contain all the switching logic for the digesting.

use crate::neutral_filters::NeutralFilters;
use crate::params::RuntimeParams;
use crate::profile::Profile;

/// Halation low-level presets keyed by `(use, antihalation)`. Mirrors
/// upstream `_HALATION_PRESETS`: `sigma_h` follows the base material
/// (still → triacetate ~65 µm, cine → PET ~50 µm), the strength follows
/// the antihalation undercoat. The user-facing knobs
/// (`scatter_amount`, `halation_amount`, …) stay at 1.0 so these seeds
/// define the physical baseline.
const HALATION_PRESETS: &[((&str, &str), ([f64; 3], [f64; 3]))] = &[
    (("still", "strong"), ([65.0, 65.0, 65.0], [0.015, 0.005, 0.0])),
    (("still", "weak"), ([65.0, 65.0, 65.0], [0.08, 0.02, 0.0])),
    (("still", "no"), ([65.0, 65.0, 65.0], [0.30, 0.10, 0.015])),
    (("cine", "strong"), ([50.0, 50.0, 50.0], [0.015, 0.005, 0.0])),
    (("cine", "weak"), ([50.0, 50.0, 50.0], [0.08, 0.02, 0.0])),
    (("cine", "no"), ([50.0, 50.0, 50.0], [0.30, 0.10, 0.015])),
];

/// Digest the params to prepare for use in the runtime pipeline.
///
/// Mirrors upstream `digest_params(params, apply_stocks_specifics=True)`:
/// `apply_stocks_specifics = false` skips the stock-specific overrides so a
/// GUI can seed them once per profile selection (upstream
/// `apply_profile_defaults`) and keep user edits of those groups on later
/// re-digests. Everything else (database filters, preview, debug switches)
/// is applied on every digest.
///
/// The `database` is the neutral-print-filter lookup; `None` behaves like
/// upstream's missing database file (no lookup, defaults kept). Digestion
/// never fails — unknown enum-like strings are rejected earlier by
/// [`RuntimeParams::validate`].
pub fn digest_params(
    mut params: RuntimeParams,
    film: &Profile,
    print: &Profile,
    database: Option<&NeutralFilters>,
    apply_stocks_specifics: bool,
) -> RuntimeParams {
    params = apply_database_neutral_print_filters(params, film, print, database);

    if params.settings.preview_mode {
        // Upstream preview: kill the spatially-variant and stochastic
        // cost centres but keep the (cheap, per-pixel) colour chain.
        // Scatter/halation kernel sigmas are preserved in preview mode.
        params.enlarger.lens_blur = 0.0;
        params.film_render.dir_couplers.diffusion_size_um = 0.0;
        params.film_render.grain.active = false;
        params.film_render.grain.particle_area_um2 = 0.0;
        params.film_render.grain.rms_granularity = [0.0; 3];
        params.film_render.grain.blur = 0.0;
        params.film_render.grain.mult_usm_sigma = 0.0;
        params.film_render.grain.mult_usm_amount = 0.0;
        params.print_render.glare.blur = 0.0;
        params.camera.lens_blur_um = 0.0;
        params.scanner.lens_blur = 0.0;
        params.scanner.unsharp_mask = [0.0, 0.0];
    }

    if apply_stocks_specifics {
        apply_film_specifics(&mut params, film);
        // Upstream `_apply_print_specifics` is intentionally empty in 0.3.4.
    }

    if params.debug.lut_mode {
        // LUT-sampling regime: force the pipeline into a deterministic
        // per-pixel transform. Enabling lut_mode promotes spatial and
        // stochastic deactivation and disables image-aware adjustments.
        params.debug.deactivate_spatial_effects = true;
        params.debug.deactivate_stochastic_effects = true;
        // exposure control
        params.camera.auto_exposure = false;
        params.camera.exposure_compensation_ev = 0.0;
        params.enlarger.print_exposure_compensation = false;
        params.enlarger.print_exposure = 1.0;
        // Highlight boost normalizes by the image-wide max (np.max(x)), so it
        // is an image-global transform — the same input value maps to
        // different outputs depending on the rest of the frame. That cannot
        // be represented by a static 3D LUT (a bake would freeze in the cube
        // grid's max), so it must be off in lut_mode, exactly like
        // auto_exposure above.
        params.film_render.halation.boost_ev = 0.0;
        params.scanner.white_correction = false;
        params.scanner.black_correction = false;
        params.scanner.unsharp_mask = [0.0, 0.0];
        params.enlarger.y_filter_shift = 0.0;
        params.enlarger.m_filter_shift = 0.0;
        params.enlarger.preflash_exposure = 0.0;
        params.enlarger.preflash_y_filter_shift = 0.0;
        params.enlarger.preflash_m_filter_shift = 0.0;
        params.io.crop = false;
        params.io.upscale_factor = 1.0;
        params.settings.use_enlarger_lut = false;
        params.settings.use_scanner_lut = false;
    }

    if params.debug.deactivate_spatial_effects {
        // Halation is fully spatial (scatter + back-reflection blurs); kill
        // it at the active flag as well as zeroing the kernel sigmas, so it
        // stays a no-op even if a future sigma-independent term is added
        // inside apply_halation_um.
        params.film_render.halation.active = false;
        params.film_render.halation.scatter_core_um = [0.0, 0.0, 0.0];
        params.film_render.halation.scatter_tail_um = [0.0, 0.0, 0.0];
        params.film_render.halation.halation_first_sigma_um = [0.0, 0.0, 0.0];
        params.film_render.dir_couplers.diffusion_size_um = 0.0;
        params.film_render.grain.blur = 0.0;
        params.film_render.grain.blur_dye_clouds_um = 0.0;
        params.film_render.grain.mult_usm_sigma = 0.0;
        params.film_render.grain.mult_usm_amount = 0.0;
        params.print_render.glare.blur = 0.0;
        params.camera.lens_blur_um = 0.0;
        params.enlarger.lens_blur = 0.0;
        params.enlarger.diffusion_filter.active = false;
        params.camera.diffusion_filter.active = false;
        params.scanner.lens_blur = 0.0;
        params.scanner.unsharp_mask = [0.0, 0.0];
    }

    if params.debug.deactivate_stochastic_effects {
        params.film_render.grain.active = false;
        params.print_render.glare.active = false;
    }

    params
}

/// Set neutral dichroic filters from the stock database. Missing combinations
/// leave the caller's configured defaults unchanged, matching upstream.
pub fn apply_database_neutral_print_filters(
    mut params: RuntimeParams,
    film: &Profile,
    print: &Profile,
    database: Option<&NeutralFilters>,
) -> RuntimeParams {
    if !params.settings.neutral_print_filters_from_database {
        return params;
    }
    let print_stock = print.info.stock.as_deref().unwrap_or("");
    let film_stock = film.info.stock.as_deref().unwrap_or("");
    match database.and_then(|db| db.lookup(print_stock, &params.enlarger.illuminant, film_stock)) {
        Some([c, m, y]) => {
            params.enlarger.c_filter_neutral = c as f32;
            params.enlarger.m_filter_neutral = m as f32;
            params.enlarger.y_filter_neutral = y as f32;
        }
        None => {}
    }
    params
}

/// Film-specific overrides — mirrors upstream `_apply_film_specifics`:
/// positive/negative DIR-coupler gammas, the `(use, antihalation)` halation
/// preset, and the velvia/provia slide-film coupler retunes.
pub(crate) fn apply_film_specifics(params: &mut RuntimeParams, film: &Profile) {
    apply_grain_preset(params, film);
    apply_coupler_preset(params, film);
    apply_halation_preset(params, film);
}

#[derive(serde::Deserialize)]
struct GrainPresetFile {
    defaults: Option<toml::Value>,
    #[serde(flatten)]
    stocks: std::collections::HashMap<String, toml::Value>,
}

fn apply_grain_preset(params: &mut RuntimeParams, film: &Profile) {
    let Ok(file) = toml::from_str::<GrainPresetFile>(include_str!("../../../data/presets/grain.toml")) else {
        return;
    };
    let class = if film.is_bw() { "bw" } else { "color" };
    let polarity = if film.is_positive() { "positive" } else { "negative" };
    let defaults = file.defaults.as_ref().and_then(|v| v.get(class)).and_then(|v| v.get(polarity));
    let stock = film.info.stock.as_deref().and_then(|s| file.stocks.get(s));
    if stock.is_none() { return; }
    let value = |key: &str| stock.and_then(|v| v.get(key)).or_else(|| defaults.and_then(|v| v.get(key)));
    let g = &mut params.film_render.grain;
    let array = |key: &str, dst: &mut [f64]| {
        if let Some(toml::Value::Array(values)) = value(key) {
            for (d, v) in dst.iter_mut().zip(values) {
                if let Some(v) = v.as_float() { *d = v; }
            }
        }
    };
    array("rms_granularity", &mut g.rms_granularity);
    array("density_min", &mut g.density_min);
    array("uniformity", &mut g.uniformity);
    array("particle_scale_sublayers", &mut g.particle_scale_sublayers);
}

#[derive(serde::Deserialize)]
struct CouplerPresetFile {
    defaults: Option<toml::Value>,
    #[serde(flatten)]
    stocks: std::collections::HashMap<String, toml::Value>,
}

fn apply_coupler_preset(params: &mut RuntimeParams, film: &Profile) {
    let Ok(file) = toml::from_str::<CouplerPresetFile>(include_str!("../../../data/presets/couplers.toml")) else {
        return;
    };
    let class = if film.is_bw() { "bw" } else { "color" };
    let polarity = if film.is_positive() { "positive" } else { "negative" };
    let defaults = file.defaults.as_ref().and_then(|v| v.get(class)).and_then(|v| v.get(polarity));
    let cine = if class == "color" && polarity == "negative" && film.is_film() && film.info.usage == "cine" {
        let branch = if film.info.reference_illuminant.to_ascii_uppercase().starts_with('T') { "tungsten" } else { "daylight" };
        defaults.and_then(|v| v.get("cine")).and_then(|v| v.get(branch))
    } else { None };
    let stock = film.info.stock.as_deref().and_then(|s| file.stocks.get(s));
    let set = |key: &str, dst: &mut [f64]| {
        let value = stock.and_then(|v| v.get(key))
            .or_else(|| cine.and_then(|v| v.get(key)))
            .or_else(|| defaults.and_then(|v| v.get(key)));
        if let Some(toml::Value::Array(values)) = value {
            for (d, v) in dst.iter_mut().zip(values) {
                if let Some(v) = v.as_float() { *d = v; }
            }
        }
    };
    let d = &mut params.film_render.dir_couplers;
    set("gamma_samelayer_rgb", &mut d.gamma_samelayer_rgb);
    set("gamma_interlayer_r_to_gb", &mut d.gamma_interlayer_r_to_gb);
    set("gamma_interlayer_g_to_rb", &mut d.gamma_interlayer_g_to_rb);
    set("gamma_interlayer_b_to_rg", &mut d.gamma_interlayer_b_to_rg);
}

/// Seed low-level halation parameters from the profile's `use` /
/// `antihalation` tags — mirrors upstream `_apply_halation_preset`.
/// Papers (and films without a preset) keep their values.
fn apply_halation_preset(params: &mut RuntimeParams, film: &Profile) {
    if !film.is_film() {
        return;
    }
    let key = (film.info.usage.as_str(), film.info.antihalation.as_str());
    if let Some((sigma_h, strength)) = HALATION_PRESETS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
    {
        params.film_render.halation.halation_first_sigma_um = *sigma_h;
        params.film_render.halation.halation_strength = *strength;
    }
}

/// B&W engine-layout broadcast, applied by the pipeline after the digest
/// (digest output stays in user units so GUI state seeds cleanly).
///
/// Upstream `n_channels == 1` profiles keep channel-0 of every per-channel
/// tuple (the DIR matrix degenerates to 1×1 self-inhibition with no
/// interlayer terms). The Rust engine always runs 3 channels, so forcing
/// the arrays to channel-0 keeps it numerically identical to the upstream
/// single-channel run. Grain `monochrome` shares one noise field (one
/// emulsion, seed ch=0) and is derived from the film, never user-set —
/// cleared for colour so a stray params file can't correlate the colour
/// channels' noise.
pub fn broadcast_monochrome_layout(film: &Profile, params: &mut RuntimeParams) {
    params.film_render.grain.monochrome = film.is_bw();
    if !film.is_bw() {
        return;
    }
    let g = &mut params.film_render.grain;
    let dir = &mut params.film_render.dir_couplers;
    dir.gamma_samelayer_rgb = [dir.gamma_samelayer_rgb[0]; 3];
    dir.gamma_interlayer_r_to_gb = [0.0, 0.0];
    dir.gamma_interlayer_g_to_rb = [0.0, 0.0];
    dir.gamma_interlayer_b_to_rg = [0.0, 0.0];
    g.rms_granularity = [g.rms_granularity[0]; 3];
    g.density_min = [g.density_min[0]; 3];
    g.uniformity = [g.uniformity[0]; 3];
    let h = &mut params.film_render.halation;
    h.halation_strength = [h.halation_strength[0]; 3];
    h.scatter_core_um = [h.scatter_core_um[0]; 3];
    h.scatter_tail_um = [h.scatter_tail_um[0]; 3];
    h.scatter_tail_weight = [h.scatter_tail_weight[0]; 3];
    h.halation_first_sigma_um = [h.halation_first_sigma_um[0]; 3];
}

/// Bound an image's long edge to `max_size` for preview rendering — mirrors
/// upstream `spektrafilm/utils/preview.py:resize_for_preview`
/// (bilinear + anti-aliasing; no-op when already within bounds). Used by
/// `Pipeline::process_preview` (upstream `simulate_preview`).
pub fn resize_for_preview(
    image: &spektrafilm_math::image::ImageBuf,
    max_size: u32,
) -> spektrafilm_math::image::ImageBuf {
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::{Scalar, from_f32, to_f32};

    let max_size = max_size.max(1) as f32;
    let (w, h) = (image.width as f32, image.height as f32);
    if w.max(h) <= max_size {
        return image.clone();
    }
    let scale_factor = max_size / w.max(h);
    // Python: resize(image, (int(h * scale), int(w * scale))) — int() truncates.
    let new_w = ((w * scale_factor) as u32).max(1);
    let new_h = ((h * scale_factor) as u32).max(1);
    let f32buf: Vec<f32> = image.data.iter().map(|&v| to_f32(v)).collect();
    let src = image::ImageBuffer::<image::Rgb<f32>, _>::from_raw(
        image.width,
        image.height,
        f32buf,
    )
    .expect("ImageBuf dims match its data length");
    // Triangle = bilinear (skimage order=1); its area-averaging kernel
    // provides the anti-aliasing of skimage's anti_aliasing=True.
    let dst = image::imageops::resize(&src, new_w, new_h, image::imageops::FilterType::Triangle);
    let data: Vec<Scalar> = dst.into_raw().into_iter().map(from_f32).collect();
    ImageBuf::from_data(new_w, new_h, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{DebugParams, RuntimeParams, Tap, TapsParams};

    fn blank_profile() -> Profile {
        // Every Profile field has a serde default; only the three section
        // keys are structurally required.
        serde_json::from_str(r#"{"metadata":{},"info":{},"data":{}}"#).unwrap()
    }

    fn film_profile(stock: &str, film_type: &str, usage: &str, antihalation: &str) -> Profile {
        let mut p = blank_profile();
        p.info.stock = Some(stock.to_string());
        p.info.film_type = film_type.to_string();
        p.info.support = "film".to_string();
        p.info.usage = usage.to_string();
        p.info.antihalation = antihalation.to_string();
        p
    }

    #[test]
    fn lut_mode_digest_matches_upstream_switch_set() {
        let film = film_profile("kodak_portra_400", "negative", "still", "strong");
        let print = blank_profile();
        let mut params = RuntimeParams::default();
        // User cranks everything the lut_mode digest must force off.
        params.camera.auto_exposure = true;
        params.camera.exposure_compensation_ev = 2.5;
        params.enlarger.print_exposure = 3.0;
        params.enlarger.print_exposure_compensation = true;
        params.film_render.halation.boost_ev = 1.5;
        params.scanner.white_correction = true;
        params.scanner.black_correction = true;
        params.scanner.unsharp_mask = [0.7, 0.7];
        params.debug.lut_mode = true;
        params.enlarger.y_filter_shift = 12.0;
        params.enlarger.m_filter_shift = -8.0;
        params.enlarger.preflash_exposure = 0.1;
        params.enlarger.preflash_y_filter_shift = 3.0;
        params.enlarger.preflash_m_filter_shift = 4.0;
        params.io.crop = true;
        params.io.upscale_factor = 2.0;
        params.settings.use_enlarger_lut = true;
        params.settings.use_scanner_lut = true;

        let d = digest_params(params, &film, &print, None, true);
        assert!(d.debug.deactivate_spatial_effects, "lut_mode promotes spatial deactivation");
        assert!(d.debug.deactivate_stochastic_effects, "lut_mode promotes stochastic deactivation");
        assert!(!d.camera.auto_exposure);
        assert_eq!(d.camera.exposure_compensation_ev, 0.0);
        assert!(!d.enlarger.print_exposure_compensation);
        assert_eq!(d.enlarger.print_exposure, 1.0);
        assert_eq!(d.film_render.halation.boost_ev, 0.0);
        assert!(!d.scanner.white_correction);
        assert!(!d.scanner.black_correction);
        assert_eq!(d.scanner.unsharp_mask, [0.0, 0.0]);
        assert_eq!(d.enlarger.y_filter_shift, 0.0);
        assert_eq!(d.enlarger.m_filter_shift, 0.0);
        assert_eq!(d.enlarger.preflash_exposure, 0.0);
        assert_eq!(d.enlarger.preflash_y_filter_shift, 0.0);
        assert_eq!(d.enlarger.preflash_m_filter_shift, 0.0);
        assert!(!d.io.crop);
        assert_eq!(d.io.upscale_factor, 1.0);
        assert!(!d.settings.use_enlarger_lut);
        assert!(!d.settings.use_scanner_lut);
        // Spatial deactivation payload
        assert!(!d.film_render.halation.active);
        assert_eq!(d.film_render.halation.scatter_core_um, [0.0, 0.0, 0.0]);
        assert_eq!(d.film_render.halation.scatter_tail_um, [0.0, 0.0, 0.0]);
        assert_eq!(d.film_render.halation.halation_first_sigma_um, [0.0, 0.0, 0.0]);
        assert_eq!(d.film_render.dir_couplers.diffusion_size_um, 0.0);
        assert_eq!(d.film_render.grain.blur, 0.0);
        assert_eq!(d.film_render.grain.blur_dye_clouds_um, 0.0);
        assert_eq!(d.print_render.glare.blur, 0.0);
        assert_eq!(d.camera.lens_blur_um, 0.0);
        assert_eq!(d.enlarger.lens_blur, 0.0);
        assert!(!d.camera.diffusion_filter.active);
        assert!(!d.enlarger.diffusion_filter.active);
        assert_eq!(d.scanner.lens_blur, 0.0);
        // Stochastic deactivation payload
        assert!(!d.film_render.grain.active);
        assert!(!d.print_render.glare.active);
        // DIR stays active (upstream: non-spatial coupler chemistry keeps running)
        assert!(d.film_render.dir_couplers.active);
    }

    #[test]
    fn preview_mode_digest_zeroes_only_the_upstream_set() {
        let film = film_profile("kodak_portra_400", "negative", "still", "strong");
        let print = blank_profile();
        let mut params = RuntimeParams::default();
        params.settings.preview_mode = true;
        // Halation stays fully configured in preview (upstream keeps it).
        params.film_render.halation.scatter_core_um = [2.2, 2.0, 1.6];

        let d = digest_params(params, &film, &print, None, true);
        assert_eq!(d.enlarger.lens_blur, 0.0);
        assert_eq!(d.film_render.dir_couplers.diffusion_size_um, 0.0);
        assert!(!d.film_render.grain.active);
        assert_eq!(d.film_render.grain.particle_area_um2, 0.0);
        assert_eq!(d.film_render.grain.blur, 0.0);
        assert_eq!(d.print_render.glare.blur, 0.0);
        assert_eq!(d.camera.lens_blur_um, 0.0);
        assert_eq!(d.scanner.lens_blur, 0.0);
        assert_eq!(d.scanner.unsharp_mask, [0.0, 0.0]);
        // kernel sigmas preserved, halation/glare stay active in preview
        assert_eq!(d.film_render.halation.scatter_core_um, [2.2, 2.0, 1.6]);
        assert!(d.film_render.halation.active);
        assert!(d.print_render.glare.active);
        assert!(d.film_render.dir_couplers.active);
    }

    #[test]
    fn debug_switches_win_over_preview_mode() {
        // Upstream order: preview block first, debug last → debug zeroing wins.
        let film = film_profile("x", "negative", "still", "weak");
        let print = blank_profile();
        let mut params = RuntimeParams::default();
        params.settings.preview_mode = true;
        params.debug.deactivate_spatial_effects = true;

        let d = digest_params(params, &film, &print, None, true);
        assert!(!d.film_render.halation.active, "spatial-off overrides preview's halation-preserving behaviour");
    }

    #[test]
    fn halation_preset_seeds_from_use_and_antihalation() {
        let film = film_profile("some_cine_stock", "negative", "cine", "no");
        let print = blank_profile();
        let d = digest_params(RuntimeParams::default(), &film, &print, None, true);
        assert_eq!(d.film_render.halation.halation_first_sigma_um, [50.0, 50.0, 50.0]);
        assert_eq!(d.film_render.halation.halation_strength, [0.30, 0.10, 0.015]);

        let film_still = film_profile("some_stock", "negative", "still", "weak");
        let d = digest_params(RuntimeParams::default(), &film_still, &print, None, true);
        assert_eq!(d.film_render.halation.halation_first_sigma_um, [65.0, 65.0, 65.0]);
        assert_eq!(d.film_render.halation.halation_strength, [0.08, 0.02, 0.0]);
    }

    #[test]
    fn stock_specific_preset_overrides_and_their_skip() {
        let film = film_profile("fujifilm_velvia_100", "positive", "still", "strong");
        let print = blank_profile();
        let d = digest_params(RuntimeParams::default(), &film, &print, None, true);
        assert_eq!(d.film_render.dir_couplers.gamma_samelayer_rgb, [0.108, 0.072, 0.054]);

        // apply_stocks_specifics=false keeps user values (GUI edit path).
        let mut user = RuntimeParams::default();
        user.film_render.dir_couplers.gamma_samelayer_rgb = [0.9, 0.8, 0.7];
        let d = digest_params(user, &film, &print, None, false);
        assert_eq!(d.film_render.dir_couplers.gamma_samelayer_rgb, [0.9, 0.8, 0.7]);
    }

    #[test]
    fn presets_override_controls_when_stock_specifics_are_requested() {
        let film = film_profile("kodak_portra_400", "negative", "still", "strong");
        let print = blank_profile();
        let mut params = RuntimeParams::default();
        params.film_render.grain.rms_granularity = [99.0; 3];
        params.film_render.grain.uniformity = [0.5; 3];
        params.film_render.dir_couplers.gamma_samelayer_rgb = [0.9; 3];
        let seeded = digest_params(params.clone(), &film, &print, None, true);
        assert_eq!(seeded.film_render.grain.rms_granularity, [4.5; 3]);
        assert_eq!(seeded.film_render.grain.uniformity, [0.97, 0.99, 0.97]);
        assert_eq!(seeded.film_render.dir_couplers.gamma_samelayer_rgb, [0.336, 0.319, 0.273]);
        let edited = digest_params(params, &film, &print, None, false);
        assert_eq!(edited.film_render.grain.rms_granularity, [99.0; 3]);
        assert_eq!(edited.film_render.dir_couplers.gamma_samelayer_rgb, [0.9; 3]);
        let unknown = film_profile("custom_stock", "negative", "still", "strong");
        let mut custom = RuntimeParams::default();
        custom.film_render.grain.rms_granularity = [99.0; 3];
        let custom = digest_params(custom, &unknown, &print, None, true);
        assert_eq!(custom.film_render.grain.rms_granularity, [99.0; 3]);
    }

    #[test]
    fn missing_neutral_filter_combination_keeps_configured_defaults() {
        let film = film_profile("custom_stock", "negative", "still", "strong");
        let print = blank_profile();
        let mut params = RuntimeParams::default();
        params.enlarger.c_filter_neutral = 17.0;
        params.enlarger.m_filter_neutral = 23.0;
        params.enlarger.y_filter_neutral = 91.0;
        let params = apply_database_neutral_print_filters(params, &film, &print, None);
        assert_eq!([params.enlarger.c_filter_neutral, params.enlarger.m_filter_neutral, params.enlarger.y_filter_neutral], [17.0, 23.0, 91.0]);
    }

    #[test]
    fn taps_parse_and_reject() {
        for name in [
            "rgb_in", "rgb_pre", "log_e_film", "cmy_film", "log_e_print", "cmy_print", "rgb_out",
        ] {
            assert_eq!(Tap::parse(name).unwrap().name(), name);
        }
        let err = Tap::parse("cmy_prints").unwrap_err();
        assert!(err.contains("unknown tap"), "{err}");
        assert!(err.contains("rgb_in") && err.contains("cmy_print"), "error lists valid taps: {err}");

        let mut params = RuntimeParams::default();
        params.taps = TapsParams {
            inject: Some("bogus".into()),
            collect: None,
        };
        assert!(params.validate().is_err());
        params.taps = TapsParams {
            inject: Some("cmy_film".into()),
            collect: Some("rgb_out".into()),
        };
        params.validate().unwrap();
        let _ = DebugParams::default();
    }

    #[test]
    fn validate_rejects_unknown_enums() {
        let mut params = RuntimeParams::default();
        params.io.input_color_space = "WideGamut".into();
        let err = params.validate().unwrap_err();
        assert!(err.starts_with("io.input_color_space"), "{err}");

        let mut params = RuntimeParams::default();
        params.camera.diffusion_filter.active = true;
        params.camera.diffusion_filter.filter_family = "soft_1".into();
        assert!(params.validate().unwrap_err().contains("soft_1"));

        params.enlarger.illuminant = "BB1000".into();
        assert!(params.validate().unwrap_err().contains("BB1000"));

        let mut params = RuntimeParams::default();
        params.settings.rgb_to_raw_method = "hanatos2019".into();
        assert!(params.validate().unwrap_err().contains("hanatos2019"));

        RuntimeParams::default().validate().unwrap();
    }

    #[test]
    fn unknown_json_fields_are_rejected() {
        let err = serde_json::from_str::<RuntimeParams>(r#"{"camera": {"aut_exposure": true}}"#);
        assert!(err.is_err(), "typo'd field must not deserialize silently");
        let err = serde_json::from_str::<RuntimeParams>(r#"{"debugo": {}}"#);
        assert!(err.is_err());
        let params: RuntimeParams = serde_json::from_str(
            r#"{"film_render": {"grain": {"active": true,
               "rms_granularity": [6.0, 8.0, 10.0],
               "density_min": [0.03, 0.03, 0.03],
               "uniformity": [0.97, 0.97, 0.97],
               "particle_scale_sublayers": [1.0, 0.5, 0.25],
               "blur": 0.89, "mult_usm_sigma": 0.7,
               "mult_usm_amount": 1.5, "blur_dye_clouds_um": 2.0,
               "micro_structure": [0.2, 30.0]}}}"#,
        )
        .unwrap();
        assert_eq!(params.film_render.grain.rms_granularity, [6.0, 8.0, 10.0]);
        assert!(serde_json::from_str::<RuntimeParams>(
            r#"{"film_render": {"grain": {"particle_area_um2": 0.4}}}"#
        )
        .is_err());
    }

    #[test]
    fn nested_print_morph_requires_explicit_activation() {
        assert!(!RuntimeParams::default().print_render.density_curves_morph.active);
        for json in [
            r#"{}"#,
            r#"{"print_render":{}}"#,
            r#"{"print_render":{"density_curves_morph":{}}}"#,
            r#"{"print_render":{"density_curves_morph":{"gamma_factor":1.5}}}"#,
        ] {
            let params: RuntimeParams = serde_json::from_str(json).unwrap();
            assert!(!params.print_render.density_curves_morph.active, "{json}");
        }
        let params: RuntimeParams = serde_json::from_str(
            r#"{"print_render":{"density_curves_morph":{"gamma_factor":1.5,"active":true}}}"#,
        ).unwrap();
        assert!(params.print_render.density_curves_morph.active);
        assert_eq!(params.print_render.density_curves_morph.gamma_factor, 1.5);
        assert!(serde_json::from_str::<RuntimeParams>(
            r#"{"print_render":{"density_curves_morph":{"gamma_facotr":1.5}}}"#,
        ).is_err());
        assert!(crate::params::PrintCurvesMorphParams::default().active);
    }

    #[test]
    fn preview_resize_bounds_long_edge() {
        use spektrafilm_math::image::ImageBuf;
        use spektrafilm_math::precision::from_f64;
        let img = ImageBuf::from_data(1000, 500, vec![from_f64(0.5); 1000 * 500 * 3]);
        let out = resize_for_preview(&img, 640);
        assert_eq!(out.width, 640);
        assert_eq!(out.height, 320, "int(500 * 640/1000) = 320");
        // Within bounds → untouched (same dims and data).
        let small = ImageBuf::from_data(300, 200, vec![from_f64(0.25); 300 * 200 * 3]);
        let out = resize_for_preview(&small, 640);
        assert_eq!(out.width, 300);
        assert_eq!(out.height, 200);
    }
}
