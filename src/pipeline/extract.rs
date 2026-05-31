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
