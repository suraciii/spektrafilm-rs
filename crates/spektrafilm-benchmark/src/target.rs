//! Construction of pinned renderer targets without executing the renderer.
use crate::{
    common::{Error, Result, destination_exists_error, sha256_file},
    documents,
};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path};

/// Build a schema-1 target with canonical dependency paths and the renderer's hash.
/// Changes and environment retain the existing target schema semantics. This only
/// validates document shape; the persisted target loader validates dependencies.
pub fn construct(
    renderer: &Path,
    data_dir: &Path,
    backend: &str,
    id: &str,
    changes: Value,
    environment: Value,
) -> Result<Value> {
    let renderer = renderer.canonicalize()?;
    let data_dir = data_dir.canonicalize()?;
    let value = json!({"schema_version":1,"id":id,
        "renderer":{"path":renderer,"sha256":sha256_file(&renderer)?},
        "data_dir":data_dir,"backend":backend,"changes":changes,"environment":environment});
    documents::validate_shape("target", &value)?;
    Ok(value)
}

/// Write a new pinned target with empty changes, then validate it with the target
/// loader. The output stem is its ID and must satisfy the existing ID grammar.
/// Existing files, directories and symlinks are refused; no device probe runs.
pub fn create(renderer: &Path, data_dir: &Path, backend: &str, out: &Path) -> Result<Value> {
    if fs::symlink_metadata(out).is_ok() {
        return Err(destination_exists_error(out));
    }
    let id = out.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
        Error::new(
            "invalid_id",
            "target output must have a UTF-8 file stem matching the target ID grammar",
        )
    })?;
    let value = construct(
        renderer,
        data_dir,
        backend,
        id,
        json!({"set_files":[],"set":[]}),
        json!({}),
    )?;
    let bytes = serde_json::to_vec_pretty(&value)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                destination_exists_error(out)
            } else {
                error.into()
            }
        })?;
    let persisted = (|| -> Result<()> {
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        documents::load_target(out)?;
        Ok(())
    })();
    if let Err(error) = persisted {
        drop(file);
        fs::remove_file(out)?;
        return Err(error);
    }
    Ok(
        json!({"schema_version":1,"command":"target","execution_status":"completed",
        "artifact":out.canonicalize()?,"target_id":id,"target":value}),
    )
}
