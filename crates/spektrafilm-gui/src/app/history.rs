//! Source-local accepted adjustment transactions. Snapshots contain controls only,
//! never images, prepared profiles, worker requests, preferences, or file artifacts.
use super::state::App;
use crate::*;
use std::rc::Rc;

#[derive(Clone, PartialEq)]
struct AdjustmentState {
    params: RuntimeParams,
    film_name: String,
    print_name: String,
    film_reference_illuminant: String,
    film_medium: controls::ScanMedium,
    film_dev_times: Vec<f64>,
    print_dev_times: Vec<f64>,
    special: serde_json::Value,
    raw_settings: serde_json::Value,
    applied: Option<presets::AppliedPreset>,
    scan_for_print: Option<bool>,
    rotation: i32,
}

impl AdjustmentState {
    fn capture(app: &App) -> Self {
        let mut params = app.params.clone();
        // Session execution preferences are not photographic adjustments.
        params.settings.preview_max_size = 0;
        params.settings.preview_mode = false;
        params.settings.use_enlarger_lut = false;
        params.settings.use_scanner_lut = false;
        params.settings.lut_resolution = 0;
        params.settings.use_fast_stats = false;
        params.settings.neutral_print_filters_from_database = true;
        params.debug = Default::default();
        params.taps = Default::default();
        params.random_seed = 0;
        Self {
            params,
            film_name: app.film_name.clone(),
            print_name: app.print_name.clone(),
            film_reference_illuminant: app.film_reference_illuminant.clone(),
            film_medium: app.film_medium,
            film_dev_times: app.film_dev_times.clone(),
            print_dev_times: app.print_dev_times.clone(),
            special: app.gui_state.sections["special"].clone(),
            raw_settings: app.gui_state.sections["load_raw"].clone(),
            applied: app.presets.state.applied.clone(),
            scan_for_print: app.scan_for_print_snapshot,
            rotation: app.input_rotation,
        }
    }

    /// Borrowed comparisons keep idle UI frames allocation-free.
    fn controls_match(&self, app: &App) -> bool {
        let a = &self.params;
        let b = &app.params;
        a.camera == b.camera
            && a.enlarger == b.enlarger
            && a.scanner == b.scanner
            && a.film_render == b.film_render
            && a.print_render == b.print_render
            && a.io == b.io
            && a.workflow == b.workflow
            && a.cmyk == b.cmyk
            && a.output_mix == b.output_mix
            && a.neutral_print_filters_protected == b.neutral_print_filters_protected
            && a.settings.spectral_shape == b.settings.spectral_shape
            && a.settings.rgb_to_raw_method == b.settings.rgb_to_raw_method
            && a.settings.apply_hanatos2025_adaptation_window
                == b.settings.apply_hanatos2025_adaptation_window
            && a.settings.apply_hanatos2025_adaptation_surface
                == b.settings.apply_hanatos2025_adaptation_surface
            && a.settings.spectral_gaussian_blur == b.settings.spectral_gaussian_blur
            && a.settings.use_cat16 == b.settings.use_cat16
            && a.settings.print_exposure_ev_tracking == b.settings.print_exposure_ev_tracking
            && self.film_name == app.film_name
            && self.print_name == app.print_name
            && self.special == app.gui_state.sections["special"]
            && self.raw_settings == app.gui_state.sections["load_raw"]
            && self.scan_for_print == app.scan_for_print_snapshot
            && self.rotation == app.input_rotation
    }

    fn restore(&self, app: &mut App) {
        let settings = &app.params.settings;
        let mut params = self.params.clone();
        params.settings.preview_max_size = settings.preview_max_size;
        params.settings.preview_mode = settings.preview_mode;
        params.settings.use_enlarger_lut = settings.use_enlarger_lut;
        params.settings.use_scanner_lut = settings.use_scanner_lut;
        params.settings.lut_resolution = settings.lut_resolution;
        params.settings.use_fast_stats = settings.use_fast_stats;
        params.settings.neutral_print_filters_from_database =
            settings.neutral_print_filters_from_database;
        params.debug = app.params.debug.clone();
        params.taps = app.params.taps.clone();
        params.random_seed = app.params.random_seed;
        app.params = params;
        app.film_name.clone_from(&self.film_name);
        app.print_name.clone_from(&self.print_name);
        app.film_reference_illuminant
            .clone_from(&self.film_reference_illuminant);
        app.film_medium = self.film_medium;
        app.film_dev_times.clone_from(&self.film_dev_times);
        app.gui_state.sections["simulation"]["film_stock"] = self.film_name.clone().into();
        app.gui_state.sections["simulation"]["print_paper"] = self.print_name.clone().into();
        app.print_dev_times.clone_from(&self.print_dev_times);
        app.gui_state.sections["special"] = self.special.clone();
        app.gui_state.sections["load_raw"] = self.raw_settings.clone();
        app.presets.state.applied = self.applied.clone();
        app.scan_for_print_snapshot = self.scan_for_print;
        if app.input_rotation != self.rotation {
            let turns = (self.rotation - app.input_rotation).rem_euclid(4);
            if let Some(image) = &app.image {
                app.image = Some(Arc::new(image.rotated_quarter_turns(turns)));
                app.input_epoch = app.input_epoch.wrapping_add(1);
                app.input_rotation = self.rotation;
                app.refresh_input_viewer();
            }
        }
        app.viewer.set_input_display_source(
            &app.params.io.input_color_space,
            app.params.io.input_cctf_decoding,
        );
    }
}

