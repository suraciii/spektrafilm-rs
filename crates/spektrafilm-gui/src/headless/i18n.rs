//! Localization through the production App, controls, workers and persistence.
use super::*;
use crate::i18n::{LanguagePreference, Locale};

fn language(h: &mut Harness<'_, App>, preference: LanguagePreference) {
    let ctx = h.ctx.clone();
    h.state_mut().select_language(&ctx, preference).unwrap();
    h.run_steps(3);
}

#[test]
fn switching_language_preserves_photographic_identity_and_bilingual_search() {
    let mut s = session();
    let h = &mut s.harness;
    preview(h);
    let params = h.state().params.clone();
    let sections = h.state().gui_state.sections.clone();
    let revision = h.state().parameter_revision;
    let input_epoch = h.state().input_epoch;
    let input = h.state().image.clone().unwrap();
    language(h, LanguagePreference::SimplifiedChinese);
    assert_eq!(crate::i18n::locale(&h.ctx), Locale::SimplifiedChinese);
    for label in [
        "打开",
        "撤销",
        "重做",
        "设置",
        "导出",
        "扫描",
        "保存",
        "原图",
        "修改图",
    ] {
        assert!(
            h.query_all_by_label(label).next().is_some(),
            "Chinese action {label}"
        );
    }
    assert!(h.query_by_label("预览").is_none());
    for query in [
        "胶片规格",
        "Film Format",
        "画面长边",
        "Frame Long Edge",
        "camera.film_format_mm",
    ] {
        h.get_by_label("搜索控件").focus();
        h.step();
        h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.get_by_label("搜索控件").type_text(query);
        h.run_steps(3);
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "胶片规格")
                .is_some(),
            "canonical owner for {query}"
        );
    }
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().gui_state.sections, sections);
    assert_eq!(h.state().parameter_revision, revision);
    assert_eq!(h.state().input_epoch, input_epoch);
    assert!(h.state().image.as_ref().unwrap().same_source(&input));
    assert!(h.state().render_job.is_none());
    language(h, LanguagePreference::English);
    assert!(h.query_by_label("Search controls").is_some());
    assert!(h.query_by_label("Original").is_some());
    assert!(h.query_by_label("Modified").is_some());
    assert_eq!(
        h.get_by_label("Search controls").value(),
        Some("camera.film_format_mm".into())
    );
}

#[test]
fn grain_correlation_search_survives_language_switch_without_changing_the_recipe() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "film_render.grain.engine");
    choose(h, "Engine", "Dye");
    let params = h.state().params.clone();
    let revision = h.state().parameter_revision;
    for (preference, search, label) in [
        (
            LanguagePreference::English,
            "Search controls",
            "Channel correlation",
        ),
        (
            LanguagePreference::SimplifiedChinese,
            "搜索控件",
            "通道相关性",
        ),
        (
            LanguagePreference::English,
            "Search controls",
            "Channel correlation",
        ),
    ] {
        language(h, preference);
        for query in [
            "Channel correlation",
            "通道相关性",
            "film_render.grain.channel_correlation",
        ] {
            h.get_by_label(search).focus();
            h.step();
            h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.get_by_label(search).type_text(query);
            h.run_steps(3);
            assert!(
                h.query_by_role_and_label(egui::accesskit::Role::TextInput, label)
                    .is_some(),
                "grain control unavailable for {query}"
            );
            assert_eq!(h.state().params, params);
            assert_eq!(h.state().parameter_revision, revision);
        }
    }
}

