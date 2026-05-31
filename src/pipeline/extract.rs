use crate::error::Result;
use std::path::{Path, PathBuf};

pub async fn extract(archive_path: &Path, dest_dir: &Path) -> Result<PathBuf> {
    let archive = archive_path.to_path_buf();
    let dest = dest_dir.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        std::fs::create_dir_all(&dest)?;
        let f = std::fs::File::open(&archive)?;
        let gz = flate2::read::GzDecoder::new(f);
        let mut tar = tar::Archive::new(gz);
        tar.unpack(&dest)?;
        for entry in std::fs::read_dir(&dest)? {
            let entry = entry?;
            if entry.path().is_dir() {
                return Ok(entry.path());
            }
        }
        Ok(dest)
    })
    .await
    .map_err(|e| crate::error::OlmaError::Other(format!("extract task failed: {e}")))?
}

pub async fn extract_and_relocate(
    item: &crate::pipeline::download::DownloadedWithFormula,
    config: &crate::config::Config,
) -> Result<()> {
    use crate::pipeline::relocate;
    let staging = config.cache().join("staging")
        .join(format!("{}-{}", item.formula.name, item.formula.version()));
    if staging.exists() { std::fs::remove_dir_all(&staging)?; }
    std::fs::create_dir_all(&staging)?;
    extract(&item.dl.path, &staging).await?;
    let name_dir = staging.join(&item.formula.name);
    let inner = first_subdir(&name_dir).ok_or_else(|| crate::error::OlmaError::Other(format!(
        "unexpected bottle layout: no version directory under {}",
        name_dir.display()
    )))?;
    let bottle_version_dir = inner.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| item.formula.version().to_string());
    let dest = config.package_dir(&item.formula.name, item.formula.version());
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(&inner, &dest)?;
    let _ = std::fs::remove_dir_all(&staging);

    let new_cellar_pkg = dest.to_string_lossy().to_string();
    let new_packages = config.packages().to_string_lossy().to_string();
    let new_root = config.root.to_string_lossy().to_string();
    let swaps: Vec<(String, String)> = vec![
        (format!("@@HOMEBREW_CELLAR@@/{}/{}", item.formula.name, bottle_version_dir), new_cellar_pkg.clone()),
        (format!("/opt/homebrew/Cellar/{}/{}", item.formula.name, bottle_version_dir), new_cellar_pkg.clone()),
        (format!("/usr/local/Cellar/{}/{}", item.formula.name, bottle_version_dir), new_cellar_pkg.clone()),
        ("@@HOMEBREW_CELLAR@@".into(), new_packages.clone()),
        ("@@HOMEBREW_PREFIX@@".into(), new_root.clone()),
        ("/opt/homebrew".into(), new_root.clone()),
        ("/usr/local".into(), new_root.clone()),
    ];
    relocate::relocate_tree(&dest, swaps).await?;
    Ok(())
}

fn first_subdir(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?
        .filter_map(|e| e.ok())
        .find(|e| e.path().is_dir())
        .map(|e| e.path())
}
