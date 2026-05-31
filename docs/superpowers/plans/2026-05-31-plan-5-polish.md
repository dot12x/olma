# Plan 5 — Polish (Verbosity, JSON, Did-You-Mean, Completions, Self-Update) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close out the V1 surface with the polish layer that turns olma from "it works" into "it ships". Add `-q / -v / -vv` logging tiers, a `--json` machine-readable mode for `list` / `info` / `outdated` / `search` / `history`, did-you-mean suggestions on unknown formula names, a `self-update` verb that swaps the binary in place, a `completions <shell>` generator for zsh/bash/fish, first-run PATH hints, the spec §12 error layout, and a cache-age advisory footer on `outdated` / `upgrade`.

**Architecture:** Extend the `Reporter` trait with two new methods (`verbose` and `debug`) that default to no-ops; existing `SoftReporter` overrides only `success` / `error` / `status` so nothing else changes. Add a `VerboseReporter` that honors the verbosity level. `--json` lives on `Cli` as a global flag and short-circuits the reporter — each supported subcommand builds a `serde_json::Value` and prints it with `serde_json::to_string_pretty` at the end. Did-you-mean reads `<root>/cache/formulae/__index.json` (the full dump produced by `olma update --full` in Plan 4); a missing dump triggers a graceful degradation to a static footer hint. `self-update` lives in `cli/self_update.rs` and uses the GitHub Releases API for the `dot12x/olma` repo. The `clap_complete` crate ships the completion generator.

**Tech Stack:** Rust 2024 · `clap_complete` (zsh / bash / fish output) · `strsim` (Levenshtein) · existing `serde_json` · existing `reqwest` (for self-update) · existing `sha2` (for self-update checksum). No native code.

**Depends on:** Plan 1 (CLI scaffold, `Reporter`), Plan 2 (`Pipeline`), Plan 3 (`state.db`, `list`, `history`), **Plan 4** (`olma update --full` which writes `cache/formulae/__index.json`; `info`, `outdated`, `search` whose output we route through `--json`).

> If Plan 4 has not yet landed `__index.json`, Task 5 (did-you-mean) automatically falls into the "degraded" branch and prints the `Run \`olma update --full\` for search suggestions.` hint. Task 7 (`--json` for `info` / `outdated` / `search`) wires into the existing handlers from Plan 4; if a particular handler isn't merged yet, that subcommand's JSON arm becomes a stub that returns an empty object. The dependency is flagged in the per-task header.

Spec reference: `docs/superpowers/specs/2026-05-31-olma-architecture-design.md` §8 (global flags), §12 (error format & did-you-mean degradation).

---

## File structure (touched in this plan)

```
src/
├── output/
│   ├── mod.rs              MODIFY  add verbose/debug methods to Reporter, JSON sink helper
│   ├── soft.rs             MODIFY  inherit defaults for new methods
│   ├── verbose.rs          NEW     VerboseReporter honoring -v / -vv / -q
│   └── quiet.rs            NEW     QuietReporter (errors only)
├── cli/
│   ├── mod.rs              MODIFY  global -q / -v / --json flags, reporter factory, footer hooks
│   ├── add.rs              MODIFY  emit first-run PATH hint on success
│   ├── list.rs             MODIFY  JSON branch
│   ├── history.rs          MODIFY  JSON branch
│   ├── info.rs             MODIFY  JSON branch                   (depends on Plan 4 landing info.rs)
│   ├── outdated.rs         MODIFY  JSON branch + cache-age footer (depends on Plan 4)
│   ├── search.rs           MODIFY  JSON branch                   (depends on Plan 4)
│   ├── upgrade.rs          MODIFY  cache-age footer               (depends on Plan 4)
│   ├── self_update.rs      NEW     rustup-style binary swap
│   └── completions.rs      NEW     clap_complete generator
├── suggest.rs              NEW     did-you-mean: substring + Levenshtein over __index.json
├── error.rs                MODIFY  format_error helper for spec §12 layout
└── cache.rs                MODIFY  formulae_index_mtime() helper

Cargo.toml                  MODIFY  add clap_complete, strsim

tests/
├── unit_did_you_mean.rs    NEW     substring then Levenshtein behavior
├── unit_json_emit.rs       NEW     stable shape for list --json
├── unit_error_format.rs    NEW     spec §12 error layout
└── integration_polish_did_you_mean.rs   NEW   end-to-end did-you-mean on add
```

---

## Task 1: Cargo deps + `Reporter` trait extension

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/output/mod.rs`
- Modify: `src/output/soft.rs`

The `verbose` and `debug` methods are added to `Reporter` with default empty bodies so every existing implementer (including the live `SoftReporter`) keeps compiling untouched.

- [ ] **Step 1: Add deps to `Cargo.toml`**

In `[dependencies]`, alphabetical:

```toml
clap_complete = "4"
strsim = "0.11"
```

- [ ] **Step 2: Extend the trait**

Replace `src/output/mod.rs`:

```rust
mod quiet;
mod soft;
mod verbose;

pub use quiet::QuietReporter;
pub use soft::SoftReporter;
pub use verbose::VerboseReporter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
    Debug,
}

impl Verbosity {
    pub fn from_flags(quiet: bool, v_count: u8) -> Self {
        if quiet {
            return Verbosity::Quiet;
        }
        match v_count {
            0 => Verbosity::Normal,
            1 => Verbosity::Verbose,
            _ => Verbosity::Debug,
        }
    }
}

pub trait Reporter: Send + Sync {
    fn status(&self, _msg: &str) {}
    fn success(&self, _msg: &str) {}
    fn error(&self, _msg: &str) {}
    fn verbose(&self, _msg: &str) {}
    fn debug(&self, _msg: &str) {}
    fn footer(&self, _msg: &str) {}
}

