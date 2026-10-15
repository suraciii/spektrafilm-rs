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

/// Re-normalize scanner selections after a profile or route transition.
pub(crate) fn normalize_scan_output(params: &mut RuntimeParams, film_medium: ScanMedium) {
    normalize_scanner_selections(params, film_medium);
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
    let mut changed = false;
    egui::ComboBox::from_label(label)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for &(option, label) in choices {
                changed |= ui.selectable_value(value, option, label).changed();
            }
        })
        .response
        .on_hover_text(tooltip);
    changed
}

/// Per-path table keys for the shared density-curve morph editor. Film and
/// print chemistry are separate parameter paths with one range each.
struct ChemistryPaths {
    gamma_factor: &'static str,
    gamma_factor_fast: &'static str,
    gamma_factor_slow: &'static str,
    gamma_factor_red: &'static str,
    gamma_factor_green: &'static str,
    gamma_factor_blue: &'static str,
    developer_exhaustion: &'static str,
}

const FILM_CHEMISTRY_PATHS: ChemistryPaths = ChemistryPaths {
    gamma_factor: "film_render.chemistry.gamma_factor",
    gamma_factor_fast: "film_render.chemistry.gamma_factor_fast",
    gamma_factor_slow: "film_render.chemistry.gamma_factor_slow",
    gamma_factor_red: "film_render.chemistry.gamma_factor_red",
    gamma_factor_green: "film_render.chemistry.gamma_factor_green",
    gamma_factor_blue: "film_render.chemistry.gamma_factor_blue",
    developer_exhaustion: "film_render.chemistry.developer_exhaustion",
};

const PRINT_CHEMISTRY_PATHS: ChemistryPaths = ChemistryPaths {
    gamma_factor: "print_render.density_curves_morph.gamma_factor",
    gamma_factor_fast: "print_render.density_curves_morph.gamma_factor_fast",
    gamma_factor_slow: "print_render.density_curves_morph.gamma_factor_slow",
    gamma_factor_red: "print_render.density_curves_morph.gamma_factor_red",
    gamma_factor_green: "print_render.density_curves_morph.gamma_factor_green",
    gamma_factor_blue: "print_render.density_curves_morph.gamma_factor_blue",
    developer_exhaustion: "print_render.density_curves_morph.developer_exhaustion",
};

fn render_chemistry(
    ui: &mut Ui,
    params: &mut PrintCurvesMorphParams,
    flags: &mut ChangeFlags,
    film: bool,
    filter: &mut ControlFilter<'_>,
) {
    let paths = if film {
        &FILM_CHEMISTRY_PATHS
    } else {
        &PRINT_CHEMISTRY_PATHS
    };
    let what = if film { "film" } else { "print" };
    if filter_matches(filter, "active") {
        flags.runtime_changed |= toggle(
            ui,
            "active",
            &mut params.active,
            &format!("Enable {what} density-curve morphing."),
        );
    }
    if filter_matches(filter, "gamma factor") {
        flags.runtime_changed |= param_numeric(
            ui,
            "gamma factor",
            &mut params.gamma_factor,
            paths.gamma_factor,
            &format!("Global coupled gamma multiplier for the {what} density curves."),
        );
    }
    if filter_matches(filter, "gamma factor fast") {
        flags.runtime_changed |= param_numeric(
            ui,
            "gamma factor fast",
            &mut params.gamma_factor_fast,
            paths.gamma_factor_fast,
            "Gamma factor applied to the fast sub-layer.",
        );
    }
    if filter_matches(filter, "gamma factor slow") {
        flags.runtime_changed |= param_numeric(
            ui,
            "gamma factor slow",
            &mut params.gamma_factor_slow,
            paths.gamma_factor_slow,
            "Gamma factor applied to the mid and slow sub-layers.",
        );
    }
    if filter_matches(filter, "gamma factor red") {
        flags.runtime_changed |= param_numeric(
            ui,
            "gamma factor red",
            &mut params.gamma_factor_red,
            paths.gamma_factor_red,
            "Per-channel gamma factor applied to the red channel.",
        );
    }
    if filter_matches(filter, "gamma factor green") {
        flags.runtime_changed |= param_numeric(
            ui,
            "gamma factor green",
            &mut params.gamma_factor_green,
            paths.gamma_factor_green,
            "Per-channel gamma factor applied to the green channel.",
        );
    }
    if filter_matches(filter, "gamma factor blue") {
        flags.runtime_changed |= param_numeric(
            ui,
            "gamma factor blue",
            &mut params.gamma_factor_blue,
            paths.gamma_factor_blue,
            "Per-channel gamma factor applied to the blue channel.",
        );
    }
    if filter_matches(filter, "developer exhaustion") {
        flags.runtime_changed |= param_numeric(
            ui,
            "developer exhaustion",
            &mut params.developer_exhaustion,
            paths.developer_exhaustion,
            &format!(
                "Blend all three {what} sub-layers toward the matched Gumbel shoulder while preserving midgray via a common horizontal offset."
            ),
        );
    }
}

