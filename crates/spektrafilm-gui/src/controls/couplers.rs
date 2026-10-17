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
    if section == "Couplers" {
        ui.scope(|ui| {
            ui.label(
                egui::RichText::new("Key controls: inhibition samelayer (contrast) · inhibition interlayer (saturation) · inhibition amount (global)")
                    .small()
                    .color(theme::FG_MUTED),
            );
            if filter_matches(filter, "enable dir couplers") {
                flags.runtime_changed |= toggle(
                    ui,
                    "Enable DIR couplers",
                    &mut params.film_render.dir_couplers.active,
                    "Enable DIR coupler inhibition.",
                );
            }
            if filter_matches(filter, "inhibition amount") {
                flags.runtime_changed |= param_numeric(ui, "inhibition amount", &mut params.film_render.dir_couplers.amount, "film_render.dir_couplers.amount", "Global multiplier on the DIR coupler inhibition matrix. 1.0 leaves the per-channel gammas as-is.");
            }
            if filter_matches(filter, "inhibition samelayer") {
                flags.runtime_changed |= param_numeric(ui, "inhibition samelayer", &mut params.film_render.dir_couplers.inhibition_samelayer, "film_render.dir_couplers.inhibition_samelayer", "Multiplier on the same-layer (diagonal) inhibition. Controls overall contrast / gamma reduction within each RGB layer.");
            }
            if filter_matches(filter, "inhibition interlayer") {
                flags.runtime_changed |= param_numeric(ui, "inhibition interlayer", &mut params.film_render.dir_couplers.inhibition_interlayer, "film_render.dir_couplers.inhibition_interlayer", "Multiplier on the cross-layer (off-diagonal) inhibition. Controls saturation enhancement from interlayer DIR effects.");
            }
            if filter_matches(filter, "gamma samelayer rgb") {
                flags.runtime_changed |= param_tuple(ui, "gamma samelayer rgb", &mut params.film_render.dir_couplers.gamma_samelayer_rgb, "film_render.dir_couplers.gamma_samelayer_rgb", "Per-channel same-layer DIR gamma (R, G, B). Effective gamma reduction of each layer's density curve.");
            }
            if filter_matches(filter, "gamma interlayer r to gb") {
                flags.runtime_changed |= param_tuple(ui, "gamma interlayer r to gb", &mut params.film_render.dir_couplers.gamma_interlayer_r_to_gb, "film_render.dir_couplers.gamma_interlayer_r_to_gb", "DIR inhibition from the R layer onto the G and B layers respectively (g_R->G, g_R->B).");
            }
            if filter_matches(filter, "gamma interlayer g to rb") {
                flags.runtime_changed |= param_tuple(ui, "gamma interlayer g to rb", &mut params.film_render.dir_couplers.gamma_interlayer_g_to_rb, "film_render.dir_couplers.gamma_interlayer_g_to_rb", "DIR inhibition from the G layer onto the R and B layers respectively (g_G->R, g_G->B).");
            }
            if filter_matches(filter, "gamma interlayer b to rg") {
                flags.runtime_changed |= param_tuple(ui, "gamma interlayer b to rg", &mut params.film_render.dir_couplers.gamma_interlayer_b_to_rg, "film_render.dir_couplers.gamma_interlayer_b_to_rg", "DIR inhibition from the B layer onto the R and G layers respectively (g_B->R, g_B->G).");
            }
            if filter_matches(filter, "langmuir donor k rgb") {
                flags.runtime_changed |= param_tuple(ui, "langmuir donor k rgb", &mut params.film_render.dir_couplers.langmuir_donor_k_rgb, "film_render.dir_couplers.langmuir_donor_k_rgb", "NEGATIVE film: per-channel Langmuir saturation of DIR inhibitor release, normalized to each layer's d_max (R, G, B). Lower = earlier, stronger roll-off at high density (gentler shoulder, tames high-amount breakage); large values approach the linear model.");
            }
            if filter_matches(filter, "langmuir receiver k rgb") {
                flags.runtime_changed |= param_tuple(ui, "langmuir receiver k rgb", &mut params.film_render.dir_couplers.langmuir_receiver_k_rgb, "film_render.dir_couplers.langmuir_receiver_k_rgb", "POSITIVE/reversal film: per-channel receiver-side Langmuir saturation (R, G, B), normalized to the arrived-inhibitor ceiling (the receiver analogue of langmuir_donor_k_rgb's d_max normalization). The donor (silver release) stays linear; this rolls off the receiving layer's response under a pushed amount. Lower = harder, more crash-proof knee (alters the operating-point look); large values approach linear.");
            }
            if filter_matches(filter, "diffusion size um") {
                flags.runtime_changed |= param_numeric(ui, "diffusion size um", &mut params.film_render.dir_couplers.diffusion_size_um, "film_render.dir_couplers.diffusion_size_um", "Sigma in um for the diffusion of the couplers, (5-20 um), controls sharpness and affects saturation.");
            }
            if filter_matches(filter, "diffusion tail um") {
                flags.runtime_changed |= param_numeric(ui, "diffusion tail um", &mut params.film_render.dir_couplers.diffusion_tail_um, "film_render.dir_couplers.diffusion_tail_um", "Exponential tail scale in um for long-range coupler spread. Larger values extend the rare long-distance inhibition halo beyond the Gaussian core.");
            }
            if filter_matches(filter, "diffusion tail weight") {
                flags.runtime_changed |= param_numeric(ui, "diffusion tail weight", &mut params.film_render.dir_couplers.diffusion_tail_weight, "film_render.dir_couplers.diffusion_tail_weight", "Fraction of coupler diffusion energy assigned to the long tail. 0 = pure Gaussian spread, higher values add a longer-range inhibition shoulder.");
            }
        });
    }
}
