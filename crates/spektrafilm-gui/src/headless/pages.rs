use super::*;

fn edit_control(h: &mut Harness<'_, App>, path: &str, label: &str, text: &str) {
    reveal_control(h, path);
    edit(h, label, text);
}

fn choose_control(h: &mut Harness<'_, App>, path: &str, label: &str, option: &str) {
    reveal_control(h, path);
    choose(h, label, option);
}

fn toggle_control(h: &mut Harness<'_, App>, path: &str, label: &str) {
    reveal_control(h, path);
    scroll_controls_into_view(h, label);
    let node = h
        .query_by_role_and_label(egui::accesskit::Role::Switch, label)
        .or_else(|| h.query_by_role_and_label(egui::accesskit::Role::CheckBox, label))
        .unwrap_or_else(|| panic!("No boolean editor found for {path}: {label}"));
    node.simulate_click();
    h.run_steps(3);
}

fn set_route(h: &mut Harness<'_, App>, route: &str) {
    choose_control(h, "workflow.route", "workflow", route);
    assert_eq!(h.state().params.workflow.route, route);
}

fn select_film(h: &mut Harness<'_, App>, label: &str) {
    search_controls(h, "");
    navigate_stage(h, controls::Stage::Film);
    if h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "film profile")
        .is_none()
    {
        click(h, "Film material");
    }
    choose(h, "film profile", label);
}

/// Convert retains its free-form calibration editor beside the matrix rows.
fn edit_calibration(h: &mut Harness<'_, App>, text: &str) {
    reveal_control(h, "film_render.convert.calibration");
    let field = h.get(
        egui_kittest::kittest::by()
            .role(egui::accesskit::Role::TextInput)
            .predicate(|node| node.label().is_none()),
    );
    field.focus();
    h.step();
    h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    h.step();
    h.press_key(egui::Key::Enter);
    h.run_steps(3);
}

