//! Non-spectral LUT look: declaration, file validation, lookup, and effect order.
//!
//! The LUT file stays external. A look stores its path and content hash, and
//! resolves the file only when LUT mode is active. The renderer consumes an
//! image in the declared source space and returns the selected output space.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::iter::{IndexedParallelIterator, ParallelIterator};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::colorspace::{self, Cctf, RgbColorSpace, convert_rgb};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

use crate::gamut_compression::OutputGamutCompress;
use crate::lut_formats::{LutDocument, read_cube_bytes};
use crate::params::{
    DiffusionFilterParams, FinishingParams, HalationParams, OutputGamutCompressParams,
};

/// LUT-mode workflow selector. Film mode never reads the stored LUT file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowMode {
    #[default]
    Film,
    Lut,
}

impl WorkflowMode {
    pub const fn is_lut(self) -> bool {
        matches!(self, Self::Lut)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Film => "film",
            Self::Lut => "lut",
        }
    }
}

/// The three supported LUT transport encodings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LutSpace {
    #[serde(rename = "sRGB Linear", alias = "Linear sRGB", alias = "srgb_linear")]
    SrgbLinear,
    #[default]
    #[serde(rename = "sRGB", alias = "srgb")]
    Srgb,
    #[serde(rename = "Display P3", alias = "display_p3", alias = "displayp3")]
    DisplayP3,
}

impl LutSpace {
    pub const fn name(self) -> &'static str {
        match self {
            Self::SrgbLinear => "sRGB Linear",
            Self::Srgb => "sRGB",
            Self::DisplayP3 => "Display P3",
        }
    }

    pub const fn encoded(self) -> bool {
        !matches!(self, Self::SrgbLinear)
    }

    pub fn color_space(self) -> RgbColorSpace {
        match self {
            Self::SrgbLinear => RgbColorSpace::new(
                "sRGB Linear",
                colorspace::SRGB.matrix_rgb_to_xyz,
                colorspace::SRGB.matrix_xyz_to_rgb,
                colorspace::SRGB.whitepoint_xy,
                Cctf::Linear,
            ),
            Self::Srgb => colorspace::SRGB,
            Self::DisplayP3 => colorspace::DISPLAY_P3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LutDomainPolicy {
    #[default]
    Fail,
    Clamp,
}

impl LutDomainPolicy {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Clamp => "clamp",
        }
    }
}

/// Optional 1D shaper carried by a CUBE file with `LUT_1D_SIZE`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LutShaper {
    #[default]
    None,
    #[serde(rename = "cube_1d", alias = "cube1d")]
    Cube1d,
}

impl LutShaper {
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Cube1d => "cube_1d",
        }
    }
}

fn default_grain_amount() -> f64 {
    0.15
}
fn default_grain_size() -> f64 {
    1.0
}
fn default_grain_shadow() -> f64 {
    1.0
}
fn default_grain_midtone() -> f64 {
    1.0
}
fn default_grain_highlight() -> f64 {
    0.5
}
fn default_true() -> bool {
    true
}

/// Output-referred appearance grain for a LUT look. It is deliberately not a
/// film grain engine: no density, RMS granularity, or film profile is read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LutGrainParams {
    #[serde(default)]
    pub active: bool,
    #[serde(default = "default_grain_amount")]
    pub amount: f64,
    #[serde(default = "default_grain_size")]
    pub size: f64,
    #[serde(default = "default_grain_shadow")]
    pub shadows: f64,
    #[serde(default = "default_grain_midtone")]
    pub midtones: f64,
    #[serde(default = "default_grain_highlight")]
    pub highlights: f64,
    #[serde(default)]
    pub chroma: f64,
    #[serde(default)]
    pub seed: u64,
}

impl Default for LutGrainParams {
    fn default() -> Self {
        Self {
            active: false,
            amount: default_grain_amount(),
            size: default_grain_size(),
            shadows: default_grain_shadow(),
            midtones: default_grain_midtone(),
            highlights: default_grain_highlight(),
            chroma: 0.0,
            seed: 0,
        }
    }
}

impl LutGrainParams {
    fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("amount", self.amount),
            ("size", self.size),
            ("shadows", self.shadows),
            ("midtones", self.midtones),
            ("highlights", self.highlights),
            ("chroma", self.chroma),
        ] {
            if !value.is_finite() {
                return Err(format!("lut_look.grain.{name} must be finite"));
            }
        }
        if !(0.0..=1.0).contains(&self.amount) {
            return Err("lut_look.grain.amount must be between 0 and 1".into());
        }
        if self.size <= 0.0 {
            return Err("lut_look.grain.size must be greater than 0".into());
        }
        for (name, value) in [
            ("shadows", self.shadows),
            ("midtones", self.midtones),
            ("highlights", self.highlights),
            ("chroma", self.chroma),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(format!("lut_look.grain.{name} must be between 0 and 1"));
            }
        }
        Ok(())
    }
}

fn inactive_halation() -> HalationParams {
    HalationParams {
        active: false,
        ..HalationParams::default()
    }
}

fn deserialize_inactive_halation<'de, D>(deserializer: D) -> Result<HalationParams, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut value = serde_json::Value::deserialize(deserializer)?;
    if let Some(object) = value.as_object_mut()
        && !object.contains_key("active")
    {
        object.insert("active".into(), false.into());
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

/// Stored LUT look declaration. An inactive look is retained in film mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LutLookParams {
    #[serde(default)]
    pub path: PathBuf,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub input_space: LutSpace,
    #[serde(default)]
    pub output_space: LutSpace,
    #[serde(default)]
    pub domain_min: [f64; 3],
    #[serde(default)]
    pub domain_max: [f64; 3],
    #[serde(default)]
    pub domain_policy: LutDomainPolicy,
    #[serde(default)]
    pub shaper: LutShaper,
    #[serde(default)]
    pub strength: f64,
    #[serde(default = "default_true")]
    pub grade_active: bool,
    #[serde(default)]
    pub grade: FinishingParams,
    #[serde(
        default = "inactive_halation",
        deserialize_with = "deserialize_inactive_halation"
    )]
    pub halation: HalationParams,
    #[serde(default)]
    pub diffusion: DiffusionFilterParams,
    #[serde(default)]
    pub grain: LutGrainParams,
}

impl Default for LutLookParams {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            sha256: String::new(),
            title: String::new(),
            input_space: LutSpace::Srgb,
            output_space: LutSpace::Srgb,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            domain_policy: LutDomainPolicy::Fail,
            shaper: LutShaper::None,
            strength: 1.0,
            grade_active: true,
            grade: FinishingParams::default(),
            halation: inactive_halation(),
            diffusion: DiffusionFilterParams::default(),
            grain: LutGrainParams::default(),
        }
    }
}

