# Plan 6 — Services Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the full `olma services` subsystem — parse the `service` block from each formula's JSON, generate macOS launchd `.plist` files into `~/Library/LaunchAgents/`, drive `launchctl` via its modern bootstrap/bootout/kickstart API, and integrate the service lifecycle with `add`/`remove`/`upgrade`/`readd`.

**Architecture:** A new `services` module owns parsing, plist generation, launchctl dispatch, and persistence. A new `cli/services/` subtree owns the verbs (`list`, `start`, `stop`, `restart`, `status`, `enable`, `disable`, `logs`, `reload`). The `services` table in `state.db` tracks enable state and the timestamp of the last action; the live process state is always queried from `launchctl` rather than mirrored. Logs land under `/opt/olma/log/<name>/{out,err}.log`.

**Tech Stack:** Rust 2024 · `quick-xml` for plist generation · `tokio::process::Command` for shelling out to `launchctl` and `id` · `rusqlite` for the `services` table · existing modules (`config`, `error`, `output`, `state`, `metadata`, `relocator/text` for path swaps).

**Depends on:** Plan 1 (foundation, metadata client, add pipeline), Plan 3 (state.db infrastructure, transaction log), Plan 4 (`remove`, `upgrade`, `readd` so the lifecycle hooks have somewhere to land).

Spec reference: `docs/superpowers/specs/2026-05-31-olma-architecture-design.md` §11.

---

## File structure (created in this plan)

```
src/
├── services/
│   ├── mod.rs           Service struct + parsing from Formula JSON
│   ├── plist.rs         Plist XML rendering
│   ├── launchctl.rs     launchctl wrapper (bootstrap/bootout/kickstart/print/disable)
│   └── store.rs         services table CRUD over state.db
├── cli/
│   └── services/
│       ├── mod.rs       clap subcommand router
│       ├── list.rs
│       ├── start.rs
│       ├── stop.rs
│       ├── restart.rs
│       ├── status.rs
│       ├── enable.rs
│       ├── disable.rs
│       ├── logs.rs
│       └── reload.rs

tests/
├── unit_plist.rs            plist rendering against known fixtures
├── unit_launchctl_parse.rs  parse `launchctl print` output
└── integration_services_mosquitto.rs   end-to-end against a real service
```

Touched files:
- `Cargo.toml` — add `quick-xml`
- `src/metadata/mod.rs` — extend `Formula` with `service: Option<Service>`
- `src/cli/mod.rs` — register the `services` subcommand
- `src/state/schema.rs` — add `services` table migration
- `src/cli/add.rs`, `src/cli/remove.rs`, `src/cli/upgrade.rs`, `src/cli/readd.rs` — lifecycle hooks

---

## Task 1: Extend `Formula` with the `service` field

**Files:**
- Modify: `src/metadata/mod.rs`

The formula JSON ships an optional `service` object. Parse it into a struct now so every downstream task can read it.

- [ ] **Step 1: Add the `Service` struct alongside `Formula`**

```rust
#[derive(Debug, Deserialize, Clone)]
pub struct Service {
    #[serde(default)]
    pub run: Vec<String>,
    pub keep_alive: Option<KeepAlive>,
    pub working_dir: Option<String>,
    pub log_path: Option<String>,
    pub error_log_path: Option<String>,
    #[serde(default)]
    pub environment_variables: std::collections::HashMap<String, String>,
    pub process_type: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum KeepAlive {
    Bool(bool),
    Obj(std::collections::HashMap<String, serde_json::Value>),
}
```

Then add to `Formula`:

```rust
pub service: Option<Service>,
```

- [ ] **Step 2: Verify the lib still builds**

Run: `cargo check --lib 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/metadata/mod.rs
git commit -m "feat(services): parse the service block from formula JSON"
```

---

## Task 2: `services` table migration

**Files:**
- Modify: `src/state/schema.rs`

Add a tiny table; `state.db` infrastructure must already exist from Plan 3.

- [ ] **Step 1: Append migration**

```rust
pub const MIGRATIONS: &[&str] = &[
    /* ... earlier migrations ... */
    r#"
    CREATE TABLE services (
        name           TEXT PRIMARY KEY,
        plist_path     TEXT NOT NULL,
        enabled        INTEGER NOT NULL DEFAULT 0,
        last_action    TEXT,
        last_action_ts INTEGER
    );
    "#,
];
```

- [ ] **Step 2: Build + run migration tests already present**

Run: `cargo test --lib state::`
Expected: pre-existing migration tests still pass; new migration is applied.

