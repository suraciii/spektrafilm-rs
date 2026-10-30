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
    if !active {
        ui.label(localized(ui.ctx(), "Inactive: requires a negative film, an eligible negative-to-positive route and standard conversion style. Stored corrections are retained."));
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
                "Finished-positive exposure in stops. Positive values brighten the result before the fixed tone curve; this does not change camera exposure or film density.");
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
                let response = ui.add_enabled(*operator != ScanOperatorParams::default(), egui::Button::new(localized(ui.ctx(), "Restore automatic")))
                    .on_hover_text(localized(ui.ctx(), "Reset the four operator values only; keep this frame's own automatic setup."));
                focus_response(ui, "Restore automatic", &response);
                if response.clicked() {
                    *operator = ScanOperatorParams::default();
                }
            }
            if filter_matches(filter, "copy operator correction") {
                let response = ui.button(localized(ui.ctx(), "Copy operator correction"))
                    .on_hover_text(localized(ui.ctx(), "Copy only the four operator values for this session, not automatic setup, film density or capture settings."));
                focus_response(ui, "Copy operator correction", &response);
                if response.clicked() {
                    editor.clipboard = Some(*operator);
                }
            }
            if filter_matches(filter, "paste operator correction") {
                let response = ui.add_enabled(editor.clipboard.is_some(), egui::Button::new(localized(ui.ctx(), "Paste operator correction")))
                    .on_hover_text(localized(ui.ctx(), "Apply the copied four values to this frame's own automatic setup."));
                focus_response(ui, "Paste operator correction", &response);
                if response.clicked()
                    && let Some(copied) = editor.clipboard
                {
                    *operator = copied;
                }
            }
            if filter_matches(filter, "hold automatic comparison") {
                let response = ui.button(localized(ui.ctx(), "Hold automatic comparison"))
                    .on_hover_text(localized(ui.ctx(), "Hold the primary mouse button or focused Space to view this same frame with default operator values. Release to restore the corrected view. Saved and exported corrections are unchanged."));
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
            let capture_active = crate::ui::control_inactive_reason(ui.ctx(), "camera.capture_illuminant", params).is_none();
            if !capture_active {
                ui.label(localized(ui.ctx(), "Capture illuminant inactive on this route; stored values are retained."));
            }
            if filter_matches(filter, "exposure compensation ev") {
                flags.runtime_changed |= applicable_control(ui, params, "camera.exposure_compensation_ev", |ui, params| param_numeric(ui, "exposure compensation ev", &mut params.camera.exposure_compensation_ev, "camera.exposure_compensation_ev", "Add a bias to the auto-exposure of the camera."));
            }
            if filter_matches(filter, "auto exposure") {
                flags.runtime_changed |= applicable_control(ui, params, "camera.auto_exposure", |ui, params| toggle(ui, "auto exposure", &mut params.camera.auto_exposure, "Use the auto-exposure feature of the virtual camera."));
            }
            if filter_matches(filter, "Film Format") {
                flags.runtime_changed |= film_format(ui, &mut params.camera.film_format_mm);
            }
            if filter_matches(filter, "auto exposure method") {
                flags.runtime_changed |= applicable_control(ui, params, "camera.auto_exposure_method", |ui, params| choice_tip(ui, "auto exposure method", &mut params.camera.auto_exposure_method, &["center_weighted", "matrix", "multi_zone", "partial", "highlight_weighted", "median", "average"], "Metering method used by the virtual camera's auto-exposure."));
            }
            if filter_matches(filter, "color filter") {
                flags.runtime_changed |= applicable_control(ui, params, "camera.color_filter", |ui, params| choice_tip(ui, "color filter", &mut params.camera.color_filter, &["none", "hoya_x0", "hoya_x1", "hoya_y2", "hoya_ya3", "hoya_r1"], "Camera taking filter held in front of the lens. Its measured spectral transmittance multiplies the incoming light, so it both darkens and color-casts the exposure (e.g. Hoya Y2/YA3 yellow, R1 red, X0/X1 UV/haze). 'none' = no filter."));
            }
            if filter_matches(filter, "capture illuminant") {
                ui.add_enabled_ui(capture_active, |ui| {
                    let help = crate::tr!(ui.ctx(), "controls.scanner.capture_illuminant_help", reference = capture_reference_illuminant);
                    let inherited = params.camera.capture_illuminant.is_none().then(|| {
                        crate::tr!(ui.ctx(), "controls.scanner.capture_inherit_effective", reference = capture_reference_illuminant)
                    });
                    flags.runtime_changed |= optional_choice_tip(
                        ui,
                        "capture illuminant",
                        &mut params.camera.capture_illuminant,
                        "inherit (film profile)",
                        inherited.as_deref(),
                        CAPTURE_ILLUMINANTS,
                        &help,
                    );
                });
            }
        });
    }
    if section == "Enlarger" {
        let print_active =
            crate::ui::control_inactive_reason(ui.ctx(), "enlarger.print_exposure", params)
                .is_none();
        if !print_active {
            ui.label(localized(
                ui.ctx(),
                "Enlarger inactive on this route; stored values are retained.",
            ));
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
            let scan_active = crate::ui::control_inactive_reason(ui.ctx(), "scanner.scan_output", params).is_none();
            let medium = if scan_active { film_medium.for_route(&params.workflow.route) } else { ScanMedium::None };
            let positive_route = scan_active && medium.positive_output_eligible(&params.workflow.route);
            let medium_label = match medium {
                ScanMedium::None => "Inactive",
                ScanMedium::Paper => "Paper",
                ScanMedium::Slide => "Slide",
                ScanMedium::Negative => "Negative",
            };
            ui.label(
                egui::RichText::new(crate::tr!(ui.ctx(), "controls.scanner.scanning_medium", medium = localized(ui.ctx(), medium_label)))
                    .small()
                    .color(theme::FG_MUTED),
            );
            // Product readout illumination is fixed by the material category;
            // only output polarity and readout optics are adjustable here.
            ui.label(
                egui::RichText::new(localized(ui.ctx(), vocabulary::SCANNER_SEQUENCE))
                    .small()
                    .color(theme::FG_MUTED),
            );
            if medium == ScanMedium::None {
                ui.label(localized(ui.ctx(), "Scanner inactive on this route; stored values are retained."));
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
                if !conversion_active { ui.label(localized(ui.ctx(), "Conversion style inactive until Negative to positive is selected; stored values are retained.")); }
                let styles = conversion_styles(data_dir);
                if ui.add_enabled_ui(conversion_active, |ui| widgets::choice_tip_profile_entries(
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
            // The whole-frame measurement is an off-by-default experiment, so it
            // is offered next to the style it modifies rather than as a mode.
            let negative_route =
                positive_route && params.scanner.scan_output == "negative_to_positive";
            if filter_matches(filter, vocabulary::EXPERIMENTAL_SETUP) {
                ui.add_enabled_ui(negative_route, |ui| {
                    let changed = toggle(
                        ui,
                        vocabulary::EXPERIMENTAL_SETUP,
                        &mut params.scanner.experimental_setup,
                        vocabulary::EXPERIMENTAL_SETUP_HELP,
                    );
                    flags.runtime_changed |= changed;
                });
            }
            if filter_matches(filter, "lens blur (readout optics)") {
                flags.runtime_changed |= param_numeric(ui, "Lens blur (readout optics)", &mut params.scanner.lens_blur, "scanner.lens_blur", "Gaussian sigma in pixels on linear transmitted or reflected readout light, before negative conversion, gamut processing, sharpening and encoding. Identity points retain this optical order.");
            }
            if filter_matches(filter, "unsharp mask (digital sharpening)") {
                flags.runtime_changed |= param_tuple(ui, "Unsharp mask (digital sharpening)", &mut params.scanner.unsharp_mask, "scanner.unsharp_mask", "Digital scan sharpening after interpretation and output gamut processing: [sigma in pixels, amount].");
            }
            });
            let operator_active = positive_route && params.scanner.operator_eligible();
            section_header(ui, filter, "Negative conversion (exposure domain)", |ui, filter| {
                operator_controls(ui, params, operator_active, flags, filter);
            });
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
        let path = if section == "Camera diffusion" {
            "camera.diffusion_filter.active"
        } else {
            "enlarger.diffusion_filter.active"
        };
        let inactive = crate::ui::control_inactive_reason(ui.ctx(), path, params);
        if let Some(reason) = &inactive {
            ui.label(localized(ui.ctx(), reason));
        }
        ui.add_enabled_ui(inactive.is_none(), |ui| {
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

/// Names are film specifications; values remain exposed-image long edges.
fn film_format<T: egui::emath::Numeric>(ui: &mut Ui, value: &mut T) -> bool {
    const PATH: &str = "camera.film_format_mm";
    const FORMATS: [(&str, f64); 3] = [("35mm", 36.0), ("120", 56.0), ("4×5", 127.0)];
    const HELP: &str = "Sets the physical film scale for grain and other film effects. Does not crop, resize, or change the image's aspect ratio. 35mm uses a 36 mm frame long edge; 120 uses a representative 56 mm; 4×5 uses 127 mm.";
    if !row_path_matches(ui.ctx(), PATH) {
        return false;
    }
    let _identity = row_identity(ui.ctx(), PATH);
    let current = value.to_f64();
    let preset = FORMATS.iter().position(|(_, mm)| *mm == current);
    let id = ui.make_persistent_id("film-format-custom");
    let mut custom =
        ui.ctx().data(|data| data.get_temp::<f64>(id)) == Some(current) || preset.is_none();
    let mut selected = preset;
    let mark = row_modified::RowMark::resolve(ui.ctx(), Some(PATH));
    ui.horizontal(|ui| {
        let label =
            widgets::row_label(ui, "Film Format", false).on_hover_text(localized(ui.ctx(), HELP));
        mark.paint(ui, &label);
        let width = ui
            .available_width()
            .clamp(theme::WIDTH_COMBO_MIN, theme::WIDTH_COMBO_MAX);
        ui.add_space((ui.available_width() - width).max(0.0));
        let response = mark.gold(ui, |ui| {
            egui::ComboBox::from_id_salt("Film Format")
                .selected_text(if custom {
                    localized(ui.ctx(), "Custom")
                } else {
                    std::borrow::Cow::Borrowed(FORMATS[preset.unwrap()].0)
                })
                .width(width)
                .show_ui(ui, |ui| {
                    for (index, (name, _)) in FORMATS.iter().enumerate() {
                        if ui
                            .selectable_label(!custom && preset == Some(index), *name)
                            .clicked()
                        {
                            selected = Some(index);
                            custom = false;
                        }
                    }
                    if ui
                        .push_id("Custom", |ui| {
                            ui.selectable_label(custom, localized(ui.ctx(), "Custom"))
                        })
                        .inner
                        .clicked()
                    {
                        custom = true;
                    }
                })
                .response
                .on_hover_text(localized(ui.ctx(), HELP))
        });
        focus_response(ui, "Film Format", &response);
        widgets::name_choice(ui, "Film Format");
    });
    let mut changed = false;
    if !custom && let Some(index) = selected {
        let mm = FORMATS[index].1;
        if current != mm && row_modified::accepts_value(ui.ctx(), PATH, Value::from(mm)) {
            *value = T::from_f64(mm);
            record_edited_path(ui.ctx(), PATH);
            changed = true;
        }
    }
    if custom {
        changed |= param_numeric(ui, "Frame Long Edge", value, PATH, HELP);
    }
    ui.ctx().data_mut(|data| {
        if custom {
            data.insert_temp(id, value.to_f64());
        } else {
            data.remove::<f64>(id);
        }
    });
    changed
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
    let active = crate::ui::control_inactive_reason(ui.ctx(), "enlarger.preflash_exposure", params)
        .is_none();
    if !active {
        ui.label(localized(
            ui.ctx(),
            "Preflash inactive on this route; stored values are retained.",
        ));
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
