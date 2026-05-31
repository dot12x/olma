use crate::error::Result;
use crate::relocator::{classify::{classify_path, FileKind}, macho, text};
use std::path::Path;
use walkdir::WalkDir;

/// A list of `(from_prefix, to_prefix)` swaps applied to every file in `dir`.
/// The swaps are tried in order; the first matching prefix is rewritten and the
/// remaining swaps still run independently (Mach-O load commands and text files
/// may carry different references).
pub type SwapList = Vec<(String, String)>;

/// Relocates prefix references inside `dir` according to `swaps`.
pub async fn relocate_tree(dir: &Path, swaps: SwapList) -> Result<RelocStats> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || relocate_tree_blocking(&dir, &swaps))
        .await
        .map_err(|e| crate::error::OlmaError::Other(format!("relocate task failed: {e}")))?
}

#[derive(Debug, Default)]
pub struct RelocStats {
    pub macho_files: usize,
    pub text_files: usize,
    pub symlinks: usize,
}

fn relocate_tree_blocking(dir: &Path, swaps: &[(String, String)]) -> Result<RelocStats> {
    let mut stats = RelocStats::default();

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_symlink() {
            relocate_symlink(path, swaps)?;
            stats.symlinks += 1;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let kind = classify_path(path)?;
        match kind {
            FileKind::MachO => {
                for (old, new) in swaps {
                    macho::relocate_macho(path, old, new)?;
                }
                stats.macho_files += 1;
            }
            FileKind::Text => {
                let mut changed = false;
                for (old, new) in swaps {
                    changed |= text::relocate_text_file(path, old, new)?;
                }
                if changed { stats.text_files += 1; }
            }
            FileKind::Binary => {}
        }
    }
    Ok(stats)
}

fn relocate_symlink(link: &Path, swaps: &[(String, String)]) -> Result<()> {
    let target = std::fs::read_link(link)?;
    let s = target.to_string_lossy().to_string();
    for (old, new) in swaps {
        if let Some(rest) = s.strip_prefix(old.as_str()) {
            let new_target = format!("{new}{rest}");
            std::fs::remove_file(link)?;
            std::os::unix::fs::symlink(&new_target, link)?;
            return Ok(());
        }
    }
    Ok(())
}
