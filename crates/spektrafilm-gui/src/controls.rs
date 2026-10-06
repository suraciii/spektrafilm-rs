//! Supplemental controls for the pinned 0.3.4 GUI manifest.
//!
//! Existing exposure, scanner, chemistry, output CCTF, and basic effect editors
//! remain in main.rs. Runtime fields are edited directly; GUI-only fields use
//! the same flat section keys as upstream persistence.
use egui::{DragValue, Ui};
use serde_json::{Map, Value};
use spektrafilm_core::params::{DiffusionFilterParams, RuntimeParams};

const COLOR_SPACES: &[&str] = &[
    "sRGB", "DCI-P3", "Display P3", "Adobe RGB (1998)", "ITU-R BT.2020",
    "ProPhoto RGB", "ACES2065-1",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationAction {
    DetectBase,
    BlindCalibration,
    NeutralizeFilters,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ChangeFlags {
    /// A simulation input changed. Invalidate the applicable pipeline cache.
    pub runtime_changed: bool,
    /// Display, preview-size, or saving/auto-preview workflow state changed.
    pub display_changed: bool,
    /// Reload the selected RAW with extras.load_raw before rendering it.
    pub raw_reload: bool,
    /// The preview-size Update button was pressed, independently of auto-preview.
    pub preview_requested: bool,
    /// A real controller action was requested from the Convert panel.
    pub action: Option<CalibrationAction>,
}

/// Draw only the controls missing from the existing main.rs panels.
///
/// `extras` is GuiState.sections: top-level flat `load_raw`, `display`, and
/// `simulation` sections. Saving controls do not modify simulation output;
/// export must read simulation.saving_color_space/saving_cctf_encoding.
/// Preview-size edits update params but wait for Update, matching upstream.
pub fn show(ui: &mut Ui, params: &mut RuntimeParams, extras: &mut Value) -> ChangeFlags {
    let mut flags = ChangeFlags::default();
    ui.push_id("supplemental_controls", |ui| {
        ui.collapsing("Crop", |ui| {
            flags.runtime_changed |= ui.checkbox(&mut params.io.crop, "Crop").changed();
            flags.runtime_changed |= tuple(ui, "Crop center (x, y)", &mut params.io.crop_center, 0.0, 1.0, 0.01);
            flags.runtime_changed |= tuple(ui, "Crop size (x, y)", &mut params.io.crop_size, 0.0, 1.0, 0.01);
        });
        ui.collapsing("Workflow", |ui| {
            flags.runtime_changed |= choice(
                ui,
                "Route",
                &mut params.workflow.route,
                &[
                    "input",
                    "input > film > scan",
                    "input > film > print > scan",
                    "input > convert-film > print > scan",
                    "input > convert-film > scan-minus-base",
                    "input > convert-film > scan",
                ],
            );
        });
        ui.collapsing("Film base", |ui| {
            let base = &mut params.film_render.base;
            flags.runtime_changed |= ui.checkbox(&mut base.active, "Active").changed();
            flags.runtime_changed |= number(ui, "Scale", &mut base.scale, 0.0, f64::INFINITY, 0.01);
            flags.runtime_changed |= number(ui, "Spectral tilt", &mut base.tilt, -2.0, 2.0, 0.01);
            let mut channels = [base.cyan, base.magenta, base.yellow];
            flags.runtime_changed |= tuple(
                ui,
                "Cyan / magenta / yellow",
                &mut channels,
                0.0,
                f64::INFINITY,
                0.01,
            );
            [base.cyan, base.magenta, base.yellow] = channels;
        });
        ui.collapsing("Print base", |ui| {
            let base = &mut params.print_render.base;
            flags.runtime_changed |= ui.checkbox(&mut base.active, "Active").changed();
            flags.runtime_changed |= number(ui, "Scale", &mut base.scale, 0.0, f64::INFINITY, 0.01);
            let mut channels = [base.cyan, base.magenta, base.yellow];
            flags.runtime_changed |= tuple(
                ui,
                "Cyan / magenta / yellow",
                &mut channels,
                0.0,
                f64::INFINITY,
                0.01,
            );
            [base.cyan, base.magenta, base.yellow] = channels;
        });
        ui.collapsing("Convert film", |ui| {
            let convert = &mut params.film_render.convert;
            flags.runtime_changed |= choice(
                ui,
                "Scan illuminant",
                &mut convert.scan_illuminant,
                &["D50", "D55", "D65", "A", "BB3200", "BB5000", "T", "K75P", "E"],
            );
            flags.runtime_changed |= number(
                ui,
                "Exposure compensation (EV)",
                &mut convert.exposure_compensation_ev,
                -8.0,
                8.0,
                0.05,
            );
            flags.runtime_changed |= number(
                ui,
                "Base percentile",
                &mut convert.base_percentile,
                0.0,
                100.0,
                0.1,
            );
            ui.horizontal(|ui| {
                ui.label("Calibration (row-major)");
                flags.runtime_changed |= ui.text_edit_singleline(&mut convert.calibration).changed();
            });
            ui.horizontal(|ui| {
                if ui.button("Detect base").clicked() {
                    flags.action = Some(CalibrationAction::DetectBase);
                }
                if ui.button("Blind calibration").clicked() {
                    flags.action = Some(CalibrationAction::BlindCalibration);
                }
                if ui.button("Neutralize print filters").clicked() {
                    flags.action = Some(CalibrationAction::NeutralizeFilters);
                }
            });
        });
        ui.collapsing("Spectral adaptation", |ui| {
            flags.runtime_changed |= ui.checkbox(&mut params.settings.apply_hanatos2025_adaptation_window, "Hanatos2025 adaptation window").changed();
            flags.runtime_changed |= ui.checkbox(&mut params.settings.apply_hanatos2025_adaptation_surface, "Hanatos2025 adaptation surface").changed();
            flags.runtime_changed |= number(ui, "Spectral Gaussian blur (nm)", &mut params.settings.spectral_gaussian_blur, 0.0, f64::INFINITY, 0.1);
            flags.runtime_changed |= tuple(ui, "Input gamut knee (threshold, limit, power)", &mut params.io.input_gamut_compress.knee, 0.0, f64::INFINITY, 0.05);
            flags.runtime_changed |= tuple(ui, "Output gamut knee (threshold, limit, power)", &mut params.io.output_gamut_compress.knee, 0.0, f64::INFINITY, 0.05);
        });
        ui.collapsing("Camera filters", |ui| {
            flags.runtime_changed |= choice(
                ui,
                "Taking filter",
                &mut params.camera.color_filter,
                &["none", "hoya_x0", "hoya_x1", "hoya_y2", "hoya_ya3", "hoya_r1"],
            );
        });
        ui.collapsing("Camera UV / IR filters", |ui| {
            ui.label("Each filter: strength, cutoff wavelength (nm), transition width (nm).");
            flags.runtime_changed |= filter(ui, "UV filter", &mut params.camera.filter_uv);
            flags.runtime_changed |= filter(ui, "IR filter", &mut params.camera.filter_ir);
            flags.runtime_changed |= number(ui, "Camera diffusion core size", &mut params.camera.diffusion_filter.core_size, 0.1, 4.0, 0.05);
        });
        ui.collapsing("Layered grain details", |ui| {
            let grain = &mut params.film_render.grain;
            flags.runtime_changed |= tuple(ui, "Particle scale (R, G, B)", &mut grain.particle_scale, 0.0, f64::INFINITY, 0.05);
            flags.runtime_changed |= tuple(ui, "Particle scale (fast, mid, slow)", &mut grain.particle_scale_layers, 0.0, f64::INFINITY, 0.25);
            flags.runtime_changed |= tuple(ui, "Minimum density (R, G, B)", &mut grain.density_min, 0.0, f64::INFINITY, 0.01);
            flags.runtime_changed |= tuple(ui, "Uniformity (R, G, B)", &mut grain.uniformity, 0.0, 1.0, 0.01);
            flags.runtime_changed |= tuple(ui, "Micro structure (blur µm, clump nm)", &mut grain.micro_structure, 0.0, f64::INFINITY, 0.1);
        });
        ui.collapsing("Advanced halation", |ui| {
            let halation = &mut params.film_render.halation;
            flags.runtime_changed |= number(ui, "Highlight boost (EV)", &mut halation.boost_ev, 0.0, f64::INFINITY, 0.5);
            flags.runtime_changed |= number(ui, "Protected highlight range (EV)", &mut halation.protect_ev, 0.0, f64::INFINITY, 0.5);
            flags.runtime_changed |= number(ui, "Boost range", &mut halation.boost_range, 0.0, 1.0, 0.05);
            flags.runtime_changed |= tuple(ui, "Scatter core (R, G, B; µm)", &mut halation.scatter_core_um, 0.0, f64::INFINITY, 0.5);
            flags.runtime_changed |= tuple(ui, "Scatter tail (R, G, B; µm)", &mut halation.scatter_tail_um, 0.0, f64::INFINITY, 1.0);
            flags.runtime_changed |= tuple(ui, "Scatter tail weight (R, G, B)", &mut halation.scatter_tail_weight, 0.0, 1.0, 0.01);
            flags.runtime_changed |= tuple(ui, "Halation strength (R, G, B)", &mut halation.halation_strength, 0.0, f64::INFINITY, 0.005);
            flags.runtime_changed |= tuple(ui, "First bounce sigma (R, G, B; µm)", &mut halation.halation_first_sigma_um, 0.0, f64::INFINITY, 1.0);
        });
        ui.collapsing("Advanced DIR couplers", |ui| {
            let couplers = &mut params.film_render.dir_couplers;
            flags.runtime_changed |= number(ui, "Same-layer inhibition", &mut couplers.inhibition_samelayer, 0.0, f64::INFINITY, 0.05);
            flags.runtime_changed |= number(ui, "Interlayer inhibition", &mut couplers.inhibition_interlayer, 0.0, f64::INFINITY, 0.05);
            flags.runtime_changed |= tuple(ui, "Same-layer gamma (R, G, B)", &mut couplers.gamma_samelayer_rgb, 0.0, f64::INFINITY, 0.02);
            flags.runtime_changed |= tuple(ui, "Gamma R → (G, B)", &mut couplers.gamma_interlayer_r_to_gb, 0.0, f64::INFINITY, 0.02);
            flags.runtime_changed |= tuple(ui, "Gamma G → (R, B)", &mut couplers.gamma_interlayer_g_to_rb, 0.0, f64::INFINITY, 0.02);
            flags.runtime_changed |= tuple(ui, "Gamma B → (R, G)", &mut couplers.gamma_interlayer_b_to_rg, 0.0, f64::INFINITY, 0.02);
        });
        ui.collapsing("Enlarger details", |ui| {
            flags.runtime_changed |= choice(ui, "Print illuminant", &mut params.enlarger.illuminant, &["TH-KG3", "D50", "D55", "D65"]);
            flags.runtime_changed |= ui.checkbox(&mut params.enlarger.print_exposure_compensation, "Print auto compensation").changed();
            flags.runtime_changed |= number(ui, "Enlarger lens blur (px)", &mut params.enlarger.lens_blur, 0.0, f64::INFINITY, 0.05);
            flags.runtime_changed |= number(ui, "Film density curve gamma", &mut params.film_render.density_curve_gamma, 0.0, f64::INFINITY, 0.05);
        });
        ui.collapsing("Enlarger diffusion", |ui| {
            flags.runtime_changed |= diffusion(ui, &mut params.enlarger.diffusion_filter);
        });
        ui.collapsing("Experimental channel swaps", |ui| {
            flags.runtime_changed |= extra_channels(ui, extras, "film_channel_swap", "Film channel swap (R, G, B)");
            flags.runtime_changed |= extra_channels(ui, extras, "print_channel_swap", "Print channel swap (R, G, B)");
        });
        ui.collapsing("Saving color", |ui| {
            flags.display_changed |= extra_choice(ui, extras, "simulation", "saving_color_space", "Saving color space", "sRGB", COLOR_SPACES);
            flags.display_changed |= extra_bool(ui, extras, "simulation", "saving_cctf_encoding", "Saving CCTF encoding", true);
        });
        ui.collapsing("Preview workflow", |ui| {
            flags.display_changed |= extra_bool(ui, extras, "simulation", "auto_preview", "Auto preview", true);
            if number(ui, "Preview max size (px)", &mut params.settings.preview_max_size, 128.0, u32::MAX as f64, 128.0) {
                set_extra(extras, "display", "preview_max_size", Value::from(params.settings.preview_max_size));
                flags.display_changed = true;
            }
            flags.preview_requested |= ui.button("Update preview").clicked();
        });
        ui.collapsing("Import RAW", |ui| {
            flags.raw_reload |= extra_choice(ui, extras, "load_raw", "white_balance", "White balance", "as_shot", &["as_shot", "daylight", "tungsten", "custom"]);
            let custom = extras["load_raw"]["white_balance"].as_str() == Some("custom");
            ui.add_enabled_ui(custom, |ui| {
                flags.raw_reload |= extra_number(ui, extras, "load_raw", "temperature", "Temperature (K)", 5500.0, 1000.0, f64::INFINITY, 100.0);
                flags.raw_reload |= extra_number(ui, extras, "load_raw", "tint", "Tint", 1.0, 0.0, f64::INFINITY, 0.01);
            });
            flags.raw_reload |= extra_bool(ui, extras, "load_raw", "lens_correction", "Lensfun lens correction", false);
            flags.raw_reload |= ui.button("Reprocess RAW").clicked();
        });
    });
    flags
}

fn diffusion(ui: &mut Ui, value: &mut DiffusionFilterParams) -> bool {
    let mut changed = ui.checkbox(&mut value.active, "Active").changed();
    changed |= choice(ui, "Family", &mut value.filter_family, &["glimmerglass", "black_pro_mist", "pro_mist", "cinebloom"]);
    changed |= number(ui, "Strength", &mut value.strength, 0.0, 2.0, 0.125);
    changed |= number(ui, "Spatial scale", &mut value.spatial_scale, 0.0, f64::INFINITY, 0.1);
    changed |= number(ui, "Halo warmth", &mut value.halo_warmth, -1.5, 1.5, 0.05);
    changed |= number(ui, "Core intensity", &mut value.core_intensity, 0.0, 4.0, 0.05);
    changed |= number(ui, "Core size", &mut value.core_size, 0.1, 4.0, 0.05);
    changed |= number(ui, "Halo intensity", &mut value.halo_intensity, 0.0, 4.0, 0.05);
    changed |= number(ui, "Halo size", &mut value.halo_size, 0.1, 4.0, 0.05);
    changed |= number(ui, "Bloom intensity", &mut value.bloom_intensity, 0.0, 4.0, 0.05);
    changed |= number(ui, "Bloom size", &mut value.bloom_size, 0.1, 4.0, 0.05);
    changed
}

fn filter(ui: &mut Ui, label: &str, value: &mut [f32; 3]) -> bool {
    ui.push_id(label, |ui| {
        ui.label(label);
        let mut changed = number(ui, "Strength", &mut value[0], 0.0, 1.0, 0.01);
        changed |= number(ui, "Cutoff (nm)", &mut value[1], 0.0, f64::INFINITY, 1.0);
        changed |= number(ui, "Transition (nm)", &mut value[2], 0.0, f64::INFINITY, 0.1);
        changed
    }).inner
}

fn number<T: egui::emath::Numeric>(ui: &mut Ui, label: &str, value: &mut T, min: f64, max: f64, step: f64) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(DragValue::new(value).range(min..=max).clamp_existing_to_range(false).speed(step)).changed()
    }).inner
}

