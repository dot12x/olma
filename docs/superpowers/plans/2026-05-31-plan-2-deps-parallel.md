# Plan 2 — Dependencies + Parallel Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lift olma from "one zero-dep formula at a time" to "any number of formulas with full dependency resolution, downloaded and processed in parallel." Add an ETag-cached metadata layer, a topological resolver, an install-plan preview with confirmation, and a streaming pipeline whose stages all run concurrently.

**Architecture:** A new `resolver` module owns dependency graph construction and topological ordering. `metadata::client::FormulaeClient` grows ETag-aware caching and the `--refresh`/`--no-cache`/`--offline` policy. The `pipeline` module is reworked: each stage (`download`, `verify`, `extract`, `relocate`, `link`) becomes a Tokio task fed by a bounded channel; concurrency is auto-tuned from `num_cpus` and a hard ceiling. `cli::add` accepts multiple package arguments, prints an apt-style preview, and prompts unless `-y`/non-interactive.

**Tech Stack:** Rust 2024 · `tokio` channels + semaphores · `num_cpus` for auto-tune · existing crates from Plan 1. No new heavy dependencies.

**Depends on:** Plan 1 (`add`, foundation, `metadata`, `pipeline`, `relocator`).

Spec reference: `docs/superpowers/specs/2026-05-31-olma-architecture-design.md` §4 (metadata cache), §5 (resolver), §6 (parallel pipeline).

---

## File structure (touched in this plan)

```
src/
├── resolver/
│   └── mod.rs                NEW   InstallPlan, Resolver, topo sort
├── metadata/
│   └── client.rs             MODIFY  ETag cache + policy flags
├── cache.rs                  NEW   helpers for cache/formulae read/write
├── pipeline/
│   ├── mod.rs                MODIFY  Pipeline runner over Tokio channels
│   ├── download.rs           MODIFY  parallel-safe (no global state)
│   ├── verify.rs             (unchanged)
│   ├── extract.rs            MODIFY  per-package staging dir
│   ├── relocate.rs           (unchanged)
│   └── link.rs               (unchanged)
├── cli/
│   ├── mod.rs                MODIFY  global flags (--yes, --dry-run, --offline, --no-cache, --refresh)
│   └── add.rs                MODIFY  multi-package, preview, deps, parallel
└── platform.rs               MODIFY  add `num_cpus_for_pool` helper

Cargo.toml                    MODIFY  add `num_cpus`

tests/
├── unit_resolver.rs          NEW   graph build, topo sort, cycle detection
├── unit_cache_etag.rs        NEW   ETag write/read/304 path
├── integration_add_with_deps.rs    NEW   add ripgrep (has pcre2)
└── integration_add_many.rs   NEW   add jq tree fd in one shot
```

---

## Task 1: Cache helpers for formula JSON + ETag

**Files:**
- Create: `src/cache.rs`
- Modify: `src/lib.rs` (`pub mod cache;`)

- [ ] **Step 1: Add the module declaration**

In `src/lib.rs`, add `pub mod cache;` alphabetically.

- [ ] **Step 2: Implement read/write helpers**

```rust
use crate::config::Config;
use crate::error::{OlmaError, Result};
use std::path::PathBuf;

pub struct FormulaCacheEntry {
    pub json: String,
    pub etag: Option<String>,
}

pub fn formula_path(config: &Config, name: &str) -> PathBuf {
    config.cache_formulae().join(format!("{name}.json"))
}

pub fn etag_path(config: &Config, name: &str) -> PathBuf {
    config.cache_formulae().join(format!("{name}.etag"))
}

pub fn read_formula(config: &Config, name: &str) -> Result<Option<FormulaCacheEntry>> {
    let jp = formula_path(config, name);
    if !jp.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&jp)?;
    let etag = std::fs::read_to_string(etag_path(config, name)).ok();
    Ok(Some(FormulaCacheEntry { json, etag }))
}

pub fn write_formula(config: &Config, name: &str, json: &str, etag: Option<&str>) -> Result<()> {
    std::fs::create_dir_all(config.cache_formulae())?;
    let jp = formula_path(config, name);
    let tmp = jp.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &jp)?;
    if let Some(e) = etag {
        let ep = etag_path(config, name);
        std::fs::write(ep, e)?;
    }
    Ok(())
}

pub fn cache_age_days(config: &Config, name: &str) -> Result<Option<u64>> {
    let jp = formula_path(config, name);
    if !jp.exists() {
        return Ok(None);
    }
    let meta = std::fs::metadata(&jp)?;
    let modified = meta.modified()?;
    let elapsed = std::time::SystemTime::now()
        .duration_since(modified)
        .map_err(|e| OlmaError::Other(format!("clock error: {e}")))?;
    Ok(Some(elapsed.as_secs() / 86_400))
}
```

