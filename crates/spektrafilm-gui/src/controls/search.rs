//! Cross-page control-search index.
//!
//! The control filter shows the current page only, so a parameter the user
//! remembers by an approximate name is invisible while another page owns it.
//! The index answers "which pages and sections hold a matching control"
//! without a second catalog of names to maintain.
//!
//! The index is built by rendering every section of every page into an
//! offscreen context, using the same `show` render path the panel uses. A
//! section is indexed for a filter when its render records a label that
//! matches, so the visibility rules a user sees — the grain engine that is
//! selected, the route- and medium-dependent scanner groups — are the rules
//! the index applies. A result therefore always points at a control the
//! owning page really shows, so navigating to it cannot dead-end.
//!
//! The index is rebuilt only when the filter text, the parameter revision or
//! the scan medium changes; it is transient state, never persisted.

use super::{
    ControlFilter, PAGE_LAYOUT, ScanMedium, SectionLabelCache, label_matches, render_sections,
};
use crate::theme;
use serde_json::Value;

/// One section of one page whose controls match the filter.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SearchResult {
    pub(crate) page: crate::ui::GuiTab,
    pub(crate) section: &'static str,
    /// The control labels this section rendered, in render order. The first
    /// matching one is the row's summary; the rest is what the section holds.
    pub(crate) labels: Vec<&'static str>,
}

impl SearchResult {
    /// One-line summary of the matching controls this result reveals.
    pub(crate) fn summary(&self) -> String {
        match self.labels.as_slice() {
            [] => String::new(),
            [label] => (*label).to_owned(),
            [first, .., last] => format!("{first}, … {last}"),
        }
    }
}

/// The sections the index may report, in the order the pages render them.
///
/// A section name is the identity the control renderers dispatch on, so the
/// index and the panel share one name per section. The import surface is not a
/// searchable destination: it owns an action, not a parameter group.
pub(crate) fn searchable_sections() -> impl Iterator<Item = (crate::ui::GuiTab, &'static str)> {
    PAGE_LAYOUT.iter().flat_map(|(page, items)| {
        items.iter().filter_map(move |item| {
            let section = item.section()?;
            (section != "Import Raw").then_some((*page, section))
        })
    })
}

/// Build the index for `filter` by rendering every searchable section into one
/// offscreen context with the live parameter state.
///
/// The render runs with collapsed sections forced open, so a section whose
/// controls sit behind a collapsed header is indexed exactly like the panel
/// reveals it during a search.
pub(crate) fn build(
    filter: &str,
    params: &spektrafilm_core::params::RuntimeParams,
    extras: &Value,
    film_medium: ScanMedium,
) -> Vec<SearchResult> {
    let mut results = Vec::new();
    if filter.is_empty() {
        return results;
    }
    let context = egui::Context::default();
    context.memory_mut(|memory| memory.set_everything_is_visible(true));
    let mut extras = extras.clone();
    let mut params = params.clone();
    // One frame renders every section with the same filter, so each section's
    // label cache is a complete filtered render. The frame is never shown, so
    // its output is discarded.
    let _ = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(theme::WIDTH_CONTROL_PANEL, 20000.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for (page, section) in searchable_sections() {
                    let mut cache = SectionLabelCache::default();
                    let mut state = ControlFilter::new(filter, &mut cache);
                    let mut flags = super::ChangeFlags::default();
                    ui.push_id(("search_index", section), |ui| {
                        render_sections(
                            ui,
                            &mut params,
                            &mut extras,
                            section,
                            film_medium,
                            &mut flags,
                            &mut state,
                        );
                    });
                    state.finish();
                    if let Some(labels) = cache.labels()
                        && let Some(_) = labels.iter().find(|label| label_matches(filter, label))
                    {
                        results.push(SearchResult {
                            page,
                            section,
                            labels: labels.to_vec(),
                        });
                    }
                }
            });
        },
    );
    results
}

