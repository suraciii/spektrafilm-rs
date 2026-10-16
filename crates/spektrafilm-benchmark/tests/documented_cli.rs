use std::{collections::BTreeSet, process::Command};

#[test]
fn documented_benchmark_commands_use_supported_flags() {
    let documents = [
        ("README.md", include_str!("../../../README.md")),
        (
            "docs/benchmark.md",
            include_str!("../../../docs/benchmark.md"),
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
            if let Some(executable) = words.first().and_then(|word| word.rsplit('/').next()) {
                let binary = match executable {
                    "spektrafilm-bench" => Some(env!("CARGO_BIN_EXE_spektrafilm-bench")),
                    "spektrafilm-bench-fixtures" => {
                        Some(env!("CARGO_BIN_EXE_spektrafilm-bench-fixtures"))
                    }
                    _ => None,
                };
                if let Some(binary) = binary {
                    let subcommand = if executable == "spektrafilm-bench" {
                        words.get(1).filter(|word| !word.starts_with('-')).copied()
                    } else {
                        None
                    };
                    let mut help = Command::new(binary);
                    if let Some(subcommand) = subcommand {
                        help.arg(subcommand);
                    }
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
                    checked.insert((executable.to_owned(), subcommand.map(str::to_owned)));
                }
            }
            command.clear();
        }
    }
    // Protect the documented workflow, not just an empty extraction.
    for subcommand in [
        "validate", "target", "run", "compare", "inspect", "report", "review", "regions",
    ] {
        assert!(checked.contains(&("spektrafilm-bench".into(), Some(subcommand.into()))));
    }
    assert!(checked.contains(&("spektrafilm-bench-fixtures".into(), None)));
}
