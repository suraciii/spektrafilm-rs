//! Offline fixed-reference renders. Browsing never schedules photograph work.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const REFERENCE: &[u8] = include_bytes!("../../assets/preset-reference.jpg");
// Increment when the reference rendering recipe changes independently of the application version.
const RECIPE: &str = "fixed-kiwi-reference-v1-cpu-srgb-320";
const BUNDLE: &str = include_str!("../../assets/preset-thumbnails.json");

#[derive(Serialize, Deserialize)]
struct Thumbnail {
    name: String,
    fingerprint: String,
    png: Vec<u8>,
}
#[derive(Default, Serialize, Deserialize)]
struct Bundle {
    entries: Vec<Thumbnail>,
}

/// Exact document + exact material bytes, not an optional document version hint.
fn fingerprint(preset: &Preset, data_dir: &Path) -> anyhow::Result<String> {
    let mut hash = Sha256::new();
    hash.update(RECIPE);
    hash.update(env!("CARGO_PKG_VERSION"));
    hash.update(REFERENCE);
    hash.update(preset.to_json().map_err(anyhow::Error::msg)?);
    hash.update(serde_json::to_vec(&reference_dependencies(
        preset, data_dir,
    )?)?);
    Ok(format!("{:x}", hash.finalize()))
}

fn reference_dependencies(
    preset: &Preset,
    data_dir: &Path,
) -> anyhow::Result<Vec<crate::photograph::record::Dependency>> {
    let resolved = reference_look(preset, data_dir)?;
    let settings = crate::photograph::record::PhotographSettings {
        preparation: resolved.params.render_preparation_state(),
        params: resolved.params,
        film: resolved.film_name,
        print: resolved.print_name,
        special: serde_json::json!({"film_channel_swap": [0, 1, 2], "print_channel_swap": [0, 1, 2]}),
        raw: serde_json::json!({}),
        rotation: 0,
        applied: None,
    };
    crate::photograph::record::dependencies(&settings, data_dir)
}

fn reference_look(
    preset: &Preset,
    data_dir: &Path,
) -> anyhow::Result<spektrafilm_core::presets::ResolvedPreset> {
    let mut context = RuntimeParams::default();
    context.io.input_color_space = "sRGB".into();
    context.io.input_cctf_decoding = true;
    context.random_seed = 0;
    preset
        .resolve(data_dir, &context)
        .map_err(anyhow::Error::msg)
}

