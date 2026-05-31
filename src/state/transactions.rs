use crate::error::Result;
use crate::state::Db;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct TransactionRow {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub payload: String,
    pub reverted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageChange {
    pub name: String,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub requested: bool,
}

impl TransactionRow {
    pub fn insert(db: &Db, kind: &str, changes: &[PackageChange]) -> Result<i64> {
        let payload = serde_json::to_string(changes)
            .map_err(|e| crate::error::OlmaError::Other(format!("payload encode: {e}")))?;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO transactions (ts, kind, payload, reverted) VALUES (?1, ?2, ?3, 0)",
                rusqlite::params![ts, kind, payload],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn list(db: &Db, limit: i64) -> Result<Vec<TransactionRow>> {
        db.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, ts, kind, payload, reverted FROM transactions ORDER BY id DESC LIMIT ?1"
            )?;
            let rows = stmt.query_map(rusqlite::params![limit], |row| Ok(TransactionRow {
                id: row.get(0)?,
                ts: row.get(1)?,
                kind: row.get(2)?,
                payload: row.get(3)?,
                reverted: row.get::<_, i64>(4)? != 0,
            }))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn latest_unreverted(db: &Db) -> Result<Option<TransactionRow>> {
        db.with_conn(|c| {
            use rusqlite::OptionalExtension;
            c.query_row(
                "SELECT id, ts, kind, payload, reverted FROM transactions
                 WHERE reverted = 0 ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok(TransactionRow {
                    id: row.get(0)?,
                    ts: row.get(1)?,
                    kind: row.get(2)?,
                    payload: row.get(3)?,
                    reverted: row.get::<_, i64>(4)? != 0,
                }),
            ).optional()
        })
    }

    pub fn mark_reverted(db: &Db, id: i64) -> Result<()> {
        db.with_conn(|c| {
            c.execute("UPDATE transactions SET reverted = 1 WHERE id = ?1", rusqlite::params![id]).map(|_| ())
        })
    }

    pub fn parse_changes(&self) -> Result<Vec<PackageChange>> {
        serde_json::from_str(&self.payload)
            .map_err(|e| crate::error::OlmaError::Other(format!("payload decode: {e}")))
    }
}
