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
    if section == "Grain" {
        ui.scope(|ui| {
            let grain = &mut params.film_render.grain;
            if filter_matches(filter, "active") {
                flags.runtime_changed |= toggle(ui, "active", &mut grain.active, "Add grain to the negative");
            }
            if filter_matches(filter, "engine") {
                flags.runtime_changed |= enum_choice(ui, "Engine", &mut grain.engine, &[
                    (GrainEngine::Spektra, "Spektra"),
                    (GrainEngine::Procedural, "Procedural"),
                    (GrainEngine::Dye, "Dye"),
                    (GrainEngine::Fotufilm, "Fotufilm Film"),
                ], "Select the grain engine: Spektra for material grain in developed film density, Procedural for post-scan grain appearance in encoded RGB, Dye for material grain formed in a film-coordinate dye field, Fotufilm Film for effective-appearance film population grain model.");
            }
            ui.label(
                egui::RichText::new("Recommended: Spektra (material grain) · Procedural (post-scan, lightweight)")
                    .small()
                    .color(egui::Color32::GRAY),
                );
            if grain.engine == GrainEngine::Procedural {
                if filter_matches(filter, "grain profiles") {
                    let mut profile = grain.v2_profile.clone();
                    egui::ComboBox::from_label("Grain Profiles").selected_text(&profile).show_ui(ui, |ui| {
                        for name in PROFILE_NAMES { ui.selectable_value(&mut profile, (*name).to_owned(), name); }
                        ui.selectable_value(&mut profile, "custom".to_owned(), "Custom");
                    });
                    if profile != grain.v2_profile {
                        if profile == "custom" { grain.select_custom_grain_v2(); } else { grain.v2_profile = profile; grain.v2_amount = None; }
                        flags.runtime_changed = true;
                    }
                }
                if grain.v2_profile == "custom" {
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
                }
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
            } else if grain.engine == GrainEngine::Fotufilm {
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
                    flags.runtime_changed |=
                        fotufilm_sublayer_shares(ui, &mut fotufilm.sublayer_shares, filter);
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
            } else if grain.engine == GrainEngine::Dye {
                if filter_matches(filter, "dye support (µm)") {
                    flags.runtime_changed |= param_numeric(ui, "Dye support (µm)", &mut grain.v3_dye_support_um, "film_render.grain.v3_dye_support_um", "Gaussian sigma of the dye support in film micrometers; it conditions the field, which is read in film coordinates.");
                }
                if filter_matches(filter, "event area (µm²)") {
                    flags.runtime_changed |= param_numeric(ui, "Event area (µm²)", &mut grain.v3_event_area_um2, "film_render.grain.v3_event_area_um2", "Base statistical event area in square film micrometers; controls grain strength independently of dye support.");
                }
                if filter_matches(filter, "channel correlation") {
                    flags.runtime_changed |= param_numeric(ui, "Channel correlation", &mut grain.channel_correlation, "film_render.grain.channel_correlation", "Shared fraction of each emulsion layer's per-channel event rate; the realized channel correlation is a lower bound of this value, and no channel's strength changes.");
                }
                if filter_matches(filter, "rms granularity (experimental)") {
                    flags.runtime_changed |= param_tuple(ui, "rms granularity (experimental)", &mut grain.rms_granularity, "film_render.grain.rms_granularity", "Per-channel stock granularity; sets each channel's strength relative to the reference stock (6).");
                }
                if filter_matches(filter, "particle scale sublayers") {
                    flags.runtime_changed |= param_tuple(ui, "particle scale sublayers", &mut grain.particle_scale_sublayers, "film_render.grain.particle_scale_sublayers", "Positive relative particle scale per emulsion sublayer.");
                }
                if filter_matches(filter, "density min") {
                    flags.runtime_changed |= param_tuple(ui, "density min", &mut grain.density_min, "film_render.grain.density_min", "Minimum grain density.");
                }
                if filter_matches(filter, "uniformity") {
                    flags.runtime_changed |= param_tuple(ui, "uniformity", &mut grain.uniformity, "film_render.grain.uniformity", "Per-channel grain uniformity.");
                }
            } else {
                if filter_matches(filter, "rms granularity (experimental)") {
                    flags.runtime_changed |= param_tuple(ui, "rms granularity (experimental)", &mut grain.rms_granularity, "film_render.grain.rms_granularity", "Per-channel RMS granularity.");
                }
                if filter_matches(filter, "density min") {
                    flags.runtime_changed |= param_tuple(ui, "density min", &mut grain.density_min, "film_render.grain.density_min", "Minimum grain density.");
                }
                if filter_matches(filter, "uniformity") {
                    flags.runtime_changed |= param_tuple(ui, "uniformity", &mut grain.uniformity, "film_render.grain.uniformity", "Per-channel grain uniformity.");
                }
                if filter_matches(filter, "particle scale sublayers") {
                    flags.runtime_changed |= param_tuple(ui, "particle scale sublayers", &mut grain.particle_scale_sublayers, "film_render.grain.particle_scale_sublayers", "Relative particle scale per emulsion sublayer.");
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
            }
        });
    }
}
