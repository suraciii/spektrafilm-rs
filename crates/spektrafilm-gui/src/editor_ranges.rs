//! Per-path editor range table.
//!
//! Single source of truth for GUI editor ranges and CLI discovery output.
//! A missing entry is a build-time defect, not a silent default to the
//! machine range. The editor range must be contained within the
//! validator's accepted range and must be wide enough to reach the
//! parameter's documented usable values.

/// Editor range for a numeric parameter: (min, max, step, decimals).
pub type EditorRange = (f64, f64, f64, usize);

/// Look up the editor range for a parameter path.
///
/// Returns `None` when the path has no entry. The GUI editor treats a
/// missing entry as a defect; the CLI discovery output omits the field.
pub fn editor_range(path: &str) -> Option<EditorRange> {
    match path {
        // Camera
        "camera.exposure_compensation_ev" => Some((-10.0, 10.0, 0.25, 2)),
        "camera.film_format_mm" => Some((1.0, 1000.0, 0.5, 2)),

        // Enlarger
        "enlarger.print_exposure" => Some((-2.0, 2.0, 0.02, 2)),
        "enlarger.preflash_exposure" => Some((0.0, 1.0, 0.005, 2)),
        "enlarger.y_filter_shift" => Some((-100.0, 100.0, 1.0, 2)),
        "enlarger.m_filter_shift" => Some((-100.0, 100.0, 1.0, 2)),
        "enlarger.preflash_y_filter_shift" => Some((-100.0, 100.0, 1.0, 2)),
        "enlarger.preflash_m_filter_shift" => Some((-100.0, 100.0, 1.0, 2)),

        // Magazine print color
        "magazine_print_color.strength" => Some((0.0, 1.0, 0.01, 2)),

        // Camera and enlarger diffusion filters
        "camera.diffusion_filter.strength" => Some((0.0, 2.0, 0.125, 2)),
        "camera.diffusion_filter.spatial_scale" => Some((0.0, 1000000.0, 0.1, 2)),
        "camera.diffusion_filter.halo_warmth" => Some((-1.5, 1.5, 0.05, 2)),
        "camera.diffusion_filter.core_intensity" => Some((0.0, 4.0, 0.05, 2)),
        "camera.diffusion_filter.core_size" => Some((0.1, 4.0, 0.05, 2)),
        "camera.diffusion_filter.halo_intensity" => Some((0.0, 4.0, 0.05, 2)),
        "camera.diffusion_filter.halo_size" => Some((0.1, 4.0, 0.05, 2)),
        "camera.diffusion_filter.bloom_intensity" => Some((0.0, 4.0, 0.05, 2)),
        "camera.diffusion_filter.bloom_size" => Some((0.1, 4.0, 0.05, 2)),
        "enlarger.diffusion_filter.strength" => Some((0.0, 2.0, 0.125, 2)),
        "enlarger.diffusion_filter.spatial_scale" => Some((0.0, 1000000.0, 0.1, 2)),
        "enlarger.diffusion_filter.halo_warmth" => Some((-1.5, 1.5, 0.05, 2)),
        "enlarger.diffusion_filter.core_intensity" => Some((0.0, 4.0, 0.05, 2)),
        "enlarger.diffusion_filter.core_size" => Some((0.1, 4.0, 0.05, 2)),
        "enlarger.diffusion_filter.halo_intensity" => Some((0.0, 4.0, 0.05, 2)),
        "enlarger.diffusion_filter.halo_size" => Some((0.1, 4.0, 0.05, 2)),
        "enlarger.diffusion_filter.bloom_intensity" => Some((0.0, 4.0, 0.05, 2)),
        "enlarger.diffusion_filter.bloom_size" => Some((0.1, 4.0, 0.05, 2)),

        // Film base
        "film_render.base.scale" => Some((0.0, 2.0, 0.05, 2)),
        "film_render.base.tilt" => Some((-1.0, 1.0, 0.05, 2)),
        "film_render.base.cyan" => Some((0.0, 2.0, 0.05, 2)),
        "film_render.base.magenta" => Some((0.0, 2.0, 0.05, 2)),
        "film_render.base.yellow" => Some((0.0, 2.0, 0.05, 2)),

        // Film chemistry
        "film_render.chemistry.gamma_factor" => Some((0.25, 4.0, 0.05, 2)),
        "film_render.chemistry.gamma_factor_fast" => Some((0.25, 4.0, 0.05, 2)),
        "film_render.chemistry.gamma_factor_slow" => Some((0.25, 4.0, 0.05, 2)),
        "film_render.chemistry.gamma_factor_red" => Some((0.25, 4.0, 0.02, 2)),
        "film_render.chemistry.gamma_factor_green" => Some((0.25, 4.0, 0.02, 2)),
        "film_render.chemistry.gamma_factor_blue" => Some((0.25, 4.0, 0.02, 2)),
        "film_render.chemistry.developer_exhaustion" => Some((0.0, 1.0, 0.02, 2)),

        // Film convert
        "film_render.convert.base_percentile" => Some((50.0, 100.0, 0.5, 2)),
        "film_render.convert.exposure_compensation_ev" => Some((-10.0, 10.0, 0.25, 2)),

        // Film grain
        "film_render.grain.v2_size" => Some((1.0, 48.0, 0.5, 1)),
        "film_render.grain.v2_shadows" => Some((0.0, 100.0, 1.0, 1)),
        "film_render.grain.v2_midtones" => Some((0.0, 100.0, 1.0, 1)),
        "film_render.grain.v2_highlights" => Some((0.0, 100.0, 1.0, 1)),
        "film_render.grain.v2_resolution_factor" => Some((0.0, 100.0, 1.0, 1)),
        "film_render.grain.v2_chroma" => Some((0.0, 100.0, 1.0, 1)),
        "film_render.grain.v2_amount" => Some((0.0, 100.0, 1.0, 1)),
        "film_render.grain.v2_resolution_type" => Some((0.0, 1.0, 1.0, 0)),
        "film_render.grain.v2_timer" => Some((0.0, 0.999999, 0.01, 6)),
        "film_render.grain.v3_dye_support_um" => Some((1.0, 32.0, 0.5, 1)),
        "film_render.grain.v3_event_area_um2" => Some((0.1, 8.0, 0.05, 2)),
        "film_render.grain.rms_granularity" => Some((0.0, 1000000.0, 1.0, 2)),
        // Engine-shared fields: the single range is the union of the former
        // per-engine editor bounds.
        "film_render.grain.particle_scale_sublayers" => Some((0.0, 1000000.0, 0.25, 2)),
        "film_render.grain.density_min" => Some((-1000000.0, 1000000.0, 1.0, 2)),
        "film_render.grain.uniformity" => Some((-1000000.0, 1000000.0, 0.01, 2)),
        "film_render.grain.blur" => Some((0.0, 3.0, 0.05, 2)),
        "film_render.grain.blur_dye_clouds_um" => Some((0.0, 10.0, 0.1, 2)),
        "film_render.grain.mult_usm_amount" => Some((0.0, 1000000.0, 0.1, 2)),
        "film_render.grain.mult_usm_sigma" => Some((0.0, 1000000.0, 0.1, 2)),
        "film_render.grain.micro_structure" => Some((0.0, 1000000.0, 0.1, 2)),

        // Film grain Fotufilm
        "film_render.grain.fotufilm.amount" => Some((0.0, 4.0, 0.05, 2)),
        "film_render.grain.fotufilm.size" => Some((0.5, 4.0, 0.05, 2)),
        "film_render.grain.fotufilm.softness" => Some((0.5, 4.0, 0.05, 2)),
        "film_render.grain.fotufilm.color" => Some((0.0, 1.0, 0.01, 2)),
        "film_render.grain.fotufilm.layers" => Some((0.0, 4.0, 0.05, 2)),
        "film_render.grain.fotufilm.rms_density" => Some((f64::MIN_POSITIVE, 0.1, 0.001, 6)),
        "film_render.grain.fotufilm.clump_radius_um" => Some((f64::MIN_POSITIVE, 32.0, 0.1, 3)),
        "film_render.grain.fotufilm.layer_weights" => Some((f64::MIN_POSITIVE, 4.0, 0.05, 6)),
        "film_render.grain.fotufilm.layer_size_ratio" => Some((0.25, 4.0, 0.05, 2)),
        "film_render.grain.fotufilm.radius_span" => Some((1.0, 20.0, 0.1, 1)),
        "film_render.grain.fotufilm.sublayer_shares" => Some((1e-9, 1.0, 0.01, 6)),
        "film_render.grain.fotufilm.coating_density_scale" => Some((0.25, 4.0, 0.05, 2)),
        "film_render.grain.fotufilm.fog_density" => Some((0.0, 0.3, 0.005, 3)),

        // DIR couplers
        "film_render.dir_couplers.amount" => Some((0.0, 10.0, 0.05, 2)),
        "film_render.dir_couplers.inhibition_samelayer" => Some((0.0, 10.0, 0.05, 2)),
        "film_render.dir_couplers.inhibition_interlayer" => Some((0.0, 10.0, 0.05, 2)),
        "film_render.dir_couplers.diffusion_size_um" => Some((0.0, 50.0, 5.0, 2)),
        "film_render.dir_couplers.diffusion_tail_um" => Some((0.0, 50.0, 10.0, 2)),
        "film_render.dir_couplers.diffusion_tail_weight" => Some((0.0, 1.0, 0.01, 2)),
        "film_render.dir_couplers.gamma_samelayer_rgb" => Some((0.0, 1000000.0, 0.02, 2)),
        "film_render.dir_couplers.gamma_interlayer_r_to_gb" => Some((0.0, 1000000.0, 0.02, 2)),
        "film_render.dir_couplers.gamma_interlayer_g_to_rb" => Some((0.0, 1000000.0, 0.02, 2)),
        "film_render.dir_couplers.gamma_interlayer_b_to_rg" => Some((0.0, 1000000.0, 0.02, 2)),
        "film_render.dir_couplers.langmuir_donor_k_rgb" => Some((0.1, 1000000.0, 0.1, 2)),
        "film_render.dir_couplers.langmuir_receiver_k_rgb" => Some((0.1, 1000000.0, 0.1, 2)),

        // Halation
        "film_render.halation.halation_amount" => Some((0.0, 1.0, 0.05, 2)),
        "film_render.halation.halation_bounce_decay" => Some((0.0, 1.0, 0.05, 2)),
        "film_render.halation.halation_n_bounces" => Some((1.0, 5.0, 1.0, 2)),
        "film_render.halation.halation_spatial_scale" => Some((0.0, 100.0, 0.1, 2)),
        "film_render.halation.boost_ev" => Some((0.0, 5.0, 0.5, 2)),
        "film_render.halation.boost_range" => Some((0.0, 1.0, 0.05, 2)),
        "film_render.halation.protect_ev" => Some((0.0, 5.0, 0.5, 2)),
        "film_render.halation.scatter_amount" => Some((0.0, 1.0, 0.05, 2)),
        "film_render.halation.scatter_spatial_scale" => Some((0.0, 100.0, 0.1, 2)),
        "film_render.halation.scatter_core_um" => Some((0.0, 1000000.0, 0.5, 2)),
        "film_render.halation.scatter_tail_um" => Some((0.0, 1000000.0, 1.0, 2)),
        "film_render.halation.scatter_tail_weight" => Some((0.0, 1.0, 0.01, 2)),
        "film_render.halation.halation_strength" => Some((0.0, 1000000.0, 0.005, 3)),
        "film_render.halation.halation_first_sigma_um" => Some((0.0, 1000000.0, 1.0, 2)),

        // IO
        "io.input_gamut_compress.hull_detail" => Some((0.5, 48.0, 0.5, 1)),
        "io.upscale_factor" => Some((0.0, 4.0, 0.5, 2)),
        "io.input_gamut_compress.knee" => Some((0.0, 1000000.0, 0.05, 3)),
        "io.output_gamut_compress.knee" => Some((0.0, 1000000.0, 0.05, 3)),
        "io.crop_center" => Some((0.0, 1.0, 0.01, 2)),
        "io.crop_size" => Some((0.0, 1.0, 0.01, 2)),

        // Output mix
        "output_mix.amount" => Some((0.0, 1.0, 0.01, 3)),

        // Print base
        "print_render.base.scale" => Some((0.0, 2.0, 0.05, 2)),
        "print_render.base.cyan" => Some((0.0, 2.0, 0.05, 2)),
        "print_render.base.magenta" => Some((0.0, 2.0, 0.05, 2)),
        "print_render.base.yellow" => Some((0.0, 2.0, 0.05, 2)),

        // Print chemistry
        "print_render.density_curves_morph.gamma_factor" => Some((0.25, 4.0, 0.05, 2)),
        "print_render.density_curves_morph.gamma_factor_fast" => Some((0.25, 4.0, 0.05, 2)),
        "print_render.density_curves_morph.gamma_factor_slow" => Some((0.25, 4.0, 0.05, 2)),
        "print_render.density_curves_morph.gamma_factor_red" => Some((0.25, 4.0, 0.02, 2)),
        "print_render.density_curves_morph.gamma_factor_green" => Some((0.25, 4.0, 0.02, 2)),
        "print_render.density_curves_morph.gamma_factor_blue" => Some((0.25, 4.0, 0.02, 2)),
        "print_render.density_curves_morph.developer_exhaustion" => Some((0.0, 1.0, 0.02, 2)),

        // Print glare
        "print_render.glare.percent" => Some((0.0, 1.0, 0.01, 2)),
        "print_render.glare.roughness" => Some((0.0, 1.0, 0.05, 2)),
        "print_render.glare.blur" => Some((0.0, 10.0, 0.1, 2)),

        // Scanner
        "scanner.lens_blur" => Some((0.0, 5.0, 0.05, 2)),
        "scanner.white_level" => Some((0.0, 1.0, 0.005, 3)),
        "scanner.black_level" => Some((0.0, 1.0, 0.005, 3)),
        "scanner.finish_strength" => Some((0.0, 1.0, 0.01, 3)),
        "scanner.reference_exposure_ev" => Some((-16.0, 16.0, 0.05, 3)),
        "scanner.positive_density_ev" => Some((-16.0, 16.0, 0.05, 3)),
        "scanner.positive_contrast" => Some((0.0001, 16.0, 0.02, 3)),
        "scanner.positive_dmax" => Some((0.0001, 16.0, 0.02, 3)),
        "scanner.black_point" => Some((0.0, 1.0, 0.005, 3)),
        "scanner.white_point" => Some((0.0, 1.0, 0.005, 3)),
        "scanner.reference_balance_ev" => Some((-16.0, 16.0, 0.05, 3)),
        "scanner.positive_balance_ev" => Some((-16.0, 16.0, 0.05, 3)),
        "scanner.unsharp_mask" => Some((0.0, 1000000.0, 0.05, 2)),

        // Settings
        "settings.preview_max_size" => Some((128.0, 4096.0, 128.0, 2)),
        "settings.spectral_gaussian_blur" => Some((0.0, 10.0, 0.1, 2)),

        // Extras (GUI-managed state outside the parameter document)
        "extras.load_raw.temperature" => Some((1000.0, 1000000.0, 100.0, 2)),
        "extras.load_raw.tint" => Some((0.0, 1000000.0, 0.01, 2)),
        "extras.special.film_channel_swap" => Some((0.0, 2.0, 1.0, 0)),
        "extras.special.print_channel_swap" => Some((0.0, 2.0, 1.0, 0)),

        // Display (viewer-only settings)
        "display.white_padding" => Some((0.0, 1.0, 0.01, 2)),

        _ => None,
    }
}