- [ ] **Step 3: Build**

Run: `cargo check --lib 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/lib.rs src/cache.rs
git commit -m "feat(cache): formula JSON + ETag read/write helpers"
```

---

## Task 2: ETag-aware `FormulaeClient::fetch`

**Files:**
- Modify: `src/metadata/client.rs`

- [ ] **Step 1: Replace `fetch` with policy-aware fetch**

```rust
use crate::cache;
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::metadata::Formula;
use reqwest::header::{HeaderMap, HeaderValue, ETAG, IF_NONE_MATCH};
use reqwest::StatusCode;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchPolicy {
    CacheFirst,
    ForceRefresh,
    BypassCache,
    OfflineOnly,
}

pub struct FormulaeClient {
    http: reqwest::Client,
    base: String,
    config: Config,
}

impl FormulaeClient {
    pub fn new(config: &Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("olma/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        Ok(Self {
            http,
            base: config.formulae_api_base().to_string(),
            config: config.clone(),
        })
    }

    pub async fn fetch(&self, name: &str, policy: FetchPolicy) -> Result<Formula> {
        if policy == FetchPolicy::OfflineOnly {
            return self.from_cache_only(name);
        }

        let cached = if policy == FetchPolicy::BypassCache {
            None
        } else {
            cache::read_formula(&self.config, name)?
        };

        if policy == FetchPolicy::CacheFirst
            && let Some(entry) = &cached
        {
            return serde_json::from_str(&entry.json)
                .map_err(|e| OlmaError::Network(format!("cached JSON parse failed: {e}")));
        }

        let url = format!("{}/{}.json", self.base, name);
        let mut headers = HeaderMap::new();
        if policy == FetchPolicy::ForceRefresh
            && let Some(entry) = &cached
            && let Some(etag) = &entry.etag
            && let Ok(v) = HeaderValue::from_str(etag)
        {
            headers.insert(IF_NONE_MATCH, v);
        }

        let resp = self.http.get(&url).headers(headers).send().await
            .map_err(|e| OlmaError::Network(e.to_string()))?;

        match resp.status() {
            StatusCode::NOT_MODIFIED => {
                let entry = cached.ok_or_else(|| OlmaError::Network("304 without cache".into()))?;
                serde_json::from_str(&entry.json)
                    .map_err(|e| OlmaError::Network(format!("cached JSON parse failed: {e}")))
            }
            StatusCode::OK => {
                let etag = resp.headers().get(ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let body = resp.text().await
                    .map_err(|e| OlmaError::Network(e.to_string()))?;
                if policy != FetchPolicy::BypassCache {
                    cache::write_formula(&self.config, name, &body, etag.as_deref())?;
                }
                serde_json::from_str(&body)
                    .map_err(|e| OlmaError::Network(format!("formula JSON parse failed: {e}")))
            }
            StatusCode::NOT_FOUND => Err(OlmaError::FormulaNotFound(name.to_string())),
            s => Err(OlmaError::Network(format!("formulae.brew.sh returned {s}"))),
        }
    }

    fn from_cache_only(&self, name: &str) -> Result<Formula> {
        let entry = cache::read_formula(&self.config, name)?
            .ok_or_else(|| OlmaError::Network(format!("offline: no cached metadata for {name}")))?;
        serde_json::from_str(&entry.json)
            .map_err(|e| OlmaError::Network(format!("cached JSON parse failed: {e}")))
    }
}
```

- [ ] **Step 2: Build**

Run: `cargo check --lib 2>&1 | tail -5`
Expected: errors at the existing `client.fetch(name).await` call sites (only `cli/add.rs` for now). Fix by passing `FetchPolicy::CacheFirst` until Task 5 wires the global flag through.

```rust
// in cli/add.rs, temporarily:
let formula = client.fetch(name, crate::metadata::client::FetchPolicy::CacheFirst).await?;
```

