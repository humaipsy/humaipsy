# Architecture

This document describes how Humai is put together: process model, module boundaries,
data flow, storage formats, and the main runtime subsystems. For contributor-facing
rules (cross-platform gotchas, build commands, things to leave alone), see
[AGENTS.md](AGENTS.md). For the one-paragraph pitch, see [README.md](README.md).

## Summary

Humai is a desktop application for running structured Human-AI Interaction (HCI)
experiments, built as two Cargo workspace members: **`humai-core`**, a UI-free
library crate holding the engine (agent prompting, the Ollama client, experiment
recording, the chat SQLite store, the master-password vault), and **`apps/humai`**,
the `eframe`/`egui` desktop binary that renders the UI on top of it. A researcher
configures an experiment design (participant ID, topic, per-trial time limit), runs
participants through a timed chat trial sequence against a locally- or
remotely-hosted LLM served by [Ollama](https://ollama.com), and the app records every
trial as a row in a CSV file, a mirrored JSONL event ledger, and the full chat
transcript in a local SQLite database. There is no backend service, no telemetry, and
no required network dependency beyond the Ollama host the operator configures.

The `humai-core`/`apps/humai` split exists specifically so the engine can be
exercised without booting a GUI window — unit tests import `humai-core` directly, and
it's what a future CLI batch-runner or HTTP API would depend on too (see
[Roadmap](#roadmap--next-steps)).

## Process model

```
main() [apps/humai/src/main.rs]
  ├─ spawns a dedicated OS thread running a Tokio multi-thread runtime
  │    (kept alive for the process lifetime; used for all humai_core::ollama HTTP
  │     calls and model-list refreshes, via rt_handle.spawn(...))
  └─ eframe::run_native(...) on the calling thread
       └─ HumaiApp::update() — called once per UI frame
            ├─ vault gate (humai_core::vault::MasterVault) — blocks all UI until unlocked
            ├─ Humai::render_main_panel() — tab dispatch (Overview / Design / Settings)
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
crates/humai-core/          Library crate — zero egui/eframe dependency
  src/
    lib.rs                    Module tree / public API
    agent.rs                  HumaiAgent: builds instruction/input prompts for two call
                              shapes (opening question, memory-conditioned reply) and calls
                              ollama::send_to_ollama; sentence-count clamping on output
    chat_types.rs               Pure chat data types shared by the Store and the UI's
                              ChatExample: ChatMessage, Room, MessageSource,
                              AssistantGeneration, MessageCorrelation, UiInbox<T>
    experiment/                  Trial-sequencing/counterbalancing engine — pure,
                              parameterized functions (no process-global state), unit
                              tested directly. See "Experiment engine" below.
    ollama/                        Ollama HTTP client + streaming chat engine
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
      metrics.rs                          Thin re-export shim: `humai_core::metrics` ==
                              experiment_csv's public types, a stable import path
    store.rs / store_schema.rs           SQLite persistence for chat transcripts
                              (rusqlite, chat.sqlite), idempotent schema migration
    sync_ext.rs                            MutexExt/RwLockExt poison-recovering lock
                              accessors, used by both crates
    timer.rs                                Process-global trial countdown timer
                              (start/pause/reset)
    vault.rs                                 Master-password gate (Argon2id) +
                              encrypted Vault blob (ChaCha20-Poly1305)

apps/humai/                  UI binary — depends on humai-core as a path dependency
  src/
    main.rs                    Process entry point: Tokio runtime bootstrap, eframe::run_native
    lib.rs                      Module tree / re-exports (pub use ... as the crate's public API)
    app_model.rs                  Humai: top-level in-memory app state (selected model,
                              prompts, active tab, theme, HTTP policy flags, metrics config)
    app_state.rs                    AppState: Arc<RwLock<..>>-guarded runtime config shared
                              across UI and background tasks (currently just MetricsConfig)
    window_icon.rs                    Platform-split viewport/taskbar icon setup (Win32 vs egui)
    ui/                                  egui rendering, one module per panel
      mod.rs                              HumaiApp (the eframe::App impl), theme + font
                              bootstrap, vault-gated top-level frame dispatch
      panel_render.rs                       Top-level tab bar (Overview / Design / Settings)
      settings_panel.rs                       Ollama host/model selection, connection test,
                              CSV recording toggle/output dir, Catppuccin theme picker
      vault_panel.rs                            Renders MasterVault state; the only file
                              allowed to import both egui and humai_core::vault together
      overview_chat/                              Experiment engine UI + chat rendering
        panel.rs                                    Trial sequencing orchestration (delegates
                              its math to humai_core::experiment), condition/timeout handling,
                              CSV export shortcut, chat room UI, participant-ID modal.
                              State is still held in process-global OnceLock<Mutex<T>>
                              statics here — see Known Limitations.
        chat.rs                                       ChatExample: in-memory chat/room
                              rendering state (sidebar_ui, room management), UiInbox-based
                              background-to-UI message channel
  bin/
    gen_master_hash.rs               Standalone binary: interactively writes
                              vault/.master_hash, depends only on humai_core::vault
```

## Experiment engine

The pure trial-sequencing math lives in `humai_core::experiment` — no process-global
state, no UI dependency, fully unit tested (see `crates/humai-core/src/experiment/mod.rs`).
It implements a within-subject design: up to [`MAX_CONDITIONS`] = 2 named conditions
(`COOP` — Cooperative, `ARGUM` — Argumentative), each mapped to a distinct system
prompt (`HUMAI_COOP_AGENT_PROMPT` / `HUMAI_ARGUM_AGENT_PROMPT` env vars, defaults in
`apps/humai/src/app_model.rs`), with a researcher-configurable number of trials per
condition. Condition order is counterbalanced by parity of the participant ID's
trailing digits (even IDs run COOP→ARGUM, odd IDs run ARGUM→COOP) so consecutive
participants don't share an order bias.

The Design tab's "Conditions" control is clamped to `1..=2` (with a tooltip
explaining why) rather than accepting an arbitrary count that the engine would
silently ignore — Humai only has two named condition prompts today, so a design
asking for more can't actually be honored yet (see [Roadmap](#roadmap--next-steps)
for generalizing this). "Trials per condition" has no such ceiling and is fully
wired through to `humai_core::experiment::total_experiment_trials`.

