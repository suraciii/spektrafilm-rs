//! Editors for the pinned experimental GUI sections.
use egui::Ui;
use serde_json::Value;
use spektrafilm_core::params::{
    NegativeConversionMethod, PrintCurvesMorphParams, PrintExposureMode, ReceiverProfile,
    RuntimeParams, ScannerAutoSetup, ScannerFinish,
    grain::{FotufilmMaterial, GrainEngine, GrainV2FilmType, GrainV2Mode},
};
use spektrafilm_core::profile::Profile;
use spektrafilm_model::grain::v2::PROFILE_NAMES;

const COLOR_SPACES: &[&str] = &[
    "sRGB",
    "DCI-P3",
    "Display P3",
    "Adobe RGB (1998)",
    "ITU-R BT.2020",
    "ProPhoto RGB",
    "ACES2065-1",
    "DaVinci Wide Gamut",
    "V-Gamut",
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
    /// Reload the selected RAW with extras.load_raw before rendering it.
    pub raw_reload: bool,
    /// The preview-size Update button was pressed, independently of auto-preview.
    pub preview_requested: bool,
    /// A real controller action was requested from the Convert panel.
    pub action: Option<CalibrationAction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanMedium {
    None,
    Paper,
    Slide,
    Negative,
}

impl ScanMedium {
    pub(crate) fn for_route(self, route: &str) -> Self {
        if route.contains(" > print > scan") {
            Self::Paper
        } else if route.ends_with(" > scan") || route.ends_with(" > scan-minus-base") {
            self
        } else {
            Self::None
        }
    }

    pub(crate) fn positive_output_eligible(self, route: &str) -> bool {
        self == Self::Negative
            && matches!(route, "input > film > scan" | "input > convert-film > scan")
    }

    pub(crate) fn correction_eligible(self) -> bool {
        matches!(self, Self::Paper | Self::Slide)
    }
}

/// Scan medium implied by a film profile: negative, positive (slide), or
/// none when the stock is not a filming-stage film.
pub(crate) fn scan_medium(film: &Profile) -> ScanMedium {
    if film.is_film() && film.is_negative() {
        ScanMedium::Negative
    } else if film.is_film() && film.is_positive() {
        ScanMedium::Slide
    } else {
        ScanMedium::None
    }
}

/// Normalize only GUI transitions; imported states and recipes retain validation errors.
pub(crate) fn normalize_scanner_selections(params: &mut RuntimeParams, film_medium: ScanMedium) {
    let medium = film_medium.for_route(&params.workflow.route);
    if params.scanner.negative_conversion_method == NegativeConversionMethod::DigitalReference
        && (film_medium != ScanMedium::Negative
            || params.workflow.route != "input > film > scan"
            || params.scanner.scan_output != "negative_to_positive")
    {
        params.scanner.negative_conversion_method = NegativeConversionMethod::default();
    }
    if params.scanner.negative_conversion_method != NegativeConversionMethod::DigitalReference {
        if params.scanner.receiver_profile == ReceiverProfile::DigitalReference {
            params.scanner.receiver_profile = ReceiverProfile::LegacyColorimetric;
        }
        params.scanner.reference_exposure_ev = 0.0;
        params.scanner.reference_balance_ev = [0.0; 3];
    }
    if !matches!(medium, ScanMedium::Slide | ScanMedium::Negative) {
        params.scanner.receiver_profile = ReceiverProfile::LegacyColorimetric;
    }
    if !film_medium.positive_output_eligible(&params.workflow.route) {
        params.scanner.scan_output = "direct_scan".into();
    }
    if params.scanner.scan_output != "negative_to_positive" {
        params.scanner.negative_conversion_method = NegativeConversionMethod::default();
        params.scanner.auto_setup = ScannerAutoSetup::Off;
        params.scanner.finish = ScannerFinish::Neutral;
    }
}

pub(crate) fn enum_choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    label: &str,
    value: &mut T,
    choices: &[(T, &str)],
    tooltip: &str,
) -> bool {
    let selected = choices
        .iter()
        .find(|(option, _)| *option == *value)
        .map(|(_, label)| *label)
        .unwrap_or("");
    let prev = *value;
    // Label first, selector second, so a choice row reads like every other
    // parameter row. The selector paints in its own horizontal row inside a
    // vertical section body, so both share one explicit row here.
    ui.horizontal(|ui| {
        ui.label(label).on_hover_text(tooltip);
        egui::ComboBox::from_id_salt(label)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for &(option, label) in choices {
                    ui.selectable_value(value, option, label);
                }
            })
            .response
            .on_hover_text(tooltip);
        // The selector carries the parameter's own accessible name.
        widgets::name_choice(ui, label);
    });
    prev != *value
}