- [ ] **Step 3: Write the ETag round-trip unit test**

`tests/unit_cache_etag.rs`:

```rust
use olma::cache;
use olma::config::Config;

#[test]
fn etag_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("OLMA_ROOT", tmp.path()); }
    let config = Config::from_env();

    let body = r#"{"name":"x","versions":{"stable":"1.0"},"bottle":{"stable":{"rebuild":null,"root_url":"r","files":{}}}}"#;
    cache::write_formula(&config, "x", body, Some("\"abc123\"")).unwrap();

    let entry = cache::read_formula(&config, "x").unwrap().unwrap();
    assert_eq!(entry.json, body);
    assert_eq!(entry.etag.as_deref(), Some("\"abc123\""));
}
```

- [ ] **Step 4: Run**

Run: `cargo test --test unit_cache_etag 2>&1 | tail -10`
Expected: 1 pass.

- [ ] **Step 5: Commit**

```bash
git add src/metadata/client.rs src/cli/add.rs tests/unit_cache_etag.rs
git commit -m "feat(metadata): ETag-aware fetch with cache policy enum"
```

---

## Task 3: `resolver` module — `InstallPlan` and graph

**Files:**
- Create: `src/resolver/mod.rs`
- Modify: `src/lib.rs` (`pub mod resolver;`)

- [ ] **Step 1: Register the module**

Add `pub mod resolver;` alphabetically in `src/lib.rs`.

- [ ] **Step 2: Implement the resolver**

```rust
use crate::error::{OlmaError, Result};
use crate::metadata::{Formula, client::{FetchPolicy, FormulaeClient}};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub ordered: Vec<Formula>,
    pub requested: HashSet<String>,
}

impl InstallPlan {
    pub fn total_bottle_size(&self, tag: &str) -> u64 {
        self.ordered.iter()
            .filter_map(|f| f.bottle_for_tag(tag).map(|_| 0u64))
            .sum()
    }
}

pub async fn resolve(
    targets: &[String],
    client: &FormulaeClient,
    policy: FetchPolicy,
) -> Result<InstallPlan> {
    let requested: HashSet<String> = targets.iter().cloned().collect();
    let mut graph: HashMap<String, Formula> = HashMap::new();
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    let mut queue: VecDeque<String> = targets.iter().cloned().collect();

    while let Some(name) = queue.pop_front() {
        if graph.contains_key(&name) {
            continue;
        }
        let formula = client.fetch(&name, policy).await?;
        let dep_names = formula.dependencies.clone();
        for d in &dep_names {
            if !graph.contains_key(d) {
                queue.push_back(d.clone());
            }
        }
        deps.insert(name.clone(), dep_names);
        graph.insert(name, formula);
    }

    let ordered = topo_sort(&deps)?;
    let ordered: Vec<Formula> = ordered.into_iter()
        .filter_map(|n| graph.remove(&n))
        .collect();

    Ok(InstallPlan { ordered, requested })
}

fn topo_sort(deps: &HashMap<String, Vec<String>>) -> Result<Vec<String>> {
    let mut indegree: HashMap<&str, usize> = HashMap::new();
    for node in deps.keys() {
        indegree.entry(node).or_insert(0);
    }
    for (_node, ds) in deps {
        for d in ds {
            *indegree.entry(d).or_insert(0) += 1;
        }
    }

    let mut ready: VecDeque<&str> = indegree.iter()
        .filter(|(_, &d)| d == 0)
        .map(|(n, _)| *n)
        .collect();

    let mut out: Vec<String> = Vec::new();
    while let Some(node) = ready.pop_front() {
        out.push(node.to_string());
        if let Some(children) = deps.get(node) {
            for c in children {
                if let Some(d) = indegree.get_mut(c.as_str()) {
                    *d -= 1;
                    if *d == 0 {
                        ready.push_back(c.as_str());
                    }
                }
            }
        }
    }

    if out.len() != indegree.len() {
        return Err(OlmaError::Other("dependency cycle detected".into()));
    }
    out.reverse();
    Ok(out)
}
```

The reverse at the end gives install order (deps first, requested package last).

- [ ] **Step 3: Build**

Run: `cargo check --lib 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/lib.rs src/resolver/mod.rs
git commit -m "feat(resolver): build dep graph and topo-sort install order"
```

