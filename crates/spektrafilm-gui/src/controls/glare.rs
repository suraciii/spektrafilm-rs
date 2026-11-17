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
                if parent_selector_visible(filter, "simulate viewing glare") {
                    flags.runtime_changed |= toggle(ui, "Simulate viewing glare", &mut params.print_render.glare.active, "Adds stray light from viewing or scanning a paper print: shadows lift and contrast drops. It does not change Print exposure, chemistry, or developed density.");
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
                // The amount is the parent of roughness and blur: keep it
                // editable while glare is on, because it is the control that
                // brings the effect back from zero. The dependent rows leave
                // the ordinary list and return through search with a reason.
                if parent_owned_visible(params.print_render.glare.active, filter)
                    && filter_matches(filter, "percent")
                {
                    let enabled = params.print_render.glare.active;
                    flags.runtime_changed |= ui
                        .add_enabled_ui(enabled, |ui| {
                            param_numeric(ui, "percent", &mut params.print_render.glare.percent, "print_render.glare.percent", "Stray light as a fraction of readout light. 0 turns the effect off. Default 0.03 (3%).")
                        })
                        .inner;
                    if !enabled
                        && let Some(reason) = crate::controls::control_inactive_reason(
                            ui.ctx(),
                            "print_render.glare.percent",
                            params,
                        )
                    {
                        ui.label(
                            egui::RichText::new(localized(ui.ctx(), &reason))
                                .small()
                                .color(theme::FG_MUTED),
                        );
                    }
                }
                let effective = params.print_render.glare.active
                    && params.print_render.glare.percent > 0.0
                    && !params.debug.deactivate_stochastic_effects;
                if (effective || filter.active()) && filter_matches(filter, "roughness") {
                    flags.runtime_changed |= applicable_control(ui, params, "print_render.glare.roughness", |ui, params| {
                        param_numeric(ui, "roughness", &mut params.print_render.glare.roughness, "print_render.glare.roughness", "Random variation of the glare amount: 0 is even, 1 is strongly mottled.")
                    });
                }
                if (effective || filter.active()) && filter_matches(filter, "blur") {
                    flags.runtime_changed |= applicable_control(ui, params, "print_render.glare.blur", |ui, params| {
                        param_numeric(ui, "Blur (viewing glare)", &mut params.print_render.glare.blur, "print_render.glare.blur", "Spatial spread of the glare amount in pixels; 0 is per-pixel, larger values look like broad reflections.")
                    });
                }
            });
        });
    }
}
