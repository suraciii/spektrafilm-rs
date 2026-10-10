//! Editors for the pinned experimental GUI sections.
use crate::numeric::{numeric, numeric_field};
use egui::Ui;
use serde_json::{Map, Value};
use spektrafilm_core::params::{
    RuntimeParams,
    diffusion::DiffusionFilterParams,
    grain::{GrainEngine, GrainV2FilmType, GrainV2Mode},
};
use spektrafilm_model::grain::v2::PROFILE_NAMES;

const COLOR_SPACES: &[&str] = &[
    "sRGB",
    "DCI-P3",
    "Display P3",
    "Adobe RGB (1998)",
    "ITU-R BT.2020",
    "ProPhoto RGB",
    "ACES2065-1",
    "DaVinci Wide Gamut",
    "V-Gamut",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationAction {
    DetectBase,
    BlindCalibration,
    NeutralizeFilters,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ChangeFlags {
    /// A simulation input changed. Invalidate the applicable pipeline cache.
    pub runtime_changed: bool,
    /// Display, preview-size, or saving/auto-preview workflow state changed.
    pub display_changed: bool,
    /// Reload the selected RAW with extras.load_raw before rendering it.
    pub raw_reload: bool,
    /// The preview-size Update button was pressed, independently of auto-preview.
    pub preview_requested: bool,
    /// A real controller action was requested from the Convert panel.
    pub action: Option<CalibrationAction>,
}

pub fn show(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    extras: &mut Value,
    section: &str,
) -> ChangeFlags {
    let mut flags = ChangeFlags::default();
    ui.push_id(("supplemental_controls", section), |ui| {
        if section == "Input" {
            egui::CollapsingHeader::new("Input").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= choice_tip(ui, "input color space", &mut params.io.input_color_space, COLOR_SPACES, "Color space of the input image, will be internally converted to sRGB and negative values clipped");
                flags.runtime_changed |= toggle(ui, "apply cctf decoding", &mut params.io.input_cctf_decoding, "Apply the inverse cctf transfer function of the color space");
            });
        }
        if section == "Camera" {
            egui::CollapsingHeader::new("Camera").default_open(true).show(ui, |ui| {
                flags.runtime_changed |= numeric(ui, "exposure compensation ev", &mut params.camera.exposure_compensation_ev, -100.0, 100.0, 0.25, 2, "Add a bias to the auto-exposure of the camera.");
                flags.runtime_changed |= toggle(ui, "auto exposure", &mut params.camera.auto_exposure, "Use the auto-exposure feature of the virtual camera.");
                flags.runtime_changed |= numeric(ui, "film format mm", &mut params.camera.film_format_mm, 8.0, 120.0, 1.0, 0, "Long edge of the film format in millimeters, e.g. 8, 16, 35, 60, 120.");
                flags.runtime_changed |= choice_tip(ui, "auto exposure method", &mut params.camera.auto_exposure_method, &["center_weighted", "matrix", "multi_zone", "partial", "highlight_weighted", "median", "average"], "Metering method used by the virtual camera's auto-exposure.");
                flags.runtime_changed |= choice_tip(ui, "color filter", &mut params.camera.color_filter, &["none", "hoya_x0", "hoya_x1", "hoya_y2", "hoya_ya3", "hoya_r1"], "Camera taking filter held in front of the lens. Its measured spectral transmittance multiplies the incoming light, so it both darkens and color-casts the exposure (e.g. Hoya Y2/YA3 yellow, R1 red, X0/X1 UV/haze). 'none' = no filter.");
            });
        }
        if section == "Enlarger" {
            egui::CollapsingHeader::new("Enlarger").default_open(true).show(ui, |ui| {
                flags.runtime_changed |= numeric(ui, "print exposure", &mut params.enlarger.print_exposure, 0.0, 1000000.0, 0.02, 2, "Changes the exposure time set in the virtual enlarger");
                flags.runtime_changed |= toggle(ui, "print auto compensation", &mut params.enlarger.print_exposure_compensation, "Auto adjust the print exposure for the camera exposure compensation ev");
                flags.runtime_changed |= numeric(ui, "print y filter shift", &mut params.enlarger.y_filter_shift, -200.0, 1000000.0, 1.0, 2, "Y filter shift of the color enlarger from a neutral position, in Kodak CC units");
                flags.runtime_changed |= numeric(ui, "print m filter shift", &mut params.enlarger.m_filter_shift, -200.0, 1000000.0, 1.0, 2, "M filter shift of the color enlarger from a neutral position, in Kodak CC units");
            });
        }
        if section == "Scanner" {
            egui::CollapsingHeader::new("Scanner").default_open(false).show(ui, |ui| {
                if params.workflow.route == "input > film > scan" {
                    flags.runtime_changed |= choice_tip(
                        ui,
                        "scan output",
                        &mut params.scanner.scan_output,
                        &["direct_scan", "positive_scan"],
                        "direct_scan preserves the negative scanner capture; positive_scan interprets a direct negative-film scan as a positive image.",
                    );
                }
                flags.runtime_changed |= numeric(ui, "lens blur", &mut params.scanner.lens_blur, 0.0, 1000000.0, 0.05, 2, "Sigma of gaussian filter in pixel for the scanner lens blur.");
                flags.runtime_changed |= toggle(ui, "white correction", &mut params.scanner.white_correction, "Enable white point correction applied to the scanner output.");
                flags.runtime_changed |= numeric(ui, "white level", &mut params.scanner.white_level, 0.0, 1.0, 0.005, 3, "Target white level applied when white correction is enabled.");
                flags.runtime_changed |= toggle(ui, "black correction", &mut params.scanner.black_correction, "Enable black point correction applied to the scanner output.");
                flags.runtime_changed |= numeric(ui, "black level", &mut params.scanner.black_level, 0.0, 1.0, 0.005, 3, "Target black level applied when black correction is enabled.");
                flags.runtime_changed |= tuple(ui, "unsharp mask", &mut params.scanner.unsharp_mask, 0.0, 1000000.0, 0.05, 2, "Apply unsharp mask to the scan, [sigma in pixel, amount].");
            });
        }
        if section == "Film chemistry" {
            flags.runtime_changed |= toggle(ui, "active", &mut params.film_render.chemistry.active, "Enable film density-curve morphing.");
            flags.runtime_changed |= numeric(ui, "gamma factor", &mut params.film_render.chemistry.gamma_factor, 0.25, 4.0, 0.05, 2, "Global coupled gamma multiplier for the film density curves.");
            flags.runtime_changed |= numeric(ui, "gamma factor fast", &mut params.film_render.chemistry.gamma_factor_fast, 0.25, 4.0, 0.05, 2, "Gamma factor applied to the fast sub-layer.");
            flags.runtime_changed |= numeric(ui, "gamma factor slow", &mut params.film_render.chemistry.gamma_factor_slow, 0.25, 4.0, 0.05, 2, "Gamma factor applied to the mid and slow sub-layers.");
            flags.runtime_changed |= numeric(ui, "gamma factor red", &mut params.film_render.chemistry.gamma_factor_red, 0.25, 4.0, 0.02, 2, "Per-channel gamma factor applied to the red channel.");
            flags.runtime_changed |= numeric(ui, "gamma factor green", &mut params.film_render.chemistry.gamma_factor_green, 0.25, 4.0, 0.02, 2, "Per-channel gamma factor applied to the green channel.");
            flags.runtime_changed |= numeric(ui, "gamma factor blue", &mut params.film_render.chemistry.gamma_factor_blue, 0.25, 4.0, 0.02, 2, "Per-channel gamma factor applied to the blue channel.");
            flags.runtime_changed |= numeric(ui, "developer exhaustion", &mut params.film_render.chemistry.developer_exhaustion, 0.0, 1.0, 0.02, 2, "Blend all three film sub-layers toward the matched Gumbel shoulder while preserving midgray via a common horizontal offset.");
        }
        if section == "Print chemistry" {
            flags.runtime_changed |= toggle(ui, "active", &mut params.print_render.density_curves_morph.active, "Enable print density-curve morphing.");
            flags.runtime_changed |= numeric(ui, "gamma factor", &mut params.print_render.density_curves_morph.gamma_factor, 0.25, 4.0, 0.05, 2, "Global coupled gamma multiplier for the print density curves.");
            flags.runtime_changed |= numeric(ui, "gamma factor fast", &mut params.print_render.density_curves_morph.gamma_factor_fast, 0.25, 4.0, 0.05, 2, "Gamma factor applied to the fast sub-layer.");
            flags.runtime_changed |= numeric(ui, "gamma factor slow", &mut params.print_render.density_curves_morph.gamma_factor_slow, 0.25, 4.0, 0.05, 2, "Gamma factor applied to the mid and slow sub-layers.");
            flags.runtime_changed |= numeric(ui, "gamma factor red", &mut params.print_render.density_curves_morph.gamma_factor_red, 0.25, 4.0, 0.02, 2, "Per-channel gamma factor applied to the red channel.");
            flags.runtime_changed |= numeric(ui, "gamma factor green", &mut params.print_render.density_curves_morph.gamma_factor_green, 0.25, 4.0, 0.02, 2, "Per-channel gamma factor applied to the green channel.");
            flags.runtime_changed |= numeric(ui, "gamma factor blue", &mut params.print_render.density_curves_morph.gamma_factor_blue, 0.25, 4.0, 0.02, 2, "Per-channel gamma factor applied to the blue channel.");
            flags.runtime_changed |= numeric(ui, "developer exhaustion", &mut params.print_render.density_curves_morph.developer_exhaustion, 0.0, 1.0, 0.02, 2, "Blend all three print sub-layers toward the matched Gumbel shoulder while preserving midgray via a common horizontal offset.");
        }
        if section == "Preflash" {
            egui::CollapsingHeader::new("Preflash").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= numeric(ui, "exposure", &mut params.enlarger.preflash_exposure, 0.0, 1000000.0, 0.005, 2, "Preflash exposure value in ev for the print");
                flags.runtime_changed |= numeric(ui, "y filter shift", &mut params.enlarger.preflash_y_filter_shift, -1000000.0, 1000000.0, 1.0, 2, "Shift the Y filter of the enlarger from the neutral position for the preflash, typical values (-20-20), in Kodak CC units");
                flags.runtime_changed |= numeric(ui, "m filter shift", &mut params.enlarger.preflash_m_filter_shift, -1000000.0, 1000000.0, 1.0, 2, "Shift the M filter of the enlarger from the neutral position for the preflash, typical values (-20-20), in Kodak CC units");
            });
        }
        if section == "Glare" {
            egui::CollapsingHeader::new("Glare").default_open(false).show(ui, |ui| {
                ui.add_enabled_ui(params.workflow.route.contains(" > print > "), |ui| {
                    flags.runtime_changed |= toggle(ui, "active", &mut params.print_render.glare.active, "Add glare to the print");
                    flags.runtime_changed |= numeric(ui, "percent", &mut params.print_render.glare.percent, 0.0, 1.0, 0.01, 2, "Percentage of the glare light (typically 0.1-0.25)");
                    flags.runtime_changed |= numeric(ui, "roughness", &mut params.print_render.glare.roughness, 0.0, 1.0, 0.05, 2, "Roughness of the glare light (0-1)");
                    flags.runtime_changed |= numeric(ui, "blur", &mut params.print_render.glare.blur, 0.0, 1000000.0, 0.1, 2, "Sigma of gaussian blur in pixels for the glare");
                });
            });
        }
        if section == "Magazine print color" {
            ui.collapsing(section, |ui| {
                let magazine = &mut params.magazine_print_color;
                flags.runtime_changed |= ui.checkbox(&mut magazine.active, "Active").changed();
                flags.runtime_changed |= numeric(
                    ui,
                    "strength",
                    &mut magazine.strength,
                    0.0,
                    1.0,
                    0.01,
                    2,
                    "Magazine print color appearance strength.",
                );
            });
        }
        if section == "Input gamut compress" {
            egui::CollapsingHeader::new("Input gamut compress").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= toggle(ui, "active", &mut params.io.input_gamut_compress.active, "Compress input chromaticities toward the visible spectral locus before spectral upsampling. Off passes input through unchanged.");
                flags.runtime_changed |= choice_tip(ui, "algorithm", &mut params.io.input_gamut_compress.algorithm, &["xy"], "xy: radial compression in CIE 1931 chromaticity toward the spectral locus / inscribed hull (ACES RGC family, preserves dominant wavelength). The only input algorithm.");
                flags.runtime_changed |= tuple(ui, "knee", &mut params.io.input_gamut_compress.knee, 0.0, 1000000.0, 0.05, 3, "Reinhard knee (threshold, limit, power). Default (0.815, 1.0, 1.2) — ACES RGC pair: the 0.815 threshold leaves in-gamut colours untouched, the 1.2 power gives a slow tail so deep-imaginary colours keep their gradation, and limit 1.0 asymptotes on the boundary.");
                flags.runtime_changed |= numeric(ui, "hull detail", &mut params.io.input_gamut_compress.hull_detail, 0.5, 48.0, 0.5, 1, "Detail of the inscribed-hull compression boundary = number of FFT modes kept. HIGHER keeps more locus detail (more cusp chroma) but re-admits polygon kinks; LOWER is smoother, rounding the spectral tips inward toward a circle. Note: this is modes-kept, not a blur width — larger means LESS smoothing.");
            });
        }
        if section == "Output gamut compress" {
            egui::CollapsingHeader::new("Output gamut compress").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= choice_tip(ui, "algorithm", &mut params.io.output_gamut_compress.algorithm, &["off", "oklch", "aces_rgc", "oklrab", "jzazbz", "cam16ucs"], "off: disable output gamut compression. aces_rgc (default): per-channel ACES RGC v1.3, matches Resolve/Nuke/OCIO and is the cheapest per pixel. cam16ucs: CAM16-UCS chroma reduction with the smoothest constant-hue behavior (heaviest). oklch: perceptual chroma reduction in OkLab, preserves hue + lightness. oklrab/jzazbz: alternative perceptual spaces.");
                flags.runtime_changed |= tuple(ui, "knee", &mut params.io.output_gamut_compress.knee, 0.0, 1000000.0, 0.05, 3, "Reinhard knee (threshold, limit, power) on normalized chroma C/C_max. Default (0.0, 1.0, 6.0) rolls off smoothly from the center with the asymptote on the output cube edge.");
            });
        }
        if section == "Crop and upscale" {
            egui::CollapsingHeader::new("Crop and upscale").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= numeric(ui, "upscale factor", &mut params.io.upscale_factor, 0.0, 1000000.0, 0.5, 2, "Scale image size up to increase resolution");
                flags.runtime_changed |= toggle(ui, "crop", &mut params.io.crop, "Crop image to a fraction of the original size to preview details at full scale");
                flags.runtime_changed |= tuple(ui, "crop center", &mut params.io.crop_center, 0.0, 1.0, 0.01, 2, "Center of the crop region in relative coordinates in x, y (0-1)");
                flags.runtime_changed |= tuple(ui, "crop size", &mut params.io.crop_size, 0.0, 1.0, 0.01, 2, "Normalized size of the crop region in x, y (0,1), as fraction of the long side.");
            });
        }
        if section == "Film base" {
            egui::CollapsingHeader::new("Base").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= toggle(ui, "active", &mut params.film_render.base.active, "Enable film base-density (film base + fog / orange mask) tuning.");
                flags.runtime_changed |= numeric(ui, "scale", &mut params.film_render.base.scale, 0.0, 1000000.0, 0.05, 2, "Overall multiplier on the film base density.");
                flags.runtime_changed |= numeric(ui, "tilt", &mut params.film_render.base.tilt, -1000000.0, 1000000.0, 0.05, 2, "Spectral tilt of the film base, pivoting at 555 nm (1.0 = +0.1 density at 650 nm; negative tilts the other way).");
                flags.runtime_changed |= numeric(ui, "cyan", &mut params.film_render.base.cyan, 0.0, 1000000.0, 0.05, 2, "Cyan-channel film base density shift around 650 nm (1.0 = neutral; adds (value - 1) density).");
                flags.runtime_changed |= numeric(ui, "magenta", &mut params.film_render.base.magenta, 0.0, 1000000.0, 0.05, 2, "Magenta-channel film base density shift around 555 nm (1.0 = neutral; adds (value - 1) density).");
                flags.runtime_changed |= numeric(ui, "yellow", &mut params.film_render.base.yellow, 0.0, 1000000.0, 0.05, 2, "Yellow-channel film base density shift around 460 nm (1.0 = neutral; adds (value - 1) density).");
            });
        }
        if section == "Print base" {
            egui::CollapsingHeader::new("Base").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= toggle(ui, "active", &mut params.print_render.base.active, "Enable print base-density tuning.");
                flags.runtime_changed |= numeric(ui, "scale", &mut params.print_render.base.scale, 0.0, 1000000.0, 0.05, 2, "Overall multiplier on the print base density.");
                flags.runtime_changed |= numeric(ui, "cyan", &mut params.print_render.base.cyan, 0.0, 1000000.0, 0.05, 2, "Cyan-channel multiplicative scale of the print base min around 610 nm (1.0 = neutral).");
                flags.runtime_changed |= numeric(ui, "magenta", &mut params.print_render.base.magenta, 0.0, 1000000.0, 0.05, 2, "Magenta-channel multiplicative scale of the print base min around 530 nm (1.0 = neutral).");
                flags.runtime_changed |= numeric(ui, "yellow", &mut params.print_render.base.yellow, 0.0, 1000000.0, 0.05, 2, "Yellow-channel multiplicative scale of the print base min around 445 nm (1.0 = neutral).");
            });
        }
        if section == "Convert" {
            egui::CollapsingHeader::new("Convert").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= choice_tip(ui, "scan illuminant", &mut params.film_render.convert.scan_illuminant, &["D50", "D55", "D65", "A", "BB3200", "BB5000", "LED-B3", "LED-B5", "LED-RGB1", "LED-V2", "FL2"], "Light source of the scanning / capture rig used to digitize the negative. Set it to match your rig (a physical input); the film Base is the creative lever. Only used by the 'convert-film' workflow routes.");
                flags.runtime_changed |= numeric(ui, "exposure compensation ev", &mut params.film_render.convert.exposure_compensation_ev, -100.0, 100.0, 0.25, 2, "Aligns the scan's overall brightness to the model (gain = 2^ev). Brighter input -> less recovered film density.");
                flags.runtime_changed |= numeric(ui, "base percentile", &mut params.film_render.convert.base_percentile, 50.0, 100.0, 0.5, 2, "Brightest-pixels percentile used by 'Detect base' to sample the clear / unexposed film (99 = brightest 1%). Detect base fits the film Base so that clear film maps to density 0 (unexposed -> neutral).");
                ui.horizontal(|ui| {
                    ui.label("calibration").on_hover_text("3x3 device-correction matrix (9 numbers, row-major) applied to the input before inversion, to undo the scanner/camera colour rendering vs the standard observer over the film dyes (the cross-channel cast that the scan illuminant and base cannot reach). Editable and copy/paste-able; an invalid string falls back to identity (no correction). Use 'Blind calibration' to fit it from the current image.");
                    flags.runtime_changed |= ui.text_edit_singleline(&mut params.film_render.convert.calibration).on_hover_text("3x3 device-correction matrix (9 numbers, row-major) applied to the input before inversion, to undo the scanner/camera colour rendering vs the standard observer over the film dyes (the cross-channel cast that the scan illuminant and base cannot reach). Editable and copy/paste-able; an invalid string falls back to identity (no correction). Use 'Blind calibration' to fit it from the current image.").changed();
                });
                ui.horizontal(|ui| {
                    if ui.button("detect base").on_hover_text("Sample the clear/unexposed film from the brightest pixels of the current image (see Base percentile) and fit the film Base + exposure so the unexposed film maps to density 0. Run this first.").clicked() {
                        flags.action = Some(CalibrationAction::DetectBase);
                    }
                    if ui.button("blind calibration").on_hover_text("Fit the Calibration matrix from the CURRENT image and write it above. Works best on a vibrant, colour-filled frame (it aligns the scanned colours to the film's dye gamut); a flat/neutral frame gives a weak fit. Review/edit the values afterwards.").clicked() {
                        flags.action = Some(CalibrationAction::BlindCalibration);
                    }
                    if ui.button("neutralize print filters").on_hover_text("Solve the print enlarger M and Y filter shifts so the current film + base midgray prints neutral. Image-independent; run it after Detect base (a tuned base changes the print balance). Writes Print M/Y filter shift in Enlarger.").clicked() {
                        flags.action = Some(CalibrationAction::NeutralizeFilters);
                    }
                });
            });
        }
        if section == "Spectral upsampling" {
            egui::CollapsingHeader::new("Spectral upsampling").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= choice_tip(ui, "spectral upsampling", &mut params.settings.rgb_to_raw_method, &["arctic2026beta04", "hanatos2025", "mallett2019"], "Method to upsample the spectral resolution of the image, hanatos2025 works on the full visible locus, mallett2019 works only on sRGB (will clip input).");
                flags.runtime_changed |= toggle(ui, "hanatos2025 adaptation window", &mut params.settings.apply_hanatos2025_adaptation_window, "Apply the hanatos2025 bandpass adaptation window when reconstructing spectra.");
                flags.runtime_changed |= toggle(ui, "hanatos2025 adaptation surface", &mut params.settings.apply_hanatos2025_adaptation_surface, "Apply the hanatos2025 surface adaptation polynomial when reconstructing spectra.");
                flags.runtime_changed |= numeric(ui, "spectral gaussian blur", &mut params.settings.spectral_gaussian_blur, 0.0, 1000000.0, 0.1, 2, "Sigma in nm for Gaussian blur applied to reconstructed spectra.");
            });
        }
        if section == "Grain" {
            egui::CollapsingHeader::new("Grain").default_open(true).show(ui, |ui| {
                let grain = &mut params.film_render.grain;
                flags.runtime_changed |= toggle(ui, "active", &mut grain.active, "Add grain to the negative");
                egui::ComboBox::from_label("Engine")
                    .selected_text(match grain.engine { GrainEngine::V1 => "V1 — emulsion grain", GrainEngine::V2 => "V2 — procedural grain" })
                    .show_ui(ui, |ui| {
                        flags.runtime_changed |= ui.selectable_value(&mut grain.engine, GrainEngine::V1, "V1 — emulsion grain").changed();
                        flags.runtime_changed |= ui.selectable_value(&mut grain.engine, GrainEngine::V2, "V2 — procedural grain").changed();
                    });
                if grain.engine == GrainEngine::V2 {
                    let mut profile = grain.v2_profile.clone();
                    egui::ComboBox::from_label("Grain Profiles").selected_text(&profile).show_ui(ui, |ui| {
                        for name in PROFILE_NAMES { ui.selectable_value(&mut profile, (*name).to_owned(), name); }
                        ui.selectable_value(&mut profile, "custom".to_owned(), "Custom");
                    });
                    if profile != grain.v2_profile {
                        if profile == "custom" { grain.select_custom_grain_v2(); } else { grain.v2_profile = profile; grain.v2_amount = None; }
                        flags.runtime_changed = true;
                    }
                    if grain.v2_profile == "custom" {
                        egui::ComboBox::from_label("Film Type")
                            .selected_text(match grain.v2_film_type { GrainV2FilmType::Negative => "Negative", GrainV2FilmType::Positive => "Positive" })
                            .show_ui(ui, |ui| {
                                flags.runtime_changed |= ui.selectable_value(&mut grain.v2_film_type, GrainV2FilmType::Negative, "Negative").changed();
                                flags.runtime_changed |= ui.selectable_value(&mut grain.v2_film_type, GrainV2FilmType::Positive, "Positive").changed();
                            });
                        egui::ComboBox::from_label("Processing Mode")
                            .selected_text(match grain.v2_mode { GrainV2Mode::Analogue => "Analogue", GrainV2Mode::Noise => "Noise" })
                            .show_ui(ui, |ui| {
                                flags.runtime_changed |= ui.selectable_value(&mut grain.v2_mode, GrainV2Mode::Analogue, "Analogue").changed();
                                flags.runtime_changed |= ui.selectable_value(&mut grain.v2_mode, GrainV2Mode::Noise, "Noise").changed();
                            });
                        let resolved = grain.resolved_grain_v2();
                        flags.runtime_changed |= optional_numeric(ui, "Size", &mut grain.v2_size, resolved.size, 1.0, 48.0, 0.5, 1, "Grain particle size.");
                        flags.runtime_changed |= optional_numeric(ui, "Shadows", &mut grain.v2_shadows, resolved.shadows * 100.0, 0.0, 100.0, 1.0, 1, "Shadow grain amount.");
                        flags.runtime_changed |= optional_numeric(ui, "Midtones", &mut grain.v2_midtones, resolved.midtones * 100.0, 0.0, 100.0, 1.0, 1, "Midtone grain amount.");
                        flags.runtime_changed |= optional_numeric(ui, "Highlights", &mut grain.v2_highlights, resolved.highlights * 100.0, 0.0, 100.0, 1.0, 1, "Highlight grain amount.");
                        flags.runtime_changed |= optional_numeric(ui, "Film Resolution", &mut grain.v2_resolution_factor, resolved.resolution_factor, 0.0, 100.0, 1.0, 1, "Film resolution factor.");
                        flags.runtime_changed |= optional_numeric(ui, "Chroma", &mut grain.v2_chroma, resolved.color * 100.0, 0.0, 100.0, 1.0, 1, "Chromatic grain amount.");
                    }
                    let resolved = grain.resolved_grain_v2();
                    flags.runtime_changed |= optional_numeric(ui, "Amount", &mut grain.v2_amount, resolved.amount * 100.0, 0.0, 100.0, 1.0, 1, "Overall grain amount.");
                } else {
                    flags.runtime_changed |= tuple(ui, "rms granularity", &mut grain.rms_granularity, 0.0, 1000000.0, 1.0, 2, "Per-channel RMS granularity.");
                    flags.runtime_changed |= tuple(ui, "density min", &mut grain.density_min, -1000000.0, 1000000.0, 1.0, 2, "Minimum grain density.");
                    flags.runtime_changed |= tuple(ui, "uniformity", &mut grain.uniformity, -1000000.0, 1000000.0, 1.0, 2, "Per-channel grain uniformity.");
                    flags.runtime_changed |= tuple(ui, "particle scale sublayers", &mut grain.particle_scale_sublayers, 0.0, 1000000.0, 0.25, 2, "Relative particle scale per emulsion sublayer.");
                    flags.runtime_changed |= numeric(ui, "blur", &mut grain.blur, 0.0, 3.0, 0.05, 2, "Post-blur sigma.");
                    flags.runtime_changed |= numeric(ui, "dye-cloud blur", &mut grain.blur_dye_clouds_um, 0.0, 10.0, 0.1, 2, "Dye-cloud blur in micrometers.");
                    flags.runtime_changed |= numeric(ui, "mult usm amount", &mut grain.mult_usm_amount, 0.0, 1000000.0, 0.1, 2, "Density unsharp-mask amount.");
                    flags.runtime_changed |= numeric(ui, "mult usm sigma", &mut grain.mult_usm_sigma, 0.0, 1000000.0, 0.1, 2, "Density unsharp-mask radius.");
                    flags.runtime_changed |= tuple(ui, "micro structure", &mut grain.micro_structure, 0.0, 1000000.0, 0.1, 2, "Micro-structure scale and clump size.");
                }
            });
        }
        if section == "Halation" {
            egui::CollapsingHeader::new("Halation").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= toggle(ui, "active", &mut params.film_render.halation.active, "Enable halation and in-emulsion scatter.");
                flags.runtime_changed |= numeric(ui, "scatter amount", &mut params.film_render.halation.scatter_amount, 0.0, 1000000.0, 0.05, 2, "High-level scatter strength. 1.0 = full physical scatter, 0.0 = no scatter. Scales the fraction of light that undergoes in-emulsion scattering.");
                flags.runtime_changed |= numeric(ui, "scatter spatial scale", &mut params.film_render.halation.scatter_spatial_scale, 0.0, 1000000.0, 0.1, 2, "High-level scatter size multiplier (1.0 = physical defaults). Scales both core and tail sigmas.");
                flags.runtime_changed |= numeric(ui, "halation amount", &mut params.film_render.halation.halation_amount, 0.0, 1000000.0, 0.05, 2, "High-level halation strength multiplier (1.0 = physical defaults). Scales the per-channel halation amplitudes.");
                flags.runtime_changed |= numeric(ui, "halation spatial scale", &mut params.film_render.halation.halation_spatial_scale, 0.0, 1000000.0, 0.1, 2, "High-level halation size multiplier (1.0 = physical defaults). Scales the first-bounce sigma.");
                flags.runtime_changed |= numeric(ui, "boost ev", &mut params.film_render.halation.boost_ev, 0.0, 1000000.0, 0.5, 2, "Maximum highlight boost in stops.");
                flags.runtime_changed |= numeric(ui, "protect ev", &mut params.film_render.halation.protect_ev, 0.0, 1000000.0, 0.5, 2, "Protected range above midgray for the boost onset in stops.");
                flags.runtime_changed |= numeric(ui, "boost range", &mut params.film_render.halation.boost_range, 0.0, 1.0, 0.05, 2, "Controls how quickly the highlight boost ramps in, from 0 to 1.");
                flags.runtime_changed |= tuple(ui, "scatter core um", &mut params.film_render.halation.scatter_core_um, 0.0, 1000000.0, 0.5, 2, "Sigma of the scatter core Gaussian per channel [R,G,B], in micrometers. Controls fine-scale sharpness loss in the emulsion.");
                flags.runtime_changed |= tuple(ui, "scatter tail um", &mut params.film_render.halation.scatter_tail_um, 0.0, 1000000.0, 1.0, 2, "Decay constant of the scatter exponential tail per channel [R,G,B], in micrometers (internally approximated by a sum of Gaussians). Controls extended low-level spread within the emulsion.");
                flags.runtime_changed |= tuple(ui, "scatter tail weight", &mut params.film_render.halation.scatter_tail_weight, 0.0, 1.0, 0.01, 2, "Weight of the scatter tail Gaussian per channel [R,G,B] (0-1). Tail weight + core weight = 1. Higher values put more scattered light into the long tail.");
                flags.runtime_changed |= tuple(ui, "halation strength", &mut params.film_render.halation.halation_strength, 0.0, 1000000.0, 0.005, 3, "Total back-reflection halation amplitude per channel [R,G,B] (0-1). Typical red channel: weak AH 0.02-0.08, no AH 0.08-0.25. The blue channel is usually near zero.");
                flags.runtime_changed |= tuple(ui, "halation first sigma um", &mut params.film_render.halation.halation_first_sigma_um, 0.0, 1000000.0, 1.0, 2, "Sigma of the first halation bounce per channel [R,G,B], in micrometers. Set by the base thickness (40-80 um for typical cine/still bases).");
                flags.runtime_changed |= numeric(ui, "halation n bounces", &mut params.film_render.halation.halation_n_bounces, 1.0, 5.0, 1.0, 2, "Number of multi-bounce Gaussians summed in the halation pass. Subsequent bounces use sqrt(k)-spaced widths. Typical: 2-3.");
                flags.runtime_changed |= numeric(ui, "halation bounce decay", &mut params.film_render.halation.halation_bounce_decay, 0.0, 1.0, 0.05, 2, "Per-bounce amplitude decay ratio (rho). Physical range 0.3-0.7. Controls how fast the halation energy falls off between bounces.");
                flags.runtime_changed |= toggle(ui, "halation renormalize", &mut params.film_render.halation.halation_renormalize, "If enabled, divide by (1 + sum of bounce amplitudes) so mid-grey is preserved. If disabled, halation is purely additive and subtly lifts shadows as well as highlights.");
            });
        }
        if section == "Couplers" {
            egui::CollapsingHeader::new("Couplers").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= toggle(ui, "active", &mut params.film_render.dir_couplers.active, "Enable DIR coupler inhibition.");
                flags.runtime_changed |= numeric(ui, "amount", &mut params.film_render.dir_couplers.amount, 0.0, 1000000.0, 0.05, 2, "Global multiplier on the DIR coupler inhibition matrix. 1.0 leaves the per-channel gammas as-is.");
                flags.runtime_changed |= numeric(ui, "inhibition samelayer", &mut params.film_render.dir_couplers.inhibition_samelayer, 0.0, 1000000.0, 0.05, 2, "Multiplier on the same-layer (diagonal) inhibition. Controls overall contrast / gamma reduction within each RGB layer.");
                flags.runtime_changed |= numeric(ui, "inhibition interlayer", &mut params.film_render.dir_couplers.inhibition_interlayer, 0.0, 1000000.0, 0.05, 2, "Multiplier on the cross-layer (off-diagonal) inhibition. Controls saturation enhancement from interlayer DIR effects.");
                flags.runtime_changed |= tuple(ui, "gamma samelayer rgb", &mut params.film_render.dir_couplers.gamma_samelayer_rgb, 0.0, 1000000.0, 0.02, 2, "Per-channel same-layer DIR gamma (R, G, B). Effective gamma reduction of each layer's density curve.");
                flags.runtime_changed |= tuple(ui, "gamma interlayer r to gb", &mut params.film_render.dir_couplers.gamma_interlayer_r_to_gb, 0.0, 1000000.0, 0.02, 2, "DIR inhibition from the R layer onto the G and B layers respectively (g_R->G, g_R->B).");
                flags.runtime_changed |= tuple(ui, "gamma interlayer g to rb", &mut params.film_render.dir_couplers.gamma_interlayer_g_to_rb, 0.0, 1000000.0, 0.02, 2, "DIR inhibition from the G layer onto the R and B layers respectively (g_G->R, g_G->B).");
                flags.runtime_changed |= tuple(ui, "gamma interlayer b to rg", &mut params.film_render.dir_couplers.gamma_interlayer_b_to_rg, 0.0, 1000000.0, 0.02, 2, "DIR inhibition from the B layer onto the R and G layers respectively (g_B->R, g_B->G).");
                flags.runtime_changed |= tuple(ui, "langmuir donor k rgb", &mut params.film_render.dir_couplers.langmuir_donor_k_rgb, 0.1, 1000000.0, 0.1, 2, "NEGATIVE film: per-channel Langmuir saturation of DIR inhibitor release, normalized to each layer's d_max (R, G, B). Lower = earlier, stronger roll-off at high density (gentler shoulder, tames high-amount breakage); large values approach the linear model.");
                flags.runtime_changed |= tuple(ui, "langmuir receiver k rgb", &mut params.film_render.dir_couplers.langmuir_receiver_k_rgb, 0.1, 1000000.0, 0.1, 2, "POSITIVE/reversal film: per-channel receiver-side Langmuir saturation (R, G, B), normalized to the arrived-inhibitor ceiling (the receiver analogue of langmuir_donor_k_rgb's d_max normalization). The donor (silver release) stays linear; this rolls off the receiving layer's response under a pushed amount. Lower = harder, more crash-proof knee (alters the operating-point look); large values approach linear.");
                flags.runtime_changed |= numeric(ui, "diffusion size um", &mut params.film_render.dir_couplers.diffusion_size_um, 0.0, 1000000.0, 5.0, 2, "Sigma in um for the diffusion of the couplers, (5-20 um), controls sharpness and affects saturation.");
                flags.runtime_changed |= numeric(ui, "diffusion tail um", &mut params.film_render.dir_couplers.diffusion_tail_um, 0.0, 1000000.0, 10.0, 2, "Exponential tail scale in um for long-range coupler spread. Larger values extend the rare long-distance inhibition halo beyond the Gaussian core.");
                flags.runtime_changed |= numeric(ui, "diffusion tail weight", &mut params.film_render.dir_couplers.diffusion_tail_weight, 0.0, 1.0, 0.01, 2, "Fraction of coupler diffusion energy assigned to the long tail. 0 = pure Gaussian spread, higher values add a longer-range inhibition shoulder.");
            });
        }
        if section == "Camera diffusion" || section == "Enlarger diffusion" {
            ui.collapsing("Diffusion", |ui| {
                let filter = if section == "Camera diffusion" { &mut params.camera.diffusion_filter } else { &mut params.enlarger.diffusion_filter };
                flags.runtime_changed |= diffusion(ui, filter);
            });
        }
        if section == "Experimental" {
            egui::CollapsingHeader::new("Experimental").default_open(false).show(ui, |ui| {
                flags.runtime_changed |= choice_tip(ui, "print illuminant", &mut params.enlarger.illuminant, &["TH-KG3"], "Print illuminant to simulate");
                flags.runtime_changed |= extra_channels(ui, extras, "film_channel_swap", "film channel swap");
                flags.runtime_changed |= extra_channels(ui, extras, "print_channel_swap", "print channel swap");
            });
        }
        if section == "Output" {
            flags.runtime_changed |= choice_tip(ui, "output color space", &mut params.io.output_color_space, COLOR_SPACES, "Output color space of the simulation");
            flags.display_changed |= extra_choice(ui, extras, "simulation", "saving_color_space", "saving color space", "sRGB", COLOR_SPACES, "Color space of the saved image file");
            flags.display_changed |= extra_bool_tip(ui, extras, "simulation", "saving_cctf_encoding", "saving cctf encoding", true, "Add or not the CCTF to the saved image file");
        }
        if section == "Display" {
            if numeric(ui, "preview max size", &mut params.settings.preview_max_size, 128.0, 1000000.0, 128.0, 2, "max size of the long edge of the preview image in pixels") {
                set_extra(extras, "display", "preview_max_size", Value::from(params.settings.preview_max_size));
                flags.display_changed = true;
            }
            flags.preview_requested |= ui.button("update").clicked();
        }
        if section == "Import Raw" {
            flags.raw_reload |= extra_choice(ui, extras, "load_raw", "white_balance", "white balance", "daylight", &["as_shot", "daylight", "tungsten", "custom"], "Leave at daylight (D65): it is the colorimetric reference the rest of the pipeline assumes, and should not be changed. Do not use it to neutralize a colour cast — fix white balance downstream with the enlarger filters instead. (custom exposes temperature/tint for special cases.)");
            flags.raw_reload |= extra_number(ui, extras, "load_raw", "temperature", "temperature", 5500.0, 1000.0, 1000000.0, 100.0, 2, "Temperature in Kelvin for the custom whitebalance, not used for the other white balance settings");
            flags.raw_reload |= extra_number(ui, extras, "load_raw", "tint", "tint", 1.0, 0.0, 1000000.0, 0.01, 2, "Tint value for the custom white balance, not used for the other white balance settings");
            flags.raw_reload |= extra_bool_tip(ui, extras, "load_raw", "lens_correction", "lens correction", false, "Apply lens corrections");
            flags.raw_reload |= ui.button("reprocess raw").clicked();
        }
    });
    flags
}

