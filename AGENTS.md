# AGENTS.md

Guide for AI coding agents (and humans) working in this repository. `CLAUDE.md` at the
repo root points here — this file is the canonical reference.

## What this is

Humai is a cross-platform (Windows + Linux) desktop application for running Human-AI
Interaction / HCI experiments. It is a single Rust binary built with `eframe`/`egui`,
talks to a local or remote [Ollama](https://ollama.com) server for model inference, and
records experiment trials to CSV plus a local SQLite chat log. There is no server
component and no build step beyond `cargo` — the whole product is `apps/humai`.

License is proprietary (see [README.md](README.md)); do not add third-party code or
dependencies without checking the license is compatible.

## Repository layout

```
Cargo.toml                 # workspace root, members = ["apps/humai"]
apps/humai/                # the only workspace member — the actual app
  Cargo.toml                # package manifest (version is the release version, e.g. 0.1.2)
  build.rs                  # Windows-only: generates the .ico and embeds it as a resource
  src/
    main.rs                  # entry point: Tokio runtime + eframe::run_native
    lib.rs                   # module tree / re-exports (pub use ... as the crate's public API)
    app_model.rs              # `Humai` struct: top-level in-memory app state
    app_state.rs               # `AppState`: shared, lockable runtime config (metrics)
    agent.rs                  # `HumaiAgent`: prompt construction + calls into ollama::
    vault.rs                  # master-password gate (Argon2id) + encrypted Vault blob —
                                #   pure logic, no egui/UI dependency (rendering lives in
                                #   ui/vault_panel.rs; keep it that way, see below)
    timer.rs                  # trial countdown timer used by the experiment UI
    window_icon.rs             # platform-split: Win32 SendMessage icons vs egui IconData
    ollama/                   # Ollama HTTP client, chat engine, model discovery
    recording/                # experiment_csv (per-trial rows), ledger, audit, timing, metrics
    ui/                        # egui panels: settings, design, overview chat, vault unlock UI
      overview_chat/            # chat UI + SQLite-backed message store (rusqlite, "chat.sqlite")
      vault_panel.rs            # renders MasterVault; the only file that may import egui
                                #   *and* crate::vault together
  assets/                    # window/topbar icon PNGs (source of truth; .ico is generated)
  vault/.master_hash          # gitignored — local Argon2id hash file, created on first run
  metrics/                    # gitignored except structure — CSV exports + chat.sqlite land here
distribute_app.sh / .bat    # build release + stage a standalone dist/ folder (see below)
run_app.sh / .bat           # `cargo clean && cargo run` — quick dev loop
.github/workflows/ci.yml               # runs on every push to main + every PR: check/test/clippy on Linux + Windows
.github/workflows/release-linux.yml   # tag-triggered Linux release build + GitHub Release upload
.github/workflows/release-windows.yml # tag-triggered Windows release build + GitHub Release upload
```

Both release workflows fire on a `v*` tag push and upload to the GitHub Release for
that tag: `release-linux.yml` produces `humai-<version>-ubu.tar.gz`,
`release-windows.yml` produces `humai-<version>-win.zip` (`<version>` is the tag with
its leading `v` stripped, e.g. tag `v0.1.2` → `humai-0.1.2-ubu`/`humai-0.1.2-win`). Keep
these two workflows in parity the same way `distribute_app.sh`/`.bat` are kept in
parity — they both wrap the corresponding distribute script with a versioned
`HUMAI_DIST_DIR`, then archive and upload it.

## Cross-platform rules (read this before touching platform-sensitive code)

This app must build and behave the same on Windows and Linux. Concretely:

- **Never assume `HOME` is set.** Windows sets `USERPROFILE`, not `HOME`. Any code that
  needs the user's home directory must check both (see `copy_experiment_csv_to_desktop`
  in `apps/humai/src/ui/overview_chat/panel.rs` for the pattern:
  `std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))`).