/// Exercise the production Settings state writer and loader, not a runtime-only
/// serialization roundtrip. Dialog navigation must not change accepted values.
/// Saving GUI parameters is not saving the photograph record: reloading must
/// preserve the current adjustment until the user explicitly discards it.
fn save_mutate_and_reload(
    s: &mut Session,
    file_name: &str,
    between: impl FnOnce(&mut Harness<'_, App>),
) {
    let accepted = s.harness.state().params.clone();
    let path = s.directory.path().join(file_name);
    open_settings(&mut s.harness);
    if s.harness.query_by_label("Save current to file").is_none() {
        click(&mut s.harness, "GUI parameters");
    }
    script(
        &mut s.harness,
        chooser::ChooserResult::Scripted(path.clone()),
    );
    click(&mut s.harness, "Save current to file");
    assert_eq!(
        state::GuiState::load(&path)
            .unwrap()
            .runtime_params()
            .unwrap(),
        accepted
    );
    close_settings(&mut s.harness);
    between(&mut s.harness);
    let changed = s.harness.state().params.clone();
    let revision = s.harness.state().parameter_revision;
    open_settings(&mut s.harness);
    if s.harness.query_by_label("Load from file").is_none() {
        click(&mut s.harness, "GUI parameters");
    }
    script(&mut s.harness, chooser::ChooserResult::Scripted(path));
    click(&mut s.harness, "Load from file");
    assert!(s.harness.query_by_label("Discard changes").is_some());
    assert_eq!(s.harness.state().params, changed);
    assert_eq!(s.harness.state().parameter_revision, revision);
    click(&mut s.harness, "Discard changes");
    assert!(s.harness.query_by_label("Discard changes").is_none());
    assert_eq!(s.harness.state().params, accepted);
    close_settings(&mut s.harness);
}

#[test]
fn film_chemistry_material_and_capture_controls_accept_edits_and_persist() {
    let mut s = session();
    let h = &mut s.harness;
    let double_x = profile::load_profile_by_name(&data_dir(), "kodak_doublex").unwrap();
    select_film(h, double_x.info.name.as_deref().unwrap_or("kodak_doublex"));
    assert_eq!(h.state().film_name, "kodak_doublex");
    set_route(h, "input > film > scan");

    let base_active = h.state().params.film_render.base.active;
    toggle_control(h, "film_render.base.active", "Enable film base");
    edit_control(h, "film_render.base.scale", "scale", "1.25");
    edit_control(h, "film_render.base.tilt", "tilt", "0.75");
    assert_eq!(h.state().params.film_render.base.active, !base_active);
    assert_eq!(h.state().params.film_render.base.scale, 1.25);
    assert_eq!(h.state().params.film_render.base.tilt, 0.75);

    let halation_active = h.state().params.film_render.halation.active;
    toggle_control(h, "film_render.halation.active", "Enable halation");
    edit_control(
        h,
        "film_render.halation.scatter_amount",
        "scatter amount",
        "0.25",
    );
    edit_control(h, "film_render.halation.boost_ev", "boost ev", "1.5");
    assert_eq!(
        h.state().params.film_render.halation.active,
        !halation_active
    );
    assert_eq!(h.state().params.film_render.halation.scatter_amount, 0.25);
    assert_eq!(h.state().params.film_render.halation.boost_ev, 1.5);

    let couplers_active = h.state().params.film_render.dir_couplers.active;
    toggle_control(h, "film_render.dir_couplers.active", "Enable DIR couplers");
    edit_control(
        h,
        "film_render.dir_couplers.amount",
        "inhibition amount",
        "1.5",
    );
    edit_control(
        h,
        "film_render.dir_couplers.inhibition_samelayer",
        "inhibition samelayer",
        "0.75",
    );
    assert_eq!(
        h.state().params.film_render.dir_couplers.active,
        !couplers_active
    );
    assert_eq!(h.state().params.film_render.dir_couplers.amount, 1.5);
    assert_eq!(
        h.state()
            .params
            .film_render
            .dir_couplers
            .inhibition_samelayer,
        0.75
    );

    let diffusion_active = h.state().params.camera.diffusion_filter.active;
    toggle_control(h, "camera.diffusion_filter.active", "Camera diffusion");
    choose_control(
        h,
        "camera.diffusion_filter.filter_family",
        "filter family",
        "cinebloom",
    );
    edit_control(h, "camera.diffusion_filter.strength", "strength", "0.5");
    assert_eq!(
        h.state().params.camera.diffusion_filter.active,
        !diffusion_active
    );
    assert_eq!(
        h.state().params.camera.diffusion_filter.filter_family,
        "cinebloom"
    );
    assert_eq!(h.state().params.camera.diffusion_filter.strength, 0.5);

    choose_control(
        h,
        "film_render.development_time",
        "development time",
        "9 min",
    );
    let chemistry_active = h.state().params.film_render.chemistry.active;
    toggle_control(
        h,
        "film_render.chemistry.active",
        "Enable film density curves",
    );
    edit_control(
        h,
        "film_render.chemistry.gamma_factor",
        "gamma factor",
        "1.25",
    );
    edit_control(
        h,
        "film_render.chemistry.developer_exhaustion",
        "developer exhaustion",
        "0.375",
    );
    assert_eq!(h.state().params.film_render.development_time, Some(9.0));
    assert_eq!(
        h.state().params.film_render.chemistry.active,
        !chemistry_active
    );
    assert_eq!(h.state().params.film_render.chemistry.gamma_factor, 1.25);
    assert_eq!(
        h.state().params.film_render.chemistry.developer_exhaustion,
        0.375
    );

    edit_calibration(h, "1.05 0 0  0 1 0  0 0 0.95");
    assert_eq!(
        h.state().params.film_render.convert.calibration,
        "1.05 0 0  0 1 0  0 0 0.95"
    );
    let before_reject = h.state().params.clone();
    edit_control(
        h,
        "film_render.halation.scatter_amount",
        "scatter amount",
        "not-a-number",
    );
    assert_eq!(h.state().params, before_reject);
    assert_eq!(
        h.get_by_label("scatter amount")
            .value()
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        0.25
    );

    save_mutate_and_reload(&mut s, "film-controls.json", |h| {
        edit_control(h, "film_render.base.tilt", "tilt", "-0.5");
        assert_eq!(h.state().params.film_render.base.tilt, -0.5);
    });
}

#[test]
fn print_chemistry_base_glare_and_diffusion_controls_accept_edits_and_persist() {
    let mut s = session();
    let h = &mut s.harness;
    set_route(h, "input > film > print > scan");
    let chemistry_active = h.state().params.print_render.density_curves_morph.active;
    toggle_control(
        h,
        "print_render.density_curves_morph.active",
        "Enable print density curves",
    );
    edit_control(
        h,
        "print_render.density_curves_morph.gamma_factor",
        "gamma factor",
        "1.5",
    );
    edit_control(
        h,
        "print_render.density_curves_morph.developer_exhaustion",
        "developer exhaustion",
        "0.375",
    );
    assert_eq!(
        h.state().params.print_render.density_curves_morph.active,
        !chemistry_active
    );
    assert_eq!(
        h.state()
            .params
            .print_render
            .density_curves_morph
            .gamma_factor,
        1.5
    );
    assert_eq!(
        h.state()
            .params
            .print_render
            .density_curves_morph
            .developer_exhaustion,
        0.375
    );

    let base_active = h.state().params.print_render.base.active;
    toggle_control(h, "print_render.base.active", "Enable print base");
    edit_control(h, "print_render.base.scale", "scale", "1.25");
    edit_control(h, "print_render.base.cyan", "cyan", "0.75");
    assert_eq!(h.state().params.print_render.base.active, !base_active);
    assert_eq!(h.state().params.print_render.base.scale, 1.25);
    assert_eq!(h.state().params.print_render.base.cyan, 0.75);

    let glare_active = h.state().params.print_render.glare.active;
    toggle_control(h, "print_render.glare.active", "Enable print glare");
    edit_control(h, "print_render.glare.percent", "percent", "0.25");
    edit_control(h, "print_render.glare.roughness", "roughness", "0.5");
    assert_eq!(h.state().params.print_render.glare.active, !glare_active);
    assert_eq!(h.state().params.print_render.glare.percent, 0.25);
    assert_eq!(h.state().params.print_render.glare.roughness, 0.5);

    let diffusion_active = h.state().params.enlarger.diffusion_filter.active;
    toggle_control(h, "enlarger.diffusion_filter.active", "Enlarger diffusion");
    choose_control(
        h,
        "enlarger.diffusion_filter.filter_family",
        "filter family",
        "black_pro_mist",
    );
    edit_control(h, "enlarger.diffusion_filter.strength", "strength", "0.375");
    edit_control(
        h,
        "enlarger.diffusion_filter.halo_warmth",
        "halo warmth",
        "0.25",
    );
    assert_eq!(
        h.state().params.enlarger.diffusion_filter.active,
        !diffusion_active
    );
    assert_eq!(
        h.state().params.enlarger.diffusion_filter.filter_family,
        "black_pro_mist"
    );
    assert_eq!(h.state().params.enlarger.diffusion_filter.strength, 0.375);
    assert_eq!(h.state().params.enlarger.diffusion_filter.halo_warmth, 0.25);

    let before_reject = h.state().params.clone();
    edit_control(h, "print_render.glare.percent", "percent", "not-a-number");
    assert_eq!(h.state().params, before_reject);
    assert_eq!(
        h.get_by_label("percent")
            .value()
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        0.25
    );
    save_mutate_and_reload(&mut s, "print-controls.json", |h| {
        edit_control(h, "print_render.base.scale", "scale", "0.5");
        assert_eq!(h.state().params.print_render.base.scale, 0.5);
    });
}

#[test]
fn spectral_cmyk_and_gamut_controls_accept_edits_and_persist() {
    let mut s = session();
    let h = &mut s.harness;
    choose_control(
        h,
        "settings.rgb_to_raw_method",
        "spectral upsampling",
        "mallett2019",
    );
    let adaptation_window = h
        .state()
        .params
        .settings
        .apply_hanatos2025_adaptation_window;
    toggle_control(
        h,
        "settings.apply_hanatos2025_adaptation_window",
        "hanatos2025 adaptation window",
    );
    edit_control(
        h,
        "settings.spectral_gaussian_blur",
        "spectral gaussian blur",
        "2.5",
    );
    assert_eq!(h.state().params.settings.rgb_to_raw_method, "mallett2019");
    assert_eq!(
        h.state()
            .params
            .settings
            .apply_hanatos2025_adaptation_window,
        !adaptation_window
    );
    assert_eq!(h.state().params.settings.spectral_gaussian_blur, 2.5);

    let compress_active = h.state().params.io.input_gamut_compress.active;
    toggle_control(
        h,
        "io.input_gamut_compress.active",
        "Enable input gamut compress",
    );
    edit_control(h, "io.input_gamut_compress.knee", "knee threshold", "0.75");
    edit_control(h, "io.input_gamut_compress.knee", "knee power", "1.5");
    edit_control(
        h,
        "io.input_gamut_compress.hull_detail",
        "hull detail",
        "12",
    );
    assert_eq!(
        h.state().params.io.input_gamut_compress.active,
        !compress_active
    );
    assert_eq!(h.state().params.io.input_gamut_compress.knee[0], 0.75);
    assert_eq!(h.state().params.io.input_gamut_compress.knee[2], 1.5);
    assert_eq!(h.state().params.io.input_gamut_compress.hull_detail, 12.0);
    edit_control(h, "io.output_gamut_compress.knee", "knee threshold", "0.5");
    edit_control(h, "io.output_gamut_compress.knee", "knee power", "4.5");
    assert_eq!(h.state().params.io.output_gamut_compress.knee[0], 0.5);
    assert_eq!(h.state().params.io.output_gamut_compress.knee[2], 4.5);

    edit_control(h, "cmyk.strength", "strength", "0.75");
    assert_eq!(h.state().params.cmyk.strength, 0.75);
    let before_reject = h.state().params.clone();
    edit(h, "strength", "not-a-number");
    assert_eq!(h.state().params, before_reject);
    assert_eq!(
        h.get_by_label("strength")
            .value()
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        0.75
    );
    save_mutate_and_reload(&mut s, "spectral-controls.json", |h| {
        edit_control(h, "cmyk.strength", "strength", "0.25");
        assert_eq!(h.state().params.cmyk.strength, 0.25);
    });
}

#[test]
fn camera_enlarger_crop_upscale_and_cmyk_controls_edit_params_geometry_and_persist() {
    let mut s = session();
    let h = &mut s.harness;
    set_route(h, "input > film > print > scan");
    let cmyk_active = h.state().params.cmyk.active;
    toggle_control(h, "cmyk.active", "Enable CMYK ink style");
    assert_eq!(h.state().params.cmyk.active, !cmyk_active);
    toggle_control(h, "cmyk.active", "Enable CMYK ink style");
    assert_eq!(h.state().params.cmyk.active, cmyk_active);

    choose_control(h, "camera.film_format_mm", "Film Format", "120");
    choose_control(
        h,
        "camera.auto_exposure_method",
        "auto exposure method",
        "matrix",
    );
    choose_control(h, "camera.color_filter", "color filter", "hoya_y2");
    assert_eq!(h.state().params.camera.film_format_mm, 56.0);
    assert_eq!(h.state().params.camera.auto_exposure_method, "matrix");
    assert_eq!(h.state().params.camera.color_filter, "hoya_y2");
    edit_control(h, "enlarger.print_exposure", "print exposure", "0.5");
    choose_control(
        h,
        "enlarger.print_exposure_mode",
        "print exposure mode",
        "manual",
    );
    edit_control(h, "enlarger.y_filter_shift", "print y filter shift", "0.75");
    edit_control(h, "enlarger.m_filter_shift", "print m filter shift", "-0.5");
    edit_control(h, "enlarger.preflash_exposure", "preflash exposure", "0.25");
    edit_control(
        h,
        "enlarger.preflash_y_filter_shift",
        "preflash y filter shift",
        "1.5",
    );
    edit_control(
        h,
        "enlarger.preflash_m_filter_shift",
        "preflash m filter shift",
        "-0.75",
    );
    assert_eq!(h.state().params.enlarger.print_exposure, 0.5);
    assert_eq!(
        h.state().params.enlarger.print_exposure_mode,
        spektrafilm_core::params::PrintExposureMode::Manual
    );
    assert_eq!(h.state().params.enlarger.y_filter_shift, 0.75);
    assert_eq!(h.state().params.enlarger.m_filter_shift, -0.5);
    assert_eq!(h.state().params.enlarger.preflash_exposure, 0.25);
    assert_eq!(h.state().params.enlarger.preflash_y_filter_shift, 1.5);
    assert_eq!(h.state().params.enlarger.preflash_m_filter_shift, -0.75);

    edit_control(h, "io.crop_center", "crop center x", "0.25");
    edit_control(h, "io.crop_center", "crop center y", "0.5");
    edit_control(h, "io.crop_size", "crop size x", "0.5");
    edit_control(h, "io.crop_size", "crop size y", "0.5");
    toggle_control(h, "io.crop", "crop");
    assert!(h.state().params.io.crop);
    assert_eq!(h.state().params.io.crop_center, [0.25, 0.5]);
    assert_eq!(h.state().params.io.crop_size, [0.5, 0.5]);
    set_route(h, "input");
    let left = scan(h);
    assert_eq!((left.width, left.height), (80, 80));
    edit_control(h, "io.crop_center", "crop center x", "0.75");
    assert_eq!(h.state().params.io.crop_center, [0.75, 0.5]);
    let right = scan(h);
    assert_eq!((right.width, right.height), (80, 80));
    assert!(
        mean(&right) > mean(&left) + 0.05,
        "crop center must move the source window: left {} right {}",
        mean(&left),
        mean(&right)
    );
    edit_control(h, "io.upscale_factor", "upscale factor", "2");
    assert_eq!(h.state().params.io.upscale_factor, 2.0);
    let up = scan(h);
    assert_eq!((up.width, up.height), (160, 160));
    toggle_control(h, "io.crop", "crop");
    assert!(!h.state().params.io.crop);
    let full = scan(h);
    assert_eq!((full.width, full.height), (320, 160));
    let before_reject = h.state().params.clone();
    edit_control(h, "io.upscale_factor", "upscale factor", "not-a-number");
    assert_eq!(h.state().params, before_reject);
    assert_eq!(
        h.get_by_label("upscale factor")
            .value()
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        2.0
    );
    save_mutate_and_reload(&mut s, "capture-controls.json", |h| {
        choose_control(h, "camera.film_format_mm", "Film Format", "Custom");
        edit(h, "Frame Long Edge", "42");
        assert_eq!(h.state().params.camera.film_format_mm, 42.0);
    });
}

#[test]
fn film_format_specs_change_only_physical_scale_and_preserve_custom_values() {
    let mut s = session();
    let h = &mut s.harness;
    let original = h.state().params.clone();
    reveal_control(h, "camera.film_format_mm");
    assert_eq!(
        h.state().params,
        original,
        "opening must retain legacy 35 mm"
    );
    assert!(h.query_by_label("Frame Long Edge").is_some());
    for query in ["Frame Long Edge", "long edge"] {
        search_controls(h, query);
        assert!(
            h.query_by_label("Locate Input and capture: Camera")
                .is_some()
        );
        assert!(h.query_by_label("Frame Long Edge").is_some());
    }
    reveal_control(h, "camera.film_format_mm");
    for (name, mm) in [("35mm", 36.0), ("120", 56.0), ("4×5", 127.0)] {
        choose(h, "Film Format", name);
        let mut expected = original.clone();
        expected.camera.film_format_mm = mm;
        let mut preparation = expected.render_preparation_state();
        preparation
            .explicit_section_paths
            .push("camera.film_format_mm".into());
        expected.restore_render_preparation_state(&preparation);
        assert_eq!(
            h.state().params,
            expected,
            "{name} must not change crop or effects"
        );
        assert!(h.query_by_label("Frame Long Edge").is_none());
    }
    click(h, "Undo");
    assert_eq!(h.state().params.camera.film_format_mm, 56.0);
    click(h, "Redo");
    assert_eq!(h.state().params.camera.film_format_mm, 127.0);
    let revision = h.state().parameter_revision;
    choose(h, "Film Format", "Custom");
    assert_eq!(
        h.state().parameter_revision,
        revision,
        "revealing custom is not an edit"
    );
    edit(h, "Frame Long Edge", "42");
    assert_eq!(h.state().params.camera.film_format_mm, 42.0);
    click(h, "reset Camera");
    assert_eq!(
        h.state().params.camera.film_format_mm,
        original.camera.film_format_mm
    );
    click(h, "Undo");
    assert_eq!(h.state().params.camera.film_format_mm, 42.0);
    save_mutate_and_reload(&mut s, "custom-format.json", |h| {
        choose_control(h, "camera.film_format_mm", "Film Format", "120");
    });
    reveal_control(&mut s.harness, "camera.film_format_mm");
    assert_eq!(s.harness.state().params.camera.film_format_mm, 42.0);
    assert!(s.harness.query_by_label("Frame Long Edge").is_some());
}

#[test]
fn simulation_blend_scales_render_between_prepared_source_and_complete_result() {
    use spektrafilm_math::colorspace::{cctf_decode, resolve};
    let mut s = session();
    let h = &mut s.harness;
    set_route(h, "input > film > scan");
    assert_eq!(h.state().params.output_mix.amount, 1.0);
    let complete = preview(h);
    edit_control(h, "output_mix.amount", "Simulation blend", "0.0");
    assert_eq!(h.state().params.output_mix.amount, 0.0);
    let prepared = preview(h);
    assert_ne!(
        prepared.data, complete.data,
        "amount 0 and 1 must reach distinct mix branches"
    );
    edit_control(h, "output_mix.amount", "Simulation blend", "0.25");
    assert_eq!(h.state().params.output_mix.amount, 0.25);
    let mixed = preview(h);
    let space = resolve(&h.state().output_color_space).unwrap();
    let encoded = h.state().output_cctf_encoding;
    let linear = |value: f64| {
        if encoded {
            cctf_decode(value, space.cctf)
        } else {
            value
        }
    };
    assert_eq!(mixed.data.len(), complete.data.len());
    assert_eq!(prepared.data.len(), complete.data.len());
    for ((&m, &p), &c) in mixed.data.iter().zip(&prepared.data).zip(&complete.data) {
        let expected = 0.75 * linear(p as f64) + 0.25 * linear(c as f64);
        let actual = linear(m as f64);
        assert!(
            (actual - expected).abs() <= 1e-5,
            "mixed sample {actual} differs from interpolated {expected}"
        );
    }
    assert_ne!(mixed.data, complete.data);
    assert_ne!(mixed.data, prepared.data);
    let before_reject = h.state().params.clone();
    edit(h, "Simulation blend", "not-a-number");
    assert_eq!(h.state().params, before_reject);
    assert_eq!(
        h.get_by_label("Simulation blend")
            .value()
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        0.25
    );

    choose_control(
        h,
        "io.output_color_space",
        "output color space",
        "Display P3",
    );
    choose_control(
        h,
        "extras.simulation.saving_color_space",
        "saving color space",
        "ProPhoto RGB",
    );
    let saving_cctf = h.state().gui_state.sections["simulation"]["saving_cctf_encoding"]
        .as_bool()
        .unwrap_or(true);
    toggle_control(
        h,
        "extras.simulation.saving_cctf_encoding",
        "saving cctf encoding",
    );
    assert_eq!(h.state().params.io.output_color_space, "Display P3");
    assert_eq!(
        h.state().gui_state.sections["simulation"]["saving_color_space"].as_str(),
        Some("ProPhoto RGB")
    );
    assert_eq!(
        h.state().gui_state.sections["simulation"]["saving_cctf_encoding"].as_bool(),
        Some(!saving_cctf)
    );

    reveal_control(h, "settings.preview_max_size");
    edit(h, "preview max size", "256");
    assert_eq!(h.state().params.settings.preview_max_size, 256);
    assert!(h.query_by_label("update").is_none());
    close_settings(h);
    let resized = preview(h);
    assert_eq!((resized.width, resized.height), (160, 80));
    assert!(
        h.state().status.to_string().starts_with("Preview ·"),
        "{}",
        h.state().status
    );
    assert_eq!(
        h.state().gui_state.sections["display"]["preview_max_size"].as_u64(),
        Some(256)
    );
    save_mutate_and_reload(&mut s, "output-mix.json", |h| {
        edit_control(h, "output_mix.amount", "Simulation blend", "0.75");
        assert_eq!(h.state().params.output_mix.amount, 0.75);
    });
}

#[test]
fn preflash_zero_and_nonprinting_route_retain_independent_filters_and_main_exposure() {
    let mut s = session();
    let h = &mut s.harness;
    set_route(h, "input > film > print > scan");
    edit_control(h, "enlarger.print_exposure", "print exposure", "0.5");
    edit_control(h, "enlarger.preflash_exposure", "preflash exposure", "0.25");
    edit_control(
        h,
        "enlarger.preflash_y_filter_shift",
        "preflash y filter shift",
        "1.5",
    );
    edit_control(
        h,
        "enlarger.preflash_m_filter_shift",
        "preflash m filter shift",
        "-0.75",
    );
    edit_control(h, "enlarger.preflash_exposure", "preflash exposure", "0");
    assert_eq!(h.state().params.enlarger.preflash_exposure, 0.0);
    assert_eq!(h.state().params.enlarger.preflash_y_filter_shift, 1.5);
    assert_eq!(h.state().params.enlarger.preflash_m_filter_shift, -0.75);
    assert_eq!(h.state().params.enlarger.print_exposure, 0.5);
    let retained = h.state().params.enlarger.clone();
    set_route(h, "input > film > scan");
    for (path, label) in [
        ("enlarger.preflash_exposure", "preflash exposure"),
        (
            "enlarger.preflash_y_filter_shift",
            "preflash y filter shift",
        ),
        (
            "enlarger.preflash_m_filter_shift",
            "preflash m filter shift",
        ),
    ] {
        reveal_control(h, path);
        assert!(
            h.get_by_label(label).node().is_disabled(),
            "inactive row {path} must be readable and not editable"
        );
        assert_eq!(h.state().params.enlarger, retained);
    }
    set_route(h, "input > film > print > scan");
    reveal_control(h, "enlarger.preflash_y_filter_shift");
    assert!(
        !h.get_by_label("preflash y filter shift")
            .node()
            .is_disabled()
    );
    assert_eq!(h.state().params.enlarger, retained);
}

#[test]
fn canonical_search_reveals_unselected_grain_engines_without_mutating_values() {
    let mut s = session();
    let h = &mut s.harness;
    choose_control(h, "film_render.grain.engine", "Engine", "Spektra");
    let params = h.state().params.clone();
    let sections = h.state().gui_state.sections.clone();
    for (path, label) in [
        ("film_render.grain.fotufilm.rms_density", "RMS density"),
        ("film_render.grain.v3_event_area_um2", "Event area (µm²)"),
        ("film_render.grain.v2_timer", "Timer"),
    ] {
        reveal_control(h, path);
        assert!(
            h.get_by_label(label).node().is_disabled(),
            "unselected engine control {path} must be read-only"
        );
        assert_eq!(h.state().params, params);
        assert_eq!(h.state().gui_state.sections, sections);
    }
    choose_control(h, "film_render.grain.engine", "Engine", "Fotufilm Film");
    edit_control(
        h,
        "film_render.grain.fotufilm.rms_density",
        "RMS density",
        "0.04",
    );
    let fotufilm = h.state().params.film_render.grain.fotufilm.clone();
    choose_control(h, "film_render.grain.engine", "Engine", "Dye");
    edit_control(
        h,
        "film_render.grain.v3_event_area_um2",
        "Event area (µm²)",
        "0.5",
    );
    let event_area = h.state().params.film_render.grain.v3_event_area_um2;
    choose_control(h, "film_render.grain.engine", "Engine", "Fotufilm Film");
    reveal_control(h, "film_render.grain.fotufilm.rms_density");
    assert!(!h.get_by_label("RMS density").node().is_disabled());
    assert_eq!(h.state().params.film_render.grain.fotufilm, fotufilm);
    assert_eq!(
        h.state().params.film_render.grain.v3_event_area_um2,
        event_area
    );
}

#[test]
fn dye_correlation_survives_bw_profile_switch_and_reactivates_for_color() {
    let mut s = session();
    let h = &mut s.harness;
    let color = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    select_film(h, color.info.name.as_deref().unwrap_or("kodak_portra_400"));
    choose_control(h, "film_render.grain.engine", "Engine", "Dye");
    edit_control(
        h,
        "film_render.grain.channel_correlation",
        "Channel correlation",
        "0.35",
    );
    let mut retained = h.state().params.film_render.grain.clone();
    assert!(!retained.monochrome);
    assert_eq!(retained.channel_correlation, 0.35);

    let bw = profile::load_profile_by_name(&data_dir(), "kodak_doublex").unwrap();
    select_film(h, bw.info.name.as_deref().unwrap_or("kodak_doublex"));
    retained.monochrome = true;
    assert_eq!(h.state().params.film_render.grain, retained);
    reveal_control(h, "film_render.grain.channel_correlation");
    assert!(h.get_by_label("Channel correlation").node().is_disabled());
    assert!(
        h.query_by_label("Inactive for single-record black-and-white material; the requested value is retained for color film.")
            .is_some()
    );
    // Ordinary GUI edits must not reset the profile-derived applicability.
    edit_control(
        h,
        "film_render.grain.v3_event_area_um2",
        "Event area (µm²)",
        "0.5",
    );
    assert!(h.state().params.film_render.grain.monochrome);
    assert_eq!(h.state().params.film_render.grain.channel_correlation, 0.35);

    select_film(h, color.info.name.as_deref().unwrap_or("kodak_portra_400"));
    assert!(!h.state().params.film_render.grain.monochrome);
    reveal_control(h, "film_render.grain.channel_correlation");
    assert!(!h.get_by_label("Channel correlation").node().is_disabled());
    assert_eq!(h.state().params.film_render.grain.channel_correlation, 0.35);
    edit(h, "Channel correlation", "0.25");
    assert_eq!(h.state().params.film_render.grain.channel_correlation, 0.25);
}

#[test]
fn dye_bw_grain_reset_preserves_layout_and_clears_factory_marker() {
    let mut s = session();
    let h = &mut s.harness;
    choose_control(h, "film_render.grain.engine", "Engine", "Dye");
    edit_control(
        h,
        "film_render.grain.channel_correlation",
        "Channel correlation",
        "0.35",
    );
    let bw = profile::load_profile_by_name(&data_dir(), "kodak_doublex").unwrap();
    select_film(h, bw.info.name.as_deref().unwrap_or("kodak_doublex"));
    let edited = h.state().params.film_render.grain.clone();
    assert!(edited.monochrome);
    reveal_control(h, "film_render.grain.channel_correlation");
    click(h, "reset Grain");
    assert!(h.state().params.film_render.grain.monochrome);
    assert!(!section_reset::is_modified(
        "Grain",
        &h.state().params,
        &serde_json::json!({})
    ));
    click(h, "Undo");
    assert_eq!(h.state().params.film_render.grain, edited);
    click(h, "Redo");
    assert!(h.state().params.film_render.grain.monochrome);
    assert!(!section_reset::is_modified(
        "Grain",
        &h.state().params,
        &serde_json::json!({})
    ));
    choose_control(h, "film_render.grain.engine", "Engine", "Dye");
    reveal_control(h, "film_render.grain.channel_correlation");
    assert!(h.get_by_label("Channel correlation").node().is_disabled());
}

#[test]
fn control_search_by_label_module_and_path_never_changes_parameters_or_saved_sections() {
    let mut s = session();
    let h = &mut s.harness;
    let params = h.state().params.clone();
    let sections = h.state().gui_state.sections.clone();
    search_controls(h, "UPSCALE");
    assert!(h.query_by_label("upscale factor").is_some());
    search_controls(h, "Halation");
    assert!(h.query_by_label("scatter amount").is_some());
    reveal_control(h, "film_render.chemistry.active");
    assert!(h.query_by_label("Enable film density curves").is_some());
    reveal_control(h, "print_render.density_curves_morph.active");
    assert!(h.query_by_label("Enable print density curves").is_some());
    search_controls(h, "no-such-control-label");
    assert!(h.query_by_label("upscale factor").is_none());
    assert!(h.query_by_label("scatter amount").is_none());
    search_controls(h, "");
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().gui_state.sections, sections);
}

