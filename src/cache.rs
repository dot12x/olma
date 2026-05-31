use crate::config::Config;
use crate::error::{OlmaError, Result};
use std::path::PathBuf;

pub struct FormulaCacheEntry {
    pub json: String,
    pub etag: Option<String>,
}

pub struct FormulaCache<'a> {
    config: &'a Config,
}

impl<'a> FormulaCache<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    pub fn formula_path(&self, name: &str) -> PathBuf {
        self.config.cache_formulae().join(format!("{name}.json"))
    }

    pub fn etag_path(&self, name: &str) -> PathBuf {
        self.config.cache_formulae().join(format!("{name}.etag"))
    }

    pub fn read(&self, name: &str) -> Result<Option<FormulaCacheEntry>> {
        let jp = self.formula_path(name);
        if !jp.exists() {
            return Ok(None);
        }
        let json = std::fs::read_to_string(&jp)?;
        let etag = std::fs::read_to_string(self.etag_path(name)).ok();
        Ok(Some(FormulaCacheEntry { json, etag }))
    }

    pub fn write(&self, name: &str, json: &str, etag: Option<&str>) -> Result<()> {
        std::fs::create_dir_all(self.config.cache_formulae())?;
        let jp = self.formula_path(name);
        let tmp = jp.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &jp)?;
        if let Some(e) = etag {
            std::fs::write(self.etag_path(name), e)?;
        }
        Ok(())
    }

    pub fn age_days(&self, name: &str) -> Result<Option<u64>> {
        let jp = self.formula_path(name);
        if !jp.exists() {
            return Ok(None);
        }
        let meta = std::fs::metadata(&jp)?;
        let modified = meta.modified()?;
        let elapsed = std::time::SystemTime::now()
            .duration_since(modified)
            .map_err(|e| OlmaError::Other(format!("clock error: {e}")))?;
        Ok(Some(elapsed.as_secs() / 86_400))
    }
}