#[test]
fn language_switch_retains_invalid_numeric_draft_and_control_focus() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "camera.exposure_compensation_ev");
    h.get_by_label("exposure compensation ev").focus();
    h.step();
    h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.input_mut().events.push(egui::Event::Text("-".into()));
    h.step();
    let focus = h.ctx.memory(|m| m.focused());
    let params = h.state().params.clone();
    let revision = h.state().parameter_revision;
    language(h, LanguagePreference::SimplifiedChinese);
    assert_eq!(h.ctx.memory(|m| m.focused()), focus);
    let field = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .find(|node| node.node().value() == Some("-".into()))
        .expect("draft retained");
    assert!(field.node().is_focused());
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().parameter_revision, revision);
    language(h, LanguagePreference::English);
    assert_eq!(
        h.get_by_label("exposure compensation ev").value(),
        Some("-".into())
    );
    h.press_key(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().params, params);
}

#[test]
fn locale_preference_survives_state_load_factory_and_application_restart() {
    let mut s = session();
    let path = s.directory.path().join("photograph.json");
    let state = s.harness.state().current_state().unwrap();
    state.save(&path).unwrap();
    language(&mut s.harness, LanguagePreference::SimplifiedChinese);
    s.harness
        .state_mut()
        .apply_state_with_source(state::GuiState::load(&path).unwrap(), |_| {})
        .unwrap();
    await_input(&mut s.harness);
    assert_eq!(
        crate::i18n::locale(&s.harness.ctx),
        Locale::SimplifiedChinese
    );
    s.harness
        .state_mut()
        .apply_state_with_source(state::GuiState::factory(), |_| {})
        .unwrap();
    await_input(&mut s.harness);
    assert_eq!(
        crate::i18n::locale(&s.harness.ctx),
        Locale::SimplifiedChinese
    );
    let saved = s.harness.state().current_state().unwrap().sections;
    assert!(
        !saved.to_string().contains("language"),
        "photographic carrier excludes language"
    );
    let browser = s.directory.path().join("restart-browser.json");
    let restarted =
        App::new_with_browser_path(Arc::new(CpuBackend), None, None, None, browser, data_dir());
    let ctx = egui::Context::default();
    restarted.bind_language(&ctx);
    assert_eq!(crate::i18n::locale(&ctx), Locale::SimplifiedChinese);
}

#[test]
fn live_language_switch_does_not_change_retained_save_or_independent_export_pixels() {
    let mut s = session();
    let h = &mut s.harness;
    let output = scan(h);
    let revision = h.state().parameter_revision;
    let before = s.directory.path().join("english.png");
    save_retained(h, &before);
    let retained = h.state().output_image.clone().unwrap();
    language(h, LanguagePreference::SimplifiedChinese);
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &retained
    ));
    assert_eq!(h.state().parameter_revision, revision);
    let after = s.directory.path().join("中文保存.png");
    save_retained(h, &after);
    let english = image_io::load(&before).unwrap().image;
    let chinese = image_io::load(&after).unwrap().image;
    assert_eq!(
        (english.width, english.height),
        (chinese.width, chinese.height)
    );
    assert_eq!(english.data, chinese.data);
    let exported = s.directory.path().join("中文导出.png");
    let options = h.state().export_options.clone();
    let ctx = h.ctx.clone();
    h.state_mut().start_export_to_path(&ctx, options, &exported);
    wait_until(h, |app| app.export_job.is_none());
    let expected = image_io::convert_image(
        &output,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&exported, &expected);
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &retained
    ));
}

#[test]
fn chinese_ime_composition_commits_search_without_adjusting_the_photograph() {
    let mut s = session();
    let h = &mut s.harness;
    language(h, LanguagePreference::SimplifiedChinese);
    let params = h.state().params.clone();
    let revision = h.state().parameter_revision;
    h.get_by_label("搜索控件").focus();
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::Ime(egui::ImeEvent::Enabled));
    h.input_mut()
        .events
        .push(egui::Event::Ime(egui::ImeEvent::Preedit("jiaopian".into())));
    h.step();
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().parameter_revision, revision);
    h.input_mut()
        .events
        .push(egui::Event::Ime(egui::ImeEvent::Commit("胶片规格".into())));
    h.input_mut()
        .events
        .push(egui::Event::Ime(egui::ImeEvent::Disabled));
    h.run_steps(3);
    assert_eq!(h.get_by_label("搜索控件").value(), Some("胶片规格".into()));
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "胶片规格")
            .is_some()
    );
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().parameter_revision, revision);
}

