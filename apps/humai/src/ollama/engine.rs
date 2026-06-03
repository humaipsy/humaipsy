use std::collections::HashMap;
use std::time::Duration;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Result, anyhow};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use super::{OLLAMA_STOPPED_MSG, OllamaStopEpoch, TokenUsage, client};

#[derive(Clone)]
pub(crate) struct InferenceEngine {
    host: String,
    client: reqwest::Client,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InferenceOptions {
    pub max_output_tokens: Option<u32>,
    pub context_window: Option<u32>,
}

pub(crate) struct InferenceRequest<'a> {
    pub instruction: &'a str,
    pub input: &'a str,
    pub model_override: Option<&'a str>,
    pub options: InferenceOptions,
    pub stop_epoch: Option<OllamaStopEpoch>,
}

pub(crate) struct InferenceResponse {
    pub model: String,
    pub text: String,
    pub usage: Option<TokenUsage>,
    pub ttft: Option<Duration>,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<ChatMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<ChatOptions>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_ctx: Option<u32>,
}

#[derive(Deserialize)]
struct ChatResponse {
    model: Option<String>,
    message: Option<ChatMessageOwned>,
    response: Option<String>,
    prompt_eval_count: Option<u64>,
    eval_count: Option<u64>,
    done: Option<bool>,
}

#[derive(Deserialize)]
struct ChatMessageOwned {
    content: String,
}

impl InferenceEngine {
    pub(crate) fn from_host(ollama_host: &str) -> Arc<Self> {
        static CACHE: OnceLock<Mutex<HashMap<String, Arc<InferenceEngine>>>> = OnceLock::new();
        let host = client::normalize_ollama_host(ollama_host);
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = cache.lock().unwrap();
        if let Some(existing) = guard.get(&host) {
            return existing.clone();
        }
        let engine = Arc::new(Self {
            host: host.clone(),
            client: reqwest::Client::new(),
        });
        guard.insert(host, engine.clone());
        engine
    }

    pub(crate) async fn infer(&self, req: InferenceRequest<'_>) -> Result<InferenceResponse> {
        if let Some((epoch, caught)) = &req.stop_epoch
            && epoch.load(std::sync::atomic::Ordering::SeqCst) != *caught
        {
            return Err(anyhow!(OLLAMA_STOPPED_MSG));
        }

        let model = if let Some(m) = req.model_override {
            if m.trim().is_empty() {
                std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "glm-4.7-flash:latest".to_string())
            } else {
                m.to_string()
            }
        } else {
            std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "glm-4.7-flash:latest".to_string())
        };

        let url = format!("{}/api/chat", self.host);
        crate::web::guard_ollama_request(&url)?;

        let options = if req.options.max_output_tokens.is_some() || req.options.context_window.is_some() {
            Some(ChatOptions {
                num_predict: req.options.max_output_tokens,
                num_ctx: req.options.context_window,
            })
        } else {
            None
        };

        let payload = ChatRequest {
            model: &model,
            stream: true,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: req.instruction,
                },
                ChatMessage {
                    role: "user",
                    content: req.input,
                },
            ],
            options,
        };

        let response = self
            .client
            .post(url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        if let Some((epoch, caught)) = &req.stop_epoch
            && epoch.load(std::sync::atomic::Ordering::SeqCst) != *caught
        {
            return Err(anyhow!(OLLAMA_STOPPED_MSG));
        }

        let stream_started = std::time::Instant::now();
        let mut ttft: Option<Duration> = None;
        let mut model_seen = parsed_model_default(&model);
        let mut prompt_eval_count: Option<u64> = None;
        let mut eval_count: Option<u64> = None;
        let mut text = String::new();
        let mut pending = String::new();

        let mut stream = response.bytes_stream();
        while let Some(chunk_result) = stream.next().await {
            if let Some((epoch, caught)) = &req.stop_epoch
                && epoch.load(std::sync::atomic::Ordering::SeqCst) != *caught
            {
                return Err(anyhow!(OLLAMA_STOPPED_MSG));
            }

            let chunk = chunk_result?;
            if ttft.is_none() && !chunk.is_empty() {
                ttft = Some(stream_started.elapsed());
            }
            pending.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(nl) = pending.find('\n') {
                let line = pending[..nl].trim();
                if !line.is_empty()
                    && let Ok(parsed) = serde_json::from_str::<ChatResponse>(line)
                {
                    apply_chat_response_fragment(
                        &parsed,
                        &mut model_seen,
                        &mut text,
                        &mut prompt_eval_count,
                        &mut eval_count,
                    );
                    if parsed.done.unwrap_or(false) {
                        // Continue draining stream in case server sends trailing lines.
                    }
                }
                pending.drain(..=nl);
            }
        }

        let tail = pending.trim();
        if !tail.is_empty()
            && let Ok(parsed) = serde_json::from_str::<ChatResponse>(tail)
        {
            apply_chat_response_fragment(
                &parsed,
                &mut model_seen,
                &mut text,
                &mut prompt_eval_count,
                &mut eval_count,
            );
        }

        if text.trim().is_empty() {
            return Err(anyhow!("ollama returned an empty response"));
        }

        let usage = match (prompt_eval_count, eval_count) {
            (Some(prompt), Some(candidates)) => Some(TokenUsage {
                prompt_token_count: prompt,
                candidates_token_count: candidates,
                total_token_count: prompt + candidates,
            }),
            _ => None,
        };

        Ok(InferenceResponse {
            model: model_seen,
            text,
            usage,
            ttft,
        })
    }
}

fn parsed_model_default(default_model: &str) -> String {
    default_model.to_string()
}

fn apply_chat_response_fragment(
    parsed: &ChatResponse,
    model_seen: &mut String,
    text: &mut String,
    prompt_eval_count: &mut Option<u64>,
    eval_count: &mut Option<u64>,
) {
    if let Some(model) = &parsed.model {
        *model_seen = model.clone();
    }
    if let Some(message) = &parsed.message {
        text.push_str(&message.content);
    } else if let Some(resp) = &parsed.response {
        text.push_str(resp);
    }
    if parsed.prompt_eval_count.is_some() {
        *prompt_eval_count = parsed.prompt_eval_count;
    }
    if parsed.eval_count.is_some() {
        *eval_count = parsed.eval_count;
    }
}
