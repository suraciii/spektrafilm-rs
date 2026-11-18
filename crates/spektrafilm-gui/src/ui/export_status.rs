//! The export job strip: one visible object for a delivery that outlives the
//! dialog. It states which file, which backend, how long, and how it ended,
//! and keeps the recovery actions beside the outcome instead of in the shared
//! header status line.
use crate::app::App;
use crate::app::Message;
use crate::export_job::ExportNotice;
use crate::{controls, theme};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

impl App {
    pub(super) fn export_job_strip(&mut self, ctx: &egui::Context) {
        // Queued exports keep their per-entry outcome in the work list.
        let running = self
            .export_job
            .as_ref()
            .filter(|job| job.queue_entry.is_none())
            .map(|job| {
                (
                    job.destination.clone(),
                    job.backend.label(),
                    job.started_at,
                    job.cancel.load(Ordering::SeqCst),
                )
            });
        let notice = self.export_notice.clone();
        if running.is_none() && notice.is_none() {
            return;
        }

        let mut cancel = false;
        let mut dismiss = false;
        let mut reveal: Option<PathBuf> = None;
        let mut again = false;

        let panel = egui::TopBottomPanel::top("export-job-strip")
            .exact_height(theme::HEIGHT_EXPORT_STRIP)
            .frame(
                egui::Frame::NONE
                    .fill(theme::SURFACE2)
                    .inner_margin(egui::Margin::symmetric(theme::SPACE_4 as i8, 0))
                    .stroke(egui::Stroke::new(theme::STROKE_DEFAULT, theme::BORDER)),
            )
            .show(ctx, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), theme::HEIGHT_EXPORT_STRIP),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = theme::SPACE_3;
                        let text_width = (ui.available_width() - 320.0).max(120.0);

