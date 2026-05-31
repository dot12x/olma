# Olma — Architecture Design (V1)

**Status:** Approved (2026-05-31)
**Author:** sattorbek
**Scope:** First implementable version of the `olma` macOS package manager.

## 1. Overview

Olma is a fast, parallel-first macOS package manager written in Rust. It uses **Homebrew's bottle infrastructure** as its binary source — pulling pre-built tarballs from `ghcr.io` and metadata from `formulae.brew.sh` — but ships its own CLI, install layout, output style, and concurrency model. Olma never invokes the `brew` binary, never reads or writes the Homebrew cellar, and is fully standalone.

### Design goals

1. **Speed is the product.** Pipeline parallelism, no Ruby boot, streaming verify, ETag-cached metadata, auto-tuned concurrency (no `-j` flag).
2. **Great UX.** Minimal-soft CLI (braille spinner, single-line status), apt-style verbs (`add`, `remove`, `update`, `upgrade`), actionable errors with did-you-mean.
3. **Standalone.** `/opt/olma` only. No interaction with an existing brew installation. Works even on a machine without Homebrew.
4. **Predictable.** Apt-style explicit `update` vs `upgrade`. No implicit network calls during `add`. Cache-first reads.

### Non-goals (V1)

- Source builds (binary only — bottles only)
- Casks / GUI apps (formulas only)
- Custom taps (homebrew-core only)
- Project-local installs (system-wide only)
- Linux (macOS arm64 + x64 only)

## 2. Architecture

```
            ┌──────────────────────────────────────┐
            │  olma CLI (clap, single binary)      │
            └────────────┬─────────────────────────┘
                         │
       ┌─────────────────┼─────────────────────────┐
       ▼                 ▼                         ▼
 ┌──────────┐    ┌──────────────┐         ┌──────────────┐
 │ Resolver │    │  Installer   │         │ State (FS)   │
 │  (deps)  │◀──▶│  (pipeline)  │◀───────▶│ /opt/olma    │
 └────┬─────┘    └──────┬───────┘         └──────────────┘
      │                 │
      ▼                 ▼
 ┌──────────┐    ┌──────────────┐
 │ Metadata │    │  Bottle CDN  │
 │  Client  │    │  (ghcr.io)   │
 │ formulae.│    │              │
 │ brew.sh  │    │              │
 └──────────┘    └──────────────┘
```

## 3. Filesystem layout

```
/opt/olma/
├── bin/                          ← Added to PATH. Symlinks → packages/.../bin/...
│   ├── rg     → ../packages/ripgrep/14.1.0/bin/rg
│   ├── python → ../packages/python/3.13.1/bin/python3.13
│   └── python3.11 → ../packages/python@3.11/3.11.10/bin/python3.11
│
├── packages/
│   ├── ripgrep/
│   │   └── 14.1.0/               ← bottle extracted + relocated here
│   │       ├── bin/, lib/, share/
│   │       └── INSTALL_RECEIPT.json
│   ├── python/                   ← formula name "python" (latest major)
│   │   └── 3.13.1/
│   └── python@3.11/              ← brew @suffix formula = separate package
│       └── 3.11.10/
│
├── cache/
│   ├── bottles/                  ← downloaded .tar.gz (named by sha256)
│   └── formulae/                 ← formulae.brew.sh JSON cache (with ETag)
│
├── state.db                      ← SQLite: installed packages, defaults, tx log
└── .lock                         ← fs2 lock (acquired by write operations)
```

**Why SQLite:** atomic transactions, fast queries for `list`/`outdated`/`history`, safe concurrent reads via WAL mode. Adds ~600KB to binary via `rusqlite` (bundled).

## 4. Metadata client & cache

### Endpoints

- **Single formula:** `https://formulae.brew.sh/api/formula/{name}.json` (~5–50KB)
- **Full dump:** `https://formulae.brew.sh/api/formula.json` (~30MB) — used only by `olma update --full` and `olma search`.

### Cache policy

ETag-based, no TTL. Each formula JSON cached at `cache/formulae/{name}.json` with `{name}.etag` sidecar.

