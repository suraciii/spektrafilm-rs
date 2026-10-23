use super::*;

impl App {
    pub(crate) fn load_preset(&mut self, group: PresetGroup, name: &str) {
        let mut current = self.params.clone();
        if let Some(glare) = self.scan_for_print_snapshot {
            current.print_render.glare.active = glare;
        }
        let before_scanner = current.scanner.clone();
        let result = self.presets.library.load(group, name).and_then(|selected| {
            if let PresetKind::Section(section) = selected.preset.kind()? {
                // Only the print payload owns the temporarily bypassed glare.
                let mut section_current = self.params.clone();
                if section == Section::Print {
                    section_current.print_render.glare.active = current.print_render.glare.active;
                }
                let film = spektrafilm_core::profile::load_profile_by_name(
                    &self.data_dir,
                    &self.film_name,
                )
                .map_err(|error| error.to_string())?;
                let candidate = selected.preset.apply_section(&section_current, &film)?;
                self.params = candidate;
                if section == Section::Print {
                    self.scan_for_print_snapshot = None;
                }
                self.mark_parameters_changed_invalidating_calibration();
                self.label_adjustment(&format!("Apply {} preset '{name}'", section.name()));
                return Ok(());
            }
            let resolved = selected.preset.resolve(&self.data_dir, &current)?;
            self.film_medium = crate::scan_medium(&resolved.film);
            self.film_reference_illuminant = resolved.film.info.reference_illuminant.clone();
            self.film_name = resolved.film_name;
            self.print_name = resolved.print_name;
            self.gui_state.sections["simulation"]["film_stock"] = self.film_name.clone().into();
            self.gui_state.sections["simulation"]["print_paper"] = self.print_name.clone().into();
            self.params = resolved.params;
            self.film_dev_times = resolved.film.data.development_time;
            self.print_dev_times = resolved.print.data.development_time;
            self.presets.state.applied = Some(AppliedPreset {
                group,
                name: name.to_owned(),
            });
            self.invalidate_look_render();
            self.label_adjustment(&format!("Apply look preset '{name}'"));
            Ok(())
        });
        self.status = match result {
            Ok(()) => {
                let cause = format!("Applied preset '{name}'");
                App::normalization_status(
                    &before_scanner,
                    &self.params.scanner,
                    self.film_medium,
                    &self.params.workflow.route,
                    &cause,
                )
            }
            Err(e) => format!("Preset error: {e}"),
        };
    }
    pub(super) fn save_section_preset_as(&mut self, section: Section) {
        match Preset::capture_section("New preset", section, &self.params, provenance()) {
            Ok(preset) => {
                self.presets.name_dialog = Some(NameDialog {
                    action: NameAction::Save(preset),
                    name: String::new(),
                    error: None,
                })
            }
            Err(error) => self.status = error,
        }
    }
    pub(crate) fn save_preset_as(&mut self) {
        let result = Ok(Preset::capture(
            "New preset",
            &self.film_name,
            &self.print_name,
            &self.params,
            provenance(),
        ));
        match result {
            Ok(preset) => {
                self.presets.name_dialog = Some(NameDialog {
                    action: NameAction::Save(preset),
                    name: String::new(),
                    error: None,
                })
            }
            Err(e) => self.status = e,
        }
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
                self.status = match self.presets.library.update(&captured, &self.data_dir) {
                    Ok(_) => format!("Updated preset '{}'", entry.name),
                    Err(e) => e,
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
                    name: format!("{} copy", entry.name),
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
                    self.status = match self.presets.library.export(preset, &path, &self.data_dir) {
                        Ok(path) => format!("Exported preset to {}", path.display()),
                        Err(e) => e,
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
            match self.presets.library.import(&path, None, &self.data_dir) {
                Ok(path) => {
                    self.status = format!("Imported preset to {}", path.display());
                    self.presets.refresh();
                }
                Err(e) => {
                    self.status = e.clone();
                    self.presets.name_dialog = Some(NameDialog {
                        action: NameAction::Import(path),
                        name: String::new(),
                        error: Some(format!("{e}\nImport a renamed copy:")),
                    });
                }
            }
        }
    }
}
