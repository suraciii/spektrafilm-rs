use super::*;
use egui::Ui;

/// One `show()` section group, split out of the dispatcher for readability.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    section: &str,
    flags: &mut ChangeFlags,
) {
    if section == "Convert" {
        ui.scope(|ui| {
            ui.label(
                egui::RichText::new("Workflow: 1) Detect base → 2) Blind calibration (vibrant frame) → 3) Neutralize filters")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            flags.runtime_changed |= choice_tip(ui, "scan illuminant", &mut params.film_render.convert.scan_illuminant, &["D50", "D55", "D65", "A", "BB3200", "BB5000", "LED-B3", "LED-B5", "LED-RGB1", "LED-V2", "FL2"], "Light source of the scanning / capture rig used to digitize the negative. Set it to match your rig (a physical input); the film Base is the creative lever. Only used by the 'convert-film' workflow routes.");
            flags.runtime_changed |= param_numeric(ui, "exposure compensation ev", &mut params.film_render.convert.exposure_compensation_ev, "film_render.convert.exposure_compensation_ev", "Aligns the scan's overall brightness to the model (gain = 2^ev). Brighter input -> less recovered film density.");
            flags.runtime_changed |= param_numeric(ui, "base percentile", &mut params.film_render.convert.base_percentile, "film_render.convert.base_percentile", "Brightest-pixels percentile used by 'Detect base' to sample the clear / unexposed film (99 = brightest 1%). Detect base fits the film Base so that clear film maps to density 0 (unexposed -> neutral).");
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
}
