//! Session-local photographs. Inactive entries own accepted context; the active
//! context remains in App. Workers prepare pixels and validate copied settings;
//! only successful UI commits move histories, drafts, and list membership.
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, ensure};
use eframe::egui;
use spektrafilm_core::{
    feature_profiles::{self, FeatureProfileFamily},
    params::{RuntimeParams, sources::ParameterEdits},
    presets::{Preset, Provenance, Section},
    profile,
};

use super::{PhotoAction, PhotoContext, record::PhotographSettings};
use crate::{
    App,
    app::{Message, export_queue::QueueSubmission},
    render::RenderSnapshot,
};

struct Entry {
    path: PathBuf,
    initial: PhotographSettings,
    context: Option<PhotoContext>,
    selected: bool,
    outcome: Option<Message>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CopyScope {
    Look,
    Section(Section),
    Feature(FeatureProfileFamily),
    Operator,
}

#[derive(Default)]
pub(crate) struct WorkList {
    pub(crate) open: bool,
    pub(crate) discard_outgoing: bool,
    entries: Vec<Entry>,
    active: Option<usize>,
    scope: Option<CopyScope>,
    output_directory: Option<PathBuf>,
    replace: bool,
}

impl WorkList {
    /// Only membership changes; callers retain the outgoing context first.
    pub(crate) fn detach_active(&mut self) {
        self.active = None;
    }

