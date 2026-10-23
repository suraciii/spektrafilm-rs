//! Cross-owner discovery over the canonical inventory: no rendering, profile
//! loading, editor mutation, or visibility-dependent omissions.
use super::{
    ScanMedium,
    inventory::{ControlOwner, inventory},
};
use serde_json::Value;
use spektrafilm_core::params::RuntimeParams;
use std::path::Path;

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

fn build_filtered(
    query: &str,
    params: &RuntimeParams,
    extras: &Value,
    only_modified: bool,
) -> Vec<SearchResult> {
    if query.is_empty() && !only_modified {
        return Vec::new();
    }
    let projection = only_modified.then(|| super::control_projection(params, extras));
    let query = query.to_lowercase();
    let mut results: Vec<SearchResult> = Vec::new();
    for entry in inventory() {
        if !entry.searchable()
            || !entry.matches(&query)
            || projection.as_ref().is_some_and(|projection| {
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

/// Revision keys are owned by accepted edits. Inactive controls remain in the
/// result set: applicability describes availability, not inventory membership.
#[derive(Default)]
pub(crate) struct SearchIndex {
    query: String,
    revision: u64,
    medium: Option<ScanMedium>,
    only_modified: bool,
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
        if !self.built
            || self.query != query
            || self.revision != revision
            || self.medium != Some(medium)
            || self.only_modified != only_modified
        {
            self.query.clear();
            self.query.push_str(query);
            self.revision = revision;
            self.medium = Some(medium);
            self.only_modified = only_modified;
            self.results = build_filtered(query, params, extras, only_modified);
            self.built = true;
        }
        &self.results
    }

    pub(crate) fn reset(&mut self) {
        self.query.clear();
        self.results.clear();
        self.medium = None;
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