impl LutLookParams {
    /// Validate the declaration without reading the LUT file.
    pub fn validate(&self, required: bool) -> Result<(), String> {
        if !required {
            return Ok(());
        }
        if self.path.as_os_str().is_empty() {
            return Err("lut_look.path is required in LUT mode".into());
        }
        if !is_sha256(&self.sha256) {
            return Err("lut_look.sha256 must contain 64 lowercase hexadecimal characters".into());
        }
        if self.title.contains(['\r', '\n']) {
            return Err("lut_look.title must occupy one line".into());
        }
        if !self
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cube"))
        {
            return Err("lut_look.path must name a .cube carrier".into());
        }
        if !self.strength.is_finite() || !(0.0..=1.0).contains(&self.strength) {
            return Err("lut_look.strength must be finite and between 0 and 1".into());
        }
        if self
            .domain_min
            .iter()
            .chain(self.domain_max.iter())
            .any(|value| !value.is_finite())
        {
            return Err("lut_look domain bounds must be finite".into());
        }
        for axis in 0..3 {
            if self.domain_min[axis] >= self.domain_max[axis] {
                return Err(format!("lut_look domain axis {axis} must have min < max"));
            }
        }
        self.grade.validate()?;
        let halation = &self.halation;
        for (name, value) in [
            ("scatter_amount", halation.scatter_amount),
            ("scatter_spatial_scale", halation.scatter_spatial_scale),
            ("halation_amount", halation.halation_amount),
            ("halation_spatial_scale", halation.halation_spatial_scale),
            ("source_compression", halation.source_compression),
            ("boost_ev", f64::from(halation.boost_ev)),
            ("boost_range", f64::from(halation.boost_range)),
            ("protect_ev", f64::from(halation.protect_ev)),
            ("halation_bounce_decay", halation.halation_bounce_decay),
        ] {
            if !value.is_finite() {
                return Err(format!("lut_look.halation.{name} must be finite"));
            }
        }
        for (name, values) in [
            ("scatter_core_um", halation.scatter_core_um),
            ("scatter_tail_um", halation.scatter_tail_um),
            ("scatter_tail_weight", halation.scatter_tail_weight),
            ("halation_strength", halation.halation_strength),
            ("halation_first_sigma_um", halation.halation_first_sigma_um),
        ] {
            if values.iter().any(|value| !value.is_finite()) {
                return Err(format!("lut_look.halation.{name} must be finite"));
            }
        }
        if !(0.0..=0.999999).contains(&halation.source_compression) {
            return Err("lut_look.halation.source_compression must be in [0, 0.999999]".into());
        }
        if halation
            .halation_strength
            .iter()
            .any(|value| !(0.0..=1.0).contains(value))
        {
            return Err("lut_look.halation.halation_strength must be between 0 and 1".into());
        }
        if halation
            .scatter_tail_weight
            .iter()
            .any(|value| !(0.0..=1.0).contains(value))
        {
            return Err("lut_look.halation.scatter_tail_weight must be between 0 and 1".into());
        }
        if !(0.0..=1.0).contains(&halation.halation_bounce_decay) {
            return Err("lut_look.halation.halation_bounce_decay must be between 0 and 1".into());
        }
        for (name, value) in [
            ("scatter_amount", halation.scatter_amount),
            ("scatter_spatial_scale", halation.scatter_spatial_scale),
            ("halation_amount", halation.halation_amount),
            ("halation_spatial_scale", halation.halation_spatial_scale),
        ]
        .into_iter()
        .chain(
            halation
                .scatter_core_um
                .iter()
                .copied()
                .map(|value| ("scatter_core_um", value)),
        )
        .chain(
            halation
                .scatter_tail_um
                .iter()
                .copied()
                .map(|value| ("scatter_tail_um", value)),
        )
        .chain(
            halation
                .halation_first_sigma_um
                .iter()
                .copied()
                .map(|value| ("halation_first_sigma_um", value)),
        ) {
            if value < 0.0 {
                return Err(format!("lut_look.halation.{name} must not be negative"));
            }
        }
        if !spektrafilm_model::diffusion::VALID_FAMILIES
            .contains(&self.diffusion.filter_family.as_str())
        {
            return Err(format!(
                "lut_look.diffusion.filter_family {:?} is unsupported",
                self.diffusion.filter_family
            ));
        }
        let diffusion = &self.diffusion;
        for (name, value) in [
            ("strength", diffusion.strength),
            ("spatial_scale", diffusion.spatial_scale),
            ("halo_warmth", diffusion.halo_warmth),
            ("core_intensity", diffusion.core_intensity),
            ("core_size", diffusion.core_size),
            ("halo_intensity", diffusion.halo_intensity),
            ("halo_size", diffusion.halo_size),
            ("bloom_intensity", diffusion.bloom_intensity),
            ("bloom_size", diffusion.bloom_size),
        ] {
            if !value.is_finite() {
                return Err(format!("lut_look.diffusion.{name} must be finite"));
            }
        }
        self.grain.validate()?;
        Ok(())
    }

