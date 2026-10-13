#![cfg(unix)]

use serde_json::{Value, json};
use spektrafilm_benchmark::common;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

// Deliberately omit the final newline: the export must preserve stdout bytes.
const PRESET: &str = "schema_version = 2\nname = \"Kodak Gold 200\"\n[parameters.film_render.grain]\nactive = true\nv2_amount = 17.0";
const SHARED_SET: &[&str] = &[
    "camera.auto_exposure=false",
    "camera.exposure_compensation_ev=0",
    "camera.film_format_mm=35",
    "scanner.auto_setup=off",
    "scanner.finish=neutral",
    "magazine_print_color.active=false",
    "io.output_color_space=sRGB",
    "io.output_cctf_encoding=true",
    "io.crop=false",
    "io.upscale_factor=1",
    "settings.preview_mode=false",
    "settings.use_enlarger_lut=false",
    "settings.use_scanner_lut=false",
];

fn configuration(overlay: &[&str]) -> Value {
    let set: Vec<_> = SHARED_SET.iter().chain(overlay).copied().collect();
    json!({
        "presets": ["look.toml"],
        "route": "input > film > print > scan",
        "set_files": [],
        "set": set
    })
}

fn fixture(fail_export: bool) -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("data")).unwrap();
    fs::create_dir_all(root.join("config/presets")).unwrap();
    let body = if fail_export {
        "printf '%s' 'export deliberately rejected' >&2\nexit 7".to_owned()
    } else {
        format!("printf '%s' '{PRESET}'\nprintf '%s' 'stub export diagnostic' >&2\nexit 0")
    };
    let renderer = root.join("renderer");
    fs::write(
        &renderer,
        format!(
            r#"#!/bin/sh
if [ "$#" -ne 7 ] || [ "$1" != preset ] || [ "$2" != show ] || [ "$3" != 'built-in:Kodak Gold 200' ] || [ "$4" != --format ] || [ "$5" != toml ] || [ "$6" != --data-dir ] || [ "$7" != "$SPEKTRAFILM_TEST_DATA_DIR" ]; then
    printf '%s' 'unexpected renderer invocation' >&2
    exit 9
fi
printf '%s\n' "$3" >> "$0.calls"
{body}
"#
        ),
    )
    .unwrap();
    fs::set_permissions(renderer, fs::Permissions::from_mode(0o755)).unwrap();
    temp
}

fn fixtures(root: &Path, destination: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm-bench-fixtures"));
    command
        .arg("--renderer")
        .arg(root.join("renderer"))
        .arg("--data-dir")
        .arg(root.join("data"))
        .arg("--out")
        .arg(root.join(destination))
        .env("SPEKTRAFILM_CONFIG_DIR", root.join("config"))
        .env("SPEKTRAFILM_TEST_DATA_DIR", root.join("data"))
        .env("RUST_LOG", "off")
        .current_dir(root);
    command
}

