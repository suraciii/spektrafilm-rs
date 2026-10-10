//! Session-only native diagnostics. No field belongs to persisted GUI state.
use eframe::egui;
pub use spektrafilm_core::telemetry::BackendRequested;
use spektrafilm_core::telemetry::{
    ColorSpaceRole, Operation, Outcome, Report, validate_report_destination,
};
use spektrafilm_gpu::telemetry::CollectionMode;
use std::{collections::VecDeque, path::Path};

#[derive(Default)]
pub(crate) struct Diagnostics {
    pub mode: CollectionMode,
    history: VecDeque<Report>,
    selected: Option<u64>,
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
    pub fn finish(&mut self, operation: Operation, outcome: Outcome) {
        if let Some(report) = operation.finish(outcome) {
            self.retain(report);
        }
    }

    fn retain(&mut self, report: Report) {
        self.selected = Some(report.operation.id);
        self.history.push_front(report);
        self.history.truncate(20);
    }

    pub fn clear(&mut self) {
        self.history.clear();
        self.selected = None;
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
        egui::CollapsingHeader::new("Diagnostics").default_open(true).show(ui, |ui| {
            egui::ComboBox::from_id_salt("diagnostics-mode").selected_text(mode_label(self.mode)).show_ui(ui, |ui| {
                for mode in [CollectionMode::Off, CollectionMode::Summary, CollectionMode::GpuTiming] { ui.selectable_value(&mut self.mode, mode, mode_label(mode)); }
            });
            ui.small("Session only. Changes apply to the next accepted operation; they do not render.");
            for (kind, mode) in render.into_iter().chain(export) { ui.label(format!("{kind} in flight · {} (captured)", mode_label(mode))); }
            if let Some((kind, mode)) = queued { ui.label(format!("{kind} queued · {} (captured)", mode_label(mode))); }
            if self.history.is_empty() { ui.label("No completed reports. Collection applies to the next operation."); }
            for report in &self.history {
                let label = format!("#{} {} · {}", report.operation.id, words(&report.operation.kind), words(&report.operation.outcome));
                ui.selectable_value(&mut self.selected, Some(report.operation.id), label);
            }
            if let Some(report) = self.history.iter().find(|report| Some(report.operation.id) == self.selected) {
                let summary = report.summary();
                let dimensions = summary.working_dimensions.map_or_else(|| "working dimensions unavailable".into(), |[w,h]| format!("{w} × {h}"));
                let duration = summary.duration_seconds.map_or_else(|| "duration unavailable".into(), |value| format!("{:.1} ms", value * 1000.0));
                ui.label(format!("{} · {} · {dimensions} · {duration}", words(&summary.kind), words(&summary.outcome)));
                ui.label(format!("Requested: {} · Selected: {} · Route: {}", optional_words(summary.backend_requested), optional_words(summary.backend_selected), optional_words(summary.path)));
                if let Some((phase, duration)) = summary.largest_phase { ui.label(format!("Largest nonnested phase: {} · {:.1} ms", phase.replace('_', " "), duration * 1000.0)); }
                if summary.software_adapter { ui.label("Software compute adapter — not a hardware GPU."); }
                for reason in summary.resident_decline_reasons { ui.label(format!("Resident route declined: {}", words(&reason))); }
                for (stage, reason) in summary.cpu_stage_reasons { ui.label(format!("CPU {}: {}", stage.replace('_', " "), words(&reason))); }
                if let Some(source) = &report.source_render { ui.label(format!("Pixels from render #{} · {} × {} · diagnostics {}", source.operation_id, source.dimensions[0], source.dimensions[1], if source.diagnostics_collected { "collected" } else { "not collected" })); }
                ui.collapsing("Phase durations (inclusive; nested costs are not additive)", |ui| { details(ui, &report.phases); });
                ui.collapsing("Stages and CPU reasons", |ui| { details(ui, &report.stages); });
                ui.collapsing("Logical transfers and work counters", |ui| { ui.small("Logical bytes are not physical bus traffic. Host waits are not shader time."); details(ui, &report.measurements); });
                ui.collapsing("GPU passes and measurement availability", |ui| { ui.small("Pass intervals are not operation GPU duration. Unavailable values are not zero."); details(ui, &report.gpu_batches); details(ui, &report.execution); details(ui, &report.coverage); details(ui, &report.diagnostic_issues); });
                save = ui.button("Save report…").clicked();
            }
        });
        save
    }
}

fn mode_label(mode: CollectionMode) -> &'static str {
    match mode {
        CollectionMode::Off => "Off",
        CollectionMode::Summary => "Summary",
        CollectionMode::GpuTiming => "GPU timing",
    }
}
fn words(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .map(|value| {
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
                .replace('_', " ")
        })
        .unwrap_or_else(|_| "unavailable".into())
}
fn optional_words<T: serde::Serialize>(value: Option<T>) -> String {
    value
        .as_ref()
        .map(words)
        .unwrap_or_else(|| "not applicable / not reached".into())
}
fn details(ui: &mut egui::Ui, value: &impl serde::Serialize) {
    match serde_json::to_value(value) {
        Ok(value) => show_value(ui, None, &value),
        Err(_) => {
            ui.label("Diagnostic details unavailable.");
        }
    }
}
fn show_value(ui: &mut egui::Ui, key: Option<&str>, value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                ui.push_id(key, |ui| {
                    if value.is_object() || value.is_array() {
                        ui.collapsing(key.replace('_', " "), |ui| show_value(ui, None, value));
                    } else {
                        show_value(ui, Some(key), value);
                    }
                });
            }
        }
        serde_json::Value::Array(values) => {
            if values.is_empty() {
                ui.label("No observations in this scope.");
            }
            for (index, value) in values.iter().enumerate() {
                ui.push_id(index, |ui| show_value(ui, None, value));
            }
        }
        _ => {
            ui.label(format!(
                "{}{}",
                key.map_or_else(String::new, |key| format!("{}: ", key.replace('_', " "))),
                value
                    .as_str()
                    .map_or_else(|| value.to_string(), |text| text.replace('_', " "))
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
    fn changing_mode_does_not_change_accepted_operation() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.mode = CollectionMode::Summary;
        let operation = Operation::new(OperationKind::Preview, diagnostics.mode);
        diagnostics.mode = CollectionMode::Off;
        diagnostics.finish(operation, Outcome::Superseded);
        assert_eq!(diagnostics.history.len(), 1);
        assert_eq!(
            diagnostics.history.front().unwrap().operation.outcome,
            Outcome::Superseded
        );
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
}
