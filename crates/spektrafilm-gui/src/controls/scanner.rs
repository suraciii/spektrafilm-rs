use super::*;
use crate::routes::vocabulary;
use egui::Ui;
use serde_json::Value;
use spektrafilm_core::params::ScanOperatorParams;
use std::path::Path;

/// Session-only clipboard and held comparison; App owns all adjustment history.
#[derive(Clone, Default)]
struct OperatorEditor {
    clipboard: Option<ScanOperatorParams>,
    comparing: bool,
}

fn operator_editor_id() -> egui::Id {
    egui::Id::new("scanner.operator.editor")
}

/// End a held comparison even if its section has since been hidden.
pub(crate) fn release_operator_compare(ctx: &egui::Context) -> bool {
    let holding =
        ctx.input(|input| input.pointer.primary_down() || input.key_down(egui::Key::Space));
    if holding {
        return false;
    }
    ctx.data_mut(|data| {
        let editor = data.get_temp_mut_or_default::<OperatorEditor>(operator_editor_id());
        std::mem::take(&mut editor.comparing)
    })
}

fn operator_controls(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    active: bool,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    if !filter_matches_any(
        filter,
        &[
            "finished exposure",
            "warmth",
            "green–magenta tint",
            "contrast",
            "restore automatic",
            "copy operator correction",
            "paste operator correction",
            "hold automatic comparison",
        ],
    ) {
        return;
    }
    ui.separator();
    ui.label("Operator correction");
    if !active {
        ui.label("Inactive: requires a negative film, an eligible negative-to-positive route and standard conversion style. Stored corrections are retained.");
    }
    let mut editor = ui.ctx().data_mut(|data| {
        data.remove_temp::<OperatorEditor>(operator_editor_id())
            .unwrap_or_default()
    });
    let operator = &mut params.scanner.operator;
    let before = *operator;
    ui.add_enabled_ui(active, |ui| {
        if filter_matches(filter, "Finished exposure") {
            param_numeric(ui, "Finished exposure", &mut operator.exposure_ev,
                "scanner.operator.exposure_ev",
                "Finished-positive exposure in stops. Positive values brighten the result after automatic setup, before the fixed tone curve; this does not change camera exposure or film density.");
        }
        if filter_matches(filter, "Warmth") {
            param_numeric(ui, "Warmth", &mut operator.warmth_ev,
                "scanner.operator.warmth_ev",
                "Red-minus-blue exposure difference in EV. Positive values are warmer; negative values are cooler. This is not Kelvin and does not change capture white balance.");
        }
        if filter_matches(filter, "Green–magenta tint") {
            param_numeric(ui, "Green–magenta tint", &mut operator.tint_ev,
                "scanner.operator.tint_ev",
                "Mean red/blue minus green exposure difference in EV. Positive values add magenta; negative values add green.");
        }
        if filter_matches(filter, "Contrast") {
            param_numeric(ui, "Contrast", &mut operator.contrast,
                "scanner.operator.contrast",
                "Common exposure contrast around the fixed reference exposure. Above 1 is harder; below 1 is softer. Channel differences and automatic setup are unchanged.");
        }
        ui.horizontal_wrapped(|ui| {
            if filter_matches(filter, "restore automatic") {
                let response = ui.add_enabled(*operator != ScanOperatorParams::default(), egui::Button::new("Restore automatic"))
                    .on_hover_text("Reset the four operator values only; keep this frame's own automatic setup.");
                focus_response(ui, "Restore automatic", &response);
                if response.clicked() {
                    *operator = ScanOperatorParams::default();
                }
            }
            if filter_matches(filter, "copy operator correction") {
                let response = ui.button("Copy operator correction")
                    .on_hover_text("Copy only the four operator values for this session, not automatic setup, film density or capture settings.");
                focus_response(ui, "Copy operator correction", &response);
                if response.clicked() {
                    editor.clipboard = Some(*operator);
                }
            }
            if filter_matches(filter, "paste operator correction") {
                let response = ui.add_enabled(editor.clipboard.is_some(), egui::Button::new("Paste operator correction"))
                    .on_hover_text("Apply the copied four values to this frame's own automatic setup.");
                focus_response(ui, "Paste operator correction", &response);
                if response.clicked()
                    && let Some(copied) = editor.clipboard
                {
                    *operator = copied;
                }
            }
            if filter_matches(filter, "hold automatic comparison") {
                let response = ui.button("Hold automatic comparison")
                    .on_hover_text("Hold the primary mouse button or focused Space to view this same frame with default operator values. Release to restore the corrected view. Saved and exported corrections are unchanged.");
                focus_response(ui, "Hold automatic comparison", &response);
                let comparing = (response.is_pointer_button_down_on()
                    && ui.input(|input| input.pointer.primary_down()))
                    || (response.has_focus() && ui.input(|input| input.key_down(egui::Key::Space)));
                if comparing != editor.comparing {
                    editor.comparing = comparing;
                    flags.operator_compare = Some(comparing);
                }
            }
        });
    });
    if !active && editor.comparing {
        editor.comparing = false;
        flags.operator_compare = Some(false);
    }
    for (path, previous, current) in [
        (
            "scanner.operator.exposure_ev",
            before.exposure_ev,
            operator.exposure_ev,
        ),
        (
            "scanner.operator.warmth_ev",
            before.warmth_ev,
            operator.warmth_ev,
        ),
        ("scanner.operator.tint_ev", before.tint_ev, operator.tint_ev),
        (
            "scanner.operator.contrast",
            before.contrast,
            operator.contrast,
        ),
    ] {
        if previous != current {
            flags.runtime_changed = true;
            record_edited_path(ui.ctx(), path);
        }
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(operator_editor_id(), editor));
}

