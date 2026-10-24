//! Session-only native diagnostics. No field belongs to persisted GUI state.
use eframe::egui;
use spektrafilm_backend::telemetry::CollectionMode;
pub use spektrafilm_core::telemetry::BackendRequested;
use spektrafilm_core::telemetry::{
    ColorSpaceRole, Operation, Outcome, Report, validate_report_destination,
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(crate) struct Diagnostics {
    pub mode: CollectionMode,
    history: VecDeque<Report>,
    selected: Option<u64>,
    user_selected: bool,
    publication: Option<PathBuf>,
    publication_error: Option<String>,
}

pub(crate) fn color_space(value: &str) -> ColorSpaceRole {
    ColorSpaceRole::parse(value)
}

pub(crate) fn preview_backend_requested() -> BackendRequested {
    match std::env::var("SPEKTRAFILM_BACKEND")
        .ok()
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("cpu") => BackendRequested::Cpu,
        Some("wgpu") => BackendRequested::Wgpu,
        _ => BackendRequested::Auto,
    }
}

impl Diagnostics {
    /// Publish every completed operation's report as a new file in `directory`.
    ///
    /// A launch-time choice: it stays out of saved GUI state, it never replaces
    /// an existing file, and a failure surfaces as a status diagnostic instead of
    /// failing the operation.
    pub fn publish_completed_to(&mut self, directory: PathBuf) {
        self.publication = Some(directory);
    }

    pub fn take_publication_error(&mut self) -> Option<String> {
        self.publication_error.take()
    }

    pub fn finish(&mut self, operation: Operation, outcome: Outcome) {
        if let Some(report) = operation.finish(outcome) {
            if let Err(error) = self.publish(&report) {
                self.publication_error = Some(error);
            }
            self.retain(report);
        }
    }

    fn publish(&self, report: &Report) -> Result<(), String> {
        let Some(directory) = self.publication.as_deref() else {
            return Ok(());
        };
        let path = directory.join(format!("operation-{}.json", report.operation.id));
        validate_report_destination(&path, &[])
            .and_then(|destination| destination.write_new(report))
            .map_err(|error| error.to_string())
    }

    fn retain(&mut self, report: Report) {
        if !self.user_selected {
            self.selected = Some(report.operation.id);
        }
        self.history.push_front(report);
        self.history.truncate(20);
        // If the selected report aged out of history, resume auto-follow.
        if self.user_selected
            && !self
                .history
                .iter()
                .any(|r| Some(r.operation.id) == self.selected)
        {
            self.user_selected = false;
            self.selected = self.history.front().map(|r| r.operation.id);
        }
    }

    #[cfg(test)]
    pub(crate) fn selected_report_id(&self) -> Option<u64> {
        self.selected
    }

    pub fn clear(&mut self) {
        self.history.clear();
        self.selected = None;
        self.user_selected = false;
    }