/// A section or sub-section header. During a control search the body is
/// revealed instead of collapsed, and a section whose controls do not match is
/// dropped entirely.
pub(crate) fn section_header<R>(
    ui: &mut Ui,
    filter: &mut ControlFilter<'_>,
    title: &str,
    add_contents: impl FnOnce(&mut Ui, &mut ControlFilter<'_>) -> R,
) {
    if filter.active() {
        if !filter.section_matches() {
            return;
        }
        ui.label(
            egui::RichText::new(title)
                .color(theme::FG)
                .size(theme::SIZE_TITLE)
                .strong(),
        );
        add_contents(ui, filter);
        return;
    }
    let response = egui::CollapsingHeader::new(
        egui::RichText::new(title)
            .color(theme::FG)
            .size(theme::SIZE_TITLE)
            .strong(),
    )
    .show(ui, |ui| add_contents(ui, filter));
    // A closed sub-section states what it holds, so a page of collapsed
    // headers still names its controls. Display-only: it reads no value.
    if response.openness <= 0.0
        && let Some(summary) = section_summary(title)
    {
        ui.label(egui::RichText::new(summary).color(theme::FG_MUTED).small());
    }
}

/// What a closed sub-section holds, in one line naming its own controls.
///
/// Keyed by the header title the sub-section renders, so the wording stays with
/// the section that owns it. A sub-section without an entry shows its header
/// alone.
fn section_summary(title: &str) -> Option<&'static str> {
    match title {
        "Base" => Some("Base density: scale and per-channel masks"),
        "Print viewing glare" => Some("Print viewing glare: percent, roughness, blur"),
        "Input gamut compress" => Some("Input gamut compression: algorithm, knee, hull detail"),
        "Output gamut compress" => Some("Output gamut compression: algorithm, knee"),
        "Spectral upsampling" => Some("Spectral reconstruction: method, adaptation, blur"),
        "Input" => Some("Input decoding: color space and cctf"),
        "Crop and upscale" => Some("Geometry: crop toggle, center, size, upscale factor"),
        _ => None,
    }
}

pub(crate) fn show(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    extras: &mut Value,
    section: &str,
    film_medium: ScanMedium,
    filter: &str,
    cache: &mut SectionLabelCache,
) -> ChangeFlags {
    let mut flags = ChangeFlags::default();
    let _row_scope = row_modified::install(ui.ctx(), section, params, extras);
    let mut filter = ControlFilter::new(filter, cache);
    ui.push_id(("supplemental_controls", section), |ui| {
        render_sections(
            ui,
            params,
            extras,
            section,
            film_medium,
            &mut flags,
            &mut filter,
        );
    });
    filter.finish();
    flags
}

pub(crate) fn render_sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    extras: &mut Value,
    section: &str,
    film_medium: ScanMedium,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    scanner::sections(ui, params, extras, film_medium, section, flags, filter);
    chemistry::sections(ui, params, section, flags, filter);
    glare::sections(ui, params, section, flags, filter);
    gamut::sections(ui, params, section, flags, filter);
    crop::sections(ui, params, section, flags, filter);
    base::sections(ui, params, section, flags, filter);
    convert::sections(ui, params, extras, section, flags, filter);
    upsampling::sections(ui, params, section, flags, filter);
    grain::sections(ui, params, section, flags, filter);
    halation::sections(ui, params, section, flags, filter);
    couplers::sections(ui, params, section, flags, filter);
    magazine::sections(ui, params, section, flags, filter);
    output::sections(ui, params, extras, section, flags, filter);
    display::sections(ui, params, extras, section, flags, filter);
}

