use crate::error::{OlmaError, Result};
use rusqlite::Connection;

pub const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE IF NOT EXISTS packages (
        name           TEXT NOT NULL,
        version        TEXT NOT NULL,
        installed_at   INTEGER NOT NULL,
        requested      INTEGER NOT NULL DEFAULT 0,
        previous_ver   TEXT,
        PRIMARY KEY (name)
    );
    CREATE TABLE IF NOT EXISTS transactions (
        id        INTEGER PRIMARY KEY AUTOINCREMENT,
        ts        INTEGER NOT NULL,
        kind      TEXT NOT NULL,
        payload   TEXT NOT NULL,
        reverted  INTEGER NOT NULL DEFAULT 0
    );
    "#,
    r#"
    CREATE TABLE IF NOT EXISTS bottle_consumers (
        formula        TEXT NOT NULL,
        bottle_sha256  TEXT NOT NULL,
        PRIMARY KEY (formula, bottle_sha256)
    );
    CREATE INDEX IF NOT EXISTS idx_bottle_consumers_sha ON bottle_consumers (bottle_sha256);
    "#,
    r#"
    CREATE TABLE IF NOT EXISTS services (
        name           TEXT PRIMARY KEY,
        plist_path     TEXT NOT NULL,
        enabled        INTEGER NOT NULL DEFAULT 0,
        last_action    TEXT,
        last_action_ts INTEGER
    );
    "#,
];

pub fn apply(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| OlmaError::Other(format!("read user_version: {e}")))?;
    let target = MIGRATIONS.len() as i64;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let v = (i as i64) + 1;
        if v <= current { continue; }
        conn.execute_batch(sql)
            .map_err(|e| OlmaError::Other(format!("migration {v}: {e}")))?;
    }
    if target > current {
        conn.execute_batch(&format!("PRAGMA user_version = {target};"))
            .map_err(|e| OlmaError::Other(format!("set user_version: {e}")))?;
    }
    Ok(())
}
