//! Native little-endian raster storage; no color or transfer conversion.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::Scalar;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    F32,
    F64,
}

impl ScalarType {
    pub const fn computation() -> Self {
        if cfg!(feature = "precision-f64") {
            Self::F64
        } else {
            Self::F32
        }
    }

    pub const fn byte_width(self) -> u64 {
        match self {
            Self::F32 => 4,
            Self::F64 => 8,
        }
    }
}

#[derive(Debug, Clone)]
pub enum NativeSamples {
    F32(Vec<f32>),
    F64(Vec<f64>),
}

#[derive(Debug, Clone)]
pub struct NativeRaster {
    pub width: u32,
    pub height: u32,
    pub samples: NativeSamples,
}

impl NativeRaster {
    pub fn from_image(image: ImageBuf) -> Self {
        Self {
            width: image.width,
            height: image.height,
            #[cfg(not(feature = "precision-f64"))]
            samples: NativeSamples::F32(image.data),
            #[cfg(feature = "precision-f64")]
            samples: NativeSamples::F64(image.data),
        }
    }

    pub fn scalar_type(&self) -> ScalarType {
        match self.samples {
            NativeSamples::F32(_) => ScalarType::F32,
            NativeSamples::F64(_) => ScalarType::F64,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let expected = sample_count(self.width, self.height)?;
        let (len, finite) = match &self.samples {
            NativeSamples::F32(values) => (values.len(), values.iter().all(|v| v.is_finite())),
            NativeSamples::F64(values) => (values.len(), values.iter().all(|v| v.is_finite())),
        };
        ensure!(
            len == expected,
            "native raster sample count does not match dimensions"
        );
        ensure!(finite, "native raster contains non-finite values");
        Ok(())
    }

    /// Conversion is explicit, checked, and never changes the producer's precision claim.
    /// The returned flag identifies a storage-to-computation conversion.
    pub fn into_image(self) -> Result<(ImageBuf, bool)> {
        self.validate()?;
        let converted = self.scalar_type() != ScalarType::computation();
        let data: Vec<Scalar> = match self.samples {
            #[cfg(not(feature = "precision-f64"))]
            NativeSamples::F32(values) => values,
            #[cfg(feature = "precision-f64")]
            NativeSamples::F64(values) => values,
            #[cfg(feature = "precision-f64")]
            NativeSamples::F32(values) => values.into_iter().map(f64::from).collect(),
            #[cfg(not(feature = "precision-f64"))]
            NativeSamples::F64(values) => {
                ensure!(
                    values.iter().all(|v| (*v as f32).is_finite()),
                    "f64 artifact values exceed f32 computation range"
                );
                values.into_iter().map(|v| v as f32).collect()
            }
        };
        Ok((
            ImageBuf::from_data(self.width, self.height, data),
            converted,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RasterMember {
    pub path: String,
    pub byte_length: u64,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub channel_order: String,
    pub scalar_type: ScalarType,
    pub byte_order: String,
    pub layout: String,
    pub domain: String,
    pub units: String,
}

pub(super) fn sample_count(width: u32, height: u32) -> Result<usize> {
    ensure!(width > 0 && height > 0, "raster dimensions must be nonzero");
    let count = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(3))
        .context("raster dimensions overflow")?;
    let count = usize::try_from(count).context("raster exceeds addressable memory")?;
    ensure!(
        count <= isize::MAX as usize / 8,
        "raster exceeds addressable allocation"
    );
    Ok(count)
}

impl RasterMember {
    pub(super) fn validate(&self, density: bool) -> Result<()> {
        super::publication::validate_member_path(&self.path)?;
        let count = sample_count(self.width, self.height)?;
        let bytes = (count as u64)
            .checked_mul(self.scalar_type.byte_width())
            .context("raster byte length overflow")?;
        ensure!(
            self.byte_length == bytes,
            "member {} byte length disagrees with its layout",
            self.path
        );
        ensure!(
            self.channels == 3,
            "member {} must have three record channels",
            self.path
        );
        ensure!(
            self.byte_order == "little_endian" && self.layout == "row_major_channel_interleaved",
            "unsupported raster byte order or layout"
        );
        if density {
            ensure!(
                ["CMY", "monochrome_record_replicated"].contains(&self.channel_order.as_str())
                    && self.domain == "record_density"
                    && self.units == "optical_density",
                "density member must declare material record density in optical-density units"
            );
        } else {
            ensure!(
                self.channel_order == "RGB"
                    && self.domain == "prepared_source_rgb"
                    && self.units == "relative",
                "prepared source member has invalid color layout"
            );
        }
        validate_digest(&self.sha256)?;
        Ok(())
    }
}

pub(super) fn validate_digest(digest: &str) -> Result<()> {
    ensure!(
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid SHA-256 identity"
    );
    Ok(())
}

pub(super) fn write_raster(
    root: &Path,
    path: &str,
    raster: &NativeRaster,
    density: bool,
) -> Result<RasterMember> {
    raster.validate()?;
    match &raster.samples {
        NativeSamples::F32(values) => write_samples(
            root,
            path,
            raster.width,
            raster.height,
            ScalarType::F32,
            values.iter().map(|v| v.to_le_bytes()),
            density,
        ),
        NativeSamples::F64(values) => write_samples(
            root,
            path,
            raster.width,
            raster.height,
            ScalarType::F64,
            values.iter().map(|v| v.to_le_bytes()),
            density,
        ),
    }
}

pub(super) fn write_image(
    root: &Path,
    path: &str,
    image: &ImageBuf,
    density: bool,
) -> Result<RasterMember> {
    ensure!(
        image.data.len() == sample_count(image.width, image.height)?,
        "image sample count does not match dimensions"
    );
    ensure!(
        image.data.iter().all(|v| v.is_finite()),
        "image contains non-finite values"
    );
    write_samples(
        root,
        path,
        image.width,
        image.height,
        ScalarType::computation(),
        image.data.iter().map(|v| v.to_le_bytes()),
        density,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_samples<const N: usize>(
    root: &Path,
    path: &str,
    width: u32,
    height: u32,
    scalar_type: ScalarType,
    values: impl Iterator<Item = [u8; N]>,
    density: bool,
) -> Result<RasterMember> {
    let mut writer = BufWriter::new(
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(path))?,
    );
    let mut digest = Sha256::new();
    for bytes in values {
        writer.write_all(&bytes)?;
        digest.update(bytes);
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(RasterMember {
        path: path.into(),
        byte_length: sample_count(width, height)? as u64 * scalar_type.byte_width(),
        sha256: format!("{:x}", digest.finalize()),
        width,
        height,
        channels: 3,
        channel_order: if density { "CMY" } else { "RGB" }.into(),
        scalar_type,
        byte_order: "little_endian".into(),
        layout: "row_major_channel_interleaved".into(),
        domain: if density {
            "record_density"
        } else {
            "prepared_source_rgb"
        }
        .into(),
        units: if density {
            "optical_density"
        } else {
            "relative"
        }
        .into(),
    })
}

pub(super) fn read_raster(
    root: &super::publication::ArtifactRoot,
    member: &RasterMember,
    density: bool,
) -> Result<NativeRaster> {
    member.validate(density)?;
    let file = super::publication::open_member(root, &member.path)?;
    ensure!(
        file.metadata()?.len() == member.byte_length,
        "member {} has incorrect byte length",
        member.path
    );
    let count = sample_count(member.width, member.height)?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let samples = match member.scalar_type {
        ScalarType::F32 => {
            let mut values = Vec::new();
            values
                .try_reserve_exact(count)
                .context("allocating native f32 payload")?;
            for _ in 0..count {
                let mut bytes = [0; 4];
                reader.read_exact(&mut bytes)?;
                digest.update(bytes);
                values.push(f32::from_le_bytes(bytes));
            }
            NativeSamples::F32(values)
        }
        ScalarType::F64 => {
            let mut values = Vec::new();
            values
                .try_reserve_exact(count)
                .context("allocating native f64 payload")?;
            for _ in 0..count {
                let mut bytes = [0; 8];
                reader.read_exact(&mut bytes)?;
                digest.update(bytes);
                values.push(f64::from_le_bytes(bytes));
            }
            NativeSamples::F64(values)
        }
    };
    let mut trailing = [0];
    if reader.read(&mut trailing)? != 0 {
        bail!("member {} grew while reading", member.path);
    }
    ensure!(
        format!("{:x}", digest.finalize()) == member.sha256,
        "member {} SHA-256 mismatch",
        member.path
    );
    let raster = NativeRaster {
        width: member.width,
        height: member.height,
        samples,
    };
    raster.validate()?;
    Ok(raster)
}
