//! Real App acceptance without a display server or a native file chooser.
//! Only confirmed chooser paths cross the explicit Save/Export boundary here.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use spektrafilm_gpu::cpu_backend::CpuBackend;
use tempfile::TempDir;

struct Session {
    harness: Harness<'static, App>,
    directory: TempDir,
}
impl Drop for Session {
    fn drop(&mut self) {
        eframe::App::on_exit(self.harness.state_mut(), None);
    }
}

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn app(state_path: PathBuf, image_path: PathBuf, browser_path: PathBuf) -> App {
    App::new_with_browser_path(
        Arc::new(CpuBackend),
        Some(image_path),
        Some(state_path),
        None,
        browser_path,
        data_dir(),
    )
}

fn session() -> Session {
    let directory = tempfile::tempdir().unwrap();
    let image_path = directory.path().join("asymmetric-ramp.png");
    let mut image = ImageBuf::new(160, 80);
    for y in 0..image.height {
        for x in 0..image.width {
            let i = (y * image.width + x) as usize * 3;
            let ramp = 0.08 + 0.8 * x as f64 / (image.width - 1) as f64;
            image.data[i] = spektrafilm_math::precision::from_f64(ramp);
            image.data[i + 1] =
                spektrafilm_math::precision::from_f64(ramp * 0.85 + y as f64 * 0.0005);
            image.data[i + 2] = spektrafilm_math::precision::from_f64(ramp * 0.7 + 0.04);
        }
    }
    image_io::save(
        &image_path,
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
    let factory = state::GuiState::factory();
    let mut params = factory.runtime_params().unwrap();
    params.settings.preview_max_size = 128;
    params.camera.auto_exposure = false;
    params.debug.deactivate_spatial_effects = true;
    params.debug.deactivate_stochastic_effects = true;
    params.io.crop = false;
    let mut initial =
        state::GuiState::from_runtime(&params, factory.film(), factory.paper(), &factory.sections)
            .unwrap();
    initial.sections["simulation"]["auto_preview"] = serde_json::json!(false);
    let state_path = directory.path().join("initial.json");
    initial.save(&state_path).unwrap();
    let browser_path = directory.path().join("browser.json");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1500.0, 1400.0))
        .build_eframe(move |_| app(state_path, image_path, browser_path));
    assert_eq!(harness.state().image.as_ref().unwrap().width, 160);
    click(&mut harness, "◀");
    Session { harness, directory }
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).simulate_click();
    h.run_steps(3);
}

fn choose(h: &mut Harness<'_, App>, label: &str, option: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, label)
        .simulate_click();
    h.run_steps(3);
    if h.state().export_dialog.is_open() {
        h.get_by_label(option).simulate_click();
        h.run_steps(3);
    } else {
        h.get_by_label(option).click();
        h.run_steps(3);
        h.press_key(egui::Key::Escape);
        h.run_steps(3);
    }
}