struct Entry {
    before: Rc<AdjustmentState>,
    after: Rc<AdjustmentState>,
    label: String,
}

#[derive(Clone, Copy)]
struct FrameBoundary {
    revision: u64,
    calibration: u64,
    dirty: bool,
    dirty_since: Option<Instant>,
    force_preview: bool,
    full_scan: bool,
}

#[derive(Default)]
pub(crate) struct AdjustmentHistory {
    accepted: Option<Rc<AdjustmentState>>,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    gesture: Option<(Rc<AdjustmentState>, String)>,
    frame: Option<FrameBoundary>,
    label: Option<String>,
}

impl App {
    pub(crate) fn reset_adjustment_history(&mut self) {
        self.adjustment_history = AdjustmentHistory {
            accepted: Some(Rc::new(AdjustmentState::capture(self))),
            ..Default::default()
        };
    }

    pub(crate) fn begin_adjustment_frame(&mut self, ctx: &egui::Context) {
        if self.adjustment_history.accepted.is_none() {
            self.reset_adjustment_history();
        }
        // Discard signals left by an owner outside the adjustment boundary.
        let _ = numeric::take_adjustment_signals(ctx);
        let _ = controls::take_edited_paths(ctx);
        self.adjustment_history.frame = Some(FrameBoundary {
            revision: self.parameter_revision,
            calibration: self.calibration_epoch,
            dirty: self.dirty,
            dirty_since: self.dirty_since,
            force_preview: self.force_preview,
            full_scan: self.full_scan_requested,
        });
    }

    pub(crate) fn label_adjustment(&mut self, label: &str) {
        self.adjustment_history.label = Some(label.to_owned());
    }

    fn validate_adjustment(&self) -> Result<()> {
        self.current_state()?.runtime_params()?;
        if self.params.scanner.scan_output == "negative_to_positive"
            && !self
                .film_medium
                .positive_output_eligible(&self.params.workflow.route)
        {
            anyhow::bail!(
                "scanner.scan_output: negative_to_positive requires negative film and an eligible scan route"
            );
        }
        Ok(())
    }

