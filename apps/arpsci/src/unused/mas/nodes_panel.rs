//! Agent graph state, manifests, run graph, and evaluator/researcher sidecars.

#[path = "nodes_panel_manifest_ops.rs"]
mod manifest_ops;
#[path = "nodes_panel_model.rs"]
mod model;
#[path = "nodes_panel_play_plan.rs"]
mod play_plan;
#[path = "nodes_panel_presets.rs"]
mod presets;
#[path = "nodes_panel_run_graph.rs"]
mod run_graph;
#[path = "nodes_panel_state.rs"]
mod state;

pub(crate) use manifest_ops::sync_evaluator_researcher_activity;
pub(crate) use model::{AgentNodeKind, EvaluatorAgentsPick, NodePayload};
pub(crate) use presets::TOPIC_PRESETS;
pub(crate) use state::{AgentRecord, NodesPanelState, PanelTab, WORKSPACE_MODULE_ENABLED};
