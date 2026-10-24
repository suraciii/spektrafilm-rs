//! Work-list controls cross the same unsaved-work boundary as single-photo Open.
use super::*;

#[test]
fn work_list_navigation_cancel_discard_and_remove_use_production_controls() {
    let mut session = session();
    let second = session.directory.path().join("second.png");
    let first = session.harness.state().image_path.clone().unwrap();
    std::fs::copy(&first, &second).unwrap();
    let h = &mut session.harness;
    h.state_mut().photograph.work_list.open = true;
    h.run_steps(2);
    script(h, chooser::ChooserResult::Scripted(second.clone()));
    click(h, "Add photographs…");
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "exposure compensation ev", "1.25");
    h.get_by_label("Search controls").focus();
    h.run_steps(3);
    click(h, "Next photograph");
    h.press_key(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().image_path.as_ref(), Some(&first));
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 1.25);
    click(h, "Next photograph");
    let discard =
        crate::i18n::text_for(crate::i18n::Locale::English, "photo.unsaved.discard").into_owned();
    click(h, &discard);
    assert_eq!(h.state().image_path.as_ref(), Some(&second));
    click(h, "Previous photograph");
    assert_eq!(h.state().image_path.as_ref(), Some(&first));
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 0.0);
    assert!(first.is_file() && second.is_file());
}

fn unsaved_label(key: &str) -> String {
    crate::i18n::text_for(crate::i18n::Locale::English, key).into_owned()
}

fn modal_action(h: &mut Harness<'_, App>, key: &str) {
    click(h, &unsaved_label(key));
}

fn assert_guard_source(h: &Harness<'_, App>, source: &Path) {
    assert!(
        h.query_by_label(&unsaved_label("photo.unsaved.title"))
            .is_some()
    );
    assert!(
        h.query_by_label(&source.display().to_string()).is_some(),
        "guard must identify the affected photograph"
    );
}

fn copied_inactive_session() -> (Session, PathBuf, PathBuf) {
    let mut s = session();
    let first = s.harness.state().image_path.clone().unwrap();
    let second = s.directory.path().join("second.png");
    std::fs::copy(&first, &second).unwrap();
    let record = s.directory.path().join("source.spektra-photo.json");
    let h = &mut s.harness;
    h.state_mut().add_work_paths(vec![second.clone()]);
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "exposure compensation ev", "1.25");
    h.get_by_label("Search controls").focus();
    h.run_steps(3);
    h.state_mut().select_work_destination(1, true);
    h.state_mut()
        .copy_work_adjustments(crate::photograph::worklist::CopyScope::Section(
            spektrafilm_core::presets::Section::Camera,
        ))
        .unwrap();
    await_input(h);
    h.state_mut().save_photograph_record_to(&record).unwrap();
    assert!(!h.state().photograph_has_unsaved_changes());
    (s, first, second)
}

#[test]
fn inactive_copy_removal_cancel_and_save_chooser_cancel_preserve_work_then_save_removes_membership_only()
 {
    let (mut s, first, second) = copied_inactive_session();
    let destination = s.directory.path().join("second.spektra-photo.json");
    let h = &mut s.harness;
    let pixels = h.state().image.clone().unwrap();
    h.state_mut()
        .request_photo_action(crate::photograph::PhotoAction::RemoveEntry(1));
    h.run_steps(2);
    assert_guard_source(h, &second);
    modal_action(h, "photo.unsaved.cancel");
    assert!(h.state().work_entry_path(1).is_some());
    assert_eq!(
        h.state()
            .affected_unsaved_work(&crate::photograph::PhotoAction::Close),
        Some(1)
    );
    h.state_mut()
        .request_photo_action(crate::photograph::PhotoAction::RemoveEntry(1));
    h.run_steps(2);
    script(h, chooser::ChooserResult::Canceled);
    modal_action(h, "photo.unsaved.save");
    assert_guard_source(h, &second);
    assert!(h.state().work_entry_path(1).is_some());
    script(h, chooser::ChooserResult::Scripted(destination.clone()));
    modal_action(h, "photo.unsaved.save");
    assert!(
        h.query_by_label(&unsaved_label("photo.unsaved.title"))
            .is_none()
    );
    assert!(h.state().work_entry_path(1).is_none());
    assert_eq!(h.state().image_path.as_ref(), Some(&first));
    assert!(h.state().image.as_ref().unwrap().same_source(&pixels));
    let record = crate::photograph::record::PhotographRecord::load(&destination).unwrap();
    assert_eq!(record.settings.params.camera.exposure_compensation_ev, 1.25);
    assert_eq!(record.source.path, second.canonicalize().unwrap());
    assert!(first.is_file() && second.is_file());
}