    pub(crate) fn end_adjustment_frame(&mut self, ctx: &egui::Context) {
        let signal = numeric::take_adjustment_signals(ctx);
        let paths = controls::take_edited_paths(ctx);
        let Some(frame) = self.adjustment_history.frame.take() else {
            return;
        };
        let before = self
            .adjustment_history
            .accepted
            .as_ref()
            .expect("history initialized")
            .clone();
        let changed = !before.controls_match(self)
            || (self.parameter_revision != frame.revision
                && (before.params.render_preparation_state()
                    != self.params.render_preparation_state()
                    || before.applied != self.presets.state.applied));
        if signal.canceled {
            let initial = self
                .adjustment_history
                .gesture
                .take()
                .map(|(state, _)| state)
                .unwrap_or(before);
            let invalidated =
                self.parameter_revision != frame.revision || !initial.controls_match(self);
            initial.restore(self);
            if invalidated {
                self.mark_parameters_changed_invalidating_calibration();
                self.render_worker.cancel_active();
                self.dirty_since = Some(Instant::now());
            }
            self.adjustment_history.accepted = Some(initial);
            self.adjustment_history.label = None;
            return;
        }
        if changed && let Err(error) = self.validate_adjustment() {
            before.restore(self);
            self.parameter_revision = frame.revision;
            self.calibration_epoch = frame.calibration;
            self.dirty = frame.dirty;
            self.dirty_since = frame.dirty_since;
            self.force_preview = frame.force_preview;
            self.full_scan_requested = frame.full_scan;
            self.status = format!("Adjustment rejected: {error:#}");
            self.adjustment_history.label = None;
            return;
        }
        if !paths.is_empty() && changed {
            let mut ownership = self.params.render_preparation_state();
            for path in paths {
                match path.as_str() {
                    "enlarger.c_filter_neutral" => {
                        ownership.neutral_print_filters_protected[0] = true
                    }
                    "enlarger.m_filter_neutral" => {
                        ownership.neutral_print_filters_protected[1] = true
                    }
                    "enlarger.y_filter_neutral" => {
                        ownership.neutral_print_filters_protected[2] = true
                    }
                    _ => {}
                }
                if !ownership.explicit_section_paths.contains(&path) {
                    ownership.explicit_section_paths.push(path);
                }
            }
            self.params.restore_render_preparation_state(&ownership);
        }
        if !changed
            && !signal.started
            && !signal.active
            && !signal.completed
            && self.adjustment_history.gesture.is_none()
        {
            if before.applied != self.presets.state.applied {
                let mut updated = (*before).clone();
                updated.applied = self.presets.state.applied.clone();
                self.adjustment_history.accepted = Some(Rc::new(updated));
            }
            self.adjustment_history.label = None;
            return;
        }
        let label = self
            .adjustment_history
            .label
            .take()
            .or(signal.label)
            .unwrap_or_else(|| self.adjustment_label(&before));
        // Finish the preceding gesture before a new active field starts.
        if signal.completed
            && signal.started
            && signal.active
            && let Some((initial, previous_label)) = self.adjustment_history.gesture.take()
        {
            self.commit_adjustment(initial, before.clone(), previous_label);
        }
        if (signal.started || signal.active) && self.adjustment_history.gesture.is_none() {
            self.adjustment_history.gesture = Some((before.clone(), label.clone()));
        }
        let after = if changed {
            if self.parameter_revision == frame.revision {
                self.mark_parameters_changed_invalidating_calibration();
            }
            self.render_worker.cancel_active();
            self.dirty_since = Some(Instant::now());
            let accepted = Rc::new(AdjustmentState::capture(self));
            self.adjustment_history.accepted = Some(accepted.clone());
            accepted
        } else {
            // Library rename/save markers aren't adjustment actions. Refresh only
            // their small associated metadata, never create an Undo entry.
            if before.applied != self.presets.state.applied {
                let mut updated = (*before).clone();
                updated.applied = self.presets.state.applied.clone();
                self.adjustment_history.accepted = Some(Rc::new(updated));
            }
            before.clone()
        };
        if signal.completed || !signal.active {
            if let Some((initial, gesture_label)) = self.adjustment_history.gesture.take() {
                self.commit_adjustment(initial, after, gesture_label);
            } else if changed {
                self.commit_adjustment(before, after, label);
            }
        }
    }

    fn adjustment_label(&self, before: &AdjustmentState) -> String {
        if before.film_name != self.film_name {
            "Change film material"
        } else if before.print_name != self.print_name {
            "Change print material"
        } else if before.applied != self.presets.state.applied {
            "Apply look preset"
        } else if before.params.workflow != self.params.workflow {
            "Change workflow"
        } else if before.rotation != self.input_rotation {
            "Rotate source"
        } else if before.scan_for_print != self.scan_for_print_snapshot {
            "Scan for print"
        } else {
            "Adjust photograph"
        }
        .to_owned()
    }

    fn commit_adjustment(
        &mut self,
        before: Rc<AdjustmentState>,
        after: Rc<AdjustmentState>,
        label: String,
    ) {
        // Compare recorded endpoints, not the mutable UI (a new gesture can
        // already have started when the preceding one completes).
        if before == after {
            return;
        }
        self.adjustment_history.undo.push(Entry {
            before,
            after,
            label,
        });
        self.adjustment_history.redo.clear();
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.adjustment_history.undo.is_empty()
    }
    pub(crate) fn can_redo(&self) -> bool {
        !self.adjustment_history.redo.is_empty()
    }
    /// The next operation, without an "Undo" / "Redo" prefix.
    pub(crate) fn undo_label(&self) -> Option<&str> {
        self.adjustment_history
            .undo
            .last()
            .map(|entry| entry.label.as_str())
    }
    pub(crate) fn redo_label(&self) -> Option<&str> {
        self.adjustment_history
            .redo
            .last()
            .map(|entry| entry.label.as_str())
    }