---

## Task 4: Resolver unit tests

**Files:**
- Create: `tests/unit_resolver.rs`

The resolver does real network fetches in production. The unit tests build the topological order from a hand-rolled graph; testing the live HTTP path belongs in the integration test.

- [ ] **Step 1: Tests against a fake graph**

Expose a thin helper to make `topo_sort` testable. Add to `src/resolver/mod.rs`:

```rust
pub fn topo_sort_for_test(deps: &std::collections::HashMap<String, Vec<String>>) -> Result<Vec<String>> {
    let inner: std::collections::HashMap<String, Vec<String>> = deps.clone();
    topo_sort(&inner)
}
```

(Keep the original `topo_sort` private.)

`tests/unit_resolver.rs`:

```rust
use std::collections::HashMap;
use olma::resolver::topo_sort_for_test;

fn graph(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
    pairs.iter()
        .map(|(k, vs)| (k.to_string(), vs.iter().map(|s| s.to_string()).collect()))
        .collect()
}

#[test]
fn linear_chain_orders_deps_first() {
    let g = graph(&[("a", &["b"]), ("b", &["c"]), ("c", &[])]);
    let order = topo_sort_for_test(&g).unwrap();
    let pos = |x: &str| order.iter().position(|s| s == x).unwrap();
    assert!(pos("c") < pos("b"));
    assert!(pos("b") < pos("a"));
}

#[test]
fn diamond_dependency_appears_once() {
    let g = graph(&[
        ("a", &["b", "c"]),
        ("b", &["d"]),
        ("c", &["d"]),
        ("d", &[]),
    ]);
    let order = topo_sort_for_test(&g).unwrap();
    assert_eq!(order.iter().filter(|s| s.as_str() == "d").count(), 1);
    let pos = |x: &str| order.iter().position(|s| s == x).unwrap();
    assert!(pos("d") < pos("b"));
    assert!(pos("d") < pos("c"));
    assert!(pos("b") < pos("a"));
    assert!(pos("c") < pos("a"));
}

#[test]
fn cycle_returns_error() {
    let g = graph(&[("a", &["b"]), ("b", &["a"])]);
    assert!(topo_sort_for_test(&g).is_err());
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test unit_resolver 2>&1 | tail -10`
Expected: 3 passing.

- [ ] **Step 3: Commit**

```bash
git add src/resolver/mod.rs tests/unit_resolver.rs
git commit -m "test(resolver): graph topo sort with linear / diamond / cycle"
```

---

## Task 5: Global flags + `FetchPolicy` derivation

**Files:**
- Modify: `src/cli/mod.rs`

- [ ] **Step 1: Add the top-level flags**

```rust
#[derive(Parser)]
#[command(name = "olma", version, about = "macOS package manager")]
pub struct Cli {
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
    #[arg(long, global = true)]
    pub dry_run: bool,
    #[arg(long, global = true)]
    pub offline: bool,
    #[arg(long, global = true)]
    pub no_cache: bool,
    #[arg(long, global = true)]
    pub refresh: bool,

    #[command(subcommand)]
    cmd: Cmd,
}
```

Helper:

```rust
use crate::metadata::client::FetchPolicy;

impl Cli {
    pub fn fetch_policy(&self) -> Result<FetchPolicy, &'static str> {
        match (self.offline, self.no_cache, self.refresh) {
            (true, true, _) => Err("--offline and --no-cache are mutually exclusive"),
            (true, _, true) => Err("--offline and --refresh are mutually exclusive"),
            (true, _, _) => Ok(FetchPolicy::OfflineOnly),
            (_, true, _) => Ok(FetchPolicy::BypassCache),
            (_, _, true) => Ok(FetchPolicy::ForceRefresh),
            _ => Ok(FetchPolicy::CacheFirst),
        }
    }
}
```

Pass `Cli` (or just the policy + `yes`/`dry_run` flags) into `add::run`.

- [ ] **Step 2: Build**

Run: `cargo build 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/cli/mod.rs
git commit -m "feat(cli): add -y, --dry-run, --offline, --no-cache, --refresh"
```

---

## Task 6: Parallel pipeline runner

**Files:**
- Modify: `src/pipeline/mod.rs`
- Modify: `Cargo.toml` (add `num_cpus`)

