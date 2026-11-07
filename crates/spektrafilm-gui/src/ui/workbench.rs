//! Window header, photograph action bar and viewer status surfaces.
use crate::*;

impl App {
    pub(super) fn simulation_action_bar(&mut self, ui: &mut egui::Ui) {
        let busy = self.render_job.is_some()
            || self.export_job.is_some()
            || self.calibration_job.is_some();
        let width = (ui.available_width() - ui.spacing().item_spacing.x) * 0.5;
        let action = |ui: &mut egui::Ui, enabled: bool, label_key, caption: &str, icon, primary| {
            let label = crate::i18n::text(ui.ctx(), label_key);
            let label_color = if !enabled {
                theme::FG_MUTED
            } else if primary {
                theme::ACCENT
            } else {
                theme::FG
            };
            let caption_color = if enabled {
                theme::FG_SECONDARY
            } else {
                theme::FG_MUTED
            };
            let icon_color = if !enabled {
                theme::FG_MUTED
            } else if primary {
                theme::ACCENT
            } else {
                theme::FG_SECONDARY
            };
            let mut text = egui::text::LayoutJob::default();
            text.append(
                &label,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(theme::SIZE_LABEL),
                    color: label_color,
                    ..Default::default()
                },
            );
            text.append("\n", 0.0, egui::TextFormat::default());
            text.append(
                caption,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(theme::SIZE_HINT),
                    color: caption_color,
                    ..Default::default()
                },
            );
            text.wrap.max_width = (width - 44.0).max(1.0);
            let galley = ui.fonts(|fonts| fonts.layout_job(text));
            let button = egui::Button::new("")
                .fill(if primary || !enabled {
                    theme::SURFACE2
                } else {
                    theme::SURFACE3
                })
                .stroke(theme::stroke(if primary && enabled {
                    theme::ACCENT_DIM
                } else {
                    theme::BORDER
                }))
                .min_size(egui::vec2(width, 64.0));
            let response = ui.add_enabled(enabled, button);
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(response.rect.left() + 20.0, response.rect.center().y),
                egui::vec2(17.0, 17.0),
            );
            controls::paint_icon(ui.painter(), icon_rect, icon, icon_color);
            ui.painter().galley(
                egui::pos2(
                    response.rect.left() + 36.0,
                    response.rect.center().y - galley.size().y * 0.5,
                ),
                galley,
                label_color,
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.as_ref())
            });
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_label(label.as_ref());
                node.set_description(caption);
            });
            controls::focus_feedback(ui, &response);
            response
        };
        ui.horizontal(|ui| {
            let scan_caption = if self.image.is_some() {
                crate::i18n::text(ui.ctx(), "surfaces.action.scan_caption")
            } else {
                crate::i18n::text(ui.ctx(), "surfaces.action.scan_unavailable")
            };
            let scan = action(
                ui,
                self.image.is_some() && !busy,
                "surfaces.action.scan",
                scan_caption.as_ref(),
                controls::Icon::Scan,
                true,
            );
            if scan
                .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.action.scan_help"))
                .clicked()
            {
                self.dirty = true;
                self.full_scan_requested = true;
            }
            let save_caption = self.retained_save_caption(ui.ctx());
            let save = action(
                ui,
                (self.output_image.is_some() || self.output_retained.is_some())
                    && !busy
                    && self.pending_saves.is_empty(),
                "surfaces.action.save_retained",
                &save_caption,
                controls::Icon::Save,
                false,
            );
            controls::locate_row(ui, "action.save_retained", &save);
            if save
                .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.action.save_help"))
                .clicked()
            {
                self.save_dialog();
            }
        });
    }

    fn open_image_dialog(&mut self) {
        if let Some(path) = self.chooser.pick_file(
            self.file_dialog("load")
                .add_filter("Image", &image_io::accepted_extensions()),
        ) {
            self.remember_dialog("load", &path);
            self.request_photo_action(crate::photograph::PhotoAction::Open(path));
        }
    }

    /// One stable line describes the displayed layer and retained result.
    pub(super) fn viewer_status(&self, ui: &mut egui::Ui) {
        let has_output = self.output_image.is_some() || self.output_retained.is_some();
        let original = self.viewer.settings.layer == display::ViewLayer::Input;
        let identity = if self.image.is_none() {
            crate::i18n::text(ui.ctx(), "result.identity.empty").into_owned()
        } else if original || self.viewer.settings.layer == display::ViewLayer::PaperBack {
            let layer = crate::i18n::text(
                ui.ctx(),
                if original {
                    "surfaces.viewer.input"
                } else {
                    "surfaces.viewer.paper_back"
                },
            );
            format!("{} · {}", layer, self.retained_result_identity(ui.ctx()))
        } else {
            self.retained_result_identity(ui.ctx())
        };
        let preview = !original
            && self.viewer.settings.layer == display::ViewLayer::Output
            && has_output
            && self.retained_kind == Some(RenderKind::Preview);
        ui.allocate_ui_with_layout(
            ui.available_size(),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                let help = crate::i18n::text(ui.ctx(), "surfaces.viewer.gestures_help");
                let help_button = controls::icon_button(ui, help.as_ref(), controls::Icon::Help);
                let help_id = ui.make_persistent_id("viewer-gesture-help");
                if help_button.clicked() {
                    ui.memory_mut(|memory| memory.toggle_popup(help_id));
                }
                egui::popup::popup_below_widget(
                    ui,
                    help_id,
                    &help_button,
                    egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                    |ui| {
                        ui.set_max_width(320.0);
                        let layer = crate::i18n::text(
                            ui.ctx(),
                            if original {
                                "surfaces.viewer.input"
                            } else if self.viewer.settings.layer == display::ViewLayer::PaperBack {
                                "surfaces.viewer.paper_back"
                            } else {
                                "surfaces.viewer.output"
                            },
                        );
                        ui.label(crate::tr!(
                            ui.ctx(),
                            "surfaces.viewer.gestures",
                            layer = layer,
                            zoom = format!(
                                "{:.0}",
                                self.viewer
                                    .zoom_percent()
                                    .unwrap_or(self.viewer.zoom * 100.0)
                            )
                        ));
                    },
                );
                if self.viewer_render_pending() {
                    let label = ui.label(
                        egui::RichText::new(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.viewer.rendering",
                        ))
                        .small(),
                    );
                    controls::locate_row(ui, "viewer.rendering", &label);
                }
                let width = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(width, theme::HEIGHT_CONTROL),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let color = if has_output && self.accepted_frame_is_stale() {
                            theme::WARNING
                        } else {
                            theme::FG_SECONDARY
                        };
                        let (dot, _) =
                            ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot.center(), 2.5, color);
                        let caveat = preview
                            .then(|| crate::i18n::text(ui.ctx(), "result.identity.preview_caveat"));
                        // Reserve the whole warning before the identity gets
                        // the rest; truncating the warning makes a required
                        // Preview/Scan distinction hover-only.
                        let caveat_width = caveat.as_ref().map_or(0.0, |caveat| {
                            ui.fonts(|fonts| {
                                fonts
                                    .layout_no_wrap(
                                        caveat.to_string(),
                                        egui::FontId::proportional(theme::SIZE_HINT),
                                        theme::FG_MUTED,
                                    )
                                    .size()
                                    .x
                            }) + theme::SPACE_4
                        });
                        let separator_width = if preview {
                            ui.spacing().item_spacing.x * 2.0 + 1.0
                        } else {
                            0.0
                        };
                        let identity_width =
                            (ui.available_width() - caveat_width - separator_width).max(0.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(identity_width, theme::HEIGHT_CONTROL),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(identity_width);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&identity).small().color(color),
                                    )
                                    .truncate(),
                                )
                            },
                        )
                        .inner
                        .on_hover_text(&identity);
                        if let Some(caveat) = caveat {
                            ui.separator();
                            ui.allocate_ui_with_layout(
                                egui::vec2(caveat_width, theme::HEIGHT_CONTROL),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(caveat.as_ref())
                                                .small()
                                                .color(theme::FG_MUTED),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(caveat.to_string());
                                },
                            );
                        }
                    },
                );
            },
        );
    }

    /// One toolbar row owns global actions and two-line photograph context.
    pub(super) fn window_header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("window-header")
            .exact_height(theme::HEIGHT_HEADER)
            .frame(
                egui::Frame::NONE
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(theme::SPACE_4 as i8, 14)),
            )
            .show(ctx, |ui| {
                ui.allocate_ui_with_layout(
                    ui.available_size(),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = theme::SPACE_3;
                        let (mark, _) =
                            ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                        ui.painter().circle_stroke(
                            mark.center(),
                            11.5,
                            theme::stroke(theme::ACCENT_DIM),
                        );
                        controls::paint_icon(
                            ui.painter(),
                            mark,
                            controls::Icon::Aperture,
                            theme::ACCENT,
                        );
                        if ctx.screen_rect().width() > theme::WIDTH_COMPACT_WORKSPACE {
                            ui.label(
                                egui::RichText::new("SPEKTRAFILM")
                                    .strong()
                                    .size(theme::SIZE_LABEL),
                            );
                        }
                        let open_name = crate::i18n::text(ctx, "surfaces.action.open");
                        if controls::action_button(ui, open_name.as_ref(), controls::Icon::Open)
                            .on_hover_text(crate::i18n::text(ctx, "surfaces.action.open_help"))
                            .clicked()
                        {
                            self.open_image_dialog();
                        }
                        self.photograph_toolbar(ui);
                        ui.separator();
                        let settings_name = crate::i18n::text(ctx, "surfaces.settings.title");
                        let export_name = crate::i18n::text(
                            ctx,
                            if self.export_job.is_some() {
                                "surfaces.export.cancel"
                            } else {
                                "surfaces.action.export"
                            },
                        );
                        let text_action_width = |label: &str| {
                            let text = egui::WidgetText::from(label).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Extend),
                                f32::INFINITY,
                                egui::TextStyle::Button,
                            );
                            text.size().x
                                + 17.0
                                + theme::SPACE_2
                                + 2.0 * ui.spacing().button_padding.x
                        };
                        let right_width = text_action_width(&settings_name)
                            + text_action_width(&export_name)
                            + 2.0 * theme::HEIGHT_CONTROL
                            + 3.0 * theme::SPACE_3;
                        let context_width =
                            (ui.available_width() - right_width - theme::SPACE_3).max(1.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(context_width, 36.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_width(context_width);
                                ui.spacing_mut().item_spacing.y = 0.0;
                                let source = self
                                    .image_path
                                    .as_deref()
                                    .and_then(Path::file_name)
                                    .map(|name| name.to_string_lossy())
                                    .unwrap_or_else(|| {
                                        crate::i18n::text(ctx, "surfaces.header.no_source")
                                    });
                                let source_label =
                                    crate::tr!(ctx, "surfaces.header.source", source = source);
                                let visible_source = if self.photograph_has_unsaved_changes() {
                                    format!(
                                        "{source} · {}",
                                        crate::i18n::text(ctx, "photo.record.unsaved")
                                    )
                                } else {
                                    source.into_owned()
                                };
                                let response = ui.add(
                                    egui::Label::new(egui::RichText::new(visible_source).strong())
                                        .truncate(),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Label,
                                        true,
                                        source_label.as_ref(),
                                    )
                                });
                                ctx.accesskit_node_builder(response.id, |node| {
                                    node.set_label(source_label.as_ref())
                                });
                                response.on_hover_text(self.image_path.as_ref().map_or_else(
                                    || source_label.into_owned(),
                                    |path| path.display().to_string(),
                                ));
                                let mut print =
                                    profiles::selected_display(&self.papers, &self.print_name);
                                if !routes::vocabulary::route_includes_print(
                                    &self.params.workflow.route,
                                ) {
                                    print.push_str(&format!(
                                        " ({})",
                                        crate::i18n::text(
                                            ctx,
                                            "surfaces.status.stored_print_inactive"
                                        )
                                    ));
                                }
                                let context = format!(
                                    "{} · {print} · {}",
                                    profiles::selected_display(&self.films, &self.film_name),
                                    controls::localized(
                                        ctx,
                                        routes::vocabulary::route_description(
                                            &self.params.workflow.route
                                        )
                                    ),
                                );
                                let status = self.status.localized(ctx);
                                let routine = status.is_empty()
                                    || matches!(
                                        &self.status,
                                        crate::app::Message::Text {
                                            key: "operations.rendered"
                                                | "operations.comparing_baseline"
                                                | "surfaces.status.loaded"
                                                | "surfaces.status.loaded_raw"
                                                | "surfaces.status.preparing_input",
                                            ..
                                        }
                                    );
                                // Material and workflow identity is persistent context;
                                // an operation outcome gets its own bounded segment
                                // instead of replacing that identity.
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = theme::SPACE_2;
                                    let available = ui.available_width();
                                    let status_width = if routine {
                                        0.0
                                    } else {
                                        (available * 0.38).min(190.0)
                                    };
                                    let context_width = (available
                                        - if routine {
                                            0.0
                                        } else {
                                            status_width + ui.spacing().item_spacing.x
                                        })
                                    .max(1.0);
                                    let context_response = ui.add_sized(
                                        [context_width, ui.available_height().max(14.0)],
                                        egui::Label::new(
                                            egui::RichText::new(&context)
                                                .small()
                                                .color(theme::FG_MUTED),
                                        )
                                        .truncate(),
                                    );
                                    context_response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Label,
                                            true,
                                            context.as_str(),
                                        )
                                    });
                                    ctx.accesskit_node_builder(context_response.id, |node| {
                                        node.set_label(context.as_str())
                                    });
                                    context_response.on_hover_text(context.as_str());
                                    if !routine {
                                        ui.separator();
                                        let status_response = ui.add_sized(
                                            [status_width, ui.available_height().max(14.0)],
                                            egui::Label::new(
                                                egui::RichText::new(status.as_str())
                                                    .small()
                                                    .color(theme::FG_MUTED),
                                            )
                                            .halign(egui::Align::Max)
                                            .truncate(),
                                        );
                                        status_response.widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::WidgetType::Label,
                                                true,
                                                status.as_str(),
                                            )
                                        });
                                        ctx.accesskit_node_builder(status_response.id, |node| {
                                            node.set_label(status.as_str())
                                        });
                                        status_response.on_hover_text(status.as_str());
                                    }
                                });
                            },
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.export_job.is_some() {
                                if ui
                                    .add_enabled_ui(true, |ui| {
                                        controls::action_button(
                                            ui,
                                            export_name.as_ref(),
                                            controls::Icon::Close,
                                        )
                                    })
                                    .inner
                                    .on_hover_text(crate::i18n::text(
                                        ctx,
                                        "surfaces.export.cancel_help",
                                    ))
                                    .clicked()
                                {
                                    self.cancel_export();
                                }
                            } else {
                                let enabled = self.image.is_some()
                                    && self.calibration_job.is_none()
                                    && !self.export_dialog.is_open();
                                let export = ui
                                    .add_enabled_ui(enabled, |ui| {
                                        let visuals = ui.visuals_mut();
                                        for visual in [
                                            &mut visuals.widgets.inactive,
                                            &mut visuals.widgets.hovered,
                                            &mut visuals.widgets.active,
                                        ] {
                                            visual.weak_bg_fill = theme::ACCENT;
                                            visual.bg_fill = theme::ACCENT;
                                            visual.fg_stroke =
                                                theme::stroke(theme::CONTRAST_ON_ACCENT);
                                        }
                                        controls::action_button(
                                            ui,
                                            export_name.as_ref(),
                                            controls::Icon::Export,
                                        )
                                    })
                                    .inner;
                                if export.clicked() {
                                    self.export_return_focus = Some(export.id);
                                    self.export_dialog.open(&self.export_options);
                                }
                            }
                            let settings = controls::action_button(
                                ui,
                                settings_name.as_ref(),
                                controls::Icon::Settings,
                            );
                            if settings.is_pointer_button_down_on() {
                                crate::numeric::suspend_numeric_draft(ctx);
                            }
                            if settings.clicked() {
                                crate::numeric::suspend_numeric_draft(ctx);
                                self.settings_return_focus = Some(settings.id);
                                self.settings_open = true;
                            }
                            let redo_help = self.redo_label_localized(ctx).map_or_else(
                                || crate::i18n::text(ctx, "surfaces.history.no_redo").into_owned(),
                                |label| {
                                    crate::tr!(ctx, "surfaces.history.redo", label = label)
                                        .into_owned()
                                },
                            );
                            let redo_name = crate::i18n::text(ctx, "surfaces.action.redo");
                            if ui
                                .add_enabled_ui(
                                    self.can_redo() && !self.input_transition_pending(),
                                    |ui| {
                                        controls::icon_button(
                                            ui,
                                            redo_name.as_ref(),
                                            controls::Icon::Redo,
                                        )
                                    },
                                )
                                .inner
                                .on_hover_text(redo_help)
                                .clicked()
                            {
                                self.redo_adjustment();
                            }
                            let undo_help = self.undo_label_localized(ctx).map_or_else(
                                || crate::i18n::text(ctx, "surfaces.history.no_undo").into_owned(),
                                |label| {
                                    crate::tr!(ctx, "surfaces.history.undo", label = label)
                                        .into_owned()
                                },
                            );
                            let undo_name = crate::i18n::text(ctx, "surfaces.action.undo");
                            if ui
                                .add_enabled_ui(
                                    self.can_undo() && !self.input_transition_pending(),
                                    |ui| {
                                        controls::icon_button(
                                            ui,
                                            undo_name.as_ref(),
                                            controls::Icon::Undo,
                                        )
                                    },
                                )
                                .inner
                                .on_hover_text(undo_help)
                                .clicked()
                            {
                                self.undo_adjustment();
                            }
                        });
                    },
                );
            });
    }
}