#[test]
fn scanner_search_reveals_independent_rows_on_an_inactive_route() {
    let mut s = session();
    let h = &mut s.harness;
    set_route(h, "input");
    let params = h.state().params.clone();
    reveal_control(h, "scanner.conversion_style");
    assert!(
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "conversion style")
            .node()
            .is_disabled()
    );
    reveal_control(h, "scanner.lens_blur");
    assert!(
        h.get_by_label("Lens blur (readout optics)")
            .node()
            .is_disabled()
    );

    assert!(
        h.query_by_label("Unsharp mask (digital sharpening) sigma")
            .is_none()
    );
    assert_eq!(h.state().params, params);
}

#[test]
fn module_resets_restore_only_their_owned_values_and_are_reversible() {
    let mut s = session();
    let h = &mut s.harness;
    set_route(h, "input > film > print > scan");
    let factory = state::GuiState::factory().runtime_params().unwrap();
    edit_control(h, "enlarger.print_exposure", "print exposure", "0.5");
    edit_control(h, "enlarger.preflash_exposure", "preflash exposure", "0.25");
    edit_control(
        h,
        "enlarger.preflash_y_filter_shift",
        "preflash y filter shift",
        "1.5",
    );
    edit_control(
        h,
        "enlarger.preflash_m_filter_shift",
        "preflash m filter shift",
        "-0.75",
    );
    let edited = h.state().params.clone();
    click(h, "Only modified");
    assert_eq!(
        h.state().params,
        edited,
        "Only modified is presentation, not an edit"
    );

    reveal_control(h, "enlarger.preflash_y_filter_shift");
    click(h, "reset Preflash");
    let mut expected = edited.clone();
    expected.enlarger.preflash_exposure = factory.enlarger.preflash_exposure;
    expected.enlarger.preflash_y_filter_shift = factory.enlarger.preflash_y_filter_shift;
    expected.enlarger.preflash_m_filter_shift = factory.enlarger.preflash_m_filter_shift;
    assert_eq!(
        h.state().params,
        expected,
        "Preflash reset must not own main enlarger exposure or diffusion"
    );
    assert!(
        h.query_by_label("preflash y filter shift").is_none(),
        "factory preflash rows leave Only modified results"
    );
    click(h, "Undo");
    assert_eq!(h.state().params, edited);
    assert!(h.query_by_label("preflash y filter shift").is_some());
    click(h, "Only modified");

    reveal_control(h, "enlarger.print_exposure");
    click(h, "reset Enlarger");
    expected = edited.clone();
    expected.enlarger.print_exposure = factory.enlarger.print_exposure;
    assert_eq!(
        h.state().params,
        expected,
        "Enlarger reset must retain separate Preflash controls"
    );
    assert_eq!(
        h.state().params.enlarger.preflash_y_filter_shift,
        edited.enlarger.preflash_y_filter_shift
    );
}

