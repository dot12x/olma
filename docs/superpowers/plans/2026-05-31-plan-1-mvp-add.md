# Plan 1 — MVP `olma add` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the smallest end-to-end flow of `olma add <name>` for a zero-dependency Homebrew formula, proving the foundation, metadata fetch, bottle download, relocation, and PATH linking work together against real `formulae.brew.sh` and `ghcr.io`.

**Architecture:** Single Rust crate with Tokio async runtime. Sequential pipeline (parallelism added in Plan 2). One CLI command (`add`). No state.db, no dep resolver, no rollback — those come in later plans. The target test formula is `tree` (no runtime deps, small Mach-O, simple `bin/` layout).

**Tech Stack:** Rust 2024 edition · `clap` (derive) · `tokio` (rt-multi-thread, fs, macros) · `reqwest` (rustls, gzip, http2) · `serde`/`serde_json` · `sha2` · `tar` · `flate2` · `fs2` · `walkdir` · `memchr` · `anyhow` · `dirs`. Shell-outs to `install_name_tool` and `codesign` (Xcode CLT).

Spec reference: `docs/superpowers/specs/2026-05-31-olma-architecture-design.md`

---

## File structure (created in this plan)

```
src/
├── main.rs                    [modify] clap entry, Tokio bootstrap
├── lib.rs                     [create] re-export modules so tests can use them
├── error.rs                   [create] OlmaError enum + exit code mapping
├── platform.rs                [create] arch + macOS codename detection
├── config.rs                  [create] paths under /opt/olma (overridable via OLMA_ROOT)
├── fs_lock.rs                 [create] fs2 wrapper, blocking wait
├── output/
│   ├── mod.rs                 [create] Reporter trait + factory
│   └── soft.rs                [create] minimal braille spinner + status line
├── cli/
│   ├── mod.rs                 [create] Cli struct, Command enum (just Add for now)
│   └── add.rs                 [create] orchestrates add pipeline
├── metadata/
│   ├── mod.rs                 [create] Formula struct
│   ├── client.rs              [create] formulae.brew.sh GET
│   └── ghcr.rs                [create] GHCR token + blob fetch
├── pipeline/
│   ├── mod.rs                 [create] Pipeline runner (sequential in this plan)
│   ├── download.rs            [create] streaming write with sha256 hasher
│   ├── verify.rs              [create] sha256 compare
│   ├── extract.rs             [create] tar.gz to packages/<name>/<version>/
│   ├── relocate.rs            [create] walk dir, dispatch by file kind
│   └── link.rs                [create] symlinks into bin/
└── relocator/
    ├── mod.rs                 [create] re-exports
    ├── classify.rs            [create] file kind detection (MachO/Text/Symlink/Skip)
    ├── macho.rs               [create] install_name_tool + codesign shellout
    └── text.rs                [create] memchr-based prefix replace

Cargo.toml                     [modify] add deps

tests/
├── unit_classify.rs           [create] file kind detection
├── unit_text_relocate.rs      [create] prefix replace
├── unit_platform.rs           [create] codename mapping
├── helpers/mod.rs             [create] sandbox helpers (OLMA_ROOT)
└── integration_add_tree.rs    [create] end-to-end against real network
```

Tests live as separate `tests/*.rs` files (Cargo treats each as an integration test crate). Pure-unit tests for module internals live `#[cfg(test)] mod tests` inside the module file.

---

## Task 1: Cargo dependencies and crate skeleton

**Files:**
- Modify: `Cargo.toml`
- Create: `src/lib.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Replace `Cargo.toml` with the V1 dependency set**

Open `Cargo.toml` and replace its contents with:

```toml
[package]
name = "olma"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
license = "MIT OR Apache-2.0"

[lib]
name = "olma"
path = "src/lib.rs"

[[bin]]
name = "olma"
path = "src/main.rs"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
dirs = "5"
flate2 = "1"
fs2 = "0.4"
memchr = "2"
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "gzip", "http2", "json", "stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
tar = "0.4"
tokio = { version = "1", features = ["rt-multi-thread", "fs", "macros", "io-util", "process", "signal"] }
walkdir = "2"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Create `src/lib.rs` exposing the module tree**

```rust
pub mod cli;
pub mod config;
pub mod error;
pub mod fs_lock;
pub mod metadata;
pub mod output;
pub mod pipeline;
pub mod platform;
pub mod relocator;
```

- [ ] **Step 3: Replace `src/main.rs` with the Tokio bootstrap**

```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: failed to start runtime: {e}");
            return ExitCode::from(1);
        }
    };
    rt.block_on(olma::cli::run())
}
```

- [ ] **Step 4: Verify the crate compiles (it will fail until later tasks land — that's fine, just confirm cargo accepts the manifest)**

Run: `cargo check 2>&1 | head -40`
Expected: errors about missing modules (`cli`, `config`, etc.) — this is expected; we land them in subsequent tasks. The manifest itself should parse.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/lib.rs src/main.rs
git commit -m "feat: introduce Cargo manifest and crate skeleton for MVP"
```

---

## Task 2: `error.rs` — `OlmaError` enum and exit code mapping

**Files:**
- Create: `src/error.rs`

- [ ] **Step 1: Create the error module**

```rust
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, thiserror::Error)]
pub enum OlmaError {
    #[error("package \"{0}\" not found")]
    FormulaNotFound(String),

    #[error("no macOS bottle for {name} on {platform}")]
    BottleNotForPlatform { name: String, platform: String },

    #[error("network error: {0}")]
    Network(String),

    #[error("bottle checksum mismatch — possible corruption or tampering")]
    ChecksumMismatch,

    #[error("cannot write {0}. Run installer or check permissions.")]
    RootWriteDenied(PathBuf),

    #[error("olma requires Xcode Command Line Tools. Run: xcode-select --install")]
    MissingXcodeCLT,

