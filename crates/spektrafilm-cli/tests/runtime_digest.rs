use spektrafilm_core::{
    params::RuntimeParams,
    runtime::{self, DigestMode},
};
use spektrafilm_gpu::cpu_backend::CpuBackend;
use spektrafilm_math::precision::to_f64;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn gui_child_preserves_edits_while_batch_applies_stock_specifics() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "spektrafilm-digest-policy-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    )));
    std::fs::create_dir_all(&scratch.0).unwrap();
    let input = scratch.0.join("input.png");
    let pixels = [40_u8, 90, 170].repeat(16);
    image::save_buffer(&input, &pixels, 4, 4, image::ColorType::Rgb8).unwrap();
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.debug.deactivate_spatial_effects = true;
    params.debug.deactivate_stochastic_effects = true;
    params.film_render.dir_couplers.gamma_samelayer_rgb = [0.9, 0.8, 0.7];
    let params_path = scratch.0.join("params.json");
    std::fs::write(&params_path, serde_json::to_vec(&params).unwrap()).unwrap();
    let run = |preserve: bool| {
        let name = if preserve { "gui" } else { "batch" };
        let raw = scratch.0.join(format!("{name}.raw"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm"));
        command
            .arg("process")
            .arg(&input)
            .arg("--output")
            .arg(scratch.0.join(format!("{name}.png")))
            .arg("--film")
            .arg("kodak_portra_400")
            .arg("--paper")
            .arg("kodak_portra_endura")
            .arg("--params")
            .arg(&params_path)
            .arg("--data-dir")
            .arg(&data)
            .arg("--backend")
            .arg("cpu")
            .arg("--raw-out")
            .arg(&raw)
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS");
        if preserve {
            command.env("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS", "1");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::read(raw)
            .unwrap()
            .chunks_exact(8)
            .map(|chunk| f64::from_ne_bytes(chunk.try_into().unwrap()))
            .collect::<Vec<_>>()
    };
    let gui = run(true);
    let batch = run(false);
    let mut photo = runtime::init_params("kodak_portra_400", "kodak_portra_endura", &data).unwrap();
    photo.params = params;
    let reference = photo
        .into_runtime(DigestMode::PreserveUserEdits)
        .unwrap()
        .process(
            spektrafilm_core::image_io::load(&input).unwrap().image,
            &CpuBackend,
        )
        .unwrap();
    let expected: Vec<_> = reference.data.into_iter().map(to_f64).collect();
    assert_eq!(
        gui, expected,
        "GUI export must use the edited stock controls"
    );
    assert!(
        gui.iter().zip(&batch).any(|(a, b)| (a - b).abs() > 1e-5),
        "ordinary batch processing must still apply stock defaults"
    );
}

#[test]
fn bounded_png_recipe_publishes_an_eight_bit_preview() {
    use sha2::{Digest, Sha256};
    use tiff::encoder::{TiffEncoder, colortype::RGB32Float};

    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "spektrafilm-png-recipe-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    )));
    std::fs::create_dir_all(&scratch.0).unwrap();
    let input = scratch.0.join("input.tiff");
    let pixels = [0.1_f32, 0.2, 0.3].repeat(8);
    {
        let mut encoder = TiffEncoder::new(std::fs::File::create(&input).unwrap()).unwrap();
        encoder.write_image::<RGB32Float>(4, 2, &pixels).unwrap();
    }
    let recipe = serde_json::json!({
        "schemaVersion": "spektrafilm-rs-params-1",
        "filmProfile": "kodak_portra_400",
        "printProfile": "kodak_portra_endura",
        "parameters": {
            "camera": {"auto_exposure": false},
            "debug": {"deactivate_spatial_effects": true, "deactivate_stochastic_effects": true}
        },
        "output": {
            "format": "png",
            "precisionBits": 8,
            "colorSpace": "sRGB",
            "transferFunction": "srgb",
            "geometry": "bounded",
            "encoding": "bounded-preview",
            "maxEdge": 2
        },
        "seed": 42
    });
    let recipe_path = scratch.0.join("recipe.json");
    std::fs::write(&recipe_path, serde_json::to_vec(&recipe).unwrap()).unwrap();
    let output_path = scratch.0.join("preview.png");
    let output = Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
        .arg("render")
        .arg("--input")
        .arg(&input)
        .arg("--recipe")
        .arg(&recipe_path)
        .arg("--output")
        .arg(&output_path)
        .arg("--data-dir")
        .arg(&data)
        .env("SPEKTRAFILM_BACKEND", "cpu")
        .env("RUST_LOG", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let facts: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let bytes = std::fs::read(&output_path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let preview = image::open(&output_path).unwrap();
    assert_eq!((preview.width(), preview.height()), (2, 1));
    assert_eq!(preview.color(), image::ColorType::Rgb8);
    assert_eq!(facts["terminalOutcome"], "succeeded");
    assert_eq!(facts["input"]["width"], 4);
    assert_eq!(facts["input"]["height"], 2);
    assert_eq!(facts["output"]["format"], "png");
    assert_eq!(facts["output"]["width"], 2);
    assert_eq!(facts["output"]["height"], 1);
    assert_eq!(facts["output"]["size"], bytes.len());
    assert_eq!(
        facts["output"]["sha256"],
        format!("{:x}", Sha256::digest(&bytes))
    );
    assert_eq!(facts["seed"], 42);
    assert!(!std::fs::read_dir(&scratch.0).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".spektrafilm-")
    }));
}

#[test]
fn lut_cli_applies_legacy_exposure_once() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "spektrafilm-lut-exposure-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    )));
    for (ev, stops, gain) in [("0", 4.0, 2.88), ("1", 5.0, 5.76)] {
        let output = Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
            .args([
                "lut",
                "build",
                "--name",
                ev,
                "--film",
                "kodak_portra_400",
                "--print",
                "kodak_portra_endura",
                "--input",
                "srgb",
                "--output",
                "srgb",
                "--resolution",
                "2",
                "--stops-above-midgray",
                "4",
                "--exposure-ev",
                ev,
            ])
            .arg(&scratch.0)
            .arg("--data-dir")
            .arg(&data)
            .env("SPEKTRAFILM_BACKEND", "cpu")
            .env("RUST_LOG", "off")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(scratch.0.join(ev).join("bundle.json")).unwrap())
                .unwrap();
        let exposure = &metadata["input_exposure"];
        assert_eq!(exposure["stops_above_midgray"], stops);
        assert_eq!(exposure["exposure_ev"], ev.parse::<f64>().unwrap());
        assert!((exposure["gain"].as_f64().unwrap() - gain).abs() < 1e-12);
    }
}