#[test]
fn global_search_locates_settings_and_export_without_mutation_or_filtering_the_dialog() {
    let mut s = session();
    let h = &mut s.harness;
    let params = h.state().params.clone();
    let sections = h.state().gui_state.sections.clone();
    let options = h.state().export_options.clone();
    reveal_control(h, "settings.preview_max_size");
    assert!(h.state().settings_open);
    assert!(h.query_by_label("preview max size").is_some());
    assert!(h.query_by_label("auto preview").is_none());
    assert!(h.query_by_label("Update").is_none());
    assert!(!h.state().export_dialog.is_open());
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().gui_state.sections, sections);
    close_settings(h);

    reveal_control(h, "extras.rust.export_saving_color_space");
    assert!(h.state().export_dialog.is_open());
    assert!(!h.state().settings_open);
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "Saving color space")
            .is_some()
    );
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "Format")
            .is_some(),
        "Export must show its real unfiltered draft, not an inspector editor"
    );
    click(h, "Cancel");
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().gui_state.sections, sections);
    let current = &h.state().export_options;
    assert_eq!(current.backend, options.backend);
    assert_eq!(current.format, options.format);
    assert_eq!(current.depth, options.depth);
    assert_eq!(current.jpeg_quality, options.jpeg_quality);
    assert_eq!(current.jpeg_subsampling, options.jpeg_subsampling);
    assert_eq!(current.compression, options.compression);
    assert_eq!(current.saving_color_space, options.saving_color_space);
    assert_eq!(current.saving_cctf_encoding, options.saving_cctf_encoding);
}

