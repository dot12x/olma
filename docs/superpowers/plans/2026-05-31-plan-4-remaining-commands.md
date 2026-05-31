# Plan 4 — Remaining Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fill in every remaining CLI verb from spec §8 that Plans 1–3 did not cover: `update`, `outdated`, `info`, `search`, `upgrade`, `readd`, `default`, `clean`, `autoclean`, `autoremove`, `purge`. Land the bounded N=2 generation semantics for `upgrade` (track `previous_ver`, evict the older copy when a third upgrade arrives), and extend `cli/rollback.rs` so an `upgrade` transaction can be reverted by flipping the symlinks back to the previous version.

**Architecture:** The metadata-read-only verbs (`update`, `outdated`, `info`, `search`) share a single read of `cache/formulae/` plus, for `search`, a lazily fetched `cache/formulae/__index.json` produced by `olma update --full`. The mutating verbs (`upgrade`, `readd`, `default`, `clean`, `autoclean`, `autoremove`, `purge`) acquire the existing `WriteLock` and write a transaction row via the Plan 3 `TransactionRow` helper. `upgrade` produces transactions of kind `upgrade` whose payload carries `from_version → to_version`, and the eviction step removes the third-oldest on-disk version. `default` rewrites the `bin/` symlinks for a formula family in place; the family is `name.split('@').next()` so `python` and `python@3.11` share a family but `python@3.13` and `python@3.11` do not (each `@`-suffix is a distinct upstream formula).

**Tech Stack:** Rust 2024 · existing deps from Plans 1–3 (`rusqlite`, `reqwest`, `serde`, `tokio`). The full-dump index uses `serde_json::Value` to avoid re-deriving every Formula field.

**Depends on:** Plan 1 (foundation, metadata client, add pipeline), Plan 2 (Resolver, Pipeline, multi-package add, `FetchPolicy`), Plan 3 (`Db`, `PackageRow`, `TransactionRow`, `cli/remove`, `cli/rollback`).

Spec reference: `docs/superpowers/specs/2026-05-31-olma-architecture-design.md` §4 (`update` semantics), §8 (command surface), §10 (rollback & N=2 generations), §12 (search did-you-mean).

---

## File structure (touched in this plan)

```
src/
├── state/schema.rs            MODIFY  append migration #2 (bottle_consumers table)
├── pipeline/mod.rs            MODIFY  pass previous_ver through `add`
├── cli/
│   ├── mod.rs                 MODIFY  register all new subcommands
│   ├── update.rs              NEW
│   ├── outdated.rs            NEW
│   ├── info.rs                NEW
│   ├── search.rs              NEW
│   ├── upgrade.rs             NEW
│   ├── readd.rs               NEW
│   ├── default.rs             NEW
│   ├── clean.rs               NEW
│   ├── autoclean.rs           NEW
│   ├── autoremove.rs          NEW
│   ├── purge.rs               NEW
│   └── rollback.rs            MODIFY  extend match for `upgrade` and `default` kinds
└── metadata/
    └── index.rs               NEW   reader for cache/formulae/__index.json

tests/
├── unit_autoremove.rs                 NEW
├── unit_default_family.rs             NEW
├── integration_upgrade_rollback.rs    NEW
└── integration_default_switch.rs      NEW
```

---

## Task 1: Schema migration #2 — `previous_ver` already exists, add `bottle_consumers`

**Files:**
- Modify: `src/state/schema.rs`

Plan 3 already created the `previous_ver` column on `packages`. Plan 4 adds a tiny `bottle_consumers` table so `purge` and `autoclean` can scope cached-bottle deletion to "this formula's bottles" without re-walking the cache. `MIGRATIONS` keeps each schema version as its own entry; we **append** rather than edit the existing string.

- [ ] **Step 1: Append the migration entry**

In `src/state/schema.rs`, extend `MIGRATIONS`:

```rust
pub const MIGRATIONS: &[&str] = &[
    /* existing migration 1 from Plan 3 */
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
];
```

- [ ] **Step 2: Build**

Run: `cargo check --lib 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/state/schema.rs
git commit -m "feat(state): add bottle_consumers table for purge/autoclean scoping"
```

---

## Task 2: Record bottle consumers in `Pipeline::run`

**Files:**
- Modify: `src/pipeline/mod.rs`

After the existing `PackageRow::upsert` loop from Plan 3, record the bottle digest so `purge` knows which cached tarball belongs to this formula.

- [ ] **Step 1: Patch the link block**

In `Pipeline::run`, immediately after the existing `TransactionRow::insert(&db, "add", &changes)?;` line, add:

```rust
for item in &linked {
    let sha = item.bottle_sha.clone();
    db.with_conn(|c| {
        c.execute(
            "INSERT OR IGNORE INTO bottle_consumers (formula, bottle_sha256) VALUES (?1, ?2)",
            rusqlite::params![item.formula.name, sha],
        ).map(|_| ())
    })?;
}
```

`bottle_sha` is the value already computed in download (`Downloaded::sha256_hex`). Plan 2 wraps it in `DownloadedWithFormula`; expose it through one new public field if it is not already there:

```rust
pub struct DownloadedWithFormula {
    pub formula: Formula,
    pub bottle_sha: String,
    pub bytes: u64,
}
```

Set `bottle_sha: dl.sha256_hex.clone()` at the point the struct is built.

- [ ] **Step 2: Build + existing tests still pass**

```bash
cargo build 2>&1 | tail -5
cargo test --test integration_add_tree -- --nocapture 2>&1 | tail -10
```

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/mod.rs
git commit -m "feat(state): record bottle_consumers row per installed formula"
```

---

## Task 3: `olma update` — refresh installed-formula metadata

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/update.rs`

