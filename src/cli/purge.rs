use crate::config::Config;
use crate::error::Result;
use crate::fs_lock::WriteLock;
use crate::output::Reporter;
use crate::state::Db;

pub async fn run(name: &str, yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;

    let scoped = scoped_bottles(&db, name)?;

    crate::cli::remove::run(&[name.to_string()], yes, reporter).await?;

    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let mut deleted = 0usize;
    for sha in &scoped {
        let path = config.cache_bottles().join(format!("{sha}.tar.gz"));
        if path.exists() {
            let _ = std::fs::remove_file(&path);
            deleted += 1;
        }
    }
    db.with_conn(|c| {
        c.execute("DELETE FROM bottle_consumers WHERE formula = ?1", rusqlite::params![name]).map(|_| ())
    })?;

    reporter.success(&format!("Purged {name} and {deleted} cached bottles"));
    Ok(())
}

fn scoped_bottles(db: &Db, formula: &str) -> Result<Vec<String>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT bottle_sha256 FROM bottle_consumers
             WHERE formula = ?1
               AND bottle_sha256 NOT IN (
                 SELECT bottle_sha256 FROM bottle_consumers WHERE formula <> ?1
               )"
        )?;
        let rows = stmt.query_map(rusqlite::params![formula], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}
