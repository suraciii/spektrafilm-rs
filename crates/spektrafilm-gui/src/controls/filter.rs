//! Control-search filter state shared by the parameter section renderers.
//!
//! [`ControlFilter`] threads the filter text through one section render and
//! records the labels it reached. [`SectionLabelCache`] stores those labels so
//! the panel can decide section visibility without rendering a section body.

/// Case-insensitive substring match of the control-search filter against a label.
pub(crate) fn label_matches(filter: &str, label: &str) -> bool {
    filter.is_empty() || label.to_lowercase().contains(&filter.to_lowercase())
}

/// Whether any of `labels` matches the control-search filter.
pub(crate) fn any_label_matches(filter: &str, labels: &[&str]) -> bool {
    labels.iter().any(|label| label_matches(filter, label))
}

/// The control labels a section reached in its last render, and whether that
/// render ran with a non-empty filter. A miss hides the section only after a
/// filtered render: an unfiltered render can skip the controls of a collapsed
/// sub-section, so its label cache can be incomplete.
#[derive(Default)]
pub(crate) struct SectionLabelCache {
    labels: Vec<&'static str>,
    search_rendered: bool,
}

impl SectionLabelCache {
    /// Whether `labels` came from a filtered render, so the control search may
    /// hide the section on a cache miss.
    fn complete_for_search(&self) -> bool {
        self.search_rendered && !self.labels.is_empty()
    }
}

/// Whether a section's cached control labels include a match for `filter`. A
/// section without a cache, or with an incomplete one, reports a match so it
/// renders until a filtered render fills the cache.
pub(crate) fn cached_labels_match(filter: &str, cache: Option<&SectionLabelCache>) -> bool {
    let Some(cache) = cache else {
        return true;
    };
    filter.is_empty() || !cache.complete_for_search() || any_label_matches(filter, &cache.labels)
}

/// Control-search state threaded through one section render: the filter text,
/// the labels reached by this render, and the label cache the panel keeps per
/// section. The cached labels decide which sections a search shows and hides,
/// so the decision does not depend on the section body being rendered.
pub(crate) struct ControlFilter<'a> {
    filter: &'a str,
    reached: Vec<&'static str>,
    cache: &'a mut SectionLabelCache,
    allowed_paths: Option<Vec<String>>,
}

impl<'a> ControlFilter<'a> {
    pub(crate) fn new(filter: &'a str, cache: &'a mut SectionLabelCache) -> Self {
        Self {
            filter,
            reached: Vec::new(),
            cache,
            allowed_paths: None,
        }
    }

    pub(crate) fn configure(
        &mut self,
        section: &str,
        params: &spektrafilm_core::params::RuntimeParams,
        extras: &serde_json::Value,
        only_modified: bool,
    ) {
        if self.filter.is_empty() && !only_modified {
            return;
        }
        let projection = only_modified.then(|| super::control_projection(params, extras));
        let query = self.filter.to_lowercase();
        self.allowed_paths = Some(
            super::inventory()
                .iter()
                .filter(|entry| {
                    entry.section == section
                        && entry.searchable()
                        && entry.matches(&query)
                        && projection.as_ref().is_none_or(|projection| {
                            entry.is_modified(&projection.runtime, &projection.extras)
                        })
                })
                .map(|entry| entry.path.clone())
                .collect(),
        );
    }

    pub(crate) fn install_row_filter(&self, ctx: &egui::Context) {
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("control_allowed_paths"),
                self.allowed_paths.clone().map(std::sync::Arc::new),
            )
        });
    }

    /// Query a group's actual canonical inventory entries without projecting
    /// through labels shared by another engine.
    pub(crate) fn any_matching_entry(
        &self,
        predicate: impl Fn(&super::inventory::ControlEntry) -> bool,
    ) -> bool {
        self.allowed_paths.as_ref().is_none_or(|paths| {
            super::inventory()
                .iter()
                .any(|entry| paths.contains(&entry.path) && predicate(entry))
        })
    }

    /// Store the labels this render reached as the section's label cache.
    pub(crate) fn finish(mut self) {
        std::mem::swap(&mut self.cache.labels, &mut self.reached);
        self.cache.search_rendered = self.active();
    }

    /// Whether the control search is active.
    pub(crate) fn active(&self) -> bool {
        !self.filter.is_empty() || self.allowed_paths.is_some()
    }

    /// Whether the filter matches a label the section rendered before.
    pub(crate) fn section_matches(&self) -> bool {
        self.allowed_paths.as_ref().map_or_else(
            || cached_labels_match(self.filter, Some(self.cache)),
            |paths| !paths.is_empty(),
        )
    }

    fn record(&mut self, label: &'static str) {
        if !self.reached.contains(&label) {
            self.reached.push(label);
        }
    }
}

/// Filter a control by its label, recording the label as reached.
pub(crate) fn filter_matches(filter: &mut ControlFilter<'_>, label: &'static str) -> bool {
    filter.record(label);
    filter.allowed_paths.as_ref().map_or_else(
        || label_matches(filter.filter, label),
        |paths| {
            super::inventory().iter().any(|entry| {
                paths.contains(&entry.path)
                    && (entry.label.eq_ignore_ascii_case(label)
                        || entry
                            .components
                            .iter()
                            .any(|component| component.label.eq_ignore_ascii_case(label)))
            })
        },
    )
}

/// Filter a group of labels that guard one control, recording every label.
pub(crate) fn filter_matches_any(filter: &mut ControlFilter<'_>, labels: &[&'static str]) -> bool {
    let mut matched = false;
    for &label in labels {
        matched |= filter_matches(filter, label);
    }
    matched
}