- [ ] **Step 3: Commit**

```bash
git add src/state/schema.rs
git commit -m "feat(services): add services table to state.db schema"
```

---

## Task 3: Plist generation (TDD)

**Files:**
- Create: `src/services/mod.rs`
- Create: `src/services/plist.rs`
- Create: `tests/unit_plist.rs`
- Modify: `Cargo.toml` (add `quick-xml`)

- [ ] **Step 1: Add `quick-xml` to Cargo.toml**

Place alphabetically in `[dependencies]`:

```toml
quick-xml = { version = "0.36", features = ["serialize"] }
```

- [ ] **Step 2: Create the `services` module root**

`src/services/mod.rs`:

```rust
pub mod launchctl;
pub mod plist;
pub mod store;

use crate::metadata::Service;
use crate::relocator::text::replace_prefix_in_bytes;

pub fn label_for(name: &str) -> String {
    format!("sh.olma.{name}")
}

pub fn relocate_service(service: &mut Service, swaps: &[(String, String)]) {
    for entry in service.run.iter_mut() {
        *entry = swap_all(entry, swaps);
    }
    if let Some(p) = service.working_dir.as_mut() {
        *p = swap_all(p, swaps);
    }
    if let Some(p) = service.log_path.as_mut() {
        *p = swap_all(p, swaps);
    }
    if let Some(p) = service.error_log_path.as_mut() {
        *p = swap_all(p, swaps);
    }
    for (_k, v) in service.environment_variables.iter_mut() {
        *v = swap_all(v, swaps);
    }
}

fn swap_all(s: &str, swaps: &[(String, String)]) -> String {
    let mut bytes = s.as_bytes().to_vec();
    for (old, new) in swaps {
        bytes = replace_prefix_in_bytes(&bytes, old, new);
    }
    String::from_utf8(bytes).unwrap_or_else(|_| s.to_string())
}
```

- [ ] **Step 3: Write the failing test**

`tests/unit_plist.rs`:

```rust
use olma::metadata::Service;
use olma::services::plist::render;

#[test]
fn renders_minimal_plist() {
    let svc = Service {
        run: vec!["/opt/olma/packages/redis/7.4.0/bin/redis-server".to_string()],
        keep_alive: Some(olma::metadata::KeepAlive::Bool(true)),
        working_dir: Some("/opt/olma/var".to_string()),
        log_path: None,
        error_log_path: None,
        environment_variables: Default::default(),
        process_type: None,
    };

    let xml = render(
        "redis",
        &svc,
        std::path::Path::new("/opt/olma/log/redis/out.log"),
        std::path::Path::new("/opt/olma/log/redis/err.log"),
        true,
    );

    assert!(xml.contains("<key>Label</key>\n\t<string>sh.olma.redis</string>"));
    assert!(xml.contains("<key>RunAtLoad</key>\n\t<true/>"));
    assert!(xml.contains("<key>KeepAlive</key>\n\t<true/>"));
    assert!(xml.contains("<key>WorkingDirectory</key>\n\t<string>/opt/olma/var</string>"));
    assert!(xml.contains("<key>StandardOutPath</key>\n\t<string>/opt/olma/log/redis/out.log</string>"));
    assert!(xml.contains("<key>ProgramArguments</key>"));
    assert!(xml.contains("<string>/opt/olma/packages/redis/7.4.0/bin/redis-server</string>"));
}

#[test]
fn run_at_load_off_when_not_enabled() {
    let svc = Service {
        run: vec!["/usr/bin/true".to_string()],
        keep_alive: None,
        working_dir: None,
        log_path: None,
        error_log_path: None,
        environment_variables: Default::default(),
        process_type: None,
    };
    let xml = render(
        "noop",
        &svc,
        std::path::Path::new("/tmp/out"),
        std::path::Path::new("/tmp/err"),
        false,
    );
    assert!(xml.contains("<key>RunAtLoad</key>\n\t<false/>"));
}
```

- [ ] **Step 4: Run to verify failure**

Run: `cargo test --test unit_plist 2>&1 | tail -10`
Expected: compile error — `render` does not exist.

- [ ] **Step 5: Implement `plist.rs`**

