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
    for protected_path in protected {
        if same_file(path, protected_path) {
            return Err(PersistenceError::ProtectedAlias);
        }
    }
    Ok(ReportDestination {
        path: path.to_path_buf(),
    })
}

fn path_identity(path: &Path) -> Result<PathBuf, PersistenceError> {
    let mut resolved = path.to_path_buf();
    // Match the usual filesystem symlink traversal limit, including dangling chains.
    for followed in 0..=40 {
        match fs::symlink_metadata(&resolved) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if followed == 40 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "protected path exceeds the symlink traversal limit",
                    )
                    .into());
                }
                let target = fs::read_link(&resolved)?;
                resolved = if target.is_absolute() {
                    target
                } else {
                    resolved
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(target)
                };
            }
            Ok(_) => return Ok(fs::canonicalize(&resolved)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(file_name) = resolved.file_name() else {
                    return Err(PersistenceError::InvalidDestination);
                };
                let parent = resolved
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
                return if parent.exists() {
                    Ok(fs::canonicalize(parent)?.join(file_name))
                } else {
                    lexical_absolute(&resolved)
                };
            }
            Err(error) => return Err(error.into()),
        }
    }
    unreachable!("symlink traversal returns at its limit")
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
fn same_file(left: &Path, right: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(left), Ok(right)) = (fs::metadata(left), fs::metadata(right)) else {
        return false;
    };
    left.dev() == right.dev() && left.ino() == right.ino()
}

/// Stable file identity for Windows. The standard library's volume/file-index
/// metadata accessors still require the unstable `windows_by_handle` feature, so
/// the identity comes from the documented handle interface instead.
#[cfg(windows)]
fn same_file(left: &Path, right: &Path) -> bool {
    let (Some(left), Some(right)) = (handle_identity(left), handle_identity(right)) else {
        return false;
    };
    left == right
}

#[cfg(windows)]
fn handle_identity(path: &Path) -> Option<(u32, u64)> {
    use std::ffi::c_void;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    #[allow(dead_code)] // Unused fields keep the platform layout.
    struct ByHandleFileInformation {
        file_attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            handle: *mut c_void,
            information: *mut ByHandleFileInformation,
        ) -> i32;
    }

    // Identity needs no data access: the standard library's Windows
    // `fs::metadata` also opens with access mode 0 before querying the handle.
    let file = fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .ok()?;
    let mut information = ByHandleFileInformation::default();
    // SAFETY: `file` owns a live handle and `information` is writable storage of
    // the exact layout `GetFileInformationByHandle` fills.
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    (ok != 0).then_some((
        information.volume_serial_number,
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low),
    ))
}

#[cfg(not(any(unix, windows)))]
fn same_file(_left: &Path, _right: &Path) -> bool {
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
