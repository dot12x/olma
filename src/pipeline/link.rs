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
    link_opt(config, package_dir)?;
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

fn link_opt(config: &Config, package_dir: &Path) -> Result<()> {
    let name = package_dir.parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| crate::error::OlmaError::Other(format!(
            "cannot infer package name from {}", package_dir.display()
        )))?;
    std::fs::create_dir_all(config.opt())?;
    let link = config.opt_link(&name);
    if link.exists() || link.is_symlink() {
        let _ = std::fs::remove_file(&link);
    }
    unix_fs::symlink(package_dir, &link)?;
    Ok(())
}