```rust
use crate::metadata::{KeepAlive, Service};
use std::fmt::Write as _;
use std::path::Path;

pub fn render(
    name: &str,
    service: &Service,
    out_log: &Path,
    err_log: &Path,
    run_at_load: bool,
) -> String {
    let label = format!("sh.olma.{name}");
    let mut s = String::with_capacity(1024);
    s.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
"#);

    kv_string(&mut s, "Label", &label);
    write!(s, "\t<key>RunAtLoad</key>\n\t<{}/>\n", if run_at_load { "true" } else { "false" }).unwrap();

    if let Some(ka) = &service.keep_alive {
        match ka {
            KeepAlive::Bool(b) => {
                write!(s, "\t<key>KeepAlive</key>\n\t<{}/>\n", if *b { "true" } else { "false" }).unwrap();
            }
            KeepAlive::Obj(map) => {
                s.push_str("\t<key>KeepAlive</key>\n\t<dict>\n");
                for (k, v) in map {
                    if let Some(b) = v.as_bool() {
                        write!(s, "\t\t<key>{k}</key>\n\t\t<{}/>\n", if b { "true" } else { "false" }).unwrap();
                    } else if let Some(n) = v.as_i64() {
                        write!(s, "\t\t<key>{k}</key>\n\t\t<integer>{n}</integer>\n").unwrap();
                    }
                }
                s.push_str("\t</dict>\n");
            }
        }
    }

    if let Some(wd) = &service.working_dir {
        kv_string(&mut s, "WorkingDirectory", wd);
    }
    if let Some(pt) = &service.process_type {
        kv_string(&mut s, "ProcessType", pt);
    } else {
        kv_string(&mut s, "ProcessType", "Background");
    }
    kv_string(&mut s, "StandardOutPath", &out_log.to_string_lossy());
    kv_string(&mut s, "StandardErrorPath", &err_log.to_string_lossy());

    if !service.run.is_empty() {
        s.push_str("\t<key>ProgramArguments</key>\n\t<array>\n");
        for arg in &service.run {
            write!(s, "\t\t<string>{}</string>\n", xml_escape(arg)).unwrap();
        }
        s.push_str("\t</array>\n");
    }

    if !service.environment_variables.is_empty() {
        s.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
        for (k, v) in &service.environment_variables {
            write!(s, "\t\t<key>{}</key>\n\t\t<string>{}</string>\n", xml_escape(k), xml_escape(v)).unwrap();
        }
        s.push_str("\t</dict>\n");
    }

    s.push_str("</dict>\n</plist>\n");
    s
}

fn kv_string(buf: &mut String, key: &str, value: &str) {
    write!(buf, "\t<key>{key}</key>\n\t<string>{}</string>\n", xml_escape(value)).unwrap();
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
```

- [ ] **Step 6: Tests green**

Run: `cargo test --test unit_plist 2>&1 | tail -10`
Expected: 2 passing tests.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/services/mod.rs src/services/plist.rs tests/unit_plist.rs
git commit -m "feat(services): render launchd plist from Service struct"
```

---

## Task 4: launchctl wrapper

**Files:**
- Create: `src/services/launchctl.rs`
- Create: `tests/unit_launchctl_parse.rs`

- [ ] **Step 1: Define the wrapper surface**

```rust
use crate::error::{OlmaError, Result};
use std::path::Path;

