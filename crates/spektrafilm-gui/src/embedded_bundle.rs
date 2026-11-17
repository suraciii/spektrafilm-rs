//! Embedded release bundle materialization.
//!
//! When the binary is built with `SPEKTRAFILM_EMBED_MANIFEST`, the manifest's
//! data files are written into a per-user cache directory on first launch and
//! the environment is pointed at it. Without the manifest the setup is a no-op.

#[cfg(embed_bundle)]
mod imp {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};

    include!(env!("SPEKTRAFILM_EMBED_MANIFEST"));

    pub fn setup() -> Result<()> {
        let root = cache_root().join(BUNDLE_ID);
        let data_dir = root.join("data");
        let marker = root.join(".ready");

        if !marker.is_file() || !data_dir.join("profiles").is_dir() {
            std::fs::create_dir_all(&data_dir)
                .with_context(|| format!("creating {}", data_dir.display()))?;
            for (rel, bytes) in DATA_FILES {
                write_file(&data_dir.join(rel), bytes)?;
            }
            write_file(&marker, BUNDLE_ID.as_bytes())?;
        }

        // This runs before any worker threads are spawned.
        unsafe {
            std::env::set_var("SPEKTRAFILM_DATA_DIR", &data_dir);
        }
        Ok(())
    }

    fn cache_root() -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("spektrafilm")
            .join("embedded")
    }

    fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut file =
            std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

#[cfg(not(embed_bundle))]
mod imp {
    use anyhow::Result;

    pub fn setup() -> Result<()> {
        Ok(())
    }
}

pub(super) use imp::setup;
