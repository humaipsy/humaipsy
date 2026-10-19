use std::sync::Arc;

use crate::metrics::InferenceSnapshot;
use crate::ollama::InferenceTimingSnapshot;
use crate::provider::{ChatRequest, Provider, token_budget};
use crate::recording::timing::InferenceTraceContext;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMemoryMessage {
    pub role: AgentRole,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct AgentReply {
    pub text: String,
    pub timing: InferenceTimingSnapshot,
    pub model: Option<String>,
}

#[derive(Clone)]
pub struct HumaiPsyAgent {
    provider: Arc<dyn Provider>,
    model: String,
    assistant_prompt: String,
    history_size: usize,
    max_sentences: usize,
    /// Approximate token budget applied on top of `history_size`'s message-
    /// count window (see `format_memory_input`). `None` (the default) means
    /// no additional trimming — behavior identical to before this existed.
    token_budget: Option<usize>,
}

impl HumaiPsyAgent {
    pub fn new(provider: Arc<dyn Provider>, model: String, assistant_prompt: String) -> Self {
        Self {
            provider,
            model,
            assistant_prompt,
            history_size: 5,
            max_sentences: 5,
            token_budget: None,
        }
    }

    pub fn with_history_size(mut self, history_size: usize) -> Self {
        self.history_size = history_size.max(1);
        self
    }

    /// Applies an additional, approximate token-count trim on top of
    /// `history_size`'s message-count window — see `crate::provider::token_budget`
    /// for what "approximate" means here (no real tokenizer, character-based
    /// estimate). Purely additive: without calling this, behavior is
    /// unchanged from before this existed.
    pub fn with_token_budget(mut self, token_budget: usize) -> Self {
        self.token_budget = Some(token_budget.max(1));
        self
    }

    pub async fn generate_opening_question(
        &self,
        participant_id: &str,
        topic: &str,
        trace_context: InferenceTraceContext,
    ) -> Result<AgentReply, String> {
        let instruction = format!(
            "{}\n\nYou are starting an experiment session. Ask exactly one concise, neutral opening question for the participant. Return only the question text. Maximum {} sentences.",
            self.assistant_prompt, self.max_sentences
        );
        let input = format!(
            "Participant ID: {}\nTopic: {}\nGenerate the opening question now.",
            participant_id.trim(),
            topic.trim()
        );

        let response = self
            .provider
            .chat(ChatRequest {
                model: self.model.as_str(),
                instruction: &instruction,
                input: &input,
                max_output_tokens: None,
                stop_epoch: None,
                trace_context,
            })
            .await
            .map_err(|e| e.to_string())?;

        Ok(AgentReply {
            text: clamp_to_max_sentences(response.text.trim_end(), self.max_sentences),
            model: Some(response.model),
            timing: response.timing,
        })
    }

    pub async fn reply_from_memory(
        &self,
        memory: &[AgentMemoryMessage],
        trace_context: InferenceTraceContext,
    ) -> Result<AgentReply, String> {
        let instruction = format!(
            "{}\n\nYou are the active intelligent agent for this conversation. Use the provided memory context to stay coherent across turns. Never exceed {} sentences in your answer.",
            self.assistant_prompt, self.max_sentences
        );
        let input = format_memory_input(memory, self.history_size, self.token_budget);

        let response = self
            .provider
            .chat(ChatRequest {
                model: self.model.as_str(),
                instruction: &instruction,
                input: &input,
                max_output_tokens: None,
                stop_epoch: None,
                trace_context,
            })
            .await
            .map_err(|e| e.to_string())?;

        Ok(AgentReply {
            text: clamp_to_max_sentences(response.text.trim_end(), self.max_sentences),
            model: Some(response.model),
            timing: response.timing,
        })
    }
}

pub fn timing_to_inference_snapshot(
    timing: &InferenceTimingSnapshot,
    model: Option<String>,
) -> InferenceSnapshot {
    InferenceSnapshot {
        model,
        duration_ms: timing.duration_ms,
        ttft_us: timing.ttft_us,
        input_chars: timing.input_chars,
        output_chars: timing.output_chars,
        prompt_token_count: timing.prompt_token_count,
        candidates_token_count: timing.candidates_token_count,
        total_token_count: timing.total_token_count,
        source: timing.source.clone(),
        success: timing.success,
        error: timing.error.clone(),
    }
}

fn format_memory_input(
    memory: &[AgentMemoryMessage],
    history_size: usize,
    token_budget_limit: Option<usize>,
) -> String {
    let start = memory.len().saturating_sub(history_size);
    let windowed = &memory[start..];

    let lines: Vec<String> = windowed
        .iter()
        .map(|msg| {
            let role = match msg.role {
                AgentRole::User => "User",
                AgentRole::Assistant => "Assistant",
            };
            format!("{role}: {}", msg.content.trim())
        })
        .collect();

    let line_refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let kept: Vec<&str> = match token_budget_limit {
        Some(budget) => token_budget::trim_to_token_budget(&line_refs, budget),
        None => line_refs,
    };

    let mut out = String::from("Conversation memory (oldest to newest in this window):\n");
    for line in kept {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("\nRespond to the latest user message.");
    out
}

fn clamp_to_max_sentences(text: &str, max_sentences: usize) -> String {
    if text.trim().is_empty() {
        return String::new();
    }

    let mut count = 0usize;
    let mut cutoff = text.len();
    let chars: Vec<(usize, char)> = text.char_indices().collect();

    for (i, (idx, ch)) in chars.iter().enumerate() {
        if matches!(ch, '.' | '!' | '?') {
            count += 1;
            if count >= max_sentences {
                let next_idx = if i + 1 < chars.len() {
                    chars[i + 1].0
                } else {
                    text.len()
                };
                cutoff = next_idx;
                break;
            }
        }
        if *idx == text.len() {
            break;
        }
    }

    text[..cutoff].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_memory_input_without_token_budget_matches_prior_behavior() {
        let memory = vec![
            AgentMemoryMessage {
                role: AgentRole::User,
                content: "hello".to_string(),
            },
            AgentMemoryMessage {
                role: AgentRole::Assistant,
                content: "hi there".to_string(),
            },
        ];
        let out = format_memory_input(&memory, 5, None);
        assert!(out.contains("User: hello"));
        assert!(out.contains("Assistant: hi there"));
        assert!(out.ends_with("Respond to the latest user message."));
    }

    #[test]
    fn format_memory_input_with_token_budget_trims_oldest_first() {
        let memory = vec![
            AgentMemoryMessage {
                role: AgentRole::User,
                content: "a".repeat(200),
            },
            AgentMemoryMessage {
                role: AgentRole::Assistant,
                content: "recent".to_string(),
            },
        ];
        let out = format_memory_input(&memory, 5, Some(2));
        assert!(!out.contains(&"a".repeat(200)));
        assert!(out.contains("Assistant: recent"));
    }
}