mod base;
mod chemistry;
mod convert;
mod couplers;
mod crop;
mod display;
mod filter;
pub(crate) use filter::{
    ControlFilter, SectionLabelCache, cached_labels_match, filter_matches, filter_matches_any,
    label_matches,
};
pub(crate) mod row_modified;
/// One item of a page's control panel, in the order the page renders it.
///
/// The table is the single source of a page's layout: the panel renders from
/// it and the cross-page search index derives the searchable sections from it,
/// so a section added to a page is searchable on the same change.
pub(crate) enum PanelItem {
    /// A parameter section, rendered by `ui::App::parameter_section`.
    Section(&'static str),
    /// The chemistry section of one medium: film or print.
    Chemistry { film: bool },
    /// One intake surface: false is RGB, true is RAW.
    Import { raw: bool },
    /// The profile selectors of the main page.
    Profiles,
    /// The export actions of the main page.
    ExportOptions,
    /// The diagnostics readout of the configuration page.
    Diagnostics,
    /// The GUI parameter toolbar of the configuration page.
    GuiParameters,
    /// The display settings of the configuration page.
    DisplaySettings,
}

impl PanelItem {
    /// The parameter section this item renders, when it renders one.
    ///
    /// The Display item renders its parameter section inside its own
    /// collapsible, so it owns one as well; the import surface's RAW editor is
    /// excluded by the caller.
    pub(crate) fn section(&self) -> Option<&'static str> {
        match self {
            Self::Section(section) => Some(section),
            Self::Chemistry { film: true } => Some("Film chemistry"),
            Self::Chemistry { film: false } => Some("Print chemistry"),
            Self::Import { raw: true } => Some("Import Raw"),
            Self::DisplaySettings => Some("Display"),
            _ => None,
        }
    }
}

/// Every page, with the items its control panel renders and in which order.
pub(crate) const PAGE_LAYOUT: &[(crate::ui::GuiTab, &[PanelItem])] = &[
    (
        crate::ui::GuiTab::Main,
        &[
            PanelItem::Import { raw: false },
            PanelItem::Import { raw: true },
            PanelItem::Section("Crop and upscale"),
            PanelItem::Section("Input"),
            PanelItem::Section("Camera"),
            PanelItem::Profiles,
            PanelItem::Section("Enlarger"),
            PanelItem::Section("Scanner"),
            PanelItem::Section("Output"),
            PanelItem::ExportOptions,
        ],
    ),
    (
        crate::ui::GuiTab::Film,
        &[
            PanelItem::Chemistry { film: true },
            PanelItem::Section("Film base"),
            PanelItem::Section("Halation"),
            PanelItem::Section("Couplers"),
            PanelItem::Section("Grain"),
            PanelItem::Section("Camera diffusion"),
            PanelItem::Section("Convert"),
        ],
    ),
    (
        crate::ui::GuiTab::Print,
        &[
            PanelItem::Chemistry { film: false },
            PanelItem::Section("Print base"),
            PanelItem::Section("Glare"),
            PanelItem::Section("Enlarger diffusion"),
            PanelItem::Section("Magazine finish color"),
        ],
    ),
    (
        crate::ui::GuiTab::Advanced,
        &[
            PanelItem::Section("Spectral upsampling"),
            PanelItem::Section("Input gamut compress"),
            PanelItem::Section("Output gamut compress"),
        ],
    ),
    (
        crate::ui::GuiTab::Config,
        &[
            PanelItem::Diagnostics,
            PanelItem::GuiParameters,
            PanelItem::DisplaySettings,
        ],
    ),
];
mod gamut;
mod glare;
mod grain;
mod halation;
mod magazine;
mod output;
mod scanner;
mod search;
mod upsampling;
mod widgets;
pub(crate) use search::SearchIndex;
pub(crate) use widgets::*;

// Every control editor reads presentation tokens; re-export so a control file
// does not each add its own import.
pub(crate) use crate::theme;

#[cfg(test)]
#[path = "controls_acceptance.rs"]
mod acceptance;

#[cfg(test)]
mod tests {
    use super::*;

