use super::*;
use egui::Ui;
use egui::collapsing_header::CollapsingState;

/// The label the Engine combo and a group header use for one engine, so the
/// group a parameter belongs to reads the same as the selection.
fn engine_label(engine: GrainEngine) -> &'static str {
    match engine {
        GrainEngine::Spektra => "Spektra",
        GrainEngine::Procedural => "Procedural",
        GrainEngine::Dye => "Dye",
        GrainEngine::Fotufilm => "Fotufilm Film",
    }
}

/// Identity of one engine's parameter group. A global id keeps a group's open
/// state independent of the control panel's render path, so a collapsed group
/// survives the panel relaying out around it.
fn group_id(engine: GrainEngine) -> egui::Id {
    egui::Id::new(("grain_engine_parameters", engine_label(engine)))
}

/// Search reveals every engine without selecting it. Inactive engines retain
/// their readable controls, and disclosure state never follows engine changes.
fn engine_group(
    ui: &mut Ui,
    filter: &mut ControlFilter<'_>,
    selected: GrainEngine,
    engine: GrainEngine,
    add_contents: impl FnOnce(&mut Ui, &mut ControlFilter<'_>),
) {
    if filter.active()
        && !filter.any_matching_entry(|entry| {
            let engine_condition = match engine {
                GrainEngine::Spektra => "film_render.grain.engine = spektra",
                GrainEngine::Procedural => "film_render.grain.engine = procedural",
                GrainEngine::Dye => "film_render.grain.engine = dye",
                GrainEngine::Fotufilm => "film_render.grain.engine = fotufilm",
            };
            entry
                .conditions
                .iter()
                .any(|condition| condition.starts_with(engine_condition))
        })
    {
        return;
    }
    // Amount and Size are distinct leaves in multiple engines. Keep their
    // widget identities separate when search reveals them together.
    ui.push_id(group_id(engine), |ui| {
        let active = engine == selected;
        if filter.active() {
            ui.label(
                egui::RichText::new(engine_label(engine))
                    .color(if active { theme::FG } else { theme::FG_MUTED })
                    .strong(),
            );
            ui.add_enabled_ui(active, |ui| {
                if !active {
                    ui.label(
                        egui::RichText::new(
                            "Inactive until this engine is selected; stored values are retained.",
                        )
                        .color(theme::FG_MUTED),
                    );
                }
                add_contents(ui, filter);
            });
            return;
        }
        let mut label_clicked = false;
        let mut header =
            CollapsingState::load_with_default_open(ui.ctx(), group_id(engine), active)
                .show_header(ui, |ui| {
                    label_clicked = ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(engine_label(engine)).color(if active {
                                    theme::FG
                                } else {
                                    theme::FG_MUTED
                                }),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .clicked();
                });
        if label_clicked {
            header.toggle();
        }
        header.body(|ui| {
            if !active {
                ui.label(
                    egui::RichText::new(
                        "Inactive until this engine is selected; stored values are retained.",
                    )
                    .small()
                    .color(theme::FG_MUTED),
                );
                ui.add_enabled_ui(false, |ui| add_contents(ui, filter));
                return;
            }
            add_contents(ui, filter);
        });
    });
}

fn feature_profile_selector(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    filter: &mut ControlFilter<'_>,
    flags: &mut ChangeFlags,
    family: FeatureProfileFamily,
    label: &'static str,
    unavailable_reason: Option<&'static str>,
) {
    if !filter_matches(filter, label) {
        return;
    }
    let current = feature_profiles::effective_name(family, params);
    let mut selected = current.to_owned();
    let choices = feature_profiles::names(family)
        .iter()
        .copied()
        .map(|name| (name, name))
        .chain(std::iter::once(("custom", "Custom")));
    if let Some(reason) = unavailable_reason {
        ui.label(
            egui::RichText::new(format!("{label}: {reason}; stored values retained."))
                .color(theme::FG_MUTED),
        );
        ui.add_enabled_ui(false, |ui| {
            choice_tip_entries(
                ui,
                label,
                &mut selected,
                choices.clone(),
                "Built-in feature profile; selecting it changes only the declared profile scope.",
            );
        });
        return;
    }
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
                ui.label(egui::RichText::new(error).color(theme::FG_MUTED));
            }
        }
    }
}

