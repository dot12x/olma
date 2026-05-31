use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow, transactions::{PackageChange, TransactionRow}};

pub async fn run(target: &str, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let db = Db::open(&config)?;

    let family = family_of(target);
    let chosen = resolve_choice(&db, target, family)?;
    let current = current_default(&db, family)?;

    if current.as_ref().map(|r| r.name.as_str() == chosen.name.as_str()).unwrap_or(false) {
        reporter.success(&format!("{} is already the default for {family}", chosen.name));
        return Ok(());
    }

    let chosen_dir = config.package_dir(&chosen.name, &chosen.version);
    if !chosen_dir.exists() {
        return Err(OlmaError::Other(format!(
            "{}@{} is not installed; run `olma add {target}` first",
            chosen.name, chosen.version
        )));
    }

    relink_family(&config, family, &chosen_dir)?;

    let mut change = PackageChange {
        name: family.to_string(),
        from_version: current.as_ref().map(|r| format!("{}:{}", r.name, r.version)),
        to_version: Some(format!("{}:{}", chosen.name, chosen.version)),
        requested: true,
    };
    if change.from_version.is_none() { change.from_version = Some(String::new()); }
    TransactionRow::insert(&db, "default", &[change])?;

    reporter.success(&format!("default for {family} → {}", chosen.name));
    Ok(())
}

pub fn family_of(name: &str) -> &str {
    name.split('@').next().unwrap_or(name)
}

fn resolve_choice(db: &Db, target: &str, family: &str) -> Result<PackageRow> {
    let rows = PackageRow::list(db)?;
    let exact = rows.iter().find(|r| r.name == target);
    if let Some(r) = exact { return Ok(r.clone()); }
    let candidates: Vec<&PackageRow> = rows.iter()
        .filter(|r| family_of(&r.name) == family)
        .collect();
    if candidates.is_empty() {
        return Err(OlmaError::Other(format!("no installed package in family {family}")));
    }
    if !target.contains('@') {
        if let Some(r) = candidates.iter().find(|r| r.name == family) {
            return Ok((*r).clone());
        }
        let mut sorted: Vec<&&PackageRow> = candidates.iter().collect();
        sorted.sort_by(|a, b| b.name.cmp(&a.name));
        return Ok((**sorted[0]).clone());
    }
    Err(OlmaError::Other(format!("{target} is not installed; run `olma add {target}` first")))
}

fn current_default(db: &Db, family: &str) -> Result<Option<PackageRow>> {
    let rows = PackageRow::list(db)?;
    Ok(rows.into_iter().find(|r| r.name == family))
}

pub(crate) fn relink_family(config: &Config, family: &str, version_dir: &std::path::Path) -> Result<()> {
    let bin_src = version_dir.join("bin");
    if !bin_src.exists() { return Ok(()); }
    let bin_dst = config.bin();
    std::fs::create_dir_all(&bin_dst)?;
    for entry in std::fs::read_dir(&bin_src)? {
        let entry = entry?;
        let raw_name = entry.file_name().to_string_lossy().to_string();
        let stripped = strip_family_suffix(&raw_name, family);
        let link = bin_dst.join(&stripped);
        if link.is_symlink() || link.exists() {
            let _ = std::fs::remove_file(&link);
        }
        std::os::unix::fs::symlink(entry.path(), &link)?;
    }
    Ok(())
}

fn strip_family_suffix(exe: &str, family: &str) -> String {
    if let Some(stem) = exe.strip_prefix(family)
        && stem.starts_with('-')
        && stem.chars().skip(1).all(|c| c.is_ascii_digit() || c == '.')
    {
        return family.to_string();
    }
    exe.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_extraction() {
        assert_eq!(family_of("python"), "python");
        assert_eq!(family_of("python@3.11"), "python");
        assert_eq!(family_of("node@18"), "node");
    }

    #[test]
    fn strips_versioned_binary_suffix() {
        assert_eq!(strip_family_suffix("python3.11", "python"), "python3.11");
        assert_eq!(strip_family_suffix("python-3.11", "python"), "python");
        assert_eq!(strip_family_suffix("tree", "tree"), "tree");
    }
}