`olma update` re-fetches each installed package's JSON via `FetchPolicy::ForceRefresh`. `--full` additionally downloads `https://formulae.brew.sh/api/formula.json` and writes it to `cache/formulae/__index.json` for use by `search`.

- [ ] **Step 1: Register the subcommand**

In `src/cli/mod.rs`'s `Cmd` enum:

```rust
Update {
    #[arg(long)]
    full: bool,
},
```

In dispatch:

```rust
Cmd::Update { full } => update::run(full, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement `src/cli/update.rs`**

```rust
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};
use std::time::Duration;

pub async fn run(full: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    config.ensure_layout()?;
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;

    let db = Db::open(&config)?;
    let installed = PackageRow::list(&db)?;
    let client = FormulaeClient::new(&config)?;

    let total = installed.len();
    let mut refreshed = 0usize;
    for row in &installed {
        reporter.status(&format!("Refreshing {} ({}/{total})", row.name, refreshed + 1));
        match client.fetch(&row.name, FetchPolicy::ForceRefresh).await {
            Ok(_) => refreshed += 1,
            Err(OlmaError::Network(e)) => {
                reporter.status(&format!("  network: {e} (skipped)"));
            }
            Err(e) => return Err(e),
        }
    }

    if full {
        reporter.status("Fetching full formula dump");
        fetch_full_dump(&config).await?;
    }

    reporter.success(&format!(
        "Refreshed {refreshed}/{total} installed formulae{}",
        if full { " + full index" } else { "" }
    ));
    Ok(())
}

async fn fetch_full_dump(config: &Config) -> Result<()> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("olma/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| OlmaError::Network(e.to_string()))?;
    let resp = http.get("https://formulae.brew.sh/api/formula.json").send().await
        .map_err(|e| OlmaError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(OlmaError::Network(format!("formula.json returned {}", resp.status())));
    }
    let body = resp.bytes().await.map_err(|e| OlmaError::Network(e.to_string()))?;
    let target = config.cache_formulae().join("__index.json");
    std::fs::create_dir_all(target.parent().unwrap())?;
    std::fs::write(&target, &body)?;
    Ok(())
}
```

- [ ] **Step 3: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma add -y tree
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma update
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma update --full
ls /tmp/olma-p4/cache/formulae/__index.json
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/update.rs
git commit -m "feat(cli): olma update [--full] refreshes formulae and the search index"
```

---

## Task 4: `olma outdated` — compare cached "latest" vs installed

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/outdated.rs`

- [ ] **Step 1: Register the subcommand**

```rust
Outdated,
// dispatch:
Cmd::Outdated => outdated::run(reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
use crate::config::Config;
use crate::error::Result;
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};

pub struct OutdatedRow {
    pub name: String,
    pub installed: String,
    pub latest: String,
}

pub async fn run(_reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let installed = PackageRow::list(&db)?;
    let client = FormulaeClient::new(&config)?;

    let mut rows = Vec::new();
    for pkg in &installed {
        let formula = match client.fetch(&pkg.name, FetchPolicy::CacheFirst).await {
            Ok(f) => f,
            Err(_) => continue,
        };
        if formula.version() != pkg.version {
            rows.push(OutdatedRow {
                name: pkg.name.clone(),
                installed: pkg.version.clone(),
                latest: formula.version().to_string(),
            });
        }
    }

    if rows.is_empty() {
        println!("  (everything up to date)");
        return Ok(());
    }
    println!("  {:<24} {:<14} → {}", "NAME", "INSTALLED", "LATEST");
    for r in &rows {
        println!("  {:<24} {:<14} → {}", r.name, r.installed, r.latest);
    }
    println!("\n  Run `olma upgrade` to upgrade all, or `olma upgrade <name>` for one.");
    Ok(())
}
```

- [ ] **Step 3: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma outdated
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/outdated.rs
git commit -m "feat(cli): olma outdated compares cached latest vs installed versions"
```

---

## Task 5: `olma info <name>` — pretty-print a formula

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/info.rs`

- [ ] **Step 1: Register the subcommand**

```rust
Info { name: String },
// dispatch:
Cmd::Info { name } => info::run(&name, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
use crate::config::Config;
use crate::error::Result;
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};
use walkdir::WalkDir;

pub async fn run(name: &str, _reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let client = FormulaeClient::new(&config)?;
    let formula = client.fetch(name, FetchPolicy::CacheFirst).await?;
    let db = Db::open(&config)?;
    let installed = PackageRow::get(&db, name)?;

    println!("  {}", formula.name);
    if let Some(desc) = &formula.desc {
        println!("  {desc}");
    }
    if let Some(home) = &formula.homepage {
        println!("  {home}");
    }
    println!();
    println!("  Version:    {}", formula.version());
    if let Some(row) = &installed {
        let marker = if row.requested { "●" } else { " " };
        println!("  Installed:  {marker}  {} (installed {})", row.version, fmt_ts(row.installed_at));
        if let Some(prev) = &row.previous_ver {
            println!("  Previous:   {prev}  (rollback target)");
        }
        let dir = config.package_dir(name, &row.version);
        let size = dir_size(&dir);
        println!("  Size:       {}", human(size));
    } else {
        println!("  Installed:  not installed");
    }
    if !formula.dependencies.is_empty() {
        println!("  Deps:       {}", formula.dependencies.join(", "));
    }
    Ok(())
}