    /// Render the required neutral ramp and color-checker import preview as
    /// display-encoded sRGB bytes. The preview runs the declared LUT, shaper,
    /// clamp policy, and grade; import defaults leave effects and grain off.
    pub fn preview_srgb(&self) -> Result<Vec<[u8; 3]>, String> {
        let look = self.load()?;
        let input = self.input_space.color_space();
        let input_encoded = self.input_space.encoded();
        let source = colorspace::SRGB;
        let matrix = colorspace::conversion_matrix(&source, &input);
        let ramp_steps = 22;
        let checker = [
            [115, 82, 68],
            [194, 150, 130],
            [98, 122, 157],
            [87, 108, 67],
            [133, 128, 177],
            [103, 189, 170],
            [214, 126, 44],
            [80, 91, 166],
            [193, 90, 99],
            [94, 60, 108],
            [157, 188, 64],
            [224, 163, 46],
            [56, 61, 150],
            [70, 148, 73],
            [175, 54, 60],
            [231, 199, 31],
            [187, 86, 149],
            [8, 133, 161],
            [243, 243, 242],
            [200, 200, 200],
            [160, 160, 160],
            [122, 122, 121],
            [85, 85, 85],
            [52, 52, 52],
        ];
        let mut image = ImageBuf::new((ramp_steps + checker.len()) as u32, 1);
        let convert_preview = |rgb: [f64; 3]| {
            if source.name == input.name && input_encoded {
                rgb
            } else {
                convert_rgb(rgb, &source, true, &input, input_encoded, &matrix)
            }
        };
        for index in 0..ramp_steps {
            let value = index as f64 / (ramp_steps - 1) as f64;
            let rgb = convert_preview([value; 3]);
            image.set(index as u32, 0, rgb.map(from_f64));
        }
        for (offset, rgb) in checker.iter().enumerate() {
            let rgb = rgb.map(|value| value as f64 / 255.0);
            let rgb = convert_rgb(rgb, &source, true, &input, input_encoded, &matrix);
            image.set((ramp_steps + offset) as u32, 0, rgb.map(from_f64));
        }
        let gamut = OutputGamutCompress::build(
            &OutputGamutCompressParams {
                algorithm: "off".into(),
                ..Default::default()
            },
            "sRGB",
        )?;
        let rendered = look.render(
            &image,
            &input,
            input_encoded,
            &source,
            true,
            1.0,
            &gamut,
            &spektrafilm_backend::cpu_backend::CpuBackend,
        )?;
        Ok(rendered
            .data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|pixel| {
                std::array::from_fn(|channel| {
                    (to_f64(pixel[channel]).clamp(0.0, 1.0) * 255.0).round() as u8
                })
            })
            .collect())
    }

    /// Resolve the file and verify its content hash and declared domain.
    pub fn load(&self) -> Result<LutLook, String> {
        self.validate(true)?;
        let bytes = std::fs::read(&self.path)
            .map_err(|error| format!("reading LUT {}: {error}", self.path.display()))?;
        let hash = hash_bytes(&bytes);
        if hash != self.sha256.to_ascii_lowercase() {
            return Err(format!(
                "LUT {}: content hash differs from lut_look.sha256",
                self.path.display()
            ));
        }
        let document = read_cube_bytes(&bytes)
            .map_err(|error| format!("reading LUT {}: {error}", self.path.display()))?;
        if document.domain_min != self.domain_min || document.domain_max != self.domain_max {
            return Err(format!(
                "LUT {}: declared domain does not match the .cube header",
                self.path.display()
            ));
        }
        match (self.shaper, document.shaper.is_some()) {
            (LutShaper::None, false) | (LutShaper::Cube1d, true) => {}
            (LutShaper::None, true) => {
                return Err(format!(
                    "LUT {}: file contains a 1D shaper but lut_look.shaper is none",
                    self.path.display()
                ));
            }
            (LutShaper::Cube1d, false) => {
                return Err(format!(
                    "LUT {}: lut_look.shaper is cube_1d but the file has no LUT_1D_SIZE",
                    self.path.display()
                ));
            }
        }
        Ok(LutLook {
            document,
            params: self.clone(),
            input: self.input_space.color_space(),
            output: self.output_space.color_space(),
            clipped_pixels: AtomicU64::new(0),
            evaluated_pixels: AtomicU64::new(0),
        })
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("reading LUT {}: {error}", path.display()))?;
    Ok(hash_bytes(&bytes))
}

/// A validated LUT file and its declaration.
pub struct LutLook {
    document: LutDocument,
    params: LutLookParams,
    input: RgbColorSpace,
    output: RgbColorSpace,
    clipped_pixels: AtomicU64,
    evaluated_pixels: AtomicU64,
}

impl LutLook {
    pub fn params(&self) -> &LutLookParams {
        &self.params
    }

    pub fn document(&self) -> &LutDocument {
        &self.document
    }

    fn verify_source(&self) -> Result<(), String> {
        if self.params.path.as_os_str().is_empty() {
            return Ok(());
        }
        let hash = hash_file(&self.params.path)?;
        if hash != self.params.sha256.to_ascii_lowercase() {
            return Err(format!(
                "LUT {}: content hash differs from lut_look.sha256",
                self.params.path.display()
            ));
        }
        Ok(())
    }

    /// Fraction of evaluated pixels clipped by an explicit clamp policy.
    pub fn clipped_fraction(&self) -> f64 {
        let evaluated = self.evaluated_pixels.load(Ordering::Relaxed);
        if evaluated == 0 {
            0.0
        } else {
            self.clipped_pixels.load(Ordering::Relaxed) as f64 / evaluated as f64
        }
    }

