use crate::app_state::AppState;
use crate::metrics::MetricsConfig;
use crate::run::event_ledger::EventLedger;
use crate::run::manifest::{RunContext, RunManifest};
use crate::web::HttpPolicy;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

/// A completed agent dialogue turn forwarded to the overview chat room.
pub struct AgentChatEvent {
    pub from: String,
    pub content: String,
    pub room_id: String,
}

pub mod agent_conversation_loop;
pub mod conversation_sidecars;
pub mod dialogue;
pub(crate) mod nodes_panel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatppuccinTheme {
    Latte,
    Frappe,
    Macchiato,
    Mocha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalChatMode {
    HumanToAgent,
    AgentToAgent,
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
    pub(crate) http_endpoint: String,
    pub(crate) last_message_in_chat: Arc<Mutex<Option<String>>>,
    pub(crate) conversation_message_events: Arc<Mutex<Vec<String>>>,
    pub(crate) last_evaluated_message_by_evaluator:
        Arc<Mutex<std::collections::HashMap<usize, String>>>,
    pub(crate) last_researched_message_by_researcher:
        Arc<Mutex<std::collections::HashMap<usize, String>>>,
    pub(crate) evaluator_event_queues:
        Arc<Mutex<std::collections::HashMap<usize, std::collections::VecDeque<(String, String)>>>>,
    pub(crate) researcher_event_queues:
        Arc<Mutex<std::collections::HashMap<usize, std::collections::VecDeque<(String, String)>>>>,
    pub(crate) evaluator_inflight_nodes: Arc<Mutex<std::collections::HashSet<usize>>>,
    pub(crate) researcher_inflight_nodes: Arc<Mutex<std::collections::HashSet<usize>>>,
    conversation_loop_handles: Vec<(usize, Arc<Mutex<bool>>, tokio::task::JoinHandle<()>)>,
    /// Incremented on Stop (and at each run restart) so in-flight Ollama streams can exit promptly.
    pub(super) ollama_run_epoch: Arc<AtomicU64>,
    pub(crate) current_run_context: Option<RunContext>,
    current_manifest: Option<RunManifest>,
    /// Append-only ledger for the active run (`events.jsonl`), if any.
    pub(super) event_ledger: Option<Arc<EventLedger>>,
    pub(crate) read_only_replay_mode: bool,
    /// True while a started play is active (cleared on Stop or when all conversation tasks finish).
    pub(crate) conversation_graph_running: Arc<AtomicBool>,
    /// Bumped on each `run_graph`; stale conversation tasks ignore completion.
    conversation_run_generation: Arc<AtomicU64>,
    pub(crate) nodes_panel: nodes_panel::NodesPanelState,
    /// Sender half of the agent→chat channel; cloned into each conversation loop.
    pub(crate) chat_turn_tx: Option<std::sync::mpsc::Sender<AgentChatEvent>>,
    /// Receiver half drained every Overview frame to push agent turns into the chat UI.
    pub(crate) chat_turn_rx: Option<std::sync::mpsc::Receiver<AgentChatEvent>>,
    /// The room ID that was active when the last run started; agent turns are posted there.
    pub(crate) chat_active_room_id: Option<String>,
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
            http_endpoint: std::env::var("CONVERSATION_HTTP_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:3000/".to_string()),
            last_message_in_chat: Arc::new(Mutex::new(None)),
            conversation_message_events: Arc::new(Mutex::new(Vec::new())),
            last_evaluated_message_by_evaluator: Arc::new(Mutex::new(
                std::collections::HashMap::new(),
            )),
            last_researched_message_by_researcher: Arc::new(Mutex::new(
                std::collections::HashMap::new(),
            )),
            evaluator_event_queues: Arc::new(Mutex::new(std::collections::HashMap::new())),
            researcher_event_queues: Arc::new(Mutex::new(std::collections::HashMap::new())),
            evaluator_inflight_nodes: Arc::new(Mutex::new(std::collections::HashSet::new())),
            researcher_inflight_nodes: Arc::new(Mutex::new(std::collections::HashSet::new())),
            conversation_loop_handles: Vec::new(),
            ollama_run_epoch: Arc::new(AtomicU64::new(0)),
            current_run_context: None,
            current_manifest: None,
            event_ledger: None,
            read_only_replay_mode: false,
            conversation_graph_running: Arc::new(AtomicBool::new(false)),
            conversation_run_generation: Arc::new(AtomicU64::new(0)),
            nodes_panel: nodes_panel::NodesPanelState::default(),
            chat_turn_tx: None,
            chat_turn_rx: None,
            chat_active_room_id: None,
            chat_mode: GlobalChatMode::HumanToAgent,
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