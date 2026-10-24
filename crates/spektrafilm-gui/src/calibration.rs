use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use spektrafilm_core::profile;

use super::{App, controls};

pub(super) enum CalibrationResult {
    Base {
        params: spektrafilm_core::params::FilmBaseParams,
        exposure_ev: f64,
    },
    BlindCalibration(String),
    NeutralizeFilters {
        m_shift: f32,
        y_shift: f32,
    },
}

pub(super) struct CalibrationJob {
    epoch: u64,
    rx: mpsc::Receiver<Result<CalibrationResult, String>>,
    pub(super) handle: Option<JoinHandle<()>>,
}

impl App {
    pub(super) fn start_calibration(&mut self, action: controls::CalibrationAction) {
        if self.calibration_job.is_some() || self.render_job.is_some() || self.export_job.is_some()
        {
            self.status = crate::app::Message::new("operations.calibration_wait");
            return;
        }
        let Some(image) = self.image.clone() else {
            self.status = crate::app::Message::new("operations.calibration_load");
            return;
        };
        let mut params = match self
            .current_state()
            .and_then(|state| state.runtime_params())
        {
            Ok(params) => params,
            Err(error) => {
                self.status = crate::app::Message::new("operations.calibration_state_error")
                    .message_arg("detail", crate::app::Message::error(&error));
                return;
            }
        };
        let film = match profile::load_profile_by_name(&self.data_dir, &self.film_name) {
            Ok(profile) => profile,
            Err(error) => {
                self.status = crate::app::Message::new("operations.calibration_film_error")
                    .arg("detail", error);
                return;
            }
        };
        let print = match profile::load_profile_by_name(&self.data_dir, &self.print_name) {
            Ok(profile) => profile,
            Err(error) => {
                self.status = crate::app::Message::new("operations.calibration_print_error")
                    .arg("detail", error);
                return;
            }
        };
        let data_dir = self.data_dir.clone();
        let backend = Arc::clone(&self.backend);
        let epoch = self.calibration_epoch.wrapping_add(1);
        self.calibration_epoch = epoch;
        let (tx, rx) = mpsc::channel();
        self.status = crate::app::Message::new(match action {
            controls::CalibrationAction::DetectBase => "operations.detecting_base",
            controls::CalibrationAction::BlindCalibration => "operations.fitting_calibration",
            controls::CalibrationAction::NeutralizeFilters => "operations.neutralizing_filters",
        });
        let handle = std::thread::Builder::new()
            .name("spektrafilm-calibration".into())
            .spawn(move || {
                let result = (|| {
                    params.workflow.route = "input > convert-film > scan".into();
                    match action {
                        controls::CalibrationAction::DetectBase => {
                            // Explicit source-analysis action, not a stage in
                            // the selected GPU numerical render.
                            let image = image.to_host(backend.as_ref())?;
                            spektrafilm_core::stages::converting::detect_base(
                                &image, &film, &params,
                            )
                            .map(|(params, exposure_ev)| {
                                CalibrationResult::Base {
                                    params,
                                    exposure_ev,
                                }
                            })
                        }
                        controls::CalibrationAction::BlindCalibration => {
                            let image = image.to_host(backend.as_ref())?;
                            spektrafilm_core::stages::converting::blind_calibration(
                                &image, &film, &params,
                            )
                            .map(CalibrationResult::BlindCalibration)
                        }
                        controls::CalibrationAction::NeutralizeFilters => {
                            spektrafilm_core::stages::converting::neutralize_filters(
                                &film,
                                &print,
                                &params,
                                &data_dir,
                                backend.as_ref(),
                            )
                            .map(|(m_shift, y_shift)| {
                                CalibrationResult::NeutralizeFilters { m_shift, y_shift }
                            })
                        }
                    }
                })();
                let _ = tx.send(result);
            })
            .map_err(|error| error.to_string());
        match handle {
            Ok(handle) => {
                self.calibration_job = Some(CalibrationJob {
                    epoch,
                    rx,
                    handle: Some(handle),
                });
            }
            Err(error) => {
                self.status = crate::app::Message::new("operations.calibration_worker_error")
                    .arg("detail", error);
            }
        }
    }

    pub(super) fn poll_calibration_job(&mut self, ctx: &egui::Context) {
        let Some(job) = self.calibration_job.as_ref() else {
            return;
        };
        let result = match job.rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(16));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let mut job = self.calibration_job.take().expect("calibration job exists");
                if let Some(handle) = job.handle.take() {
                    let _ = handle.join();
                }
                self.status = crate::app::Message::new("operations.calibration_worker_vanished");
                return;
            }
        };
        let mut job = self.calibration_job.take().expect("calibration job exists");
        let epoch = job.epoch;
        if let Some(handle) = job.handle.take() {
            let _ = handle.join();
        }
        if epoch != self.calibration_epoch {
            self.status = crate::app::Message::new("operations.calibration_stale");
            return;
        }
        match result {
            Ok(CalibrationResult::Base {
                params,
                exposure_ev,
            }) => {
                self.params.film_render.base = params;
                self.params.film_render.convert.exposure_compensation_ev = exposure_ev;
                self.mark_parameters_changed();

                self.status = crate::app::Message::new("operations.base_detected")
                    .arg("exposure", format!("{exposure_ev:+.2}"));
            }
            Ok(CalibrationResult::BlindCalibration(calibration)) => {
                self.params.film_render.convert.calibration = calibration;
                self.mark_parameters_changed();

                self.status = crate::app::Message::new("operations.calibration_fitted");
            }
            Ok(CalibrationResult::NeutralizeFilters { m_shift, y_shift }) => {
                self.params.enlarger.m_filter_shift = m_shift;
                self.params.enlarger.y_filter_shift = y_shift;
                self.mark_parameters_changed();

                self.status = crate::app::Message::new("operations.filters_neutralized")
                    .arg("magenta", format!("{m_shift:+.2}"))
                    .arg("yellow", format!("{y_shift:+.2}"));
            }
            Err(error) => {
                self.status =
                    crate::app::Message::new("operations.calibration_failed").arg("detail", error);
            }
        }
        ctx.request_repaint();
    }
}
