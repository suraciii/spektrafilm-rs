//! The lab refuses every endpoint another binary owns, and names the owner.
//!
//! The two research binaries share one endpoint vocabulary. An invocation that
//! reaches the wrong binary must say which one owns it, because a caller
//! reading a script's failure has no other way to find out.

#![cfg(feature = "precision-f64")]

use std::process::Command;

fn lab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
}

fn rejected(args: &[&str], owner: &str, endpoint: &str) {
    let output = lab().args(args).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{args:?} must be an argument syntax rejection: {stderr}"
    );
    assert!(
        stderr.contains("\" argument_syntax".trim_start())
            || stderr.starts_with("argument_syntax:"),
        "{args:?} must report argument_syntax: {stderr}"
    );
    assert!(
        stderr.contains(owner),
        "{args:?} must name {owner}: {stderr}"
    );
    assert!(
        stderr.contains(endpoint),
        "{args:?} must name {endpoint}: {stderr}"
    );
}

#[test]
fn endpoints_of_the_renderer_are_refused() {
    for (args, endpoint) in [
        (vec!["measure", "run"], "measure"),
        (vec!["spectral", "evaluate"], "spectral"),
        (vec!["fit"], "fit"),
    ] {
        rejected(&args, "spektrafilm-f64", endpoint);
    }
}
