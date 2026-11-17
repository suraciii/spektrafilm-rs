use std::process::Command;

fn output(binary: &str, argument: &str) -> String {
    let result = Command::new(binary)
        .arg(argument)
        .env("RUST_LOG", "off")
        .output()
        .expect("run binary");
    assert!(
        result.status.success(),
        "{} {argument} failed: {}",
        binary,
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        result.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8_lossy(&result.stdout).into_owned()
}

#[test]
fn version_reports_the_compiled_host_precision() {
    let expected = spektrafilm_backend::Precision::HOST.name();
    let text = output(env!("CARGO_BIN_EXE_spektrafilm"), "--version");
    assert!(
        text.contains(&format!("native CPU {expected}")),
        "version did not report native CPU {expected}: {text}"
    );
}

#[test]
fn help_exposes_the_public_command_surface() {
    let text = output(env!("CARGO_BIN_EXE_spektrafilm"), "--help");
    for command in ["process", "list-profiles", "describe", "preset"] {
        assert!(text.contains(command), "help missing {command}: {text}");
    }
}

#[cfg(feature = "precision-f64")]
#[test]
fn f64_alias_reports_the_f64_reference_precision() {
    let text = output(env!("CARGO_BIN_EXE_spektrafilm-f64"), "--version");
    assert!(
        text.contains("native CPU f64"),
        "f64 alias did not report f64: {text}"
    );
}
