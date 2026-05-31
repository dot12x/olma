pub mod packages;
pub mod schema;
pub mod transactions;

use crate::config::Config;
use crate::error::{OlmaError, Result};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Db {
    pub fn open(config: &Config) -> Result<Self> {
        std::fs::create_dir_all(&config.root)?;
        let path = config.root.join("state.db");
        let conn = Connection::open(&path)
            .map_err(|e| OlmaError::Other(format!("open state.db: {e}")))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
            .map_err(|e| OlmaError::Other(format!("pragmas: {e}")))?;
        schema::apply(&conn)?;
        Ok(Self { conn: Mutex::new(conn), path })
    }

    pub fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<T>,
    {
        let guard = self.conn.lock().map_err(|e| OlmaError::Other(format!("db mutex: {e}")))?;
        f(&guard).map_err(|e| OlmaError::Other(format!("sqlite: {e}")))
    }

    pub fn with_tx<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&rusqlite::Transaction) -> rusqlite::Result<T>,
    {
        let mut guard = self.conn.lock().map_err(|e| OlmaError::Other(format!("db mutex: {e}")))?;
        let tx = guard.transaction().map_err(|e| OlmaError::Other(format!("begin tx: {e}")))?;
        let out = f(&tx).map_err(|e| OlmaError::Other(format!("sqlite tx: {e}")))?;
        tx.commit().map_err(|e| OlmaError::Other(format!("commit: {e}")))?;
        Ok(out)
    }

    pub fn path(&self) -> &std::path::Path { &self.path }
}
