# Plan 3 — State, History, Rollback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist what olma installs and what the user has done. Add a SQLite `state.db` with `packages` and `transactions` tables, record every install/remove as a transaction, surface that history via `olma list` and `olma history`, implement `olma remove`, and ship the bounded-N=2 rollback for `add` operations. (Upgrade-rollback comes in Plan 4 alongside `upgrade`.)

**Architecture:** A new `state` module owns the SQLite connection (WAL mode, bundled `rusqlite`), schema migrations, and CRUD for `packages` and `transactions`. The `Pipeline::run` flow records each successful install inside a single SQLite transaction (no half-state). New CLI verbs (`list`, `history`, `remove`, `rollback`) read or mutate the same store. `rollback` of an `add` is equivalent to `remove`, gated by the latest non-reverted transaction.

**Tech Stack:** Rust 2024 · `rusqlite` (bundled SQLite). No new heavy deps.

**Depends on:** Plan 1 (foundation), Plan 2 (Pipeline, Resolver, multi-package add).

Spec reference: `docs/superpowers/specs/2026-05-31-olma-architecture-design.md` §3 (state.db location), §10 (rollback & history).

---

## File structure (touched in this plan)

```
src/
├── state/
│   ├── mod.rs              NEW   Db struct, connection, with_conn
│   ├── schema.rs           NEW   migration ladder
│   ├── packages.rs         NEW   PackageRow CRUD
│   └── transactions.rs     NEW   TransactionRow CRUD
├── pipeline/mod.rs         MODIFY  record on successful link
├── cli/
│   ├── mod.rs              MODIFY  register list / history / remove / rollback
│   ├── list.rs             NEW
│   ├── history.rs          NEW
│   ├── remove.rs           NEW
│   └── rollback.rs         NEW

Cargo.toml                  MODIFY  add rusqlite bundled

tests/
├── unit_state.rs           NEW   schema migrations + packages/transactions CRUD
└── integration_add_list_remove.rs   NEW   end-to-end full lifecycle
```

---

## Task 1: Add `rusqlite` and `Db` skeleton

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/lib.rs`
- Create: `src/state/mod.rs`
- Create: `src/state/schema.rs`

- [ ] **Step 1: Add the dep**

In `[dependencies]` (alphabetical, between `reqwest` and `serde`):

```toml
rusqlite = { version = "0.31", features = ["bundled"] }
```

- [ ] **Step 2: Register the module**

In `src/lib.rs`, add `pub mod state;` alphabetically (after `pub mod resolver;`).

- [ ] **Step 3: Create `src/state/mod.rs` with `Db` struct**

```rust
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
```

- [ ] **Step 4: Create `src/state/schema.rs`**

```rust
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
```

- [ ] **Step 5: Build**

Run: `cargo check --lib 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/state/mod.rs src/state/schema.rs
git commit -m "feat(state): add SQLite Db with WAL mode and schema migrations"
```

---

## Task 2: `packages` table CRUD

**Files:**
- Create: `src/state/packages.rs`

- [ ] **Step 1: Implement**

```rust
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
```

- [ ] **Step 2: Commit**

```bash
cargo check --lib 2>&1 | tail -5
git add src/state/packages.rs
git commit -m "feat(state): PackageRow CRUD over the packages table"
```

---

## Task 3: `transactions` table CRUD

**Files:**
- Create: `src/state/transactions.rs`

- [ ] **Step 1: Implement**

```rust
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
```

- [ ] **Step 2: Commit**

```bash
cargo check --lib 2>&1 | tail -5
git add src/state/transactions.rs
git commit -m "feat(state): TransactionRow CRUD and change payload type"
```

---

## Task 4: Schema + CRUD unit tests

**Files:**
- Create: `tests/unit_state.rs`

- [ ] **Step 1: Test the migration ladder and round-trips**

```rust
use olma::config::Config;
use olma::state::{Db, packages::PackageRow, transactions::{TransactionRow, PackageChange}};

fn temp_config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::Builder::new().prefix("olma-state-").tempdir().unwrap();
    unsafe { std::env::set_var("OLMA_ROOT", tmp.path()); }
    let config = Config::from_env();
    (tmp, config)
}

#[test]
fn migration_runs_idempotently() {
    let (_t, c) = temp_config();
    let db1 = Db::open(&c).unwrap();
    drop(db1);
    let _db2 = Db::open(&c).unwrap();
}

