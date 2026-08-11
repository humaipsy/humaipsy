//! The live experiment-status readout shown on the browser dashboard, and the
//! shared flag that gates it on the desktop app's master-password vault.

use std::sync::atomic::AtomicBool;
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
    /// Whether the desktop app's master-password vault is currently unlocked.
    /// The route handlers check this directly and withhold status/message
    /// content when it's `false` — this field isn't just informational, it's
    /// load-bearing: see `routes::get_status`/`get_messages`.
    pub vault_unlocked: bool,
}

/// Shared handle both the desktop app (writer) and the web server (reader) hold.
pub type SharedWebStatus = Arc<Mutex<WebStatusSnapshot>>;

pub fn new_shared_status() -> SharedWebStatus {
    Arc::new(Mutex::new(WebStatusSnapshot::default()))
}

/// Shared flag mirroring `MasterVault::is_unlocked()`. Starts `false`
/// (fail-closed: treated as locked) until the desktop app explicitly syncs it
/// on every frame — see `HumaiApp::update` in `apps/humai`. Locking the app
/// (the Settings "Lock" button) must stop the dashboard from serving the chat
/// transcript just as surely as it blanks the desktop UI; without this, the
/// dashboard would keep serving the last-known transcript over the network
/// regardless of the vault's lock state, which defeats the point of the lock.
pub type SharedVaultLock = Arc<AtomicBool>;

pub fn new_shared_vault_lock() -> SharedVaultLock {
    Arc::new(AtomicBool::new(false))
}
