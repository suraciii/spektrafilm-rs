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
    if section == "Spectral upsampling" {
        section_header(ui, filter, "Spectral upsampling", |ui, filter| {
            if filter_matches(filter, "spectral upsampling") {
                flags.runtime_changed |= choice_tip(
                    ui,
                    "spectral upsampling",
                    &mut params.settings.rgb_to_raw_method,
                    &["arctic2026beta04", "hanatos2025", "mallett2019"],
                    "Method to upsample the spectral resolution of the image, hanatos2025 works on the full visible locus, mallett2019 works only on sRGB (will clip input).",
                );
            }
            if filter_matches(filter, "hanatos2025 adaptation window") {
                flags.runtime_changed |= toggle(
                    ui,
                    "hanatos2025 adaptation window",
                    &mut params.settings.apply_hanatos2025_adaptation_window,
                    "Apply the hanatos2025 bandpass adaptation window when reconstructing spectra.",
                );
            }
            if filter_matches(filter, "hanatos2025 adaptation surface") {
                flags.runtime_changed |= toggle(
                    ui,
                    "hanatos2025 adaptation surface",
                    &mut params.settings.apply_hanatos2025_adaptation_surface,
                    "Apply the hanatos2025 surface adaptation polynomial when reconstructing spectra.",
                );
            }
            if filter_matches(filter, "spectral gaussian blur") {
                flags.runtime_changed |= param_numeric(
                    ui,
                    "spectral gaussian blur",
                    &mut params.settings.spectral_gaussian_blur,
                    "settings.spectral_gaussian_blur",
                    "Sigma in nm for Gaussian blur applied to reconstructed spectra.",
                );
            }
            if filter_matches(filter, "use cat16") {
                flags.runtime_changed |= toggle(
                    ui,
                    "use cat16",
                    &mut params.settings.use_cat16,
                    "Use the CAT16 chromatic adaptation transform for the input RGB to film-space adaptation that feeds the Hanatos spectral upsampling. Off selects CAT02, the pre-0.3.4 behavior.",
                );
            }
            if filter_matches(filter, "print exposure ev tracking") {
                flags.runtime_changed |= enum_choice(
                    ui,
                    "print exposure ev tracking",
                    &mut params.settings.print_exposure_ev_tracking,
                    &[
                        (PrintExposureEvTracking::FollowMode, "follow_mode"),
                        (PrintExposureEvTracking::Never, "never"),
                        (PrintExposureEvTracking::Always, "always"),
                    ],
                    "How the print exposure follows camera exposure compensation. follow_mode tracks it in auto mode and does not track it in manual; never disables tracking in auto; always applies the relative compensation factor in manual.",
                );
            }
        });
    }
}