fn preflash_controls(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    let active = params.workflow.route.contains(" > print > ");
    if !active {
        ui.label("Preflash inactive on this route; stored values are retained.");
    }
    ui.add_enabled_ui(active, |ui| {
        if filter_matches(filter, "preflash exposure") {
            flags.runtime_changed |= param_numeric(ui, "preflash exposure", &mut params.enlarger.preflash_exposure, "enlarger.preflash_exposure", "Linear multiplier of the integrated spectral flash, not EV. Zero disables flash and retains its filters. For negative-to-photographic-paper printing, flash can bring weak highlight exposure differences into a responsive part of the paper curve; it cannot recover absent detail. Excess may lower paper white, add fog, alter midtones and change color. Other print materials retain their own polarity.");
        }
        if filter_matches(filter, "preflash y filter shift") {
            flags.runtime_changed |= param_numeric(ui, "preflash y filter shift", &mut params.enlarger.preflash_y_filter_shift, "enlarger.preflash_y_filter_shift", "Independent preflash Yellow shift around the calibrated neutral filter pack, in Kodak CC units.");
        }
        if filter_matches(filter, "preflash m filter shift") {
            flags.runtime_changed |= param_numeric(ui, "preflash m filter shift", &mut params.enlarger.preflash_m_filter_shift, "enlarger.preflash_m_filter_shift", "Independent preflash Magenta shift around the calibrated neutral filter pack, in Kodak CC units.");
        }
    });
}

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

/// Whether a section's cached control labels include a match for `filter`. An
/// incomplete cache reports a match so the section renders until the cache
/// fills.
pub(crate) fn cached_labels_match(filter: &str, cache: &SectionLabelCache) -> bool {
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
}

impl<'a> ControlFilter<'a> {
    pub(crate) fn new(filter: &'a str, cache: &'a mut SectionLabelCache) -> Self {
        Self {
            filter,
            reached: Vec::new(),
            cache,
        }
    }

    /// Store the labels this render reached as the section's label cache.
    pub(crate) fn finish(&mut self) {
        std::mem::swap(&mut self.cache.labels, &mut self.reached);
        self.reached.clear();
        self.cache.search_rendered = self.active();
    }

    /// Whether the control search is active.
    pub(crate) fn active(&self) -> bool {
        !self.filter.is_empty()
    }

    /// Whether the filter matches a label the section rendered before.
    pub(crate) fn section_matches(&self) -> bool {
        cached_labels_match(self.filter, self.cache)
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
    label_matches(filter.filter, label)
}

/// Filter a group of labels that guard one control, recording every label.
pub(crate) fn filter_matches_any(filter: &mut ControlFilter<'_>, labels: &[&'static str]) -> bool {
    let mut matched = false;
    for &label in labels {
        matched |= filter_matches(filter, label);
    }
    matched
}