/// All known numeric parameter paths.
pub const ALL_PATHS: &[&str] = &[
    "camera.exposure_compensation_ev",
    "camera.film_format_mm",
    "enlarger.print_exposure",
    "enlarger.preflash_exposure",
    "enlarger.y_filter_shift",
    "enlarger.m_filter_shift",
    "enlarger.preflash_y_filter_shift",
    "enlarger.preflash_m_filter_shift",
    "magazine_print_color.strength",
    "camera.diffusion_filter.strength",
    "camera.diffusion_filter.spatial_scale",
    "camera.diffusion_filter.halo_warmth",
    "camera.diffusion_filter.core_intensity",
    "camera.diffusion_filter.core_size",
    "camera.diffusion_filter.halo_intensity",
    "camera.diffusion_filter.halo_size",
    "camera.diffusion_filter.bloom_intensity",
    "camera.diffusion_filter.bloom_size",
    "enlarger.diffusion_filter.strength",
    "enlarger.diffusion_filter.spatial_scale",
    "enlarger.diffusion_filter.halo_warmth",
    "enlarger.diffusion_filter.core_intensity",
    "enlarger.diffusion_filter.core_size",
    "enlarger.diffusion_filter.halo_intensity",
    "enlarger.diffusion_filter.halo_size",
    "enlarger.diffusion_filter.bloom_intensity",
    "enlarger.diffusion_filter.bloom_size",
    "film_render.base.scale",
    "film_render.base.tilt",
    "film_render.base.cyan",
    "film_render.base.magenta",
    "film_render.base.yellow",
    "film_render.chemistry.gamma_factor",
    "film_render.chemistry.gamma_factor_fast",
    "film_render.chemistry.gamma_factor_slow",
    "film_render.chemistry.gamma_factor_red",
    "film_render.chemistry.gamma_factor_green",
    "film_render.chemistry.gamma_factor_blue",
    "film_render.chemistry.developer_exhaustion",
    "film_render.convert.base_percentile",
    "film_render.convert.exposure_compensation_ev",
    "film_render.grain.v2_size",
    "film_render.grain.v2_shadows",
    "film_render.grain.v2_midtones",
    "film_render.grain.v2_highlights",
    "film_render.grain.v2_resolution_factor",
    "film_render.grain.v2_chroma",
    "film_render.grain.v2_amount",
    "film_render.grain.v2_resolution_type",
    "film_render.grain.v2_timer",
    "film_render.grain.v3_dye_support_um",
    "film_render.grain.v3_event_area_um2",
    "film_render.grain.rms_granularity",
    "film_render.grain.particle_scale_sublayers",
    "film_render.grain.density_min",
    "film_render.grain.uniformity",
    "film_render.grain.blur",
    "film_render.grain.blur_dye_clouds_um",
    "film_render.grain.mult_usm_amount",
    "film_render.grain.mult_usm_sigma",
    "film_render.grain.micro_structure",
    "film_render.grain.fotufilm.amount",
    "film_render.grain.fotufilm.size",
    "film_render.grain.fotufilm.softness",
    "film_render.grain.fotufilm.color",
    "film_render.grain.fotufilm.layers",
    "film_render.grain.fotufilm.rms_density",
    "film_render.grain.fotufilm.clump_radius_um",
    "film_render.grain.fotufilm.layer_weights",
    "film_render.grain.fotufilm.layer_size_ratio",
    "film_render.grain.fotufilm.radius_span",
    "film_render.grain.fotufilm.sublayer_shares",
    "film_render.grain.fotufilm.coating_density_scale",
    "film_render.grain.fotufilm.fog_density",
    "film_render.dir_couplers.amount",
    "film_render.dir_couplers.inhibition_samelayer",
    "film_render.dir_couplers.inhibition_interlayer",
    "film_render.dir_couplers.diffusion_size_um",
    "film_render.dir_couplers.diffusion_tail_um",
    "film_render.dir_couplers.diffusion_tail_weight",
    "film_render.dir_couplers.gamma_samelayer_rgb",
    "film_render.dir_couplers.gamma_interlayer_r_to_gb",
    "film_render.dir_couplers.gamma_interlayer_g_to_rb",
    "film_render.dir_couplers.gamma_interlayer_b_to_rg",
    "film_render.dir_couplers.langmuir_donor_k_rgb",
    "film_render.dir_couplers.langmuir_receiver_k_rgb",
    "film_render.halation.halation_amount",
    "film_render.halation.halation_bounce_decay",
    "film_render.halation.halation_n_bounces",
    "film_render.halation.halation_spatial_scale",
    "film_render.halation.boost_ev",
    "film_render.halation.boost_range",
    "film_render.halation.protect_ev",
    "film_render.halation.scatter_amount",
    "film_render.halation.scatter_spatial_scale",
    "film_render.halation.scatter_core_um",
    "film_render.halation.scatter_tail_um",
    "film_render.halation.scatter_tail_weight",
    "film_render.halation.halation_strength",
    "film_render.halation.halation_first_sigma_um",
    "io.input_gamut_compress.hull_detail",
    "io.upscale_factor",
    "io.input_gamut_compress.knee",
    "io.output_gamut_compress.knee",
    "io.crop_center",
    "io.crop_size",
    "output_mix.amount",
    "print_render.base.scale",
    "print_render.base.cyan",
    "print_render.base.magenta",
    "print_render.base.yellow",
    "print_render.density_curves_morph.gamma_factor",
    "print_render.density_curves_morph.gamma_factor_fast",
    "print_render.density_curves_morph.gamma_factor_slow",
    "print_render.density_curves_morph.gamma_factor_red",
    "print_render.density_curves_morph.gamma_factor_green",
    "print_render.density_curves_morph.gamma_factor_blue",
    "print_render.density_curves_morph.developer_exhaustion",
    "print_render.glare.percent",
    "print_render.glare.roughness",
    "print_render.glare.blur",
    "scanner.lens_blur",
    "scanner.white_level",
    "scanner.black_level",
    "scanner.finish_strength",
    "scanner.reference_exposure_ev",
    "scanner.positive_density_ev",
    "scanner.positive_contrast",
    "scanner.positive_dmax",
    "scanner.black_point",
    "scanner.white_point",
    "scanner.reference_balance_ev",
    "scanner.positive_balance_ev",
    "scanner.unsharp_mask",
    "settings.preview_max_size",
    "settings.spectral_gaussian_blur",
    "extras.load_raw.temperature",
    "extras.load_raw.tint",
    "extras.special.film_channel_swap",
    "extras.special.print_channel_swap",
    "display.white_padding",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// String literal contents of `source`, ignoring comments and char literals.
    fn string_literals(source: &str) -> Vec<&str> {
        let bytes = source.as_bytes();
        let mut literals = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => {
                    let start = i + 1;
                    let mut end = start;
                    while end < bytes.len() && bytes[end] != b'"' {
                        end += if bytes[end] == b'\\' { 2 } else { 1 };
                    }
                    literals.push(&source[start..end]);
                    i = end + 1;
                }
                b'\'' => {
                    // Char literal, or a lifetime that never contains quotes.
                    i += if bytes.get(i + 1) == Some(&b'\\') {
                        4
                    } else if bytes.get(i + 2) == Some(&b'\'') {
                        3
                    } else {
                        1
                    };
                }
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    i = source[i..].find('\n').map_or(bytes.len(), |n| i + n);
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    i = source[i + 2..]
                        .find("*/")
                        .map_or(bytes.len(), |n| i + 2 + n + 2);
                }
                _ => i += 1,
            }
        }
        literals
    }

    /// A parameter path: at least two dot-separated segments that start
    /// with a lowercase letter.
    fn is_parameter_path(literal: &str) -> bool {
        let mut segments = literal.split('.');
        let mut count = 0;
        for segment in &mut segments {
            count += 1;
            if !segment.starts_with(|ch: char| ch.is_ascii_lowercase())
                || !segment
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
            {
                return false;
            }
        }
        count > 1
    }

    #[test]
    fn all_paths_have_entries() {
        for path in ALL_PATHS {
            assert!(
                editor_range(path).is_some(),
                "missing editor range for {path}"
            );
        }
    }

    /// Every parameter path a GUI editor passes to a `param_*` wrapper must
    /// have a table entry and be listed in `ALL_PATHS`, and every table path
    /// must be used by an editor, so absence is caught at test time instead
    /// of at the first frame that draws the editor.
    #[test]
    fn every_gui_editor_path_resolves() {
        // Attribute arguments such as `#[path = "controls_acceptance.rs"]`
        // name source files, not parameter paths.
        let code: Vec<String> = [include_str!("controls.rs"), include_str!("display.rs")]
            .into_iter()
            .map(|source| {
                source
                    .lines()
                    .filter(|line| !line.trim_start().starts_with("#["))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect();
        let mut used = Vec::new();
        for source in &code {
            for literal in string_literals(source) {
                if is_parameter_path(literal) {
                    used.push(literal);
                }
            }
        }
        assert!(
            used.len() > 100,
            "editor path scan found only {} paths",
            used.len()
        );
        for path in &used {
            assert!(
                editor_range(path).is_some(),
                "editor path {path} has no editor range"
            );
            assert!(
                ALL_PATHS.contains(path),
                "editor path {path} is not listed in ALL_PATHS"
            );
        }
        for path in ALL_PATHS {
            assert!(
                used.contains(path),
                "ALL_PATHS entry {path} is not used by any GUI editor"
            );
        }
    }

    #[test]
    fn ranges_are_valid() {
        for path in ALL_PATHS {
            if let Some((min, max, step, decimals)) = editor_range(path) {
                assert!(min < max, "{path}: min >= max");
                assert!(step > 0.0, "{path}: step <= 0");
                assert!(decimals <= 6, "{path}: decimals > 6");
            }
        }
    }

    #[test]
    fn film_format_mm_matches_spec() {
        // The spec model: editor 1-1000, validator strictly positive
        assert_eq!(
            editor_range("camera.film_format_mm"),
            Some((1.0, 1000.0, 0.5, 2))
        );
    }
}
