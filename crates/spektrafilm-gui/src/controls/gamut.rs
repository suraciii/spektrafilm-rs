use super::*;
use egui::Ui;

/// One `show()` section group, split out of the dispatcher for readability.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    section: &str,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    if section == "Input gamut compress" {
        section_header(ui, filter, "Input gamut compress", |ui, filter| {
            if filter_matches(filter, "enable input gamut compress") {
                flags.runtime_changed |= toggle(
                    ui,
                    "Enable input gamut compress",
                    &mut params.io.input_gamut_compress.active,
                    "Compress input chromaticities toward the visible spectral locus before spectral upsampling. Off passes input through unchanged.",
                );
            }
            if filter_matches(filter, "algorithm") {
                flags.runtime_changed |= choice_tip_experimental(
                    ui,
                    "algorithm",
                    &mut params.io.input_gamut_compress.algorithm,
                    &["xy", "oklch", "off"],
                    "xy: radial compression in CIE 1931 chromaticity toward the spectral locus / inscribed hull (ACES RGC family, preserves dominant wavelength). oklch: perceptual chroma reduction in OkLab. off: disable input gamut compression.",
                );
            }
            if filter_matches(filter, "boundary") {
                flags.runtime_changed |= choice_tip(
                    ui,
                    "boundary",
                    &mut params.io.input_gamut_compress.boundary,
                    &["locus", "inscribed_hull"],
                    "Use the visible spectral locus or its smoothed inscribed hull as the input gamut boundary.",
                );
            }
            if filter_matches(filter, "knee") {
                flags.runtime_changed |= param_tuple(
                    ui,
                    "knee",
                    &mut params.io.input_gamut_compress.knee,
                    "io.input_gamut_compress.knee",
                    "Reinhard knee (threshold, limit, power). Default (0.815, 1.0, 1.2) — ACES RGC pair: the 0.815 threshold leaves in-gamut colours untouched, the 1.2 power gives a slow tail so deep-imaginary colours keep their gradation, and limit 1.0 asymptotes on the boundary.",
                );
            }
            if filter_matches(filter, "hull detail") {
                flags.runtime_changed |= param_numeric(
                    ui,
                    "hull detail",
                    &mut params.io.input_gamut_compress.hull_detail,
                    "io.input_gamut_compress.hull_detail",
                    "Detail of the inscribed-hull compression boundary = number of FFT modes kept. HIGHER keeps more locus detail (more cusp chroma) but re-admits polygon kinks; LOWER is smoother, rounding the spectral tips inward toward a circle. Note: this is modes-kept, not a blur width — larger means LESS smoothing.",
                );
            }
        });
    }
    if section == "Output gamut compress" {
        section_header(ui, filter, "Output gamut compress", |ui, filter| {
            if filter_matches(filter, "algorithm") {
                flags.runtime_changed |= choice_tip(
                    ui,
                    "algorithm",
                    &mut params.io.output_gamut_compress.algorithm,
                    &["off", "oklch", "aces_rgc", "oklrab", "jzazbz", "cam16ucs"],
                    "off: disable output gamut compression. aces_rgc (default): per-channel ACES RGC v1.3, matches Resolve/Nuke/OCIO and is the cheapest per pixel. cam16ucs: CAM16-UCS chroma reduction with the smoothest constant-hue behavior (heaviest). oklch: perceptual chroma reduction in OkLab, preserves hue + lightness. oklrab/jzazbz: alternative perceptual spaces.",
                );
            }
            if filter_matches(filter, "knee") {
                flags.runtime_changed |= param_tuple(
                    ui,
                    "knee",
                    &mut params.io.output_gamut_compress.knee,
                    "io.output_gamut_compress.knee",
                    "Reinhard knee (threshold, limit, power) on normalized chroma C/C_max. Default (0.0, 1.0, 6.0) rolls off smoothly from the center with the asymptote on the output cube edge.",
                );
            }
        });
    }
}
