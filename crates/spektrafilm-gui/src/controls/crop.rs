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
    if section == "Crop and upscale" {
        section_header(ui, filter, "Crop and upscale", false, |ui, filter| {
            if filter_matches(filter, "upscale factor") {
                flags.runtime_changed |= param_numeric(
                    ui,
                    "upscale factor",
                    &mut params.io.upscale_factor,
                    "io.upscale_factor",
                    "Scale image size up to increase resolution",
                );
            }
            if filter_matches(filter, "crop") {
                flags.runtime_changed |= toggle(
                    ui,
                    "crop",
                    &mut params.io.crop,
                    "Crop image to a fraction of the original size to preview details at full scale",
                );
            }
            if filter_matches(filter, "crop center") {
                flags.runtime_changed |= param_tuple(
                    ui,
                    "crop center",
                    &mut params.io.crop_center,
                    "io.crop_center",
                    "Center of the crop region in relative coordinates in x, y (0-1)",
                );
            }
            if filter_matches(filter, "crop size") {
                flags.runtime_changed |= param_tuple(
                    ui,
                    "crop size",
                    &mut params.io.crop_size,
                    "io.crop_size",
                    "Normalized size of the crop region in x, y (0,1), as fraction of the long side.",
                );
            }
        });
    }
}