#[test]
fn export_draft_survives_language_switch_and_cancel_without_committing_preferences() {
    let mut s = session();
    let h = &mut s.harness;
    open_export(h);
    let committed = h.state().export_options.clone();
    choose(h, "Format", "TIFF");
    choose(h, "Saving color space", "ACES2065-1");
    let params = h.state().params.clone();
    language(h, LanguagePreference::SimplifiedChinese);
    assert!(h.state().export_dialog.is_open());
    assert_eq!(h.state().export_options, committed);
    assert_eq!(h.state().params, params);
    language(h, LanguagePreference::English);
    assert!(h.query_all_by_label_contains("TIFF").next().is_some());
    assert!(h.query_all_by_label_contains("ACES2065-1").next().is_some());
    click(h, "Cancel");
    assert_eq!(h.state().export_options, committed);
    assert!(h.state().export_job.is_none());
}

#[test]
fn language_switch_preserves_active_render_identity_and_accepts_its_result() {
    let mut s = session();
    let h = &mut s.harness;
    preview(h);
    let active = h.state().render_worker.active.clone();
    let guard = active.lock();
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "exposure compensation ev", "0.5");
    wait_until(h, |a| a.render_job.is_some());
    let revision = h.state().parameter_revision;
    let epoch = h.state().input_epoch;
    let request = h.state().render_job.as_ref().unwrap().request_id;
    assert!(h.state().render_job.is_some());
    language(h, LanguagePreference::SimplifiedChinese);
    assert!(h.state().render_job.is_some());
    assert_eq!(h.state().render_job.as_ref().unwrap().request_id, request);
    assert!(h.query_by_label("渲染中……").is_some());
    assert_eq!(h.state().parameter_revision, revision);
    assert_eq!(h.state().input_epoch, epoch);
    drop(guard);
    wait_until(h, |app| {
        app.render_job.is_none() && app.output_image.is_some()
    });
    assert_eq!(h.state().parameter_revision, revision);
    assert!(
        h.state()
            .output_image
            .as_ref()
            .unwrap()
            .data
            .iter()
            .all(|v| v.is_finite())
    );
}

#[test]
fn chinese_header_actions_and_search_remain_reachable_at_supported_desktop_sizes() {
    for size in [egui::vec2(1460.0, 980.0), egui::vec2(1100.0, 800.0)] {
        let directory = tempfile::tempdir().unwrap();
        let (image_path, state_path) = deterministic_inputs(directory.path());
        let browser = directory.path().join("browser.json");
        let mut h = Harness::builder().with_size(size).build_eframe(move |cc| {
            theme::apply(&cc.egui_ctx);
            app(state_path, image_path, browser)
        });
        if h.query_by_label("◀").is_some() {
            click(&mut h, "◀");
        }
        language(&mut h, LanguagePreference::SimplifiedChinese);
        for label in ["打开", "设置", "导出", "搜索控件", "扫描", "保存"] {
            let mut nodes = h.query_all_by_label(label).peekable();
            assert!(nodes.peek().is_some(), "missing {label}");
            for node in nodes {
                let bounds = node.node().raw_bounds().unwrap();
                assert!(
                    bounds.x0 >= 0.0
                        && bounds.y0 >= 0.0
                        && bounds.x1 <= size.x as f64
                        && bounds.y1 <= size.y as f64,
                    "{label} unreachable at {size:?}: {bounds:?}"
                );
            }
        }
        let viewer = h.state().viewer.observation().viewport;
        assert!(viewer.width() > 0.0 && viewer.height() > 0.0);
    }
}