fn result(output: &Output, code: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Parse the entire stdout stream to require a single structured result.
    serde_json::from_slice(&output.stdout).unwrap()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn directory_contents(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if path.is_dir() {
                entries.insert(relative, None);
                visit(root, &path, entries);
            } else {
                entries.insert(relative, Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

#[test]
fn standard_bundle_is_portable_validated_and_frozen_against_library_and_destination_changes() {
    let temp = fixture(false);
    let root = temp.path();
    let output = fixtures(root, "bundle").output().unwrap();
    let generated = result(&output, 0);
    let bundle = root.join("bundle");
    assert_eq!(generated["schema_version"], 1);
    assert_eq!(generated["command"], "fixtures");
    assert_eq!(generated["execution_status"], "completed");
    assert_eq!(generated["artifact"], json!(bundle));
    assert_eq!(generated["preset"], "look.toml");
    assert_eq!(
        generated["configurations"],
        json!({"color_tone":"color-tone.json", "appearance":"appearance.json"})
    );
    assert_eq!(generated["suites"], json!(["quick.json", "complete.json"]));
    assert_eq!(
        fs::read(bundle.join("look.toml")).unwrap(),
        PRESET.as_bytes()
    );

    let isolated = configuration(&[
        "debug.deactivate_spatial_effects=true",
        "debug.deactivate_stochastic_effects=true",
        "film_render.grain.active=false",
    ]);
    let appearance = configuration(&[
        "debug.deactivate_spatial_effects=false",
        "debug.deactivate_stochastic_effects=false",
    ]);
    for (name, expected) in [
        ("color-tone.json", &isolated),
        ("appearance.json", &appearance),
    ] {
        let path = bundle.join(name);
        assert!(path.is_file());
        assert_eq!(&read_json(&path), expected);
        let validated = Command::new(env!("CARGO_BIN_EXE_spektrafilm-bench"))
            .args(["validate", "--kind", "configuration", "--file"])
            .arg(&path)
            .env("SPEKTRAFILM_CONFIG_DIR", root.join("config"))
            .current_dir(root)
            .output()
            .unwrap();
        assert_eq!(result(&validated, 0)["execution_status"], "completed");
    }

    let files: Vec<_> = ["look.toml", "color-tone.json", "appearance.json"]
        .into_iter()
        .map(|name| json!({"path":name, "sha256":common::sha256_file(&bundle.join(name)).unwrap()}))
        .collect();
    let provenance = read_json(&bundle.join("provenance.json"));
    assert_eq!(
        provenance["standard_configurations"],
        json!({
            "revision":"standard-print-scan-v1",
            "preset_selector":"built-in:Kodak Gold 200",
            "renderer":{
                "path":fs::canonicalize(root.join("renderer")).unwrap(),
                "sha256":common::sha256_file(&root.join("renderer")).unwrap()
            },
            "files":files
        })
    );
    assert!(provenance["natural_scene"].is_null());
    assert!(!bundle.join("natural.json").exists());
    assert!(!bundle.join("natural.KDC").exists());

    let quick = read_json(&bundle.join("quick.json"));
    let complete = read_json(&bundle.join("complete.json"));
    for (suite, ids) in [
        (&quick, vec!["color_chart", "exposure_response"]),
        (
            &complete,
            vec![
                "color_chart",
                "exposure_response",
                "grain_fields",
                "halation_point",
                "edge_response",
                "final_appearance",
            ],
        ),
    ] {
        let cases = suite["cases"].as_array().unwrap();
        assert_eq!(
            cases
                .iter()
                .map(|case| case["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ids
        );
        for case in cases {
            let config = &case["configuration"];
            assert_eq!(config["presets"], json!(["look.toml"]));
            assert!(config.get("film").is_none());
            assert!(config.get("paper").is_none());
            assert_ne!(case["id"], "kodak_dc50_scene");
            assert_ne!(case["id"], "natural_appearance");
        }
        for case in &cases[..2] {
            assert_eq!(case["configuration"], isolated);
            assert_eq!(case["seeds"], json!([0]));
        }
    }
    assert_eq!(
        quick["cases"],
        json!([complete["cases"][0], complete["cases"][1]])
    );
    assert_eq!(
        complete["cases"][2]["configuration"],
        configuration(&[
            "debug.deactivate_spatial_effects=true",
            "debug.deactivate_stochastic_effects=false",
            "film_render.grain.active=true",
            "film_render.grain.engine=procedural",
            "film_render.grain.v2_profile=custom",
            "film_render.grain.v2_mode=noise",
            "film_render.grain.v2_amount=25",
        ])
    );
    for index in [3, 4] {
        assert_eq!(
            complete["cases"][index]["configuration"],
            configuration(&[
                "debug.deactivate_spatial_effects=false",
                "debug.deactivate_stochastic_effects=true",
                "film_render.grain.active=false",
            ])
        );
    }
    let final_appearance = &complete["cases"][5];
    assert_eq!(final_appearance["kind"], "probe");
    assert_eq!(final_appearance["input"]["path"], "chart.png");
    assert_eq!(final_appearance["configuration"], appearance);
    assert_eq!(final_appearance["seeds"], json!([0, 42, 99]));
    assert_eq!(
        final_appearance["measurements"],
        json!(["numeric", "color", "tone", "flip"])
    );
    assert_eq!(final_appearance["regions"], json!([]));
    assert!(final_appearance.get("probe").is_none());

    let library_file = root.join("config/presets/Kodak Gold 200.json");
    let conflicting_preset = serde_json::to_vec(&json!({
        "schema_version":2,
        "name":"Kodak Gold 200",
        "parameters":{"film_render":{"grain":{"active":false,"v2_amount":99.0}}},
        "provenance":{"implementation_version":"test","model_version":"test"}
    }))
    .unwrap();
    fs::write(&library_file, &conflicting_preset).unwrap();
    let library_before = directory_contents(&root.join("config"));
    let shadowed = fixtures(root, "shadowed-bundle").output().unwrap();
    assert_eq!(result(&shadowed, 0)["execution_status"], "completed");
    for name in ["look.toml", "color-tone.json", "appearance.json"] {
        assert_eq!(
            fs::read(root.join("shadowed-bundle").join(name)).unwrap(),
            fs::read(bundle.join(name)).unwrap()
        );
    }
    assert_eq!(fs::read(&library_file).unwrap(), conflicting_preset);
    assert_eq!(directory_contents(&root.join("config")), library_before);
    let calls = fs::read(root.join("renderer.calls")).unwrap();
    assert_eq!(calls, b"built-in:Kodak Gold 200\nbuilt-in:Kodak Gold 200\n");

    let before = directory_contents(&bundle);
    let repeated = fixtures(root, "bundle").output().unwrap();
    let failure = result(&repeated, 1);
    assert_eq!(failure["command"], "fixtures");
    assert_eq!(failure["execution_status"], "failed");
    assert_eq!(failure["errors"][0]["code"], "destination_exists");
    assert_eq!(directory_contents(&bundle), before);
    assert_eq!(directory_contents(&root.join("config")), library_before);
    assert_eq!(fs::read(root.join("renderer.calls")).unwrap(), calls);
}

#[test]
fn renderer_export_failure_returns_structured_error_without_provenance() {
    let temp = fixture(true);
    let root = temp.path();
    let output = fixtures(root, "failed-bundle").output().unwrap();
    let failure = result(&output, 1);
    assert_eq!(failure["schema_version"], 1);
    assert_eq!(failure["command"], "fixtures");
    assert_eq!(failure["execution_status"], "failed");
    assert_eq!(failure["errors"][0]["code"], "preset_export");
    assert!(
        failure["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("export deliberately rejected")
    );
    assert!(!root.join("failed-bundle/provenance.json").exists());
    assert_eq!(
        fs::read(root.join("renderer.calls")).unwrap(),
        b"built-in:Kodak Gold 200\n"
    );
}

#[test]
fn natural_input_and_download_are_mutually_exclusive_before_generation() {
    let temp = fixture(false);
    let root = temp.path();
    let input = root.join("local.KDC");
    fs::write(&input, b"never acquired: conflicting arguments").unwrap();
    let output = fixtures(root, "conflicting-bundle")
        .arg("--natural-input")
        .arg(input)
        .arg("--download-natural")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!root.join("conflicting-bundle").exists());
    assert!(!root.join("renderer.calls").exists());
}
