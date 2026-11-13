//! Control inspector: stage/module rendering, search and section resets.
use crate::*;

impl App {
    fn parameter_section(&mut self, ui: &mut egui::Ui, section: &str) {
        if !controls::section_matches(
            section,
            &self.filter,
            &self.params,
            &self.gui_state.sections,
            self.only_modified,
        ) {
            return;
        }
        ui.push_id(("module", section), |ui| {
            let reveal = self.pending_scroll.as_deref() == Some(section);
            let disclosure_id = ui.make_persistent_id("open");
            let default_open = matches!(
                section,
                "Workflow"
                    | "LUT look"
                    | "Camera"
                    | "Film material"
                    | "Print material"
                    | "Output"
                    | "Finishing"
            );
            let normal_open = ui.ctx().data_mut(|data| {
                *data.get_persisted_mut_or_insert_with(disclosure_id, || default_open)
            });
            let temporary_open = !self.filter.is_empty() || self.only_modified;
            let open = normal_open || temporary_open || reveal;
            let frame = egui::Frame::NONE
                .fill(if open {
                    theme::SURFACE2
                } else {
                    egui::Color32::TRANSPARENT
                })
                .stroke(if open {
                    theme::stroke(theme::BORDER)
                } else {
                    egui::Stroke::NONE
                })
                .corner_radius(theme::RADIUS_CARD)
                .inner_margin(egui::Margin::symmetric(
                    theme::SPACE_MODULE as i8,
                    if open {
                        theme::SPACE_MODULE as i8
                    } else {
                        theme::SPACE_1 as i8
                    },
                ));
            let inactive = controls::materials(ui.ctx()).and_then(|materials| {
                controls::module_applicability(
                    section,
                    &self.params,
                    &materials.film,
                    materials.print.as_deref(),
                )
            });
            frame.show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = theme::SPACE_2;
                let mut heading = None;
                ui.horizontal(|ui| {
                    let title = if section == "Glare" {
                        "Print viewing glare"
                    } else {
                        section
                    };
                    let mut response = controls::disclosure_header(ui, title, open);
                    if let Some(summary) = controls::section_summary(section) {
                        response = response.on_hover_text(controls::localized(ui.ctx(), summary));
                    }
                    if response.clicked() && !temporary_open {
                        ui.ctx()
                            .data_mut(|data| data.insert_persisted(disclosure_id, !normal_open));
                    }
                    heading = Some(response);
                    if let Some(reason) = &inactive {
                        let reason = controls::localized(ui.ctx(), reason);
                        let badge = ui.label(
                            egui::RichText::new(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.section.inactive_compact",
                            ))
                            .small()
                            .color(theme::FG_MUTED),
                        );
                        ui.ctx().accesskit_node_builder(badge.id, |node| {
                            node.set_description(reason.as_ref())
                        });
                        badge.on_hover_text(reason);
                    } else if section == "Glare" {
                        let glare = &self.params.print_render.glare;
                        let (key, color) = if !glare.active {
                            ("surfaces.section.glare_off", theme::FG_MUTED)
                        } else if glare.percent <= 0.0 {
                            ("surfaces.section.glare_no_effect", theme::WARNING)
                        } else {
                            ("surfaces.section.glare_on", theme::ACCENT)
                        };
                        ui.label(
                            egui::RichText::new(crate::i18n::text(ui.ctx(), key))
                                .small()
                                .color(color),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.section_reset_controls_with_order(ui, section, true);
                    });
                });
                if reveal {
                    self.pending_scroll = None;
                    if !temporary_open {
                        ui.ctx()
                            .data_mut(|data| data.insert_persisted(disclosure_id, true));
                    }
                    if let Some(response) = heading {
                        response.request_focus();
                        ui.scroll_to_rect(response.rect, Some(egui::Align::TOP));
                    }
                }
                if open {
                    let inactive_in_lut =
                        controls::inactive_in_lut(section, self.params.workflow.mode);
                    ui.push_id("body", |ui| {
                        ui.add_enabled_ui(!inactive_in_lut, |ui| self.module_body(ui, section));
                    });
                    if inactive_in_lut {
                        ui.weak(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.section.inactive_in_lut",
                        ));
                    }
                }
                if open && let Some(reason) = &inactive {
                    ui.label(
                        egui::RichText::new(crate::tr!(
                            ui.ctx(),
                            "surfaces.section.inactive",
                            reason = controls::localized(ui.ctx(), reason)
                        ))
                        .color(theme::FG_MUTED)
                        .small(),
                    );
                }
            });
            ui.add_space(if open {
                theme::SPACE_MODULE
            } else {
                theme::SPACE_2
            });
        });
    }

    fn module_body(&mut self, ui: &mut egui::Ui, section: &str) {
        let _row_scope = controls::row_modified::install(
            ui.ctx(),
            section,
            &self.params,
            &self.gui_state.sections,
        );
        match section {
            "Workflow" => {
                let film_mode =
                    self.params.workflow.mode == spektrafilm_core::params::WorkflowMode::Film;
                if film_mode
                    && ui
                        .button(crate::i18n::text(ui.ctx(), "surfaces.lut.export"))
                        .clicked()
                {
                    self.begin_export_lut();
                }
                // In LUT mode the live route is the inactive `input` default;
                // show the retained film route that switching back restores.
                let mut route = if film_mode {
                    self.params.workflow.route.clone()
                } else {
                    self.film_route
                        .clone()
                        .unwrap_or_else(|| self.params.workflow.route.clone())
                };
                let changed = ui
                    .add_enabled_ui(film_mode, |ui| {
                        controls::choice_tip_entries(
                            ui,
                            "Film route",
                            &mut route,
                            routes::entries(),
                            crate::i18n::text(ui.ctx(), "surfaces.route.help").as_ref(),
                        )
                    })
                    .inner;
                if changed && film_mode && route != self.params.workflow.route {
                    self.params.workflow.route = route;
                    controls::record_edited_path(ui.ctx(), "workflow.route");
                    self.label_adjustment("Change workflow");
                    self.finish_route_change();
                }
                if !film_mode {
                    ui.weak(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.workflow.route_stored",
                    ));
                }
            }
            "LUT look" => {
                if self.params.workflow.mode != spektrafilm_core::params::WorkflowMode::Lut {
                    ui.weak(crate::i18n::text(ui.ctx(), "surfaces.lut.inactive"));
                    if ui
                        .button(crate::i18n::text(ui.ctx(), "surfaces.lut.import"))
                        .clicked()
                    {
                        self.begin_import_lut(false);
                    }
                } else if self
                    .params
                    .lut
                    .as_ref()
                    .is_none_or(|lut| lut.path.as_os_str().is_empty())
                {
                    ui.weak(crate::i18n::text(ui.ctx(), "surfaces.lut.empty"));
                    if ui
                        .button(crate::i18n::text(ui.ctx(), "surfaces.lut.import"))
                        .clicked()
                    {
                        self.begin_import_lut(false);
                    }
                } else {
                    let changed = {
                        let lut = self.params.lut.get_or_insert_with(Default::default);
                        ui.weak(crate::i18n::text(ui.ctx(), "surfaces.lut.non_spectral"));
                        if !lut.title.is_empty() {
                            ui.label(&lut.title);
                        }
                        ui.monospace(lut.path.display().to_string());
                        if let Some(fraction) = self.lut_clipped_fraction {
                            ui.colored_label(
                                theme::WARNING,
                                crate::tr!(
                                    ui.ctx(),
                                    "surfaces.lut.domain_clipped",
                                    percent = format!("{:.2}", fraction * 100.0)
                                ),
                            );
                        }
                        ui.small(crate::i18n::text(ui.ctx(), "surfaces.lut.space_note"));
                        let mut changed = false;
                        changed |= ui
                            .add(
                                egui::Slider::new(&mut lut.strength, 0.0..=1.0)
                                    .text(crate::i18n::text(ui.ctx(), "surfaces.lut.strength")),
                            )
                            .changed();
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text(ui.ctx(), "surfaces.lut.input_space"));
                            let width =
                                (ui.available_width() - theme::SPACE_2).max(theme::WIDTH_COMBO_MIN);
                            let mut input_space = lut.input_space;
                            egui::ComboBox::from_id_salt("lut-input-space")
                                .selected_text(input_space.name())
                                .width(width)
                                .show_ui(ui, |ui| {
                                    for candidate in [
                                        spektrafilm_core::lut_look::LutSpace::SrgbLinear,
                                        spektrafilm_core::lut_look::LutSpace::Srgb,
                                        spektrafilm_core::lut_look::LutSpace::DisplayP3,
                                    ] {
                                        changed |= ui
                                            .selectable_value(
                                                &mut input_space,
                                                candidate,
                                                candidate.name(),
                                            )
                                            .changed();
                                    }
                                });
                            if input_space != lut.input_space {
                                lut.input_space = input_space;
                                changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(crate::i18n::text(ui.ctx(), "surfaces.lut.output_space"));
                            let width =
                                (ui.available_width() - theme::SPACE_2).max(theme::WIDTH_COMBO_MIN);
                            let mut space = lut.output_space;
                            egui::ComboBox::from_id_salt("lut-output-space")
                                .selected_text(space.name())
                                .width(width)
                                .show_ui(ui, |ui| {
                                    for candidate in [
                                        spektrafilm_core::lut_look::LutSpace::SrgbLinear,
                                        spektrafilm_core::lut_look::LutSpace::Srgb,
                                        spektrafilm_core::lut_look::LutSpace::DisplayP3,
                                    ] {
                                        changed |= ui
                                            .selectable_value(
                                                &mut space,
                                                candidate,
                                                candidate.name(),
                                            )
                                            .changed();
                                    }
                                });
                            if space != lut.output_space {
                                lut.output_space = space;
                                changed = true;
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label(crate::i18n::text(ui.ctx(), "surfaces.lut.domain_policy"));
                            changed |= ui
                                .radio_value(
                                    &mut lut.domain_policy,
                                    spektrafilm_core::lut_look::LutDomainPolicy::Clamp,
                                    crate::i18n::text(ui.ctx(), "surfaces.lut.policy_clamp"),
                                )
                                .changed();
                            changed |= ui
                                .radio_value(
                                    &mut lut.domain_policy,
                                    spektrafilm_core::lut_look::LutDomainPolicy::Fail,
                                    crate::i18n::text(ui.ctx(), "surfaces.lut.policy_fail"),
                                )
                                .changed();
                        });
                        ui.small(crate::i18n::text(ui.ctx(), "surfaces.lut.policy_help"));
                        ui.small(format!(
                            "{}: R [{:.6}, {:.6}], G [{:.6}, {:.6}], B [{:.6}, {:.6}] | {}: {}",
                            crate::i18n::text(ui.ctx(), "surfaces.lut.domain"),
                            lut.domain_min[0],
                            lut.domain_max[0],
                            lut.domain_min[1],
                            lut.domain_max[1],
                            lut.domain_min[2],
                            lut.domain_max[2],
                            crate::i18n::text(ui.ctx(), "surfaces.lut.shaper"),
                            lut.shaper.name(),
                        ));
                        grade_controls(
                            ui,
                            self.params.grade.get_or_insert_with(Default::default),
                            &mut changed,
                        );
                        changed
                    };
                    ui.horizontal(|ui| {
                        if ui
                            .button(crate::i18n::text(ui.ctx(), "surfaces.lut.import_new"))
                            .clicked()
                        {
                            self.begin_import_lut(false);
                        }
                        if ui
                            .button(crate::i18n::text(ui.ctx(), "surfaces.lut.replace"))
                            .clicked()
                        {
                            self.begin_import_lut(true);
                        }
                    });
                    if changed {
                        controls::record_edited_path(ui.ctx(), "lut");
                        self.label_adjustment("Adjust LUT look");
                        self.invalidate_look_render();
                    }
                }
            }
            "Effects" => {
                if self.params.workflow.mode == spektrafilm_core::params::WorkflowMode::Film {
                    ui.weak(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.effects.inactive_film",
                    ));
                } else {
                    let mut changed = false;
                    effects_controls(
                        ui,
                        self.params.effects.get_or_insert_with(Default::default),
                        &mut changed,
                    );
                    if changed {
                        controls::record_edited_path(ui.ctx(), "effects");
                        self.label_adjustment("Adjust Effects");
                        self.invalidate_look_render();
                    }
                }
            }
            "Film material" | "Print material" => {
                let film = section == "Film material";
                let path = if film {
                    "extras.simulation.film_stock"
                } else {
                    "extras.simulation.print_paper"
                };
                if controls::entry_matches(
                    path,
                    &self.filter,
                    &self.params,
                    &self.gui_state.sections,
                    self.only_modified,
                ) {
                    let mut selected = if film {
                        self.film_name.clone()
                    } else {
                        self.print_name.clone()
                    };
                    let (changed, response) = if film {
                        profile_combo(ui, "film", "film profile", &self.films, &mut selected)
                    } else {
                        profile_combo(ui, "paper", "print profile", &self.papers, &mut selected)
                    };
                    controls::locate_row(ui, path, &response);
                    if changed {
                        self.label_adjustment(if film {
                            "Change film material"
                        } else {
                            "Change print material"
                        });
                        if film {
                            self.select_film_profile(selected);
                        } else {
                            self.select_print_profile(selected);
                        }
                    }
                }
                self.chemistry_dev_time(ui, film);
            }
            "Source rotation" => {
                ui.horizontal_wrapped(|ui| {
                    let ccw = ui
                        .add_enabled(
                            self.image.is_some(),
                            egui::Button::new(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.source.rotate_ccw",
                            )),
                        )
                        .on_hover_text(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.source.rotate_ccw_help",
                        ));
                    controls::locate_row(ui, "action.rotate_ccw", &ccw);
                    if ccw.clicked() {
                        self.label_adjustment("Rotate source counter-clockwise");
                        self.rotate_input_image_counterclockwise();
                    }
                    let cw = ui
                        .add_enabled(
                            self.image.is_some(),
                            egui::Button::new(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.source.rotate_cw",
                            )),
                        )
                        .on_hover_text(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.source.rotate_cw_help",
                        ));
                    controls::locate_row(ui, "action.rotate_cw", &cw);
                    if cw.clicked() {
                        self.label_adjustment("Rotate source clockwise");
                        self.rotate_input_image_clockwise();
                    }
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "surfaces.source.rotation",
                        degrees = self.input_rotation * 90
                    ));
                });
            }
            "Import Raw" => {
                self.parameter_section_controls(ui, section);
                let raw = self.image_path.as_deref().is_some_and(image_io::is_raw);
                if raw {
                    if self.raw_applied_settings.as_ref()
                        != Some(&self.gui_state.sections["load_raw"])
                    {
                        ui.weak(crate::i18n::text(ui.ctx(), "surfaces.raw.pending"));
                    } else {
                        ui.weak(crate::i18n::text(ui.ctx(), "surfaces.raw.current"));
                    }
                } else {
                    ui.weak(crate::i18n::text(ui.ctx(), "surfaces.raw.inactive"));
                }
                let reprocess = ui.add_enabled(
                    raw,
                    egui::Button::new(crate::i18n::text(ui.ctx(), "surfaces.raw.reprocess")),
                );
                controls::locate_row(ui, "action.reprocess_raw", &reprocess);
                if reprocess.clicked()
                    && let Some(path) = self.image_path.clone()
                {
                    self.load_image_from_path(&path);
                }
            }
            "Scanner" => {
                Self::scanner_print_glare_status(
                    ui,
                    &self.params,
                    &mut self.pending_scroll,
                    &mut self.filter,
                    &mut self.only_modified,
                );
                self.parameter_section_controls(ui, section);
            }
            _ => self.parameter_section_controls(ui, section),
        }
    }

    pub(crate) fn scanner_print_glare_status(
        ui: &mut egui::Ui,
        params: &RuntimeParams,
        pending_scroll: &mut Option<String>,
        filter: &mut String,
        only_modified: &mut bool,
    ) {
        if controls::control_inactive_reason(ui.ctx(), "print_render.glare.active", params)
            .is_some()
        {
            return;
        }
        let glare = &params.print_render.glare;
        let text = if !glare.active {
            crate::i18n::text(ui.ctx(), "surfaces.scanner.print_glare_off").to_string()
        } else if glare.percent <= 0.0 {
            crate::tr!(
                ui.ctx(),
                "surfaces.scanner.print_glare_no_effect",
                amount = format!("{:.2}", glare.percent)
            )
            .to_string()
        } else {
            crate::tr!(
                ui.ctx(),
                "surfaces.scanner.print_glare_on",
                amount = format!("{:.2}", glare.percent)
            )
            .to_string()
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(text).small().color(theme::FG_MUTED));
            let adjust = ui
                .link(crate::i18n::text(
                    ui.ctx(),
                    "surfaces.scanner.adjust_print_glare",
                ))
                .on_hover_text(crate::i18n::text(
                    ui.ctx(),
                    "surfaces.scanner.adjust_print_glare_help",
                ));
            if adjust.clicked() {
                filter.clear();
                *only_modified = false;
                controls::request_focus(ui.ctx(), "Glare", "print_render.glare.active");
                *pending_scroll = Some("Glare".to_owned());
            }
        });
    }

    /// The `modified` chip and the reset action of one section header.
    ///
    /// Every resettable section presents these on its header, so the reset scope
    /// the section table declares is reachable from the surface rendering it.
    /// `trailing` places the action after the chip in a right-aligned header.
    pub(super) fn section_reset_controls_with_order(
        &mut self,
        ui: &mut egui::Ui,
        section: &str,
        trailing: bool,
    ) -> bool {
        let material_modified = match section {
            "Film material" => controls::entry_matches(
                "extras.simulation.film_stock",
                "",
                &self.params,
                &self.gui_state.sections,
                true,
            ),
            "Print material" => controls::entry_matches(
                "extras.simulation.print_paper",
                "",
                &self.params,
                &self.gui_state.sections,
                true,
            ),
            _ => false,
        };
        let baseline = self
            .current_material_baseline()
            .unwrap_or_else(|| Arc::new(section_reset::unavailable_material_baseline().clone()));
        let modified = material_modified
            || if section == "Workflow" && !self.params.workflow.mode.is_film() {
                let route = self
                    .film_route
                    .as_deref()
                    .unwrap_or(self.params.workflow.route.as_str());
                section_reset::route_modified(route, &baseline)
            } else {
                section_reset::is_modified(
                    section,
                    &self.params,
                    &self.gui_state.sections,
                    &baseline,
                )
            };
        let resettable = section_reset::sections().any(|name| name == section);
        let reset_allowed = !controls::inactive_in_lut(section, self.params.workflow.mode)
            && (section != "Workflow"
                || self.params.workflow.mode == spektrafilm_core::params::WorkflowMode::Film);
        let mut changed = false;
        if trailing && resettable {
            changed |= ui
                .add_enabled_ui(reset_allowed, |ui| self.section_reset_action(ui, section))
                .inner;
        }
        if modified {
            ui.label(
                egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.state.modified"))
                    .color(theme::WARNING)
                    .size(theme::SIZE_HINT),
            );
        }
        if !trailing && resettable {
            changed |= ui
                .add_enabled_ui(reset_allowed, |ui| self.section_reset_action(ui, section))
                .inner;
        }
        changed
    }

    fn section_reset_action(&mut self, ui: &mut egui::Ui, section: &str) -> bool {
        let response = controls::module_reset(ui, section);
        if !response.clicked() {
            return false;
        }
        self.reset_parameter_section(section)
    }

    fn retain_locator_focus(ctx: &egui::Context, current: Vec<egui::Id>) {
        let key = egui::Id::new("control-locator-response-ids");
        let previous = ctx
            .data_mut(|data| data.get_temp::<Vec<egui::Id>>(key))
            .unwrap_or_default();
        if ctx
            .memory(|memory| memory.focused())
            .is_some_and(|id| previous.contains(&id) && !current.contains(&id))
        {
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("control-search")));
        }
        ctx.data_mut(|data| data.insert_temp(key, current));
    }

    pub(super) fn search_results(&mut self, ui: &mut egui::Ui) {
        if self.filter.is_empty() && !self.only_modified {
            Self::retain_locator_focus(ui.ctx(), Vec::new());
            return;
        }
        let results = self.search_index.results_filtered(
            &self.filter,
            &self.params,
            &self.gui_state.sections,
            self.film_medium,
            self.parameter_revision,
            &self.data_dir,
            self.only_modified,
        );
        if results.is_empty() {
            Self::retain_locator_focus(ui.ctx(), Vec::new());
            let message = if self.filter.is_empty() {
                crate::i18n::text(ui.ctx(), "surfaces.search.none_modified").into_owned()
            } else if self.only_modified {
                crate::tr!(
                    ui.ctx(),
                    "surfaces.search.no_modified_match",
                    query = &self.filter
                )
                .into_owned()
            } else {
                crate::tr!(ui.ctx(), "surfaces.search.no_match", query = &self.filter).into_owned()
            };
            ui.label(egui::RichText::new(message).color(theme::FG_MUTED).small());
            return;
        }
        let mut destination = None;
        let mut locator_ids = Vec::with_capacity(results.len());
        egui::ScrollArea::vertical()
            .id_salt("control-locators")
            .max_height(120.0)
            .show(ui, |ui| {
                for result in results {
                    let label = result.localized_locator_label(ui.ctx());
                    let mut details = result.localized_details(ui.ctx());
                    if let Some(reason) = controls::control_inactive_reason(
                        ui.ctx(),
                        &result.target_path,
                        &self.params,
                    ) {
                        details.push('\n');
                        details.push_str(controls::localized(ui.ctx(), &reason).as_ref());
                    }
                    let response = ui.button(&label).on_hover_text(details);
                    locator_ids.push(response.id);
                    if response.clicked() {
                        destination = Some((
                            result.owner,
                            result.section,
                            result.target_path.clone(),
                            response.id,
                        ));
                    }
                }
            });
        Self::retain_locator_focus(ui.ctx(), locator_ids);
        if let Some((owner, section, path, _invoking_id)) = destination {
            controls::request_focus(ui.ctx(), section, &path);
            match owner {
                controls::ControlOwner::Stage(stage) => {
                    self.pending_stage = Some(stage);
                    self.pending_scroll = Some(section.to_owned());
                }
                controls::ControlOwner::Settings => {
                    crate::numeric::suspend_numeric_draft(ui.ctx());
                    self.settings_open = true;
                    self.settings_return_focus = Some(egui::Id::new("control-search"));
                    self.pending_scroll = Some(section.to_owned());
                }
                controls::ControlOwner::Export => {
                    self.export_return_focus = Some(egui::Id::new("control-search"));
                    if !self.export_dialog.is_open() {
                        self.export_dialog.open(&self.export_options);
                    }
                }
                controls::ControlOwner::Viewer | controls::ControlOwner::Global => {
                    self.pending_scroll = None;
                }
                controls::ControlOwner::Header | controls::ControlOwner::Result => {
                    self.pending_scroll = None;
                }
                controls::ControlOwner::WorkList => {
                    self.photograph.work_list.open = true;
                    self.pending_scroll = None;
                }
                controls::ControlOwner::Presets => {
                    if ui.ctx().screen_rect().width() > theme::WIDTH_COMPACT_WORKSPACE {
                        self.presets.state.expanded = true;
                    } else {
                        self.presets.narrow_open = true;
                    }
                    self.pending_scroll = None;
                }
            }
        }
    }

    fn parameter_section_controls(&mut self, ui: &mut egui::Ui, section: &str) {
        let scanned_medium_positive = self.scanned_medium_positive();
        let cache = self.section_labels.entry(section.to_owned()).or_default();
        let capture_reference_illuminant = &self.film_reference_illuminant;
        let data_dir = self.data_dir.clone();
        let changes = controls::show(
            ui,
            &mut self.params,
            &mut self.gui_state.sections,
            section,
            self.film_medium,
            scanned_medium_positive,
            &self.filter,
            cache,
            capture_reference_illuminant,
            &data_dir,
        );
        if changes.runtime_changed {
            self.mark_parameters_changed_invalidating_calibration();
        }
        if let Some(comparing) = changes.operator_compare {
            self.set_operator_compare(comparing);
        }
        if let Some(comparing) = changes.finishing_compare {
            self.set_finishing_compare(comparing);
        }
        if let Some(action) = changes.action {
            self.start_calibration(action);
        }
    }

    fn chemistry_dev_time(&mut self, ui: &mut egui::Ui, film: bool) {
        let path = if film {
            "film_render.development_time"
        } else {
            "print_render.development_time"
        };
        if !controls::entry_matches(
            path,
            &self.filter,
            &self.params,
            &self.gui_state.sections,
            self.only_modified,
        ) {
            return;
        }
        let times = if film {
            &self.film_dev_times
        } else {
            &self.print_dev_times
        };
        if times.is_empty() {
            return;
        }
        if times.len() == 1 {
            let effective = times[0];
            ui.label(crate::tr!(
                ui.ctx(),
                "surfaces.chemistry.material_time",
                time = effective
            ));
            return;
        }
        let inactive = controls::materials(ui.ctx()).and_then(|materials| {
            spektrafilm_core::params::sources::inactive_reason_with_materials(
                path,
                &self.params,
                Some(&materials.film),
                materials.print.as_deref(),
            )
        });
        if let Some(reason) = &inactive {
            ui.weak(crate::tr!(
                ui.ctx(),
                "surfaces.chemistry.inactive",
                reason = controls::localized(ui.ctx(), reason)
            ));
        }
        let (times, selected) = if film {
            (
                &self.film_dev_times,
                &mut self.params.film_render.development_time,
            )
        } else {
            (
                &self.print_dev_times,
                &mut self.params.print_render.development_time,
            )
        };
        let (changed, response) = ui
            .add_enabled_ui(inactive.is_none(), |ui| {
                dev_time_combo(
                    ui,
                    if film { "film-time" } else { "print-time" },
                    "development time",
                    times,
                    selected,
                )
            })
            .inner;
        if changed {
            controls::record_edited_path(
                ui.ctx(),
                if film {
                    "film_render.development_time"
                } else {
                    "print_render.development_time"
                },
            );
            self.mark_parameters_changed();
        }
        controls::locate_row(ui, path, &response);
    }

    pub(super) fn controls_panel(&mut self, ui: &mut egui::Ui) {
        let input_view_before = (
            self.params.io.input_color_space.clone(),
            self.params.io.input_cctf_decoding,
        );
        // The Convert filter-calibration action uses the selected print stock
        // even when the current render route does not print. Cache both materials
        // so its applicability cannot depend on a previous route's cache state.
        let cached = controls::materials(ui.ctx());
        let cache_current = cached.as_ref().is_some_and(|materials| {
            materials.film_name == self.film_name
                && materials.print_name == self.print_name
                && materials.print.is_some()
        });
        if !cache_current {
            let film = cached
                .as_ref()
                .filter(|materials| materials.film_name == self.film_name)
                .map(|materials| Arc::clone(&materials.film))
                .or_else(|| {
                    profile::load_profile_by_name(&self.data_dir, &self.film_name)
                        .ok()
                        .map(Arc::new)
                });
            let print = cached
                .as_ref()
                .filter(|materials| materials.print_name == self.print_name)
                .and_then(|materials| materials.print.as_ref().map(Arc::clone))
                .or_else(|| {
                    profile::load_profile_by_name(&self.data_dir, &self.print_name)
                        .ok()
                        .map(Arc::new)
                });
            if let Some(film) = film {
                controls::install_material_context(
                    ui.ctx(),
                    self.film_name.clone(),
                    self.print_name.clone(),
                    film,
                    print,
                );
            }
        }
        let workflow_mode = self.params.workflow.mode;
        for &(stage, sections) in controls::STAGE_LAYOUT {
            let reveal_stage = self.pending_stage == Some(stage);
            let has_locator = sections
                .iter()
                .any(|section| self.pending_scroll.as_deref() == Some(*section));
            let visible = sections.iter().any(|section| {
                controls::section_matches(
                    section,
                    &self.filter,
                    &self.params,
                    &self.gui_state.sections,
                    self.only_modified,
                )
            });
            if !visible && !reveal_stage && !has_locator {
                continue;
            }
            let stage_index = controls::Stage::ALL
                .iter()
                .position(|candidate| *candidate == stage);
            let stage_active = stage.active_in(workflow_mode);
            // Inactive stages get a per-mode disclosure identity so they start
            // collapsed when the workflow changes. Stages active in both modes
            // keep one identity, so toggling a module is not lost on a switch.
            let disclosure_mode = if stage.active_in(spektrafilm_core::params::WorkflowMode::Film)
                && stage.active_in(spektrafilm_core::params::WorkflowMode::Lut)
            {
                None
            } else {
                Some(workflow_mode.name())
            };
            ui.push_id(("stage", stage.label(), disclosure_mode), |ui| {
                ui.add_space(theme::SPACE_2);
                let open_id = ui.make_persistent_id("open");
                let normal_open = ui.ctx().data_mut(|data| {
                    *data.get_persisted_mut_or_insert_with(open_id, || stage_active)
                });
                let filtered = !self.filter.is_empty() || self.only_modified;
                let open = normal_open || filtered || reveal_stage || has_locator;
                let response = ui
                    .horizontal(|ui| {
                        if let Some(index) = stage_index {
                            let number = format!("{:02}", index + 1);
                            ui.label(
                                egui::RichText::new(number)
                                    .monospace()
                                    .small()
                                    .color(theme::ACCENT_DIM),
                            );
                        }
                        let response = controls::stage_disclosure(ui, stage.label(), open);
                        if !stage_active {
                            ui.label(
                                egui::RichText::new(crate::i18n::text(
                                    ui.ctx(),
                                    "surfaces.section.inactive_compact",
                                ))
                                .small()
                                .color(theme::FG_MUTED),
                            );
                        }
                        response
                    })
                    .inner;
                let accessible_name = crate::tr!(
                    ui.ctx(),
                    "surfaces.section.stage",
                    stage = controls::localized(ui.ctx(), stage.label())
                );
                ui.ctx()
                    .accesskit_node_builder(response.id, |node| node.set_label(accessible_name));
                controls::focus_feedback(ui, &response);
                if !stage_active {
                    ui.label(
                        egui::RichText::new(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.section.inactive_in_lut",
                        ))
                        .small()
                        .color(theme::FG_MUTED),
                    );
                }
                if response.clicked() && !filtered {
                    ui.ctx()
                        .data_mut(|data| data.insert_persisted(open_id, !normal_open));
                }
                if reveal_stage || has_locator {
                    ui.ctx()
                        .data_mut(|data| data.insert_persisted(open_id, true));
                    if reveal_stage && !has_locator {
                        response.request_focus();
                        ui.scroll_to_rect(response.rect, Some(egui::Align::TOP));
                    }
                    self.pending_stage = None;
                }
                if open {
                    for section in sections {
                        self.parameter_section(ui, section);
                    }
                }
                ui.add_space(theme::SPACE_MODULE);
            });
        }
        if input_view_before
            != (
                self.params.io.input_color_space.clone(),
                self.params.io.input_cctf_decoding,
            )
        {
            self.refresh_input_viewer();
            self.refresh_viewing_artifacts();
        }
    }
}

