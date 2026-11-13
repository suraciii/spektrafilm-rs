use super::*;

fn comparison_id() -> egui::Id {
    egui::Id::new("finishing.compare")
}

/// End a held comparison even when the module is no longer visible.
pub(crate) fn release_finishing_compare(ctx: &egui::Context) -> bool {
    if ctx.input(|input| input.pointer.primary_down() || input.key_down(egui::Key::Space)) {
        return false;
    }
    ctx.data_mut(|data| {
        let comparing = data.get_temp_mut_or_default::<bool>(comparison_id());
        std::mem::take(comparing)
    })
}

pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    scanned_medium_positive: bool,
    section: &str,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    if section != "Finishing" {
        return;
    }
    let active = finishing_applicable(params, scanned_medium_positive);
    let finishing = &mut params.finishing;
    for (label, path, value, help) in [
        (
            "Brightness",
            "finishing.brightness",
            &mut finishing.brightness,
            "Raise or lower internal tones with less influence near reference endpoints. This is not source exposure or linear EV.",
        ),
        (
            "Contrast",
            "finishing.contrast",
            &mut finishing.contrast,
            "Change internal tonal separation with less influence near reference endpoints. This is not sharpening or an endpoint crop.",
        ),
        (
            "Tone & Color warmth",
            "finishing.warmth",
            &mut finishing.warmth,
            "Shift along a fixed warm–cool direction weighted by tone and chroma. This is not Kelvin or capture white balance.",
        ),
        (
            "Saturation",
            "finishing.saturation",
            &mut finishing.saturation,
            "Change color strength with restrained amplification of strong colors. This is not physical dye density or automatic warming.",
        ),
    ] {
        if filter_matches(filter, label) && row_path_matches(ui.ctx(), path) {
            flags.runtime_changed |= param_numeric(ui, label, value, path, help);
        }
    }
    section_header(ui, filter, "Expand", |ui, filter| {
        if filter_matches(filter, "Enable Expand")
            && row_path_matches(ui.ctx(), "finishing.expand.enabled")
            && toggle(
                ui,
                "Enable Expand",
                &mut finishing.expand.enabled,
                "Map fixed sRGB-encoded input references to black and white. Hard clipping destroys differences for this render; later controls cannot recover them.",
            )
        {
            record_edited_path(ui.ctx(), "finishing.expand.enabled");
            flags.runtime_changed = true;
        }
        if filter_matches(filter, "Black reference")
            && row_path_matches(ui.ctx(), "finishing.expand.black")
        {
            flags.runtime_changed |= param_numeric(
                ui,
                "Black reference",
                &mut finishing.expand.black,
                "finishing.expand.black",
                "Encoded input reference mapped to black. This is not a scanner calibration point or output endpoint.",
            );
        }
        if filter_matches(filter, "White reference")
            && row_path_matches(ui.ctx(), "finishing.expand.white")
        {
            flags.runtime_changed |= param_numeric(
                ui,
                "White reference",
                &mut finishing.expand.white,
                "finishing.expand.white",
                "Encoded input reference mapped to white. This is not a scanner calibration point or output endpoint.",
            );
        }
    });
    if filter_matches(filter, "Hold finishing comparison") {
        let response = ui.add_enabled(active, egui::Button::new(localized(ui.ctx(), "Hold finishing comparison")))
            .on_hover_text(localized(ui.ctx(), "Hold the primary mouse button or focused Space to bypass only finishing. Release to restore it. Saved and exported adjustments are unchanged."));
        focus_response(ui, "Hold finishing comparison", &response);
        let comparing = active
            && ((response.is_pointer_button_down_on()
                && ui.input(|input| input.pointer.primary_down()))
                || (response.has_focus() && ui.input(|input| input.key_down(egui::Key::Space))));
        let previous = ui.ctx().data_mut(|data| {
            let state = data.get_temp_mut_or_default::<bool>(comparison_id());
            std::mem::replace(state, comparing)
        });
        if previous != comparing {
            flags.finishing_compare = Some(comparing);
        }
    }
}
