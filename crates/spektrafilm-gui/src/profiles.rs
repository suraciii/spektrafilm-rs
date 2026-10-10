use std::path::Path;

use eframe::egui;
use spektrafilm_core::profile;

/// One entry in the film / paper combo box. `stock` is the filename
/// stem (the unique key the profile loader expects); `display` is the
/// human-readable label from the profile's `info.name`, falling back
/// to the stock id when missing.
#[derive(Debug, Clone)]
pub(super) struct ProfileEntry {
    stock: String,
    display: String,
}

/// Scan `<data_dir>/profiles/*.json`, parse each profile's `info`, and
/// bucket the results by `info.support`. Films go into the first vec,
/// papers (and any other print-stage supports) into the second.
/// Each entry carries the filename stem (the unique loader key) plus a
/// human-readable display label.
pub(super) fn scan_profiles(data_dir: &Path) -> (Vec<ProfileEntry>, Vec<ProfileEntry>) {
    let mut films = Vec::new();
    let mut papers = Vec::new();
    let dir = data_dir.join("profiles");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (films, papers);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_lossy = name.to_string_lossy().to_string();
        let Some(stem) = name_lossy.strip_suffix(".json") else {
            continue;
        };
        let stock = stem.to_string();
        // Cheap probe: just deserialize the file's `info` field. We
        // could skip the rest of the profile but `Profile` already does
        // the right thing — and we pay this once at startup.
        let (display, is_paper) = match profile::load_profile_by_name(data_dir, &stock) {
            Ok(p) => {
                let display = p.info.name.clone().unwrap_or_else(|| stock.clone());
                let is_paper = p.info.support == "paper" || p.info.stage == "printing";
                (display, is_paper)
            }
            Err(_) => (stock.clone(), false),
        };
        let entry = ProfileEntry { stock, display };
        if is_paper {
            papers.push(entry);
        } else {
            films.push(entry);
        }
    }
    films.sort_by(|a, b| a.display.cmp(&b.display));
    papers.sort_by(|a, b| a.display.cmp(&b.display));
    (films, papers)
}

/// Load the development-time family of a profile (empty on load failure).
pub(super) fn profile_dev_times(data_dir: &Path, stock: &str) -> Vec<f64> {
    profile::load_profile_by_name(data_dir, stock)
        .map(|p| p.data.development_time)
        .unwrap_or_default()
}

/// Development-time picker for a B&W development-time family. `selection`
/// of `None` means the profile's default (the floor-middle entry, matching
/// upstream's `select_development_time`). Returns true when changed.
pub(super) fn dev_time_combo(
    ui: &mut egui::Ui,
    salt: &str,
    label: &str,
    times: &[f64],
    selection: &mut Option<f64>,
) -> bool {
    let times = if times.is_empty() { &[1.0][..] } else { times };
    // Same resolution as the render path, so the combo always highlights
    // exactly the entry the pipeline will use.
    let current_idx = profile::development_time_index(times, *selection);
    let mut changed = false;
    let tooltip = "Development time for a BW development-time family: selects the density curve and base+fog to render. '—' uses the representative middle development; ignored for single-curve and color stocks.";
    ui.label(label).on_hover_text(tooltip);
    egui::ComboBox::from_id_salt(salt)
        .selected_text(format!("{} min", times[current_idx]))
        .width(ui.available_width().min(280.0))
        .show_ui(ui, |ui| {
            for (i, t) in times.iter().enumerate() {
                if ui
                    .selectable_label(i == current_idx, format!("{t} min"))
                    .clicked()
                    && i != current_idx
                {
                    *selection = Some(*t);
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    changed
}

/// Combo box that picks one of `entries` by its `stock` id (the
/// underlying file stem) while showing `display` (the human-readable
/// name) as the label. Falls back to showing the raw stock id if no
/// entry with the current `selected_stock` exists.
pub(super) fn profile_combo(
    ui: &mut egui::Ui,
    salt: &str,
    label: &str,
    entries: &[ProfileEntry],
    selected_stock: &mut String,
) -> bool {
    let tooltip = if salt == "film" {
        "Film stock to simulate"
    } else {
        "Print stock to simulate"
    };
    ui.label(label).on_hover_text(tooltip);
    let display = entries
        .iter()
        .find(|e| &e.stock == selected_stock)
        .map(|e| e.display.clone())
        .unwrap_or_else(|| selected_stock.clone());
    let prev = selected_stock.clone();
    egui::ComboBox::from_id_salt(salt)
        .selected_text(&display)
        .width(ui.available_width().min(280.0))
        .show_ui(ui, |ui| {
            for entry in entries {
                ui.selectable_value(selected_stock, entry.stock.clone(), &entry.display);
            }
        })
        .response
        .on_hover_text(tooltip);
    prev != *selected_stock
}
