use std::{fs, path::Path, process::Command};

#[test]
fn debug_changes_only_stderr_for_images_and_required_material_payloads() {
    let root = tempfile::tempdir().unwrap();
    image::save_buffer(root.path().join("input.png"), &[90; 6 * 4 * 3], 6, 4, image::ColorType::Rgb8).unwrap();
    let binary = env!("CARGO_BIN_EXE_spektrafilm");
    for artifact in [false, true] {
        for debug in [false, true] {
            let stem = format!("{}-{}", if artifact {"material"} else {"image"}, debug);
            let output = format!("{stem}.{}", if artifact {"sfstage"} else {"tif"});
            let mut command = Command::new(binary);
            command.current_dir(root.path()).args(["process", "input.png", "--film", "kodak_portra_400", "--backend", "cpu", "--data-dir"])
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
                .args(["--input-color-space", "sRGB", "--input-cctf-decoding", "true", "--route", if artifact {"input > film"} else {"input > film > scan"}, "--set", "film_render.grain.active=false", "--seed", "42", "-o", &output]);
            if !artifact { command.args(["--raw-out", &format!("{stem}.raw")]); }
            if debug { command.arg("--debug"); }
            let result = command.env("RUST_LOG", "off").env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS").output().unwrap();
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            assert!(result.stdout.is_empty());
            assert_eq!(String::from_utf8_lossy(&result.stderr).contains("Debug:"), debug);
        }
    }
    assert_eq!(fs::read(root.path().join("image-false.raw")).unwrap(), fs::read(root.path().join("image-true.raw")).unwrap());
    fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut entries: Vec<_> = fs::read_dir(root).unwrap().map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            (name, fs::read(entry.path()).unwrap())
        }).collect();
        entries.sort_by(|a,b| a.0.cmp(&b.0)); entries
    }
    assert_eq!(tree(&root.path().join("material-false.sfstage")), tree(&root.path().join("material-true.sfstage")));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 7, "debug created an unrequested file");
}
