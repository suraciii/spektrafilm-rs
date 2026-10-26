//! Canonical control ownership, independent of disclosure and applicability.
//!
//! Runtime field descriptions remain core-owned. This table adds presentation
//! labels, actual editor owners, and explicit reasons for non-editor leaves.
use super::Stage;
use serde_json::Value;
use spektrafilm_core::params::{RuntimeParams, sources};
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControlOwner {
    Stage(Stage),
    Settings,
    Export,
    Viewer,
    Header,
    WorkList,
    Presets,
    Result,
}
impl ControlOwner {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Stage(stage) => stage.label(),
            Self::Settings => "Settings",
            Self::Export => "Export",
            Self::Viewer => "Viewer",
            Self::Header => "Photograph toolbar",
            Self::WorkList => "Work list",
            Self::Presets => "Presets",
            Self::Result => "Result actions",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorKind {
    Numeric,
    OptionalNumeric,
    Tuple,
    Toggle,
    Choice,
    Text,
    Profile,
    Action,
    /// Supported field not connected to a production editor: a coverage defect.
    Unconnected,
    /// A non-creative core leaf, with its owning source rationale.
    Excluded(&'static str),
}

#[derive(Clone, Debug)]
pub(crate) struct ControlComponent {
    pub(crate) index: usize,
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) derived: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub(crate) struct ControlEntry {
    pub(crate) path: String,
    pub(crate) label: String,
    pub(crate) owner: ControlOwner,
    pub(crate) section: &'static str,
    pub(crate) editor: EditorKind,
    /// Unmodified `core::params::sources::describe_field` record, including
    /// type, default, unit, bounds, enum vocabulary and component domains.
    pub(crate) validation: Option<Value>,
    /// The validator/operation owning non-runtime values and actions.
    pub(crate) validator: &'static str,
    pub(crate) conditions: Vec<String>,
    pub(crate) components: Vec<ControlComponent>,
    /// Multi-leaf actions name their real affected canonical leaves.
    pub(crate) affected_paths: &'static [&'static str],
    search_text: String,
}
impl ControlEntry {
    pub(crate) fn searchable(&self) -> bool {
        !matches!(self.editor, EditorKind::Excluded(_))
    }

    /// `query` is lowercased once by the query owner, not once per entry.
    pub(crate) fn matches(&self, query: &str) -> bool {
        self.search_text.contains(query)
    }

    pub(crate) fn is_modified(&self, runtime: &Value, extras: &Value) -> bool {
        if self.path == "preference.interface_language"
            || matches!(self.editor, EditorKind::Action | EditorKind::Excluded(_))
        {
            return false;
        }
        if self.affected_paths.is_empty() {
            crate::section_reset::path_modified(&self.path, runtime, extras)
        } else {
            self.affected_paths
                .iter()
                .any(|path| crate::section_reset::path_modified(path, runtime, extras))
        }
    }

    /// Human-readable inspection of the same canonical metadata used by editors.
    pub(crate) fn localized_description(&self, ctx: &egui::Context) -> String {
        let mut text = crate::tr!(
            ctx,
            "controls.description.header",
            path = &self.path,
            owner = super::localized(ctx, self.owner.label()),
            section = super::localized(ctx, self.section),
            editor = super::localized(ctx, &format!("{:?}", self.editor)),
            validator = self.validator
        )
        .into_owned();
        if let Some(validation) = &self.validation {
            for key in [
                "type", "unit", "minimum", "maximum", "step", "enum", "nullable",
            ] {
                if let Some(value) = validation.get(key) {
                    text.push_str(&crate::tr!(
                        ctx,
                        "controls.description.field",
                        field = super::localized(ctx, key),
                        value = value
                    ));
                }
            }
        }
        for condition in &self.conditions {
            text.push_str(&crate::tr!(
                ctx,
                "controls.description.condition",
                condition = super::localized(ctx, condition)
            ));
        }
        for component in &self.components {
            text.push_str(&crate::tr!(
                ctx,
                "controls.description.component",
                index = component.index,
                id = &component.id,
                label = self.localized_component_label(ctx, component)
            ));
            if component.derived.is_some() {
                text.push_str(&crate::tr!(
                    ctx,
                    "controls.description.derived",
                    formula = crate::i18n::text(ctx, "controls.description.remainder")
                ));
            }
        }
        if !self.affected_paths.is_empty() {
            text.push_str(&crate::tr!(
                ctx,
                "controls.description.scope",
                paths = self.affected_paths.join(", ")
            ));
        }
        text
    }

    fn localized_component_label(
        &self,
        ctx: &egui::Context,
        component: &ControlComponent,
    ) -> String {
        self.localized_component_label_for(crate::i18n::locale(ctx), component)
    }

    fn localized_component_label_for(
        &self,
        locale: crate::i18n::Locale,
        component: &ControlComponent,
    ) -> String {
        if self.path == "film_render.grain.fotufilm.sublayer_shares" {
            return super::localized_for(
                locale,
                [
                    "Sublayer 1",
                    "Sublayer 2",
                    "Sublayer 3",
                    "Sublayer 4 (remainder)",
                ][component.index],
            )
            .into_owned();
        }
        rust_i18n::t!(
            "controls.component.name",
            locale = locale.as_str(),
            label = super::localized_for(locale, &self.label),
            component = super::localized_for(locale, &component.id)
        )
        .into_owned()
    }
}

pub(crate) fn inventory() -> &'static [ControlEntry] {
    static INVENTORY: LazyLock<Vec<ControlEntry>> = LazyLock::new(|| {
        let mut entries = Vec::new();
        for group in sources::describe_root_groups() {
            let description = sources::describe_module(&group).expect("core discovery root exists");
            for field in description["fields"]
                .as_array()
                .expect("core module fields")
            {
                let path = field["path"].as_str().expect("core canonical path");
                // Resolve through the public leaf API rather than introducing
                // a second schema or inferring a numeric validator from JSON.
                let validation = sources::describe_field(path).expect("discovered leaf exists");
                entries.push(runtime_entry(path, validation));
            }
        }
        // This root leaf is provenance, not a parameter group.
        entries.push(runtime_entry(
            "neutral_print_filters_protected",
            sources::describe_field("neutral_print_filters_protected")
                .expect("core provenance leaf"),
        ));
        entries.push(nonruntime(
            SurfaceEntry {
                path: "workflow.route",
                label: "workflow",
                owner: ControlOwner::Stage(Stage::Input),
                section: "Workflow",
                editor: EditorKind::Choice,
                validator: "core::params::sources::supported_routes",
                conditions: &[],
            },
            &[],
        ));
        append_nonruntime(&mut entries);
        entries.push(nonruntime(
            SurfaceEntry {
                path: "preference.interface_language",
                label: "语言 / Language",
                owner: ControlOwner::Settings,
                section: "Session",
                editor: EditorKind::Choice,
                validator: "i18n::LanguagePreference",
                conditions: &[],
            },
            &[],
        ));
        for entry in &mut entries {
            let module = if entry.section == "Glare" {
                "Print viewing glare"
            } else {
                entry.section
            };
            let mut text = format!(
                "{}\n{}\n{}\n{}\n{}",
                entry.path,
                entry.label,
                entry.section,
                module,
                entry.owner.label()
            );
            if entry.path.starts_with("scanner.operator.")
                || entry.path.starts_with("action.operator_")
            {
                text.push_str("\nOperator correction");
            }
            if entry.path == "camera.film_format_mm" {
                text.push_str("\nFrame Long Edge");
            }
            for component in &entry.components {
                text.push('\n');
                text.push_str(&component.label);
            }
            for locale in [
                crate::i18n::Locale::English,
                crate::i18n::Locale::SimplifiedChinese,
            ] {
                for label in [&entry.label[..], entry.section, module, entry.owner.label()] {
                    text.push('\n');
                    text.push_str(&super::localized_for(locale, label));
                }
                if entry.path == "camera.film_format_mm" {
                    text.push('\n');
                    text.push_str(&super::localized_for(locale, "Frame Long Edge"));
                }
                if entry.path == "preference.interface_language" {
                    text.push_str("\nGeneral\n通用\nLanguage\n语言");
                }
                for component in &entry.components {
                    text.push('\n');
                    text.push_str(&entry.localized_component_label_for(locale, component));
                }
            }
            entry.search_text = text.to_lowercase();
        }
        entries
    });
    &INVENTORY
}

