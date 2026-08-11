//! The live experiment-status readout shown on the browser dashboard.

use std::sync::{Arc, Mutex};

/// A snapshot of "what's happening right now" in the desktop app. `apps/humai`
/// updates this at the same points it already tracks trial/session state (see
/// `Humai::web_status`); the web server only ever reads it. Kept intentionally
/// small and plainly serializable — this is a read-only status line, not a
/// control channel, and never grows write semantics.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct WebStatusSnapshot {
    pub design_name: String,
    pub participant_id: String,
    pub condition_label: String,
    pub trial_in_condition: usize,
    pub total_trials: usize,
    pub running: bool,
    pub timer_display: String,
}

/// Shared handle both the desktop app (writer) and the web server (reader) hold.
pub type SharedWebStatus = Arc<Mutex<WebStatusSnapshot>>;

pub fn new_shared_status() -> SharedWebStatus {
    Arc::new(Mutex::new(WebStatusSnapshot::default()))
}
