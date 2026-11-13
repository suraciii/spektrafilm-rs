//! Native chooser boundaries and remembered dialog directories.
use super::state::App;
use crate::*;

impl App {
    pub(crate) fn file_dialog(&self, key: &str) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match self.gui_state.dialog_dirs()[key].as_str() {
            Some(path) => dialog.set_directory(path),
            None => dialog,
        }
    }

    pub(crate) fn remember_dialog(&mut self, key: &str, path: &Path) {
        if let Some(parent) = path.parent() {
            if !self.gui_state.sections["rust"].is_object() {
                self.gui_state.sections["rust"] =
                    serde_json::json!({"version":state::CURRENT_STATE_VERSION,"dialog_dirs":{}});
            }
            if !self.gui_state.sections["rust"]["dialog_dirs"].is_object() {
                self.gui_state.sections["rust"]["dialog_dirs"] = serde_json::json!({});
            }
            self.gui_state.sections["rust"]["dialog_dirs"][key] =
                serde_json::json!(parent.to_string_lossy());
            let directory = self.browser_state_path.parent().unwrap_or(Path::new("."));
            let path = directory.join("dialog_dirs.json");
            if let Err(e) = std::fs::create_dir_all(directory).and_then(|_| {
                std::fs::write(
                    path,
                    self.gui_state.sections["rust"]["dialog_dirs"].to_string(),
                )
            }) {
                self.status =
                    crate::app::Message::new("surfaces.status.directory_error").arg("error", e);
            }
        }
    }
}