    pub(crate) fn undo_adjustment(&mut self) -> bool {
        self.navigate_adjustment(false)
    }
    pub(crate) fn redo_adjustment(&mut self) -> bool {
        self.navigate_adjustment(true)
    }

    fn navigate_adjustment(&mut self, redo: bool) -> bool {
        if let Some((initial, _)) = self.adjustment_history.gesture.take() {
            initial.restore(self);
            self.adjustment_history.accepted = Some(initial);
            self.mark_parameters_changed_invalidating_calibration();
            self.render_worker.cancel_active();
            self.dirty_since = Some(Instant::now());
        }
        let entry = if redo {
            self.adjustment_history.redo.pop()
        } else {
            self.adjustment_history.undo.pop()
        };
        let Some(entry) = entry else {
            return false;
        };
        let target = if redo {
            entry.after.clone()
        } else {
            entry.before.clone()
        };
        target.restore(self);
        // Recorded states were validated on acceptance; no preset/profile file
        // is consulted here, so edits to a library cannot change Redo.
        self.adjustment_history.accepted = Some(target);
        self.status = format!("{} {}", if redo { "Redid" } else { "Undid" }, entry.label);
        if redo {
            self.adjustment_history.undo.push(entry);
        } else {
            self.adjustment_history.redo.push(entry);
        }
        self.mark_parameters_changed_invalidating_calibration();
        self.render_worker.cancel_active();
        self.dirty_since = Some(Instant::now());
        self.full_scan_requested = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (App, tempfile::TempDir, egui::Context) {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        state::GuiState::factory().save(&state_path).unwrap();
        let app = App::new_with_browser_path(
            Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
            None,
            Some(state_path),
            None,
            directory.path().join("browser.json"),
            crate::headless::data_dir(),
        );
        (app, directory, egui::Context::default())
    }

    fn signals(ctx: &egui::Context, active: bool, completed: bool, canceled: bool) {
        numeric::signal_for_test(
            ctx,
            "Camera exposure",
            numeric::AdjustmentSignals {
                started: active,
                active,
                completed,
                canceled,
                label: Some("Camera exposure".into()),
            },
        );
    }

    fn exposure(app: &mut App, ctx: &egui::Context, value: f32) {
        app.begin_adjustment_frame(ctx);
        app.params.camera.exposure_compensation_ev = value;
        controls::record_edited_path(ctx, "camera.exposure_compensation_ev");
        app.end_adjustment_frame(ctx);
    }

    #[test]
    fn undo_restores_explicit_ownership_and_redo_replays_the_captured_successor() {
        let (mut app, _directory, ctx) = app();
        let original = app.params.camera.exposure_compensation_ev;
        let ownership = app.params.render_preparation_state();
        exposure(&mut app, &ctx, original + 1.0);
        let accepted = app.params.clone();
        assert!(
            accepted
                .render_preparation_state()
                .explicit_section_paths
                .contains(&"camera.exposure_compensation_ev".to_owned())
        );
        let revision = app.parameter_revision;
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, original);
        assert_eq!(app.params.render_preparation_state(), ownership);
        assert!(app.parameter_revision > revision);
        assert!(app.redo_adjustment());
        assert_eq!(app.params, accepted);
        assert!(app.undo_adjustment());
        exposure(&mut app, &ctx, original + 2.0);
        assert!(!app.can_redo());
    }

    #[test]
    fn imported_acquisition_light_is_validated_and_reversible_without_scanner_edits() {
        let (mut app, _directory, ctx) = app();
        let original = app.params.film_render.convert.scan_illuminant.clone();
        let scanner = app.params.scanner.clone();
        app.begin_adjustment_frame(&ctx);
        app.params.film_render.convert.scan_illuminant = "BB4321".into();
        controls::record_edited_path(&ctx, "film_render.convert.scan_illuminant");
        app.end_adjustment_frame(&ctx);
        assert!(app.can_undo());
        assert_eq!(app.params.scanner, scanner);
        assert!(app.undo_adjustment());
        assert_eq!(app.params.film_render.convert.scan_illuminant, original);
        assert!(app.redo_adjustment());
        assert_eq!(app.params.film_render.convert.scan_illuminant, "BB4321");
        let revision = app.parameter_revision;
        app.begin_adjustment_frame(&ctx);
        app.params.film_render.convert.scan_illuminant = "BB1000".into();
        controls::record_edited_path(&ctx, "film_render.convert.scan_illuminant");
        app.end_adjustment_frame(&ctx);
        assert_eq!(app.params.film_render.convert.scan_illuminant, "BB4321");
        assert_eq!(app.params.scanner, scanner);
        assert_eq!(app.parameter_revision, revision);
        assert!(app.status.starts_with("Adjustment rejected:"));
    }