| Flag | Network? | Cache used? |
|---|---|---|
| (default) | No (cache-first, no implicit refresh) | Yes |
| `--refresh` | Yes (force re-fetch) | Yes (overwritten) |
| `--no-cache` | Yes (cache bypassed) | Read-only |
| `--offline` | No | Yes only |

`--no-cache` + `--offline` → error (contradiction).

### `update` semantics (apt-style)

- **`olma update`** — refresh metadata for installed packages. No installs.
- **`olma update --full`** — re-fetch the full `formula.json` dump (enables fresh `search` suggestions).
- **`olma upgrade [<name>]`** — install newer versions using current cache.

Cache age > 7 days: footer reminder on `outdated` and `upgrade` to run `olma update`.

### HTTP client

- `reqwest` with rustls (no OpenSSL dep), HTTP/2, connection pool.
- Timeout: connect 5s, total 60s per request. Overridable via `--timeout`.
- Retries: 3× exponential backoff (100ms, 500ms, 2s) on 5xx and connection errors. Never on 4xx.
- User-Agent: `olma/<version> (<os>-<arch>)`.

### GHCR (bottle) fetch

Bottles live at `ghcr.io/v2/homebrew/core/<name>` (Docker Registry V2 API). Anonymous bearer token required:

```
GET /token?service=ghcr.io&scope=repository:homebrew/core/<name>:pull
→ { "token": "..." }

GET /v2/homebrew/core/<name>/blobs/sha256:<digest>
Authorization: Bearer <token>
```

Token cached for its declared TTL (typically ~5 minutes; refetched on expiry).

## 5. Dependency resolver

### Input

Each formula JSON exposes:

- `dependencies` — runtime deps (we follow)
- `build_dependencies` — ignored (we install binaries, not sources)
- `uses_from_macos` — ignored (system-provided)

### Algorithm

1. BFS from each requested formula, collecting all transitive `dependencies`.
2. Topological sort (Kahn). Cycles → error.
3. Skip nodes already installed at the requested version (`state.db` lookup).
4. Return `InstallPlan { ordered_packages, total_size, requested, auto }`.

### Conflict cases

- **Version mismatch between deps:** rare in homebrew-core (each `@suffix` is its own formula). When it does occur, install all required @suffix variants alongside.
- **Diamond:** common; install the shared dep once.
- **Cycle:** error `Cycle detected: A → B → A`.

### User-facing plan (apt-style)

```
$ olma add ripgrep

  Will install 2 packages:
    pcre2          10.43      2.1 MB
    ripgrep        14.1.0     6.2 MB
                              ────────
                   Total:     8.3 MB  (2 new)

  Proceed? [Y/n]
```

`-y` skips prompt; non-TTY requires `-y`; `--dry-run` prints plan and exits 0.

## 6. Install pipeline (parallel)

Five stages, each a Tokio task with a bounded channel to the next. Packages stream through; the pipeline is fully parallel from `Download` through `Relocate`. `Link` is serialized under `.lock`.

```
        ┌──────────┐   ┌────────┐   ┌─────────┐   ┌──────────┐   ┌──────┐
plan ──▶│ Download │──▶│ Verify │──▶│ Extract │──▶│ Relocate │──▶│ Link │
        └──────────┘   └────────┘   └─────────┘   └──────────┘   └──────┘
            ║              ║            ║              ║            │
        ════╩══════════════╩════════════╩══════════════╩══════════  │
                          PARALLEL                                   │
                                                              SEQUENTIAL
                                                              (under .lock)
```

### Auto-tuning (no `-j` flag)

- **Download pool:** `min(8, num_cpus * 2)`. Static in V1; adaptive bandwidth measurement in V2.
- **CPU pools (verify/extract/relocate):** `num_cpus`.
- **Link:** always 1.

Rationale: speed is the product — users should not tune knobs. The right number is the package manager's job.

### Stage details

