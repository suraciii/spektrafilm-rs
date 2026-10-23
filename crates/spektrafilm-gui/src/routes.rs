//! Workflow route choices offered by the GUI selector.
//!
//! The route vocabulary belongs to the workflow validator; this table adds the
//! presentation (label and description) for the selector.
//! `choices_match_the_supported_route_vocabulary` keeps the two in step, so a
//! route added or removed by the validator cannot silently disappear from, or
//! survive in, the selector.

use std::sync::LazyLock;

/// The GUI-owned presentation vocabulary.
///
/// One map holds every visible word the GUI writes for the workflow selector
/// and the scanner surface, separated from the serialized values those words
/// name. The domain glossary (`CONTEXT.md`) owns the physical terms and the
/// film-chain specification owns the scanner operations; this module only
/// restates them for presentation, so a label change can never drift from the
/// operation it names or invent a synonym that crosses a physical boundary.
///
/// Route descriptions follow the direct physical descriptions in
/// specs/gui-product-design/spec.md#workflow-presentation: each describes the
/// process sequence only, never a creative category, and keeps the exact
/// serialized route value beside it.
pub(crate) mod vocabulary {
    /// CCTF spelled out at its first use, so the abbreviation does not hide
    /// whether the samples are encoded or linear.
    pub(crate) const CCTF: &str = "color component transfer function";

    /// How the scanner surface is sequenced: the physical light source, the
    /// output polarity, then the readout optics and digital sharpening.
    pub(crate) const SCANNER_SEQUENCE: &str = "Scanner sequence: scanner lamp, scan output \
         polarity, then readout optics and digital sharpening.";

    /// `conversion style`: the named positive-conversion tone of the
    /// negative-to-positive interpretation.
    pub(crate) const CONVERSION_STYLE: &str = "conversion style";

    /// `conversion style` help, stating that it is declared data, that the
    /// selector lists only what the user has installed, and that it applies
    /// only to the negative interpretation.
    pub(crate) const CONVERSION_STYLE_HELP: &str = "Named positive-conversion tone of the \
         negative-to-positive interpretation. `neutral` is the frozen reference tone; any other \
         name is a declared style you place in data/scan-styles/, never a fitted surface. The \
         selector lists the styles found there. A style applies only with scan output set to \
         Negative to positive.";

    /// `neutral`, the frozen reference tone every style is measured against.
    pub(crate) const CONVERSION_STYLE_NEUTRAL: &str = "neutral";

    /// `scanner lamp`: the one physical light source of a scan.
    pub(crate) const SCANNER_LAMP: &str = "scanner lamp";

    /// `scanner lamp` help, stating the boundary between the lamp and the
    /// film's own calibration.
    pub(crate) fn scanner_lamp_help() -> &'static str {
        static HELP: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
            format!(
                "Light source of the scanning rig that reads the scanned medium; the model is \
                 one colorimetric scan of the clear film base. It changes scanner acquisition \
                 only: it does not change film exposure, development, film-domain effects, \
                 print exposure, or camera exposure. {SCANNER_LAMP_SCOPE}"
            )
        });
        &HELP
    }

    /// Scope note shared by the lamp help.
    pub(crate) const SCANNER_LAMP_SCOPE: &str = "The catalog covers daylight, fluorescent, LED \
         and discharge series plus any BB<kelvin> blackbody.";

    /// `scan output`: native polarity or negative conversion.
    pub(crate) const SCAN_OUTPUT: &str = "scan output";

    pub(crate) const SCAN_OUTPUT_HELP: &str = "Preserve medium polarity keeps the scanned \
         medium's native scan polarity: a negative film scan reads the negative image on the film, \
         and a photographic-paper or slide scan already reads a positive image. Negative to \
         positive interprets a developed negative as a positive image by inverting the film's \
         own density records and applying the fixed positive tone response.";

    /// The direct physical description of one exact route value, describing
    /// the process sequence the image takes. The descriptions are the constants
    /// below, so the selector table and any other reader share one wording.
    pub(crate) fn route_description(value: &str) -> &'static str {
        match value {
            "input" => INPUT,
            "input > film > scan" => FILM_SCAN,
            "input > film > print > scan" => FILM_PRINT_SCAN,
            "input > convert-film > print > scan" => CONVERT_FILM_PRINT_SCAN,
            "input > convert-film > scan-minus-base" => CONVERT_FILM_SCAN_MINUS_BASE,
            "input > convert-film > scan" => CONVERT_FILM_SCAN,
            _ => "",
        }
    }

    /// `input`: an already rendered RGB image reaches the output space.
    pub(crate) const INPUT: &str = "Color-manage the input RGB image.";
    /// `input > film > scan`: the selected film medium is exposed and scanned.
    pub(crate) const FILM_SCAN: &str = "Expose film and scan the selected film medium.";
    /// `input > film > print > scan`: the paper print is scanned, not the film.
    pub(crate) const FILM_PRINT_SCAN: &str =
        "Expose film, print it photographically, then scan the paper.";
    /// `input > convert-film > print > scan`: a negative capture is printed and
    /// its paper is scanned.
    pub(crate) const CONVERT_FILM_PRINT_SCAN: &str =
        "Convert the input to a negative capture, print it photographically, then scan the paper.";
    /// `input > convert-film > scan-minus-base`: the film's base is removed.
    pub(crate) const CONVERT_FILM_SCAN_MINUS_BASE: &str =
        "Convert the input and scan the film with its base removed.";
    /// `input > convert-film > scan`: the film is scanned with its base.
    pub(crate) const CONVERT_FILM_SCAN: &str =
        "Convert the input to a negative capture, then scan the film.";
}