pub(crate) fn lookup(path: &str) -> Option<&'static ControlEntry> {
    inventory().iter().find(|entry| entry.path == path)
}

pub(crate) fn path_for_label(section: &str, label: &str) -> Option<&'static str> {
    inventory()
        .iter()
        .find(|entry| entry.section == section && entry.label.eq_ignore_ascii_case(label))
        .map(|entry| entry.path.as_str())
}

pub(crate) fn section_matches(
    section: &str,
    query: &str,
    params: &RuntimeParams,
    extras: &Value,
    only_modified: bool,
) -> bool {
    if query.is_empty() && !only_modified {
        return true;
    }
    let projection = only_modified.then(|| super::control_projection(params, extras));
    let query = query.to_lowercase();
    inventory().iter().any(|entry| {
        entry.section == section
            && entry.searchable()
            && entry.matches(&query)
            && projection
                .as_ref()
                .is_none_or(|projection| entry.is_modified(&projection.runtime, &projection.extras))
    })
}

fn runtime_entry(path: &str, validation: Value) -> ControlEntry {
    let (owner, section) = runtime_owner(path);
    let label = LABELS
        .iter()
        .find(|(candidate, _)| *candidate == path)
        .map(|(_, label)| (*label).to_owned())
        .unwrap_or_else(|| path.rsplit('.').next().unwrap_or(path).replace('_', " "));
    let editor = if let Some(reason) = exclusion(path, &validation) {
        EditorKind::Excluded(reason)
    } else if !LABELS.iter().any(|(candidate, _)| *candidate == path) {
        EditorKind::Unconnected
    } else if path == "film_render.grain.v2_profile" || path.ends_with("development_time") {
        EditorKind::Profile
    } else if path == "camera.film_format_mm" {
        EditorKind::Choice
    } else if validation["default"].is_array() {
        EditorKind::Tuple
    } else if validation["type"] == "bool" {
        EditorKind::Toggle
    } else if validation["nullable"] == true
        && validation["type"]
            .as_str()
            .is_some_and(|ty| ty.contains("f32") || ty.contains("f64"))
    {
        EditorKind::OptionalNumeric
    } else if validation["default"].is_number() {
        EditorKind::Numeric
    } else if path == "film_render.convert.calibration" {
        EditorKind::Text
    } else {
        EditorKind::Choice
    };
    let conditions = validation["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|condition| condition.as_str().map(str::to_owned))
        .collect();
    let matrix = path == "film_render.convert.calibration";
    let count = if matrix {
        9
    } else {
        validation["default"].as_array().map_or(0, Vec::len)
    };
    let identifiers = super::widgets::tuple_identifiers(path);
    let components = (0..count)
        .map(|index| {
            let shares = path == "film_render.grain.fotufilm.sublayer_shares";
            let id = if matrix {
                format!("{}:{}", index / 3, index % 3)
            } else {
                identifiers
                    .get(index)
                    .map(|id| (*id).to_owned())
                    .unwrap_or_else(|| index.to_string())
            };
            let component_label = if shares {
                if index == 3 {
                    "Sublayer 4 (remainder)".to_owned()
                } else {
                    format!("Sublayer {}", index + 1)
                }
            } else {
                format!("{label} {id}")
            };
            ControlComponent {
                index,
                id,
                label: component_label,
                derived: (shares && index == 3).then_some(
                    "1 - sum of the first three shares; widgets::fotufilm_sublayer_shares",
                ),
            }
        })
        .collect();
    ControlEntry {
        path: path.to_owned(),
        label,
        owner,
        section,
        editor,
        validation: Some(validation),
        validator: "core::params::RuntimeParams::validate",
        conditions,
        components,
        affected_paths: &[],
        search_text: String::new(),
    }
}

fn exclusion(path: &str, description: &Value) -> Option<&'static str> {
    if description["conditions"]
        .as_array()
        .is_some_and(|conditions| {
            conditions
                .iter()
                .any(|condition| condition == "preserved upstream no-op")
        })
    {
        return Some("core params::sources::effects: preserved upstream no-op");
    }
    match path {
        "camera.lens_blur_um" | "camera.filter_uv" | "camera.filter_ir" => {
            Some("section_reset Camera scope: calibrated optical/filter outputs, not user rows")
        }
        "enlarger.y_filter_neutral" | "enlarger.m_filter_neutral" | "enlarger.c_filter_neutral" => {
            Some(
                "core params_builder::apply_database_neutral_print_filters: database-calibration anchors; specs/presets/design.md#sections",
            )
        }
        "enlarger.lens_blur" => Some(
            "digest zeros this field; no upstream stage reads it; specs/presets/design.md#sections",
        ),
        "settings.spectral_shape" => {
            Some("core params::validation admits only the bundled wavelength grid")
        }
        "settings.preview_mode" => {
            Some("render.rs derives this request flag from Preview versus Scan")
        }
        "settings.use_enlarger_lut"
        | "settings.use_scanner_lut"
        | "settings.lut_resolution"
        | "settings.use_fast_stats" => Some(
            "core SettingsParams / params_builder: execution approximation and performance configuration, outside photographic look ownership; specs/presets/design.md",
        ),
        "settings.neutral_print_filters_from_database" => Some(
            "core params_builder::apply_database_neutral_print_filters: calibration preparation policy, not a photographic editor",
        ),
        "neutral_print_filters_protected" => {
            Some("core RuntimeParams: explicit-leaf calibration protection provenance")
        }
        "taps.inject" | "taps.collect" => Some(
            "core TapsParams: internal pipeline entry/exit setup, not end-to-end photographic controls",
        ),
        "debug.lut_mode"
        | "debug.deactivate_spatial_effects"
        | "debug.deactivate_stochastic_effects" => Some(
            "core params_builder digest: diagnostic/LUT preparation switches, not creative controls",
        ),
        _ if path.starts_with("film_render.glare.") => {
            Some("pipeline never reads film_render.glare; specs/presets/design.md#sections")
        }
        _ => None,
    }
}

