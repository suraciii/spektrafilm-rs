use super::*;
use crate::export_job::ExportPause;
use parking_lot::{Condvar, Mutex};
use spektrafilm_backend::telemetry::CollectionMode;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

fn staged_export(h: &mut Harness<'_, App>, destination: &Path) -> Arc<ExportPause> {
    let (staged, ready) = mpsc::channel();
    let pause = Arc::new(ExportPause {
        staged,
        released: (Mutex::new(false), Condvar::new()),
        cancelled: (Mutex::new(false), Condvar::new()),
    });
    h.state_mut().diagnostics.mode = CollectionMode::Summary;
    let ctx = h.ctx.clone();
    let options = h.state().export_options.clone();
    h.state_mut()
        .start_export_to_path_inner(&ctx, options, destination, Some(pause.clone()));
    // The real writer stages output on its own thread. Under the workspace
    // gate many test binaries and worker threads run at once, so wait on the
    // repo's real-worker budget rather than a tight barrier: measured 4-9s
    // unloaded, with a 20s barrier failing intermittently on loaded machines.
    ready
        .recv_timeout(WAIT_DEADLINE)
        .expect("real export writer did not produce staged output");
    h.run_steps(3);
    pause
}

fn files(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

fn assert_cancelled_report(path: &Path) {
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["operation"]["outcome"], "cancelled");
    assert_eq!(report["operation"]["kind"], "export");
}

#[test]
fn cancel_discards_real_staged_export_and_preserves_existing_destination() {
    for existing in [false, true] {
        let mut s = session();
        let output_directory = s.directory.path().join("output");
        std::fs::create_dir(&output_directory).unwrap();
        let destination = output_directory.join("cancelled.png");
        let original = b"previous user output must survive cancellation";
        if existing {
            std::fs::write(&destination, original).unwrap();
        }
        let h = &mut s.harness;
        let pause = staged_export(h, &destination);
        let staged: Vec<_> = files(&output_directory)
            .into_iter()
            .filter(|path| path != &destination)
            .collect();
        assert_eq!(staged.len(), 1, "the real writer must stage a file");
        assert!(std::fs::metadata(&staged[0]).unwrap().len() > 0);
        if existing {
            assert_eq!(std::fs::read(&destination).unwrap(), original);
        } else {
            assert!(!destination.exists());
        }
        click(h, "Cancel export");
        assert!(
            h.state()
                .export_job
                .as_ref()
                .unwrap()
                .cancel
                .load(Ordering::SeqCst)
        );
        pause.release();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !h
            .state()
            .export_job
            .as_ref()
            .unwrap()
            .handle
            .as_ref()
            .unwrap()
            .is_finished()
        {
            assert!(
                Instant::now() < deadline,
                "cancelled export worker did not stop"
            );
            wait_tick();
        }
        h.run_steps(3);
        assert!(
            h.state().export_job.is_none(),
            "App must drain and join the export"
        );
        assert!(!staged[0].exists(), "staged file must be removed");
        if existing {
            assert_eq!(std::fs::read(&destination).unwrap(), original);
            assert_eq!(files(&output_directory), vec![destination]);
        } else {
            assert!(!destination.exists());
            assert!(files(&output_directory).is_empty());
        }
        let report = s.directory.path().join("cancelled-report.json");
        h.state().diagnostics.save_selected(&report).unwrap();
        assert_cancelled_report(&report);
    }
}

#[test]
fn on_exit_cancels_and_joins_staged_export_without_publication() {
    let mut s = session();
    let output_directory = s.directory.path().join("output");
    let reports = s.directory.path().join("reports");
    std::fs::create_dir(&output_directory).unwrap();
    std::fs::create_dir(&reports).unwrap();
    let destination = output_directory.join("exit.png");
    let h = &mut s.harness;
    h.state_mut()
        .diagnostics
        .publish_completed_to(reports.clone());
    let pause = staged_export(h, &destination);
    assert_eq!(files(&output_directory).len(), 1);
    let cancel = h.state().export_job.as_ref().unwrap().cancel.clone();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut cancelled = pause.cancelled.0.lock();
            while !*cancelled {
                pause.cancelled.1.wait(&mut cancelled);
            }
            drop(cancelled);
            pause.release();
        });
        eframe::App::on_exit(h.state_mut(), None);
    });
    assert!(cancel.load(Ordering::SeqCst));
    assert!(h.state().export_job.is_none());
    assert!(h.state().render_worker.handle.is_none());
    assert!(!destination.exists());
    assert!(files(&output_directory).is_empty());
    let export_reports: Vec<_> = files(&reports)
        .into_iter()
        .filter(|path| {
            let report: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            report["operation"]["kind"] == "export"
        })
        .collect();
    assert_eq!(export_reports.len(), 1);
    assert_cancelled_report(&export_reports[0]);
}

#[test]
fn on_exit_joins_active_render_and_discards_queued_results() {
    let mut s = session();
    let h = &mut s.harness;
    reveal_control(h, "workflow.route");
    choose(h, "workflow", "input");
    let baseline = preview(h);
    let ctx = h.ctx.clone();
    let active = h.state().render_worker.active.clone();
    let shutdown = h.state().render_worker.shutdown.clone();
    std::thread::scope(|scope| {
        let (held, ready) = mpsc::channel();
        scope.spawn(move || {
            let guard = active.lock();
            held.send(()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !shutdown.load(Ordering::SeqCst) {
                assert!(
                    Instant::now() < deadline,
                    "on_exit did not request shutdown"
                );
                wait_tick();
            }
            drop(guard);
        });
        ready.recv_timeout(Duration::from_secs(10)).unwrap();
        h.state_mut().dispatch_render(&ctx);
        let deadline = Instant::now() + Duration::from_secs(10);
        while h.state().render_worker.state.0.lock().is_some() {
            assert!(
                Instant::now() < deadline,
                "render worker did not take request"
            );
            wait_tick();
        }
        h.state_mut().full_scan_requested = true;
        h.state_mut().dispatch_render(&ctx);
        assert!(h.state().render_worker.state.0.lock().is_some());
        eframe::App::on_exit(h.state_mut(), None);
    });
    assert!(h.state().render_worker.shutdown.load(Ordering::SeqCst));
    assert!(
        h.state().render_worker.handle.is_none(),
        "worker must have been joined"
    );
    assert!(h.state().render_worker.state.0.lock().is_none());
    assert!(h.state().render_worker.active.lock().is_none());
    assert!(h.state().render_worker.active_cancel.lock().is_none());
    assert!(matches!(
        h.state().render_worker.rx.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    assert!(Arc::ptr_eq(
        &baseline,
        h.state().output_image.as_ref().unwrap()
    ));
}
