#![allow(dead_code, unused_imports)]

use crate::app_state::AppState;
use humaipsy_core::metrics::MetricsConfig;
use humaipsy_core::ollama::HttpPolicy;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::runtime::Handle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatppuccinTheme {
    Latte,
    Frappe,
    Macchiato,
    Mocha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalChatMode {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainPanelTab {
    Overview,
    Design,
    Settings,
}

pub struct HumaiPsy {
    pub(crate) rt_handle: Handle,
    pub(crate) app_state: Arc<AppState>,
    pub(crate) selected_ollama_model: String,
    pub(crate) assistant_prompt: String,
    pub(crate) coop_agent_prompt: String,
    pub(crate) argum_agent_prompt: String,
    pub(crate) conversation_history_size: usize,
    pub(crate) air_gap_enabled: bool,
    pub(crate) allow_local_ollama: bool,
    /// Base URL for the Ollama API (e.g. http://127.0.0.1:11434).
    pub(crate) ollama_host: String,
    pub(crate) metrics_config: MetricsConfig,
    /// Retained for compatibility with existing UI lock logic.
    pub(crate) conversation_graph_running: Arc<AtomicBool>,
    pub(crate) active_tab: MainPanelTab,
    /// Global Overview chat mode selector.
    pub(crate) chat_mode: GlobalChatMode,
    /// Global UI theme for catppuccin egui shell.
    pub(crate) catppuccin_theme: CatppuccinTheme,
    /// Live status the local read-only web dashboard (`humaipsy-web`) serves.
    /// The dashboard's chat transcript comes straight from the SQLite Store
    /// (same one the desktop app writes to) rather than through this field —
    /// this only carries the "what trial/condition are we on" summary that
    /// isn't durably recorded anywhere else to read back.
    pub(crate) web_status: humaipsy_web::SharedWebStatus,
    /// The address the web dashboard was configured to bind, or `None` if
    /// `HUMAIPSY_WEB_DISABLED` was set. Shown in Settings so the URL is
    /// discoverable without reading docs. This reflects what was *attempted*,
    /// not confirmed-bound — the bind itself happens asynchronously in the
    /// spawned server task, after `HumaiPsy::new()` has already returned; a
    /// failure there (e.g. the port is already in use) is logged to stderr,
    /// not surfaced in the UI.
    pub(crate) web_dashboard_addr: Option<SocketAddr>,
    /// Mirrors `MasterVault::is_unlocked()` — synced every frame by
    /// `HumaiPsyApp::update` (see `ui/mod.rs`), since the vault itself lives on
    /// `HumaiPsyApp`, not `HumaiPsy`. The web dashboard checks this on every
    /// request and withholds status/message content while locked; without
    /// this, locking the app would not stop the dashboard from continuing to
    /// serve the last-known transcript over the network.
    pub(crate) vault_unlocked: humaipsy_web::SharedVaultLock,
}

impl HumaiPsy {
    pub fn new(rt_handle: Handle) -> Self {
        let http_policy = humaipsy_core::ollama::HttpPolicy::from_env();
        humaipsy_core::ollama::set_http_policy(http_policy);
        let metrics_config = MetricsConfig::from_env();
        let app_state = Arc::new(AppState::new(metrics_config.clone()));
        let coop_agent_prompt = humaipsy_core::env_compat::var("COOP_AGENT_PROMPT").unwrap_or_else(|_| {
            "Experimental condition COOP: be cooperative with the human participant. Be supportive, collaborative, and help them progress constructively while staying safe and on-topic.".to_string()
        });
        let argum_agent_prompt = humaipsy_core::env_compat::var("ARGUM_AGENT_PROMPT").unwrap_or_else(|_| {
            "Experimental condition ARGUM: be argumentative with the human participant. You must challenge assumptions, present counterpoints, and disagree when appropriate. Do NOT use a supportive or reassuring therapeutic style.".to_string()
        });

        let web_status = humaipsy_web::new_shared_status();
        let vault_unlocked = humaipsy_web::new_shared_vault_lock();
        let web_dashboard_addr = spawn_web_dashboard(
            &rt_handle,
            &metrics_config,
            web_status.clone(),
            vault_unlocked.clone(),
        );

        Self {
            rt_handle,
            app_state,
            web_status,
            web_dashboard_addr,
            vault_unlocked,
            selected_ollama_model: std::env::var("OLLAMA_MODEL").unwrap_or_default(),
            assistant_prompt: humaipsy_core::env_compat::var("ASSISTANT_PROMPT").unwrap_or_else(|_| {
                "You are HumaiPsyAgent, the experiment assistant for human-to-agent conversations. Be clear, neutral, and helpful. Keep responses concise and actionable when possible.".to_string()
            }),
            coop_agent_prompt,
            argum_agent_prompt,
            conversation_history_size: 5,
            air_gap_enabled: http_policy.air_gap_enabled,
            allow_local_ollama: http_policy.allow_local_ollama,
            ollama_host: std::env::var("OLLAMA_HOST")
                .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string()),
            metrics_config,
            conversation_graph_running: Arc::new(AtomicBool::new(false)),
            active_tab: MainPanelTab::Overview,
            chat_mode: GlobalChatMode::Enabled,
            catppuccin_theme: CatppuccinTheme::Latte,
        }
    }

    pub(crate) fn sync_http_policy(&self) {
        humaipsy_core::ollama::set_http_policy(HttpPolicy {
            air_gap_enabled: self.air_gap_enabled,
            allow_local_ollama: self.allow_local_ollama,
        });
    }

    pub(crate) fn refresh_metrics_sink(&mut self) {
        self.app_state
            .update_metrics_config(self.metrics_config.clone());
    }
}

const DEFAULT_WEB_DASHBOARD_BIND: &str = "127.0.0.1:8787";

/// Spawns the local read-only web dashboard (`humaipsy-web`) onto `rt_handle`,
/// unless `HUMAIPSY_WEB_DISABLED` is set. Bound to loopback only by default
/// (`HUMAIPSY_WEB_BIND` to override) — experiment transcripts are potentially
/// sensitive human-subjects data, so inbound exposure gets the same opt-in
/// caution the outbound air-gap policy already applies. A bind failure (e.g.
/// the port is already in use) is logged to stderr and otherwise ignored —
/// the desktop app must still start regardless of dashboard availability.
fn spawn_web_dashboard(
    rt_handle: &Handle,
    metrics_config: &MetricsConfig,
    status: humaipsy_web::SharedWebStatus,
    vault_unlocked: humaipsy_web::SharedVaultLock,
) -> Option<SocketAddr> {
    if parse_bool_env("WEB_DISABLED") {
        return None;
    }

    let bind_addr: SocketAddr = humaipsy_core::env_compat::var("WEB_BIND")
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or_else(|| {
            DEFAULT_WEB_DASHBOARD_BIND
                .parse()
                .expect("DEFAULT_WEB_DASHBOARD_BIND is a valid socket address")
        });
    // Mirrors chat_db_path() in ui/overview_chat/panel.rs: the dashboard reads
    // the same chat.sqlite the desktop app writes to, under the configurable
    // metrics output dir rather than a separately hardcoded path.
    let chat_db_path = metrics_config.output_path().join("chat.sqlite");

    rt_handle.spawn(async move {
        let config = humaipsy_web::WebServerConfig {
            bind_addr,
            chat_db_path,
            status,
            vault_unlocked,
        };
        if let Err(e) = humaipsy_web::serve(config).await {
            eprintln!("[humaipsy-web] dashboard server failed to start on {bind_addr}: {e}");
        }
    });

    Some(bind_addr)
}

fn parse_bool_env(name: &str) -> bool {
    humaipsy_core::env_compat::var(name)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}