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
    if section == "Film base" {
        ui.scope(|ui| {
            if filter_matches(filter, "active") {
                flags.runtime_changed |= toggle(ui, "active", &mut params.film_render.base.active, "Enable film base-density (film base + fog / orange mask) tuning.");
            }
            if filter_matches(filter, "scale") {
                flags.runtime_changed |= param_numeric(ui, "scale", &mut params.film_render.base.scale, "film_render.base.scale", "Overall multiplier on the film base density.");
            }
            if filter_matches(filter, "tilt") {
                flags.runtime_changed |= param_numeric(ui, "tilt", &mut params.film_render.base.tilt, "film_render.base.tilt", "Spectral tilt of the film base, pivoting at 555 nm (1.0 = +0.1 density at 650 nm; negative tilts the other way).");
            }
            if filter_matches(filter, "cyan") {
                flags.runtime_changed |= param_numeric(ui, "cyan", &mut params.film_render.base.cyan, "film_render.base.cyan", "Cyan-channel film base density shift around 650 nm (1.0 = neutral; adds (value - 1) density).");
            }
            if filter_matches(filter, "magenta") {
                flags.runtime_changed |= param_numeric(ui, "magenta", &mut params.film_render.base.magenta, "film_render.base.magenta", "Magenta-channel film base density shift around 555 nm (1.0 = neutral; adds (value - 1) density).");
            }
            if filter_matches(filter, "yellow") {
                flags.runtime_changed |= param_numeric(ui, "yellow", &mut params.film_render.base.yellow, "film_render.base.yellow", "Yellow-channel film base density shift around 460 nm (1.0 = neutral; adds (value - 1) density).");
            }
        });
    }
    if section == "Print base" {
        section_header(ui, filter, "Base", |ui, filter| {
            if filter_matches(filter, "active") {
                flags.runtime_changed |= toggle(
                    ui,
                    "active",
                    &mut params.print_render.base.active,
                    "Enable print base-density tuning.",
                );
            }
            if filter_matches(filter, "scale") {
                flags.runtime_changed |= param_numeric(
                    ui,
                    "scale",
                    &mut params.print_render.base.scale,
                    "print_render.base.scale",
                    "Overall multiplier on the print base density.",
                );
            }
            if filter_matches(filter, "cyan") {
                flags.runtime_changed |= param_numeric(
                    ui,
                    "cyan",
                    &mut params.print_render.base.cyan,
                    "print_render.base.cyan",
                    "Cyan-channel multiplicative scale of the print base min around 610 nm (1.0 = neutral).",
                );
            }
            if filter_matches(filter, "magenta") {
                flags.runtime_changed |= param_numeric(
                    ui,
                    "magenta",
                    &mut params.print_render.base.magenta,
                    "print_render.base.magenta",
                    "Magenta-channel multiplicative scale of the print base min around 530 nm (1.0 = neutral).",
                );
            }
            if filter_matches(filter, "yellow") {
                flags.runtime_changed |= param_numeric(
                    ui,
                    "yellow",
                    &mut params.print_render.base.yellow,
                    "print_render.base.yellow",
                    "Yellow-channel multiplicative scale of the print base min around 445 nm (1.0 = neutral).",
                );
            }
        });
    }
}
