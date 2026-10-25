//! App state ownership: construction, applied GUI state and persisted browser/session state.
use crate::*;

pub(crate) struct App {
    pub(crate) backend: Arc<dyn ComputeBackend>,
    pub(crate) data_dir: PathBuf,
    /// Profiles where `info.support == "film"`.
    pub(crate) films: Vec<ProfileEntry>,
    /// Print stocks usable as the print stage (`Profile::is_print_stock`:
    /// paper/printing or film/printing).
    pub(crate) papers: Vec<ProfileEntry>,
    pub(crate) presets: presets::Browser,
    pub(crate) browser_state_path: PathBuf,
    /// Directory owning startup defaults, remembered dialog directories and
    /// the preset library. Production resolves it to the platform
    /// configuration directory; acceptance supplies an isolated one.
    pub(crate) config_directory: PathBuf,
    /// Independent application preferences; never projected into photographic state.
    pub(crate) language_preference: i18n::LanguagePreference,
    pub(crate) resolved_locale: i18n::Locale,
    pub(crate) language_preference_error: Option<i18n::PreferenceError>,
    /// Native chooser boundary; acceptance scripts its result.
    pub(crate) chooser: chooser::Chooser,
    pub(crate) observed_browser_state: presets::BrowserState,
    pub(crate) browser_state_changed_at: Option<Instant>,
    pub(crate) film_reference_illuminant: String,
    pub(crate) film_name: String,
    pub(crate) film_medium: controls::ScanMedium,
    pub(crate) print_name: String,
    /// non-trivial only for B&W stocks. Drives the dev-time pickers.
    pub(crate) film_dev_times: Vec<f64>,
    pub(crate) print_dev_times: Vec<f64>,
    pub(crate) params: RuntimeParams,
    pub(crate) gui_state: state::GuiState,
    pub(crate) full_scan_requested: bool,
    /// Original print-glare switch captured while Scan-for-print is active.
    /// This transient workflow state is deliberately excluded from persistence.
    pub(crate) scan_for_print_snapshot: Option<bool>,
    /// Temporary automatic-baseline display. Never enters saved parameters.
    pub(crate) operator_compare: bool,
    pub(crate) operator_comparison: Option<super::render::OperatorComparison>,
    pub(crate) adjustment_history: super::history::AdjustmentHistory,
    pub(crate) photograph: crate::photograph::PhotographWork,
    pub(crate) retained_kind: Option<RenderKind>,
    pub(crate) stale_save_confirmation: bool,
    pub(crate) export_queue: super::export_queue::ExportQueue,
    pub(crate) image_path: Option<PathBuf>,
    pub(crate) image: Option<crate::render::InputImage>,
    pub(crate) raw_lens_info: Option<String>,
    pub(crate) raw_applied_settings: Option<serde_json::Value>,
    /// Quarter turns applied to the loaded source, used for the rotation status.
    pub(crate) input_rotation: i32,
    /// The retained input with `input_rotation` applied, for presentation only.
    /// Rendering receives the unrotated source plus the deferred quarter turns,
    /// so orientation never rewrites the source pixels.
    pub(crate) oriented_input: Option<crate::render::InputImage>,
    pub(crate) source_metadata: Option<ImageMetadata>,
    pub(crate) export_options: ExportOptions,
    pub(crate) export_dialog: ExportDialog,
    pub(crate) input_epoch: u64,
    /// Latest accepted floating output, shared with background viewing workers.
    pub(crate) output_image: Option<Arc<ImageBuf>>,
    /// Device result from the same accepted transaction; Save and viewer reuse it.
    pub(crate) output_retained: Option<spektrafilm_backend::DeviceImage>,
    /// Reduced bins accepted atomically with the device output, never a host raster.
    pub(crate) output_histogram: Option<histogram::HistogramData>,
    pub(crate) output_metadata: Option<ImageMetadata>,
    pub(crate) source_render: Option<SourceRender>,
    pub(crate) diagnostics: diagnostics::Diagnostics,
    pub(crate) viewer: display::Viewer,
    pub(crate) settings_open: bool,
    pub(crate) only_modified: bool,
    pub(crate) pending_stage: Option<controls::Stage>,
    pub(crate) settings_return_focus: Option<egui::Id>,
    pub(crate) export_return_focus: Option<egui::Id>,
    pub(crate) output_color_space: String,
    pub(crate) output_cctf_encoding: bool,
    pub(crate) status: crate::app::Message,
    pub(crate) last_state_normalization: Option<crate::app::Message>,
    pub(crate) last_export_error: Option<crate::app::Message>,
    /// Controls changed since the latest submitted render snapshot.
    pub(crate) dirty: bool,
    pub(crate) dirty_since: Option<Instant>,
    /// macOS-only: tag the wgpu CAMetalLayer's `colorspace` as sRGB on
    /// the first `update()` tick (it's not yet wired up at
    /// `App::new` time). Set to `true` once the call succeeds so we
    /// don't retry every frame.
    #[cfg(target_os = "macos")]
    pub(crate) metal_colorspace_tagged: bool,
    /// Latest submitted render, including a request queued behind active work.
    pub(crate) render_job: Option<RenderJob>,
    pub(crate) render_worker: RenderWorker,
    pub(crate) input_worker: crate::input_worker::InputWorker,
    pub(crate) input_job: Option<crate::input_worker::InputJob>,
    pub(crate) input_view_job: Option<crate::input_worker::InputJob>,
    pub(crate) input_command_job: Option<crate::input_worker::InputJob>,
    /// In-flight export job. `Some` while the immutable snapshot is rendering;
    /// the `update()` loop polls the receiver each frame and
    /// surfaces success/failure in `status` when the worker thread
    /// completes. Joined eagerly to release the thread.
    pub(crate) export_job: Option<ExportJob>,
    /// Captured retained Saves waiting nonblockingly for their diagnostic begin.
    pub(crate) pending_saves: std::collections::VecDeque<super::export::PendingSave>,
    /// Transient control-search filter. Not persisted.
    pub(crate) filter: String,
    /// Restore-factory-default confirmation is open. Transient, never
    /// persisted: it exists only between the trigger click and the choice.
    pub(crate) factory_reset_confirm: bool,
    /// Labels the control sections rendered last, keyed by section name. The
    /// control search hides a section whose cache was recorded by a filtered
    /// render and does not match.
    pub(crate) section_labels:
        std::collections::HashMap<String, crate::controls::SectionLabelCache>,
    /// Which pages and sections hold a control matching `filter`, from the
    /// last indexed render. Transient, rebuilt when the filter, the parameter
    /// revision or the scan medium changes.
    pub(crate) search_index: crate::controls::SearchIndex,
    /// Section the panel reveals next, set when a search result is selected.
    /// Consumed by that section's header, so the panel scrolls to it and the
    /// state does not outlive the frame it was set in.
    pub(crate) pending_scroll: Option<String>,
    /// In-flight Convert controller action. The epoch drops results made stale
    /// by later parameter edits or a newer action.
    pub(crate) calibration_job: Option<CalibrationJob>,
    pub(crate) calibration_epoch: u64,
    pub(crate) parameter_revision: u64,
    pub(crate) histogram: histogram::HistogramStrip,
}

