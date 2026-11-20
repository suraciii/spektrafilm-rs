//! Multiple-exposure stack panel shared by the Film and RGB workflows.

use crate::multiple_exposure::{MultipleExposureMetering, MultipleExposureMethod};
use crate::*;

impl App {
    pub(super) fn multiple_exposure_panel(&mut self, ctx: &egui::Context) {
        if !self.multiple_exposure.open {
            return;
        }
        let backend = self.backend.clone();
        self.multiple_exposure.ensure_thumbnails(
            self.image_path.as_deref(),
            self.image.as_ref(),
            backend.as_ref(),
            ctx,
        );
        let mut open = true;
        let mut add = false;
        let mut reload = false;
        let mut changed = false;
        let mut move_shot = None;
        let mut remove = None;
        let mut replace = None;
        let mut selected = self.multiple_exposure.selected;
        let film_available =
            self.params.workflow.mode.is_film() && self.params.workflow.route_enum().exposes_film();
        let primary_raw = self.image_path.as_deref().is_some_and(image_io::is_raw);
        let rgb_available = !self.params.workflow.mode.is_film() && !primary_raw;
        egui::Window::new(crate::i18n::text(ctx, "multiple_exposure.title"))
            .id(egui::Id::new("multiple-exposure-panel"))
            .open(&mut open)
            .default_width(360.0)
            .resizable(true)
            .show(ctx, |ui| {
                if self.input_transition_pending() {
                    ui.disable();
                }
                let mut settings = self.multiple_exposure.settings.clone();
                ui.horizontal_wrapped(|ui| {
                    ui.label(crate::i18n::text(ui.ctx(), "multiple_exposure.method"));
                    let film = ui
                        .selectable_label(
                            settings.method == MultipleExposureMethod::Film,
                            crate::i18n::text(ui.ctx(), "multiple_exposure.method_film"),
                        )
                        .on_hover_text(crate::i18n::text(
                            ui.ctx(),
                            "multiple_exposure.method_film_help",
                        ));
                    let rgb = ui
                        .selectable_label(
                            settings.method == MultipleExposureMethod::Rgb,
                            crate::i18n::text(ui.ctx(), "multiple_exposure.method_rgb"),
                        )
                        .on_hover_text(crate::i18n::text(
                            ui.ctx(),
                            "multiple_exposure.method_rgb_help",
                        ));
                    if film.clicked() && film_available {
                        settings.method = MultipleExposureMethod::Film;
                    }
                    if rgb.clicked() && rgb_available {
                        settings.method = MultipleExposureMethod::Rgb;
                    }
                });
                if !film_available && settings.method == MultipleExposureMethod::Film {
                    ui.colored_label(
                        theme::WARNING,
                        crate::i18n::text(ui.ctx(), "multiple_exposure.film_unavailable"),
                    );
                }
                if primary_raw && settings.method == MultipleExposureMethod::Rgb {
                    ui.colored_label(
                        theme::WARNING,
                        crate::i18n::text(ui.ctx(), "multiple_exposure.rgb_raw_unavailable"),
                    );
                } else if !rgb_available && settings.method == MultipleExposureMethod::Rgb {
                    ui.colored_label(
                        theme::WARNING,
                        crate::i18n::text(ui.ctx(), "multiple_exposure.rgb_unavailable"),
                    );
                }
                if settings.method == MultipleExposureMethod::Film {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::text(ui.ctx(), "multiple_exposure.metering"));
                        egui::ComboBox::from_id_salt("multiple-exposure-metering")
                            .selected_text(crate::i18n::text(
                                ui.ctx(),
                                settings.metering.label_key(),
                            ))
                            .show_ui(ui, |ui| {
                                for metering in [
                                    MultipleExposureMetering::Manual,
                                    MultipleExposureMetering::Each,
                                    MultipleExposureMetering::LockToFirst,
                                ] {
                                    ui.selectable_value(
                                        &mut settings.metering,
                                        metering,
                                        crate::i18n::text(ui.ctx(), metering.label_key()),
                                    );
                                }
                            });
                    });
                }
                let primary_name = self
                    .image_path
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| {
                        crate::i18n::text(ui.ctx(), "multiple_exposure.no_primary").into_owned()
                    });
                ui.horizontal(|ui| {
                    if let Some(path) = self.image_path.as_deref()
                        && let Some(texture) = self.multiple_exposure.thumbnails.get(path)
                    {
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(
                            texture.id(),
                            egui::vec2(48.0, 32.0),
                        )));
                    }
                    ui.label(
                        egui::RichText::new(crate::i18n::text(
                            ui.ctx(),
                            "multiple_exposure.primary",
                        ))
                        .strong(),
                    );
                    if ui.selectable_label(selected == 0, primary_name).clicked() {
                        selected = 0;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::DragValue::new(&mut settings.primary_exposure_ev)
                                .range(-10.0..=10.0)
                                .speed(0.05)
                                .prefix("EV "),
                        );
                    });
                });
                for (index, shot) in settings.shots.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if let Some(texture) = self.multiple_exposure.thumbnails.get(&shot.path) {
                            ui.add(egui::Image::new(egui::load::SizedTexture::new(
                                texture.id(),
                                egui::vec2(48.0, 32.0),
                            )));
                        }
                        let name = shot
                            .path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| shot.path.display().to_string());
                        if ui
                            .selectable_label(
                                selected == index + 1,
                                format!("{}. {name}", index + 2),
                            )
                            .clicked()
                        {
                            selected = index + 1;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button(crate::i18n::text(
                                    ui.ctx(),
                                    "multiple_exposure.remove",
                                ))
                                .clicked()
                            {
                                remove = Some(index);
                            }
                            if ui
                                .small_button("…")
                                .on_hover_text(crate::i18n::text(
                                    ui.ctx(),
                                    "multiple_exposure.replace_source",
                                ))
                                .clicked()
                            {
                                replace = Some(index);
                                selected = index + 1;
                            }
                            if ui
                                .small_button("↓")
                                .on_hover_text(crate::i18n::text(
                                    ui.ctx(),
                                    "multiple_exposure.move_down",
                                ))
                                .clicked()
                            {
                                move_shot = Some((index, 1));
                            }
                            if ui
                                .small_button("↑")
                                .on_hover_text(crate::i18n::text(
                                    ui.ctx(),
                                    "multiple_exposure.move_up",
                                ))
                                .clicked()
                            {
                                move_shot = Some((index, -1));
                            }
                            ui.add(
                                egui::DragValue::new(&mut shot.exposure_ev)
                                    .range(-10.0..=10.0)
                                    .speed(0.05)
                                    .prefix("EV "),
                            );
                        });
                    });
                }
                ui.separator();
                let total = settings.total_shots();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            total < 8 && self.image.is_some() && !self.input_transition_pending(),
                            egui::Button::new(crate::i18n::text(ui.ctx(), "multiple_exposure.add")),
                        )
                        .clicked()
                    {
                        add = true;
                    }
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "multiple_exposure.count",
                        count = total
                    ));
                    let missing = settings
                        .shots
                        .iter()
                        .any(|shot| self.multiple_exposure.image(&shot.path).is_none());
                    if missing
                        && ui
                            .button(crate::i18n::text(ui.ctx(), "multiple_exposure.reload"))
                            .clicked()
                    {
                        reload = true;
                    }
                });
                if settings.shots.is_empty() {
                    ui.label(crate::i18n::text(ui.ctx(), "multiple_exposure.empty_help"));
                }
                if let Some(error) = self.multiple_exposure.error.as_deref() {
                    ui.colored_label(theme::ERROR, error);
                } else if settings.active()
                    && let Err(error) = settings.validate()
                {
                    ui.colored_label(theme::ERROR, error);
                }
                ui.label(crate::i18n::text(ui.ctx(), "multiple_exposure.scan_help"));
                changed = settings != self.multiple_exposure.settings;
                if changed {
                    self.multiple_exposure.settings = settings;
                }
            });
        self.multiple_exposure.open = open;
        if let Some((index, delta)) = move_shot {
            let target = index as isize + delta;
            if (0..self.multiple_exposure.settings.shots.len() as isize).contains(&target) {
                self.multiple_exposure
                    .settings
                    .shots
                    .swap(index, target as usize);
                if self.multiple_exposure.selected == index + 1 {
                    self.multiple_exposure.selected = target as usize + 1;
                } else if self.multiple_exposure.selected == target as usize + 1 {
                    self.multiple_exposure.selected = index + 1;
                }
                changed = true;
            }
        }
        if let Some(index) = remove
            && index < self.multiple_exposure.settings.shots.len()
        {
            self.multiple_exposure.settings.shots.remove(index);
            if self.multiple_exposure.selected == index + 1 {
                self.multiple_exposure.selected = 0;
            } else if self.multiple_exposure.selected > index + 1 {
                self.multiple_exposure.selected -= 1;
            }
            changed = true;
        }
        if selected != self.multiple_exposure.selected {
            self.multiple_exposure.selected = selected;
        }
        self.multiple_exposure.selected = self
            .multiple_exposure
            .selected
            .min(self.multiple_exposure.settings.shots.len());
        if changed {
            self.multiple_exposure.clear_error();
            self.mark_parameters_changed();
            self.render_worker.cancel_active();
            self.source_render = None;
            self.label_adjustment_message(crate::app::Message::new(
                "multiple_exposure.history_edit",
            ));
        }
        if let Some(index) = replace {
            self.replace_multiple_exposure_source(index);
        }
        if add {
            self.add_multiple_exposure_shots();
        }
        if reload {
            self.reload_multiple_exposure_sources();
        }
    }
}