    /// Render one image. `source_encoded` describes the incoming samples;
    /// `output_encoded` selects the final transfer state.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        image: &ImageBuf,
        source_space: &RgbColorSpace,
        source_encoded: bool,
        output_space: &RgbColorSpace,
        output_encoded: bool,
        pixel_size_um: f64,
        gamut: &crate::gamut_compression::OutputGamutCompress,
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        self.verify_source()?;
        self.clipped_pixels.store(0, Ordering::Relaxed);
        self.evaluated_pixels.store(0, Ordering::Relaxed);
        // One declaration, one effect order. The device route evaluates the
        // same parameters and fails by name when a stage is unavailable.
        if backend.is_gpu() {
            return self.render_gpu(
                image,
                source_space,
                source_encoded,
                output_space,
                output_encoded,
                pixel_size_um,
                gamut,
                backend,
            );
        }
        let linear_srgb = LutSpace::SrgbLinear.color_space();
        let effects = self.params.diffusion.active || self.params.halation.active;
        let mut out = image.clone();
        let (lookup_space, lookup_encoded) = if effects {
            let matrix = colorspace::conversion_matrix(source_space, &linear_srgb);
            out.par_pixels_mut().for_each(|pixel| {
                let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
                let linear = convert_rgb(
                    rgb,
                    source_space,
                    source_encoded,
                    &linear_srgb,
                    false,
                    &matrix,
                );
                for channel in 0..3 {
                    pixel[channel] = from_f64(linear[channel]);
                }
            });
            if self.params.diffusion.active {
                out = spektrafilm_model::diffusion::apply_diffusion_filter_um(
                    &out,
                    &self.params.diffusion.to_model(),
                    pixel_size_um.max(f64::EPSILON),
                )?;
            }
            if self.params.halation.active {
                let halation = &self.params.halation;
                // Halation is an additive highlight layer, not a rewrite of the
                // base image. Build the highlight mask from linear luminance,
                // apply the spatial response to that mask alone, and add only
                // the difference back to the untouched LUT result.
                let low = 0.18 * 2.0_f64.powf(f64::from(halation.boost_ev));
                let high = 0.18 * 2.0_f64.powf(f64::from(halation.protect_ev));
                let transition = (2.0_f64.powf(f64::from(halation.boost_range)) - 1.0) * low * 2.0;
                let mut mask = out.clone();
                mask.par_pixels_mut().for_each(|pixel| {
                    let luma = 0.2126 * to_f64(pixel[0])
                        + 0.7152 * to_f64(pixel[1])
                        + 0.0722 * to_f64(pixel[2]);
                    let rise = smoothstep(low, low + transition.max(f64::EPSILON), luma);
                    let fall = 1.0
                        - smoothstep(
                            high + transition.max(f64::EPSILON),
                            high + 2.0 * transition.max(f64::EPSILON),
                            luma,
                        );
                    let value = from_f64(rise * fall);
                    pixel.fill(value);
                });
                let halo = spektrafilm_model::halation::apply_halation_um(
                    &mask,
                    pixel_size_um.max(1.0) as f32,
                    0.0,
                    halation.scatter_spatial_scale,
                    [0.0; 3],
                    [0.0; 3],
                    [0.0; 3],
                    halation.halation_amount,
                    halation.halation_spatial_scale,
                    halation.source_compression,
                    halation.halation_strength,
                    halation.halation_first_sigma_um,
                    halation.halation_n_bounces,
                    halation.halation_bounce_decay,
                    halation.halation_renormalize,
                    backend,
                );
                out.par_pixels_mut()
                    .zip(halo.data.as_chunks::<3>().0)
                    .zip(mask.data.as_chunks::<3>().0)
                    .for_each(|((pixel, halo), mask)| {
                        for channel in 0..3 {
                            let addition = to_f64(halo[channel]) - to_f64(mask[channel]);
                            pixel[channel] = from_f64(to_f64(pixel[channel]) + addition.max(0.0));
                        }
                    });
            }
            (linear_srgb, false)
        } else {
            (*source_space, source_encoded)
        };
        let input_matrix = colorspace::conversion_matrix(&lookup_space, &self.input);
        let input_to_output = colorspace::conversion_matrix(&self.input, &self.output);
        let output_matrix = colorspace::conversion_matrix(&self.output, output_space);
        let n = self.document.resolution;
        let table = &self.document.table;
        let shaper_table = self.document.shaper.as_deref().unwrap_or(&[]);
        let domain_min = self.params.domain_min;
        let span: [f64; 3] =
            std::array::from_fn(|axis| self.params.domain_max[axis] - self.params.domain_min[axis]);
        let strength = self.params.strength;
        out.par_pixels_mut()
            .try_for_each(|pixel| -> Result<(), String> {
                let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
                let direct_input = lookup_space.name == self.input.name
                    && lookup_encoded == self.params.input_space.encoded();
                let mut lut_input = if direct_input {
                    rgb
                } else {
                    convert_rgb(
                        rgb,
                        &lookup_space,
                        lookup_encoded,
                        &self.input,
                        self.params.input_space.encoded(),
                        &input_matrix,
                    )
                };
                let original = convert_rgb(
                    lut_input,
                    &self.input,
                    self.params.input_space.encoded(),
                    &self.output,
                    self.params.output_space.encoded(),
                    &input_to_output,
                );
                let lut_out = if strength == 0.0 {
                    original
                } else {
                    self.evaluated_pixels.fetch_add(1, Ordering::Relaxed);
                    let mut clipped = false;
                    if !shaper_table.is_empty() {
                        clipped = enforce_domain(
                            &mut lut_input,
                            domain_min,
                            span,
                            self.params.domain_policy,
                        )?;
                        for (axis, value) in lut_input.iter_mut().enumerate() {
                            *value = sample_1d(shaper_table, shaper_table.len(), *value, axis);
                        }
                    }
                    clipped |= enforce_domain(
                        &mut lut_input,
                        domain_min,
                        span,
                        self.params.domain_policy,
                    )?;
                    if clipped {
                        self.clipped_pixels.fetch_add(1, Ordering::Relaxed);
                    }
                    let lookup = normalize_lut_input(lut_input, domain_min, span);
                    let lut_rgb = sample_tetrahedral(table, n, lookup);
                    if strength == 1.0 {
                        lut_rgb
                    } else {
                        std::array::from_fn(|channel| {
                            original[channel] * (1.0 - strength) + lut_rgb[channel] * strength
                        })
                    }
                };
                let linear = convert_rgb(
                    lut_out,
                    &self.output,
                    self.params.output_space.encoded(),
                    output_space,
                    false,
                    &output_matrix,
                );
                for channel in 0..3 {
                    pixel[channel] = from_f64(linear[channel]);
                }
                Ok(())
            })?;
        if self.params.grade_active {
            crate::finishing::apply(&mut out, &self.params.grade, output_space);
        }
        if gamut.is_active() {
            out.par_pixels_mut().for_each(|pixel| {
                let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
                let compressed = gamut.compress(rgb);
                for channel in 0..3 {
                    pixel[channel] = from_f64(compressed[channel]);
                }
            });
        }
        if output_encoded {
            out.par_pixels_mut().for_each(|pixel| {
                let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
                let encoded = colorspace::encode_rgb(rgb, output_space);
                for channel in 0..3 {
                    pixel[channel] = from_f64(encoded[channel]);
                }
            });
        }
        if self.params.grain.active && self.params.grain.amount > 0.0 {
            let grain = &self.params.grain;
            let width = out.width as usize;
            out.par_pixels_mut().enumerate().for_each(|(index, pixel)| {
                let x = index % width;
                let y = index / width;
                let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
                let g = apply_lut_grain(rgb, x, y, grain);
                for channel in 0..3 {
                    pixel[channel] = from_f64(g[channel]);
                }
            });
        }
        Ok(out)
    }
}