pub(crate) struct RouteChoice {
    pub value: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

pub(crate) const CHOICES: [RouteChoice; 6] = [
    RouteChoice {
        value: "input",
        label: "input",
        description: vocabulary::INPUT,
    },
    RouteChoice {
        value: "input > film > scan",
        label: "input > film > scan",
        description: vocabulary::FILM_SCAN,
    },
    RouteChoice {
        value: "input > film > print > scan",
        label: "input > film > print > scan",
        description: vocabulary::FILM_PRINT_SCAN,
    },
    RouteChoice {
        value: "input > convert-film > print > scan",
        label: "input > convert-film > print > scan",
        description: vocabulary::CONVERT_FILM_PRINT_SCAN,
    },
    RouteChoice {
        value: "input > convert-film > scan-minus-base",
        label: "input > convert-film > scan-minus-base",
        description: vocabulary::CONVERT_FILM_SCAN_MINUS_BASE,
    },
    RouteChoice {
        value: "input > convert-film > scan",
        label: "input > convert-film > scan",
        description: vocabulary::CONVERT_FILM_SCAN,
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
            .map(|choice| {
                let conditional = match choice.value {
                    "input > film > scan" => " The scanned medium is the selected film; a negative film may preserve its native negative polarity or use Negative to positive conversion, while a slide film retains its native positive polarity.",
                    "input > film > print > scan" | "input > convert-film > print > scan" => " The scanned medium is photographic paper and reads as a positive image.",
                    "input > convert-film > scan" => " The scanned medium is negative film; preserve medium polarity delivers the native negative, while Negative to positive performs negative conversion to a positive image.",
                    "input > convert-film > scan-minus-base" => " The scanned medium is negative film with its base removed; it remains in native negative polarity.",
                    _ => "",
                };
                format!("{} ({}){}", choice.label, choice.description, conditional)
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "Which physical path the image takes through the pipeline: {routes} CMYK ink style can be enabled independently for every workflow."
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
    fn every_route_description_names_its_process_sequence() {
        for choice in &CHOICES {
            assert_eq!(
                choice.description,
                vocabulary::route_description(choice.value),
                "{} must read its description from the vocabulary",
                choice.value
            );
            // A direct physical description states the sequence the image takes;
            // it must not invent a look category or hide a scan.
            assert!(
                !choice.description.is_empty(),
                "{} needs a direct physical description",
                choice.value
            );
        }
        // An unknown route keeps an empty description rather than borrowing a
        // neighbouring route's wording.
        assert_eq!(vocabulary::route_description("input > film"), "");
    }

    #[test]
    fn the_scanner_sequence_line_groups_the_controls_it_explains() {
        // The explanatory line is explanatory only: it names the groups without
        // renaming a runtime concept or adding a second control taxonomy.
        for group in [
            "scanner lamp",
            "scan output polarity",
            "readout optics",
            "digital sharpening",
        ] {
            assert!(
                vocabulary::SCANNER_SEQUENCE.contains(group),
                "scanner sequence must mention {group}"
            );
        }
    }

    #[test]
    fn scanner_help_keeps_the_physical_boundaries_distinct() {
        // The lamp is the acquisition light source: it stays separate from
        // the material stages the scan reads.
        assert_eq!(vocabulary::SCANNER_LAMP, "scanner lamp");
        assert!(
            vocabulary::scanner_lamp_help()
                .contains("does not change film exposure, development, film-domain effects"),
            "lamp help must state what it does not change"
        );
        // CCTF expands at first use.
        assert_eq!(vocabulary::CCTF, "color component transfer function");
    }
    #[test]
    fn route_tooltip_names_conditional_scanned_medium_and_polarity() {
        let tooltip = tooltip();
        assert!(tooltip.contains("scanned medium is photographic paper"));
        assert!(tooltip.contains("negative film may preserve"));
        assert!(tooltip.contains("slide film retains its native positive polarity"));
        assert!(tooltip.contains("remains in native negative polarity"));
    }
}