    fn scanner_frame(
        ctx: &egui::Context,
        params: &mut RuntimeParams,
        medium: ScanMedium,
        events: Vec<egui::Event>,
    ) -> (ChangeFlags, egui::FullOutput) {
        let mut flags = ChangeFlags::default();
        let mut cache = SectionLabelCache::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 1600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    flags = show(
                        ui,
                        params,
                        &mut serde_json::json!({}),
                        "Scanner",
                        medium,
                        "",
                        &mut cache,
                    );
                });
            },
        );
        (flags, output)
    }

    fn has_text(output: &egui::FullOutput, text: &str) -> bool {
        output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(shape) if shape.galley.job.text == text))
    }

    #[test]
    fn scanner_controls_follow_route_medium_and_output() {
        for (route, medium, positive) in [
            ("input", ScanMedium::Negative, false),
            ("input > film > print > scan", ScanMedium::Negative, false),
            ("input > film > scan", ScanMedium::Slide, false),
            ("input > film > scan", ScanMedium::Negative, false),
            ("input > film > scan", ScanMedium::Negative, true),
            ("input > convert-film > scan", ScanMedium::Negative, true),
            (
                "input > convert-film > scan-minus-base",
                ScanMedium::Negative,
                false,
            ),
        ] {
            let ctx = egui::Context::default();
            let mut params = RuntimeParams::default();
            params.workflow.route = route.into();
            params.scanner.scan_output = if positive {
                "negative_to_positive"
            } else {
                "direct_scan"
            }
            .into();
            params.scanner.finish = if positive {
                ScannerFinish::LabScan
            } else {
                ScannerFinish::Neutral
            };
            let (_, output) = scanner_frame(&ctx, &mut params, medium, vec![]);
            assert_eq!(
                has_text(&output, "receiver profile"),
                route != "input" && !route.contains(" > print > "),
                "{route}"
            );
            for label in [
                "Negative conversion method",
                "scanner auto setup",
                "scanner finish",
                "finish strength",
            ] {
                assert_eq!(has_text(&output, label), positive, "{route}: {label}");
            }
            assert_eq!(
                has_text(&output, "scan output"),
                medium.positive_output_eligible(route),
                "{route}"
            );
            if positive {
                params.scanner.finish = ScannerFinish::Neutral;
                let (_, output) = scanner_frame(&ctx, &mut params, medium, vec![]);
                assert!(!has_text(&output, "finish strength"));
            }
        }
    }

    #[test]
    fn approximate_receiver_and_coupled_choices_are_clickable() {
        let ctx = egui::Context::default();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.scanner.scan_output = "negative_to_positive".into();
        for (selected, next) in [
            ("legacy colorimetric", "lab scan (approximate)"),
            ("density", "separable"),
            ("separable", "coupled"),
            ("off", "per_frame"),
            ("neutral", "lab_scan"),
        ] {
            let (_, output) = scanner_frame(&ctx, &mut params, ScanMedium::Negative, vec![]);
            let position = text_position(&output, selected);
            scanner_frame(
                &ctx,
                &mut params,
                ScanMedium::Negative,
                click(position, true),
            );
            scanner_frame(
                &ctx,
                &mut params,
                ScanMedium::Negative,
                click(position, false),
            );
            // Popup areas lay out their first frame before becoming visible and hittable.
            let (_, output) = scanner_frame(&ctx, &mut params, ScanMedium::Negative, vec![]);
            let position = text_position(&output, next);
            scanner_frame(
                &ctx,
                &mut params,
                ScanMedium::Negative,
                click(position, true),
            );
            let (flags, _) = scanner_frame(
                &ctx,
                &mut params,
                ScanMedium::Negative,
                click(position, false),
            );
            assert!(flags.runtime_changed, "{next}");
            scanner_frame(&ctx, &mut params, ScanMedium::Negative, vec![]);
        }
        assert_eq!(
            params.scanner.receiver_profile,
            ReceiverProfile::LabScanApprox
        );
        assert_eq!(
            params.scanner.negative_conversion_method,
            NegativeConversionMethod::Coupled
        );
        assert_eq!(params.scanner.auto_setup, ScannerAutoSetup::PerFrame);
        assert_eq!(params.scanner.finish, ScannerFinish::LabScan);
    }

    #[test]
    fn direct_scan_selection_resets_positive_only_selectors() {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.scanner.receiver_profile = ReceiverProfile::LabScanApprox;
        params.scanner.negative_conversion_method = NegativeConversionMethod::Coupled;
        params.scanner.auto_setup = ScannerAutoSetup::PerFrame;
        params.scanner.finish = ScannerFinish::LabScan;
        params.scanner.finish_strength = 0.0;
        normalize_scanner_selections(&mut params, ScanMedium::Negative);
        assert_eq!(
            params.scanner.receiver_profile,
            ReceiverProfile::LabScanApprox
        );
        assert_eq!(
            params.scanner.negative_conversion_method,
            NegativeConversionMethod::Density
        );
        assert_eq!(params.scanner.auto_setup, ScannerAutoSetup::Off);
        assert_eq!(params.scanner.finish, ScannerFinish::Neutral);
        assert_eq!(params.scanner.finish_strength, 0.0);
    }

    fn click(position: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    fn text_position(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(shape) if shape.galley.job.text == text => {
                    Some(shape.pos + shape.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("rendered control text {text:?}"))
    }

    /// Position of an option inside an open combo popup.
    ///
    /// A popup paints in the foreground layer, after the panel that opened it,
    /// so its option text is the last matching shape. Panel content can share
    /// an option's label — a grain engine group header carries the engine's
    /// name — and that label paints first, so the last match is the popup's.
    fn popup_text_position(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(shape) if shape.galley.job.text == text => {
                    Some(shape.pos + shape.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("rendered popup text {text:?}"))
    }
    fn replace_text(text: &str) -> Vec<egui::Event> {
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            },
            egui::Event::Text(text.into()),
        ]
    }

    #[test]
    fn fotufilm_amount_widget_edits_native_runtime_without_touching_dye() {
        let ctx = egui::Context::default();
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Fotufilm;
        let old_dye_support = params.film_render.grain.v3_dye_support_um;
        let mut frame = |events| {
            let mut flags = ChangeFlags::default();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 1600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        flags = show(
                            ui,
                            &mut params,
                            &mut serde_json::json!({}),
                            "Grain",
                            ScanMedium::Negative,
                            "",
                            &mut Default::default(),
                        );
                    });
                },
            );
            (
                flags.runtime_changed,
                params.film_render.grain.clone(),
                output,
            )
        };
        let (_, _, output) = frame(vec![]);
        assert!(has_text(&output, "Fotufilm Film"));
        assert!(has_text(&output, "Softness"));
        assert!(has_text(&output, "Layer amounts"));
        assert!(!has_text(&output, "Dye support (µm)"));
        let position = text_position(&output, "1.00");
        frame(click(position, true));
        frame(click(position, false));
        let (changed, grain, _) = frame(replace_text("2.25"));
        assert!(changed);
        assert_eq!(grain.fotufilm.amount, 2.25);
        assert_eq!(grain.v3_dye_support_um, old_dye_support);
    }
    #[test]
    fn fotufilm_selector_switches_real_widget_and_replaces_dye_controls() {
        let ctx = egui::Context::default();
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Dye;
        let mut frame = |events| {
            let mut flags = ChangeFlags::default();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 3000.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        flags = show(
                            ui,
                            &mut params,
                            &mut serde_json::json!({}),
                            "Grain",
                            ScanMedium::Negative,
                            "",
                            &mut Default::default(),
                        );
                    });
                },
            );
            (flags, output)
        };

        let (_, initial) = frame(vec![]);
        assert!(has_text(&initial, "Dye support (µm)"));
        assert!(!has_text(&initial, "Layer amounts"));
        let selector = text_position(&initial, "Dye");
        frame(click(selector, true));
        frame(click(selector, false));
        let (_, popup) = frame(vec![]);
        assert!(has_text(&popup, "Fotufilm Film"));
        let fotufilm = popup_text_position(&popup, "Fotufilm Film");
        frame(click(fotufilm, true));
        let (flags, _) = frame(click(fotufilm, false));
        assert!(flags.runtime_changed);
        // The accordion animates the selected engine's group open and the
        // previous one closed; a mid-animation frame still clips the tail of
        // the opening group and the body of the closing one, so wait for the
        // layout to settle before reading the controls.
        let mut selected = frame(vec![]).1;
        for _ in 0..8 {
            selected = frame(vec![]).1;
        }
        let advanced = text_position(&selected, "Fotufilm advanced");
        frame(click(advanced, true));
        frame(click(advanced, false));
        let (_, selected) = frame(vec![]);
        for label in [
            "Amount",
            "Size",
            "Softness",
            "Color",
            "Layer amounts",
            "RMS density",
        ] {
            assert!(
                has_text(&selected, label),
                "missing Fotufilm control: {label}"
            );
        }
        assert!(has_text(&selected, "RMS density"));
        assert!(!has_text(&selected, "Dye support (µm)"));
    }

    #[test]
    fn fotufilm_share_editor_keeps_total_one() {
        let ctx = egui::Context::default();
        let mut shares = [0.25; 4];
        let mut frame = |events| {
            let mut changed = false;
            let output = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut cache = SectionLabelCache::default();
                        let mut filter = ControlFilter::new("", &mut cache);
                        changed = fotufilm_sublayer_shares(ui, &mut shares, &mut filter);
                    });
                },
            );
            (changed, shares, output)
        };
        let (_, _, output) = frame(vec![]);
        let position = text_position(&output, "0.250000");
        frame(click(position, true));
        frame(click(position, false));
        let (changed, shares, _) = frame(replace_text("0.4"));
        assert!(changed);
        assert_eq!(shares[0], 0.4);
        assert!((shares[3] - 0.1).abs() < 1e-12);
        assert!((shares.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(shares.iter().all(|share| *share > 0.0));
    }

    #[test]
    fn point_text_edits_commit_only_valid_channel_pairs() {
        let ctx = egui::Context::default();
        let mut black = [0.1, 0.2, 0.3];
        let mut white = [0.7, 0.8, 0.9];
        let mut frame = |events: Vec<egui::Event>| {
            let mut changed = false;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut cache = SectionLabelCache::default();
                        let mut filter = ControlFilter::new("", &mut cache);
                        changed = scan_points(ui, &mut black, &mut white, &mut filter);
                    });
                },
            );
            (changed, black, white, output)
        };
        let (_, _, _, output) = frame(vec![]);
        let position = text_position(&output, "0.100");
        frame(click(position, true));
        frame(click(position, false));
        for text in ["0.7", "0.95", "NaN", "-"] {
            let (changed, black, white, _) = frame(replace_text(text));
            assert!(!changed, "{text}");
            assert_eq!(black, [0.1, 0.2, 0.3]);
            assert_eq!(white, [0.7, 0.8, 0.9]);
        }
        let (changed, black, white, _) = frame(replace_text("0.25"));
        assert!(changed);
        assert_eq!(black, [0.25, 0.2, 0.3]);
        assert_eq!(white, [0.7, 0.8, 0.9]);
    }

    #[test]
    fn rejected_point_edit_preserves_last_committed_value() {
        let ctx = egui::Context::default();
        let mut black = [0.1, 0.2, 0.3];
        let mut white = [0.7, 0.8, 0.9];
        let mut frame = |events: Vec<egui::Event>| {
            let mut changed = false;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut cache = SectionLabelCache::default();
                        let mut filter = ControlFilter::new("", &mut cache);
                        changed = scan_points(ui, &mut black, &mut white, &mut filter);
                    });
                },
            );
            (changed, black, white, output)
        };
        let (_, _, _, output) = frame(vec![]);
        let position = text_position(&output, "0.100");
        frame(click(position, true));
        frame(click(position, false));
        let (changed, black, _, _) = frame(replace_text("0.250"));
        assert!(changed);
        assert_eq!(black[0], 0.25);
        let (changed, black, _, _) = frame(replace_text("0.700"));
        assert!(!changed);
        assert_eq!(black[0], 0.25);
    }

    #[test]
    fn white_point_draft_survives_sequential_typing_and_external_changes() {
        let ctx = egui::Context::default();
        let mut black = [0.1; 3];
        let mut white = [1.0; 3];
        let mut frame = |events: Vec<egui::Event>, external: Option<f64>| {
            if let Some(value) = external {
                white[0] = value;
            }
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut cache = SectionLabelCache::default();
                        let mut filter = ControlFilter::new("", &mut cache);
                        scan_points(ui, &mut black, &mut white, &mut filter);
                    });
                },
            );
            (black, white, output)
        };
        let (_, _, output) = frame(vec![], None);
        let position = text_position(&output, "1.000");
        frame(click(position, true), None);
        frame(click(position, false), None);
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        };
        let (_, white, _) = frame(vec![select_all, egui::Event::Text("0".into())], None);
        assert_eq!(white, [1.0; 3]);
        let (_, white, _) = frame(vec![egui::Event::Text(".".into())], None);
        assert_eq!(white, [1.0; 3]);
        let (_, white, _) = frame(vec![egui::Event::Text("9".into())], None);
        assert_eq!(white, [0.9, 1.0, 1.0]);
        let (black, white, _) = frame(vec![egui::Event::Text("5".into())], None);
        assert_eq!(white, [0.95, 1.0, 1.0]);
        assert_eq!(black, [0.1; 3]);
        frame(vec![], Some(0.8));
        let (_, white, _) = frame(
            vec![
                egui::Event::Key {
                    key: egui::Key::End,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::Text("5".into()),
            ],
            None,
        );
        assert_eq!(white, [0.8005, 1.0, 1.0]);
    }
}
