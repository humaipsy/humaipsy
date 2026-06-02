//! Sidecar types, research-injection helpers, and per-turn sidecar runner functions.
//!
//! Evaluator sidecars run after each dialogue line; researcher sidecars run pre-turn
//! and inject references into the speaking worker's prompt before inference.

use crate::run::event_ledger::EventLedger;
use crate::run::manifest::RunContext;
use crate::app_state::AppState;
use crate::metrics::InferenceTraceContext;
use crate::web::{send_evaluator_result, send_researcher_result};
use crate::ollama::OllamaStopEpoch;
use futures_util::future::join_all;
use std::sync::Arc;

// ─── Sidecar configuration ────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct ConversationSidecarConfig {
    pub evaluators: Vec<SidecarEvaluator>,
    pub researchers: Vec<SidecarResearcher>,
    pub scheduling: SidecarSchedulingPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchExecutionPolicy {
    Off,
    Inline,
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluatorExecutionPolicy {
    Off,
    InlineEveryTurn,
    BatchedEvery(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidecarSchedulingPolicy {
    pub research: ResearchExecutionPolicy,
    pub evaluator: EvaluatorExecutionPolicy,
}

impl Default for SidecarSchedulingPolicy {
    fn default() -> Self {
        Self {
            research: ResearchExecutionPolicy::Inline,
            evaluator: EvaluatorExecutionPolicy::InlineEveryTurn,
        }
    }
}

impl SidecarSchedulingPolicy {
    pub fn from_env() -> Self {
        let mut policy = Self::default();

        policy.research = match std::env::var("ARPSCI_RESEARCH_POLICY")
            .unwrap_or_else(|_| "inline".to_string())
            .to_lowercase()
            .as_str()
        {
            "off" => ResearchExecutionPolicy::Off,
            "background" => ResearchExecutionPolicy::Background,
            _ => ResearchExecutionPolicy::Inline,
        };

        let raw_eval = std::env::var("ARPSCI_EVALUATOR_POLICY")
            .unwrap_or_else(|_| "inline".to_string())
            .to_lowercase();
        policy.evaluator = if raw_eval == "off" {
            EvaluatorExecutionPolicy::Off
        } else if raw_eval.starts_with("batched:") {
            let every = raw_eval
                .split(':')
                .nth(1)
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(3)
                .max(1);
            EvaluatorExecutionPolicy::BatchedEvery(every)
        } else {
            EvaluatorExecutionPolicy::InlineEveryTurn
        };

        policy
    }

    pub fn should_run_evaluators(self, turn_index: usize) -> bool {
        match self.evaluator {
            EvaluatorExecutionPolicy::Off => false,
            EvaluatorExecutionPolicy::InlineEveryTurn => true,
            EvaluatorExecutionPolicy::BatchedEvery(n) => (turn_index + 1) % n == 0,
        }
    }
}

#[derive(Clone)]
pub struct SidecarEvaluator {
    pub global_id: String,
    pub instruction: String,
    pub analysis_mode: String,
    pub limit_token: bool,
    pub num_predict: String,
}

#[derive(Clone)]
pub struct SidecarResearcher {
    pub global_id: String,
    pub topic_mode: String,
    pub instruction: String,
    pub limit_token: bool,
    pub num_predict: String,
    /// Worker row id this researcher is wired to ("Injection"); pre-turn context uses the partner's last line.
    pub target_worker_id: usize,
}

// ─── Research injection ───────────────────────────────────────────────────

const RESEARCH_INJECTION_HEADER: &str =
    "\n\n---\nResearch references for your turn (consider when responding):\n";
const RESEARCH_INJECTION_FOOTER: &str = "\n---\n";

/// Where pre-turn research text is merged into the Ollama prompt for this worker turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchInjectionPlacement {
    /// After formatted history + "Your turn…" (default).
    ConversationContext,
    /// After the system instruction block, before the transcript (stronger emphasis on references).
    EnhancedInstruction,
}

pub const DEFAULT_RESEARCH_INJECTION_PLACEMENT: ResearchInjectionPlacement =
    ResearchInjectionPlacement::ConversationContext;

/// Merge non-empty `research_blocks` into the turn prompt per `placement`.
pub fn apply_research_injection(
    placement: ResearchInjectionPlacement,
    mut enhanced_instruction: String,
    mut conversation_context: String,
    research_blocks: &str,
) -> (String, String) {
    if research_blocks.is_empty() {
        return (enhanced_instruction, conversation_context);
    }
    let mut injection = String::with_capacity(
        RESEARCH_INJECTION_HEADER.len() + research_blocks.len() + RESEARCH_INJECTION_FOOTER.len(),
    );
    injection.push_str(RESEARCH_INJECTION_HEADER);
    injection.push_str(research_blocks);
    injection.push_str(RESEARCH_INJECTION_FOOTER);
    match placement {
        ResearchInjectionPlacement::ConversationContext => {
            conversation_context.push_str(&injection)
        }
        ResearchInjectionPlacement::EnhancedInstruction => {
            enhanced_instruction.push_str(&injection)
        }
    }
    (enhanced_instruction, conversation_context)
}

/// Which utterance grounds pre-turn research (for ledger / debugging).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchMessageGrounding {
    /// `message_for_research` is the tied worker's own last line in this dialogue.
    TiedWorkerLastMessage,
    /// Tied worker has not spoken yet; we use the partner's last line so first turn still gets references.
    PartnerFallbackFirstTurn,
}

fn researcher_pre_turn_instruction(
    rs: &SidecarResearcher,
    tied_worker_name: &str,
    category: &str,
    grounding: ResearchMessageGrounding,
) -> String {
    let ground_line = match grounding {
        ResearchMessageGrounding::TiedWorkerLastMessage => format!(
            "The text below is the **last message from worker \"{name}\"** (the agent this researcher is tied to via Injection). \
             Base your suggestions only on that content.",
            name = tied_worker_name,
        ),
        ResearchMessageGrounding::PartnerFallbackFirstTurn => format!(
            "The tied worker \"{name}\" has not spoken yet in this dialogue. \
             The text below is the **partner's last message**—use it as the only ground for your {category} suggestions until the tied worker has their own prior line.",
            name = tied_worker_name,
            category = category,
        ),
    };
    format!(
        "{}\n\nReference category: **{category}** (e.g. films, albums, papers—stay in this category).\n\
         {ground_line}\n\
         Suggest exactly 3 items in that category. Bullet points: title/name + one line why it fits.",
        rs.instruction,
        category = category,
        ground_line = ground_line,
    )
}

// ─── Sidecar runner functions ─────────────────────────────────────────────

/// Run researchers targeting `speaking_worker_id` **before** that worker's reply.
///
/// **Latency:** This always runs **before** the dialogue model. Each researcher row is one extra Ollama
/// round-trip; we run those in **parallel** when multiple researchers share the same injection worker.
/// Expect roughly `(research time) + (dialogue time)` per turn when research is active—not a bug, but
/// strictly more work than a plain dialogue turn.
pub async fn run_researchers_before_worker_turn(
    sidecars: &ConversationSidecarConfig,
    speaking_worker_id: usize,
    tied_worker_name: &str,
    message_for_research: &str,
    grounding: ResearchMessageGrounding,
    ollama_host: &str,
    endpoint: &str,
    run_context: Option<&RunContext>,
    selected_model: Option<&str>,
    ollama_stop_epoch: Option<OllamaStopEpoch>,
    post_http: bool,
    ledger: Option<&Arc<EventLedger>>,
    app_state: Arc<AppState>,
) -> Result<String, ()> {
    let researchers: Vec<SidecarResearcher> = sidecars
        .researchers
        .iter()
        .filter(|rs| rs.target_worker_id == speaking_worker_id)
        .cloned()
        .collect();

    if researchers.is_empty() {
        return Ok(String::new());
    }

    let msg = message_for_research.to_string();
    let host = ollama_host.to_string();
    let model = selected_model.map(|s| s.to_string());
    let epoch = ollama_stop_epoch.clone();
    let experiment_id = run_context.map(|r| r.experiment_id.clone());
    let run_id = run_context.map(|r| r.run_id.clone());

    let futures = researchers.into_iter().map(|rs| {
        let msg = msg.clone();
        let host = host.clone();
        let model = model.clone();
        let epoch = epoch.clone();
        let tied = tied_worker_name.to_string();
        let app_state = app_state.clone();
        let experiment_id = experiment_id.clone();
        let run_id = run_id.clone();
        let category = if rs.topic_mode.trim().is_empty() {
            "Articles".to_string()
        } else {
            rs.topic_mode.clone()
        };
        let instruction = researcher_pre_turn_instruction(&rs, &tied, &category, grounding);
        async move {
            let ollama_input = format!("{}\n{}", instruction, msg);
            let out = crate::ollama::send_to_ollama(
                host.as_str(),
                &instruction,
                &msg,
                rs.limit_token,
                &rs.num_predict,
                model.as_deref(),
                epoch,
                app_state,
                InferenceTraceContext {
                    source: "sidecar.researcher.pre_turn".to_string(),
                    experiment_id,
                    run_id,
                    node_global_id: Some(rs.global_id.clone()),
                    turn_index: None,
                },
            )
            .await;
            (rs, category, ollama_input, out)
        }
    });

    let joined = join_all(futures).await;
    let mut blocks = Vec::new();

    for (rs, topic, ollama_input, out) in joined {
        match out {
            Ok(response) => {
                if let Some(l) = ledger {
                    let _ = l.append_with_hashes(
                        "sidecar.researcher",
                        Some(rs.global_id.clone()),
                        selected_model.map(|s| s.to_string()),
                        &ollama_input,
                        &response,
                        serde_json::json!({
                            "topic": topic,
                            "phase": "pre_turn_injection",
                            "grounding": match grounding {
                                ResearchMessageGrounding::TiedWorkerLastMessage => "tied_worker_last",
                                ResearchMessageGrounding::PartnerFallbackFirstTurn => "partner_fallback",
                            },
                        }),
                    );
                }
                if post_http {
                    if let Err(e) = send_researcher_result(
                        endpoint,
                        "Agent Researcher",
                        &topic,
                        &response,
                        run_context,
                        ledger,
                    )
                    .await
                    {
                        eprintln!("[Researcher] Failed to send to ams-chat: {}", e);
                    }
                }
                blocks.push(format!(
                    "### References ({topic})\n{response}",
                    topic = topic,
                    response = response
                ));
            }
            Err(e) => {
                if e.to_string() == crate::ollama::OLLAMA_STOPPED_MSG {
                    return Err(());
                }
                if let Some(l) = ledger {
                    let _ = l.append_with_hashes(
                        "sidecar.researcher",
                        Some(rs.global_id.clone()),
                        selected_model.map(|s| s.to_string()),
                        &ollama_input,
                        "",
                        serde_json::json!({
                            "topic": topic,
                            "phase": "pre_turn_injection",
                            "stage": "ollama",
                            "error": e.to_string(),
                        }),
                    );
                }
                eprintln!("[Researcher] Ollama error (pre-turn): {}", e);
            }
        }
    }
    Ok(blocks.join("\n\n"))
}

fn evaluator_sentiment(analysis_mode: &str, response: &str) -> &'static str {
    let response_lower = response.to_lowercase();
    match analysis_mode {
        "Topic Extraction" => "topic",
        "Decision Analysis" => "decision",
        "Sentiment Classification" => {
            if response_lower.contains("positive") || response_lower.contains("happy") {
                "sentiment"
            } else if response_lower.contains("negative")
                || response_lower.contains("sad")
                || response_lower.contains("angry")
                || response_lower.contains("frustrated")
            {
                "sentiment"
            } else if response_lower.contains("neutral") {
                "sentiment"
            } else {
                "unknown"
            }
        }
        _ => {
            if response_lower.contains("happy") {
                "happy"
            } else if response_lower.contains("sad") {
                "sad"
            } else {
                "analysis"
            }
        }
    }
}

