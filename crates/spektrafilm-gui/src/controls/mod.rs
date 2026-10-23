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

#[derive(Clone)]
pub(crate) struct ControlProjection {
    pub(crate) runtime: std::sync::Arc<Value>,
    pub(crate) extras: std::sync::Arc<Value>,
}

thread_local! {
    static FRAME_PROJECTION: std::cell::RefCell<Option<(usize, usize, ControlProjection)>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn begin_control_frame(_ctx: &egui::Context, params: &RuntimeParams, extras: &Value) {
    let projection = ControlProjection {
        runtime: std::sync::Arc::new(serde_json::to_value(params).expect("runtime serializes")),
        extras: std::sync::Arc::new(extras.clone()),
    };
    FRAME_PROJECTION.with(|cache| {
        *cache.borrow_mut() = Some((
            params as *const _ as usize,
            extras as *const _ as usize,
            projection,
        ))
    });
}

pub(crate) fn end_control_frame() {
    FRAME_PROJECTION.with(|cache| *cache.borrow_mut() = None);
}

pub(crate) fn control_projection(params: &RuntimeParams, extras: &Value) -> ControlProjection {
    FRAME_PROJECTION
        .with(|cache| {
            cache
                .borrow()
                .as_ref()
                .filter(|(p, e, _)| {
                    *p == params as *const _ as usize && *e == extras as *const _ as usize
                })
                .map(|(_, _, projection)| projection.clone())
        })
        .unwrap_or_else(|| ControlProjection {
            runtime: std::sync::Arc::new(serde_json::to_value(params).expect("runtime serializes")),
            extras: std::sync::Arc::new(extras.clone()),
        })
}

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
        let response = egui::ComboBox::from_id_salt(label)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for &(option, label) in choices {
                    ui.selectable_value(value, option, label);
                }
            })
            .response
            .on_hover_text(tooltip);
        if let Some(entry) = inventory().iter().find(|entry| {
            entry.owner == ControlOwner::Export && entry.label.eq_ignore_ascii_case(label)
        }) {
            locate_row(ui, &entry.path, &response);
        }
        focus_response(ui, label, &response);
        // The selector carries the parameter's own accessible name.
        widgets::name_choice(ui, label);
    });
    row_modified::offer_reset(ui, label, None);
    if prev != *value {
        record_label_edit(ui.ctx(), label);
    }
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
    if row_modified::current_section(ui.ctx()).is_some_and(|section| {
        section == title || (section == "Glare" && title == "Print viewing glare")
    }) {
        add_contents(ui, filter);
        return;
    }
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
    let only_modified = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(egui::Id::new("controls_only_modified")))
        .unwrap_or(false);
    filter.configure(section, params, extras, only_modified);
    filter.install_row_filter(ui.ctx());
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
    ui.ctx().data_mut(|data| {
        data.remove_temp::<Option<std::sync::Arc<Vec<String>>>>(egui::Id::new(
            "control_allowed_paths",
        ))
    });
    flags.runtime_changed |= apply_pending_resets(ui.ctx(), params, extras);
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
pub(crate) use filter::{ControlFilter, SectionLabelCache, filter_matches, filter_matches_any};
pub(crate) mod row_modified;
/// Photographic stages in the continuous adjustment inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Stage {
    Input,
    Film,
    Print,
    Scan,
    Output,
}

impl Stage {
    pub(crate) const ALL: [Self; 5] = [
        Self::Input,
        Self::Film,
        Self::Print,
        Self::Scan,
        Self::Output,
    ];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Input => "Input and capture",
            Self::Film => "Film",
            Self::Print => "Print",
            Self::Scan => "Scan",
            Self::Output => "Output",
        }
    }
}

/// Each module has one visual owner; this is not serialized preset scope.
pub(crate) const STAGE_LAYOUT: &[(Stage, &[&str])] = &[
    (
        Stage::Input,
        &[
            "Workflow",
            "Input",
            "Import Raw",
            "Source rotation",
            "Crop and upscale",
            "Camera",
            "Camera diffusion",
            "Input gamut compress",
            "Spectral upsampling",
            "Convert",
        ],
    ),
    (
        Stage::Film,
        &[
            "Film material",
            "Film chemistry",
            "Film base",
            "Halation",
            "Couplers",
            "Grain",
        ],
    ),
    (
        Stage::Print,
        &[
            "Print material",
            "Enlarger",
            "Preflash",
            "Enlarger diffusion",
            "Print chemistry",
            "Print base",
        ],
    ),
    (Stage::Scan, &["Scanner", "Glare"]),
    (
        Stage::Output,
        &["Output", "Output gamut compress", "CMYK ink style"],
    ),
];

mod inventory;
pub(crate) use inventory::{ControlOwner, inventory, lookup, section_matches};

pub(crate) fn install_view_filter(ctx: &egui::Context, only_modified: bool) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("controls_only_modified"), only_modified));
}

pub(crate) fn take_edited_paths(ctx: &egui::Context) -> Vec<String> {
    ctx.data_mut(|data| data.remove_temp::<Vec<String>>(egui::Id::new("controls_edited_paths")))
        .unwrap_or_default()
}

pub(crate) fn record_edited_path(ctx: &egui::Context, path: &str) {
    ctx.data_mut(|data| {
        let paths =
            data.get_temp_mut_or_default::<Vec<String>>(egui::Id::new("controls_edited_paths"));
        if !paths.iter().any(|existing| existing == path) {
            paths.push(path.to_owned());
        }
    });
}

