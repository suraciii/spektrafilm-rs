//! Cross-owner discovery over the canonical inventory: no rendering, profile
//! loading, editor mutation, or visibility-dependent omissions.
use super::{
    ScanMedium,
    inventory::{ControlOwner, inventory},
};
use serde_json::Value;
use spektrafilm_core::params::RuntimeParams;
use std::{path::Path, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SearchResult {
    pub(crate) owner: ControlOwner,
    pub(crate) section: &'static str,
    pub(crate) labels: Vec<String>,
    pub(crate) target_path: String,
    details: String,
}
impl SearchResult {
    pub(crate) fn locator_label(&self) -> String {
        format!("Locate {}: {}", self.owner.label(), self.section)
    }

    pub(crate) fn details(&self) -> &str {
        &self.details
    }
}

#[cfg(test)]
fn build_filtered(
    query: &str,
    params: &RuntimeParams,
    extras: &Value,
    only_modified: bool,
) -> Vec<SearchResult> {
    let projection = only_modified.then(|| super::control_projection(params, extras));
    build_with_projection(query, projection.as_ref())
}

fn build_with_projection(
    query: &str,
    projection: Option<&super::ControlProjection>,
) -> Vec<SearchResult> {
    if query.is_empty() && projection.is_none() {
        return Vec::new();
    }
    let query = query.to_lowercase();
    let mut results: Vec<SearchResult> = Vec::new();
    for entry in inventory() {
        if !entry.searchable()
            || !entry.matches(&query)
            || projection.is_some_and(|projection| {
                !entry.is_modified(&projection.runtime, &projection.extras)
            })
        {
            continue;
        }
        let index = results
            .iter()
            .position(|result| result.owner == entry.owner && result.section == entry.section);
        let result = if let Some(index) = index {
            &mut results[index]
        } else {
            results.push(SearchResult {
                owner: entry.owner,
                section: entry.section,
                labels: Vec::new(),
                target_path: entry.path.clone(),
                details: String::new(),
            });
            results.last_mut().expect("just inserted result")
        };
        if !result.labels.contains(&entry.label) {
            result.labels.push(entry.label.clone());
        }
        if !result.details.is_empty() {
            result.details.push_str("\n\n");
        }
        result.details.push_str(&entry.description());
    }
    results
}

/// Photographic edits own revision keys; modified-only results also compare
/// session/viewer extras and preview size, which do not advance that revision.
/// Inactive controls remain discoverable regardless of applicability.
#[derive(Default)]
pub(crate) struct SearchIndex {
    query: String,
    revision: u64,
    medium: Option<ScanMedium>,
    only_modified: bool,
    modified_extras: Option<Arc<Value>>,
    preview_max_size: u32,
    built: bool,
    results: Vec<SearchResult>,
}
impl SearchIndex {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn results_filtered(
        &mut self,
        query: &str,
        params: &RuntimeParams,
        extras: &Value,
        medium: ScanMedium,
        revision: u64,
        _data_dir: &Path,
        only_modified: bool,
    ) -> &[SearchResult] {
        if query.is_empty() && !only_modified {
            self.reset();
            return &self.results;
        }
        let projection = only_modified.then(|| super::control_projection(params, extras));
        let preferences_changed = projection.as_ref().is_some_and(|projection| {
            self.preview_max_size != params.settings.preview_max_size
                || self.modified_extras.as_deref() != Some(projection.extras.as_ref())
        });
        if !self.built
            || self.query != query
            || self.revision != revision
            || self.medium != Some(medium)
            || self.only_modified != only_modified
            || preferences_changed
        {
            self.query.clear();
            self.query.push_str(query);
            self.revision = revision;
            self.medium = Some(medium);
            self.only_modified = only_modified;
            self.results = build_with_projection(query, projection.as_ref());
            self.modified_extras = projection.map(|projection| projection.extras);
            self.preview_max_size = params.settings.preview_max_size;
            self.built = true;
        }
        &self.results
    }

    pub(crate) fn reset(&mut self) {
        self.query.clear();
        self.results.clear();
        self.medium = None;
        self.modified_extras = None;
        self.built = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        controls::{
            Stage,
            inventory::{EditorKind, lookup},
        },
        state::GuiState,
    };

    fn factory() -> (RuntimeParams, Value) {
        let state = GuiState::factory();
        (state.runtime_params().unwrap(), state.sections)
    }

    #[test]
    fn canonical_destination_and_uppercase_custom_rows_use_the_same_identity() {
        let (params, extras) = factory();
        let found = build_filtered("film_render.grain.fotufilm.size", &params, &extras, false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].target_path, "film_render.grain.fotufilm.size");
        for path in [
            "extras.simulation.film_stock",
            "film_render.development_time",
        ] {
            assert!(super::super::entry_matches(
                path,
                &path.to_uppercase(),
                &params,
                &extras,
                false
            ));
        }
    }

    #[test]
    fn canonical_path_and_label_queries_retain_inactive_engines() {
        let (params, extras) = factory();
        for (query, label) in [
            ("FILM_RENDER.GRAIN.FOTUFILM.SIZE", "Size"),
            ("Dye support (µm)", "Dye support (µm)"),
            ("film_render.grain.v2_resolution_factor", "Film Resolution"),
            ("halation spatial scale", "halation spatial scale"),
        ] {
            let found = build_filtered(query, &params, &extras, false);
            assert!(
                found
                    .iter()
                    .any(|result| result.owner == ControlOwner::Stage(Stage::Film)
                        && result.labels.iter().any(|candidate| candidate == label)),
                "{query}"
            );
        }
    }

    #[test]
    fn operator_fields_and_session_actions_share_the_scanner_destination() {
        let (mut params, extras) = factory();
        params.workflow.route = "input".into();
        let found = build_filtered("Operator correction", &params, &extras, false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].owner, ControlOwner::Stage(Stage::Scan));
        assert_eq!(found[0].section, "Scanner");
        assert_eq!(found[0].labels.len(), 8);
        for (path, label, editor) in [
            (
                "scanner.operator.exposure_ev",
                "Finished exposure",
                EditorKind::Numeric,
            ),
            ("scanner.operator.warmth_ev", "Warmth", EditorKind::Numeric),
            (
                "scanner.operator.tint_ev",
                "Green–magenta tint",
                EditorKind::Numeric,
            ),
            ("scanner.operator.contrast", "Contrast", EditorKind::Numeric),
            (
                "action.operator_restore_automatic",
                "Restore automatic",
                EditorKind::Action,
            ),
            (
                "action.operator_copy",
                "Copy operator correction",
                EditorKind::Action,
            ),
            (
                "action.operator_paste",
                "Paste operator correction",
                EditorKind::Action,
            ),
            (
                "action.operator_compare",
                "Hold automatic comparison",
                EditorKind::Action,
            ),
        ] {
            let entry = lookup(path).unwrap();
            assert_eq!(entry.editor, editor, "{path}");
            assert_eq!(entry.owner, ControlOwner::Stage(Stage::Scan));
            assert_eq!(entry.section, "Scanner");
            assert!(!entry.conditions.is_empty(), "{path}");
            assert!(found[0].labels.iter().any(|candidate| candidate == label));
            let located = build_filtered(path, &params, &extras, false);
            assert_eq!(located.len(), 1, "{path}");
            assert_eq!(located[0].target_path, path);
            assert_eq!(located[0].locator_label(), "Locate Scan: Scanner");
            if editor == EditorKind::Action {
                assert_eq!(entry.affected_paths.len(), 4, "{path}");
            }
        }
    }

    #[test]
    fn inactive_operator_edits_remain_modified_searchable_and_reset_when_reactivated() {
        let (mut params, mut extras) = factory();
        params.workflow.route = "input".into();
        params.scanner.scan_output = "direct_scan".into();
        params.scanner.conversion_style = "standard".into();
        params.scanner.operator = spektrafilm_core::params::ScanOperatorParams {
            exposure_ev: 1.25,
            warmth_ev: 0.35,
            tint_ev: -0.2,
            contrast: 1.4,
        };
        let camera_exposure = params.camera.exposure_compensation_ev;
        let optics = params.scanner.lens_blur;
        let found = build_filtered("Operator correction", &params, &extras, true);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].labels.len(),
            4,
            "session actions are not modified runtime values"
        );
        // Inactive values are GUI storage, not a core-valid render request.
        // Their disabled rows become resettable again on the eligible route.
        params.workflow.route = "input > film > scan".into();
        params.scanner.scan_output = "negative_to_positive".into();
        for path in [
            "scanner.operator.exposure_ev",
            "scanner.operator.warmth_ev",
            "scanner.operator.tint_ev",
            "scanner.operator.contrast",
        ] {
            assert!(crate::section_reset::reset_leaf(
                &mut params,
                &mut extras,
                path
            ));
            assert!(
                build_filtered(path, &params, &extras, true).is_empty(),
                "{path}"
            );
            assert_eq!(build_filtered(path, &params, &extras, false).len(), 1);
        }
        assert_eq!(params.scanner.operator, Default::default());
        assert_eq!(params.camera.exposure_compensation_ev, camera_exposure);
        assert_eq!(params.scanner.lens_blur, optics);
        assert!(build_filtered("Operator correction", &params, &extras, true).is_empty());
        params.scanner.operator.exposure_ev = 0.5;
        assert!(crate::section_reset::is_modified(
            "Scanner", &params, &extras
        ));
        assert!(crate::section_reset::reset(
            &mut params,
            &mut extras,
            "Scanner"
        ));
        assert_eq!(params.scanner.operator, Default::default());
        assert_eq!(params.camera.exposure_compensation_ev, camera_exposure);
    }

    #[test]
    fn results_name_real_dialog_and_viewer_owners() {
        let (params, extras) = factory();
        for (query, owner, section) in [
            ("JPEG quality", ControlOwner::Export, "Export options"),
            (
                "settings.preview_max_size",
                ControlOwner::Settings,
                "Session",
            ),
            ("select ICC profile", ControlOwner::Settings, "Display"),
            ("reset view", ControlOwner::Viewer, "Viewer"),
        ] {
            let found = build_filtered(query, &params, &extras, false);
            assert!(
                found
                    .iter()
                    .any(|result| result.owner == owner && result.section == section)
            );
        }
    }

    #[test]
    fn only_modified_intersects_the_query_and_shared_factory_comparison() {
        let (mut params, extras) = factory();
        assert!(build_filtered("", &params, &extras, true).is_empty());
        params.film_render.halation.halation_amount += 0.5;
        let root = serde_json::to_value(&params).unwrap();
        let entry = lookup("film_render.halation.halation_amount").unwrap();
        assert_eq!(
            entry.is_modified(&root, &extras),
            crate::section_reset::path_modified(&entry.path, &root, &extras)
        );
        let found = build_filtered("halation", &params, &extras, true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].labels, ["halation amount"]);
        assert!(build_filtered("grain", &params, &extras, true).is_empty());
    }

    #[test]
    fn modified_search_refreshes_session_preferences_without_a_photo_revision() {
        let (mut params, mut extras) = factory();
        let original_auto_preview = extras["simulation"]["auto_preview"].clone();
        let original_preview_max_size = params.settings.preview_max_size;
        let ctx = egui::Context::default();
        let mut index = SearchIndex::default();
        for (auto_modified, size_modified) in [
            (false, false),
            (true, false),
            (true, true),
            (false, true),
            (false, false),
        ] {
            extras["simulation"]["auto_preview"] = if auto_modified {
                Value::from(!original_auto_preview.as_bool().unwrap())
            } else {
                original_auto_preview.clone()
            };
            params.settings.preview_max_size = if size_modified {
                original_preview_max_size + 1
            } else {
                original_preview_max_size
            };
            super::super::begin_control_frame(&ctx, &params, &extras);
            let found = index.results_filtered(
                "Session",
                &params,
                &extras,
                ScanMedium::Negative,
                7,
                Path::new("."),
                true,
            );
            if auto_modified || size_modified {
                assert_eq!(found.len(), 1);
                assert_eq!(found[0].owner, ControlOwner::Settings);
                assert_eq!(
                    found[0].labels.iter().any(|label| label == "auto preview"),
                    auto_modified
                );
                assert_eq!(
                    found[0]
                        .labels
                        .iter()
                        .any(|label| label == "preview max size"),
                    size_modified
                );
            } else {
                assert!(found.is_empty());
            }
            super::super::end_control_frame();
        }
    }

    #[test]
    fn extras_modified_comparison_uses_the_same_factory_projection() {
        let (params, mut extras) = factory();
        extras["load_raw"]["temperature"] = serde_json::json!(6200.0);
        let root = serde_json::to_value(&params).unwrap();
        let entry = lookup("extras.load_raw.temperature").unwrap();
        assert!(entry.is_modified(&root, &extras));
        assert_eq!(
            entry.is_modified(&root, &extras),
            crate::section_reset::path_modified(&entry.path, &root, &extras)
        );
        let found = build_filtered("temperature", &params, &extras, true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].section, "Import Raw");
    }

    #[test]
    fn inventory_reuses_core_descriptions_and_covers_components() {
        for entry in inventory() {
            if let Some(description) = &entry.validation {
                assert_eq!(
                    *description,
                    spektrafilm_core::params::sources::describe_field(&entry.path).unwrap()
                );
                if let Some(array) = description["default"].as_array() {
                    assert_eq!(entry.components.len(), array.len(), "{}", entry.path);
                    for (index, component) in entry.components.iter().enumerate() {
                        assert_eq!(component.index, index);
                        assert!(!component.id.is_empty());
                        assert!(!component.label.is_empty());
                    }
                }
            }
        }
        let shares = lookup("film_render.grain.fotufilm.sublayer_shares").unwrap();
        assert!(shares.components[3].derived.is_some());
        assert!(matches!(
            lookup("film_render.grain.micro_sublayers").unwrap().editor,
            EditorKind::Excluded(_)
        ));
        let calibration = lookup("film_render.convert.calibration").unwrap();
        assert_eq!(calibration.components.len(), 9);
        assert_eq!(
            lookup("film_render.development_time").unwrap().section,
            "Film material"
        );
        assert_eq!(
            lookup("print_render.development_time").unwrap().section,
            "Print material"
        );
    }

    #[test]
    fn every_discovered_core_leaf_is_inspectable_without_duplicate_paths() {
        let mut paths = std::collections::BTreeSet::new();
        for entry in inventory() {
            assert!(paths.insert(&entry.path), "duplicate {}", entry.path);
            assert_ne!(
                entry.editor,
                EditorKind::Unconnected,
                "supported control lacks an editor: {}",
                entry.path
            );
        }
        for group in spektrafilm_core::params::sources::describe_root_groups() {
            let description = spektrafilm_core::params::sources::describe_module(&group).unwrap();
            for field in description["fields"].as_array().unwrap() {
                assert!(lookup(field["path"].as_str().unwrap()).is_some());
            }
        }
    }

    #[test]
    fn module_queries_and_locators_use_canonical_owner_names() {
        let (params, extras) = factory();
        let found = build_filtered("Preflash", &params, &extras, false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].owner, ControlOwner::Stage(Stage::Print));
        assert_eq!(
            found[0].locator_label(),
            format!("Locate {}: Preflash", Stage::Print.label())
        );
        assert!(
            found[0]
                .labels
                .iter()
                .all(|label| label.starts_with("preflash"))
        );
    }
}
