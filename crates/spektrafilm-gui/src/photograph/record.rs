//! Accepted photograph state; external pixels and model assets remain external.
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spektrafilm_core::{
    params::{RenderPreparationState, RuntimeParams},
    profile,
};

use crate::app::Message;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PhotographSettings {
    pub(crate) params: RuntimeParams,
    pub(crate) preparation: RenderPreparationState,
    pub(crate) film: String,
    pub(crate) print: String,
    pub(crate) special: serde_json::Value,
    pub(crate) raw: serde_json::Value,
    pub(crate) rotation: i32,
    #[serde(deserialize_with = "deserialize_applied")]
    pub(crate) applied: Option<crate::presets::AppliedPreset>,
}

fn deserialize_applied<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<crate::presets::AppliedPreset>, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Context {
        group: spektrafilm_core::presets::PresetGroup,
        name: String,
    }
    Ok(
        Option::<Context>::deserialize(deserializer)?.map(|context| {
            crate::presets::AppliedPreset {
                group: context.group,
                name: context.name,
            }
        }),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceIdentity {
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
}

/// Relative assets relocate with the installation; an external LUT selected by
/// a descriptor retains its absolute path. Optional data records absence too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Dependency {
    pub(crate) path: PathBuf,
    pub(crate) sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PhotographRecord {
    schema_version: u32,
    kind: String,
    runtime_model: String,
    seed: u64,
    pub(crate) source: SourceIdentity,
    pub(crate) settings: PhotographSettings,
    pub(crate) dependencies: Vec<Dependency>,
    raw_decoder: Option<String>,
    lens_model: Option<String>,
}

impl PhotographRecord {
    pub(crate) fn capture(
        source: &Path,
        mut settings: PhotographSettings,
        data_dir: &Path,
    ) -> Result<Self> {
        settings
            .params
            .restore_render_preparation_state(&settings.preparation);
        validate_settings(&settings)?;
        let source = absolute(source).map_err(|e| {
            Message::new("operations.record.source_read")
                .arg("path", source.display())
                .context(e)
        })?;
        let raw = spektrafilm_core::image_io::is_raw(&source);
        let lens_model = if raw && settings.raw["lens_correction"] == true {
            Some(lens_identity(&source)?)
        } else {
            None
        };
        let record = Self {
            schema_version: 1,
            kind: "photograph_adjustment_record".into(),
            runtime_model: spektrafilm_core::runtime::MODEL_VERSION.into(),
            seed: settings.params.random_seed,
            source: SourceIdentity {
                sha256: source_digest(&source)?,
                path: source,
            },
            dependencies: dependencies(&settings, data_dir)?,
            raw_decoder: raw.then(spektrafilm_raw::decoder_version),
            lens_model,
            settings,
        };
        record.validate_format()?;
        Ok(record)
    }

    /// Compare the model selected by the accepted RAW decode, not a fresh
    /// lookup. Call before saving and after decoding a record for reopening.
    pub(crate) fn validate_accepted_lens_model(&self, expected: Option<&str>) -> Result<()> {
        let accepted =
            expected.map(|identity| format!("{:x}", Sha256::digest(identity.as_bytes())));
        ensure!(
            accepted == self.lens_model,
            Message::new("operations.record.lens_changed")
        );
        Ok(())
    }

    /// Publish only after serialization, validation, all writes and fsync succeed.
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        self.validate_format()?;
        let destination = destination_path(path)?;
        ensure!(
            destination != self.source.path,
            Message::new("operations.record.protected_destination").arg("path", path.display())
        );
        // A record must not replace a material or preset document, either.
        if destination.is_file() {
            let bytes = read_regular(&destination).map_err(|e| {
                Message::new("operations.record.write")
                    .arg("path", path.display())
                    .context(e)
            })?;
            Self::decode(&bytes).map_err(|e| {
                Message::new("operations.record.replace_record_only")
                    .arg("path", path.display())
                    .context(e)
            })?;
        }
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| Message::new("operations.record.format").context(e))?;
        let parent = destination.parent().unwrap_or(Path::new("."));
        let result = (|| -> std::io::Result<()> {
            let mut staged = tempfile::Builder::new()
                .prefix(".photograph-record-")
                .suffix(".tmp")
                .tempfile_in(parent)?;
            staged.write_all(&bytes)?;
            staged.as_file().sync_all()?;
            staged.persist(&destination).map_err(|e| e.error)?;
            Ok(())
        })();
        result.map_err(|e| {
            Message::new("operations.record.write")
                .arg("path", path.display())
                .context(e)
        })
    }

    pub(crate) fn load(path: &Path) -> Result<Self> {
        let bytes = read_regular(path).map_err(|e| {
            Message::new("operations.record.read")
                .arg("path", path.display())
                .context(e)
        })?;
        Self::decode(&bytes).map_err(|e| {
            Message::new("operations.record.read")
                .arg("path", path.display())
                .context(e)
        })
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut record: Self = serde_json::from_slice(bytes)
            .map_err(|e| Message::new("operations.record.format").context(e))?;
        let carrier: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| Message::new("operations.record.format").context(e))?;
        let canonical = serde_json::to_value(&record.settings.params)?;
        ensure!(
            complete_shape(&carrier["settings"]["params"], &canonical),
            Message::new("operations.record.parameters")
        );
        record.settings.params.random_seed = record.seed;
        record
            .settings
            .params
            .restore_render_preparation_state(&record.settings.preparation);
        record.validate_format()?;
        Ok(record)
    }

    /// An override is an explicit user selection, never a same-name search.
    pub(crate) fn validate(
        &self,
        source_override: Option<&Path>,
        data_dir: &Path,
    ) -> Result<PathBuf> {
        self.validate_format()?;
        let source = source_override.unwrap_or(&self.source.path);
        ensure!(
            source_digest(source)? == self.source.sha256,
            Message::new("operations.record.source_changed").arg("path", source.display())
        );
        ensure!(
            spektrafilm_core::image_io::is_raw(source) == self.raw_decoder.is_some(),
            Message::new("operations.record.source_interpretation").arg("path", source.display())
        );
        ensure!(
            dependencies(&self.settings, data_dir)? == self.dependencies,
            Message::new("operations.record.dependencies_changed")
        );
        if let Some(decoder) = &self.raw_decoder {
            ensure!(
                decoder == &spektrafilm_raw::decoder_version(),
                Message::new("operations.record.decoder_changed")
            );
        }
        if let Some(model) = &self.lens_model {
            ensure!(
                &lens_identity(source)? == model,
                Message::new("operations.record.lens_changed")
            );
        }
        absolute(source).map_err(|e| {
            Message::new("operations.record.source_read")
                .arg("path", source.display())
                .context(e)
        })
    }

    fn validate_format(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.kind == "photograph_adjustment_record",
            Message::new("operations.record.version").arg("version", self.schema_version)
        );
        ensure!(
            self.runtime_model == spektrafilm_core::runtime::MODEL_VERSION,
            Message::new("operations.record.model_changed")
        );
        ensure!(
            self.source.path.is_absolute() && valid_digest(&self.source.sha256),
            Message::new("operations.record.source_identity")
        );
        ensure!(
            self.seed == self.settings.params.random_seed,
            Message::new("operations.record.seed")
        );
        validate_settings(&self.settings)?;
        let mut paths = BTreeSet::new();
        ensure!(
            !self.dependencies.is_empty()
                && self.dependencies.iter().all(|d| {
                    let normalized = !d.path.as_os_str().is_empty()
                        && d.path.components().all(|c| {
                            !matches!(
                                c,
                                std::path::Component::ParentDir | std::path::Component::CurDir
                            )
                        });
                    normalized
                        && paths.insert(&d.path)
                        && d.sha256.as_ref().is_none_or(|s| valid_digest(s))
                        && (d.sha256.is_some()
                            || d.path == Path::new("filters/neutral_print_filters.json"))
                }),
            Message::new("operations.record.dependency_manifest")
        );
        ensure!(
            self.raw_decoder.is_some() == spektrafilm_core::image_io::is_raw(&self.source.path)
                && self.raw_decoder.as_ref().is_none_or(|s| !s.is_empty())
                && self.lens_model.is_some()
                    == (self.raw_decoder.is_some() && self.settings.raw["lens_correction"] == true)
                && self.lens_model.as_ref().is_none_or(|s| valid_digest(s)),
            Message::new("operations.record.raw_dependencies")
        );
        Ok(())
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Compare the serialized canonical type, not a second parameter schema. Typed
/// decoding owns values and unknown fields; this boundary rejects every field
/// that serde would otherwise silently supply from a default.
fn complete_shape(actual: &serde_json::Value, canonical: &serde_json::Value) -> bool {
    match (actual, canonical) {
        (serde_json::Value::Object(actual), serde_json::Value::Object(canonical)) => {
            actual.len() == canonical.len()
                && canonical.iter().all(|(key, value)| {
                    actual
                        .get(key)
                        .is_some_and(|actual| complete_shape(actual, value))
                })
        }
        (serde_json::Value::Array(actual), serde_json::Value::Array(canonical)) => {
            actual.len() == canonical.len()
                && actual
                    .iter()
                    .zip(canonical)
                    .all(|(actual, canonical)| complete_shape(actual, canonical))
        }
        _ => !actual.is_object() && !canonical.is_object(),
    }
}

