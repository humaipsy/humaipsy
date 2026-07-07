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
    vault.rs                  # master-password gate (Argon2id) + encrypted Vault blob
    timer.rs                  # trial countdown timer used by the experiment UI
    window_icon.rs             # platform-split: Win32 SendMessage icons vs egui IconData
    ollama/                   # Ollama HTTP client, chat engine, model discovery
    recording/                # experiment_csv (per-trial rows), ledger, audit, timing, metrics
    ui/                        # egui panels: settings, design, overview chat, etc.
      overview_chat/            # chat UI + SQLite-backed message store (rusqlite, "chat.sqlite")
  assets/                    # window/topbar icon PNGs (source of truth; .ico is generated)
  vault/.master_hash          # gitignored — local Argon2id hash file, created on first run
  metrics/                    # gitignored except structure — CSV exports + chat.sqlite land here
distribute_app.sh / .bat    # build release + stage a standalone dist/ folder (see below)
run_app.sh / .bat           # `cargo clean && cargo run` — quick dev loop
scripts/sign_windows_release.ps1  # optional Authenticode signing, invoked by distribute_app.bat
signing/windows-signing.env.example  # copy to windows-signing.env (gitignored) to enable signing
.github/workflows/release-linux.yml # tag-triggered Linux release build + GitHub Release upload
```

There is currently **no Windows release workflow** — only Linux is automated via CI.
Windows distribution today is manual, via `distribute_app.bat`. Do not add a Windows
release workflow or change the `dist/` naming scheme (e.g. `humai-0.1.2-win`,
`humai-0.1.2-ubu`) without being asked — that's an intentional pending decision.

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
  other. Windows additionally supports Authenticode signing
  (`scripts/sign_windows_release.ps1`), which has no Linux equivalent — that's fine,
  don't try to add code-signing to the Linux script.
- **Line endings are enforced via `.gitattributes`**: `*.sh` is forced to LF, `*.bat`/
  `*.ps1` to CRLF, regardless of a contributor's local `core.autocrlf`. If you add new
  shell or batch scripts, make sure `.gitattributes` covers them — a `.sh` file
  checked in with CRLF will fail to run on Linux/CI (bad interpreter).
- Paths should be built with `PathBuf`/`Path::join`, never string concatenation with a
  literal `/` or `\`.

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

`rust-toolchain.toml` pins `channel = "stable"`. Edition is 2024 (`rustc >= 1.85`), which
enables let-chains (`if let Some(x) = y && cond`) — this is the idiomatic style already
used throughout the codebase for what would otherwise be nested `if let { if ... }`.

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

## Ollama integration

- `OLLAMA_HOST` (default `http://127.0.0.1:11434`) and `OLLAMA_MODEL` select the
  inference target; both can be changed at runtime from the Settings panel, which sets
  the process env var so downstream code picks it up immediately.
- `HUMAI_AIR_GAP=1` disables all outbound HTTP; `HUMAI_ALLOW_LOCAL_OLLAMA=0` additionally
  blocks the loopback Ollama call. This is enforced at request time in `ollama/mod.rs`
  (`guard_ollama_request`), not just hidden in the UI — don't bypass it when adding new
  HTTP call sites.

## Experiment recording

- Per-trial rows are appended to `metrics/<ExpName>.csv` (configurable via
  `HUMAI_METRICS_DIR`); the header/schema is in `apps/humai/src/recording/experiment_csv.rs`.
  Adding a new CSV column is a breaking format change for anyone with existing exported
  data — treat it as such (bump a version marker, don't just insert a column).
- The Overview chat's message history is a separate SQLite database at
  `metrics/chat.sqlite` (`apps/humai/src/ui/overview_chat/store.rs` +
  `store_schema.rs`), independent of the CSV experiment log.

## Things to leave alone unless explicitly asked

- Do not merge branches or push to `main` without being asked.
- Do not add a Windows release CI workflow or introduce the
  `humai-<version>-win` / `humai-<version>-ubu` dist-naming scheme — pending a decision.
- Do not run a blanket `cargo fmt` across the whole tree. The codebase predates a
  consistent formatting pass; a full reformat would bury real changes in noise. Match
  the surrounding file's existing style when editing, and only fix formatting in lines
  you're already touching.
- `apps/humai/metrics/*.csv`, `metrics/chat.sqlite`, and `vault/.master_hash` are
  gitignored local/experiment artifacts — never add them to git.