- **Don't add dependencies that pull in vendored OpenSSL.** A leftover
  `openssl = { features = ["vendored"] } ` direct dependency used to make the app
  fail to build on plain Windows dev machines (`openssl-sys` tries to compile OpenSSL
  from source via Perl/NASM, which most Windows boxes don't have). It has been removed;
  `reqwest` uses `features = ["rustls-tls"]` and `rusqlite` uses `features = ["bundled"]`
  specifically so no system OpenSSL/pkg-config is required on either platform. If a new
  dependency wants to pull in `openssl`/`native-tls`, prefer a rustls-based alternative.
- **Fonts must be bundled, not loaded from OS-specific filesystem paths.** The chat UI
  needs Unicode emoji glyph coverage (Ollama responses can contain emoji), which egui's
  stock fonts don't provide. `apps/humai/src/ui/mod.rs` (`prepare_shell`) bundles
  `assets/NotoEmoji-Regular.ttf` via `include_bytes!` and registers it as a fallback in
  both font families — this works identically on Windows and Linux with no runtime
  dependency on fonts being installed. An earlier version of this code probed hardcoded
  `/usr/share/fonts/...` paths, which found nothing on Windows (no such paths) and was
  unreliable on Linux (path varies by distro) — don't reintroduce that pattern. Also
  note: egui/epaint's text rasterizer only supports single-color glyph outlines, not
  COLR/CBDT color tables, so this renders emoji as tinted monochrome pictographs, not
  full-color — that's a renderer limitation, not a bug in the font setup. See
  `ui::tests::bundled_emoji_font_covers_common_emoji_after_prepare_shell` for the
  regression test (uses `Fonts::has_glyphs` to prove real glyph resolution).
- **Platform-specific code lives behind `#[cfg(windows)]` / `#[cfg(not(windows))]`**,
  never behind a `target_os = "linux"` check unless macOS is explicitly meant to be
  excluded too. See `window_icon.rs` and `Cargo.toml`'s `[target.'cfg(windows)'...]`
  sections for the existing pattern — mirror it rather than inventing a new one.
- **`distribute_app.sh` and `distribute_app.bat` must stay in parity.** Both build the
  release binary, stage it into a dist folder with `assets/` and (if present)
  `vault/.master_hash`, and respect a `HUMAI_DIST_DIR` env var override. If you change
  what one script stages or how it names things, make the equivalent change in the
  other. Neither binary is code-signed — the Windows build will trigger a SmartScreen
  "unknown publisher" warning; that's a known, accepted state, not a bug to silently
  work around. Don't add signing scripts/CI steps unless explicitly asked.
- **Line endings are enforced via `.gitattributes`**: `*.sh` is forced to LF, `*.bat`/
  `*.ps1` to CRLF, regardless of a contributor's local `core.autocrlf`. If you add new
  shell or batch scripts, make sure `.gitattributes` covers them — a `.sh` file
  checked in with CRLF will fail to run on Linux/CI (bad interpreter).
- Paths should be built with `PathBuf`/`Path::join`, never string concatenation with a
  literal `/` or `\`.

## Locking

Never write `.lock().unwrap()` / `.read().unwrap()` / `.write().unwrap()` — that
propagates a *single* panic while a lock was held into a permanent crash on every
subsequent call to that lock (some of these locks are touched every UI frame). Use
`apps/humai/src/sync_ext.rs`'s `MutexExt`/`RwLockExt` instead:
`mutex.lock_recover()`, `rwlock.read_recover()`/`.write_recover()`. Every `Mutex`/`RwLock`
in the codebase goes through these; a new one that doesn't is a regression.

## Build / run / test

All commands are run from `apps/humai/` (or via the root-level `run_app.sh`/`.bat` /
`distribute_app.sh`/`.bat` wrappers, which `cd` into it for you).

```sh
# Fast compile check
cargo check --all-targets

# Run the app (debug build; vault lock is skipped by default in debug — see vault.rs)
cargo run

# Run tests
cargo test

# Lint — CI-equivalent gate, must pass clean
cargo clippy --all-targets -- -D warnings

# Regenerate vault/.master_hash from the command line (prompts for a password)
cargo run --bin gen_master_hash

# Release build
cargo build --release
```

`rust-toolchain.toml` pins `channel = "stable"` and `components = ["clippy", "rustfmt"]`,
so a fresh `rustup show` in this repo provisions everything the workflow above needs.
Edition is 2024 (`rustc >= 1.85`), which enables let-chains (`if let Some(x) = y && cond`)
— this is the idiomatic style already used throughout the codebase for what would
otherwise be nested `if let { if ... }`.

`[profile.release]` (LTO, `codegen-units = 1`, `strip = true`) lives in the **root**
`Cargo.toml`, not `apps/humai/Cargo.toml` — Cargo only reads profile settings from the
workspace root and silently ignores them in member manifests, so don't move it. Panic
strategy is deliberately left at the default (`unwind`), not `abort`: the app spawns
Tokio tasks (`rt.spawn`) for Ollama requests, and Tokio isolates a panicking task via
unwind + `catch_unwind` so the rest of the app keeps running; `panic = "abort"` would
turn any single bad task panic into a full process crash.

`.github/workflows/ci.yml` runs `cargo check`/`test`/`clippy -D warnings` on Linux and
Windows for every push to `main` and every PR — this is the gate that catches breakage
before a release tag is ever cut. Keep it green.

### Distributing a standalone build

```sh
# Linux
./distribute_app.sh                 # stages to $HOME/Desktop/humai by default
HUMAI_DIST_DIR=/custom/path ./distribute_app.sh

# Windows
distribute_app.bat                  # stages to %USERPROFILE%\Desktop\humai by default
set HUMAI_DIST_DIR=C:\custom\path && distribute_app.bat
```

Both scripts: build `--release`, wipe/recreate the dist dir, copy the binary + `assets/`,
and copy `vault/.master_hash` into `dist/vault/` if it exists locally (otherwise the
distributed app starts with no master password configured and prompts to create one on
first launch — this is expected, not a bug).

## The vault / master password gate

- Release builds always enforce the Argon2id master-password lock. Debug builds skip it
  by default (set `HUMAI_ENABLE_PASSWORD_LOCK=1` to test lock behavior in a debug build,
  or `HUMAI_SKIP_VAULT=1` to force-skip even if the lock env var is set).
- The hash lives at `vault/.master_hash` (relative to CWD) or can be supplied via
  `HUMAI_MASTER_HASH`. This file is gitignored — never commit a real hash.
- KDF tuning (`HUMAI_ARGON2_M_KIB`, `HUMAI_ARGON2_T`, `HUMAI_ARGON2_P`) and the
  ChaCha20-Poly1305 vault blob logic are in `apps/humai/src/vault.rs`; see the module
  doc comment at the top of that file for the full env-var list.
- `vault.rs` must not depend on `egui`/`eframe`. It's crypto and state (`MasterVault`,
  `Vault`, KDF params); rendering lives in `ui/vault_panel.rs`, which talks to it only
  through `pub(crate)` accessors (`is_skip_vault`, `needs_password_setup`, `status`,
  `master_input_mut`, `confirm_input_mut`, `try_unlock`, `try_setup_password`). If you
  need the UI to observe new state, add an accessor rather than making the fields
  `pub` or pulling `egui` back into `vault.rs`.

## Ollama integration

- `OLLAMA_HOST` (default `http://127.0.0.1:11434`) and `OLLAMA_MODEL` select the
  inference target; both can be changed at runtime from the Settings panel, which sets
  the process env var so downstream code picks it up immediately.
- `HUMAI_AIR_GAP=1` disables all outbound HTTP; `HUMAI_ALLOW_LOCAL_OLLAMA=0` additionally
  blocks the loopback Ollama call. This is enforced at request time in `ollama/mod.rs`
  (`guard_ollama_request`), not just hidden in the UI — don't bypass it when adding new
  HTTP call sites. Note this is opt-in, not the default: with air-gap off (the default),
  there's no restriction on what host `OLLAMA_HOST` points to, so a remote host receives
  full experiment transcripts. That's intentional (remote-Ollama is a supported use
  case), just be aware of it.
