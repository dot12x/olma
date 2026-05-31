use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow, transactions::{PackageChange, TransactionRow}};

pub async fn run(names: &[String], yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    config.ensure_layout()?;
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let db = Db::open(&config)?;
    let client = FormulaeClient::new(&config)?;

    let mut targets: Vec<(PackageRow, String)> = Vec::new();
    let candidates = if names.is_empty() {
        PackageRow::list(&db)?.into_iter().filter(|r| r.requested).collect::<Vec<_>>()
    } else {
        let mut out = Vec::new();
        for n in names {
            let row = PackageRow::get(&db, n)?
                .ok_or_else(|| OlmaError::Other(format!("{n} is not installed")))?;
            out.push(row);
        }
        out
    };

    for row in candidates {
        let formula = client.fetch(&row.name, FetchPolicy::CacheFirst).await?;
        if formula.version() != row.version {
            targets.push((row, formula.version().to_string()));
        }
    }

    if targets.is_empty() {
        reporter.success("Nothing to upgrade");
        return Ok(());
    }

    reporter.status(&format!("Will upgrade {} packages:", targets.len()));
    for (row, new) in &targets {
        reporter.status(&format!("  {} {} → {}", row.name, row.version, new));
    }
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    let install_names: Vec<String> = targets.iter().map(|(r, _)| r.name.clone()).collect();
    crate::cli::add::run(
        &install_names,
        FetchPolicy::CacheFirst,
        true,
        false,
        reporter,
    ).await?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut changes = Vec::with_capacity(targets.len());
    for (old_row, new_ver) in &targets {
        if let Some(evict) = &old_row.previous_ver {
            let evict_dir = config.package_dir(&old_row.name, evict);
            if evict_dir.exists() {
                let _ = std::fs::remove_dir_all(&evict_dir);
            }
        }
        PackageRow::upsert(&db, &PackageRow {
            name: old_row.name.clone(),
            version: new_ver.clone(),
            installed_at: now,
            requested: old_row.requested,
            previous_ver: Some(old_row.version.clone()),
        })?;
        changes.push(PackageChange {
            name: old_row.name.clone(),
            from_version: Some(old_row.version.clone()),
            to_version: Some(new_ver.clone()),
            requested: old_row.requested,
        });
    }
    TransactionRow::insert(&db, "upgrade", &changes)?;

    reporter.success(&format!("Upgraded {} packages", targets.len()));
    Ok(())
}

fn confirm(yes: bool) -> Result<bool> {
    use std::io::IsTerminal;
    if yes { return Ok(true); }
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
