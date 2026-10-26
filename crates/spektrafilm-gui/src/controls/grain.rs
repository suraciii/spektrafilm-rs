use super::*;
use egui::Ui;
use spektrafilm_core::params::grain::GrainParams;

fn engine_label(engine: GrainEngine) -> &'static str {
    match engine {
        GrainEngine::Spektra => "Spektra",
        GrainEngine::Procedural => "Procedural",
        GrainEngine::Dye => "Dye",
        GrainEngine::Fotufilm => "Fotufilm Film",
    }
}

/// Applicability is core-owned, including loaded material capabilities. Search
/// reveals stored requests without changing their engine, profile or values.
fn control(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    filter: &mut ControlFilter<'_>,
    label: &'static str,
    path: &str,
    edit: impl FnOnce(&mut Ui, &mut RuntimeParams) -> bool,
) -> bool {
    if !filter_matches(filter, label) || !row_path_matches(ui.ctx(), path) {
        return false;
    }
    let reason = crate::ui::grain_control_inactive_reason(ui.ctx(), path, params, None);
    if reason.is_some() && !filter.active() {
        return false;
    }
    let _identity = row_identity(ui.ctx(), path);
    let changed = ui
        .add_enabled_ui(reason.is_none(), |ui| edit(ui, params))
        .inner;
    if let Some(reason) = reason {
        ui.label(
            egui::RichText::new(localized(ui.ctx(), reason))
                .small()
                .color(theme::FG_MUTED),
        );
    }
    changed
}

#[allow(clippy::too_many_arguments)]
fn grain_tuple<T: egui::emath::Numeric, const N: usize>(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    filter: &mut ControlFilter<'_>,
    label: &'static str,
    path: &str,
    tooltip: &str,
    values: fn(&mut GrainParams) -> &mut [T; N],
) -> bool {
    let discovery = filter.active();
    control(ui, params, filter, label, path, |ui, params| {
        let reasons = std::array::from_fn(|index| {
            crate::ui::grain_control_inactive_reason(ui.ctx(), path, params, Some(index))
        });
        widgets::param_tuple_applicable(
            ui,
            label,
            values(&mut params.film_render.grain),
            path,
            tooltip,
            &reasons,
            discovery,
            path == "film_render.grain.rms_granularity",
        )
    })
}