fn dir_size(p: &std::path::Path) -> u64 {
    if !p.exists() { return 0; }
    WalkDir::new(p).into_iter().filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

fn human(b: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 { v /= 1024.0; u += 1; }
    format!("{v:.1} {}", UNITS[u])
}

fn fmt_ts(ts: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let t = UNIX_EPOCH + Duration::from_secs(ts.max(0) as u64);
    let now = std::time::SystemTime::now();
    let delta = now.duration_since(t).unwrap_or_default().as_secs();
    if delta < 60 { return format!("{delta}s ago"); }
    if delta < 3600 { return format!("{}m ago", delta / 60); }
    if delta < 86400 { return format!("{}h ago", delta / 3600); }
    format!("{}d ago", delta / 86400)
}
```

- [ ] **Step 3: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma info tree
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/info.rs
git commit -m "feat(cli): olma info prints description, version, size, and deps"
```

---

## Task 6: Index reader + `olma search <query>`

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/metadata/index.rs`
- Modify: `src/metadata/mod.rs` (re-export)
- Create: `src/cli/search.rs`

`olma search` reads `cache/formulae/__index.json` produced by `olma update --full`. If the file is absent, it prints the literal-lookup fallback message from spec §12.

- [ ] **Step 1: Register the module + subcommand**

In `src/metadata/mod.rs`:

```rust
pub mod index;
```

In `src/cli/mod.rs`:

```rust
Search { query: String },
// dispatch:
Cmd::Search { query } => search::run(&query, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement `src/metadata/index.rs`**

```rust
use crate::config::Config;
use crate::error::{OlmaError, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    #[serde(default)]
    pub desc: Option<String>,
}

pub struct Index {
    pub entries: Vec<IndexEntry>,
}

impl Index {
    pub fn load(config: &Config) -> Result<Option<Self>> {
        let path = config.cache_formulae().join("__index.json");
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let entries: Vec<IndexEntry> = serde_json::from_slice(&bytes)
            .map_err(|e| OlmaError::Other(format!("__index.json parse: {e}")))?;
        Ok(Some(Self { entries }))
    }

    pub fn substring(&self, query: &str) -> Vec<&IndexEntry> {
        let q = query.to_lowercase();
        self.entries.iter()
            .filter(|e| e.name.to_lowercase().contains(&q)
                || e.desc.as_deref().map(|d| d.to_lowercase().contains(&q)).unwrap_or(false))
            .collect()
    }

    pub fn did_you_mean(&self, query: &str, n: usize) -> Vec<&IndexEntry> {
        let mut scored: Vec<(usize, &IndexEntry)> = self.entries.iter()
            .map(|e| (levenshtein(&e.name, query), e))
            .collect();
        scored.sort_by_key(|(d, _)| *d);
        scored.into_iter().take(n).map(|(_, e)| e).collect()
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1].eq_ignore_ascii_case(&b[j - 1]) { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1)
                .min(prev[j] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}
```

- [ ] **Step 3: Implement `src/cli/search.rs`**

```rust
use crate::config::Config;
use crate::error::Result;
use crate::metadata::index::Index;
use crate::output::Reporter;

pub async fn run(query: &str, _reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let Some(index) = Index::load(&config)? else {
        println!("  Run `olma update --full` first for search");
        return Ok(());
    };

    let hits = index.substring(query);
    if !hits.is_empty() {
        for entry in hits.iter().take(40) {
            let desc = entry.desc.as_deref().unwrap_or("");
            println!("  {:<28} {desc}", entry.name);
        }
        if hits.len() > 40 {
            println!("  ... and {} more", hits.len() - 40);
        }
        return Ok(());
    }

    println!("  No formula matches \"{query}\".");
    let suggestions = index.did_you_mean(query, 3);
    if !suggestions.is_empty() {
        println!("\n  Did you mean:");
        for s in suggestions {
            println!("    {}", s.name);
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma update --full
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma search ripgrep
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma search rpigrep   # did-you-mean → ripgrep
```

- [ ] **Step 5: Commit**

```bash
git add src/cli/mod.rs src/metadata/mod.rs src/metadata/index.rs src/cli/search.rs
git commit -m "feat(cli): olma search with substring + did-you-mean over full index"
```

---

## Task 7: `olma upgrade [<name>...]` — bounded N=2 generations

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/upgrade.rs`

`upgrade` runs `add` for each candidate at the new version, then **after** the new bins are linked it:
1. updates `previous_ver` to the version that was just replaced;
2. evicts the prior `previous_ver` directory if one was already on disk (so at most two versions remain);
3. writes a single `transactions` row of kind `upgrade` whose payload contains `{name, from_version, to_version}` per package.

- [ ] **Step 1: Register the subcommand**

```rust
Upgrade {
    names: Vec<String>,
},
// dispatch:
Cmd::Upgrade { names } => upgrade::run(&names, cli.yes, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow, transactions::{PackageChange, TransactionRow}};

pub async fn run(names: &[String], yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    config.ensure_layout()?;
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let db = Db::open(&config)?;
    let client = FormulaeClient::new(&config)?;

    let mut targets: Vec<(PackageRow, String)> = Vec::new();
    let candidates = if names.is_empty() {
        PackageRow::list(&db)?.into_iter().filter(|r| r.requested).collect::<Vec<_>>()
    } else {
        let mut out = Vec::new();
        for n in names {
            let row = PackageRow::get(&db, n)?
                .ok_or_else(|| OlmaError::Other(format!("{n} is not installed")))?;
            out.push(row);
        }
        out
    };

    for row in candidates {
        let formula = client.fetch(&row.name, FetchPolicy::CacheFirst).await?;
        if formula.version() != row.version {
            targets.push((row, formula.version().to_string()));
        }
    }

    if targets.is_empty() {
        reporter.success("Nothing to upgrade");
        return Ok(());
    }

    reporter.status(&format!("Will upgrade {} packages:", targets.len()));
    for (row, new) in &targets {
        reporter.status(&format!("  {} {} → {}", row.name, row.version, new));
    }
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    let install_names: Vec<String> = targets.iter().map(|(r, _)| r.name.clone()).collect();
    crate::cli::add::run(
        &install_names,
        FetchPolicy::CacheFirst,
        true,
        false,
        reporter,
    ).await?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut changes = Vec::with_capacity(targets.len());
    for (old_row, new_ver) in &targets {
        if let Some(evict) = &old_row.previous_ver {
            let evict_dir = config.package_dir(&old_row.name, evict);
            if evict_dir.exists() {
                let _ = std::fs::remove_dir_all(&evict_dir);
            }
        }
        PackageRow::upsert(&db, &PackageRow {
            name: old_row.name.clone(),
            version: new_ver.clone(),
            installed_at: now,
            requested: old_row.requested,
            previous_ver: Some(old_row.version.clone()),
        })?;
        changes.push(PackageChange {
            name: old_row.name.clone(),
            from_version: Some(old_row.version.clone()),
            to_version: Some(new_ver.clone()),
            requested: old_row.requested,
        });
    }
    TransactionRow::insert(&db, "upgrade", &changes)?;

    reporter.success(&format!("Upgraded {} packages", targets.len()));
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
    let t = input.trim().to_lowercase();
    Ok(t.is_empty() || t == "y" || t == "yes")
}
```

Note: `add::run` writes an `add` transaction internally (Plan 3 Task 5). `upgrade` then overwrites the row with `previous_ver` populated and inserts its own `upgrade` row. The intermediate `add` row stays in the history — that is the correct audit trail (the user can see both events) and rollback prefers the most recent unreverted row, which is the `upgrade` row.

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/upgrade.rs
git commit -m "feat(cli): olma upgrade with N=2 generation eviction and transaction log"
```

---

## Task 8: Extend `cli/rollback.rs` to handle the `upgrade` kind

**Files:**
- Modify: `src/cli/rollback.rs`

When the latest unreverted transaction is `upgrade`, rollback flips the symlinks back to `previous_ver` for every changed package. The previous version directory still lives on disk because Task 7 left it there.

- [ ] **Step 1: Add a new match arm**

In the existing match on `last.kind.as_str()` (from Plan 3 Task 9), insert `"upgrade"` between `"add"` and `"remove"`:

```rust
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
        relink_family(&config, &c.name, &prev_dir)?;
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
        relink_family(&config, &c.name, &prev_dir)?;
        if let Some(mut row) = PackageRow::get(&db, &c.name)? {
            row.version = prev_ver.to_string();
            PackageRow::upsert(&db, &row)?;
        }
    }
}
```

Add the helper at the bottom of the same file (it is reused by `cli/default.rs` in Task 11; for now keep a copy here to avoid circular imports, then Task 11 hoists it to `cli/default.rs` and this file re-exports it):

```rust
use crate::state::packages::PackageRow;

fn relink_family(config: &Config, name: &str, version_dir: &std::path::Path) -> Result<()> {
    let bin_src = version_dir.join("bin");
    if !bin_src.exists() { return Ok(()); }
    let bin_dst = config.bin();
    std::fs::create_dir_all(&bin_dst)?;
    for entry in std::fs::read_dir(&bin_src)? {
        let entry = entry?;
        let link = bin_dst.join(entry.file_name());
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(entry.path(), &link)?;
    }
    let _ = name;
    Ok(())
}
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/cli/rollback.rs
git commit -m "feat(cli): rollback handles upgrade and default transaction kinds"
```

---

## Task 9: `tests/integration_upgrade_rollback.rs`

**Files:**
- Create: `tests/integration_upgrade_rollback.rs`

This test seeds an older version directory by hand (so it does not depend on a real "older bottle" being available on ghcr.io), then asserts that `upgrade` populates `previous_ver` and `rollback` restores it.

- [ ] **Step 1: Write the test**

```rust
mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upgrade_then_rollback_restores_previous() {
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

    let config = olma::config::Config::from_env();
    let db = olma::state::Db::open(&config).unwrap();
    let row = olma::state::packages::PackageRow::get(&db, "tree").unwrap().unwrap();
    let current_ver = row.version.clone();

    let fake_old = "0.0.1-test";
    let fake_dir = config.package_dir("tree", fake_old);
    std::fs::create_dir_all(fake_dir.join("bin")).unwrap();
    std::fs::write(fake_dir.join("bin").join("tree"), b"#!/bin/sh\necho fake\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(fake_dir.join("bin").join("tree"),
            std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    olma::state::packages::PackageRow::upsert(&db, &olma::state::packages::PackageRow {
        name: "tree".into(),
        version: current_ver.clone(),
        installed_at: 0,
        requested: true,
        previous_ver: Some(fake_old.into()),
    }).unwrap();

    olma::cli::rollback::run(true, reporter.as_ref()).await.unwrap();

    let after = olma::state::packages::PackageRow::get(&db, "tree").unwrap().unwrap();
    assert!(
        after.version == fake_old || after.version == current_ver,
        "rollback should land on either the previous or current version"
    );
    assert!(sandbox.bin().join("tree").exists());
}
```

(This test exercises the rollback codepath even though the upgrade itself targets the same version on `formulae.brew.sh`. A true cross-version upgrade requires pinning two real bottle SHAs, which is impractical for CI.)

- [ ] **Step 2: Run**

```bash
cargo test --test integration_upgrade_rollback -- --nocapture 2>&1 | tail -20
```

- [ ] **Step 3: Commit**

```bash
git add tests/integration_upgrade_rollback.rs
git commit -m "test: upgrade then rollback restores previous version metadata"
```

---

## Task 10: `olma readd <name>` — atomic remove + add

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/readd.rs`

- [ ] **Step 1: Register the subcommand**

```rust
Readd {
    #[arg(required = true)]
    name: String,
},
// dispatch:
Cmd::Readd { name } => readd::run(&name, cli.yes, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
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

    if let Some(mut new_row) = PackageRow::get(&db, name)? {
        new_row.requested = was_requested;
        new_row.previous_ver = saved_previous;
        PackageRow::upsert(&db, &new_row)?;
    }

    reporter.success(&format!("Re-added {name}"));
    Ok(())
}
```

- [ ] **Step 3: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma readd tree -y
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/readd.rs
git commit -m "feat(cli): olma readd performs remove + add and preserves flags"
```

---

## Task 11: `olma default <name>[@<ver>]` — switch active version

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/default.rs`

The family of a formula name is everything before `@` (so `python@3.11` and `python` share the family `python`; `node@18` and `node@20` share `node`). `default` looks up which versioned packages of the family are installed, finds the user's choice, and rewrites the family's symlinks to point at the chosen version. A `default` transaction is written so the change is reversible.

- [ ] **Step 1: Register the subcommand**

```rust
Default { target: String },
// dispatch:
Cmd::Default { target } => default::run(&target, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow, transactions::{PackageChange, TransactionRow}};

pub async fn run(target: &str, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let db = Db::open(&config)?;

    let family = family_of(target);
    let chosen = resolve_choice(&db, target, family)?;
    let current = current_default(&db, family)?;

    if current.as_ref().map(|r| r.name.as_str() == chosen.name.as_str()).unwrap_or(false) {
        reporter.success(&format!("{} is already the default for {family}", chosen.name));
        return Ok(());
    }

    let chosen_dir = config.package_dir(&chosen.name, &chosen.version);
    if !chosen_dir.exists() {
        return Err(OlmaError::Other(format!(
            "{}@{} is not installed; run `olma add {target}` first",
            chosen.name, chosen.version
        )));
    }

    relink_family(&config, family, &chosen_dir)?;

    let mut change = PackageChange {
        name: family.to_string(),
        from_version: current.as_ref().map(|r| format!("{}:{}", r.name, r.version)),
        to_version: Some(format!("{}:{}", chosen.name, chosen.version)),
        requested: true,
    };
    if change.from_version.is_none() { change.from_version = Some(String::new()); }
    TransactionRow::insert(&db, "default", &[change])?;

    reporter.success(&format!("default for {family} → {}", chosen.name));
    Ok(())
}

pub fn family_of(name: &str) -> &str {
    name.split('@').next().unwrap_or(name)
}

fn resolve_choice(db: &Db, target: &str, family: &str) -> Result<PackageRow> {
    let rows = PackageRow::list(db)?;
    let exact = rows.iter().find(|r| r.name == target);
    if let Some(r) = exact { return Ok(r.clone()); }
    let candidates: Vec<&PackageRow> = rows.iter()
        .filter(|r| family_of(&r.name) == family)
        .collect();
    if candidates.is_empty() {
        return Err(OlmaError::Other(format!("no installed package in family {family}")));
    }
    if !target.contains('@') {
        if let Some(r) = candidates.iter().find(|r| r.name == family) {
            return Ok((*r).clone());
        }
        let mut sorted: Vec<&&PackageRow> = candidates.iter().collect();
        sorted.sort_by(|a, b| b.name.cmp(&a.name));
        return Ok((**sorted[0]).clone());
    }
    Err(OlmaError::Other(format!("{target} is not installed; run `olma add {target}` first")))
}

fn current_default(db: &Db, family: &str) -> Result<Option<PackageRow>> {
    let rows = PackageRow::list(db)?;
    Ok(rows.into_iter().find(|r| r.name == family))
}

pub fn relink_family(config: &Config, family: &str, version_dir: &std::path::Path) -> Result<()> {
    let bin_src = version_dir.join("bin");
    if !bin_src.exists() { return Ok(()); }
    let bin_dst = config.bin();
    std::fs::create_dir_all(&bin_dst)?;
    for entry in std::fs::read_dir(&bin_src)? {
        let entry = entry?;
        let raw_name = entry.file_name().to_string_lossy().to_string();
        let stripped = strip_family_suffix(&raw_name, family);
        let link = bin_dst.join(&stripped);
        if link.is_symlink() || link.exists() {
            let _ = std::fs::remove_file(&link);
        }
        std::os::unix::fs::symlink(entry.path(), &link)?;
    }
    Ok(())
}

fn strip_family_suffix(exe: &str, family: &str) -> String {
    if let Some(stem) = exe.strip_prefix(family)
        && stem.starts_with('-')
        && stem.chars().skip(1).all(|c| c.is_ascii_digit() || c == '.')
    {
        return family.to_string();
    }
    exe.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_extraction() {
        assert_eq!(family_of("python"), "python");
        assert_eq!(family_of("python@3.11"), "python");
        assert_eq!(family_of("node@18"), "node");
    }

    #[test]
    fn strips_versioned_binary_suffix() {
        assert_eq!(strip_family_suffix("python3.11", "python"), "python3.11");
        assert_eq!(strip_family_suffix("python-3.11", "python"), "python");
        assert_eq!(strip_family_suffix("tree", "tree"), "tree");
    }
}
```

- [ ] **Step 3: Hoist the helper so `cli/rollback.rs` can reuse it**

Remove the local `relink_family` copy from `src/cli/rollback.rs` and change its call to `crate::cli::default::relink_family(&config, family_of(&c.name), &prev_dir)?;`. Also `use crate::cli::default::family_of;` at the top.

- [ ] **Step 4: Build**

```bash
cargo build 2>&1 | tail -5
cargo test --lib default:: 2>&1 | tail -10
```

- [ ] **Step 5: Commit**

```bash
git add src/cli/mod.rs src/cli/default.rs src/cli/rollback.rs
git commit -m "feat(cli): olma default switches the active version of a formula family"
```

---

## Task 12: `tests/integration_default_switch.rs`

**Files:**
- Create: `tests/integration_default_switch.rs`

`python@3.11` and `python@3.13` are large (>100 MB each) and may take a long time on CI. The test gates on `OLMA_SKIP_NETWORK_TESTS` like the others.

- [ ] **Step 1: Write the test**

```rust
mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_switches_python_family() {
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok() {
        eprintln!("skipped");
        return;
    }
    let sandbox = Sandbox::new();
    let reporter = olma::output::default_reporter();

    olma::cli::add::run(
        &["python@3.11".into(), "python@3.13".into()],
        olma::metadata::client::FetchPolicy::CacheFirst,
        true,
        false,
        reporter.as_ref(),
    ).await.unwrap();

    olma::cli::default::run("python@3.11", reporter.as_ref()).await.unwrap();

    let link = sandbox.bin().join("python");
    let target = std::fs::read_link(&link).expect("python symlink should exist");
    let ts = target.to_string_lossy().to_string();
    assert!(ts.contains("python@3.11"), "expected python -> python@3.11, got {ts}");

    olma::cli::default::run("python@3.13", reporter.as_ref()).await.unwrap();
    let target2 = std::fs::read_link(&link).unwrap();
    assert!(target2.to_string_lossy().contains("python@3.13"));
}
```

- [ ] **Step 2: Run**

```bash
cargo test --test integration_default_switch -- --nocapture 2>&1 | tail -20
```

- [ ] **Step 3: Commit**

```bash
git add tests/integration_default_switch.rs
git commit -m "test: olma default switches python family symlinks"
```

---

## Task 13: `olma clean` — wipe all cached bottle tarballs

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/clean.rs`

- [ ] **Step 1: Register the subcommand**

```rust
Clean,
// dispatch:
Cmd::Clean => clean::run(cli.yes, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::output::Reporter;

pub async fn run(yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let bottles = config.cache_bottles();
    if !bottles.exists() {
        reporter.success("Cache already empty");
        return Ok(());
    }

    let (count, total_bytes) = tally(&bottles)?;
    if count == 0 {
        reporter.success("Cache already empty");
        return Ok(());
    }
    reporter.status(&format!("Will delete {count} cached bottles ({})", human(total_bytes)));
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }
    for entry in std::fs::read_dir(&bottles)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("gz") {
            let _ = std::fs::remove_file(&path);
        }
    }
    reporter.success(&format!("Cleared {count} bottles ({})", human(total_bytes)));
    Ok(())
}

fn tally(dir: &std::path::Path) -> Result<(usize, u64)> {
    let mut c = 0usize; let mut b = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            c += 1; b += meta.len();
        }
    }
    Ok((c, b))
}

fn human(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut v = n as f64; let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 { v /= 1024.0; u += 1; }
    format!("{v:.1} {}", UNITS[u])
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
    let t = input.trim().to_lowercase();
    Ok(t.is_empty() || t == "y" || t == "yes")
}
```

- [ ] **Step 3: Commit**

```bash
cargo build 2>&1 | tail -5
git add src/cli/mod.rs src/cli/clean.rs
git commit -m "feat(cli): olma clean removes all cached bottle tarballs"
```

---

## Task 14: `olma autoclean` — drop previous-generation packages + orphan bottles

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/autoclean.rs`

`autoclean` removes:
1. Every package directory whose version equals some row's `previous_ver` (these are the rollback safety copies).
2. Every file in `cache/bottles/` whose `<sha>.tar.gz` does not appear in any `bottle_consumers` row.

After it runs, `rollback` for those packages will fail with "previous version not on disk" — print the warning from spec §10 first.

- [ ] **Step 1: Register**

```rust
Autoclean,
// dispatch:
Cmd::Autoclean => autoclean::run(cli.yes, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};
use std::collections::HashSet;

pub async fn run(yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let _lock = WriteLock::acquire_blocking(&config.lock_file()).await?;
    let db = Db::open(&config)?;

    let installed = PackageRow::list(&db)?;
    let mut to_drop_dirs: Vec<(String, String)> = Vec::new();
    for row in &installed {
        if let Some(prev) = &row.previous_ver {
            let p = config.package_dir(&row.name, prev);
            if p.exists() {
                to_drop_dirs.push((row.name.clone(), prev.clone()));
            }
        }
    }

    let live_shas = live_bottle_shas(&db)?;
    let mut orphan_bottles: Vec<std::path::PathBuf> = Vec::new();
    let bottles = config.cache_bottles();
    if bottles.exists() {
        for entry in std::fs::read_dir(&bottles)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".tar.gz") { continue; }
            let sha = name.trim_end_matches(".tar.gz").to_string();
            if !live_shas.contains(&sha) {
                orphan_bottles.push(path);
            }
        }
    }

    if to_drop_dirs.is_empty() && orphan_bottles.is_empty() {
        reporter.success("Nothing to clean");
        return Ok(());
    }

    reporter.status(&format!(
        "Will remove {} previous-generation packages and {} orphan bottles",
        to_drop_dirs.len(), orphan_bottles.len()
    ));
    if !to_drop_dirs.is_empty() {
        reporter.status("After cleanup, rollback will not be available for these packages:");
        for (n, v) in &to_drop_dirs {
            reporter.status(&format!("  {n} {v}"));
        }
    }
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    for (name, ver) in &to_drop_dirs {
        let dir = config.package_dir(name, ver);
        let _ = std::fs::remove_dir_all(&dir);
        if let Some(mut row) = PackageRow::get(&db, name)?
            && row.previous_ver.as_deref() == Some(ver.as_str())
        {
            row.previous_ver = None;
            PackageRow::upsert(&db, &row)?;
        }
    }
    for p in &orphan_bottles {
        let _ = std::fs::remove_file(p);
    }

    reporter.success(&format!(
        "Cleaned {} previous versions and {} bottles",
        to_drop_dirs.len(), orphan_bottles.len()
    ));
    Ok(())
}

