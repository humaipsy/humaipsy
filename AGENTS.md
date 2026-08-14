# AGENTS.md

Guide for AI coding agents (and humans) working in this repository. `CLAUDE.md` at the
repo root points here — this file is the canonical reference.

## What this is

Humai is a cross-platform (Windows + Linux + macOS) desktop application for running
Human-AI Interaction / HCI experiments. It's a three-crate Cargo workspace:
`humai-core` is a UI-free library (agent prompting, the Ollama client, experiment
recording, the chat SQLite store, the master-password vault); `apps/humai` is the
`eframe`/`egui` desktop binary built on top of it; `humai-web` is a small local
read-only browser dashboard, spawned by `apps/humai` in-process, that serves plain
HTML/JS/CSS plus a JSON API reading the same chat `Store`. The app talks to a local
or remote [Ollama](https://ollama.com) server for model inference, and records
experiment trials to CSV plus a mirrored JSONL event ledger plus a local SQLite chat
log. There is no separate backend service and no build step beyond `cargo`.

Licensed under AGPL-3.0-only (see [LICENSE](LICENSE)); do not add a dependency
whose license is incompatible with the AGPL (e.g. code under a "no derivatives" or
similarly restrictive license) without checking first.

For a narrative walkthrough of how these modules fit together — process model, the
experiment engine's trial/condition logic, storage formats, security model, known
limitations, and the roadmap — see [ARCHITECTURE.md](ARCHITECTURE.md). For what's
actually protected (and what isn't) about experiment data — network boundaries, the
vault, file permissions, what's plaintext at rest — see [PRIVACY.md](PRIVACY.md).
Any change that touches the vault, air-gap policy, the web dashboard's gating, or
file permissions should keep that document accurate; it's read by people deciding
whether this tool is safe to use with real participant data.

## Repository layout

```
Cargo.toml                       # workspace root, members = ["apps/humai", "crates/humai-core", "crates/humai-web"]
crates/humai-core/                 # library crate — zero egui/eframe dependency
  Cargo.toml
  src/
    lib.rs                          # module tree / public API
    agent.rs                         # `HumaiAgent`: prompt construction + calls into
                                       #   provider::Provider::chat (not ollama:: directly)
    chat_types.rs                      # ChatMessage/Room/MessageSource/AssistantGeneration/
                                       #   MessageCorrelation/UiInbox — shared by store.rs and
                                       #   apps/humai's ChatExample (which does the egui rendering)
    experiment/                          # pure trial-sequencing/counterbalancing engine —
                                       #   parameterized, unit-tested, no process-global state.
                                       #   apps/humai/src/ui/overview_chat/panel.rs delegates its
                                       #   condition/trial math here instead of hardcoding it.
    provider/                              # backend-agnostic chat-completion trait +
                                       #   token budgeting. ollama_provider.rs is the only
                                       #   impl today — see "Adding a new provider" below.
    ollama/                                # Ollama HTTP client, chat engine, model discovery —
                                       #   unchanged by the provider abstraction; still the
                                       #   only thing that speaks HTTP to a model backend
    recording/                              # experiment_csv (CSV writer + JSONL ledger mirror),
                                       #   ledger (JSONL event log + summary/manifest hashing —
                                       #   wired into experiment_csv, not standalone), timing, metrics
    store.rs / store_schema.rs               # SQLite chat store (rusqlite, "chat.sqlite")
    sync_ext.rs                                # MutexExt/RwLockExt poison-recovering lock
                                       #   accessors — used by all three crates, see Locking below
    timer.rs                                    # trial countdown timer used by the experiment UI
    vault.rs                                     # master-password gate (Argon2id) — no
                                       #   egui/UI dependency (rendering lives in
                                       #   apps/humai/src/ui/vault_panel.rs; keep it that way)
crates/humai-web/                  # library crate — local read-only browser dashboard
  Cargo.toml                        # depends on humai-core only, not apps/humai
  web/                                # plain HTML/JS/CSS, no build step — embedded via include_str!
                                     #   (routes.rs), not served from a filesystem path at runtime
  src/
    lib.rs                              # `pub async fn serve(WebServerConfig)`, spawned by
                                       #   apps/humai onto its existing Tokio runtime
    status.rs                            # WebStatusSnapshot + the Arc<Mutex<..>> apps/humai writes into
    routes.rs                             # GET /, /api/status, /api/messages (reads humai_core::store::Store)
apps/humai/                      # the UI binary — depends on humai-core and humai-web as path dependencies
  Cargo.toml                        # package manifest (version is the release version, e.g. 0.1.3)
  .cargo/config.toml                 # target-dir = "target" so `cargo` works from this dir without -p
  build.rs                            # Windows-only: generates the .ico and embeds it as a resource
  src/
    main.rs                              # entry point: Tokio runtime + eframe::run_native
    lib.rs                                # module tree / re-exports (pub use ... as the crate's public API)
    app_model.rs                            # `Humai` struct: top-level in-memory app state, incl.
                                       #   web_status/web_dashboard_addr and spawn_web_dashboard()
    app_state.rs                             # `AppState`: shared, lockable runtime config (metrics)
    window_icon.rs                            # platform-split: Win32 SendMessage icons vs egui IconData
    ui/                                        # egui panels: settings, design, overview chat, vault unlock UI
      mod.rs                                    # HumaiUiState also holds the persisted egui_tiles::Tree
                                       #   for the Overview tab's resizable sidebar/chat split
      overview_chat/                             # experiment engine UI + chat rendering (largest module,
                                       #   panel.rs is ~2k lines) — see "Known state-management debt" below
      vault_panel.rs                              # renders MasterVault; the only file that may import egui
                                       #   *and* humai_core::vault together
    bin/
      gen_master_hash.rs                            # standalone binary: interactively writes
                                       #   vault/.master_hash; depends only on humai_core::vault
  assets/                            # window/topbar icon PNGs + bundled NotoEmoji-Regular.ttf (source of
                                     #   truth; the Windows .ico is generated from icon_window_bar.png)
  vault/.master_hash                  # gitignored — local Argon2id hash file, created on first run
  metrics/                            # gitignored except structure — CSV/ledger exports + chat.sqlite land here
distribute_app.sh / .bat            # build release + stage a standalone dist/ folder (see below)
run_app.sh / .bat                   # `cargo clean && cargo run` — quick dev loop
.github/workflows/ci.yml               # runs on every PR: check/test/clippy --workspace on Linux + Windows + macOS
.github/workflows/release-linux.yml   # tag-triggered Linux release build + GitHub Release upload
.github/workflows/release-windows.yml # tag-triggered Windows release build + GitHub Release upload
.github/workflows/release-macos.yml   # tag-triggered macOS (arm64) release build + GitHub Release upload
```

All three release workflows fire on a `v*` tag push and upload to the GitHub Release
for that tag: `release-linux.yml` produces `humai-<version>-ubu.tar.gz`,
`release-windows.yml` produces `humai-<version>-win.zip`, `release-macos.yml` produces
`humai-<version>-mac.tar.gz` (`<version>` is the tag with its leading `v` stripped,
e.g. tag `v0.1.2` → `humai-0.1.2-{ubu,win,mac}`). The macOS build runs on GitHub's
`macos-latest` runner, which is Apple Silicon (arm64) — there is currently no
Intel/x86_64 macOS build; add a second job to `release-macos.yml` (or a
`lipo`-merged universal binary) if that's ever needed. Keep these three workflows in
parity the same way `distribute_app.sh`/`.bat` are kept in parity — Linux and macOS
both wrap `distribute_app.sh` (it's plain POSIX bash, no Linux-specific bits) and
Windows wraps `distribute_app.bat`, each with a versioned `HUMAI_DIST_DIR`, then
archive and upload the result. Both scripts build from `apps/humai/`; Cargo resolves
the `humai-core`/`humai-web` path dependencies as part of that build automatically,
so the workspace split needed no changes to either script. `humai-web`'s static
assets are embedded into the binary at compile time (not copied as files), so
nothing needs staging into `dist/` for the dashboard to work in a distributed build.

## `humai-core` / `apps/humai` / `humai-web`: where new code goes

- **If it doesn't need `egui`/`eframe`, it belongs in `humai-core`.** Business logic
  (prompt construction, trial sequencing, recording, the chat store, the vault) lives
  there so it can be unit tested directly and so `humai-web` (and a future CLI) can
  depend on it without pulling in a GUI toolkit. `crates/humai-core/src/experiment/mod.rs`
  is the reference example: pure functions taking their inputs as parameters, no
  `OnceLock`/global state, straightforward to test.
- **If it renders (`egui::Ui`, `egui::Context`, ...), it belongs in `apps/humai`.**
  `apps/humai/src/ui/overview_chat/chat.rs`'s `ChatExample` is the reference example
  of the seam: the pure data types it operates on (`ChatMessage`, `Room`, ...) live in
  `humai_core::chat_types`, but `ChatExample` itself and its `sidebar_ui` rendering
  method stay in the UI crate.
- **If it's an HTTP route or anything the browser dashboard serves, it belongs in
  `humai-web`.** `humai-web` depends only on `humai-core`, never on `apps/humai` —
  don't add a dependency in that direction, it would invert the whole point of the
  split (a UI-free engine that a server can depend on). New read endpoints go in
  `crates/humai-web/src/routes.rs` and should read from `humai-core` state (the
  `Store`, or a snapshot type like `WebStatusSnapshot`) rather than trying to reach
  into `apps/humai`'s private UI-module statics — see "Known state-management debt"
  below for why that's not accessible today anyway. `apps/humai/src/app_model.rs`'s
  `spawn_web_dashboard`/`update_web_status` (called from `ui/overview_chat/panel.rs`
  at the trial-lifecycle points) is the pattern for feeding new status fields
  through: add to `WebStatusSnapshot`, populate it in `update_web_status`, done — no
  changes needed on the `humai-web` side for a field that's just carried through.
- **If it's a new model backend (OpenAI, Anthropic, ...), it's a new
  `impl Provider` in `crates/humai-core/src/provider/`, modeled on
  `ollama_provider.rs`.** Do not add a second field to `HumaiAgent` or an `if
  self.backend == ...` branch anywhere — the entire reason `Provider` exists is so
  `agent.rs` and its callers never need to know which backend is active. A new
  provider should call whatever HTTP/SDK client it needs itself (mirroring how
  `OllamaProvider` calls into `ollama::`, not by extending the `ollama` module to
  handle a second service) and honor `ChatRequest.max_output_tokens` the way
  `OllamaProvider` maps it onto Ollama's `num_predict`. As of this writing exactly
  one provider exists (Ollama) — this guidance is for when that changes, not a
  hint that more should be added speculatively now.
- **State-management pattern**: `apps/humai/src/ui/overview_chat/panel.rs` and
  `chat.rs` hold experiment/chat UI state (current trial, chat rooms, participant
  ID, modal open/closed flags) on `OverviewChatUiState`, a field of `HumaiUiState`
  owned by `HumaiApp`, threaded through render calls explicitly — this used to be
  process-global `OnceLock<Mutex<T>>` statics (predating the `humai-core` split);
  don't reintroduce that pattern. New per-app UI state should be a field on `Humai`
  (`app_model.rs`) or a small nested struct held on `HumaiApp` (see
  `HumaiUiState`/`OllamaUiState`/`OverviewChatUiState` in `ui/mod.rs` /
  `overview_chat/panel.rs` for the pattern to follow), threaded through render
  calls explicitly. The one `OnceLock`-based cache that *is* the right pattern is a
  genuine cross-cutting singleton keyed by something other than "the app started":
  `ollama::engine::InferenceEngine::from_host`'s per-host connection-pool cache, and
  `panel.rs`'s `shared_store` chat-database cache modeled after it — both are
  `OnceLock<Mutex<HashMap<Key, Arc<T>>>>` caches with a real cache-key, not a
  disguised global variable for state that has exactly one owner.

## Cross-platform rules (read this before touching platform-sensitive code)

This app must build and behave the same on Windows, Linux, and macOS. Concretely:

- **Never assume `HOME` is set.** Windows sets `USERPROFILE`, not `HOME`. Any code that
  needs the user's home directory must check both (see `copy_experiment_csv_to_desktop`
  in `apps/humai/src/ui/overview_chat/panel.rs` for the pattern:
  `std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))`).
- **Don't add dependencies that pull in vendored OpenSSL.** A leftover
  `openssl = { features = ["vendored"] } ` direct dependency used to make the app
  fail to build on plain Windows dev machines (`openssl-sys` tries to compile OpenSSL
  from source via Perl/NASM, which most Windows boxes don't have). It has been removed;
  `reqwest` (in `humai-core`) uses `features = ["rustls-tls"]` and `rusqlite` uses
  `features = ["bundled"]` specifically so no system OpenSSL/pkg-config is required on
  any platform. If a new dependency wants to pull in `openssl`/`native-tls`, prefer a
  rustls-based alternative.
- **Fonts must be bundled, not loaded from OS-specific filesystem paths.** The chat UI
  needs Unicode emoji glyph coverage (Ollama responses can contain emoji), which egui's
  stock fonts don't provide. `apps/humai/src/ui/mod.rs` (`prepare_shell`) bundles
  `assets/NotoEmoji-Regular.ttf` via `include_bytes!` and registers it as a fallback in
  both font families — this works identically on Windows, Linux, and macOS with no
  runtime dependency on fonts being installed. An earlier version of this code probed
  hardcoded `/usr/share/fonts/...` paths, which found nothing on Windows (no such
  paths) and was unreliable on Linux (path varies by distro) — don't reintroduce that
  pattern. Also note: egui/epaint's text rasterizer only supports single-color glyph
  outlines, not COLR/CBDT color tables, so this renders emoji as tinted monochrome
  pictographs, not full-color — that's a renderer limitation, not a bug in the font
  setup. See `ui::tests::bundled_emoji_font_covers_common_emoji_after_prepare_shell`
  for the regression test (uses `Fonts::has_glyphs` to prove real glyph resolution).
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
`humai_core::sync_ext`'s `MutexExt`/`RwLockExt` instead:
`mutex.lock_recover()`, `rwlock.read_recover()`/`.write_recover()`. Every `Mutex`/`RwLock`
in the codebase goes through these; a new one that doesn't is a regression.

## Build / run / test

All commands are run from `apps/humai/` (or via the root-level `run_app.sh`/`.bat` /
`distribute_app.sh`/`.bat` wrappers, which `cd` into it for you). Cargo resolves the
`humai-core` path dependency automatically, so these commands build/test both crates'
code as needed even though they're invoked from the binary crate's directory; to
target `humai-core` directly (e.g. running only its tests), pass `-p humai-core` or
run the same commands from `crates/humai-core/` instead. `default-members =
["apps/humai"]` in the workspace root `Cargo.toml` means a bare `cargo check`/`test`
run from the *repository root* only covers `apps/humai` — use `--workspace` from the
root if you want both crates.

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
otherwise be nested `if let { if ... }`. Because `channel = "stable"` floats forward,
a new stable Rust release can occasionally graduate a new `clippy`/`rustc` lint that
trips `-D warnings` on previously-fine code with zero changes on our side (this
happened with `float_literal_f32_fallback` — untyped float literals passed to a
generic `f32`-bound parameter). If CI goes red on `main` with no relevant diff,
check whether the lint is new before assuming a regression.

`[profile.release]` (LTO, `codegen-units = 1`, `strip = true`) lives in the **root**
`Cargo.toml`, not either member's manifest — Cargo only reads profile settings from the
workspace root and silently ignores them in member manifests, so don't move it. Panic
strategy is deliberately left at the default (`unwind`), not `abort`: the app spawns
Tokio tasks (`rt.spawn`) for Ollama requests, and Tokio isolates a panicking task via
unwind + `catch_unwind` so the rest of the app keeps running; `panic = "abort"` would
turn any single bad task panic into a full process crash.

`.github/workflows/ci.yml` runs `cargo check`/`test`/`clippy -D warnings --workspace`
on Linux, Windows, and macOS for every PR — this is the gate that catches breakage
before a merge, and therefore before a release tag is ever cut. Keep it green. It
does not also run on every push to `main`: merges land through squash-merged PRs
that already ran these checks against the same tree, so re-running on `main` would
just be duplicate work, not extra coverage. `--workspace` is load-bearing here — a
plain `cargo check`/`test`/`clippy` invoked from inside a single member's directory
(`apps/humai/` or `crates/humai-core/`) only covers that one package and silently
skips the other member's tests and lints.

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
- KDF tuning (`HUMAI_ARGON2_M_KIB`, `HUMAI_ARGON2_T`, `HUMAI_ARGON2_P`) is in
  `crates/humai-core/src/vault.rs`; see the module doc comment at the top of that
  file for the full env-var list.
- `vault.rs` must not depend on `egui`/`eframe` — it lives in `humai-core` specifically
  so it can't. It's crypto and state (`MasterVault`, KDF params); rendering
  lives in `apps/humai/src/ui/vault_panel.rs`, which talks to it only through `pub`
  accessors (`is_skip_vault`, `needs_password_setup`, `status`, `master_input_mut`,
  `confirm_input_mut`, `try_unlock`, `try_setup_password`). If you need the UI to
  observe new state, add an accessor rather than making the fields more visible than
  `pub(crate)` within `vault.rs` or pulling `egui` into `humai-core`.

## Ollama integration

- `OLLAMA_HOST` (default `http://127.0.0.1:11434`) and `OLLAMA_MODEL` select the
  inference target. `OLLAMA_HOST` can be changed at runtime from the Settings panel,
  which sets the process env var so downstream code picks it up immediately.
  `OLLAMA_MODEL` is read from the environment exactly once, in `Humai::new()`, to
  seed the *initial* model selection at startup — after that, the selected model is
  plain typed state (`Humai::selected_ollama_model`) passed explicitly into every
  inference call; nothing reads `OLLAMA_MODEL` from the environment again at
  runtime. Don't reintroduce a runtime `std::env::set_var("OLLAMA_MODEL", ..)` call
  as a way to propagate a UI change — that was removed specifically because it made
  the selected model exist in two unsynchronized places at once.
- If a caller doesn't have a model configured yet, `humai_core::ollama` falls back to
  the `DEFAULT_OLLAMA_MODEL` constant (`ollama/mod.rs`) — a plain string, not an
  env-var lookup.
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
  `HUMAI_METRICS_DIR`); the header/schema is in `crates/humai-core/src/recording/experiment_csv.rs`.
  Adding a new CSV column is a breaking format change for anyone with existing exported
  data — treat it as such (bump a version marker, don't just insert a column).
- The CSV export filename is derived from `design.name`/`design.csv_name`, both
  free-text UI fields — `csv_path_for_design` runs them through `sanitize_file_stem`
  (strips `/`, `\`, `:`, control chars, and leading `.`) before ever joining them into a
  path. That's the *only* place this sanitization happens; if you add another code path
  that turns a design field into a filesystem path, sanitize there too rather than
  assuming the UI already validated it.
- Every CSV row is also mirrored into a JSONL event ledger at
  `metrics/runs/<experiment_id>/<run_id>/events.jsonl` (`recording::ledger::EventLedger`,
  opened lazily by `experiment_csv::ensure_ledger` the first time a row is emitted for
  a given session, finalized — `system.run_stopped` + `summary.json` with SHA-256
  hashes and event counts — in `end_experiment()`). This used to be fully-implemented,
  fully-tested, and completely unused; it's wired in now, so a new CSV event type
  should get a corresponding ledger event automatically (both go through the shared
  `emit()`/`ledger_append()` path) — don't add a new call site that writes a CSV row
  without going through `emit()`.
- The Overview chat's message history is a separate SQLite database at
  `<metrics output dir>/chat.sqlite` (`crates/humai-core/src/store.rs` + `store_schema.rs`),
  independent of the CSV/ledger experiment log, and its path now flows through the
  same `MetricsConfig` output directory as the CSV instead of a separately-hardcoded
  path. `Store::open` sets `busy_timeout` + WAL mode, restricts the file to `0600` on
  Unix, and `Store::conn()` recovers from a poisoned mutex instead of panicking — use
  `self.conn()` for any new method, never `self.conn.lock().unwrap()` directly.
  Multi-statement writes (see `append_message`) must go through `conn.transaction()`,
  not sequential `execute()` calls, so a crash mid-write can't leave a message row
  without its generation row or a stale `conversations.updated_at`. The `Store` itself
  is opened at most once per path and cached (`apps/humai/src/ui/overview_chat/panel.rs`'s
  `shared_store`, an `Arc<Store>` keyed by `PathBuf`) — don't call `Store::open`
  directly from UI render code; go through `shared_store`/`chat_db_path` instead, or
  you'll reintroduce the every-frame-reopen bug that pattern fixed.
- Schema changes are "ensure column exists" (`ensure_conversations_columns` in
  `store_schema.rs`), not real stepped migrations — fine for adding a nullable/defaulted
  column, not sufficient for a column type change or a data backfill. If the schema
  needs that kind of change, branch on `PRAGMA user_version` properly rather than
  extending the idempotent-column-add pattern past what it's good for.
- `vault/.master_hash` is also restricted to `0600` on Unix (`vault.rs::restrict_to_owner`)
  — it's an Argon2id password hash, readable-by-anyone-on-the-box was the previous state.

## Local web dashboard

- `crates/humai-web` is spawned automatically by `Humai::new()`
  (`apps/humai/src/app_model.rs::spawn_web_dashboard`) onto the Tokio runtime
  `main.rs` already keeps alive — there is no separate server process to start or
  stop, and no way to run it without also running the desktop app (by design; see
  `crates/humai-web/src/lib.rs`'s doc comment if you want to change that).
- `HUMAI_WEB_BIND` (default `127.0.0.1:8787`) sets the bind address; must be a
  `host:port` socket address (`SocketAddr`), not a hostname. `HUMAI_WEB_DISABLED=1`
  turns the server off entirely. Both are read once, at `Humai::new()` — changing
  them requires a restart, same as `HUMAI_AIR_GAP`/`HUMAI_METRICS_DIR`.
- **The default bind is loopback-only and that default matters.** Experiment
  transcripts are potentially sensitive human-subjects data; don't change the
  default to `0.0.0.0` or otherwise widen exposure without it being an explicit,
  deliberate ask — this mirrors the same caution `HUMAI_AIR_GAP` already applies to
  outbound Ollama traffic, just for the inbound direction.
- The dashboard is **read-only** — `crates/humai-web/src/routes.rs` has no route
  that mutates anything. Keep it that way unless de-globalizing `panel.rs`'s
  experiment state (see "Known state-management debt" above) has actually happened
  first; a write endpoint today would have nothing safe to write to.
- `crates/humai-web/web/*.html`/`.js`/`.css` are embedded into the binary via
  `include_str!` in `routes.rs`, not served from a filesystem path — editing them
  requires a rebuild to take effect (no live-reload), and nothing needs to be staged
  into `dist/` by `distribute_app.sh`/`.bat` for the dashboard to work in a
  distributed build.
- **The dashboard is gated by the master-password vault, mirrored via a shared
  flag.** `HumaiApp::update` (`ui/mod.rs`) syncs `humai_core::vault::MasterVault::is_unlocked()`
  into `Humai::vault_unlocked` (`humai_web::SharedVaultLock`, an `Arc<AtomicBool>`)
  every frame; `routes::get_status`/`get_messages` check it first and withhold
  content when `false`. If you add a new route that reads any experiment data, check
  this flag the same way — don't assume the loopback bind alone is enough, since
  "locked" is a real state a user actively puts the app into and expects respected.
  See [PRIVACY.md](PRIVACY.md) for the full reasoning.

## Release process

Releases are tagged directly on `main`, not from a dedicated release branch — that's
Git Flow-style overhead this project doesn't need (it earns its keep only once you must
maintain multiple release lines in parallel, e.g. patching an old version while `main`
has diverged; not the case here). The flow: bump `version` in `apps/humai/Cargo.toml`
(and let `Cargo.lock` update) on its own short-lived branch, PR it into `main` like any
other change, then tag `vX.Y.Z` at that merge commit on `main` and push the tag — that
fires `release-linux.yml`/`release-windows.yml`/`release-macos.yml`, which build and
upload `humai-<version>-ubu.tar.gz`/`humai-<version>-win.zip`/`humai-<version>-mac.tar.gz`
to the tag's GitHub Release.

## Things to leave alone unless explicitly asked

- Do not merge branches or push to `main` without being asked.
- Do not push a `v*` tag or run `gh release create`/`gh release upload` yourself —
  that cuts a real, public GitHub Release and is the user's call, not something to do
  as a side effect of an unrelated change.
- Do not run a blanket `cargo fmt` across the whole tree. The codebase predates a
  consistent formatting pass; a full reformat would bury real changes in noise. Match
  the surrounding file's existing style when editing, and only fix formatting in lines
  you're already touching.
- `apps/humai/metrics/*.csv`, `metrics/runs/`, `metrics/chat.sqlite`, and
  `vault/.master_hash` are gitignored local/experiment artifacts — never add them to git.
- Don't add a new process-global `OnceLock<Mutex<T>>` singleton for state that has a
  natural owner (`Humai`, `HumaiApp`, or a small nested state struct). See "`humai-core`
  vs `apps/humai`: where new code goes" above — `panel.rs`/`chat.rs` used to do this;
  it's fixed now (see `OverviewChatUiState`), don't reintroduce the pattern.
