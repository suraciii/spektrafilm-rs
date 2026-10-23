use super::*;
use egui::Ui;

/// One `show()` section group, split out of the dispatcher for readability.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    section: &str,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    if section == "Film chemistry" {
        render_chemistry(ui, &mut params.film_render.chemistry, flags, true, filter);
        if filter_matches(filter, "density curve gamma") {
            flags.runtime_changed |= param_numeric(
                ui,
                "density curve gamma",
                &mut params.film_render.density_curve_gamma,
                "film_render.density_curve_gamma",
                "Gamma applied to the film density curves during development; 1.0 keeps the profile curves unchanged. Requires an actual film-development route.",
            );
        }
    }
    if section == "Print chemistry" {
        render_chemistry(
            ui,
            &mut params.print_render.density_curves_morph,
            flags,
            false,
            filter,
        );
        if filter_matches(filter, "density curve gamma") {
            flags.runtime_changed |= param_numeric(
                ui,
                "density curve gamma",
                &mut params.print_render.density_curve_gamma,
                "print_render.density_curve_gamma",
                "Gamma applied to the paper density curves during print development; 1.0 keeps the profile curves unchanged. Requires an admitted print-development route.",
            );
        }
    }
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
    let (enable_key, enable_label) = if film {
        ("enable film density curves", "Enable film density curves")
    } else {
        ("enable print density curves", "Enable print density curves")
    };
    if filter_matches(filter, enable_key) {
        flags.runtime_changed |= toggle(
            ui,
            enable_label,
            &mut params.active,
            if film {
                "Enable film density-curve morphing."
            } else {
                "Enable print density-curve morphing."
            },
        );
    }
    if filter_matches(filter, "gamma factor") {
        flags.runtime_changed |= param_numeric(
            ui,
            "gamma factor",
            &mut params.gamma_factor,
            paths.gamma_factor,
            if film {
                "Global coupled gamma multiplier for the film density curves."
            } else {
                "Global coupled gamma multiplier for the print density curves."
            },
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
            if film {
                "Blend all three film sub-layers toward the matched Gumbel shoulder while preserving midgray via a common horizontal offset."
            } else {
                "Blend all three print sub-layers toward the matched Gumbel shoulder while preserving midgray via a common horizontal offset."
            },
        );
    }
}
