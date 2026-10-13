//! Shared application configuration-directory policy.
use std::path::PathBuf;
pub fn config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("SPEKTRAFILM_CONFIG_DIR") {
        return path.into();
    }
    #[cfg(target_os = "windows")]
    if let Some(path) = std::env::var_os("APPDATA") {
        return PathBuf::from(path).join("spektrafilm");
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Application Support/spektrafilm");
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("spektrafilm");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/spektrafilm")
}

/// Candidate data directories in probe order: the executable-relative install
/// layouts (`data`, `../share/data`, `../Resources/data`), the working
/// directory, then the workspace root for `cargo run`. `SPEKTRAFILM_DATA_DIR`
/// is handled by the callers that accept an override, not here.
pub fn data_dir_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("data"));
        candidates.push(dir.join("..").join("share").join("data"));
        candidates.push(dir.join("..").join("Resources").join("data"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("data"));
    }
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        candidates.push(PathBuf::from(manifest).join("..").join("..").join("data"));
    }
    candidates
}

/// First existing candidate from [`data_dir_candidates`].
pub fn locate_data_dir() -> Option<PathBuf> {
    data_dir_candidates().into_iter().find(|path| path.is_dir())
}
