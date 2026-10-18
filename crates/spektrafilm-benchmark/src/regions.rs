//! Deterministic generation of standalone measurement regions.
//!
//! This module owns only the finite grid-to-regions document transform. It
//! does not inspect images, render, sample, or build a visual report.

use crate::{common, documents};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::{Error, Result};

fn invalid(message: impl Into<String>) -> Error {
    Error::new("invalid_regions_grid", message.into()).boundary("validation")
}

fn column_label(column: u64) -> String {
    let mut label = String::new();
    let mut value = column;
    while value > 0 {
        let rest = (value - 1) % 26;
        label.insert(0, char::from(b'A' + rest as u8));
        value = (value - 1) / 26;
    }
    label
}

fn checked_span(count: u64, patch: u64, gap: u64) -> Option<u64> {
    count
        .checked_mul(patch)?
        .checked_add(count.checked_sub(1)?.checked_mul(gap)?)
}

fn checked_position(origin: u64, index: u64, patch: u64, gap: u64) -> Option<u64> {
    index
        .checked_mul(patch.checked_add(gap)?)?
        .checked_add(origin)
}

const MAX_GRID_REGIONS: u64 = 1_000_000;
const MAX_GRID_COORDINATE: u64 = 4_294_967_295;

fn valid_identifier(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Generate one ordinary `regions` document from a finite row-major grid.
///
/// The output is published only after the generated document passes the same
/// shape and local-dependency validation used by authored regions documents.
#[allow(clippy::too_many_arguments)]
pub fn generate_grid(
    id: &str,
    rows: u64,
    columns: u64,
    origin: [u64; 2],
    patch_size: [u64; 2],
    gap: [u64; 2],
    sample: Option<[u64; 2]>,
    space: &str,
    role: &str,
    measurements: &[String],
    out: &Path,
) -> Result<Value> {
    if !valid_identifier(id) {
        return Err(invalid(format!("invalid regions document id {id:?}")));
    }
    if rows == 0 || columns == 0 {
        return Err(invalid("rows and columns must be positive"));
    }
    if patch_size[0] == 0 || patch_size[1] == 0 {
        return Err(invalid("patch width and height must be positive"));
    }
    if !documents::AUTHORING_COORDINATE_SPACES.contains(&space)
        && !documents::NATIVE_COORDINATE_SPACES.contains(&space)
    {
        return Err(invalid(format!("unsupported coordinate space {space:?}")));
    }
    let region_count = rows
        .checked_mul(columns)
        .ok_or_else(|| invalid("region count overflows 64-bit geometry"))?;
    if region_count > MAX_GRID_REGIONS {
        return Err(invalid(format!(
            "grid contains more than {MAX_GRID_REGIONS} regions"
        )));
    }
    if !documents::region_roles()?.iter().any(|value| value == role) {
        return Err(invalid(format!("unknown region role {role:?}")));
    }
    let families = documents::measurement_families()?;
    if let Some(unknown) = measurements
        .iter()
        .find(|value| !families.iter().any(|known| known == *value))
    {
        return Err(invalid(format!("unknown measurement family {unknown:?}")));
    }
    if let Some(sample) = sample {
        if sample[0] == 0 || sample[1] == 0 {
            return Err(invalid("sample width and height must be positive"));
        }
        if sample[0] > patch_size[0] || sample[1] > patch_size[1] {
            return Err(invalid("sample window must fit inside every patch"));
        }
    }
    let grid_width = checked_span(columns, patch_size[0], gap[0])
        .and_then(|span| span.checked_add(origin[0]))
        .ok_or_else(|| invalid("grid width overflows 64-bit geometry"))?;
    let grid_height = checked_span(rows, patch_size[1], gap[1])
        .and_then(|span| span.checked_add(origin[1]))
        .ok_or_else(|| invalid("grid height overflows 64-bit geometry"))?;
    if grid_width > MAX_GRID_COORDINATE || grid_height > MAX_GRID_COORDINATE {
        return Err(invalid("grid geometry exceeds sampler coordinate range"));
    }
    let region_count = rows
        .checked_mul(columns)
        .ok_or_else(|| invalid("region count overflows 64-bit geometry"))?;
    let mut regions = Vec::with_capacity(
        usize::try_from(region_count).map_err(|_| invalid("region count is too large"))?,
    );
    for row in 1..=rows {
        for column in 1..=columns {
            let x = checked_position(origin[0], column - 1, patch_size[0], gap[0])
                .ok_or_else(|| invalid("grid x coordinate overflows 64-bit geometry"))?;
            let y = checked_position(origin[1], row - 1, patch_size[1], gap[1])
                .ok_or_else(|| invalid("grid y coordinate overflows 64-bit geometry"))?;
            let (x, y, width, height) = match sample {
                Some([width, height]) => (
                    x.checked_add((patch_size[0] - width) / 2)
                        .ok_or_else(|| invalid("sample x coordinate overflows 64-bit geometry"))?,
                    y.checked_add((patch_size[1] - height) / 2)
                        .ok_or_else(|| invalid("sample y coordinate overflows 64-bit geometry"))?,
                    width,
                    height,
                ),
                None => (x, y, patch_size[0], patch_size[1]),
            };
            let region_id = format!("{}{row}", column_label(column));
            regions.push(json!({
                "id": region_id,
                "label": region_id,
                "role": role,
                "geometry": {"x": x, "y": y, "width": width, "height": height, "space": space},
                "measurements": measurements,
            }));
        }
    }
    let value = json!({"schema_version": 1, "id": id, "regions": regions});
    let bytes = serde_json::to_vec_pretty(&value)?;
    let root = documents::normalize_path(out)?;
    documents::validate_shape("regions", &value)?;
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    let temp = out.with_extension(format!(
        "{}.tmp-{}",
        out.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("json"),
        std::process::id()
    ));
    if fs::symlink_metadata(&temp).is_ok() {
        return Err(common::destination_exists(&temp));
    }
    let publication = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        let stored = documents::load_json_value(&temp)?;
        documents::validate_shape("regions", &stored)?;
        match fs::hard_link(&temp, out) {
            Ok(()) => {
                let _ = fs::remove_file(&temp);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(common::destination_exists(out));
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    })();
    if let Err(error) = publication {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(json!({
        "schema_version": 1,
        "command": "regions",
        "operation": "grid",
        "execution_status": "completed",
        "artifact": root,
        "region_count": region_count,
    }))
}