pub fn reporter_for(v: Verbosity, json: bool) -> Box<dyn Reporter> {
    if json {
        return Box::new(QuietReporter::new());
    }
    match v {
        Verbosity::Quiet => Box::new(QuietReporter::new()),
        Verbosity::Normal => Box::new(SoftReporter::new()),
        Verbosity::Verbose => Box::new(VerboseReporter::new(false)),
        Verbosity::Debug => Box::new(VerboseReporter::new(true)),
    }
}

pub fn default_reporter() -> Box<dyn Reporter> {
    Box::new(SoftReporter::new())
}
```

- [ ] **Step 3: Make `SoftReporter` honor the new shape**

The Reporter methods are now provided by trait defaults, so `SoftReporter` must keep `status`/`success`/`error` overridden but inherit `verbose`/`debug`/`footer` as no-ops. Modify `src/output/soft.rs` — the existing method bodies stay, only the impl block header gets `footer` to render a final dim line:

Add the footer method to `impl Reporter for SoftReporter`:

```rust
fn footer(&self, msg: &str) {
    let mut err = std::io::stderr().lock();
    use std::io::Write;
    if self.is_tty {
        let _ = write!(err, "\x1b[2m{msg}\x1b[0m\n");
    } else {
        let _ = writeln!(err, "{msg}");
    }
    let _ = err.flush();
}
```

- [ ] **Step 4: Build**

```bash
cargo check --lib 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/output/mod.rs src/output/soft.rs
git commit -m "feat(output): extend Reporter with verbose/debug/footer hooks"
```

---

## Task 2: `QuietReporter` and `VerboseReporter`

**Files:**
- Create: `src/output/quiet.rs`
- Create: `src/output/verbose.rs`

- [ ] **Step 1: Implement `QuietReporter`**

`src/output/quiet.rs`:

```rust
use super::Reporter;
use std::io::{IsTerminal, Write};

pub struct QuietReporter;

impl QuietReporter {
    pub fn new() -> Self { QuietReporter }
}

impl Default for QuietReporter {
    fn default() -> Self { Self::new() }
}

impl Reporter for QuietReporter {
    fn error(&self, msg: &str) {
        let mut err = std::io::stderr().lock();
        let is_tty = std::io::stderr().is_terminal();
        if is_tty {
            let _ = writeln!(err, "\x1b[31merror:\x1b[0m {msg}");
        } else {
            let _ = writeln!(err, "error: {msg}");
        }
        let _ = err.flush();
    }
}
```

- [ ] **Step 2: Implement `VerboseReporter`**

`src/output/verbose.rs`:

```rust
use super::Reporter;
use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::time::Instant;

pub struct VerboseReporter {
    debug: bool,
    started: Instant,
    is_tty: bool,
    lock: Mutex<()>,
}

impl VerboseReporter {
    pub fn new(debug: bool) -> Self {
        VerboseReporter {
            debug,
            started: Instant::now(),
            is_tty: std::io::stderr().is_terminal(),
            lock: Mutex::new(()),
        }
    }

    fn write(&self, level: &str, msg: &str) {
        let _g = self.lock.lock().unwrap();
        let secs = self.started.elapsed().as_secs_f64();
        let mut err = std::io::stderr().lock();
        if self.is_tty {
            let _ = writeln!(err, "\x1b[2m[{secs:>7.3}s {level:>5}]\x1b[0m {msg}");
        } else {
            let _ = writeln!(err, "[{secs:>7.3}s {level:>5}] {msg}");
        }
        let _ = err.flush();
    }
}

impl Reporter for VerboseReporter {
    fn status(&self, msg: &str) { self.write("info", msg); }
    fn success(&self, msg: &str) { self.write(" ok ", msg); }
    fn error(&self, msg: &str) { self.write("error", msg); }
    fn verbose(&self, msg: &str) { self.write("info", msg); }
    fn debug(&self, msg: &str) {
        if self.debug {
            self.write("debug", msg);
        }
    }
    fn footer(&self, msg: &str) { self.write("note", msg); }
}
```

- [ ] **Step 3: Build**

```bash
cargo check --lib 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add src/output/quiet.rs src/output/verbose.rs
git commit -m "feat(output): add QuietReporter (errors only) and VerboseReporter (-v/-vv)"
```

---

## Task 3: Wire `-q / -v / -vv / --json` into `Cli`

**Files:**
- Modify: `src/cli/mod.rs`

- [ ] **Step 1: Add global flags + factory**

In the `Cli` struct, add inside the existing globals block (after `refresh`):

```rust
#[arg(short = 'q', long, global = true, conflicts_with = "verbose")]
pub quiet: bool,
#[arg(short = 'v', long, action = clap::ArgAction::Count, global = true)]
pub verbose: u8,
#[arg(long, global = true)]
pub json: bool,
```

- [ ] **Step 2: Use the factory in `run`**

Replace the `let reporter = default_reporter();` line:

```rust
let verbosity = crate::output::Verbosity::from_flags(cli.quiet, cli.verbose);
let reporter = crate::output::reporter_for(verbosity, cli.json);
```

- [ ] **Step 3: Plumb `json` into each subcommand call**

Where each handler is dispatched, add `cli.json` as the last argument once we modify the handler signatures in Task 6 and Task 7. For now, just compute the value once into a local so the diff is small later:

```rust
let json = cli.json;
```

Use `json` in the dispatch arms updated in subsequent tasks.

- [ ] **Step 4: Build**

```bash
cargo check 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add src/cli/mod.rs
git commit -m "feat(cli): global -q / -v / -vv / --json flags with reporter factory"
```

---

## Task 4: Did-you-mean suggester

**Files:**
- Create: `src/suggest.rs`
- Modify: `src/lib.rs`
- Modify: `src/cache.rs`

The suggester reads `<root>/cache/formulae/__index.json` produced by `olma update --full` (Plan 4). When the file is missing the suggester returns `Suggestions::Degraded`, which the caller renders as the static footer hint.

- [ ] **Step 1: Helper that reads the index mtime**

Open `src/cache.rs` and add to the existing `impl Cache` block (or append the impl if there isn't one — match the file's pre-existing style):

```rust
pub fn formulae_full_index_path(config: &crate::config::Config) -> std::path::PathBuf {
    config.cache_formulae().join("__index.json")
}

