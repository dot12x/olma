use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};
use std::collections::HashSet;

pub async fn run(yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let db = Db::open(&config)?;

    let installed = PackageRow::list(&db)?;
    let mut to_drop_dirs: Vec<(String, String)> = Vec::new();
    for row in &installed {
        if let Some(prev) = &row.previous_ver {
            let p = config.package_dir(&row.name, prev);
            if p.exists() {
                to_drop_dirs.push((row.name.clone(), prev.clone()));
            }
        }
    }

    let live_shas = live_bottle_shas(&db)?;
    let mut orphan_bottles: Vec<std::path::PathBuf> = Vec::new();
    let bottles = config.cache_bottles();
    if bottles.exists() {
        for entry in std::fs::read_dir(&bottles)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".tar.gz") {
                continue;
            }
            let sha = name.trim_end_matches(".tar.gz").to_string();
            if !live_shas.contains(&sha) {
                orphan_bottles.push(path);
            }
        }
    }

    if to_drop_dirs.is_empty() && orphan_bottles.is_empty() {
        reporter.success("Nothing to clean");
        return Ok(());
    }

    reporter.status(&format!(
        "Will remove {} previous-generation packages and {} orphan bottles",
        to_drop_dirs.len(),
        orphan_bottles.len()
    ));
    if !to_drop_dirs.is_empty() {
        reporter.status("After cleanup, rollback will not be available for these packages:");
        for (n, v) in &to_drop_dirs {
            reporter.status(&format!("  {n} {v}"));
        }
    }
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    for (name, ver) in &to_drop_dirs {
        let dir = config.package_dir(name, ver);
        let _ = std::fs::remove_dir_all(&dir);
        if let Some(mut row) = PackageRow::get(&db, name)?
            && row.previous_ver.as_deref() == Some(ver.as_str())
        {
            row.previous_ver = None;
            PackageRow::upsert(&db, &row)?;
        }
    }
    for p in &orphan_bottles {
        let _ = std::fs::remove_file(p);
    }

    reporter.success(&format!(
        "Cleaned {} previous versions and {} bottles",
        to_drop_dirs.len(),
        orphan_bottles.len()
    ));
    Ok(())
}

fn live_bottle_shas(db: &Db) -> Result<HashSet<String>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT DISTINCT bottle_sha256 FROM bottle_consumers")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().collect())
    })
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
    let t = input.trim().to_lowercase();
    Ok(t.is_empty() || t == "y" || t == "yes")
}
