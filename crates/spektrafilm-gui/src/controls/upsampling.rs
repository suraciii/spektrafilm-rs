use super::*;
use egui::Ui;

/// One `show()` section group, split out of the dispatcher for readability.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    section: &str,
    flags: &mut ChangeFlags,
) {
    if section == "Spectral upsampling" {
        egui::CollapsingHeader::new("Spectral upsampling").default_open(false).show(ui, |ui| {
            flags.runtime_changed |= choice_tip(ui, "spectral upsampling", &mut params.settings.rgb_to_raw_method, &["arctic2026beta04", "hanatos2025", "mallett2019"], "Method to upsample the spectral resolution of the image, hanatos2025 works on the full visible locus, mallett2019 works only on sRGB (will clip input).");
            flags.runtime_changed |= toggle(ui, "hanatos2025 adaptation window", &mut params.settings.apply_hanatos2025_adaptation_window, "Apply the hanatos2025 bandpass adaptation window when reconstructing spectra.");
            flags.runtime_changed |= toggle(ui, "hanatos2025 adaptation surface", &mut params.settings.apply_hanatos2025_adaptation_surface, "Apply the hanatos2025 surface adaptation polynomial when reconstructing spectra.");
            flags.runtime_changed |= param_numeric(ui, "spectral gaussian blur", &mut params.settings.spectral_gaussian_blur, "settings.spectral_gaussian_blur", "Sigma in nm for Gaussian blur applied to reconstructed spectra.");
        });
    }
}
