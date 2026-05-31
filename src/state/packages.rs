use crate::error::Result;
use crate::state::Db;
use rusqlite::OptionalExtension;

#[derive(Debug, Clone)]
pub struct PackageRow {
    pub name: String,
    pub version: String,
    pub installed_at: i64,
    pub requested: bool,
    pub previous_ver: Option<String>,
}

impl PackageRow {
    pub fn upsert(db: &Db, row: &PackageRow) -> Result<()> {
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO packages (name, version, installed_at, requested, previous_ver)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(name) DO UPDATE SET
                     version       = excluded.version,
                     installed_at  = excluded.installed_at,
                     requested     = excluded.requested,
                     previous_ver  = excluded.previous_ver",
                rusqlite::params![row.name, row.version, row.installed_at, row.requested as i64, row.previous_ver],
            ).map(|_| ())
        })
    }

    pub fn get(db: &Db, name: &str) -> Result<Option<PackageRow>> {
        db.with_conn(|c| {
            c.query_row(
                "SELECT name, version, installed_at, requested, previous_ver FROM packages WHERE name = ?1",
                rusqlite::params![name],
                |row| Ok(PackageRow {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    installed_at: row.get(2)?,
                    requested: row.get::<_, i64>(3)? != 0,
                    previous_ver: row.get(4)?,
                }),
            ).optional()
        })
    }

    pub fn list(db: &Db) -> Result<Vec<PackageRow>> {
        db.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT name, version, installed_at, requested, previous_ver FROM packages ORDER BY name"
            )?;
            let rows = stmt.query_map([], |row| Ok(PackageRow {
                name: row.get(0)?,
                version: row.get(1)?,
                installed_at: row.get(2)?,
                requested: row.get::<_, i64>(3)? != 0,
                previous_ver: row.get(4)?,
            }))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn delete(db: &Db, name: &str) -> Result<()> {
        db.with_conn(|c| {
            c.execute("DELETE FROM packages WHERE name = ?1", rusqlite::params![name]).map(|_| ())
        })
    }
}