pub fn formulae_index_age_days(config: &crate::config::Config) -> Option<u64> {
    let p = formulae_full_index_path(config);
    let meta = std::fs::metadata(&p).ok()?;
    let modified = meta.modified().ok()?;
    let age = modified.elapsed().ok()?;
    Some(age.as_secs() / 86_400)
}
```

If the existing `src/cache.rs` exposes its helpers as methods on a struct rather than free functions, place these as `impl Cache` methods on the same struct and call them with `Cache::formulae_full_index_path(config)` style. Match the prevailing pattern in the file.

- [ ] **Step 2: Register the new module**

In `src/lib.rs`, add `pub mod suggest;` alphabetically (after `pub mod state;`).

- [ ] **Step 3: Implement `src/suggest.rs`**

```rust
use crate::config::Config;
use crate::error::Result;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct IndexEntry {
    name: String,
    #[serde(default)]
    desc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub name: String,
    pub desc: Option<String>,
}

#[derive(Debug)]
pub enum Suggestions {
    Degraded,
    Found(Vec<Hit>),
    None,
}

pub struct Suggester {
    names: Vec<IndexEntry>,
}

impl Suggester {
    pub fn from_config(config: &Config) -> Option<Self> {
        let path = crate::cache::formulae_full_index_path(config);
        Self::from_path(&path)
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let names: Vec<IndexEntry> = serde_json::from_slice(&bytes).ok()?;
        Some(Suggester { names })
    }

    pub fn from_entries(entries: Vec<(String, Option<String>)>) -> Self {
        Suggester {
            names: entries.into_iter()
                .map(|(name, desc)| IndexEntry { name, desc })
                .collect(),
        }
    }

    pub fn suggest(&self, query: &str, max: usize) -> Suggestions {
        if self.names.is_empty() {
            return Suggestions::None;
        }
        let q = query.to_ascii_lowercase();
        let mut subs: Vec<&IndexEntry> = self.names.iter()
            .filter(|e| e.name.to_ascii_lowercase().contains(&q))
            .collect();
        subs.sort_by_key(|e| e.name.len());
        if !subs.is_empty() {
            let out: Vec<Hit> = subs.into_iter().take(max).map(|e| Hit {
                name: e.name.clone(),
                desc: e.desc.clone(),
            }).collect();
            return Suggestions::Found(out);
        }
        let mut leven: Vec<(usize, &IndexEntry)> = self.names.iter()
            .map(|e| (strsim::levenshtein(&q, &e.name.to_ascii_lowercase()), e))
            .filter(|(d, _)| *d <= 2)
            .collect();
        leven.sort_by_key(|(d, _)| *d);
        if leven.is_empty() {
            return Suggestions::None;
        }
        let out: Vec<Hit> = leven.into_iter().take(max).map(|(_, e)| Hit {
            name: e.name.clone(),
            desc: e.desc.clone(),
        }).collect();
        Suggestions::Found(out)
    }
}

pub fn build_suggestion_footer(config: &Config, query: &str) -> Result<Option<String>> {
    let suggester = match Suggester::from_config(config) {
        Some(s) => s,
        None => {
            return Ok(Some(
                "Run `olma update --full` for search suggestions.".into(),
            ));
        }
    };
    match suggester.suggest(query, 5) {
        Suggestions::None => Ok(None),
        Suggestions::Degraded => Ok(Some(
            "Run `olma update --full` for search suggestions.".into(),
        )),
        Suggestions::Found(hits) => {
            let mut out = String::from("\n  Did you mean:\n");
            for h in &hits {
                match &h.desc {
                    Some(d) => out.push_str(&format!("    {:<24} {}\n", h.name, d)),
                    None => out.push_str(&format!("    {}\n", h.name)),
                }
            }
            out.push_str(&format!("\n  Try `olma search {}` to see more.", &query[..query.len().min(8)]));
            Ok(Some(out))
        }
    }
}
```

- [ ] **Step 4: Build**

```bash
cargo check --lib 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/cache.rs src/suggest.rs
git commit -m "feat(suggest): substring + Levenshtein suggester over cache index"
```

---

## Task 5: Unit tests for did-you-mean

**Files:**
- Create: `tests/unit_did_you_mean.rs`

- [ ] **Step 1: Synthetic index, both code paths**

```rust
use olma::suggest::{Suggester, Suggestions};

fn fixture() -> Suggester {
    Suggester::from_entries(vec![
        ("ripgrep".into(), Some("Fast search tool".into())),
        ("ripgrep-all".into(), Some("ripgrep wrapper".into())),
        ("rg".into(), None),
        ("tree".into(), Some("Print directory tree".into())),
        ("jq".into(), None),
        ("fd".into(), None),
    ])
}

