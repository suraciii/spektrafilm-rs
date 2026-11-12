use super::*;

#[test]
fn vision3_50d_builtin_preview_scan_state_and_save() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "workflow.route");
    choose(h, "Film route", "input > film > print > scan");
    h.state_mut().load_preset(
        spektrafilm_core::presets::PresetGroup::BuiltIn,
        "Kodak Vision3 50D",
    );
    h.run_steps(3);
    assert_eq!(h.state().film_name, "kodak_vision3_50d");
    assert_eq!(h.state().print_name, "kodak_2383");
    assert!(h.state().params.film_render.grain.active);
    assert!(h.state().params.film_render.halation.active);
    let pre = preview(h);
    assert_eq!((pre.width, pre.height), (128, 64));
    let scanned = scan(h);
    assert!(scanned.data.iter().all(|v| v.is_finite()));
    assert!(scanned.data.iter().any(|v| *v > 0.1));
    let saved = s.directory.path().join("vision3-50d-state.json");
    h.state().current_state().unwrap().save(&saved).unwrap();
    let restored = state::GuiState::load(&saved).unwrap();
    assert_eq!(restored.film(), "kodak_vision3_50d");
    assert_eq!(restored.paper(), "kodak_2383");
    let destination = s.directory.path().join("vision3-50d.png");
    script(h, chooser::ChooserResult::Scripted(destination.clone()));
    click(h, "SAVE");
    wait_until(h, |app| app.pending_saves.is_empty());
    let expected = image_io::convert_image(
        &scanned,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&destination, &expected);
}
