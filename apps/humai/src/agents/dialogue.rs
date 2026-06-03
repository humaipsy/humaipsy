use std::collections::{HashMap, VecDeque};

use crate::ollama::TokenUsage;

#[derive(Clone)]
pub struct DialogueMessage {
    pub agent_id: usize,
    pub agent_name: String,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct TokenBudgetStats {
    pub last_prompt_tokens: Option<u64>,
    pub last_output_tokens: Option<u64>,
    pub last_total_tokens: Option<u64>,
    pub avg_total_tokens: Option<u64>,
    samples: u64,
}

impl TokenBudgetStats {
    pub fn record_usage(&mut self, usage: Option<&TokenUsage>) {
        let Some(usage) = usage else {
            return;
        };
        self.last_prompt_tokens = Some(usage.prompt_token_count);
        self.last_output_tokens = Some(usage.candidates_token_count);
        self.last_total_tokens = Some(usage.total_token_count);
        self.samples += 1;
        let prior = self.avg_total_tokens.unwrap_or(usage.total_token_count);
        let weighted = ((prior * (self.samples - 1)) + usage.total_token_count) / self.samples;
        self.avg_total_tokens = Some(weighted);
    }

    fn as_line(&self) -> Option<String> {
        self.last_total_tokens.map(|total| {
            let avg = self.avg_total_tokens.unwrap_or(total);
            format!("Token budget (last/avg total): {total}/{avg}")
        })
    }
}

pub struct DialogueSessionState {
    pub session_id: String,
    rolling_summary: String,
    recent_exchanges: VecDeque<DialogueMessage>,
    per_agent_last: HashMap<usize, String>,
    max_recent: usize,
    pub token_budget: TokenBudgetStats,
}

impl DialogueSessionState {
    pub fn new(session_id: String, max_recent: usize) -> Self {
        Self {
            session_id,
            rolling_summary: String::new(),
            recent_exchanges: VecDeque::new(),
            per_agent_last: HashMap::new(),
            max_recent: max_recent.max(1),
            token_budget: TokenBudgetStats::default(),
        }
    }

    pub fn last_message_from_agent(&self, agent_id: usize) -> Option<&str> {
        self.per_agent_last.get(&agent_id).map(String::as_str)
    }

    pub fn record_turn(
        &mut self,
        agent_id: usize,
        agent_name: String,
        message: String,
        usage: Option<&TokenUsage>,
    ) {
        self.per_agent_last.insert(agent_id, message.clone());
        self.recent_exchanges.push_back(DialogueMessage {
            agent_id,
            agent_name,
            message,
        });
        while self.recent_exchanges.len() > self.max_recent {
            if let Some(old) = self.recent_exchanges.pop_front() {
                if !self.rolling_summary.is_empty() {
                    self.rolling_summary.push(' ');
                }
                self.rolling_summary.push_str(&format!(
                    "{} said: {}",
                    old.agent_name,
                    truncate_for_summary(&old.message)
                ));
            }
        }
        self.token_budget.record_usage(usage);
    }

    pub fn memory_block(&self, partner_name: &str, topic: &str) -> String {
        if self.recent_exchanges.is_empty() && self.rolling_summary.is_empty() {
            return format!(
                "Session {}. You are discussing \"{}\" with {}. Please start the conversation.",
                self.session_id, topic, partner_name
            );
        }

        let mut out = format!(
            "Session {}. Topic: \"{}\" with {}.\n",
            self.session_id, topic, partner_name
        );
        if !self.rolling_summary.is_empty() {
            out.push_str("Summary so far:\n");
            out.push_str(&self.rolling_summary);
            out.push_str("\n\n");
        }
        if let Some(budget) = self.token_budget.as_line() {
            out.push_str(&budget);
            out.push_str("\n\n");
        }
        out.push_str("Recent exchanges:\n");
        for msg in &self.recent_exchanges {
            out.push_str(&format!("{}: {}\n", msg.agent_name, msg.message));
        }
        out
    }
}

pub struct PromptBuildInput<'a> {
    pub base_instruction: &'a str,
    pub manager_name: &'a str,
    pub turn_index: usize,
    pub sender_name: &'a str,
    pub receiver_name: &'a str,
    pub topic: &'a str,
    pub memory_block: &'a str,
    pub sidecar_augmentation: &'a str,
}

pub struct AssembledPrompt {
    pub system_instruction: String,
    pub memory_block: String,
    pub turn_directive: String,
    pub sidecar_augmentation: String,
    pub user_prompt: String,
}

pub struct PromptAssembler;

impl PromptAssembler {
    pub fn assemble(input: PromptBuildInput<'_>) -> AssembledPrompt {
        let turn_directive = format!(
            "Step {} directive from {}: Respond only as {} to {} about \"{}\". Keep it concise (2-3 sentences).",
            input.turn_index + 1,
            input.manager_name,
            input.sender_name,
            input.receiver_name,
            input.topic
        );
        let system_instruction = format!(
            "{}\n\n{}\n\nRespond as yourself only. Do not fabricate extra speakers.",
            input.base_instruction, turn_directive
        );
        let mut user_prompt = input.memory_block.to_string();
        if !input.sidecar_augmentation.is_empty() {
            user_prompt.push_str("\n\n---\nSidecar augmentation\n");
            user_prompt.push_str(input.sidecar_augmentation);
            user_prompt.push_str("\n---");
        }
        user_prompt.push_str(&format!(
            "\n\nYour turn: respond to {}'s latest message.",
            input.receiver_name
        ));

        AssembledPrompt {
            system_instruction,
            memory_block: input.memory_block.to_string(),
            turn_directive,
            sidecar_augmentation: input.sidecar_augmentation.to_string(),
            user_prompt,
        }
    }
}

fn truncate_for_summary(text: &str) -> String {
    const MAX: usize = 140;
    if text.len() <= MAX {
        return text.to_string();
    }
    let mut cut = MAX;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}...", &text[..cut])
}

#[cfg(test)]
mod tests {
    use super::{DialogueSessionState, PromptAssembler, PromptBuildInput};

    #[test]
    fn prompt_contains_all_sections() {
        let p = PromptAssembler::assemble(PromptBuildInput {
            base_instruction: "You are helpful",
            manager_name: "Manager",
            turn_index: 2,
            sender_name: "A",
            receiver_name: "B",
            topic: "X",
            memory_block: "history",
            sidecar_augmentation: "refs",
        });
        assert!(p.system_instruction.contains("Manager"));
        assert!(p.user_prompt.contains("history"));
        assert!(p.user_prompt.contains("refs"));
    }

    #[test]
    fn session_summary_rolls_when_history_exceeds_limit() {
        let mut s = DialogueSessionState::new("sid".to_string(), 1);
        s.record_turn(1, "A".to_string(), "First line".to_string(), None);
        s.record_turn(2, "B".to_string(), "Second line".to_string(), None);
        let block = s.memory_block("A", "Topic");
        assert!(block.contains("Summary so far"));
        assert!(block.contains("Second line"));
    }
}
