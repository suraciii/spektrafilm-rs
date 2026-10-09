use std::{path::{Path, PathBuf}, process::Command};
use spektrafilm_core::{params::RuntimeParams, runtime::{self, DigestMode}};
use spektrafilm_gpu::cpu_backend::CpuBackend;
use spektrafilm_math::precision::to_f64;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
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
        command.arg("process").arg(&input)
            .arg("--output").arg(scratch.0.join(format!("{name}.png")))
            .arg("--film").arg("kodak_portra_400")
            .arg("--paper").arg("kodak_portra_endura")
            .arg("--params").arg(&params_path)
            .arg("--data-dir").arg(&data)
            .arg("--backend").arg("cpu")
            .arg("--raw-out").arg(&raw)
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS");
        if preserve { command.env("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS", "1"); }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        std::fs::read(raw).unwrap().chunks_exact(8)
            .map(|chunk| f64::from_ne_bytes(chunk.try_into().unwrap())).collect::<Vec<_>>()
    };
    let gui = run(true);
    let batch = run(false);
    let mut photo = runtime::init_params("kodak_portra_400", "kodak_portra_endura", &data).unwrap();
    photo.params = params;
    let reference = photo.into_runtime(DigestMode::PreserveUserEdits).unwrap()
        .process(spektrafilm_core::image_io::load(&input).unwrap().image, &CpuBackend).unwrap();
    let expected: Vec<_> = reference.data.into_iter().map(to_f64).collect();
    assert_eq!(gui, expected, "GUI export must use the edited stock controls");
    assert!(gui.iter().zip(&batch).any(|(a, b)| (a - b).abs() > 1e-5),
        "ordinary batch processing must still apply stock defaults");
}
