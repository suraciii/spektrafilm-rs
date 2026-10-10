//! Workflow route choices offered by the GUI selector.
//!
//! The route vocabulary belongs to the workflow validator; this table adds the
//! presentation (label and description) for the selector.
//! `choices_match_the_supported_route_vocabulary` keeps the two in step, so a
//! route added or removed by the validator cannot silently disappear from, or
//! survive in, the selector.

use std::sync::LazyLock;

pub(crate) struct RouteChoice {
    pub value: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

pub(crate) const CHOICES: [RouteChoice; 6] = [
    RouteChoice {
        value: "input",
        label: "Finished RGB",
        description: "colour-manage an already rendered RGB image to the output space",
    },
    RouteChoice {
        value: "input > film > scan",
        label: "input > film > scan",
        description: "scan the negative directly",
    },
    RouteChoice {
        value: "input > film > print > scan",
        label: "input > film > print > scan",
        description: "full chain",
    },
    RouteChoice {
        value: "input > convert-film > print > scan",
        label: "input > convert-film > print > scan",
        description: "print a scene-referred input and scan it",
    },
    RouteChoice {
        value: "input > convert-film > scan-minus-base",
        label: "input > convert-film > scan-minus-base",
        description: "convert input and scan with base removed",
    },
    RouteChoice {
        value: "input > convert-film > scan",
        label: "input > convert-film > scan",
        description: "convert input, then scan the film with its base",
    },
];

/// Selector entries in the order the validator declares them.
pub(crate) fn entries() -> impl Iterator<Item = (&'static str, &'static str)> + Clone {
    CHOICES.iter().map(|choice| (choice.value, choice.label))
}

pub(crate) fn tooltip() -> &'static str {
    static TOOLTIP: LazyLock<String> = LazyLock::new(|| {
        let routes = CHOICES
            .iter()
            .map(|choice| format!("{} ({})", choice.label, choice.description))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Which path the image takes through the pipeline: {routes}. \
             Magazine print can be enabled independently for every workflow."
        )
    });
    &TOOLTIP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_match_the_supported_route_vocabulary() {
        let values: Vec<&str> = CHOICES.iter().map(|choice| choice.value).collect();
        assert_eq!(
            values,
            spektrafilm_core::params::sources::supported_routes()
        );
        for choice in &CHOICES {
            assert!(
                !choice.label.is_empty() && !choice.description.is_empty(),
                "{} needs a label and a description",
                choice.value
            );
        }
    }

    #[test]
    fn tooltip_describes_every_choice() {
        let tooltip = tooltip();
        for choice in &CHOICES {
            assert!(
                tooltip.contains(&format!("{} ({})", choice.label, choice.description)),
                "tooltip omits {}",
                choice.value
            );
        }
    }
}