    fn adjacent(&self, next: bool) -> Option<usize> {
        match (self.active, next) {
            (Some(index), true) if index + 1 < self.entries.len() => Some(index + 1),
            (Some(index), false) if index > 0 => Some(index - 1),
            (None, true) if !self.entries.is_empty() => Some(0),
            _ => None,
        }
    }
}

/// Canonicalize existing files (including symlinks), and normalize nonexistent
/// paths lexically so adding the same pending path twice cannot duplicate it.
fn normalized_path(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn provenance() -> Provenance {
    Provenance {
        implementation_version: env!("CARGO_PKG_VERSION").into(),
        model_version: spektrafilm_core::runtime::MODEL_VERSION.into(),
        scanner: None,
    }
}

fn accepted_params(settings: &PhotographSettings) -> RuntimeParams {
    let mut params = settings.params.clone();
    params.restore_render_preparation_state(&settings.preparation);
    params
}

fn snapshot(settings: &PhotographSettings, data_dir: &Path) -> RenderSnapshot {
    RenderSnapshot::new(
        accepted_params(settings),
        settings.film.clone(),
        settings.print.clone(),
        data_dir.to_path_buf(),
        settings.special.clone(),
    )
}

fn captured_scope(source: &PhotographSettings, scope: CopyScope) -> Result<Option<Preset>> {
    let params = accepted_params(source);
    Ok(match scope {
        CopyScope::Look => Some(Preset::capture(
            "Copied adjustments",
            &source.film,
            &source.print,
            &params,
            provenance(),
        )),
        CopyScope::Section(section) => Some(Preset::capture_section_detailed(
            "Copied adjustments",
            section,
            &params,
            provenance(),
        )?),
        CopyScope::Operator | CopyScope::Feature(_) => None,
    })
}

/// Material and parameter resolution stays in the existing preset boundary.
/// Capture calibration is explicitly destination-owned even in Look/Convert.
fn copied_settings(
    source: &PhotographSettings,
    destination: &PhotographSettings,
    scope: CopyScope,
    data_dir: &Path,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<PhotographSettings> {
    let current = accepted_params(destination);
    let captured = captured_scope(source, scope)?;
    let mut result = destination.clone();
    result.params = match scope {
        CopyScope::Look => {
            let resolved = captured
                .as_ref()
                .expect("look payload")
                .resolve_detailed(data_dir, &current)?;
            result.film = resolved.film_name;
            result.print = resolved.print_name;
            result.applied = source.applied.clone();
            resolved.params
        }
        CopyScope::Section(_) => {
            let film = profile::load_profile_by_name(data_dir, &destination.film)?;
            captured
                .as_ref()
                .expect("section payload")
                .apply_section_detailed(&current, &film)?
        }
        CopyScope::Feature(family) => {
            let values = feature_profiles::effective_values(family, &accepted_params(source));
            let assignments = values
                .as_object()
                .expect("canonical feature values")
                .iter()
                .map(|(path, value)| format!("{path}={value}"))
                .collect::<Vec<_>>()
                .join(",");
            let edits = ParameterEdits::parse(&assignments).map_err(anyhow::Error::msg)?;
            let mut params = current.clone();
            edits.apply_detailed(&mut params)?;
            params.mark_explicit_paths(edits.paths());
            params
        }
        CopyScope::Operator => {
            let operator = source.params.scanner.operator;
            let edits = ParameterEdits::parse(&format!(
                "scanner.operator.exposure_ev={},scanner.operator.warmth_ev={},scanner.operator.tint_ev={},scanner.operator.contrast={}",
                operator.exposure_ev, operator.warmth_ev, operator.tint_ev, operator.contrast,
            )).map_err(anyhow::Error::msg)?;
            let mut params = current.clone();
            edits.apply_detailed(&mut params)?;
            params.mark_explicit_paths(edits.paths());
            params
        }
    };
    let capture = &destination.params.film_render.convert;
    result
        .params
        .film_render
        .convert
        .scan_illuminant
        .clone_from(&capture.scan_illuminant);
    result
        .params
        .film_render
        .convert
        .calibration
        .clone_from(&capture.calibration);
    result.params.film_render.convert.base_percentile = capture.base_percentile;
    let capture_paths = [
        "film_render.convert.scan_illuminant",
        "film_render.convert.calibration",
        "film_render.convert.base_percentile",
    ];
    result.params.clear_explicit_paths(&capture_paths);
    result.params.mark_explicit_paths(
        destination
            .preparation
            .explicit_section_paths
            .iter()
            .map(String::as_str)
            .filter(|path| capture_paths.contains(path)),
    );
    result.preparation = result.params.render_preparation_state();
    result.params.validate_detailed()?;
    let film = profile::load_profile_by_name(data_dir, &result.film)?;
    result.params.film_render.grain.monochrome = film.is_bw();
    spektrafilm_core::validate_scan_output(&result.params, &film).map_err(anyhow::Error::msg)?;
    // Building the same snapshot boundary used by Export rejects missing styles,
    // profiles, and unsupported material combinations before any history mutation.
    crate::render::build_runtime_observed_with_backend(
        &snapshot(&result, data_dir),
        false,
        &Default::default(),
        backend,
    )?;
    Ok(result)
}

fn flatten_paths(prefix: &str, value: &serde_json::Value, paths: &mut Vec<String>) {
    if let Some(object) = value.as_object() {
        for (key, child) in object {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            flatten_paths(&path, child, paths);
        }
    } else {
        paths.push(prefix.into());
    }
}

fn scope_paths(source: &PhotographSettings, scope: CopyScope) -> Result<Vec<String>> {
    if let CopyScope::Feature(family) = scope {
        return Ok(feature_profiles::scope(family)
            .iter()
            .map(|path| (*path).to_owned())
            .collect());
    }
    if scope == CopyScope::Operator {
        return Ok(["exposure_ev", "warmth_ev", "tint_ev", "contrast"]
            .into_iter()
            .map(|leaf| format!("scanner.operator.{leaf}"))
            .collect());
    }
    let preset = captured_scope(source, scope)?.expect("preset scope");
    let value = serde_json::to_value(&preset)?;
    let mut paths = Vec::new();
    flatten_paths("", &value["parameters"], &mut paths);
    paths.retain(|path| {
        !matches!(
            path.as_str(),
            "film_render.convert.scan_illuminant"
                | "film_render.convert.calibration"
                | "film_render.convert.base_percentile"
        )
    });
    if scope == CopyScope::Look {
        paths.extend(["film_profile.stock".into(), "print_profile.stock".into()]);
    }
    Ok(paths)
}

impl App {
    pub(crate) fn add_work_paths(&mut self, paths: Vec<PathBuf>) {
        if self.input_transition_pending() {
            self.status = Message::new("surfaces.status.preparing_input");
            return;
        }
        let initial = match self.capture_photograph_settings() {
            Ok(settings) => settings,
            Err(error) => {
                self.status = Message::error(&error);
                return;
            }
        };
        if let Some(path) = self.image_path.as_ref().filter(|_| self.image.is_some()) {
            let path = normalized_path(path);
            if let Some(index) = self
                .photograph
                .work_list
                .entries
                .iter()
                .position(|entry| entry.path == path)
            {
                // A standalone reopen can share a path with independently
                // retained work. Membership addition must not replace it.
                if self.photograph.work_list.active.is_none()
                    && self.photograph.work_list.entries[index].context.is_none()
                {
                    self.photograph.work_list.active = Some(index);
                }
            } else {
                self.photograph.work_list.entries.push(Entry {
                    path,
                    initial: initial.clone(),
                    context: None,
                    selected: false,
                    outcome: None,
                });
                self.photograph.work_list.active =
                    Some(self.photograph.work_list.entries.len() - 1);
            }
        }
        for path in paths {
            let path = normalized_path(&path);
            if self
                .photograph
                .work_list
                .entries
                .iter()
                .any(|entry| entry.path == path)
            {
                continue;
            }
            let mut settings = initial.clone();
            settings.rotation = 0;
            if spektrafilm_core::image_io::is_raw(&path) {
                settings.params.io.input_color_space = "ACES2065-1".into();
                settings.params.io.input_cctf_decoding = false;
            }
            self.photograph.work_list.entries.push(Entry {
                path,
                initial: settings,
                context: None,
                selected: false,
                outcome: None,
            });
        }
    }

    /// Capture discard preparation without moving the active history or drafts.
    pub(crate) fn stage_outgoing_work(&self) -> Result<Option<super::PhotoPreparation>> {
        if self.photograph.work_list.active.is_some()
            && self.photograph.work_list.discard_outgoing
            && let (Some(path), Some(settings)) = (&self.image_path, &self.photograph.baseline)
        {
            return self
                .photo_preparation(path, Some(settings.clone()))
                .map(Some);
        }
        Ok(None)
    }

    /// Called only after every candidate and outgoing input has prepared.
    pub(crate) fn retain_prepared_work_context(
        &mut self,
        restored: Option<PhotoContext>,
    ) -> Result<()> {
        if let Some(index) = self.photograph.work_list.active {
            let context = if let Some(mut context) = restored {
                ensure!(
                    Some(context.source_digest.as_str())
                        == self.photograph.source_digest.as_deref(),
                    Message::new("photo.error.source_changed")
                );
                context.record_path = self.photograph.record_path.clone();
                context
            } else {
                self.take_photo_context()?
            };
            self.photograph.work_list.entries[index].context = Some(context);
            self.photograph.work_list.detach_active();
        }
        self.photograph.work_list.discard_outgoing = false;
        Ok(())
    }

    fn prepare_work_entry(&self, index: usize) -> Result<super::PhotoPreparation> {
        let entry = self
            .photograph
            .work_list
            .entries
            .get(index)
            .ok_or_else(|| anyhow::Error::new(Message::new("photo_work.list.invalid_entry")))?;
        self.photo_preparation(&entry.path, Some(entry.initial.clone()))
    }

    pub(crate) fn perform_work_select(&mut self, index: usize) -> Result<()> {
        ensure!(
            index < self.photograph.work_list.entries.len(),
            Message::new("photo_work.list.invalid_entry")
        );
        if self.photograph.work_list.active == Some(index) {
            return Ok(());
        }
        let preparation = if self.photograph.work_list.entries[index].context.is_none() {
            Some(self.prepare_work_entry(index)?)
        } else {
            None
        };
        let max_size = Some(self.params.settings.preview_max_size);
        let candidate = if let Some(preparation) = &preparation {
            preparation.input(max_size)
        } else {
            let context = self.photograph.work_list.entries[index]
                .context
                .as_ref()
                .expect("retained entry");
            self.validate_photo_context(context)?;
            crate::input_worker::InputSpec {
                source: crate::input_worker::InputSource::Existing(context.image.clone()),
                rotation: context.settings.rotation,
                space: context.settings.params.io.input_color_space.clone(),
                decode: context.settings.params.io.input_cctf_decoding,
                max_size,
            }
        };
        let outgoing = self.stage_outgoing_work()?;
        let mut inputs = vec![candidate];
        if let Some(outgoing) = &outgoing {
            inputs.push(outgoing.input(None));
        }
        self.queue_input_command(inputs, None, move |app, mut loaded| {
            let restored = outgoing.map(|preparation| preparation.finish(loaded.remove(1)).0);
            let mut loaded = loaded.remove(0);
            let view = loaded.view.take().expect("selection prepares its view");
            let candidate = preparation.map(|preparation| preparation.finish(loaded).0);
            app.retain_prepared_work_context(restored)?;
            let candidate = candidate
                .or_else(|| app.photograph.work_list.entries[index].context.take())
                .expect("prepared entry");
            app.install_photo_context(candidate, view);
            app.photograph.work_list.active = Some(index);
            Ok(())
        })
    }

    pub(crate) fn perform_work_remove(&mut self, index: usize) -> Result<()> {
        ensure!(
            !self.input_transition_pending(),
            Message::new("surfaces.status.preparing_input")
        );
        ensure!(
            index < self.photograph.work_list.entries.len(),
            Message::new("photo_work.list.invalid_entry")
        );
        self.photograph.work_list.entries.remove(index);
        self.photograph.work_list.active = match self.photograph.work_list.active {
            Some(active) if active == index => None,
            Some(active) if active > index => Some(active - 1),
            active => active,
        };
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn select_work_destination(&mut self, index: usize, selected: bool) {
        if let Some(entry) = self.photograph.work_list.entries.get_mut(index) {
            entry.selected = selected;
        }
    }

    /// Each destination prepares independently; histories remain UI-owned.
    pub(crate) fn copy_work_adjustments(&mut self, scope: CopyScope) -> Result<()> {
        ensure!(
            self.image.is_some(),
            Message::new("photo_work.list.no_source")
        );
        let source = self.capture_photograph_settings()?;
        let indices = self
            .photograph
            .work_list
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.selected.then_some(index))
            .collect();
        self.queue_work_copy(indices, source, scope)
    }

    fn queue_work_copy(
        &mut self,
        indices: Vec<usize>,
        source: PhotographSettings,
        scope: CopyScope,
    ) -> Result<()> {
        ensure!(
            !self.input_transition_pending(),
            Message::new("surfaces.status.preparing_input")
        );
        source.params.validate_detailed()?;
        captured_scope(&source, scope)?;
        let mut stages = Vec::new();
        let mut plans = Vec::new();
        for index in indices {
            let entry = &self.photograph.work_list.entries[index];
            if self.photograph.work_list.active == Some(index)
                || self
                    .image_path
                    .as_ref()
                    .is_some_and(|path| normalized_path(path) == entry.path)
            {
                self.photograph.work_list.entries[index].outcome =
                    Some(Message::new("photo_work.list.source_skipped"));
                continue;
            }
            let preparation = if entry.context.is_none() {
                match self.prepare_work_entry(index) {
                    Ok(preparation) => Some(preparation),
                    Err(error) => {
                        self.photograph.work_list.entries[index].outcome = Some(
                            Message::new("photo_work.list.entry_failed")
                                .message_arg("detail", Message::error(&error)),
                        );
                        continue;
                    }
                }
            } else {
                None
            };
            let settings = preparation
                .as_ref()
                .map(|p| p.settings.clone())
                .unwrap_or_else(|| {
                    entry
                        .context
                        .as_ref()
                        .expect("retained destination")
                        .settings
                        .clone()
                });
            plans.push((preparation.as_ref().map(|p| p.input(None)), settings));
            stages.push((index, preparation));
        }
        let data_dir = self.data_dir.clone();
        let (send, receive) = std::sync::mpsc::channel();
        self.queue_input_command(
            Vec::new(),
            Some(Box::new(move |backend, _| {
                let results = plans
                    .into_iter()
                    .map(|(input, destination)| {
                        let loaded = input
                            .map(|input| crate::input_worker::prepare(input, backend))
                            .transpose()?;
                        let settings =
                            copied_settings(&source, &destination, scope, &data_dir, backend)?;
                        let film = profile::load_profile_by_name(&data_dir, &settings.film)?;
                        let print = profile::load_profile_by_name(&data_dir, &settings.print)?;
                        Ok((loaded, settings, film, print))
                    })
                    .collect::<Vec<Result<_>>>();
                send.send(results)
                    .map_err(|_| anyhow::anyhow!("copy result receiver closed"))?;
                Ok(())
            })),
            move |app, _| {
                let results = receive
                    .try_recv()
                    .map_err(|_| anyhow::anyhow!("copy preparation result missing"))?;
                for ((index, preparation), result) in stages.into_iter().zip(results) {
                    let outcome = match result {
                        Ok((loaded, settings, film, print)) => {
                            if let Some(preparation) = preparation {
                                app.photograph.work_list.entries[index].context = Some(
                                    preparation.finish(loaded.expect("new destination input")).0,
                                );
                            }
                            let context = app.photograph.work_list.entries[index]
                                .context
                                .as_mut()
                                .expect("prepared destination");
                            let changed = context.history.record_copied_params(
                                accepted_params(&settings),
                                settings.film.clone(),
                                settings.print.clone(),
                                &film,
                                &print,
                                settings.applied.clone(),
                            );
                            context.film_reference_illuminant =
                                film.info.reference_illuminant.clone();
                            context.film_medium = crate::scan_medium(&film);
                            context.film_dev_times = film.data.development_time;
                            context.print_dev_times = print.data.development_time;
                            context.settings = settings;
                            Message::new(if changed {
                                "photo_work.list.copy_done"
                            } else {
                                "photo_work.list.copy_unchanged"
                            })
                        }
                        Err(error) => Message::new("photo_work.list.entry_failed")
                            .message_arg("detail", Message::error(&error)),
                    };
                    app.photograph.work_list.entries[index].outcome = Some(outcome);
                }
                Ok(())
            },
        )
    }

    /// The active photograph is guarded by the photograph controller. This
    /// identifies additional contexts that the action would permanently lose.
    pub(crate) fn affected_unsaved_work(&self, action: &PhotoAction) -> Option<usize> {
        let dirty = |index: usize, entry: &Entry| {
            self.photograph.work_list.active != Some(index)
                && entry
                    .context
                    .as_ref()
                    .is_some_and(|context| context.baseline.as_ref() != Some(&context.settings))
        };
        match action {
            PhotoAction::RemoveEntry(index) => self
                .photograph
                .work_list
                .entries
                .get(*index)
                .filter(|entry| dirty(*index, entry))
                .map(|_| *index),
            PhotoAction::Close => self
                .photograph
                .work_list
                .entries
                .iter()
                .enumerate()
                .find_map(|(index, entry)| dirty(index, entry).then_some(index)),
            _ => None,
        }
    }

    pub(crate) fn work_entry_path(&self, index: usize) -> Option<&Path> {
        self.photograph
            .work_list
            .entries
            .get(index)
            .map(|entry| entry.path.as_path())
    }

    fn inactive_work_context(&self, index: usize) -> Result<&PhotoContext> {
        ensure!(
            self.photograph.work_list.active != Some(index),
            Message::new("photo_work.list.invalid_entry")
        );
        self.photograph
            .work_list
            .entries
            .get(index)
            .and_then(|entry| entry.context.as_ref())
            .ok_or_else(|| anyhow::Error::new(Message::new("photo_work.list.invalid_entry")))
    }

    /// A canceled chooser or failed publication leaves this entry's accepted
    /// settings, record path, baseline, and undo stack exactly as they were.
    pub(crate) fn save_inactive_record_dialog(&mut self, index: usize) -> Result<bool> {
        let context = self.inactive_work_context(index)?;
        let path = if let Some(path) = context.record_path.clone() {
            path
        } else {
            let filename = context
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("photograph");
            let Some(path) = self.chooser.save_file(
                self.file_dialog("photo_record")
                    .add_filter("JSON", &["json"])
                    .set_file_name(format!("{filename}.spektra-photo.json")),
            ) else {
                return Ok(false);
            };
            path
        };
        self.save_inactive_record_to(index, &path)?;
        Ok(true)
    }

    pub(crate) fn save_inactive_record_to(&mut self, index: usize, path: &Path) -> Result<()> {
        let context = self.inactive_work_context(index)?;
        ensure!(
            super::record::source_digest(&context.path)? == context.source_digest,
            Message::new("photo.error.source_changed")
        );
        let settings = context.settings.clone();
        let record = super::record::PhotographRecord::capture(
            &context.path,
            settings.clone(),
            &self.data_dir,
        )?;
        ensure!(
            record.source.sha256 == context.source_digest,
            Message::new("photo.error.source_changed")
        );
        record.validate_accepted_lens_model(context.lens_model_identity.as_deref())?;
        record.save(path)?;
        let context = self.photograph.work_list.entries[index]
            .context
            .as_mut()
            .expect("validated inactive entry");
        context.baseline = Some(settings);
        context.record_path = Some(path.to_path_buf());
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn discard_inactive_work(&mut self, index: usize) -> Result<()> {
        self.discard_inactive_work_then(index, |_| Ok(()))
    }

    /// Keep the old context until its baseline source has completely prepared.
    pub(crate) fn discard_inactive_work_then(
        &mut self,
        index: usize,
        continuation: impl FnOnce(&mut Self) -> Result<()> + 'static,
    ) -> Result<()> {
        let context = self.inactive_work_context(index)?;
        let settings = context
            .baseline
            .as_ref()
            .ok_or_else(|| anyhow::Error::new(Message::new("photo_work.list.invalid_entry")))?;
        let preparation = self.photo_preparation(&context.path, Some(settings.clone()))?;
        let digest = context.source_digest.clone();
        let record_path = context.record_path.clone();
        self.queue_input_command(
            vec![preparation.input(None)],
            None,
            move |app, mut loaded| {
                let (mut restored, _) = preparation.finish(loaded.remove(0));
                ensure!(
                    restored.source_digest == digest,
                    Message::new("photo.error.source_changed")
                );
                restored.record_path = record_path;
                app.photograph.work_list.entries[index].context = Some(restored);
                continuation(app)
            },
        )
    }

    pub(crate) fn submit_work_exports(
        &mut self,
        ctx: &egui::Context,
        directory: &Path,
        replace: bool,
    ) -> Result<()> {
        ensure!(
            !self.input_transition_pending(),
            Message::new("surfaces.status.preparing_input")
        );
        ensure!(
            directory.is_dir(),
            Message::new("photo_work.list.output_missing").arg("path", directory.display())
        );
        let options = self.export_options.clone();
        let mut plans = Vec::new();
        for (index, entry) in self.photograph.work_list.entries.iter().enumerate() {
            if !entry.selected {
                continue;
            }
            let plan = (|| -> Result<_> {
                let (settings, input, retained) = if self.photograph.work_list.active == Some(index)
                {
                    let image = self.image.clone().ok_or_else(|| {
                        anyhow::Error::new(Message::new("photo_work.list.no_source"))
                    })?;
                    (
                        self.capture_photograph_settings()?,
                        None,
                        Some((image, self.source_metadata.clone())),
                    )
                } else if let Some(context) = &entry.context {
                    (
                        context.settings.clone(),
                        None,
                        Some((context.image.clone(), context.metadata.clone())),
                    )
                } else {
                    let preparation = self.prepare_work_entry(index)?;
                    let input = preparation.input(None);
                    (preparation.settings, Some(input), None)
                };
                let stem = entry.path.file_stem().ok_or_else(|| {
                    anyhow::Error::new(Message::new("photo_work.list.invalid_entry"))
                })?;
                let mut name = stem.to_os_string();
                name.push(format!(".{}", options.format.extension()));
                Ok((
                    settings,
                    input,
                    retained,
                    entry.path.display().to_string(),
                    directory.join(name),
                ))
            })();
            plans.push((index, plan));
        }
        ensure!(
            !plans.is_empty(),
            Message::new("photo_work.list.no_submissions")
        );
        let data_dir = self.data_dir.clone();
        let ctx = ctx.clone();
        let (send, receive) = std::sync::mpsc::channel();
        self.queue_input_command(
            Vec::new(),
            Some(Box::new(move |backend, _| {
                let results = plans
                    .into_iter()
                    .map(|(index, plan)| {
                        let result = (|| -> Result<QueueSubmission> {
                            let (settings, input, retained, source_name, destination) = plan?;
                            let (image, metadata) = if let Some(input) = input {
                                let loaded = crate::input_worker::prepare(input, backend)?;
                                (loaded.source, loaded.metadata)
                            } else {
                                retained.expect("accepted export source")
                            };
                            Ok(QueueSubmission {
                                source_name,
                                image,
                                metadata,
                                destination,
                                rotation_quarter_turns: settings.rotation.rem_euclid(4) as u32,
                                snapshot: snapshot(&settings, &data_dir),
                                options: options.clone(),
                            })
                        })();
                        (index, result)
                    })
                    .collect::<Vec<_>>();
                send.send(results)
                    .map_err(|_| anyhow::anyhow!("export result receiver closed"))?;
                Ok(())
            })),
            move |app, _| {
                let results = receive
                    .try_recv()
                    .map_err(|_| anyhow::anyhow!("export preparation result missing"))?;
                let mut submissions = Vec::new();
                let mut indices = Vec::new();
                for (index, result) in results {
                    match result {
                        Ok(submission) => {
                            indices.push(index);
                            submissions.push(submission);
                        }
                        Err(error) => {
                            app.photograph.work_list.entries[index].outcome = Some(
                                Message::new("photo_work.list.entry_failed")
                                    .message_arg("detail", Message::error(&error)),
                            )
                        }
                    }
                }
                ensure!(
                    !submissions.is_empty(),
                    Message::new("photo_work.list.no_submissions")
                );
                let result = app.enqueue_exports(&ctx, submissions, replace);
                for index in indices {
                    app.photograph.work_list.entries[index].outcome = Some(match &result {
                        Ok(()) => Message::new("photo_work.list.queued"),
                        Err(error) => Message::new("photo_work.list.entry_failed")
                            .message_arg("detail", Message::error(error)),
                    });
                }
                result
            },
        )
    }

    pub(crate) fn work_list_controls(&mut self, ctx: &egui::Context) {
        if !self.photograph.work_list.open {
            return;
        }
        let mut work = std::mem::take(&mut self.photograph.work_list);
        let mut open = work.open;
        let mut add = false;
        let mut action = None;
        let mut copy = false;
        let mut choose_directory = false;
        let mut export = false;
        let source = self
            .image
            .as_ref()
            .and_then(|_| self.capture_photograph_settings().ok());
        egui::Window::new(crate::i18n::text(ctx, "photo_work.list.title"))
            .id(egui::Id::new("photograph-work-list"))
            .open(&mut open)
            .default_width(650.0)
            .vscroll(true)
            .show(ctx, |ui| {
                if self.input_transition_pending() {
                    ui.disable();
                }
                ui.label(crate::i18n::text(ctx, "photo_work.list.introduction"));
                ui.horizontal_wrapped(|ui| {
                    let response = ui.button(crate::i18n::text(ctx, "photo_work.list.add"));
                    crate::controls::locate_row(ui, "photo_work.list.add", &response);
                    add = response.clicked();
                    for (next, key) in [
                        (false, "photo_work.list.previous"),
                        (true, "photo_work.list.next"),
                    ] {
                        let target = work.adjacent(next);
                        let response = ui.add_enabled(
                            target.is_some(),
                            egui::Button::new(crate::i18n::text(ctx, key)),
                        );
                        crate::controls::locate_row(ui, key, &response);
                        if response.clicked() {
                            action = target.map(PhotoAction::SelectEntry);
                        }
                    }
                });
                if work.entries.is_empty() {
                    ui.label(crate::i18n::text(ctx, "photo_work.list.empty"));
                }
                for (index, entry) in work.entries.iter_mut().enumerate() {
                    ui.push_id(index, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(
                                &mut entry.selected,
                                crate::i18n::text(ctx, "photo_work.list.destination"),
                            );
                            let active = work.active == Some(index);
                            if ui
                                .selectable_label(
                                    active,
                                    entry
                                        .path
                                        .file_name()
                                        .unwrap_or(entry.path.as_os_str())
                                        .to_string_lossy(),
                                )
                                .on_hover_text(entry.path.display().to_string())
                                .clicked()
                            {
                                action = Some(PhotoAction::SelectEntry(index));
                            }
                            let dirty = if active {
                                source.as_ref().is_some_and(|settings| {
                                    self.photograph.baseline.as_ref() != Some(settings)
                                })
                            } else {
                                entry.context.as_ref().is_some_and(|context| {
                                    context.baseline.as_ref() != Some(&context.settings)
                                })
                            };
                            if dirty {
                                ui.label(crate::i18n::text(ctx, "photo_work.list.unsaved"));
                            }
                            let response =
                                ui.small_button(crate::i18n::text(ctx, "photo_work.list.remove"));
                            crate::controls::locate_row(ui, "photo_work.list.remove", &response);
                            if response.clicked() {
                                action = Some(PhotoAction::RemoveEntry(index));
                            }
                        });
                        if let Some(outcome) = &entry.outcome {
                            ui.label(outcome.for_locale(self.resolved_locale));
                        }
                    });
                }
                ui.separator();
                ui.heading(crate::i18n::text(ctx, "photo_work.list.copy_title"));
                ui.label(crate::i18n::text(ctx, "photo_work.list.copy_source"));
                let scope_response = egui::ComboBox::from_id_salt("work-copy-scope")
                    .selected_text(
                        work.scope
                            .map(|scope| scope_label(ctx, scope))
                            .unwrap_or_else(|| {
                                crate::i18n::text(ctx, "photo_work.list.choose_scope").into_owned()
                            }),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut work.scope,
                            Some(CopyScope::Look),
                            scope_label(ctx, CopyScope::Look),
                        );
                        for &section in spektrafilm_core::presets::SECTIONS {
                            ui.selectable_value(
                                &mut work.scope,
                                Some(CopyScope::Section(section)),
                                scope_label(ctx, CopyScope::Section(section)),
                            );
                        }
                        for family in [
                            FeatureProfileFamily::GrainProcedural,
                            FeatureProfileFamily::Halation,
                        ] {
                            ui.selectable_value(
                                &mut work.scope,
                                Some(CopyScope::Feature(family)),
                                scope_label(ctx, CopyScope::Feature(family)),
                            );
                        }
                        ui.selectable_value(
                            &mut work.scope,
                            Some(CopyScope::Operator),
                            scope_label(ctx, CopyScope::Operator),
                        );
                    });
                crate::controls::locate_row(
                    ui,
                    "photo_work.list.copy_scope",
                    &scope_response.response,
                );
                ui.label(crate::i18n::text(ctx, "photo_work.list.copy_excluded"));
                if let (Some(source), Some(scope)) = (&source, work.scope) {
                    match scope_paths(source, scope) {
                        Ok(paths) => {
                            egui::CollapsingHeader::new(crate::i18n::text(
                                ctx,
                                "photo_work.list.replaced_controls",
                            ))
                            .show(ui, |ui| {
                                for path in paths {
                                    ui.monospace(path);
                                }
                            });
                        }
                        Err(error) => {
                            ui.label(Message::error(&error).for_locale(self.resolved_locale));
                        }
                    }
                }
                let selected = work.entries.iter().any(|entry| entry.selected);
                let response = ui.add_enabled(
                    selected && source.is_some() && work.scope.is_some(),
                    egui::Button::new(crate::i18n::text(ctx, "photo_work.list.copy")),
                );
                crate::controls::locate_row(ui, "photo_work.list.copy", &response);
                copy = response.clicked();
                ui.separator();
                ui.heading(crate::i18n::text(ctx, "photo_work.list.export_title"));
                ui.label(crate::i18n::text(ctx, "photo_work.list.export_accepted"));
                ui.horizontal_wrapped(|ui| {
                    let response =
                        ui.button(crate::i18n::text(ctx, "photo_work.list.choose_directory"));
                    crate::controls::locate_row(ui, "photo_work.list.choose_directory", &response);
                    choose_directory = response.clicked();
                    ui.label(
                        work.output_directory
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| {
                                crate::i18n::text(ctx, "photo_work.list.no_directory").into_owned()
                            }),
                    );
                });
                let options = &self.export_options;
                ui.label(crate::tr!(
                    ctx,
                    "photo_work.list.export_summary",
                    format = options.format.label(),
                    backend = options.backend.label(),
                    depth = format!("{:?}", options.depth),
                    color = &options.saving_color_space,
                    encoded = options.saving_cctf_encoding,
                    quality = options.jpeg_quality,
                    subsampling = format!("{:?}", options.jpeg_subsampling),
                    compression = options.compression.argument()
                ));
                ui.label(crate::i18n::text(
                    ctx,
                    "photo_work.list.export_options_guidance",
                ));
                let response = ui.checkbox(
                    &mut work.replace,
                    crate::i18n::text(ctx, "photo_work.list.replace"),
                );
                crate::controls::locate_row(ui, "photo_work.list.replace", &response);
                ui.label(crate::i18n::text(ctx, "photo_work.list.collision_guidance"));
                let response = ui.add_enabled(
                    selected && work.output_directory.is_some(),
                    egui::Button::new(crate::i18n::text(ctx, "photo_work.list.submit")),
                );
                crate::controls::locate_row(ui, "photo_work.list.submit", &response);
                export = response.clicked();
                ui.separator();
                self.export_queue_controls(ui);
            });
        work.open = open;
        let scope = work.scope;
        let directory = work.output_directory.clone();
        let replace = work.replace;
        self.photograph.work_list = work;
        if add && let Some(paths) = self.pick_work_files() {
            self.add_work_paths(paths);
        }
        if choose_directory && let Some(path) = self.pick_work_output_directory() {
            self.photograph.work_list.output_directory = Some(path);
        }
        if let Some(action) = action {
            self.request_photo_action(action);
        }
        let result = if copy {
            self.copy_work_adjustments(scope.expect("enabled scope"))
        } else if export {
            self.submit_work_exports(
                ctx,
                directory.as_deref().expect("enabled directory"),
                replace,
            )
        } else {
            Ok(())
        };
        if let Err(error) = result {
            self.status = Message::error(&error);
        }
    }
}

