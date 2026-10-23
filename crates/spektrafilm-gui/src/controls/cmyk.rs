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
            let cmyk = &mut params.cmyk;
            if filter_matches(filter, "enable CMYK ink style") {
                flags.runtime_changed |= toggle(
                    ui,
                    "Enable CMYK ink style",
                    &mut cmyk.active,
                    "Enable the independent CMYK ink style processing stage; retains its strength while disabled.",
                );
            }
            if filter_matches(filter, "strength") {
                flags.runtime_changed |= param_numeric(
                    ui,
                    "strength",
                    &mut cmyk.strength,
                    "cmyk.strength",
                    "CMYK ink style appearance strength.",
                );
            }
        });
    }
}
