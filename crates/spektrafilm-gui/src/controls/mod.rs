//! Editors for the pinned experimental GUI sections.
use crate::routes::vocabulary;
use crate::theme;
use egui::Ui;
use serde_json::Value;
use spektrafilm_core::feature_profiles::{self, FeatureProfileFamily, FeatureProfileSelection};
use spektrafilm_core::params::{
    PrintCurvesMorphParams, PrintExposureEvTracking, PrintExposureMode, RuntimeParams,
    grain::{FotufilmMaterial, GrainEngine, GrainV2FilmType, GrainV2Mode},
};
use spektrafilm_core::profile::Profile;
use std::path::Path;

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

/// Capture/scene illuminants offered by the Camera control. This is the
/// curated subset of the shared illuminant vocabulary that covers the
/// photographic scene-light range; the bundled catalog carries the full set
/// (daylight, fluorescent, LED and discharge series), and the canonical
/// `--set camera.capture_illuminant=…` / preset path accepts every catalog
/// name plus any `BB<kelvin>` blackbody. Keeping one list here avoids a
/// second vocabulary while keeping the dropdown usable.
const CAPTURE_ILLUMINANTS: &[&str] = &[
    "D50", "D55", "D65", "D75", "T", "A", "BB3200", "BB4000", "BB5000", "BB5600", "BB6500",
    "BB7500", "FL2", "FL11", "LED-B3", "LED-B5", "LED-RGB1", "LED-V2", "B", "C", "D60", "E",
    "BB2000", "BB3400", "TH-KG3", "TH-KG3-L", "K75P",
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
    if !film_medium.positive_output_eligible(&params.workflow.route) {
        params.scanner.scan_output = "direct_scan".into();
    }
    // A conversion style belongs to the negative interpretation, so leaving
    // that interpretation resets it to the frozen reference tone. Resetting
    // keeps the transition actioned; a non-neutral style with a direct scan
    // is a validation error the render would otherwise surface.
    if params.scanner.scan_output != "negative_to_positive"
        && params.scanner.conversion_style != vocabulary::CONVERSION_STYLE_STANDARD
    {
        params.scanner.conversion_style = vocabulary::CONVERSION_STYLE_STANDARD.into();
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

pub(crate) fn apply_feature_profile(
    params: &mut RuntimeParams,
    family: FeatureProfileFamily,
    name: &str,
) -> Result<(), String> {
    let selection = FeatureProfileSelection {
        family,
        name,
        position: 0,
    };
    feature_profiles::apply(params, &[selection])
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    extras: &mut Value,
    section: &str,
    film_medium: ScanMedium,
    filter: &str,
    cache: &mut SectionLabelCache,
    capture_reference_illuminant: &str,
    data_dir: &Path,
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
            capture_reference_illuminant,
            data_dir,
        );
    });
    filter.finish();
    flags
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    extras: &mut Value,
    section: &str,
    film_medium: ScanMedium,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
    capture_reference_illuminant: &str,
    data_dir: &Path,
) {
    scanner::sections(
        ui,
        params,
        extras,
        film_medium,
        section,
        flags,
        filter,
        capture_reference_illuminant,
        data_dir,
    );
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
    cmyk::sections(ui, params, section, flags, filter);
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
            PanelItem::Section("CMYK ink style"),
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
mod cmyk;
mod gamut;
mod glare;
mod grain;
mod halation;
mod output;
mod scanner;
mod search;
mod upsampling;
mod widgets;
pub(crate) use search::SearchIndex;
pub(crate) use widgets::*;

// Every control editor reads presentation tokens; re-export so a control file
// does not each add its own import.

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
                        "D55",
                        &crate::profiles::pick_data_dir(),
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
            let (_, output) = scanner_frame(&ctx, &mut params, medium, vec![]);
            // The scanner surface derives its medium from the route, so the
            // controls follow the route rather than the selected film stock.
            let active = medium.for_route(route) != ScanMedium::None;
            assert_eq!(
                has_text(&output, "Lens blur (readout optics)"),
                active,
                "{route}: lens blur"
            );
            assert_eq!(
                has_text(&output, "scan output"),
                medium.positive_output_eligible(route),
                "{route}"
            );
        }
    }

    #[test]
    fn ineligible_routes_reset_the_scan_polarity() {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.scanner.scan_output = "negative_to_positive".into();
        normalize_scanner_selections(&mut params, ScanMedium::Negative);
        assert_eq!(params.scanner.scan_output, "negative_to_positive");
        params.workflow.route = "input > film > print > scan".into();
        normalize_scanner_selections(&mut params, ScanMedium::Negative);
        assert_eq!(params.scanner.scan_output, "direct_scan");
    }

    /// A style file body the tests author: the same per-channel tone triple
    /// shape `data/scan-styles/<name>.json` carries.
    const PROBE_STYLE_BODY: &str = r#"{"channels":[
 {"brightness_ev":1.2,"contrast":0.9,"dmax":1.8},
 {"brightness_ev":1.2,"contrast":0.9,"dmax":1.8},
 {"brightness_ev":1.2,"contrast":0.9,"dmax":1.8}
]}"#;

    #[test]
    fn conversion_style_defaults_to_neutral_and_is_gated_on_the_polarity() {
        let ctx = egui::Context::default();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        assert_eq!(params.scanner.conversion_style, "standard");
        // The style belongs to the negative interpretation, so the direct scan
        // does not offer it at all.
        let (_, output) = scanner_frame(&ctx, &mut params, ScanMedium::Negative, vec![]);
        assert!(!has_text(&output, "conversion style"));
        params.scanner.scan_output = "negative_to_positive".into();
        let (_, output) = scanner_frame(&ctx, &mut params, ScanMedium::Negative, vec![]);
        assert!(has_text(&output, "conversion style"));
        assert!(has_text(&output, "standard"));
    }

    #[test]
    fn the_repository_data_directory_offers_only_the_neutral_style() {
        // No style ships with the install, so the shipped vocabulary is
        // exactly `standard` until the user drops a file into the data dir.
        assert_eq!(
            crate::profiles::scan_conversion_styles(&crate::profiles::pick_data_dir()),
            vec!["standard".to_owned()]
        );
    }

    #[test]
    fn a_data_directory_with_styles_offers_them_beside_neutral() {
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("scan-styles")).unwrap();
        for name in ["warm-probe", "cool-probe"] {
            std::fs::write(
                data.path().join("scan-styles").join(format!("{name}.json")),
                PROBE_STYLE_BODY,
            )
            .unwrap();
        }
        assert_eq!(
            crate::profiles::scan_conversion_styles(data.path()),
            vec![
                "standard".to_owned(),
                "cool-probe".to_owned(),
                "warm-probe".to_owned()
            ],
            "neutral first, then the installed styles sorted"
        );
    }

    #[test]
    fn leaving_the_negative_interpretation_resets_the_conversion_style() {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.conversion_style = "warm-probe".into();
        normalize_scanner_selections(&mut params, ScanMedium::Negative);
        assert_eq!(params.scanner.conversion_style, "warm-probe");
        // Switching back to the direct scan resets the style rather than
        // leaving a combination validation rejects.
        params.scanner.scan_output = "direct_scan".into();
        normalize_scanner_selections(&mut params, ScanMedium::Negative);
        assert_eq!(params.scanner.conversion_style, "standard");
        params.validate().unwrap();
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

    /// Camera section frame, so the capture-illuminant control is exercised
    /// under its real section rather than a synthetic one.
    fn camera_frame(
        ctx: &egui::Context,
        params: &mut RuntimeParams,
        events: Vec<egui::Event>,
    ) -> (ChangeFlags, egui::FullOutput) {
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
                    let mut cache = SectionLabelCache::default();
                    flags = show(
                        ui,
                        params,
                        &mut serde_json::json!({}),
                        "Camera",
                        ScanMedium::Negative,
                        "",
                        &mut cache,
                        "D55",
                        &crate::profiles::pick_data_dir(),
                    );
                });
            },
        );
        (flags, output)
    }

    #[test]
    fn camera_capture_illuminant_control_sets_and_clears_the_option() {
        let ctx = egui::Context::default();
        let mut params = RuntimeParams::default();
        // The control renders inside the Camera section, not Scanner/Convert.
        let (_, output) = camera_frame(&ctx, &mut params, vec![]);
        assert!(has_text(&output, "inherit (film profile)"));
        assert_eq!(params.camera.capture_illuminant, None);
        /// `option` inside the popup.
        fn select(
            ctx: &egui::Context,
            params: &mut RuntimeParams,
            summary: &str,
            option: &str,
        ) -> ChangeFlags {
            let (_, output) = camera_frame(ctx, params, vec![]);
            let button = text_position(&output, summary);
            camera_frame(ctx, params, click(button, true));
            camera_frame(ctx, params, click(button, false));
            // Popup areas lay out their first frame before becoming visible and hittable.
            let (_, output) = camera_frame(ctx, params, vec![]);
            let entry = popup_text_position(&output, option);
            camera_frame(ctx, params, click(entry, true));
            camera_frame(ctx, params, click(entry, false)).0
        }

        // An explicit illuminant writes Some and flags the runtime change.
        let flags = select(&ctx, &mut params, "inherit (film profile)", "BB4000");
        assert!(flags.runtime_changed);
        assert_eq!(params.camera.capture_illuminant.as_deref(), Some("BB4000"));
        // The selection is retained across frames.
        let (_, output) = camera_frame(&ctx, &mut params, vec![]);
        assert!(has_text(&output, "BB4000"));

        // Clearing returns to inherit (None), which is a real unset value.
        let flags = select(&ctx, &mut params, "BB4000", "inherit (film profile)");
        assert!(flags.runtime_changed);
        assert_eq!(params.camera.capture_illuminant, None);
    }

    #[test]
    fn camera_capture_illuminant_is_not_rendered_by_other_sections() {
        let ctx = egui::Context::default();
        for section in ["Scanner", "Convert", "Enlarger", "Input"] {
            let mut params = RuntimeParams::default();
            params.workflow.route = "input > film > scan".into();
            params.scanner.scan_output = "negative_to_positive".into();
            let mut flags = ChangeFlags::default();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 1600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut cache = SectionLabelCache::default();
                        flags = show(
                            ui,
                            &mut params,
                            &mut serde_json::json!({}),
                            section,
                            ScanMedium::Negative,
                            "",
                            &mut cache,
                            "D55",
                            &crate::profiles::pick_data_dir(),
                        );
                    });
                },
            );
            assert!(
                !has_text(&output, "capture illuminant"),
                "{section} must not own the capture illuminant"
            );
            assert!(!flags.runtime_changed);
            assert_eq!(params.camera.capture_illuminant, None);
        }
    }

    #[test]
    fn camera_capture_illuminant_offered_values_are_supported_illuminants() {
        // Every offered entry must be accepted by the shared core validator,
        // so the dropdown cannot produce an unsupported value.
        for name in CAPTURE_ILLUMINANTS {
            assert!(
                spektrafilm_core::spectral_service::is_supported_illuminant(name),
                "{name} must be a supported illuminant"
            );
        }
    }

    #[test]
    fn camera_capture_illuminant_accepts_custom_blackbody_text() {
        let ctx = egui::Context::default();
        let mut params = RuntimeParams::default();
        let (_, output) = camera_frame(&ctx, &mut params, vec![]);
        let position = text_position(&output, "custom BB<kelvin>");
        camera_frame(&ctx, &mut params, click(position, true));
        camera_frame(&ctx, &mut params, click(position, false));
        let flags = camera_frame(&ctx, &mut params, replace_text("BB4321")).0;
        assert!(flags.runtime_changed);
        assert_eq!(params.camera.capture_illuminant.as_deref(), Some("BB4321"));
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
                            "D55",
                            &crate::profiles::pick_data_dir(),
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
                            "D55",
                            &crate::profiles::pick_data_dir(),
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
}
