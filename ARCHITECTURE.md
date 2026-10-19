# Architecture

This document describes how HumaiPsy is put together: process model, module boundaries,
data flow, storage formats, and the main runtime subsystems. For contributor-facing
rules (cross-platform gotchas, build commands, things to leave alone), see
[AGENTS.md](AGENTS.md). For the one-paragraph pitch, see [README.md](README.md). For
what's actually protected — and what isn't — about experiment data, see
[SECURITY.md](SECURITY.md).

## Summary

HumaiPsy is a desktop application for running structured Human-AI Interaction (HCI)
experiments, built as three Cargo workspace members: **`humaipsy-core`**, a UI-free
library crate holding the engine (agent prompting, the Ollama client, experiment
recording, the chat SQLite store, the master-password vault); **`apps/humaipsy`**, the
`eframe`/`egui` desktop binary that renders the UI on top of it; and
**`humaipsy-web`**, a small local read-only browser dashboard served from within the
desktop app's own process. A researcher configures an experiment design (participant
ID, topic, per-trial time limit), runs participants through a timed chat trial
sequence against a locally- or remotely-hosted LLM served by
[Ollama](https://ollama.com), and the app records every trial as a row in a CSV file,
a mirrored JSONL event ledger, and the full chat transcript in a local SQLite
database. There is no separate backend service and no telemetry; the only inbound
network exposure is the optional dashboard, bound to loopback by default (see
[Local web dashboard](#local-web-dashboard) below), and the only outbound one is the
Ollama host the operator configures.

The `humaipsy-core`/`apps/humaipsy` split exists specifically so the engine can be
exercised without booting a GUI window — unit tests import `humaipsy-core` directly, and
it's what `humaipsy-web` (and a future CLI batch-runner) depend on too.

## Process model

```
main() [apps/humaipsy/src/main.rs]
  ├─ spawns a dedicated OS thread running a Tokio multi-thread runtime
  │    (kept alive for the process lifetime; used for all humaipsy_core::ollama HTTP
  │     calls, model-list refreshes, and the humaipsy-web dashboard server, all via
  │     rt_handle.spawn(...) — one runtime, no second process)
  └─ eframe::run_native(...) on the calling thread
       └─ HumaiPsyApp::update() — called once per UI frame
            ├─ vault gate (humaipsy_core::vault::MasterVault) — blocks all UI until unlocked
            ├─ HumaiPsy::render_main_panel() — tab dispatch (Overview / Design / Settings)
            └─ background task results are drained from Arc<Mutex<..>> /
               mpsc channels populated by Tokio tasks, then ctx.request_repaint()
               wakes the UI thread
```

egui is immediate-mode: every frame rebuilds the widget tree from current state.
Long-running work (Ollama inference, model listing) never happens on the UI thread —
it's spawned onto the Tokio runtime via `Handle::spawn`, writes its result into a
shared `Arc<Mutex<T>>` (or an `mpsc` channel for streamed chat messages), and calls
`ctx.request_repaint()` so the next UI frame picks it up.

## Crate map

```
crates/humaipsy-core/          Library crate — zero egui/eframe dependency
  src/
    lib.rs                    Module tree / public API
    agent.rs                  HumaiPsyAgent: builds instruction/input prompts for two call
                              shapes (opening question, memory-conditioned reply) and calls
                              provider::Provider::chat; sentence-count clamping on output
    chat_types.rs               Pure chat data types shared by the Store and the UI's
                              ChatExample: ChatMessage, Room, MessageSource,
                              AssistantGeneration, MessageCorrelation, UiInbox<T>
    experiment/                  Trial-sequencing/counterbalancing engine — pure,
                              parameterized functions (no process-global state), unit
                              tested directly. See "Experiment engine" below.
    provider/                      Backend-agnostic chat-completion abstraction —
                              see "Provider abstraction" below. HumaiPsyAgent depends on
                              this, not on ollama:: directly.
      mod.rs                        Provider trait (id, chat, list_models), ChatRequest,
                              ChatResponse
      ollama_provider.rs             OllamaProvider — the only impl today, a thin
                              adapter over ollama:: below
      token_budget.rs                 Approximate (character-based) conversation-
                              history trimming, provider-agnostic
    ollama/                        Ollama HTTP client + streaming chat engine — unchanged
                              by the provider abstraction, still the only thing that
                              actually speaks HTTP to a model backend
      mod.rs                        Public API: send_to_ollama, fetch_ollama_models,
                              test_ollama, HttpPolicy (air-gap enforcement),
                              DEFAULT_OLLAMA_MODEL, timing/usage types
      client.rs                      GET /api/tags — model discovery, tightly timeboxed
      engine.rs                       POST /api/chat — streaming NDJSON chat completion,
                              per-host client cache, TTFT + token-usage capture
    recording/                        Experiment data capture
      experiment_csv.rs                Per-trial-event CSV writer + JSONL event-ledger
                              mirror (see "Experiment recording" below)
      ledger.rs                         Append-only JSONL event ledger + SHA-256
                              run-summary/manifest hashing
      timing.rs                          RFC3339 timestamp helpers, UUID id generation,
                              InferenceTraceContext
      metrics.rs                          Thin re-export shim: `humaipsy_core::metrics` ==
                              experiment_csv's public types, a stable import path
    store.rs / store_schema.rs           SQLite persistence for chat transcripts
                              (rusqlite, chat.sqlite), idempotent schema migration
    sync_ext.rs                            MutexExt/RwLockExt poison-recovering lock
                              accessors, used by all three crates
    timer.rs                                Process-global trial countdown timer
                              (start/pause/reset)
    vault.rs                                 Master-password gate (Argon2id)

crates/humaipsy-web/            Library crate — local read-only browser dashboard.
  Cargo.toml                  Depends on humaipsy-core only (not apps/humaipsy) — same
                              one-directional dependency shape as humaipsy-core itself,
                              so it could run standalone later even though today
                              it's always spawned embedded in apps/humaipsy.
  web/                          Plain HTML/JS/CSS, no build step, no framework —
                              embedded into the binary at compile time (include_str!),
                              not served from a filesystem path (see routes.rs)
  src/
    lib.rs                       pub async fn serve(WebServerConfig) — binds a
                              TcpListener and runs axum::serve until the process exits
    status.rs                     WebStatusSnapshot (serde Serialize) + the
                              Arc<Mutex<..>> alias apps/humaipsy writes into
    routes.rs                      GET / (+ /app.js, /style.css, embedded), GET
                              /api/status, GET /api/messages (reads humaipsy_core::store::Store
                              directly — no new Store methods needed)

apps/humaipsy/                  UI binary — depends on humaipsy-core and humaipsy-web as path dependencies
  src/
    main.rs                    Process entry point: Tokio runtime bootstrap, eframe::run_native
    lib.rs                      Module tree / re-exports (pub use ... as the crate's public API)
    app_model.rs                  HumaiPsy: top-level in-memory app state (selected model,
                              prompts, active tab, theme, HTTP policy flags, metrics config,
                              web_status/web_dashboard_addr — see Local web dashboard below)
    app_state.rs                    AppState: Arc<RwLock<..>>-guarded runtime config shared
                              across UI and background tasks (currently just MetricsConfig)
    window_icon.rs                    Platform-split viewport/taskbar icon setup (Win32 vs egui)
    ui/                                  egui rendering, one module per panel
      mod.rs                              HumaiPsyApp (the eframe::App impl), theme + font
                              bootstrap, vault-gated top-level frame dispatch;
                              HumaiPsyUiState also holds the persisted egui_tiles::Tree
                              for the Overview tab's resizable layout
      panel_render.rs                       Top-level tab bar (Overview / Design / Settings)
      settings_panel.rs                       Ollama host/model selection, connection test,
                              CSV recording toggle/output dir, dashboard URL, theme picker
      vault_panel.rs                            Renders MasterVault state; the only file
                              allowed to import both egui and humaipsy_core::vault together
      overview_chat/                              Experiment engine UI + chat rendering
        panel.rs                                    Trial sequencing orchestration (delegates
                              its math to humaipsy_core::experiment), condition/timeout handling,
                              CSV export shortcut, chat room UI, participant-ID modal, the
                              egui_tiles OverviewPane/Behavior for the sidebar/chat split.
                              Orchestration state (OverviewChatUiState: current trial, chat
                              rooms, participant ID, modal state) is a field on HumaiPsyUiState,
                              threaded through render calls explicitly — not process-global.
        chat.rs                                       ChatExample: in-memory chat/room
                              rendering state (sidebar_ui, room management), UiInbox-based
                              background-to-UI message channel; owns the rename-room modal's
                              state directly as a field
  bin/
    gen_master_hash.rs               Standalone binary: interactively writes
                              vault/.master_hash, depends only on humaipsy_core::vault
```

## Experiment engine

The pure trial-sequencing math lives in `humaipsy_core::experiment` — no process-global
state, no UI dependency, fully unit tested (see `crates/humaipsy-core/src/experiment/mod.rs`).
It implements a within-subject design: up to [`MAX_CONDITIONS`] = 2 named conditions
(`COOP` — Cooperative, `ARGUM` — Argumentative), each mapped to a distinct system
prompt (`HUMAIPSY_COOP_AGENT_PROMPT` / `HUMAIPSY_ARGUM_AGENT_PROMPT` env vars, defaults in
`apps/humaipsy/src/app_model.rs`), with a researcher-configurable number of trials per
condition. Condition order is counterbalanced by parity of the participant ID's
trailing digits (even IDs run COOP→ARGUM, odd IDs run ARGUM→COOP) so consecutive
participants don't share an order bias.

The Design tab's "Conditions" control is clamped to `1..=2` (with a tooltip
explaining why) rather than accepting an arbitrary count that the engine would
silently ignore — HumaiPsy only has two named condition prompts today, so a design
asking for more can't actually be honored yet (see [Roadmap](#roadmap--next-steps)
for generalizing this). "Trials per condition" has no such ceiling and is fully
wired through to `humaipsy_core::experiment::total_experiment_trials`.

`apps/humaipsy/src/ui/overview_chat/panel.rs` owns trial *orchestration* — timing via
the `timer` module, streaming a chat transcript through `ChatExample`/`Store`, and
emitting `TrialStart`/`Transcript`/`TrialEnd`/`TrialTotal` rows via
`recording::experiment_csv` — but delegates every piece of counterbalancing/
condition-assignment math to `humaipsy_core::experiment` rather than reimplementing it.
This orchestration state (current trial index, timeout popup, participant ID, chat
room list) lives on `OverviewChatUiState`, a field of `HumaiPsyUiState` owned by
`HumaiPsyApp` and threaded through render calls explicitly — not process-global state,
so it's scoped to one `HumaiPsyApp` instance rather than the whole process.

After the last trial the participant is prompted for a free-text final answer, the
experiment is marked complete, and the operator can copy the just-written CSV to the
Desktop directly from the completion popup (`copy_experiment_csv_to_desktop`,
cross-platform via `HOME`/`USERPROFILE`).

## Provider abstraction

`humaipsy_core::agent::HumaiPsyAgent` — the thing that actually builds prompts and asks
for a completion — depends on `humaipsy_core::provider::Provider`, not on Ollama by
name. `Provider` is a small `async-trait` trait (`id`, `chat`, `list_models`); the
only implementation today is `provider::ollama_provider::OllamaProvider`, a thin
adapter that calls straight into the `ollama::` module below with no new HTTP logic
of its own. Adding a second backend (OpenAI, Anthropic, ...) later means writing a
new `impl Provider`, not touching `agent.rs` or the two `HumaiPsyAgent::new(...)` call
sites in `apps/humaipsy/src/ui/overview_chat/panel.rs` — both construct an
`Arc<dyn Provider>` (currently always an `OllamaProvider`) and hand it to the agent.
No second backend exists yet; this is a seam, not a feature.

`ChatRequest.max_output_tokens` is the "cut the tokens" control promoted to the
trait level — it maps directly onto Ollama's existing `num_predict` generation-limit
option, so every future provider takes the same field instead of inventing its own.
On the input side, `provider::token_budget::trim_to_token_budget` is a second,
optional, *approximate* trim `HumaiPsyAgent::with_token_budget(n)` can apply on top of
the existing message-count-based `history_size` window — approximate because there's
no real tokenizer wired up for any backend here (Ollama models and any future
OpenAI/Anthropic backend all tokenize differently); it's a ~4-characters-per-token
heuristic, documented as such, not a guarantee of fitting a specific context window.
Calling neither of these keeps behavior identical to before the abstraction existed.

## Ollama integration

`ollama::send_to_ollama` (and the result-returning `send_to_ollama_with_result`) is
the single entry point every caller (agent.rs, settings test button, opening-question
generation) goes through. Internally:

- `guard_ollama_request` enforces the air-gap policy (`HUMAIPSY_AIR_GAP`,
  `HUMAIPSY_ALLOW_LOCAL_OLLAMA`) at the point of every outbound request, not just in the
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
- The model to use is always passed explicitly by the caller
  (`HumaiPsy::selected_ollama_model`, threaded through `HumaiPsyAgent`); `engine.rs` only
  falls back to `ollama::DEFAULT_OLLAMA_MODEL` — a plain constant, not an env-var
  read — when no model has been configured yet. There is exactly one legitimate
  env-var read of `OLLAMA_MODEL` in the whole codebase, in `HumaiPsy::new()`, to seed
  the *initial* selection at startup; nothing reads it again after that, so there is
  only one source of truth for "which model is selected" at runtime.

## Storage

Three on-disk artifacts, all under the app's working directory by default and all
gitignored:

| Artifact | Format | Location | Purpose |
|---|---|---|---|
| Experiment log | CSV, one row per event | `metrics/<ExpName>.csv` (`HUMAIPSY_METRICS_DIR`) | Primary statistical-analysis output: one append-only file per experiment design, header defined in `recording/experiment_csv.rs`. Filenames are derived from free-text UI fields and passed through `sanitize_file_stem` to block path traversal (`../`, absolute paths, drive prefixes). |
| Event ledger | JSONL, one line per event + `summary.json` | `metrics/runs/<experiment_id>/<run_id>/events.jsonl` | Reproducible-audit-trail mirror of the CSV: every CSV row `experiment_csv::emit()` writes is also appended here (same event name, SHA-256-hashed input/output) via `recording::ledger::EventLedger`, opened lazily per (experiment_id, run_id) and finalized (`system.run_stopped` + `summary.json`, event counts, first/last timestamp) when the experiment ends. |
| Chat transcript | SQLite (WAL) | `<metrics output dir>/chat.sqlite` | Full message history per "room" (conversation), independent of the CSV/ledger. `conversations` / `messages` / `generations` tables; `Store::append_message` writes across all three inside one transaction so a mid-write crash can't leave an orphaned row. Opened once and cached (`Arc<Store>`, keyed by path) rather than reopened every UI frame — see [Known Limitations](#known-limitations) for the one remaining case (a changed output dir mid-session) this cache doesn't handle perfectly. |
| Master password hash | One-line Argon2id PHC string | `vault/.master_hash` | Gate on app startup in release builds; regenerated via `cargo run --bin gen_master_hash`. |

Both `chat.sqlite` and `vault/.master_hash` are restricted to `0600` on Unix at
creation time (no-op on Windows, where per-user profile ACLs already isolate them).

## Security model

- **Master-password gate**: Argon2id-hashed password required to unlock the app in
  release builds (`vault.rs`), with exponential backoff on repeated failures. Debug
  builds skip this by default for faster iteration (`HUMAIPSY_ENABLE_PASSWORD_LOCK=1` to
  opt in).
- **Air-gap mode** (`HUMAIPSY_AIR_GAP=1`): blocks all outbound HTTP by default; a second
  flag (`HUMAIPSY_ALLOW_LOCAL_OLLAMA`) can still permit loopback-only Ollama traffic.
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
render identically on Windows, Linux, and macOS without depending on OS-installed
fonts (egui's rasterizer only supports single-color glyphs, so emoji render tinted
monochrome, not full-color — a renderer limitation, not a bug).

Three top-level tabs (`app_model::MainPanelTab`):
- **Overview** — the live chat/experiment room. The sidebar (timer, participant ID,
  room management) and the message thread are two `egui_tiles` panes with a
  user-draggable divider (`OverviewPane`/`OverviewPanesBehavior` in `panel.rs`),
  not a fixed-width split.
- **Design** — experiment design fields (name, participant ID, topic, condition
  count, trials-per-condition, trial duration) that feed `ExperimentDesignSnapshot`.
- **Settings** — Ollama host/model configuration and connectivity test, CSV recording
  toggle and output directory, the web dashboard URL, theme picker.

## Local web dashboard

`crates/humaipsy-web` is a small `axum` server, spawned by `HumaiPsy::new()` onto the same
Tokio runtime `main.rs` already keeps alive — not a second process, not a WASM build
of the egui UI. It serves a plain HTML/JS/CSS page (`crates/humaipsy-web/web/`, embedded
into the binary at compile time) that polls two JSON endpoints every ~1.5s:

- `GET /api/status` — a `WebStatusSnapshot` (design name, participant ID, condition,
  trial progress, running state, timer) that `apps/humaipsy`'s `panel.rs` updates at the
  same points it already mutates its trial-session state
  (`update_web_status`, called from `start_experiment_from_play`,
  `advance_to_next_trial`, `finalize_experiment`)
- `GET /api/messages` — the most-recently-updated conversation's messages, read
  straight from the same SQLite `Store` the desktop app writes to
  (`Store::list_conversations`/`load_messages`, unchanged). No new state, no push —
  WAL mode means this concurrent read never blocks the app's writes.

**It is read-only by design.** There is no endpoint that can start a trial, send a
message, or otherwise mutate app state — see the Roadmap if that's ever wanted.
Bound to `127.0.0.1:8787` by default (`HUMAIPSY_WEB_BIND` to change, `HUMAIPSY_WEB_DISABLED=1`
to turn it off entirely) — experiment transcripts are potentially sensitive
human-subjects data, so inbound exposure gets the same opt-in caution the outbound
`HUMAIPSY_AIR_GAP` policy already applies to Ollama traffic. A bind failure (e.g. the
port is already in use) is logged to stderr; the desktop app starts regardless.

**Gated by the master-password vault, not independently authenticated.** The
dashboard has no login of its own — instead, `HumaiPsyApp::update` (`ui/mod.rs`) mirrors
`MasterVault::is_unlocked()` into a shared `humaipsy_web::SharedVaultLock` flag every
frame, and `get_status`/`get_messages` check it before touching any state, returning
blanked/empty responses while locked. Locking the desktop app (the Settings "Lock"
button) stops the dashboard from serving transcript content within one frame. This
was a real gap during development — the dashboard initially had no relationship to
the vault at all — caught and fixed before merge, not after. See
[SECURITY.md](SECURITY.md) for the full security/privacy write-up, including what this
gating does *not* protect against (the dashboard still has no auth of its own; the
loopback-only bind is the actual barrier once the app is unlocked).

## Known limitations

Honest gaps as of this writing — not hidden, so the next change to touch this code
knows what it's standing on:

- **HumaiPsy only supports two named conditions** (Cooperative/Argumentative), each with
  a single env-var-configured prompt. The Design tab's "Conditions" control is
  honestly clamped to this rather than silently ignored, but a researcher wanting a
  3-condition study can't author one without a code change.
- **The chat-store cache doesn't react to a changed output directory mid-session.**
  If an operator changes the metrics output folder in Settings while a chat room is
  already open, the already-cached `Store` for the old path keeps being used until
  restart (a new path gets its own cache entry, but nothing evicts the old one or
  moves in-flight state). Low practical impact — the output dir is not commonly
  changed mid-run — but worth knowing.
- **macOS release build is Apple Silicon (arm64) only** — see AGENTS.md's release-
  process section.
- **The web dashboard's chat DB path is fixed at startup.** Like the desktop-side
  `Store` cache above, `humaipsy-web`'s `chat_db_path` is computed once in
  `spawn_web_dashboard` from the metrics config at launch; changing the output
  directory in Settings later doesn't move the dashboard's read target. Same low
  practical impact, same root cause, not yet fixed for either.
- **The dashboard is read-only and has no CLI counterpart yet.** `humaipsy-web`/
  `humaipsy-core` make both structurally possible (see Roadmap), but an interactive
  (write) dashboard and a headless CLI runner are both still just ideas, not code.

## Roadmap / next steps

Rough priority order, highest-value first:

1. A headless experiment runner (`humaipsy-core` + a small CLI) for scripted/CI-driven
   regression testing of prompts and trial logic without a GUI.
2. Generalize `humaipsy_core::experiment` beyond exactly two named conditions to an
   arbitrary researcher-authored set, with a matching Design-tab UI for entering each
   condition's prompt — turning "Conditions: N" from a clamped, honest control into a
   fully general one.
3. Push updates for the web dashboard (SSE/WebSocket) instead of polling, and/or
   write endpoints (send a message, advance a trial) — `OverviewChatUiState` being a
   real, addressable field (rather than private statics inside `panel.rs`) removes
   the structural blocker this used to have; this is now an `humaipsy-web`-side scoping
   question, not something waiting on `apps/humaipsy` first.
4. A real hostname/TLS story for the dashboard (`https://humaipsy.local` via mDNS, or a
   self-signed localhost cert) if plain `http://localhost:8787` stops being enough.
5. An Intel (`x86_64-apple-darwin`) macOS release build, or a `lipo`-merged universal
   binary, if Intel Mac users are a real audience.

## Build & release

- Three-member Cargo workspace (`apps/humaipsy`, `crates/humaipsy-core`, `crates/humaipsy-web`);
  `[profile.release]` (LTO, `codegen-units = 1`, `strip = true`) lives in the
  workspace-root `Cargo.toml` because Cargo only reads profile settings from the root.
- `apps/humaipsy/build.rs` is Windows-only: generates a multi-resolution `.ico` from
  `assets/icon_window_bar.png` and embeds it as a Win32 resource via `winres`.
- `reqwest` uses `rustls-tls` and `rusqlite` uses the `bundled` SQLite — both
  deliberately chosen to avoid a system OpenSSL/pkg-config dependency on Windows dev
  machines. Both live in `humaipsy-core`; `apps/humaipsy` only adds UI-specific
  dependencies (`eframe`, `egui`, `catppuccin-egui`, `egui-phosphor`, `egui_tiles`,
  the Windows icon toolchain) on top of `humaipsy-core`/`humaipsy-web` path dependencies.
  `humaipsy-web` itself adds `axum` — chosen over alternatives (Rocket, actix-web)
  specifically because `hyper`/`tower` are already resolved transitively via
  `reqwest`, so it shares the existing async HTTP stack instead of introducing a
  second one.
- CI (`.github/workflows/ci.yml`) runs `cargo check` / `cargo test` / `cargo clippy
  -D warnings --workspace` on Linux, Windows, and macOS for every PR (not on push to
  `main` — merges land via squash-merged PRs that already ran these checks).
- Tag-triggered release workflows (`release-linux.yml`, `release-windows.yml`,
  `release-macos.yml`) build `--release` and upload
  `humaipsy-<version>-{ubu,win,mac}.{tar.gz,zip}` to the tag's GitHub Release;
  `distribute_app.sh`/`.bat` do the equivalent for local standalone builds — both
  build from `apps/humaipsy/`, and Cargo resolves the `humaipsy-core` path dependency as
  part of that build automatically, so the workspace split needed no changes to
  either distribute script.

See [AGENTS.md](AGENTS.md) for exact commands, environment variables, and the
cross-platform rules that constrain how this code can evolve.