#[cfg(feature = "gui-snapshots")]
#[test]
fn chinese_workspace_and_settings_render_with_bundled_font() {
    let mut s = snapshot_session();
    let h = &mut s.harness;
    language(h, LanguagePreference::SimplifiedChinese);
    h.run_steps(5);
    let image = h.render().expect("Chinese offscreen workspace");
    save_snapshot(&image, "zh-CN-workspace.png");
    h.get_by_label("设置").simulate_click();
    h.run_steps(5);
    let image = h.render().expect("Chinese offscreen Settings");
    save_snapshot(&image, "zh-CN-settings.png");
    h.get_by_label("关闭设置").simulate_click();
    h.ctx.set_pixels_per_point(1.5);
    h.run_steps(5);
    let image = h.render().expect("Chinese high-DPI workspace");
    save_snapshot(&image, "zh-CN-workspace-150pct.png");
}

#[test]
fn real_language_selector_retains_invalid_and_deferred_numeric_drafts() {
    for draft in ["-", "0"] {
        let mut s = session();
        let h = &mut s.harness;
        reveal_control(h, "camera.exposure_compensation_ev");
        let params = h.state().params.clone();
        let revision = h.state().parameter_revision;
        h.get_by_label("exposure compensation ev").focus();
        h.step();
        h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.input_mut().events.push(egui::Event::Text(draft.into()));
        h.step();
        let bounds = h.get_by_label("Settings").node().raw_bounds().unwrap();
        let pos = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        h.input_mut().events.push(egui::Event::PointerMoved(pos));
        h.input_mut().events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        h.input_mut().events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        h.run_steps(3);
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "语言 / Language")
            .click();
        h.run_steps(3);
        h.get_by_label("简体中文").click();
        h.run_steps(3);
        assert_eq!(crate::i18n::locale(&h.ctx), Locale::SimplifiedChinese);
        assert_eq!(h.state().params, params);
        assert_eq!(h.state().parameter_revision, revision);
        h.get_by_label("关闭设置").click();
        h.run_steps(3);
        h.get_by_label("清除搜索").click();
        h.run_steps(3);
        h.get_by_label("自动曝光").click();
        h.run_steps(3);
        let mut independent = params.clone();
        independent.camera.auto_exposure = !independent.camera.auto_exposure;
        // The suspension must not monopolize another numeric field's wheel.
        h.get_by_label("画面长边").hover();
        h.step();
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 1.0),
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(3);
        assert_ne!(
            h.state().params.camera.film_format_mm,
            params.camera.film_format_mm
        );
        independent.camera.film_format_mm = h.state().params.camera.film_format_mm;
        let field = h
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .find(|node| node.node().value() == Some(draft.into()))
            .expect("original numeric draft remains after actual selector navigation");
        field.focus();
        h.step();
        h.press_key(egui::Key::Escape);
        h.run_steps(3);
        assert_eq!(
            h.state().params.camera.exposure_compensation_ev,
            params.camera.exposure_compensation_ev
        );
        assert_eq!(
            h.state().params.camera.auto_exposure,
            independent.camera.auto_exposure
        );
        assert_eq!(
            h.state().params.camera.film_format_mm,
            independent.camera.film_format_mm
        );
        h.get_by_label("撤销").click();
        h.run_steps(3);
        assert_eq!(
            h.state().params.camera.film_format_mm,
            params.camera.film_format_mm
        );
        assert_eq!(
            h.state().params.camera.auto_exposure,
            independent.camera.auto_exposure
        );
    }
}