    pub fn save_selected(&self, path: &Path) -> Result<(), String> {
        let report = self
            .history
            .iter()
            .find(|report| Some(report.operation.id) == self.selected)
            .ok_or("Select a completed operation first")?;
        validate_report_destination(path, &[])
            .and_then(|destination| destination.write_confirmed(report))
            .map_err(|error| error.to_string())
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        render: Option<(&str, CollectionMode)>,
        queued: Option<(&str, CollectionMode)>,
        export: Option<(&str, CollectionMode)>,
    ) -> bool {
        let mut save = false;
        egui::CollapsingHeader::new(crate::i18n::text(ui.ctx(), "surfaces.diagnostics.title"))
            .id_salt("Diagnostics")
            .default_open(true)
            .show(ui, |ui| {
                let mode_response = egui::ComboBox::from_id_salt("diagnostics-mode")
                    .selected_text(mode_label(ui.ctx(), self.mode))
                    .show_ui(ui, |ui| {
                        for mode in [
                            CollectionMode::Off,
                            CollectionMode::Summary,
                            CollectionMode::GpuTiming,
                        ] {
                            ui.selectable_value(&mut self.mode, mode, mode_label(ui.ctx(), mode));
                        }
                    })
                    .response;
                crate::controls::locate_row(ui, "diagnostics.collection_mode", &mode_response);
                ui.small(crate::i18n::text(ui.ctx(), "surfaces.diagnostics.session"));
                for (kind, mode) in render.into_iter().chain(export) {
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "surfaces.diagnostics.in_flight",
                        kind = crate::controls::localized(ui.ctx(), kind),
                        mode = mode_label(ui.ctx(), mode)
                    ));
                }
                if let Some((kind, mode)) = queued {
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "surfaces.diagnostics.queued",
                        kind = crate::controls::localized(ui.ctx(), kind),
                        mode = mode_label(ui.ctx(), mode)
                    ));
                }
                if self.history.is_empty() {
                    ui.label(crate::i18n::text(ui.ctx(), "surfaces.diagnostics.empty"));
                }
                for report in &self.history {
                    let label = format!(
                        "#{} {} · {}",
                        report.operation.id,
                        words(&report.operation.kind),
                        words(&report.operation.outcome)
                    );
                    if ui
                        .selectable_value(&mut self.selected, Some(report.operation.id), label)
                        .clicked()
                    {
                        self.user_selected = true;
                    }
                }
                if let Some(report) = self
                    .history
                    .iter()
                    .find(|report| Some(report.operation.id) == self.selected)
                {
                    let summary = report.summary();
                    let dimensions = summary.working_dimensions.map_or_else(
                        || {
                            crate::i18n::text(ui.ctx(), "surfaces.diagnostics.dimensions_missing")
                                .into_owned()
                        },
                        |[w, h]| format!("{w} × {h}"),
                    );
                    let duration = summary.duration_seconds.map_or_else(
                        || {
                            crate::i18n::text(ui.ctx(), "surfaces.diagnostics.duration_missing")
                                .into_owned()
                        },
                        |value| format!("{:.1} ms", value * 1000.0),
                    );
                    ui.label(format!(
                        "{} · {} · {dimensions} · {duration}",
                        words(&summary.kind),
                        words(&summary.outcome)
                    ));
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "surfaces.diagnostics.backend",
                        requested = optional_words(ui.ctx(), summary.backend_requested),
                        selected = optional_words(ui.ctx(), summary.backend_selected),
                        route = optional_words(ui.ctx(), summary.path)
                    ));
                    if let Some((phase, duration)) = summary.largest_phase {
                        ui.label(crate::tr!(
                            ui.ctx(),
                            "surfaces.diagnostics.phase",
                            phase = phase,
                            ms = format!("{:.1}", duration * 1000.0)
                        ));
                    }
                    if summary.software_adapter {
                        ui.label(crate::i18n::text(ui.ctx(), "surfaces.diagnostics.software"));
                    }
                    for reason in summary.resident_decline_reasons {
                        ui.label(crate::tr!(
                            ui.ctx(),
                            "surfaces.diagnostics.declined",
                            reason = words(&reason)
                        ));
                    }
                    for (stage, reason) in summary.cpu_stage_reasons {
                        ui.label(crate::tr!(
                            ui.ctx(),
                            "surfaces.diagnostics.cpu_reason",
                            stage = stage,
                            reason = words(&reason)
                        ));
                    }
                    if let Some(source) = &report.source_render {
                        ui.label(crate::tr!(
                            ui.ctx(),
                            "surfaces.diagnostics.source",
                            id = source.operation_id,
                            width = source.dimensions[0],
                            height = source.dimensions[1],
                            state = crate::i18n::text(
                                ui.ctx(),
                                if source.diagnostics_collected {
                                    "surfaces.diagnostics.collected"
                                } else {
                                    "surfaces.diagnostics.not_collected"
                                }
                            )
                        ));
                    }
                    egui::CollapsingHeader::new(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.diagnostics.phases",
                    ))
                    .id_salt("diagnostics.phases")
                    .show(ui, |ui| {
                        details(ui, &report.phases);
                    });
                    egui::CollapsingHeader::new(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.diagnostics.stages",
                    ))
                    .id_salt("diagnostics.stages")
                    .show(ui, |ui| {
                        details(ui, &report.stages);
                    });
                    egui::CollapsingHeader::new(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.diagnostics.transfers",
                    ))
                    .id_salt("diagnostics.transfers")
                    .show(ui, |ui| {
                        ui.small(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.diagnostics.transfers_help",
                        ));
                        details(ui, &report.measurements);
                    });
                    egui::CollapsingHeader::new(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.diagnostics.gpu",
                    ))
                    .id_salt("diagnostics.gpu")
                    .show(ui, |ui| {
                        ui.small(crate::i18n::text(ui.ctx(), "surfaces.diagnostics.gpu_help"));
                        details(ui, &report.gpu_batches);
                        details(ui, &report.execution);
                        details(ui, &report.coverage);
                        details(ui, &report.diagnostic_issues);
                    });
                    let response =
                        ui.button(crate::i18n::text(ui.ctx(), "surfaces.diagnostics.save"));
                    crate::controls::locate_row(ui, "action.save_report", &response);
                    save = response.clicked();
                }
            });
        save
    }
}