fn runtime_owner(path: &str) -> (ControlOwner, &'static str) {
    use ControlOwner::Stage as Owner;
    if path.starts_with("film_render.grain.") {
        (Owner(Stage::Film), "Grain")
    } else if path.starts_with("film_render.halation.") {
        (Owner(Stage::Film), "Halation")
    } else if path.starts_with("film_render.dir_couplers.") {
        (Owner(Stage::Film), "Couplers")
    } else if path.starts_with("film_render.base.") {
        (Owner(Stage::Film), "Film base")
    } else if path.starts_with("film_render.convert.") {
        (Owner(Stage::Input), "Convert")
    } else if path == "film_render.development_time" {
        (Owner(Stage::Film), "Film material")
    } else if path.starts_with("film_render.") {
        (Owner(Stage::Film), "Film chemistry")
    } else if path.starts_with("print_render.glare.") {
        (Owner(Stage::Scan), "Glare")
    } else if path.starts_with("print_render.base.") {
        (Owner(Stage::Print), "Print base")
    } else if path == "print_render.development_time" {
        (Owner(Stage::Print), "Print material")
    } else if path.starts_with("print_render.") {
        (Owner(Stage::Print), "Print chemistry")
    } else if path.starts_with("camera.diffusion_filter.") {
        (Owner(Stage::Input), "Camera diffusion")
    } else if path.starts_with("camera.") {
        (Owner(Stage::Input), "Camera")
    } else if path.starts_with("enlarger.preflash_") {
        (Owner(Stage::Print), "Preflash")
    } else if path.starts_with("enlarger.diffusion_filter.") {
        (Owner(Stage::Print), "Enlarger diffusion")
    } else if path.starts_with("enlarger.") || path == "settings.print_exposure_ev_tracking" {
        (Owner(Stage::Print), "Enlarger")
    } else if path.starts_with("scanner.") {
        (Owner(Stage::Scan), "Scanner")
    } else if path.starts_with("cmyk.") {
        (Owner(Stage::Output), "CMYK ink style")
    } else if path.starts_with("io.input_gamut_compress.") {
        (Owner(Stage::Input), "Input gamut compress")
    } else if path.starts_with("io.output_gamut_compress.") {
        (Owner(Stage::Output), "Output gamut compress")
    } else if path.starts_with("io.crop") || path == "io.upscale_factor" {
        (Owner(Stage::Input), "Crop and upscale")
    } else if path.starts_with("io.input_") {
        (Owner(Stage::Input), "Input")
    } else if path.starts_with("io.output_") || path.starts_with("output_mix.") {
        (Owner(Stage::Output), "Output")
    } else if matches!(
        path,
        "settings.rgb_to_raw_method"
            | "settings.apply_hanatos2025_adaptation_window"
            | "settings.apply_hanatos2025_adaptation_surface"
            | "settings.spectral_gaussian_blur"
            | "settings.use_cat16"
            | "settings.spectral_shape"
    ) {
        (Owner(Stage::Input), "Spectral upsampling")
    } else {
        (ControlOwner::Settings, "Session")
    }
}

#[derive(Clone, Copy)]
struct SurfaceEntry {
    path: &'static str,
    label: &'static str,
    owner: ControlOwner,
    section: &'static str,
    editor: EditorKind,
    validator: &'static str,
    conditions: &'static [&'static str],
}

fn nonruntime(surface: SurfaceEntry, affected_paths: &'static [&'static str]) -> ControlEntry {
    let SurfaceEntry {
        path,
        label,
        owner,
        section,
        editor,
        validator,
        conditions,
    } = surface;
    ControlEntry {
        path: path.to_owned(),
        label: label.to_owned(),
        owner,
        section,
        editor,
        validation: None,
        validator,
        conditions: conditions
            .iter()
            .map(|condition| (*condition).to_owned())
            .collect(),
        components: Vec::new(),
        affected_paths,
        search_text: String::new(),
    }
}

const OPERATOR_PATHS: &[&str] = &[
    "scanner.operator.exposure_ev",
    "scanner.operator.warmth_ev",
    "scanner.operator.tint_ev",
    "scanner.operator.contrast",
];

const OPERATOR_ACTIONS: &[(&str, &str, &[&str])] = &[
    (
        "action.operator_restore_automatic",
        "Restore automatic",
        OPERATOR_PATHS,
    ),
    (
        "action.operator_copy",
        "Copy operator correction",
        OPERATOR_PATHS,
    ),
    (
        "action.operator_paste",
        "Paste operator correction",
        OPERATOR_PATHS,
    ),
    (
        "action.operator_compare",
        "Hold automatic comparison",
        OPERATOR_PATHS,
    ),
];

fn append_nonruntime(entries: &mut Vec<ControlEntry>) {
    use ControlOwner::Stage as Owner;
    use EditorKind::*;
    for &surface in SURFACES {
        entries.push(nonruntime(surface, &[]));
    }
    for (path, label, affected_paths) in OPERATOR_ACTIONS {
        entries.push(nonruntime(
            SurfaceEntry {
                path,
                label,
                owner: Owner(Stage::Scan),
                section: "Scanner",
                editor: Action,
                validator: "controls::scanner::operator_controls",
                conditions: &[
                    "negative film; eligible negative-to-positive route; standard conversion style",
                    "stored corrections retained while inactive; clipboard and comparison are session-only",
                ],
            },
            affected_paths,
        ));
    }
    entries.push(nonruntime(
        SurfaceEntry {
            path: "action.halation_profile",
            label: "Halation Profiles",
            owner: Owner(Stage::Film),
            section: "Halation",
            editor: Action,
            validator: "core::feature_profiles::{effective_name,apply}",
            conditions: &["core Halation feature-profile scope; inactive values retained"],
        },
        &[
            "film_render.halation.halation_first_sigma_um",
            "film_render.halation.halation_strength",
        ],
    ));
    entries.push(nonruntime(
        SurfaceEntry {
            path: "action.scan_for_print",
            label: "scan for print",
            owner: Owner(Stage::Scan),
            section: "Scanner",
            editor: Action,
            validator: "App::toggle_scan_for_print",
            conditions: &[],
        },
        &["print_render.glare.active"],
    ));
    // Extras vectors are edited independently by the production tuple helper.
    for (path, label) in [
        ("extras.special.film_channel_swap", "film_channel_swap"),
        ("extras.special.print_channel_swap", "print_channel_swap"),
    ] {
        let mut entry = nonruntime(
            SurfaceEntry {
                path,
                label,
                owner: Owner(Stage::Input),
                section: "Convert",
                editor: Tuple,
                validator: "state::validation channel permutation",
                conditions: &["captured-negative Convert route"],
            },
            &[],
        );
        entry.components = (0..3)
            .map(|index| ControlComponent {
                index,
                id: index.to_string(),
                label: format!("{label} {index}"),
                derived: None,
            })
            .collect();
        entries.push(entry);
    }
}

