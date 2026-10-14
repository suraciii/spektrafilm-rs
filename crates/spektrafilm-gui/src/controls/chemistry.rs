use super::*;
use egui::Ui;

/// One `show()` section group, split out of the dispatcher for readability.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    section: &str,
    flags: &mut ChangeFlags,
) {
    if section == "Film chemistry" {
        render_chemistry(ui, &mut params.film_render.chemistry, flags, true);
    }
    if section == "Print chemistry" {
        render_chemistry(
            ui,
            &mut params.print_render.density_curves_morph,
            flags,
            false,
        );
    }
}