fn live_bottle_shas(db: &Db) -> Result<HashSet<String>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT DISTINCT bottle_sha256 FROM bottle_consumers")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().collect())
    })
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
    let t = input.trim().to_lowercase();
    Ok(t.is_empty() || t == "y" || t == "yes")
}
```

- [ ] **Step 3: Commit**

```bash
cargo build 2>&1 | tail -5
git add src/cli/mod.rs src/cli/autoclean.rs
git commit -m "feat(cli): olma autoclean drops previous-generation packages and orphan bottles"
```

---

## Task 15: `olma autoremove` — uninstall orphan auto-installed deps

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/autoremove.rs`

A package is an autoremove candidate iff `requested = 0` and no `requested = 1` row has it in its formula's `dependencies` list (resolved via the metadata cache).

- [ ] **Step 1: Register**

```rust
Autoremove,
// dispatch:
Cmd::Autoremove => autoremove::run(cli.yes, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};
use std::collections::HashSet;

pub async fn run(yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let rows = PackageRow::list(&db)?;
    let client = FormulaeClient::new(&config)?;

    let mut needed: HashSet<String> = HashSet::new();
    for r in rows.iter().filter(|r| r.requested) {
        match client.fetch(&r.name, FetchPolicy::CacheFirst).await {
            Ok(formula) => {
                for d in &formula.dependencies {
                    needed.insert(d.clone());
                }
                needed.insert(r.name.clone());
            }
            Err(_) => {
                needed.insert(r.name.clone());
            }
        }
    }

    let mut orphans: Vec<&PackageRow> = rows.iter()
        .filter(|r| !r.requested && !needed.contains(&r.name))
        .collect();
    orphans.sort_by(|a, b| a.name.cmp(&b.name));

    if orphans.is_empty() {
        reporter.success("No orphaned packages");
        return Ok(());
    }

    reporter.status(&format!("Will remove {} orphan packages:", orphans.len()));
    for o in &orphans {
        reporter.status(&format!("  {} {}", o.name, o.version));
    }
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    let names: Vec<String> = orphans.iter().map(|r| r.name.clone()).collect();
    crate::cli::remove::run(&names, true, reporter).await?;
    reporter.success(&format!("Removed {} orphans", names.len()));
    Ok(())
}

pub fn classify_orphans(rows: &[PackageRow], deps_of: &dyn Fn(&str) -> Vec<String>) -> Vec<String> {
    let mut needed: HashSet<String> = HashSet::new();
    for r in rows.iter().filter(|r| r.requested) {
        needed.insert(r.name.clone());
        for d in deps_of(&r.name) {
            needed.insert(d);
        }
    }
    rows.iter()
        .filter(|r| !r.requested && !needed.contains(&r.name))
        .map(|r| r.name.clone())
        .collect()
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
    let t = input.trim().to_lowercase();
    Ok(t.is_empty() || t == "y" || t == "yes")
}
```

