#![allow(dead_code, unused_imports)]

use crate::app_state::AppState;
use crate::metrics::MetricsConfig;
use crate::web::HttpPolicy;
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
    Ollama,
    Settings,
}

pub struct Arpsci {
    pub(crate) rt_handle: Handle,
    pub(crate) app_state: Arc<AppState>,
    pub(crate) selected_ollama_model: String,
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
}

impl Arpsci {
    pub fn new(rt_handle: Handle) -> Self {
        let http_policy = crate::web::HttpPolicy::from_env();
        crate::web::set_policy(http_policy);
        let metrics_config = MetricsConfig::from_env();
        let app_state = Arc::new(AppState::new(metrics_config.clone()));

        Self {
            rt_handle,
            app_state,
            selected_ollama_model: std::env::var("OLLAMA_MODEL").unwrap_or_default(),
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
        crate::web::set_policy(HttpPolicy {
            air_gap_enabled: self.air_gap_enabled,
            allow_local_ollama: self.allow_local_ollama,
        });
    }

    pub(crate) fn refresh_metrics_sink(&mut self) {
        self.app_state
            .update_metrics_config(self.metrics_config.clone());
    }
}