fn diffusion(ui: &mut Ui, value: &mut DiffusionFilterParams) -> bool {
    let mut changed = toggle(
        ui,
        "active",
        &mut value.active,
        "Toggle the diffusion filter (Pro-Mist family).",
    );
    changed |= choice_tip(
        ui,
        "filter family",
        &mut value.filter_family,
        &["glimmerglass", "black_pro_mist", "pro_mist", "cinebloom"],
        "PSF family. pro_mist / glimmerglass / cinebloom are transparent (energy-preserving); black_pro_mist absorbs a fraction of the deflected light, lifting shadows by reducing local contrast.",
    );
    changed |= numeric(
        ui,
        "strength",
        &mut value.strength,
        0.0,
        2.0,
        0.125,
        2,
        "Commercial filter stop: 0, 1/8=0.125, 1/4=0.25, 1/2=0.5, 1, 2. Maps internally to the (p_s, p_a) deflected/absorbed photon fractions.",
    );
    changed |= numeric(
        ui,
        "spatial scale",
        &mut value.spatial_scale,
        0.0,
        1000000.0,
        0.1,
        2,
        "Multiplier on the image-plane PSF widths (all per-group lambdas). Adjust for image-format / print-size differences.",
    );
    changed |= numeric(
        ui,
        "halo warmth",
        &mut value.halo_warmth,
        -1.5,
        1.5,
        0.05,
        2,
        "Additive offset on the family's halo warmth axis. Positive = warm outer halo / cool inner halo. Energy-preserving per channel. 0 = use family default.",
    );
    changed |= numeric(
        ui,
        "core intensity",
        &mut value.core_intensity,
        0.0,
        4.0,
        0.05,
        2,
        "Advanced. Multiplier on the core weight; the three group weights are renormalized to sum to 1. 1.0 = use family default.",
    );
    changed |= numeric(
        ui,
        "core size",
        &mut value.core_size,
        0.1,
        4.0,
        0.05,
        2,
        "Advanced. Multiplier on the core lambda. 1.0 = use family default.",
    );
    changed |= numeric(
        ui,
        "halo intensity",
        &mut value.halo_intensity,
        0.0,
        4.0,
        0.05,
        2,
        "Advanced. Multiplier on the halo weight; the three group weights are renormalized to sum to 1. 1.0 = use family default.",
    );
    changed |= numeric(
        ui,
        "halo size",
        &mut value.halo_size,
        0.1,
        4.0,
        0.05,
        2,
        "Advanced. Multiplier on the halo lambda. 1.0 = use family default.",
    );
    changed |= numeric(
        ui,
        "bloom intensity",
        &mut value.bloom_intensity,
        0.0,
        4.0,
        0.05,
        2,
        "Advanced. Multiplier on the bloom weight; the three group weights are renormalized to sum to 1. 1.0 = use family default.",
    );
    changed |= numeric(
        ui,
        "bloom size",
        &mut value.bloom_size,
        0.1,
        4.0,
        0.05,
        2,
        "Advanced. Multiplier on the bloom lambda. 1.0 = use family default.",
    );
    changed
}