impl LutLook {
    /// The selected-device twin of the host render. Every image stage stays on
    /// the device; `None` from a device operation is an error, never a CPU
    /// fallback. Host code still resolves the LUT file, domain policy, PSF,
    /// effect order, and clamp accounting.
    #[allow(clippy::too_many_arguments)]
    fn render_gpu(
        &self,
        image: &ImageBuf,
        source_space: &RgbColorSpace,
        source_encoded: bool,
        output_space: &RgbColorSpace,
        output_encoded: bool,
        pixel_size_um: f64,
        gamut: &crate::gamut_compression::OutputGamutCompress,
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        let to_f32_matrix = |matrix: &[[f64; 3]; 3]| {
            std::array::from_fn(|row| std::array::from_fn(|column| matrix[row][column] as f32))
        };
        let convert = |image: &ImageBuf,
                       source: &RgbColorSpace,
                       decode: bool,
                       destination: &RgbColorSpace,
                       encode: bool|
         -> Result<ImageBuf, String> {
            // The terminal encode is transfer-only in the reference; using a
            // same-space matrix there would add a second rounding step.
            let matrix = if encode && !decode && source.name == destination.name {
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
            } else {
                colorspace::conversion_matrix(source, destination)
            };
            let params = spektrafilm_backend::ConvertColorspaceGpuParams {
                src_cctf: spektrafilm_backend::cctf_kind(source.cctf),
                dst_cctf: spektrafilm_backend::cctf_kind(destination.cctf),
                decode,
                encode,
                matrix: to_f32_matrix(&matrix),
            };
            // The host conversion operation uploads one bounded image. Large
            // LUT rasters are split at pixel boundaries; the transform is
            // per-pixel, so the result and its order are unchanged.
            let chunk_pixels = 1_048_576;
            let count = image.pixel_count();
            if count <= chunk_pixels {
                return backend.convert_colorspace(image, &params)?.ok_or_else(|| {
                    "convert_colorspace: selected GPU has no LUT colour conversion".to_owned()
                });
            }
            let mut out = Vec::with_capacity(count * 3);
            for first in (0..count).step_by(chunk_pixels) {
                let chunk_pixels = chunk_pixels.min(count - first);
                let start = first * 3;
                let end = start + chunk_pixels * 3;
                let chunk =
                    ImageBuf::from_data(chunk_pixels as u32, 1, image.data[start..end].to_vec());
                let converted = backend
                    .convert_colorspace(&chunk, &params)?
                    .ok_or_else(|| {
                        "convert_colorspace: selected GPU has no LUT colour conversion".to_owned()
                    })?;
                out.extend(converted.data);
            }
            Ok(ImageBuf::from_data(image.width, image.height, out))
        };
        let linear_srgb = LutSpace::SrgbLinear.color_space();
        let effects = self.params.diffusion.active || self.params.halation.active;
        let (mut lookup, lookup_space, lookup_encoded) = if effects {
            let mut out = convert(image, source_space, source_encoded, &linear_srgb, false)?;
            if self.params.diffusion.active {
                let filter = self.params.diffusion.to_model();
                if let Ok((psf, radius, scatter_fraction)) =
                    spektrafilm_model::diffusion::resolve_psf(
                        &filter,
                        pixel_size_um.max(f64::EPSILON),
                        out.width as usize,
                        out.height as usize,
                    )
                {
                    let params = spektrafilm_backend::OpticalDiffusionGpuParams {
                        psf: &psf,
                        radius: radius as u32,
                        scatter_fraction,
                    };
                    out = backend.optical_diffusion(&out, &params)?.ok_or_else(|| {
                        "optical_diffusion: selected GPU has no LUT diffusion operation".to_owned()
                    })?;
                }
            }
            if self.params.halation.active {
                let halation = &self.params.halation;
                let low = 0.18 * 2.0_f64.powf(f64::from(halation.boost_ev));
                let high = 0.18 * 2.0_f64.powf(f64::from(halation.protect_ev));
                let transition = (2.0_f64.powf(f64::from(halation.boost_range)) - 1.0) * low * 2.0;
                let pix_um = pixel_size_um.max(1.0);
                let params = spektrafilm_backend::LutHalationGpuParams {
                    mask_low: low as f32,
                    mask_high: high as f32,
                    mask_transition: transition as f32,
                    halation: spektrafilm_backend::HalationGpuParams {
                        scatter_amount: 0.0,
                        scatter_core_px: [0.0; 3],
                        scatter_tail_px: [0.0; 3],
                        scatter_tail_weight: [0.0; 3],
                        source_compression: halation.source_compression as f32,
                        halation_a_tot: std::array::from_fn(|c| {
                            (halation.halation_strength[c] * halation.halation_amount) as f32
                        }),
                        halation_first_sigma_px: std::array::from_fn(|c| {
                            (halation.halation_first_sigma_um[c] * halation.halation_spatial_scale
                                / pix_um) as f32
                        }),
                        halation_n_bounces: halation.halation_n_bounces,
                        halation_bounce_decay: halation.halation_bounce_decay as f32,
                        halation_renormalize: halation.halation_renormalize,
                    },
                };
                out = backend.lut_halation(&out, &params)?.ok_or_else(|| {
                    "lut_halation: selected GPU has no LUT halation operation".to_owned()
                })?;
            }
            (out, linear_srgb, false)
        } else {
            (image.clone(), *source_space, source_encoded)
        };

        let same_lookup = lookup_space.name == self.input.name
            && lookup_encoded == self.params.input_space.encoded();
        if !same_lookup {
            lookup = convert(
                &lookup,
                &lookup_space,
                lookup_encoded,
                &self.input,
                self.params.input_space.encoded(),
            )?;
        }

        let original = convert(
            &lookup,
            &self.input,
            self.params.input_space.encoded(),
            &self.output,
            self.params.output_space.encoded(),
        )?;

        let strength = self.params.strength;
        let mut out = if strength == 0.0 {
            original
        } else {
            let domain_min = self.params.domain_min;
            let span: [f64; 3] = std::array::from_fn(|axis| {
                self.params.domain_max[axis] - self.params.domain_min[axis]
            });
            let n_pixels = lookup.pixel_count() as u64;
            self.evaluated_pixels.store(n_pixels, Ordering::Relaxed);
            let shaper_entries = self.document.shaper.as_deref().unwrap_or_default();
            let mut clipped = 0u64;
            for pixel in lookup.data.as_chunks::<3>().0 {
                let mut pixel_clipped = false;
                let mut value = [0.0; 3];
                for axis in 0..3 {
                    let sample = to_f64(pixel[axis]);
                    if !sample.is_finite() {
                        return Err(format!("LUT input channel {axis} is not finite"));
                    }
                    let high = domain_min[axis] + span[axis];
                    value[axis] = sample;
                    if sample < domain_min[axis] || sample > high {
                        if self.params.domain_policy == LutDomainPolicy::Fail {
                            return Err(format!(
                                "LUT input channel {axis} value {sample} is outside the declared domain [{}, {}]",
                                domain_min[axis], high
                            ));
                        }
                        pixel_clipped = true;
                        value[axis] = sample.clamp(domain_min[axis], high);
                    }
                }
                if !shaper_entries.is_empty() {
                    for axis in 0..3 {
                        value[axis] =
                            sample_1d(shaper_entries, shaper_entries.len(), value[axis], axis);
                        let high = domain_min[axis] + span[axis];
                        if value[axis] < domain_min[axis] || value[axis] > high {
                            if self.params.domain_policy == LutDomainPolicy::Fail {
                                return Err(format!(
                                    "LUT shaper channel {axis} value {} is outside the declared domain [{}, {}]",
                                    value[axis], domain_min[axis], high
                                ));
                            }
                            pixel_clipped = true;
                        }
                    }
                }
                if pixel_clipped {
                    clipped += 1;
                }
            }
            self.clipped_pixels.store(clipped, Ordering::Relaxed);
            let shaper: Vec<f32> = shaper_entries
                .iter()
                .flatten()
                .map(|value| *value as f32)
                .collect();
            let table: Vec<f32> = self
                .document
                .table
                .iter()
                .flatten()
                .map(|value| *value as f32)
                .collect();
            let params = spektrafilm_backend::Lut3dLookGpuParams {
                size: self.document.resolution as u32,
                table: &table,
                shaper_size: self.document.shaper.as_ref().map_or(0, Vec::len) as u32,
                shaper: &shaper,
                domain_min: self.params.domain_min.map(|value| value as f32),
                domain_span: span.map(|value| value as f32),
                clamp_domain: self.params.domain_policy == LutDomainPolicy::Clamp,
                strength: strength as f32,
            };
            backend
                .lut3d_look(&lookup, &original, &params)?
                .ok_or_else(|| "LUT look: selected GPU has no LUT table operation".to_owned())?
        };

        out = convert(
            &out,
            &self.output,
            self.params.output_space.encoded(),
            output_space,
            false,
        )?;
        if self.params.grade_active
            && let Some(params) = crate::finishing::gpu_params(&self.params.grade, output_space)
        {
            out = backend.finishing(&out, &params)?.ok_or_else(|| {
                "finishing: selected GPU has no LUT finishing operation".to_owned()
            })?;
        }
        if gamut.is_active() {
            let params = gamut
                .gpu_params()
                .ok_or_else(|| "gamut: active compressor has no device parameters".to_owned())?;
            out = backend
                .output_gamut_compress(&out, &params)?
                .ok_or_else(|| {
                    "output_gamut_compress: selected GPU has no gamut operation".to_owned()
                })?;
        }
        if output_encoded {
            out = convert(&out, output_space, false, output_space, true)?;
        }
        if self.params.grain.active && self.params.grain.amount > 0.0 {
            let grain = &self.params.grain;
            let params = spektrafilm_backend::LutGrainGpuParams {
                amount: grain.amount as f32,
                size: grain.size as f32,
                shadows: grain.shadows as f32,
                midtones: grain.midtones as f32,
                highlights: grain.highlights as f32,
                chroma: grain.chroma as f32,
                seed: grain.seed as u32 ^ (grain.seed >> 32) as u32,
            };
            out = backend
                .lut_grain(&out, &params)?
                .ok_or_else(|| "lut_grain: selected GPU has no LUT grain operation".to_owned())?;
        }
        Ok(out)
    }
}