fn advanced(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    filter: &mut ControlFilter<'_>,
    title: &'static str,
    contents: impl FnOnce(&mut Ui, &mut RuntimeParams, &mut ControlFilter<'_>) -> bool,
) -> bool {
    if filter.active() {
        return contents(ui, params, filter);
    }
    let mut changed = false;
    section_header(ui, filter, title, |ui, filter| {
        changed = contents(ui, params, filter);
    });
    changed
}

macro_rules! number {
    ($ui:expr, $params:expr, $filter:expr, $($field:ident).+, $label:literal, $tip:literal) => {
        control($ui, $params, $filter, $label,
            concat!("film_render.grain", $(".", stringify!($field)),+), |ui, params| {
                param_numeric(ui, $label, &mut params.film_render.grain.$($field).+,
                    concat!("film_render.grain", $(".", stringify!($field)),+), $tip)
            })
    };
}

macro_rules! vector {
    ($ui:expr, $params:expr, $filter:expr, $($field:ident).+, $label:literal, $tip:literal) => {
        grain_tuple($ui, $params, $filter, $label,
            concat!("film_render.grain", $(".", stringify!($field)),+), $tip,
            |grain| &mut grain.$($field).+)
    };
}

macro_rules! optional {
    ($ui:expr, $params:expr, $filter:expr, $field:ident, $label:literal, $resolved:expr, $tip:literal) => {
        control(
            $ui,
            $params,
            $filter,
            $label,
            concat!("film_render.grain.", stringify!($field)),
            |ui, params| {
                optional_numeric(
                    ui,
                    $label,
                    &mut params.film_render.grain.$field,
                    $resolved,
                    concat!("film_render.grain.", stringify!($field)),
                    $tip,
                )
            },
        )
    };
}

fn feature_profile_selector(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    filter: &mut ControlFilter<'_>,
    flags: &mut ChangeFlags,
    family: FeatureProfileFamily,
    label: &'static str,
) {
    if !filter_matches(filter, label) {
        return;
    }
    // Amount overrides do not change the canonical named/Custom selection.
    let current = params.film_render.grain.v2_profile.as_str();
    let mut selected = current.to_owned();
    let choices = feature_profiles::names(family)
        .iter()
        .copied()
        .map(|name| (name, name))
        .chain(std::iter::once(("custom", "Custom")));
    if choice_tip_entries(
        ui,
        label,
        &mut selected,
        choices,
        "Built-in feature profile; selecting it changes only the declared profile scope.",
    ) {
        let mut candidate = params.clone();
        match apply_feature_profile(&mut candidate, family, &selected) {
            Ok(()) => {
                *params = candidate;
                flags.runtime_changed = true;
            }
            Err(error) => {
                ui.label(
                    egui::RichText::new(crate::tr!(
                        ui.ctx(),
                        "controls.grain.profile_apply_error",
                        error = error
                    ))
                    .color(theme::FG_MUTED),
                );
            }
        }
    }
}

/// Ordinary controls expose the selected engine; discovery uses the same
/// owners to reveal retained requests without changing engine or profile.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    section: &str,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    if section != "Grain" {
        return;
    }
    if filter_matches(filter, "Enable grain") {
        flags.runtime_changed |= toggle(
            ui,
            "Enable grain",
            &mut params.film_render.grain.active,
            "Add grain to the negative",
        );
    }
    if filter_matches(filter, "Engine") {
        flags.runtime_changed |= enum_choice(
            ui,
            "Engine",
            &mut params.film_render.grain.engine,
            &[
                (GrainEngine::Dye, engine_label(GrainEngine::Dye)),
                (GrainEngine::Spektra, engine_label(GrainEngine::Spektra)),
                (
                    GrainEngine::Procedural,
                    engine_label(GrainEngine::Procedural),
                ),
                (GrainEngine::Fotufilm, engine_label(GrainEngine::Fotufilm)),
            ],
            "Select the grain engine: Spektra for material grain in developed film density, Procedural for post-scan grain appearance in encoded RGB, Dye for material grain formed in a film-coordinate dye field, Fotufilm Film for effective-appearance film population grain model.",
        );
    }
    if !params.film_render.grain.active && !filter.active() {
        notice(ui, "controls.grain.off");
        return;
    }
    if filter.active() {
        for engine in [
            GrainEngine::Dye,
            GrainEngine::Spektra,
            GrainEngine::Procedural,
            GrainEngine::Fotufilm,
        ] {
            ui.push_id(("grain", engine_label(engine)), |ui| {
                flags.runtime_changed |= engine_controls(ui, params, filter, engine);
            });
        }
        flags.runtime_changed |= shared_controls(ui, params, filter);
    } else {
        let engine = params.film_render.grain.engine;
        notice(
            ui,
            match engine {
                GrainEngine::Dye => "controls.grain.dye_description",
                GrainEngine::Spektra => "controls.grain.spektra_description",
                GrainEngine::Procedural => "controls.grain.procedural_description",
                GrainEngine::Fotufilm => "controls.grain.fotufilm_description",
            },
        );
        ui.push_id(("grain", engine_label(engine)), |ui| {
            flags.runtime_changed |= engine_controls(ui, params, filter, engine);
        });
        if engine == GrainEngine::Dye {
            notice(ui, "controls.grain.dye_preview_notice");
            if params.film_render.grain.monochrome {
                notice(ui, "controls.grain.single_record_notice");
            }
        } else if engine == GrainEngine::Procedural {
            if params.film_render.grain.v2_profile != "custom" {
                notice(ui, "controls.grain.named_profile_custom_controls");
            } else if params.film_render.grain.v2_mode == GrainV2Mode::Noise {
                notice(ui, "controls.grain.noise_notice");
            }
        }
    }
}