fn mode_label(ctx: &egui::Context, mode: CollectionMode) -> std::borrow::Cow<'static, str> {
    crate::i18n::text(
        ctx,
        match mode {
            CollectionMode::Off => "surfaces.diagnostics.off",
            CollectionMode::Summary => "surfaces.diagnostics.summary",
            CollectionMode::GpuTiming => "surfaces.diagnostics.gpu_timing",
        },
    )
}

fn words(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .map(|value| {
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
        })
        .unwrap_or_else(|_| "unavailable".into())
}
fn optional_words<T: serde::Serialize>(ctx: &egui::Context, value: Option<T>) -> String {
    value.as_ref().map(words).unwrap_or_else(|| {
        crate::i18n::text(ctx, "surfaces.diagnostics.not_applicable").into_owned()
    })
}
fn details(ui: &mut egui::Ui, value: &impl serde::Serialize) {
    match serde_json::to_value(value) {
        Ok(value) => show_value(ui, None, &value),
        Err(_) => {
            ui.label(crate::i18n::text(
                ui.ctx(),
                "surfaces.diagnostics.unavailable",
            ));
        }
    }
}
fn show_value(ui: &mut egui::Ui, key: Option<&str>, value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                ui.push_id(key, |ui| {
                    if value.is_object() || value.is_array() {
                        ui.collapsing(key, |ui| show_value(ui, None, value));
                    } else {
                        show_value(ui, Some(key), value);
                    }
                });
            }
        }
        serde_json::Value::Array(values) => {
            if values.is_empty() {
                ui.label(crate::i18n::text(
                    ui.ctx(),
                    "surfaces.diagnostics.no_observations",
                ));
            }
            for (index, value) in values.iter().enumerate() {
                ui.push_id(index, |ui| show_value(ui, None, value));
            }
        }
        _ => {
            ui.label(format!(
                "{}{}",
                key.map_or_else(String::new, |key| format!("{key}: ")),
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_owned)
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_core::telemetry::{OperationKind, SourceRender};
    #[test]
    fn history_is_bounded_newest_first_and_source_survives_eviction() {
        let mut diagnostics = Diagnostics::default();
        let original = Operation::new(OperationKind::Preview, CollectionMode::Summary);
        let source = SourceRender::new(
            original.id(),
            Some(3),
            Some(7),
            12,
            8,
            true,
            ColorSpaceRole::Srgb,
            true,
        );
        diagnostics.finish(original, Outcome::Succeeded);
        for _ in 0..25 {
            diagnostics.finish(
                Operation::new(OperationKind::Export, CollectionMode::Summary),
                Outcome::Cancelled,
            );
        }
        assert_eq!(diagnostics.history.len(), 20);
        assert_eq!(
            diagnostics.selected,
            diagnostics
                .history
                .front()
                .map(|report| report.operation.id)
        );
        assert!(
            !diagnostics
                .history
                .iter()
                .any(|report| report.operation.id == source.operation_id)
        );
        let mut save = Operation::new(OperationKind::Save, CollectionMode::Summary);
        save.set_source_render(source.clone());
        diagnostics.finish(save, Outcome::Succeeded);
        assert_eq!(
            diagnostics.history.front().unwrap().source_render.as_ref(),
            Some(&source)
        );
    }
    #[test]
    fn off_output_can_be_saved_with_collected_provenance() {
        let render = Operation::new(OperationKind::Scan, CollectionMode::Off);
        let source = SourceRender::new(
            render.id(),
            Some(1),
            Some(2),
            4,
            3,
            false,
            ColorSpaceRole::Srgb,
            false,
        );
        let mut diagnostics = Diagnostics::default();
        diagnostics.finish(render, Outcome::Succeeded);
        assert!(diagnostics.history.is_empty());
        let mut save = Operation::new(OperationKind::Save, CollectionMode::Summary);
        save.set_source_render(source.clone());
        diagnostics.finish(save, Outcome::Failed);
        let report = diagnostics.history.front().unwrap();
        assert_eq!(report.operation.outcome, Outcome::Failed);
        assert_eq!(report.source_render.as_ref(), Some(&source));
        assert!(report.attempts.is_empty());
    }
    #[test]
    fn report_export_and_persistence_failure_do_not_change_history() {
        let mut diagnostics = Diagnostics::default();
        let operation = Operation::new(OperationKind::Preview, CollectionMode::Summary);
        let id = operation.id();
        diagnostics.finish(operation, Outcome::Succeeded);
        let directory = std::env::temp_dir().join(format!(
            "spektrafilm-gui-report-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("report.json");
        std::fs::write(&path, "old confirmed destination").unwrap();
        diagnostics.save_selected(&path).unwrap();
        let report = Report::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(report.operation.id, id);
        assert!(
            diagnostics
                .save_selected(&directory.join("missing").join("report.json"))
                .is_err()
        );
        assert_eq!(diagnostics.history.len(), 1);
        assert_eq!(diagnostics.selected, Some(id));
        assert_eq!(
            diagnostics.history.front().unwrap().operation.outcome,
            Outcome::Succeeded
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn publication_writes_one_new_file_per_completed_operation() {
        let directory = std::env::temp_dir().join(format!(
            "spektrafilm-gui-publication-{}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let mut diagnostics = Diagnostics::default();
        diagnostics.publish_completed_to(directory.clone());
        let preview = Operation::new(OperationKind::Preview, CollectionMode::Summary);
        let preview_id = preview.id();
        diagnostics.finish(preview, Outcome::Succeeded);
        let scan = Operation::new(OperationKind::Scan, CollectionMode::Summary);
        let scan_id = scan.id();
        diagnostics.finish(scan, Outcome::Succeeded);
        assert_ne!(preview_id, scan_id);
        for (id, kind) in [
            (preview_id, OperationKind::Preview),
            (scan_id, OperationKind::Scan),
        ] {
            let path = directory.join(format!("operation-{id}.json"));
            let report = Report::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(report.operation.id, id);
            assert_eq!(report.operation.kind, kind);
            assert_eq!(report.operation.outcome, Outcome::Succeeded);
        }
        assert!(diagnostics.take_publication_error().is_none());
        // Every completed operation publishes its own file; none replaces another.
        diagnostics.finish(
            Operation::new(OperationKind::Preview, CollectionMode::Summary),
            Outcome::Succeeded,
        );
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 3);
        std::fs::remove_dir_all(&directory).unwrap();
    }
    #[test]
    fn publication_failure_surfaces_without_losing_the_report() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.publish_completed_to(std::env::temp_dir().join(format!(
            "spektrafilm-gui-missing-publication-{}",
            std::process::id()
        )));
        diagnostics.finish(
            Operation::new(OperationKind::Preview, CollectionMode::Summary),
            Outcome::Succeeded,
        );
        assert!(diagnostics.take_publication_error().is_some());
        assert_eq!(diagnostics.history.len(), 1);
        assert_eq!(
            diagnostics.history.front().unwrap().operation.outcome,
            Outcome::Succeeded
        );
        assert!(diagnostics.take_publication_error().is_none());
    }
}