fn sample_1d(table: &[[f64; 3]], n: usize, x: f64, axis: usize) -> f64 {
    if n == 0 {
        return x;
    }
    if n == 1 {
        return table[0][axis];
    }
    let position = x.clamp(0.0, 1.0) * (n - 1) as f64;
    let low = position.floor() as usize;
    let high = (low + 1).min(n - 1);
    let frac = position - low as f64;
    table[low][axis] * (1.0 - frac) + table[high][axis] * frac
}

fn smoothstep(edge0: f64, edge1: f64, value: f64) -> f64 {
    if edge1 <= edge0 {
        return f64::from(value >= edge1);
    }
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn normalize_lut_input(input: [f64; 3], min: [f64; 3], span: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| ((input[axis] - min[axis]) / span[axis]).clamp(0.0, 1.0))
}

/// Apply the declared domain policy to one sample. Returns whether an explicit
/// clamp changed a value. This runs before and after an optional 1D shaper so
/// a shaper cannot silently absorb an out-of-range source value.
fn enforce_domain(
    input: &mut [f64; 3],
    min: [f64; 3],
    span: [f64; 3],
    policy: LutDomainPolicy,
) -> Result<bool, String> {
    let mut clipped = false;
    for (axis, value) in input.iter_mut().enumerate() {
        if !value.is_finite() {
            return Err(format!("LUT input channel {axis} is not finite"));
        }
        let high = min[axis] + span[axis];
        if *value < min[axis] || *value > high {
            if policy == LutDomainPolicy::Fail {
                return Err(format!(
                    "LUT input channel {axis} value {value} is outside the declared domain [{}, {}]",
                    min[axis], high
                ));
            }
            clipped = true;
            *value = value.clamp(min[axis], high);
        }
    }
    Ok(clipped)
}

fn sample_tetrahedral(table: &[[f64; 3]], n: usize, p: [f64; 3]) -> [f64; 3] {
    if n == 0 {
        return [0.0; 3];
    }
    if n == 1 {
        return table[0];
    }
    let scale = (n - 1) as f64;
    let position = p.map(|value| value * scale);
    let low = position.map(|value| (value.floor() as usize).min(n - 1));
    let high = low.map(|value| (value + 1).min(n - 1));
    let frac = position.map(|value| value - value.floor());
    let at = |r: usize, g: usize, b: usize| table[(r * n + g) * n + b];
    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| frac[b].total_cmp(&frac[a]));
    let mut index = low;
    let v0 = at(index[0], index[1], index[2]);
    let mut out = [0.0; 3];
    let first_weight = 1.0 - frac[order[0]];
    for channel in 0..3 {
        out[channel] += v0[channel] * first_weight;
    }
    for (step, &axis) in order.iter().enumerate() {
        index[axis] = high[axis];
        let vertex = at(index[0], index[1], index[2]);
        let weight = if step == 2 {
            frac[axis]
        } else {
            frac[axis] - frac[order[step + 1]]
        };
        for channel in 0..3 {
            out[channel] += vertex[channel] * weight;
        }
    }
    out
}

fn hash_noise(mut value: u32) -> f64 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    f64::from(value) / f64::from(u32::MAX) * 2.0 - 1.0
}

fn value_noise(x: f64, y: f64, seed: u32) -> f64 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let fx = x - x.floor();
    let fy = y - y.floor();
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let hash = |dx: i32, dy: i32| {
        hash_noise(
            seed ^ xi.wrapping_add(dx) as u32
                ^ (yi.wrapping_add(dy) as u32).wrapping_mul(0x9e37_79b9),
        )
    };
    let n00 = hash(0, 0);
    let n10 = hash(1, 0);
    let n01 = hash(0, 1);
    let n11 = hash(1, 1);
    let nx0 = n00 * (1.0 - sx) + n10 * sx;
    let nx1 = n01 * (1.0 - sx) + n11 * sx;
    nx0 * (1.0 - sy) + nx1 * sy
}