- **Download.** Streaming write to `cache/bottles/<sha256>.tar.gz.partial`, atomic rename on success. SHA256 hasher inline (zero-cost verify). HTTP/2 multiplexing reuses connections. No resumable downloads in V1 (bottles are 1–50MB).
- **Verify.** Compare streamed sha256 to expected. Mismatch → abort install, delete partial, exit 1.
- **Extract.** `tar` + `flate2` to `packages/<name>/<version>/`. Symlinks preserved.
- **Relocate.** See §7.
- **Link.** Walk extracted `bin/`, create symlinks in `/opt/olma/bin/<exe>`. Acquired write lock. State.db transaction committed only after successful link.

### Progress (minimal soft)

```
⠋  Installing ripgrep, pcre2  ·  2/3 downloaded  ·  4.2/8.3 MB
✓  Installed ripgrep 14.1.0 + 1 dep  (3.4s)
```

`-v` shows per-stage log. `--json` emits structured final result, no progress.

### Transactionality

Mid-pipeline failure for a package: that package's partial extraction is removed; other packages already past `Link` remain. State.db only records packages that completed `Link`. No half-installs in state.

## 7. Bottle relocation

### Problem

Homebrew bottles are built with `/opt/homebrew` (Apple Silicon) or `/usr/local` (Intel) baked into Mach-O load commands and text files. Olma installs into `/opt/olma`, requiring path rewriting.

### Cellar tag (from bottle DSL)

- `:any` — relocatable anywhere; no work.
- `:any_skip_relocation` — no prefix references; no work.
- Specific path (e.g. `/opt/homebrew/Cellar`) — must relocate. Most common case.

### Algorithm

Walk the extracted directory:

| File type | Action |
|---|---|
| Mach-O (`.dylib`, executable) | Rewrite `LC_ID_DYLIB`, `LC_LOAD_DYLIB`, `LC_RPATH`. Re-sign ad-hoc (`codesign --force --sign -`). |
| `.pc`, `.cmake`, shebang scripts, `.la` (often deleted) | Text replace `/opt/homebrew` → `/opt/olma`. |
| Symlinks (absolute, pointing at old prefix) | Re-link to new prefix. |
| Other text | Inspect first 8 bytes for binary signature; replace only if UTF-8. |
| Other binary | Skip. |

### V1 implementation: shell-out

- **Mach-O:** invoke `install_name_tool` and `codesign` (ship with Xcode CLT). At install time, olma checks for CLT; if missing, prompts: `Run: xcode-select --install`.
- **Text:** native Rust (`memchr` + `Vec::splice`).
- **Symlinks:** native Rust.

V2 plan: replace shell-out with native Mach-O rewriting via `goblin`.

### Failure modes

- CLT missing → exit 78 with `xcode-select --install` hint.
- `install_name_tool` fails → mark package broken, do not commit to state.db, clean up.
- SHA256 mismatch upstream of relocate → handled in Verify (never reaches Relocate).

## 8. CLI commands & versioning model

### Command surface

| Command | Purpose |
|---|---|
| `olma add <name>[@<ver>]…` | Install package(s) with deps |
| `olma remove <name>` / `olma rm <name>` | Remove a package |
| `olma readd <name>` | Atomic `remove` + `add`; preserves default version |
| `olma upgrade [<name>]` | Install newer version(s) |
| `olma update [--full]` | Refresh metadata cache |
| `olma outdated` | Show packages with newer versions available |
| `olma list` | Per-package summary of installed |
| `olma info <name>` | Versions, deps, size, homepage |
| `olma search <query>` | Substring + did-you-mean |
| `olma default <name>[@<ver>]` | Switch default version (within a versioned formula) |
| `olma rollback [<name>]` | Revert last transaction or specific package |
| `olma history [--limit N]` | Transaction log |
| `olma clean` | Remove all cached bottles |
| `olma autoclean` | Remove non-default-version bottles + previous-generation packages |
| `olma autoremove` | Remove orphaned auto-installed deps |
| `olma purge <name>` | `remove` + delete its cached bottles |
| `olma self-update` | Update the olma binary itself |

### Versioning model (replaces prior multi-version-per-formula design)

One **active** version per formula in PATH. Multi-version comes via Homebrew's @suffix formulas (`python@3.11`, `node@18`), which olma treats as separate packages.