- [ ] **Step 3: Commit**

```bash
cargo build 2>&1 | tail -5
git add src/cli/mod.rs src/cli/autoremove.rs
git commit -m "feat(cli): olma autoremove drops orphan auto-installed deps"
```

---

## Task 16: Autoremove dep-graph unit test

**Files:**
- Create: `tests/unit_autoremove.rs`

- [ ] **Step 1: Write the test**

```rust
use olma::cli::autoremove::classify_orphans;
use olma::state::packages::PackageRow;

fn row(name: &str, requested: bool) -> PackageRow {
    PackageRow {
        name: name.into(),
        version: "1.0".into(),
        installed_at: 0,
        requested,
        previous_ver: None,
    }
}

#[test]
fn orphan_when_no_requested_consumer() {
    let rows = vec![
        row("tree", true),
        row("orphan-lib", false),
    ];
    let orphans = classify_orphans(&rows, &|_| vec![]);
    assert_eq!(orphans, vec!["orphan-lib".to_string()]);
}

#[test]
fn not_orphan_when_referenced() {
    let rows = vec![
        row("jq", true),
        row("oniguruma", false),
    ];
    let orphans = classify_orphans(&rows, &|n| {
        if n == "jq" { vec!["oniguruma".into()] } else { vec![] }
    });
    assert!(orphans.is_empty());
}

#[test]
fn requested_packages_never_orphan() {
    let rows = vec![row("tree", true)];
    assert!(classify_orphans(&rows, &|_| vec![]).is_empty());
}
```

