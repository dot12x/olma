use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    /// Reads `OLMA_ROOT` env var if set (used for sandbox tests),
    /// otherwise returns `/opt/olma`.
    pub fn from_env() -> Self {
        let root = std::env::var_os("OLMA_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/opt/olma"));
        Paths { root }
    }

    pub fn bin(&self) -> PathBuf { self.root.join("bin") }
    pub fn packages(&self) -> PathBuf { self.root.join("packages") }
    pub fn cache(&self) -> PathBuf { self.root.join("cache") }
    pub fn cache_bottles(&self) -> PathBuf { self.cache().join("bottles") }
    pub fn cache_formulae(&self) -> PathBuf { self.cache().join("formulae") }
    pub fn lock_file(&self) -> PathBuf { self.root.join(".lock") }

    pub fn package_dir(&self, name: &str, version: &str) -> PathBuf {
        self.packages().join(name).join(version)
    }

    /// Ensures the directory layout exists. Creates dirs as needed.
    pub fn ensure_layout(&self) -> std::io::Result<()> {
        for p in [
            self.bin(),
            self.packages(),
            self.cache_bottles(),
            self.cache_formulae(),
        ] {
            std::fs::create_dir_all(&p)?;
        }
        Ok(())
    }
}

/// Path to ghcr.io's host name template (extension point for tests).
pub fn formulae_api_base() -> &'static str {
    "https://formulae.brew.sh/api/formula"
}

pub fn ghcr_base() -> &'static str {
    "https://ghcr.io"
}

/// Sanity check: returns Err if the root is not writable.
pub fn check_writable(p: &Path) -> std::io::Result<()> {
    let probe = p.join(".write_probe");
    std::fs::write(&probe, b"olma")?;
    std::fs::remove_file(&probe)?;
    Ok(())
}