/// Non-runtime paths name their real persisted carrier; action identities do
/// not pretend to be serialized numeric/boolean parameters.
const SURFACES: &[SurfaceEntry] = &[
    SurfaceEntry {
        path: "viewer.reveal",
        label: "reveal transition",
        owner: ControlOwner::Viewer,
        section: "Viewer",
        editor: EditorKind::Toggle,
        validator: "display::DisplaySettings",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "viewer.crossfade",
        label: "crossfade transition",
        owner: ControlOwner::Viewer,
        section: "Viewer",
        editor: EditorKind::Toggle,
        validator: "display::DisplaySettings",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "action.save_retained",
        label: "SAVE",
        owner: ControlOwner::Result,
        section: "Retained image",
        editor: EditorKind::Action,
        validator: "App::save_dialog",
        conditions: &["retained image"],
    },
    SurfaceEntry {
        path: "action.confirm_stale_save",
        label: "Continue saving retained pixels",
        owner: ControlOwner::Result,
        section: "Retained image",
        editor: EditorKind::Action,
        validator: "App::show_stale_save_confirmation",
        conditions: &["stale Save confirmation is open"],
    },
    SurfaceEntry {
        path: "action.cancel_stale_save",
        label: "Cancel",
        owner: ControlOwner::Result,
        section: "Retained image",
        editor: EditorKind::Action,
        validator: "App::show_stale_save_confirmation",
        conditions: &["stale Save confirmation is open"],
    },
    SurfaceEntry {
        path: "action.save_photo_record",
        label: "Save adjustments…",
        owner: ControlOwner::Header,
        section: "Photograph records",
        editor: EditorKind::Action,
        validator: "App::save_photograph_record_to",
        conditions: &["loaded source; accepted adjustments only"],
    },
    SurfaceEntry {
        path: "action.reopen_photo_record",
        label: "Reopen adjustments…",
        owner: ControlOwner::Header,
        section: "Photograph records",
        editor: EditorKind::Action,
        validator: "App::reopen_photograph_record",
        conditions: &[],
    },
    SurfaceEntry {
        path: "action.work_list",
        label: "Photographs…",
        owner: ControlOwner::Header,
        section: "Work list",
        editor: EditorKind::Action,
        validator: "App::work_list_controls",
        conditions: &[],
    },
    SurfaceEntry {
        path: "photo_work.list.add",
        label: "Add photographs…",
        owner: ControlOwner::WorkList,
        section: "Work list",
        editor: EditorKind::Action,
        validator: "App::add_work_paths",
        conditions: &[],
    },
    SurfaceEntry {
        path: "photo_work.list.previous",
        label: "Previous photograph",
        owner: ControlOwner::WorkList,
        section: "Work list",
        editor: EditorKind::Action,
        validator: "App::perform_work_select",
        conditions: &["adjacent work list entry"],
    },
    SurfaceEntry {
        path: "photo_work.list.next",
        label: "Next photograph",
        owner: ControlOwner::WorkList,
        section: "Work list",
        editor: EditorKind::Action,
        validator: "App::perform_work_select",
        conditions: &["adjacent work list entry"],
    },
    SurfaceEntry {
        path: "photo_work.list.remove",
        label: "Remove entry",
        owner: ControlOwner::WorkList,
        section: "Work list",
        editor: EditorKind::Action,
        validator: "App::perform_work_remove",
        conditions: &["work list entry; removes membership, not files"],
    },
    SurfaceEntry {
        path: "photo_work.list.copy_scope",
        label: "Choose an adjustment scope…",
        owner: ControlOwner::WorkList,
        section: "Work list",
        editor: EditorKind::Choice,
        validator: "photograph::worklist::CopyScope",
        conditions: &[],
    },
    SurfaceEntry {
        path: "photo_work.list.copy",
        label: "Copy scope to selected destinations",
        owner: ControlOwner::WorkList,
        section: "Work list",
        editor: EditorKind::Action,
        validator: "App::copy_work_adjustments",
        conditions: &["loaded source; selected destinations; explicit scope"],
    },
    SurfaceEntry {
        path: "photo_work.list.choose_directory",
        label: "Choose output directory…",
        owner: ControlOwner::WorkList,
        section: "Work list",
        editor: EditorKind::Action,
        validator: "App::pick_work_output_directory",
        conditions: &[],
    },
    SurfaceEntry {
        path: "photo_work.list.replace",
        label: "Allow replacement of existing output files",
        owner: ControlOwner::WorkList,
        section: "Work list",
        editor: EditorKind::Toggle,
        validator: "App::submit_work_exports",
        conditions: &[],
    },
    SurfaceEntry {
        path: "photo_work.list.submit",
        label: "Submit selected photographs",
        owner: ControlOwner::WorkList,
        section: "Work list",
        editor: EditorKind::Action,
        validator: "App::submit_work_exports",
        conditions: &["selected destinations; output directory"],
    },
    SurfaceEntry {
        path: "photo.queue.cancel_remaining",
        label: "Cancel remaining queue entries",
        owner: ControlOwner::WorkList,
        section: "Export queue",
        editor: EditorKind::Action,
        validator: "App::cancel_remaining_exports",
        conditions: &["pending queued exports"],
    },
    SurfaceEntry {
        path: "photo.queue.cancel",
        label: "Cancel this export",
        owner: ControlOwner::WorkList,
        section: "Export queue",
        editor: EditorKind::Action,
        validator: "App::cancel_queued_export",
        conditions: &["pending queue entry; first eligible entry"],
    },
    SurfaceEntry {
        path: "photo.queue.retry",
        label: "Retry submitted snapshot",
        owner: ControlOwner::WorkList,
        section: "Export queue",
        editor: EditorKind::Action,
        validator: "App::retry_queued_export",
        conditions: &["failed queue entry; first eligible entry"],
    },
    SurfaceEntry {
        path: "presets.discovery.search",
        label: "Search presets",
        owner: ControlOwner::Presets,
        section: "Preset discovery",
        editor: EditorKind::Text,
        validator: "presets::Browser::update_search",
        conditions: &[],
    },
    SurfaceEntry {
        path: "presets.discovery.all",
        label: "All looks",
        owner: ControlOwner::Presets,
        section: "Preset discovery",
        editor: EditorKind::Action,
        validator: "presets::discovery::Filter",
        conditions: &[],
    },
    SurfaceEntry {
        path: "presets.discovery.favorites",
        label: "Favorites",
        owner: ControlOwner::Presets,
        section: "Preset discovery",
        editor: EditorKind::Action,
        validator: "presets::discovery::Filter",
        conditions: &["favorites retain built-in or user group identity"],
    },
    SurfaceEntry {
        path: "presets.discovery.recent",
        label: "Recently used",
        owner: ControlOwner::Presets,
        section: "Preset discovery",
        editor: EditorKind::Action,
        validator: "presets::discovery::Filter",
        conditions: &[],
    },
    SurfaceEntry {
        path: "presets.discovery.reset",
        label: "Clear search and filters",
        owner: ControlOwner::Presets,
        section: "Preset discovery",
        editor: EditorKind::Action,
        validator: "presets::Browser::update_search",
        conditions: &["no matching presets"],
    },
    SurfaceEntry {
        path: "extras.simulation.film_stock",
        label: "film profile",
        owner: ControlOwner::Stage(Stage::Film),
        section: "Film material",
        editor: EditorKind::Profile,
        validator: "App::select_film_profile",
        conditions: &[],
    },
    SurfaceEntry {
        path: "extras.simulation.print_paper",
        label: "print profile",
        owner: ControlOwner::Stage(Stage::Print),
        section: "Print material",
        editor: EditorKind::Profile,
        validator: "App::select_print_profile",
        conditions: &[],
    },
    SurfaceEntry {
        path: "extras.simulation.saving_color_space",
        label: "saving color space",
        owner: ControlOwner::Stage(Stage::Output),
        section: "Output",
        editor: EditorKind::Choice,
        validator: "GuiState::runtime_params saving conversion validation",
        conditions: &[],
    },
    SurfaceEntry {
        path: "extras.simulation.saving_cctf_encoding",
        label: "saving cctf encoding",
        owner: ControlOwner::Stage(Stage::Output),
        section: "Output",
        editor: EditorKind::Toggle,
        validator: "GuiState::runtime_params saving conversion validation",
        conditions: &[],
    },
    SurfaceEntry {
        path: "extras.load_raw.white_balance",
        label: "white balance",
        owner: ControlOwner::Stage(Stage::Input),
        section: "Import Raw",
        editor: EditorKind::Choice,
        validator: "spektrafilm_raw::RawOptions",
        conditions: &["RAW source; explicit Reprocess RAW"],
    },
    SurfaceEntry {
        path: "extras.load_raw.temperature",
        label: "temperature",
        owner: ControlOwner::Stage(Stage::Input),
        section: "Import Raw",
        editor: EditorKind::Numeric,
        validator: "spektrafilm_raw::RawOptions",
        conditions: &["RAW source; custom white balance; explicit Reprocess RAW"],
    },
    SurfaceEntry {
        path: "extras.load_raw.tint",
        label: "tint",
        owner: ControlOwner::Stage(Stage::Input),
        section: "Import Raw",
        editor: EditorKind::Numeric,
        validator: "spektrafilm_raw::RawOptions",
        conditions: &["RAW source; custom white balance; explicit Reprocess RAW"],
    },
    SurfaceEntry {
        path: "extras.load_raw.lens_correction",
        label: "lens correction",
        owner: ControlOwner::Stage(Stage::Input),
        section: "Import Raw",
        editor: EditorKind::Toggle,
        validator: "spektrafilm_raw::RawOptions",
        conditions: &["RAW source; explicit Reprocess RAW"],
    },
    SurfaceEntry {
        path: "extras.display.use_display_transform",
        label: "use display transform",
        owner: ControlOwner::Settings,
        section: "Display",
        editor: EditorKind::Toggle,
        validator: "display::DisplaySettings",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "extras.display.gray_18_canvas",
        label: "gray 18% canvas",
        owner: ControlOwner::Settings,
        section: "Display",
        editor: EditorKind::Toggle,
        validator: "display::DisplaySettings",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "extras.display.white_padding",
        label: "white padding",
        owner: ControlOwner::Settings,
        section: "Display",
        editor: EditorKind::Numeric,
        validator: "display::DisplaySettings",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "extras.display.output_interpolation",
        label: "output interpolation",
        owner: ControlOwner::Settings,
        section: "Display",
        editor: EditorKind::Choice,
        validator: "display::raster::Interpolation",
        conditions: &["viewer output layer only"],
    },
    SurfaceEntry {
        path: "extras.rust.display_profile",
        label: "select ICC profile…",
        owner: ControlOwner::Settings,
        section: "Display",
        editor: EditorKind::Profile,
        validator: "GuiState::set_display_profile / lcms2::Profile",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "extras.rust.export_backend",
        label: "Backend",
        owner: ControlOwner::Export,
        section: "Export options",
        editor: EditorKind::Choice,
        validator: "export::ExportOptions::normalize",
        conditions: &[],
    },
    SurfaceEntry {
        path: "extras.rust.export_format",
        label: "Format",
        owner: ControlOwner::Export,
        section: "Export options",
        editor: EditorKind::Choice,
        validator: "export::ExportOptions::normalize",
        conditions: &[],
    },
    SurfaceEntry {
        path: "extras.rust.save_bit_depth",
        label: "Bit depth",
        owner: ControlOwner::Export,
        section: "Export options",
        editor: EditorKind::Choice,
        validator: "export::ExportOptions::normalize",
        conditions: &["format-dependent"],
    },
    SurfaceEntry {
        path: "extras.rust.jpeg_quality",
        label: "JPEG quality",
        owner: ControlOwner::Export,
        section: "Export options",
        editor: EditorKind::Numeric,
        validator: "core::image_io::save_option_violations",
        conditions: &["JPEG format"],
    },
    SurfaceEntry {
        path: "extras.rust.jpeg_subsampling",
        label: "JPEG subsampling",
        owner: ControlOwner::Export,
        section: "Export options",
        editor: EditorKind::Choice,
        validator: "core::image_io::save_option_violations",
        conditions: &["JPEG format"],
    },
    SurfaceEntry {
        path: "extras.rust.export_compression",
        label: "Compression",
        owner: ControlOwner::Export,
        section: "Export options",
        editor: EditorKind::Choice,
        validator: "core::image_io::save_option_violations",
        conditions: &["TIFF format; EXR fixed ZIP"],
    },
    SurfaceEntry {
        path: "extras.rust.export_saving_color_space",
        label: "Saving color space",
        owner: ControlOwner::Export,
        section: "Export options",
        editor: EditorKind::Choice,
        validator: "core::image_io::save_option_violations",
        conditions: &["format-dependent color spaces"],
    },
    SurfaceEntry {
        path: "extras.rust.export_saving_cctf_encoding",
        label: "Apply saving CCTF encoding",
        owner: ControlOwner::Export,
        section: "Export options",
        editor: EditorKind::Toggle,
        validator: "core::image_io::save_option_violations",
        conditions: &["TIFF editable; JPEG/PNG encoded, EXR linear"],
    },
    SurfaceEntry {
        path: "action.reprocess_raw",
        label: "Reprocess RAW",
        owner: ControlOwner::Stage(Stage::Input),
        section: "Import Raw",
        editor: EditorKind::Action,
        validator: "App::load_image_from_path",
        conditions: &["RAW source"],
    },
    SurfaceEntry {
        path: "action.rotate_ccw",
        label: "ccw rotate",
        owner: ControlOwner::Stage(Stage::Input),
        section: "Source rotation",
        editor: EditorKind::Action,
        validator: "App::rotate_input_image_counterclockwise",
        conditions: &["loaded source"],
    },
    SurfaceEntry {
        path: "action.rotate_cw",
        label: "cw rotate",
        owner: ControlOwner::Stage(Stage::Input),
        section: "Source rotation",
        editor: EditorKind::Action,
        validator: "App::rotate_input_image_clockwise",
        conditions: &["loaded source"],
    },
    SurfaceEntry {
        path: "action.detect_base",
        label: "detect base",
        owner: ControlOwner::Stage(Stage::Input),
        section: "Convert",
        editor: EditorKind::Action,
        validator: "CalibrationAction::DetectBase",
        conditions: &["loaded captured negative"],
    },
    SurfaceEntry {
        path: "action.blind_calibration",
        label: "blind calibration",
        owner: ControlOwner::Stage(Stage::Input),
        section: "Convert",
        editor: EditorKind::Action,
        validator: "CalibrationAction::BlindCalibration",
        conditions: &["loaded captured negative"],
    },
    SurfaceEntry {
        path: "action.neutralize_filters",
        label: "neutralize print filters",
        owner: ControlOwner::Stage(Stage::Input),
        section: "Convert",
        editor: EditorKind::Action,
        validator: "CalibrationAction::NeutralizeFilters",
        conditions: &[],
    },
    SurfaceEntry {
        path: "action.save_state",
        label: "Save current to file",
        owner: ControlOwner::Settings,
        section: "GUI parameters",
        editor: EditorKind::Action,
        validator: "App::save_state_with_dialog",
        conditions: &[],
    },
    SurfaceEntry {
        path: "action.load_state",
        label: "Load from file",
        owner: ControlOwner::Settings,
        section: "GUI parameters",
        editor: EditorKind::Action,
        validator: "App::load_state_with_dialog",
        conditions: &[],
    },
    SurfaceEntry {
        path: "action.save_startup",
        label: "Save current as default",
        owner: ControlOwner::Settings,
        section: "GUI parameters",
        editor: EditorKind::Action,
        validator: "GuiState::save",
        conditions: &[],
    },
    SurfaceEntry {
        path: "action.restore_factory",
        label: "Restore factory default",
        owner: ControlOwner::Settings,
        section: "GUI parameters",
        editor: EditorKind::Action,
        validator: "App::factory_reset_confirmation",
        conditions: &["confirmation required"],
    },
    SurfaceEntry {
        path: "action.clear_display_profile",
        label: "clear",
        owner: ControlOwner::Settings,
        section: "Display",
        editor: EditorKind::Action,
        validator: "GuiState::set_display_profile",
        conditions: &["selected ICC profile"],
    },
    SurfaceEntry {
        path: "diagnostics.collection_mode",
        label: "Collection mode",
        owner: ControlOwner::Settings,
        section: "Diagnostics",
        editor: EditorKind::Choice,
        validator: "telemetry::CollectionMode",
        conditions: &["session-only; next operation"],
    },
    SurfaceEntry {
        path: "action.open_diagnostic_directory",
        label: "Open log folder",
        owner: ControlOwner::Settings,
        section: "Diagnostics",
        editor: EditorKind::Action,
        validator: "diagnostics::Action::OpenDirectory",
        conditions: &[],
    },
    SurfaceEntry {
        path: "action.save_report",
        label: "Save report…",
        owner: ControlOwner::Settings,
        section: "Diagnostics",
        editor: EditorKind::Action,
        validator: "diagnostics::Diagnostics::save_selected",
        conditions: &["selected report"],
    },
    SurfaceEntry {
        path: "action.export",
        label: "Export file…",
        owner: ControlOwner::Export,
        section: "Export options",
        editor: EditorKind::Action,
        validator: "ExportDialog::show",
        conditions: &["loaded source; immutable submitted request"],
    },
    SurfaceEntry {
        path: "action.cancel_export_draft",
        label: "Cancel",
        owner: ControlOwner::Export,
        section: "Export options",
        editor: EditorKind::Action,
        validator: "ExportDialog::show",
        conditions: &["unsubmitted export draft"],
    },
    SurfaceEntry {
        path: "viewer.layer.input",
        label: "Original",
        owner: ControlOwner::Viewer,
        section: "Viewer",
        editor: EditorKind::Action,
        validator: "Viewer::layer_controls",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "viewer.layer.output",
        label: "Modified",
        owner: ControlOwner::Viewer,
        section: "Viewer",
        editor: EditorKind::Action,
        validator: "Viewer::layer_controls",
        conditions: &["retained output; viewer-only"],
    },
    SurfaceEntry {
        path: "viewer.layer.paper_back",
        label: "Paper back",
        owner: ControlOwner::Viewer,
        section: "Viewer",
        editor: EditorKind::Action,
        validator: "Viewer::layer_controls",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "viewer.zoom.100",
        label: "100%",
        owner: ControlOwner::Viewer,
        section: "Viewer",
        editor: EditorKind::Action,
        validator: "Viewer::set_zoom_percent",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "viewer.zoom.200",
        label: "200%",
        owner: ControlOwner::Viewer,
        section: "Viewer",
        editor: EditorKind::Action,
        validator: "Viewer::set_zoom_percent",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "viewer.zoom.400",
        label: "400%",
        owner: ControlOwner::Viewer,
        section: "Viewer",
        editor: EditorKind::Action,
        validator: "Viewer::set_zoom_percent",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "viewer.fit",
        label: "fit",
        owner: ControlOwner::Viewer,
        section: "Viewer",
        editor: EditorKind::Action,
        validator: "Viewer::fit_view",
        conditions: &["viewer-only"],
    },
    SurfaceEntry {
        path: "viewer.reset",
        label: "reset view",
        owner: ControlOwner::Viewer,
        section: "Viewer",
        editor: EditorKind::Action,
        validator: "Viewer::reset_view",
        conditions: &["viewer-only"],
    },
];

