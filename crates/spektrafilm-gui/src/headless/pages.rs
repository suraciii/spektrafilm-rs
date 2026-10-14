use super::*;

/// The Convert calibration matrix editor is a plain text field beside its
/// visible "calibration" label, so it is selected by role instead of by label;
/// typing then follows the same replace-and-commit flow as `edit`.
fn edit_calibration(h: &mut Harness<'_, App>, text: &str) {
    let field = h.get(
        egui_kittest::kittest::by()
            .role(egui::accesskit::Role::TextInput)
            .predicate(|node| node.label().is_none()),
    );
    field.focus();
    h.step();
    // egui's command modifier is platform dependent; no global environment changes.
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: !cfg!(target_os = "macos"),
            mac_cmd: cfg!(target_os = "macos"),
            command: true,
            ..Default::default()
        },
    });
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    h.step();
    h.press_key(egui::Key::Enter);
    h.run_steps(3);
}

/// Save the accepted GUI state to a scripted destination, prove the document
/// itself projects back to the accepted runtime parameters, run `between` to
/// move the live state away from the saved one, then load the file back and
/// require the exact accepted parameters again (the canonical-state roundtrip
/// from `workflow.rs`).
fn save_mutate_and_reload(
    s: &mut Session,
    file_name: &str,
    between: impl FnOnce(&mut Harness<'_, App>),
) {
    let accepted = s.harness.state().params.clone();
    let path = s.directory.path().join(file_name);
    click(&mut s.harness, "CONFIG");
    if s.harness.query_by_label("Save current to file").is_none() {
        click(&mut s.harness, "GUI parameters");
    }
    script(
        &mut s.harness,
        chooser::ChooserResult::Scripted(path.clone()),
    );
    click(&mut s.harness, "Save current to file");
    assert!(
        s.harness.state().status.starts_with("Saved GUI state"),
        "{}",
        s.harness.state().status
    );
    assert_eq!(
        state::GuiState::load(&path)
            .unwrap()
            .runtime_params()
            .unwrap(),
        accepted
    );
    between(&mut s.harness);
    click(&mut s.harness, "CONFIG");
    if s.harness.query_by_label("Load from file").is_none() {
        click(&mut s.harness, "GUI parameters");
    }
    script(&mut s.harness, chooser::ChooserResult::Scripted(path));
    click(&mut s.harness, "Load from file");
    assert!(
        s.harness.state().status.starts_with("Loaded GUI state"),
        "{}",
        s.harness.state().status
    );
    assert_eq!(s.harness.state().params, accepted);
}

// The FILM page keeps every material section expanded into the shared control
// panel; driving one section at a time keeps their repeated labels ("active",
// "scale", …) unambiguous without any production hooks.
#[test]
fn film_page_chemistry_and_material_controls_accept_edits_and_persist_through_save_load() {
    let mut s = session();
    let h = &mut s.harness;
    // A B&W stock with a development-time family gives the Chemistry
    // development-time selector real alternatives to pick from.
    click(h, "Profiles");
    let double_x = profile::load_profile_by_name(&data_dir(), "kodak_doublex").unwrap();
    let double_x_label = double_x
        .info
        .name
        .as_deref()
        .unwrap_or("kodak_doublex")
        .to_owned();
    choose(h, "film profile", &double_x_label);
    assert_eq!(h.state().film_name, "kodak_doublex");
    choose(h, "workflow", "input > film > scan");
    assert_eq!(h.state().params.workflow.route, "input > film > scan");
    click(h, "FILM");
    click(h, "Grain");

    click(h, "Base");
    let base_active = h.state().params.film_render.base.active;
    click(h, "active");
    edit(h, "scale", "1.25");
    edit(h, "tilt", "0.75");
    assert_eq!(h.state().params.film_render.base.active, !base_active);
    assert_eq!(h.state().params.film_render.base.scale, 1.25);
    assert_eq!(h.state().params.film_render.base.tilt, 0.75);
    click(h, "Base");

    click(h, "Halation");
    let halation_active = h.state().params.film_render.halation.active;
    click(h, "active");
    edit(h, "scatter amount", "0.25");
    edit(h, "boost ev", "1.5");
    assert_eq!(
        h.state().params.film_render.halation.active,
        !halation_active
    );
    assert_eq!(h.state().params.film_render.halation.scatter_amount, 0.25);
    assert_eq!(h.state().params.film_render.halation.boost_ev, 1.5);
    click(h, "Halation");

    click(h, "Couplers");
    let couplers_active = h.state().params.film_render.dir_couplers.active;
    click(h, "active");
    edit(h, "amount", "1.5");
    edit(h, "inhibition samelayer", "0.75");
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
    click(h, "Couplers");

    click(h, "Diffusion");
    let diffusion_active = h.state().params.camera.diffusion_filter.active;
    click(h, "active");
    choose(h, "filter family", "cinebloom");
    edit(h, "strength", "0.5");
    assert_eq!(
        h.state().params.camera.diffusion_filter.active,
        !diffusion_active
    );
    assert_eq!(
        h.state().params.camera.diffusion_filter.filter_family,
        "cinebloom"
    );
    assert_eq!(h.state().params.camera.diffusion_filter.strength, 0.5);
    click(h, "Diffusion");

    click(h, "Chemistry");
    choose(h, "development time", "9 min");
    let chemistry_active = h.state().params.film_render.chemistry.active;
    click(h, "active");
    edit(h, "gamma factor", "1.25");
    edit(h, "developer exhaustion", "0.375");
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
    click(h, "Chemistry");

    click(h, "Convert");
    edit_calibration(h, "1.05 0 0  0 1 0  0 0 0.95");
    assert_eq!(
        h.state().params.film_render.convert.calibration,
        "1.05 0 0  0 1 0  0 0 0.95"
    );
    click(h, "Convert");

    click(h, "Halation");
    let before_reject = h.state().params.clone();
    edit(h, "scatter amount", "not-a-number");
    assert_eq!(h.state().params, before_reject);
    assert_eq!(
        h.get_by_label("scatter amount").value().as_deref(),
        Some("0.25")
    );
    click(h, "Halation");

    save_mutate_and_reload(&mut s, "film-page.json", |h| {
        click(h, "FILM");
        click(h, "Base");
        edit(h, "tilt", "-0.5");
        assert_eq!(h.state().params.film_render.base.tilt, -0.5);
    });
}

// PRINT page sections share the FILM page's repeated labels across Chemistry,
// Print base, Glare and Enlarger diffusion; same one-section-at-a-time drive.
#[test]
fn print_page_chemistry_base_glare_and_diffusion_controls_accept_edits_and_persist_through_save_load()
 {
    let mut s = session();
    let h = &mut s.harness;
    // Glare and the enlarger controls are only active on printing routes.
    choose(h, "workflow", "input > film > print > scan");
    assert_eq!(
        h.state().params.workflow.route,
        "input > film > print > scan"
    );
    click(h, "PRINT");

    click(h, "Chemistry");
    let chemistry_active = h.state().params.print_render.density_curves_morph.active;
    click(h, "active");
    edit(h, "gamma factor", "1.5");
    edit(h, "developer exhaustion", "0.375");
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
    click(h, "Chemistry");

    click(h, "Base");
    let base_active = h.state().params.print_render.base.active;
    click(h, "active");
    edit(h, "scale", "1.25");
    edit(h, "cyan", "0.75");
    assert_eq!(h.state().params.print_render.base.active, !base_active);
    assert_eq!(h.state().params.print_render.base.scale, 1.25);
    assert_eq!(h.state().params.print_render.base.cyan, 0.75);
    click(h, "Base");

    click(h, "Print viewing glare");
    let glare_active = h.state().params.print_render.glare.active;
    click(h, "active");
    edit(h, "percent", "0.25");
    edit(h, "roughness", "0.5");
    assert_eq!(h.state().params.print_render.glare.active, !glare_active);
    assert_eq!(h.state().params.print_render.glare.percent, 0.25);
    assert_eq!(h.state().params.print_render.glare.roughness, 0.5);
    click(h, "Print viewing glare");

    click(h, "Diffusion");
    let diffusion_active = h.state().params.enlarger.diffusion_filter.active;
    click(h, "active");
    choose(h, "filter family", "black_pro_mist");
    edit(h, "strength", "0.375");
    edit(h, "halo warmth", "0.25");
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
    click(h, "Diffusion");

    click(h, "Print viewing glare");
    let before_reject = h.state().params.clone();
    edit(h, "percent", "not-a-number");
    assert_eq!(h.state().params, before_reject);
    assert_eq!(h.get_by_label("percent").value().as_deref(), Some("0.25"));
    click(h, "Print viewing glare");

    save_mutate_and_reload(&mut s, "print-page.json", |h| {
        click(h, "PRINT");
        click(h, "Base");
        edit(h, "scale", "0.5");
        assert_eq!(h.state().params.print_render.base.scale, 0.5);
    });
}

// ADVANCED page: spectral upsampling, Magazine print color strength and the
// input/output gamut compression strengths (never their algorithm selectors).
#[test]
fn advanced_page_spectral_magazine_and_gamut_controls_accept_edits_and_persist_through_save_load() {
    let mut s = session();
    let h = &mut s.harness;
    click(h, "ADVANCED");

    click(h, "Spectral upsampling");
    choose(h, "spectral upsampling", "mallett2019");
    let adaptation_window = h
        .state()
        .params
        .settings
        .apply_hanatos2025_adaptation_window;
    click(h, "hanatos2025 adaptation window");
    edit(h, "spectral gaussian blur", "2.5");
    assert_eq!(h.state().params.settings.rgb_to_raw_method, "mallett2019");
    assert_eq!(
        h.state()
            .params
            .settings
            .apply_hanatos2025_adaptation_window,
        !adaptation_window
    );
    assert_eq!(h.state().params.settings.spectral_gaussian_blur, 2.5);
    click(h, "Spectral upsampling");

    click(h, "Magazine print color");
    edit(h, "strength", "0.75");
    assert_eq!(h.state().params.magazine_print_color.strength, 0.75);
    click(h, "Magazine print color");

    click(h, "Input gamut compress");
    let compress_active = h.state().params.io.input_gamut_compress.active;
    click(h, "active");
    edit(h, "knee 0", "0.75");
    edit(h, "knee 2", "1.5");
    edit(h, "hull detail", "12");
    assert_eq!(
        h.state().params.io.input_gamut_compress.active,
        !compress_active
    );
    assert_eq!(h.state().params.io.input_gamut_compress.knee[0], 0.75);
    assert_eq!(h.state().params.io.input_gamut_compress.knee[2], 1.5);
    assert_eq!(h.state().params.io.input_gamut_compress.hull_detail, 12.0);
    click(h, "Input gamut compress");

    click(h, "Output gamut compress");
    edit(h, "knee 0", "0.5");
    edit(h, "knee 2", "4.5");
    assert_eq!(h.state().params.io.output_gamut_compress.knee[0], 0.5);
    assert_eq!(h.state().params.io.output_gamut_compress.knee[2], 4.5);
    click(h, "Output gamut compress");

    click(h, "Magazine print color");
    let before_reject = h.state().params.clone();
    edit(h, "strength", "not-a-number");
    assert_eq!(h.state().params, before_reject);
    assert_eq!(h.get_by_label("strength").value().as_deref(), Some("0.75"));
    click(h, "Magazine print color");

    save_mutate_and_reload(&mut s, "advanced-page.json", |h| {
        click(h, "ADVANCED");
        click(h, "Magazine print color");
        edit(h, "strength", "0.25");
        assert_eq!(h.state().params.magazine_print_color.strength, 0.25);
    });
}

// MAIN page singles: Magazine print, Camera film format and selectors, the
// print-active Enlarger block including preflash, and Crop and upscale whose
// normalized crop moves the source window before the upscale resamples it.
#[test]
fn camera_enlarger_crop_upscale_and_magazine_print_controls_edit_accepted_params_geometry_and_persist()
 {
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "input > film > print > scan");
    assert_eq!(
        h.state().params.workflow.route,
        "input > film > print > scan"
    );

    let magazine_active = h.state().params.magazine_print_color.active;
    click(h, "Magazine print");
    assert_eq!(
        h.state().params.magazine_print_color.active,
        !magazine_active
    );
    click(h, "Magazine print");
    assert_eq!(
        h.state().params.magazine_print_color.active,
        magazine_active
    );

    edit(h, "film format mm", "56");
    choose(h, "auto exposure method", "matrix");
    choose(h, "color filter", "hoya_y2");
    assert_eq!(h.state().params.camera.film_format_mm, 56.0);
    assert_eq!(h.state().params.camera.auto_exposure_method, "matrix");
    assert_eq!(h.state().params.camera.color_filter, "hoya_y2");

    edit(h, "print exposure", "0.5");
    choose(h, "print exposure mode", "manual");
    edit(h, "print y filter shift", "0.75");
    edit(h, "print m filter shift", "-0.5");
    edit(h, "preflash exposure", "0.25");
    edit(h, "preflash y filter shift", "1.5");
    edit(h, "preflash m filter shift", "-0.75");
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

    click(h, "Crop and upscale");
    edit(h, "crop center 0", "0.25");
    edit(h, "crop center 1", "0.5");
    edit(h, "crop size 0", "0.5");
    edit(h, "crop size 1", "0.5");
    click(h, "crop");
    assert!(h.state().params.io.crop);
    assert_eq!(h.state().params.io.crop_center, [0.25, 0.5]);
    assert_eq!(h.state().params.io.crop_size, [0.5, 0.5]);

    // Finished RGB keeps the input ramp's polarity, so the crop window's
    // source region is observable in the output pixels.
    choose(h, "workflow", "Finished RGB");
    assert_eq!(h.state().params.workflow.route, "input");
    let left = render(h, "SCAN");
    assert_eq!((left.width, left.height), (80, 80));
    edit(h, "crop center 0", "0.75");
    assert_eq!(h.state().params.io.crop_center, [0.75, 0.5]);
    let right = render(h, "SCAN");
    assert_eq!((right.width, right.height), (80, 80));
    assert!(
        mean(&right) > mean(&left) + 0.05,
        "crop center must move the source window: left {} right {}",
        mean(&left),
        mean(&right)
    );
    edit(h, "upscale factor", "2");
    assert_eq!(h.state().params.io.upscale_factor, 2.0);
    let up = render(h, "SCAN");
    assert_eq!((up.width, up.height), (160, 160));
    click(h, "crop");
    assert!(!h.state().params.io.crop);
    let full = render(h, "SCAN");
    assert_eq!((full.width, full.height), (320, 160));

    let before_reject = h.state().params.clone();
    edit(h, "upscale factor", "not-a-number");
    assert_eq!(h.state().params, before_reject);
    assert_eq!(
        h.get_by_label("upscale factor").value().as_deref(),
        Some("2.00")
    );

    save_mutate_and_reload(&mut s, "main-page.json", |h| {
        click(h, "MAIN");
        edit(h, "film format mm", "42");
        assert_eq!(h.state().params.camera.film_format_mm, 42.0);
    });
}