#[test]
fn locating_another_export_control_preserves_the_open_draft() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "extras.rust.export_saving_color_space");
    let committed = h.state().export_options.clone();
    choose(h, "Format", "TIFF");
    choose(h, "Saving color space", "ACES2065-1");
    search_controls(h, "extras.rust.export_saving_color_space");
    click(h, "Locate Export: Export options");
    choose(h, "Bit depth", "32 bit");
    let destination = s.directory.path().join("located-draft.tif");
    script(h, chooser::Chooser::chooser_path(&destination));
    export_action(h, "Export file…");
    wait_until(h, |app| app.export_job.is_none());
    assert!(destination.is_file());
    assert_eq!(h.state().export_options.format, export::ExportFormat::Tiff);
    assert_eq!(h.state().export_options.saving_color_space, "ACES2065-1");
    assert_eq!(h.state().params.io.output_color_space, "sRGB");
    assert_ne!(h.state().export_options, committed);
}

#[test]
fn collapsed_grain_subsections_remain_accessible_and_shared_fields_have_one_active_editor() {
    let mut s = session();
    let h = &mut s.harness;
    choose_control(h, "film_render.grain.engine", "Engine", "Fotufilm Film");
    edit_control(
        h,
        "film_render.grain.fotufilm.rms_density",
        "RMS density",
        "0.04",
    );
    assert_eq!(
        h.state().params.film_render.grain.fotufilm.rms_density,
        0.04
    );
    reveal_control(h, "film_render.grain.fotufilm.sublayer_shares");
    assert!(h.query_by_label("Sublayer 2").is_some());
    let retained = h.state().params.film_render.grain.fotufilm.clone();
    choose_control(h, "film_render.grain.engine", "Engine", "Spektra");
    reveal_control(h, "film_render.grain.rms_granularity");
    assert_eq!(
        h.query_all_by_label("rms granularity R")
            .filter(|node| !node.node().is_disabled())
            .count(),
        1,
        "one persisted component has one active editor"
    );
    assert_eq!(
        h.query_all_by_label("rms granularity R").count(),
        1,
        "inactive engines must not duplicate a shared editor"
    );
    assert_eq!(h.state().params.film_render.grain.fotufilm, retained);
}

