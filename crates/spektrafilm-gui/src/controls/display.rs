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
    if section == "Session"
        && filter_matches(filter, "preview max size")
        && param_numeric(
            ui,
            "preview max size",
            &mut params.settings.preview_max_size,
            "settings.preview_max_size",
            "max size of the long edge of the preview image in pixels",
        )
    {
        set_extra(
            extras,
            "display",
            "preview_max_size",
            Value::from(params.settings.preview_max_size),
        );
        flags.runtime_changed = true;
    }
}
