//! Self-contained, strictly validated completed film/print material persistence.
//!
//! `read_header` checks the envelope and small embedded state only. `read` also
//! verifies every native member before exposing it to execution. Publication is
//! a sibling-directory transaction with an atomic no-replace final rename.

mod payload;
mod profile;
mod publication;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use spektrafilm_math::image::ImageBuf;

use crate::params::{RenderPreparationState, RuntimeParams};

pub use payload::{NativeRaster, NativeSamples, RasterMember, ScalarType};
pub use profile::EmbeddedProfile;
pub use publication::validate_destination;

pub const SCHEMA_VERSION: u32 = 1;
pub const REPRESENTATION_VERSION: u32 = 1;
pub const NUMERICAL_MODEL: &str = "main-0.3.4-material-v1";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind { Film, Print }

impl MaterialKind {
    pub const fn boundary(self) -> &'static str {
        match self { Self::Film => "cmy_film", Self::Print => "cmy_print" }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageGeometry {
    pub source_width: u32,
    pub source_height: u32,
    pub pixel_size_um: f64,
    /// x, y, width, height on the stored full-frame density grid. Applied
    /// downstream of scanning, not while decoding or restoring density.
    pub pending_crop: Option<[usize; 4]>,
    pub print_spatial_completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageExposure {
    pub auto_enabled: bool,
    pub method: String,
    pub metered_ev: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterpretationContext {
    pub exposure: StageExposure,
    pub camera_compensation_ev: f64,
    pub print_exposure_factor: f64,
    pub print_illuminant: Vec<f64>,
    pub preflash_raw: [f64; 3],
    #[serde(with = "profile::nullable_spectrum")]
    pub midgray_density: Vec<f64>,
    #[serde(with = "profile::nullable_spectrum")]
    pub compensated_midgray_density: Vec<f64>,
    pub prepared_source_color_space: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageProvenance {
    pub producer_version: String,
    pub numerical_model: String,
    pub runtime_model_version: String,
    pub backend: String,
    pub computation_precision: ScalarType,
    pub producing_route: String,
    pub input_identity: String,
    pub inherited_operations: Vec<String>,
}

/// Prior completed material production, retained as provenance only. These
/// parameters must never be replayed through camera preparation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageLineage {
    pub seed: u64,
    pub provenance: StageProvenance,
    pub effective_parameters: RuntimeParams,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrecisionConversion {
    pub storage: ScalarType,
    pub computation: ScalarType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageHeader {
    pub schema_version: u32,
    pub kind: String,
    pub state: MaterialKind,
    pub boundary: String,
    pub representation_version: u32,
    /// Completed-state authority; never replayed as camera input. The resolver
    /// uses this canonical baseline only for remaining operations.
    pub baseline: RuntimeParams,
    pub preparation: RenderPreparationState,
    pub film: EmbeddedProfile,
    pub print: Option<EmbeddedProfile>,
    pub original_print: Option<EmbeddedProfile>,
    pub geometry: StageGeometry,
    pub context: InterpretationContext,
    pub seed: u64,
    pub provenance: StageProvenance,
    pub lineage: Vec<StageLineage>,
    pub density: Option<RasterMember>,
    pub prepared_source: Option<RasterMember>,
    #[serde(skip)]
    pub storage_conversion: Option<PrecisionConversion>,
}

impl StageHeader {
    #[allow(clippy::too_many_arguments)]
    pub fn new(state: MaterialKind, baseline: RuntimeParams, film: EmbeddedProfile,
        print: Option<EmbeddedProfile>, original_print: Option<EmbeddedProfile>,
        geometry: StageGeometry, context: InterpretationContext, seed: u64) -> Self {
        let producing_route = baseline.workflow.route.clone();
        let preparation = baseline.render_preparation_state();
        Self {
            schema_version: SCHEMA_VERSION, kind: "stage_artifact".into(), state,
            boundary: state.boundary().into(), representation_version: REPRESENTATION_VERSION,
            baseline, preparation, film, print, original_print, geometry, context, seed,
            provenance: StageProvenance {
                producer_version: env!("CARGO_PKG_VERSION").into(), numerical_model: NUMERICAL_MODEL.into(),
                runtime_model_version: crate::runtime::MODEL_VERSION.into(),
                backend: String::new(), computation_precision: ScalarType::computation(), producing_route,
                input_identity: String::new(), inherited_operations: Vec::new(),
            },
            lineage: Vec::new(), density: None, prepared_source: None, storage_conversion: None,
        }
    }

    fn validate(&self, require_members: bool) -> Result<()> {
        ensure!(self.schema_version == SCHEMA_VERSION, "unsupported stage artifact schema version {}", self.schema_version);
        ensure!(self.kind == "stage_artifact", "not a stage artifact");
        ensure!(self.representation_version == REPRESENTATION_VERSION, "unsupported material representation version {}", self.representation_version);
        ensure!(self.boundary == self.state.boundary(), "material state and numerical boundary disagree");
        ensure!(self.provenance.numerical_model == NUMERICAL_MODEL, "unsupported artifact numerical model {:?}", self.provenance.numerical_model);
        ensure!(self.provenance.runtime_model_version == crate::runtime::MODEL_VERSION, "unsupported artifact runtime numerical model {:?}", self.provenance.runtime_model_version);
        ensure!(!self.provenance.producer_version.is_empty() && !self.provenance.backend.is_empty() && !self.provenance.producing_route.is_empty() && !self.provenance.input_identity.is_empty(), "artifact provenance is incomplete");
        let route = self.provenance.producing_route.split('>').map(str::trim).collect::<Vec<_>>();
        ensure!(match self.state {
            MaterialKind::Film => route == ["input", "film"],
            MaterialKind::Print => route == ["input", "film", "print"] || route == ["film", "print"],
        }, "producing route does not match the stored material boundary");
        let inherited = self.provenance.inherited_operations.iter().map(String::as_str).collect::<BTreeSet<_>>();
        ensure!(inherited.len() == self.provenance.inherited_operations.len() && inherited.iter().all(|v| ["film", "print"].contains(v)), "invalid inherited-operation provenance");
        ensure!(self.baseline.taps.inject.is_none() && self.baseline.taps.collect.is_none(), "artifact state cannot contain diagnostic tap controls");
        // Round-tripping the canonical parameter schema rejects non-finite
        // values which serde_json would otherwise serialize as null.
        let mut restored: RuntimeParams = serde_json::from_value(serde_json::to_value(&self.baseline)?)
            .context("artifact baseline contains invalid required parameter values")?;
        restored.random_seed = self.baseline.random_seed;
        restored.explicit_section_paths.clone_from(&self.baseline.explicit_section_paths);
        ensure!(restored == self.baseline, "artifact baseline contains non-finite or unrepresentable parameter values");
        self.baseline.validate().map_err(anyhow::Error::msg)?;
        for prior in &self.lineage {
            ensure!(prior.provenance.numerical_model == NUMERICAL_MODEL
                && prior.provenance.runtime_model_version == crate::runtime::MODEL_VERSION
                && !prior.provenance.producer_version.is_empty() && !prior.provenance.backend.is_empty()
                && !prior.provenance.input_identity.is_empty(), "incomplete inherited production provenance");
            let mut restored: RuntimeParams = serde_json::from_value(serde_json::to_value(&prior.effective_parameters)?)?;
            restored.random_seed = prior.effective_parameters.random_seed;
            restored.explicit_section_paths.clone_from(&prior.effective_parameters.explicit_section_paths);
            ensure!(restored == prior.effective_parameters, "inherited effective parameters contain non-finite values");
            prior.effective_parameters.validate().map_err(anyhow::Error::msg)?;
        }
        ensure!(self.provenance.inherited_operations.is_empty() || !self.lineage.is_empty(), "inherited material operations require original producing provenance");
        for path in &self.preparation.explicit_section_paths {
            crate::params::sources::describe_field(path).map_err(anyhow::Error::msg)?;
        }
        self.film.validate()?;
        ensure!(self.film.profile.info.stage == "filming", "film dependency is not a filming material");
        if let Some(print) = &self.print {
            print.validate()?;
            ensure!(print.profile.info.stage == "printing", "print dependency is not a printing material");
        }
        if let Some(print) = &self.original_print {
            print.validate()?;
            ensure!(print.profile.info.stage == "printing", "original print dependency is not a printing material");
        }
        ensure!(self.state != MaterialKind::Print || self.print.is_some(), "print state is missing its completed print material");
        ensure!(self.geometry.source_width > 0 && self.geometry.source_height > 0 && self.geometry.pixel_size_um.is_finite() && self.geometry.pixel_size_um > 0.0, "invalid artifact physical geometry");
        let context = &self.context;
        ensure!(context.camera_compensation_ev.is_finite() && context.print_exposure_factor.is_finite() && context.print_exposure_factor > 0.0, "invalid frozen exposure context");
        ensure!(context.exposure.metered_ev.is_finite() && !context.exposure.method.is_empty(), "invalid actual upstream metering context");
        ensure!(context.exposure.auto_enabled || context.exposure.metered_ev == 0.0, "disabled exposure metering must record zero metered EV");
        ensure!(context.preflash_raw.iter().chain(context.print_illuminant.iter()).all(|v| v.is_finite()), "non-finite interpretation context");
        ensure!(context.midgray_density.iter().chain(context.compensated_midgray_density.iter()).all(|v| !v.is_infinite()), "invalid infinite midgray spectral context");
        ensure!(self.state != MaterialKind::Print || !context.print_illuminant.is_empty(), "print state is missing its prepared illuminant");
        let expected_illuminant = crate::spectral_service::SpectralShape::new(self.baseline.settings.spectral_shape)
            .map_err(anyhow::Error::msg)?.samples;
        ensure!(context.midgray_density.len() == expected_illuminant && context.compensated_midgray_density.len() == expected_illuminant, "frozen midgray spectral calibration disagrees with the working wavelength grid");
        ensure!(context.print_illuminant.is_empty() || context.print_illuminant.len() == expected_illuminant, "prepared print illuminant shape disagrees with working wavelength grid");
        if let Some(color_space) = &context.prepared_source_color_space {
            spektrafilm_math::colorspace::resolve(color_space).map_err(anyhow::Error::msg)?;
        }
        if require_members { ensure!(self.density.is_some(), "artifact is missing its required native density member"); }
        let mut paths = BTreeSet::new();
        paths.insert("stage.json");
        if let Some(member) = &self.density {
            member.validate(true)?;
            let material = match self.state { MaterialKind::Film => &self.film, MaterialKind::Print => self.print.as_ref().context("missing print material")? };
            ensure!(member.channel_order == if material.profile.is_bw() { "monochrome_record_replicated" } else { "CMY" }, "density channel semantics disagree with the owning material");
            ensure!(paths.insert(&member.path), "duplicate artifact member path");
            if let Some([x,y,width,height]) = self.geometry.pending_crop {
                ensure!(width > 0 && height > 0 && x.checked_add(width).is_some_and(|v| v <= member.width as usize) && y.checked_add(height).is_some_and(|v| v <= member.height as usize), "pending crop lies outside stored material grid");
            }
        }
        if let Some(member) = &self.prepared_source {
            member.validate(false)?;
            ensure!(paths.insert(&member.path), "duplicate artifact member path");
            let density = self.density.as_ref().context("source member requires a material member")?;
            ensure!(member.scalar_type == density.scalar_type, "prepared source storage precision differs from material storage");
            let (width, height) = self.geometry.pending_crop.map(|[_, _, w, h]| (w, h))
                .unwrap_or((density.width as usize, density.height as usize));
            ensure!(member.width as usize == width && member.height as usize == height, "prepared source is not aligned to the delivered material grid");
        }
        if require_members {
            ensure!(self.prepared_source.is_some() == context.prepared_source_color_space.is_some(), "prepared source branch and its color declaration must occur together");
            ensure!(self.baseline.output_mix.amount >= 1.0 || self.prepared_source.is_some(), "output mixing requires a retained prepared-source branch");
        }
        Ok(())
    }

    fn restore_profiles(&mut self) {
        self.film.restore();
        if let Some(profile) = &mut self.print { profile.restore(); }
        if let Some(profile) = &mut self.original_print { profile.restore(); }
        self.baseline.random_seed = self.seed;
        self.baseline.explicit_section_paths = self.preparation.explicit_section_paths.iter().cloned().collect();
    }
}

#[derive(Debug, Clone)]
pub struct MaterialState {
    pub header: StageHeader,
    pub density: ImageBuf,
    pub prepared_source: Option<ImageBuf>,
}

/// Checks small required records, paths and member lengths, without hashing or
/// decoding native payloads. Callers must report payload/integrity unchecked.
pub fn read_header(path: impl AsRef<Path>) -> Result<StageHeader> {
    let root = publication::root(path.as_ref())?;
    let file = publication::open_member(&root, "stage.json").context("reading completed stage manifest")?;
    ensure!(file.metadata()?.len() <= MAX_MANIFEST_BYTES, "stage manifest exceeds maximum record size");
    let mut text = String::new();
    file.take(MAX_MANIFEST_BYTES + 1).read_to_string(&mut text).context("stage.json must be UTF-8 JSON")?;
    ensure!(text.len() as u64 <= MAX_MANIFEST_BYTES, "stage manifest exceeds maximum record size");
    let value = crate::params::sources::parse_carrier(&text, false).map_err(anyhow::Error::msg)?;
    for name in ["film", "print", "original_print"] {
        if let Some(embedded) = value.get(name).filter(|v| !v.is_null()) {
            profile::validate_profile_fields(embedded.get("profile").context("missing embedded profile content")?)?;
        }
    }
    let mut header = StageHeader::deserialize(&value).context("decoding strict stage artifact manifest")?;
    require_complete_records(&value, &serde_json::to_value(&header)?, "stage")?;
    header.validate(true)?;
    for member in header.density.iter().chain(header.prepared_source.iter()) {
        let file = publication::open_member(&root, &member.path)?;
        ensure!(file.metadata()?.len() == member.byte_length, "member {} has incorrect byte length", member.path);
    }
    header.restore_profiles();
    Ok(header)
}

/// Verifies hashes and finite native values before any stored state is used.
pub fn read(path: impl AsRef<Path>) -> Result<MaterialState> {
    let root = publication::root(path.as_ref())?;
    let mut header = read_header(&root)?;
    let member = header.density.as_ref().context("missing material member")?;
    let storage = member.scalar_type;
    let (density, _) = payload::read_raster(&root, member, true)?.into_image()?;
    let prepared_source = header.prepared_source.as_ref().map(|member| {
        payload::read_raster(&root, member, false)?.into_image().map(|(image, _)| image)
    }).transpose()?;
    if storage != ScalarType::computation() {
        header.storage_conversion = Some(PrecisionConversion { storage, computation: ScalarType::computation() });
    }
    Ok(MaterialState { header, density, prepared_source })
}

/// Publishes exactly one completed state. Existing destinations, including a
/// destination created after preflight, are never replaced.
pub fn write(path: impl AsRef<Path>, state: &MaterialState) -> Result<()> {
    // Borrowed image buffers are streamed without copying them into native DTOs.
    write_with_native(path.as_ref(), &state.header, NativeRasterRef::Image(&state.density),
        state.prepared_source.as_ref().map(NativeRasterRef::Image))
}

/// Low-level typed entry for preserving an explicitly selected native storage
/// precision. It uses the same complete-state validator and publisher.
pub fn write_native(path: impl AsRef<Path>, header: &StageHeader, density: &NativeRaster,
    prepared_source: Option<&NativeRaster>) -> Result<()> {
    write_with_native(path.as_ref(), header, NativeRasterRef::Native(density), prepared_source.map(NativeRasterRef::Native))
}

enum NativeRasterRef<'a> { Image(&'a ImageBuf), Native(&'a NativeRaster) }
impl NativeRasterRef<'_> {
    fn write(&self, root: &Path, name: &str, density: bool) -> Result<RasterMember> {
        match self {
            Self::Native(raster) => payload::write_raster(root, name, raster, density),
            Self::Image(image) => payload::write_image(root, name, image, density),
        }
    }
}

fn write_with_native(path: &Path, header: &StageHeader, density: NativeRasterRef<'_>, source: Option<NativeRasterRef<'_>>) -> Result<()> {
    validate_destination(path)?;
    let mut manifest = header.clone();
    manifest.density = None;
    manifest.prepared_source = None;
    manifest.validate(false)?;
    let staging = tempfile::Builder::new().prefix(".sfstage-staging-").tempdir_in(publication::parent(path))?;
    manifest.density = Some(density.write(staging.path(), "density.bin", true)?);
    let material = match manifest.state { MaterialKind::Film => &manifest.film, MaterialKind::Print => manifest.print.as_ref().context("missing completed print material")? };
    if material.profile.is_bw() {
        manifest.density.as_mut().expect("density just assigned").channel_order = "monochrome_record_replicated".into();
    }
    manifest.prepared_source = source.map(|raster| raster.write(staging.path(), "prepared-source.bin", false)).transpose()?;
    manifest.validate(true)?;
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    ensure!(bytes.len() as u64 <= MAX_MANIFEST_BYTES, "stage manifest exceeds maximum record size");
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(staging.path().join("stage.json"))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    std::fs::File::open(staging.path())?.sync_all()?;
    publication::publish(staging.path(), path)?;
    // The temporary directory no longer exists after the rename; its guard's
    // cleanup is harmless and handles every error before publication.
    Ok(())
}

fn require_complete_records(input: &serde_json::Value, canonical: &serde_json::Value, path: &str) -> Result<()> {
    match (input, canonical) {
        (serde_json::Value::Object(actual), serde_json::Value::Object(expected)) => {
            for key in actual.keys() {
                ensure!(expected.contains_key(key), "unknown generated state field {path}.{key}");
            }
            for (key, value) in expected {
                let actual = actual.get(key).with_context(|| format!("missing required generated state field {path}.{key}"))?;
                require_complete_records(actual, value, &format!("{path}.{key}"))?;
            }
        }
        (serde_json::Value::Array(actual), serde_json::Value::Array(expected)) => {
            for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                require_complete_records(actual, expected, &format!("{path}[{index}]"))?;
            }
        }
        _ => {},
    }
    Ok(())
}
