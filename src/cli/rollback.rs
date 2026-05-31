use crate::cli::default::{family_of, relink_family};
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow, transactions::TransactionRow};

pub async fn run(yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let last = TransactionRow::latest_unreverted(&db)?
        .ok_or_else(|| OlmaError::Other("nothing to rollback".into()))?;
    let changes = last.parse_changes()?;

    reporter.status(&format!("Reverting transaction #{} ({})", last.id, last.kind));
    for c in &changes {
        match (c.from_version.as_deref(), c.to_version.as_deref()) {
            (None, Some(to)) => reporter.status(&format!("  remove {} {to}", c.name)),
            (Some(from), None) => reporter.status(&format!("  reinstall {} {from}", c.name)),
            (Some(from), Some(to)) => reporter.status(&format!("  revert {} {to}→{from}", c.name)),
            _ => {}
        }
    }

    match last.kind.as_str() {
        "add" => {
            let names: Vec<String> = changes.iter().map(|c| c.name.clone()).collect();
            crate::cli::remove::run(&names, yes, reporter).await?;
        }
        "remove" => {
            let names: Vec<String> = changes.iter().map(|c| c.name.clone()).collect();
            crate::cli::add::run(
                &names,
                crate::metadata::client::FetchPolicy::CacheFirst,
                yes,
                false,
                reporter,
            ).await?;
        }
        "upgrade" => {
            for c in &changes {
                let Some(prev_ver) = c.from_version.as_deref() else {
                    return Err(OlmaError::Other(format!(
                        "transaction #{} for {} has no previous version", last.id, c.name
                    )));
                };
                let prev_dir = config.package_dir(&c.name, prev_ver);
                if !prev_dir.exists() {
                    return Err(OlmaError::Other(format!(
                        "previous version {prev_ver} of {} is not on disk; rollback unavailable",
                        c.name
                    )));
                }
                let current_dir = c.to_version.as_deref()
                    .map(|v| config.package_dir(&c.name, v));
                relink_family(&config, family_of(&c.name), &prev_dir)?;
                PackageRow::upsert(&db, &PackageRow {
                    name: c.name.clone(),
                    version: prev_ver.to_string(),
                    installed_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64).unwrap_or(0),
                    requested: c.requested,
                    previous_ver: c.to_version.clone(),
                })?;
                let _ = current_dir;
            }
        }
        "default" => {
            for c in &changes {
                let Some(prev_ver) = c.from_version.as_deref() else {
                    return Err(OlmaError::Other(format!(
                        "transaction #{} for {} has no prior default", last.id, c.name
                    )));
                };
                let prev_dir = config.package_dir(&c.name, prev_ver);
                if !prev_dir.exists() {
                    return Err(OlmaError::Other(format!(
                        "previous default {prev_ver} of {} is not on disk", c.name
                    )));
                }
                relink_family(&config, family_of(&c.name), &prev_dir)?;
                if let Some(mut row) = PackageRow::get(&db, &c.name)? {
                    row.version = prev_ver.to_string();
                    PackageRow::upsert(&db, &row)?;
                }
            }
        }
        other => {
            return Err(OlmaError::Other(format!("rollback for {other} not implemented yet")));
        }
    }

    TransactionRow::mark_reverted(&db, last.id)?;
    Ok(())
}