Disk-side, each formula keeps up to two generations (current + previous) as rollback safety; see §10. Only the current generation is active in `/opt/olma/bin/`; the previous generation lives at `packages/<name>/<old_version>/` and is reachable only via `olma rollback`.

```
$ olma list

  python      ●  default    3.13.1     67 MB
  python@3.11 ●  default    3.11.10    52 MB    # separate package, own default
  ripgrep     ●  default    14.1.0     6.2 MB
```

### `bin/` symlink rules

For each installed package, olma creates `/opt/olma/bin/<exe>` symlinks for every executable in the bottle's `bin/`. Name collisions between **distinct packages** (not between versions of the same formula) → error; resolution via `olma add --force-link` is deferred to V2. Upgrades within the same formula always replace the formula's own symlinks atomically.

### Global flags

`-y`, `--dry-run`, `--offline`, `--no-cache`, `--refresh`, `-q/-v/-vv`, `--json`, `--timeout SECS`.

Excluded: `-j`, `--force` (use specific `--force-*`), `--no-deps`, `--no-color` (color is always on; user cannot disable).

### Exit codes

`0` success · `1` general · `2` CLI usage · `64` user input · `65` formula not found · `66` no bottle for platform · `74` I/O · `78` configuration (missing CLT, perms).

## 9. Concurrency, locking, network

### Single-writer lock

`/opt/olma/.lock` via `fs2::FileExt::try_lock_exclusive`. Acquired by: `add`, `remove`, `upgrade`, `default`, `update`, `clean`, `autoclean`, `autoremove`, `purge`, `self-update`, `rollback`.

Read operations (`list`, `info`, `search`, `outdated`, `history`) acquire no lock; SQLite WAL allows safe concurrent reads.

### Lock-busy behavior

Wait by default with 1-second poll. Message: `"/opt/olma is locked by another olma process (PID X). Waiting..."`. User aborts with Ctrl-C. No timeout. `--no-wait` flag deferred to V2.

### Signal handling

- **SIGINT:** complete current stage (1–3s), clean up partial work, exit 130.
- **SIGTERM:** same with 5s grace, then forced.
- **SIGKILL recovery:** next olma run detects orphan partial dirs via pid file and cleans them.

## 10. Rollback & history

### Model: bounded N=2 generations

Each package keeps at most two versions on disk: current default and the immediately previous version. The third upgrade evicts the oldest.

```
packages/ripgrep/
├── 14.1.0/          ●  default (active)
└── 14.0.3/             previous (rollback target)
```

### Transaction log

```sql
CREATE TABLE transactions (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  ts        INTEGER NOT NULL,
  kind      TEXT NOT NULL,            -- add | upgrade | remove | default | rollback
  packages  TEXT NOT NULL,            -- JSON: [{name, from_ver, to_ver}, ...]
  reverted  INTEGER DEFAULT 0
);
```

### Rollback semantics

- `upgrade` rollback → flip default symlink to previous version (~100ms).
- `add` rollback → remove package; auto-installed deps go to `autoremove` candidacy.
- `remove` rollback → V2 (re-fetch if bottle missing from cache).
- `default` rollback → flip default to prior value.

### Limits

- No previous on disk → `Nothing to rollback for X`.
- `autoclean` warns: "After cleanup, rollback will not be available for these packages."
- No redo (forward) in V1; after rollback, a subsequent `upgrade` is a fresh transaction.

## 11. Error handling & UX

### Format

```
error: package "ripgep" not found

  Did you mean:
    ripgrep    Fast search tool (grep alternative)
    ripgrep-all

  Try `olma search rip` to see more.
```

- Line 1: `error:` prefix + plain statement.
- Suggestion block: did-you-mean (substring first, then Levenshtein ≤ 2 against the cached `formula.json` dump).
- Footer: next-action hint.

### Did-you-mean degradation

If full dump never fetched (`olma update --full` not run): literal lookup only, footer: `"Run \`olma update --full\` for search suggestions."`.

### Common errors → exit codes

See §8.

