use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub root: PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        let root = std::env::var_os("OLMA_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/opt/olma"));
        Config { root }
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

    pub fn formulae_api_base(&self) -> &'static str {
        "https://formulae.brew.sh/api/formula"
    }

    pub fn ghcr_base(&self) -> &'static str {
        "https://ghcr.io"
    }

    pub fn check_writable(&self) -> std::io::Result<()> {
        let probe = self.root.join(".write_probe");
        std::fs::write(&probe, b"olma")?;
        std::fs::remove_file(&probe)?;
        Ok(())
    }
}