fn scope_label(ctx: &egui::Context, scope: CopyScope) -> String {
    match scope {
        CopyScope::Look => crate::i18n::text(ctx, "photo_work.list.scope_look").into_owned(),
        CopyScope::Operator => {
            crate::i18n::text(ctx, "photo_work.list.scope_operator").into_owned()
        }
        CopyScope::Section(section) => crate::app::Message::control(section.name()).localized(ctx),
        CopyScope::Feature(family) => crate::i18n::text(
            ctx,
            match family {
                FeatureProfileFamily::GrainProcedural => "photo_work.list.scope_feature_grain",
                FeatureProfileFamily::Halation => "photo_work.list.scope_feature_halation",
            },
        )
        .into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headless::data_dir;

    fn app() -> (App, tempfile::TempDir, egui::Context) {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state.json");
        let factory = crate::state::GuiState::factory();
        let mut params = factory.runtime_params().unwrap();
        params.camera.auto_exposure = false;
        params.debug.deactivate_spatial_effects = true;
        params.debug.deactivate_stochastic_effects = true;
        params.io.crop = false;
        crate::state::GuiState::from_runtime(
            &params,
            factory.film(),
            factory.paper(),
            &factory.sections,
        )
        .unwrap()
        .save(&state)
        .unwrap();
        let mut app = App::new_with_browser_path(
            std::sync::Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
            None,
            Some(state),
            None,
            directory.path().join("browser.json"),
            data_dir(),
        );
        app.chooser.set_strict(true);
        let path = source(directory.path(), "first.png");
        let ctx = egui::Context::default();
        app.request_photo_action(PhotoAction::Open(path));
        crate::headless::await_app_input(&mut app, &ctx);
        (app, directory, ctx)
    }

    fn source(directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(name);
        let image = spektrafilm_math::image::ImageBuf::new(24, 16);
        spektrafilm_core::image_io::save(
            &path,
            &image,
            spektrafilm_core::image_io::SaveOptions {
                depth: spektrafilm_core::image_io::BitDepth::Eight,
                color_space: "sRGB",
                cctf_encoding: true,
                jpeg_quality: None,
                jpeg_subsampling: None,
                compression: None,
            },
            None,
        )
        .unwrap();
        path
    }

    fn exposure(app: &mut App, ctx: &egui::Context, value: f64) {
        app.begin_adjustment_frame(ctx);
        app.params.camera.exposure_compensation_ev = value as _;
        app.mark_parameters_changed_invalidating_calibration();
        app.end_adjustment_frame(ctx);
    }

    #[test]
    fn membership_normalizes_paths_and_navigation_retains_settings_baseline_and_history() {
        let (mut app, directory, ctx) = app();
        let first = app.image_path.clone().unwrap();
        let second = source(directory.path(), "second.png");
        app.add_work_paths(vec![
            second.clone(),
            directory.path().join("./second.png"),
            first.clone(),
        ]);
        assert_eq!(app.photograph.work_list.entries.len(), 2);
        assert_eq!(app.photograph.work_list.adjacent(false), None);
        assert_eq!(app.photograph.work_list.adjacent(true), Some(1));
        let baseline = app.photograph.baseline.clone();
        exposure(&mut app, &ctx, 1.25);
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
        exposure(&mut app, &ctx, -0.5);
        app.perform_work_select(0).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.params.camera.exposure_compensation_ev, 1.25);
        assert_eq!(app.photograph.baseline, baseline);
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
        app.perform_work_remove(0).unwrap();
        assert_eq!(app.image_path.as_ref(), Some(&first));
        assert!(first.is_file() && second.is_file());
        assert_eq!(app.photograph.work_list.active, None);
        assert_eq!(app.photograph.work_list.entries.len(), 1);
    }

    #[test]
    fn adding_files_from_standalone_same_path_preserves_retained_copy_history_record_and_draft() {
        let (mut app, directory, ctx) = app();
        let second = source(directory.path(), "second.png");
        let third = source(directory.path(), "third.png");
        let second_record = directory.path().join("second.spektra-photo.json");
        app.add_work_paths(vec![second.clone()]);
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        app.save_photograph_record_to(&second_record).unwrap();
        app.perform_work_select(0).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        exposure(&mut app, &ctx, 1.25);
        app.select_work_destination(1, true);
        app.copy_work_adjustments(CopyScope::Section(Section::Camera))
            .unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        let retained = app.photograph.work_list.entries[1]
            .context
            .as_mut()
            .unwrap();
        retained.raw_draft["white_balance"] = "custom".into();
        retained.raw_draft["temperature"] = 4200.0.into();
        let draft = retained.raw_draft.clone();
        let settings = retained.settings.clone();
        let baseline = retained.baseline.clone();
        let pixels = retained.image.clone();
        app.save_photograph_record_to(&directory.path().join("first.spektra-photo.json"))
            .unwrap();
        app.request_photo_action(PhotoAction::Open(second.clone()));
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.image_path.as_ref(), Some(&second));
        assert_eq!(app.photograph.work_list.active, None);
        app.add_work_paths(vec![third]);
        assert_eq!(app.photograph.work_list.entries.len(), 3);
        assert_eq!(app.photograph.work_list.active, None);
        let retained = app.photograph.work_list.entries[1]
            .context
            .as_ref()
            .unwrap();
        assert_eq!(retained.settings, settings);
        assert_eq!(retained.baseline, baseline);
        assert_eq!(retained.record_path.as_ref(), Some(&second_record));
        assert_eq!(retained.raw_draft, draft);
        assert!(pixels.same_source(&retained.image));
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.gui_state.sections["load_raw"], draft);
        assert_eq!(app.photograph.record_path.as_ref(), Some(&second_record));
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
    }

    #[test]
    fn failed_navigation_even_with_discard_intent_preserves_active_pixels_and_history() {
        let (mut app, directory, ctx) = app();
        app.add_work_paths(vec![directory.path().join("missing.png")]);
        exposure(&mut app, &ctx, 0.75);
        let image = app.image.clone().unwrap();
        let path = app.image_path.clone();
        app.photograph.work_list.discard_outgoing = true;
        app.perform_work_select(1).unwrap();
        assert!(app.input_pending());
        crate::headless::await_app_input(&mut app, &ctx);
        assert!(
            app.status
                .for_locale(crate::i18n::Locale::English)
                .contains("missing.png")
        );
        assert_eq!(app.image_path, path);
        assert!(image.same_source(app.image.as_ref().unwrap()));
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.75);
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
    }

    #[test]
    fn failed_outgoing_discard_reload_preserves_both_history_owners_and_membership() {
        let (mut app, directory, ctx) = app();
        let second = source(directory.path(), "second.png");
        app.add_work_paths(vec![second]);
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        exposure(&mut app, &ctx, -0.5);
        app.perform_work_select(0).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        exposure(&mut app, &ctx, 1.25);
        app.gui_state.sections["load_raw"]["temperature"] = 4200.0.into();
        let draft = app.gui_state.sections["load_raw"].clone();
        let image = app.image.clone().unwrap();
        let baseline = app.photograph.baseline.clone();
        std::fs::remove_file(app.image_path.as_ref().unwrap()).unwrap();
        app.photograph.work_list.discard_outgoing = true;
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.photograph.work_list.active, Some(0));
        assert_eq!(app.photograph.baseline, baseline);
        assert_eq!(app.gui_state.sections["load_raw"], draft);
        assert!(image.same_source(app.image.as_ref().unwrap()));
        assert_eq!(app.params.camera.exposure_compensation_ev, 1.25);
        assert_eq!(
            app.photograph.work_list.entries[1]
                .context
                .as_ref()
                .unwrap()
                .settings
                .params
                .camera
                .exposure_compensation_ev,
            -0.5
        );
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
        app.photograph.work_list.discard_outgoing = false;
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
    }

    #[test]
    fn pending_explicit_selection_rejects_new_commands_without_superseding_it() {
        let (mut app, directory, ctx) = app();
        let second = source(directory.path(), "second.png");
        let third = source(directory.path(), "third.png");
        app.add_work_paths(vec![second.clone(), third]);
        app.perform_work_select(1).unwrap();
        assert!(app.perform_work_select(2).is_err());
        assert!(app.perform_work_remove(1).is_err());
        assert!(app.copy_work_adjustments(CopyScope::Operator).is_err());
        assert_eq!(app.photograph.work_list.active, Some(0));
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.image_path.as_ref(), Some(&second));
        assert_eq!(app.photograph.work_list.active, Some(1));
    }

    #[test]
    fn successful_discard_navigation_retains_the_previous_saved_baseline_only() {
        let (mut app, directory, ctx) = app();
        let second = source(directory.path(), "second.png");
        app.add_work_paths(vec![second]);
        exposure(&mut app, &ctx, 1.0);
        app.photograph.work_list.discard_outgoing = true;
        app.perform_work_select(1).unwrap();
        app.photograph.work_list.discard_outgoing = false;
        crate::headless::await_app_input(&mut app, &ctx);
        assert!(!app.photograph.work_list.discard_outgoing);
        app.perform_work_select(0).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
        assert!(!app.can_undo());
    }

    #[test]
    fn duplicate_export_destinations_fail_before_any_submission_or_publication() {
        let (mut app, directory, ctx) = app();
        let other = directory.path().join("other");
        std::fs::create_dir(&other).unwrap();
        let duplicate = source(&other, "first.png");
        app.add_work_paths(vec![duplicate]);
        app.select_work_destination(0, true);
        app.select_work_destination(1, true);
        let output = directory.path().join("output");
        std::fs::create_dir(&output).unwrap();
        app.submit_work_exports(&ctx, &output, false).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert!(app.photograph.work_list.entries.iter().all(|entry| {
            entry.outcome.as_ref().is_some_and(|outcome| {
                outcome
                    .for_locale(crate::i18n::Locale::English)
                    .contains("More than one pending export")
            })
        }));
        assert!(app.export_job.is_none());
        assert_eq!(std::fs::read_dir(output).unwrap().count(), 0);
    }

    #[test]
    fn copy_is_atomic_per_destination_and_reversible_after_selecting_it() {
        let (mut app, directory, ctx) = app();
        let second = source(directory.path(), "second.png");
        app.add_work_paths(vec![second, directory.path().join("missing.png")]);
        exposure(&mut app, &ctx, 1.5);
        app.select_work_destination(1, true);
        app.select_work_destination(2, true);
        app.copy_work_adjustments(CopyScope::Section(Section::Camera))
            .unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        let work = &app.photograph.work_list;
        assert!(work.entries[2].context.is_none());
        assert!(
            work.entries[2]
                .outcome
                .as_ref()
                .unwrap()
                .for_locale(crate::i18n::Locale::English)
                .contains("missing.png")
        );
        let copied = work.entries[1].context.as_ref().unwrap();
        assert_eq!(copied.settings.params.camera.exposure_compensation_ev, 1.5);
        assert_eq!(
            copied
                .baseline
                .as_ref()
                .unwrap()
                .params
                .camera
                .exposure_compensation_ev,
            0.0
        );
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
        assert!(app.redo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, 1.5);
    }

    #[test]
    fn rejected_copy_preserves_initialized_destination_context_and_undo_stack() {
        let (mut app, directory, ctx) = app();
        let second = source(directory.path(), "second.png");
        app.add_work_paths(vec![second]);
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        exposure(&mut app, &ctx, -0.25);
        app.perform_work_select(0).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        let before = app.photograph.work_list.entries[1]
            .context
            .as_ref()
            .unwrap()
            .settings
            .clone();
        let baseline = app.photograph.work_list.entries[1]
            .context
            .as_ref()
            .unwrap()
            .baseline
            .clone();
        let mut invalid = app.capture_photograph_settings().unwrap();
        invalid.film = "missing-copy-material".into();
        app.queue_work_copy(vec![1], invalid, CopyScope::Look)
            .unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert!(
            app.photograph.work_list.entries[1]
                .outcome
                .as_ref()
                .unwrap()
                .for_locale(crate::i18n::Locale::English)
                .contains("missing-copy-material")
        );
        let context = app.photograph.work_list.entries[1]
            .context
            .as_ref()
            .unwrap();
        assert_eq!(context.settings, before);
        assert_eq!(context.baseline, baseline);
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
    }

    fn copied_inactive(app: &mut App, directory: &Path, ctx: &egui::Context) -> PathBuf {
        let second = source(directory, "second.png");
        app.add_work_paths(vec![second.clone()]);
        exposure(app, ctx, 1.25);
        app.select_work_destination(1, true);
        app.copy_work_adjustments(CopyScope::Section(Section::Camera))
            .unwrap();
        crate::headless::await_app_input(app, ctx);
        second
    }

    #[test]
    fn copied_inactive_entries_are_guarded_for_close_and_targeted_removal_only() {
        let (mut app, directory, ctx) = app();
        let second = copied_inactive(&mut app, directory.path(), &ctx);
        assert_eq!(app.affected_unsaved_work(&PhotoAction::Close), Some(1));
        assert_eq!(
            app.affected_unsaved_work(&PhotoAction::RemoveEntry(1)),
            Some(1)
        );
        assert_eq!(
            app.affected_unsaved_work(&PhotoAction::RemoveEntry(0)),
            None
        );
        assert_eq!(
            app.affected_unsaved_work(&PhotoAction::SelectEntry(1)),
            None
        );
        assert_eq!(app.work_entry_path(1), Some(second.as_path()));
        let active_image = app.image.clone().unwrap();
        app.discard_inactive_work(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.affected_unsaved_work(&PhotoAction::Close), None);
        assert!(active_image.same_source(app.image.as_ref().unwrap()));
        assert_eq!(app.params.camera.exposure_compensation_ev, 1.25);
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
        assert!(!app.can_undo());
    }

    #[test]
    fn close_guard_resolves_each_copied_inactive_destination_independently() {
        let (mut app, directory, ctx) = app();
        let second = source(directory.path(), "second.png");
        let third = source(directory.path(), "third.png");
        app.add_work_paths(vec![second, third]);
        exposure(&mut app, &ctx, 1.0);
        app.select_work_destination(1, true);
        app.select_work_destination(2, true);
        app.copy_work_adjustments(CopyScope::Section(Section::Camera))
            .unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.affected_unsaved_work(&PhotoAction::Close), Some(1));
        app.discard_inactive_work(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.affected_unsaved_work(&PhotoAction::Close), Some(2));
        assert_eq!(
            app.affected_unsaved_work(&PhotoAction::RemoveEntry(1)),
            None
        );
        assert_eq!(
            app.affected_unsaved_work(&PhotoAction::RemoveEntry(2)),
            Some(2)
        );
        app.save_inactive_record_to(2, &directory.path().join("third.spektra-photo.json"))
            .unwrap();
        assert_eq!(app.affected_unsaved_work(&PhotoAction::Close), None);
    }

    #[test]
    fn fresh_raw_work_entries_start_linear_without_overwriting_existing_interpretation() {
        let (mut app, directory, _ctx) = app();
        let interpretation = app.params.io.clone();
        let raw = directory.path().join("fresh.dng");
        app.add_work_paths(vec![raw]);
        let settings = &app.photograph.work_list.entries[1].initial;
        assert_eq!(settings.params.io.input_color_space, "ACES2065-1");
        assert!(!settings.params.io.input_cctf_decoding);
        assert_eq!(settings.rotation, 0);
        assert_eq!(app.params.io, interpretation);
    }

    #[test]
    fn inactive_save_cancel_and_write_failure_preserve_baseline_and_copy_undo() {
        let (mut app, directory, ctx) = app();
        copied_inactive(&mut app, directory.path(), &ctx);
        let before = app.inactive_work_context(1).unwrap().settings.clone();
        let baseline = app.inactive_work_context(1).unwrap().baseline.clone();
        app.chooser.set_strict(true);
        app.chooser
            .scripted(crate::chooser::ChooserResult::Canceled);
        assert!(!app.save_inactive_record_dialog(1).unwrap());
        assert_eq!(app.affected_unsaved_work(&PhotoAction::Close), Some(1));
        assert!(
            app.save_inactive_record_to(1, &directory.path().join("absent/record.json"))
                .is_err()
        );
        assert_eq!(app.inactive_work_context(1).unwrap().settings, before);
        assert_eq!(app.inactive_work_context(1).unwrap().baseline, baseline);
        assert!(app.inactive_work_context(1).unwrap().record_path.is_none());
        let record = directory.path().join("second.spektra-photo.json");
        app.chooser
            .scripted(crate::chooser::ChooserResult::Scripted(record.clone()));
        assert!(app.save_inactive_record_dialog(1).unwrap());
        assert_eq!(app.affected_unsaved_work(&PhotoAction::Close), None);
        assert_eq!(
            app.inactive_work_context(1).unwrap().record_path.as_ref(),
            Some(&record)
        );
        let saved = super::super::record::PhotographRecord::load(&record).unwrap();
        assert_eq!(saved.settings, before);
        assert_eq!(app.params.camera.exposure_compensation_ev, 1.25);
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
        assert!(app.photograph_has_unsaved_changes());
    }

    #[test]
    fn changed_inactive_source_blocks_save_and_discard_without_losing_work() {
        let (mut app, directory, ctx) = app();
        let second = copied_inactive(&mut app, directory.path(), &ctx);
        let before = app.inactive_work_context(1).unwrap().settings.clone();
        let baseline = app.inactive_work_context(1).unwrap().baseline.clone();
        std::fs::write(&second, b"changed source identity").unwrap();
        let record = directory.path().join("second.spektra-photo.json");
        assert!(app.save_inactive_record_to(1, &record).is_err());
        assert!(!record.exists());
        app.discard_inactive_work(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert_eq!(app.inactive_work_context(1).unwrap().settings, before);
        assert_eq!(app.inactive_work_context(1).unwrap().baseline, baseline);
        assert_eq!(app.affected_unsaved_work(&PhotoAction::Close), Some(1));
        app.perform_work_select(1).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, 0.0);
    }

    #[test]
    fn copy_look_and_finishing_scope_leave_unrelated_scopes_independent() {
        let (app, _directory, _ctx) = app();
        let mut destination = app.capture_photograph_settings().unwrap();
        destination.params.finishing.warmth = -0.3;
        let mut source = destination.clone();
        source.params.finishing.warmth = 0.4;
        source.params.finishing.expand.black = 0.1;
        for scope in [CopyScope::Look, CopyScope::Section(Section::Finishing)] {
            let copied = copied_settings(
                &source,
                &destination,
                scope,
                &app.data_dir,
                app.backend.as_ref(),
            )
            .unwrap();
            assert_eq!(copied.params.finishing, source.params.finishing);
        }
        let copied = copied_settings(
            &source,
            &destination,
            CopyScope::Section(Section::Camera),
            &app.data_dir,
            app.backend.as_ref(),
        )
        .unwrap();
        assert_eq!(copied.params.finishing, destination.params.finishing);
    }

    #[test]
    fn copy_preserves_source_calibration_geometry_delivery_and_rejects_invalid_candidates() {
        let (app, _directory, _ctx) = app();
        let destination = app.capture_photograph_settings().unwrap();
        let mut source = destination.clone();
        source.params.film_render.convert.scan_illuminant = "D65".into();
        source.params.film_render.convert.calibration = "1.1 0 0  0 1 0  0 0 1".into();
        source.params.film_render.convert.base_percentile = 90.0;
        source.params.film_render.convert.exposure_compensation_ev = 0.5;
        source.params.io.input_color_space = "ACES2065-1".into();
        source.params.io.crop = !destination.params.io.crop;
        source.rotation = 1;
        for scope in [CopyScope::Look, CopyScope::Section(Section::Convert)] {
            let copied = copied_settings(
                &source,
                &destination,
                scope,
                &app.data_dir,
                app.backend.as_ref(),
            )
            .unwrap();
            assert_eq!(
                copied.params.film_render.convert.scan_illuminant,
                destination.params.film_render.convert.scan_illuminant
            );
            assert_eq!(
                copied.params.film_render.convert.calibration,
                destination.params.film_render.convert.calibration
            );
            assert_eq!(
                copied.params.film_render.convert.base_percentile,
                destination.params.film_render.convert.base_percentile
            );
            assert_eq!(
                copied.params.film_render.convert.exposure_compensation_ev,
                0.5
            );
            assert_eq!(copied.params.io, destination.params.io);
            assert_eq!(copied.rotation, destination.rotation);
            assert_eq!(copied.raw, destination.raw);
            assert_eq!(copied.special, destination.special);
        }
        source.params.scanner.operator.contrast = -1.0;
        let before = destination.clone();
        assert!(
            copied_settings(
                &source,
                &destination,
                CopyScope::Operator,
                &app.data_dir,
                app.backend.as_ref()
            )
            .is_err()
        );
        assert_eq!(destination, before);
    }

    #[test]
    fn copied_dye_look_derives_layout_without_erasing_channel_requests() {
        let (app, _directory, _ctx) = app();
        let baseline = app.capture_photograph_settings().unwrap();
        for (source_film, destination_film, monochrome) in [
            ("kodak_doublex", "kodak_portra_800", true),
            ("kodak_portra_800", "kodak_doublex", false),
        ] {
            let mut source = baseline.clone();
            source.film = source_film.into();
            source.params.film_render.grain.engine = spektrafilm_core::params::GrainEngine::Dye;
            source.params.film_render.grain.channel_correlation = 0.35;
            source.params.film_render.grain.uniformity = [0.2, 0.4, 0.6];
            let mut destination = baseline.clone();
            destination.film = destination_film.into();
            destination.params.film_render.grain.monochrome = !monochrome;
            let copied = copied_settings(
                &source,
                &destination,
                CopyScope::Look,
                &app.data_dir,
                app.backend.as_ref(),
            )
            .unwrap();
            assert_eq!(copied.film, source_film);
            assert_eq!(copied.params.film_render.grain.monochrome, monochrome);
            assert_eq!(copied.params.film_render.grain.channel_correlation, 0.35);
            assert_eq!(copied.params.film_render.grain.uniformity, [0.2, 0.4, 0.6]);
        }
    }

    #[test]
    fn feature_copy_uses_canonical_family_leaves_including_custom_and_null_overrides() {
        let (app, _directory, _ctx) = app();
        let mut destination = app.capture_photograph_settings().unwrap();
        destination.params.film_render.grain.procedural_size = Some(33.0);
        let mut source = destination.clone();
        source.params.film_render.grain.procedural_profile = "custom".into();
        source.params.film_render.grain.procedural_size = None;
        source.params.film_render.grain.procedural_amount = Some(23.0);
        source.params.film_render.grain.active = !destination.params.film_render.grain.active;
        source.params.film_render.grain.engine =
            spektrafilm_core::params::grain::GrainEngine::Procedural;
        source.params.film_render.grain.procedural_resolution_type = 0;
        source.params.film_render.halation.halation_first_sigma_um = [41.0, 42.0, 43.0];
        source.params.film_render.halation.halation_strength = [0.11, 0.02, 0.001];
        source.params.film_render.halation.active = !destination.params.film_render.halation.active;
        source.params.film_render.halation.scatter_amount = 2.0;
        source.params.film_render.halation.halation_amount = 0.5;
        for family in [
            FeatureProfileFamily::GrainProcedural,
            FeatureProfileFamily::Halation,
        ] {
            let scope = CopyScope::Feature(family);
            let copied = copied_settings(
                &source,
                &destination,
                scope,
                &app.data_dir,
                app.backend.as_ref(),
            )
            .unwrap();
            assert_eq!(
                feature_profiles::effective_values(family, &copied.params),
                feature_profiles::effective_values(family, &source.params)
            );
            assert_eq!(
                scope_paths(&source, scope).unwrap(),
                feature_profiles::scope(family)
                    .iter()
                    .map(|path| (*path).to_owned())
                    .collect::<Vec<_>>()
            );
            let owned = scope_paths(&source, scope).unwrap();
            let mut before = Vec::new();
            flatten_paths(
                "",
                &serde_json::to_value(&destination.params).unwrap(),
                &mut before,
            );
            let destination_values = serde_json::to_value(&destination.params).unwrap();
            let copied_values = serde_json::to_value(&copied.params).unwrap();
            for path in before.into_iter().filter(|path| !owned.contains(path)) {
                let pointer = format!("/{}", path.replace('.', "/"));
                assert_eq!(
                    copied_values.pointer(&pointer),
                    destination_values.pointer(&pointer),
                    "unowned feature path {path}"
                );
            }
            assert_eq!(copied.film, destination.film);
            assert_eq!(copied.print, destination.print);
            assert_eq!(copied.raw, destination.raw);
            assert_eq!(copied.rotation, destination.rotation);
        }
    }

    #[test]
    fn each_feature_scope_copy_is_one_immutable_reversible_destination_transaction() {
        for family in [
            FeatureProfileFamily::GrainProcedural,
            FeatureProfileFamily::Halation,
        ] {
            let (mut app, directory, ctx) = app();
            let second = source(directory.path(), "second.png");
            app.add_work_paths(vec![second]);
            app.begin_adjustment_frame(&ctx);
            match family {
                FeatureProfileFamily::GrainProcedural => {
                    app.params.film_render.grain.procedural_profile = "custom".into();
                    app.params.film_render.grain.procedural_amount = Some(23.0);
                    app.params.film_render.grain.procedural_size = None;
                }
                FeatureProfileFamily::Halation => {
                    app.params.film_render.halation.halation_first_sigma_um = [41.0, 42.0, 43.0];
                    app.params.film_render.halation.halation_strength = [0.11, 0.02, 0.001];
                }
            }
            app.mark_parameters_changed_invalidating_calibration();
            app.end_adjustment_frame(&ctx);
            let captured = feature_profiles::effective_values(family, &app.params);
            app.select_work_destination(1, true);
            app.copy_work_adjustments(CopyScope::Feature(family))
                .unwrap();
            crate::headless::await_app_input(&mut app, &ctx);
            let expected = app.photograph.work_list.entries[1]
                .context
                .as_ref()
                .unwrap()
                .settings
                .clone();
            app.begin_adjustment_frame(&ctx);
            app.params.film_render.grain.procedural_amount = Some(72.0);
            app.params.film_render.halation.halation_strength = [0.2, 0.1, 0.05];
            app.mark_parameters_changed_invalidating_calibration();
            app.end_adjustment_frame(&ctx);
            app.perform_work_select(1).unwrap();
            crate::headless::await_app_input(&mut app, &ctx);
            assert_eq!(
                feature_profiles::effective_values(family, &app.params),
                captured
            );
            assert!(app.undo_adjustment());
            assert!(!app.can_undo());
            assert!(app.redo_adjustment());
            assert_eq!(app.capture_photograph_settings().unwrap(), expected);
            assert!(!app.can_redo());
        }
    }

    #[test]
    fn submitted_work_snapshot_survives_removal_and_live_edits_and_publishes_real_pixels() {
        let (mut app, directory, ctx) = app();
        app.add_work_paths(Vec::new());
        app.select_work_destination(0, true);
        let output = directory.path().join("output");
        std::fs::create_dir(&output).unwrap();
        let reference = directory.path().join("reference");
        std::fs::create_dir(&reference).unwrap();
        app.submit_work_exports(&ctx, &reference, false).unwrap();
        drain(&mut app, &ctx);
        app.submit_work_exports(&ctx, &output, false).unwrap();
        crate::headless::await_app_input(&mut app, &ctx);
        app.perform_work_remove(0).unwrap();
        app.params.camera.exposure_compensation_ev = 4.0;
        app.image = Some(crate::render::InputImage::Host(std::sync::Arc::new(
            spektrafilm_math::image::ImageBuf::new(2, 2),
        )));
        app.film_name = "unrelated-live-material".into();
        drain(&mut app, &ctx);
        let name = format!("first.{}", app.export_options.format.extension());
        assert_eq!(
            spektrafilm_core::image_io::load(&output.join(&name))
                .unwrap()
                .image
                .data,
            spektrafilm_core::image_io::load(&reference.join(&name))
                .unwrap()
                .image
                .data
        );
        assert_eq!(app.image.as_ref().unwrap().width(), 2);
        assert!(directory.path().join("first.png").is_file());
    }

    fn drain(app: &mut App, ctx: &egui::Context) {
        crate::headless::await_app_input(app, ctx);
        let deadline = std::time::Instant::now() + crate::headless::WAIT_DEADLINE;
        while app.export_job.is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "work list export did not finish"
            );
            app.poll_export_job(ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
