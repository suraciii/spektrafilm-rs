use std::path::{Path, PathBuf};

use eframe::egui;
use spektrafilm_core::profile;

/// One entry in the film / paper combo box. `stock` is the filename
/// stem (the unique key the profile loader expects); `display` is the
/// human-readable label from the profile's `info.name`, falling back
/// to the stock id when missing.
#[derive(Debug, Clone)]
pub(super) struct ProfileEntry {
    pub(super) stock: String,
    pub(super) display: String,
}

/// Find the data directory. Probes, in order:
///   1. `$SPEKTRAFILM_DATA_DIR` — explicit override.
///   2. [`spektrafilm_core::config::data_dir_candidates`] — the exe-relative
///      install layouts, the working directory, then the workspace root for
///      `cargo run -p spektrafilm-gui`.
///
/// Finder launches apps with `cwd=/`, so the exe-relative probes matter for a
/// double-clicked `.app`. Falls back to `./data` so the downstream loader
/// surfaces a clear "profiles not found" error.
pub(super) fn pick_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SPEKTRAFILM_DATA_DIR") {
        return PathBuf::from(dir);
    }
    spektrafilm_core::config::data_dir_candidates()
        .into_iter()
        .find(|path| path.is_dir())
        .unwrap_or_else(|| PathBuf::from("data"))
}

/// Scan `<data_dir>/profiles/*.json`, parse each profile's `info`, and
/// bucket the results into print stocks (usable as the print stage:
/// `Profile::is_print_stock`) and everything else as films.
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
        // Full load (parse + validation) through the profile loader; the
        // bucket only needs `info`, but startup pays this once.
        let (display, is_print_stock) = match profile::load_profile_by_name(data_dir, &stock) {
            Ok(p) => {
                let display = p.info.name.clone().unwrap_or_else(|| stock.clone());
                (display, p.is_print_stock())
            }
            Err(_) => (stock.clone(), false),
        };
        let entry = ProfileEntry { stock, display };
        if is_print_stock {
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
    let label_response = ui.label(label).on_hover_text(tooltip);
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
    if let Some(response) = ui
        .ctx()
        .read_response(ui.make_persistent_id(egui::Id::new(salt)))
    {
        let id = response.id;
        response.labelled_by(label_response.id);
        // egui's unlabelled ComboBox installs an empty explicit label, which
        // overrides the associated visible label in AccessKit consumers.
        ui.ctx()
            .accesskit_node_builder(id, |node| node.clear_label());
    }
    changed
}

/// The human-readable name of the selected stock: the matching entry's
/// display label, or the raw stock id when no entry matches it.
pub(super) fn selected_display(entries: &[ProfileEntry], selected_stock: &str) -> String {
    entries
        .iter()
        .find(|e| e.stock == selected_stock)
        .map(|e| e.display.clone())
        .unwrap_or_else(|| selected_stock.to_owned())
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
    let label_response = ui.label(label).on_hover_text(tooltip);
    let display = selected_display(entries, selected_stock);
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
    if let Some(response) = ui
        .ctx()
        .read_response(ui.make_persistent_id(egui::Id::new(salt)))
    {
        let id = response.id;
        response.labelled_by(label_response.id);
        ui.ctx()
            .accesskit_node_builder(id, |node| node.clear_label());
    }
    prev != *selected_stock
}

#[cfg(test)]
mod headless {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn profile_and_development_labels_resolve_to_selectors_and_select_real_values() {
        let entries = vec![
            ProfileEntry {
                stock: "first".into(),
                display: "First film".into(),
            },
            ProfileEntry {
                stock: "second".into(),
                display: "Second film".into(),
            },
        ];
        let mut harness = Harness::new_ui_state(
            |ui, state: &mut (String, Option<f64>)| {
                profile_combo(ui, "film", "Film", &entries, &mut state.0);
                dev_time_combo(
                    ui,
                    "film-dev",
                    "Film development",
                    &[4.0, 8.0, 12.0],
                    &mut state.1,
                );
            },
            ("first".to_owned(), None),
        );
        for label in ["Film", "Film development"] {
            assert_eq!(
                harness.get_by_label(label).role(),
                egui::accesskit::Role::ComboBox
            );
        }
        let film_id = harness.get_by_label("Film").id();
        harness.get_by_label("Film").click();
        harness.run();
        harness.get_by_label("Second film").click();
        harness.step();
        assert_eq!(harness.state().0, "second");
        harness.run();
        assert_eq!(harness.get_by_label("Film").id(), film_id);
        harness.get_by_label("Film development").click();
        harness.run();
        harness.get_by_label("12 min").click();
        harness.step();
        assert_eq!(harness.state().1, Some(12.0));
    }
}
