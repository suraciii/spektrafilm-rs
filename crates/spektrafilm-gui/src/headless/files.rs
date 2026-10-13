use super::*;

fn state_controls(h: &mut Harness<'_, App>) {
    click(h, "CONFIG");
    if h.query_by_label("Save current to file").is_none() {
        click(h, "GUI parameters");
    }
}

// gui_acceptance.py:1635, 1939–1960, 2051–2069: production chooser actions.
#[test]
fn save_import_and_export_buttons_use_scripted_chooser_and_remember_directories() {
    let mut s = session();
    let h = &mut s.harness;
    let preview = render(h, "PREVIEW");
    let saved = s.directory.path().join("button-save.png");
    script(h, chooser::ChooserResult::Scripted(saved.clone()));
    click(h, "SAVE");
    let expected = image_io::convert_image(
        &preview,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&saved, &expected);
    let status = h.state().status.clone();
    let saved_bytes = std::fs::read(&saved).unwrap();
    script(h, chooser::ChooserResult::Canceled);
    click(h, "SAVE");
    assert_eq!(std::fs::read(&saved).unwrap(), saved_bytes);
    assert_eq!(h.state().status, status);
    let replacement = s.directory.path().join("replacement.png");
    let image = ImageBuf {
        width: 7,
        height: 3,
        data: vec![0.25; 7 * 3 * 3],
    };
    image_io::save(
        &replacement,
        &image,
        SaveOptions {
            depth: BitDepth::Eight,
            color_space: "sRGB",
            cctf_encoding: true,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
        },
        None,
    )
    .unwrap();
    click(h, "Import RGB");
    script(h, chooser::ChooserResult::Scripted(replacement.clone()));
    click(h, "select file");
    assert_eq!(h.state().image_path.as_ref(), Some(&replacement));
    assert_eq!(
        (
            h.state().image.as_ref().unwrap().width,
            h.state().image.as_ref().unwrap().height
        ),
        (7, 3)
    );
    let before = h.state().image.as_ref().unwrap().clone();
    let status = h.state().status.clone();
    script(h, chooser::ChooserResult::Canceled);
    click(h, "select file");
    assert!(Arc::ptr_eq(h.state().image.as_ref().unwrap(), &before));
    assert_eq!(h.state().status, status);
    let scan = render(h, "SCAN");
    click(h, "Export options");
    export_action(h);
    let exported = s.directory.path().join("button-export.png");
    script(h, chooser::ChooserResult::Scripted(exported.clone()));
    export_action(h);
    wait_until(h, |a| {
        a.export_job.is_none() && a.status.starts_with("Exported")
    });
    let expected = image_io::convert_image(
        &scan,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&exported, &expected);
    let dirs: serde_json::Value = serde_json::from_slice(
        &std::fs::read(s.directory.path().join("dialog_dirs.json")).unwrap(),
    )
    .unwrap();
    for key in ["save_output", "load", "export"] {
        assert_eq!(dirs[key].as_str(), s.directory.path().to_str(), "{key}");
    }
}

// gui_acceptance.py:1825–1895, 2075–2085: startup persistence and factory restoration.
#[test]
fn startup_default_and_factory_restore_stay_in_session_directory() {
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "Finished RGB");
    let expected = h.state().params.clone();
    let image = h.state().image_path.clone();
    state_controls(h);
    click(h, "Save current as default");
    assert!(h.state().status.starts_with("Saved current GUI state"));
    let directory = s.directory.path().to_path_buf();
    assert_eq!(h.state().config_directory, directory);
    assert!(directory.join("gui_default_state.json").is_file());
    eframe::App::on_exit(h.state_mut(), None);
    let browser = directory.join("restarted-browser.json");
    s.harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 1400.0))
        .build_eframe(move |_| {
            App::new_with_browser_path(Arc::new(CpuBackend), image, None, None, browser, data_dir())
        });
    let h = &mut s.harness;
    assert_eq!(h.state().params, expected);
    assert_eq!(h.state().config_directory, directory);
    state_controls(h);
    click(h, "Restore factory default");
    assert!(
        h.state().status.starts_with("Restored factory default"),
        "{}",
        h.state().status
    );
    assert!(!directory.join("gui_default_state.json").exists());
    let mut factory = state::GuiState::factory().runtime_params().unwrap();
    normalize_scan_output(&mut factory, h.state().film_medium);
    assert_eq!(h.state().params, factory);
    assert_eq!(h.state().params.film_render.grain.rms_granularity, [5.0; 3]);
    assert_eq!(h.state().params.io.upscale_factor, 1.0);
}