/// A fully validated candidate; source preparation must succeed before installation.
struct StateApplication {
    state: state::GuiState,
    params: RuntimeParams,
    film: profile::Profile,
    print: profile::Profile,
    film_medium: controls::ScanMedium,
    normalization: Option<crate::app::Message>,
}

/// Fresh persistent folder for one GUI run's automatic diagnostics, created
/// under the shared `<config directory>/logs/diagnostics` root.
///
/// Each run keeps its own `session-*` folder: `keep()` preserves it and its
/// reports past shutdown, and no run ever reuses an earlier folder. A creation
/// failure is a status error only; it never falls back to another directory.
fn default_publication_directory(config_directory: &Path) -> std::result::Result<PathBuf, String> {
    let root = config_directory.join("logs").join("diagnostics");
    let session = std::fs::create_dir_all(&root)
        .and_then(|()| {
            tempfile::Builder::new()
                .prefix("session-")
                .tempdir_in(&root)
        })
        .map_err(|error| {
            format!(
                "automatic report directory under {} cannot be created: {error}",
                root.display()
            )
        })?;
    Ok(session.keep())
}

impl App {
    pub(crate) fn new(
        backend: Arc<dyn ComputeBackend>,
        initial_image: Option<PathBuf>,
        initial_state: Option<PathBuf>,
        diagnostics_dir: Option<PathBuf>,
    ) -> Self {
        Self::new_with_browser_path(
            backend,
            initial_image,
            initial_state,
            diagnostics_dir,
            state::config_dir().join("preset_browser_state.json"),
            pick_data_dir(),
        )
    }