fn optional_numeric(
    ui: &mut Ui,
    label: &str,
    value: &mut Option<f32>,
    inherited: f32,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
) -> bool {
    let mut displayed = value.unwrap_or(inherited);
    let changed = numeric(ui, label, &mut displayed, min, max, step, decimals, tooltip);
    if changed {
        *value = Some(displayed);
    }
    changed
}

fn tuple<T: egui::emath::Numeric, const N: usize>(
    ui: &mut Ui,
    label: &str,
    values: &mut [T; N],
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
) -> bool {
    ui.push_id(label, |ui| {
        ui.label(label).on_hover_text(tooltip);
        ui.horizontal(|ui| {
            values
                .iter_mut()
                .enumerate()
                .map(|(index, value)| {
                    numeric_field(ui, (label, index), value, min, max, step, decimals, tooltip)
                })
                .fold(false, |changed, field_changed| changed | field_changed)
        })
        .inner
    })
    .inner
}

fn toggle(ui: &mut Ui, label: &str, value: &mut bool, tooltip: &str) -> bool {
    ui.checkbox(value, label).on_hover_text(tooltip).changed()
}

pub fn choice_tip(
    ui: &mut Ui,
    label: &str,
    value: &mut String,
    choices: &[&str],
    tooltip: &str,
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_label(label)
        .selected_text(value.as_str())
        .show_ui(ui, |ui| {
            for &option in choices {
                // Allocate only when a selection changes, not for every option each frame.
                if ui
                    .selectable_label(value.as_str() == option, option)
                    .on_hover_text(tooltip)
                    .clicked()
                    && value.as_str() != option
                {
                    *value = option.to_owned();
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    changed
}

fn extra_choice(
    ui: &mut Ui,
    extras: &mut Value,
    section: &str,
    key: &str,
    label: &str,
    default: &str,
    choices: &[&str],
    tooltip: &str,
) -> bool {
    let current = extras[section][key].as_str().unwrap_or(default);
    let mut selected = None;
    egui::ComboBox::from_label(label)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for &option in choices {
                if ui
                    .selectable_label(current == option, option)
                    .on_hover_text(tooltip)
                    .clicked()
                    && current != option
                {
                    selected = Some(option);
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    if let Some(selected) = selected {
        set_extra(extras, section, key, Value::from(selected));
        true
    } else {
        false
    }
}

pub fn extra_bool_tip(
    ui: &mut Ui,
    extras: &mut Value,
    section: &str,
    key: &str,
    label: &str,
    default: bool,
    tooltip: &str,
) -> bool {
    let mut value = extras[section][key].as_bool().unwrap_or(default);
    if toggle(ui, label, &mut value, tooltip) {
        set_extra(extras, section, key, Value::from(value));
        true
    } else {
        false
    }
}

fn extra_number(
    ui: &mut Ui,
    extras: &mut Value,
    section: &str,
    key: &str,
    label: &str,
    default: f64,
    min: f64,
    max: f64,
    step: f64,
    decimals: usize,
    tooltip: &str,
) -> bool {
    let mut value = extras[section][key].as_f64().unwrap_or(default);
    if numeric(ui, label, &mut value, min, max, step, decimals, tooltip) {
        set_extra(extras, section, key, Value::from(value));
        true
    } else {
        false
    }
}

fn extra_channels(ui: &mut Ui, extras: &mut Value, key: &str, label: &str) -> bool {
    let mut values = [0_u32, 1, 2];
    if let Some(saved) = extras["special"][key].as_array() {
        for (value, saved) in values.iter_mut().zip(saved) {
            if let Some(channel) = saved.as_u64().filter(|&channel| channel <= 2) {
                *value = channel as u32;
            }
        }
    }
    if tuple(ui, label, &mut values, 0.0, 2.0, 1.0, 0, "") {
        set_extra(extras, "special", key, Value::from(values.to_vec()));
        true
    } else {
        false
    }
}

fn set_extra(extras: &mut Value, section: &str, key: &str, value: Value) {
    if !extras.is_object() {
        *extras = Value::Object(Map::new());
    }
    if !extras[section].is_object() {
        extras[section] = Value::Object(Map::new());
    }
    extras[section][key] = value;
}