#[test]
fn inactive_copy_removal_discard_uses_real_modal_without_changing_active_adjustments() {
    let (mut s, first, second) = copied_inactive_session();
    let h = &mut s.harness;
    let before = h.state().capture_photograph_settings().unwrap();
    h.state_mut()
        .request_photo_action(crate::photograph::PhotoAction::RemoveEntry(1));
    h.run_steps(2);
    assert_guard_source(h, &second);
    modal_action(h, "photo.unsaved.discard");
    assert!(
        h.query_by_label(&unsaved_label("photo.unsaved.title"))
            .is_none()
    );
    assert!(h.state().work_entry_path(1).is_none());
    assert_eq!(h.state().capture_photograph_settings().unwrap(), before);
    assert_eq!(h.state().image_path.as_ref(), Some(&first));
    assert!(first.is_file() && second.is_file());
}

#[test]
fn close_guards_each_inactive_copy_cancel_failed_save_save_and_discard_through_modal() {
    let (mut s, first, second) = copied_inactive_session();
    let third = s.directory.path().join("third.png");
    std::fs::copy(&first, &third).unwrap();
    let invalid_record = s.directory.path().join("missing/second.json");
    let destination = s.directory.path().join("second.spektra-photo.json");
    let h = &mut s.harness;
    h.state_mut().add_work_paths(vec![third.clone()]);
    h.state_mut().select_work_destination(1, false);
    h.state_mut().select_work_destination(2, true);
    // The third entry starts with current settings; change source again so the
    // copied destination is genuinely dirty, then save only the active source.
    reveal_control(h, "camera.exposure_compensation_ev");
    edit(h, "exposure compensation ev", "1.5");
    h.get_by_label("Search controls").focus();
    h.run_steps(3);
    h.state_mut()
        .copy_work_adjustments(crate::photograph::worklist::CopyScope::Section(
            spektrafilm_core::presets::Section::Camera,
        ))
        .unwrap();
    await_input(h);
    let active_record = h.state().photograph.record_path.clone().unwrap();
    h.state_mut()
        .save_photograph_record_to(&active_record)
        .unwrap();
    h.state_mut()
        .request_photo_action(crate::photograph::PhotoAction::Close);
    h.run_steps(2);
    assert_guard_source(h, &second);
    modal_action(h, "photo.unsaved.cancel");
    assert_eq!(
        h.state()
            .affected_unsaved_work(&crate::photograph::PhotoAction::Close),
        Some(1)
    );
    h.state_mut()
        .request_photo_action(crate::photograph::PhotoAction::Close);
    h.run_steps(2);
    script(h, chooser::ChooserResult::Scripted(invalid_record));
    modal_action(h, "photo.unsaved.save");
    assert_guard_source(h, &second);
    assert_eq!(
        h.state()
            .affected_unsaved_work(&crate::photograph::PhotoAction::Close),
        Some(1)
    );
    script(h, chooser::ChooserResult::Scripted(destination.clone()));
    modal_action(h, "photo.unsaved.save");
    assert_guard_source(h, &third);
    assert_eq!(
        h.state()
            .affected_unsaved_work(&crate::photograph::PhotoAction::Close),
        Some(2)
    );
    modal_action(h, "photo.unsaved.discard");
    assert_eq!(
        h.state()
            .affected_unsaved_work(&crate::photograph::PhotoAction::Close),
        None
    );
    assert!(
        h.query_by_label(&unsaved_label("photo.unsaved.title"))
            .is_none()
    );
    assert!(destination.is_file());
    assert!(first.is_file() && second.is_file() && third.is_file());
    assert_eq!(h.state().params.camera.exposure_compensation_ev, 1.5);
}