fn effects_controls(
    ui: &mut egui::Ui,
    effects: &mut spektrafilm_core::lut_look::EffectsParams,
    changed: &mut bool,
) {
    *changed |= ui
        .checkbox(
            &mut effects.diffusion.active,
            crate::i18n::text(ui.ctx(), "surfaces.lut.diffusion"),
        )
        .changed();
    *changed |= ui
        .checkbox(
            &mut effects.halation.active,
            crate::i18n::text(ui.ctx(), "surfaces.lut.halation"),
        )
        .changed();
    *changed |= ui
        .checkbox(
            &mut effects.grain.active,
            crate::i18n::text(ui.ctx(), "surfaces.lut.grain"),
        )
        .changed();
    if effects.diffusion.active {
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::i18n::text(
                ui.ctx(),
                "surfaces.lut.diffusion_response",
            ));
            egui::ComboBox::from_id_salt("effects-diffusion-family")
                .selected_text(effects.diffusion.filter_family.clone())
                .show_ui(ui, |ui| {
                    for family in spektrafilm_model::diffusion::VALID_FAMILIES {
                        *changed |= ui
                            .selectable_value(
                                &mut effects.diffusion.filter_family,
                                family.to_owned(),
                                family,
                            )
                            .changed();
                    }
                });
        });
        *changed |= ui
            .add(
                egui::Slider::new(&mut effects.diffusion.strength, 0.0..=1.0)
                    .text(crate::i18n::text(ui.ctx(), "controls.label.strength")),
            )
            .changed();
        *changed |= ui
            .add(
                egui::Slider::new(&mut effects.diffusion.spatial_scale, 0.0..=4.0)
                    .text(crate::i18n::text(ui.ctx(), "surfaces.lut.diffusion_scale")),
            )
            .changed();
    }
    if effects.halation.active {
        *changed |= ui
            .add(
                egui::Slider::new(&mut effects.halation.halation_amount, 0.0..=4.0)
                    .text(crate::i18n::text(ui.ctx(), "surfaces.lut.halation_amount")),
            )
            .changed();
        *changed |= ui
            .add(
                egui::Slider::new(&mut effects.halation.boost_ev, -4.0..=8.0).text(
                    crate::i18n::text(ui.ctx(), "surfaces.lut.halation_threshold"),
                ),
            )
            .changed();
        *changed |= ui
            .add(
                egui::Slider::new(&mut effects.halation.boost_range, 0.0..=1.0)
                    .text(crate::i18n::text(ui.ctx(), "surfaces.lut.halation_range")),
            )
            .changed();
        *changed |= ui
            .add(
                egui::Slider::new(&mut effects.halation.protect_ev, 0.0..=8.0).text(
                    crate::i18n::text(ui.ctx(), "surfaces.lut.halation_protection"),
                ),
            )
            .changed();
        for (channel, label) in ["R", "G", "B"].into_iter().enumerate() {
            *changed |= ui
                .add(
                    egui::Slider::new(
                        &mut effects.halation.halation_first_sigma_um[channel],
                        0.0..=200.0,
                    )
                    .text(format!(
                        "{} {label} (µm)",
                        crate::i18n::text(ui.ctx(), "surfaces.lut.halation_radius")
                    )),
                )
                .changed();
            *changed |= ui
                .add(
                    egui::Slider::new(&mut effects.halation.halation_strength[channel], 0.0..=1.0)
                        .text(format!(
                            "{} {label}",
                            crate::i18n::text(ui.ctx(), "surfaces.lut.halation_tint")
                        )),
                )
                .changed();
        }
        *changed |= ui
            .add(
                egui::Slider::new(&mut effects.halation.halation_spatial_scale, 0.0..=4.0).text(
                    crate::i18n::text(ui.ctx(), "surfaces.lut.halation_radius_scale"),
                ),
            )
            .changed();
    }
    if effects.grain.active {
        *changed |= ui
            .add(
                egui::Slider::new(&mut effects.grain.amount, 0.0..=1.0)
                    .text(crate::i18n::text(ui.ctx(), "surfaces.lut.grain_amount")),
            )
            .changed();
        *changed |= ui
            .add(
                egui::Slider::new(&mut effects.grain.size, 0.25..=16.0)
                    .text(crate::i18n::text(ui.ctx(), "surfaces.lut.grain_size")),
            )
            .changed();
        *changed |= ui
            .add(
                egui::Slider::new(&mut effects.grain.chroma, 0.0..=1.0)
                    .text(crate::i18n::text(ui.ctx(), "surfaces.lut.grain_chroma")),
            )
            .changed();
        for (label, field) in [
            (
                crate::i18n::text(ui.ctx(), "surfaces.lut.grain_shadows"),
                &mut effects.grain.shadows,
            ),
            (
                crate::i18n::text(ui.ctx(), "surfaces.lut.grain_midtones"),
                &mut effects.grain.midtones,
            ),
            (
                crate::i18n::text(ui.ctx(), "surfaces.lut.grain_highlights"),
                &mut effects.grain.highlights,
            ),
        ] {
            *changed |= ui
                .add(egui::Slider::new(field, 0.0..=1.0).text(label))
                .changed();
        }
        *changed |= ui
            .add(
                egui::DragValue::new(&mut effects.grain.seed)
                    .speed(1.0)
                    .prefix(format!(
                        "{} ",
                        crate::i18n::text(ui.ctx(), "surfaces.lut.grain_seed")
                    )),
            )
            .changed();
    }
}