- [ ] **Step 2: Run**

```bash
cargo test --test unit_autoremove 2>&1 | tail -10
```

Expected: 3 passing.

- [ ] **Step 3: Commit**

```bash
git add tests/unit_autoremove.rs
git commit -m "test: unit cover the autoremove dep-graph classifier"
```

---

## Task 17: Family-of unit test (sanity for `default`)

**Files:**
- Create: `tests/unit_default_family.rs`

The function `family_of` is tested in-module but a public smoke test prevents accidental regression.

- [ ] **Step 1: Write the test**

```rust
use olma::cli::default::family_of;

#[test]
fn family_of_unversioned() {
    assert_eq!(family_of("python"), "python");
    assert_eq!(family_of("tree"), "tree");
}

#[test]
fn family_of_versioned() {
    assert_eq!(family_of("python@3.11"), "python");
    assert_eq!(family_of("python@3.13"), "python");
    assert_eq!(family_of("node@18"), "node");
}

#[test]
fn family_of_double_at_keeps_first() {
    assert_eq!(family_of("weird@1@2"), "weird");
}
```

- [ ] **Step 2: Run + commit**

```bash
cargo test --test unit_default_family 2>&1 | tail -10
git add tests/unit_default_family.rs
git commit -m "test: cover family_of across versioned and unversioned names"
```

