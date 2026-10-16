use super::*;
use egui::Ui;

/// One `show()` section group, split out of the dispatcher for readability.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    extras: &mut serde_json::Value,
    section: &str,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    if section == "Convert" {
        ui.scope(|ui| {
            ui.label(
                egui::RichText::new("Workflow: 1) Detect base → 2) Blind calibration (vibrant frame) → 3) Neutralize filters")
                    .small()
                    .color(theme::FG_MUTED),
            );
            if filter_matches(filter, "scan illuminant") {
                flags.runtime_changed |= choice_tip(ui, "scan illuminant", &mut params.film_render.convert.scan_illuminant, &["D50", "D55", "D65", "A", "BB3200", "BB5000", "LED-B3", "LED-B5", "LED-RGB1", "LED-V2", "FL2"], "Light source of the scanning / capture rig used to digitize the negative. Set it to match your rig (a physical input); the film Base is the creative lever. Only used by the 'convert-film' workflow routes.");
            }
            if filter_matches(filter, "exposure compensation ev") {
                flags.runtime_changed |= param_numeric(ui, "exposure compensation ev", &mut params.film_render.convert.exposure_compensation_ev, "film_render.convert.exposure_compensation_ev", "Aligns the scan's overall brightness to the model (gain = 2^ev). Brighter input -> less recovered film density.");
            }
            if filter_matches(filter, "base percentile") {
                flags.runtime_changed |= param_numeric(ui, "base percentile", &mut params.film_render.convert.base_percentile, "film_render.convert.base_percentile", "Brightest-pixels percentile used by 'Detect base' to sample the clear / unexposed film (99 = brightest 1%). Detect base fits the film Base so that clear film maps to density 0 (unexposed -> neutral).");
            }
            if filter_matches(filter, "calibration") {
                ui.label("calibration").on_hover_text("3x3 device-correction matrix (9 numbers, row-major) applied to the input before inversion, to undo the scanner/camera colour rendering vs the standard observer over the film dyes (the cross-channel cast that the scan illuminant and base cannot reach). Use 'Blind calibration' to fit it from the current image.");
                flags.runtime_changed |= calibration_matrix(ui, &mut params.film_render.convert.calibration);
                flags.runtime_changed |= ui.text_edit_singleline(&mut params.film_render.convert.calibration).changed();
            }
            if filter_matches_any(
                filter,
                &["detect base", "blind calibration", "neutralize print filters"],
            ) {
                ui.horizontal(|ui| {
                    if filter_matches(filter, "detect base")
                        && ui.button("detect base").on_hover_text("Sample the clear/unexposed film from the brightest pixels of the current image (see Base percentile) and fit the film Base + exposure so the unexposed film maps to density 0. Run this first.").clicked()
                    {
                        flags.action = Some(CalibrationAction::DetectBase);
                    }
                    if filter_matches(filter, "blind calibration")
                        && ui.button("blind calibration").on_hover_text("Fit the Calibration matrix from the CURRENT image and write it above. Works best on a vibrant, colour-filled frame (it aligns the scanned colours to the film's dye gamut); a flat/neutral frame gives a weak fit. Review/edit the values afterwards.").clicked()
                    {
                        flags.action = Some(CalibrationAction::BlindCalibration);
                    }
                    if filter_matches(filter, "neutralize print filters")
                        && ui.button("neutralize print filters").on_hover_text("Solve the print enlarger M and Y filter shifts so the current film + base midgray prints neutral. Image-independent; run it after Detect base (a tuned base changes the print balance). Writes Print M/Y filter shift in Enlarger.").clicked()
                    {
                        flags.action = Some(CalibrationAction::NeutralizeFilters);
                    }
                });
            }
            if filter_matches(filter, "film_channel_swap (experimental)") {
                flags.runtime_changed |= extra_channels(
                    ui,
                    extras,
                    "film_channel_swap",
                    "film_channel_swap (experimental)",
                    "extras.special.film_channel_swap",
                );
            }
            if filter_matches(filter, "print_channel_swap (experimental)") {
                flags.runtime_changed |= extra_channels(
                    ui,
                    extras,
                    "print_channel_swap",
                    "print_channel_swap (experimental)",
                    "extras.special.print_channel_swap",
                );
            }
        });
    }
}

/// Serialize a 3x3 matrix to the calibration string format.
fn calibration_to_string(matrix: &[[f64; 3]; 3]) -> String {
    matrix
        .iter()
        .flat_map(|row| row.iter())
        .map(|v| format!("{v}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse the calibration string into a 3x3 matrix.
/// Returns `None` if the text is invalid (non-numeric, non-finite, or wrong count).
fn calibration_from_string(text: &str) -> Option<[[f64; 3]; 3]> {
    let values: Vec<f64> = text
        .split(|c: char| c.is_ascii_whitespace() || matches!(c, ',' | ';' | '[' | ']' | '(' | ')'))
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() != 9 {
        return None;
    }
    if values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    Some([
        [values[0], values[1], values[2]],
        [values[3], values[4], values[5]],
        [values[6], values[7], values[8]],
    ])
}

/// 3x3 grid editor for the calibration matrix. Each cell is a numeric field;
/// the matrix is serialized to the calibration string on every edit.
/// Returns `false` (no change) if the current text is invalid.
fn calibration_matrix(ui: &mut Ui, calibration: &mut String) -> bool {
    let Some(mut matrix) = calibration_from_string(calibration) else {
        return false;
    };
    let mut changed = false;
    for row in matrix.iter_mut() {
        ui.horizontal(|ui| {
            for cell in row.iter_mut() {
                let mut value = *cell;
                if ui
                    .add(egui::DragValue::new(&mut value).speed(0.01))
                    .changed()
                {
                    *cell = value;
                    changed = true;
                }
            }
        });
    }
    if changed {
        *calibration = calibration_to_string(&matrix);
    }
    changed
}
