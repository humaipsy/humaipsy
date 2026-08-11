# Architecture

This document describes how Humai is put together: process model, module boundaries,
data flow, storage formats, and the main runtime subsystems. For contributor-facing
rules (cross-platform gotchas, build commands, things to leave alone), see
[AGENTS.md](AGENTS.md). For the one-paragraph pitch, see [README.md](README.md).

## Summary

Humai is a single-binary, offline-first desktop application for running structured
Human-AI Interaction (HCI) experiments. A researcher configures a between-subjects
experiment design (participant ID, topic, per-trial time limit), runs participants
through a fixed sequence of timed chat trials against a locally- or remotely-hosted
LLM served by [Ollama](https://ollama.com), and the app records every trial as a row
in a CSV file plus the full chat transcript in a local SQLite database. There is no
backend service, no telemetry, and no required network dependency beyond the Ollama
host the operator configures.

The whole product is one Cargo workspace member, `apps/humai`, built with
[`eframe`/`egui`](https://github.com/emilk/egui) (immediate-mode GUI) and a
background [Tokio](https://tokio.rs) runtime for async HTTP calls to Ollama.

## Process model

```
main() [main.rs]
  ├─ spawns a dedicated OS thread running a Tokio multi-thread runtime
  │    (kept alive for the process lifetime; used for all Ollama HTTP calls
  │     and model-list refreshes, via rt_handle.spawn(...))
  └─ eframe::run_native(...) on the calling thread
       └─ HumaiApp::update() — called once per UI frame
            ├─ vault gate (MasterVault) — blocks all UI until unlocked
            ├─ Humai::render_main_panel() — tab dispatch (Overview / Design / Settings)
            └─ background task results are drained from Arc<Mutex<..>> /
               mpsc channels populated by Tokio tasks, then ctx.request_repaint()
               wakes the UI thread
```

egui is immediate-mode: every frame rebuilds the widget tree from current state.
Long-running work (Ollama inference, model listing) never happens on the UI thread —
it's spawned onto the Tokio runtime via `Handle::spawn`, writes its result into a
shared `Arc<Mutex<T>>` (or an `mpsc` channel for streamed chat messages), and calls
`ctx.request_repaint()` so the next UI frame picks it up. This is the same
send/poll pattern used throughout `ui/settings_panel.rs` and
`ui/overview_chat/panel.rs`.

## Module map

```
apps/humai/src/
  main.rs              Process entry point: Tokio runtime bootstrap, eframe::run_native
  lib.rs                Module tree / public re-exports (pub use ... as the crate API)
  app_model.rs           Humai: top-level in-memory app state (selected model, prompts,
                          active tab, theme, HTTP policy flags, metrics config)
  app_state.rs            AppState: Arc<RwLock<..>>-guarded runtime config shared across
                          UI and background tasks (currently just MetricsConfig)
  agent.rs                HumaiAgent: builds instruction/input prompts for two call
                          shapes (opening question, memory-conditioned reply) and calls
                          ollama::send_to_ollama; sentence-count clamping on output
  vault.rs                 Master-password gate (Argon2id) + encrypted Vault blob
                          (ChaCha20-Poly1305). No egui/eframe dependency by design.
  timer.rs                 Process-global trial countdown timer (start/pause/reset)
  window_icon.rs            Platform-split viewport/taskbar icon setup (Win32 vs egui)
  sync_ext.rs                MutexExt/RwLockExt: poison-recovering lock accessors used
                          everywhere instead of `.lock().unwrap()`
  ollama/                  Ollama HTTP client + streaming chat engine
    mod.rs                  Public API: send_to_ollama, fetch_ollama_models, test_ollama,
                          HttpPolicy (air-gap enforcement), timing/usage types
    client.rs                GET /api/tags — model discovery, tightly timeboxed
    engine.rs                 POST /api/chat — streaming NDJSON chat completion,
                          per-host client cache, TTFT + token-usage capture
  recording/                Experiment data capture
    experiment_csv.rs         Per-trial-event CSV writer (the primary experiment
                          artifact) — process-global recorder state, one row per event
    ledger.rs                  Append-only JSONL event ledger + run summary/manifest
                          hashing (scaffolding for a future audit-bundle feature; not
                          currently wired into the UI/experiment flow)
    audit.rs                   Generic JSON-line audit log writer (also not currently
                          wired in; shared by future ledger/audit consumers)
    timing.rs                   RFC3339 timestamp helpers, InferenceTraceContext
    metrics.rs                  Thin re-export shim: `crate::metrics` == experiment_csv's
                          public types, kept as a stable import path for the rest of
                          the crate
  ui/                       egui rendering, one module per panel
    mod.rs                    HumaiApp (the eframe::App impl), theme + font bootstrap,
                          vault-gated top-level frame dispatch
    panel_render.rs            Top-level tab bar (Overview / Design / Settings) + frame
    settings_panel.rs           Ollama host/model selection, connection test, CSV
                          recording toggle/output dir, Catppuccin theme picker
    vault_panel.rs               Renders MasterVault state; the only file allowed to
                          import both egui and crate::vault (see vault.rs docs)
    overview_chat/                Experiment engine + chat UI (the largest module)
      mod.rs                       Re-exports render_overview_panel / render_design_panel
                          / render_experiment_modals
      panel.rs                      Trial sequencing, condition assignment/counter-
                          balancing, timeout handling, final-answer capture, CSV export
                          shortcut, chat room UI, participant-ID modal (~2k lines)
      chat.rs                        ChatExample: in-memory chat/room state, UiInbox
                          (mpsc-based background-to-UI message channel)
      incoming.rs                     MessageSource enum (Human/Api/System)
      store.rs / store_schema.rs        SQLite persistence for chat transcripts
                          (rusqlite, chat.sqlite), idempotent schema migration
  bin/
    gen_master_hash.rs         Standalone binary: interactively writes
                          vault/.master_hash (used in CI-less local setup and by
                          distribute_app scripts)
```

## Experiment engine

The experiment model (`ui/overview_chat/panel.rs`) is a fixed 2×2 within-subject
design: two conditions (`COOP` — cooperative, `ARGUM` — argumentative), two trials per
condition, four trial windows total. Condition order is counterbalanced by parity of
the participant ID's trailing digits (even IDs run COOP→ARGUM→COOP→ARGUM, odd IDs run
the reverse) so consecutive participants don't share an order bias.

Each condition maps to a distinct system prompt (`HUMAI_COOP_AGENT_PROMPT` /
`HUMAI_ARGUM_AGENT_PROMPT`, overridable via env var, defaults set in
[`app_model.rs`](apps/humai/src/app_model.rs)) that `HumaiAgent` prepends to every
inference call for that trial. Trial state — current condition/trial index, timeout
popup, "awaiting final answer" flag, completion flag — lives in a process-global
`ExperimentSessionState` behind a `Mutex` (module-level `OnceLock`, not app-model
state), because it needs to be reachable from both the egui render pass and the
async completion callback of a spawned Ollama request.

Each trial:
1. is timed by the global `timer` module (start/pause/reset, backed by a
   `Mutex<MainTimerState>`, displayed live in the sidebar),
2. streams a chat transcript through `ChatExample`/`Store` (see below),
3. ends on either participant timeout or explicit trial completion, and
4. emits `TrialStart` / `Transcript` / `TrialEnd` / `TrialTotal` rows to the
   experiment CSV via `recording::experiment_csv`.

After the fourth trial the participant is prompted for a free-text final answer
(`record_final_answer`), the experiment is marked complete, and the operator can copy
the just-written CSV to the Desktop directly from the completion popup
(`copy_experiment_csv_to_desktop`, cross-platform via `HOME`/`USERPROFILE`).

## Ollama integration

`ollama::send_to_ollama` (and the result-returning `send_to_ollama_with_result`) is
the single entry point every caller (agent.rs, settings test button, opening-question
generation) goes through. Internally:

- `guard_ollama_request` enforces the air-gap policy (`HUMAI_AIR_GAP`,
  `HUMAI_ALLOW_LOCAL_OLLAMA`) at the point of every outbound request, not just in the
  UI — so a new call site can't accidentally bypass it.
- `engine::InferenceEngine` is cached per normalized host (`OnceLock<Mutex<HashMap<..>>>`)
  so repeated calls to the same Ollama host reuse one `reqwest::Client` /
  connection pool instead of reconnecting every turn.
- Chat completion streams NDJSON from `POST /api/chat` line-by-line, accumulating
  content, capturing time-to-first-token (TTFT), and reading `prompt_eval_count`/
  `eval_count` for token usage — all surfaced back as an `InferenceTimingSnapshot`
  that flows straight into the experiment CSV row for that turn.
- Model listing (`GET /api/tags`, used by the Settings model picker) uses a separate,
  tightly-timeboxed client so an unreachable host fails fast instead of hanging the
  Settings panel.
- A cooperative stop mechanism (`OllamaStopEpoch`, an `Arc<AtomicU64>` + captured
  epoch value) lets the UI cancel an in-flight generation by bumping a counter; the
  streaming loop checks it between chunks and between lines.

## Storage

Two independent on-disk artifacts, both under the app's working directory by default
and both gitignored:

| Artifact | Format | Location | Purpose |
|---|---|---|---|
| Experiment log | CSV, one row per event | `metrics/<ExpName>.csv` (`HUMAI_METRICS_DIR`) | Primary statistical-analysis output: one append-only file per experiment design, header defined in `recording/experiment_csv.rs`. Filenames are derived from free-text UI fields and passed through `sanitize_file_stem` to block path traversal (`../`, absolute paths, drive prefixes). |
| Chat transcript | SQLite (WAL) | `metrics/chat.sqlite` | Full message history per "room" (conversation), independent of the CSV log — lets the operator review/replay conversations. `conversations` / `messages` / `generations` tables; `Store::append_message` writes across all three inside one transaction so a mid-write crash can't leave an orphaned row. |
| Master password hash | One-line Argon2id PHC string | `vault/.master_hash` | Gate on app startup in release builds; regenerated via `cargo run --bin gen_master_hash`. |

Both `chat.sqlite` and `vault/.master_hash` are restricted to `0600` on Unix at
creation time (no-op on Windows, where per-user profile ACLs already isolate them).

Two additional recording primitives exist in `recording/` — an append-only JSONL
event ledger (`ledger.rs`) with SHA-256 run-summary hashing, and a generic audit-log
writer (`audit.rs`) — but neither is currently invoked from the experiment flow or
UI. They're scaffolding for a future reproducible-audit-bundle feature; treat them as
inactive until something calls `EventLedger::open`/`AuditHandle::open`.

## Security model

- **Master-password gate**: Argon2id-hashed password required to unlock the app in
  release builds (`vault.rs`), with exponential backoff on repeated failures. Debug
  builds skip this by default for faster iteration (`HUMAI_ENABLE_PASSWORD_LOCK=1` to
  opt in). A `Vault` container (ChaCha20-Poly1305, HKDF-derived key) exists for future
  encrypted-payload storage but has no current caller beyond its own round-trip test.
- **Air-gap mode** (`HUMAI_AIR_GAP=1`): blocks all outbound HTTP by default; a second
  flag (`HUMAI_ALLOW_LOCAL_OLLAMA`) can still permit loopback-only Ollama traffic.
  Enforced centrally in `ollama::guard_ollama_request`, checked before every request.
  This is opt-in — with air-gap off (the default), `OLLAMA_HOST` can point anywhere,
  including a remote host that will receive full experiment transcripts.
- **Poison-safe locking**: every `Mutex`/`RwLock` in the codebase goes through
  `sync_ext`'s `lock_recover`/`read_recover`/`write_recover` instead of
  `.lock().unwrap()`, so one panicking task can't permanently wedge a lock used on
  the render path.
- **Path sanitization**: experiment CSV filenames (user-editable "design name" /
  "CSV name" fields) are sanitized to a single safe path component before ever being
  joined onto `output_dir` (see `sanitize_file_stem` in `experiment_csv.rs`).
- **File permissions**: the master-password hash and the chat SQLite database are
  both `chmod 600` on Unix at creation time.

## UI shell

Built on `eframe`/`egui` 0.33 with the [Catppuccin](https://github.com/catppuccin)
color themes (Latte/Frappé/Macchiato/Mocha, user-selectable in Settings) and
[Phosphor icons](https://github.com/amPerl/egui-phosphor). A bundled monochrome
Unicode emoji font (`assets/NotoEmoji-Regular.ttf`, `include_bytes!`) is installed as
a fallback in both font families at startup so Ollama responses containing emoji
render identically on Windows and Linux without depending on OS-installed fonts
(egui's rasterizer only supports single-color glyphs, so emoji render tinted
monochrome, not full-color — a renderer limitation, not a bug).

Three top-level tabs (`app_model::MainPanelTab`):
- **Overview** — the live chat/experiment room, sidebar (timer, participant ID, room
  management), and the message thread itself.
- **Design** — experiment design fields (name, participant ID, topic, trial count,
  trial duration) that feed `ExperimentDesignSnapshot`.
- **Settings** — Ollama host/model configuration and connectivity test, CSV recording
  toggle and output directory, theme picker.

## Build & release

- Single Cargo workspace, one member (`apps/humai`); `[profile.release]` (LTO,
  `codegen-units = 1`, `strip = true`) lives in the workspace-root `Cargo.toml`
  because Cargo only reads profile settings from the root.
- `build.rs` is Windows-only: generates a multi-resolution `.ico` from
  `assets/icon_window_bar.png` and embeds it as a Win32 resource via `winres`.
- `reqwest` uses `rustls-tls` and `rusqlite` uses the `bundled` SQLite — both
  deliberately chosen to avoid a system OpenSSL/pkg-config dependency on Windows dev
  machines.
- CI (`.github/workflows/ci.yml`) runs `cargo check` / `cargo test` / `cargo clippy
  -D warnings` on Linux, Windows, and macOS for every push to `main` and every PR.
- Tag-triggered release workflows (`release-linux.yml`, `release-windows.yml`,
  `release-macos.yml`) build `--release` and upload
  `humai-<version>-{ubu,win,mac}.{tar.gz,zip}` to the tag's GitHub Release;
  `distribute_app.sh`/`.bat` do the equivalent for local standalone builds. The macOS
  build targets Apple Silicon (arm64) only — GitHub's `macos-latest` runner moved off
  Intel in late 2023; an Intel/x86_64 build would need a second job or a
  `lipo`-merged universal binary.

See [AGENTS.md](AGENTS.md) for exact commands, environment variables, and the
cross-platform rules that constrain how this code can evolve.
