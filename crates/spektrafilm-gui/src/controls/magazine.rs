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
    if section == "Magazine finish color" {
        section_header(ui, filter, section, |ui, filter| {
            let magazine = &mut params.magazine_print_color;
            if filter_matches(filter, "strength") {
                flags.runtime_changed |= param_numeric(
                    ui,
                    "strength",
                    &mut magazine.strength,
                    "magazine_print_color.strength",
                    "Magazine finish appearance strength.",
                );
            }
        });
    }
}