    #[test]
    fn a_drag_commits_one_boundary_and_escape_restores_values_and_provenance() {
        let (mut app, _directory, ctx) = app();
        let original = app.params.clone();
        for delta in [0.25, 0.5, 1.0] {
            app.begin_adjustment_frame(&ctx);
            app.params.camera.exposure_compensation_ev =
                original.camera.exposure_compensation_ev + delta;
            controls::record_edited_path(&ctx, "camera.exposure_compensation_ev");
            signals(&ctx, true, false, false);
            app.end_adjustment_frame(&ctx);
            assert!(!app.can_undo());
        }
        app.begin_adjustment_frame(&ctx);
        signals(&ctx, false, true, false);
        app.end_adjustment_frame(&ctx);
        assert_eq!(app.adjustment_history.undo.len(), 1);
        assert!(app.undo_adjustment());
        assert_eq!(app.params, original);
        let redo = app.redo_label().map(str::to_owned);
        app.begin_adjustment_frame(&ctx);
        app.params.camera.exposure_compensation_ev += 0.75;
        controls::record_edited_path(&ctx, "camera.exposure_compensation_ev");
        signals(&ctx, true, false, false);
        app.end_adjustment_frame(&ctx);
        let revision = app.parameter_revision;
        app.begin_adjustment_frame(&ctx);
        signals(&ctx, false, false, true);
        app.end_adjustment_frame(&ctx);
        assert_eq!(app.params, original);
        assert!(app.parameter_revision > revision);
        assert!(!app.can_undo());
        assert_eq!(app.redo_label(), redo.as_deref());
    }

    #[test]
    fn history_restores_material_metadata_and_scan_for_print_without_preferences() {
        let (mut app, _directory, ctx) = app();
        app.params.print_render.glare.active = true;
        app.reset_adjustment_history();
        let film = app.film_name.clone();
        let times = app.film_dev_times.clone();
        app.begin_adjustment_frame(&ctx);
        app.toggle_scan_for_print();
        app.end_adjustment_frame(&ctx);
        app.begin_adjustment_frame(&ctx);
        app.select_film_profile("kodak_ektachrome_100".into());
        app.end_adjustment_frame(&ctx);
        app.params.settings.preview_max_size = 1024;
        app.gui_state.sections["simulation"]["auto_preview"] = false.into();
        app.gui_state.set_display_profile(Some("user-display.icc"));
        assert!(app.undo_adjustment());
        assert_eq!(app.film_name, film);
        assert_eq!(app.film_dev_times, times);
        assert_eq!(app.scan_for_print_snapshot, Some(true));
        assert!(!app.params.print_render.glare.active);
        assert_eq!(app.params.settings.preview_max_size, 1024);
        assert!(!app.gui_state.auto_preview());
        assert_eq!(app.gui_state.display_profile(), Some("user-display.icc"));
        assert!(app.undo_adjustment());
        assert_eq!(app.scan_for_print_snapshot, None);
        assert!(app.params.print_render.glare.active);
    }

    #[test]
    fn invalid_combination_rejects_values_metadata_provenance_and_revision() {
        let (mut app, _directory, ctx) = app();
        exposure(&mut app, &ctx, 1.0);
        let accepted = app.params.clone();
        let revision = app.parameter_revision;
        let calibration = app.calibration_epoch;
        let entries = app.adjustment_history.undo.len();
        app.begin_adjustment_frame(&ctx);
        app.params.scanner.scan_output = "negative_to_positive".into();
        app.params.workflow.route = "input".into();
        app.film_name = "unaccepted".into();
        app.scan_for_print_snapshot = Some(true);
        app.mark_parameters_changed_invalidating_calibration();
        app.end_adjustment_frame(&ctx);
        assert_eq!(app.params, accepted);
        assert_ne!(app.film_name, "unaccepted");
        assert_eq!(app.scan_for_print_snapshot, None);
        assert_eq!(app.parameter_revision, revision);
        assert_eq!(app.calibration_epoch, calibration);
        assert_eq!(app.adjustment_history.undo.len(), entries);
        assert!(app.status.starts_with("Adjustment rejected:"));
    }

