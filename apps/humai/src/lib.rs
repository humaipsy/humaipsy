pub mod app_state;
pub mod app_model;
pub mod agent;
#[path = "recording/metrics.rs"]
pub mod metrics;
pub mod ollama;
pub mod recording;
pub mod timer;
pub mod ui;
pub mod vault;

pub use app_model::Humai;
pub use ui::HumaiApp;