#[test]
fn packages_round_trip() {
    let (_t, c) = temp_config();
    let db = Db::open(&c).unwrap();
    PackageRow::upsert(&db, &PackageRow {
        name: "tree".into(),
        version: "2.3.2".into(),
        installed_at: 100,
        requested: true,
        previous_ver: None,
    }).unwrap();
    let got = PackageRow::get(&db, "tree").unwrap().unwrap();
    assert_eq!(got.version, "2.3.2");
    assert!(got.requested);
}

#[test]
fn transactions_log_and_revert() {
    let (_t, c) = temp_config();
    let db = Db::open(&c).unwrap();
    let id = TransactionRow::insert(&db, "add", &[PackageChange {
        name: "tree".into(),
        from_version: None,
        to_version: Some("2.3.2".into()),
        requested: true,
    }]).unwrap();
    let latest = TransactionRow::latest_unreverted(&db).unwrap().unwrap();
    assert_eq!(latest.id, id);
    TransactionRow::mark_reverted(&db, id).unwrap();
    assert!(TransactionRow::latest_unreverted(&db).unwrap().is_none());
}
```

- [ ] **Step 2: Run**

```bash
cargo test --test unit_state 2>&1 | tail -10
```

Expected: 3 passing.

- [ ] **Step 3: Commit**

```bash
git add tests/unit_state.rs
git commit -m "test(state): migration, packages, transactions round trip"
```

---

## Task 5: Record installs in `Pipeline::run`

**Files:**
- Modify: `src/pipeline/mod.rs`

After the link phase succeeds, open `Db`, upsert each linked package, and insert one `add` transaction with all packages.

- [ ] **Step 1: Patch `Pipeline::run`**

Add at the top of the file:

```rust
use crate::state::{Db, packages::PackageRow, transactions::{PackageChange, TransactionRow}};
```

After the existing link loop (before the final `success` message), add:

```rust
let db = Db::open(&self.config)?;
let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0);
let mut changes = Vec::with_capacity(linked.len());
for item in &linked {
    let was = PackageRow::get(&db, &item.formula.name)?.map(|r| r.version);
    PackageRow::upsert(&db, &PackageRow {
        name: item.formula.name.clone(),
        version: item.formula.version().to_string(),
        installed_at: now,
        requested: plan.requested.contains(&item.formula.name),
        previous_ver: was.clone(),
    })?;
    changes.push(PackageChange {
        name: item.formula.name.clone(),
        from_version: was,
        to_version: Some(item.formula.version().to_string()),
        requested: plan.requested.contains(&item.formula.name),
    });
}
TransactionRow::insert(&db, "add", &changes)?;
```

- [ ] **Step 2: Build + existing tests still pass**

```bash
cargo build 2>&1 | tail -5
cargo test --test integration_add_tree -- --nocapture 2>&1 | tail -10
```

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/mod.rs
git commit -m "feat(pipeline): record installs in packages + transactions"
```

---

## Task 6: `olma list`

**Files:**
- Modify: `src/cli/mod.rs` (register subcommand)
- Create: `src/cli/list.rs`

- [ ] **Step 1: Add the subcommand**

```rust
// In Cmd enum:
List,
// In dispatch:
Cmd::List => list::run(reporter.as_ref()).await,
```

- [ ] **Step 2: Implement `src/cli/list.rs`**

```rust
use crate::config::Config;
use crate::error::Result;
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};

pub async fn run(_reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let rows = PackageRow::list(&db)?;
    if rows.is_empty() {
        println!("  (no packages installed)");
        return Ok(());
    }
    for row in rows {
        let marker = if row.requested { "●" } else { " " };
        println!("  {marker}  {:<24} {}", row.name, row.version);
    }
    Ok(())
}
```

- [ ] **Step 3: Smoke test**

```bash
cargo build
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma add -y tree
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma list
```

Expected: `●  tree                     2.3.2`.

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/list.rs
git commit -m "feat(cli): olma list reads installed packages from state.db"
```

---

## Task 7: `olma history`

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/history.rs`

- [ ] **Step 1: Add the subcommand**