/// A section or sub-section header. During a control search the body is
/// revealed instead of collapsed, and a section whose controls do not match is
/// dropped entirely.
pub(crate) fn section_header<R>(
    ui: &mut Ui,
    filter: &mut ControlFilter<'_>,
    title: &str,
    default_open: bool,
    add_contents: impl FnOnce(&mut Ui, &mut ControlFilter<'_>) -> R,
) {
    if filter.active() {
        if !filter.section_matches() {
            return;
        }
        ui.label(egui::RichText::new(title).strong());
        add_contents(ui, filter);
        return;
    }
    egui::CollapsingHeader::new(title)
        .default_open(default_open)
        .show(ui, |ui| add_contents(ui, filter));
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

fn render_sections(
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
    if section == "Magazine finish color" {
        section_header(ui, filter, section, false, |ui, filter| {
            let magazine = &mut params.magazine_print_color;
            if filter_matches(filter, "strength") {
                flags.runtime_changed |= param_numeric(
                    ui,
                    "strength",
                    &mut magazine.strength,
                    "magazine_print_color.strength",
                    "Magazine finish appearance strength.",
                );
            }
        });
    }
    if section == "Halation" {
        ui.scope(|ui| {
            ui.label(
                egui::RichText::new("Key controls: halation amount (strength) · halation spatial scale (size) · scatter amount (highlight glow)")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            if filter_matches(filter, "active") {
                flags.runtime_changed |= toggle(ui, "active", &mut params.film_render.halation.active, "Enable halation and in-emulsion scatter.");
            }
            if filter_matches(filter, "scatter amount") {
                flags.runtime_changed |= param_numeric(ui, "scatter amount", &mut params.film_render.halation.scatter_amount, "film_render.halation.scatter_amount", "High-level scatter strength. 1.0 = full physical scatter, 0.0 = no scatter. Scales the fraction of light that undergoes in-emulsion scattering.");
            }
            if filter_matches(filter, "scatter spatial scale") {
                flags.runtime_changed |= param_numeric(ui, "scatter spatial scale", &mut params.film_render.halation.scatter_spatial_scale, "film_render.halation.scatter_spatial_scale", "High-level scatter size multiplier (1.0 = physical defaults). Scales both core and tail sigmas.");
            }
            if filter_matches(filter, "halation amount") {
                flags.runtime_changed |= param_numeric(ui, "halation amount", &mut params.film_render.halation.halation_amount, "film_render.halation.halation_amount", "High-level halation strength multiplier (1.0 = physical defaults). Scales the per-channel halation amplitudes.");
            }
            if filter_matches(filter, "halation spatial scale") {
                flags.runtime_changed |= param_numeric(ui, "halation spatial scale", &mut params.film_render.halation.halation_spatial_scale, "film_render.halation.halation_spatial_scale", "High-level halation size multiplier (1.0 = physical defaults). Scales the first-bounce sigma.");
            }
            if filter_matches(filter, "boost ev") {
                flags.runtime_changed |= param_numeric(ui, "boost ev", &mut params.film_render.halation.boost_ev, "film_render.halation.boost_ev", "Maximum highlight boost in stops.");
            }
            if filter_matches(filter, "protect ev") {
                flags.runtime_changed |= param_numeric(ui, "protect ev", &mut params.film_render.halation.protect_ev, "film_render.halation.protect_ev", "Protected range above midgray for the boost onset in stops.");
            }
            if filter_matches(filter, "boost range") {
                flags.runtime_changed |= param_numeric(ui, "boost range", &mut params.film_render.halation.boost_range, "film_render.halation.boost_range", "Controls how quickly the highlight boost ramps in, from 0 to 1.");
            }
            if filter_matches(filter, "scatter core um") {
                flags.runtime_changed |= param_tuple(ui, "scatter core um", &mut params.film_render.halation.scatter_core_um, "film_render.halation.scatter_core_um", "Sigma of the scatter core Gaussian per channel [R,G,B], in micrometers. Controls fine-scale sharpness loss in the emulsion.");
            }
            if filter_matches(filter, "scatter tail um") {
                flags.runtime_changed |= param_tuple(ui, "scatter tail um", &mut params.film_render.halation.scatter_tail_um, "film_render.halation.scatter_tail_um", "Decay constant of the scatter exponential tail per channel [R,G,B], in micrometers (internally approximated by a sum of Gaussians). Controls extended low-level spread within the emulsion.");
            }
            if filter_matches(filter, "scatter tail weight") {
                flags.runtime_changed |= param_tuple(ui, "scatter tail weight", &mut params.film_render.halation.scatter_tail_weight, "film_render.halation.scatter_tail_weight", "Weight of the scatter tail Gaussian per channel [R,G,B] (0-1). Tail weight + core weight = 1. Higher values put more scattered light into the long tail.");
            }
            if filter_matches(filter, "halation strength") {
                flags.runtime_changed |= param_tuple(ui, "halation strength", &mut params.film_render.halation.halation_strength, "film_render.halation.halation_strength", "Total back-reflection halation amplitude per channel [R,G,B] (0-1). Typical red channel: weak AH 0.02-0.08, no AH 0.08-0.25. The blue channel is usually near zero.");
            }
            if filter_matches(filter, "halation first sigma um") {
                flags.runtime_changed |= param_tuple(ui, "halation first sigma um", &mut params.film_render.halation.halation_first_sigma_um, "film_render.halation.halation_first_sigma_um", "Sigma of the first halation bounce per channel [R,G,B], in micrometers. Set by the base thickness (40-80 um for typical cine/still bases).");
            }
            if filter_matches(filter, "halation n bounces") {
                flags.runtime_changed |= param_numeric(ui, "halation n bounces", &mut params.film_render.halation.halation_n_bounces, "film_render.halation.halation_n_bounces", "Number of multi-bounce Gaussians summed in the halation pass. Subsequent bounces use sqrt(k)-spaced widths. Typical: 2-3.");
            }
            if filter_matches(filter, "halation bounce decay") {
                flags.runtime_changed |= param_numeric(ui, "halation bounce decay", &mut params.film_render.halation.halation_bounce_decay, "film_render.halation.halation_bounce_decay", "Per-bounce amplitude decay ratio (rho). Physical range 0.3-0.7. Controls how fast the halation energy falls off between bounces.");
            }
            if filter_matches(filter, "halation renormalize") {
                flags.runtime_changed |= toggle(ui, "halation renormalize", &mut params.film_render.halation.halation_renormalize, "If enabled, divide by (1 + sum of bounce amplitudes) so mid-grey is preserved. If disabled, halation is purely additive and subtly lifts shadows as well as highlights.");
            }
        });
    }
    if section == "Couplers" {
        ui.scope(|ui| {
            ui.label(
                egui::RichText::new("Key controls: inhibition samelayer (contrast) · inhibition interlayer (saturation) · amount (global)")
                    .small()
                    .color(egui::Color32::GRAY),
            );
            if filter_matches(filter, "active") {
                flags.runtime_changed |= toggle(ui, "active", &mut params.film_render.dir_couplers.active, "Enable DIR coupler inhibition.");
            }
            if filter_matches(filter, "amount") {
                flags.runtime_changed |= param_numeric(ui, "amount", &mut params.film_render.dir_couplers.amount, "film_render.dir_couplers.amount", "Global multiplier on the DIR coupler inhibition matrix. 1.0 leaves the per-channel gammas as-is.");
            }
            if filter_matches(filter, "inhibition samelayer") {
                flags.runtime_changed |= param_numeric(ui, "inhibition samelayer", &mut params.film_render.dir_couplers.inhibition_samelayer, "film_render.dir_couplers.inhibition_samelayer", "Multiplier on the same-layer (diagonal) inhibition. Controls overall contrast / gamma reduction within each RGB layer.");
            }
            if filter_matches(filter, "inhibition interlayer") {
                flags.runtime_changed |= param_numeric(ui, "inhibition interlayer", &mut params.film_render.dir_couplers.inhibition_interlayer, "film_render.dir_couplers.inhibition_interlayer", "Multiplier on the cross-layer (off-diagonal) inhibition. Controls saturation enhancement from interlayer DIR effects.");
            }
            if filter_matches(filter, "gamma samelayer rgb") {
                flags.runtime_changed |= param_tuple(ui, "gamma samelayer rgb", &mut params.film_render.dir_couplers.gamma_samelayer_rgb, "film_render.dir_couplers.gamma_samelayer_rgb", "Per-channel same-layer DIR gamma (R, G, B). Effective gamma reduction of each layer's density curve.");
            }
            if filter_matches(filter, "gamma interlayer r to gb") {
                flags.runtime_changed |= param_tuple(ui, "gamma interlayer r to gb", &mut params.film_render.dir_couplers.gamma_interlayer_r_to_gb, "film_render.dir_couplers.gamma_interlayer_r_to_gb", "DIR inhibition from the R layer onto the G and B layers respectively (g_R->G, g_R->B).");
            }
            if filter_matches(filter, "gamma interlayer g to rb") {
                flags.runtime_changed |= param_tuple(ui, "gamma interlayer g to rb", &mut params.film_render.dir_couplers.gamma_interlayer_g_to_rb, "film_render.dir_couplers.gamma_interlayer_g_to_rb", "DIR inhibition from the G layer onto the R and B layers respectively (g_G->R, g_G->B).");
            }
            if filter_matches(filter, "gamma interlayer b to rg") {
                flags.runtime_changed |= param_tuple(ui, "gamma interlayer b to rg", &mut params.film_render.dir_couplers.gamma_interlayer_b_to_rg, "film_render.dir_couplers.gamma_interlayer_b_to_rg", "DIR inhibition from the B layer onto the R and G layers respectively (g_B->R, g_B->G).");
            }
            if filter_matches(filter, "langmuir donor k rgb") {
                flags.runtime_changed |= param_tuple(ui, "langmuir donor k rgb", &mut params.film_render.dir_couplers.langmuir_donor_k_rgb, "film_render.dir_couplers.langmuir_donor_k_rgb", "NEGATIVE film: per-channel Langmuir saturation of DIR inhibitor release, normalized to each layer's d_max (R, G, B). Lower = earlier, stronger roll-off at high density (gentler shoulder, tames high-amount breakage); large values approach the linear model.");
            }
            if filter_matches(filter, "langmuir receiver k rgb") {
                flags.runtime_changed |= param_tuple(ui, "langmuir receiver k rgb", &mut params.film_render.dir_couplers.langmuir_receiver_k_rgb, "film_render.dir_couplers.langmuir_receiver_k_rgb", "POSITIVE/reversal film: per-channel receiver-side Langmuir saturation (R, G, B), normalized to the arrived-inhibitor ceiling (the receiver analogue of langmuir_donor_k_rgb's d_max normalization). The donor (silver release) stays linear; this rolls off the receiving layer's response under a pushed amount. Lower = harder, more crash-proof knee (alters the operating-point look); large values approach linear.");
            }
            if filter_matches(filter, "diffusion size um") {
                flags.runtime_changed |= param_numeric(ui, "diffusion size um", &mut params.film_render.dir_couplers.diffusion_size_um, "film_render.dir_couplers.diffusion_size_um", "Sigma in um for the diffusion of the couplers, (5-20 um), controls sharpness and affects saturation.");
            }
            if filter_matches(filter, "diffusion tail um") {
                flags.runtime_changed |= param_numeric(ui, "diffusion tail um", &mut params.film_render.dir_couplers.diffusion_tail_um, "film_render.dir_couplers.diffusion_tail_um", "Exponential tail scale in um for long-range coupler spread. Larger values extend the rare long-distance inhibition halo beyond the Gaussian core.");
            }
            if filter_matches(filter, "diffusion tail weight") {
                flags.runtime_changed |= param_numeric(ui, "diffusion tail weight", &mut params.film_render.dir_couplers.diffusion_tail_weight, "film_render.dir_couplers.diffusion_tail_weight", "Fraction of coupler diffusion energy assigned to the long tail. 0 = pure Gaussian spread, higher values add a longer-range inhibition shoulder.");
            }
        });
    }
    if section == "Camera diffusion" || section == "Enlarger diffusion" {
        ui.scope(|ui| {
            let (diffusion_filter, paths) = if section == "Camera diffusion" {
                (&mut params.camera.diffusion_filter, &CAMERA_DIFFUSION_PATHS)
            } else {
                (
                    &mut params.enlarger.diffusion_filter,
                    &ENLARGER_DIFFUSION_PATHS,
                )
            };
            flags.runtime_changed |= diffusion(ui, diffusion_filter, paths, filter);
        });
    }
    if section == "Output" {
        if filter_matches(filter, "simulation blend") {
            flags.runtime_changed |= param_numeric(
                ui,
                "Simulation blend",
                &mut params.output_mix.amount,
                "output_mix.amount",
                "Blend the complete result with the decoded, geometrically aligned prepared source in linear output RGB. 1 returns the complete result; 0 returns the prepared source before virtual camera exposure. For a captured negative, zero returns the prepared negative, not a positive interpretation. Input/Output viewing remains independent.",
            );
        }
        ui.label(
            egui::RichText::new("1.0 = full simulation · 0.0 = original")
                .small()
                .color(egui::Color32::GRAY),
        );
        if filter_matches(filter, "output color space") {
            flags.runtime_changed |= choice_tip(
                ui,
                "output color space",
                &mut params.io.output_color_space,
                COLOR_SPACES,
                "Output color space of the simulation",
            );
        }
        if filter_matches(filter, "saving color space") {
            extra_choice(
                ui,
                extras,
                "simulation",
                "saving_color_space",
                "saving color space",
                "sRGB",
                COLOR_SPACES,
                "Color space of the saved image file",
            );
        }
        if filter_matches(filter, "saving cctf encoding") {
            extra_bool_tip(
                ui,
                extras,
                "simulation",
                "saving_cctf_encoding",
                "saving cctf encoding",
                true,
                "Add or not the CCTF to the saved image file",
            );
        }
    }
    if section == "Display" {
        if filter_matches(filter, "preview max size")
            && param_numeric(
                ui,
                "preview max size",
                &mut params.settings.preview_max_size,
                "settings.preview_max_size",
                "max size of the long edge of the preview image in pixels",
            )
        {
            set_extra(
                extras,
                "display",
                "preview_max_size",
                Value::from(params.settings.preview_max_size),
            );
        }
        if filter_matches(filter, "update") {
            flags.preview_requested |= ui.button("update").clicked();
        }
    }
}

mod base;
mod chemistry;
mod convert;
mod crop;
mod gamut;
mod glare;
mod grain;
/// Editor range for `path`, from the single per-path table. A missing entry
/// is a defect, not a silent default to the machine range.
mod scanner;
mod upsampling;
mod widgets;
pub(crate) use widgets::*;

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
                        &mut Default::default(),
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
        let fotufilm = text_position(&popup, "Fotufilm Film");
        frame(click(fotufilm, true));
        let (flags, _) = frame(click(fotufilm, false));
        assert!(flags.runtime_changed);
        let (_, selected) = frame(vec![]);
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
                        changed = scan_points(ui, &mut black, &mut white);
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
                        changed = scan_points(ui, &mut black, &mut white);
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
                        scan_points(ui, &mut black, &mut white);
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
