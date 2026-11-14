//! The benchmark refuses every endpoint the lab owns, and names the lab.
//!
//! The image comparison in particular is easy to reach for here: this prefix
//! also has a `compare`, and clap would otherwise report it as an unknown
//! subcommand with no mention of the binary that owns it.

use std::process::Command;

fn bench() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-bench"))
}

#[test]
fn endpoints_of_the_instrument_are_refused() {
    for (args, endpoint) in [
        (
            vec!["compare", "images", "--baseline", "a"],
            "compare images",
        ),
        (vec!["analyze", "--source", "x"], "analyze"),
        (vec!["regions", "grid"], "regions"),
        (vec!["schema", "--kind", "regions"], "schema"),
        (vec!["validate", "--kind", "regions"], "validate"),
    ] {
        let output = bench().args(&args).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(
            stderr.starts_with("argument_syntax:"),
            "{args:?} must report argument_syntax: {stderr}"
        );
        assert!(
            stderr.contains("spektrafilm-lab"),
            "{args:?} must name spektrafilm-lab: {stderr}"
        );
        assert!(
            stderr.contains(endpoint),
            "{args:?} must name {endpoint}: {stderr}"
        );
    }
}

/// The endpoints this binary does own still parse, so the guard cannot swallow
/// them.
#[test]
fn owned_endpoints_still_reach_their_command() {
    for args in [
        vec!["compare", "runs", "--help"],
        vec!["compare", "readings", "--help"],
        vec!["compare", "pairs", "--help"],
        vec!["sample", "--help"],
        vec!["inspect", "--help"],
        vec!["report", "comparison", "--help"],
        vec!["report", "photos", "--help"],
    ] {
        let output = bench().args(&args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?} must parse: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