- Both `reqwest::Client` construction sites (`ollama/client.rs`, `ollama/engine.rs`) set
  `connect_timeout` so an unreachable host fails fast. `ollama/client.rs` (model
  listing) also sets a short overall `timeout` since that request should always be
  quick. `ollama/engine.rs` (chat inference) deliberately does *not* set a tight overall
  timeout — a legitimate generation can stream for minutes — but caps it generously (10
  minutes) as a backstop against a connection that goes silent forever after accepting.
  Don't tighten the inference client's timeout without checking it won't cut off real
  long-running generations.

## Experiment recording

- Per-trial rows are appended to `metrics/<ExpName>.csv` (configurable via
  `HUMAI_METRICS_DIR`); the header/schema is in `apps/humai/src/recording/experiment_csv.rs`.
  Adding a new CSV column is a breaking format change for anyone with existing exported
  data — treat it as such (bump a version marker, don't just insert a column).
- The CSV export filename is derived from `design.name`/`design.csv_name`, both
  free-text UI fields — `csv_path_for_design` runs them through `sanitize_file_stem`
  (strips `/`, `\`, `:`, control chars, and leading `.`) before ever joining them into a
  path. That's the *only* place this sanitization happens; if you add another code path
  that turns a design field into a filesystem path, sanitize there too rather than
  assuming the UI already validated it.
- The Overview chat's message history is a separate SQLite database at
  `metrics/chat.sqlite` (`apps/humai/src/ui/overview_chat/store.rs` +
  `store_schema.rs`), independent of the CSV experiment log. `Store::open` sets
  `busy_timeout` + WAL mode, restricts the file to `0600` on Unix, and `Store::conn()`
  recovers from a poisoned mutex instead of panicking — use `self.conn()` for any new
  method, never `self.conn.lock().unwrap()` directly. Multi-statement writes (see
  `append_message`) must go through `conn.transaction()`, not sequential `execute()`
  calls, so a crash mid-write can't leave a message row without its generation row or a
  stale `conversations.updated_at`.
- Schema changes are "ensure column exists" (`ensure_conversations_columns` in
  `store_schema.rs`), not real stepped migrations — fine for adding a nullable/defaulted
  column, not sufficient for a column type change or a data backfill. If the schema
  needs that kind of change, branch on `PRAGMA user_version` properly rather than
  extending the idempotent-column-add pattern past what it's good for.
- `vault/.master_hash` is also restricted to `0600` on Unix (`vault.rs::restrict_to_owner`)
  — it's an Argon2id password hash, readable-by-anyone-on-the-box was the previous state.

## Release process

Releases are tagged directly on `main`, not from a dedicated release branch — that's
Git Flow-style overhead this project doesn't need (it earns its keep only once you must
maintain multiple release lines in parallel, e.g. patching an old version while `main`
has diverged; not the case here). The flow: bump `version` in `apps/humai/Cargo.toml`
(and let `Cargo.lock` update) on its own short-lived branch, PR it into `main` like any
other change, then tag `vX.Y.Z` at that merge commit on `main` and push the tag — that
fires `release-linux.yml`/`release-windows.yml`, which build and upload
`humai-<version>-ubu.tar.gz`/`humai-<version>-win.zip` to the tag's GitHub Release.

## Things to leave alone unless explicitly asked

- Do not merge branches or push to `main` without being asked.
- Do not push a `v*` tag or run `gh release create`/`gh release upload` yourself —
  that cuts a real, public GitHub Release and is the user's call, not something to do
  as a side effect of an unrelated change.
- Do not run a blanket `cargo fmt` across the whole tree. The codebase predates a
  consistent formatting pass; a full reformat would bury real changes in noise. Match
  the surrounding file's existing style when editing, and only fix formatting in lines
  you're already touching.
- `apps/humai/metrics/*.csv`, `metrics/chat.sqlite`, and `vault/.master_hash` are
  gitignored local/experiment artifacts — never add them to git.