pub(crate) fn record_label_edit(ctx: &egui::Context, label: &str) {
    if let Some(section) = row_modified::current_section(ctx)
        && let Some(path) = inventory::path_for_label(&section, label)
    {
        record_edited_path(ctx, path);
    }
}

pub(crate) fn apply_pending_resets(
    ctx: &egui::Context,
    params: &mut RuntimeParams,
    extras: &mut Value,
) -> bool {
    let mut changed = false;
    for path in row_modified::take_resets(ctx) {
        if crate::section_reset::reset_leaf(params, extras, &path) {
            record_edited_path(ctx, &path);
            changed = true;
        }
    }
    changed
}

pub(crate) fn entry_matches(
    path: &str,
    query: &str,
    params: &RuntimeParams,
    extras: &Value,
    only_modified: bool,
) -> bool {
    let Some(entry) = lookup(path) else {
        return false;
    };
    entry.matches(&query.to_lowercase())
        && (!only_modified || {
            let projection = control_projection(params, extras);
            entry.is_modified(&projection.runtime, &projection.extras)
        })
}

pub(crate) fn request_focus(ctx: &egui::Context, section: &str, label: &str) {
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new("control_focus_request"),
            (section.to_owned(), label.to_owned()),
        )
    });
}

pub(crate) fn row_path_matches(ctx: &egui::Context, path: &str) -> bool {
    ctx.data(|data| {
        data.get_temp::<Option<std::sync::Arc<Vec<String>>>>(egui::Id::new("control_allowed_paths"))
    })
    .flatten()
    .is_none_or(|paths| paths.iter().any(|candidate| candidate == path))
}

pub(crate) struct RowIdentity {
    ctx: egui::Context,
}
pub(crate) fn row_identity(ctx: &egui::Context, path: &str) -> RowIdentity {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("control_row_path"), path.to_owned()));
    RowIdentity { ctx: ctx.clone() }
}
impl Drop for RowIdentity {
    fn drop(&mut self) {
        self.ctx
            .data_mut(|data| data.remove_temp::<String>(egui::Id::new("control_row_path")));
    }
}
pub(crate) fn focus_response(ui: &Ui, label: &str, response: &egui::Response) {
    let pending = ui
        .ctx()
        .data(|data| data.get_temp::<(String, String)>(egui::Id::new("control_focus_request")));
    let row_path = ui
        .ctx()
        .data(|data| data.get_temp::<String>(egui::Id::new("control_row_path")));
    if let Some((section, target)) = pending
        && (row_modified::current_section(ui.ctx()).as_deref() == Some(section.as_str())
            || (section == "Export"
                && inventory().iter().any(|entry| {
                    entry.owner == ControlOwner::Export && entry.label.eq_ignore_ascii_case(label)
                })))
        && if lookup(&target).is_some() {
            row_path.as_deref() == Some(target.as_str())
                || (row_path.is_none()
                    && lookup(&target).is_some_and(|entry| entry.label.eq_ignore_ascii_case(label)))
        } else {
            target.eq_ignore_ascii_case(label)
        }
    {
        response.request_focus();
        response.scroll_to_me(Some(egui::Align::Center));
        let highlight_until = ui.input(|input| input.time) + 2.0;
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("control_located_row"),
                (response.id, highlight_until),
            )
        });
        ui.ctx().data_mut(|data| {
            data.remove_temp::<(String, String)>(egui::Id::new("control_focus_request"))
        });
    }
    let highlighted = ui
        .ctx()
        .data(|data| data.get_temp::<(egui::Id, f64)>(egui::Id::new("control_located_row")))
        .is_some_and(|(id, until)| id == response.id && ui.input(|input| input.time) < until);
    if highlighted {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            2.0_f32,
            egui::Stroke::new(2.0_f32, theme::ACCENT),
            egui::StrokeKind::Outside,
        );
        ui.painter().text(
            response.rect.left_top() - egui::vec2(0.0, 3.0),
            egui::Align2::LEFT_BOTTOM,
            "Located control",
            egui::FontId::proportional(10.0),
            theme::FG,
        );
    }
}

pub(crate) fn locate_row(ui: &Ui, path: &str, response: &egui::Response) {
    let Some(entry) = lookup(path) else { return };
    let pending = ui
        .ctx()
        .data(|data| data.get_temp::<(String, String)>(egui::Id::new("control_focus_request")));
    if pending.as_ref().is_some_and(|(section, target)| {
        (section == entry.section || (section == "Export" && entry.owner == ControlOwner::Export))
            && (target == path || target.eq_ignore_ascii_case(&entry.label))
    }) {
        response.request_focus();
        response.scroll_to_me(Some(egui::Align::Center));
        ui.ctx().data_mut(|data| {
            data.remove_temp::<(String, String)>(egui::Id::new("control_focus_request"))
        });
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            2.0_f32,
            egui::Stroke::new(2.0_f32, theme::ACCENT),
            egui::StrokeKind::Outside,
        );
        ui.painter().text(
            response.rect.left_top(),
            egui::Align2::LEFT_BOTTOM,
            "Located control",
            egui::FontId::proportional(10.0),
            theme::FG,
        );
    }
}

pub(crate) fn module_applicability(
    section: &str,
    params: &RuntimeParams,
    film: &Profile,
    print: &Profile,
) -> Option<String> {
    inventory()
        .iter()
        .filter(|entry| entry.section == section && entry.searchable())
        .find_map(|entry| {
            spektrafilm_core::params::sources::inactive_reason(&entry.path, params, film, print)
        })
}
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

    fn has_text(output: &egui::FullOutput, text: &str) -> bool {
        output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(shape) if shape.galley.job.text == text))
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
