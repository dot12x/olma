use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::metadata::client::FetchPolicy;
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};

pub async fn run(name: &str, yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let db = Db::open(&config)?;
    let row = PackageRow::get(&db, name)?
        .ok_or_else(|| OlmaError::Other(format!("{name} is not installed")))?;

    let was_requested = row.requested;
    let saved_previous = row.previous_ver.clone();

    drop(_lock);
    crate::cli::remove::run(&[name.to_string()], yes, reporter).await?;
    crate::cli::add::run(
        &[name.to_string()],
        FetchPolicy::CacheFirst,
        true,
        false,
        reporter,
    ).await?;

    let db = Db::open(&config)?;
    if let Some(mut new_row) = PackageRow::get(&db, name)? {
        new_row.requested = was_requested;
        new_row.previous_ver = saved_previous;
        PackageRow::upsert(&db, &new_row)?;
    }

    reporter.success(&format!("Re-added {name}"));
    Ok(())
}