fn grade_controls(
    ui: &mut egui::Ui,
    grade: &mut spektrafilm_core::lut_look::GradeParams,
    changed: &mut bool,
) {
    egui::CollapsingHeader::new(crate::i18n::text(ui.ctx(), "surfaces.lut.grade"))
        .id_salt("lut-grade")
        .show(ui, |ui| {
            *changed |= ui
                .checkbox(
                    &mut grade.active,
                    crate::i18n::text(ui.ctx(), "surfaces.lut.grade_enable"),
                )
                .changed();
            if grade.active {
                for (label, field) in [
                    (
                        "controls.finishing.brightness",
                        &mut grade.values.brightness,
                    ),
                    ("controls.finishing.contrast", &mut grade.values.contrast),
                    ("controls.finishing.warmth", &mut grade.values.warmth),
                    (
                        "controls.finishing.saturation",
                        &mut grade.values.saturation,
                    ),
                ] {
                    *changed |= ui
                        .add(
                            egui::Slider::new(field, -1.0..=1.0)
                                .text(crate::i18n::text(ui.ctx(), label)),
                        )
                        .changed();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use spektrafilm_backend::cpu_backend::CpuBackend;
    use std::sync::Arc;

    #[test]
    fn scanner_adjust_locator_reveals_glare_under_filter() {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        crate::state::GuiState::factory().save(&state_path).unwrap();
        let mut app = App::new_with_browser_path(
            Arc::new(CpuBackend),
            None,
            Some(state_path),
            None,
            directory.path().join("browser.json"),
            crate::headless::data_dir(),
        );
        app.params.workflow.route = "input > film > print > scan".into();
        app.params.print_render.glare.active = false;
        app.filter = "scanner".into();
        app.only_modified = false;
        let mut harness = Harness::new_ui_state(
            |ui, app: &mut App| {
                app.parameter_section(ui, "Scanner");
                app.parameter_section(ui, "Glare");
            },
            app,
        );
        harness.run();
        assert!(harness.query_by_label("Adjust").is_some());
        harness.get_by_label("Adjust").simulate_click();
        harness.run();
        assert!(harness.state().filter.is_empty());
        assert!(harness.query_by_label("Simulate viewing glare").is_some());
        assert!(
            harness
                .get_by_label("Simulate viewing glare")
                .node()
                .is_focused()
        );
    }
}
