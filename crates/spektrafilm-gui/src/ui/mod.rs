//! Frame composition root: panel order, modal lifecycle and viewer projection.
use crate::*;
use std::sync::atomic::Ordering;

mod inspector;
mod settings;
mod workbench;

impl App {
    fn refresh_viewing_artifacts(&mut self) {
        self.viewer.set_input_display_source(
            &self.params.io.input_color_space,
            self.params.io.input_cctf_decoding,
        );
        let (space, encoded) = self.displayed_comparison.as_ref().map_or(
            (self.output_color_space.as_str(), self.output_cctf_encoding),
            |comparison| (comparison.color_space.as_str(), comparison.encoded),
        );
        self.viewer.set_output_display_source(
            space,
            encoded,
            self.viewer.settings.use_display_transform,
            self.gui_state.display_profile().map(Path::new),
        );
        self.viewer.invalidate_display();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.photograph.context = Some(ctx.clone());
        self.bind_language(ctx);
        self.protect_photograph_close(ctx);
        if controls::release_operator_compare(ctx) {
            self.set_operator_compare(false);
        }
        if controls::release_finishing_compare(ctx) {
            self.set_finishing_compare(false);
        }
        if self.finishing_compare {
            // A route or stock change may make an already-held comparison inapplicable.
            self.set_finishing_compare(true);
        }
        // First-paint hook: tag the wgpu Metal layer's colorspace as sRGB.
        // Has to happen here (not in `App::new`) because the CAMetalLayer
        // isn't wired up at construction time.
        #[cfg(target_os = "macos")]
        if !self.metal_colorspace_tagged
            && raw_window_handle::HasWindowHandle::window_handle(frame).is_ok()
        {
            match crate::platform::tag_metal_layer_srgb(frame) {
                Ok(()) => {
                    eprintln!("[spektrafilm] CAMetalLayer.colorspace = sRGB — tagged OK");
                    self.metal_colorspace_tagged = true;
                }
                Err(e) => {
                    eprintln!("[spektrafilm] colorspace tag attempt: {e}");
                }
            }
        }
        let _ = frame;
        self.poll_input_job(ctx);

        self.begin_adjustment_frame(ctx);
        self.poll_render_job(ctx);
        self.poll_calibration_job(ctx);
        self.poll_export_job(ctx);
        self.poll_pending_saves(ctx);
        let editing_text = ctx
            .memory(|memory| memory.focused())
            .is_some_and(|id| egui::text_edit::TextEditState::load(ctx, id).is_some());
        if !editing_text
            && !self.settings_open
            && !self.export_dialog.is_open()
            && !self.stale_save_confirmation
            && !self.input_transition_pending()
        {
            if ctx.input_mut(|input| {
                input.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                )
            }) {
                self.redo_adjustment();
            } else if ctx
                .input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z))
            {
                self.undo_adjustment();
            }
        }
        // Share the renderer's exact crop and rescale rounding without
        // allocating an image just to describe its delivered dimensions.
        let export_dimensions = self
            .oriented_input
            .as_ref()
            .and_then(|image| {
                crate::export::output_dimensions([image.width(), image.height()], &self.params.io)
                    .ok()
            })
            .unwrap_or([0, 0]);
        if let Some(options) =
            self.export_dialog
                .show(ctx, export_dimensions, self.image_path.as_deref())
        {
            self.start_export(ctx, options);
        }
        self.window_header(ctx);
        self.show_stale_save_confirmation(ctx);
        self.photograph_dialogs(ctx);
        self.work_list_controls(ctx);
        self.preset_panel(ctx);
        self.sync_display_controls();
        let baseline = self
            .current_material_baseline()
            .unwrap_or_else(|| Arc::new(section_reset::unavailable_material_baseline().clone()));
        controls::begin_control_frame(ctx, &self.params, &self.gui_state.sections, baseline);
        // Fixed search and stage jumps above one scrolling inspector.
        egui::SidePanel::right("controls")
            .resizable(true)
            .default_width(theme::WIDTH_CONTROL_PANEL)
            .min_width(300.0)
            .max_width((ctx.screen_rect().width() * 0.48).max(300.0))
            .frame(
                egui::Frame::NONE
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(
                        theme::SPACE_MODULE as i8,
                        theme::SPACE_2 as i8,
                    )),
            )
            .show(ctx, |ui| {
                if self.input_transition_pending() {
                    ui.disable();
                }
                // Panel sizing belongs to the user's resize gesture, never to a
                // long query, locator, or applicability explanation inside it.
                let inspector_rect = ui.available_rect_before_wrap();
                ui.allocate_space(inspector_rect.size());
                let mut inspector = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("inspector-content")
                        .max_rect(inspector_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                inspector.set_clip_rect(inspector_rect);
                inspector.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                let ui = &mut inspector;
                ui.add_space(theme::SPACE_BLOCK);
                ui.horizontal(|ui| {
                    let clear_width = if self.filter.is_empty() {
                        0.0
                    } else {
                        theme::HEIGHT_CONTROL + theme::SPACE_2
                    };
                    let modified_label = crate::i18n::text(ui.ctx(), "surfaces.settings.modified");
                    let label_width = egui::WidgetText::from(modified_label.as_ref())
                        .into_galley(
                            ui,
                            Some(egui::TextWrapMode::Extend),
                            f32::INFINITY,
                            egui::TextStyle::Body,
                        )
                        .size()
                        .x;
                    let toggle_width =
                        29.0 + ui.spacing().item_spacing.x + label_width + theme::SPACE_1;
                    let search_width =
                        (ui.available_width() - clear_width - toggle_width - theme::SPACE_2)
                            .max(1.0);
                    let response = ui.add_sized(
                        [search_width, ui.spacing().interact_size.y],
                        egui::TextEdit::singleline(&mut self.filter)
                            .margin(egui::vec2(theme::SPACE_2, 5.0))
                            .id(egui::Id::new("control-search"))
                            .hint_text(crate::i18n::text(ui.ctx(), "surfaces.search.label"))
                            .clip_text(true)
                            .desired_width(search_width),
                    );
                    let accessible_name = crate::i18n::text(ui.ctx(), "surfaces.search.label");
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_label(accessible_name)
                    });
                    if !self.filter.is_empty() {
                        let clear = crate::i18n::text(ui.ctx(), "surfaces.search.clear");
                        if controls::icon_button(ui, clear.as_ref(), controls::Icon::Close)
                            .on_hover_text(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.search.clear_help",
                            ))
                            .clicked()
                        {
                            self.filter.clear();
                        }
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(toggle_width, theme::HEIGHT_CONTROL),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(toggle_width);
                            let label = crate::i18n::text(ui.ctx(), "surfaces.settings.modified");
                            let help = crate::i18n::text(ui.ctx(), "surfaces.search.modified_help");
                            controls::toggle_row(
                                ui,
                                label.as_ref(),
                                &mut self.only_modified,
                                help.as_ref(),
                            );
                        },
                    );
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SPACE_1;
                    let stage_width = (ui.available_width() - 4.0 * theme::SPACE_1) / 5.0;
                    for stage in controls::Stage::ALL {
                        let label = if stage == controls::Stage::Input {
                            crate::i18n::text(ui.ctx(), "surfaces.search.input_jump")
                        } else {
                            controls::localized(ui.ctx(), stage.label())
                        };
                        let response = ui
                            .add_sized(
                                [stage_width, theme::HEIGHT_CONTROL],
                                egui::Button::new(egui::RichText::new(label).small()).frame(false),
                            )
                            .on_hover_text(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.search.stage_help",
                            ));
                        // A pointer activation navigates to the heading; the
                        // boxed keyboard-focus ring must not linger and read as
                        // a selected tab.
                        if response.clicked() && ui.input(|input| input.pointer.any_click()) {
                            response.surrender_focus();
                        }
                        controls::focus_feedback(ui, &response);
                        let accessible_name = controls::localized(ui.ctx(), stage.label());
                        ui.ctx().accesskit_node_builder(response.id, |node| {
                            node.set_label(accessible_name.as_ref())
                        });
                        if response.clicked() {
                            self.pending_stage = Some(stage);
                        }
                    }
                });
                ui.separator();
                self.search_results(ui);
                controls::install_view_filter(ui.ctx(), self.only_modified);
                ui.add_space(theme::SPACE_ROW);
                // Reserve the action bar's height below the scrollable
                // controls, so the bar stays inside the panel.
                let scroll_height =
                    (ui.available_height() - theme::HEIGHT_ACTION_BAR_RESERVE).max(1.0);
                ui.allocate_ui(egui::vec2(ui.available_width(), scroll_height), |ui| {
                    let scroll = egui::ScrollArea::vertical()
                        .id_salt((
                            "controls-scroll",
                            !self.filter.is_empty() || self.only_modified,
                        ))
                        .max_width(ui.available_width())
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            self.controls_panel(ui);
                            ui.unique_id()
                        });
                    let content_label = crate::i18n::text(ctx, "surfaces.search.content");
                    ctx.accesskit_node_builder(scroll.inner, |node| {
                        node.set_role(egui::accesskit::Role::ScrollView);
                        node.set_label(content_label.as_ref());
                        node.set_bounds(egui::accesskit::Rect {
                            x0: f64::from(scroll.inner_rect.left()),
                            y0: f64::from(scroll.inner_rect.top()),
                            x1: f64::from(scroll.inner_rect.right()),
                            y1: f64::from(scroll.inner_rect.bottom()),
                        });
                    });
                    controls::install_view_filter(ui.ctx(), false);
                });
                ui.separator();
                self.simulation_action_bar(ui);
            });
        self.persist_browser_state(false);
        self.persist_analysis_preferences(false);
        self.persist_clipping_preferences(false);
        if self.browser_state_changed_at.is_some()
            || self.analysis_preferences_changed_at.is_some()
            || self.clipping_preferences_changed_at.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
        if self.diagnostics.pending_publications() > 0 {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if let Some(error) = self.diagnostics.take_publication_error() {
            self.status =
                crate::app::Message::new("surfaces.status.publication_error").arg("error", error);
        }
        // Central viewer
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                // Reserve both fixed strips before allocating any of the shelf.
                let status_rect = egui::TopBottomPanel::top("viewer-status")
                    .exact_height(theme::HEIGHT_VIEWER_STATUS)
                    .frame(
                        egui::Frame::NONE
                            .fill(theme::BG)
                            .inner_margin(egui::Margin::symmetric(theme::SPACE_3 as i8, 3)),
                    )
                    .show_inside(ui, |ui| ui.available_rect_before_wrap())
                    .inner;
                let available = (ui.available_height() - theme::HEIGHT_VIEWER_FOOTER).max(0.0);
                let capacity = (available - theme::HEIGHT_PHOTOGRAPH_MIN)
                    .clamp(0.0, theme::HEIGHT_ANALYSIS_MAX);
                let space_available = capacity >= theme::HEIGHT_ANALYSIS_MIN;
                self.viewer.clipping_suspended = self.operator_compare
                    || self.finishing_compare
                    || self.displayed_comparison.is_some();
                let clipping_source_available = match self.viewer.settings.layer {
                    crate::display::ViewLayer::Input => self.oriented_input.is_some(),
                    crate::display::ViewLayer::Output => {
                        self.output_image.is_some() || self.output_retained.is_some()
                    }
                    crate::display::ViewLayer::PaperBack => false,
                };
                let clipping_modal_open = self.settings_open
                    || self.export_dialog.is_open()
                    || self.factory_reset_confirm
                    || self.stale_save_confirmation
                    || self.photograph_confirmation_open();
                let footer_id = egui::TopBottomPanel::bottom("viewer-footer")
                    .exact_height(theme::HEIGHT_VIEWER_FOOTER)
                    .frame(
                        egui::Frame::NONE
                            .fill(theme::SURFACE)
                            .inner_margin(egui::Margin::symmetric(theme::SPACE_3 as i8, 9)),
                    )
                    .show_inside(ui, |ui| {
                        self.viewer.footer_controls(
                            ui,
                            &mut self.analysis.visible,
                            space_available,
                            clipping_source_available,
                            clipping_modal_open,
                        )
                    })
                    .inner;

                // Identify only controls removed with the shelf, not inspector focus.
                let shelf_rect_id = egui::Id::new("analysis-shelf-previous-rect");
                let viewer_controls_id = egui::Id::new("analysis-viewer-controls-previous-rect");
                let focused_removed_control = |ctx: &egui::Context| {
                    let focused = ctx.memory(|memory| memory.focused());
                    let rect = focused.and_then(|id| ctx.read_response(id)).map(|r| r.rect);
                    let (shelf, viewer_controls) = ctx.data(|data| {
                        (
                            data.get_temp::<egui::Rect>(shelf_rect_id),
                            data.get_temp::<egui::Rect>(viewer_controls_id),
                        )
                    });
                    rect.is_some_and(|rect| {
                        shelf.is_some_and(|area| area.contains_rect(rect))
                            || viewer_controls.is_some_and(|area| area.contains_rect(rect))
                    })
                };
                let expanded = self.analysis.visible && space_available;
                if self.analysis.expanded && !expanded {
                    if focused_removed_control(ctx) {
                        ctx.memory_mut(|memory| memory.request_focus(footer_id));
                    }
                    self.suspend_image_analysis();
                }
                self.analysis.expanded = expanded;
                if expanded {
                    let height = self
                        .analysis
                        .preferences
                        .shelf_height
                        .clamp(theme::HEIGHT_ANALYSIS_MIN, theme::HEIGHT_ANALYSIS_MAX)
                        .min(capacity);
                    let shelf = egui::TopBottomPanel::bottom("image-analysis-workspace")
                        .exact_height(height)
                        .show_separator_line(false)
                        .frame(egui::Frame::NONE.fill(theme::SURFACE))
                        .show_inside(ui, |ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 6.0),
                                egui::Sense::hover(),
                            );
                            let divider = ui
                                .interact(
                                    rect,
                                    egui::Id::new("analysis-shelf-divider"),
                                    egui::Sense::click_and_drag(),
                                )
                                .on_hover_cursor(egui::CursorIcon::ResizeVertical);
                            let label = crate::i18n::text(ui.ctx(), "surfaces.analysis.resize");
                            divider.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    label.as_ref(),
                                )
                            });
                            ui.painter().hline(
                                rect.x_range(),
                                rect.center().y,
                                theme::stroke(theme::FG_MUTED),
                            );
                            let grip = if divider.hovered() || divider.has_focus() {
                                theme::FG
                            } else {
                                theme::FG_MUTED.gamma_multiply(0.6)
                            };
                            for offset in [-4.0, 0.0, 4.0] {
                                ui.painter().circle_filled(
                                    rect.center() + egui::vec2(offset, 0.0),
                                    1.0,
                                    grip,
                                );
                            }
                            crate::controls::focus_feedback(ui, &divider);
                            if divider.clicked() || divider.drag_started() {
                                divider.request_focus();
                            }
                            let drag_id = divider.id.with("height-at-press");
                            if divider.drag_started()
                                && let Some(origin) = ui.input(|input| input.pointer.press_origin())
                            {
                                ctx.data_mut(|data| {
                                    data.insert_temp(
                                        drag_id,
                                        (self.analysis.preferences.shelf_height, height, origin.y),
                                    )
                                });
                            }
                            if (divider.dragged() || divider.drag_stopped())
                                && let Some(position) = divider.interact_pointer_pos()
                                && let Some((requested, displayed, origin)) =
                                    ctx.data(|data| data.get_temp::<(f32, f32, f32)>(drag_id))
                            {
                                let delta = origin - position.y;
                                let resized = displayed + delta;
                                self.analysis.preferences.shelf_height = if delta >= 0.0 {
                                    requested.max(resized)
                                } else {
                                    resized
                                }
                                .clamp(theme::HEIGHT_ANALYSIS_MIN, theme::HEIGHT_ANALYSIS_MAX);
                                ctx.request_repaint();
                            }
                            if divider.drag_stopped() {
                                ctx.data_mut(|data| data.remove::<(f32, f32, f32)>(drag_id));
                            }
                            if divider.has_focus() {
                                ui.memory_mut(|memory| {
                                    memory.set_focus_lock_filter(
                                        divider.id,
                                        egui::EventFilter {
                                            vertical_arrows: true,
                                            ..Default::default()
                                        },
                                    )
                                });
                                let (up, down, reset) = ui.input(|input| {
                                    (
                                        input.key_pressed(egui::Key::ArrowUp),
                                        input.key_pressed(egui::Key::ArrowDown),
                                        input.key_pressed(egui::Key::Home),
                                    )
                                });
                                let delta = (up as i32 - down as i32) as f32 * 8.0;
                                if delta != 0.0 {
                                    let resized = height + delta;
                                    self.analysis.preferences.shelf_height = if delta > 0.0 {
                                        self.analysis.preferences.shelf_height.max(resized)
                                    } else {
                                        resized
                                    }
                                    .clamp(theme::HEIGHT_ANALYSIS_MIN, theme::HEIGHT_ANALYSIS_MAX);
                                    ctx.request_repaint();
                                }
                                if reset {
                                    self.analysis.preferences.shelf_height =
                                        theme::HEIGHT_ANALYSIS_DEFAULT;
                                    ctx.request_repaint();
                                }
                            }
                            if divider.double_clicked() {
                                self.analysis.preferences.shelf_height =
                                    theme::HEIGHT_ANALYSIS_DEFAULT;
                                ctx.request_repaint();
                            }
                            self.show_image_analysis(ui);
                        });
                    ctx.data_mut(|data| data.insert_temp(shelf_rect_id, shelf.response.rect));
                    if !self.analysis.visible {
                        if focused_removed_control(ctx) {
                            ctx.memory_mut(|memory| memory.request_focus(footer_id));
                        }
                        self.analysis.expanded = false;
                        self.suspend_image_analysis();
                        ctx.request_repaint();
                    }
                }
                let viewer_rect = ui.available_rect_before_wrap();
                let viewer_controls_rect = egui::Rect::from_min_size(
                    viewer_rect.min + egui::Vec2::splat(theme::SPACE_2),
                    egui::vec2(viewer_rect.width().max(1.0), theme::HEIGHT_CONTROL * 2.0),
                );
                ctx.data_mut(|data| data.insert_temp(viewer_controls_id, viewer_controls_rect));
                // Populate status after the viewer accepts composition and transition work.

                self.viewer
                    .set_display_max_size(self.params.settings.preview_max_size);
                let analysis_input = self.analysis.viewer_input(self.viewer.settings.layer);
                let displayed_output = self.displayed_comparison.as_ref().map_or_else(
                    || self.output_image.clone(),
                    |comparison| comparison.image.host_image().cloned(),
                );
                self.viewer.show(
                    ui,
                    self.oriented_input.as_ref(),
                    displayed_output.as_ref(),
                    analysis_input,
                );
                let mut status_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("viewer-status-content")
                        .max_rect(status_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                status_ui.set_clip_rect(status_rect);
                self.viewer_status(&mut status_ui);
            });
        // Accept drag-and-dropped image files.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            self.request_photo_action(crate::photograph::PhotoAction::Open(path));
        }
        self.end_adjustment_frame(ctx);
        self.settings_dialog(ctx);
        self.factory_reset_confirmation(ctx);
        self.sync_display_controls();
        controls::end_control_frame();
        for (open, return_focus) in [
            (self.settings_open, &mut self.settings_return_focus),
            (self.export_dialog.is_open(), &mut self.export_return_focus),
        ] {
            if !open && return_focus.is_some() {
                if ctx.memory(|memory| memory.top_modal_layer().is_none()) {
                    ctx.memory_mut(|memory| memory.request_focus(return_focus.take().unwrap()));
                } else {
                    ctx.request_repaint();
                }
            }
        }
        // Accepted edits must schedule their wakeup in this frame, including
        // the last input event before the native event loop becomes idle.
        if self.dirty
            && self.image.is_some()
            && !self.input_transition_pending()
            && self.calibration_job.is_none()
            && !self.export_dialog.is_open()
            && !self.stale_save_confirmation
        {
            let now = Instant::now();
            let dirty_since = *self.dirty_since.get_or_insert(now);
            let elapsed = now.saturating_duration_since(dirty_since);
            if elapsed >= PREVIEW_DEBOUNCE {
                self.dispatch_render(ctx);

                self.dirty = false;
                self.dirty_since = None;
            } else {
                ctx.request_repaint_after(PREVIEW_DEBOUNCE - elapsed);
            }
        }
    }

    /// Cancel pending export publication and drain its worker before exiting.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_browser_state(true);
        self.persist_analysis_preferences(true);
        self.persist_clipping_preferences(true);
        self.cancel_pending_saves();
        if let Some(mut job) = self.export_job.take() {
            job.cancel.store(true, Ordering::SeqCst);
            #[cfg(test)]
            if let Some(pause) = &job.pause {
                pause.notify_cancelled();
                pause.release();
            }
            if let Some(h) = job.handle.take() {
                let _ = h.join();
            }
            if let Some(configuration) = job
                .configuration
                .as_ref()
                .and_then(|slot| slot.lock().take())
            {
                job.operation.set_render_configuration(configuration);
            }
            let _ = job.rx.try_recv();
            job.operation
                .issue(IssueCategory::Cancelled, IssueBoundary::Operation);
            self.diagnostics.finish(job.operation, Outcome::Cancelled);
        }
        if let Some(mut operation) = self.render_worker.shutdown() {
            operation.issue(IssueCategory::Cancelled, IssueBoundary::Operation);
            self.diagnostics.finish(operation, Outcome::Cancelled);
        }
        while let Ok((_, mut operation, _)) = self.render_worker.rx.try_recv() {
            operation.issue(IssueCategory::Cancelled, IssueBoundary::Operation);
            self.diagnostics.finish(operation, Outcome::Cancelled);
        }
        if let Some(mut job) = self.calibration_job.take()
            && let Some(handle) = job.handle.take()
        {
            let _ = handle.join();
        }
        self.diagnostics.shutdown();
    }
}
