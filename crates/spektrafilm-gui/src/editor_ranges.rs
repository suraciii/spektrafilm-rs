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

        // DIR couplers
        "film_render.dir_couplers.amount" => Some((0.0, 10.0, 0.05, 2)),
        "film_render.dir_couplers.inhibition_samelayer" => Some((0.0, 10.0, 0.05, 2)),
        "film_render.dir_couplers.inhibition_interlayer" => Some((0.0, 10.0, 0.05, 2)),
        "film_render.dir_couplers.diffusion_size_um" => Some((0.0, 50.0, 5.0, 2)),
        "film_render.dir_couplers.diffusion_tail_um" => Some((0.0, 50.0, 10.0, 2)),
        "film_render.dir_couplers.diffusion_tail_weight" => Some((0.0, 1.0, 0.01, 2)),

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

        // IO
        "io.input_gamut_compress.hull_detail" => Some((0.5, 48.0, 0.5, 1)),
        "io.upscale_factor" => Some((0.0, 4.0, 0.5, 2)),

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

        // Settings
        "settings.preview_max_size" => Some((128.0, 4096.0, 128.0, 2)),
        "settings.spectral_gaussian_blur" => Some((0.0, 10.0, 0.1, 2)),

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
    "film_render.dir_couplers.amount",
    "film_render.dir_couplers.inhibition_samelayer",
    "film_render.dir_couplers.inhibition_interlayer",
    "film_render.dir_couplers.diffusion_size_um",
    "film_render.dir_couplers.diffusion_tail_um",
    "film_render.dir_couplers.diffusion_tail_weight",
    "film_render.halation.halation_amount",
    "film_render.halation.halation_bounce_decay",
    "film_render.halation.halation_n_bounces",
    "film_render.halation.halation_spatial_scale",
    "film_render.halation.boost_ev",
    "film_render.halation.boost_range",
    "film_render.halation.protect_ev",
    "film_render.halation.scatter_amount",
    "film_render.halation.scatter_spatial_scale",
    "io.input_gamut_compress.hull_detail",
    "io.upscale_factor",
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
    "settings.preview_max_size",
    "settings.spectral_gaussian_blur",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_paths_have_entries() {
        for path in ALL_PATHS {
            assert!(
                editor_range(path).is_some(),
                "missing editor range for {path}"
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