const LABELS: &[(&str, &str)] = &[
    ("io.input_gamut_compress.boundary", "boundary"),
    ("camera.auto_exposure", "auto exposure"),
    ("camera.auto_exposure_method", "auto exposure method"),
    ("camera.capture_illuminant", "capture illuminant"),
    ("camera.color_filter", "color filter"),
    ("camera.diffusion_filter.active", "Camera diffusion"),
    ("camera.diffusion_filter.bloom_intensity", "bloom intensity"),
    ("camera.diffusion_filter.bloom_size", "bloom size"),
    ("camera.diffusion_filter.core_intensity", "core intensity"),
    ("camera.diffusion_filter.core_size", "core size"),
    ("camera.diffusion_filter.filter_family", "filter family"),
    ("camera.diffusion_filter.halo_intensity", "halo intensity"),
    ("camera.diffusion_filter.halo_size", "halo size"),
    ("camera.diffusion_filter.halo_warmth", "halo warmth"),
    ("camera.diffusion_filter.spatial_scale", "spatial scale"),
    ("camera.diffusion_filter.strength", "strength"),
    (
        "camera.exposure_compensation_ev",
        "exposure compensation ev",
    ),
    ("camera.film_format_mm", "Film Format"),
    ("cmyk.active", "Enable CMYK ink style"),
    ("cmyk.strength", "strength"),
    ("enlarger.diffusion_filter.active", "Enlarger diffusion"),
    (
        "enlarger.diffusion_filter.bloom_intensity",
        "bloom intensity",
    ),
    ("enlarger.diffusion_filter.bloom_size", "bloom size"),
    ("enlarger.diffusion_filter.core_intensity", "core intensity"),
    ("enlarger.diffusion_filter.core_size", "core size"),
    ("enlarger.diffusion_filter.filter_family", "filter family"),
    ("enlarger.diffusion_filter.halo_intensity", "halo intensity"),
    ("enlarger.diffusion_filter.halo_size", "halo size"),
    ("enlarger.diffusion_filter.halo_warmth", "halo warmth"),
    ("enlarger.diffusion_filter.spatial_scale", "spatial scale"),
    ("enlarger.diffusion_filter.strength", "strength"),
    ("enlarger.illuminant", "print illuminant"),
    ("enlarger.m_filter_shift", "print m filter shift"),
    ("enlarger.preflash_exposure", "preflash exposure"),
    (
        "enlarger.preflash_m_filter_shift",
        "preflash m filter shift",
    ),
    (
        "enlarger.preflash_y_filter_shift",
        "preflash y filter shift",
    ),
    ("enlarger.print_exposure", "print exposure"),
    ("enlarger.print_exposure_mode", "print exposure mode"),
    ("enlarger.y_filter_shift", "print y filter shift"),
    ("film_render.base.active", "Enable film base"),
    ("film_render.base.cyan", "cyan"),
    ("film_render.base.magenta", "magenta"),
    ("film_render.base.scale", "scale"),
    ("film_render.base.tilt", "tilt"),
    ("film_render.base.yellow", "yellow"),
    ("film_render.chemistry.active", "Enable film density curves"),
    (
        "film_render.chemistry.developer_exhaustion",
        "developer exhaustion",
    ),
    ("film_render.chemistry.gamma_factor", "gamma factor"),
    (
        "film_render.chemistry.gamma_factor_blue",
        "gamma factor blue",
    ),
    (
        "film_render.chemistry.gamma_factor_fast",
        "gamma factor fast",
    ),
    (
        "film_render.chemistry.gamma_factor_green",
        "gamma factor green",
    ),
    ("film_render.chemistry.gamma_factor_red", "gamma factor red"),
    (
        "film_render.chemistry.gamma_factor_slow",
        "gamma factor slow",
    ),
    ("film_render.convert.base_percentile", "base percentile"),
    ("film_render.convert.calibration", "calibration"),
    (
        "film_render.convert.exposure_compensation_ev",
        "exposure compensation ev",
    ),
    ("film_render.convert.scan_illuminant", "scan illuminant"),
    ("film_render.density_curve_gamma", "density curve gamma"),
    ("film_render.development_time", "development time"),
    ("film_render.dir_couplers.active", "Enable DIR couplers"),
    ("film_render.dir_couplers.amount", "inhibition amount"),
    (
        "film_render.dir_couplers.diffusion_size_um",
        "diffusion size um",
    ),
    (
        "film_render.dir_couplers.diffusion_tail_um",
        "diffusion tail um",
    ),
    (
        "film_render.dir_couplers.diffusion_tail_weight",
        "diffusion tail weight",
    ),
    (
        "film_render.dir_couplers.gamma_interlayer_b_to_rg",
        "gamma interlayer b to rg",
    ),
    (
        "film_render.dir_couplers.gamma_interlayer_g_to_rb",
        "gamma interlayer g to rb",
    ),
    (
        "film_render.dir_couplers.gamma_interlayer_r_to_gb",
        "gamma interlayer r to gb",
    ),
    (
        "film_render.dir_couplers.gamma_samelayer_rgb",
        "gamma samelayer rgb",
    ),
    (
        "film_render.dir_couplers.inhibition_interlayer",
        "inhibition interlayer",
    ),
    (
        "film_render.dir_couplers.inhibition_samelayer",
        "inhibition samelayer",
    ),
    (
        "film_render.dir_couplers.langmuir_donor_k_rgb",
        "langmuir donor k rgb",
    ),
    (
        "film_render.dir_couplers.langmuir_receiver_k_rgb",
        "langmuir receiver k rgb",
    ),
    ("film_render.grain.active", "Enable grain"),
    ("film_render.grain.blur", "blur"),
    ("film_render.grain.blur_dye_clouds_um", "dye-cloud blur"),
    (
        "film_render.grain.channel_correlation",
        "Channel correlation",
    ),
    ("film_render.grain.density_min", "density min"),
    ("film_render.grain.engine", "Engine"),
    ("film_render.grain.fotufilm.amount", "Amount"),
    (
        "film_render.grain.fotufilm.clump_radius_um",
        "Cloud radius (µm)",
    ),
    (
        "film_render.grain.fotufilm.coating_density_scale",
        "Coating density scale",
    ),
    ("film_render.grain.fotufilm.color", "Color"),
    ("film_render.grain.fotufilm.fog_density", "Fog density"),
    (
        "film_render.grain.fotufilm.layer_size_ratio",
        "Layer size ratio",
    ),
    ("film_render.grain.fotufilm.layer_weights", "Layer weights"),
    ("film_render.grain.fotufilm.layers", "Layer amounts"),
    ("film_render.grain.fotufilm.material", "Material"),
    ("film_render.grain.fotufilm.radius_span", "Radius span"),
    ("film_render.grain.fotufilm.rms_density", "RMS density"),
    ("film_render.grain.fotufilm.size", "Size"),
    ("film_render.grain.fotufilm.softness", "Softness"),
    (
        "film_render.grain.fotufilm.sublayer_shares",
        "Sublayer shares",
    ),
    ("film_render.grain.micro_structure", "micro structure"),
    ("film_render.grain.mult_usm_amount", "mult usm amount"),
    ("film_render.grain.mult_usm_sigma", "mult usm sigma"),
    (
        "film_render.grain.particle_scale_sublayers",
        "particle scale sublayers",
    ),
    ("film_render.grain.rms_granularity", "rms granularity"),
    ("film_render.grain.uniformity", "uniformity"),
    ("film_render.grain.v2_amount", "Amount"),
    ("film_render.grain.v2_chroma", "Chroma"),
    ("film_render.grain.v2_film_type", "Film Type"),
    ("film_render.grain.v2_highlights", "Highlights"),
    ("film_render.grain.v2_midtones", "Midtones"),
    ("film_render.grain.v2_mode", "Processing Mode"),
    ("film_render.grain.v2_profile", "Procedural Grain Profiles"),
    ("film_render.grain.v2_resolution_factor", "Film Resolution"),
    ("film_render.grain.v2_resolution_type", "Resolution Type"),
    ("film_render.grain.v2_shadows", "Shadows"),
    ("film_render.grain.v2_size", "Size"),
    ("film_render.grain.v2_timer", "Timer"),
    (
        "film_render.grain.v3_dye_support_um",
        "Dye support (spatial formation)",
    ),
    (
        "film_render.grain.v3_event_area_um2",
        "Event area (strength)",
    ),
    ("film_render.halation.active", "Enable halation"),
    ("film_render.halation.boost_ev", "boost ev"),
    ("film_render.halation.boost_range", "boost range"),
    ("film_render.halation.halation_amount", "halation amount"),
    (
        "film_render.halation.halation_bounce_decay",
        "halation bounce decay",
    ),
    (
        "film_render.halation.halation_first_sigma_um",
        "halation first sigma um",
    ),
    (
        "film_render.halation.halation_n_bounces",
        "halation n bounces",
    ),
    (
        "film_render.halation.halation_renormalize",
        "halation renormalize",
    ),
    (
        "film_render.halation.halation_spatial_scale",
        "halation spatial scale",
    ),
    (
        "film_render.halation.halation_strength",
        "halation strength",
    ),
    ("film_render.halation.protect_ev", "protect ev"),
    ("film_render.halation.scatter_amount", "scatter amount"),
    ("film_render.halation.scatter_core_um", "scatter core um"),
    (
        "film_render.halation.scatter_spatial_scale",
        "scatter spatial scale",
    ),
    ("film_render.halation.scatter_tail_um", "scatter tail um"),
    (
        "film_render.halation.scatter_tail_weight",
        "scatter tail weight",
    ),
    (
        "film_render.halation.source_compression",
        "source compression",
    ),
    ("io.crop", "crop"),
    ("io.crop_center", "crop center"),
    ("io.crop_size", "crop size"),
    ("io.input_cctf_decoding", "apply cctf decoding"),
    ("io.input_color_space", "input color space"),
    (
        "io.input_gamut_compress.active",
        "Enable input gamut compress",
    ),
    ("io.input_gamut_compress.algorithm", "algorithm"),
    ("io.input_gamut_compress.hull_detail", "hull detail"),
    ("io.input_gamut_compress.knee", "knee"),
    ("io.output_cctf_encoding", "output cctf encoding"),
    ("io.output_color_space", "output color space"),
    ("io.output_gamut_compress.algorithm", "algorithm"),
    ("io.output_gamut_compress.knee", "knee"),
    ("io.upscale_factor", "upscale factor"),
    ("output_mix.amount", "Simulation blend"),
    ("print_render.base.active", "Enable print base"),
    ("print_render.base.cyan", "cyan"),
    ("print_render.base.magenta", "magenta"),
    ("print_render.base.scale", "scale"),
    ("print_render.base.yellow", "yellow"),
    ("print_render.density_curve_gamma", "density curve gamma"),
    (
        "print_render.density_curves_morph.active",
        "Enable print density curves",
    ),
    (
        "print_render.density_curves_morph.developer_exhaustion",
        "developer exhaustion",
    ),
    (
        "print_render.density_curves_morph.gamma_factor",
        "gamma factor",
    ),
    (
        "print_render.density_curves_morph.gamma_factor_blue",
        "gamma factor blue",
    ),
    (
        "print_render.density_curves_morph.gamma_factor_fast",
        "gamma factor fast",
    ),
    (
        "print_render.density_curves_morph.gamma_factor_green",
        "gamma factor green",
    ),
    (
        "print_render.density_curves_morph.gamma_factor_red",
        "gamma factor red",
    ),
    (
        "print_render.density_curves_morph.gamma_factor_slow",
        "gamma factor slow",
    ),
    ("print_render.development_time", "development time"),
    ("print_render.glare.active", "Enable print glare"),
    ("print_render.glare.blur", "Blur (viewing glare)"),
    ("print_render.glare.percent", "percent"),
    ("print_render.glare.roughness", "roughness"),
    ("scanner.conversion_style", "conversion style"),
    ("scanner.lens_blur", "Lens blur (readout optics)"),
    ("scanner.operator.exposure_ev", "Finished exposure"),
    ("scanner.operator.warmth_ev", "Warmth"),
    ("scanner.operator.tint_ev", "Green–magenta tint"),
    ("scanner.operator.contrast", "Contrast"),
    ("scanner.scan_output", "scan output"),
    ("scanner.unsharp_mask", "Unsharp mask (digital sharpening)"),
    (
        "settings.apply_hanatos2025_adaptation_surface",
        "hanatos2025 adaptation surface",
    ),
    (
        "settings.apply_hanatos2025_adaptation_window",
        "hanatos2025 adaptation window",
    ),
    ("settings.preview_max_size", "preview max size"),
    (
        "settings.print_exposure_ev_tracking",
        "print exposure ev tracking",
    ),
    ("settings.rgb_to_raw_method", "spectral upsampling"),
    ("settings.spectral_gaussian_blur", "spectral gaussian blur"),
    ("settings.use_cat16", "use cat16"),
];
