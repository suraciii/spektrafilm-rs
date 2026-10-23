// Interactive spektrafilm preview and export GUI.
//
// A continuous photographic-stage inspector and collapsible preset browser
// surround the retained-image viewer. Application settings and independent
// export preferences keep separate editing owners.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use eframe::egui;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_backend::telemetry::{CollectionMode, ObservationKind, Purpose};
use spektrafilm_core::image_io::{self, BitDepth, ImageMetadata, LoadedImage, SaveOptions};
use spektrafilm_core::params::RuntimeParams;
use spektrafilm_core::presets::workflow_default;
use spektrafilm_core::profile;
use spektrafilm_core::telemetry::{IssueBoundary, IssueCategory};
use spektrafilm_core::telemetry::{Operation, OperationKind, Outcome, SourceRender};
use spektrafilm_math::image::ImageBuf;
mod app;
mod backend;
mod calibration;
mod chooser;
mod controls;
mod diagnostics;
mod display;
mod editor_ranges;
mod embedded_bundle;
mod export;
mod export_job;
#[cfg(test)]
mod headless;
mod histogram;
mod i18n;
rust_i18n::i18n!("locales", fallback = "en");
mod numeric;
mod platform;
mod presets;
mod profiles;
mod progress;
mod render;
mod render_worker;
mod routes;
mod section_reset;
mod state;
mod theme;
mod ui;
use app::App;
use backend::{gui_backend, gui_renderer};
use calibration::CalibrationJob;
use controls::{normalize_scanner_selections, scan_medium};
#[cfg(test)]
use export::ExportBackend;
use export::{ExportDialog, ExportOptions};
use export_job::ExportJob;
use profiles::{
    ProfileEntry, dev_time_combo, pick_data_dir, profile_combo, profile_dev_times, scan_profiles,
};
#[cfg(test)]
use render::RuntimeCache;
use render::{RenderSnapshot, build_runtime_observed};
use render_worker::{RenderJob, RenderKind, RenderRequest, RenderWorker, observed_result};

