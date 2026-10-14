use super::*;

// experimental_gui.py:402–428: every route, canonical state, preview/scan/save.
#[test]
fn every_supported_route_preview_scan_state_roundtrip_and_save() {
    let mut s = session();
    let h = &mut s.harness;
    for (index, (route, label)) in routes::entries().enumerate() {
        click(h, "MAIN");
        choose(h, "workflow", label);
        assert_eq!(h.state().params.workflow.route, route);
        let preview = render(h, "PREVIEW");
        assert_eq!((preview.width, preview.height), (128, 64), "{route}");
        let scan = render(h, "SCAN");
        assert_eq!((scan.width, scan.height), (160, 80), "{route}");
        let saved = s.directory.path().join(format!("route-{index}.json"));
        click(h, "CONFIG");
        if h.query_by_label("Save current to file").is_none() {
            click(h, "GUI parameters");
        }
        script(h, chooser::ChooserResult::Scripted(saved.clone()));
        click(h, "Save current to file");
        assert!(
            h.state().status.starts_with("Saved GUI state"),
            "{}",
            h.state().status
        );
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&saved).unwrap()).unwrap();
        assert_eq!(value["simulation"]["route"], route);
        assert!(value["simulation"].get("scan_film").is_none());
        assert!(value["simulation"].get("workflow").is_none());
        assert!(value["rust"]["runtime"]["workflow"].get("route").is_none());
        assert!(value["rust"]["runtime"]["io"].get("scan_film").is_none());
        script(h, chooser::ChooserResult::Scripted(saved));
        click(h, "Load from file");
        assert!(
            h.state().status.starts_with("Loaded GUI state"),
            "{}",
            h.state().status
        );
        assert_eq!(h.state().params.workflow.route, route);
        click(h, "MAIN");
        let reloaded = render(h, "SCAN");
        assert_eq!(reloaded.data, scan.data, "state reload changed {route}");
        let destination = s.directory.path().join(format!("route-{index}.png"));
        script(h, chooser::ChooserResult::Scripted(destination.clone()));
        click(h, "SAVE");
        assert!(
            h.state().status.starts_with("Saved"),
            "{}",
            h.state().status
        );
        let expected = image_io::convert_image(
            &reloaded,
            &h.state().output_color_space,
            h.state().output_cctf_encoding,
            "sRGB",
            true,
        )
        .unwrap();
        assert_png(&destination, &expected);
    }
}

// gui_acceptance.py:1743–1788: forced correction, exact restoration, loaded toggle.
#[test]
fn scan_for_print_checkbox_restores_and_clears_transient_snapshot() {
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "input > film > print > scan");
    let before = (
        h.state().params.scanner.white_correction,
        h.state().params.scanner.black_correction,
        h.state().params.print_render.glare.active,
    );
    click(h, "white and black correction");
    assert!(h.state().params.scanner.white_correction);
    assert!(h.state().params.scanner.black_correction);
    assert!(!h.state().params.print_render.glare.active);
    click(h, "CONFIG");
    click(h, "GUI parameters");
    let path = s.directory.path().join("correction.json");
    script(h, chooser::ChooserResult::Scripted(path.clone()));
    click(h, "Save current to file");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(saved.get("scan_for_print").is_none());
    assert!(saved["simulation"].get("scan_for_print").is_none());
    assert!(saved["rust"].get("scan_for_print_snapshot").is_none());
    assert!(saved["simulation"].get("workflow").is_none());
    assert!(saved["rust"]["runtime"]["workflow"].get("route").is_none());
    click(h, "white and black correction");
    assert_eq!(
        (
            h.state().params.scanner.white_correction,
            h.state().params.scanner.black_correction,
            h.state().params.print_render.glare.active
        ),
        before
    );
    click(h, "white and black correction");
    script(h, chooser::ChooserResult::Scripted(path));
    click(h, "Load from file");
    assert!(h.state().scan_for_print_snapshot.is_none());
    assert!(h.state().params.scanner.white_correction);
    assert!(h.state().params.scanner.black_correction);
    assert!(!h.state().params.print_render.glare.active);
    click(h, "white and black correction");
    choose(h, "workflow", "Finished RGB");
    assert!(h.state().scan_for_print_snapshot.is_none());
    assert!(h.state().params.scanner.white_correction);
    assert!(h.state().params.scanner.black_correction);
    assert!(!h.state().params.print_render.glare.active);
}