#[test]
fn film_material_selection_retains_the_independently_selected_print_material() {
    let mut s = session();
    let h = &mut s.harness;
    let print = h.state().print_name.clone();
    let stock = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    select_film(h, stock.info.name.as_deref().unwrap_or("kodak_portra_400"));
    assert_eq!(h.state().film_name, "kodak_portra_400");
    assert_eq!(h.state().print_name, print);
    assert!(
        h.state().params.validate().is_ok(),
        "material defaulting must leave an admitted route and scanner configuration"
    );
}

#[test]
fn applied_look_materials_follow_factory_filtering_and_undo() {
    let mut s = session();
    let h = &mut s.harness;
    if h.query_by_label("▶").is_some() {
        click(h, "▶");
    }
    edit(h, "Search presets", "Kodak Portra 400");
    click(h, "Kodak Portra 400");
    assert_eq!(h.state().film_name, "kodak_portra_400");
    search_controls(h, "extras.simulation.film_stock");
    click(h, "Only modified");
    assert!(h.query_by_label("Locate Film: Film material").is_some());
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "film profile")
            .is_some()
    );
    click(h, "Undo");
    assert_eq!(h.state().film_name, state::GuiState::factory().film());
    assert!(h.query_by_label("Locate Film: Film material").is_none());
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "film profile")
            .is_none()
    );
    click(h, "Redo");
    assert_eq!(h.state().film_name, "kodak_portra_400");
    assert!(h.query_by_label("Locate Film: Film material").is_some());
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "film profile")
            .is_some()
    );
}