pub fn current_uid() -> Result<u32> {
    let out = std::process::Command::new("id").arg("-u").output()
        .map_err(|e| OlmaError::Other(format!("id -u failed: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other("id -u failed".into()));
    }
    String::from_utf8_lossy(&out.stdout).trim().parse::<u32>()
        .map_err(|e| OlmaError::Other(format!("uid parse failed: {e}")))
}

pub fn bootstrap(plist: &Path) -> Result<()> {
    let uid = current_uid()?;
    let domain = format!("gui/{uid}");
    run(&["bootstrap", &domain, plist.to_str().unwrap()])
}

pub fn bootout(name: &str) -> Result<()> {
    let uid = current_uid()?;
    let target = format!("gui/{uid}/sh.olma.{name}");
    run(&["bootout", &target])
}

pub fn kickstart(name: &str) -> Result<()> {
    let uid = current_uid()?;
    let target = format!("gui/{uid}/sh.olma.{name}");
    run(&["kickstart", "-k", &target])
}

pub fn disable(name: &str) -> Result<()> {
    let uid = current_uid()?;
    let target = format!("gui/{uid}/sh.olma.{name}");
    run(&["disable", &target])
}

pub fn print(name: &str) -> Result<String> {
    let uid = current_uid()?;
    let target = format!("gui/{uid}/sh.olma.{name}");
    let out = std::process::Command::new("launchctl").args(["print", &target]).output()
        .map_err(|e| OlmaError::Other(format!("launchctl print failed: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other(format!(
            "launchctl print {target} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn run(args: &[&str]) -> Result<()> {
    let out = std::process::Command::new("launchctl").args(args).output()
        .map_err(|e| OlmaError::Other(format!("launchctl spawn failed: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other(format!(
            "launchctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PrintSnapshot {
    pub pid: Option<u32>,
    pub state: ServiceState,
    pub last_exit: Option<i32>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum ServiceState {
    #[default]
    Unknown,
    Running,
    Stopped,
    Failed,
}

pub fn parse_print(out: &str) -> PrintSnapshot {
    let mut snap = PrintSnapshot::default();
    for line in out.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("pid = ") {
            snap.pid = rest.split_whitespace().next().and_then(|s| s.parse().ok());
        } else if let Some(rest) = t.strip_prefix("state = ") {
            snap.state = match rest.split_whitespace().next().unwrap_or("") {
                "running" => ServiceState::Running,
                "not running" => ServiceState::Stopped,
                _ => ServiceState::Unknown,
            };
        } else if let Some(rest) = t.strip_prefix("last exit code = ") {
            snap.last_exit = rest.split_whitespace().next().and_then(|s| s.parse().ok());
        }
    }
    snap
}
```

- [ ] **Step 2: Write the parser test**

`tests/unit_launchctl_parse.rs`:

```rust
use olma::services::launchctl::{parse_print, ServiceState};

const RUNNING_SAMPLE: &str = r#"
gui/501/sh.olma.redis = {
    pid = 8421
    state = running
    last exit code = 0
}
"#;

const STOPPED_SAMPLE: &str = r#"
gui/501/sh.olma.redis = {
    state = not running
    last exit code = 0
}
"#;

#[test]
fn parses_running_state() {
    let snap = parse_print(RUNNING_SAMPLE);
    assert_eq!(snap.pid, Some(8421));
    assert!(matches!(snap.state, ServiceState::Running));
}

#[test]
fn parses_stopped_state() {
    let snap = parse_print(STOPPED_SAMPLE);
    assert_eq!(snap.pid, None);
    assert!(matches!(snap.state, ServiceState::Stopped));
}
```

- [ ] **Step 3: Tests pass**

Run: `cargo test --test unit_launchctl_parse 2>&1 | tail -10`
Expected: 2 passing tests.

- [ ] **Step 4: Commit**

```bash
git add src/services/launchctl.rs tests/unit_launchctl_parse.rs
git commit -m "feat(services): wrap launchctl bootstrap/bootout/kickstart/print"
```

---

## Task 5: Services store (state.db CRUD)

**Files:**
- Create: `src/services/store.rs`

- [ ] **Step 1: CRUD over the `services` row**

```rust
use crate::error::Result;
use crate::state::Db;

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
                rusqlite::params![row.name, row.plist_path, row.enabled as i64, row.last_action, row.last_action_ts],
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
            ).optional().map_err(Into::into)
        })
    }

    pub fn list(db: &Db) -> Result<Vec<ServiceRow>> {
        db.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT name, plist_path, enabled, last_action, last_action_ts FROM services ORDER BY name"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(ServiceRow {
                    name: row.get(0)?,
                    plist_path: row.get(1)?,
                    enabled: row.get::<_, i64>(2)? != 0,
                    last_action: row.get(3)?,
                    last_action_ts: row.get(4)?,
                })
            })?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn delete(db: &Db, name: &str) -> Result<()> {
        db.with_conn(|c| {
            c.execute("DELETE FROM services WHERE name = ?1", rusqlite::params![name])?;
            Ok(())
        })
    }
}
```

(Assumes `Db::with_conn` exists from Plan 3.)

- [ ] **Step 2: Build**

Run: `cargo check --lib 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/services/store.rs
git commit -m "feat(services): ServiceRow CRUD over state.db services table"
```

---

## Task 6: CLI scaffold — `olma services` subcommand router

**Files:**
- Create: `src/cli/services/mod.rs`
- Modify: `src/cli/mod.rs` (register subcommand)

- [ ] **Step 1: Add the `services` enum to the top-level CLI**

In `src/cli/mod.rs`:

```rust
pub mod services;

// In the existing Cmd enum:
Services {
    #[command(subcommand)]
    action: services::Action,
},
```

And in `run()`:

```rust
Cmd::Services { action } => services::run(action, reporter.as_ref()).await,
```

- [ ] **Step 2: Create `src/cli/services/mod.rs`**

```rust
pub mod disable;
pub mod enable;
pub mod list;
pub mod logs;
pub mod reload;
pub mod restart;
pub mod start;
pub mod status;
pub mod stop;