`apps/humai/src/ui/overview_chat/panel.rs` owns trial *orchestration* — timing via
the `timer` module, streaming a chat transcript through `ChatExample`/`Store`, and
emitting `TrialStart`/`Transcript`/`TrialEnd`/`TrialTotal` rows via
`recording::experiment_csv` — but delegates every piece of counterbalancing/
condition-assignment math to `humai_core::experiment` rather than reimplementing it.
This orchestration state (current trial index, timeout popup, participant ID, chat
room list) is still held in process-global statics in `panel.rs`, not owned by
`Humai`/`HumaiApp` — see [Known Limitations](#known-limitations).

After the last trial the participant is prompted for a free-text final answer, the
experiment is marked complete, and the operator can copy the just-written CSV to the
Desktop directly from the completion popup (`copy_experiment_csv_to_desktop`,
cross-platform via `HOME`/`USERPROFILE`).

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
- The model to use is always passed explicitly by the caller
  (`Humai::selected_ollama_model`, threaded through `HumaiAgent`); `engine.rs` only
  falls back to `ollama::DEFAULT_OLLAMA_MODEL` — a plain constant, not an env-var
  read — when no model has been configured yet. There is exactly one legitimate
  env-var read of `OLLAMA_MODEL` in the whole codebase, in `Humai::new()`, to seed
  the *initial* selection at startup; nothing reads it again after that, so there is
  only one source of truth for "which model is selected" at runtime.

## Storage

Three on-disk artifacts, all under the app's working directory by default and all
gitignored:

| Artifact | Format | Location | Purpose |
|---|---|---|---|
| Experiment log | CSV, one row per event | `metrics/<ExpName>.csv` (`HUMAI_METRICS_DIR`) | Primary statistical-analysis output: one append-only file per experiment design, header defined in `recording/experiment_csv.rs`. Filenames are derived from free-text UI fields and passed through `sanitize_file_stem` to block path traversal (`../`, absolute paths, drive prefixes). |
| Event ledger | JSONL, one line per event + `summary.json` | `metrics/runs/<experiment_id>/<run_id>/events.jsonl` | Reproducible-audit-trail mirror of the CSV: every CSV row `experiment_csv::emit()` writes is also appended here (same event name, SHA-256-hashed input/output) via `recording::ledger::EventLedger`, opened lazily per (experiment_id, run_id) and finalized (`system.run_stopped` + `summary.json`, event counts, first/last timestamp) when the experiment ends. |
| Chat transcript | SQLite (WAL) | `<metrics output dir>/chat.sqlite` | Full message history per "room" (conversation), independent of the CSV/ledger. `conversations` / `messages` / `generations` tables; `Store::append_message` writes across all three inside one transaction so a mid-write crash can't leave an orphaned row. Opened once and cached (`Arc<Store>`, keyed by path) rather than reopened every UI frame — see [Known Limitations](#known-limitations) for the one remaining case (a changed output dir mid-session) this cache doesn't handle perfectly. |
| Master password hash | One-line Argon2id PHC string | `vault/.master_hash` | Gate on app startup in release builds; regenerated via `cargo run --bin gen_master_hash`. |

Both `chat.sqlite` and `vault/.master_hash` are restricted to `0600` on Unix at
creation time (no-op on Windows, where per-user profile ACLs already isolate them).

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
render identically on Windows, Linux, and macOS without depending on OS-installed
fonts (egui's rasterizer only supports single-color glyphs, so emoji render tinted
monochrome, not full-color — a renderer limitation, not a bug).

Three top-level tabs (`app_model::MainPanelTab`):
- **Overview** — the live chat/experiment room, sidebar (timer, participant ID, room
  management), and the message thread itself.
- **Design** — experiment design fields (name, participant ID, topic, condition
  count, trials-per-condition, trial duration) that feed `ExperimentDesignSnapshot`.
- **Settings** — Ollama host/model configuration and connectivity test, CSV recording
  toggle and output directory, theme picker.

## Known limitations

Honest gaps as of this writing — not hidden, so the next change to touch this code
knows what it's standing on:

- **`panel.rs`/`chat.rs` still hold experiment/chat UI state in process-global
  `OnceLock<Mutex<T>>` statics** (current trial, chat rooms, participant ID, modal
  state) rather than as fields on `Humai`/`HumaiApp`. This means the app can only
  ever have one experiment session per process, and that state can't be unit tested
  in isolation the way `humai_core::experiment`'s pure math now can. The trial-
  sequencing *logic* itself has been extracted and is well-tested (see "Experiment
  engine" above); only the *orchestration* layer around it is still global-state-based.
  Removing this is real, scoped follow-up work — not attempted in the same pass as
  the correctness fixes above because it touches ~2,000 lines of deeply-nested egui
  closures and deserves its own careful, independently-reviewable change.
- **Humai only supports two named conditions** (Cooperative/Argumentative), each with
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
- **No CLI or HTTP API yet.** The `humai-core`/`apps/humai` split makes both
  structurally possible now (the engine has no UI dependency), but neither has been
  built.

## Roadmap / next steps

Rough priority order, highest-value first:

1. De-globalize `panel.rs`/`chat.rs`'s remaining `OnceLock<Mutex<T>>` state (see
   Known Limitations) — the single biggest remaining architectural debt.
2. A headless experiment runner (`humai-core` + a small CLI) for scripted/CI-driven
   regression testing of prompts and trial logic without a GUI.
3. Generalize `humai_core::experiment` beyond exactly two named conditions to an
   arbitrary researcher-authored set, with a matching Design-tab UI for entering each
   condition's prompt — turning "Conditions: N" from a clamped, honest control into a
   fully general one.
4. An HTTP API crate built on `humai-core`, once there's a concrete consumer for it.
5. An Intel (`x86_64-apple-darwin`) macOS release build, or a `lipo`-merged universal
   binary, if Intel Mac users are a real audience.

## Build & release

- Two-member Cargo workspace (`apps/humai`, `crates/humai-core`);
  `[profile.release]` (LTO, `codegen-units = 1`, `strip = true`) lives in the
  workspace-root `Cargo.toml` because Cargo only reads profile settings from the root.
- `apps/humai/build.rs` is Windows-only: generates a multi-resolution `.ico` from
  `assets/icon_window_bar.png` and embeds it as a Win32 resource via `winres`.
- `reqwest` uses `rustls-tls` and `rusqlite` uses the `bundled` SQLite — both
  deliberately chosen to avoid a system OpenSSL/pkg-config dependency on Windows dev
  machines. Both live in `humai-core`; `apps/humai` only adds UI-specific
  dependencies (`eframe`, `egui`, `catppuccin-egui`, `egui-phosphor`, the Windows
  icon toolchain) on top of a `humai-core = { path = "../../crates/humai-core" }`
  dependency.
- CI (`.github/workflows/ci.yml`) runs `cargo check` / `cargo test` / `cargo clippy
  -D warnings` on Linux, Windows, and macOS for every push to `main` and every PR.
- Tag-triggered release workflows (`release-linux.yml`, `release-windows.yml`,
  `release-macos.yml`) build `--release` and upload
  `humai-<version>-{ubu,win,mac}.{tar.gz,zip}` to the tag's GitHub Release;
  `distribute_app.sh`/`.bat` do the equivalent for local standalone builds — both
  build from `apps/humai/`, and Cargo resolves the `humai-core` path dependency as
  part of that build automatically, so the workspace split needed no changes to
  either distribute script.

See [AGENTS.md](AGENTS.md) for exact commands, environment variables, and the
cross-platform rules that constrain how this code can evolve.