#[test]
fn language_switch_preserves_staged_export_job_and_published_pixels() {
    let mut s = session();
    let h = &mut s.harness;
    let output = scan(h);
    let destination = s.directory.path().join("切换语言时导出.png");
    let pause = super::dialogs::stage_export(h, &destination);
    let cancel = h.state().export_job.as_ref().unwrap().cancel.clone();
    let params = h.state().params.clone();
    let revision = h.state().parameter_revision;
    language(h, LanguagePreference::SimplifiedChinese);
    assert!(Arc::ptr_eq(
        &h.state().export_job.as_ref().unwrap().cancel,
        &cancel
    ));
    assert!(!cancel.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().parameter_revision, revision);
    assert!(h.state().status.localized(&h.ctx).contains("导出"));
    pause.release();
    wait_until(h, |app| app.export_job.is_none());
    let expected = image_io::convert_image(
        &output,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&destination, &expected);
}

#[test]
fn language_selector_does_not_finalize_valid_pending_numeric_history() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "camera.exposure_compensation_ev");
    let params = h.state().params.clone();
    assert!(!h.state().can_undo());
    h.get_by_label("exposure compensation ev").focus();
    h.step();
    h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.input_mut().events.push(egui::Event::Text("1".into()));
    h.step();
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 1.0);
    assert!(!h.state().can_undo(), "uncommitted numeric gesture");
    let revision = h.state().parameter_revision;
    h.get_by_label("Settings").click();
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "语言 / Language")
        .click();
    h.run_steps(3);
    h.get_by_label("简体中文").click();
    h.run_steps(3);
    assert!(
        !h.state().can_undo(),
        "language navigation must not finalize history"
    );
    assert_eq!(h.state().parameter_revision, revision);
    h.get_by_label("关闭设置").click();
    h.run_steps(3);
    h.get_by_label("曝光补偿 EV").focus();
    h.step();
    h.press_key(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().params, params);
    assert!(
        !h.state().can_undo(),
        "Escape cancels original gesture, not a second adjustment"
    );
}

#[test]
fn raw_settings_draft_is_not_reprocessed_or_lost_by_language_switch() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "extras.load_raw.white_balance");
    choose(h, "white balance", "custom");
    reveal_control(h, "extras.load_raw.temperature");
    edit(h, "temperature", "5200");
    let draft = h.state().gui_state.sections["load_raw"].clone();
    let source = h.state().image.clone().unwrap();
    let epoch = h.state().input_epoch;
    let revision = h.state().parameter_revision;
    let history = h.state().can_undo();
    language(h, LanguagePreference::SimplifiedChinese);
    assert_eq!(h.state().gui_state.sections["load_raw"], draft);
    assert!(h.state().image.as_ref().unwrap().same_source(&source));
    assert_eq!(h.state().input_epoch, epoch);
    assert_eq!(h.state().parameter_revision, revision);
    assert_eq!(h.state().can_undo(), history);
    language(h, LanguagePreference::English);
    assert_eq!(
        h.get_by_label("temperature")
            .value()
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        5200.0
    );
}

#[test]
fn chinese_save_rejection_translates_owned_cause_and_recovery_without_hiding_extension() {
    let mut s = session();
    let h = &mut s.harness;
    preview(h);
    language(h, LanguagePreference::SimplifiedChinese);
    let destination = s.directory.path().join("拒绝.xyz");
    save_retained(h, &destination);
    let explanation = h.state().status.localized(&h.ctx);
    assert!(explanation.contains("xyz"), "{explanation}");
    assert!(
        explanation.contains("JPEG")
            && explanation.contains("PNG")
            && explanation.contains("TIFF")
            && explanation.contains("EXR"),
        "{explanation}"
    );
    assert!(
        !explanation.contains("Unsupported") && !explanation.contains("use JPEG"),
        "{explanation}"
    );
    assert!(!destination.exists());
}

