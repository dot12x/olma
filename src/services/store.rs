use crate::error::Result;
use crate::state::Db;
use rusqlite::OptionalExtension;

#[derive(Debug, Clone)]
pub struct ServiceRow {
    pub name: String,
    pub plist_path: String,
    pub enabled: bool,
    pub last_action: Option<String>,
    pub last_action_ts: Option<i64>,
}

impl ServiceRow {
    pub fn upsert(db: &Db, row: &ServiceRow) -> Result<()> {
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO services (name, plist_path, enabled, last_action, last_action_ts)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(name) DO UPDATE SET
                     plist_path = excluded.plist_path,
                     enabled = excluded.enabled,
                     last_action = excluded.last_action,
                     last_action_ts = excluded.last_action_ts",
                rusqlite::params![
                    row.name,
                    row.plist_path,
                    row.enabled as i64,
                    row.last_action,
                    row.last_action_ts
                ],
            )?;
            Ok(())
        })
    }

    pub fn get(db: &Db, name: &str) -> Result<Option<ServiceRow>> {
        db.with_conn(|c| {
            c.query_row(
                "SELECT name, plist_path, enabled, last_action, last_action_ts FROM services WHERE name = ?1",
                rusqlite::params![name],
                |row| {
                    Ok(ServiceRow {
                        name: row.get(0)?,
                        plist_path: row.get(1)?,
                        enabled: row.get::<_, i64>(2)? != 0,
                        last_action: row.get(3)?,
                        last_action_ts: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn list(db: &Db) -> Result<Vec<ServiceRow>> {
        db.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT name, plist_path, enabled, last_action, last_action_ts FROM services ORDER BY name",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(ServiceRow {
                        name: row.get(0)?,
                        plist_path: row.get(1)?,
                        enabled: row.get::<_, i64>(2)? != 0,
                        last_action: row.get(3)?,
                        last_action_ts: row.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn delete(db: &Db, name: &str) -> Result<()> {
        db.with_conn(|c| {
            c.execute(
                "DELETE FROM services WHERE name = ?1",
                rusqlite::params![name],
            )?;
            Ok(())
        })
    }
}
