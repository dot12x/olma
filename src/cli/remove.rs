use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow, transactions::{PackageChange, TransactionRow}};

pub async fn run(names: &[String], yes: bool, reporter: &dyn Reporter) -> Result<()> {
    if names.is_empty() {
        return Err(OlmaError::Other("no packages specified".into()));
    }
    let config = Config::from_env();
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let db = Db::open(&config)?;

    let mut to_remove: Vec<PackageRow> = Vec::new();
    for name in names {
        let row = PackageRow::get(&db, name)?
            .ok_or_else(|| OlmaError::Other(format!("{name} is not installed")))?;
        to_remove.push(row);
    }

    reporter.status(&format!("Will remove {} packages:", to_remove.len()));
    for row in &to_remove {
        reporter.status(&format!("  {} {}", row.name, row.version));
    }
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    let mut changes = Vec::with_capacity(to_remove.len());
    for row in &to_remove {
        let pkg_dir = config.package_dir(&row.name, &row.version);
        if pkg_dir.exists() {
            std::fs::remove_dir_all(&pkg_dir)?;
        }
        let _ = std::fs::remove_dir(config.packages().join(&row.name));
        let opt_link = config.opt_link(&row.name);
        if opt_link.is_symlink() || opt_link.exists() {
            let _ = std::fs::remove_file(&opt_link);
        }
        unlink_dangling_bins(&config)?;
        PackageRow::delete(&db, &row.name)?;
        changes.push(PackageChange {
            name: row.name.clone(),
            from_version: Some(row.version.clone()),
            to_version: None,
            requested: row.requested,
        });
    }
    TransactionRow::insert(&db, "remove", &changes)?;
    reporter.success(&format!("Removed {} packages", to_remove.len()));
    Ok(())
}

fn unlink_dangling_bins(config: &Config) -> Result<()> {
    let bin = config.bin();
    if !bin.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&bin)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_symlink()
            && let Ok(target) = std::fs::read_link(&path)
        {
            let resolved = if target.is_absolute() {
                target.clone()
            } else {
                bin.join(&target)
            };
            if !resolved.exists() {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

fn confirm(yes: bool) -> Result<bool> {
    use std::io::IsTerminal;
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(OlmaError::Other("non-interactive: use -y".into()));
    }
    eprint!("Proceed? [Y/n] ");
    use std::io::Write;
    let _ = std::io::stderr().flush();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).map_err(OlmaError::Io)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed.is_empty() || trimmed == "y" || trimmed == "yes")
}
