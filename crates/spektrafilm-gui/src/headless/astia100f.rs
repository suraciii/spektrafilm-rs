use super::*;

#[test]
fn astia100f_builtin_preview_scan_state_and_save() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input > film > print > scan");
    // Use the same application transaction as card activation, then the real
    // Preview/Scan/Save controls and worker path.
    h.state_mut().load_preset(
        spektrafilm_core::presets::PresetGroup::BuiltIn,
        "Fujifilm Astia 100F",
    );
    h.run_steps(3);
    assert_eq!(h.state().film_name, "fujifilm_astia_100f");
    assert_eq!(h.state().print_name, "fujifilm_crystal_archive_typeii");
    assert_eq!(h.state().params.workflow.route, "input > film > scan");
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
    assert!(h.state().params.film_render.grain.active);
    assert!(h.state().params.film_render.halation.active);
    let pre = preview(h);
    assert_eq!((pre.width, pre.height), (128, 64));
    let scanned = scan(h);
    assert!(scanned.data.iter().all(|v| v.is_finite()));
    assert!(scanned.data.iter().any(|v| *v > 0.1));
    let saved = s.directory.path().join("astia-state.json");
    h.state().current_state().unwrap().save(&saved).unwrap();
    let restored = state::GuiState::load(&saved).unwrap();
    assert_eq!(restored.film(), "fujifilm_astia_100f");
    assert_eq!(restored.paper(), "fujifilm_crystal_archive_typeii");
    let destination = s.directory.path().join("astia.png");
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