use clap::Subcommand;

use crate::error::Result;
use crate::output::Reporter;

#[derive(Subcommand)]
pub enum Action {
    List,
    Start { name: String },
    Stop { name: String },
    Restart { name: String },
    Status { name: String },
    Enable { name: String },
    Disable { name: String },
    Logs { name: String },
    Reload { name: String },
}

pub async fn run(action: Action, reporter: &dyn Reporter) -> Result<()> {
    match action {
        Action::List => list::run(reporter).await,
        Action::Start { name } => start::run(&name, reporter).await,
        Action::Stop { name } => stop::run(&name, reporter).await,
        Action::Restart { name } => restart::run(&name, reporter).await,
        Action::Status { name } => status::run(&name, reporter).await,
        Action::Enable { name } => enable::run(&name, reporter).await,
        Action::Disable { name } => disable::run(&name, reporter).await,
        Action::Logs { name } => logs::run(&name, reporter).await,
        Action::Reload { name } => reload::run(&name, reporter).await,
    }
}
```

Create all nine subcommand stub files with a `pub async fn run(...) -> Result<()> { Ok(()) }` placeholder so the build is green. Each will be implemented in Tasks 7–15.

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -5`
Expected: builds; `./target/debug/olma services --help` lists all nine actions.

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/services
git commit -m "feat(services): scaffold olma services CLI router and action enum"
```

---

## Task 7: `services enable`

**Files:**
- Modify: `src/cli/services/enable.rs`

- [ ] **Step 1: Implement**

```rust
use crate::cli::services::log_paths;
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::metadata::client::FormulaeClient;
use crate::output::Reporter;
use crate::services::{label_for, launchctl, plist, relocate_service, store::ServiceRow};
use crate::state::Db;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn run(name: &str, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let client = FormulaeClient::new(&config)?;
    let formula = client.fetch(name).await?;
    let mut service = formula.service.clone()
        .ok_or_else(|| OlmaError::Other(format!("{name} does not provide a service")))?;

    let swaps = vec![
        (format!("/opt/homebrew/Cellar/{}/{}", formula.name, formula.version()),
         config.package_dir(&formula.name, formula.version()).to_string_lossy().to_string()),
        ("/opt/homebrew".into(), config.root.to_string_lossy().to_string()),
        ("/usr/local".into(), config.root.to_string_lossy().to_string()),
    ];
    relocate_service(&mut service, &swaps);

    let (out_log, err_log) = log_paths(&config, name);
    std::fs::create_dir_all(out_log.parent().unwrap())?;
    let xml = plist::render(name, &service, &out_log, &err_log, true);

    let plist_path = launch_agents_dir()?.join(format!("{}.plist", label_for(name)));
    std::fs::create_dir_all(plist_path.parent().unwrap())?;
    std::fs::write(&plist_path, &xml)?;

    launchctl::bootstrap(&plist_path)?;

    ServiceRow::upsert(&db, &ServiceRow {
        name: name.to_string(),
        plist_path: plist_path.to_string_lossy().to_string(),
        enabled: true,
        last_action: Some("enable".into()),
        last_action_ts: Some(now_ts()),
    })?;

    if let Some(caveats) = &formula.caveats {
        reporter.status(caveats);
    }
    reporter.success(&format!("Enabled service {name}"));
    Ok(())
}

fn launch_agents_dir() -> Result<std::path::PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| OlmaError::Other("no home dir".into()))?;
    Ok(home.join("Library").join("LaunchAgents"))
}

fn now_ts() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
```

Add helpers to `src/cli/services/mod.rs`:

```rust
pub fn log_paths(config: &crate::config::Config, name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let base = config.root.join("log").join(name);
    (base.join("out.log"), base.join("err.log"))
}
```

Also extend `Formula` (Task 1 already adds `service`) with `caveats: Option<String>`.

- [ ] **Step 2: Build**

Run: `cargo check --lib 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/cli/services
git commit -m "feat(services): implement olma services enable"
```

---

## Task 8: `services disable`

**Files:**
- Modify: `src/cli/services/disable.rs`

- [ ] **Step 1: Implement**

```rust
use crate::config::Config;
use crate::error::Result;
use crate::output::Reporter;
use crate::services::{launchctl, store::ServiceRow};
use crate::state::Db;

