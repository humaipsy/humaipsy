//! The only [`super::Provider`] implementation today. A thin adapter over
//! `crate::ollama` — no new HTTP logic, no changes to `guard_ollama_request`/
//! `HttpPolicy` air-gap enforcement, no changes to the streaming engine. It
//! exists so `HumaiPsyAgent` can depend on `Provider` instead of on Ollama by name.

use super::{ChatRequest, ChatResponse, Provider};

pub struct OllamaProvider {
    ollama_host: String,
}

impl OllamaProvider {
    pub fn new(ollama_host: String) -> Self {
        Self { ollama_host }
    }
}

#[async_trait::async_trait]
impl Provider for OllamaProvider {
    fn id(&self) -> &'static str {
        "ollama"
    }

    async fn chat(&self, request: ChatRequest<'_>) -> anyhow::Result<ChatResponse> {
        let limit_token = request.max_output_tokens.is_some();
        let num_predict = request
            .max_output_tokens
            .map(|v| v.to_string())
            .unwrap_or_default();

        let result = crate::ollama::send_to_ollama_with_result(
            &self.ollama_host,
            request.instruction,
            request.input,
            limit_token,
            &num_predict,
            // Empty model string is intentionally passed through as-is —
            // send_to_ollama_with_result already falls back to
            // ollama::DEFAULT_OLLAMA_MODEL when the override is empty.
            Some(request.model),
            request.stop_epoch,
            request.trace_context,
        )
        .await?;

        Ok(ChatResponse {
            model: result.model.unwrap_or_default(),
            text: result.response,
            usage: result.usage,
            timing: result.timing,
        })
    }

    async fn list_models(&self) -> anyhow::Result<Vec<String>> {
        crate::ollama::fetch_ollama_models(&self.ollama_host).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable() {
        let provider = OllamaProvider::new("http://127.0.0.1:11434".to_string());
        assert_eq!(provider.id(), "ollama");
    }
}
