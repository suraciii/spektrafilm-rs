//! The f64 build refuses every endpoint another binary owns, and names it.
//!
//! This build carries both the renderer's commands and the research surface it
//! absorbed, so it is the one most likely to name the wrong owner.

use std::process::Command;

fn f64_binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-f64"))
}

fn rejected(args: &[&str], owner: &str, endpoint: &str) {
    let output = f64_binary().args(args).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{args:?}: {stderr}");
    assert!(
        stderr.starts_with("argument_syntax:"),
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
fn endpoints_of_the_lab_are_refused() {
    for (args, endpoint) in [
        (vec!["analyze"], "analyze"),
        (vec!["schema"], "schema"),
        (vec!["validate"], "validate"),
        (vec!["regions"], "regions"),
    ] {
        rejected(&args, "spektrafilm-lab", endpoint);
    }
}

/// The reading family and the evaluation surface moved past the lab, so naming
/// the lab here would send a caller to a binary that also refuses.
#[test]
fn endpoints_of_the_benchmark_are_refused_naming_the_benchmark() {
    for (args, endpoint) in [
        (vec!["sample"], "sample"),
        (vec!["report"], "report"),
        (vec!["compare", "runs", "--baseline", "a"], "compare runs"),
        (
            vec!["compare", "readings", "--baseline", "a"],
            "compare readings",
        ),
        (vec!["compare", "pairs", "--run", "a"], "compare pairs"),
    ] {
        rejected(&args, "spektrafilm-bench", endpoint);
    }
}
