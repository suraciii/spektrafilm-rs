use super::*;

#[test]
fn trix400_builtin_preview_scan_state_and_save() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input > film > print > scan");
    // Use the same application transaction as card activation, then the real
    // Preview/Scan/Save controls and worker path.
    h.state_mut().load_preset(
        spektrafilm_core::presets::PresetGroup::BuiltIn,
        "Kodak Tri-X 400",
    );
    h.run_steps(3);
    assert_eq!(h.state().film_name, "kodak_trix_400");
    assert_eq!(h.state().print_name, "kodak_polymax_fine_art");
    assert_eq!(h.state().film_dev_times, [6.]);
    assert!(!h.state().params.film_render.grain.active);
    let pre = preview(h);
    assert_eq!((pre.width, pre.height), (128, 64));
    let scanned = scan(h);
    assert!(scanned.data.iter().all(|v| v.is_finite()));
    for rgb in scanned.data.as_chunks::<3>().0 {
        assert!((rgb[0] - rgb[1]).abs() < 0.002 && (rgb[1] - rgb[2]).abs() < 0.002);
    }
    let saved = s.directory.path().join("trix-state.json");
    h.state().current_state().unwrap().save(&saved).unwrap();
    let restored = state::GuiState::load(&saved).unwrap();
    assert_eq!(restored.film(), "kodak_trix_400");
    assert_eq!(restored.paper(), "kodak_polymax_fine_art");
    let destination = s.directory.path().join("trix.png");
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