### Output styles

- **Default (TTY):** minimal soft, braille spinner, single replacing line.
- **`-v`:** per-stage timestamped log.
- **`-q`:** errors only.
- **`--json`:** structured result, no progress.

Color is always emitted (cyan spinner, green ✓, red ✗). There is no `--no-color` opt-out — the design decision is that olma owns its visual identity.

## 12. Project structure (Rust)

Single crate `olma` (workspace deferred). Source tree:

```
src/
├── main.rs
├── cli/{mod,add,remove,list,info,search,update,upgrade,outdated,
│       default,clean,autoclean,autoremove,purge,rollback,history,
│       self_update}.rs
├── metadata/{mod,client,ghcr}.rs
├── resolver/mod.rs
├── pipeline/{mod,download,verify,extract,relocate,link}.rs
├── relocator/{mod,macho,text,classify}.rs
├── state/{mod,schema}.rs
├── fs_lock.rs
├── cache.rs
├── output/{mod,soft,verbose,json}.rs
├── error.rs
├── config.rs
└── platform.rs
tests/
├── integration/{add_ripgrep,add_with_deps,upgrade_rollback,
│               relocation_python,...}.rs
└── helpers/sandbox.rs
```

### Dependencies

`clap` (derive), `tokio` (rt-multi-thread, fs, macros), `reqwest` (rustls, gzip, http2), `serde`, `serde_json`, `sha2`, `tar`, `flate2`, `rusqlite` (bundled), `fs2`, `console` (or `crossterm`), `indicatif`, `walkdir`, `memchr`, `anyhow`, `dirs`.

Excluded V1: `tracing`, `clap_complete` (installer script generates completions), YAML/TOML parsers.

## 13. Testing

### Layers

- **Unit (`cargo test`):** resolver (graph, cycles, diamonds), cache (ETag policy), relocator (mock fs + path replace), state.db schema, did-you-mean.
- **Integration (real bottles in sandboxed `OLMA_ROOT=/tmp/...`):**
  1. `ripgrep` — minimal, few deps.
  2. `python` — many `.dylib`, `.pc`, shebang scripts, many deps.
  3. `openssl@3` — heavily-depended; many `.cmake`.
  4. `git` — large text-relocation surface.
  5. `node` + `node@20` — versioned formula coexistence.
- **End-to-end:**
  - `add` → `upgrade` (mocked formula bump) → `rollback` → version restored.
  - `add python` + `add python@3.11` → both coexist → `remove python` → `python@3.11` remains.

### CI matrix

GitHub Actions: `macos-14` (arm64, Sonoma), `macos-13` (x64, Ventura). Tahoe self-hosted in V2.

### Network testing

V1: hit `formulae.brew.sh` and `ghcr.io` with retries. V2: fixture bottles cached in S3 for hermetic runs.

### Coverage target

Unit ≥ 80% (relocator is critical). Integration covers every CLI verb plus all listed error paths.

## 14. V1 scope summary

In scope:
- All formulas in homebrew-core (libraries arrive as auto-deps).
- macOS arm64 + x64 across Ventura, Sonoma, Sequoia, Tahoe.
- All commands in §8.
- Bottle relocation via shell-out to `install_name_tool` + `codesign`.
- Rollback with N=2 generations.

Deferred to V2:
- Native Mach-O rewriting (no shell-out).
- Custom taps.
- Casks / GUI apps.
- Bandwidth-adaptive download concurrency.
- Resumable downloads.
- `--no-wait` flag.
- `remove` rollback (re-fetch when bottle is gone).
- Redo (forward generations).
- Self-hosted Tahoe CI runner.
- `--force-link` for symlink collisions.

## 15. Open questions

None blocking. The following are minor and can be decided during implementation:

1. **`indicatif` vs hand-rolled spinner.** Either works for the minimal-soft style; pick whichever lands less binary weight.
2. **`console` vs `crossterm`.** Both handle TTY detect; `console` is simpler and likely sufficient.
3. **`state.db` migration tooling.** Hand-rolled `PRAGMA user_version` ladder is fine for V1's small schema.