/// One `show()` section group, split out of the dispatcher for readability.
pub(crate) fn sections(
    ui: &mut Ui,
    params: &mut RuntimeParams,
    section: &str,
    flags: &mut ChangeFlags,
    filter: &mut ControlFilter<'_>,
) {
    if section == "Grain" {
        feature_profile_selector(
            ui,
            params,
            filter,
            flags,
            FeatureProfileFamily::GrainProcedural,
            "Procedural Grain Profiles",
            (params.film_render.grain.engine != GrainEngine::Procedural)
                .then_some("available only when the Procedural engine is selected"),
        );
        ui.scope(|ui| {
            let grain = &mut params.film_render.grain;
            if filter_matches(filter, "enable grain") {
                flags.runtime_changed |= toggle(ui, "Enable grain", &mut grain.active, "Add grain to the negative");
            }
            if filter_matches(filter, "engine") {
                flags.runtime_changed |= enum_choice(ui, "Engine", &mut grain.engine, &[
                    (GrainEngine::Spektra, engine_label(GrainEngine::Spektra)),
                    (GrainEngine::Procedural, engine_label(GrainEngine::Procedural)),
                    (GrainEngine::Dye, engine_label(GrainEngine::Dye)),
                    (GrainEngine::Fotufilm, engine_label(GrainEngine::Fotufilm)),
                ], "Select the grain engine: Spektra for material grain in developed film density, Procedural for post-scan grain appearance in encoded RGB, Dye for material grain formed in a film-coordinate dye field, Fotufilm Film for effective-appearance film population grain model.");
            }
            ui.label(
                egui::RichText::new("Recommended: Spektra (material grain) · Procedural (post-scan, lightweight)")
                    .small()
                    .color(theme::FG_MUTED),
                );
            let engine = grain.engine;
            section_header(ui, filter, "Shared material controls", |ui, filter| {
                let active = matches!(engine, GrainEngine::Spektra | GrainEngine::Dye);
                if !active {
                    ui.label(
                        egui::RichText::new(
                            "Shared by Spektra and Dye; inactive until either engine is selected, stored values are retained.",
                        )
                        .small()
                        .color(theme::FG_MUTED),
                    );
                }
                ui.add_enabled_ui(active, |ui| {
                    if filter_matches(filter, "rms granularity") {
                        flags.runtime_changed |= param_tuple_experimental(ui, "rms granularity", &mut grain.rms_granularity, "film_render.grain.rms_granularity", "Per-channel stock granularity: Dye scales event area relative to reference stock (6); upstream Spektra retains this value as a no-op. Distinct from Procedural Amount.");
                    }
                    if filter_matches(filter, "density min") {
                        flags.runtime_changed |= param_tuple(ui, "density min", &mut grain.density_min, "film_render.grain.density_min", "Minimum grain density shared by Spektra and Dye.");
                    }
                    if filter_matches(filter, "uniformity") {
                        flags.runtime_changed |= param_tuple(ui, "uniformity", &mut grain.uniformity, "film_render.grain.uniformity", "Per-channel grain uniformity shared by Spektra and Dye.");
                    }
                    if filter_matches(filter, "particle scale sublayers") {
                        flags.runtime_changed |= param_tuple(ui, "particle scale sublayers", &mut grain.particle_scale_sublayers, "film_render.grain.particle_scale_sublayers", "Positive relative particle scale per emulsion sublayer, shared by Spektra and Dye.");
                    }
                });
            });
            engine_group(ui, filter, engine, GrainEngine::Spektra, |ui, filter| {
                if filter_matches(filter, "sublayers active") {
                    flags.runtime_changed |= toggle(
                        ui,
                        "sublayers active",
                        &mut grain.sublayers_active,
                        "Split the developed density into the film profile's emulsion sublayers instead of applying grain to the composite density. Spektra-only; Dye always uses sublayers and Procedural and Fotufilm ignore it.",
                    );
                }
                if filter_matches(filter, "particle area (µm²)") {
                    flags.runtime_changed |= param_numeric(
                        ui,
                        "particle area (µm²)",
                        &mut grain.particle_area_um2,
                        "film_render.grain.particle_area_um2",
                        "Base statistical particle area in square film micrometers; smaller values give finer grain. Spektra reads this value; Dye uses its own event area.",
                    );
                }
                if filter_matches(filter, "particle scale") {
                    flags.runtime_changed |= param_tuple(
                        ui,
                        "particle scale",
                        &mut grain.particle_scale,
                        "film_render.grain.particle_scale",
                        "Per-channel multiplier on the particle area; the default makes the blue record coarser. Spektra reads this value; Dye uses the granularity preset instead.",
                    );
                }
                if filter_matches(filter, "sub-layers") {
                    flags.runtime_changed |= param_numeric(
                        ui,
                        "sub-layers",
                        &mut grain.n_sub_layers,
                        "film_render.grain.n_sub_layers",
                        "Number of emulsion sub-layers the composite density is split into when sublayers active is off; each sub-layer draws its own grain realization. Spektra-only.",
                    );
                }
                if filter_matches(filter, "blur") {
                    flags.runtime_changed |= param_numeric(ui, "blur", &mut grain.blur, "film_render.grain.blur", "Post-blur sigma.");
                }
                if filter_matches(filter, "dye-cloud blur") {
                    flags.runtime_changed |= param_numeric(ui, "dye-cloud blur", &mut grain.blur_dye_clouds_um, "film_render.grain.blur_dye_clouds_um", "Dye-cloud blur in micrometers.");
                }
                if filter_matches(filter, "mult usm amount") {
                    flags.runtime_changed |= param_numeric(ui, "mult usm amount", &mut grain.mult_usm_amount, "film_render.grain.mult_usm_amount", "Density unsharp-mask amount.");
                }
                if filter_matches(filter, "mult usm sigma") {
                    flags.runtime_changed |= param_numeric(ui, "mult usm sigma", &mut grain.mult_usm_sigma, "film_render.grain.mult_usm_sigma", "Density unsharp-mask radius.");
                }
                if filter_matches(filter, "micro structure") {
                    flags.runtime_changed |= param_tuple(ui, "micro structure", &mut grain.micro_structure, "film_render.grain.micro_structure", "Micro-structure scale and clump size.");
                }
            });
            engine_group(ui, filter, engine, GrainEngine::Procedural, |ui, filter| {
                let custom = grain.v2_profile == "custom";
                if !custom {
                    ui.label("Custom-only controls are inactive under a named profile; stored overrides are retained. Select Custom to edit them.");
                }
                ui.add_enabled_ui(custom, |ui| {
                    if filter_matches(filter, "film type") {
                        flags.runtime_changed |= enum_choice(ui, "Film Type", &mut grain.v2_film_type, &[
                            (GrainV2FilmType::Negative, "Negative"),
                            (GrainV2FilmType::Positive, "Positive"),
                        ], "Film polarity for procedural grain.");
                    }
                    if filter_matches(filter, "processing mode") {
                        flags.runtime_changed |= enum_choice(ui, "Processing Mode", &mut grain.v2_mode, &[
                            (GrainV2Mode::Analogue, "Analogue"),
                            (GrainV2Mode::Noise, "Noise"),
                        ], "Procedural grain processing mode.");
                    }
                    let resolved = grain.resolved_grain_v2();
                    if filter_matches(filter, "size") {
                        flags.runtime_changed |= optional_numeric(ui, "Size", &mut grain.v2_size, resolved.size, "film_render.grain.v2_size", "Grain particle size.");
                    }
                    if filter_matches(filter, "shadows") {
                        flags.runtime_changed |= optional_numeric(ui, "Shadows", &mut grain.v2_shadows, resolved.shadows * 100.0, "film_render.grain.v2_shadows", "Shadow grain amount.");
                    }
                    if filter_matches(filter, "midtones") {
                        flags.runtime_changed |= optional_numeric(ui, "Midtones", &mut grain.v2_midtones, resolved.midtones * 100.0, "film_render.grain.v2_midtones", "Midtone grain amount.");
                    }
                    if filter_matches(filter, "highlights") {
                        flags.runtime_changed |= optional_numeric(ui, "Highlights", &mut grain.v2_highlights, resolved.highlights * 100.0, "film_render.grain.v2_highlights", "Highlight grain amount.");
                    }
                    if filter_matches(filter, "film resolution") {
                        flags.runtime_changed |= optional_numeric(ui, "Film Resolution", &mut grain.v2_resolution_factor, resolved.resolution_factor, "film_render.grain.v2_resolution_factor", "Resolution component of the post-scan Procedural appearance algorithm; this does not set scanner lens resolution.");
                    }
                    if filter_matches(filter, "chroma") {
                        flags.runtime_changed |= optional_numeric(ui, "Chroma", &mut grain.v2_chroma, resolved.color * 100.0, "film_render.grain.v2_chroma", "Chromatic grain amount.");
                    }
                });
                if filter_matches(filter, "resolution type") {
                    flags.runtime_changed |= param_numeric(ui, "Resolution Type", &mut grain.v2_resolution_type, "film_render.grain.v2_resolution_type", "Post-scan Procedural appearance resolution method: 0 = Optical Resolution; 1 = Fast Blur. This does not change scanner readout optics.");
                }
                if filter_matches(filter, "timer") {
                    flags.runtime_changed |= param_numeric(ui, "Timer", &mut grain.v2_timer, "film_render.grain.v2_timer", "Explicit Grain Procedural phase timer; 0 uses the seed-derived phase.");
                }
                let resolved = grain.resolved_grain_v2();
                if filter_matches(filter, "amount") {
                    flags.runtime_changed |= optional_numeric(ui, "Amount", &mut grain.v2_amount, resolved.amount * 100.0, "film_render.grain.v2_amount", "Overall grain amount.");
                }
            });
            engine_group(ui, filter, engine, GrainEngine::Dye, |ui, filter| {
                if filter_matches(filter, "dye support (µm)") {
                    flags.runtime_changed |= param_numeric(ui, "Dye support (µm)", &mut grain.v3_dye_support_um, "film_render.grain.v3_dye_support_um", "Gaussian sigma of the dye support in film micrometers; it conditions the field, which is read in film coordinates.");
                }
                if filter_matches(filter, "event area (µm²)") {
                    flags.runtime_changed |= param_numeric(ui, "Event area (µm²)", &mut grain.v3_event_area_um2, "film_render.grain.v3_event_area_um2", "Base statistical event area in square film micrometers; controls grain strength independently of dye support.");
                }
                if filter_matches(filter, "channel correlation") {
                    flags.runtime_changed |= param_numeric(ui, "Channel correlation", &mut grain.channel_correlation, "film_render.grain.channel_correlation", "Shared fraction of each emulsion layer's per-channel event rate; the realized channel correlation is a lower bound of this value, and no channel's strength changes.");
                }
            });
            engine_group(ui, filter, engine, GrainEngine::Fotufilm, |ui, filter| {
                let fotufilm = &mut grain.fotufilm;
                if filter_matches(filter, "amount") {
                    flags.runtime_changed |= param_numeric(ui, "Amount", &mut fotufilm.amount, "film_render.grain.fotufilm.amount", "Effective Fotufilm grain amount.");
                }
                if filter_matches(filter, "size") {
                    flags.runtime_changed |= param_numeric(ui, "Size", &mut fotufilm.size, "film_render.grain.fotufilm.size", "Effective Fotufilm particle size multiplier.");
                }
                if filter_matches(filter, "softness") {
                    flags.runtime_changed |= param_numeric(ui, "Softness", &mut fotufilm.softness, "film_render.grain.fotufilm.softness", "Effective Fotufilm particle softness.");
                }
                if filter_matches(filter, "color") {
                    flags.runtime_changed |= param_numeric(ui, "Color", &mut fotufilm.color, "film_render.grain.fotufilm.color", "Effective chromatic grain amount.");
                }
                if filter_matches(filter, "layer amounts") {
                    flags.runtime_changed |= param_tuple(ui, "Layer amounts", &mut fotufilm.layers, "film_render.grain.fotufilm.layers", "Effective grain amount per emulsion layer.");
                }
                section_header(ui, filter, "Fotufilm advanced", |ui, filter| {
                    if filter_matches(filter, "rms density") {
                        flags.runtime_changed |= param_numeric(ui, "RMS density", &mut fotufilm.rms_density, "film_render.grain.fotufilm.rms_density", "Authored effective RMS optical-density target; not measured stock calibration.");
                    }
                    if filter_matches(filter, "cloud radius (µm)") {
                        flags.runtime_changed |= param_numeric(ui, "Cloud radius (µm)", &mut fotufilm.clump_radius_um, "film_render.grain.fotufilm.clump_radius_um", "Authored effective population clump radius in film micrometers.");
                    }
                    if filter_matches(filter, "layer weights") {
                        flags.runtime_changed |= param_tuple(ui, "Layer weights", &mut fotufilm.layer_weights, "film_render.grain.fotufilm.layer_weights", "Authored effective relative population weight per layer.");
                    }
                    if filter_matches(filter, "layer size ratio") {
                        flags.runtime_changed |= param_tuple(ui, "Layer size ratio", &mut fotufilm.layer_size_ratio, "film_render.grain.fotufilm.layer_size_ratio", "Effective relative particle size per layer.");
                    }
                    if filter_matches(filter, "radius span") {
                        flags.runtime_changed |= param_numeric(ui, "Radius span", &mut fotufilm.radius_span, "film_render.grain.fotufilm.radius_span", "Effective population radius span.");
                    }
                    if filter_matches(filter, "sublayer shares") {
                        // The shared editor owns the constraints and remainder.
                        // A leaf-level search must reveal every component, not
                        // just the heading that matches the inventory label.
                        let mut cache = SectionLabelCache::default();
                        let mut components = ControlFilter::new("", &mut cache);
                        flags.runtime_changed |= fotufilm_sublayer_shares(ui, &mut fotufilm.sublayer_shares, &mut components);
                        ui.label("Sublayer 4 is read-only: one minus the first three shares.");
                    } else {
                        flags.runtime_changed |= fotufilm_sublayer_shares(ui, &mut fotufilm.sublayer_shares, filter);
                    }
                    if filter_matches(filter, "coating density scale") {
                        flags.runtime_changed |= param_numeric(ui, "Coating density scale", &mut fotufilm.coating_density_scale, "film_render.grain.fotufilm.coating_density_scale", "Effective coating density scale.");
                    }
                    if filter_matches(filter, "fog density") {
                        flags.runtime_changed |= param_numeric(ui, "Fog density", &mut fotufilm.fog_density, "film_render.grain.fotufilm.fog_density", "Effective fog density.");
                    }
                    if filter_matches(filter, "material") {
                        flags.runtime_changed |= enum_choice(ui, "Material", &mut fotufilm.material, &[
                            (FotufilmMaterial::Auto, "Auto"),
                            (FotufilmMaterial::Silver, "Silver"),
                            (FotufilmMaterial::DyeNegative, "Dye negative"),
                            (FotufilmMaterial::DyeReversal, "Dye reversal"),
                        ], "Effective grain material.");
                    }
                });
            });
        });
    }
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
            (GrainEngine::Spektra, "dye support", "Dye support (µm)"),
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
                    !matches!(selected, GrainEngine::Spektra | GrainEngine::Dye),
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
    fn selection_changes_preserve_independent_engine_disclosure() {
        let ctx = egui::Context::default();
        for (engine, open) in [
            (GrainEngine::Spektra, true),
            (GrainEngine::Dye, true),
            (GrainEngine::Fotufilm, false),
        ] {
            let mut state = CollapsingState::load_with_default_open(&ctx, group_id(engine), open);
            state.set_open(open);
            state.store(&ctx);
        }
        for selected in [
            GrainEngine::Spektra,
            GrainEngine::Dye,
            GrainEngine::Fotufilm,
        ] {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut cache = SectionLabelCache::default();
                    let mut filter = ControlFilter::new("", &mut cache);
                    for engine in [
                        GrainEngine::Spektra,
                        GrainEngine::Dye,
                        GrainEngine::Fotufilm,
                    ] {
                        engine_group(ui, &mut filter, selected, engine, |ui, _| {
                            ui.label("retained controls");
                        });
                    }
                });
            });
            assert!(
                CollapsingState::load(&ctx, group_id(GrainEngine::Spektra))
                    .unwrap()
                    .is_open()
            );
            assert!(
                CollapsingState::load(&ctx, group_id(GrainEngine::Dye))
                    .unwrap()
                    .is_open()
            );
            assert!(
                !CollapsingState::load(&ctx, group_id(GrainEngine::Fotufilm))
                    .unwrap()
                    .is_open()
            );
        }
    }
}