const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    if let Err(err) = embedded_bundle::setup() {
        eprintln!("[spektrafilm] embedded bundle setup failed: {err:#}");
    }
    let mut args = std::env::args().skip(1);
    let mut initial_image = None;
    let mut initial_state = std::env::var_os("SPEKTRAFILM_GUI_STATE").map(PathBuf::from);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--state" => {
                if let Some(path) = args.next() {
                    initial_state = Some(PathBuf::from(path));
                } else {
                    eprintln!("--state requires a GUI state JSON path");
                    std::process::exit(2);
                }
            }
            "--help" | "-h" => {
                println!("Usage: spektrafilm-gui [IMAGE] [--state GUI_STATE.json]");
                println!(
                    "SPEKTRAFILM_GUI_DIAGNOSTICS_DIR publishes each completed operation's report \
                     as operation-<id>.json in that existing directory."
                );
                return Ok(());
            }
            _ if arg.starts_with('-') => {
                eprintln!("Unknown GUI option: {arg}");
                std::process::exit(2);
            }
            _ => {
                initial_image = Some(PathBuf::from(arg));
            }
        }
    }
    let backend = gui_backend();
    let diagnostics_dir = std::env::var_os("SPEKTRAFILM_GUI_DIAGNOSTICS_DIR").map(PathBuf::from);
    if let Some(directory) = diagnostics_dir.as_deref()
        && !directory.is_dir()
    {
        eprintln!(
            "SPEKTRAFILM_GUI_DIAGNOSTICS_DIR must name an existing directory: {}",
            directory.display()
        );
        std::process::exit(2);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1460.0, 980.0]),
        renderer: gui_renderer(),
        // egui's default device descriptor hardcodes
        // `max_texture_dimension_2d: 8192`, so uploading the preview of an
        // image wider or taller than 8192 px (a single texture) panics.
        // Raise the cap to the adapter's real limit (16384 on Apple
        // Silicon Metal, higher on discrete GPUs), floored at egui's 8192
        // so we never regress. This is eframe's OWN render device — the
        // compute pipeline's wgpu device (WgpuBackend::new) is separate
        // and already lifts its storage-buffer limits.
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetupCreateNew {
                device_descriptor: Arc::new(|adapter: &eframe::wgpu::Adapter| {
                    let base_limits = if adapter.get_info().backend == eframe::wgpu::Backend::Gl {
                        eframe::wgpu::Limits::downlevel_webgl2_defaults()
                    } else {
                        eframe::wgpu::Limits::default()
                    };
                    eframe::wgpu::DeviceDescriptor {
                        label: Some("spektrafilm egui wgpu device"),
                        required_features: eframe::wgpu::Features::default(),
                        required_limits: eframe::wgpu::Limits {
                            max_texture_dimension_2d: adapter
                                .limits()
                                .max_texture_dimension_2d
                                .max(8192),
                            ..base_limits
                        },
                        memory_hints: eframe::wgpu::MemoryHints::default(),
                    }
                }),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "spektrafilm",
        options,
        Box::new(|_cc| {
            theme::apply(&_cc.egui_ctx);
            let app = App::new(backend, initial_image, initial_state, diagnostics_dir);
            app.bind_language(&_cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod telemetry_lifecycle_tests {
    use super::*;
    use spektrafilm_backend::telemetry::ObservationContext;

    fn scanner_app() -> App {
        let mut app = App::new_with_browser_path(
            Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
            None,
            None,
            None,
            std::env::temp_dir().join(format!(
                "scanner-snapshots-{}-{}.json",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
            crate::headless::data_dir(),
        );
        app.film_name = "kodak_portra_400".into();
        app.print_name = "kodak_portra_endura".into();
        app.params.workflow.route = "input > film > scan".into();
        app.params.io.scan_film = true;
        // The lamp drives the direct colorimetric scan: the negative-to-positive
        // density interpretation reads the film curves alone.
        app.params.scanner.scan_output = "direct_scan".into();
        app.params.film_render.grain.active = false;
        app.params.camera.auto_exposure = false;
        app.params.print_render.glare.active = false;
        app.params.settings.preview_max_size = 4;
        app.image = Some(Arc::new(ImageBuf::from_data(
            24,
            8,
            (0..24 * 8 * 3)
                .map(|i| {
                    spektrafilm_math::precision::from_f64(if i / 3 % 24 < 5 { 0.01 } else { 0.7 })
                })
                .collect(),
        )));
        app
    }

    #[test]
    fn app_manual_scanner_edits_reach_the_render_snapshot() {
        let mut app = scanner_app();
        let context = ObservationContext::default();
        let source = app.render_snapshot(app.params.clone());
        app.params.scanner.lens_blur = 2.5;
        app.parameter_revision = app.parameter_revision.wrapping_add(1);
        let edited = app.render_snapshot(app.params.clone());
        let source_runtime = build_runtime_observed(&source, false, &context).unwrap();
        let edited_runtime = build_runtime_observed(&edited, false, &context).unwrap();
        assert_eq!(edited_runtime.params().scanner.lens_blur, 2.5);
        assert_eq!(source_runtime.params().scanner.lens_blur, 0.0);
        let input = app.image.as_ref().unwrap();
        let before = source_runtime
            .process((**input).clone(), app.backend.as_ref())
            .unwrap();
        let after = edited_runtime
            .process((**input).clone(), app.backend.as_ref())
            .unwrap();
        assert!(
            after
                .data
                .iter()
                .all(|sample| spektrafilm_math::precision::to_f64(*sample).is_finite())
        );
        assert_ne!(before.data, after.data);
    }

    #[test]
    fn app_preview_scan_and_export_render_the_same_raster() {
        for first in [Some(true), Some(false)] {
            let mut app = scanner_app();
            let snapshot = app.render_snapshot(app.params.clone());
            let context = ObservationContext::default();
            let input = app.image.as_ref().unwrap().clone();
            let canonical = build_runtime_observed(&snapshot, false, &context).unwrap();
            let mut cache = RuntimeCache::default();
            let preview_first = first.unwrap();
            let runtime = cache
                .prepare_observed(&snapshot, preview_first, &context)
                .unwrap();
            let working = if preview_first {
                spektrafilm_core::params_builder::resize_for_preview(
                    &input,
                    runtime.params().settings.preview_max_size,
                )
            } else {
                (*input).clone()
            };
            let rendered = runtime.process(working, app.backend.as_ref()).unwrap();
            assert!(
                rendered
                    .data
                    .iter()
                    .all(|sample| spektrafilm_math::precision::to_f64(*sample).is_finite())
            );
            assert_eq!(runtime.params().settings.preview_mode, preview_first);
            let export = app.render_snapshot(app.params.clone());
            let scan_runtime = cache.prepare_observed(&export, false, &context).unwrap();
            let scan = scan_runtime
                .process((*input).clone(), app.backend.as_ref())
                .unwrap();
            let export_runtime = build_runtime_observed(&export, false, &context).unwrap();
            let exported = export_runtime
                .process((*input).clone(), app.backend.as_ref())
                .unwrap();
            assert_eq!(
                scan.data, exported.data,
                "shared snapshot must produce the same Scan and Export raster"
            );
            let preview_runtime = cache.prepare_observed(&export, true, &context).unwrap();
            let working = spektrafilm_core::params_builder::resize_for_preview(
                &input,
                preview_runtime.params().settings.preview_max_size,
            );
            let preview = preview_runtime
                .process(working, app.backend.as_ref())
                .unwrap();
            assert_eq!(preview.width.max(preview.height), 4);
            assert!(
                preview
                    .data
                    .iter()
                    .all(|sample| spektrafilm_math::precision::to_f64(*sample).is_finite())
            );
            // The neutral filters are derived from the selected print profile,
            // so the snapshot's raw value and the runtime's resolved value
            // legitimately differ. Everything else must agree exactly.
            let mut snapshot_params = serde_json::to_value(snapshot.params()).unwrap();
            let mut canonical_params = serde_json::to_value(canonical.params()).unwrap();
            for value in [&mut snapshot_params, &mut canonical_params] {
                value["enlarger"]
                    .as_object_mut()
                    .unwrap()
                    .remove("m_filter_neutral");
                value["enlarger"]
                    .as_object_mut()
                    .unwrap()
                    .remove("y_filter_neutral");
            }
            assert_eq!(snapshot_params, canonical_params);
        }
    }

    #[test]
    fn app_real_negative_setup_ignores_virtual_camera_metering() {
        let mut app = scanner_app();
        app.params.workflow.route = "input > convert-film > scan".into();
        app.params.scanner.scan_output = "negative_to_positive".into();
        let context = ObservationContext::default();
        let input = app.image.as_ref().unwrap().clone();
        app.params.camera.auto_exposure = false;
        let first = app.render_snapshot(app.params.clone());
        app.parameter_revision = app.parameter_revision.wrapping_add(1);
        app.params.camera.auto_exposure = true;
        app.params.camera.exposure_compensation_ev += 3.0;
        let changed = app.render_snapshot(app.params.clone());
        let first_runtime = build_runtime_observed(&first, false, &context).unwrap();
        let changed_runtime = build_runtime_observed(&changed, false, &context).unwrap();
        let first_output = first_runtime
            .process((*input).clone(), app.backend.as_ref())
            .unwrap();
        let changed_output = changed_runtime
            .process((*input).clone(), app.backend.as_ref())
            .unwrap();
        assert_eq!(
            first_output.data, changed_output.data,
            "real negative render must bypass virtual camera metering"
        );
    }

    #[test]
    fn app_channel_swaps_use_identical_canonical_and_render_calibration() {
        let mut app = scanner_app();
        app.gui_state.sections["special"]["film_channel_swap"] = serde_json::json!([2, 0, 1]);
        app.gui_state.sections["special"]["print_channel_swap"] = serde_json::json!([1, 2, 0]);
        let snapshot = app.render_snapshot(app.params.clone());
        let input = app.image.as_ref().unwrap().clone();
        let context = ObservationContext::default();
        let canonical = build_runtime_observed(&snapshot, false, &context).unwrap();
        let mut cache = RuntimeCache::default();
        let actual = cache.prepare_observed(&snapshot, false, &context).unwrap();
        let canonical_output = canonical
            .process((*input).clone(), app.backend.as_ref())
            .unwrap();
        let actual_output = actual
            .process((*input).clone(), app.backend.as_ref())
            .unwrap();
        assert_eq!(actual_output.data, canonical_output.data);
    }

    #[test]
    fn imported_negative_conversion_slide_state_fails_without_mutating_current_scan() {
        let mut app = scanner_app();
        let original = app.params.clone();
        let revision = app.parameter_revision;
        let mut invalid = app.current_state().unwrap();
        invalid.sections["simulation"]["film_stock"] = "fujifilm_provia_100f".into();
        invalid.sections["scanner"]["scan_output"] = "negative_to_positive".into();
        let error = app.apply_state(invalid).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("negative filming-stage film profile")
        );
        assert_eq!(app.params, original);
        assert_eq!(app.parameter_revision, revision);
        assert_eq!(app.film_name, "kodak_portra_400");
        app.image_path = None;
        let valid = app.current_state().unwrap();
        app.apply_state(valid).unwrap();
        assert_eq!(app.params.scanner.scan_output, "direct_scan");
        assert_eq!(app.params.scanner.lens_blur, 0.0);
    }

    fn job(request_id: u64) -> RenderJob {
        RenderJob {
            input_epoch: 2,
            parameter_revision: 5,
            request_id,
            kind: RenderKind::Preview,
            animate: false,
            collection_mode: CollectionMode::Summary,
        }
    }

    #[test]
    fn acceptance_rejects_both_revision_changes_and_newer_requests() {
        let old = job(1);
        let latest = job(2);
        assert!(!old.matches(Some(&latest), 2, 5));
        assert!(!latest.matches(Some(&latest), 3, 5));
        assert!(!latest.matches(Some(&latest), 2, 6));
        assert!(latest.matches(Some(&latest), 2, 5));
    }

    #[test]
    fn failed_write_keeps_partial_scope_and_failed_terminal_outcome() {
        let operation = Operation::new(OperationKind::Save, CollectionMode::Summary);
        let scope = operation.scope("file_write", ObservationKind::Phase, Purpose::Image);
        let result: Result<(), &str> = observed_result(
            scope,
            Err("write failure"),
            IssueCategory::Writing,
            IssueBoundary::FileWrite,
        );
        assert!(result.is_err());
        let report = operation.finish(Outcome::Failed).unwrap();
        assert_eq!(report.operation.outcome, Outcome::Failed);
        assert!(report.phases.iter().any(|phase| phase.name == "file_write"
            && !phase.complete
            && phase.outcome == Some(Outcome::Failed)));
        assert!(
            report
                .diagnostic_issues
                .iter()
                .any(|issue| issue.category == IssueCategory::Writing
                    && issue.boundary == IssueBoundary::FileWrite)
        );
    }

    #[test]
    fn concurrent_contexts_do_not_share_observations() {
        let preview = Operation::new(OperationKind::Preview, CollectionMode::Summary);
        let export = Operation::new(OperationKind::Export, CollectionMode::Summary);
        let preview_context = preview.context();
        let export_context = export.context();
        let handle = std::thread::spawn(move || {
            let phase = export_context.scope("file_write", ObservationKind::Phase, Purpose::Image);
            phase.finish(Outcome::Succeeded);
        });
        let phase =
            preview_context.scope("display_prepare", ObservationKind::Phase, Purpose::Image);
        phase.finish(Outcome::Succeeded);
        handle.join().unwrap();
        let preview = preview.finish(Outcome::Superseded).unwrap();
        let export = export.finish(Outcome::Cancelled).unwrap();
        assert!(
            preview
                .phases
                .iter()
                .all(|phase| phase.name != "file_write")
        );
        assert!(
            export
                .phases
                .iter()
                .all(|phase| phase.name != "display_prepare")
        );
        assert_ne!(preview.operation.id, export.operation.id);
    }
}
