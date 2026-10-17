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
    if section == "Halation" {
        ui.scope(|ui| {
            ui.label(
                egui::RichText::new("Key controls: halation amount (strength) · halation spatial scale (size) · scatter amount (highlight glow)")
                    .small()
                    .color(theme::FG_MUTED),
            );
            if filter_matches(filter, "enable halation") {
                flags.runtime_changed |= toggle(
                    ui,
                    "Enable halation",
                    &mut params.film_render.halation.active,
                    "Enable halation and in-emulsion scatter.",
                );
            }
            if filter_matches(filter, "scatter amount") {
                flags.runtime_changed |= param_numeric(ui, "scatter amount", &mut params.film_render.halation.scatter_amount, "film_render.halation.scatter_amount", "High-level scatter strength. 1.0 = full physical scatter, 0.0 = no scatter. Scales the fraction of light that undergoes in-emulsion scattering.");
            }
            if filter_matches(filter, "scatter spatial scale") {
                flags.runtime_changed |= param_numeric(ui, "scatter spatial scale", &mut params.film_render.halation.scatter_spatial_scale, "film_render.halation.scatter_spatial_scale", "High-level scatter size multiplier (1.0 = physical defaults). Scales both core and tail sigmas.");
            }
            if filter_matches(filter, "halation amount") {
                flags.runtime_changed |= param_numeric(ui, "halation amount", &mut params.film_render.halation.halation_amount, "film_render.halation.halation_amount", "High-level halation strength multiplier (1.0 = physical defaults). Scales the per-channel halation amplitudes.");
            }
            if filter_matches(filter, "halation spatial scale") {
                flags.runtime_changed |= param_numeric(ui, "halation spatial scale", &mut params.film_render.halation.halation_spatial_scale, "film_render.halation.halation_spatial_scale", "High-level halation size multiplier (1.0 = physical defaults). Scales the first-bounce sigma.");
            }
            if filter_matches(filter, "boost ev") {
                flags.runtime_changed |= param_numeric(ui, "boost ev", &mut params.film_render.halation.boost_ev, "film_render.halation.boost_ev", "Maximum highlight boost in stops.");
            }
            if filter_matches(filter, "protect ev") {
                flags.runtime_changed |= param_numeric(ui, "protect ev", &mut params.film_render.halation.protect_ev, "film_render.halation.protect_ev", "Protected range above midgray for the boost onset in stops.");
            }
            if filter_matches(filter, "boost range") {
                flags.runtime_changed |= param_numeric(ui, "boost range", &mut params.film_render.halation.boost_range, "film_render.halation.boost_range", "Controls how quickly the highlight boost ramps in, from 0 to 1.");
            }
            if filter_matches(filter, "scatter core um") {
                flags.runtime_changed |= param_tuple(ui, "scatter core um", &mut params.film_render.halation.scatter_core_um, "film_render.halation.scatter_core_um", "Sigma of the scatter core Gaussian per channel [R,G,B], in micrometers. Controls fine-scale sharpness loss in the emulsion.");
            }
            if filter_matches(filter, "scatter tail um") {
                flags.runtime_changed |= param_tuple(ui, "scatter tail um", &mut params.film_render.halation.scatter_tail_um, "film_render.halation.scatter_tail_um", "Decay constant of the scatter exponential tail per channel [R,G,B], in micrometers (internally approximated by a sum of Gaussians). Controls extended low-level spread within the emulsion.");
            }
            if filter_matches(filter, "scatter tail weight") {
                flags.runtime_changed |= param_tuple(ui, "scatter tail weight", &mut params.film_render.halation.scatter_tail_weight, "film_render.halation.scatter_tail_weight", "Weight of the scatter tail Gaussian per channel [R,G,B] (0-1). Tail weight + core weight = 1. Higher values put more scattered light into the long tail.");
            }
            if filter_matches(filter, "halation strength") {
                flags.runtime_changed |= param_tuple(ui, "halation strength", &mut params.film_render.halation.halation_strength, "film_render.halation.halation_strength", "Total back-reflection halation amplitude per channel [R,G,B] (0-1). Typical red channel: weak AH 0.02-0.08, no AH 0.08-0.25. The blue channel is usually near zero.");
            }
            if filter_matches(filter, "halation first sigma um") {
                flags.runtime_changed |= param_tuple(ui, "halation first sigma um", &mut params.film_render.halation.halation_first_sigma_um, "film_render.halation.halation_first_sigma_um", "Sigma of the first halation bounce per channel [R,G,B], in micrometers. Set by the base thickness (40-80 um for typical cine/still bases).");
            }
            if filter_matches(filter, "halation n bounces") {
                flags.runtime_changed |= param_numeric(ui, "halation n bounces", &mut params.film_render.halation.halation_n_bounces, "film_render.halation.halation_n_bounces", "Number of multi-bounce Gaussians summed in the halation pass. Subsequent bounces use sqrt(k)-spaced widths. Typical: 2-3.");
            }
            if filter_matches(filter, "halation bounce decay") {
                flags.runtime_changed |= param_numeric(ui, "halation bounce decay", &mut params.film_render.halation.halation_bounce_decay, "film_render.halation.halation_bounce_decay", "Per-bounce amplitude decay ratio (rho). Physical range 0.3-0.7. Controls how fast the halation energy falls off between bounces.");
            }
            if filter_matches(filter, "halation renormalize") {
                flags.runtime_changed |= toggle(ui, "halation renormalize", &mut params.film_render.halation.halation_renormalize, "If enabled, divide by (1 + sum of bounce amplitudes) so mid-grey is preserved. If disabled, halation is purely additive and subtly lifts shadows as well as highlights.");
            }
        });
    }
}
