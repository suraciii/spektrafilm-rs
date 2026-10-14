use super::*;
use egui::Ui;

/// One `show()` section group, split out of the dispatcher for readability.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    section: &str,
    flags: &mut ChangeFlags,
) {
    if section == "Glare" {
        egui::CollapsingHeader::new("Print viewing glare").default_open(false).show(ui, |ui| {
            ui.add_enabled_ui(params.workflow.route.contains(" > print > "), |ui| {
                flags.runtime_changed |= toggle(ui, "active", &mut params.print_render.glare.active, "Add readout stray light after lens blur and before scanner correction. This does not change print exposure or developed density.");
                flags.runtime_changed |= param_numeric(ui, "percent", &mut params.print_render.glare.percent, "print_render.glare.percent", "Percentage of the glare light (typically 0.1-0.25)");
                flags.runtime_changed |= param_numeric(ui, "roughness", &mut params.print_render.glare.roughness, "print_render.glare.roughness", "Roughness of the glare light (0-1)");
                flags.runtime_changed |= param_numeric(ui, "blur", &mut params.print_render.glare.blur, "print_render.glare.blur", "Sigma of gaussian blur in pixels for the glare");
            });
        });
    }
}
