//! A local, read-only browser dashboard for a running Humai instance.
//!
//! Not a WebAssembly port of the desktop UI — a small `axum` server, spawned
//! onto the same Tokio runtime the desktop app already keeps alive, serving
//! plain HTML/JS/CSS (`web/`, embedded at compile time) plus a tiny JSON API
//! that reads the same SQLite chat `Store` the desktop app already writes to.
//! There is no write path: nothing served here can change experiment state.
//!
//! Depends only on `humai-core`, not on `apps/humai`, so it can in principle
//! run standalone later even though today it's always spawned embedded.

mod routes;
pub mod status;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

pub use status::{
    SharedVaultLock, SharedWebStatus, WebStatusSnapshot, new_shared_status, new_shared_vault_lock,
};

/// Configuration for the local dashboard server.
pub struct WebServerConfig {
    /// Address to bind. Callers are responsible for choosing a safe default
    /// (loopback-only) unless the operator has explicitly opted into wider
    /// exposure — this crate does not impose a policy itself.
    pub bind_addr: SocketAddr,
    /// Path to the same chat SQLite database the desktop app writes to.
    pub chat_db_path: PathBuf,
    /// Shared status handle the desktop app updates and this server reads.
    pub status: SharedWebStatus,
    /// Mirrors the desktop app's master-password vault lock state. Every
    /// route that could reveal experiment content checks this before
    /// touching `status`/the `Store` — see `routes::get_status`/`get_messages`.
    pub vault_unlocked: SharedVaultLock,
}

/// Opens the chat Store at `config.chat_db_path` and serves the dashboard
/// until the listener is dropped or an error occurs. Intended to be spawned
/// as a background task (`tokio::spawn`/`Handle::spawn`) on an already-running
/// Tokio runtime — it does not create its own.
pub async fn serve(config: WebServerConfig) -> anyhow::Result<()> {
    let store = Arc::new(humai_core::store::Store::open(&config.chat_db_path)?);
    let app = routes::router(store, config.status, config.vault_unlocked);
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
