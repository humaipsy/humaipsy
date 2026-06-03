use super::model::NodePayload;
use super::state::AgentRecord;

#[derive(serde::Serialize)]
pub(super) struct RunPlayPlanJson {
    pub(crate) conversations: Vec<PlayConversationGroupJson>,
    pub(crate) managers: Vec<PlayManagerJson>,
    pub(crate) evaluators: Vec<PlayEvaluatorInPlayJson>,
    pub(crate) researchers: Vec<PlayResearcherInPlayJson>,
}

#[derive(serde::Serialize)]
pub(super) struct PlayConversationGroupJson {
    pub(crate) loop_key_node_id: usize,
    pub(crate) workers: Vec<PlayWorkerInPlayJson>,
}

#[derive(serde::Serialize)]
pub(super) struct PlayWorkerInPlayJson {
    pub(crate) node_id: usize,
    pub(crate) name: String,
    pub(crate) global_id: String,
    pub(crate) conversation_topic: String,
    pub(crate) conversation_topic_source: String,
}

#[derive(serde::Serialize)]
pub(super) struct PlayManagerJson {
    pub(crate) node_id: usize,
    pub(crate) name: String,
    pub(crate) global_id: String,
}

#[derive(serde::Serialize)]
pub(super) struct PlayEvaluatorInPlayJson {
    pub(crate) node_id: usize,
    pub(crate) name: String,
    pub(crate) global_id: String,
    pub(crate) analysis_mode: String,
    pub(crate) evaluate_all_workers: bool,
}

#[derive(serde::Serialize)]
pub(super) struct PlayResearcherInPlayJson {
    pub(crate) node_id: usize,
    pub(crate) name: String,
    pub(crate) global_id: String,
    pub(crate) topic_mode: String,
}

pub(super) fn build_conversation_sidecar_from_agents(
    agents: &[AgentRecord],
    scheduling: crate::agents::conversation_sidecars::SidecarSchedulingPolicy,
) -> crate::agents::conversation_sidecars::ConversationSidecarConfig {
    use crate::agents::conversation_sidecars::{
        ConversationSidecarConfig, SidecarEvaluator, SidecarResearcher,
    };
    let mut evaluators = Vec::new();
    let mut researchers = Vec::new();
    for r in agents {
        match &r.data.payload {
            NodePayload::Evaluator(e) if e.active => {
                evaluators.push(SidecarEvaluator {
                    global_id: e.global_id.clone(),
                    instruction: e.instruction.clone(),
                    analysis_mode: e.analysis_mode.clone(),
                    limit_token: e.limit_token,
                    num_predict: e.num_predict.clone(),
                });
            }
            NodePayload::Researcher(res) if res.active => {
                if let Some(target_worker_id) = res.worker_node {
                    researchers.push(SidecarResearcher {
                        global_id: res.global_id.clone(),
                        topic_mode: res.topic_mode.clone(),
                        instruction: res.instruction.clone(),
                        limit_token: res.limit_token,
                        num_predict: res.num_predict.clone(),
                        target_worker_id,
                    });
                }
            }
            _ => {}
        }
    }
    ConversationSidecarConfig {
        evaluators,
        researchers,
        scheduling,
    }
}

pub(super) fn collect_run_play_plan_from_agents(
    agents: &[AgentRecord],
    conversations: Vec<PlayConversationGroupJson>,
) -> RunPlayPlanJson {
    let mut managers = Vec::new();
    let mut evaluators = Vec::new();
    let mut researchers = Vec::new();
    for r in agents {
        let id = r.id;
        match &r.data.payload {
            NodePayload::Manager(m) => managers.push(PlayManagerJson {
                node_id: id,
                name: m.name.clone(),
                global_id: m.global_id.clone(),
            }),
            NodePayload::Evaluator(e) if e.active => {
                evaluators.push(PlayEvaluatorInPlayJson {
                    node_id: id,
                    name: e.name.clone(),
                    global_id: e.global_id.clone(),
                    analysis_mode: e.analysis_mode.clone(),
                    evaluate_all_workers: e.evaluate_all_workers,
                });
            }
            NodePayload::Researcher(res) if res.active => {
                researchers.push(PlayResearcherInPlayJson {
                    node_id: id,
                    name: res.name.clone(),
                    global_id: res.global_id.clone(),
                    topic_mode: res.topic_mode.clone(),
                });
            }
            _ => {}
        }
    }
    managers.sort_by_key(|m| m.node_id);
    evaluators.sort_by_key(|e| e.node_id);
    researchers.sort_by_key(|r| r.node_id);
    RunPlayPlanJson {
        conversations,
        managers,
        evaluators,
        researchers,
    }
}