// specs/output-mix/spec.md: Overall effect amount blends the prepared source
// with the complete result in linear output RGB — exact endpoints and a known
// intermediate mixture, driven through the real MAIN Output scenario.
#[test]
fn overall_effect_amount_scales_render_between_prepared_source_and_complete_result() {
    use spektrafilm_math::colorspace::{cctf_decode, resolve};

    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "input > film > scan");
    assert_eq!(h.state().params.workflow.route, "input > film > scan");

    assert_eq!(h.state().params.output_mix.amount, 1.0);
    let complete = render(h, "PREVIEW");
    edit(h, "Overall effect amount", "0.0");
    assert_eq!(h.state().params.output_mix.amount, 0.0);
    let prepared = render(h, "PREVIEW");
    assert!(
        prepared.data != complete.data,
        "amount 0 and 1 must reach distinct mix branches"
    );
    edit(h, "Overall effect amount", "0.25");
    assert_eq!(h.state().params.output_mix.amount, 0.25);
    let mixed = render(h, "PREVIEW");

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
    assert!(mixed.data != complete.data);
    assert!(mixed.data != prepared.data);

    let before_reject = h.state().params.clone();
    edit(h, "Overall effect amount", "not-a-number");
    assert_eq!(h.state().params, before_reject);
    assert_eq!(
        h.get_by_label("Overall effect amount").value().as_deref(),
        Some("0.250")
    );

    choose(h, "output color space", "Display P3");
    choose(h, "saving color space", "ProPhoto RGB");
    let saving_cctf = h.state().gui_state.sections["simulation"]["saving_cctf_encoding"]
        .as_bool()
        .unwrap_or(true);
    click(h, "saving cctf encoding");
    assert_eq!(h.state().params.io.output_color_space, "Display P3");
    assert_eq!(
        h.state().gui_state.sections["simulation"]["saving_color_space"].as_str(),
        Some("ProPhoto RGB")
    );
    assert_eq!(
        h.state().gui_state.sections["simulation"]["saving_cctf_encoding"].as_bool(),
        Some(!saving_cctf)
    );

    // The Display update button must request a preview at the new max size
    // without any further PREVIEW click.
    click(h, "CONFIG");
    click(h, "Display");
    edit(h, "preview max size", "256");
    assert_eq!(h.state().params.settings.preview_max_size, 256);
    let previous = h.state().output_image.clone();
    click(h, "update");
    wait_until(h, |a| {
        a.render_job.is_none()
            && a.output_image
                .as_ref()
                .is_some_and(|out| previous.as_ref().is_none_or(|old| !Arc::ptr_eq(old, out)))
    });
    let resized = h.state().output_image.as_ref().unwrap();
    assert_eq!((resized.width, resized.height), (160, 80));
    assert!(
        h.state().status.starts_with("Preview ·"),
        "{}",
        h.state().status
    );
    assert_eq!(
        h.state().gui_state.sections["display"]["preview_max_size"].as_u64(),
        Some(256)
    );

    save_mutate_and_reload(&mut s, "output-mix.json", |h| {
        click(h, "MAIN");
        edit(h, "Overall effect amount", "0.75");
        assert_eq!(h.state().params.output_mix.amount, 0.75);
    });
}