fn notice(ui: &mut Ui, key: &str) {
    ui.label(
        egui::RichText::new(crate::i18n::text(ui.ctx(), key))
            .small()
            .color(theme::FG_MUTED),
    );
}

fn shared_controls(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    filter: &mut ControlFilter<'_>,
) -> bool {
    let mut changed = false;
    changed |= vector!(
        ui,
        params,
        filter,
        rms_granularity,
        "rms granularity",
        "Per-channel relative Dye strength: event-area scale k = (G / 6)^2, not a certified optical RMS target. Zero retains the reference-scale fallback; upstream Spektra retains this value as a no-op. Distinct from Procedural Amount."
    );
    changed |= number!(
        ui,
        params,
        filter,
        channel_correlation,
        "Channel correlation",
        "Shared Poisson-event rate fraction among active records within each numerical component, not realized CMY or final RGB correlation. Preserves each record's marginal count distribution; correlation depends on record-rate ratios. Inactive for single-record black-and-white material; the requested value is retained for color film."
    );
    changed |= vector!(
        ui,
        params,
        filter,
        density_min,
        "density min",
        "Minimum grain density shared by Spektra and Dye."
    );
    changed |= vector!(
        ui,
        params,
        filter,
        uniformity,
        "uniformity",
        "Per-channel grain uniformity shared by Spektra and Dye."
    );
    changed |= vector!(
        ui,
        params,
        filter,
        particle_scale_sublayers,
        "particle scale sublayers",
        "Positive scale per component: Spektra uses relative particle scales per emulsion sublayer; Dye uses numerical curve components, changing event mass and rate, not separate cloud-kernel widths."
    );
    changed
}

