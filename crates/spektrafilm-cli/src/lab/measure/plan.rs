//! Regions documents: the declared geometry of one reading.
//!
//! A regions document is the only authority for a reading position. It names
//! stable sample identities and the rectangle each one reads in the declared
//! coordinate space. Nothing infers geometry from the source itself.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// A loaded regions document.
pub struct Plan {
    /// The document's optional declared name.
    pub name: Option<String>,
    /// The document's declared identity.
    pub id: Option<String>,
    /// The declared regions, in document order.
    pub samples: Vec<PlanSample>,
    /// The manifest path this document was read from.
    pub manifest_path: PathBuf,
    /// The manifest file's content hash.
    pub sha256: String,
}

/// One declared region: a stable identity, its role, the measurement families
/// it selects, and the rectangle it reads in the declared coordinate space.
pub struct PlanSample {
    /// The stable region identity.
    pub id: String,
    /// The region's optional display label.
    pub label: Option<String>,
    /// The region's source label.
    pub source: String,
    /// The region's partition label.
    pub partition: String,
    /// The region's declared role.
    pub role: String,
    /// The measurement families the region selects.
    pub measurements: Vec<String>,
    /// Left column of the rectangle, in declared-space pixels.
    pub x: u32,
    /// Top row of the rectangle, in declared-space pixels.
    pub y: u32,
    /// Rectangle width in pixels; required and positive.
    pub width: u32,
    /// Rectangle height in pixels; required and positive.
    pub height: u32,
    /// The coordinate space the geometry is expressed in; required.
    pub space: String,
}

/// The region roles a regions document may declare.
const ROLES: [&str; 10] = [
    "skin",
    "neutral",
    "shadow",
    "highlight",
    "color",
    "flat_field",
    "edge",
    "texture",
    "other",
    "full_image",
];

#[derive(Deserialize)]
struct RawPlan {
    schema_version: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    regions: Vec<RawPlanSample>,
}

#[derive(Deserialize)]
struct RawPlanSample {
    id: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default = "default_source")]
    source: String,
    #[serde(default = "default_partition")]
    partition: String,
    role: String,
    #[serde(default)]
    measurements: Vec<String>,
    geometry: RawGeometry,
}

#[derive(Deserialize)]
struct RawGeometry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    space: String,
}
fn default_source() -> String {
    String::new()
}
fn default_partition() -> String {
    String::new()
}

pub fn load(path: &Path) -> Result<Plan, String> {
    let manifest_bytes = std::fs::read(path)
        .map_err(|e| format!("reading regions document {}: {e}", path.display()))?;
    let value: serde_json::Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("parsing regions document {}: {error}", path.display()))?;
    spektrafilm_benchmark::documents::validate_shape("regions", &value)
        .map_err(|error| format!("validating regions document {}: {error}", path.display()))?;
    spektrafilm_benchmark::documents::validate_local_dependencies("regions", &value, path)
        .map_err(|error| format!("validating regions document {}: {error}", path.display()))?;
    let manifest: RawPlan = serde_json::from_value(value)
        .map_err(|error| format!("parsing regions document {}: {error}", path.display()))?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported regions document schema_version {} in {} (expected 1)",
            manifest.schema_version,
            path.display()
        ));
    }
    if manifest.regions.is_empty() {
        return Err(format!(
            "regions document {} declares no regions",
            path.display()
        ));
    }
    let mut samples: Vec<PlanSample> = Vec::with_capacity(manifest.regions.len());
    for raw in manifest.regions {
        if raw.id.is_empty() {
            return Err(format!(
                "regions document {} declares an empty id",
                path.display()
            ));
        }
        if !ROLES.contains(&raw.role.as_str()) {
            return Err(format!(
                "region {:?} declares unknown role {:?}; roles are {}",
                raw.id,
                raw.role,
                ROLES.join(", ")
            ));
        }
        if raw.geometry.width < 1 {
            return Err(format!(
                "region {:?} declares width {}",
                raw.id, raw.geometry.width
            ));
        }
        if raw.geometry.height < 1 {
            return Err(format!(
                "region {:?} declares height {}",
                raw.id, raw.geometry.height
            ));
        }
        if !spektrafilm_benchmark::documents::AUTHORING_COORDINATE_SPACES
            .contains(&raw.geometry.space.as_str())
            && !spektrafilm_benchmark::documents::NATIVE_COORDINATE_SPACES
                .contains(&raw.geometry.space.as_str())
        {
            return Err(format!(
                "region {:?} declares unsupported coordinate space {:?}",
                raw.id, raw.geometry.space
            ));
        }
        if samples.iter().any(|sample| sample.id == raw.id) {
            return Err(format!(
                "regions document {} declares region {:?} twice",
                path.display(),
                raw.id
            ));
        }
        samples.push(PlanSample {
            id: raw.id,
            label: raw.label,
            source: raw.source,
            partition: raw.partition,
            role: raw.role,
            measurements: raw.measurements,
            x: raw.geometry.x,
            y: raw.geometry.y,
            width: raw.geometry.width,
            height: raw.geometry.height,
            space: raw.geometry.space,
        });
    }
    let sha256 = format!("{:x}", Sha256::digest(&manifest_bytes));
    Ok(Plan {
        id: manifest.id,
        name: manifest.name,
        samples,
        manifest_path: path.to_path_buf(),
        sha256,
    })
}
