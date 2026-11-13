use super::*;
use egui::Ui;

/// Camera light leak: stray light that reached the film record during exposure.
///
/// The section keeps the photographic leaves together — the aperture's strength
/// in stops over the film's mid-density exposure, its place on the frame
/// perimeter, its size and inward spread, and the record ratio of the stray
/// light — so one section reads as one aperture.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    section: &str,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    if section != "Light leak" {
        return;
    }
    let inactive =
        crate::controls::control_inactive_reason(ui.ctx(), "film_render.light_leak.active", params);
    if let Some(reason) = &inactive {
        ui.label(localized(ui.ctx(), reason));
    }
    ui.add_enabled_ui(inactive.is_none(), |ui| {
        let leak = &mut params.film_render.light_leak;
        ui.label(
            egui::RichText::new(crate::i18n::text(
                ui.ctx(),
                "controls.light_leak.key_controls",
            ))
            .small()
            .color(theme::FG_MUTED),
        );
        if filter_matches(filter, "enable light leak") {
            flags.runtime_changed |= toggle(
                ui,
                "Enable light leak",
                &mut leak.active,
                "Enable the camera light leak and its stray-light aperture.",
            );
        }
        if filter_matches(filter, "leak exposure ev") {
            flags.runtime_changed |= param_numeric(
                ui,
                "leak exposure ev",
                &mut leak.exposure_ev,
                "film_render.light_leak.exposure_ev",
                "Aperture strength in stops over the film's own mid-density exposure. Zero is an exact bypass; the stray light is absolute and does not scale with scene brightness.",
            );
        }
        if filter_matches(filter, "leak position") {
            flags.runtime_changed |= param_numeric(
                ui,
                "leak position",
                &mut leak.position,
                "film_render.light_leak.position",
                "Aperture center position around the frame perimeter, from 0 to 1, measured clockwise from the top-left corner.",
            );
        }
        if filter_matches(filter, "leak width mm") {
            flags.runtime_changed |= param_numeric(
                ui,
                "leak width mm",
                &mut leak.width_mm,
                "film_render.light_leak.width_mm",
                "Aperture width along the frame perimeter, in millimetres on the film. A door hinge or camera-back gasket is a few millimetres; a failed light trap covers a whole edge.",
            );
        }
        if filter_matches(filter, "leak spread mm") {
            flags.runtime_changed |= param_numeric(
                ui,
                "leak spread mm",
                &mut leak.spread_mm,
                "film_render.light_leak.spread_mm",
                "Inward e-folding length of the leak, in millimetres on the film. A tight seal keeps the stray light near the edge; a scattering gap spreads it across the frame.",
            );
        }
        if filter_matches(filter, "leak angle deg") {
            flags.runtime_changed |= param_numeric(
                ui,
                "leak angle deg",
                &mut leak.angle_deg,
                "film_render.light_leak.angle_deg",
                "Aperture tilt relative to the perimeter tangent, in degrees.",
            );
        }
        if filter_matches(filter, "leak spectrum") {
            flags.runtime_changed |= param_tuple(
                ui,
                "leak spectrum",
                &mut leak.leak_spectrum,
                "film_render.light_leak.leak_spectrum",
                "Stray-light exposure ratio per record, in cyan, magenta, yellow order. The default is a warm path: the rim reads orange-red and a strong leak burns its core to white. The rendered colour is each material's own dye response.",
            );
        }
        if filter_matches(filter, "leak texture amount") {
            flags.runtime_changed |= param_numeric(
                ui,
                "leak texture amount",
                &mut leak.texture_amount,
                "film_render.light_leak.texture_amount",
                "Strength of the seeded streak structure over the smooth falloff, from 0 to 1. Real leaks are uneven where the seal gap is uneven.",
            );
        }
        if filter_matches(filter, "leak texture scale mm") {
            flags.runtime_changed |= param_numeric(
                ui,
                "leak texture scale mm",
                &mut leak.texture_scale_mm,
                "film_render.light_leak.texture_scale_mm",
                "Spatial scale of the streak structure, in millimetres on the film.",
            );
        }
    });
}