fn edit(h: &mut Harness<'_, App>, label: &str, text: &str) {
    h.get_by_label(label).focus();
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

fn wait_until(h: &mut Harness<'_, App>, condition: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        h.step();
        if condition(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "App completion timed out: {}",
            h.state().status
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn render(h: &mut Harness<'_, App>, button: &str) -> Arc<ImageBuf> {
    let previous = h.state().output_image.clone();
    click(h, button);
    wait_until(h, |a| {
        !a.dirty
            && a.render_job.is_none()
            && a.output_image
                .as_ref()
                .is_some_and(|out| previous.as_ref().is_none_or(|old| !Arc::ptr_eq(old, out)))
    });
    assert!(
        h.state().status.starts_with(if button == "SCAN" {
            "Scan ·"
        } else {
            "Preview ·"
        }),
        "{}",
        h.state().status
    );
    h.state().output_image.as_ref().unwrap().clone()
}

fn mean(image: &ImageBuf) -> f64 {
    image.data.iter().map(|&x| x as f64).sum::<f64>() / image.data.len() as f64
}

fn edge_difference(image: &ImageBuf) -> f64 {
    let row = image.height / 2;
    let pixel = |x| image.data[((row * image.width + x) * 3) as usize] as f64;
    pixel(image.width * 3 / 4) - pixel(image.width / 4)
}

fn assert_png(path: &Path, expected: &ImageBuf) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(bytes[24], 8, "PNG must carry 8-bit samples");
    let actual = image_io::load(path).unwrap().image;
    assert_eq!(
        (actual.width, actual.height),
        (expected.width, expected.height)
    );
    assert_eq!(actual.data.len(), expected.data.len());
    for (&a, &e) in actual.data.iter().zip(&expected.data) {
        let clipped = (e as f64).clamp(0.0, 1.0);
        assert!(
            (a as f64 - clipped).abs() <= 1.01 / 255.0,
            "saved pixel {a} differs from {e}"
        );
    }
}

// Read standard TIFF tags rather than infer the depth from roundtrip pixels.
fn assert_float_tiff(path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    let little = match &bytes[..2] {
        b"II" => true,
        b"MM" => false,
        other => panic!("TIFF byte order: {other:?}"),
    };
    let u16_at = |at| {
        let value = [bytes[at], bytes[at + 1]];
        if little {
            u16::from_le_bytes(value)
        } else {
            u16::from_be_bytes(value)
        }
    };
    let u32_at = |at| {
        let value = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        if little {
            u32::from_le_bytes(value)
        } else {
            u32::from_be_bytes(value)
        }
    };
    assert_eq!(u16_at(2), 42, "small exported TIFF uses classic IFD");
    let ifd = u32_at(4) as usize;
    let mut bits = None;
    let mut sample_format = None;
    for index in 0..u16_at(ifd) as usize {
        let entry = ifd + 2 + index * 12;
        let tag = u16_at(entry);
        if matches!(tag, 258 | 339) {
            assert_eq!(u16_at(entry + 2), 3, "sample tag must be SHORT");
            let count = u32_at(entry + 4) as usize;
            let values = if count * 2 <= 4 {
                entry + 8
            } else {
                u32_at(entry + 8) as usize
            };
            let samples: Vec<_> = (0..count).map(|i| u16_at(values + i * 2)).collect();
            if tag == 258 {
                bits = Some(samples);
            } else {
                sample_format = Some(samples);
            }
        }
    }
    let bits = bits.expect("BitsPerSample TIFF tag");
    assert!(!bits.is_empty() && bits.iter().all(|&x| x == 32));
    let format = sample_format.expect("SampleFormat TIFF tag");
    assert!(
        !format.is_empty() && format.iter().all(|&x| x == 3),
        "IEEE floating point samples"
    );
}

#[test]
fn scanner_ui_real_preview_scan_persistence_save_and_export() {
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "input > film > scan");
    assert_eq!(h.state().params.workflow.route, "input > film > scan");
    click(h, "Profiles");
    let profile = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    let profile_label = profile.info.name.as_deref().unwrap_or("kodak_portra_400");
    choose(h, "film profile", profile_label);
    assert_eq!(h.state().film_name, "kodak_portra_400");
    // Selecting a stock intentionally resets its workflow; choose the eligible
    // scanner route after that production profile-selection behavior.
    choose(h, "workflow", "input > film > scan");
    click(h, "Scanner");
    choose(h, "scan output", "direct_scan");
    assert!(h.query_by_label("positive density ev").is_none());
    let direct = render(h, "PREVIEW");
    assert_eq!((direct.width, direct.height), (128, 64));
    choose(h, "scan output", "positive_scan");
    assert!(h.query_by_label("positive density ev").is_some());
    edit(h, "positive density ev", "0");
    let positive = render(h, "PREVIEW");
    assert!(
        edge_difference(&direct) < -0.01,
        "negative capture must reverse the ramp"
    );
    assert!(
        edge_difference(&positive) > 0.01,
        "positive interpretation must restore ramp polarity"
    );
    edit(h, "positive density ev", "1");
    assert_eq!(h.state().params.scanner.positive_density_ev, 1.0);
    let brighter = render(h, "PREVIEW");
    assert!(mean(&brighter) > mean(&positive) + 0.01);
    edit(h, "positive balance ev 0", "0.25");
    edit(h, "positive balance ev 1", "-0.125");
    edit(h, "positive balance ev 2", "0.5");
    assert_eq!(
        h.state().params.scanner.positive_balance_ev,
        [0.25, -0.125, 0.5]
    );
    edit(h, "white point 0", "0.95");
    edit(h, "black point 0", "0.025");
    assert_eq!(h.state().params.scanner.white_point, [0.95, 1.0, 1.0]);
    assert_eq!(h.state().params.scanner.black_point, [0.025, 0.0, 0.0]);
    edit(h, "positive density ev", "not-a-number");
    assert_eq!(h.state().params.scanner.positive_density_ev, 1.0);
    assert_eq!(
        h.get_by_label("positive density ev").value().as_deref(),
        Some("1.000")
    );
    let preview = render(h, "PREVIEW");
    let save_path = s.directory.path().join("retained.png");
    h.state_mut().save_to_path(&save_path);
    assert!(
        h.state().status.starts_with("Saved retained.png"),
        "{}",
        h.state().status
    );
    let dialog_dirs: serde_json::Value = serde_json::from_slice(
        &std::fs::read(s.directory.path().join("dialog_dirs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        dialog_dirs["save_output"].as_str(),
        s.directory.path().to_str()
    );
    let expected_saved = image_io::convert_image(
        &preview,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&save_path, &expected_saved);
    let scan = render(h, "SCAN");
    assert_eq!((scan.width, scan.height), (160, 80));
    // Export must ignore the most recently retained preview's reduced resolution.
    render(h, "PREVIEW");
    click(h, "Export options");
    click(h, "Export…");
    assert!(h.state().export_dialog.is_open());
    choose(h, "Format", "TIFF");
    choose(h, "Bit depth", "32 bit");
    choose(h, "Backend", ExportBackend::Gpu.label());
    click(h, "Cancel");
    assert!(!h.state().export_dialog.is_open());
    assert_eq!(h.state().export_options.format, export::ExportFormat::Png);
    assert_eq!(h.state().export_options.backend, ExportBackend::Cpu);
    assert_eq!(h.state().export_options.depth, BitDepth::Eight);
    assert!(h.state().export_job.is_none());
    // The native chooser itself remains a native-gate concern. Its confirmed
    // result enters exactly the same production path used by the modal submit.
    let export_path = s.directory.path().join("full.png");
    let options = h.state().export_options.clone();
    let ctx = h.ctx.clone();
    h.state_mut()
        .start_export_to_path(&ctx, options, &export_path);
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.starts_with("Exported"),
        "{}",
        h.state().status
    );
    let expected_export = image_io::convert_image(
        &scan,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        true,
    )
    .unwrap();
    assert_png(&export_path, &expected_export);
    assert_eq!(h.state().output_image.as_ref().unwrap().width, 128);
    // Extension correction must never silently overwrite an existing file.
    let before = std::fs::read(&export_path).unwrap();
    let options = h.state().export_options.clone();
    h.state_mut().start_export_to_path(
        &ctx,
        options,
        &s.directory.path().join("full.wrong-extension"),
    );
    assert!(h.state().export_job.is_none());
    assert!(
        h.state()
            .status
            .starts_with("Export destination already exists:")
    );
    assert_eq!(std::fs::read(&export_path).unwrap(), before);
    let mut float_options = h.state().export_options.clone();
    float_options.format = export::ExportFormat::Tiff;
    float_options.depth = BitDepth::ThirtyTwo;
    float_options.saving_cctf_encoding = false;
    let float_path = s.directory.path().join("full-float.tiff");
    h.state_mut()
        .start_export_to_path(&ctx, float_options, &float_path);
    wait_until(h, |a| a.export_job.is_none());
    assert!(
        h.state().status.starts_with("Exported"),
        "{}",
        h.state().status
    );
    assert_float_tiff(&float_path);
    let float_output = image_io::load(&float_path).unwrap().image;
    let linear = image_io::convert_image(
        &scan,
        &h.state().output_color_space,
        h.state().output_cctf_encoding,
        "sRGB",
        false,
    )
    .unwrap();
    assert_eq!((float_output.width, float_output.height), (160, 80));
    for (&actual, &expected) in float_output.data.iter().zip(&linear.data) {
        assert!((actual as f64 - expected as f64).abs() < 2e-5);
    }
    let persisted = s.directory.path().join("persisted.json");
    h.state().current_state().unwrap().save(&persisted).unwrap();
    let mut restored = app(
        persisted,
        s.directory.path().join("asymmetric-ramp.png"),
        s.directory.path().join("restored-browser.json"),
    );
    assert_eq!(restored.params.workflow.route, "input > film > scan");
    assert_eq!(restored.params.scanner.scan_output, "positive_scan");
    assert_eq!(restored.params.scanner.positive_density_ev, 1.0);
    assert_eq!(
        restored.params.scanner.positive_balance_ev,
        [0.25, -0.125, 0.5]
    );
    assert_eq!(restored.film_name, "kodak_portra_400");
    assert_eq!(restored.params.scanner.white_point, [0.95, 1.0, 1.0]);
    assert_eq!(restored.params.scanner.black_point, [0.025, 0.0, 0.0]);
    eframe::App::on_exit(&mut restored, None);
    choose(h, "workflow", "Finished RGB");
    assert_eq!(h.state().params.scanner.scan_output, "direct_scan");
    assert!(h.query_by_label("positive density ev").is_none());
}

#[test]
fn real_worker_accepts_only_latest_request_and_current_revision() {
    let mut s = session();
    let h = &mut s.harness;
    choose(h, "workflow", "Finished RGB");
    let baseline = render(h, "PREVIEW");
    let ctx = h.ctx.clone();
    // Hold the worker's existing start marker while UI events change the
    // revision. This schedules real work deterministically, not a fake response.
    let active = h.state().render_worker.active.clone();
    let start_guard = active.lock();
    h.state_mut().dispatch_render(&ctx);
    let stale_revision = h.state().render_job.as_ref().unwrap().parameter_revision;
    choose(h, "workflow", "input > film > scan");
    assert!(h.state().parameter_revision > stale_revision);
    drop(start_guard);
    wait_until(h, |a| a.render_job.is_none());
    assert!(
        Arc::ptr_eq(&baseline, h.state().output_image.as_ref().unwrap()),
        "stale real worker response must not replace retained output"
    );
    h.state_mut().dispatch_render(&ctx);
    let first_request = h.state().render_job.as_ref().unwrap().request_id;
    h.state_mut().dispatch_render(&ctx);
    assert!(h.state().render_job.as_ref().unwrap().request_id > first_request);
    wait_until(h, |a| {
        a.render_job.is_none()
            && a.output_image
                .as_ref()
                .is_some_and(|out| !Arc::ptr_eq(out, &baseline))
    });
    assert!(edge_difference(h.state().output_image.as_ref().unwrap()) < -0.01);
    assert!(h.state().status.starts_with("Preview ·"));
}

#[cfg(feature = "gui-snapshots")]
mod layout_snapshots {
    use super::*;

    #[test]
    fn export_modal_offscreen_layout() {
        let mut s = session();
        let h = &mut s.harness;
        click(h, "Export options");
        click(h, "Export…");
        choose(h, "Format", "TIFF");
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::ComboBox, "Compression")
                .is_some()
        );
        assert!(h.query_by_label("Apply saving CCTF encoding").is_some());
        // Attach the offscreen renderer only in this opt-in gate; render the
        // real dialog's TIFF-specific controls selected above.
        let mut modal = Harness::builder()
            .with_size(egui::vec2(700.0, 650.0))
            .wgpu()
            .build_state(
                |ctx, dialog: &mut ExportDialog| {
                    dialog.show(ctx);
                },
                std::mem::take(&mut h.state_mut().export_dialog),
            );
        modal.run_steps(3);
        let image = modal.render().expect("offscreen export modal rendering");
        assert_eq!((image.width(), image.height()), (700, 650));
        let mut rows = Vec::new();
        for label in [
            "Backend",
            "Format",
            "Bit depth",
            "Compression",
            "Saving color space",
            "Apply saving CCTF encoding",
            "Export…",
            "Cancel",
        ] {
            let role = match label {
                "Apply saving CCTF encoding" => egui::accesskit::Role::CheckBox,
                "Export…" | "Cancel" => egui::accesskit::Role::Button,
                _ => egui::accesskit::Role::ComboBox,
            };
            let bounds = modal
                .get_by_role_and_label(role, label)
                .raw_bounds()
                .unwrap();
            assert!(
                bounds.x0 >= 0.0 && bounds.y0 >= 0.0 && bounds.x1 <= 700.0 && bounds.y1 <= 650.0,
                "{label} is clipped: {bounds:?}"
            );
            assert!(bounds.x1 > bounds.x0 && bounds.y1 > bounds.y0);
            let region_colors: std::collections::HashSet<_> = (bounds.y0.ceil() as u32
                ..bounds.y1.floor() as u32)
                .flat_map(|y| {
                    let image = &image;
                    (bounds.x0.ceil() as u32..bounds.x1.floor() as u32)
                        .map(move |x| image.get_pixel(x, y).0)
                })
                .collect();
            assert!(
                region_colors.len() > 2,
                "{label} must have visible offscreen pixels"
            );
            if !matches!(label, "Export…" | "Cancel") {
                rows.push((label, bounds));
            }
        }
        for pair in rows.windows(2) {
            assert!(
                pair[0].1.y1 <= pair[1].1.y0,
                "modal rows overlap: {} and {}",
                pair[0].0,
                pair[1].0
            );
        }
        let colors: std::collections::HashSet<_> = image.pixels().map(|p| p.0).collect();
        assert!(
            colors.len() > 20,
            "modal text and controls must render, not just a background"
        );
        if let Some(directory) = std::env::var_os("SPEKTRAFILM_GUI_SNAPSHOT_DIR") {
            let directory = PathBuf::from(directory);
            assert!(
                directory.is_absolute(),
                "snapshot directory must be absolute"
            );
            let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .canonicalize()
                .unwrap();
            std::fs::create_dir_all(&directory).unwrap();
            let directory = directory.canonicalize().unwrap();
            assert!(
                !directory.starts_with(repository),
                "snapshot artifacts must be outside the repository"
            );
            image.save(directory.join("export-modal.png")).unwrap();
        }
    }
}
