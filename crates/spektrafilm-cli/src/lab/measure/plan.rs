//! Sample plans: the declared geometry of one `measure sample` reading.
//!
//! A plan is the only authority for a reading position. It names stable
//! sample identities and the rectangle each one reads in the declared source
//! coordinate space. Nothing infers geometry from the source itself.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// A loaded sample plan.
pub struct Plan {
    /// The plan's optional declared name.
    pub name: Option<String>,
    /// The declared samples, in document order.
    pub samples: Vec<PlanSample>,
    /// The manifest path this plan was read from.
    pub manifest_path: PathBuf,
    /// The manifest file's content hash.
    pub sha256: String,
}

/// One declared sample: a stable identity, its group, and the rectangle it
/// reads in the source's coordinate space.
pub struct PlanSample {
    /// The stable sample identity.
    pub id: String,
    /// The sample's source label.
    pub source: String,
    /// The sample's partition label.
    pub partition: String,
    /// Left column of the rectangle, in source pixels.
    pub x: u32,
    /// Top row of the rectangle, in source pixels.
    pub y: u32,
    /// Rectangle width in pixels; defaults to 1.
    pub width: u32,
    /// Rectangle height in pixels; defaults to 1.
    pub height: u32,
}

#[derive(Deserialize)]
struct RawPlan {
    schema_version: u32,
    #[serde(default)]
    name: Option<String>,
    samples: Vec<RawPlanSample>,
}

#[derive(Deserialize)]
struct RawPlanSample {
    id: String,
    source: String,
    partition: String,
    x: u32,
    y: u32,
    #[serde(default = "default_span")]
    width: u32,
    #[serde(default = "default_span")]
    height: u32,
}

/// A one-pixel window is the plan's default span.
fn default_span() -> u32 {
    1
}

/// Load and validate a sample plan from its manifest path.
pub fn load(path: &Path) -> Result<Plan, String> {
    let manifest_bytes =
        std::fs::read(path).map_err(|e| format!("reading sample plan {}: {e}", path.display()))?;
    let manifest: RawPlan = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("parsing sample plan {}: {e}", path.display()))?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported sample plan schema_version {} in {} (expected 1)",
            manifest.schema_version,
            path.display()
        ));
    }
    if manifest.samples.is_empty() {
        return Err(format!(
            "sample plan {} declares no samples",
            path.display()
        ));
    }
    let mut samples: Vec<PlanSample> = Vec::with_capacity(manifest.samples.len());
    for raw in manifest.samples {
        if raw.id.is_empty() {
            return Err(format!(
                "sample plan {} declares an empty id",
                path.display()
            ));
        }
        if raw.source.is_empty() || raw.partition.is_empty() {
            return Err(format!(
                "sample {:?} declares an empty source or partition",
                raw.id
            ));
        }
        if raw.width < 1 {
            return Err(format!("sample {:?} declares width {}", raw.id, raw.width));
        }
        if raw.height < 1 {
            return Err(format!(
                "sample {:?} declares height {}",
                raw.id, raw.height
            ));
        }
        if samples.iter().any(|sample| sample.id == raw.id) {
            return Err(format!(
                "sample plan {} declares sample {:?} twice",
                path.display(),
                raw.id
            ));
        }
        samples.push(PlanSample {
            id: raw.id,
            source: raw.source,
            partition: raw.partition,
            x: raw.x,
            y: raw.y,
            width: raw.width,
            height: raw.height,
        });
    }
    let sha256 = format!("{:x}", Sha256::digest(&manifest_bytes));
    Ok(Plan {
        name: manifest.name,
        samples,
        manifest_path: path.to_path_buf(),
        sha256,
    })
}
