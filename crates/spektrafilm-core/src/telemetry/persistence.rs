use super::report::Report;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use thiserror::Error;

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("report destination must be a local regular file path")]
    InvalidDestination,
    #[error("report destination parent directory does not exist")]
    MissingParent,
    #[error("report destination aliases a protected invocation path")]
    ProtectedAlias,
    #[error("report destination already exists")]
    AlreadyExists,
    #[error("report persistence failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone)]
pub struct ReportDestination {
    path: PathBuf,
}

impl ReportDestination {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write_new(&self, report: &Report) -> Result<(), PersistenceError> {
        let json = report.to_json().map_err(|error| {
            PersistenceError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                error.to_string(),
            ))
        })?;
        let temporary = self.temporary_path();
        write_temporary(&temporary, json.as_bytes())?;
        match fs::hard_link(&temporary, &self.path) {
            Ok(()) => {
                let _ = fs::remove_file(&temporary);
                sync_parent(&self.path)?;
                Ok(())
            }
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                if error.kind() == io::ErrorKind::AlreadyExists || self.path.exists() {
                    Err(PersistenceError::AlreadyExists)
                } else {
                    Err(PersistenceError::Io(error))
                }
            }
        }
    }

    /// GUI-only publication after the platform save dialog confirmed replacement.
    pub fn write_confirmed(&self, report: &Report) -> Result<(), PersistenceError> {
        let json = report.to_json().map_err(|error| {
            PersistenceError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                error.to_string(),
            ))
        })?;
        let temporary = self.temporary_path();
        write_temporary(&temporary, json.as_bytes())?;
        match fs::rename(&temporary, &self.path) {
            Ok(()) => sync_parent(&self.path),
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                Err(PersistenceError::Io(error))
            }
        }
    }

    fn temporary_path(&self) -> PathBuf {
        let id = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("report");
        self.path
            .with_file_name(format!(".{name}.telemetry-{}-{id}.tmp", std::process::id()))
    }
}

pub fn validate_report_destination(
    path: &Path,
    protected: &[&Path],
) -> Result<ReportDestination, PersistenceError> {
    if path.as_os_str().is_empty() || path.as_os_str() == "-" || path.file_name().is_none() {
        return Err(PersistenceError::InvalidDestination);
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(PersistenceError::MissingParent);
    }
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(PersistenceError::InvalidDestination);
        }
    }
    let destination_identity = path_identity(path)?;
    for protected_path in protected {
        if path_identity(protected_path)? == destination_identity {
            return Err(PersistenceError::ProtectedAlias);
        }
    }
    if let Ok(metadata) = fs::metadata(path) {
        for protected_path in protected {
            if let Ok(protected_metadata) = fs::metadata(protected_path) {
                if same_file(&metadata, &protected_metadata) {
                    return Err(PersistenceError::ProtectedAlias);
                }
            }
        }
    }
    Ok(ReportDestination {
        path: path.to_path_buf(),
    })
}

fn path_identity(path: &Path) -> Result<PathBuf, PersistenceError> {
    if path.exists() {
        return Ok(fs::canonicalize(path)?);
    }
    let Some(file_name) = path.file_name() else {
        return Err(PersistenceError::InvalidDestination);
    };
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if parent.exists() {
        Ok(fs::canonicalize(parent)?.join(file_name))
    } else {
        lexical_absolute(path)
    }
}

fn lexical_absolute(path: &Path) -> Result<PathBuf, PersistenceError> {
    let base = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir()?
    };
    let mut output = base;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            Component::Normal(value) => output.push(value),
            Component::RootDir => output.push(component.as_os_str()),
            Component::Prefix(prefix) => output.push(prefix.as_os_str()),
        }
    }
    Ok(output)
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    left.volume_serial_number() == right.volume_serial_number()
        && left.file_index() == right.file_index()
}

#[cfg(not(any(unix, windows)))]
fn same_file(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

fn write_temporary(path: &Path, bytes: &[u8]) -> Result<(), PersistenceError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(path);
        return Err(PersistenceError::Io(error));
    }
    Ok(())
}

fn sync_parent(path: &Path) -> Result<(), PersistenceError> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::File::open(parent)?.sync_all()?;
    }
    let _ = path;
    Ok(())
}