```rust
History {
    #[arg(long, default_value_t = 20)]
    limit: i64,
},
// dispatch:
Cmd::History { limit } => history::run(limit, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
use crate::config::Config;
use crate::error::Result;
use crate::output::Reporter;
use crate::state::{Db, transactions::TransactionRow};

pub async fn run(limit: i64, _reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let rows = TransactionRow::list(&db, limit)?;
    if rows.is_empty() {
        println!("  (no transactions yet)");
        return Ok(());
    }
    for row in rows {
        let changes = row.parse_changes()?;
        let names: Vec<String> = changes.iter().map(|c| {
            match (&c.from_version, &c.to_version) {
                (None, Some(to)) => format!("{} {}", c.name, to),
                (Some(from), Some(to)) => format!("{} {}→{}", c.name, from, to),
                (Some(from), None) => format!("{}- {}", c.name, from),
                (None, None) => c.name.clone(),
            }
        }).collect();
        let r = if row.reverted { " (reverted)" } else { "" };
        println!("  #{:<4}  {:<8}  {}{}", row.id, row.kind, names.join(", "), r);
    }
    Ok(())
}
```

- [ ] **Step 3: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma history
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/history.rs
git commit -m "feat(cli): olma history prints transaction log"
```

---

## Task 8: `olma remove`

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/remove.rs`

- [ ] **Step 1: Add the subcommand**

```rust
Remove {
    #[arg(required = true)]
    names: Vec<String>,
},
// dispatch:
Cmd::Remove { names } => remove::run(&names, cli.yes, reporter.as_ref()).await,
```

Also add a `rm` alias:

```rust
#[command(alias = "rm")]
Remove { /* ... */ },
```

- [ ] **Step 2: Implement `src/cli/remove.rs`**

```rust
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow, transactions::{PackageChange, TransactionRow}};

pub async fn run(names: &[String], yes: bool, reporter: &dyn Reporter) -> Result<()> {
    if names.is_empty() {
        return Err(OlmaError::Other("no packages specified".into()));
    }
    let config = Config::from_env();
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let db = Db::open(&config)?;

    let mut to_remove: Vec<PackageRow> = Vec::new();
    for name in names {
        let row = PackageRow::get(&db, name)?
            .ok_or_else(|| OlmaError::Other(format!("{name} is not installed")))?;
        to_remove.push(row);
    }

    reporter.status(&format!("Will remove {} packages:", to_remove.len()));
    for row in &to_remove {
        reporter.status(&format!("  {} {}", row.name, row.version));
    }
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    let mut changes = Vec::with_capacity(to_remove.len());
    for row in &to_remove {
        let pkg_dir = config.package_dir(&row.name, &row.version);
        if pkg_dir.exists() {
            std::fs::remove_dir_all(&pkg_dir)?;
        }
        let _ = std::fs::remove_dir(config.packages().join(&row.name));
        let bin_src = config.package_dir(&row.name, &row.version).join("bin");
        let _ = std::fs::remove_dir_all(&bin_src);
        let opt_link = config.opt_link(&row.name);
        if opt_link.is_symlink() || opt_link.exists() {
            let _ = std::fs::remove_file(&opt_link);
        }
        unlink_dangling_bins(&config)?;
        PackageRow::delete(&db, &row.name)?;
        changes.push(PackageChange {
            name: row.name.clone(),
            from_version: Some(row.version.clone()),
            to_version: None,
            requested: row.requested,
        });
    }
    TransactionRow::insert(&db, "remove", &changes)?;
    reporter.success(&format!("Removed {} packages", to_remove.len()));
    Ok(())
}

fn unlink_dangling_bins(config: &Config) -> Result<()> {
    let bin = config.bin();
    if !bin.exists() { return Ok(()); }
    for entry in std::fs::read_dir(&bin)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_symlink() {
            if let Ok(target) = std::fs::read_link(&path)
                && !target.exists()
                && !bin.join(&target).exists()
            {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
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
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed.is_empty() || trimmed == "y" || trimmed == "yes")
}
```

- [ ] **Step 3: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma add -y tree
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma list
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma remove -y tree
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma list  # → (no packages installed)
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/remove.rs
git commit -m "feat(cli): olma remove (alias rm) deletes files and state row"
```

---

## Task 9: `olma rollback` for the last `add`

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/rollback.rs`

For Plan 3 we support rolling back the most recent `add` and `remove` transactions. `remove` rollback re-runs `add` for the package. `upgrade`/`readd` rollback lands with Plan 4 when those verbs exist.

