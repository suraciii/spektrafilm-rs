use std::process::Command;

fn output_args(binary: &str, arguments: &[&str]) -> String {
    let result = Command::new(binary)
        .args(arguments)
        .env("RUST_LOG", "off")
        .output()
        .expect("run binary");
    assert!(
        result.status.success(),
        "{} {arguments:?} failed: {}",
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

fn output(binary: &str, argument: &str) -> String {
    output_args(binary, &[argument])
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

#[test]
fn process_help_explains_dry_run() {
    let text = output_args(env!("CARGO_BIN_EXE_spektrafilm"), &["process", "--help"]);
    assert!(
        text.contains("Print effective controls and planned work without writing an image"),
        "process help has no dry-run explanation: {text}"
    );
}

#[cfg(feature = "precision-f64")]
#[test]
fn f64_help_hides_commands_owned_by_other_binaries() {
    let text = output(env!("CARGO_BIN_EXE_spektrafilm-f64"), "--help");
    for command in ["inspect", "compare"] {
        assert!(
            !text
                .lines()
                .any(|line| line.trim_start().starts_with(command)),
            "f64 help advertises moved command {command}: {text}"
        );
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
