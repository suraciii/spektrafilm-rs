use spektrafilm_fotufilm::{Request, Session};
use std::path::Path;

fn copy_resource(source: &Path, destination: &Path, name: &str) {
    let path = destination.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::copy(source.join(name), path).unwrap();
}

#[test]
fn corrupted_source_resources_fail_before_rendering_including_retained_sessions() {
    let original = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fotufilm");
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("fotufilm");
    for name in [
        "stocks/portra400.json",
        "preparation.wasm",
        "prior/rec2020-reflectance-prior.coeff",
    ] {
        copy_resource(&original, &root, name);
    }
    let session = Session::new(temporary.path(), Request::default()).unwrap();
    let prior = root.join("prior/rec2020-reflectance-prior.coeff");
    std::fs::write(&prior, b"truncated prior").unwrap();
    let error = session
        .render(1, 1, &[0.18, 0.18, 0.18], 0, None)
        .unwrap_err();
    assert!(
        error.contains("prior/rec2020-reflectance-prior.coeff does not match"),
        "{error}"
    );
    copy_resource(&original, &root, "prior/rec2020-reflectance-prior.coeff");
    std::fs::write(root.join("preparation.wasm"), b"wrong producer").unwrap();
    let error = match Session::new(temporary.path(), Request::default()) {
        Ok(_) => panic!("corrupted producer reused a cached session"),
        Err(error) => error,
    };
    assert!(error.contains("preparation.wasm does not match"), "{error}");
    copy_resource(&original, &root, "preparation.wasm");
    std::fs::write(root.join("stocks/portra400.json"), b"{}").unwrap();
    let error = match Session::new(temporary.path(), Request::default()) {
        Ok(_) => panic!("corrupted stock was accepted"),
        Err(error) => error,
    };
    assert!(
        error.contains("stocks/portra400.json does not match"),
        "{error}"
    );
}
