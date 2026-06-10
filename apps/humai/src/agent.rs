use crate::metrics::InferenceSnapshot;
use crate::ollama::InferenceTimingSnapshot;
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
pub struct HumaiAgent {
    ollama_host: String,
    model: String,
    assistant_prompt: String,
    history_size: usize,
    max_sentences: usize,
}

impl HumaiAgent {
    pub fn new(ollama_host: String, model: String, assistant_prompt: String) -> Self {
        Self {
            ollama_host,
            model,
            assistant_prompt,
            history_size: 5,
            max_sentences: 5,
        }
    }

    pub fn with_history_size(mut self, history_size: usize) -> Self {
        self.history_size = history_size.max(1);
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

        let (out, timing) = crate::ollama::send_to_ollama(
            &self.ollama_host,
            &instruction,
            &input,
            false,
            "",
            Some(self.model.as_str()),
            None,
            trace_context,
        )
        .await
        .map_err(|e| e.to_string())?;

        Ok(AgentReply {
            text: clamp_to_max_sentences(out.trim_end(), self.max_sentences),
            model: Some(self.model.clone()),
            timing,
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
        let input = format_memory_input(memory, self.history_size);

        let (out, timing) = crate::ollama::send_to_ollama(
            &self.ollama_host,
            &instruction,
            &input,
            false,
            "",
            Some(self.model.as_str()),
            None,
            trace_context,
        )
        .await
        .map_err(|e| e.to_string())?;

        Ok(AgentReply {
            text: clamp_to_max_sentences(out.trim_end(), self.max_sentences),
            model: Some(self.model.clone()),
            timing,
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

fn format_memory_input(memory: &[AgentMemoryMessage], history_size: usize) -> String {
    let start = memory.len().saturating_sub(history_size);
    let mut out = String::from(
        "Conversation memory (oldest to newest in this window):\n",
    );

    for msg in &memory[start..] {
        let role = match msg.role {
            AgentRole::User => "User",
            AgentRole::Assistant => "Assistant",
        };
        out.push_str(role);
        out.push_str(": ");
        out.push_str(msg.content.trim());
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