#[test]
fn substring_match_wins_when_present() {
    let s = fixture();
    match s.suggest("ripg", 5) {
        Suggestions::Found(hits) => {
            assert!(hits.iter().any(|h| h.name == "ripgrep"));
            assert!(hits.iter().any(|h| h.name == "ripgrep-all"));
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn substring_ordering_is_shortest_first() {
    let s = fixture();
    match s.suggest("ripg", 5) {
        Suggestions::Found(hits) => {
            assert_eq!(hits[0].name, "ripgrep");
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn levenshtein_distance_two_for_typo() {
    let s = fixture();
    match s.suggest("ripgep", 5) {
        Suggestions::Found(hits) => {
            assert_eq!(hits[0].name, "ripgrep");
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn no_match_returns_none() {
    let s = fixture();
    match s.suggest("xyzqwerty", 5) {
        Suggestions::None => {}
        other => panic!("expected None, got {other:?}"),
    }
}

#[test]
fn case_insensitive_substring() {
    let s = fixture();
    match s.suggest("TREE", 5) {
        Suggestions::Found(hits) => {
            assert!(hits.iter().any(|h| h.name == "tree"));
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn missing_index_path_returns_none_construct() {
    let p = std::path::PathBuf::from("/nonexistent/__index.json");
    assert!(Suggester::from_path(&p).is_none());
}
```

`Suggestions` must derive `Debug`. If it does not yet, add `#[derive(Debug)]` to the enum in `src/suggest.rs`.

- [ ] **Step 2: Run**

```bash
cargo test --test unit_did_you_mean 2>&1 | tail -10
```

Expected: 6 passing.

- [ ] **Step 3: Commit**

```bash
git add tests/unit_did_you_mean.rs
git commit -m "test(suggest): substring + levenshtein + case + missing index"
```

---

## Task 6: Spec §12 error layout

**Files:**
- Modify: `src/error.rs`
- Modify: `src/cli/mod.rs`
- Create: `tests/unit_error_format.rs`

The format is:

```
error: package "ripgep" not found

  Did you mean:
    ripgrep    Fast search tool
    ripgrep-all

  Try `olma search rip` to see more.
```

Line 1 is `error: <msg>`. If the error carries a queryable name and the suggester finds hits, the suggestion block is appended. If not, the footer hint comes from `OlmaError::footer_hint()`.

- [ ] **Step 1: Add error formatting helper**

In `src/error.rs`, add after `impl OlmaError`:

```rust
impl OlmaError {
    pub fn footer_hint(&self) -> Option<&'static str> {
        match self {
            OlmaError::MissingXcodeCLT => Some("Install Xcode CLT and re-run."),
            OlmaError::RootWriteDenied(_) => Some("Re-run the installer or check write permissions on /opt/olma."),
            OlmaError::LockBusy(_) => Some("Wait for the other olma run to finish, or pass --no-wait (V2)."),
            OlmaError::Network(_) => Some("Check your connection and retry. Use --offline to work from cache only."),
            OlmaError::ChecksumMismatch => Some("Run `olma clean` and retry the add."),
            _ => None,
        }
    }

    pub fn formula_name(&self) -> Option<&str> {
        match self {
            OlmaError::FormulaNotFound(n) => Some(n.as_str()),
            OlmaError::BottleNotForPlatform { name, .. } => Some(name.as_str()),
            _ => None,
        }
    }
}

pub fn render_error(err: &OlmaError, config: &crate::config::Config) -> String {
    let mut out = format!("error: {err}\n");
    if let Some(name) = err.formula_name() {
        if let Ok(Some(block)) = crate::suggest::build_suggestion_footer(config, name) {
            out.push_str(&block);
            out.push('\n');
            return out;
        }
    }
    if let Some(hint) = err.footer_hint() {
        out.push('\n');
        out.push_str("  ");
        out.push_str(hint);
        out.push('\n');
    }
    out
}
```

- [ ] **Step 2: Use it in the CLI dispatcher**

In `src/cli/mod.rs`, replace the trailing error arm:

```rust
match result {
    Ok(()) => ExitCode::from(0),
    Err(e) => {
        let config = crate::config::Config::from_env();
        let rendered = crate::error::render_error(&e, &config);
        eprint!("{rendered}");
        e.exit_code()
    }
}
```

- [ ] **Step 3: Tests**

`tests/unit_error_format.rs`:

```rust
use olma::config::Config;
use olma::error::{OlmaError, render_error};

fn temp_config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::Builder::new().prefix("olma-err-").tempdir().unwrap();
    unsafe { std::env::set_var("OLMA_ROOT", tmp.path()); }
    (tmp, Config::from_env())
}

#[test]
fn missing_clt_renders_with_hint() {
    let (_t, c) = temp_config();
    let s = render_error(&OlmaError::MissingXcodeCLT, &c);
    assert!(s.starts_with("error: olma requires"));
    assert!(s.contains("Install Xcode CLT"));
}

#[test]
fn not_found_without_index_emits_degraded_footer() {
    let (_t, c) = temp_config();
    let s = render_error(&OlmaError::FormulaNotFound("ripgep".into()), &c);
    assert!(s.contains("Run `olma update --full`"));
}

#[test]
fn not_found_with_index_emits_suggestion_block() {
    let (_t, c) = temp_config();
    let dir = c.cache_formulae();
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = r#"[{"name":"ripgrep","desc":"Fast search tool"},{"name":"tree"}]"#;
    std::fs::write(dir.join("__index.json"), fixture).unwrap();
    let s = render_error(&OlmaError::FormulaNotFound("ripgep".into()), &c);
    assert!(s.contains("Did you mean"));
    assert!(s.contains("ripgrep"));
}
```

- [ ] **Step 4: Run**

```bash
cargo test --test unit_error_format 2>&1 | tail -10
```

Expected: 3 passing.

- [ ] **Step 5: Commit**

```bash
git add src/error.rs src/cli/mod.rs tests/unit_error_format.rs
git commit -m "feat(error): spec §12 layout with did-you-mean and footer hints"
```

---

## Task 7: `--json` for `list` and `history`

**Files:**
- Modify: `src/cli/list.rs`
- Modify: `src/cli/history.rs`
- Modify: `src/cli/mod.rs`
- Create: `tests/unit_json_emit.rs`

`list` and `history` already exist (Plan 3). We wire the `json` flag through and add an early-return branch that prints a JSON document instead of human lines.

- [ ] **Step 1: `list --json` body**

Replace `src/cli/list.rs`:

```rust
use crate::config::Config;
use crate::error::Result;
use crate::output::Reporter;
use crate::state::{Db, packages::PackageRow};

pub async fn run(json: bool, _reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let rows = PackageRow::list(&db)?;
    if json {
        print_json(&rows);
        return Ok(());
    }
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

fn print_json(rows: &[PackageRow]) {
    let arr: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
        "name": r.name,
        "version": r.version,
        "installed_at": r.installed_at,
        "requested": r.requested,
        "previous_version": r.previous_ver,
    })).collect();
    let doc = serde_json::json!({
        "schema": "olma.list.v1",
        "packages": arr,
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}
```

- [ ] **Step 2: `history --json` body**

Replace `src/cli/history.rs`:

```rust
use crate::config::Config;
use crate::error::Result;
use crate::output::Reporter;
use crate::state::{Db, transactions::TransactionRow};

pub async fn run(limit: i64, json: bool, _reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let db = Db::open(&config)?;
    let rows = TransactionRow::list(&db, limit)?;
    if json {
        print_json(&rows)?;
        return Ok(());
    }
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

fn print_json(rows: &[TransactionRow]) -> Result<()> {
    let mut arr = Vec::new();
    for row in rows {
        let changes = row.parse_changes()?;
        arr.push(serde_json::json!({
            "id": row.id,
            "ts": row.ts,
            "kind": row.kind,
            "reverted": row.reverted,
            "changes": changes.iter().map(|c| serde_json::json!({
                "name": c.name,
                "from": c.from_version,
                "to": c.to_version,
                "requested": c.requested,
            })).collect::<Vec<_>>(),
        }));
    }
    let doc = serde_json::json!({
        "schema": "olma.history.v1",
        "transactions": arr,
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
    Ok(())
}
```

- [ ] **Step 3: Plumb `json` through `cli/mod.rs`**

Update the dispatch arms:

```rust
Cmd::List => list::run(cli.json, reporter.as_ref()).await,
Cmd::History { limit } => history::run(limit, cli.json, reporter.as_ref()).await,
```

- [ ] **Step 4: Stable-shape test**

`tests/unit_json_emit.rs`:

```rust
use olma::config::Config;
use olma::state::{Db, packages::PackageRow};

fn temp_config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::Builder::new().prefix("olma-json-").tempdir().unwrap();
    unsafe { std::env::set_var("OLMA_ROOT", tmp.path()); }
    (tmp, Config::from_env())
}

#[tokio::test(flavor = "current_thread")]
async fn list_json_has_expected_shape() {
    let (_t, c) = temp_config();
    let db = Db::open(&c).unwrap();
    PackageRow::upsert(&db, &PackageRow {
        name: "tree".into(),
        version: "2.3.2".into(),
        installed_at: 100,
        requested: true,
        previous_ver: None,
    }).unwrap();
    let captured = capture_stdout(|| {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let reporter = olma::output::QuietReporter::new();
            olma::cli::list::run(true, &reporter).await.unwrap();
        });
    });
    let v: serde_json::Value = serde_json::from_str(&captured).unwrap();
    assert_eq!(v["schema"], "olma.list.v1");
    assert!(v["packages"].is_array());
    assert_eq!(v["packages"][0]["name"], "tree");
    assert_eq!(v["packages"][0]["version"], "2.3.2");
    assert_eq!(v["packages"][0]["requested"], true);
}

fn capture_stdout<F: FnOnce()>(f: F) -> String {
    use std::io::Read;
    use std::os::unix::io::FromRawFd;
    let saved = unsafe { libc::dup(1) };
    let (r, w) = nix::unistd::pipe().unwrap();
    unsafe { libc::dup2(w, 1); libc::close(w); }
    f();
    unsafe { libc::fsync(1); libc::dup2(saved, 1); libc::close(saved); }
    let mut buf = String::new();
    let mut file = unsafe { std::fs::File::from_raw_fd(r) };
    let _ = file.read_to_string(&mut buf);
    buf
}
```

Capturing stdout from a subprocess-free unit test is fiddly. If wiring `libc`/`nix` adds too much surface area, replace the test with a sibling `print_json` function in `src/cli/list.rs` that returns the `serde_json::Value` instead of printing it, and assert against that directly. Acceptable shape:

```rust
#[cfg(test)]
pub(crate) fn list_json_doc(rows: &[PackageRow]) -> serde_json::Value { /* same body */ }
```

Pick whichever fits without pulling new deps.

- [ ] **Step 5: Run**

```bash
cargo test --test unit_json_emit 2>&1 | tail -10
```

- [ ] **Step 6: Commit**

```bash
git add src/cli/list.rs src/cli/history.rs src/cli/mod.rs tests/unit_json_emit.rs
git commit -m "feat(cli): --json for list and history with stable v1 schemas"
```

---

## Task 8: `--json` for `info`, `outdated`, `search` (Plan 4 dependents)

**Files:**
- Modify: `src/cli/info.rs`     (Plan 4)
- Modify: `src/cli/outdated.rs` (Plan 4)
- Modify: `src/cli/search.rs`   (Plan 4)
- Modify: `src/cli/mod.rs`

> **Depends on Plan 4.** If Plan 4 has not landed, mark the steps below as skipped in the checkbox list and ship Plan 5 without `info` / `outdated` / `search` JSON support; the schema name is reserved so we can add it later.

- [ ] **Step 1: `info --json` body**

Inside the existing `info::run` (Plan 4), branch early when `json`:

```rust
if json {
    let doc = serde_json::json!({
        "schema": "olma.info.v1",
        "name": formula.name,
        "version": formula.version(),
        "desc": formula.desc,
        "homepage": formula.homepage,
        "dependencies": formula.dependencies,
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
    return Ok(());
}
```

- [ ] **Step 2: `outdated --json` body**

`outdated::run` already gathers a list of `OutdatedRow { name, installed, latest }`. After computing it:

```rust
if json {
    let doc = serde_json::json!({
        "schema": "olma.outdated.v1",
        "outdated": rows.iter().map(|r| serde_json::json!({
            "name": r.name,
            "installed": r.installed,
            "latest": r.latest,
        })).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
    return Ok(());
}
```

- [ ] **Step 3: `search --json` body**

```rust
if json {
    let doc = serde_json::json!({
        "schema": "olma.search.v1",
        "query": query,
        "results": hits.iter().map(|h| serde_json::json!({
            "name": h.name,
            "desc": h.desc,
        })).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
    return Ok(());
}
```

- [ ] **Step 4: Plumb `json` through `cli/mod.rs`**

Update the dispatch arms for the three commands (when they exist) to pass `cli.json`.

- [ ] **Step 5: Build**

```bash
cargo check 2>&1 | tail -5
```

- [ ] **Step 6: Commit**

```bash
git add src/cli/info.rs src/cli/outdated.rs src/cli/search.rs src/cli/mod.rs
git commit -m "feat(cli): --json for info, outdated, search with v1 schemas"
```

---

## Task 9: Cache-age footer on `outdated` and `upgrade`

**Files:**
- Modify: `src/cli/outdated.rs` (Plan 4)
- Modify: `src/cli/upgrade.rs`  (Plan 4)

> **Depends on Plan 4.** Skip if those handlers don't exist yet — re-open in the follow-up.

- [ ] **Step 1: Helper**

In `src/cache.rs`, on top of the helpers from Task 4:

```rust
pub const CACHE_AGE_FOOTER_DAYS: u64 = 7;

pub fn maybe_cache_age_footer(config: &crate::config::Config) -> Option<String> {
    let age = formulae_index_age_days(config)?;
    if age > CACHE_AGE_FOOTER_DAYS {
        Some(format!(
            "Cache is {age} days old. Run `olma update` for fresh data."
        ))
    } else {
        None
    }
}
```

- [ ] **Step 2: Call from `outdated::run`**

After printing the human or JSON output, before `Ok(())`:

```rust
if !json {
    if let Some(footer) = crate::cache::maybe_cache_age_footer(&config) {
        reporter.footer(&footer);
    }
}
```

- [ ] **Step 3: Same call from `upgrade::run`**

Same snippet at the end of `upgrade::run`. `upgrade` does not currently support `--json`; the footer always renders when applicable.

- [ ] **Step 4: Build**

```bash
cargo check 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add src/cache.rs src/cli/outdated.rs src/cli/upgrade.rs
git commit -m "feat(cli): cache-age footer on outdated and upgrade (>7 days)"
```

---

## Task 10: First-run PATH hint after `add`

**Files:**
- Modify: `src/cli/add.rs`

When `add` finishes successfully **and** `<root>/bin` is not on the active `PATH`, print a one-time hint. We do not persist the "first run" flag — the hint re-appears whenever the user's PATH is wrong, which is the actually-useful behavior.

- [ ] **Step 1: PATH check helper inside `add.rs`**

At the bottom of `src/cli/add.rs`, add a private helper:

```rust
fn bin_on_path(bin: &std::path::Path) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false; };
    for entry in std::env::split_paths(&path) {
        if entry == bin {
            return true;
        }
    }
    false
}
```

- [ ] **Step 2: Call after the success message**

After the existing `reporter.success(...)` call at the end of `add::run`:

```rust
let bin = config.bin();
if !bin_on_path(&bin) {
    let hint = format!(
        "{} is not on your PATH. Add this line to your shell profile:\n      export PATH=\"{}:$PATH\"",
        bin.display(),
        bin.display(),
    );
    reporter.footer(&hint);
}
```

- [ ] **Step 3: Build + smoke**

```bash
cargo build
PATH=/usr/bin:/bin OLMA_ROOT=/tmp/olma-p5-path ./target/debug/olma add -y tree
```

Expected: success line plus a `…/bin is not on your PATH.` footer.

- [ ] **Step 4: Commit**

```bash
git add src/cli/add.rs
git commit -m "feat(cli): print PATH hint after add when /opt/olma/bin is missing"
```

---

## Task 11: `olma completions <shell>`

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/completions.rs`

Generates and writes the completion script to stdout so the user (or installer) can pipe to a file.

- [ ] **Step 1: Register the subcommand**

In `Cmd`:

```rust
Completions {
    #[arg(value_enum)]
    shell: clap_complete::Shell,
},
```

In dispatch:

```rust
Cmd::Completions { shell } => completions::run(shell).await,
```

And add `pub mod completions;` to the top of `src/cli/mod.rs`.

- [ ] **Step 2: Implement**

`src/cli/completions.rs`:

```rust
use crate::error::Result;
use clap::CommandFactory;
use clap_complete::generate;

pub async fn run(shell: clap_complete::Shell) -> Result<()> {
    let mut cmd = crate::cli::Cli::command();
    let bin = cmd.get_name().to_string();
    generate(shell, &mut cmd, bin, &mut std::io::stdout());
    Ok(())
}
```

- [ ] **Step 3: Smoke test**

```bash
cargo build
./target/debug/olma completions zsh | head -5
./target/debug/olma completions bash | head -5
./target/debug/olma completions fish | head -5
```

Expected: three different shell-syntax dumps.

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/completions.rs
git commit -m "feat(cli): olma completions <shell> generator for zsh/bash/fish"
```

---

## Task 12: `olma self-update`

**Files:**
- Modify: `src/cli/mod.rs`
- Create: `src/cli/self_update.rs`

Fetches the latest `olma-<arch>-apple-darwin.tar.gz` asset from GitHub Releases, verifies its `.sha256` sidecar if present, extracts the binary into a temp dir, and `rename`s it over the running binary. On macOS the running process keeps mapped pages; the next `olma` invocation gets the new file.

The fetch URL is computed from a `const RELEASES_REPO: &str = "dot12x/olma";` so the repo name lives in one place. An `OLMA_SELF_UPDATE_URL` env var overrides the resolved URL entirely (used by tests to point at a fixture).

- [ ] **Step 1: Register the subcommand**

In `Cmd`:

```rust
SelfUpdate,
```

In dispatch:

```rust
Cmd::SelfUpdate => self_update::run(cli.yes, reporter.as_ref()).await,
```

And add `pub mod self_update;` to the top of `src/cli/mod.rs`.

- [ ] **Step 2: Implement**

`src/cli/self_update.rs`:

```rust
use crate::error::{OlmaError, Result};
use crate::output::Reporter;
use crate::platform::Arch;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde::Deserialize;

const RELEASES_REPO: &str = "dot12x/olma";
const RELEASES_API: &str = "https://api.github.com/repos";

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub async fn run(yes: bool, reporter: &dyn Reporter) -> Result<()> {
    let current = current_exe()?;
    reporter.status("Looking for newer olma");

    let release = fetch_latest_release().await?;
    reporter.verbose(&format!("Latest release tag: {}", release.tag_name));

    let arch = match Arch::current() {
        Arch::Arm64 => "aarch64",
        Arch::X64 => "x86_64",
    };
    let archive_name = format!("olma-{arch}-apple-darwin.tar.gz");
    let asset = release.assets.iter().find(|a| a.name == archive_name)
        .ok_or_else(|| OlmaError::Other(format!("no asset named {archive_name} in release {}", release.tag_name)))?;

    let sha_asset = release.assets.iter().find(|a| a.name == format!("{archive_name}.sha256"));

    reporter.status(&format!("Downloading {}", archive_name));
    let tmp = tempfile::Builder::new().prefix("olma-self-update-").tempdir()
        .map_err(|e| OlmaError::Other(format!("tempdir: {e}")))?;
    let archive_path = tmp.path().join(&archive_name);
    download_to(&asset.browser_download_url, &archive_path).await?;

    if let Some(sa) = sha_asset {
        reporter.debug(&format!("Verifying sha256 from {}", sa.browser_download_url));
        let sha_path = tmp.path().join(format!("{archive_name}.sha256"));
        download_to(&sa.browser_download_url, &sha_path).await?;
        let expected_full = std::fs::read_to_string(&sha_path)
            .map_err(OlmaError::Io)?;
        let expected = expected_full.split_whitespace().next()
            .ok_or_else(|| OlmaError::Other("empty .sha256".into()))?
            .to_string();
        let bytes = std::fs::read(&archive_path).map_err(OlmaError::Io)?;
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&bytes);
        let actual = hex::encode(h.finalize());
        if !actual.eq_ignore_ascii_case(&expected) {
            return Err(OlmaError::ChecksumMismatch);
        }
    }

    reporter.status("Extracting new binary");
    let extract_dir = tmp.path().join("extract");
    std::fs::create_dir_all(&extract_dir).map_err(OlmaError::Io)?;
    let archive_path_owned = archive_path.clone();
    let extract_dir_owned = extract_dir.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let f = std::fs::File::open(&archive_path_owned).map_err(OlmaError::Io)?;
        let gz = flate2::read::GzDecoder::new(f);
        let mut tar = tar::Archive::new(gz);
        tar.unpack(&extract_dir_owned).map_err(OlmaError::Io)?;
        Ok(())
    }).await.map_err(|e| OlmaError::Other(format!("extract task: {e}")))??;

    let new_binary = find_olma_in(&extract_dir)
        .ok_or_else(|| OlmaError::Other("no `olma` binary in archive".into()))?;

    if !confirm_swap(&current, &release.tag_name, yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    let staged = current.with_extension("self-update.new");
    std::fs::copy(&new_binary, &staged).map_err(OlmaError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .map_err(OlmaError::Io)?;
    }
    std::fs::rename(&staged, &current).map_err(OlmaError::Io)?;
    reporter.success(&format!("olma updated to {}", release.tag_name));
    Ok(())
}

fn current_exe() -> Result<std::path::PathBuf> {
    std::env::current_exe()
        .map_err(|e| OlmaError::Other(format!("current_exe: {e}")))
}

fn find_olma_in(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_name() == "olma" && entry.path().is_file() {
            return Some(entry.path().to_path_buf());
        }
    }
    None
}

async fn fetch_latest_release() -> Result<Release> {
    let url = std::env::var("OLMA_SELF_UPDATE_URL").unwrap_or_else(|_| {
        format!("{RELEASES_API}/{RELEASES_REPO}/releases/latest")
    });
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.github+json"));
    headers.insert(USER_AGENT, HeaderValue::from_static(concat!("olma/", env!("CARGO_PKG_VERSION"))));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| OlmaError::Network(e.to_string()))?;
    let resp = client.get(&url).headers(headers).send().await
        .map_err(|e| OlmaError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(OlmaError::Network(format!("releases api: {}", resp.status())));
    }
    let body: Release = resp.json().await
        .map_err(|e| OlmaError::Network(format!("parse release json: {e}")))?;
    Ok(body)
}

async fn download_to(url: &str, dest: &std::path::Path) -> Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| OlmaError::Network(e.to_string()))?;
    let resp = client.get(url).send().await
        .map_err(|e| OlmaError::Network(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(OlmaError::Network(format!("download {url}: {}", resp.status())));
    }
    let mut f = tokio::fs::File::create(dest).await.map_err(OlmaError::Io)?;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| OlmaError::Network(e.to_string()))?;
        f.write_all(&bytes).await.map_err(OlmaError::Io)?;
    }
    f.flush().await.map_err(OlmaError::Io)?;
    Ok(())
}

fn confirm_swap(current: &std::path::Path, tag: &str, yes: bool) -> Result<bool> {
    use std::io::IsTerminal;
    if yes { return Ok(true); }
    eprintln!("Replace {} with release {}? [Y/n]", current.display(), tag);
    if !std::io::stdin().is_terminal() {
        return Err(OlmaError::Other("non-interactive: use -y".into()));
    }
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).map_err(OlmaError::Io)?;
    let t = input.trim().to_lowercase();
    Ok(t.is_empty() || t == "y" || t == "yes")
}
```

- [ ] **Step 3: Build**

```bash
cargo check 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs src/cli/self_update.rs
git commit -m "feat(cli): olma self-update with GitHub Releases binary swap"
```

---

## Task 13: Integration test — did-you-mean on `add`

**Files:**
- Create: `tests/integration_polish_did_you_mean.rs`

This test fakes the cache index so it does not depend on `formulae.brew.sh`. `olma add ripgep` must hit the local `FormulaeClient.fetch` first (which 404s); the error path then reads `__index.json` and renders a suggestion containing `ripgrep`.

To avoid an actual network call, the test pre-creates a fixture sandbox where the `OLMA_OFFLINE=1` env var (already honored by Plan 2's `FetchPolicy`) maps to `FetchPolicy::OfflineOnly` and the cache is empty, so the metadata client returns `FormulaNotFound("ripgep")` without leaving the box.

- [ ] **Step 1: Test**

```rust
mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_unknown_name_emits_did_you_mean() {
    let sandbox = Sandbox::new();
    let config = olma::config::Config::from_env();

    let dir = config.cache_formulae();
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = r#"[
        {"name":"ripgrep","desc":"Fast search tool"},
        {"name":"ripgrep-all","desc":"ripgrep wrapper"},
        {"name":"tree","desc":"Print directory tree"}
    ]"#;
    std::fs::write(dir.join("__index.json"), fixture).unwrap();

    let err = olma::cli::add::run(
        &["ripgep".into()],
        olma::metadata::client::FetchPolicy::OfflineOnly,
        true,
        false,
        olma::output::reporter_for(olma::output::Verbosity::Normal, false).as_ref(),
    ).await.unwrap_err();

    let rendered = olma::error::render_error(&err, &config);
    assert!(rendered.contains("ripgep"));
    assert!(rendered.contains("Did you mean"));
    assert!(rendered.contains("ripgrep"));
    assert_eq!(err.exit_code(), std::process::ExitCode::from(65));
    let _ = sandbox;
}
```

If `add::run`'s `OfflineOnly` path does not currently surface `FormulaNotFound` for empty cache, adjust to use `BypassCache` and accept that the test then makes a real network call (mark with the `OLMA_SKIP_NETWORK_TESTS` guard the suite already uses).

- [ ] **Step 2: Run**

```bash
cargo test --test integration_polish_did_you_mean 2>&1 | tail -10
```

- [ ] **Step 3: Commit**

```bash
git add tests/integration_polish_did_you_mean.rs
git commit -m "test: did-you-mean integration for olma add with cached index"
```

---

## Task 14: Self-review checkpoint

- [ ] **Step 1: Full test suite**

```bash
cargo test 2>&1 | grep -E '^test result|Running' | tail -20
```

Expected: every previous and new test passes. Tests gated on `OLMA_SKIP_NETWORK_TESTS` may be skipped.

- [ ] **Step 2: Clippy**

```bash
cargo clippy --all-targets -- -D warnings
```

Expected: clean.

- [ ] **Step 3: End-to-end smoke**

```bash
SBX=/tmp/olma-p5-smoke && rm -rf $SBX
OLMA_ROOT=$SBX ./target/release/olma -v add -y tree
OLMA_ROOT=$SBX ./target/release/olma -vv add -y jq
OLMA_ROOT=$SBX ./target/release/olma list --json | head -20
OLMA_ROOT=$SBX ./target/release/olma history --json | head -20
OLMA_ROOT=$SBX ./target/release/olma add ripgep 2>&1 | head -10
OLMA_ROOT=$SBX ./target/release/olma completions zsh | head -5
PATH=/usr/bin:/bin OLMA_ROOT=$SBX ./target/release/olma add -y fd 2>&1 | tail -3
```

Expected:
- `-v` shows timestamped per-stage log lines.
- `-vv` adds `debug` lines (HTTP, fs ops).
- `list --json` and `history --json` print valid JSON.
- The `ripgep` typo prints `Did you mean: ripgrep` or the degraded `Run \`olma update --full\`` footer.
- `completions zsh` prints a zsh completion script.
- The `PATH=/usr/bin:/bin` run prints the PATH-hint footer.

- [ ] **Step 4: Final commit if needed**

```bash
git status
git add -A
git commit -m "chore: address clippy warnings from Plan 5 self-review"
```

---

## Plan-level verification

After Task 14:

1. `Reporter` has `verbose`, `debug`, and `footer` with sensible defaults.
2. `-q`, `-v`, `-vv`, and `--json` are recognized as global flags on every subcommand.
3. `list --json` and `history --json` print stable v1 schemas. `info`, `outdated`, `search` route through `--json` (when the Plan 4 handlers are in place).
4. `olma add <unknown>` returns exit 65 with a `Did you mean:` block when the index exists, and the degraded footer otherwise.
5. `olma completions <shell>` emits a shell-syntax completion script.
6. `olma self-update` downloads the latest GitHub Release asset, verifies the sha256 sidecar if present, and atomically swaps the binary.
7. The PATH hint footer renders when `<root>/bin` is missing from `PATH` after a successful `add`.
8. Error output follows the spec §12 layout.
9. The cache-age footer prints on `outdated` and `upgrade` when `__index.json` is older than 7 days.

## Out of scope (reminder)

- End-to-end test of `olma self-update` (depends on a hosted release asset). The `OLMA_SELF_UPDATE_URL` override is shipped, and a fixture-based test can be added later once we publish a v0.0.0-test release. Local smoke is enough for V1.
- TOML completion installer integration (the installer is its own work item; this plan ships the generator only).
- `--no-color` opt-out is intentionally excluded (spec §12: olma owns its visual identity).
- A persistent "first-run done" file. The PATH hint is path-aware, not run-aware, which is what the user actually needs.
- Color theming and an alternative reporter style. The single soft style is the V1 identity.
- Async streaming JSON for very large `search` result sets. V1 emits a single JSON blob; large queries should add `--limit` (Plan 4).
- `--json` for `add` / `remove` / `upgrade` / `rollback`. These are mutating commands whose progress is the value; JSON would carry only the final exit, which the exit code already provides.
