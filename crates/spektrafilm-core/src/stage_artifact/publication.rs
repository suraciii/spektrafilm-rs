//! Filesystem boundary: no followed member symlinks and atomic no-replace publication.

use std::fs::File;
use std::path::{Component, Path};
#[cfg(windows)]
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};

pub(super) fn validate_member_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && !path.contains('\\') && !path.contains(':') && !path.contains('\0'),
        "unsafe artifact member path {path:?}"
    );
    ensure!(
        Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "artifact member path must be relative without traversal: {path:?}"
    );
    ensure!(
        !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".."),
        "unsafe artifact member path {path:?}"
    );
    Ok(())
}

/// Reject an existing destination before execution. Publication independently repeats
/// the no-clobber guarantee atomically, so this check is not a race-prone reservation.
pub fn validate_destination(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    ensure!(path != Path::new("-"), "artifact streams are not supported");
    let name = path
        .file_name()
        .context("artifact destination must name a new directory")?;
    ensure!(name != "." && name != "..", "invalid artifact destination");
    ensure!(
        !name.to_string_lossy().starts_with(".sfstage-staging-"),
        "artifact destination uses reserved staging-directory prefix"
    );
    let parent = parent(path);
    ensure!(
        parent.is_dir(),
        "artifact destination parent does not exist: {}",
        parent.display()
    );
    match std::fs::symlink_metadata(path) {
        Ok(_) => bail!("artifact destination already exists: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("checking artifact destination"),
    }
    Ok(())
}

pub(super) fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

pub(super) struct ArtifactRoot {
    #[cfg(unix)]
    directory: File,
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(windows)]
    _ancestors: Vec<File>,
}

pub(super) fn root(path: &Path) -> Result<ArtifactRoot> {
    ensure!(path != Path::new("-"), "artifact streams are not supported");
    ensure!(!path.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".sfstage-staging-")), "incomplete artifact staging directories are not inputs");
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let start = CString::new(if path.is_absolute() { "/" } else { "." })?;
        let fd = unsafe { libc::open(start.as_ptr(), flags) };
        if fd < 0 { return Err(std::io::Error::last_os_error()).context("pinning artifact root ancestor"); }
        let mut directory = unsafe { File::from_raw_fd(fd) };
        // Pin every ancestor while walking. Never canonicalize an untrusted
        // pathname: that would resolve a concurrently substituted symlink.
        for component in path.components() {
            let name = match component {
                Component::Normal(name) => name,
                Component::ParentDir => std::ffi::OsStr::new(".."),
                Component::RootDir | Component::CurDir => continue,
                Component::Prefix(_) => bail!("unsupported artifact root prefix"),
            };
            let name = CString::new(name.as_bytes())?;
            let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 { return Err(std::io::Error::last_os_error()).context("opening artifact root without symlinks"); }
            directory = unsafe { File::from_raw_fd(fd) };
        }
        Ok(ArtifactRoot { directory })
    }
    #[cfg(windows)]
    {
        let absolute = std::path::absolute(path)?;
        let mut current = PathBuf::new();
        let mut ancestors = Vec::new();
        for component in absolute.components() {
            current.push(component);
            if matches!(component, Component::Prefix(_)) { continue; }
            let directory = open_windows(&current)?;
            ensure!(directory.metadata()?.is_dir(), "artifact root ancestor must be a directory");
            ancestors.push(directory);
        }
        Ok(ArtifactRoot { path: absolute, _ancestors: ancestors })
    }
    #[cfg(not(any(unix, windows)))]
    bail!("safe artifact root opening is unsupported on this platform")
}

#[cfg(unix)]
pub(super) fn open_member(root: &ArtifactRoot, path: &str) -> Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    validate_member_path(path)?;
    let mut directory = root.directory.try_clone()?;
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        let name = CString::new(component)?;
        let is_directory = components.peek().is_some();
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if is_directory { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("opening artifact member {path:?} without symlinks"));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if !is_directory {
            ensure!(
                file.metadata()?.is_file(),
                "artifact member {path:?} is not a regular file"
            );
            return Ok(file);
        }
        directory = file;
    }
    bail!("empty artifact member path")
}

#[cfg(windows)]
fn open_windows(path: &Path) -> Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new().read(true).share_mode(1)
        .custom_flags(0x00200000 | 0x02000000).open(path)?;
    ensure!(file.metadata()?.file_attributes() & 0x400 == 0, "artifact reparse points are forbidden");
    Ok(file)
}

#[cfg(windows)]
pub(super) fn open_member(root: &ArtifactRoot, path: &str) -> Result<File> {
    validate_member_path(path)?;
    let mut directories = Vec::new();
    let mut current = root.path.clone();
    let mut components = path.split('/').peekable();
    while let Some(component) = components.next() {
        current.push(component);
        let file = open_windows(&current)?;
        if components.peek().is_none() {
            ensure!(
                file.metadata()?.is_file(),
                "artifact member must be a regular file"
            );
            return Ok(file);
        }
        ensure!(
            file.metadata()?.is_dir(),
            "artifact member ancestor must be a directory"
        );
        directories.push(file);
    }
    bail!("empty artifact member path")
}

#[cfg(not(any(unix, windows)))]
pub(super) fn open_member(_root: &ArtifactRoot, _path: &str) -> Result<File> {
    bail!("safe artifact member opening is unsupported on this platform")
}

pub(super) fn publish(staging: &Path, destination: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let source = CString::new(staging.as_os_str().as_bytes())?;
        let target = CString::new(destination.as_os_str().as_bytes())?;
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error())
                .context("publishing artifact without replacing destination");
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let source = CString::new(staging.as_os_str().as_bytes())?;
        let target = CString::new(destination.as_os_str().as_bytes())?;
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
        if result != 0 {
            return Err(std::io::Error::last_os_error())
                .context("publishing artifact without replacing destination");
        }
    }
    #[cfg(windows)]
    std::fs::rename(staging, destination)
        .context("publishing artifact without replacing destination")?;
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    bail!("atomic no-replace artifact publication is unsupported on this platform");
    Ok(())
}
