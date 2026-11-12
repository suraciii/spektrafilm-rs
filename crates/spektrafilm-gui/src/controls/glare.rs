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
            let reason = crate::controls::control_inactive_reason(
                ui.ctx(),
                "print_render.glare.active",
                params,
            );
            ui.add_enabled_ui(reason.is_none(), |ui| {
                if filter_matches(filter, "simulate viewing glare") {
                    flags.runtime_changed |= toggle(ui, "Simulate viewing glare", &mut params.print_render.glare.active, "Adds stray light from viewing or scanning a paper print: shadows lift and contrast drops. It does not change print exposure, chemistry, or developed density.");
                }
                if reason.is_none() {
                    let status = if !params.print_render.glare.active {
                        Some("Off. Stored values are kept.")
                    } else if params.print_render.glare.percent <= 0.0 {
                        Some("Glare amount is 0, so viewing glare is not applied. Increase the amount to enable it.")
                    } else if params.print_render.glare.percent > 0.0 && params.debug.deactivate_stochastic_effects {
                        Some("Debug settings disable stochastic effects, so viewing glare does not run.")
                    } else {
                        None
                    };
                    if let Some(status) = status {
                        ui.label(egui::RichText::new(localized(ui.ctx(), status)).small().color(theme::FG_MUTED));
                    }
                }
                if filter_matches(filter, "percent") {
                    flags.runtime_changed |= param_numeric(ui, "percent", &mut params.print_render.glare.percent, "print_render.glare.percent", "Stray light as a fraction of readout light. 0 turns the effect off. Default 0.03 (3%).");
                }
                if filter_matches(filter, "roughness") {
                    flags.runtime_changed |= param_numeric(ui, "roughness", &mut params.print_render.glare.roughness, "print_render.glare.roughness", "Random variation of the glare amount: 0 is even, 1 is strongly mottled.");
                }
                if filter_matches(filter, "blur") {
                    flags.runtime_changed |= param_numeric(ui, "Blur (viewing glare)", &mut params.print_render.glare.blur, "print_render.glare.blur", "Spatial spread of the glare amount in pixels; 0 is per-pixel, larger values look like broad reflections.");
                }
            });
        });
    }
}