/// The search index: what matched where, for the filter it was built with.
///
/// Rebuilt when the filter text, the parameter revision or the scan medium
/// changes, so the panel never reports a section whose controls the current
/// state does not render.
#[derive(Default)]
pub(crate) struct SearchIndex {
    filter: String,
    revision: u64,
    medium: Option<ScanMedium>,
    results: Vec<SearchResult>,
    built: bool,
}

impl SearchIndex {
    /// The index for `filter`, rebuilt only when its inputs changed.
    pub(crate) fn results(
        &mut self,
        filter: &str,
        params: &spektrafilm_core::params::RuntimeParams,
        extras: &Value,
        film_medium: ScanMedium,
        revision: u64,
    ) -> &[SearchResult] {
        if filter.is_empty() {
            self.reset();
            return &[];
        }
        if self.built
            && self.filter == filter
            && self.revision == revision
            && self.medium == Some(film_medium)
        {
            return &self.results;
        }
        self.filter = filter.to_owned();
        self.revision = revision;
        self.medium = Some(film_medium);
        self.results = build(filter, params, extras, film_medium);
        self.built = true;
        &self.results
    }

    /// Drop the cached results, so the next query rebuilds them.
    pub(crate) fn reset(&mut self) {
        self.filter.clear();
        self.results.clear();
        self.medium = None;
        self.built = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_core::params::RuntimeParams;

    fn sections_for(filter: &str) -> Vec<(&'static str, &'static str)> {
        let params = RuntimeParams::default();
        build(filter, &params, &serde_json::json!({}), ScanMedium::None)
            .into_iter()
            .map(|result| (result.page.label(), result.section))
            .collect()
    }

    /// A control the owning page renders is reported with that page and its
    /// section, so a result always points at a reachable control.
    #[test]
    fn a_label_is_reported_with_the_page_and_section_that_own_it() {
        let found = sections_for("halation amount");
        assert!(
            found.contains(&("FILM", "Halation")),
            "halation belongs to the film page, got {found:?}"
        );
        assert_eq!(found.len(), 1, "one section owns the label: {found:?}");
    }

    /// A shared label is reported once per owning section, so neither page's
    /// control is hidden behind the other's.
    #[test]
    fn a_label_two_sections_share_is_reported_for_each() {
        let found = sections_for("gamma factor");
        assert!(
            found.contains(&("FILM", "Film chemistry")),
            "film chemistry owns a gamma factor, got {found:?}"
        );
        assert!(
            found.contains(&("PRINT", "Print chemistry")),
            "print chemistry owns a gamma factor, got {found:?}"
        );
    }

    /// The index follows the same visibility rules the panel renders with, so
    /// a section the current state does not render is not reported.
    #[test]
    fn the_index_hides_a_section_the_current_state_does_not_render() {
        // The Scanner section's controls need a scanning route; the default
        // route is not one.
        assert!(
            !sections_for("negative conversion method")
                .iter()
                .any(|(_, section)| *section == "Scanner"),
            "an inactive route must not report scanner controls"
        );
        // A page-independent label is still reported.
        assert!(
            sections_for("halation amount")
                .iter()
                .any(|(page, _)| *page == "FILM"),
            "a route-independent control must still be reported"
        );
    }

    /// The index is a lookup over the live definitions, not a maintained
    /// catalog: every searchable section renders, so an empty filter reports
    /// nothing and a filter that matches nothing anywhere reports nothing.
    #[test]
    fn an_empty_or_unmatched_filter_reports_nothing() {
        assert!(
            sections_for("").is_empty(),
            "an empty filter has no results"
        );
        assert!(
            sections_for("zzz-not-a-control").is_empty(),
            "a filter that matches no label has no results"
        );
    }

    /// The index covers every parameter section the pages render, so a section
    /// added to a page is searchable without a second table to update.
    #[test]
    fn every_page_section_is_indexed() {
        let params = RuntimeParams::default();
        let index = build("a", &params, &serde_json::json!({}), ScanMedium::None);
        let pages: Vec<&str> = index.iter().map(|result| result.page.label()).collect();
        for page in ["MAIN", "FILM", "PRINT", "ADVANCED", "CONFIG"] {
            assert!(pages.contains(&page), "page {page} has no indexed section");
        }
    }
}
