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
    pub(crate) force_preview: bool,
    pub(crate) full_scan_requested: bool,
    /// Original print-glare switch captured while Scan-for-print is active.
    /// This transient workflow state is deliberately excluded from persistence.
    pub(crate) scan_for_print_snapshot: Option<bool>,
    pub(crate) image_path: Option<PathBuf>,
    pub(crate) image: Option<Arc<ImageBuf>>,
    pub(crate) raw_lens_info: Option<String>,
    /// Quarter turns applied to the loaded source, used for the rotation status.
    pub(crate) input_rotation: i32,
    pub(crate) source_metadata: Option<ImageMetadata>,
    pub(crate) export_options: ExportOptions,
    pub(crate) export_dialog: ExportDialog,
    pub(crate) input_epoch: u64,
    /// Latest accepted floating output, shared with background viewing workers.
    pub(crate) output_image: Option<Arc<ImageBuf>>,
    pub(crate) output_metadata: Option<ImageMetadata>,
    pub(crate) source_render: Option<SourceRender>,
    pub(crate) diagnostics: diagnostics::Diagnostics,
    pub(crate) viewer: display::Viewer,
    pub(crate) gui_tab: ui::GuiTab,
    pub(crate) output_color_space: String,
    pub(crate) output_cctf_encoding: bool,
    pub(crate) status: String,
    pub(crate) last_state_normalization: Option<String>,
    pub(crate) last_export_error: Option<String>,
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
    /// Innermost work unit of the in-flight render, shared with the render
    /// worker thread.
    pub(crate) render_progress: Arc<progress::RenderProgress>,
    /// In-flight export job. `Some` while the immutable snapshot is rendering;
    /// the `update()` loop polls the receiver each frame and
    /// surfaces success/failure in `status` when the worker thread
    /// completes. Joined eagerly to release the thread.
    pub(crate) export_job: Option<ExportJob>,
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
    pub(crate) compare_active: bool,
    pub(crate) compare_split: f32,
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
            startup_error = Some(format!("Startup state error: {error:#}"));
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
                let message = format!("Browser state load error: {error:#}");
                startup_error = Some(match startup_error {
                    Some(previous) => format!("{previous}; {message}"),
                    None => message,
                });
            }
        }

        let render_progress = Arc::new(progress::RenderProgress::default());
        let mut app = Self {
            backend,
            data_dir,
            films,
            papers,
            browser_state_path,
            config_directory,
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
            force_preview: false,
            full_scan_requested: false,
            scan_for_print_snapshot: None,
            image_path: None,
            input_rotation: 0,
            image: None,
            raw_lens_info: None,
            source_metadata: None,
            export_options,
            export_dialog: ExportDialog::default(),
            input_epoch: 0,
            output_image: None,
            output_metadata: None,
            source_render: None,
            diagnostics: diagnostics::Diagnostics::default(),
            viewer: display::Viewer::new(),
            gui_tab: ui::GuiTab::default(),
            output_color_space: "sRGB".into(),
            output_cctf_encoding: true,
            dirty_since: None,
            render_job: None,
            render_worker: RenderWorker::new(render_progress.clone()),
            render_progress,
            export_job: None,
            status: startup_error.unwrap_or_else(|| String::from("Load an image to start.")),
            last_export_error: None,
            dirty: false,
            #[cfg(target_os = "macos")]
            metal_colorspace_tagged: false,
            calibration_job: None,
            calibration_epoch: 0,
            parameter_revision: 0,
            histogram: histogram::HistogramStrip::new(),
            compare_active: false,
            compare_split: 0.5,
            filter: String::new(),
            factory_reset_confirm: false,
            section_labels: std::collections::HashMap::new(),
            last_state_normalization: None,
            search_index: crate::controls::SearchIndex::default(),
            pending_scroll: None,
        };
        app.viewer.settings =
            display::DisplaySettings::from_json(&app.gui_state.sections["display"]);
        if let Some(directory) = diagnostics_dir {
            // Requesting reports selects summary collection for the session, as the
            // CLI's report request does; the panel can still change the mode.
            app.diagnostics.mode = CollectionMode::Summary;
            app.diagnostics.publish_completed_to(directory);
        }
        if let Some(p) = initial_image {
            app.load_image_from_path(&p);
        }
        app.viewer
            .restore_state(&app.gui_state.sections["rust"]["viewer"]);
        app.histogram
            .restore_state(&app.gui_state.sections["rust"]["histogram"]);
        app
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

    pub(crate) fn apply_state(&mut self, state: state::GuiState) -> Result<()> {
        self.last_state_normalization = None;
        let mut params = state.runtime_params()?;
        let film = profile::load_profile_by_name(&self.data_dir, state.film())?;
        profile::load_profile_by_name(&self.data_dir, state.paper())?;
        spektrafilm_core::validate_scan_output(&params, &film).map_err(anyhow::Error::msg)?;
        let film_medium = scan_medium(&film);
        let film_reference_illuminant = film.info.reference_illuminant.clone();
        let before_scanner = params.scanner.clone();
        normalize_scanner_selections(&mut params, film_medium);
        self.last_state_normalization = App::scanner_normalization_detail(
            &before_scanner,
            &params.scanner,
            film_medium,
            &params.workflow.route,
        );
        self.film_name = state.film().to_owned();
        self.film_reference_illuminant = film_reference_illuminant;
        self.film_medium = film_medium;
        self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
        self.parameter_revision = self.parameter_revision.wrapping_add(1);
        self.film_dev_times = profile_dev_times(&self.data_dir, &self.film_name);
        self.print_dev_times = profile_dev_times(&self.data_dir, &self.print_name);
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
        self.raw_lens_info = None;
        self.dirty = true;
        self.force_preview = true;
        if let Some(path) = self.image_path.clone() {
            self.load_image_from_path(&path);
        }
        self.viewer
            .restore_state(&self.gui_state.sections["rust"]["viewer"]);
        self.histogram
            .restore_state(&self.gui_state.sections["rust"]["histogram"]);
        Ok(())
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
                self.status = format!("Browser state save error: {error:#}");
                if closing {
                    eprintln!("{}", self.status);
                }
            }
        }
    }
}
