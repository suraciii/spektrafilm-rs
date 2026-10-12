use spektrafilm_core::image_io::{self, BitDepth, SaveOptions};
use spektrafilm_gpu::telemetry::{CollectionMode, ObservationContext};
use spektrafilm_math::{image::ImageBuf, precision::from_f64};

fn options(extension: &str) -> SaveOptions<'static> {
    SaveOptions {
        depth: if extension == "exr" {
            BitDepth::ThirtyTwo
        } else {
            BitDepth::Eight
        },
        color_space: "sRGB",
        cctf_encoding: extension != "exr",
        jpeg_quality: None,
        jpeg_subsampling: None,
        compression: None,
    }
}

fn image() -> ImageBuf {
    ImageBuf::from_data(
        128,
        128,
        (0..128 * 128 * 3)
            .map(|index| from_f64(((index * 7919 % 65521) as f64) / 65521.0))
            .collect(),
    )
}

#[test]
fn shared_writer_publishes_and_replaces_all_containers() {
    let directory = tempfile::tempdir().unwrap();
    let image = image();
    for extension in ["png", "jpg", "tif", "exr"] {
        let output = directory.path().join(format!("output.{extension}"));
        for replace in [false, true] {
            if replace {
                std::fs::write(&output, b"old destination").unwrap();
            }
            let report = image_io::save(&output, &image, options(extension), None).unwrap();
            assert!(
                report.metadata_warning.is_none(),
                "{extension}: {:?}",
                report.metadata_warning
            );
            let loaded = image_io::load(&output).unwrap();
            assert_eq!((loaded.image.width, loaded.image.height), (128, 128));
            if extension != "exr" {
                assert!(loaded.metadata.is_some());
            }
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        }
        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn prepared_output_cancellation_and_no_replace_preserve_destination() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("output.png");
    let image = image();
    for existing in [false, true] {
        if existing {
            std::fs::write(&output, b"sentinel").unwrap();
        }
        let prepared = image_io::prepare_save_observed(
            &output,
            &image,
            options("png"),
            None,
            &ObservationContext::new(CollectionMode::Off),
        )
        .unwrap();
        if existing {
            assert_eq!(std::fs::read(&output).unwrap(), b"sentinel");
        } else {
            assert!(!output.exists());
        }
        drop(prepared);
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            usize::from(existing)
        );
    }
    let prepared = image_io::prepare_save_observed(
        &output,
        &image,
        options("png"),
        None,
        &ObservationContext::new(CollectionMode::Off),
    )
    .unwrap();
    assert!(prepared.publish(false).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"sentinel");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn failed_atomic_replacement_cleans_only_owned_staging() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("output.png");
    let image = image();
    let prepared = image_io::prepare_save_observed(
        &output,
        &image,
        options("png"),
        None,
        &ObservationContext::new(CollectionMode::Off),
    )
    .unwrap();
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("sentinel"), b"preserved").unwrap();
    let unrelated = directory.path().join(".spektrafilm-unrelated.png");
    std::fs::write(&unrelated, b"not ours").unwrap();
    assert!(prepared.publish(true).is_err());
    assert_eq!(
        std::fs::read(output.join("sentinel")).unwrap(),
        b"preserved"
    );
    assert_eq!(std::fs::read(unrelated).unwrap(), b"not ours");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn native_short_write_and_finalization_failures_preserve_destinations() {
    const CHILD: &str = "SPEKTRAFILM_PUBLICATION_LIMIT_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "native_short_write_and_finalization_failures_preserve_destinations",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    // The limit and signal disposition affect only this isolated test process.
    unsafe {
        libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
    }
    let mut original = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_FSIZE, &mut original) },
        0
    );
    let image = image();
    for extension in ["png", "jpg", "tif", "exr"] {
        for limit in [64, 8192] {
            for existing in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let output = directory.path().join(format!("output.{extension}"));
                if existing {
                    std::fs::write(&output, b"sentinel").unwrap();
                }
                let limited = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: original.rlim_max,
                };
                assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &limited) }, 0);
                let result = image_io::save(&output, &image, options(extension), None);
                assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &original) }, 0);
                let error = match result {
                    Ok(_) => panic!("{extension} limit {limit} incorrectly succeeded"),
                    Err(error) => error,
                };
                let message = error.to_string();
                assert!(message.contains("save failed"), "{extension}: {message}");
                assert!(message.contains(&output.display().to_string()), "{message}");
                if existing {
                    assert_eq!(std::fs::read(&output).unwrap(), b"sentinel");
                } else {
                    assert!(!output.exists());
                }
                assert_eq!(
                    std::fs::read_dir(directory.path()).unwrap().count(),
                    usize::from(existing)
                );
            }
        }
    }
}
