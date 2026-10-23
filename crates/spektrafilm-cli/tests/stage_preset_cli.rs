use serde_json::Value;
use spektrafilm_core::{
    params::{GrainEngine, RuntimeParams},
    presets::{Preset, Provenance},
};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn run(root: &Path, input: &str, flags: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
        .current_dir(root)
        .arg("process")
        .arg(input)
        .args(["--data-dir"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
        .args(["--backend", "cpu"])
        .args(flags)
        .env("RUST_LOG", "off")
        .output()
        .unwrap()
}
fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn provenance() -> Provenance {
    Provenance {
        implementation_version: "test".into(),
        model_version: spektrafilm_core::runtime::MODEL_VERSION.into(),
        scanner: None,
    }
}

#[test]
fn look_and_authored_sparse_filters_match_onepass_print_boundary_and_atomic_grain_section_applies()
{
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    image::save_buffer(
        root.join("input.png"),
        &[90; 8 * 6 * 3],
        8,
        6,
        image::ColorType::Rgb8,
    )
    .unwrap();
    let mut params = RuntimeParams::default();
    params.film_render.grain.active = false;
    let preset = Preset::capture(
        "Authored filters",
        "kodak_portra_400",
        "kodak_portra_endura",
        &params,
        provenance(),
    );
    fs::write(root.join("look.json"), preset.to_json().unwrap()).unwrap();
    let image = [
        "--preset",
        "look.json",
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--set",
        "enlarger.c_filter_neutral=13,enlarger.m_filter_neutral=20",
    ];
    success(run(
        root,
        "input.png",
        &[
            image.as_slice(),
            &[
                "--route",
                "input > film > print",
                "-o",
                "print.sfstage",
                "--diagnostics",
                "stage.json",
            ],
        ]
        .concat(),
    ));
    let state = spektrafilm_core::stage_artifact::read_header(root.join("print.sfstage")).unwrap();
    assert_eq!(state.baseline.enlarger.c_filter_neutral, 13.0);
    assert_eq!(state.baseline.enlarger.m_filter_neutral, 20.0);
    let restatement = format!(
        "enlarger.y_filter_neutral={}",
        state.baseline.enlarger.y_filter_neutral
    );
    success(run(
        root,
        "print.sfstage",
        &[
            "--route",
            "print > scan",
            "--set",
            &restatement,
            "--dry-run",
        ],
    ));
    success(run(
        root,
        "print.sfstage",
        &[
            "--route",
            "print > scan",
            "-o",
            "split.tif",
            "--raw-out",
            "split.raw",
            "--diagnostics",
            "continued.json",
        ],
    ));
    success(run(
        root,
        "input.png",
        &[
            image.as_slice(),
            &[
                "--route",
                "input > film > print > scan",
                "-o",
                "direct.tif",
                "--raw-out",
                "direct.raw",
            ],
        ]
        .concat(),
    ));
    assert_eq!(
        fs::read(root.join("split.raw")).unwrap(),
        fs::read(root.join("direct.raw")).unwrap()
    );
    for report in ["stage.json", "continued.json"] {
        let report: Value = serde_json::from_slice(&fs::read(root.join(report)).unwrap()).unwrap();
        assert!(!report["configuration"]["render"].is_null());
    }
    // Form real material grain once, then switch only the remaining appearance
    // with an atomic section containing both active and engine leaves.
    success(run(
        root,
        "input.png",
        &[
            "--film",
            "kodak_portra_400",
            "--input-color-space",
            "sRGB",
            "--input-cctf-decoding",
            "true",
            "--route",
            "input > film",
            "-o",
            "grain.sfstage",
        ],
    ));
    let mut completed = spektrafilm_core::stage_artifact::read_header(root.join("grain.sfstage"))
        .unwrap()
        .baseline;
    completed.film_render.grain.engine = GrainEngine::Procedural;
    completed.film_render.grain.active = false;
    let section = Preset::capture_section(
        "Postscan appearance",
        spektrafilm_core::presets::Section::Grain,
        &completed,
        provenance(),
    )
    .unwrap();
    fs::write(root.join("grain.json"), section.to_json().unwrap()).unwrap();
    success(run(
        root,
        "grain.sfstage",
        &[
            "--route",
            "film > scan",
            "--preset",
            "grain.json",
            "--dry-run",
        ],
    ));
}
