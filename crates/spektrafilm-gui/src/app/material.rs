//! Film/print material selection, route normalization and section resets.
use super::state::App;
use crate::*;

impl App {
    pub(crate) fn toggle_scan_for_print(&mut self) {
        if let Some(glare) = self.scan_for_print_snapshot.take() {
            self.params.print_render.glare.active = glare;
        } else {
            self.scan_for_print_snapshot = Some(self.params.print_render.glare.active);
            self.params.print_render.glare.active = false;
        }
        self.mark_parameters_changed();
    }

    pub(crate) fn finish_route_change(&mut self) {
        if let Some(glare) = self.scan_for_print_snapshot.take() {
            self.params.print_render.glare.active = glare;
        }
        let before = self.params.scanner.clone();
        self.params.io.scan_film = self.params.workflow.route == "input > film > scan";
        normalize_scanner_selections(&mut self.params, self.film_medium);
        self.status = Self::normalization_status(
            &before,
            &self.params.scanner,
            self.film_medium,
            &self.params.workflow.route,
            crate::app::Message::new("surfaces.status.route_changed"),
        );
        self.invalidate_look_render();
    }

    pub(crate) fn scanner_normalization_detail(
        before: &spektrafilm_core::params::ScannerParams,
        after: &spektrafilm_core::params::ScannerParams,
        film_medium: controls::ScanMedium,
        route: &str,
    ) -> Option<crate::app::Message> {
        let normalized = before.scan_output != after.scan_output;
        let inactive = film_medium.for_route(route) == controls::ScanMedium::None;
        let key = match (normalized, inactive) {
            (false, false) => return None,
            (true, false) => "surfaces.status.normalized",
            (false, true) => "surfaces.status.scanner_inactive",
            (true, true) => "surfaces.status.normalized_inactive",
        };
        Some(crate::app::Message::new(key))
    }

    pub(crate) fn normalization_status(
        before: &spektrafilm_core::params::ScannerParams,
        after: &spektrafilm_core::params::ScannerParams,
        film_medium: controls::ScanMedium,
        route: &str,
        cause: crate::app::Message,
    ) -> crate::app::Message {
        match Self::scanner_normalization_detail(before, after, film_medium, route) {
            Some(detail) => crate::app::Message::new("surfaces.status.normalization")
                .message_arg("cause", cause)
                .message_arg("detail", detail),
            None => cause,
        }
    }

    pub(crate) fn invalidate_look_render(&mut self) {
        self.scan_for_print_snapshot = None;
        self.mark_parameters_changed_invalidating_calibration();
    }

    pub(crate) fn current_material_baseline(&mut self) -> Option<Arc<serde_json::Value>> {
        if let Some((data_dir, film, baseline)) = &self.material_baseline
            && data_dir == &self.data_dir
            && film == &self.film_name
        {
            return baseline.clone();
        }
        let baseline = match profile::load_profile_by_name(&self.data_dir, &self.film_name) {
            Ok(film) if film.is_film() && film.is_filming() => {
                Some(Arc::new(section_reset::material_baseline(&film)))
            }
            Ok(_) => {
                self.status = crate::app::Message::new("surfaces.status.profile_not_film")
                    .arg("name", &self.film_name);
                None
            }
            Err(error) => {
                self.status = crate::app::Message::new("surfaces.presets.profile_load")
                    .message_arg("kind", crate::app::Message::control("film"))
                    .arg("stock", &self.film_name)
                    .message_arg("detail", crate::app::Message::profile(&error));
                None
            }
        };
        self.material_baseline = Some((
            self.data_dir.clone(),
            self.film_name.clone(),
            baseline.clone(),
        ));
        baseline
    }

    pub(crate) fn select_film_profile(&mut self, selected: String) {
        let film = match profile::load_profile_by_name(&self.data_dir, &selected) {
            Ok(profile) if profile.is_film() && profile.is_filming() => profile,
            Ok(_) => {
                self.status = crate::app::Message::new("surfaces.status.profile_not_film")
                    .arg("name", selected);
                return;
            }
            Err(error) => {
                self.status = crate::app::Message::new("surfaces.presets.profile_load")
                    .message_arg("kind", crate::app::Message::control("film"))
                    .arg("stock", &selected)
                    .message_arg("detail", crate::app::Message::profile(&error));
                return;
            }
        };
        spektrafilm_core::params_builder::apply_film_specifics(&mut self.params, &film);
        self.material_baseline = Some((
            self.data_dir.clone(),
            selected.clone(),
            Some(Arc::new(section_reset::material_baseline(&film))),
        ));
        self.film_name = selected;
        self.params.film_render.grain.monochrome = film.is_bw();
        self.params.film_render.development_time = None;
        self.gui_state.sections["simulation"]["film_stock"] = serde_json::json!(self.film_name);
        self.film_reference_illuminant = film.info.reference_illuminant.clone();
        self.film_dev_times = film.data.development_time.clone();
        self.film_medium = scan_medium(&film);
        let before = self.params.scanner.clone();
        self.params.workflow.route = workflow_default(&film);
        self.params.io.scan_film = film.is_positive();
        self.params.scanner.scan_output = "direct_scan".into();
        normalize_scanner_selections(&mut self.params, self.film_medium);
        self.status = Self::normalization_status(
            &before,
            &self.params.scanner,
            self.film_medium,
            &self.params.workflow.route,
            crate::app::Message::new("surfaces.status.film_changed"),
        );
        self.invalidate_look_render();
    }

    pub(crate) fn select_print_profile(&mut self, selected: String) {
        let print = match profile::load_profile_by_name(&self.data_dir, &selected) {
            Ok(profile) if profile.is_print_stock() => profile,
            Ok(_) => {
                self.status = crate::app::Message::new("surfaces.status.profile_not_print")
                    .arg("name", selected);
                return;
            }
            Err(error) => {
                self.status = crate::app::Message::new("surfaces.presets.profile_load")
                    .message_arg("kind", crate::app::Message::control("print"))
                    .arg("stock", &selected)
                    .message_arg("detail", crate::app::Message::profile(&error));
                return;
            }
        };
        self.print_name = selected;
        self.params.print_render.development_time = None;
        self.gui_state.sections["simulation"]["print_paper"] = serde_json::json!(self.print_name);
        self.print_dev_times = print.data.development_time.clone();
        self.invalidate_look_render();
    }

    pub(crate) fn reset_parameter_section(&mut self, section: &str) -> bool {
        if !matches!(section, "Session" | "Display") {
            self.label_adjustment_message(
                crate::app::Message::new("operations.history.reset_section")
                    .message_arg("section", crate::app::Message::control(section)),
            );
        }
        if section == "Film material" {
            let factory = state::GuiState::factory();
            if self.film_name != factory.film() {
                self.select_film_profile(factory.film().to_owned());
            }
        } else if section == "Print material" {
            let factory = state::GuiState::factory();
            if self.print_name != factory.paper() {
                self.select_print_profile(factory.paper().to_owned());
            }
        }
        let baseline = self.current_material_baseline();
        if baseline.is_none() && matches!(section, "Grain" | "Halation" | "Couplers") {
            return false;
        }
        let baseline = baseline
            .unwrap_or_else(|| Arc::new(section_reset::unavailable_material_baseline().clone()));
        section_reset::reset(
            &mut self.params,
            &mut self.gui_state.sections,
            section,
            &baseline,
        );
        if section == "Workflow" {
            self.finish_route_change();
        } else if section != "Display" {
            self.mark_parameters_changed_invalidating_calibration();
        }
        self.search_index.reset();
        true
    }
}
