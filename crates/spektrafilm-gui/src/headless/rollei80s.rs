use super::*;

#[test]
fn rollei80s_builtin_preview_scan_state_and_save() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "workflow.route");
    choose(h, "Film route", "input > film > print > scan");
    h.state_mut().load_preset(
        spektrafilm_core::presets::PresetGroup::BuiltIn,
        "Rollei Retro 80S 35mm (Neofin Blau)",
    );
    h.run_steps(3);
    assert_eq!(h.state().film_name, "rollei_retro_80s_35mm_neofin_blau");
    assert_eq!(h.state().print_name, "kodak_polymax_fine_art");
    assert_eq!(h.state().film_dev_times, [6.0]);
    assert!(h.state().params.film_render.grain.active);
    assert!(h.state().params.film_render.halation.active);
    assert_eq!(
        h.state().params.film_render.grain.engine,
        spektrafilm_core::params::GrainEngine::Spektra
    );
    assert_eq!(
        h.state().params.film_render.grain.rms_granularity,
        [14.0; 3]
    );
    let pre = preview(h);
    assert_eq!((pre.width, pre.height), (128, 64));
    let scanned = scan(h);
    assert!(scanned.data.iter().all(|v| v.is_finite()));
    assert!(scanned.data.iter().any(|v| *v > 0.1));
    for rgb in scanned.data.as_chunks::<3>().0 {
        assert!((rgb[0] - rgb[1]).abs() < 0.01 && (rgb[1] - rgb[2]).abs() < 0.01);
    }
    let saved = s.directory.path().join("rollei80s-state.json");
    h.state().current_state().unwrap().save(&saved).unwrap();
    let restored = state::GuiState::load(&saved).unwrap();
    assert_eq!(restored.film(), "rollei_retro_80s_35mm_neofin_blau");
    assert_eq!(restored.paper(), "kodak_polymax_fine_art");
    let destination = s.directory.path().join("rollei80s.png");
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
