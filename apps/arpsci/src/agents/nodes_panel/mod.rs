//! Agent graph state, manifests, run graph, and evaluator/researcher sidecars.

mod manifest_ops;
mod model;
mod play_plan;
mod presets;
mod run_graph;
mod state;

pub(crate) use manifest_ops::sync_evaluator_researcher_activity;
pub(crate) use model::{AgentNodeKind, EvaluatorAgentsPick, NodePayload};
pub(crate) use presets::TOPIC_PRESETS;
pub(crate) use state::{AgentRecord, NodesPanelState, PanelTab, WORKSPACE_MODULE_ENABLED};