    #[test]
    fn complete_state_preflight_failure_keeps_source_and_history_but_success_clears_it() {
        let (mut app, directory, ctx) = app();
        exposure(&mut app, &ctx, 1.0);
        app.image = Some(Arc::new(ImageBuf::new(3, 2)));
        let source = app.image.as_ref().unwrap().clone();
        app.image_path = Some(directory.path().join("missing.png"));
        let params = app.params.clone();
        let revision = app.parameter_revision;
        let mut state = app.current_state().unwrap();
        state.sections["camera"]["exposure_compensation_ev"] = 2.0.into();
        assert!(app.apply_state(state).is_err());
        assert!(Arc::ptr_eq(&source, app.image.as_ref().unwrap()));
        assert_eq!(app.params, params);
        assert_eq!(app.parameter_revision, revision);
        assert!(app.can_undo());
        app.image_path = None;
        app.apply_state(app.current_state().unwrap()).unwrap();
        assert!(!app.can_undo());
        assert!(!app.can_redo());
    }

    #[test]
    fn look_marker_only_application_is_reversible_but_library_rename_is_not_an_edit() {
        let (mut app, _directory, ctx) = app();
        app.begin_adjustment_frame(&ctx);
        app.presets.state.applied = Some(presets::AppliedPreset {
            group: spektrafilm_core::presets::PresetGroup::BuiltIn,
            name: "Accepted look".into(),
        });
        app.mark_parameters_changed_invalidating_calibration();
        app.end_adjustment_frame(&ctx);
        assert!(app.can_undo());
        assert!(app.undo_adjustment());
        assert!(app.presets.state.applied.is_none());
        assert!(app.redo_adjustment());
        assert_eq!(
            app.presets.state.applied.as_ref().unwrap().name,
            "Accepted look"
        );
        let count = app.adjustment_history.undo.len();
        app.begin_adjustment_frame(&ctx);
        app.presets.state.applied.as_mut().unwrap().name = "Renamed library entry".into();
        app.end_adjustment_frame(&ctx);
        assert_eq!(app.adjustment_history.undo.len(), count);
        assert_eq!(
            app.presets.state.applied.as_ref().unwrap().name,
            "Renamed library entry"
        );
    }

    #[test]
    fn undo_uses_new_revisions_and_marks_the_retained_frame_stale() {
        let (mut app, _directory, ctx) = app();
        exposure(&mut app, &ctx, 1.0);
        let prior_revision = app.parameter_revision;
        let prior_epoch = app.input_epoch;
        let old_job = RenderJob {
            input_epoch: prior_epoch,
            parameter_revision: prior_revision,
            request_id: 1,
            kind: RenderKind::Preview,
            animate: false,
            backend_name: "CPU".into(),
            collection_mode: CollectionMode::Off,
        };
        app.output_image = Some(Arc::new(ImageBuf::new(2, 1)));
        app.source_render = Some(SourceRender::new(
            1,
            Some(prior_epoch),
            Some(prior_revision),
            2,
            1,
            false,
            diagnostics::color_space("sRGB"),
            true,
        ));
        assert!(!app.accepted_frame_is_stale());
        assert!(app.undo_adjustment());
        assert!(app.accepted_frame_is_stale());
        assert!(!old_job.matches(Some(&old_job), app.input_epoch, app.parameter_revision));
        assert!(app.output_image.is_some());
    }

    #[test]
    fn rotation_history_stores_only_orientation_and_restores_pixels_by_delta() {
        let (mut app, _directory, ctx) = app();
        let image = ImageBuf::from_data(2, 1, vec![0.1, 0.2, 0.3, 0.7, 0.8, 0.9]);
        app.image = Some(Arc::new(image.clone()));
        app.reset_adjustment_history();
        app.begin_adjustment_frame(&ctx);
        app.image = Some(Arc::new(image.rotated_quarter_turns(1)));
        app.input_rotation = 1;
        app.input_epoch += 1;
        app.end_adjustment_frame(&ctx);
        let epoch = app.input_epoch;
        assert!(app.undo_adjustment());
        let restored = app.image.as_ref().unwrap();
        assert_eq!((restored.width, restored.height), (2, 1));
        assert_eq!(restored.data, image.data);
        assert_eq!(app.input_rotation, 0);
        assert!(app.input_epoch > epoch);
        assert!(app.redo_adjustment());
        assert_eq!(app.input_rotation, 1);
        assert_eq!(
            (
                app.image.as_ref().unwrap().width,
                app.image.as_ref().unwrap().height
            ),
            (1, 2)
        );
    }
}
