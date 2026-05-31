use crate::config::Config;
use crate::error::Result;
use std::os::unix::fs as unix_fs;
use std::path::Path;

#[derive(Debug, Default)]
pub struct LinkStats {
    pub linked: Vec<String>,
    pub skipped_collisions: Vec<String>,
}

pub fn link_bin(config: &Config, package_dir: &Path) -> Result<LinkStats> {
    let mut stats = LinkStats::default();
    let bin_src = package_dir.join("bin");
    if !bin_src.exists() {
        return Ok(stats);
    }
    std::fs::create_dir_all(config.bin())?;
    for entry in std::fs::read_dir(&bin_src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let link = config.bin().join(&name);
        let target = entry.path();

        if link.is_symlink() {
            let existing = std::fs::read_link(&link)?;
            let absolute = if existing.is_absolute() {
                existing.clone()
            } else {
                config.bin().join(existing)
            };
            if absolute.starts_with(package_dir) {
                std::fs::remove_file(&link)?;
            } else {
                stats.skipped_collisions.push(name);
                continue;
            }
        } else if link.exists() {
            stats.skipped_collisions.push(name);
            continue;
        }

        unix_fs::symlink(&target, &link)?;
        stats.linked.push(name);
    }
    Ok(stats)
}