fn tuple<T: egui::emath::Numeric, const N: usize>(ui: &mut Ui, label: &str, values: &mut [T; N], min: f64, max: f64, step: f64) -> bool {
    ui.push_id(label, |ui| {
        ui.label(label);
        ui.horizontal(|ui| {
            let mut changed = false;
            for value in values {
                changed |= ui.add(DragValue::new(value).range(min..=max).clamp_existing_to_range(false).speed(step)).changed();
            }
            changed
        }).inner
    }).inner
}

fn choice(ui: &mut Ui, label: &str, value: &mut String, choices: &[&str]) -> bool {
    let mut changed = false;
    egui::ComboBox::from_label(label).selected_text(value.as_str()).show_ui(ui, |ui| {
        for &option in choices {
            // Allocate only when a selection changes, not for every option each frame.
            if ui.selectable_label(value.as_str() == option, option).clicked() && value.as_str() != option {
                *value = option.to_owned();
                changed = true;
            }
        }
    });
    changed
}

fn extra_choice(ui: &mut Ui, extras: &mut Value, section: &str, key: &str, label: &str, default: &str, choices: &[&str]) -> bool {
    let current = extras[section][key].as_str().unwrap_or(default);
    let mut selected = None;
    egui::ComboBox::from_label(label).selected_text(current).show_ui(ui, |ui| {
        for &option in choices {
            if ui.selectable_label(current == option, option).clicked() && current != option {
                selected = Some(option);
            }
        }
    });
    if let Some(selected) = selected {
        set_extra(extras, section, key, Value::from(selected));
        true
    } else {
        false
    }
}

