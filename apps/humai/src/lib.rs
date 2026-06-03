#![doc = include_str!("../../../README.md")]

pub mod app_state;
pub mod agents;
#[path = "recording/metrics.rs"]
pub mod metrics;
pub mod ollama;
pub mod recording;
pub mod ui;
pub mod vault;
pub mod web;

pub use agents::Arpsci;
pub use ui::ArpsciApp;