Each stage runs as its own task. Items move stage-to-stage through bounded channels. Concurrency is auto-tuned: `min(8, num_cpus * 2)` for download, `num_cpus` for CPU stages, 1 for link.

- [ ] **Step 1: Add `num_cpus`**

In `[dependencies]`:

```toml
num_cpus = "1.16"
```

- [ ] **Step 2: Implement the runner**

```rust
pub mod download;
pub mod extract;
pub mod link;
pub mod relocate;
pub mod verify;

use crate::config::Config;
use crate::error::Result;
use crate::metadata::Formula;
use crate::metadata::ghcr::GhcrClient;
use crate::output::Reporter;
use crate::resolver::InstallPlan;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;

pub async fn run(
    plan: InstallPlan,
    bottle_tag: String,
    config: Arc<Config>,
    reporter: Arc<dyn Reporter>,
) -> Result<()> {
    let download_n = std::cmp::min(8, num_cpus::get() * 2);
    let cpu_n = num_cpus::get();

    let dl_sem = Arc::new(Semaphore::new(download_n));
    let cpu_sem = Arc::new(Semaphore::new(cpu_n));

    let (dl_tx, mut dl_rx) = mpsc::channel::<download::Downloaded>(64);
    let ghcr = Arc::new(GhcrClient::new()?);

    let total = plan.ordered.len();
    reporter.status(&format!("Resolving {total} packages"));

    let download_handles: Vec<_> = plan.ordered.iter().cloned().map(|formula| {
        let tag = bottle_tag.clone();
        let config = config.clone();
        let ghcr = ghcr.clone();
        let dl_sem = dl_sem.clone();
        let dl_tx = dl_tx.clone();
        let reporter = reporter.clone();
        tokio::spawn(async move {
            let _permit = dl_sem.acquire_owned().await.unwrap();
            let bottle = match formula.bottle_for_tag(&tag) {
                Some(b) => b,
                None => return Err(crate::error::OlmaError::BottleNotForPlatform {
                    name: formula.name.clone(), platform: tag.clone(),
                }),
            };
            reporter.status(&format!("Downloading {} {}", formula.name, formula.version()));
            let dl = download::download(&ghcr, bottle, &config).await?;
            verify::verify_sha256(&bottle.sha256, &dl.sha256_hex)?;
            let _ = dl_tx.send(download::with_formula(dl, formula)).await;
            Ok::<(), crate::error::OlmaError>(())
        })
    }).collect();
    drop(dl_tx);

    let mut staged = Vec::new();
    while let Some(item) = dl_rx.recv().await {
        let cpu_sem = cpu_sem.clone();
        let config = config.clone();
        let reporter = reporter.clone();
        let handle = tokio::spawn(async move {
            let _permit = cpu_sem.acquire_owned().await.unwrap();
            extract::extract_and_relocate(&item, &config).await?;
            Ok::<download::DownloadedWithFormula, crate::error::OlmaError>(item)
        });
        staged.push(handle);
        let _ = reporter;
    }

    let mut linked: Vec<download::DownloadedWithFormula> = Vec::new();
    for h in staged {
        let item = h.await.map_err(|e| crate::error::OlmaError::Other(format!("stage task: {e}")))??;
        linked.push(item);
    }

    let lock_guard = crate::fs_lock::WriteLock::acquire_blocking(&config.lock_file()).await?;
    for item in &linked {
        let dest = config.package_dir(&item.formula.name, item.formula.version());
        link::link_bin(&config, &dest)?;
    }
    drop(lock_guard);

    for h in download_handles {
        h.await.map_err(|e| crate::error::OlmaError::Other(format!("dl task: {e}")))??;
    }

    let n = linked.len();
    let requested: Vec<&str> = plan.requested.iter().map(|s| s.as_str()).collect();
    reporter.success(&format!("Installed {} packages ({})", n, requested.join(", ")));
    Ok(())
}
```

(The download/extract/relocate stages keep their existing implementations; only `pipeline::mod.rs` orchestrates them in parallel.)

- [ ] **Step 3: Add `DownloadedWithFormula` and `extract_and_relocate`**

In `src/pipeline/download.rs`:

```rust
pub struct DownloadedWithFormula {
    pub dl: Downloaded,
    pub formula: crate::metadata::Formula,
}

pub fn with_formula(dl: Downloaded, formula: crate::metadata::Formula) -> DownloadedWithFormula {
    DownloadedWithFormula { dl, formula }
}
```

In `src/pipeline/extract.rs`, add a single function that does extract → flatten → relocate for one package:

```rust
pub async fn extract_and_relocate(
    item: &crate::pipeline::download::DownloadedWithFormula,
    config: &crate::config::Config,
) -> crate::error::Result<()> {
    use crate::pipeline::relocate;
    let staging = config.cache().join("staging")
        .join(format!("{}-{}", item.formula.name, item.formula.version()));
    if staging.exists() { std::fs::remove_dir_all(&staging)?; }
    std::fs::create_dir_all(&staging)?;
    extract(&item.dl.path, &staging).await?;
    let inner = staging.join(&item.formula.name).join(item.formula.version());
    let dest = config.package_dir(&item.formula.name, item.formula.version());
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(&inner, &dest)?;
    let _ = std::fs::remove_dir_all(&staging);

    let new_cellar = dest.to_string_lossy().to_string();
    let new_root = config.root.to_string_lossy().to_string();
    let swaps: Vec<(String, String)> = vec![
        (format!("/opt/homebrew/Cellar/{}/{}", item.formula.name, item.formula.version()), new_cellar.clone()),
        (format!("/usr/local/Cellar/{}/{}", item.formula.name, item.formula.version()), new_cellar.clone()),
        ("/opt/homebrew".into(), new_root.clone()),
        ("/usr/local".into(), new_root.clone()),
    ];
    relocate::relocate_tree(&dest, swaps).await?;
    Ok(())
}
```

- [ ] **Step 4: Build**

Run: `cargo build 2>&1 | tail -10`
Expected: clean. Existing serial code from `cli::add::run` will be replaced in Task 7.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/pipeline
git commit -m "feat(pipeline): channel-based parallel runner with auto-tuned concurrency"
```

---

## Task 7: Multi-package `add` with plan preview

**Files:**
- Modify: `src/cli/add.rs`

- [ ] **Step 1: Accept multiple names + flags**

```rust
use std::sync::Arc;
use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::Reporter;
use crate::platform::current_bottle_tag;
use crate::relocator::macho;
use crate::resolver;
use crate::pipeline;

pub async fn run(
    names: &[String],
    policy: FetchPolicy,
    yes: bool,
    dry_run: bool,
    reporter: &dyn Reporter,
) -> Result<()> {
    macho::check_clt_available()?;

    let config = Arc::new(Config::from_env());
    config.ensure_layout()?;

    reporter.status("Resolving dependencies");
    let client = FormulaeClient::new(&config)?;
    let plan = resolver::resolve(names, &client, policy).await?;

    let tag = current_bottle_tag().await?;

    print_plan(&plan, &tag, reporter);
    if dry_run {
        reporter.success("dry run — nothing installed");
        return Ok(());
    }
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    pipeline::run(plan, tag, config, Arc::from(crate::output::default_reporter())).await?;
    Ok(())
}

fn print_plan(plan: &resolver::InstallPlan, tag: &str, reporter: &dyn Reporter) {
    let n = plan.ordered.len();
    reporter.status(&format!("Will install {n} packages"));
    for f in &plan.ordered {
        let _ = f.bottle_for_tag(tag);
        reporter.status(&format!("  {} {}", f.name, f.version()));
    }
}