- [ ] **Step 1: Add the subcommand**

```rust
Rollback,
// dispatch:
Cmd::Rollback => rollback::run(cli.yes, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
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
```

- [ ] **Step 3: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma add -y tree
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma rollback -y
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma list  # → empty
OLMA_ROOT=/tmp/olma-p3 ./target/debug/olma history  # → #2 remove tree- 2.3.2; #1 add tree 2.3.2 (reverted)
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/rollback.rs
git commit -m "feat(cli): olma rollback reverts last add or remove transaction"
```

---

## Task 10: Integration test — full lifecycle

**Files:**
- Create: `tests/integration_add_list_remove.rs`

- [ ] **Step 1: Test add → list → remove → list → rollback**

```rust
mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_lifecycle() {
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok() {
        eprintln!("skipped");
        return;
    }
    let sandbox = Sandbox::new();
    let reporter = olma::output::default_reporter();

    olma::cli::add::run(
        &["tree".into()],
        olma::metadata::client::FetchPolicy::CacheFirst,
        true,
        false,
        reporter.as_ref(),
    ).await.unwrap();

    assert!(sandbox.bin().join("tree").exists());

    let config = olma::config::Config::from_env();
    let db = olma::state::Db::open(&config).unwrap();
    let rows = olma::state::packages::PackageRow::list(&db).unwrap();
    assert_eq!(rows.iter().filter(|r| r.name == "tree").count(), 1);

    olma::cli::remove::run(
        &["tree".into()],
        true,
        reporter.as_ref(),
    ).await.unwrap();

    assert!(!sandbox.bin().join("tree").exists());
    let rows_after = olma::state::packages::PackageRow::list(&db).unwrap();
    assert!(rows_after.iter().all(|r| r.name != "tree"));

    olma::cli::rollback::run(true, reporter.as_ref()).await.unwrap();
    assert!(sandbox.bin().join("tree").exists());
}
```

- [ ] **Step 2: Run**

```bash
cargo test --test integration_add_list_remove -- --nocapture 2>&1 | tail -30
```

Expected: pass.

- [ ] **Step 3: Commit**

```bash
git add tests/integration_add_list_remove.rs
git commit -m "test: full lifecycle integration test for add/list/remove/rollback"
```

---

## Task 11: Self-review checkpoint

- [ ] **Step 1: Full test suite**

```bash
cargo test 2>&1 | grep -E '^test result|Running' | tail -20
```

Expected: every previous and new test passes.

- [ ] **Step 2: Clippy**

```bash
cargo clippy --all-targets -- -D warnings
```

Expected: clean.

- [ ] **Step 3: Smoke test**

```bash
SBX=/tmp/olma-p3-smoke && rm -rf $SBX
OLMA_ROOT=$SBX ./target/release/olma add -y jq tree
OLMA_ROOT=$SBX ./target/release/olma list
OLMA_ROOT=$SBX ./target/release/olma history
OLMA_ROOT=$SBX ./target/release/olma rollback -y
OLMA_ROOT=$SBX ./target/release/olma list
OLMA_ROOT=$SBX ./target/release/olma history
OLMA_ROOT=$SBX ./target/release/olma rm -y oniguruma
OLMA_ROOT=$SBX ./target/release/olma list
```

- [ ] **Step 4: Final commit if needed**

```bash
git status
git add -A
git commit -m "chore: address clippy warnings from Plan 3 self-review"
```

---

## Plan-level verification

After Task 11:

1. `state.db` lives under `<root>/state.db` with WAL mode and proper migrations.
2. Every install/remove writes a `transactions` row; `packages` reflects the live set.
3. `olma list` and `olma history` read from the same store.
4. `olma remove` cleans files, symlinks, and state in a single locked operation.
5. `olma rollback` reverts the latest `add` or `remove` transaction.

## Out of scope (reminder)

- `upgrade`, `readd`, and their rollback semantics → Plan 4.
- `info`, `search`, `outdated`, `clean`, `autoclean`, `autoremove`, `purge`, `default` → Plan 4.
- Bounded N=2 generation cleanup → Plan 4 (lands with `upgrade`).
- Services → Plan 6.
- Polish (verbose, JSON, did-you-mean, completions, self-update) → Plan 5.
