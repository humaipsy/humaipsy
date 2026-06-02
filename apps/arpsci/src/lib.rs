#![doc = include_str!("../../../README.md")]

pub mod app_state;
#[path = "unused/mas/mod.rs"]
pub mod mas;
pub use mas as agents;
#[path = "recording/metrics.rs"]
pub mod metrics;
pub mod ollama;
pub mod python;
pub mod recording;
pub mod run;
pub mod ui;
pub mod vault;
#[path = "unused/web/mod.rs"]
pub mod web;

pub use agents::Arpsci;
pub use ui::ArpsciApp;