/// Evaluator sidecars after each dialogue line (`agent_message` = the line just spoken).
/// Researchers are **not** run here; they run pre-turn via [`run_researchers_before_worker_turn`].
pub async fn run_evaluator_sidecars_for_message(
    sidecars: &ConversationSidecarConfig,
    agent_message: &str,
    ollama_host: &str,
    endpoint: &str,
    run_context: Option<&RunContext>,
    selected_model: Option<&str>,
    ollama_stop_epoch: Option<OllamaStopEpoch>,
    post_http: bool,
    ledger: Option<&Arc<EventLedger>>,
    app_state: Arc<AppState>,
) -> Result<Vec<String>, ()> {
    let experiment_id = run_context.map(|r| r.experiment_id.clone());
    let run_id = run_context.map(|r| r.run_id.clone());

    let mut evaluator_outputs = Vec::new();

    for ev in &sidecars.evaluators {
        let ollama_input = format!("{}\n{}", ev.instruction, agent_message);
        match crate::ollama::send_to_ollama(
            ollama_host,
            &ev.instruction,
            agent_message,
            ev.limit_token,
            &ev.num_predict,
            selected_model,
            ollama_stop_epoch.clone(),
            app_state.clone(),
            InferenceTraceContext {
                source: "sidecar.evaluator".to_string(),
                experiment_id: experiment_id.clone(),
                run_id: run_id.clone(),
                node_global_id: Some(ev.global_id.clone()),
                turn_index: None,
            },
        )
        .await
        {
            Ok(response) => {
                if let Some(l) = ledger {
                    let _ = l.append_with_hashes(
                        "sidecar.evaluator",
                        Some(ev.global_id.clone()),
                        selected_model.map(|s| s.to_string()),
                        &ollama_input,
                        &response,
                        serde_json::json!({ "analysis_mode": ev.analysis_mode }),
                    );
                }
                let sentiment = evaluator_sentiment(ev.analysis_mode.as_str(), &response);
                if post_http {
                    if let Err(e) = send_evaluator_result(
                        endpoint,
                        "Agent Evaluator",
                        sentiment,
                        &response,
                        run_context,
                        ledger,
                    )
                    .await
                    {
                        eprintln!("[Evaluator] Failed to send to ams-chat: {}", e);
                    }
                }
                evaluator_outputs.push(response);
            }
            Err(e) => {
                if e.to_string() == crate::ollama::OLLAMA_STOPPED_MSG {
                    return Err(());
                }
                if let Some(l) = ledger {
                    let _ = l.append_with_hashes(
                        "sidecar.evaluator",
                        Some(ev.global_id.clone()),
                        selected_model.map(|s| s.to_string()),
                        &ollama_input,
                        "",
                        serde_json::json!({
                            "analysis_mode": ev.analysis_mode,
                            "stage": "ollama",
                            "error": e.to_string(),
                        }),
                    );
                }
                eprintln!("[Evaluator] Ollama error: {}", e);
            }
        }
    }

    Ok(evaluator_outputs)
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_RESEARCH_INJECTION_PLACEMENT, ResearchInjectionPlacement, apply_research_injection,
    };

    #[test]
    fn apply_research_injection_empty_is_noop() {
        let (e, c) = apply_research_injection(
            DEFAULT_RESEARCH_INJECTION_PLACEMENT,
            "sys".into(),
            "ctx".into(),
            "",
        );
        assert_eq!(e, "sys");
        assert_eq!(c, "ctx");
    }

    #[test]
    fn apply_research_injection_appends_to_context_by_default() {
        let (e, c) = apply_research_injection(
            ResearchInjectionPlacement::ConversationContext,
            "sys".into(),
            "ctx".into(),
            "refs",
        );
        assert_eq!(e, "sys");
        assert!(c.contains("refs"));
        assert!(c.contains("Research references"));
    }

    #[test]
    fn apply_research_injection_can_target_enhanced_instruction() {
        let (e, c) = apply_research_injection(
            ResearchInjectionPlacement::EnhancedInstruction,
            "sys".into(),
            "ctx".into(),
            "refs",
        );
        assert!(e.contains("refs"));
        assert_eq!(c, "ctx");
    }
}