    pub(crate) fn new_with_browser_path(
        backend: Arc<dyn ComputeBackend>,
        initial_image: Option<PathBuf>,
        initial_state: Option<PathBuf>,
        diagnostics_dir: Option<PathBuf>,
        browser_state_path: PathBuf,
        data_dir: PathBuf,
    ) -> Self {
        let explicit_state = initial_state.is_some();
        // Startup state, remembered dialog directories and the preset library
        // share one session directory; the browser state names it.
        let config_directory = browser_state_path
            .parent()
            .filter(|directory| !directory.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let (language_preference, language_preference_error) =
            i18n::load_preference(&config_directory);
        let resolved_locale = language_preference.resolve();
        let (films, papers) = scan_profiles(&data_dir);
        let mut startup_error = None;
        let gui_state = match initial_state {
            Some(path) => state::GuiState::load(&path),
            None => state::startup(&config_directory),
        }
        .and_then(|state| {
            // A startup state the runtime cannot accept (a removed workflow
            // route, an unsupported combination) must fall back to the factory
            // state with a visible reason instead of failing at construction.
            let params = state.runtime_params()?;
            let film = profile::load_profile_by_name(&data_dir, state.film())?;
            spektrafilm_core::validate_scan_output(&params, &film).map_err(anyhow::Error::msg)?;
            Ok(state)
        })
        .unwrap_or_else(|error| {
            startup_error = Some(crate::app::Message::error(&error));
            state::GuiState::factory()
        });
        let film_name = gui_state.film().to_owned();
        let print_name = gui_state.paper().to_owned();
        let film_profile = profile::load_profile_by_name(&data_dir, &film_name).ok();
        let film_reference_illuminant = film_profile
            .as_ref()
            .map(|profile| profile.info.reference_illuminant.clone())
            .unwrap_or_else(|| "D65".to_owned());
        let film_medium = film_profile
            .as_ref()
            .map(scan_medium)
            .unwrap_or(controls::ScanMedium::None);
        let mut params = gui_state
            .runtime_params()
            .expect("validated GUI factory state");
        normalize_scanner_selections(&mut params, film_medium);
        let film_dev_times = film_profile
            .as_ref()
            .map(|f| f.data.development_time.clone())
            .unwrap_or_default();
        let print_dev_times = profile_dev_times(&data_dir, &print_name);
        let export_options = ExportOptions::from_state(&gui_state.sections);
        let mut presets = presets::Browser::new(
            &gui_state.sections,
            spektrafilm_core::presets::PresetLibrary::new(config_directory.join("presets")),
        );
        match presets::BrowserState::restore(
            &gui_state.sections,
            explicit_state,
            &browser_state_path,
        ) {
            Ok(browser_state) => presets.state = browser_state,
            Err(error) => {
                let message = crate::app::Message::new("app.browser.load_failed")
                    .message_arg("detail", crate::app::Message::error(&error));
                startup_error = Some(match startup_error {
                    Some(previous) => crate::app::Message::new("operations.error_chain")
                        .message_arg("context", previous)
                        .message_arg("detail", message),
                    None => message,
                });
            }
        }

        let viewer_backend = Arc::clone(&backend);
        let mut app = Self {
            backend,
            data_dir,
            films,
            papers,
            browser_state_path,
            config_directory,
            language_preference,
            resolved_locale,
            language_preference_error,
            observed_browser_state: presets.state.clone(),
            browser_state_changed_at: explicit_state.then(Instant::now),
            film_reference_illuminant,
            film_name,
            film_medium,
            presets,
            chooser: chooser::Chooser::default(),
            print_name,
            film_dev_times,
            print_dev_times,
            params,
            gui_state,
            full_scan_requested: false,
            scan_for_print_snapshot: None,
            operator_compare: false,
            operator_comparison: None,
            adjustment_history: super::history::AdjustmentHistory::default(),
            photograph: crate::photograph::PhotographWork::default(),
            retained_kind: None,
            stale_save_confirmation: false,
            export_queue: super::export_queue::ExportQueue::default(),
            image_path: None,
            input_rotation: 0,
            oriented_input: None,
            image: None,
            raw_lens_info: None,
            raw_applied_settings: None,
            source_metadata: None,
            export_options,
            export_dialog: ExportDialog::default(),
            input_epoch: 0,
            output_image: None,
            output_retained: None,
            output_histogram: None,
            output_metadata: None,
            source_render: None,
            diagnostics: diagnostics::Diagnostics::default(),
            viewer: display::Viewer::with_backend(viewer_backend),
            settings_open: false,
            only_modified: false,
            pending_stage: None,
            settings_return_focus: None,
            export_return_focus: None,
            output_color_space: "sRGB".into(),
            output_cctf_encoding: true,
            dirty_since: None,
            render_job: None,
            render_worker: RenderWorker::new(),
            input_worker: crate::input_worker::InputWorker::new(),
            input_job: None,
            input_view_job: None,
            input_command_job: None,
            export_job: None,
            pending_saves: std::collections::VecDeque::new(),
            status: startup_error
                .map(|error| {
                    crate::app::Message::new("operations.startup_error")
                        .message_arg("detail", error)
                })
                .unwrap_or_else(|| crate::app::Message::new("operations.load_to_start")),
            last_export_error: None,
            dirty: false,
            #[cfg(target_os = "macos")]
            metal_colorspace_tagged: false,
            calibration_job: None,
            calibration_epoch: 0,
            parameter_revision: 0,
            histogram: histogram::HistogramStrip::new(),
            filter: String::new(),
            factory_reset_confirm: false,
            section_labels: std::collections::HashMap::new(),
            last_state_normalization: None,
            search_index: crate::controls::SearchIndex::default(),
            pending_scroll: None,
        };
        app.viewer.settings =
            display::DisplaySettings::from_json(&app.gui_state.sections["display"]);
        // Every GUI session collects a Summary from startup on, configured
        // before the first input loads so the startup preview is covered; the
        // panel can still change the mode. Reports publish where an explicit
        // request names, or into a fresh per-run session folder otherwise.
        app.diagnostics.mode = CollectionMode::Summary;
        match diagnostics_dir {
            Some(directory) => app.diagnostics.publish_completed_to(directory),
            None => match default_publication_directory(&app.config_directory) {
                Ok(directory) => app.diagnostics.publish_completed_to(directory),
                // Publication is expendable: collection and the image session
                // continue, and the failure surfaces as the status error.
                Err(error) => app.diagnostics.record_publication_error(error),
            },
        }
        if let Some(p) = initial_image {
            app.load_image_from_path(&p);
        }
        app.viewer
            .restore_state(&app.gui_state.sections["rust"]["viewer"]);
        app.histogram
            .restore_state(&app.gui_state.sections["rust"]["histogram"]);
        app.reset_adjustment_history();
        app
    }

    pub(crate) fn bind_language(&self, ctx: &egui::Context) {
        self.resolved_locale.install(ctx);
    }

    /// Relabel immediately, even if publishing the independent preference fails.
    pub(crate) fn select_language(
        &mut self,
        ctx: &egui::Context,
        preference: i18n::LanguagePreference,
    ) -> std::result::Result<(), i18n::PreferenceError> {
        self.language_preference = preference;
        self.resolved_locale = preference.resolve();
        self.bind_language(ctx);
        let saved = i18n::save_preference(&self.config_directory, preference);
        self.language_preference_error = saved.as_ref().err().cloned();
        ctx.request_repaint();
        saved
    }

    pub(crate) fn language_selector(&mut self, ui: &mut egui::Ui) {
        let mut selected = self.language_preference;
        let mut chosen = None;
        let response = egui::ComboBox::from_id_salt("application.language_preference")
            .selected_text(selected.label())
            .show_ui(ui, |ui| {
                for preference in i18n::LanguagePreference::ALL {
                    if ui
                        .selectable_value(&mut selected, preference, preference.label())
                        .clicked()
                    {
                        chosen = Some(preference);
                    }
                }
            })
            .response
            .on_hover_text(i18n::text(ui.ctx(), "app.language.help"));
        crate::numeric::register_language_selector(ui.ctx(), &response);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::ComboBox,
                ui.is_enabled(),
                "语言 / Language",
            )
        });
        controls::locate_row(ui, "preference.interface_language", &response);
        if let Some(preference) = chosen {
            // A repeated System choice re-resolves the OS preferences; a
            // repeated explicit choice also retries failed publication.
            let _ = self.select_language(ui.ctx(), preference);
        }
        if let Some(error) = &self.language_preference_error {
            ui.colored_label(ui.visuals().error_fg_color, error.localized(ui.ctx()));
        }
    }

    pub(crate) fn current_state(&self) -> Result<state::GuiState> {
        let mut extras = self.gui_state.sections.clone();
        let display = self.viewer.settings.to_json();
        for key in [
            "use_display_transform",
            "gray_18_canvas",
            "white_padding",
            "output_interpolation",
        ] {
            extras["display"][key] = display[key].clone();
        }
        if !extras["rust"].is_object() {
            extras["rust"] = serde_json::json!({"version":1});
        }
        extras["rust"]["viewer"] = self.viewer.persistent_state();
        extras["rust"]["histogram"] = self.histogram.persistent_state();
        self.export_options.write_state(&mut extras);
        extras["rust"]["presets"] = serde_json::to_value(&self.presets.state)?;
        state::GuiState::from_runtime(&self.params, &self.film_name, &self.print_name, &extras)
    }

    pub(crate) fn mark_parameters_changed(&mut self) {
        self.parameter_revision = self.parameter_revision.wrapping_add(1);
        self.dirty = true;
    }

    pub(crate) fn mark_parameters_changed_invalidating_calibration(&mut self) {
        self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
        self.mark_parameters_changed();
    }

    fn validate_state(&self, state: state::GuiState) -> Result<StateApplication> {
        let mut params = state.runtime_params()?;
        let film = profile::load_profile_by_name(&self.data_dir, state.film())?;
        let print = profile::load_profile_by_name(&self.data_dir, state.paper())?;
        if !film.is_film() || !film.is_filming() {
            return Err(crate::app::Message::new("app.state.invalid_film_profile")
                .arg("profile", state.film())
                .into());
        }
        if !print.is_print_stock() {
            return Err(crate::app::Message::new("app.state.invalid_print_profile")
                .arg("profile", state.paper())
                .into());
        }
        spektrafilm_core::validate_scan_output(&params, &film).map_err(anyhow::Error::msg)?;
        if self.image_path.as_deref().is_some_and(image_io::is_raw) {
            params.io.input_color_space = "ACES2065-1".into();
            params.io.input_cctf_decoding = false;
        }
        let film_medium = scan_medium(&film);
        let before_scanner = params.scanner.clone();
        normalize_scanner_selections(&mut params, film_medium);
        let normalization = App::scanner_normalization_detail(
            &before_scanner,
            &params.scanner,
            film_medium,
            &params.workflow.route,
        );
        Ok(StateApplication {
            state,
            params,
            film,
            print,
            film_medium,
            normalization,
        })
    }

    /// Apply state synchronously only when there is no source to prepare.
    pub(crate) fn apply_state(&mut self, state: state::GuiState) -> Result<()> {
        let candidate = self.validate_state(state)?;
        anyhow::ensure!(
            !self.input_transition_pending(),
            crate::app::Message::new("surfaces.status.preparing_input")
        );
        anyhow::ensure!(
            self.image_path.is_none() && self.image.is_none(),
            "loaded-source state application requires asynchronous preparation"
        );
        self.install_state(candidate, None);
        Ok(())
    }

    /// Validate now, then install the candidate only after source and view preparation.
    /// The completion callback runs on the UI thread after a successful commit.
    pub(crate) fn apply_state_with_source(
        &mut self,
        state: state::GuiState,
        on_success: impl FnOnce(&mut Self) + 'static,
    ) -> Result<()> {
        if self.image_path.is_none() && self.image.is_none() {
            self.apply_state(state)?;
            on_success(self);
            return Ok(());
        }
        let candidate = self.validate_state(state)?;
        anyhow::ensure!(
            !self.input_transition_pending(),
            crate::app::Message::new("surfaces.status.preparing_input")
        );
        let source = if let Some(path) = self
            .image_path
            .as_ref()
            .filter(|path| image_io::is_raw(path))
        {
            Some(crate::input_worker::InputSource::File {
                path: path.clone(),
                raw: candidate.state.sections["load_raw"].clone(),
            })
        } else {
            self.image
                .as_ref()
                .cloned()
                .map(crate::input_worker::InputSource::Existing)
        };
        let source =
            source.ok_or_else(|| crate::app::Message::new("photo.error.source_required"))?;
        let spec = crate::input_worker::InputSpec {
            source,
            rotation: 0,
            space: candidate.params.io.input_color_space.clone(),
            decode: candidate.params.io.input_cctf_decoding,
            max_size: Some(candidate.params.settings.preview_max_size),
        };
        self.queue_input_command(vec![spec], None, move |app, mut prepared| {
            app.install_state(candidate, Some(prepared.remove(0)));
            on_success(app);
            Ok(())
        })
    }

    fn install_state(
        &mut self,
        candidate: StateApplication,
        loaded: Option<crate::input_worker::PreparedInput>,
    ) {
        let StateApplication {
            state,
            params,
            film,
            print,
            film_medium,
            normalization,
        } = candidate;
        self.last_state_normalization = normalization;
        self.film_name = state.film().to_owned();
        self.print_name = state.paper().to_owned();
        self.film_reference_illuminant = film.info.reference_illuminant;
        self.film_medium = film_medium;
        self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
        self.parameter_revision = self.parameter_revision.wrapping_add(1);
        self.film_dev_times = film.data.development_time;
        self.print_dev_times = print.data.development_time;
        self.viewer.settings = display::DisplaySettings::from_json(&state.sections["display"]);
        self.params = params;
        self.export_options = ExportOptions::from_state(&state.sections);
        self.export_dialog = ExportDialog::default();
        self.presets = presets::Browser::new(
            &state.sections,
            spektrafilm_core::presets::PresetLibrary::new(
                self.browser_state_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("presets"),
            ),
        );
        self.browser_state_changed_at = Some(Instant::now());
        self.gui_state = state;
        self.scan_for_print_snapshot = None;
        self.dirty = true;
        self.dirty_since = Some(Instant::now());

        self.full_scan_requested = false;
        self.render_worker.cancel_active();
        if let Some(prepared) = loaded {
            let view = prepared
                .view
                .expect("state application prepared its input view");
            self.image = Some(prepared.source);
            if prepared.digest.is_some() {
                self.source_metadata = prepared.metadata;
                self.raw_lens_info = prepared.lens_info;
                self.photograph.lens_model_identity = prepared.lens_model_identity;
                self.photograph.source_digest = prepared.digest;
            }
            self.raw_applied_settings = self
                .image_path
                .as_ref()
                .filter(|path| image_io::is_raw(path))
                .map(|_| self.gui_state.sections["load_raw"].clone());
            self.input_rotation = 0;
            self.input_epoch = self.input_epoch.wrapping_add(1);
            self.output_image = None;
            self.output_retained = None;
            self.output_histogram = None;
            self.viewer.set_retained_output(None);
            self.output_metadata = None;
            self.source_render = None;
            self.publish_input_viewer(view.oriented, view.raster);
        }
        self.viewer
            .restore_state(&self.gui_state.sections["rust"]["viewer"]);
        self.histogram
            .restore_state(&self.gui_state.sections["rust"]["histogram"]);
        if let Some(ctx) = &self.photograph.context {
            crate::numeric::NumericDrafts::take(ctx);
        }
        self.reset_adjustment_history();
        self.stale_save_confirmation = false;
        self.retained_kind = None;
        self.photograph.baseline = self.capture_photograph_settings().ok();
        self.photograph.record_path = None;
    }

    pub(crate) fn persist_browser_state(&mut self, closing: bool) {
        if self.presets.state != self.observed_browser_state {
            self.observed_browser_state = self.presets.state.clone();
            self.browser_state_changed_at = Some(Instant::now());
        }
        if self
            .browser_state_changed_at
            .is_some_and(|changed| closing || changed.elapsed() >= Duration::from_millis(300))
        {
            self.browser_state_changed_at = None;
            if let Err(error) = self.presets.state.save(&self.browser_state_path) {
                self.status = crate::app::Message::new("operations.browser_save_error")
                    .message_arg("detail", crate::app::Message::error(&error));
                if closing {
                    eprintln!("{}", self.status);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Automatic session diagnostics at the real GUI startup boundary: the
    //! production default records every session without any diagnostics
    //! request, into a folder no later run disturbs.
    use super::*;
    use spektrafilm_backend::cpu_backend::CpuBackend;
    use spektrafilm_core::telemetry::{OperationKind, Outcome, Report};

    /// Sorted direct entries of `directory`, files and folders alike.
    fn sorted_entries(directory: &Path) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        paths
    }

    /// Every published `.json` file parses as a startup preview report,
    /// collected under the automatic Summary default. Sidecars are independent.
    fn assert_preview_reports(directory: &Path) -> Vec<PathBuf> {
        let paths: Vec<_> = sorted_entries(directory)
            .into_iter()
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        let mut succeeded = false;
        for path in &paths {
            let report = Report::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
            assert_eq!(report.operation.kind, OperationKind::Preview);
            assert_eq!(
                report.configuration.collection_mode_requested,
                CollectionMode::Summary
            );
            if report.operation.outcome == Outcome::Succeeded {
                assert!(
                    report.phases.iter().any(|phase| {
                        phase.name == "simulation"
                            && phase.duration.is_finite()
                            && phase.duration > 0.0
                    }),
                    "startup reports include measured work"
                );
                succeeded = true;
            }
        }
        assert!(succeeded, "the startup preview succeeded");
        paths
    }

    /// One real GUI startup with production defaults; returns the automatic
    /// publication directory and the file set shutdown left in it.
    fn completed_startup(config_root: &Path, work: &Path) -> (PathBuf, Vec<PathBuf>) {
        let mut harness = crate::headless::default_startup(config_root, work);
        assert_eq!(harness.state().diagnostics.mode, CollectionMode::Summary);
        let directory = harness
            .state()
            .diagnostics
            .publication_directory()
            .expect("every GUI session records automatically")
            .to_path_buf();
        let reports = assert_preview_reports(&directory);
        eframe::App::on_exit(harness.state_mut(), None);
        // Shutdown keeps the run's folder and every report in it.
        assert!(reports.iter().all(|path| path.is_file()));
        assert!(directory.is_dir());
        let kept = sorted_entries(&directory);
        (directory, kept)
    }

    #[test]
    fn every_startup_publishes_summary_reports_to_a_fresh_persistent_session_directory() {
        let root = tempfile::tempdir().unwrap();
        let config_root = root.path().join("config");
        let logs_root = config_root.join("logs").join("diagnostics");

        // Two fresh runs sharing one config directory, as two launches of the
        // GUI would.
        let work_one = root.path().join("work-1");
        let (first_directory, first_kept) = completed_startup(&config_root, &work_one);
        let first_contents: Vec<_> = first_kept
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect();
        let work_two = root.path().join("work-2");
        let (second_directory, _) = completed_startup(&config_root, &work_two);
        assert_ne!(
            first_directory, second_directory,
            "a restart never reuses an earlier session directory"
        );
        for directory in [&first_directory, &second_directory] {
            assert_eq!(directory.parent(), Some(logs_root.as_path()));
            assert!(
                directory
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("session-")
            );
        }
        // The later run left the earlier run's folder and files untouched.
        assert_eq!(sorted_entries(&first_directory), first_kept);
        for (path, contents) in first_kept.iter().zip(first_contents) {
            assert_eq!(std::fs::read(path).unwrap(), contents);
        }
        let mut expected = vec![first_directory, second_directory];
        expected.sort();
        assert_eq!(
            sorted_entries(&logs_root),
            expected,
            "exactly one persistent folder per run"
        );
    }

    #[test]
    fn blocked_default_root_surfaces_the_error_and_keeps_the_session_running() {
        let root = tempfile::tempdir().unwrap();
        let config_root = root.path().join("config");
        // A regular file where the diagnostics root must be: no session
        // directory can be created, anywhere.
        let logs = config_root.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("diagnostics"), b"blocked").unwrap();
        let work = root.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let (image_path, _) = crate::headless::deterministic_inputs(&work);

        let mut app = App::new_with_browser_path(
            Arc::new(CpuBackend),
            Some(image_path),
            None,
            None,
            config_root.join("preset_browser_state.json"),
            crate::headless::data_dir(),
        );
        // Startup survives the failed folder: the image loads through the
        // real input worker and collection stays on.
        crate::headless::await_app_input(&mut app, &egui::Context::default());
        assert!(app.image.is_some(), "the image session survives");
        assert_eq!(app.diagnostics.mode, CollectionMode::Summary);
        assert!(
            app.diagnostics.publication_directory().is_none(),
            "publication never falls back to another directory"
        );
        let error = app
            .diagnostics
            .take_publication_error()
            .expect("the creation failure surfaces");
        let blocked = logs.join("diagnostics").display().to_string();
        assert!(
            error.contains(blocked.as_str()),
            "the error names the blocked root: {error}"
        );
        assert!(logs.join("diagnostics").is_file());
    }
}
