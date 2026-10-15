use super::*;

impl App {
    pub(crate) fn load_preset(&mut self, group: PresetGroup, name: &str) {
        let result = self.presets.library.load(group, name).and_then(|selected| {
            let mut current = self.params.clone();
            if let Some((white, black, glare)) = self.scan_for_print_snapshot {
                current.scanner.white_correction = white;
                current.scanner.black_correction = black;
                current.print_render.glare.active = glare;
            }
            match selected.preset.kind()? {
                PresetKind::Look => {
                    let resolved = selected.preset.resolve(&self.data_dir, &current)?;
                    self.film_medium = crate::scan_medium(&resolved.film);
                    self.film_name = resolved.film_name;
                    self.print_name = resolved.print_name;
                    self.params = resolved.params;
                    self.film_dev_times = resolved.film.data.development_time;
                    self.print_dev_times = resolved.print.data.development_time;
                    self.presets.state.applied = Some(AppliedPreset {
                        group,
                        name: name.to_owned(),
                    });
                }
                PresetKind::Section(_) => {
                    let film = spektrafilm_core::profile::load_profile_by_name(
                        &self.data_dir,
                        &self.film_name,
                    )
                    .map_err(|error| error.to_string())?;
                    self.params = selected.preset.apply_section(&current, &film)?;
                    self.film_medium = crate::scan_medium(&film);
                }
            }
            self.invalidate_look_render();
            Ok(())
        });
        self.status = match result {
            Ok(()) => format!("Applied preset '{name}'"),
            Err(e) => format!("Preset error: {e}"),
        };
    }
    pub(crate) fn save_preset_as(&mut self, section: Option<Section>) {
        let result = match section {
            Some(section) => {
                Preset::capture_section("New preset", section, &self.params, provenance())
            }
            None => Ok(Preset::capture(
                "New preset",
                &self.film_name,
                &self.print_name,
                &self.params,
                provenance(),
            )),
        };
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
