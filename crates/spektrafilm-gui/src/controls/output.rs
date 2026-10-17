use super::*;
use egui::Ui;
use serde_json::Value;

/// One `show()` section group, split out of the dispatcher for readability.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    extras: &mut Value,
    section: &str,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    if section == "Output" {
        if filter_matches(filter, "simulation blend") {
            flags.runtime_changed |= param_numeric(
                ui,
                "Simulation blend",
                &mut params.output_mix.amount,
                "output_mix.amount",
                "Blend the complete result with the decoded, geometrically aligned prepared source in linear output RGB. 1 returns the complete result; 0 returns the prepared source before virtual camera exposure. For a captured negative, zero returns the prepared negative, not a positive interpretation. Input/Output viewing remains independent.",
            );
        }
        ui.label(
            egui::RichText::new("1.0 = full simulation · 0.0 = original")
                .small()
                .color(theme::FG_MUTED),
        );
        if filter_matches(filter, "output color space") {
            flags.runtime_changed |= choice_tip(
                ui,
                "output color space",
                &mut params.io.output_color_space,
                COLOR_SPACES,
                "Color space and transfer encoding of the runtime digital output. It describes what the simulation produces, which is distinct from the saving color space used for the file and from the display transform, which is viewer-only.",
            );
        }
        if filter_matches(filter, "saving color space") {
            extra_choice(
                ui,
                extras,
                "simulation",
                "saving_color_space",
                "saving color space",
                "sRGB",
                COLOR_SPACES,
                "Color space of the file Save writes. Save writes the retained output in this space without rerendering; Export renders and writes independently with its own saving color space.",
            );
        }
        if filter_matches(filter, "saving cctf encoding") {
            extra_bool_tip(
                ui,
                extras,
                "simulation",
                "saving_cctf_encoding",
                "saving cctf encoding",
                true,
                &format!(
                    "Add or not the CCTF — {} — encoding to the samples Save writes. Off writes linear samples. This file-conversion setting is independent of the output color space and of the viewer-only display transform.",
                    crate::routes::vocabulary::CCTF
                ),
            );
        }
    }
}