fn open_regular(path: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?
    };
    #[cfg(not(unix))]
    let file = {
        if !std::fs::metadata(path)?.is_file() {
            return Err(std::io::Error::other("not a regular file"));
        }
        File::open(path)?
    };
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    Ok(file)
}

fn read_regular(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open_regular(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn digest(path: &Path) -> std::io::Result<String> {
    let mut file = open_regular(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub(crate) fn source_digest(path: &Path) -> Result<String> {
    digest(path).map_err(|e| {
        Message::new("operations.record.source_read")
            .arg("path", path.display())
            .context(e)
    })
}
fn absolute(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}
fn destination_path(path: &Path) -> Result<PathBuf> {
    let result = if path.exists() {
        absolute(path)
    } else {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        absolute(parent).and_then(|p| {
            path.file_name()
                .map(|name| p.join(name))
                .ok_or_else(|| std::io::Error::other("missing file name"))
        })
    };
    result.map_err(|e| {
        Message::new("operations.record.write")
            .arg("path", path.display())
            .context(e)
    })
}
fn lens_identity(source: &Path) -> Result<String> {
    let model = spektrafilm_raw::lens_model_identity(source).map_err(|e| {
        Message::new("operations.record.lens_read")
            .arg("path", source.display())
            .context(e)
    })?;
    Ok(format!("{:x}", Sha256::digest(model.as_bytes())))
}

fn validate_settings(settings: &PhotographSettings) -> Result<()> {
    // The canonical typed roundtrip rejects non-finite values serialized as null.
    let mut params: RuntimeParams = serde_json::from_value(serde_json::to_value(&settings.params)?)
        .map_err(|e| Message::new("operations.record.parameters").context(e))?;
    params.restore_render_preparation_state(&settings.preparation);
    crate::state::validate_stored_runtime(&mut params)
        .map_err(|e| Message::new("operations.record.parameters").context(e))?;
    ensure!(
        params.render_preparation_state() == settings.preparation,
        Message::new("operations.record.preparation")
    );
    params.random_seed = settings.params.random_seed;
    ensure!(
        params == settings.params,
        Message::new("operations.record.parameters")
    );
    ensure!(
        (0..4).contains(&settings.rotation),
        Message::new("operations.record.rotation").arg("value", settings.rotation)
    );
    for (name, value, keys) in [
        (
            "special",
            &settings.special,
            &["film_channel_swap", "print_channel_swap"][..],
        ),
        (
            "raw",
            &settings.raw,
            &["white_balance", "temperature", "tint", "lens_correction"][..],
        ),
    ] {
        ensure!(
            value
                .as_object()
                .is_some_and(|o| o.len() == keys.len() && keys.iter().all(|k| o.contains_key(*k))),
            Message::new("operations.record.settings_object").arg("name", name)
        );
    }
    // Reuse the GUI's canonical extras validation rather than maintaining a
    // second channel-permutation / white-balance / numeric schema.
    let mut extras = crate::state::GuiState::factory();
    extras.sections["special"] = settings.special.clone();
    extras.sections["load_raw"] = settings.raw.clone();
    extras
        .runtime_params()
        .map_err(|e| Message::new("operations.record.parameters").context(e))?;
    let raw: spektrafilm_raw::RawOptions = serde_json::from_value({
        let mut raw = settings.raw.clone();
        if raw["white_balance"] == "as-shot" {
            raw["white_balance"] = "as_shot".into();
        }
        raw
    })
    .map_err(|e| Message::new("operations.record.raw_settings").context(e))?;
    raw.validate()
        .map_err(|e| Message::new("operations.record.raw_settings").context(e))?;
    if let Some(applied) = &settings.applied {
        spektrafilm_core::presets::validate_name(&applied.name).map_err(|e| {
            Message::new("operations.record.preset_context").context(anyhow::Error::msg(e))
        })?;
    }
    Ok(())
}

pub(crate) fn dependencies(
    settings: &PhotographSettings,
    data_dir: &Path,
) -> Result<Vec<Dependency>> {
    let result = (|| -> Result<Vec<Dependency>> {
        let mut paths = BTreeSet::new();
        for stock in [&settings.film, &settings.print] {
            ensure!(
                !stock.is_empty() && !stock.contains(['/', '\\']) && stock != "." && stock != "..",
                Message::new("operations.record.material_name").arg("name", stock)
            );
            paths.insert(PathBuf::from(format!("profiles/{stock}.json")));
        }
        let film = profile::load_profile_by_name(data_dir, &settings.film)?;
        let print = profile::load_profile_by_name(data_dir, &settings.print)?;
        ensure!(
            film.is_film() && film.is_filming() && print.is_print_stock(),
            Message::new("operations.record.material_kind")
        );
        spektrafilm_core::validate_scan_output(&settings.params, &film)
            .map_err(anyhow::Error::msg)?;
        // RenderSnapshot always loads the neutral database and both materials.
        let neutral = spektrafilm_core::neutral_filters::NeutralFilters::load(data_dir)
            .map_err(anyhow::Error::msg)?;
        let mut params = settings.params.clone();
        params.restore_render_preparation_state(&settings.preparation);
        params = spektrafilm_core::runtime::digest_params_with_neutral(
            params,
            &film,
            &print,
            &neutral,
            spektrafilm_core::runtime::DigestMode::PreserveUserEdits,
        );
        let method = params.settings.rgb_to_raw_method.as_str();
        if method != "mallett2019" {
            let descriptor = spektrafilm_core::spectral_service::lut_descriptor(data_dir, method)
                .map_err(anyhow::Error::msg)?;
            let asset = PathBuf::from(&descriptor.file);
            let asset = if asset.is_absolute()
                || asset.components().any(|c| {
                    matches!(
                        c,
                        std::path::Component::ParentDir | std::path::Component::CurDir
                    )
                }) {
                absolute(&data_dir.join("luts/spectral_upsampling").join(asset))?
            } else {
                PathBuf::from("luts/spectral_upsampling").join(asset)
            };
            let root = PathBuf::from("luts/spectral_upsampling");
            let mut selected = Vec::new();
            for entry in std::fs::read_dir(data_dir.join(&root))? {
                let path = entry?.path();
                if path.extension().is_some_and(|e| e == "toml") {
                    let parsed = spektrafilm_core::spectral_service::parse_lut_descriptor(
                        &std::fs::read_to_string(&path)?,
                    )
                    .map_err(anyhow::Error::msg)?;
                    if parsed.identifier == method {
                        selected.push(path.strip_prefix(data_dir)?.to_owned());
                    }
                }
            }
            ensure!(
                selected.len() == 1,
                Message::new("operations.record.lut_identity").arg("name", method)
            );
            paths.insert(selected.remove(0));
            paths.insert(asset);
        }
        if params.camera.color_filter != "none" {
            let name = params
                .camera
                .color_filter
                .strip_prefix("hoya_")
                .ok_or_else(|| anyhow::Error::new(Message::new("operations.record.parameters")))?;
            paths.insert(PathBuf::from(format!("filters/colored/hoya/{name}.csv")));
        }
        if !matches!(
            params.scanner.conversion_style.as_str(),
            "standard" | "neutral"
        ) {
            paths.insert(PathBuf::from(format!(
                "scan-styles/{}.json",
                params.scanner.conversion_style
            )));
        }
        let mut dependencies = paths
            .into_iter()
            .map(|path| {
                digest(&data_dir.join(&path)).map(|sha256| Dependency {
                    path,
                    sha256: Some(sha256),
                })
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        let path = PathBuf::from("filters/neutral_print_filters.json");
        let sha256 = match digest(&data_dir.join(&path)) {
            Ok(hash) => Some(hash),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        dependencies.push(Dependency { path, sha256 });
        dependencies.sort_by(|a, b| a.path.cmp(&b.path));
        // Validate through the same runtime builder; never substitute a default
        // profile, ignore an invalid style, or accept an unusable external LUT.
        let mut runtime_film = profile::load_profile_by_name(data_dir, &settings.film)?;
        let print = profile::load_profile_by_name(data_dir, &settings.print)?;
        let neutral = spektrafilm_core::neutral_filters::NeutralFilters::load(data_dir)
            .map_err(anyhow::Error::msg)?;
        params = settings.params.clone();
        params.restore_render_preparation_state(&settings.preparation);
        params = spektrafilm_core::runtime::digest_params_with_neutral(
            params,
            &runtime_film,
            &print,
            &neutral,
            spektrafilm_core::runtime::DigestMode::PreserveUserEdits,
        );
        let mut runtime_print = if params.io.scan_film {
            runtime_film.clone()
        } else {
            print
        };
        for (profile, key) in [
            (&mut runtime_film, "film_channel_swap"),
            (&mut runtime_print, "print_channel_swap"),
        ] {
            let order = settings.special[key]
                .as_array()
                .expect("validated channel swap");
            for row in &mut profile.data.channel_density {
                if row.len() >= 3 {
                    let original = [row[0], row[1], row[2]];
                    for ch in 0..3 {
                        row[ch] = original[order[ch].as_u64().expect("validated channel") as usize];
                    }
                }
            }
        }
        if !params.scanner.operator_eligible() {
            params.scanner.operator = Default::default();
        }
        spektrafilm_core::runtime::Runtime::new(runtime_film, runtime_print, params, data_dir)
            .map_err(anyhow::Error::msg)?;
        for dependency in &dependencies {
            let current = match digest(&data_dir.join(&dependency.path)) {
                Ok(hash) => Some(hash),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            ensure!(
                current == dependency.sha256,
                Message::new("operations.record.dependencies_changed")
            );
        }
        Ok(dependencies)
    })();
    result.map_err(|e| Message::new("operations.record.dependencies").context(e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PhotographSettings) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(data.join("profiles")).unwrap();
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        for name in ["kodak_portra_400", "kodak_portra_endura"] {
            std::fs::copy(
                bundled.join(format!("profiles/{name}.json")),
                data.join(format!("profiles/{name}.json")),
            )
            .unwrap();
        }
        let source = dir.path().join("source.tiff");
        std::fs::write(&source, b"exact source image bytes").unwrap();
        let gui = crate::state::GuiState::factory();
        let mut params = RuntimeParams::default();
        params.random_seed = 0x12345678;
        params.settings.rgb_to_raw_method = "mallett2019".into();
        let preparation = RenderPreparationState {
            explicit_section_paths: vec!["camera.exposure_compensation_ev".into()],
            neutral_print_filters_protected: [true, false, true],
        };
        params.restore_render_preparation_state(&preparation);
        let settings = PhotographSettings {
            params,
            preparation,
            film: "kodak_portra_400".into(),
            print: "kodak_portra_endura".into(),
            special: gui.sections["special"].clone(),
            raw: gui.sections["load_raw"].clone(),
            rotation: 1,
            applied: None,
        };
        (dir, data, source, settings)
    }

    #[test]
    fn roundtrip_restores_seed_and_preparation_without_preferences() {
        let (dir, data, source, settings) = fixture();
        let record = PhotographRecord::capture(&source, settings.clone(), &data).unwrap();
        let path = dir.path().join("photograph.json");
        record.save(&path).unwrap();
        let restored = PhotographRecord::load(&path).unwrap();
        assert_eq!(restored.settings, settings);
        assert_eq!(
            restored.validate(None, &data).unwrap(),
            absolute(&source).unwrap()
        );
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(value.get("preferences").is_none());
        assert!(value["settings"].get("history").is_none());
        assert!(value["settings"].get("export").is_none());
    }

    #[test]
    fn relocation_requires_explicit_selection_and_exact_content() {
        let (dir, data, source, settings) = fixture();
        let record = PhotographRecord::capture(&source, settings, &data).unwrap();
        let relocated = dir.path().join("relocated.tiff");
        std::fs::rename(&source, &relocated).unwrap();
        assert!(record.validate(None, &data).is_err());
        assert_eq!(
            record.validate(Some(&relocated), &data).unwrap(),
            absolute(&relocated).unwrap()
        );
        std::fs::write(&relocated, b"same name is not source identity").unwrap();
        assert!(record.validate(Some(&relocated), &data).is_err());
    }

    #[test]
    fn changed_source_or_selected_material_is_rejected() {
        let (_dir, data, source, settings) = fixture();
        let record = PhotographRecord::capture(&source, settings, &data).unwrap();
        let bytes = std::fs::read(&source).unwrap();
        std::fs::write(&source, b"changed pixels").unwrap();
        assert!(record.validate(None, &data).is_err());
        std::fs::write(&source, bytes).unwrap();
        let material = data.join("profiles/kodak_portra_400.json");
        let mut bytes = std::fs::read(&material).unwrap();
        bytes.push(b' ');
        std::fs::write(material, bytes).unwrap();
        assert!(record.validate(None, &data).is_err());
    }

    #[test]
    fn unrelated_material_changes_do_not_invalidate_record() {
        let (_dir, data, source, settings) = fixture();
        let record = PhotographRecord::capture(&source, settings, &data).unwrap();
        std::fs::write(data.join("profiles/unselected.json"), b"not loaded").unwrap();
        assert!(record.validate(None, &data).is_ok());
        std::fs::create_dir_all(data.join("filters")).unwrap();
        std::fs::write(data.join("filters/neutral_print_filters.json"), b"{}").unwrap();
        assert!(
            record.validate(None, &data).is_err(),
            "adding a previously absent preparation dependency changes identity"
        );
    }

    #[test]
    fn selected_style_and_camera_filter_changes_are_dependencies() {
        let (_dir, data, source, mut settings) = fixture();
        std::fs::create_dir_all(data.join("scan-styles")).unwrap();
        let style = data.join("scan-styles/probe.json");
        let bytes = br#"{"channels":[{"brightness_ev":0.9,"contrast":0.7,"dmax":1.5},{"brightness_ev":0.9,"contrast":0.7,"dmax":1.5},{"brightness_ev":0.9,"contrast":0.7,"dmax":1.5}]}"#;
        std::fs::write(&style, bytes).unwrap();
        settings.params.scanner.conversion_style = "probe".into();
        settings.params.scanner.scan_output = "negative_to_positive".into();
        settings.params.workflow.route = "input > film > scan".into();
        std::fs::create_dir_all(data.join("filters/colored/hoya")).unwrap();
        let filter = data.join("filters/colored/hoya/x0.csv");
        std::fs::write(&filter, b"380,0.5\n780,0.9\n").unwrap();
        settings.params.camera.color_filter = "hoya_x0".into();
        let record = PhotographRecord::capture(&source, settings, &data).unwrap();
        std::fs::write(&style, [bytes.as_slice(), b" "].concat()).unwrap();
        assert!(record.validate(None, &data).is_err());
        std::fs::write(&style, bytes).unwrap();
        std::fs::write(&filter, b"380,0.6\n780,0.9\n").unwrap();
        assert!(record.validate(None, &data).is_err());
        std::fs::remove_file(&filter).unwrap();
        assert!(record.validate(None, &data).is_err());
    }

    #[test]
    fn selected_external_lut_bytes_are_checked() {
        let (_dir, data, source, mut settings) = fixture();
        let root = data.join("luts/spectral_upsampling");
        std::fs::create_dir_all(&root).unwrap();
        let bundled =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/luts/spectral_upsampling");
        for name in [
            "hanatos2025_irradiance_xy_tc.toml",
            "hanatos2025_irradiance_xy_tc.npy",
        ] {
            std::fs::copy(bundled.join(name), root.join(name)).unwrap();
        }
        settings.params.settings.rgb_to_raw_method = "hanatos2025".into();
        let record = PhotographRecord::capture(&source, settings, &data).unwrap();
        let asset = root.join("hanatos2025_irradiance_xy_tc.npy");
        let mut bytes = std::fs::read(&asset).unwrap();
        let end = bytes.len() - 1;
        bytes[end] ^= 1;
        std::fs::write(&asset, bytes).unwrap();
        assert!(record.validate(None, &data).is_err());
    }

    #[test]
    fn strict_format_rejects_unknown_version_fields_and_invalid_values() {
        let (dir, data, source, settings) = fixture();
        let record = PhotographRecord::capture(&source, settings, &data).unwrap();
        let value = serde_json::to_value(&record).unwrap();
        for (pointer, replacement) in [
            ("/schema_version", json!(2)),
            ("/kind", json!("preset")),
            ("/source/sha256", json!("not-a-digest")),
            ("/settings/rotation", json!(4)),
            ("/settings/special/film_channel_swap", json!([0, 0, 9])),
            ("/settings/params/workflow/route", json!("unknown")),
            ("/settings/raw/white_balance", json!("unknown")),
        ] {
            let mut invalid = value.clone();
            *invalid.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                PhotographRecord::decode(&serde_json::to_vec(&invalid).unwrap()).is_err(),
                "{pointer}"
            );
        }
        for parent in [
            "",
            "/settings",
            "/source",
            "/settings/params",
            "/settings/raw",
        ] {
            let mut invalid = value.clone();
            invalid
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unknown".into(), json!(true));
            let path = dir.path().join("invalid.json");
            std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(PhotographRecord::load(&path).is_err(), "{parent}");
        }
        fn reject_missing_fields(
            value: &serde_json::Value,
            root: &serde_json::Value,
            pointer: &str,
        ) {
            if let Some(object) = value.as_object() {
                for (key, value) in object {
                    let mut invalid = root.clone();
                    invalid
                        .pointer_mut(pointer)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove(key);
                    assert!(
                        PhotographRecord::decode(&serde_json::to_vec(&invalid).unwrap()).is_err(),
                        "missing {pointer}/{key}"
                    );
                    reject_missing_fields(value, root, &format!("{pointer}/{key}"));
                }
            }
        }
        reject_missing_fields(&value["settings"]["params"], &value, "/settings/params");
        let mut invalid = value.clone();
        invalid["settings"]["applied"] =
            json!({"group":"user", "name":"Accepted look", "preferences":{}});
        assert!(PhotographRecord::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        let mut invalid = value.clone();
        invalid["dependencies"] = json!([]);
        assert!(PhotographRecord::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
        let mut invalid = value;
        invalid["dependencies"][0]["path"] = json!("../outside.json");
        assert!(PhotographRecord::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }

    #[test]
    fn failed_replacement_preserves_previous_record_and_source() {
        let (dir, data, source, settings) = fixture();
        let record = PhotographRecord::capture(&source, settings, &data).unwrap();
        let destination = dir.path().join("record.json");
        record.save(&destination).unwrap();
        let before = std::fs::read(&destination).unwrap();
        let mut invalid = record.clone();
        invalid.settings.params.camera.exposure_compensation_ev = f32::NAN;
        assert!(invalid.save(&destination).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), before);
        let source_before = std::fs::read(&source).unwrap();
        assert!(record.save(&source).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), source_before);
        let blocked = dir.path().join("directory.json");
        std::fs::create_dir(&blocked).unwrap();
        assert!(record.save(&blocked).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), before);
        assert!(std::fs::read_dir(dir.path()).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".photograph-record-")
        }));
        let preset = dir.path().join("preset.json");
        std::fs::write(&preset, b"{\"kind\":\"look\"}").unwrap();
        assert!(record.save(&preset).is_err());
        assert_eq!(std::fs::read(preset).unwrap(), b"{\"kind\":\"look\"}");
    }

    #[test]
    fn raw_decoder_and_source_specific_lens_identity_are_validated() {
        let (dir, data, source, mut settings) = fixture();
        let raw = dir.path().join("source.dng");
        std::fs::rename(source, &raw).unwrap();
        settings.raw["lens_correction"] = json!(true);
        let record = PhotographRecord::capture(&raw, settings, &data).unwrap();
        assert!(record.raw_decoder.is_some());
        assert!(record.lens_model.is_some());
        assert!(record.validate(None, &data).is_ok());
        let accepted = spektrafilm_raw::lens_model_identity(&raw).unwrap();
        assert!(record.validate_accepted_lens_model(Some(&accepted)).is_ok());
        assert!(record.validate_accepted_lens_model(None).is_err());
        assert!(
            record
                .validate_accepted_lens_model(Some("changed-native-calibration"))
                .is_err()
        );
        let mut changed_decoder = record.clone();
        changed_decoder.raw_decoder = Some("different-decoder".into());
        assert!(changed_decoder.validate(None, &data).is_err());
        let mut changed_lens = record;
        changed_lens.lens_model = Some("0".repeat(64));
        assert!(changed_lens.validate(None, &data).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn nonregular_sources_are_rejected_without_blocking() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let (dir, data, source, settings) = fixture();
        let mut record = PhotographRecord::capture(&source, settings, &data).unwrap();
        let fifo = dir.path().join("source-fifo.tiff");
        let c_path = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        assert!(source_digest(&fifo).is_err());
        assert!(PhotographRecord::load(&fifo).is_err());
        let link = dir.path().join("fifo-link.tiff");
        symlink(&fifo, &link).unwrap();
        assert!(source_digest(&link).is_err());
        record.source.path = fifo;
        assert!(record.validate(None, &data).is_err());
        assert!(source_digest(dir.path()).is_err());
    }
}