/// Conversion-style names the selector offers.
///
/// Resolved from the same data directory the pipeline reads, so the selector
/// lists exactly the styles a render would accept. Nothing ships a style, so
/// the installed set is `standard` alone until the user drops a
/// `scan-styles/<name>.json` into their data directory.
fn conversion_styles(data_dir: &Path) -> Vec<String> {
    crate::profiles::scan_conversion_styles(data_dir)
}

/// One `show()` section group, split out of the dispatcher for readability.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    extras: &mut Value,
    film_medium: ScanMedium,
    section: &str,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
    capture_reference_illuminant: &str,
    data_dir: &Path,
) {
    if section == "Input" {
        section_header(ui, filter, "Input", |ui, filter| {
            if filter_matches(filter, "input color space") {
                flags.runtime_changed |= choice_tip(
                    ui,
                    "input color space",
                    &mut params.io.input_color_space,
                    COLOR_SPACES,
                    "Color space of the input image, will be internally converted to sRGB and negative values clipped",
                );
            }
            if filter_matches(filter, "apply cctf decoding") {
                flags.runtime_changed |= toggle(
                    ui,
                    "apply cctf decoding",
                    &mut params.io.input_cctf_decoding,
                    "Apply the inverse color component transfer function (CCTF) of the input color space, so the declared encoded samples become linear before the pipeline reads them.",
                );
            }
        });
    }
    if section == "Camera" {
        ui.scope(|ui| {
            let capture_active = matches!(
                params.workflow.route_enum(),
                spektrafilm_core::WorkflowRoute::FilmScan
                    | spektrafilm_core::WorkflowRoute::FilmPrintScan
            );
            if !capture_active {
                ui.label("Capture illuminant inactive on this route; stored values are retained.");
            }
            if filter_matches(filter, "exposure compensation ev") {
                flags.runtime_changed |= param_numeric(ui, "exposure compensation ev", &mut params.camera.exposure_compensation_ev, "camera.exposure_compensation_ev", "Add a bias to the auto-exposure of the camera.");
            }
            if filter_matches(filter, "auto exposure") {
                flags.runtime_changed |= toggle(ui, "auto exposure", &mut params.camera.auto_exposure, "Use the auto-exposure feature of the virtual camera.");
            }
            if filter_matches(filter, "film format mm") {
                flags.runtime_changed |= param_numeric(ui, "film format mm", &mut params.camera.film_format_mm, "camera.film_format_mm", "Exposed full-image long edge in millimeters: 36 for a 24×36 mm frame, 56 for a 56×56 mm frame, 127 for 4×5 inch sheet film. This is not the roll width or the 120 roll-film designation. Physical film effects, including Grain Dye, use this scale. The editor holds to 1-1000 mm; recipes, presets and CLI parameters carry any strictly positive value.");
            }
            if filter_matches(filter, "auto exposure method") {
                flags.runtime_changed |= choice_tip(ui, "auto exposure method", &mut params.camera.auto_exposure_method, &["center_weighted", "matrix", "multi_zone", "partial", "highlight_weighted", "median", "average"], "Metering method used by the virtual camera's auto-exposure.");
            }
            if filter_matches(filter, "color filter") {
                flags.runtime_changed |= choice_tip(ui, "color filter", &mut params.camera.color_filter, &["none", "hoya_x0", "hoya_x1", "hoya_y2", "hoya_ya3", "hoya_r1"], "Camera taking filter held in front of the lens. Its measured spectral transmittance multiplies the incoming light, so it both darkens and color-casts the exposure (e.g. Hoya Y2/YA3 yellow, R1 red, X0/X1 UV/haze). 'none' = no filter.");
            }
            if filter_matches(filter, "capture illuminant") {
                ui.add_enabled_ui(capture_active, |ui| {
                    flags.runtime_changed |= optional_choice_tip(
                        ui,
                        "capture illuminant",
                        &mut params.camera.capture_illuminant,
                        "inherit (film profile)",
                        CAPTURE_ILLUMINANTS,
                        &format!("Scene illuminant for the filming front pass; unset inherits the selected film profile reference illuminant ({capture_reference_illuminant}). Distinct from film sensitivity calibration, scan and enlarger illuminants, and output white point."),
                    );
                });
            }
        });
    }
    if section == "Enlarger" {
        let print_active = params.workflow.route.contains(" > print > ");
        if !print_active {
            ui.label("Enlarger inactive on this route; stored values are retained.");
        }
        ui.add_enabled_ui(print_active, |ui| {
            if filter_matches(filter, "print exposure") {
                flags.runtime_changed |= param_numeric(ui, "print exposure", &mut params.enlarger.print_exposure, "enlarger.print_exposure", "Changes the exposure time set in the virtual enlarger");
            }
            if filter_matches(filter, "print exposure mode") {
                flags.runtime_changed |= enum_choice(ui, "print exposure mode", &mut params.enlarger.print_exposure_mode, &[
                    (PrintExposureMode::Auto, "auto"),
                    (PrintExposureMode::Manual, "manual"),
                ], "Auto normalizes the print exposure so the developed midgray density prints to the paper midgray and follows camera exposure compensation, like a color analyzer or minilab. Manual applies the print exposure time alone, like a test strip.");
            }
            if filter_matches(filter, "print y filter shift") {
                flags.runtime_changed |= param_numeric(ui, "print y filter shift", &mut params.enlarger.y_filter_shift, "enlarger.y_filter_shift", "Y filter shift of the color enlarger from a neutral position, in Kodak CC units");
            }
            if filter_matches(filter, "print m filter shift") {
                flags.runtime_changed |= param_numeric(ui, "print m filter shift", &mut params.enlarger.m_filter_shift, "enlarger.m_filter_shift", "M filter shift of the color enlarger from a neutral position, in Kodak CC units");
            }
            if filter_matches(filter, "print illuminant") {
                flags.runtime_changed |= choice_tip_experimental(ui, "print illuminant", &mut params.enlarger.illuminant, &["TH-KG3", "TH-KG3-L", "K75P", "T", "D50", "D55", "D65", "A", "BB3200", "BB5000", "LED-B3", "LED-B5", "LED-RGB1", "LED-V2", "FL2"], "Print illuminant to simulate. Default is the bundled TH-KG3.");
            }
            if filter_matches(filter, "print exposure ev tracking") {
                flags.runtime_changed |= enum_choice(
                    ui,
                    "print exposure ev tracking",
                    &mut params.settings.print_exposure_ev_tracking,
                    &[
                        (PrintExposureEvTracking::FollowMode, "follow_mode"),
                        (PrintExposureEvTracking::Never, "never"),
                        (PrintExposureEvTracking::Always, "always"),
                    ],
                    "How the print exposure follows camera exposure compensation. follow_mode tracks it in auto mode and does not track it in manual; never disables tracking in auto; always applies the relative compensation factor in manual.",
                );
            }
        });
    }
    if section == "Preflash" {
        preflash_controls(ui, params, flags, filter);
    }
    if section == "Scanner" {
        ui.scope(|ui| {
            let medium = film_medium.for_route(&params.workflow.route);
            let positive_route = medium.positive_output_eligible(&params.workflow.route);
            let medium_label = match medium {
                ScanMedium::None => "Inactive",
                ScanMedium::Paper => "Paper",
                ScanMedium::Slide => "Slide",
                ScanMedium::Negative => "Negative",
            };
            ui.label(
                egui::RichText::new(format!("Scanning: {medium_label}"))
                    .small()
                    .color(theme::FG_MUTED),
            );
            // Product readout illumination is fixed by the material category;
            // only output polarity and readout optics are adjustable here.
            ui.label(
                egui::RichText::new(vocabulary::SCANNER_SEQUENCE)
                    .small()
                    .color(theme::FG_MUTED),
            );
            if medium == ScanMedium::None {
                ui.label("Scanner inactive on this route; stored values are retained.");
            }
            ui.add_enabled_ui(medium != ScanMedium::None, |ui| {
            if filter_matches(filter, vocabulary::SCAN_OUTPUT)
                && ui.add_enabled_ui(positive_route, |ui| choice_tip_entries(
                    ui,
                    vocabulary::SCAN_OUTPUT,
                    &mut params.scanner.scan_output,
                    [
                        ("direct_scan", "Preserve medium polarity"),
                        ("negative_to_positive", "Negative to positive"),
                    ]
                    .into_iter(),
                    vocabulary::SCAN_OUTPUT_HELP,
                )).inner
            {
                flags.runtime_changed = true;
                normalize_scanner_selections(params, film_medium);
            }
            // A conversion style is a property of the negative interpretation,
            // so it is offered only while that interpretation is selected.
            if filter_matches(filter, vocabulary::CONVERSION_STYLE) {
                let conversion_active = positive_route && params.scanner.scan_output == "negative_to_positive";
                if !conversion_active { ui.label("Conversion style inactive until Negative to positive is selected; stored values are retained."); }
                let styles = conversion_styles(data_dir);
                if ui.add_enabled_ui(conversion_active, |ui| choice_tip_entries(
                    ui,
                    vocabulary::CONVERSION_STYLE,
                    &mut params.scanner.conversion_style,
                    styles
                        .iter()
                        .map(|style| (style.as_str(), style.as_str())),
                    vocabulary::CONVERSION_STYLE_HELP,
                )).inner {
                    flags.runtime_changed = true;
                }
            }
            if filter_matches(filter, "lens blur (readout optics)") {
                flags.runtime_changed |= param_numeric(ui, "Lens blur (readout optics)", &mut params.scanner.lens_blur, "scanner.lens_blur", "Gaussian sigma in pixels on linear transmitted or reflected readout light, before negative conversion, gamut processing, sharpening and encoding. Identity points retain this optical order.");
            }
            if filter_matches(filter, "unsharp mask (digital sharpening)") {
                flags.runtime_changed |= param_tuple(ui, "Unsharp mask (digital sharpening)", &mut params.scanner.unsharp_mask, "scanner.unsharp_mask", "Digital scan sharpening after interpretation and output gamut processing: [sigma in pixels, amount].");
            }
            });
            let operator_active = positive_route
                && params.scanner.scan_output == "negative_to_positive"
                && params.scanner.conversion_style == vocabulary::CONVERSION_STYLE_STANDARD;
            operator_controls(ui, params, operator_active, flags, filter);
        });
    }
    if section == "Import Raw" {
        if filter_matches(filter, "white balance") {
            extra_choice(
                ui,
                extras,
                "load_raw",
                "white_balance",
                "white balance",
                "daylight",
                &["as_shot", "daylight", "tungsten", "custom"],
                "Leave at daylight (D65): it is the colorimetric reference the rest of the pipeline assumes, and should not be changed. Do not use it to neutralize a colour cast — fix white balance downstream with the enlarger filters instead. (custom exposes temperature/tint for special cases.)",
            );
        }
        if filter_matches(filter, "temperature") {
            extra_number(
                ui,
                extras,
                "load_raw",
                "temperature",
                "temperature",
                5500.0,
                "extras.load_raw.temperature",
                "Temperature in Kelvin for the custom whitebalance, not used for the other white balance settings",
            );
        }
        if filter_matches(filter, "tint") {
            extra_number(
                ui,
                extras,
                "load_raw",
                "tint",
                "tint",
                1.0,
                "extras.load_raw.tint",
                "Tint value for the custom white balance, not used for the other white balance settings",
            );
        }
        if filter_matches(filter, "lens correction") {
            extra_bool_tip(
                ui,
                extras,
                "load_raw",
                "lens_correction",
                "lens correction",
                false,
                "Apply lens corrections",
            );
        }
    }
    if section == "Camera diffusion" || section == "Enlarger diffusion" {
        ui.scope(|ui| {
            let (diffusion_filter, paths) = if section == "Camera diffusion" {
                (&mut params.camera.diffusion_filter, &CAMERA_DIFFUSION_PATHS)
            } else {
                (
                    &mut params.enlarger.diffusion_filter,
                    &ENLARGER_DIFFUSION_PATHS,
                )
            };
            flags.runtime_changed |= diffusion(ui, section, diffusion_filter, paths, filter);
        });
    }
}