fn confirm(yes: bool) -> Result<bool> {
    use std::io::IsTerminal;
    if yes { return Ok(true); }
    if !std::io::stdin().is_terminal() {
        return Err(OlmaError::Other("non-interactive: use -y".into()));
    }
    eprint!("Proceed? [Y/n] ");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).map_err(OlmaError::Io)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed.is_empty() || trimmed == "y" || trimmed == "yes")
}
```

- [ ] **Step 2: Update `cli/mod.rs` to pass new args**

```rust
Add {
    #[arg(required = true)]
    names: Vec<String>,
},
// ...
Cmd::Add { names } => {
    let policy = cli.fetch_policy()
        .map_err(|e| crate::error::OlmaError::Other(e.into()))?;
    add::run(&names, policy, cli.yes, cli.dry_run, reporter.as_ref()).await
},
```

- [ ] **Step 3: Build**

Run: `cargo build 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 4: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p2 ./target/debug/olma add --dry-run tree jq
```

Expected: shows a plan with both packages and exits 0 without installing.

- [ ] **Step 5: Commit**

```bash
git add src/cli
git commit -m "feat(cli): multi-package add with plan preview and confirmation"
```

---

## Task 8: Integration test — add with deps

**Files:**
- Create: `tests/integration_add_with_deps.rs`

- [ ] **Step 1: Write the test**

```rust
mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_ripgrep_pulls_pcre2() {
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok() {
        eprintln!("skipped");
        return;
    }

    let sandbox = Sandbox::new();
    let reporter = olma::output::default_reporter();
    olma::cli::add::run(
        &["ripgrep".into()],
        olma::metadata::client::FetchPolicy::CacheFirst,
        true,
        false,
        reporter.as_ref(),
    ).await.expect("add ripgrep");

    let rg = sandbox.bin().join("rg");
    assert!(rg.exists(), "rg should be linked");

    let out = std::process::Command::new(&rg).arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ripgrep"), "got: {stdout}");
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test integration_add_with_deps -- --nocapture 2>&1 | tail -30`
Expected: pass; ripgrep + pcre2 install in parallel.

- [ ] **Step 3: Commit**

```bash
git add tests/integration_add_with_deps.rs
git commit -m "test: integration test for add with one dep (ripgrep + pcre2)"
```

---

## Task 9: Integration test — add many at once

**Files:**
- Create: `tests/integration_add_many.rs`

- [ ] **Step 1: Write the test**

```rust
mod helpers;
use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_three_packages_in_parallel() {
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok() {
        eprintln!("skipped");
        return;
    }
    let sandbox = Sandbox::new();
    let reporter = olma::output::default_reporter();
    olma::cli::add::run(
        &["tree".into(), "jq".into(), "fd".into()],
        olma::metadata::client::FetchPolicy::CacheFirst,
        true,
        false,
        reporter.as_ref(),
    ).await.expect("add three");

    for exe in ["tree", "jq", "fd"] {
        let p = sandbox.bin().join(exe);
        assert!(p.exists(), "expected {} linked", exe);
        let out = std::process::Command::new(&p).arg("--version").output().unwrap();
        assert!(out.status.success(), "{} --version failed", exe);
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test --test integration_add_many -- --nocapture 2>&1 | tail -30`
Expected: pass; three downloads in parallel, all three binaries work.

- [ ] **Step 3: Commit**

```bash
git add tests/integration_add_many.rs
git commit -m "test: integration test for parallel multi-package add"
```

---

## Task 10: Self-review checkpoint

- [ ] **Step 1: Full suite**

Run: `cargo test 2>&1 | tail -30`
Expected: every test passes (existing Plan 1 tests + Plan 2 tests).

- [ ] **Step 2: Clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 3: Smoke test**

```bash
OLMA_ROOT=/tmp/olma-p2 ./target/release/olma add ripgrep gh
OLMA_ROOT=/tmp/olma-p2 ./target/release/olma add --refresh tree
OLMA_ROOT=/tmp/olma-p2 ./target/release/olma add --offline jq    # uses cache
OLMA_ROOT=/tmp/olma-p2 ./target/release/olma add --dry-run htop
```

- [ ] **Step 4: Final commit if needed**

```bash
git status
git add -A
git commit -m "chore: address clippy warnings from Plan 2 self-review"
```

---

## Plan-level verification

After Task 10:

1. `olma add <a> <b> <c>` resolves dependencies and installs everything in parallel.
2. `--offline`, `--no-cache`, `--refresh` drive metadata behavior; `--dry-run` previews without installing; `-y` skips the prompt.
3. ETag conditional GETs cut server round-trips when nothing changed.
4. Cycles in the dep graph are reported clearly; missing bottles error with the platform tag.
5. The integration test confirms `ripgrep` (with `pcre2`) and `tree`/`jq`/`fd` all work end-to-end.

## Out of scope (reminder)

- State.db, history, rollback → Plan 3.
- `remove`, `info`, `search`, `outdated`, `clean`, `default` → Plan 4.
- Polish (verbose, JSON, did-you-mean, completions, self-update) → Plan 5.
- Services → Plan 6.