    #[error("{0} is locked by another olma process")]
    LockBusy(PathBuf),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl OlmaError {
    pub fn exit_code(&self) -> ExitCode {
        match self {
            OlmaError::FormulaNotFound(_) => ExitCode::from(65),
            OlmaError::BottleNotForPlatform { .. } => ExitCode::from(66),
            OlmaError::Network(_) => ExitCode::from(74),
            OlmaError::ChecksumMismatch => ExitCode::from(1),
            OlmaError::RootWriteDenied(_) => ExitCode::from(78),
            OlmaError::MissingXcodeCLT => ExitCode::from(78),
            OlmaError::LockBusy(_) => ExitCode::from(1),
            OlmaError::Io(_) => ExitCode::from(74),
            OlmaError::Other(_) => ExitCode::from(1),
        }
    }
}

pub type Result<T> = std::result::Result<T, OlmaError>;
```

Note: `thiserror` is not yet in `Cargo.toml`. Add it now.

- [ ] **Step 2: Add `thiserror` to `Cargo.toml`**

In `[dependencies]` add:
```toml
thiserror = "1"
```

- [ ] **Step 3: Verify it builds**

Run: `cargo check --lib 2>&1 | grep -E '^error|warning' | head -20`
Expected: only errors about missing sibling modules (cli, etc.), no errors inside `error.rs`.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml src/error.rs
git commit -m "feat: add OlmaError enum with exit code mapping"
```

---

## Task 3: `platform.rs` — arch + macOS codename detection

**Files:**
- Create: `src/platform.rs`
- Create: `tests/unit_platform.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/unit_platform.rs`:

```rust
use olma::platform::{Arch, macos_codename_for_product_version};

#[test]
fn arch_current_returns_known_value() {
    let a = Arch::current();
    assert!(matches!(a, Arch::Arm64 | Arch::X64));
}

#[test]
fn arch_as_bottle_str_returns_homebrew_names() {
    assert_eq!(Arch::Arm64.as_bottle_str(), "arm64");
    assert_eq!(Arch::X64.as_bottle_str(), "x86_64");
}

#[test]
fn codename_known_versions() {
    assert_eq!(macos_codename_for_product_version("13.0"), Some("ventura"));
    assert_eq!(macos_codename_for_product_version("13.6.1"), Some("ventura"));
    assert_eq!(macos_codename_for_product_version("14.5"), Some("sonoma"));
    assert_eq!(macos_codename_for_product_version("15.0"), Some("sequoia"));
    assert_eq!(macos_codename_for_product_version("26.0"), Some("tahoe"));
}

#[test]
fn codename_unknown_returns_none() {
    assert_eq!(macos_codename_for_product_version("99.0"), None);
    assert_eq!(macos_codename_for_product_version("not a version"), None);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test unit_platform 2>&1 | tail -10`
Expected: compile errors — `platform` module does not exist yet.

- [ ] **Step 3: Implement `src/platform.rs`**

```rust
use crate::error::{OlmaError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    Arm64,
    X64,
}

impl Arch {
    pub fn current() -> Self {
        if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else {
            Arch::X64
        }
    }

    pub fn as_bottle_str(&self) -> &'static str {
        match self {
            Arch::Arm64 => "arm64",
            Arch::X64 => "x86_64",
        }
    }
}

pub fn macos_codename_for_product_version(product_version: &str) -> Option<&'static str> {
    let major: u32 = product_version.split('.').next()?.parse().ok()?;
    match major {
        11 => Some("bigsur"),
        12 => Some("monterey"),
        13 => Some("ventura"),
        14 => Some("sonoma"),
        15 => Some("sequoia"),
        26 => Some("tahoe"),
        _ => None,
    }
}

pub async fn detect_macos_codename() -> Result<String> {
    let out = tokio::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .await
        .map_err(|e| OlmaError::Other(format!("failed to run sw_vers: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other("sw_vers failed".into()));
    }
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    macos_codename_for_product_version(&version)
        .map(|s| s.to_string())
        .ok_or_else(|| OlmaError::Other(format!("unsupported macOS version: {version}")))
}

/// Returns the Homebrew bottle tag for this host, e.g. "arm64_sonoma".
pub async fn current_bottle_tag() -> Result<String> {
    Ok(format!("{}_{}", Arch::current().as_bottle_str(), detect_macos_codename().await?))
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test unit_platform 2>&1 | tail -10`
Expected: 4 passing tests.

- [ ] **Step 5: Commit**

```bash
git add src/platform.rs tests/unit_platform.rs
git commit -m "feat: detect arch and macOS codename for bottle tag"
```

---

## Task 4: `config.rs` — paths under `/opt/olma` with `OLMA_ROOT` override

**Files:**
- Create: `src/config.rs`

- [ ] **Step 1: Implement the config module**

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    /// Reads `OLMA_ROOT` env var if set (used for sandbox tests),
    /// otherwise returns `/opt/olma`.
    pub fn from_env() -> Self {
        let root = std::env::var_os("OLMA_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/opt/olma"));
        Paths { root }
    }

    pub fn bin(&self) -> PathBuf { self.root.join("bin") }
    pub fn packages(&self) -> PathBuf { self.root.join("packages") }
    pub fn cache(&self) -> PathBuf { self.root.join("cache") }
    pub fn cache_bottles(&self) -> PathBuf { self.cache().join("bottles") }
    pub fn cache_formulae(&self) -> PathBuf { self.cache().join("formulae") }
    pub fn lock_file(&self) -> PathBuf { self.root.join(".lock") }

    pub fn package_dir(&self, name: &str, version: &str) -> PathBuf {
        self.packages().join(name).join(version)
    }

    /// Ensures the directory layout exists. Creates dirs as needed.
    pub fn ensure_layout(&self) -> std::io::Result<()> {
        for p in [
            self.bin(),
            self.packages(),
            self.cache_bottles(),
            self.cache_formulae(),
        ] {
            std::fs::create_dir_all(&p)?;
        }
        Ok(())
    }
}

/// Path to ghcr.io's host name template (extension point for tests).
pub fn formulae_api_base() -> &'static str {
    "https://formulae.brew.sh/api/formula"
}

pub fn ghcr_base() -> &'static str {
    "https://ghcr.io"
}