fn apply_lut_grain(rgb: [f64; 3], x: usize, y: usize, grain: &LutGrainParams) -> [f64; 3] {
    let luma = (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]).clamp(0.0, 1.0);
    let weight = grain.shadows * (1.0 - luma).powi(2)
        + grain.midtones * 4.0 * luma * (1.0 - luma)
        + grain.highlights * luma * luma;
    let scale = grain.size.max(f64::MIN_POSITIVE);
    let x = x as f64 / scale;
    let y = y as f64 / scale;
    let seed = grain.seed as u32 ^ (grain.seed >> 32) as u32;
    let mono = value_noise(x, y, seed);
    let chroma = value_noise(x, y, seed ^ 0x61a5_f5d3) * grain.chroma;
    let amount = grain.amount * weight;
    [
        rgb[0] + amount * (mono + chroma * 0.5),
        rgb[1] + amount * mono,
        rgb[2] + amount * (mono - chroma * 0.5),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::cpu_backend::CpuBackend;
    use spektrafilm_math::precision::from_f64;

    fn identity(n: usize) -> LutDocument {
        let mut table = Vec::with_capacity(n * n * n);
        for r in 0..n {
            for g in 0..n {
                for b in 0..n {
                    table.push([
                        r as f64 / (n - 1) as f64,
                        g as f64 / (n - 1) as f64,
                        b as f64 / (n - 1) as f64,
                    ]);
                }
            }
        }
        LutDocument {
            resolution: n,
            table,
            shaper: None,
            domain_min: [0.0; 3],
            domain_max: [1.0; 3],
            title: "identity".into(),
        }
    }

    #[test]
    fn identity_lut_preserves_display_rgb_and_honors_strength() {
        let document = identity(2);
        let look = LutLook {
            document: document.clone(),
            params: LutLookParams::default(),
            input: LutSpace::Srgb.color_space(),
            output: LutSpace::Srgb.color_space(),
            clipped_pixels: AtomicU64::new(0),
            evaluated_pixels: AtomicU64::new(0),
        };
        let image = ImageBuf::from_data(1, 1, vec![from_f64(0.25), from_f64(0.5), from_f64(0.75)]);
        let gamut = crate::gamut_compression::OutputGamutCompress::build(
            &crate::params::OutputGamutCompressParams {
                algorithm: "off".into(),
                ..Default::default()
            },
            "sRGB",
        )
        .unwrap();
        let rendered = look
            .render(
                &image,
                &LutSpace::Srgb.color_space(),
                true,
                &LutSpace::Srgb.color_space(),
                true,
                1.0,
                &gamut,
                &CpuBackend,
            )
            .unwrap();
        assert!((to_f64(rendered.data[0]) - 0.25).abs() < 1e-3);
        assert!((to_f64(rendered.data[1]) - 0.5).abs() < 1e-3);
        assert!((to_f64(rendered.data[2]) - 0.75).abs() < 1e-3);
    }

    #[test]
    fn halation_is_an_additive_highlight_layer_and_never_rewrites_dark_pixels() {
        let document = identity(2);
        let mut params = LutLookParams {
            halation: HalationParams {
                active: true,
                halation_amount: 1.0,
                scatter_amount: 1.0,
                ..HalationParams::default()
            },
            grade_active: false,
            ..LutLookParams::default()
        };
        params.grain.active = false;
        let look = LutLook {
            document,
            params,
            input: LutSpace::Srgb.color_space(),
            output: LutSpace::Srgb.color_space(),
            clipped_pixels: AtomicU64::new(0),
            evaluated_pixels: AtomicU64::new(0),
        };
        let dark = [from_f64(0.02), from_f64(0.02), from_f64(0.02)];
        let image = ImageBuf::from_data(1, 1, vec![dark[0], dark[1], dark[2]]);
        let gamut = crate::gamut_compression::OutputGamutCompress::build(
            &crate::params::OutputGamutCompressParams {
                algorithm: "off".into(),
                ..Default::default()
            },
            "sRGB",
        )
        .unwrap();
        let rendered = look
            .render(
                &image,
                &LutSpace::Srgb.color_space(),
                true,
                &LutSpace::Srgb.color_space(),
                true,
                6.0,
                &gamut,
                &CpuBackend,
            )
            .unwrap();
        for (channel, expected) in dark.iter().enumerate() {
            assert!((rendered.data[channel] - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn device_lut_look_matches_the_reference_table_and_grain() {
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let mut document = identity(2);
        document.shaper = Some(vec![[0.0, 0.0, 0.0], [0.2, 0.35, 0.5], [1.0, 1.0, 1.0]]);
        let mut params = LutLookParams {
            strength: 0.7,
            ..LutLookParams::default()
        };
        params.grain.active = true;
        params.grain.amount = 0.2;
        params.grain.size = 1.5;
        params.grain.chroma = 0.4;
        params.grain.seed = 0x1234_5678_9abc_def0;
        let look = LutLook {
            document,
            params,
            input: LutSpace::Srgb.color_space(),
            output: LutSpace::Srgb.color_space(),
            clipped_pixels: AtomicU64::new(0),
            evaluated_pixels: AtomicU64::new(0),
        };
        let image = ImageBuf::from_data(
            4,
            2,
            [
                [0.02, 0.2, 0.8],
                [0.7, 0.1, 0.4],
                [0.33, 0.66, 0.99],
                [0.5, 0.5, 0.5],
                [0.9, 0.3, 0.1],
                [0.15, 0.85, 0.45],
                [0.25, 0.35, 0.45],
                [1.0, 1.0, 1.0],
            ]
            .into_iter()
            .flatten()
            .map(from_f64)
            .collect(),
        );
        let gamut = crate::gamut_compression::OutputGamutCompress::build(
            &crate::params::OutputGamutCompressParams {
                algorithm: "off".into(),
                ..Default::default()
            },
            "sRGB",
        )
        .unwrap();
        let run = |backend: &dyn ComputeBackend| {
            look.render(
                &image,
                &LutSpace::Srgb.color_space(),
                true,
                &LutSpace::Srgb.color_space(),
                true,
                6.0,
                &gamut,
                backend,
            )
            .unwrap()
        };
        let expected = run(&CpuBackend);
        let actual = run(gpu.backend());
        for (index, (&actual, &expected)) in actual.data.iter().zip(&expected.data).enumerate() {
            let difference = (to_f64(actual) - to_f64(expected)).abs();
            if difference >= 2e-4 {
                panic!("index {index}: {actual} != {expected} (difference {difference})");
            }
        }
    }

    #[test]
    fn device_lut_effects_match_the_reference_highlight_layer() {
        use spektrafilm_backend::ComputeBackend;
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let mut params = LutLookParams {
            halation: HalationParams {
                active: true,
                ..HalationParams::default()
            },
            grade_active: false,
            domain_max: [2.0; 3],
            ..LutLookParams::default()
        };
        params.grain.active = false;
        params.diffusion.active = true;
        params.diffusion.strength = 0.35;
        let look = LutLook {
            document: identity(3),
            params,
            input: LutSpace::Srgb.color_space(),
            output: LutSpace::Srgb.color_space(),
            clipped_pixels: AtomicU64::new(0),
            evaluated_pixels: AtomicU64::new(0),
        };
        let mut data = vec![from_f64(0.03); 9 * 7 * 3];
        for index in [4 * 9 + 4, 3 * 9 + 2, 5 * 9 + 6] {
            data[index * 3] = from_f64(1.7);
            data[index * 3 + 1] = from_f64(1.2);
            data[index * 3 + 2] = from_f64(0.4);
        }
        let image = ImageBuf::from_data(9, 7, data);
        let gamut = crate::gamut_compression::OutputGamutCompress::build(
            &crate::params::OutputGamutCompressParams {
                algorithm: "off".into(),
                ..Default::default()
            },
            "sRGB",
        )
        .unwrap();
        let run = |backend: &dyn ComputeBackend| {
            look.render(
                &image,
                &LutSpace::Srgb.color_space(),
                false,
                &LutSpace::Srgb.color_space(),
                true,
                6.0,
                &gamut,
                backend,
            )
            .unwrap()
        };
        let expected = run(&CpuBackend);
        let actual = run(gpu.backend());
        let mut maximum = 0.0f64;
        for (&actual, &expected) in actual.data.iter().zip(&expected.data) {
            maximum = maximum.max((to_f64(actual) - to_f64(expected)).abs());
        }
        assert!(maximum < 5e-4, "maximum difference {maximum}");
    }

    #[test]
    fn import_preview_covers_the_ramp_and_color_checker() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.cube");
        crate::lut_formats::write_lut(
            crate::lut_formats::LutFormat::Cube,
            &identity(2),
            &path,
            &[],
            None,
        )
        .unwrap();
        let params = LutLookParams {
            path: path.clone(),
            sha256: hash_file(&path).unwrap(),
            ..LutLookParams::default()
        };
        let preview = params.preview_srgb().unwrap();
        assert_eq!(preview.len(), 46);
        assert_eq!(preview[0], [0, 0, 0]);
        assert!(preview[21][0] > 250);
        assert!((i32::from(preview[22][0]) - 115).abs() <= 2);
        assert!((i32::from(preview[22][1]) - 82).abs() <= 2);
        assert!((i32::from(preview[22][2]) - 68).abs() <= 2);
    }

    #[test]
    fn changed_external_lut_fails_before_render() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.cube");
        crate::lut_formats::write_lut(
            crate::lut_formats::LutFormat::Cube,
            &identity(2),
            &path,
            &[],
            None,
        )
        .unwrap();
        let params = LutLookParams {
            path: path.clone(),
            sha256: hash_file(&path).unwrap(),
            ..LutLookParams::default()
        };
        let look = params.load().unwrap();
        std::fs::write(&path, "LUT_3D_SIZE 2\n").unwrap();
        let image = ImageBuf::from_data(1, 1, vec![from_f64(0.5); 3]);
        let gamut = crate::gamut_compression::OutputGamutCompress::build(
            &crate::params::OutputGamutCompressParams {
                algorithm: "off".into(),
                ..Default::default()
            },
            "sRGB",
        )
        .unwrap();
        assert!(
            look.render(
                &image,
                &LutSpace::Srgb.color_space(),
                true,
                &LutSpace::Srgb.color_space(),
                true,
                1.0,
                &gamut,
                &CpuBackend,
            )
            .is_err()
        );
    }

    #[test]
    fn shaper_input_cannot_silently_absorb_out_of_domain_values() {
        let mut document = identity(2);
        document.shaper = Some(vec![[0.0; 3], [1.0; 3]]);
        let params = LutLookParams {
            shaper: LutShaper::Cube1d,
            ..LutLookParams::default()
        };
        let look = LutLook {
            document,
            params: params.clone(),
            input: LutSpace::Srgb.color_space(),
            output: LutSpace::Srgb.color_space(),
            clipped_pixels: AtomicU64::new(0),
            evaluated_pixels: AtomicU64::new(0),
        };
        let image = ImageBuf::from_data(1, 1, vec![from_f64(1.2), from_f64(0.5), from_f64(0.5)]);
        let gamut = crate::gamut_compression::OutputGamutCompress::build(
            &crate::params::OutputGamutCompressParams {
                algorithm: "off".into(),
                ..Default::default()
            },
            "sRGB",
        )
        .unwrap();
        assert!(
            look.render(
                &image,
                &LutSpace::Srgb.color_space(),
                true,
                &LutSpace::Srgb.color_space(),
                true,
                1.0,
                &gamut,
                &CpuBackend,
            )
            .is_err()
        );
        let look = LutLook {
            document: look.document,
            params: LutLookParams {
                domain_policy: LutDomainPolicy::Clamp,
                ..params
            },
            input: look.input,
            output: look.output,
            clipped_pixels: AtomicU64::new(0),
            evaluated_pixels: AtomicU64::new(0),
        };
        assert!(
            look.render(
                &image,
                &LutSpace::Srgb.color_space(),
                true,
                &LutSpace::Srgb.color_space(),
                true,
                1.0,
                &gamut,
                &CpuBackend,
            )
            .is_ok()
        );
        assert_eq!(look.clipped_fraction(), 1.0);
    }

    #[test]
    fn declared_domain_rejects_or_clamps_outside_input() {
        let document = identity(2);
        let mut params = LutLookParams {
            domain_max: [0.5; 3],
            ..LutLookParams::default()
        };
        let look = LutLook {
            document,
            params: params.clone(),
            input: LutSpace::Srgb.color_space(),
            output: LutSpace::Srgb.color_space(),
            clipped_pixels: AtomicU64::new(0),
            evaluated_pixels: AtomicU64::new(0),
        };
        let image = ImageBuf::from_data(1, 1, vec![from_f64(0.75), from_f64(0.5), from_f64(0.5)]);
        let gamut = crate::gamut_compression::OutputGamutCompress::build(
            &crate::params::OutputGamutCompressParams {
                algorithm: "off".into(),
                ..Default::default()
            },
            "sRGB",
        )
        .unwrap();
        assert!(
            look.render(
                &image,
                &LutSpace::Srgb.color_space(),
                true,
                &LutSpace::Srgb.color_space(),
                true,
                1.0,
                &gamut,
                &CpuBackend,
            )
            .is_err()
        );
        params.domain_policy = LutDomainPolicy::Clamp;
        let look = LutLook {
            document: look.document,
            params,
            input: look.input,
            output: look.output,
            clipped_pixels: AtomicU64::new(0),
            evaluated_pixels: AtomicU64::new(0),
        };
        let rendered = look
            .render(
                &image,
                &LutSpace::Srgb.color_space(),
                true,
                &LutSpace::Srgb.color_space(),
                true,
                1.0,
                &gamut,
                &CpuBackend,
            )
            .unwrap();
        assert!((to_f64(rendered.data[0]) - 1.0).abs() < 1e-3);
        assert_eq!(look.clipped_fraction(), 1.0);
    }
}
