use super::*;

#[test]
fn polymax_paper_selection_preview_scan_save_and_disabled_color_solver() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "workflow.route");
    choose(h, "Film route", "input > film > print > scan");
    reveal_control(h, "extras.simulation.film_stock");
    choose(h, "film profile", "Kodak Double-X Negative Film 5222");
    reveal_control(h, "extras.simulation.print_paper");
    choose(
        h,
        "print profile",
        "Kodak Polymax Fine-Art F / No filter (source curve)",
    );
    assert_eq!(h.state().print_name, "kodak_polymax_fine_art");
    assert!(h.state().print_dev_times.is_empty());
    let preview_image = preview(h);
    assert_eq!((preview_image.width, preview_image.height), (128, 64));
    let first = scan(h);
    assert!(first.data.iter().all(|v| v.is_finite()));
    for rgb in first.data.as_chunks::<3>().0.iter() {
        assert!((rgb[0] - rgb[1]).abs() < 0.002);
        assert!((rgb[1] - rgb[2]).abs() < 0.002);
    }
    for route in ["input > film > scan", "input > film > print > scan"] {
        reveal_control(h, "workflow.route");
        choose(h, "Film route", route);
        // A material change invalidates the inspector cache even without printing.
        h.state_mut()
            .select_print_profile("kodak_portra_endura".into());
        reveal_control(h, "action.neutralize_filters");
        assert!(
            !h.get_by_label("neutralize print filters")
                .node()
                .is_disabled()
        );
        h.state_mut()
            .select_print_profile("kodak_polymax_fine_art".into());
        reveal_control(h, "action.neutralize_filters");
        assert!(
            h.get_by_label("neutralize print filters")
                .node()
                .is_disabled()
        );
    }
    let second = scan(h);
    assert_eq!(first.data, second.data);
    let state = s.directory.path().join("polymax.json");
    h.state().current_state().unwrap().save(&state).unwrap();
    let restored = state::GuiState::load(&state).unwrap();
    assert_eq!(restored.paper(), "kodak_polymax_fine_art");
    let destination = s.directory.path().join("polymax.png");
    script(h, chooser::ChooserResult::Scripted(destination.clone()));
    click(h, "SAVE");
    wait_until(h, |app| app.pending_saves.is_empty());
    let expected = image_io::convert_image(
        &second,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&destination, &expected);
    // Material-class changes must update action eligibility without rewriting
    // authored enlarger values. Effective automatic values belong to core prep.
    let authored = (
        h.state().params.enlarger.c_filter_neutral,
        h.state().params.enlarger.m_filter_neutral,
        h.state().params.enlarger.y_filter_neutral,
    );
    for (stock, disabled) in [
        ("kodak_2302", true),
        ("kodak_portra_endura", false),
        ("kodak_polymax_fine_art", true),
    ] {
        h.state_mut().select_print_profile(stock.into());
        reveal_control(h, "action.neutralize_filters");
        assert_eq!(
            h.get_by_label("neutralize print filters")
                .node()
                .is_disabled(),
            disabled
        );
        assert_eq!(
            (
                h.state().params.enlarger.c_filter_neutral,
                h.state().params.enlarger.m_filter_neutral,
                h.state().params.enlarger.y_filter_neutral
            ),
            authored
        );
    }
}
