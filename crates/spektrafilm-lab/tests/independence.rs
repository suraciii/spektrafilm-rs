//! The instrument links no renderer.
//!
//! Independence is a build property, and a dependency is the cheapest thing to
//! add and the hardest to notice. This asks the resolver rather than a list, so
//! a transitive path through a new dependency is caught too.

use std::process::Command;

/// Crates that carry rendering, a graphics device, a raw decoder, or a native
/// codec bridge.
const FORBIDDEN: &[&str] = &[
    "spektrafilm-core",
    "spektrafilm-gpu",
    "spektrafilm-backend",
    "spektrafilm-raw",
    "spektrafilm-model",
    "wgpu",
];

#[test]
fn the_lab_links_no_renderer_crate() {
    let output = Command::new(env!("CARGO"))
        .args(["tree", "--locked", "--offline", "-p", "spektrafilm-lab"])
        .output()
        .expect("cargo tree must run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8(output.stdout).unwrap();
    for name in FORBIDDEN {
        assert!(
            !tree.contains(&format!("{name} v")),
            "the lab must not link {name}:\n{tree}"
        );
    }
}