#[test]
fn camera_module_reset_retains_camera_diffusion_and_other_stage_edits() {
    let mut s = session();
    let h = &mut s.harness;
    set_route(h, "input > film > print > scan");
    let factory = state::GuiState::factory().runtime_params().unwrap();
    edit_control(
        h,
        "camera.exposure_compensation_ev",
        "exposure compensation ev",
        "0.75",
    );
    edit_control(h, "camera.diffusion_filter.strength", "strength", "0.5");
    edit_control(h, "enlarger.print_exposure", "print exposure", "2.0");
    let mut expected = h.state().params.clone();
    expected.camera.exposure_compensation_ev = factory.camera.exposure_compensation_ev;
    expected.camera.auto_exposure = factory.camera.auto_exposure;
    reveal_control(h, "camera.exposure_compensation_ev");
    click(h, "reset Camera");
    assert_eq!(
        h.state().params,
        expected,
        "Camera reset must not replace its separately owned diffusion or another stage"
    );
}

#[test]
fn halation_feature_profile_retains_other_controls_and_rgb_edit_undo_restores_identity() {
    use spektrafilm_core::feature_profiles::{self, FeatureProfileFamily, FeatureProfileValues};
    let mut s = session();
    let h = &mut s.harness;
    set_route(h, "input > film > print > scan");
    // This fixture exercises real spatial controls rather than the shared
    // deterministic render fixture's spatial-effect bypass.
    h.state_mut().params.debug.deactivate_spatial_effects = false;
    h.step();
    if !h.state().params.film_render.halation.active {
        toggle_control(h, "film_render.halation.active", "Enable halation");
    }
    edit_control(
        h,
        "camera.exposure_compensation_ev",
        "exposure compensation ev",
        "0.75",
    );
    edit_control(
        h,
        "film_render.halation.scatter_amount",
        "scatter amount",
        "0.25",
    );
    let family = FeatureProfileFamily::Halation;
    let initial_name = feature_profiles::effective_name(family, &h.state().params);
    let name = feature_profiles::names(family)
        .iter()
        .copied()
        .find(|name| *name != initial_name)
        .unwrap();
    let before = h.state().params.clone();
    let materials = (h.state().film_name.clone(), h.state().print_name.clone());
    let FeatureProfileValues::Halation {
        first_sigma_um,
        strength,
    } = feature_profiles::resolve(family, name).unwrap().values
    else {
        unreachable!()
    };
    let mut expected = before.clone();
    expected.film_render.halation.halation_first_sigma_um = first_sigma_um;
    expected.film_render.halation.halation_strength = strength;
    choose_control(h, "action.halation_profile", "Halation Profiles", name);
    assert_eq!(
        serde_json::to_value(&h.state().params).unwrap(),
        serde_json::to_value(&expected).unwrap(),
        "a Halation feature profile owns only its declared strength and spread vectors"
    );
    assert_eq!(
        (&h.state().film_name, &h.state().print_name),
        (&materials.0, &materials.1)
    );
    assert!(h.state().params.film_render.halation.active);
    assert!(!h.state().params.debug.deactivate_spatial_effects);
    assert_eq!(
        feature_profiles::effective_name(family, &h.state().params),
        name
    );
    let named = h.state().params.clone();

    edit_control(
        h,
        "film_render.halation.halation_strength",
        "halation strength G",
        "0.037",
    );
    let mut expected_edit = serde_json::to_value(&named).unwrap();
    expected_edit["film_render"]["halation"]["halation_strength"][1] = serde_json::json!(0.037);
    assert_eq!(
        serde_json::to_value(&h.state().params).unwrap(),
        expected_edit,
        "editing green must retain the independent red and blue channels and all other controls"
    );
    assert_eq!(
        feature_profiles::effective_name(family, &h.state().params),
        "custom"
    );
    click(h, "Undo");
    assert_eq!(
        h.state().params,
        named,
        "Undo restores the accepted named profile and its edit provenance"
    );
    assert_eq!(
        feature_profiles::effective_name(family, &h.state().params),
        name
    );
}
