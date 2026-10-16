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
    if section == "Glare" {
        section_header(ui, filter, "Print viewing glare", |ui, filter| {
            ui.add_enabled_ui(params.workflow.route.contains(" > print > "), |ui| {
                if filter_matches(filter, "enable print glare") {
                    flags.runtime_changed |= toggle(ui, "Enable print glare", &mut params.print_render.glare.active, "Add readout stray light after lens blur and before scanner correction. This does not change print exposure or developed density.");
                }
                if filter_matches(filter, "percent") {
                    flags.runtime_changed |= param_numeric(ui, "percent", &mut params.print_render.glare.percent, "print_render.glare.percent", "Percentage of the glare light (typically 0.1-0.25)");
                }
                if filter_matches(filter, "roughness") {
                    flags.runtime_changed |= param_numeric(ui, "roughness", &mut params.print_render.glare.roughness, "print_render.glare.roughness", "Roughness of the glare light (0-1)");
                }
                if filter_matches(filter, "blur") {
                    flags.runtime_changed |= param_numeric(ui, "Blur (viewing glare)", &mut params.print_render.glare.blur, "print_render.glare.blur", "Sigma of gaussian blur in pixels for the glare");
                }
            });
        });
    }
}
