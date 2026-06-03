//! Agent node kinds, payloads, and `NodeData` factories.

use rand::Rng;

use super::presets::TOPIC_PRESETS;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentNodeKind {
    Manager,
    Worker,
    Evaluator,
    Researcher,
    Topic,
}

#[derive(Clone, Copy)]
pub(crate) enum EvaluatorAgentsPick {
    Unassigned,
    AllWorkers,
    Worker(usize),
}

impl AgentNodeKind {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            AgentNodeKind::Manager => "Agent Manager",
            AgentNodeKind::Worker => "Agent Worker",
            AgentNodeKind::Evaluator => "Agent Evaluator",
            AgentNodeKind::Researcher => "Agent Researcher",
            AgentNodeKind::Topic => "Topic",
        }
    }
}

#[derive(Clone)]
pub(crate) struct NodeManagerData {
    pub(crate) name: String,
    pub(crate) global_id: String,
}

#[derive(Clone)]
pub(crate) struct NodeWorkerData {
    pub(crate) name: String,
    pub(crate) global_id: String,

    pub(crate) instruction_mode: String,
    pub(crate) instruction: String,

    pub(crate) analysis_mode: String,
    pub(crate) conversation_topic: String,
    pub(crate) conversation_topic_source: String,

    /// Selected manager agent id (row model; no graph wires).
    pub(crate) manager_node: Option<usize>,

    /// Optional topic tool agent id.
    pub(crate) topic_node: Option<usize>,

    /// Optional explicit worker pair for dialogue loops.
    pub(crate) partner_worker: Option<usize>,
}

#[derive(Clone)]
pub(crate) struct NodeEvaluatorData {
    pub(crate) name: String,
    pub(crate) global_id: String,

    pub(crate) analysis_mode: String,
    pub(crate) instruction: String,

    pub(crate) limit_token: bool,
    pub(crate) num_predict: String,

    pub(crate) active: bool,

    /// When true, evaluate on traffic from all workers (no specific pin); pin 1 stays empty.
    pub(crate) evaluate_all_workers: bool,

    pub(crate) worker_node: Option<usize>,
    pub(crate) manager_node: Option<usize>,
}

#[derive(Clone)]
pub(crate) struct NodeResearcherData {
    pub(crate) name: String,
    pub(crate) global_id: String,

    pub(crate) topic_mode: String,
    pub(crate) instruction: String,

    pub(crate) limit_token: bool,
    pub(crate) num_predict: String,

    pub(crate) active: bool,

    pub(crate) worker_node: Option<usize>,
    pub(crate) manager_node: Option<usize>,
}

#[derive(Clone)]
pub(crate) struct NodeTopicData {
    pub(crate) name: String,
    pub(crate) global_id: String,

    pub(crate) analysis_mode: String,
    pub(crate) topic: String,
}

#[derive(Clone)]
pub(crate) enum NodePayload {
    Manager(NodeManagerData),
    Worker(NodeWorkerData),
    Evaluator(NodeEvaluatorData),
    Researcher(NodeResearcherData),
    Topic(NodeTopicData),
}

#[derive(Clone)]
pub(crate) struct NodeData {
    pub(crate) kind: AgentNodeKind,
    pub label: String,
    pub(crate) payload: NodePayload,
}

impl NodeData {
    pub(crate) fn new_global_id() -> String {
        const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        const LEN: usize = 10;
        let mut rng = rand::rng();

        // Collisions are extremely unlikely; we don't coordinate across the whole app here.
        (0..LEN)
            .map(|_| {
                let idx = rng.random_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect()
    }

    pub(crate) fn new_manager() -> Self {
        let global_id = Self::new_global_id();
        Self {
            kind: AgentNodeKind::Manager,
            label: AgentNodeKind::Manager.label().to_string(),
            payload: NodePayload::Manager(NodeManagerData {
                name: AgentNodeKind::Manager.label().to_string(),
                global_id,
            }),
        }
    }

    pub(crate) fn new_worker() -> Self {
        let global_id = Self::new_global_id();
        Self {
            kind: AgentNodeKind::Worker,
            label: AgentNodeKind::Worker.label().to_string(),
            payload: NodePayload::Worker(NodeWorkerData {
                name: AgentNodeKind::Worker.label().to_string(),
                global_id,
                instruction_mode: "Assistant".to_string(),
                instruction: "You are a helpful assistant. Answer clearly, stay concise, and focus on the user request.".to_string(),
                analysis_mode: TOPIC_PRESETS[0].0.to_string(),
                conversation_topic: TOPIC_PRESETS[0].1.to_string(),
                conversation_topic_source: "Own".to_string(),
                manager_node: None,
                topic_node: None,
                partner_worker: None,
            }),
        }
    }

    pub(crate) fn new_evaluator() -> Self {
        let global_id = Self::new_global_id();
        Self {
            kind: AgentNodeKind::Evaluator,
            label: AgentNodeKind::Evaluator.label().to_string(),
            payload: NodePayload::Evaluator(NodeEvaluatorData {
                name: AgentNodeKind::Evaluator.label().to_string(),
                global_id,
                analysis_mode: "Topic Extraction".to_string(),
                instruction: "Topic Extraction: extract the topic in 1 or 2 words. Identify what is the topic of the sentence being analysed.".to_string(),
                limit_token: false,
                num_predict: String::new(),
                active: false,
                evaluate_all_workers: false,
                worker_node: None,
                manager_node: None,
            }),
        }
    }

    pub(crate) fn new_researcher() -> Self {
        let global_id = Self::new_global_id();
        Self {
            kind: AgentNodeKind::Researcher,
            label: AgentNodeKind::Researcher.label().to_string(),
            payload: NodePayload::Researcher(NodeResearcherData {
                name: AgentNodeKind::Researcher.label().to_string(),
                global_id,
                topic_mode: "Articles".to_string(),
                instruction: "Generate article references connected to the message context. Prefer a mix of classic and recent pieces.".to_string(),
                limit_token: false,
                num_predict: String::new(),
                active: false,
                worker_node: None,
                manager_node: None,
            }),
        }
    }

    pub(crate) fn new_topic() -> Self {
        let global_id = Self::new_global_id();
        Self {
            kind: AgentNodeKind::Topic,
            label: AgentNodeKind::Topic.label().to_string(),
            payload: NodePayload::Topic(NodeTopicData {
                name: AgentNodeKind::Topic.label().to_string(),
                global_id,
                analysis_mode: TOPIC_PRESETS[0].0.to_string(),
                topic: TOPIC_PRESETS[0].1.to_string(),
            }),
        }
    }
}
