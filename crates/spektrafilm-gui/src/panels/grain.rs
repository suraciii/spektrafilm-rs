use spektrafilm_core::params::grain::{GrainEngine, GrainParams, GrainV2Mode};

pub fn show(ui: &mut egui::Ui, g: &mut GrainParams) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Grain")
        .default_open(true)
        .show(ui, |ui| {
            changed |= ui.checkbox(&mut g.active, "Active").changed();
            egui::ComboBox::from_label("Engine")
                .selected_text(match g.engine {
                    GrainEngine::V1 => "V1 — emulsion grain",
                    GrainEngine::V2 => "V2 — procedural grain",
                    GrainEngine::V3 => "V3 — film-coordinate dye field",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut g.engine, GrainEngine::V1, "V1 — emulsion grain")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut g.engine, GrainEngine::V2, "V2 — procedural grain")
                        .changed();
                });
            if matches!(g.engine, GrainEngine::V2) {
                use spektrafilm_core::params::grain::GrainV2FilmType;
                let mut selected_profile = g.v2_profile.clone();
                egui::ComboBox::from_label("Grain Profiles")
                    .selected_text(&selected_profile)
                    .show_ui(ui, |ui| {
                        for profile in spektrafilm_model::grain::v2::PROFILE_NAMES {
                            ui.selectable_value(&mut selected_profile, profile.to_owned(), profile);
                        }
                        ui.selectable_value(&mut selected_profile, "custom".into(), "Custom");
                    });
                if selected_profile != g.v2_profile {
                    if selected_profile == "custom" {
                        g.select_custom_grain_v2();
                    } else {
                        g.v2_profile = selected_profile;
                        g.v2_amount = None;
                    }
                    changed = true;
                }
                if g.v2_profile == "custom" {
                    egui::ComboBox::from_label("Film Type")
                        .selected_text(match g.v2_film_type {
                            GrainV2FilmType::Negative => "Negative",
                            GrainV2FilmType::Positive => "Positive",
                        })
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(
                                    &mut g.v2_film_type,
                                    GrainV2FilmType::Negative,
                                    "Negative",
                                )
                                .changed();
                            changed |= ui
                                .selectable_value(
                                    &mut g.v2_film_type,
                                    GrainV2FilmType::Positive,
                                    "Positive",
                                )
                                .changed();
                        });
                    egui::ComboBox::from_label("Processing Mode")
                        .selected_text(match g.v2_mode {
                            GrainV2Mode::Analogue => "Analogue",
                            GrainV2Mode::Noise => "Noise",
                        })
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(&mut g.v2_mode, GrainV2Mode::Analogue, "Analogue")
                                .changed();
                            changed |= ui
                                .selectable_value(&mut g.v2_mode, GrainV2Mode::Noise, "Noise")
                                .changed();
                        });
                    let resolved = g.resolved_grain_v2();
                    for (label, value, inherited, min, max) in [
                        ("Size", &mut g.v2_size, resolved.size, 1.0, 48.0),
                        (
                            "Shadows",
                            &mut g.v2_shadows,
                            resolved.shadows * 100.0,
                            0.0,
                            100.0,
                        ),
                        (
                            "Midtones",
                            &mut g.v2_midtones,
                            resolved.midtones * 100.0,
                            0.0,
                            100.0,
                        ),
                        (
                            "Highlights",
                            &mut g.v2_highlights,
                            resolved.highlights * 100.0,
                            0.0,
                            100.0,
                        ),
                        (
                            "Film Resolution",
                            &mut g.v2_resolution_factor,
                            resolved.resolution_factor,
                            0.0,
                            100.0,
                        ),
                        (
                            "Chroma",
                            &mut g.v2_chroma,
                            resolved.color * 100.0,
                            0.0,
                            100.0,
                        ),
                    ] {
                        let mut displayed = value.unwrap_or(inherited);
                        if ui
                            .add(egui::Slider::new(&mut displayed, min..=max).text(label))
                            .changed()
                        {
                            *value = Some(displayed);
                            changed = true;
                        }
                    }
                }
                let mut amount = g.resolved_grain_v2().amount * 100.0;
                if ui
                    .add(egui::Slider::new(&mut amount, 0.0..=100.0).text("Amount"))
                    .changed()
                {
                    g.v2_amount = Some(amount);
                    changed = true;
                }
            } else {
                changed |= ui
                    .checkbox(&mut g.sublayers_active, "Layered sublayer grain")
                    .on_hover_text(
                        "Split the composite density into the emulsion's sublayers and grain \
                 each with its own particle field, dye-cloud blur and micro-structure \
                 (the Python 0.3.4 default). Off = single composite-density sampler.",
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut g.particle_area_um2, 0.05..=1.0)
                            .text("Particle area (µm²)"),
                    )
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut g.blur, 0.0..=3.0).text("Post-blur σ"))
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut g.blur_dye_clouds_um, 0.0..=10.0)
                            .text("Dye-cloud blur (µm)"),
                    )
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut g.n_sub_layers, 1..=4).text("Sub-layers"))
                    .on_hover_text(
                        "Composite-sampler sub-layer count (layered grain always uses the \
                 profile's 3 emulsion sublayers).",
                    )
                    .changed();
            }
        });

    changed
}