fn engine_controls(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    filter: &mut ControlFilter<'_>,
    engine: GrainEngine,
) -> bool {
    let mut changed = false;
    match engine {
        GrainEngine::Dye => {
            changed |= number!(
                ui,
                params,
                filter,
                v3_event_area_um2,
                "Event area (strength)",
                "Base statistical event area in square film micrometers; controls grain strength independently of dye support."
            );
            changed |= number!(
                ui,
                params,
                filter,
                v3_dye_support_um,
                "Dye support (spatial formation)",
                "Dye support conditions the grain field and image detail in film micrometers; it is not a pure grain diameter control."
            );
            changed |= advanced(ui, params, filter, "Dye Advanced", |ui, params, filter| {
                let mut changed = false;
                if !filter.active() {
                    changed |= shared_controls(ui, params, filter);
                }
                changed
            });
        }
        GrainEngine::Spektra => {
            changed |= control(
                ui,
                params,
                filter,
                "sublayers active",
                "film_render.grain.sublayers_active",
                |ui, params| {
                    toggle(
                        ui,
                        "sublayers active",
                        &mut params.film_render.grain.sublayers_active,
                        "Split the developed density into the film profile's emulsion sublayers instead of applying grain to the composite density. Spektra-only; Dye always uses numerical curve components, and Procedural and Fotufilm ignore it.",
                    )
                },
            );
            if !filter.active()
                && params.film_render.grain.sublayers_active
                && let Some(reason) = crate::ui::grain_control_inactive_reason(
                    ui.ctx(),
                    "film_render.grain.particle_scale_sublayers",
                    params,
                    None,
                )
            {
                notice(ui, "controls.grain.spektra_sublayers_unavailable");
                ui.label(
                    egui::RichText::new(localized(ui.ctx(), reason))
                        .small()
                        .color(theme::FG_MUTED),
                );
            }
            changed |= number!(
                ui,
                params,
                filter,
                particle_area_um2,
                "particle area (µm²)",
                "Base statistical particle area in square film micrometers; smaller values give finer grain. Spektra reads this value; Dye uses its own event area."
            );
            changed |= vector!(
                ui,
                params,
                filter,
                particle_scale,
                "particle scale",
                "Per-channel multiplier on the particle area; the default makes the blue record coarser. Spektra reads this value; Dye uses the granularity preset instead."
            );
            changed |= number!(ui, params, filter, blur, "blur", "Post-blur sigma.");
            changed |= number!(
                ui,
                params,
                filter,
                n_sub_layers,
                "sub-layers",
                "Number of emulsion sub-layers the composite density is split into when sublayers active is off; each sub-layer draws its own grain realization. Spektra-only."
            );
            changed |= advanced(
                ui,
                params,
                filter,
                "Spektra Advanced",
                |ui, params, filter| {
                    let mut changed = false;
                    if !filter.active() {
                        changed |= shared_controls(ui, params, filter);
                    }
                    changed |= number!(
                        ui,
                        params,
                        filter,
                        blur_dye_clouds_um,
                        "dye-cloud blur",
                        "Dye-cloud blur in micrometers."
                    );
                    changed |= number!(
                        ui,
                        params,
                        filter,
                        mult_usm_amount,
                        "mult usm amount",
                        "Density unsharp-mask amount."
                    );
                    changed |= number!(
                        ui,
                        params,
                        filter,
                        mult_usm_sigma,
                        "mult usm sigma",
                        "Density unsharp-mask radius."
                    );
                    changed |= vector!(
                        ui,
                        params,
                        filter,
                        micro_structure,
                        "micro structure",
                        "Micro-structure scale and clump size."
                    );
                    changed
                },
            );
        }
        GrainEngine::Procedural => {
            changed |= control(
                ui,
                params,
                filter,
                "Procedural Grain Profiles",
                "film_render.grain.v2_profile",
                |ui, params| {
                    let mut cache = SectionLabelCache::default();
                    let mut profile_filter = ControlFilter::new("", &mut cache);
                    let mut flags = ChangeFlags::default();
                    feature_profile_selector(
                        ui,
                        params,
                        &mut profile_filter,
                        &mut flags,
                        FeatureProfileFamily::GrainProcedural,
                        "Procedural Grain Profiles",
                    );
                    flags.runtime_changed
                },
            );
            let resolved = params.film_render.grain.resolved_grain_v2();
            changed |= optional!(
                ui,
                params,
                filter,
                v2_amount,
                "Amount",
                resolved.amount * 100.0,
                "Overall grain amount."
            );
            changed |= control(
                ui,
                params,
                filter,
                "Processing Mode",
                "film_render.grain.v2_mode",
                |ui, params| {
                    enum_choice(
                        ui,
                        "Processing Mode",
                        &mut params.film_render.grain.v2_mode,
                        &[
                            (GrainV2Mode::Analogue, "Analogue"),
                            (GrainV2Mode::Noise, "Noise"),
                        ],
                        "Procedural grain processing mode.",
                    )
                },
            );
            changed |= optional!(
                ui,
                params,
                filter,
                v2_size,
                "Size",
                resolved.size,
                "Grain particle size."
            );
            changed |= advanced(
                ui,
                params,
                filter,
                "Procedural Advanced",
                |ui, params, filter| {
                    let mut changed = false;
                    let resolved = params.film_render.grain.resolved_grain_v2();
                    changed |= optional!(
                        ui,
                        params,
                        filter,
                        v2_shadows,
                        "Shadows",
                        resolved.shadows * 100.0,
                        "Shadow grain amount."
                    );
                    changed |= optional!(
                        ui,
                        params,
                        filter,
                        v2_midtones,
                        "Midtones",
                        resolved.midtones * 100.0,
                        "Midtone grain amount."
                    );
                    changed |= optional!(
                        ui,
                        params,
                        filter,
                        v2_highlights,
                        "Highlights",
                        resolved.highlights * 100.0,
                        "Highlight grain amount."
                    );
                    changed |= optional!(
                        ui,
                        params,
                        filter,
                        v2_chroma,
                        "Chroma",
                        resolved.color * 100.0,
                        "Chromatic grain amount."
                    );
                    changed |= optional!(
                        ui,
                        params,
                        filter,
                        v2_resolution_factor,
                        "Film Resolution",
                        resolved.resolution_factor,
                        "Resolution component of the post-scan Procedural appearance algorithm; this does not set scanner lens resolution."
                    );
                    changed |= number!(
                        ui,
                        params,
                        filter,
                        v2_resolution_type,
                        "Resolution Type",
                        "Post-scan Procedural appearance resolution method: 0 = Optical Resolution; 1 = Fast Blur. This does not change scanner readout optics."
                    );
                    changed |= number!(
                        ui,
                        params,
                        filter,
                        v2_timer,
                        "Timer",
                        "Explicit Grain Procedural phase timer; 0 uses the seed-derived phase."
                    );
                    changed
                },
            );
            if filter.active() {
                changed |= control(
                    ui,
                    params,
                    filter,
                    "Film Type",
                    "film_render.grain.v2_film_type",
                    |ui, params| {
                        enum_choice(
                            ui,
                            "Film Type",
                            &mut params.film_render.grain.v2_film_type,
                            &[
                                (GrainV2FilmType::Negative, "Negative"),
                                (GrainV2FilmType::Positive, "Positive"),
                            ],
                            "Retained Procedural Film Type metadata; rendering kernels do not consume it.",
                        )
                    },
                );
            }
        }
        GrainEngine::Fotufilm => {
            changed |= number!(
                ui,
                params,
                filter,
                fotufilm.amount,
                "Amount",
                "Effective Fotufilm grain amount."
            );
            changed |= number!(
                ui,
                params,
                filter,
                fotufilm.size,
                "Size",
                "Effective Fotufilm particle size multiplier."
            );
            changed |= number!(
                ui,
                params,
                filter,
                fotufilm.softness,
                "Softness",
                "Effective Fotufilm particle softness."
            );
            changed |= number!(
                ui,
                params,
                filter,
                fotufilm.color,
                "Color",
                "Effective chromatic grain amount."
            );
            changed |= vector!(
                ui,
                params,
                filter,
                fotufilm.layers,
                "Layer amounts",
                "Effective grain amount per emulsion layer."
            );
            changed |= advanced(
                ui,
                params,
                filter,
                "Fotufilm Advanced",
                |ui, params, filter| {
                    let mut changed = false;
                    changed |= number!(
                        ui,
                        params,
                        filter,
                        fotufilm.rms_density,
                        "RMS density",
                        "Authored effective RMS optical-density target; not measured stock calibration."
                    );
                    changed |= number!(
                        ui,
                        params,
                        filter,
                        fotufilm.clump_radius_um,
                        "Cloud radius (µm)",
                        "Authored effective population clump radius in film micrometers."
                    );
                    changed |= vector!(
                        ui,
                        params,
                        filter,
                        fotufilm.layer_weights,
                        "Layer weights",
                        "Authored effective relative population weight per layer."
                    );
                    changed |= vector!(
                        ui,
                        params,
                        filter,
                        fotufilm.layer_size_ratio,
                        "Layer size ratio",
                        "Effective relative particle size per layer."
                    );
                    changed |= number!(
                        ui,
                        params,
                        filter,
                        fotufilm.radius_span,
                        "Radius span",
                        "Effective population radius span."
                    );
                    changed |= control(
                        ui,
                        params,
                        filter,
                        "Sublayer shares",
                        "film_render.grain.fotufilm.sublayer_shares",
                        |ui, params| {
                            let mut cache = SectionLabelCache::default();
                            let mut components = ControlFilter::new("", &mut cache);
                            let changed = fotufilm_sublayer_shares(
                                ui,
                                &mut params.film_render.grain.fotufilm.sublayer_shares,
                                &mut components,
                            );
                            notice(ui, "controls.grain.fourth_sublayer_remainder");
                            changed
                        },
                    );
                    changed |= number!(
                        ui,
                        params,
                        filter,
                        fotufilm.coating_density_scale,
                        "Coating density scale",
                        "Effective coating density scale."
                    );
                    changed |= number!(
                        ui,
                        params,
                        filter,
                        fotufilm.fog_density,
                        "Fog density",
                        "Effective fog density."
                    );
                    changed |= control(
                        ui,
                        params,
                        filter,
                        "Material",
                        "film_render.grain.fotufilm.material",
                        |ui, params| {
                            enum_choice(
                                ui,
                                "Material",
                                &mut params.film_render.grain.fotufilm.material,
                                &[
                                    (FotufilmMaterial::Auto, "Auto"),
                                    (FotufilmMaterial::Silver, "Silver"),
                                    (FotufilmMaterial::DyeNegative, "Dye negative"),
                                    (FotufilmMaterial::DyeReversal, "Dye reversal"),
                                ],
                                "Effective grain material.",
                            )
                        },
                    );
                    changed
                },
            );
            if params.film_render.grain.fotufilm.material == FotufilmMaterial::Silver
                && !params.film_render.grain.monochrome
            {
                notice(ui, "controls.grain.unsupported_silver");
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    struct GrainUi {
        params: RuntimeParams,
        query: String,
    }

    fn grain_ui(ui: &mut Ui, state: &mut GrainUi) {
        let mut cache = SectionLabelCache::default();
        let mut filter = ControlFilter::new(&state.query, &mut cache);
        sections(
            ui,
            &mut state.params,
            "Grain",
            &mut ChangeFlags::default(),
            &mut filter,
        );
        filter.finish();
    }

    #[test]
    fn canonical_production_filter_keeps_equal_labels_as_independent_leaves() {
        for modified in [false, true] {
            let mut params = crate::state::GuiState::factory().runtime_params().unwrap();
            params.film_render.grain.engine = GrainEngine::Fotufilm;
            params.film_render.grain.fotufilm.size += 0.5;
            let mut harness = Harness::new_ui_state(
                move |ui, state: &mut GrainUi| {
                    install_view_filter(ui.ctx(), modified);
                    show(
                        ui,
                        &mut state.params,
                        &mut crate::state::GuiState::factory().sections,
                        "Grain",
                        ScanMedium::Negative,
                        &state.query,
                        &mut SectionLabelCache::default(),
                        "D65",
                        &crate::profiles::pick_data_dir(),
                    );
                },
                GrainUi {
                    params,
                    query: if modified {
                        String::new()
                    } else {
                        "film_render.grain.fotufilm.size".into()
                    },
                },
            );
            harness.run();
            assert_eq!(
                harness
                    .query_all_by_label("Size")
                    .filter(|node| node.role() == egui::accesskit::Role::TextInput)
                    .count(),
                1
            );
            assert!(harness.query_by_label("Procedural").is_none());
            assert!(harness.query_by_label("Spektra").is_none());
            assert!(harness.query_by_label("Dye").is_none());
        }
    }

    #[test]
    fn component_queries_reveal_the_owning_production_vector() {
        for query in ["rms granularity R", "Sublayer 1"] {
            let harness = Harness::new_ui_state(
                |ui, state: &mut GrainUi| {
                    show(
                        ui,
                        &mut state.params,
                        &mut crate::state::GuiState::factory().sections,
                        "Grain",
                        ScanMedium::Negative,
                        &state.query,
                        &mut SectionLabelCache::default(),
                        "D65",
                        &crate::profiles::pick_data_dir(),
                    );
                },
                GrainUi {
                    params: RuntimeParams::default(),
                    query: query.into(),
                },
            );
            assert_eq!(
                harness.get_by_label(query).role(),
                egui::accesskit::Role::TextInput
            );
        }
    }

    #[test]
    fn search_reveals_inactive_engines_without_changing_values_or_selection() {
        for (selected, query, label) in [
            (
                GrainEngine::Procedural,
                "particle area",
                "particle area (µm²)",
            ),
            (GrainEngine::Spektra, "shadows", "Shadows"),
            (
                GrainEngine::Spektra,
                "dye support",
                "Dye support (spatial formation)",
            ),
            (GrainEngine::Spektra, "rms density", "RMS density"),
            (GrainEngine::Spektra, "sublayer shares", "Sublayer 1"),
        ] {
            let mut params = RuntimeParams::default();
            params.film_render.grain.engine = selected;
            let before = params.clone();
            let harness = Harness::new_ui_state(
                grain_ui,
                GrainUi {
                    params,
                    query: query.into(),
                },
            );
            assert!(harness.get_by_label(label).node().is_disabled(), "{label}");
            assert_eq!(harness.state().params, before);
        }
    }

    #[test]
    fn shared_fields_have_one_editor_and_follow_material_engine_applicability() {
        for selected in [
            GrainEngine::Spektra,
            GrainEngine::Dye,
            GrainEngine::Procedural,
            GrainEngine::Fotufilm,
        ] {
            for (query, label) in [
                ("rms granularity", "rms granularity R"),
                ("density min", "density min R"),
                ("uniformity", "uniformity R"),
                ("particle scale sublayers", "particle scale sublayers 0"),
            ] {
                let mut params = RuntimeParams::default();
                params.film_render.grain.engine = selected;
                let harness = Harness::new_ui_state(
                    grain_ui,
                    GrainUi {
                        params,
                        query: query.into(),
                    },
                );
                assert_eq!(harness.query_all_by_label(label).count(), 1, "{label}");
                assert_eq!(
                    harness.get_by_label(label).node().is_disabled(),
                    if query == "rms granularity" {
                        selected != GrainEngine::Dye
                    } else {
                        !matches!(selected, GrainEngine::Spektra | GrainEngine::Dye)
                    },
                    "{label}",
                );
            }
        }
    }

    #[test]
    fn named_profile_retains_custom_controls_but_allows_amount_override() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Procedural;
        params.film_render.grain.v2_shadows = Some(72.0);
        let mut harness = Harness::new_ui_state(
            grain_ui,
            GrainUi {
                params,
                query: "shadows".into(),
            },
        );
        assert!(harness.get_by_label("Shadows").node().is_disabled());
        assert_eq!(
            harness.state().params.film_render.grain.v2_shadows,
            Some(72.0)
        );
        harness.state_mut().query = "amount".into();
        harness.run();
        // Fotufilm's independent Amount is also discoverable, but inactive.
        let amounts: Vec<_> = harness
            .query_all_by_label("Amount")
            .filter(|node| !node.node().is_disabled())
            .collect();
        assert_eq!(amounts.len(), 1);
        amounts[0].focus();
        drop(amounts);
        harness.run();
        harness.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        let amounts: Vec<_> = harness
            .query_all_by_label("Amount")
            .filter(|node| !node.node().is_disabled())
            .collect();
        amounts[0].type_text("42");
        drop(amounts);
        harness.run();
        let grain = &harness.state().params.film_render.grain;
        assert_eq!(grain.v2_amount, Some(42.0));
        assert_eq!(grain.v2_profile, "35mm250");
        assert!((grain.resolved_grain_v2().amount - 0.42).abs() < 1e-6);
        harness.state_mut().params.film_render.grain.v2_profile = "custom".into();
        harness.state_mut().query = "shadows".into();
        harness.run();
        assert!(!harness.get_by_label("Shadows").node().is_disabled());
    }

    #[test]
    fn fotufilm_fourth_share_is_read_only_and_owned_by_the_constrained_editor() {
        let mut harness = Harness::new_ui_state(
            |ui, shares: &mut [f64; 4]| {
                let mut cache = SectionLabelCache::default();
                let mut filter = ControlFilter::new("", &mut cache);
                fotufilm_sublayer_shares(ui, shares, &mut filter);
            },
            [0.25; 4],
        );
        assert_eq!(
            harness.get_by_label("Sublayer 1").role(),
            egui::accesskit::Role::TextInput
        );
        assert_eq!(
            harness
                .query_all_by_label_contains("Sublayer 4 (remainder)")
                .filter(|node| node.role() == egui::accesskit::Role::TextInput)
                .count(),
            0
        );
        harness.get_by_label("Sublayer 1").focus();
        harness.run();
        harness.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Sublayer 1").type_text("0.4");
        harness.run();
        assert_eq!(harness.state()[0], 0.4);
        assert!((harness.state()[3] - 0.1).abs() < 1e-12);
        assert!((harness.state().iter().sum::<f64>() - 1.0).abs() < 1e-12);
        let accepted = *harness.state();
        harness.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Sublayer 1").type_text("0.8");
        harness.run();
        assert_eq!(
            *harness.state(),
            accepted,
            "the first share cannot consume the positive remainder"
        );
    }

    #[test]
    fn black_and_white_dye_edits_record_zero_without_erasing_color_requests() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Dye;
        params.film_render.grain.monochrome = true;
        params.film_render.grain.density_min = [0.03, 0.07, 0.09];
        let mut harness = Harness::new_ui_state(
            grain_ui,
            GrainUi {
                params,
                query: String::new(),
            },
        );
        harness.get_by_label("Dye Advanced").click();
        harness.run();
        assert!(harness.query_by_label("density min G").is_none());
        assert!(harness.query_by_label("density min B").is_none());
        assert!(harness.query_by_label("Channel correlation").is_none());
        for index in 0..3 {
            assert!(
                !harness
                    .get_by_label(&format!("particle scale sublayers {index}"))
                    .node()
                    .is_disabled()
            );
        }
        harness.get_by_label("density min R").focus();
        harness.run();
        harness.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("density min R").type_text("0.05");
        harness.run();
        assert_eq!(
            harness.state().params.film_render.grain.density_min,
            [0.05, 0.07, 0.09]
        );
        let before = harness.state().params.clone();
        harness.state_mut().query = "density min".into();
        harness.run();
        assert!(harness.get_by_label("density min G").node().is_disabled());
        assert!(harness.get_by_label("density min B").node().is_disabled());
        assert_eq!(harness.state().params, before);
        harness.state_mut().params.film_render.grain.monochrome = false;
        harness.run();
        assert!(!harness.get_by_label("density min G").node().is_disabled());
        assert!(!harness.get_by_label("density min B").node().is_disabled());
        assert_eq!(
            harness.state().params.film_render.grain.density_min,
            [0.05, 0.07, 0.09]
        );
    }

    #[test]
    fn procedural_metadata_and_noise_resolution_are_read_only_discovery() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Procedural;
        params.film_render.grain.v2_profile = "custom".into();
        params.film_render.grain.v2_mode = GrainV2Mode::Noise;
        params.film_render.grain.v2_resolution_factor = Some(83.0);
        let before = params.clone();
        let mut harness = Harness::new_ui_state(
            grain_ui,
            GrainUi {
                params,
                query: "film resolution".into(),
            },
        );
        assert!(harness.get_by_label("Film Resolution").node().is_disabled());
        harness.state_mut().query = "resolution type".into();
        harness.run();
        assert!(harness.get_by_label("Resolution Type").node().is_disabled());
        harness.state_mut().query = "film type".into();
        harness.run();
        let mut metadata = harness.query_all_by_label("Film Type");
        assert!(
            metadata
                .next()
                .expect("stored Film Type metadata remains discoverable")
                .node()
                .is_disabled()
        );
        assert!(metadata.all(|node| node.node().is_disabled()));
        assert_eq!(harness.state().params, before);
    }
}