fn extra_bool(ui: &mut Ui, extras: &mut Value, section: &str, key: &str, label: &str, default: bool) -> bool {
    let mut value = extras[section][key].as_bool().unwrap_or(default);
    if ui.checkbox(&mut value, label).changed() {
        set_extra(extras, section, key, Value::from(value));
        true
    } else {
        false
    }
}

fn extra_number(ui: &mut Ui, extras: &mut Value, section: &str, key: &str, label: &str, default: f64, min: f64, max: f64, step: f64) -> bool {
    let mut value = extras[section][key].as_f64().unwrap_or(default);
    if number(ui, label, &mut value, min, max, step) {
        set_extra(extras, section, key, Value::from(value));
        true
    } else {
        false
    }
}

fn extra_channels(ui: &mut Ui, extras: &mut Value, key: &str, label: &str) -> bool {
    let mut values = [0_u32, 1, 2];
    if let Some(saved) = extras["special"][key].as_array() {
        for (value, saved) in values.iter_mut().zip(saved) {
            if let Some(channel) = saved.as_u64().filter(|&channel| channel <= 2) {
                *value = channel as u32;
            }
        }
    }
    if tuple(ui, label, &mut values, 0.0, 2.0, 1.0) {
        set_extra(extras, "special", key, Value::from(values.to_vec()));
        true
    } else {
        false
    }
}

fn set_extra(extras: &mut Value, section: &str, key: &str, value: Value) {
    if !extras.is_object() {
        *extras = Value::Object(Map::new());
    }
    if !extras[section].is_object() {
        extras[section] = Value::Object(Map::new());
    }
    extras[section][key] = value;
}
