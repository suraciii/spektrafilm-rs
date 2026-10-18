#![cfg(feature = "precision-f64")]
use std::{collections::BTreeSet, process::Command};

#[test]
fn documented_instrument_commands_use_supported_flags() {
    let documents = [
        ("README.md", include_str!("../../../README.md")),
        (
            "docs/measurement.md",
            include_str!("../../../docs/measurement.md"),
        ),
        (
            "docs/benchmark.md",
            include_str!("../../../docs/benchmark.md"),
        ),
        (
            "docs/profile-fitting.md",
            include_str!("../../../docs/profile-fitting.md"),
        ),
    ];
    let mut checked = BTreeSet::new();
    for (name, document) in documents {
        let mut bash = false;
        let mut command = String::new();
        for line in document.lines() {
            if line.starts_with("```") {
                bash = line == "```bash";
                continue;
            }
            if !bash {
                continue;
            }
            let line = line.trim();
            command.push_str(line.trim_end_matches('\\'));
            command.push(' ');
            if line.ends_with('\\') {
                continue;
            }
            let words: Vec<_> = command.split_whitespace().collect();
            // Only execute the blocks the instrument itself documents; blocks
            // that start with another program are not this binary's contract.
            if words
                .first()
                .map(|word| word.rsplit('/').next().unwrap_or(word))
                != Some("spektrafilm-lab")
            {
                command.clear();
                continue;
            }
            let binary = env!("CARGO_BIN_EXE_spektrafilm-lab");
            let command_path: Vec<_> = words
                .iter()
                .skip(1)
                .take_while(|word| !word.starts_with('-'))
                .copied()
                .collect();
            let subcommand = command_path.first().copied();
            let mut help = Command::new(binary);
            help.args(&command_path);
            let output = help.arg("--help").output().unwrap();
            assert!(
                output.status.success(),
                "{name}: unsupported command: {command}"
            );
            let help = String::from_utf8(output.stdout).unwrap();
            let supported: BTreeSet<_> = help.split_whitespace().collect();
            for flag in words.iter().filter(|word| word.starts_with("--")) {
                let flag = flag.split('=').next().unwrap();
                assert!(
                    supported.contains(flag),
                    "{name}: unsupported flag {flag} in {command}"
                );
            }
            checked.insert(subcommand.map(str::to_owned));
            command.clear();
        }
    }
    // Protect the documented workflow, not just an empty extraction.
    for subcommand in [
        "validate", "target", "run", "compare", "inspect", "report", "regions", "fixtures", "fit",
    ] {
        assert!(checked.contains(&Some(subcommand.into())), "{subcommand}");
    }
}