#[derive(Default)]
pub(super) struct Thumbnails {
    // Each embedded image is decoded at most once. None means missing/stale/unreadable.
    textures: BTreeMap<String, Option<egui::TextureHandle>>,
    bundle: Option<Bundle>,
}
impl Thumbnails {
    pub(super) fn prepare(
        &mut self,
        ctx: &egui::Context,
        entries: &[LibraryEntry],
        data_dir: &Path,
    ) {
        for entry in entries
            .iter()
            .filter(|entry| entry.group == PresetGroup::BuiltIn)
        {
            self.textures.entry(entry.name.clone()).or_insert_with(|| {
                let preset = entry.preset.as_ref()?;
                let expected = fingerprint(preset, data_dir).ok()?;
                let bundle = self
                    .bundle
                    .get_or_insert_with(|| serde_json::from_str(BUNDLE).unwrap_or_default());
                let thumbnail = bundle.entries.iter().find(|thumbnail| {
                    thumbnail.name == entry.name && thumbnail.fingerprint == expected
                })?;
                let image = image::load_from_memory(&thumbnail.png).ok()?.to_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                Some(ctx.load_texture(
                    format!("preset-reference:{}:{expected}", entry.name),
                    egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                    egui::TextureOptions::LINEAR,
                ))
            });
        }
    }
    pub(super) fn show(&self, ui: &mut egui::Ui, entry: &LibraryEntry) {
        let texture = self.textures.get(&entry.name).and_then(Option::as_ref);
        let width = 64.0_f32.min(ui.available_width() * 0.4);
        if let Some(texture) = texture {
            let size = texture.size_vec2();
            let label = crate::tr!(
                ui.ctx(),
                "surfaces.presets.discovery_reference_accessible",
                name = &entry.name
            );
            let response = ui.add(
                egui::Image::new((texture.id(), size))
                    .fit_to_exact_size(egui::vec2(width, width * size.y / size.x)),
            );
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_label(label));
        } else {
            ui.add_sized(
                egui::vec2(width, width),
                egui::Label::new(
                    egui::RichText::new(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.presets.discovery_unavailable",
                    ))
                    .weak(),
                )
                .wrap(),
            );
        }
    }

    pub(super) fn reference_credit(ui: &mut egui::Ui) {
        ui.small(crate::i18n::text(
            ui.ctx(),
            "surfaces.presets.discovery_reference",
        ));
        ui.small("Luc Viatour / Lucnix.be · CC BY-SA 2.5");
        ui.hyperlink_to(
            crate::i18n::text(ui.ctx(), "surfaces.presets.discovery_reference_rights"),
            "https://creativecommons.org/licenses/by-sa/2.5/",
        )
        .on_hover_text(
            "https://commons.wikimedia.org/wiki/File:Kiwi_(Actinidia_chinensis)_1_Luc_Viatour.jpg",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asset authoring operation: ignored in normal tests, explicitly run by the integration owner.
    #[test]
    #[ignore = "generates the distributed reference renders using the real CPU pipeline"]
    fn generate_fixed_reference_thumbnails() {
        use spektrafilm_backend::{cpu_backend::CpuBackend, telemetry::ObservationContext};
        use spektrafilm_core::presets::builtin_presets;
        let data_dir = crate::headless::data_dir();
        let rgb = image::load_from_memory(REFERENCE)
            .unwrap()
            .resize(320, 320, image::imageops::FilterType::Lanczos3)
            .to_rgb8();
        let input = spektrafilm_math::image::ImageBuf::from_data(
            rgb.width(),
            rgb.height(),
            rgb.as_raw()
                .iter()
                .map(|value| spektrafilm_math::precision::from_f64(*value as f64 / 255.0))
                .collect(),
        );
        let mut bundle = Bundle::default();
        for preset in builtin_presets()
            .into_iter()
            .filter(|preset| preset.kind() == Ok(PresetKind::Look))
        {
            let resolved = reference_look(&preset, &data_dir).unwrap();
            let snapshot = crate::render::RenderSnapshot::new(
                resolved.params,
                resolved.film_name,
                resolved.print_name,
                data_dir.clone(),
                serde_json::json!({}),
            );
            let runtime = crate::render::build_runtime_observed(
                &snapshot,
                false,
                &ObservationContext::default(),
            )
            .unwrap();
            let output = runtime.process(input.clone(), &CpuBackend).unwrap();
            let params = runtime.params();
            let raster = crate::display::input_display_raster(
                &output,
                &params.io.output_color_space,
                params.io.output_cctf_encoding,
                192,
            )
            .unwrap();
            let pixels: Vec<u8> = raster
                .rgb
                .iter()
                .flat_map(|pixel| {
                    pixel
                        .iter()
                        .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
                })
                .collect();
            let image =
                image::RgbImage::from_raw(raster.size[0] as u32, raster.size[1] as u32, pixels)
                    .unwrap();
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image)
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            bundle.entries.push(Thumbnail {
                name: preset.name.clone(),
                fingerprint: fingerprint(&preset, &data_dir).unwrap(),
                png: png.into_inner(),
            });
        }
        assert!(!bundle.entries.is_empty());
        let destination =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/preset-thumbnails.json");
        std::fs::write(destination, serde_json::to_vec(&bundle).unwrap()).unwrap();
    }

    #[test]
    fn distributed_bundle_contains_current_real_render_for_every_builtin_look() {
        let bundle: Bundle = serde_json::from_str(BUNDLE).unwrap();
        let data_dir = crate::headless::data_dir();
        for preset in spektrafilm_core::presets::builtin_presets()
            .into_iter()
            .filter(|preset| preset.kind() == Ok(PresetKind::Look))
        {
            let expected = fingerprint(&preset, &data_dir).unwrap();
            let image = bundle
                .entries
                .iter()
                .find(|image| image.name == preset.name && image.fingerprint == expected)
                .unwrap_or_else(|| {
                    panic!("regenerate the fixed reference bundle for {}", preset.name)
                });
            let decoded = image::load_from_memory(&image.png).unwrap();
            assert!(decoded.width() > 0 && decoded.height() > 0);
            assert!(decoded.width() <= 192 && decoded.height() <= 192);
        }
    }

    #[test]
    fn reference_fingerprint_tracks_document_and_actual_material_bytes() {
        let preset = spektrafilm_core::presets::builtin_presets().remove(0);
        let data_dir = crate::headless::data_dir();
        let original = fingerprint(&preset, &data_dir).unwrap();
        let mut changed = preset.clone();
        changed.name.push_str(" changed");
        assert_ne!(original, fingerprint(&changed, &data_dir).unwrap());
        let temporary = tempfile::tempdir().unwrap();
        let dependencies = reference_dependencies(&preset, &data_dir).unwrap();
        for dependency in dependencies
            .iter()
            .filter(|dependency| dependency.sha256.is_some())
        {
            assert!(
                !dependency.path.is_absolute(),
                "bundled dependencies must relocate"
            );
            let destination = temporary.path().join(&dependency.path);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::copy(data_dir.join(&dependency.path), destination).unwrap();
        }
        assert_eq!(original, fingerprint(&preset, temporary.path()).unwrap());
        let path = temporary.path().join(format!(
            "profiles/{}.json",
            preset.film_profile.as_ref().unwrap().stock
        ));
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        assert_ne!(original, fingerprint(&preset, temporary.path()).unwrap());
        assert!(fingerprint(&preset, Path::new("/nonexistent-spektrafilm-data")).is_err());
        let material_relative = format!(
            "profiles/{}.json",
            preset.film_profile.as_ref().unwrap().stock
        );
        std::fs::copy(
            data_dir.join(&material_relative),
            temporary.path().join(&material_relative),
        )
        .unwrap();
        assert_eq!(original, fingerprint(&preset, temporary.path()).unwrap());
        let descriptor = dependencies
            .iter()
            .find(|dependency| {
                dependency
                    .path
                    .extension()
                    .is_some_and(|extension| extension == "toml")
            })
            .expect("reference pipeline must identify the selected spectral descriptor");
        let descriptor_path = temporary.path().join(&descriptor.path);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&descriptor_path)
            .unwrap()
            .write_all(b"\n# changed descriptor metadata\n")
            .unwrap();
        assert_ne!(original, fingerprint(&preset, temporary.path()).unwrap());
        std::fs::copy(data_dir.join(&descriptor.path), &descriptor_path).unwrap();
        assert_eq!(original, fingerprint(&preset, temporary.path()).unwrap());
        let lut = dependencies
            .iter()
            .find(|dependency| {
                dependency
                    .path
                    .extension()
                    .is_some_and(|extension| extension == "npy")
            })
            .expect("reference pipeline must identify its selected spectral NPY");
        let lut_path = temporary.path().join(&lut.path);
        let mut bytes = std::fs::read(&lut_path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&lut_path, bytes).unwrap();
        assert_ne!(original, fingerprint(&preset, temporary.path()).unwrap());
        std::fs::remove_file(&lut_path).unwrap();
        assert!(fingerprint(&preset, temporary.path()).is_err());
    }
}
