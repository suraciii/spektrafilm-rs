//! `target` pins the renderer, data, and backend it will use.

#![cfg(feature = "precision-f64")]
use serde_json::{Value, json};
use spektrafilm_benchmark::{common, documents};
use std::{fs, process::Command};
use tempfile::TempDir;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-f64"))
}

fn success(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn target_pins_dependencies_and_refuses_existing_destinations() {
    let temp = TempDir::new().unwrap();
    let renderer = temp.path().join("renderer");
    fs::write(&renderer, b"pinned executable bytes").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&renderer, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let data = temp.path().join("data");
    fs::create_dir(&data).unwrap();
    let out = temp.path().join("my-renderer.json");
    let result = success(
        cli()
            .arg("target")
            .arg("--renderer")
            .arg(&renderer)
            .arg("--data-dir")
            .arg(&data)
            .args(["--backend", "gpu"])
            .arg("--out")
            .arg(&out)
            .output()
            .unwrap(),
    );
    assert_eq!(result["target_id"], "my-renderer");
    let target = documents::load_target(&out).unwrap();
    assert_eq!(
        target["renderer"]["path"],
        json!(renderer.canonicalize().unwrap())
    );
    assert_eq!(
        target["renderer"]["sha256"],
        common::sha256_file(&renderer).unwrap()
    );
    assert_eq!(target["data_dir"], json!(data.canonicalize().unwrap()));
    assert_eq!(target["changes"], json!({"set_files":[],"set":[]}));
    let before = fs::read(&out).unwrap();
    let repeat = cli()
        .arg("target")
        .arg("--renderer")
        .arg(&renderer)
        .arg("--data-dir")
        .arg(&data)
        .args(["--backend", "cpu"])
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(repeat.status.code(), Some(1));
    let refused: Value = serde_json::from_slice(&repeat.stdout).unwrap();
    assert_eq!(refused["errors"][0]["code"], "destination_exists");
    let message = refused["errors"][0]["message"].as_str().unwrap();
    assert!(message.contains(out.to_str().unwrap()));
    assert!(
        message.contains("remove") && message.contains("choose") && message.contains("destination")
    );
    assert!(String::from_utf8_lossy(&repeat.stderr).contains(message));
    assert_eq!(fs::read(&out).unwrap(), before);
    #[cfg(unix)]
    {
        let link = temp.path().join("dangling.json");
        std::os::unix::fs::symlink(temp.path().join("missing"), &link).unwrap();
        let output = cli()
            .arg("target")
            .arg("--renderer")
            .arg(&renderer)
            .arg("--data-dir")
            .arg(&data)
            .args(["--backend", "cpu"])
            .arg("--out")
            .arg(&link)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let refused: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(refused["errors"][0]["code"], "destination_exists");
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }
    fs::write(&renderer, b"changed executable bytes").unwrap();
    let invalid = temp.path().join("bad target.json");
    let rejected = cli()
        .arg("target")
        .arg("--renderer")
        .arg(&renderer)
        .arg("--data-dir")
        .arg(&data)
        .args(["--backend", "cpu"])
        .arg("--out")
        .arg(&invalid)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(!invalid.exists());
    assert_eq!(
        documents::load_target(&out).unwrap_err().code,
        "hash_mismatch"
    );
}
