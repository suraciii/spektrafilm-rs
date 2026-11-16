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
    if section == "CMYK ink style" {
        section_header(ui, filter, section, |ui, filter| {
            let enabled = params.workflow.mode.is_film();
            ui.add_enabled_ui(enabled, |ui| {
            if parent_selector_visible(filter, "enable CMYK ink style") {
                flags.runtime_changed |= toggle(
                    ui,
                    "Enable CMYK ink style",
                    &mut params.cmyk.active,
                    "Enable the independent CMYK ink style processing stage; retains its strength while disabled.",
                );
            }
            if parent_owned_visible(params.cmyk.active, filter)
                && filter_matches(filter, "strength")
            {
                // Strength is the amount that brings the effect back from
                // zero, so it stays editable while CMYK is enabled. Search
                // still reveals it disabled with the parent reason when CMYK
                // is off.
                let enabled = params.cmyk.active;
                flags.runtime_changed |= ui
                    .add_enabled_ui(enabled, |ui| {
                        param_numeric(
                            ui,
                            "strength",
                            &mut params.cmyk.strength,
                            "cmyk.strength",
                            "CMYK ink style appearance strength.",
                        )
                    })
                    .inner;
                if !enabled
                    && let Some(reason) =
                        crate::controls::control_inactive_reason(ui.ctx(), "cmyk.strength", params)
                {
                    ui.label(
                        egui::RichText::new(localized(ui.ctx(), &reason))
                            .small()
                            .color(theme::FG_MUTED),
                    );
                }
            }
            });
        });
    }
}
