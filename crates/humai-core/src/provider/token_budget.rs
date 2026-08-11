//! Provider-agnostic, approximate token budgeting for trimming conversation
//! history before it's sent to a model. There is no real tokenizer for any
//! backend wired up here (Ollama models, and any future OpenAI/Anthropic
//! backend, all use different tokenizers) — this is a character-count
//! heuristic, not an exact count. Treat it as a rough safety margin, not a
//! guarantee of fitting a specific context window.

/// ~4 characters per token is a commonly-cited rough approximation for
/// English text across GPT/Llama-family tokenizers. Not exact — don't rely on
/// this for a hard token-count guarantee.
const APPROX_CHARS_PER_TOKEN: usize = 4;

/// Rough token-count estimate for `text`. Zero for empty input.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(APPROX_CHARS_PER_TOKEN)
}

/// Trims `entries` (oldest to newest) from the front until the estimated
/// total fits within `budget`, keeping the most recent entries. Always keeps
/// at least the single most recent entry, even if its estimated size alone
/// exceeds `budget` — a caller asking for a memory window should get
/// *something* recent back, not nothing, if the budget is unreasonably small.
/// Returns entries in their original (oldest-to-newest) order.
pub fn trim_to_token_budget<'a>(entries: &[&'a str], budget: usize) -> Vec<&'a str> {
    if entries.is_empty() {
        return Vec::new();
    }

    let mut kept_from = entries.len();
    let mut total = 0usize;

    for (i, entry) in entries.iter().enumerate().rev() {
        let tokens = estimate_tokens(entry);
        if total + tokens > budget && kept_from != entries.len() {
            break;
        }
        total += tokens;
        kept_from = i;
        if total > budget {
            // This entry alone already exceeds the budget; it's the most
            // recent one so far, so keep it and stop rather than dropping to
            // zero entries.
            break;
        }
    }

    entries[kept_from..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_is_roughly_four_chars_per_token() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("a".repeat(40).as_str()), 10);
    }

    #[test]
    fn trim_to_token_budget_keeps_all_when_under_budget() {
        let entries = ["one", "two", "three"];
        let refs: Vec<&str> = entries.to_vec();
        assert_eq!(trim_to_token_budget(&refs, 1000), refs);
    }

    #[test]
    fn trim_to_token_budget_keeps_most_recent_entries_within_budget() {
        // Each entry below is exactly one "token" (<=4 chars).
        let entries = ["aa", "bb", "cc", "dd"];
        let refs: Vec<&str> = entries.to_vec();
        assert_eq!(trim_to_token_budget(&refs, 2), vec!["cc", "dd"]);
    }

    #[test]
    fn trim_to_token_budget_never_returns_empty_for_nonempty_input() {
        let huge_entry = "x".repeat(400); // ~100 estimated tokens, alone over budget
        let entries = ["short", huge_entry.as_str()];
        let trimmed = trim_to_token_budget(&entries, 1);
        assert_eq!(trimmed, vec![huge_entry.as_str()]);
    }

    #[test]
    fn trim_to_token_budget_handles_empty_input() {
        let empty: Vec<&str> = Vec::new();
        assert!(trim_to_token_budget(&empty, 100).is_empty());
    }
}