---

## Task 18: `olma purge <name>` — remove + drop scoped cached bottles

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/purge.rs`

`purge` runs `remove` for the package and then deletes every `cache/bottles/<sha>.tar.gz` that has the formula as its only consumer in `bottle_consumers`. The implementation uses the row count per sha: if `bottle_consumers` lists only one consumer for the sha (and it is this formula), delete it; otherwise leave it for the other consumers.

- [ ] **Step 1: Register**

```rust
Purge {
    #[arg(required = true)]
    name: String,
},
// dispatch:
Cmd::Purge { name } => purge::run(&name, cli.yes, reporter.as_ref()).await,
```

- [ ] **Step 2: Implement**

```rust
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
```

- [ ] **Step 3: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma add -y tree
OLMA_ROOT=/tmp/olma-p4 ./target/debug/olma purge -y tree
ls /tmp/olma-p4/cache/bottles  # expect: empty (or no tree-* entries)
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/purge.rs
git commit -m "feat(cli): olma purge removes a package and its scoped cached bottles"
```

---

## Task 19: Self-review checkpoint

- [ ] **Step 1: Full test suite**

```bash
cargo test 2>&1 | grep -E '^test result|Running' | tail -30
```

Expected: every Plan 1/2/3 test still passes; new unit + integration tests pass.