                        match &running {
                            Some((path, backend, started, cancelling)) => {
                                ui.add(egui::Spinner::new().size(13.0).color(theme::ACCENT));
                                ui.allocate_ui_with_layout(
                                    egui::vec2(text_width, theme::HEIGHT_EXPORT_STRIP),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        path_label(ui, path, 0.62);
                                        ui.spacing_mut().item_spacing.x = theme::SPACE_2;
                                        let facts = if *cancelling {
                                            crate::tr!(
                                                ui.ctx(),
                                                "operations.export_cancelling_elapsed",
                                                seconds = started.elapsed().as_secs()
                                            )
                                            .into_owned()
                                        } else {
                                            format!(
                                                "{} · {}",
                                                backend,
                                                crate::tr!(
                                                    ui.ctx(),
                                                    "operations.export_elapsed",
                                                    seconds = started.elapsed().as_secs()
                                                )
                                            )
                                        };
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(facts)
                                                    .small()
                                                    .color(theme::FG_MUTED),
                                            )
                                            .truncate(),
                                        );
                                    },
                                );
                            }
                            None => {
                                let notice = notice.as_ref().expect("a job or its notice exists");
                                let (glyph, color) = match notice {
                                    ExportNotice::Succeeded { warning, .. } => (
                                        if warning.is_some() { "!" } else { "✓" },
                                        if warning.is_some() {
                                            theme::WARNING
                                        } else {
                                            theme::SUCCESS
                                        },
                                    ),
                                    ExportNotice::Failed { .. } => ("!", theme::ERROR),
                                    ExportNotice::Cancelled { .. } => ("–", theme::FG_MUTED),
                                };
                                ui.label(egui::RichText::new(glyph).strong().color(color));
                                ui.allocate_ui_with_layout(
                                    egui::vec2(text_width, theme::HEIGHT_EXPORT_STRIP),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        path_label(ui, notice.path(), 0.58);
                                        ui.spacing_mut().item_spacing.x = theme::SPACE_2;
                                        let (text, color) = match notice {
                                            ExportNotice::Succeeded {
                                                size,
                                                elapsed,
                                                warning,
                                                ..
                                            } => (
                                                match warning.as_ref() {
                                                    Some(warning) => warning.localized(ui.ctx()),
                                                    None => crate::tr!(
                                                        ui.ctx(),
                                                        "operations.export_done",
                                                        width = size[0],
                                                        height = size[1],
                                                        seconds = format!("{elapsed:.1}")
                                                    )
                                                    .into_owned(),
                                                },
                                                if warning.is_some() {
                                                    theme::WARNING
                                                } else {
                                                    theme::FG_MUTED
                                                },
                                            ),
                                            ExportNotice::Failed { message, .. } => {
                                                (message.localized(ui.ctx()), theme::ERROR)
                                            }
                                            ExportNotice::Cancelled { .. } => (
                                                crate::i18n::text(
                                                    ui.ctx(),
                                                    "operations.export_cancelled",
                                                )
                                                .into_owned(),
                                                theme::FG_MUTED,
                                            ),
                                        };
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(text.as_str())
                                                    .small()
                                                    .color(color),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(text.as_str());
                                    },
                                );
                            }
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = theme::SPACE_2;
                            let dismiss_label =
                                crate::i18n::text(ui.ctx(), "operations.export_dismiss");
                            if controls::icon_button(
                                ui,
                                dismiss_label.as_ref(),
                                controls::Icon::Close,
                            )
                            .clicked()
                            {
                                dismiss = true;
                            }
                            match &running {
                                Some((_, _, _, cancelling)) => {
                                    if !*cancelling {
                                        let label =
                                            crate::i18n::text(ui.ctx(), "surfaces.export.cancel");
                                        let button = ui.add(
                                            egui::Button::new(label.as_ref())
                                                .min_size(egui::vec2(0.0, 24.0)),
                                        );
                                        let button = button.on_hover_text(crate::i18n::text(
                                            ui.ctx(),
                                            "surfaces.export.cancel_help",
                                        ));
                                        controls::locate_row(ui, "action.cancel_export", &button);
                                        if button.clicked() {
                                            cancel = true;
                                        }
                                    }
                                }
                                None => {
                                    let notice = notice.as_ref().expect("notice exists");
                                    if matches!(notice, ExportNotice::Failed { .. }) {
                                        let label =
                                            crate::i18n::text(ui.ctx(), "operations.export_again");
                                        if ui
                                            .add(
                                                egui::Button::new(label.as_ref())
                                                    .min_size(egui::vec2(0.0, 24.0)),
                                            )
                                            .clicked()
                                        {
                                            again = true;
                                        }
                                    }
                                    if !matches!(notice, ExportNotice::Cancelled { .. }) {
                                        let label = crate::i18n::text(
                                            ui.ctx(),
                                            "operations.export_open_folder",
                                        );
                                        if ui
                                            .add(
                                                egui::Button::new(label.as_ref())
                                                    .min_size(egui::vec2(0.0, 24.0)),
                                            )
                                            .clicked()
                                        {
                                            reveal = Some(notice.path().to_path_buf());
                                        }
                                    }
                                }
                            }
                        });
                    },
                );
            });
        // AccessKit builders hold the context's write lock; resolve text
        // before entering them so the locale lookup cannot deadlock.
        let strip_label = crate::i18n::text(ctx, "operations.export_job");
        panel.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, strip_label.as_ref())
        });
        ctx.accesskit_node_builder(panel.response.id, |node| {
            node.set_role(egui::accesskit::Role::Status);
            node.set_label(strip_label.as_ref());
        });

        if cancel {
            self.cancel_export();
        }
        if dismiss {
            self.export_notice = None;
        }
        if again {
            self.export_notice = None;
            self.export_return_focus = None;
            self.export_dialog.open(&self.export_options);
        }
        if let Some(path) = reveal {
            self.reveal_export(&path);
        }
    }

    fn reveal_export(&mut self, path: &Path) {
        if let Err(error) = crate::presets::reveal_command(path).spawn() {
            self.status = Message::new("operations.export_reveal_error").arg("detail", error);
        }
    }
}

/// File name first: the delivery is identified by it, and the folder truncates
/// before the name does.
fn path_label(ui: &mut egui::Ui, path: &Path, share: f32) {
    let full = path.display().to_string();
    let file = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| full.clone());
    let width = (ui.available_width() * share).max(80.0);
    ui.add_sized(
        egui::vec2(width, theme::HEIGHT_CONTROL),
        egui::Label::new(egui::RichText::new(file).color(theme::FG)).truncate(),
    )
    .on_hover_text(&full);
    if let Some(folder) = path.parent().map(|parent| parent.display().to_string()) {
        if !folder.is_empty() {
            ui.add(
                egui::Label::new(egui::RichText::new(folder).small().color(theme::FG_MUTED))
                    .truncate(),
            )
            .on_hover_text(&full);
        }
    }
}
