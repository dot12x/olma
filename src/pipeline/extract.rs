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
    let inner = staging.join(&item.formula.name).join(item.formula.version());
    if !inner.is_dir() {
        return Err(crate::error::OlmaError::Other(format!(
            "unexpected bottle layout: expected {}", inner.display()
        )));
    }
    let dest = config.package_dir(&item.formula.name, item.formula.version());
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(&inner, &dest)?;
    let _ = std::fs::remove_dir_all(&staging);

    let new_cellar = dest.to_string_lossy().to_string();
    let new_root = config.root.to_string_lossy().to_string();
    let swaps: Vec<(String, String)> = vec![
        (format!("/opt/homebrew/Cellar/{}/{}", item.formula.name, item.formula.version()), new_cellar.clone()),
        (format!("/usr/local/Cellar/{}/{}", item.formula.name, item.formula.version()), new_cellar.clone()),
        ("/opt/homebrew".into(), new_root.clone()),
        ("/usr/local".into(), new_root.clone()),
    ];
    relocate::relocate_tree(&dest, swaps).await?;
    Ok(())
}