pub async fn run(name: &str, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;

    let _ = launchctl::bootout(name);
    let _ = launchctl::disable(name);

    if let Some(row) = ServiceRow::get(&db, name)? {
        let _ = std::fs::remove_file(&row.plist_path);
        ServiceRow::upsert(&db, &ServiceRow {
            enabled: false,
            last_action: Some("disable".into()),
            last_action_ts: Some(now_ts()),
            ..row
        })?;
    }
    reporter.success(&format!("Disabled service {name}"));
    Ok(())
}

fn now_ts() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
```

- [ ] **Step 2: Build + commit**

```bash
cargo check --lib 2>&1 | tail -5
git add src/cli/services/disable.rs
git commit -m "feat(services): implement olma services disable"
```

---

## Task 9: `services start`

**Files:**
- Modify: `src/cli/services/start.rs`

- [ ] **Step 1: Implement**

```rust
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::output::Reporter;
use crate::services::{launchctl, store::ServiceRow};
use crate::state::Db;

pub async fn run(name: &str, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let row = ServiceRow::get(&db, name)?
        .ok_or_else(|| OlmaError::Other(format!(
            "{name} is not enabled. Run `olma services enable {name}` first."
        )))?;

    if !std::path::Path::new(&row.plist_path).exists() {
        return Err(OlmaError::Other(format!("plist missing for {name}; try `olma services enable {name}`")));
    }
    let _ = launchctl::bootstrap(std::path::Path::new(&row.plist_path));
    launchctl::kickstart(name)?;
    ServiceRow::upsert(&db, &ServiceRow {
        last_action: Some("start".into()),
        last_action_ts: Some(now_ts()),
        ..row
    })?;
    reporter.success(&format!("Started {name}"));
    Ok(())
}

fn now_ts() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
```

- [ ] **Step 2: Build + commit**

```bash
cargo check --lib 2>&1 | tail -5
git add src/cli/services/start.rs
git commit -m "feat(services): implement olma services start"
```

---

## Task 10: `services stop`

**Files:** Modify `src/cli/services/stop.rs`

- [ ] **Step 1: Implement**

```rust
use crate::error::Result;
use crate::output::Reporter;
use crate::services::launchctl;

pub async fn run(name: &str, reporter: &dyn Reporter) -> Result<()> {
    launchctl::bootout(name)?;
    reporter.success(&format!("Stopped {name}"));
    Ok(())
}
```

- [ ] **Step 2: Build + commit**

```bash
git add src/cli/services/stop.rs
git commit -m "feat(services): implement olma services stop"
```

---

## Task 11: `services restart`

**Files:** Modify `src/cli/services/restart.rs`

- [ ] **Step 1: Implement**

```rust
use crate::error::Result;
use crate::output::Reporter;
use crate::services::launchctl;

pub async fn run(name: &str, reporter: &dyn Reporter) -> Result<()> {
    let _ = launchctl::bootout(name);
    launchctl::kickstart(name).or_else(|_| {
        Err(crate::error::OlmaError::Other(format!(
            "could not restart {name}; ensure it is enabled"
        )))
    })?;
    reporter.success(&format!("Restarted {name}"));
    Ok(())
}
```

- [ ] **Step 2: Build + commit**

```bash
git add src/cli/services/restart.rs
git commit -m "feat(services): implement olma services restart"
```

---

## Task 12: `services status`

**Files:** Modify `src/cli/services/status.rs`

- [ ] **Step 1: Implement**

```rust
use crate::cli::services::log_paths;
use crate::config::Config;
use crate::error::Result;
use crate::output::Reporter;
use crate::services::launchctl;

pub async fn run(name: &str, reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let snap = launchctl::parse_print(&launchctl::print(name).unwrap_or_default());
    let state_str = match snap.state {
        launchctl::ServiceState::Running => "●  running",
        launchctl::ServiceState::Stopped => "○  stopped",
        launchctl::ServiceState::Failed  => "●  failed",
        launchctl::ServiceState::Unknown => "?  unknown",
    };
    println!("  {name}  {state_str}");
    if let Some(pid) = snap.pid {
        println!("  pid    {pid}");
    }
    let (_out, err) = log_paths(&config, name);
    if let Ok(contents) = std::fs::read_to_string(&err) {
        let tail: Vec<&str> = contents.lines().rev().take(10).collect::<Vec<_>>().into_iter().rev().collect();
        if !tail.is_empty() {
            println!("\n  Recent logs (last 10 lines):");
            for line in tail { println!("  {line}"); }
        }
    }
    let _ = reporter;
    Ok(())
}
```

- [ ] **Step 2: Build + commit**

```bash
git add src/cli/services/status.rs
git commit -m "feat(services): implement olma services status"
```

---

## Task 13: `services list`

**Files:** Modify `src/cli/services/list.rs`

- [ ] **Step 1: Implement**

```rust
use crate::config::Config;
use crate::error::Result;
use crate::output::Reporter;
use crate::services::{launchctl, store::ServiceRow};
use crate::state::Db;

