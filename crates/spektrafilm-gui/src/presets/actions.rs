use super::*;

impl App {
    pub(crate) fn load_preset(&mut self, group: PresetGroup, name: &str) {
        let mut current = self.params.clone();
        if let Some(glare) = self.scan_for_print_snapshot {
            current.print_render.glare.active = glare;
        }
        let before_scanner = current.scanner.clone();
        let result = self
            .presets
            .library
            .load_detailed(group, name)
            .and_then(|selected| {
                let resolved = selected.preset.resolve_detailed(&self.data_dir, &current)?;
                self.film_medium = crate::scan_medium(&resolved.film);
                self.film_reference_illuminant = resolved.film.info.reference_illuminant.clone();
                self.film_name = resolved.film_name;
                self.print_name = resolved.print_name;
                self.gui_state.sections["simulation"]["film_stock"] = self.film_name.clone().into();
                self.gui_state.sections["simulation"]["print_paper"] =
                    self.print_name.clone().into();
                self.params = resolved.params;
                self.film_dev_times = resolved.film.data.development_time;
                self.print_dev_times = resolved.print.data.development_time;
                self.presets.state.applied = Some(AppliedPreset {
                    group,
                    name: name.to_owned(),
                });
                self.presets.record_application(group, name);
                self.invalidate_look_render();
                self.label_adjustment_message(
                    crate::app::Message::new("operations.history.apply_named_look")
                        .arg("name", name),
                );
                Ok(())
            });
        self.status = match result {
            Ok(()) => {
                let cause =
                    crate::app::Message::new("surfaces.status.preset_applied").arg("name", name);
                App::normalization_status(
                    &before_scanner,
                    &self.params.scanner,
                    self.film_medium,
                    &self.params.workflow.route,
                    cause,
                )
            }
            Err(e) => crate::app::Message::new("surfaces.status.preset_error")
                .message_arg("error", super::render::library_error_message(&e)),
        };
    }
    pub(crate) fn save_preset_as(&mut self) {
        let preset = Preset::capture(
            "New preset",
            &self.film_name,
            &self.print_name,
            &self.params,
            provenance(),
        );
        self.presets.name_dialog = Some(NameDialog {
            action: NameAction::Save(preset),
            name: String::new(),
            error: None,
        });
    }
    pub(crate) fn apply_preset_action(&mut self, index: usize, action: PresetAction) {
        let entry = &self.presets.entries[index];
        match action {
            PresetAction::Load => {
                let group = entry.group;
                let name = entry.name.clone();
                self.load_preset(group, &name);
            }
            PresetAction::Update => {
                let preset = entry.preset.as_ref().unwrap();
                let mut captured = Preset::capture(
                    &entry.name,
                    &self.film_name,
                    &self.print_name,
                    &self.params,
                    preset.provenance.clone(),
                );
                captured.author = preset.author.clone();
                captured.description = preset.description.clone();
                self.status = match self
                    .presets
                    .library
                    .update_detailed(&captured, &self.data_dir)
                {
                    Ok(_) => crate::app::Message::new("surfaces.status.preset_updated")
                        .arg("name", &entry.name),
                    Err(e) => crate::app::Message::new("surfaces.status.preset_error")
                        .message_arg("error", super::render::library_error_message(&e)),
                };
                self.presets.refresh();
            }
            PresetAction::Rename => {
                self.presets.name_dialog = Some(NameDialog {
                    action: NameAction::Rename(entry.name.clone()),
                    name: entry.name.clone(),
                    error: None,
                });
            }
            PresetAction::Duplicate => {
                self.presets.name_dialog = Some(NameDialog {
                    action: NameAction::Duplicate(entry.preset.as_ref().unwrap().clone()),
                    name: rust_i18n::t!(
                        "surfaces.presets.copy_name",
                        locale = self.resolved_locale.as_str(),
                        name = &entry.name
                    )
                    .into_owned(),
                    error: None,
                });
            }
            PresetAction::Export => {
                if let Some(path) = self.chooser.save_file(
                    self.file_dialog("preset")
                        .set_file_name(format!("{}.toml", entry.name))
                        .add_filter("Preset", &["toml", "json"]),
                ) {
                    self.remember_dialog("preset", &path);
                    let preset = self.presets.entries[index].preset.as_ref().unwrap();
                    self.status =
                        match self
                            .presets
                            .library
                            .export_detailed(preset, &path, &self.data_dir)
                        {
                            Ok(path) => crate::app::Message::new("surfaces.status.preset_exported")
                                .arg("path", path.display()),
                            Err(e) => crate::app::Message::new("surfaces.status.preset_error")
                                .message_arg("error", super::render::library_error_message(&e)),
                        };
                }
            }
            PresetAction::Reveal => {
                if let Err(e) = reveal(entry.path.as_ref().unwrap()) {
                    self.status = e;
                }
            }
            PresetAction::Delete => self.presets.delete = Some(entry.name.clone()),
        }
    }
    pub(crate) fn import_preset(&mut self) {
        if let Some(path) = self.chooser.pick_file(
            self.file_dialog("preset")
                .add_filter("Preset", &["toml", "json"]),
        ) {
            self.remember_dialog("preset", &path);
            match self.import_look_preset(&path, None) {
                Ok(path) => {
                    self.status = crate::app::Message::new("surfaces.status.preset_imported")
                        .arg("path", path.display());
                    self.presets.refresh();
                }
                Err(e) => {
                    let detail = e;
                    self.status = crate::app::Message::new("surfaces.status.preset_error")
                        .message_arg("error", detail.clone());
                    self.presets.name_dialog = Some(NameDialog {
                        action: NameAction::Import(path),
                        name: String::new(),
                        error: Some(
                            crate::app::Message::new("surfaces.presets.import_rename")
                                .message_arg("detail", detail),
                        ),
                    });
                }
            }
        }
    }

    pub(super) fn import_look_preset(
        &self,
        path: &Path,
        name: Option<&str>,
    ) -> Result<PathBuf, crate::app::Message> {
        let mut preset = spektrafilm_core::presets::load_file_detailed(path)
            .map_err(|error| super::render::library_error_message(&error))?;
        if preset.kind() != Ok(PresetKind::Look) {
            return Err(crate::app::Message::new("surfaces.presets.expected_look"));
        }
        if let Some(name) = name {
            preset.name = name.to_owned();
        }
        self.presets
            .library
            .save_new_format_detailed(
                &preset,
                &self.data_dir,
                path.extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("toml")),
            )
            .map_err(|error| super::render::library_error_message(&error))
    }
}