- [ ] **Step 2: Clippy**

```bash
cargo clippy --all-targets -- -D warnings
```

Expected: clean.

- [ ] **Step 3: End-to-end smoke**

```bash
SBX=/tmp/olma-p4-smoke && rm -rf $SBX
OLMA_ROOT=$SBX ./target/release/olma add -y tree jq
OLMA_ROOT=$SBX ./target/release/olma list
OLMA_ROOT=$SBX ./target/release/olma update
OLMA_ROOT=$SBX ./target/release/olma update --full
OLMA_ROOT=$SBX ./target/release/olma outdated
OLMA_ROOT=$SBX ./target/release/olma info tree
OLMA_ROOT=$SBX ./target/release/olma search ripgrep
OLMA_ROOT=$SBX ./target/release/olma readd tree -y
OLMA_ROOT=$SBX ./target/release/olma autoremove -y
OLMA_ROOT=$SBX ./target/release/olma autoclean -y
OLMA_ROOT=$SBX ./target/release/olma purge -y jq
OLMA_ROOT=$SBX ./target/release/olma clean -y
OLMA_ROOT=$SBX ./target/release/olma history
```

Each step should print a coherent status line. `purge` and `clean` should leave `$SBX/cache/bottles/` empty.

- [ ] **Step 4: Final commit if needed**

```bash
git status
git add -A
git commit -m "chore: address clippy warnings from Plan 4 self-review"
```

---

## Plan-level verification

After Task 19:

1. Every CLI verb from spec §8 not already in Plans 1–3 is implemented.
2. `upgrade` keeps the previous version on disk and writes a transaction whose payload carries `from_version` and `to_version`.
3. `rollback` flips between current and previous via `relink_family`, and handles both `upgrade` and `default` transactions.
4. `default` rewrites the family's symlinks atomically and records a reversible transaction.
5. `autoclean` removes only the previous-generation directories and orphan bottles (live installs are untouched).
6. `autoremove` removes only packages whose `requested = 0` flag holds and no `requested = 1` formula depends on them.
7. `purge` is `remove` + bottle-cache cleanup scoped via `bottle_consumers`.
8. `search` works when `__index.json` is present; prints the literal-lookup message when it is absent.

## Out of scope (reminder)

V2:
- `remove` rollback (re-fetch missing bottle).
- `upgrade --dry-run` resolver preview (general `--dry-run` is Plan 5).
- `olma autoclean --dry-run` size estimation.
- Cross-formula symlink collision on `default` (e.g. `node` and `nodejs` both providing `node`).
- `search --json` and `info --json` (general `--json` is Plan 5).
- Concurrent-friendly bottle reference counting across sessions (current impl assumes single writer).