pub async fn run(_reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let rows = ServiceRow::list(&db)?;

    println!("  {:<14} {:<10} {:<6} {}", "NAME", "STATE", "PID", "AUTOSTART");
    for row in rows {
        let snap = launchctl::parse_print(&launchctl::print(&row.name).unwrap_or_default());
        let state = match snap.state {
            launchctl::ServiceState::Running => "running",
            launchctl::ServiceState::Stopped => "stopped",
            launchctl::ServiceState::Failed  => "failed",
            launchctl::ServiceState::Unknown => "—",
        };
        let pid = snap.pid.map(|p| p.to_string()).unwrap_or("-".into());
        let auto = if row.enabled { "on" } else { "off" };
        println!("  {:<14} {:<10} {:<6} {}", row.name, state, pid, auto);
    }
    Ok(())
}
```

- [ ] **Step 2: Build + commit**

```bash
git add src/cli/services/list.rs
git commit -m "feat(services): implement olma services list"
```

---

## Task 14: `services logs`

**Files:** Modify `src/cli/services/logs.rs`

- [ ] **Step 1: Implement**

```rust
use crate::cli::services::log_paths;
use crate::config::Config;
use crate::error::Result;
use crate::output::Reporter;

pub async fn run(name: &str, _reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let (out, err) = log_paths(&config, name);
    for (label, path) in [("stdout", out), ("stderr", err)] {
        match std::fs::read_to_string(&path) {
            Ok(c) if !c.is_empty() => {
                println!("\n  === {label} ({}) ===", path.display());
                println!("{c}");
            }
            _ => {}
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Build + commit**

```bash
git add src/cli/services/logs.rs
git commit -m "feat(services): implement olma services logs"
```

---

## Task 15: `services reload`

**Files:** Modify `src/cli/services/reload.rs`

- [ ] **Step 1: Implement**

```rust
use crate::error::Result;
use crate::output::Reporter;
use crate::services::launchctl;

pub async fn run(name: &str, reporter: &dyn Reporter) -> Result<()> {
    launchctl::kickstart(name)?;
    reporter.success(&format!("Reloaded {name}"));
    Ok(())
}
```

- [ ] **Step 2: Build + commit**

```bash
git add src/cli/services/reload.rs
git commit -m "feat(services): implement olma services reload"
```

---

## Task 16: Add-time hint when the package provides a service

**Files:** Modify `src/cli/add.rs`

- [ ] **Step 1: After successful install, print the hint**

```rust
if formula.service.is_some() {
    reporter.status(&format!(
        "ℹ  Service available. Run `olma services start {}`.",
        formula.name
    ));
}
```

- [ ] **Step 2: Build + commit**

```bash
git add src/cli/add.rs
git commit -m "feat(services): hint at services after add"
```

---

## Task 17: Remove-time stop + disable

**Files:** Modify `src/cli/remove.rs` (created in Plan 4)

- [ ] **Step 1: Before removing the package directory, deactivate its service**

```rust
let db = Db::open(&config)?;
if let Some(row) = ServiceRow::get(&db, name)?
    && row.enabled
{
    let _ = launchctl::bootout(name);
    let _ = launchctl::disable(name);
    let _ = std::fs::remove_file(&row.plist_path);
    ServiceRow::delete(&db, name)?;
    reporter.status(&format!("Stopped and disabled service {name}"));
}
```

- [ ] **Step 2: Build + commit**

```bash
git add src/cli/remove.rs
git commit -m "feat(services): stop and disable service on remove"
```

---

## Task 18: Upgrade restarts running services

**Files:** Modify `src/cli/upgrade.rs` (created in Plan 4)

- [ ] **Step 1: Record running state before upgrade**

```rust
let was_running = matches!(
    launchctl::parse_print(&launchctl::print(name).unwrap_or_default()).state,
    launchctl::ServiceState::Running
);
```

After the new version is fully installed and relocated:

```rust
if was_running && formula.service.is_some() {
    let _ = crate::cli::services::start::run(name, reporter).await;
}
```

- [ ] **Step 2: Build + commit**

```bash
git add src/cli/upgrade.rs
git commit -m "feat(services): preserve running state across upgrades"
```

---

## Task 19: Readd preserves running state

**Files:** Modify `src/cli/readd.rs` (created in Plan 4)

- [ ] **Step 1: Apply the same running-state capture and restore as upgrade**

(Identical pattern to Task 18 — capture before, restore after.)

- [ ] **Step 2: Build + commit**

```bash
git add src/cli/readd.rs
git commit -m "feat(services): preserve running state across readd"
```

---

## Task 20: Integration test against a real service

**Files:**
- Create: `tests/integration_services_mosquitto.rs`

`mosquitto` is small, has a service block, and binds to port 1883 — we override the port so the test does not collide with anything the developer already runs.

- [ ] **Step 1: Write the test**

```rust
mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enable_start_status_stop_mosquitto() {
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok()
        || std::env::var("OLMA_SKIP_SERVICE_TESTS").is_ok()
    {
        eprintln!("skipped");
        return;
    }

    let sandbox = Sandbox::new();
    let reporter = olma::output::default_reporter();

    olma::cli::add::run("mosquitto", reporter.as_ref()).await.expect("add");
    olma::cli::services::enable::run("mosquitto", reporter.as_ref()).await.expect("enable");
    olma::cli::services::start::run("mosquitto", reporter.as_ref()).await.expect("start");

    std::thread::sleep(std::time::Duration::from_millis(500));

    let print_out = olma::services::launchctl::print("mosquitto").unwrap_or_default();
    let snap = olma::services::launchctl::parse_print(&print_out);
    assert!(matches!(snap.state, olma::services::launchctl::ServiceState::Running));

    olma::cli::services::stop::run("mosquitto", reporter.as_ref()).await.expect("stop");
    olma::cli::services::disable::run("mosquitto", reporter.as_ref()).await.expect("disable");

    let plist_path = sandbox.root_path()
        .join("..")  // mosquitto plist lives under ~/Library/LaunchAgents during the test
        .join("LaunchAgents")
        .join("sh.olma.mosquitto.plist");
    assert!(!plist_path.exists(), "plist should be gone after disable");
}
```

(The test fixture lives under `~/Library/LaunchAgents` because that is what launchd reads; the sandbox handles cleanup by calling `disable` at the end. CI runners that cannot bind launchd should set `OLMA_SKIP_SERVICE_TESTS`.)

- [ ] **Step 2: Run the test**

Run: `cargo test --test integration_services_mosquitto -- --nocapture 2>&1 | tail -40`
Expected: pass on a developer Mac.

- [ ] **Step 3: Commit**

```bash
git add tests/integration_services_mosquitto.rs
git commit -m "test(services): end-to-end enable/start/status/stop with mosquitto"
```

---

## Task 21: Self-review checkpoint

- [ ] **Step 1: Full test suite**

Run: `cargo test 2>&1 | tail -30`
Expected: every existing test still passes; new plist + launchctl-parse + integration tests pass.

- [ ] **Step 2: Clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 3: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-svc ./target/release/olma add mosquitto
OLMA_ROOT=/tmp/olma-svc ./target/release/olma services enable mosquitto
OLMA_ROOT=/tmp/olma-svc ./target/release/olma services start mosquitto
OLMA_ROOT=/tmp/olma-svc ./target/release/olma services status mosquitto
OLMA_ROOT=/tmp/olma-svc ./target/release/olma services list
OLMA_ROOT=/tmp/olma-svc ./target/release/olma services stop mosquitto
OLMA_ROOT=/tmp/olma-svc ./target/release/olma services disable mosquitto
```

Each step prints a coherent state line; the plist appears under `~/Library/LaunchAgents/` between enable and disable; mosquitto runs and stops cleanly.

- [ ] **Step 4: Final commit if needed**

```bash
git status
# If anything modified:
git add -A
git commit -m "chore(services): address clippy warnings from self-review"
```

---

## Plan-level verification

After Task 21:

1. `olma services` is a full subcommand with nine actions.
2. Plists are rendered correctly from any formula's service block.
3. launchctl is driven via the modern bootstrap/bootout/kickstart API.
4. `add` hints at services. `remove` cleans them up. `upgrade` and `readd` preserve running state.
5. State is persisted in the `services` table; live state always comes from `launchctl print`.
6. Integration test exercises the full happy path against a real daemon.

## Out of scope (reminder)

V2:
- System-level services (`--system`, `/Library/LaunchDaemons`, sudo).
- `services logs --follow`.
- `pre_start_hook` for services that need `initdb`-style setup.
- Multi-instance services.