#[test]
fn preset_name_composition_does_not_submit_or_translate_authored_text() {
    let mut s = session();
    let h = &mut s.harness;
    language(h, LanguagePreference::SimplifiedChinese);
    h.get_by_label("▶").click();
    h.run_steps(3);
    h.get_by_label("保存预设…").click();
    h.run_steps(3);
    let params = h.state().params.clone();
    h.get_by_label("“我的预设”中的名称").focus();
    h.step();
    h.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.input_mut()
        .events
        .push(egui::Event::Ime(egui::ImeEvent::Enabled));
    h.input_mut()
        .events
        .push(egui::Event::Ime(egui::ImeEvent::Preedit("zhongwen".into())));
    h.step();
    h.press_key(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        h.state().presets.name_dialog.is_some(),
        "composition Enter must not submit"
    );
    h.input_mut()
        .events
        .push(egui::Event::Ime(egui::ImeEvent::Commit("中文预设".into())));
    h.input_mut()
        .events
        .push(egui::Event::Ime(egui::ImeEvent::Disabled));
    h.run_steps(3);
    assert_eq!(
        h.get_by_label("“我的预设”中的名称").value(),
        Some("中文预设".into())
    );
    language(h, LanguagePreference::English);
    assert_eq!(
        h.get_by_label("Name in My Presets").value(),
        Some("中文预设".into())
    );
    h.get_by_label("Save").click();
    h.run_steps(3);
    let path = h.state().config_directory.join("presets/中文预设.toml");
    let saved =
        spektrafilm_core::presets::Preset::from_toml_str(&std::fs::read_to_string(path).unwrap())
            .unwrap();
    assert_eq!(saved.name, "中文预设");
    assert_eq!(h.state().params, params);
}

#[test]
fn failed_preference_publication_keeps_selected_language_and_can_be_retried() {
    let mut s = session();
    let h = &mut s.harness;
    let params = h.state().params.clone();
    let revision = h.state().parameter_revision;
    let path = crate::i18n::preference_path(&h.state().config_directory);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let ctx = h.ctx.clone();
    assert!(
        h.state_mut()
            .select_language(&ctx, LanguagePreference::SimplifiedChinese)
            .is_err()
    );
    h.run_steps(3);
    assert_eq!(crate::i18n::locale(&ctx), Locale::SimplifiedChinese);
    assert!(h.query_by_label("打开").is_some());
    let error = h
        .state()
        .language_preference_error
        .as_ref()
        .unwrap()
        .localized(&ctx);
    assert!(error.contains(path.to_str().unwrap()), "{error}");
    assert!(error.contains("本次运行"), "{error}");
    assert_eq!(h.state().params, params);
    assert_eq!(h.state().parameter_revision, revision);
    std::fs::remove_dir(&path).unwrap();
    language(h, LanguagePreference::SimplifiedChinese);
    assert!(h.state().language_preference_error.is_none());
    assert_eq!(
        crate::i18n::load_preference(&h.state().config_directory).0,
        LanguagePreference::SimplifiedChinese
    );
}

#[test]
fn invalid_calibration_state_keeps_accepted_photo_and_localizes_owned_validation() {
    let mut s = session();
    let h = &mut s.harness;
    let params = h.state().params.clone();
    let source = h.state().image.clone().unwrap();
    let revision = h.state().parameter_revision;
    language(h, LanguagePreference::SimplifiedChinese);
    for calibration in ["1 0 0", "1 0 0 0 NaN 0 0 0 1"] {
        let mut candidate = h.state().current_state().unwrap().sections;
        candidate["convert"]["calibration"] = calibration.into();
        let path = s.directory.path().join("校准矩阵无效.json");
        std::fs::write(&path, serde_json::to_vec(&candidate).unwrap()).unwrap();
        let error = state::GuiState::load(&path).unwrap_err();
        let message = crate::app::Message::error(&error).localized(&h.ctx);
        assert!(
            message.contains("film_render.convert.calibration"),
            "{message}"
        );
        assert!(
            message.contains("9") && message.contains("有限"),
            "{message}"
        );
        assert!(!message.contains("must contain"), "{message}");
        assert_eq!(h.state().params, params);
        assert_eq!(h.state().parameter_revision, revision);
        assert!(h.state().image.as_ref().unwrap().same_source(&source));
    }
}