// gui_acceptance.py roundtrip transaction; scanner rejection acceptance retained in harness.
#[test]
fn bad_state_and_bad_input_buttons_preserve_accepted_images_and_parameters() {
    let mut s = session();
    let h = &mut s.harness;
    render(h, "PREVIEW");
    let params = h.state().params.clone();
    let image = h.state().image.as_ref().unwrap().clone();
    let output = h.state().output_image.as_ref().unwrap().clone();
    let mut incompatible = h.state().current_state().unwrap().sections;
    incompatible["scanner"]["scan_output"] = serde_json::json!("positive_scan");
    state_controls(h);
    for (name, bytes) in [
        ("malformed.json", b"{ broken JSON".to_vec()),
        (
            "incompatible.json",
            serde_json::to_vec(&incompatible).unwrap(),
        ),
    ] {
        let path = s.directory.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        script(h, chooser::ChooserResult::Scripted(path));
        click(h, "Load from file");
        assert!(
            h.state().status.starts_with("State load error:"),
            "{}",
            h.state().status
        );
        assert_eq!(h.state().params, params);
        assert!(Arc::ptr_eq(h.state().image.as_ref().unwrap(), &image));
        assert!(Arc::ptr_eq(
            h.state().output_image.as_ref().unwrap(),
            &output
        ));
    }
    // A file whose extension promises an image but whose bytes are not a
    // supported container must be rejected without touching the accepted
    // transaction. OIIO probes every plugin for unknown bytes, including its
    // LibRaw reader, which used to abort the process here.
    let corrupt = s.directory.path().join("corrupt.png");
    std::fs::write(&corrupt, b"not an image").unwrap();
    script(h, chooser::ChooserResult::Scripted(corrupt));
    import_file(h, false);
    assert!(
        h.state().status.starts_with("Load error:"),
        "{}",
        h.state().status
    );
    assert_eq!(h.state().params, params);
    assert!(Arc::ptr_eq(h.state().image.as_ref().unwrap(), &image));
    assert!(Arc::ptr_eq(
        h.state().output_image.as_ref().unwrap(),
        &output
    ));
}