#[cfg(feature = "gui-raw")]
#[test]
fn pending_raw_drafts_return_per_entry_without_becoming_accepted_record_settings() {
    let raw = super::raw::raw_fixture();
    let mut s = session();
    let rgb = s.harness.state().image_path.clone().unwrap();
    let second_raw = s.directory.path().join("second-raw.KDC");
    std::fs::copy(&raw, &second_raw).unwrap();
    let record_path = s.directory.path().join("accepted-raw.spektra-photo.json");
    let h = &mut s.harness;
    h.state_mut()
        .add_work_paths(vec![raw.clone(), second_raw.clone()]);
    h.state_mut()
        .request_photo_action(crate::photograph::PhotoAction::SelectEntry(1));
    await_input(h);
    h.run_steps(2);
    assert_eq!(
        h.state().image_path.as_ref(),
        Some(&raw.canonicalize().unwrap())
    );
    let accepted = h.state().capture_photograph_settings().unwrap();
    let pixels = h.state().image.clone().unwrap();
    reveal_control(h, "extras.load_raw.white_balance");
    choose(h, "white balance", "custom");
    reveal_control(h, "extras.load_raw.temperature");
    edit(h, "temperature", "4200");
    h.get_by_label("Search controls").focus();
    h.run_steps(3);
    let draft = h.state().gui_state.sections["load_raw"].clone();
    assert_eq!(
        h.state().capture_photograph_settings().unwrap().raw,
        accepted.raw
    );
    assert!(!h.state().photograph_has_unsaved_changes());
    h.state_mut()
        .save_photograph_record_to(&record_path)
        .unwrap();
    let record = crate::photograph::record::PhotographRecord::load(&record_path).unwrap();
    assert_eq!(record.settings.raw, accepted.raw);
    assert!(h.state().image.as_ref().unwrap().same_source(&pixels));
    h.state_mut()
        .request_photo_action(crate::photograph::PhotoAction::SelectEntry(2));
    await_input(h);
    h.run_steps(2);
    assert_eq!(
        h.state().capture_photograph_settings().unwrap().raw,
        accepted.raw
    );
    assert_ne!(h.state().gui_state.sections["load_raw"], draft);
    reveal_control(h, "extras.load_raw.white_balance");
    choose(h, "white balance", "tungsten");
    h.get_by_label("Search controls").focus();
    h.run_steps(3);
    let second_draft = h.state().gui_state.sections["load_raw"].clone();
    h.state_mut()
        .request_photo_action(crate::photograph::PhotoAction::SelectEntry(1));
    await_input(h);
    h.run_steps(2);
    assert_eq!(h.state().gui_state.sections["load_raw"], draft);
    assert_eq!(
        h.state().capture_photograph_settings().unwrap().raw,
        accepted.raw
    );
    assert!(h.state().image.as_ref().unwrap().same_source(&pixels));
    h.state_mut()
        .request_photo_action(crate::photograph::PhotoAction::SelectEntry(2));
    await_input(h);
    h.run_steps(2);
    assert_eq!(h.state().gui_state.sections["load_raw"], second_draft);
    assert_eq!(
        h.state().capture_photograph_settings().unwrap().raw,
        accepted.raw
    );
    h.state_mut()
        .request_photo_action(crate::photograph::PhotoAction::SelectEntry(0));
    await_input(h);
    h.run_steps(2);
    assert_eq!(h.state().image_path.as_ref(), Some(&rgb));
    assert_ne!(h.state().gui_state.sections["load_raw"], draft);
    assert_ne!(h.state().gui_state.sections["load_raw"], second_draft);
}

#[test]
fn raw_options_draft_carrier_is_restored_independently_for_each_work_context() {
    let mut s = session();
    let first = s.harness.state().image_path.clone().unwrap();
    let second = s.directory.path().join("second.png");
    std::fs::copy(&first, &second).unwrap();
    let h = &mut s.harness;
    h.state_mut().add_work_paths(vec![second.clone()]);
    reveal_control(h, "extras.load_raw.white_balance");
    choose(h, "white balance", "custom");
    reveal_control(h, "extras.load_raw.temperature");
    edit(h, "temperature", "4200");
    h.get_by_label("Search controls").focus();
    h.run_steps(3);
    let first_draft = h.state().gui_state.sections["load_raw"].clone();
    // Direct committed navigation isolates context retention from the RAW
    // decoder (the real RAW guard/accepted-decode path is exercised above).
    h.state_mut().perform_work_select(1).unwrap();
    await_input(h);
    h.run_steps(2);
    assert_ne!(h.state().gui_state.sections["load_raw"], first_draft);
    reveal_control(h, "extras.load_raw.white_balance");
    choose(h, "white balance", "tungsten");
    h.get_by_label("Search controls").focus();
    h.run_steps(3);
    let second_draft = h.state().gui_state.sections["load_raw"].clone();
    h.state_mut().perform_work_select(0).unwrap();
    await_input(h);
    h.run_steps(2);
    assert_eq!(h.state().image_path.as_ref(), Some(&first));
    assert_eq!(h.state().gui_state.sections["load_raw"], first_draft);
    h.state_mut().perform_work_select(1).unwrap();
    await_input(h);
    h.run_steps(2);
    assert_eq!(h.state().image_path.as_ref(), Some(&second));
    assert_eq!(h.state().gui_state.sections["load_raw"], second_draft);
}