/// Sanity check: returns Err if the root is not writable.
pub fn check_writable(p: &Path) -> std::io::Result<()> {
    let probe = p.join(".write_probe");
    std::fs::write(&probe, b"olma")?;
    std::fs::remove_file(&probe)?;
    Ok(())
}
```

- [ ] **Step 2: Run a quick sanity build**

Run: `cargo check --lib 2>&1 | grep -E 'config' | head -5`
Expected: no errors inside `config.rs`. The crate as a whole still won't build until later modules land.

- [ ] **Step 3: Commit**

```bash
git add src/config.rs
git commit -m "feat: add Paths with OLMA_ROOT override for sandboxing"
```

---

## Task 5: `fs_lock.rs` — blocking exclusive lock

**Files:**
- Create: `src/fs_lock.rs`

- [ ] **Step 1: Implement the lock wrapper**

```rust
use crate::error::{OlmaError, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::Duration;

pub struct WriteLock {
    file: File,
}

impl WriteLock {
    /// Try to acquire the lock immediately. Returns `Err(LockBusy)` if held.
    pub fn try_acquire(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(WriteLock { file }),
            Err(_) => Err(OlmaError::LockBusy(path.to_path_buf())),
        }
    }

    /// Acquire the lock, polling every second. Cancellable by SIGINT (caller responsibility).
    pub async fn acquire_blocking(path: &Path) -> Result<Self> {
        loop {
            match Self::try_acquire(path) {
                Ok(lock) => return Ok(lock),
                Err(OlmaError::LockBusy(_)) => {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
```

- [ ] **Step 2: Run check**

Run: `cargo check --lib 2>&1 | grep fs_lock | head -5`
Expected: no errors in `fs_lock.rs`.

- [ ] **Step 3: Commit**

```bash
git add src/fs_lock.rs
git commit -m "feat: add fs2-backed WriteLock with blocking wait"
```

---

## Task 6: Output reporter — soft style

**Files:**
- Create: `src/output/mod.rs`
- Create: `src/output/soft.rs`

- [ ] **Step 1: Implement the Reporter trait and soft impl**

`src/output/mod.rs`:

```rust
mod soft;

pub use soft::SoftReporter;

pub trait Reporter: Send + Sync {
    fn status(&self, msg: &str);
    fn success(&self, msg: &str);
    fn error(&self, msg: &str);
}

pub fn default_reporter() -> Box<dyn Reporter> {
    Box::new(SoftReporter::new())
}
```

`src/output/soft.rs`:

```rust
use super::Reporter;
use std::io::{IsTerminal, Write};
use std::sync::Mutex;

const FRAMES: [&str; 10] = ["⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏"];

pub struct SoftReporter {
    state: Mutex<State>,
    is_tty: bool,
}

struct State {
    frame: usize,
}

impl SoftReporter {
    pub fn new() -> Self {
        SoftReporter {
            state: Mutex::new(State { frame: 0 }),
            is_tty: std::io::stderr().is_terminal(),
        }
    }

    fn write_line(&self, prefix: &str, msg: &str) {
        let mut err = std::io::stderr().lock();
        if self.is_tty {
            // \r clears, no newline so the line is overwritten
            let _ = write!(err, "\r\x1b[2K{prefix}  {msg}");
            let _ = err.flush();
        } else {
            let _ = writeln!(err, "{prefix}  {msg}");
        }
    }

    fn finish_line(&self, prefix: &str, msg: &str) {
        let mut err = std::io::stderr().lock();
        if self.is_tty {
            let _ = write!(err, "\r\x1b[2K{prefix}  {msg}\n");
            let _ = err.flush();
        } else {
            let _ = writeln!(err, "{prefix}  {msg}");
        }
    }
}

impl Reporter for SoftReporter {
    fn status(&self, msg: &str) {
        let mut s = self.state.lock().unwrap();
        let frame = FRAMES[s.frame % FRAMES.len()];
        s.frame = s.frame.wrapping_add(1);
        self.write_line(frame, msg);
    }

    fn success(&self, msg: &str) {
        self.finish_line("✓", msg);
    }

    fn error(&self, msg: &str) {
        self.finish_line("✗", msg);
    }
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo check --lib 2>&1 | grep -E 'output|soft' | head -5`
Expected: no errors in output module.

- [ ] **Step 3: Commit**

```bash
git add src/output
git commit -m "feat: add SoftReporter with braille spinner and TTY detect"
```

---

## Task 7: Metadata `Formula` struct

**Files:**
- Create: `src/metadata/mod.rs`

- [ ] **Step 1: Define the structs we will parse out of formulae.brew.sh JSON**

```rust
pub mod client;
pub mod ghcr;

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct Formula {
    pub name: String,
    pub desc: Option<String>,
    pub homepage: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub versions: Versions,
    pub bottle: BottleSpec,
}

#[derive(Debug, Deserialize)]
pub struct Versions {
    pub stable: String,
}

#[derive(Debug, Deserialize)]
pub struct BottleSpec {
    pub stable: BottleStable,
}

#[derive(Debug, Deserialize)]
pub struct BottleStable {
    pub rebuild: Option<u32>,
    pub root_url: String,
    pub files: HashMap<String, BottleFile>,
}

#[derive(Debug, Deserialize)]
pub struct BottleFile {
    pub cellar: String,
    pub url: String,
    pub sha256: String,
}

impl Formula {
    pub fn bottle_for_tag(&self, tag: &str) -> Option<&BottleFile> {
        self.bottle.stable.files.get(tag)
            .or_else(|| self.bottle.stable.files.get("all"))
    }

    pub fn version(&self) -> &str {
        &self.versions.stable
    }
}
```

- [ ] **Step 2: Run check**

Run: `cargo check --lib 2>&1 | grep metadata | head -5`
Expected: no errors in metadata/mod.rs (errors about client.rs and ghcr.rs are expected).

- [ ] **Step 3: Commit**

```bash
git add src/metadata/mod.rs
git commit -m "feat: define Formula deserialization structs"
```

---

## Task 8: `metadata/client.rs` — formulae.brew.sh fetch

**Files:**
- Create: `src/metadata/client.rs`

- [ ] **Step 1: Implement the HTTP client**

```rust
use crate::config::formulae_api_base;
use crate::error::{OlmaError, Result};
use crate::metadata::Formula;
use reqwest::StatusCode;
use std::time::Duration;

pub struct FormulaeClient {
    http: reqwest::Client,
}

impl FormulaeClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("olma/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        Ok(Self { http })
    }

    pub async fn fetch(&self, name: &str) -> Result<Formula> {
        let url = format!("{}/{}.json", formulae_api_base(), name);
        let resp = self.http.get(&url).send().await
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        match resp.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Err(OlmaError::FormulaNotFound(name.to_string())),
            s => return Err(OlmaError::Network(format!("formulae.brew.sh returned {s}"))),
        }
        let formula: Formula = resp.json().await
            .map_err(|e| OlmaError::Network(format!("failed to parse formula JSON: {e}")))?;
        Ok(formula)
    }
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo check --lib 2>&1 | grep client | head -5`
Expected: no errors in client.rs.

- [ ] **Step 3: Commit**

```bash
git add src/metadata/client.rs
git commit -m "feat: add FormulaeClient with timeouts and 404 mapping"
```

---

## Task 9: `metadata/ghcr.rs` — bottle blob fetch

**Files:**
- Create: `src/metadata/ghcr.rs`

- [ ] **Step 1: Implement GHCR token + blob fetcher**

```rust
use crate::error::{OlmaError, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct TokenResp {
    token: String,
}

pub struct GhcrClient {
    http: reqwest::Client,
}

impl GhcrClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("olma/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(120))  // bottles can be large
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        Ok(Self { http })
    }

    /// Returns an anonymous pull token for a single repository scope, e.g. `homebrew/core/tree`.
    pub async fn token(&self, scope_repo: &str) -> Result<String> {
        let url = format!(
            "https://ghcr.io/token?service=ghcr.io&scope=repository:{scope_repo}:pull"
        );
        let resp = self.http.get(&url).send().await
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(OlmaError::Network(format!("ghcr.io token request failed: {}", resp.status())));
        }
        let body: TokenResp = resp.json().await
            .map_err(|e| OlmaError::Network(format!("invalid token response: {e}")))?;
        Ok(body.token)
    }

    /// Streams the blob bytes into `sink`; returns total bytes written.
    pub async fn fetch_blob<W>(
        &self,
        url: &str,
        token: &str,
        mut sink: W,
    ) -> Result<u64>
    where
        W: tokio::io::AsyncWrite + Unpin,
    {
        use tokio::io::AsyncWriteExt;
        use futures_util::StreamExt;

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {token}")).unwrap());
        // Homebrew bottle layers use the OCI image manifest media type when
        // referenced as blobs.
        headers.insert("Accept", HeaderValue::from_static("application/vnd.oci.image.layer.v1.tar+gzip"));

        let resp = self.http.get(url).headers(headers).send().await
            .map_err(|e| OlmaError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(OlmaError::Network(format!("ghcr blob fetch failed: {}", resp.status())));
        }
        let mut total: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| OlmaError::Network(e.to_string()))?;
            sink.write_all(&bytes).await?;
            total += bytes.len() as u64;
        }
        sink.flush().await?;
        Ok(total)
    }
}
```

- [ ] **Step 2: Add `futures-util` to `Cargo.toml`**

In `[dependencies]`:
```toml
futures-util = { version = "0.3", default-features = false, features = ["std"] }
```

- [ ] **Step 3: Verify it builds**

Run: `cargo check --lib 2>&1 | grep ghcr | head -5`
Expected: no errors in ghcr.rs.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml src/metadata/ghcr.rs
git commit -m "feat: add GhcrClient with anonymous token and streaming blob fetch"
```

---

## Task 10: `pipeline/download.rs` — streaming download with sha256

**Files:**
- Create: `src/pipeline/mod.rs`
- Create: `src/pipeline/download.rs`

- [ ] **Step 1: Add the pipeline module root**

`src/pipeline/mod.rs`:

```rust
pub mod download;
pub mod extract;
pub mod link;
pub mod relocate;
pub mod verify;
```

- [ ] **Step 2: Implement download with hashing**

`src/pipeline/download.rs`:

```rust
use crate::config::Paths;
use crate::error::{OlmaError, Result};
use crate::metadata::{BottleFile, GhcrClient};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWrite;

pub struct Downloaded {
    /// Final path of the cached bottle.
    pub path: PathBuf,
    /// Hex-encoded sha256 of the bytes written.
    pub sha256_hex: String,
    pub bytes: u64,
}

/// Downloads the bottle to `cache/bottles/<expected_sha256>.tar.gz`.
/// Returns the computed sha256 so the caller can verify against the formula's
/// declared value.
pub async fn download(
    ghcr: &GhcrClient,
    bottle: &BottleFile,
    paths: &Paths,
) -> Result<Downloaded> {
    std::fs::create_dir_all(paths.cache_bottles())?;

    let final_path = paths.cache_bottles().join(format!("{}.tar.gz", bottle.sha256));
    if final_path.exists() {
        // Reuse cached, but rehash to confirm integrity.
        let bytes = tokio::fs::read(&final_path).await?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hex = hex::encode(hasher.finalize());
        return Ok(Downloaded {
            path: final_path,
            sha256_hex: hex,
            bytes: bytes.len() as u64,
        });
    }

    // Homebrew bottle URLs look like:
    //   https://ghcr.io/v2/homebrew/core/<name>/blobs/sha256:<digest>
    let scope_repo = extract_scope_repo(&bottle.url)
        .ok_or_else(|| OlmaError::Network(format!("unexpected bottle url: {}", bottle.url)))?;
    let token = ghcr.token(&scope_repo).await?;

    let partial = paths.cache_bottles().join(format!("{}.tar.gz.partial", bottle.sha256));
    let f = File::create(&partial).await?;
    let mut hashing = HashingWriter::new(f);
    let bytes = ghcr.fetch_blob(&bottle.url, &token, &mut hashing).await?;
    let hex = hashing.finish_hex();

    tokio::fs::rename(&partial, &final_path).await?;
    Ok(Downloaded { path: final_path, sha256_hex: hex, bytes })
}

fn extract_scope_repo(url: &str) -> Option<String> {
    // Expect: https://ghcr.io/v2/<owner>/<repo>/blobs/sha256:<digest>
    let path = url.split("ghcr.io/").nth(1)?;
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 6 { return None; }
    Some(format!("{}/{}/{}", parts[1], parts[2], parts[3]))
}

struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
}

impl<W: AsyncWrite + Unpin> HashingWriter<W> {
    fn new(inner: W) -> Self {
        Self { inner, hasher: Sha256::new() }
    }
    fn finish_hex(self) -> String {
        hex::encode(self.hasher.finalize())
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for HashingWriter<W> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let res = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(n)) = &res {
            self.hasher.update(&buf[..*n]);
        }
        res
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
```

- [ ] **Step 3: Add `hex` to `Cargo.toml`**

```toml
hex = "0.4"
```

- [ ] **Step 4: Verify it builds**

Run: `cargo check --lib 2>&1 | grep -E 'download|hex' | head -10`
Expected: no errors in download.rs.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/pipeline/mod.rs src/pipeline/download.rs
git commit -m "feat: streaming bottle download with inline sha256 hashing"
```

---

## Task 11: `pipeline/verify.rs` — sha256 compare

**Files:**
- Create: `src/pipeline/verify.rs`

- [ ] **Step 1: Implement verify**

```rust
use crate::error::{OlmaError, Result};

pub fn verify_sha256(expected: &str, actual: &str) -> Result<()> {
    if expected.eq_ignore_ascii_case(actual) {
        Ok(())
    } else {
        Err(OlmaError::ChecksumMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_case_insensitive() {
        assert!(verify_sha256("ABC123", "abc123").is_ok());
    }

    #[test]
    fn mismatch_returns_error() {
        assert!(matches!(
            verify_sha256("abc", "def"),
            Err(OlmaError::ChecksumMismatch)
        ));
    }
}
```

- [ ] **Step 2: Run the embedded tests**

Run: `cargo test --lib verify 2>&1 | tail -10`
Expected: 2 passing tests.

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/verify.rs
git commit -m "feat: add sha256 verifier with case-insensitive compare"
```

---

## Task 12: `pipeline/extract.rs` — tar.gz extraction

**Files:**
- Create: `src/pipeline/extract.rs`

- [ ] **Step 1: Implement extract using `tar` + `flate2` on a blocking thread**

```rust
use crate::error::Result;
use std::path::{Path, PathBuf};

/// Extracts `tar.gz` from `archive_path` to `dest_dir`. Returns the absolute path
/// of the first top-level directory entry the bottle created (Homebrew bottles
/// wrap their payload under `<name>/<version>/...`).
pub async fn extract(archive_path: &Path, dest_dir: &Path) -> Result<PathBuf> {
    let archive = archive_path.to_path_buf();
    let dest = dest_dir.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        std::fs::create_dir_all(&dest)?;
        let f = std::fs::File::open(&archive)?;
        let gz = flate2::read::GzDecoder::new(f);
        let mut tar = tar::Archive::new(gz);
        tar.unpack(&dest)?;
        for entry in std::fs::read_dir(&dest)? {
            let entry = entry?;
            if entry.path().is_dir() {
                return Ok(entry.path());
            }
        }
        Ok(dest)
    })
    .await
    .map_err(|e| crate::error::OlmaError::Other(format!("extract task failed: {e}")))?
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo check --lib 2>&1 | grep extract | head -5`
Expected: no errors in extract.rs.

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/extract.rs
git commit -m "feat: extract bottle tar.gz on blocking thread"
```

---

## Task 13: `relocator/classify.rs` — file kind detection

**Files:**
- Create: `src/relocator/mod.rs`
- Create: `src/relocator/classify.rs`
- Create: `tests/unit_classify.rs`

- [ ] **Step 1: Add the module root**

`src/relocator/mod.rs`:

```rust
pub mod classify;
pub mod macho;
pub mod text;
```

- [ ] **Step 2: Write the failing tests**

`tests/unit_classify.rs`:

```rust
use olma::relocator::classify::{classify_bytes, FileKind};

#[test]
fn macho_64bit_magic_recognized() {
    let bytes = [0xCF, 0xFA, 0xED, 0xFE, 0,0,0,0];
    assert_eq!(classify_bytes(&bytes), FileKind::MachO);
}

#[test]
fn macho_universal_magic_recognized() {
    let bytes = [0xCA, 0xFE, 0xBA, 0xBE, 0,0,0,0];
    assert_eq!(classify_bytes(&bytes), FileKind::MachO);
}

#[test]
fn utf8_text_recognized() {
    let bytes = b"# Some text\nhello\n";
    assert_eq!(classify_bytes(bytes), FileKind::Text);
}

#[test]
fn null_bytes_classified_as_binary() {
    let bytes = [0u8, 1, 2, 3, 4, 5, 6, 7];
    assert_eq!(classify_bytes(&bytes), FileKind::Binary);
}

#[test]
fn empty_is_text() {
    assert_eq!(classify_bytes(b""), FileKind::Text);
}
```

- [ ] **Step 3: Run test to confirm it fails**

Run: `cargo test --test unit_classify 2>&1 | tail -10`
Expected: compile error — module does not exist.

- [ ] **Step 4: Implement `classify.rs`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    MachO,
    Text,
    Binary,
}

const MACHO_MAGICS: [[u8; 4]; 4] = [
    [0xFE, 0xED, 0xFA, 0xCE],
    [0xFE, 0xED, 0xFA, 0xCF],
    [0xCE, 0xFA, 0xED, 0xFE],
    [0xCF, 0xFA, 0xED, 0xFE],
];

const FAT_MAGICS: [[u8; 4]; 4] = [
    [0xCA, 0xFE, 0xBA, 0xBE],
    [0xBE, 0xBA, 0xFE, 0xCA],
    [0xCA, 0xFE, 0xBA, 0xBF],
    [0xBF, 0xBA, 0xFE, 0xCA],
];

pub fn classify_bytes(head: &[u8]) -> FileKind {
    if head.len() >= 4 {
        let m = [head[0], head[1], head[2], head[3]];
        if MACHO_MAGICS.contains(&m) || FAT_MAGICS.contains(&m) {
            return FileKind::MachO;
        }
    }
    let probe_len = head.len().min(512);
    let probe = &head[..probe_len];
    if probe.contains(&0) {
        return FileKind::Binary;
    }
    if std::str::from_utf8(probe).is_ok() {
        FileKind::Text
    } else {
        FileKind::Binary
    }
}

/// Reads up to `len` bytes from `path` and classifies.
pub fn classify_path(path: &std::path::Path) -> std::io::Result<FileKind> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 512];
    let n = f.read(&mut buf)?;
    Ok(classify_bytes(&buf[..n]))
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test unit_classify 2>&1 | tail -10`
Expected: 5 passing tests.

- [ ] **Step 6: Commit**

```bash
git add src/relocator/mod.rs src/relocator/classify.rs tests/unit_classify.rs
git commit -m "feat: classify files as MachO / Text / Binary by magic + null scan"
```

---

## Task 14: `relocator/text.rs` — prefix replacement

**Files:**
- Create: `src/relocator/text.rs`
- Create: `tests/unit_text_relocate.rs`

- [ ] **Step 1: Write the failing tests**

`tests/unit_text_relocate.rs`:

```rust
use olma::relocator::text::replace_prefix_in_bytes;

#[test]
fn replaces_single_occurrence() {
    let input = b"prefix /opt/homebrew/Cellar/foo and more".to_vec();
    let out = replace_prefix_in_bytes(&input, "/opt/homebrew", "/opt/olma");
    assert_eq!(out, b"prefix /opt/olma/Cellar/foo and more");
}

#[test]
fn replaces_multiple_occurrences() {
    let input = b"a /opt/homebrew b /opt/homebrew c".to_vec();
    let out = replace_prefix_in_bytes(&input, "/opt/homebrew", "/opt/olma");
    assert_eq!(out, b"a /opt/olma b /opt/olma c");
}

#[test]
fn no_match_returns_original() {
    let input = b"nothing to replace here".to_vec();
    let out = replace_prefix_in_bytes(&input, "/opt/homebrew", "/opt/olma");
    assert_eq!(out, input);
}

#[test]
fn replacement_with_different_lengths() {
    let input = b"/usr/local/bin/foo".to_vec();
    let out = replace_prefix_in_bytes(&input, "/usr/local", "/opt/olma");
    assert_eq!(out, b"/opt/olma/bin/foo");
}
```

- [ ] **Step 2: Run to confirm fail**

Run: `cargo test --test unit_text_relocate 2>&1 | tail -10`
Expected: compile error — module does not exist.

- [ ] **Step 3: Implement**

`src/relocator/text.rs`:

```rust
use memchr::memmem;

pub fn replace_prefix_in_bytes(input: &[u8], old: &str, new: &str) -> Vec<u8> {
    let old_b = old.as_bytes();
    let new_b = new.as_bytes();
    let finder = memmem::Finder::new(old_b);
    let positions: Vec<usize> = finder.find_iter(input).collect();
    if positions.is_empty() {
        return input.to_vec();
    }
    let mut out = Vec::with_capacity(input.len() + positions.len() * new_b.len().saturating_sub(old_b.len()));
    let mut cursor = 0;
    for pos in positions {
        out.extend_from_slice(&input[cursor..pos]);
        out.extend_from_slice(new_b);
        cursor = pos + old_b.len();
    }
    out.extend_from_slice(&input[cursor..]);
    out
}

/// Reads the file, replaces, and writes back if any change occurred.
/// Returns Ok(true) if the file was rewritten.
pub fn relocate_text_file(
    path: &std::path::Path,
    old_prefix: &str,
    new_prefix: &str,
) -> std::io::Result<bool> {
    let bytes = std::fs::read(path)?;
    let replaced = replace_prefix_in_bytes(&bytes, old_prefix, new_prefix);
    if replaced == bytes {
        return Ok(false);
    }
    // Preserve mode.
    let meta = std::fs::metadata(path)?;
    let tmp = path.with_extension("relocate.tmp");
    std::fs::write(&tmp, &replaced)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(meta.permissions().mode());
        std::fs::set_permissions(&tmp, perms)?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(true)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test unit_text_relocate 2>&1 | tail -10`
Expected: 4 passing tests.

- [ ] **Step 5: Commit**

```bash
git add src/relocator/text.rs tests/unit_text_relocate.rs
git commit -m "feat: byte-level prefix replacement with mode preservation"
```

---

## Task 15: `relocator/macho.rs` — install_name_tool + codesign shellout

**Files:**
- Create: `src/relocator/macho.rs`

- [ ] **Step 1: Implement Mach-O relocation via shell-out**

```rust
use crate::error::{OlmaError, Result};
use std::path::Path;
use std::process::Command;

/// Verifies `install_name_tool` and `codesign` are available.
/// Returns `MissingXcodeCLT` if either is absent.
pub fn check_clt_available() -> Result<()> {
    for tool in &["install_name_tool", "codesign", "otool"] {
        if which(tool).is_none() {
            return Err(OlmaError::MissingXcodeCLT);
        }
    }
    Ok(())
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for entry in std::env::split_paths(&path) {
        let cand = entry.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Reads all dependency load commands (LC_LOAD_DYLIB, LC_ID_DYLIB, LC_RPATH)
/// and any that begin with `old_prefix` are rewritten to start with `new_prefix`.
/// After modification the binary is re-signed ad-hoc.
pub fn relocate_macho(path: &Path, old_prefix: &str, new_prefix: &str) -> Result<()> {
    let load_cmds = otool_l(path)?;

    // 1) LC_ID_DYLIB
    if let Some(id) = load_cmds.id_dylib.as_deref() {
        if let Some(new_id) = swap_prefix(id, old_prefix, new_prefix) {
            run(Command::new("install_name_tool").args(["-id", &new_id, path.to_str().unwrap()]))?;
        }
    }

    // 2) LC_LOAD_DYLIB
    for dep in &load_cmds.load_dylibs {
        if let Some(new_dep) = swap_prefix(dep, old_prefix, new_prefix) {
            run(Command::new("install_name_tool").args([
                "-change", dep, &new_dep, path.to_str().unwrap(),
            ]))?;
        }
    }

    // 3) LC_RPATH
    for rp in &load_cmds.rpaths {
        if let Some(new_rp) = swap_prefix(rp, old_prefix, new_prefix) {
            run(Command::new("install_name_tool").args([
                "-rpath", rp, &new_rp, path.to_str().unwrap(),
            ]))?;
        }
    }

    // 4) Re-sign ad-hoc to repair the signature.
    run(Command::new("codesign").args([
        "--force", "--sign", "-", path.to_str().unwrap(),
    ]))?;
    Ok(())
}

fn swap_prefix(s: &str, old: &str, new: &str) -> Option<String> {
    s.strip_prefix(old).map(|rest| format!("{new}{rest}"))
}

#[derive(Default)]
struct LoadCmds {
    id_dylib: Option<String>,
    load_dylibs: Vec<String>,
    rpaths: Vec<String>,
}

fn otool_l(path: &Path) -> Result<LoadCmds> {
    // `otool -l <path>` prints all load commands. We parse the lines we care about.
    let out = Command::new("otool").args(["-l", path.to_str().unwrap()]).output()
        .map_err(|e| OlmaError::Other(format!("otool failed: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other(format!(
            "otool exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut cmds = LoadCmds::default();
    let mut state = State::None;
    for line in stdout.lines() {
        let t = line.trim();
        match state {
            State::None => {
                if t.starts_with("cmd LC_ID_DYLIB") { state = State::IdDylib; }
                else if t.starts_with("cmd LC_LOAD_DYLIB") { state = State::LoadDylib; }
                else if t.starts_with("cmd LC_RPATH") { state = State::Rpath; }
            }
            State::IdDylib => {
                if let Some(rest) = t.strip_prefix("name ") {
                    let value = rest.split(" (offset").next().unwrap_or(rest).trim().to_string();
                    cmds.id_dylib = Some(value);
                    state = State::None;
                }
            }
            State::LoadDylib => {
                if let Some(rest) = t.strip_prefix("name ") {
                    let value = rest.split(" (offset").next().unwrap_or(rest).trim().to_string();
                    cmds.load_dylibs.push(value);
                    state = State::None;
                }
            }
            State::Rpath => {
                if let Some(rest) = t.strip_prefix("path ") {
                    let value = rest.split(" (offset").next().unwrap_or(rest).trim().to_string();
                    cmds.rpaths.push(value);
                    state = State::None;
                }
            }
        }
    }
    Ok(cmds)
}

enum State { None, IdDylib, LoadDylib, Rpath }

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().map_err(|e| OlmaError::Other(format!("spawn failed: {e}")))?;
    if !out.status.success() {
        return Err(OlmaError::Other(format!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo check --lib 2>&1 | grep macho | head -5`
Expected: no errors in macho.rs.

- [ ] **Step 3: Commit**

```bash
git add src/relocator/macho.rs
git commit -m "feat: relocate Mach-O load commands via install_name_tool, re-sign ad-hoc"
```

---

## Task 16: `pipeline/relocate.rs` — orchestrate file walks

**Files:**
- Create: `src/pipeline/relocate.rs`

- [ ] **Step 1: Walk the extracted directory and dispatch by file kind**

```rust
use crate::error::Result;
use crate::relocator::{classify::{classify_path, FileKind}, macho, text};
use std::path::Path;
use walkdir::WalkDir;

/// A list of `(from_prefix, to_prefix)` swaps applied to every file in `dir`.
/// The swaps are tried in order; the first matching prefix is rewritten and the
/// remaining swaps still run independently (Mach-O load commands and text files
/// may carry different references).
pub type SwapList = Vec<(String, String)>;

/// Relocates prefix references inside `dir` according to `swaps`.
pub async fn relocate_tree(dir: &Path, swaps: SwapList) -> Result<RelocStats> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || relocate_tree_blocking(&dir, &swaps))
        .await
        .map_err(|e| crate::error::OlmaError::Other(format!("relocate task failed: {e}")))?
}

#[derive(Debug, Default)]
pub struct RelocStats {
    pub macho_files: usize,
    pub text_files: usize,
    pub symlinks: usize,
}

fn relocate_tree_blocking(dir: &Path, swaps: &[(String, String)]) -> Result<RelocStats> {
    let mut stats = RelocStats::default();

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_symlink() {
            relocate_symlink(path, swaps)?;
            stats.symlinks += 1;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let kind = classify_path(path)?;
        match kind {
            FileKind::MachO => {
                for (old, new) in swaps {
                    macho::relocate_macho(path, old, new)?;
                }
                stats.macho_files += 1;
            }
            FileKind::Text => {
                let mut changed = false;
                for (old, new) in swaps {
                    changed |= text::relocate_text_file(path, old, new)?;
                }
                if changed { stats.text_files += 1; }
            }
            FileKind::Binary => {}
        }
    }
    Ok(stats)
}

fn relocate_symlink(link: &Path, swaps: &[(String, String)]) -> Result<()> {
    let target = std::fs::read_link(link)?;
    let s = target.to_string_lossy().to_string();
    for (old, new) in swaps {
        if let Some(rest) = s.strip_prefix(old.as_str()) {
            let new_target = format!("{new}{rest}");
            std::fs::remove_file(link)?;
            std::os::unix::fs::symlink(&new_target, link)?;
            return Ok(());
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo check --lib 2>&1 | grep relocate | head -5`
Expected: no errors in relocate.rs.

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/relocate.rs
git commit -m "feat: walk extracted tree and relocate Mach-O, text, and symlinks"
```

---

## Task 17: `pipeline/link.rs` — symlinks into bin/

**Files:**
- Create: `src/pipeline/link.rs`

- [ ] **Step 1: Implement linking**

```rust
use crate::config::Paths;
use crate::error::Result;
use std::os::unix::fs as unix_fs;
use std::path::Path;

#[derive(Debug, Default)]
pub struct LinkStats {
    pub linked: Vec<String>,
    pub skipped_collisions: Vec<String>,
}

/// Creates symlinks in `paths.bin()` for every executable under `package_dir/bin/`.
/// Existing symlinks that already point inside the same package are replaced (idempotent).
/// Collisions with other packages are reported in `skipped_collisions`.
pub fn link_bin(paths: &Paths, package_dir: &Path) -> Result<LinkStats> {
    let mut stats = LinkStats::default();
    let bin_src = package_dir.join("bin");
    if !bin_src.exists() {
        return Ok(stats);
    }
    std::fs::create_dir_all(paths.bin())?;
    for entry in std::fs::read_dir(&bin_src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let link = paths.bin().join(&name);
        let target = entry.path();

        if link.is_symlink() {
            // If it already points inside our package_dir, replace and move on.
            // Otherwise treat as a collision with a different package.
            let existing = std::fs::read_link(&link)?;
            let absolute = if existing.is_absolute() {
                existing.clone()
            } else {
                paths.bin().join(existing)
            };
            if absolute.starts_with(package_dir) {
                std::fs::remove_file(&link)?;
            } else {
                stats.skipped_collisions.push(name);
                continue;
            }
        } else if link.exists() {
            stats.skipped_collisions.push(name);
            continue;
        }

        unix_fs::symlink(&target, &link)?;
        stats.linked.push(name);
    }
    Ok(stats)
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo check --lib 2>&1 | grep link | head -5`
Expected: no errors in link.rs.

- [ ] **Step 3: Commit**

```bash
git add src/pipeline/link.rs
git commit -m "feat: create bin/ symlinks and detect cross-package collisions"
```

---

## Task 18: `cli/add.rs` — orchestrate the pipeline

**Files:**
- Create: `src/cli/mod.rs`
- Create: `src/cli/add.rs`

- [ ] **Step 1: Implement the Cli struct and Add subcommand**

`src/cli/mod.rs`:

```rust
pub mod add;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

use crate::output::default_reporter;

#[derive(Parser)]
#[command(name = "olma", version, about = "macOS package manager")]
pub struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Install a package.
    Add { name: String },
}

pub async fn run() -> ExitCode {
    let cli = Cli::parse();
    let reporter = default_reporter();
    let result = match cli.cmd {
        Cmd::Add { name } => add::run(&name, reporter.as_ref()).await,
    };
    match result {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            reporter.error(&e.to_string());
            e.exit_code()
        }
    }
}
```

`src/cli/add.rs`:

```rust
use crate::config::Paths;
use crate::error::{OlmaError, Result};
use crate::fs_lock::WriteLock;
use crate::metadata::{client::FormulaeClient, ghcr::GhcrClient};
use crate::output::Reporter;
use crate::pipeline::{download, extract, link, relocate, verify};
use crate::platform::current_bottle_tag;
use crate::relocator::macho;

pub async fn run(name: &str, reporter: &dyn Reporter) -> Result<()> {
    macho::check_clt_available()?;

    let paths = Paths::from_env();
    paths.ensure_layout()?;
    let _lock = WriteLock::acquire_blocking(&paths.lock_file()).await?;

    reporter.status(&format!("Fetching {name} metadata"));
    let client = FormulaeClient::new()?;
    let formula = client.fetch(name).await?;

    if !formula.dependencies.is_empty() {
        return Err(OlmaError::Other(format!(
            "{name} has dependencies; MVP supports zero-dep formulas only. \
             Try `tree`, `jq`, or `fd`."
        )));
    }

    let tag = current_bottle_tag().await?;
    let bottle = formula.bottle_for_tag(&tag)
        .ok_or_else(|| OlmaError::BottleNotForPlatform {
            name: name.to_string(),
            platform: tag.clone(),
        })?;

    reporter.status(&format!("Downloading {name} {}", formula.version()));
    let ghcr = GhcrClient::new()?;
    let dl = download::download(&ghcr, bottle, &paths).await?;
    verify::verify_sha256(&bottle.sha256, &dl.sha256_hex)?;

    // Extract into a fresh staging directory; bottles unpack as
    // `<name>/<version>/...`. Then move that inner directory to
    // `/opt/olma/packages/<name>/<version>/`.
    reporter.status(&format!("Extracting {name}"));
    let staging = paths.cache().join("staging")
        .join(format!("{}-{}", formula.name, formula.version()));
    if staging.exists() { std::fs::remove_dir_all(&staging)?; }
    std::fs::create_dir_all(&staging)?;
    extract::extract(&dl.path, &staging).await?;
    let inner = staging.join(&formula.name).join(formula.version());
    if !inner.is_dir() {
        return Err(OlmaError::Other(format!(
            "unexpected bottle layout: expected {}", inner.display()
        )));
    }
    let dest = paths.package_dir(&formula.name, formula.version());
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(&inner, &dest)?;
    let _ = std::fs::remove_dir_all(&staging);

    reporter.status(&format!("Relocating {name}"));
    let new_cellar = dest.to_string_lossy().to_string();
    let new_root = paths.root.to_string_lossy().to_string();
    let swaps: Vec<(String, String)> = vec![
        // Specific Cellar swaps must come before the catch-all so they win.
        (format!("/opt/homebrew/Cellar/{}/{}", formula.name, formula.version()), new_cellar.clone()),
        (format!("/usr/local/Cellar/{}/{}", formula.name, formula.version()), new_cellar.clone()),
        ("/opt/homebrew".to_string(), new_root.clone()),
        ("/usr/local".to_string(), new_root.clone()),
    ];
    relocate::relocate_tree(&dest, swaps).await?;

    reporter.status(&format!("Linking {name} into bin/"));
    let stats = link::link_bin(&paths, &dest)?;
    for collision in &stats.skipped_collisions {
        reporter.error(&format!("symlink collision: {collision} (use --force-link to override)"));
    }

    reporter.success(&format!(
        "Installed {} {} ({} bytes)",
        formula.name, formula.version(), dl.bytes
    ));
    Ok(())
}
```

- [ ] **Step 2: Verify it builds**

Run: `cargo build 2>&1 | tail -20`
Expected: a successful build (errors here mean we have wiring bugs to fix before moving on).

- [ ] **Step 3: Run the binary against `--help`**

Run: `./target/debug/olma --help`
Expected: clap usage output listing the `add` subcommand.

- [ ] **Step 4: Commit**

```bash
git add src/cli
git commit -m "feat: wire add subcommand through fetch → download → relocate → link"
```

---

## Task 19: Sandbox test helper

**Files:**
- Create: `tests/helpers/mod.rs`

- [ ] **Step 1: Build the sandbox helper**

```rust
#![allow(dead_code)]

use std::path::PathBuf;

/// A throwaway OLMA_ROOT under tempdir.
pub struct Sandbox {
    pub root: tempfile::TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        let root = tempfile::Builder::new().prefix("olma-test-").tempdir().unwrap();
        std::env::set_var("OLMA_ROOT", root.path());
        Sandbox { root }
    }

    pub fn root_path(&self) -> PathBuf {
        self.root.path().to_path_buf()
    }

    pub fn bin(&self) -> PathBuf { self.root_path().join("bin") }
    pub fn packages(&self) -> PathBuf { self.root_path().join("packages") }
}
```

Note: `tests/helpers/mod.rs` is *not* automatically included by integration tests; each test file that needs it adds `mod helpers;` at the top.

- [ ] **Step 2: Commit**

```bash
git add tests/helpers
git commit -m "test: add Sandbox helper that sets OLMA_ROOT to a tempdir"
```

---

## Task 20: Integration test — `olma add tree`

**Files:**
- Create: `tests/integration_add_tree.rs`

- [ ] **Step 1: Write the end-to-end test**

```rust
mod helpers;

use helpers::Sandbox;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_tree_installs_and_runs() {
    // Requires network (formulae.brew.sh + ghcr.io) and Xcode CLT.
    if std::env::var("OLMA_SKIP_NETWORK_TESTS").is_ok() {
        eprintln!("skipped: OLMA_SKIP_NETWORK_TESTS set");
        return;
    }

    let sandbox = Sandbox::new();

    let reporter = olma::output::default_reporter();
    olma::cli::add::run("tree", reporter.as_ref()).await
        .expect("add tree should succeed");

    let tree_bin = sandbox.bin().join("tree");
    assert!(tree_bin.exists(), "expected {} to exist", tree_bin.display());

    // The symlink should resolve and the binary should execute.
    let out = std::process::Command::new(&tree_bin)
        .arg("--version")
        .output()
        .expect("tree --version should run");
    assert!(out.status.success(),
        "tree --version failed: stderr={}",
        String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("tree"), "unexpected version output: {stdout}");
}
```

- [ ] **Step 2: Run the integration test**

Run: `cargo test --test integration_add_tree -- --nocapture 2>&1 | tail -40`
Expected: passes. If it fails on network, retry once. If it fails on relocation, inspect the temp dir before it is cleaned up by running with `OLMA_KEEP_TEMP=1` (the helper does not implement this yet; do it manually with `TMPDIR=/tmp cargo test`).

- [ ] **Step 3: Commit**

```bash
git add tests/integration_add_tree.rs
git commit -m "test: integration test for olma add tree against real network"
```

---

## Task 21: Self-review checkpoint

- [ ] **Step 1: Run the full test suite**

Run: `cargo test 2>&1 | tail -20`
Expected: all tests pass (unit_platform: 4, unit_classify: 5, unit_text_relocate: 4, unit verify: 2, integration_add_tree: 1).

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-targets -- -D warnings 2>&1 | tail -20`
Expected: zero warnings or errors. Fix any that surface before committing.

- [ ] **Step 3: Verify `olma --help` is sane**

Run: `./target/debug/olma --help`
Expected: shows `add <NAME>` subcommand with description.

- [ ] **Step 4: Manual smoke test against a real `/tmp` sandbox**

```bash
OLMA_ROOT=/tmp/olma-smoke-$$ ./target/debug/olma add tree
ls /tmp/olma-smoke-*/bin/tree
/tmp/olma-smoke-*/bin/tree --version
```

Expected: `tree --version` prints a tree v2.x banner.

- [ ] **Step 5: Final commit if any clippy fixes were made**

```bash
git status
# If anything modified:
git add -A
git commit -m "chore: address clippy warnings from MVP self-review"
```

---

## Plan-level verification

After Task 21, the following are true:

1. `olma add tree` installs the `tree` bottle into the sandbox and produces a working `tree` binary.
2. Bottle relocation is exercised against a real Mach-O binary (re-signed ad-hoc).
3. SHA256 verification is enforced.
4. Single-writer `.lock` is acquired and released.
5. CLI emits minimal-soft status lines through `SoftReporter`.
6. The MVP fails clearly when invoked on a formula that has dependencies (deferred to Plan 2).

## Out of scope (reminder)

Plan 2 will add: ETag-cached metadata, full dep graph + topo sort, parallel pipeline, status-line aggregating multiple packages.
Plan 3 adds state.db, history, rollback.
Plan 4 adds the remaining commands.
Plan 5 polishes search, did-you-mean, completions, self-update.
