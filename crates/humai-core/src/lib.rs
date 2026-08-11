//! Humai's engine: agent prompting, the Ollama client, experiment recording
//! (CSV + JSONL event ledger), the chat SQLite store, and the master-password
//! vault. Deliberately has no dependency on `egui`/`eframe` — this is what lets
//! the engine be exercised headlessly (unit tests, a future CLI runner, a future
//! HTTP API) without booting a GUI window. UI rendering lives in `apps/humai`.

pub mod agent;
pub mod chat_types;
pub mod experiment;
#[path = "recording/metrics.rs"]
pub mod metrics;
pub mod ollama;
pub mod recording;
pub mod store;
pub mod sync_ext;
pub mod timer;
pub mod vault;
