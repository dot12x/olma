use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::output::Reporter;
use crate::state::{Db, transactions::TransactionRow};

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
        other => {
            return Err(OlmaError::Other(format!("rollback for {other} not implemented yet")));
        }
    }

    TransactionRow::mark_reverted(&db, last.id)?;
    Ok(())
}
