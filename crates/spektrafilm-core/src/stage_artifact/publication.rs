//! Filesystem boundary: no followed member symlinks and atomic no-replace publication.

use std::fs::File;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

pub(super) fn validate_member_path(path: &str) -> Result<()> {
    ensure!(!path.is_empty() && !path.contains('\\') && !path.contains(':') && !path.contains('\0'), "unsafe artifact member path {path:?}");
    ensure!(Path::new(path).components().all(|part| matches!(part, Component::Normal(_))), "artifact member path must be relative without traversal: {path:?}");
    ensure!(!path.split('/').any(|part| part.is_empty() || part == "." || part == ".."), "unsafe artifact member path {path:?}");
    Ok(())
}

/// Reject an existing destination before execution. Publication independently repeats
/// the no-clobber guarantee atomically, so this check is not a race-prone reservation.
pub fn validate_destination(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    ensure!(path != Path::new("-"), "artifact streams are not supported");
    let name = path.file_name().context("artifact destination must name a new directory")?;
    ensure!(name != "." && name != "..", "invalid artifact destination");
    ensure!(!name.to_string_lossy().starts_with(".sfstage-staging-"), "artifact destination uses reserved staging-directory prefix");
    let parent = parent(path);
    ensure!(parent.is_dir(), "artifact destination parent does not exist: {}", parent.display());
    match std::fs::symlink_metadata(path) {
        Ok(_) => bail!("artifact destination already exists: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => return Err(error).context("checking artifact destination"),
    }
    Ok(())
}

pub(super) fn parent(path: &Path) -> &Path {
    path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."))
}

pub(super) fn root(path: &Path) -> Result<PathBuf> {
    ensure!(path != Path::new("-"), "artifact streams are not supported");
    let metadata = std::fs::symlink_metadata(path).with_context(|| format!("opening artifact {}", path.display()))?;
    ensure!(metadata.is_dir() && !metadata.file_type().is_symlink(), "artifact root must be a real directory");
    // A staging directory is never an input, even after its manifest is complete.
    ensure!(!path.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".sfstage-staging-")), "incomplete artifact staging directories are not inputs");
    Ok(std::fs::canonicalize(path)?)
}

#[cfg(unix)]
pub(super) fn open_member(root: &Path, path: &str) -> Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    validate_member_path(path)?;
    let root_name = CString::new(root.as_os_str().as_bytes())?;
    // Each directory handle pins the traversed ancestor; O_NOFOLLOW prevents
    // a concurrent rename/symlink substitution from escaping the artifact.
    let fd = unsafe { libc::open(root_name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    ensure!(fd >= 0, "opening artifact root: {}", std::io::Error::last_os_error());
    let mut directory = unsafe { File::from_raw_fd(fd) };
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        let name = CString::new(component)?;
        let is_directory = components.peek().is_some();
        let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK
            | if is_directory { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 { return Err(std::io::Error::last_os_error()).with_context(|| format!("opening artifact member {path:?} without symlinks")); }
        let file = unsafe { File::from_raw_fd(fd) };
        if !is_directory {
            ensure!(file.metadata()?.is_file(), "artifact member {path:?} is not a regular file");
            return Ok(file);
        }
        directory = file;
    }
    bail!("empty artifact member path")
}

#[cfg(windows)]
pub(super) fn open_member(root: &Path, path: &str) -> Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    const OPEN_REPARSE_POINT: u32 = 0x00200000;
    const BACKUP_SEMANTICS: u32 = 0x02000000;
    const REPARSE_POINT: u32 = 0x400;
    validate_member_path(path)?;
    let open = |path: &Path| -> Result<File> {
        let file = std::fs::OpenOptions::new().read(true).share_mode(1)
            .custom_flags(OPEN_REPARSE_POINT | BACKUP_SEMANTICS).open(path)?;
        ensure!(file.metadata()?.file_attributes() & REPARSE_POINT == 0, "artifact member reparse points are forbidden");
        Ok(file)
    };
    // Handles deny delete/write sharing so checked ancestors remain pinned
    // while descendants open; reparse points are never followed.
    let mut directories = vec![open(root)?];
    let mut current = root.to_owned();
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        current.push(component);
        let file = open(&current)?;
        if components.peek().is_none() {
            ensure!(file.metadata()?.is_file(), "artifact member must be a regular file");
            return Ok(file);
        }
        ensure!(file.metadata()?.is_dir(), "artifact member ancestor must be a directory");
        directories.push(file);
    }
    bail!("empty artifact member path")
}

#[cfg(not(any(unix, windows)))]
pub(super) fn open_member(_root: &Path, _path: &str) -> Result<File> {
    bail!("safe artifact member opening is unsupported on this platform")
}

pub(super) fn publish(staging: &Path, destination: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let source = CString::new(staging.as_os_str().as_bytes())?;
        let target = CString::new(destination.as_os_str().as_bytes())?;
        let result = unsafe { libc::renameat2(libc::AT_FDCWD, source.as_ptr(), libc::AT_FDCWD, target.as_ptr(), libc::RENAME_NOREPLACE) };
        if result != 0 { return Err(std::io::Error::last_os_error()).context("publishing artifact without replacing destination"); }
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let source = CString::new(staging.as_os_str().as_bytes())?;
        let target = CString::new(destination.as_os_str().as_bytes())?;
        let result = unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
        if result != 0 { return Err(std::io::Error::last_os_error()).context("publishing artifact without replacing destination"); }
    }
    #[cfg(windows)]
    std::fs::rename(staging, destination).context("publishing artifact without replacing destination")?;
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    bail!("atomic no-replace artifact publication is unsupported on this platform");
    Ok(())
}
