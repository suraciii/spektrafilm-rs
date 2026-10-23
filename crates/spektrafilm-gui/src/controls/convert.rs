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
                flags.runtime_changed |= scan_illuminant(ui, &mut params.film_render.convert.scan_illuminant);
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
                ui.horizontal_wrapped(|ui| {
                    for (label, path, action, help) in [
                        ("detect base", "action.detect_base", CalibrationAction::DetectBase, "Sample the brightest clear film and fit film Base plus exposure so unexposed film maps to density zero."),
                        ("blind calibration", "action.blind_calibration", CalibrationAction::BlindCalibration, "Fit the correction matrix from the current vibrant image; review the fitted values afterwards."),
                        ("neutralize print filters", "action.neutralize_filters", CalibrationAction::NeutralizeFilters, "Solve main enlarger M/Y shifts so the current film base midgray prints neutral. Writes the Enlarger controls."),
                    ] {
                        if filter_matches(filter, label) {
                            let response = ui.button(label).on_hover_text(help);
                            locate_row(ui, path, &response);
                            if response.clicked() { flags.action = Some(action); }
                        }
                    }
                });
            }
            if filter_matches(filter, "film_channel_swap") {
                flags.runtime_changed |= extra_channels(
                    ui,
                    extras,
                    "film_channel_swap",
                    "film_channel_swap",
                    "extras.special.film_channel_swap",
                );
            }
            if filter_matches(filter, "print_channel_swap") {
                flags.runtime_changed |= extra_channels(
                    ui,
                    extras,
                    "print_channel_swap",
                    "print_channel_swap",
                    "extras.special.print_channel_swap",
                );
            }
        });
    }
}

/// Imported RGB acquisition has its own light, independent of product readout.
fn scan_illuminant(ui: &mut Ui, value: &mut String) -> bool {
    static ILLUMINANTS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| {
        spektrafilm_core::spectral_service::available_illuminants()
            .into_iter()
            .filter(|name| spektrafilm_core::spectral_service::is_supported_illuminant(name))
            .collect()
    });
    let mut changed = choice_tip_experimental(
        ui,
        "scan illuminant",
        value,
        &ILLUMINANTS,
        "Light used to acquire the imported negative RGB. This only changes capture inversion, not the fixed product readout light.",
    );
    let draft_id = ui.make_persistent_id("scan-illuminant-draft");
    let mut draft = ui
        .ctx()
        .data(|data| data.get_temp::<String>(draft_id))
        .unwrap_or_else(|| value.clone());
    let response = ui
        .add(
            egui::TextEdit::singleline(&mut draft)
                .desired_width(ui.available_width().min(theme::WIDTH_COMBO_MAX))
                .hint_text("custom BB<kelvin>"),
        )
        .on_hover_text("Enter any shared catalog illuminant or BB<kelvin> (1667..=25000).");
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label("custom scan illuminant"));
    if response.changed()
        && draft != *value
        && spektrafilm_core::spectral_service::is_supported_illuminant(&draft)
    {
        value.clone_from(&draft);
        changed = true;
        record_label_edit(ui.ctx(), "scan illuminant");
    }
    let stored_draft = if response.lost_focus() || !response.has_focus() {
        value.clone()
    } else {
        draft
    };
    ui.ctx()
        .data_mut(|data| data.insert_temp(draft_id, stored_draft));
    changed
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn canonical_acquisition_row_edits_shared_catalog_and_validates_custom_light() {
        let entry = lookup("film_render.convert.scan_illuminant").unwrap();
        assert_eq!(entry.owner, ControlOwner::Stage(Stage::Input));
        assert_eq!(entry.section, "Convert");
        assert_eq!(entry.editor, inventory::EditorKind::Choice);
        assert_eq!(
            entry.validation.as_ref().unwrap(),
            &spektrafilm_core::params::sources::describe_field(&entry.path).unwrap()
        );
        assert!(lookup("scanner.lamp").is_none());
        let params = crate::state::GuiState::factory().runtime_params().unwrap();
        let scanner = params.scanner.clone();
        let mut harness = Harness::new_ui_state(
            |ui, params| {
                show(
                    ui,
                    params,
                    &mut serde_json::Value::Null,
                    "Convert",
                    ScanMedium::Negative,
                    "film_render.convert.scan_illuminant",
                    &mut SectionLabelCache::default(),
                    "D55",
                    &crate::profiles::pick_data_dir(),
                );
            },
            params,
        );
        harness
            .get_by_role_and_label(egui::accesskit::Role::ComboBox, "scan illuminant")
            .click();
        harness.run();
        harness.get_by_label("D65").click();
        harness.run();
        assert_eq!(harness.state().film_render.convert.scan_illuminant, "D65");
        for (draft, expected) in [("BB4321", "BB4321"), ("BB1000", "BB4321")] {
            harness.get_by_label("custom scan illuminant").focus();
            harness.run();
            harness.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            harness
                .get_by_label("custom scan illuminant")
                .type_text(draft);
            harness.run();
            assert_eq!(
                harness.state().film_render.convert.scan_illuminant,
                expected
            );
            assert_eq!(harness.state().scanner, scanner);
            assert!(harness.state().validate().is_ok());
        }
    }
}