// Small uncompressed RGB TIFF, with real EXIF/IPTC/XMP consumed by image_io.
fn metadata_fixture(path: &Path) {
    let artist = b"GUI rotation artist\0";
    let xmp = b"<x:xmpmeta xmlns:x='adobe:ns:meta/'><rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description xmlns:dc='http://purl.org/dc/elements/1.1/'><dc:description><rdf:Alt><rdf:li xml:lang='x-default'>GUI rotation description</rdf:li></rdf:Alt></dc:description></rdf:Description></rdf:RDF></x:xmpmeta>";
    let mut iptc = vec![0x1c, 2, 120, 0, 20];
    iptc.extend_from_slice(b"GUI rotation caption");
    while iptc.len() % 4 != 0 {
        iptc.push(0);
    }
    let count = 14u16;
    let start = 8 + 2 + count as usize * 12 + 4;
    let bits = start;
    let artist_offset = bits + 6;
    let xmp_offset = artist_offset + artist.len();
    let iptc_offset = xmp_offset + xmp.len();
    let pixels = iptc_offset + iptc.len();
    let entries = [
        (256, 4, 1, 4),
        (257, 4, 1, 2),
        (258, 3, 3, bits as u32),
        (259, 3, 1, 1),
        (262, 3, 1, 2),
        (273, 4, 1, pixels as u32),
        (274, 3, 1, 1),
        (277, 3, 1, 3),
        (278, 4, 1, 2),
        (279, 4, 1, 24),
        (284, 3, 1, 1),
        (315, 2, artist.len() as u32, artist_offset as u32),
        (700, 1, xmp.len() as u32, xmp_offset as u32),
        (33723, 4, (iptc.len() / 4) as u32, iptc_offset as u32),
    ];
    let mut bytes = b"II\x2a\0\x08\0\0\0".to_vec();
    bytes.extend_from_slice(&count.to_le_bytes());
    for (tag, kind, n, value) in entries {
        bytes.extend_from_slice(&(tag as u16).to_le_bytes());
        bytes.extend_from_slice(&(kind as u16).to_le_bytes());
        bytes.extend_from_slice(&n.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..3 {
        bytes.extend_from_slice(&8u16.to_le_bytes());
    }
    bytes.extend_from_slice(artist);
    bytes.extend_from_slice(xmp);
    bytes.extend_from_slice(&iptc);
    bytes.extend_from_slice(&[
        10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200,
        210, 220, 230, 240,
    ]);
    std::fs::write(path, bytes).unwrap();
}

fn assert_rotated_metadata(path: &Path, dimensions: (u32, u32)) {
    let bytes = std::fs::read(path).unwrap();
    let little = &bytes[..2] == b"II";
    let short = |at| {
        let b = bytes[at..at + 2].try_into().unwrap();
        if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }
    };
    let long = |at| {
        let b = bytes[at..at + 4].try_into().unwrap();
        if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    };
    let ifd = long(4) as usize;
    let entries = |at| {
        (0..short(at) as usize)
            .map(move |i| at + 2 + i * 12)
            .collect::<Vec<_>>()
    };
    let main = entries(ifd);
    let entry = |list: &[usize], tag| *list.iter().find(|&&at| short(at) == tag).unwrap();
    let orientation = entry(&main, 274);
    assert_eq!(short(orientation + 8), 1);
    let exif = entries(long(entry(&main, 34665) + 8) as usize);
    assert_eq!(long(entry(&exif, 40962) + 8), dimensions.0);
    assert_eq!(long(entry(&exif, 40963) + 8), dimensions.1);
    let payload = |tag, item_size| {
        let at = entry(&main, tag);
        let size = long(at + 4) as usize * item_size;
        let offset = if size <= 4 {
            at + 8
        } else {
            long(at + 8) as usize
        };
        &bytes[offset..offset + size]
    };
    assert_eq!(payload(315, 1), b"GUI rotation artist\0");
    for (tag, item_size, text) in [
        (33723, 4, "GUI rotation caption"),
        (700, 1, "GUI rotation description"),
    ] {
        assert!(
            payload(tag, item_size)
                .windows(text.len())
                .any(|w| w == text.as_bytes()),
            "lost {text}"
        );
    }
}

// gui_acceptance.py:1933–1977: rotation pixels and metadata normalization.
#[test]
fn rotation_save_and_export_preserve_pixel_positions_and_normalize_metadata() {
    let mut s = session();
    let path = s.directory.path().join("metadata-source.tiff");
    metadata_fixture(&path);
    let source = image_io::load(&path).unwrap();
    assert!(source.metadata.is_some());
    let h = &mut s.harness;
    choose(h, "workflow", "Finished RGB");
    click(h, "Import RGB");
    script(h, chooser::ChooserResult::Scripted(path));
    click(h, "select file");
    let original = h.state().image.as_ref().unwrap().clone();
    let baseline = render(h, "SCAN");
    click(h, "cw rotate");
    assert_eq!(h.state().input_rotation, 3);
    let rotated = h.state().image.as_ref().unwrap();
    assert_eq!((rotated.width, rotated.height), (2, 4));
    for y in 0..2 {
        for x in 0..4 {
            let from = ((y * 4 + x) * 3) as usize;
            let to = ((x * 2 + 1 - y) * 3) as usize;
            assert_eq!(&rotated.data[to..to + 3], &original.data[from..from + 3]);
        }
    }
    let scan = render(h, "SCAN");
    assert_eq!((scan.width, scan.height), (2, 4));
    let expected = image_io::convert_image(
        &scan,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    for y in 0..2 {
        for x in 0..4 {
            let from = ((y * 4 + x) * 3) as usize;
            let to = ((x * 2 + 1 - y) * 3) as usize;
            for c in 0..3 {
                assert!((scan.data[to + c] - baseline.data[from + c]).abs() < 1e-6);
            }
        }
    }
    let save = s.directory.path().join("rotated-save.tiff");
    script(h, chooser::ChooserResult::Scripted(save.clone()));
    click(h, "SAVE");
    let actual = image_io::load(&save).unwrap().image;
    assert_eq!((actual.width, actual.height), (2, 4));
    for (&a, &e) in actual.data.iter().zip(&expected.data) {
        assert!((a as f64 - (e as f64).clamp(0.0, 1.0)).abs() <= 1.01 / 65535.0);
    }
    assert_rotated_metadata(&save, (2, 4));
    click(h, "Export options");
    export_action(h);
    choose(h, "Format", "TIFF");
    choose(h, "Bit depth", "32 bit");
    let exported = s.directory.path().join("rotated-export.tiff");
    script(h, chooser::ChooserResult::Scripted(exported.clone()));
    export_action(h);
    wait_until(h, |a| {
        a.export_job.is_none() && a.status.starts_with("Exported")
    });
    assert_float_tiff(&exported);
    let actual = image_io::load(&exported).unwrap().image;
    assert_eq!((actual.width, actual.height), (2, 4));
    for (&a, &e) in actual.data.iter().zip(&expected.data) {
        assert!((a - e).abs() < 2e-5);
    }
    assert_rotated_metadata(&exported, (2, 4));
    click(h, "ccw rotate");
    assert_eq!(h.state().input_rotation, 0);
    assert_eq!(h.state().image.as_ref().unwrap().data, original.data);
    let restored = render(h, "SCAN");
    assert_eq!((restored.width, restored.height), (4, 2));
    assert_eq!(restored.data, baseline.data);
}
