use super::*;
use egui::Ui;
use serde_json::Value;

/// One `show()` section group, split out of the dispatcher for readability.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    extras: &mut Value,
    film_medium: ScanMedium,
    section: &str,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    if section == "Input" {
        section_header(ui, filter, "Input", false, |ui, filter| {
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
                    "Apply the inverse cctf transfer function of the color space",
                );
            }
        });
    }
    if section == "Camera" {
        ui.scope(|ui| {
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
            if filter_matches(filter, "print illuminant (experimental)") {
                flags.runtime_changed |= choice_tip(ui, "print illuminant (experimental)", &mut params.enlarger.illuminant, &["TH-KG3", "TH-KG3-L", "K75P", "T", "D50", "D55", "D65", "A", "BB3200", "BB5000", "LED-B3", "LED-B5", "LED-RGB1", "LED-V2", "FL2"], "Print illuminant to simulate. Default is the bundled TH-KG3.");
            }
            if filter_matches_any(
                filter,
                &[
                    "preflash exposure",
                    "preflash y filter shift",
                    "preflash m filter shift",
                ],
            ) {
                preflash_controls(ui, params, flags, filter);
            }
        });
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
                    .color(egui::Color32::GRAY),
            );
            if medium == ScanMedium::None {
                return;
            }
            if matches!(medium, ScanMedium::Slide | ScanMedium::Negative)
                && params.scanner.negative_conversion_method != NegativeConversionMethod::DigitalReference
                && filter_matches(filter, "receiver profile")
            {
                flags.runtime_changed |= enum_choice(ui, "receiver profile", &mut params.scanner.receiver_profile, &[
                    (ReceiverProfile::LegacyColorimetric, "legacy colorimetric"),
                    (ReceiverProfile::LabScanApprox, "lab scan (approximate)"),
                ], "Receiver response for film acquisition. Lab scan uses an approximate, uncalibrated receiver.");
            }
            if positive_route
                && filter_matches(filter, "scan output")
                && choice_tip_entries(
                    ui,
                    "scan output",
                    &mut params.scanner.scan_output,
                    [
                        ("direct_scan", "Preserve medium polarity"),
                        ("negative_to_positive", "Negative to positive"),
                    ]
                    .into_iter(),
                    "Preserve medium polarity retains the medium's native scan polarity. Negative to positive applies calibrated negative conversion and turns off white and black capture correction.",
                )
            {
                if params.scanner.scan_output == "negative_to_positive" {
                    params.scanner.white_correction = false;
                    params.scanner.black_correction = false;
                }
                flags.runtime_changed = true;
                normalize_scanner_selections(params, film_medium);
            }
            if positive_route && params.scanner.scan_output == "negative_to_positive" {
                let methods = [
                    (NegativeConversionMethod::Density, "density"),
                    (NegativeConversionMethod::Separable, "separable"),
                    (NegativeConversionMethod::Coupled, "coupled"),
                    (NegativeConversionMethod::DigitalReference, "Digital Reference (modeled)"),
                ];
                let methods = &methods[..if params.workflow.route == "input > film > scan" { 4 } else { 3 }];
                if filter_matches(filter, "negative conversion")
                    && enum_choice(ui, "Negative conversion method", &mut params.scanner.negative_conversion_method, methods,
                        "Density, separable and coupled reconstruct the selected film. Digital Reference uses a common modeled receiver and fixed positive response to preserve stock differences.")
                {
                    if params.scanner.negative_conversion_method == NegativeConversionMethod::DigitalReference {
                        let defaults = spektrafilm_core::params::ScannerParams::default();
                        params.scanner.receiver_profile = ReceiverProfile::DigitalReference;
                        params.scanner.auto_setup = ScannerAutoSetup::Off;
                        params.scanner.black_point = [0.0; 3];
                        params.scanner.white_point = [1.0; 3];
                        params.scanner.conversion_brightness_ev = defaults.conversion_brightness_ev;
                        params.scanner.conversion_balance_ev = defaults.conversion_balance_ev;
                        params.scanner.conversion_contrast = defaults.conversion_contrast;
                        params.scanner.conversion_dmax = defaults.conversion_dmax;
                    }
                    normalize_scanner_selections(params, film_medium);
                    flags.runtime_changed = true;
                }
                if params.scanner.negative_conversion_method == NegativeConversionMethod::DigitalReference {
                    ui.label("Reference Exposure: 18% neutral (fixed)")
                        .on_hover_text("Common Kodak Endura spectral receiver sensitivity and fixed Kodak Portra 400 dye basis. One reference exposure and clear-film reading normalize each stock; no frame histogram or selected-film inverse is used.");
                } else {
                    if filter_matches(filter, "scanner auto setup") {
                        flags.runtime_changed |= enum_choice(ui, "scanner auto setup", &mut params.scanner.auto_setup, &[
                            (ScannerAutoSetup::Off, "off"),
                            (ScannerAutoSetup::PerFrame, "per_frame"),
                        ], "Derive scanner exposure, balance, and contrast placement from the full input frame.");
                    }
                }
                if filter_matches(filter, "scanner finish") {
                    flags.runtime_changed |= enum_choice(ui, "scanner finish", &mut params.scanner.finish, &[
                        (ScannerFinish::Neutral, "neutral"),
                        (ScannerFinish::LabScan, "lab_scan"),
                    ], "Optional scanner finish after negative conversion.");
                }
                if params.scanner.finish == ScannerFinish::LabScan
                    && filter_matches(filter, "finish strength")
                {
                    flags.runtime_changed |= param_numeric(ui, "finish strength", &mut params.scanner.finish_strength, "scanner.finish_strength", "Scanner finish strength. Zero returns the calibrated scan before finishing.");
                }
                if params.scanner.negative_conversion_method == NegativeConversionMethod::DigitalReference {
                    if filter_matches(filter, "reference exposure ev") {
                        flags.runtime_changed |= param_numeric(ui, "Reference exposure EV", &mut params.scanner.reference_exposure_ev, "scanner.reference_exposure_ev", "Output exposure in stops after the fixed reference response. Zero is neutral; +1 doubles linear light.");
                    }
                    if filter_matches(filter, "reference balance ev") {
                        flags.runtime_changed |= param_tuple(ui, "Reference balance EV", &mut params.scanner.reference_balance_ev, "scanner.reference_balance_ev", "Independent linear-sRGB red, green and blue output gains in stops after the fixed reference response. All zero is neutral.");
                    }
                } else {
                    if filter_matches(filter, "brightness") {
                        flags.runtime_changed |= param_numeric(ui, "brightness", &mut params.scanner.conversion_brightness_ev, "scanner.conversion_brightness_ev", "Negative conversion brightness offset in stops.");
                    }
                    if filter_matches(filter, "balance") {
                        flags.runtime_changed |= param_tuple(ui, "balance", &mut params.scanner.conversion_balance_ev, "scanner.conversion_balance_ev", "Independent red, green, and blue negative conversion exposure offsets in stops.");
                    }
                    if filter_matches(filter, "contrast") {
                        flags.runtime_changed |= param_numeric(ui, "contrast", &mut params.scanner.conversion_contrast, "scanner.conversion_contrast", "Negative conversion logistic contrast; strictly positive.");
                    }
                    if filter_matches(filter, "maximum density") {
                        flags.runtime_changed |= param_numeric(ui, "maximum density", &mut params.scanner.conversion_dmax, "scanner.conversion_dmax", "Negative conversion maximum density; strictly positive.");
                    }
                }
            }
            if (medium == ScanMedium::Slide
                || (positive_route && params.scanner.scan_output == "negative_to_positive"
                    && params.scanner.negative_conversion_method != NegativeConversionMethod::DigitalReference))
                && filter_matches_any(filter, &["film-scan black point", "film-scan white point"])
            {
                flags.runtime_changed |= scan_points(ui, &mut params.scanner.black_point, &mut params.scanner.white_point);
            }
            if filter_matches(filter, "lens blur (readout optics)") {
                flags.runtime_changed |= param_numeric(ui, "Lens blur (readout optics)", &mut params.scanner.lens_blur, "scanner.lens_blur", "Gaussian sigma in pixels on linear transmitted or reflected readout light, before negative conversion, points, correction, finishing, gamut processing, sharpening and encoding. Identity points retain this optical order.");
            }
            if medium.correction_eligible() {
                if filter_matches(filter, "white correction") {
                    flags.runtime_changed |= toggle(ui, "white correction", &mut params.scanner.white_correction, "Enable the scanner white correction applied to the scan output.");
                }
                if filter_matches(filter, "white level") {
                    flags.runtime_changed |= param_numeric(ui, "white level", &mut params.scanner.white_level, "scanner.white_level", "Target white level the scanned medium's white reference luminance maps to when white correction is enabled.");
                }
                if filter_matches(filter, "black correction") {
                    flags.runtime_changed |= toggle(ui, "black correction", &mut params.scanner.black_correction, "Enable the scanner black correction applied to the scan output.");
                }
                if filter_matches(filter, "black level") {
                    flags.runtime_changed |= param_numeric(ui, "black level", &mut params.scanner.black_level, "scanner.black_level", "Target black level the scanned medium's black reference luminance maps to when black correction is enabled.");
                }
            }
            if filter_matches(filter, "unsharp mask (digital sharpening)") {
                flags.runtime_changed |= param_tuple(ui, "Unsharp mask (digital sharpening)", &mut params.scanner.unsharp_mask, "scanner.unsharp_mask", "Digital scan sharpening after interpretation, scanner finish and output gamut processing: [sigma in pixels, amount].");
            }
        });
    }
    if section == "Import Raw" {
        if filter_matches(filter, "white balance") {
            flags.raw_reload |= extra_choice(
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
            flags.raw_reload |= extra_number(
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
            flags.raw_reload |= extra_number(
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
            flags.raw_reload |= extra_bool_tip(
                ui,
                extras,
                "load_raw",
                "lens_correction",
                "lens correction",
                false,
                "Apply lens corrections",
            );
        }
        if filter_matches(filter, "reprocess raw") {
            flags.raw_reload |= ui.button("reprocess raw").clicked();
        }
    }
}
