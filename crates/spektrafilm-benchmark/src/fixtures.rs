use crate::{
    common::{Error, Result, create_destination, dump, sha256_file},
    documents, image_data, target,
};
use image::{Rgb, RgbImage};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

const NATURAL_SOURCE: &str = include_str!("../assets/natural-source.json");
const PRESET_SELECTOR: &str = "built-in:Kodak Gold 200";
fn configuration(spatial: bool, stochastic: bool, overlay: &[&str]) -> Value {
    let mut edits = vec![
        "camera.auto_exposure=false".to_owned(),
        "camera.exposure_compensation_ev=0".into(),
        "camera.film_format_mm=35".into(),
        "scanner.auto_setup=off".into(),
        "scanner.finish=neutral".into(),
        "magazine_print_color.active=false".into(),
        "io.output_color_space=sRGB".into(),
        "io.output_cctf_encoding=true".into(),
        "io.crop=false".into(),
        "io.upscale_factor=1".into(),
        "settings.preview_mode=false".into(),
        "settings.use_enlarger_lut=false".into(),
        "settings.use_scanner_lut=false".into(),
        format!("debug.deactivate_spatial_effects={}", !spatial),
        format!("debug.deactivate_stochastic_effects={}", !stochastic),
    ];
    edits.extend(overlay.iter().map(|edit| (*edit).to_owned()));
    json!({"presets":["look.toml"],"route":"input > film > print > scan","set_files":[],"set":edits})
}
/// Run a renderer invocation, retrying the transient `ETXTBSY` that Linux
/// reports while some process still holds the executable open for writing.
/// Stub renderers are written immediately before they run, and the kernel
/// clears the refusal once the writer closes; a persistent refusal still
/// fails after the bounded retries.
fn renderer_output(command: &mut Command) -> std::io::Result<std::process::Output> {
    for _ in 1..20 {
        match command.output() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            result => return result,
        }
    }
    command.output()
}
fn export_look(renderer: &Path, data_dir: &Path, out: &Path) -> Result<()> {
    let mut command = Command::new(renderer);
    command
        .args([
            "preset",
            "show",
            PRESET_SELECTOR,
            "--format",
            "toml",
            "--data-dir",
        ])
        .arg(data_dir);
    let output = renderer_output(&mut command).map_err(|error| {
        Error::new("preset_export", error.to_string())
            .file(renderer.display().to_string())
            .boundary("renderer")
    })?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let failure = if !output.status.success() {
        Some(format!("Preset export exited with {}", output.status))
    } else if output.stdout.is_empty() {
        Some("Preset export returned empty stdout".to_owned())
    } else {
        std::str::from_utf8(&output.stdout)
            .err()
            .map(|error| format!("Preset export stdout is not UTF-8: {error}"))
    };
    if let Some(message) = failure {
        return Err(
            Error::new("preset_export", format!("{message}; stderr: {stderr}"))
                .file(renderer.display().to_string())
                .boundary("renderer"),
        );
    }
    fs::write(out.join("look.toml"), &output.stdout)?;
    Ok(())
}
/// Prove the selected renderer resolves an authored configuration before publishing it.
fn resolve_configuration(
    renderer: &Path,
    data_dir: &Path,
    out: &Path,
    input: &Path,
    name: &str,
    configuration: &Value,
) -> Result<()> {
    let path = renderer.display().to_string();
    let invalid = |message: String| {
        Error::new("renderer_configuration", message)
            .file(path.clone())
            .boundary("renderer")
    };
    let mut command = Command::new(renderer);
    command
        .arg("process")
        .arg(input)
        .args(["--dry-run", "--backend", "cpu", "--data-dir"])
        .arg(data_dir)
        .args([
            "--input-color-space",
            "sRGB",
            "--input-cctf-decoding",
            "true",
        ]);
    if let Some(route) = configuration["route"].as_str() {
        command.args(["--route", route]);
    }
    for preset in configuration["presets"].as_array().into_iter().flatten() {
        let preset = preset
            .as_str()
            .ok_or_else(|| invalid(format!("{name} preset reference must be a string")))?;
        command.arg("--preset").arg(out.join(preset));
    }
    for assignment in configuration["set"].as_array().into_iter().flatten() {
        let assignment = assignment
            .as_str()
            .ok_or_else(|| invalid(format!("{name} assignment must be a string")))?;
        command.args(["--set", assignment]);
    }
    let output = renderer_output(&mut command)
        .map_err(|error| invalid(format!("{name} resolution failed to start: {error}")))?;
    if !output.status.success() {
        let bytes = &output.stderr;
        let from = bytes.len().saturating_sub(2000);
        return Err(invalid(format!(
            "{name} resolution exited with {}; stderr: {}",
            output.status,
            String::from_utf8_lossy(&bytes[from..]).trim()
        )));
    }
    let resolved: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        invalid(format!(
            "{name} resolution returned invalid JSON: {error}; stdout: {}",
            String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(400)])
        ))
    })?;
    if !resolved["parameters"].is_object() {
        return Err(invalid(format!(
            "{name} resolution returned no effective parameters"
        )));
    }
    Ok(())
}
fn region(id: &str, rect: [usize; 4], families: &[&str], role: &str) -> Value {
    json!({
        "id": id,
        "label": human_label(id),
        "role": role,
        "geometry": {
            "x": rect[0],
            "y": rect[1],
            "width": rect[2],
            "height": rect[3],
            "space": "render",
        },
        "measurements": families,
    })
}
fn human_label(id: &str) -> String {
    id.split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or(String::new(), |c| {
                c.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn make_case(
    id: &str,
    path: &Path,
    families: &[&str],
    regions: Vec<Value>,
    config: Value,
    seeds: &[u32],
    probe: Value,
) -> Result<Value> {
    let mut value = json!({"id":id,"kind":"probe","input":{"kind":"raster","path":path.file_name().and_then(|s|s.to_str()),"sha256":sha256_file(path)?,"color_space":"sRGB","cctf_decoding":true},"configuration":config,"seeds":seeds,"regions":regions,"measurements":families});
    if !probe.is_null() {
        value["probe"] = probe;
    }
    Ok(value)
}
fn save_probe(
    path: &Path,
    width: u32,
    height: u32,
    sample: impl Fn(u32, u32) -> [f64; 3],
) -> Result<()> {
    let mut image = RgbImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = Rgb(sample(x, y).map(|v| (v * 255.0).round_ties_even().clamp(0.0, 255.0) as u8));
    }
    image
        .save(path)
        .map_err(|e| Error::new("image_write", e.to_string()).file(path.display().to_string()))
}
/// Generate controlled authored suites. Natural RAW acquisition occurs only on explicit request.
pub fn create_fixtures(
    renderer: &Path,
    data_dir: &Path,
    out: &Path,
    natural_input: Option<&Path>,
    download_natural: bool,
) -> Result<Value> {
    create_destination(out)?;
    let result = (|| -> Result<Value> {
        export_look(renderer, data_dir, out)?;
        let color_tone = configuration(false, false, &["film_render.grain.active=false"]);
        let appearance = configuration(true, true, &[]);
        documents::validate_shape("configuration", &color_tone)?;
        documents::validate_shape("configuration", &appearance)?;
        dump(&out.join("color-tone.json"), &color_tone)?;
        dump(&out.join("appearance.json"), &appearance)?;
        let colors = [
            [0.45, 0.45, 0.45],
            [0.65, 0.42, 0.32],
            [0.2, 0.55, 0.3],
            [0.1, 0.3, 0.65],
            [0.7, 0.15, 0.12],
            [0.15, 0.65, 0.65],
            [0.65, 0.15, 0.65],
            [0.7, 0.65, 0.1],
        ];
        let path = out.join("chart.png");
        save_probe(&path, 128, 128, |x, y| {
            if y < 32 {
                [x as f64 / 127.0; 3]
            } else {
                colors[x as usize / 16]
            }
        })?;
        save_probe(&out.join("chart_dim.png"), 128, 128, |x, y| {
            if y < 32 {
                [0.92 * x as f64 / 127.0; 3]
            } else {
                colors[x as usize / 16].map(|value| 0.92 * value)
            }
        })?;
        for (name, file) in [
            ("color-tone", "color-tone.json"),
            ("appearance", "appearance.json"),
        ] {
            let configuration = documents::load_json_document(&out.join(file))?;
            resolve_configuration(renderer, data_dir, out, &path, name, &configuration)?;
        }
        let chart = make_case(
            "color_chart",
            &path,
            &["numeric", "color", "tone", "flip"],
            vec![
                region("skin", [16, 32, 16, 64], &["color", "tone"], "skin"),
                region("gray", [0, 32, 16, 64], &["color", "tone"], "neutral"),
            ],
            color_tone.clone(),
            &[0],
            Value::Null,
        )?;
        let path = out.join("exposure.png");
        save_probe(&path, 128, 64, |x, _| {
            [image_data::srgb_encode((0.18 * 2_f64.powi(x as i32 / 16 - 5)).min(1.0)); 3]
        })?;
        let response: Vec<_> = (0..8)
            .map(|i| json!({"exposure_stops":(i as f64)-5.0,"geometry":{"x":i*16+3,"y":3,"width":10,"height":58,"space":"render"}}))
            .collect();
        let tone = make_case(
            "exposure_response",
            &path,
            &["numeric", "color", "tone", "flip"],
            vec![],
            color_tone.clone(),
            &[0],
            json!({"tone_response":response}),
        )?;
        let path = out.join("flat_fields.png");
        save_probe(&path, 128, 128, |x, _| {
            [[0.1, 0.3, 0.5, 0.8][x as usize / 32]; 3]
        })?;
        let grain = make_case(
            "grain_fields",
            &path,
            &["numeric", "color", "flip"],
            (0..4)
                .map(|i| {
                    region(
                        &format!("level_{i}"),
                        [i * 32 + 4, 4, 24, 120],
                        &["grain", "color"],
                        "flat_field",
                    )
                })
                .collect(),
            configuration(
                false,
                true,
                &[
                    "film_render.grain.active=true",
                    "film_render.grain.engine=procedural",
                    "film_render.grain.v2_profile=custom",
                    "film_render.grain.v2_mode=noise",
                    "film_render.grain.v2_amount=25",
                ],
            ),
            &[0, 42, 99],
            Value::Null,
        )?;
        let path = out.join("bright_point.png");
        save_probe(&path, 128, 128, |x, y| {
            [if (x as f64 - 63.5).powi(2) + (y as f64 - 63.5).powi(2) <= 9.0 {
                1.0
            } else {
                0.04
            }; 3]
        })?;
        let point = make_case(
            "halation_point",
            &path,
            &["numeric", "color", "flip", "halation"],
            vec![],
            configuration(true, false, &["film_render.grain.active=false"]),
            &[0],
            json!({"halation":{"center_px":[63.5,63.5],"core_radius_px":3.0,"background_inner_radius_px":45.0,"background_outer_radius_px":58.0,"extent_px":40.0,"bin_width_px":1.0,"domain":"linear_srgb"}}),
        )?;
        let path = out.join("edge.png");
        save_probe(&path, 128, 128, |x, _| [if x < 64 { 0.1 } else { 0.8 }; 3])?;
        let contrast =
            image_data::srgb_decode(204.0 / 255.0) - image_data::srgb_decode(26.0 / 255.0);
        let edge = make_case(
            "edge_response",
            &path,
            &["numeric", "color", "flip", "edge"],
            vec![],
            configuration(true, false, &["film_render.grain.active=false"]),
            &[0],
            json!({"edge":{"axis":"x","plateau_fraction":0.2,"domain":"linear_srgb","input_contrast":contrast,"input_exposure_stops":0.0}}),
        )?;
        let final_appearance = make_case(
            "final_appearance",
            &out.join("chart.png"),
            &["numeric", "color", "tone", "flip"],
            vec![],
            appearance.clone(),
            &[0, 42, 99],
            Value::Null,
        )?;
        let protocol = json!({"id":"appearance-v1","pixels_per_degree":60.0,"region_sampling_scales":[1.0,4.0,16.0],"grain":{"residual":{"method":"flat_mean","high_pass_radius_px":4.0,"border_px":1},"spectrum":{"window":"hann"}},"hotspots":{"count":6,"min_area_px":1024}});
        let mut quick = json!({"schema_version":1,"id":"quick-probes","protocol":protocol,"cases":[chart,tone]});
        let mut complete = json!({"schema_version":1,"id":"complete-probes","protocol":protocol,"cases":[chart,tone,grain,point,edge,final_appearance]});
        let mut provenance = json!({"schema_version":1,"controlled_images":"Deterministic RGB probes generated by this package; not measured spectral or physical film references.","natural_scene":null,"standard_configurations":{
            "revision":"standard-print-scan-v1",
            "preset_selector":PRESET_SELECTOR,
            "renderer":{"path":fs::canonicalize(renderer)?,"sha256":sha256_file(renderer)?},
            "files":[
                {"path":"look.toml","sha256":sha256_file(&out.join("look.toml"))?},
                {"path":"color-tone.json","sha256":sha256_file(&out.join("color-tone.json"))?},
                {"path":"appearance.json","sha256":sha256_file(&out.join("appearance.json"))?}
            ]
        }});
        let mut suites = vec!["quick.json", "complete.json"];
        if natural_input.is_some() || download_natural {
            let source: Value = serde_json::from_str(NATURAL_SOURCE)?;
            let destination = out.join("natural.KDC");
            if let Some(input) = natural_input {
                fs::copy(input, &destination)?;
            } else {
                let mut response = ureq::get(source["source_url"].as_str().ok_or_else(|| {
                    Error::new("invalid_document", "Natural provenance URL missing")
                })?)
                .config()
                .timeout_global(Some(std::time::Duration::from_secs(120)))
                .build()
                .call()
                .map_err(|e| Error::new("dependency_download", e.to_string()))?;
                let mut reader = response.body_mut().as_reader();
                let mut file = fs::File::create(&destination)?;
                std::io::copy(&mut reader, &mut file)?;
            }
            if sha256_file(&destination)? != source["sha256"].as_str().unwrap_or("") {
                return Err(Error::new(
                    "dependency_hash",
                    "Natural RAW does not match pinned provenance",
                )
                .file(destination.display().to_string()));
            }
            let mut config = color_tone.clone();
            config["raw"] = json!({"white_balance":"as-shot","lens_correction":false});
            if fs::metadata(&destination)?.len()
                != source["bytes"].as_u64().ok_or_else(|| {
                    Error::new("invalid_document", "Natural provenance byte count missing")
                })?
            {
                return Err(Error::new(
                    "dependency_hash",
                    "Natural RAW byte count does not match pinned provenance",
                )
                .file(destination.display().to_string()));
            }
            let natural = json!({"id":"kodak_dc50_scene","kind":"scene","input":{"kind":"raw","adapter":"spektrafilm-raw","path":"natural.KDC","sha256":source["sha256"]},"configuration":config,"seeds":[0],"regions":[region("shadow",[64,64,160,128],&["color","tone"],"shadow"),region("texture",[320,192,160,128],&["color","tone"],"texture")],"measurements":["numeric","color","tone","flip"]});
            let mut natural_appearance = natural.clone();
            natural_appearance["id"] = json!("natural_appearance");
            natural_appearance["configuration"] = appearance.clone();
            natural_appearance["configuration"]["raw"] = config["raw"].clone();
            natural_appearance["seeds"] = json!([0, 42, 99]);
            dump(
                &out.join("natural.json"),
                &json!({"schema_version":1,"id":"natural-scene","protocol":protocol,"cases":[natural]}),
            )?;
            suites.push("natural.json");
            provenance["natural_scene"] = source;
            quick["id"] = json!("quick-probes-and-scene");
            complete["id"] = json!("complete-probes-and-scene");
            quick["cases"].as_array_mut().unwrap().push(natural.clone());
            complete["cases"].as_array_mut().unwrap().push(natural);
            complete["cases"]
                .as_array_mut()
                .unwrap()
                .push(natural_appearance);
        }
        dump(&out.join("quick.json"), &quick)?;
        dump(&out.join("complete.json"), &complete)?;
        let mut target = target::construct(
            renderer,
            data_dir,
            "cpu",
            "baseline",
            json!({"set_files":[],"set":[]}),
            json!({"RAYON_NUM_THREADS":"8","OPENBLAS_NUM_THREADS":"1","OMP_NUM_THREADS":"1"}),
        )?;
        for (name, edits) in [
            ("baseline", vec![]),
            ("candidate", vec!["camera.exposure_compensation_ev=0.25"]),
            ("grain-candidate", vec!["film_render.grain.v2_amount=35"]),
        ] {
            target["id"] = json!(name);
            target["changes"]["set"] = json!(edits);
            dump(&out.join(format!("{name}.json")), &target)?;
            documents::load_target(&out.join(format!("{name}.json")))?;
        }
        let policy = json!({"schema_version":1,"id":"exploratory-color","protocol_digest":documents::protocol_digest(&complete)?,"calibration":{"status":"exploratory","evidence":[]},"rules":[{"id":"skin-color","purpose":"protect","selector":{"case_id":"color_chart","region_id":"skin"},"metric":"delta_e00.p95","max":2.0,"required":true}]});
        dump(&out.join("exploratory-policy.json"), &policy)?;
        for suite in &suites {
            documents::load_suite(&out.join(suite))?;
        }
        documents::load_policy(&out.join("exploratory-policy.json"))?;
        let review: Value = serde_json::from_str(include_str!("../assets/review.example.json"))?;
        let review_path = out.join("review.json");
        dump(&review_path, &review)?;
        crate::review::validate_manifest(&review_path)?;
        provenance["review"] = json!({"path":"review.json","sha256":sha256_file(&review_path)?});
        dump(&out.join("provenance.json"), &provenance)?;
        Ok(
            json!({"schema_version":1,"command":"fixtures","execution_status":"completed","artifact":out,"suites":suites,"preset":"look.toml","configurations":{"color_tone":"color-tone.json","appearance":"appearance.json"},"review":"review.json"}),
        )
    })();
    result.map_err(|error| crate::common::incomplete_destination(error, out, "provenance.json"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    const LOOK: &str = "schema_version = 1\nname = \"Kodak Gold 200\"\n[settings]\nfilm_profile = \"kodak_gold_200\"";

    #[cfg(unix)]
    fn stub_renderer(root: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let renderer = root.join("renderer");
        fs::write(&renderer, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&renderer, fs::Permissions::from_mode(0o755)).unwrap();
        renderer
    }

    #[cfg(unix)]
    fn successful_renderer(root: &Path) -> std::path::PathBuf {
        stub_renderer(
            root,
            &format!(
                "case \"$1\" in\npreset)\n[ \"$#\" -eq 7 ] && [ \"$2\" = show ] && [ \"$3\" = 'built-in:Kodak Gold 200' ] && [ \"$4\" = --format ] && [ \"$5\" = toml ] && [ \"$6\" = --data-dir ] && [ \"$7\" = '{}' ] || exit 2\nprintf '%s' '{LOOK}'\nprintf '%s' 'export diagnostic' >&2\n;;\nprocess)\nprintf '%s' '{{\"parameters\":{{\"seed\":0}}}}'\n;;\n*)\nprintf '%s' 'unexpected renderer invocation' >&2\nexit 9\n;;\nesac",
                root.display()
            ),
        )
    }
    // ETXTBSY is Linux-specific: macOS permits exec of a file that is open for
    // writing, so neither the precondition nor the retry it exercises exists
    // there. The production retry path is therefore only testable on Linux.
    #[cfg(target_os = "linux")]
    #[test]
    fn renderer_spawn_retries_a_file_still_open_for_writing() {
        let temp = tempfile::tempdir().unwrap();
        let renderer = stub_renderer(temp.path(), "exit 0");
        let writer = fs::File::options().write(true).open(&renderer).unwrap();
        // The kernel refuses an exec while the writer is open; the helper must
        // ride that out, and the writer closes while it retries.
        assert_eq!(
            Command::new(&renderer).output().unwrap_err().kind(),
            std::io::ErrorKind::ExecutableFileBusy
        );
        let closer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(60));
            drop(writer);
        });
        let output = renderer_output(&mut Command::new(&renderer)).unwrap();
        assert!(output.status.success());
        closer.join().unwrap();
    }
    #[test]
    fn probes_use_numpy_ties_even_quantization() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("probe.png");
        save_probe(&path, 2, 1, |x, _| [if x == 0 { 0.1 } else { 0.5 }; 3]).unwrap();
        let image = image::open(path).unwrap().to_rgb8();
        assert_eq!(image.get_pixel(0, 0).0, [26; 3]);
        assert_eq!(image.get_pixel(1, 0).0, [128; 3]);
    }
    #[cfg(unix)]
    #[test]
    fn generated_suites_targets_policy_and_provenance_are_complete() {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("fixtures");
        let renderer = successful_renderer(temp.path());
        let result = create_fixtures(&renderer, temp.path(), &out, None, false).unwrap();
        assert_eq!(result["suites"], json!(["quick.json", "complete.json"]));
        assert_eq!(result["schema_version"], 1);
        assert_eq!(result["command"], "fixtures");
        assert_eq!(result["execution_status"], "completed");
        assert_eq!(result["artifact"], json!(out));
        assert_eq!(result["preset"], "look.toml");
        assert_eq!(
            result["configurations"],
            json!({"color_tone":"color-tone.json","appearance":"appearance.json"})
        );
        assert_eq!(fs::read(out.join("look.toml")).unwrap(), LOOK.as_bytes());
        let shared = vec![
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
        let mut isolated_set = shared.clone();
        isolated_set.extend([
            "debug.deactivate_spatial_effects=true",
            "debug.deactivate_stochastic_effects=true",
            "film_render.grain.active=false",
        ]);
        let mut appearance_set = shared.clone();
        appearance_set.extend([
            "debug.deactivate_spatial_effects=false",
            "debug.deactivate_stochastic_effects=false",
        ]);
        let color_tone = documents::load_json_document(&out.join("color-tone.json")).unwrap();
        let appearance = documents::load_json_document(&out.join("appearance.json")).unwrap();
        assert_eq!(
            color_tone,
            json!({"presets":["look.toml"],"route":"input > film > print > scan","set_files":[],"set":isolated_set})
        );
        assert_eq!(
            appearance,
            json!({"presets":["look.toml"],"route":"input > film > print > scan","set_files":[],"set":appearance_set})
        );
        let quick = documents::load_suite(&out.join("quick.json")).unwrap();
        let complete = documents::load_suite(&out.join("complete.json")).unwrap();
        assert_eq!(quick["cases"].as_array().unwrap().len(), 2);
        assert_eq!(complete["cases"].as_array().unwrap().len(), 6);
        assert_eq!(
            complete["cases"]
                .as_array()
                .unwrap()
                .iter()
                .map(|case| case["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "color_chart",
                "exposure_response",
                "grain_fields",
                "halation_point",
                "edge_response",
                "final_appearance"
            ]
        );
        for case in complete["cases"].as_array().unwrap() {
            assert_eq!(case["configuration"]["presets"], json!(["look.toml"]));
            assert!(case["configuration"].get("film").is_none());
            assert!(case["configuration"].get("paper").is_none());
        }
        for index in [0, 1] {
            assert_eq!(quick["cases"][index]["configuration"], color_tone);
            assert_eq!(quick["cases"][index]["seeds"], json!([0]));
        }
        assert_eq!(complete["cases"][5]["configuration"], appearance);
        assert_eq!(complete["cases"][5]["seeds"], json!([0, 42, 99]));
        assert_eq!(complete["cases"][5]["kind"], "probe");
        assert_eq!(complete["cases"][5]["input"], complete["cases"][0]["input"]);
        assert_eq!(complete["cases"][5]["regions"], json!([]));
        assert!(complete["cases"][5].get("probe").is_none());
        assert_eq!(
            complete["cases"][5]["measurements"],
            json!(["numeric", "color", "tone", "flip"])
        );
        let mut grain_set = shared.clone();
        grain_set.extend([
            "debug.deactivate_spatial_effects=true",
            "debug.deactivate_stochastic_effects=false",
            "film_render.grain.active=true",
            "film_render.grain.engine=procedural",
            "film_render.grain.v2_profile=custom",
            "film_render.grain.v2_mode=noise",
            "film_render.grain.v2_amount=25",
        ]);
        assert_eq!(
            complete["cases"][2]["configuration"]["set"],
            json!(grain_set)
        );
        let mut spatial_set = shared;
        spatial_set.extend([
            "debug.deactivate_spatial_effects=false",
            "debug.deactivate_stochastic_effects=true",
            "film_render.grain.active=false",
        ]);
        for index in [3, 4] {
            assert_eq!(
                complete["cases"][index]["configuration"]["set"],
                json!(spatial_set)
            );
            assert_eq!(complete["cases"][index]["seeds"], json!([0]));
        }
        assert_eq!(complete["cases"][2]["seeds"], json!([0, 42, 99]));
        assert_eq!(
            complete["cases"][1]["probe"]["tone_response"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
        assert_eq!(
            complete["cases"][4]["probe"]["edge"]["input_contrast"],
            json!(image_data::srgb_decode(204.0 / 255.0) - image_data::srgb_decode(26.0 / 255.0))
        );
        let policy = documents::load_policy(&out.join("exploratory-policy.json")).unwrap();
        assert_eq!(
            policy["protocol_digest"],
            documents::protocol_digest(&complete).unwrap()
        );
        assert_eq!(policy["calibration"]["status"], "exploratory");
        let candidate = documents::load_target(&out.join("candidate.json")).unwrap();
        assert_eq!(
            candidate["changes"]["set"],
            json!(["camera.exposure_compensation_ev=0.25"])
        );
        assert_eq!(
            candidate["renderer"]["sha256"],
            sha256_file(&renderer).unwrap()
        );
        assert!(!out.join("natural.KDC").exists());
        let provenance = documents::load_json_document(&out.join("provenance.json")).unwrap();
        assert!(provenance["natural_scene"].is_null());
        assert_eq!(
            provenance["standard_configurations"],
            json!({
                "revision":"standard-print-scan-v1",
                "preset_selector":"built-in:Kodak Gold 200",
                "renderer":{"path":fs::canonicalize(&renderer).unwrap(),"sha256":sha256_file(&renderer).unwrap()},
                "files":[
                    {"path":"look.toml","sha256":sha256_file(&out.join("look.toml")).unwrap()},
                    {"path":"color-tone.json","sha256":sha256_file(&out.join("color-tone.json")).unwrap()},
                    {"path":"appearance.json","sha256":sha256_file(&out.join("appearance.json")).unwrap()}
                ]
            })
        );
        let retained_provenance = fs::read(out.join("provenance.json")).unwrap();
        fs::write(&renderer, "#!/bin/sh\nexit 2\n").unwrap();
        assert_eq!(
            create_fixtures(&renderer, temp.path(), &out, None, false)
                .unwrap_err()
                .code,
            "destination_exists"
        );
        assert_eq!(
            fs::read(out.join("provenance.json")).unwrap(),
            retained_provenance
        );
        assert_eq!(fs::read(out.join("look.toml")).unwrap(), LOOK.as_bytes());
    }
    #[cfg(unix)]
    #[test]
    fn supplied_natural_input_must_match_pinned_hash() {
        let temp = tempfile::tempdir().unwrap();
        let renderer = successful_renderer(temp.path());
        let input = temp.path().join("wrong.KDC");
        fs::write(&input, b"not the pinned RAW").unwrap();
        let error = create_fixtures(
            &renderer,
            temp.path(),
            &temp.path().join("fixtures"),
            Some(&input),
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, "dependency_hash", "{}", error.message);
    }

    #[cfg(unix)]
    #[test]
    fn preset_export_failures_leave_no_generated_files() {
        for body in [
            "printf '%s' 'partial preset'; printf '%s' 'export rejected' >&2; exit 2",
            "printf '%s' 'export rejected' >&2; exit 0",
            "printf '\\377'; printf '%s' 'export rejected' >&2; exit 0",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let renderer = stub_renderer(temp.path(), body);
            let out = temp.path().join("fixtures");
            let error = create_fixtures(&renderer, temp.path(), &out, None, false).unwrap_err();
            assert_eq!(error.code, "preset_export");
            assert_eq!(
                error.to_value()["file"],
                json!(renderer.display().to_string())
            );
            assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
        }
        let temp = tempfile::tempdir().unwrap();
        let renderer = temp.path().join("missing-renderer");
        let out = temp.path().join("fixtures");
        let error = create_fixtures(&renderer, temp.path(), &out, None, false).unwrap_err();
        assert_eq!(error.code, "preset_export");
        assert_eq!(
                error.to_value()["file"],
                json!(renderer.display().to_string())
            );
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
    }
    #[cfg(unix)]
    #[test]
    fn unresolved_configuration_blocks_publication() {
        let temp = tempfile::tempdir().unwrap();
        let renderer = stub_renderer(
            temp.path(),
            &format!(
                "case \"$1\" in\npreset)\nprintf '%s' '{LOOK}'\n;;\nprocess)\nprintf '%s' 'unsupported assignment' >&2\nexit 3\n;;\nesac"
            ),
        );
        let out = temp.path().join("fixtures");
        let error = create_fixtures(&renderer, temp.path(), &out, None, false).unwrap_err();
        assert_eq!(error.code, "renderer_configuration");
        assert_eq!(
                error.to_value()["file"],
                json!(renderer.display().to_string())
            );
        assert!(error.message.contains("unsupported assignment"));
        assert_eq!(fs::read(out.join("look.toml")).unwrap(), LOOK.as_bytes());
    }
    #[cfg(unix)]
    #[test]
    fn long_multibyte_diagnostics_stay_inside_the_error_contract() {
        let temp = tempfile::tempdir().unwrap();
        let renderer = stub_renderer(
            temp.path(),
            &format!(
                "case \"$1\" in\npreset)\nprintf '%s' '{LOOK}'\n;;\nprocess)\nprintf '%s' '{}' >&2\nexit 3\n;;\nesac",
                "中".repeat(2000)
            ),
        );
        let out = temp.path().join("fixtures");
        let error = create_fixtures(&renderer, temp.path(), &out, None, false).unwrap_err();
        assert_eq!(error.code, "renderer_configuration");
        assert!(error.message.contains("exit status: 3"));
        assert!(!out.join("provenance.json").exists());
    }
}