fn preflash_controls(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    // Gate the whole group on its member labels: a search that matches no
    // member hides the inactive notice together with the rows. Keep the list
    // complete for every row this function renders.
    if !filter_matches_any(
        filter,
        &[
            "preflash exposure",
            "preflash y filter shift",
            "preflash m filter shift",
        ],
    ) {
        return;
    }
    let active = params.workflow.route.contains(" > print > ");
    if !active {
        ui.label("Preflash inactive on this route; stored values are retained.");
    }
    ui.add_enabled_ui(active, |ui| {
        if filter_matches(filter, "preflash exposure") {
            flags.runtime_changed |= param_numeric(ui, "preflash exposure", &mut params.enlarger.preflash_exposure, "enlarger.preflash_exposure", "Linear multiplier of the integrated spectral flash, not EV. Zero disables flash and retains its filters. For negative-to-photographic-paper printing, flash can bring weak highlight exposure differences into a responsive part of the paper curve; it cannot recover absent detail. Excess may lower paper white, add fog, alter midtones and change color. Other print materials retain their own polarity.");
        }
        if filter_matches(filter, "preflash y filter shift") {
            flags.runtime_changed |= param_numeric(ui, "preflash y filter shift", &mut params.enlarger.preflash_y_filter_shift, "enlarger.preflash_y_filter_shift", "Independent preflash Yellow shift around the calibrated neutral filter pack, in Kodak CC units.");
        }
        if filter_matches(filter, "preflash m filter shift") {
            flags.runtime_changed |= param_numeric(ui, "preflash m filter shift", &mut params.enlarger.preflash_m_filter_shift, "enlarger.preflash_m_filter_shift", "Independent preflash Magenta shift around the calibrated neutral filter pack, in Kodak CC units.");
        }
    });
}
