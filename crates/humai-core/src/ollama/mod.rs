use anyhow::{Result, anyhow};
use reqwest::Url;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

use crate::recording::timing::{duration_us, InferenceTraceContext};

mod client;
mod engine;

static HTTP_POLICY: OnceLock<RwLock<HttpPolicy>> = OnceLock::new();

#[derive(Clone, Copy, Debug)]
pub struct HttpPolicy {
    pub air_gap_enabled: bool,
    pub allow_local_ollama: bool,
}

impl Default for HttpPolicy {
    fn default() -> Self {
        Self {
            air_gap_enabled: false,
            allow_local_ollama: true,
        }
    }
}

fn parse_bool_env(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        Err(_) => default,
    }
}

impl HttpPolicy {
    pub fn from_env() -> HttpPolicy {
        HttpPolicy {
            air_gap_enabled: parse_bool_env("HUMAI_AIR_GAP", false),
            allow_local_ollama: parse_bool_env("HUMAI_ALLOW_LOCAL_OLLAMA", true),
        }
    }
}

pub fn set_http_policy(policy: HttpPolicy) {
    let cell = HTTP_POLICY.get_or_init(|| RwLock::new(policy));
    if let Ok(mut guard) = cell.write() {
        *guard = policy;
    }
}

pub fn current_http_policy() -> HttpPolicy {
    let cell = HTTP_POLICY.get_or_init(|| RwLock::new(HttpPolicy::from_env()));
    match cell.read() {
        Ok(guard) => *guard,
        Err(_) => HttpPolicy::default(),
    }
}

fn parse_url_like(input: &str) -> Result<Url> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("empty URL"));
    }
    if let Ok(url) = Url::parse(trimmed) {
        return Ok(url);
    }
    Url::parse(&format!("http://{trimmed}"))
        .map_err(|e| anyhow!("invalid URL '{trimmed}': {e}"))
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

pub fn guard_ollama_request(ollama_url: &str) -> Result<()> {
    let policy = current_http_policy();
    if !policy.air_gap_enabled {
        return Ok(());
    }
    if !policy.allow_local_ollama {
        return Err(anyhow!(
            "air-gap mode blocked Ollama request (allow local Ollama is disabled)"
        ));
    }

    let parsed = parse_url_like(ollama_url)?;
    let host = parsed.host_str().unwrap_or_default().to_string();
    if is_loopback_host(&host) {
        return Ok(());
    }

    Err(anyhow!(
        "air-gap mode only allows loopback Ollama hosts; got '{}'",
        host
    ))
}

/// Sentinel error message when the user stops inference or starts a new run.
pub const OLLAMA_STOPPED_MSG: &str = "ollama inference stopped";

/// When live epoch differs from `captured_epoch`, streaming stops cooperatively (user Stop).
pub type OllamaStopEpoch = (std::sync::Arc<AtomicU64>, u64);

#[derive(Clone, Debug)]
pub struct TokenUsage {
    pub prompt_token_count: u64,
    pub candidates_token_count: u64,
    pub total_token_count: u64,
}

#[derive(Clone, Debug)]
pub struct InferenceTimingSnapshot {
    pub duration_ms: u128,
    pub ttft_us: Option<u128>,
    pub input_chars: usize,
    pub output_chars: usize,
    pub prompt_token_count: Option<u64>,
    pub candidates_token_count: Option<u64>,
    pub total_token_count: Option<u64>,
    pub source: String,
    pub success: bool,
    pub error: Option<String>,
}

pub struct OllamaInferenceResult {
    pub model: Option<String>,
    pub response: String,
    pub usage: Option<TokenUsage>,
    pub timing: InferenceTimingSnapshot,
}

pub async fn fetch_ollama_models(ollama_host: &str) -> Result<Vec<String>> {
    client::fetch_models(ollama_host).await
}

#[allow(clippy::too_many_arguments)]
pub async fn send_to_ollama(
    ollama_host: &str,
    instruction: &str,
    input: &str,
    limit_token: bool,
    num_predict: &str,
    model_override: Option<&str>,
    stop_epoch: Option<OllamaStopEpoch>,
    trace_context: InferenceTraceContext,
) -> Result<(String, InferenceTimingSnapshot)> {
    let result = send_to_ollama_with_result(
        ollama_host,
        instruction,
        input,
        limit_token,
        num_predict,
        model_override,
        stop_epoch,
        trace_context,
    )
    .await?;
    Ok((result.response, result.timing))
}

#[allow(clippy::too_many_arguments)]
pub async fn send_to_ollama_with_result(
    ollama_host: &str,
    instruction: &str,
    input: &str,
    limit_token: bool,
    num_predict: &str,
    model_override: Option<&str>,
    stop_epoch: Option<OllamaStopEpoch>,
    trace_context: InferenceTraceContext,
) -> Result<OllamaInferenceResult> {
    let t_start = std::time::Instant::now();
    let input_chars = input.chars().count();

    if let Some((epoch, caught)) = &stop_epoch
        && epoch.load(Ordering::SeqCst) != *caught
    {
        return Err(anyhow::anyhow!(OLLAMA_STOPPED_MSG));
    }

    let generation_limit = if limit_token {
        num_predict.parse::<u32>().ok()
    } else {
        None
    };
    let context_window = std::env::var("HUMAI_OLLAMA_CONTEXT_WINDOW")
        .ok()
        .and_then(|v| v.parse::<u32>().ok());
    let engine = engine::InferenceEngine::from_host(ollama_host);
    let infer = match engine
        .infer(engine::InferenceRequest {
            instruction,
            input,
            model_override,
            options: engine::InferenceOptions {
                max_output_tokens: generation_limit,
                context_window,
            },
            stop_epoch,
        })
        .await
    {
        Ok(out) => out,
        Err(e) => {
            return Err(e);
        }
    };

    let ttft_us = infer.ttft.map(duration_us);
    let output_chars = infer.text.chars().count();
    let timing = InferenceTimingSnapshot {
        duration_ms: t_start.elapsed().as_millis(),
        ttft_us,
        input_chars,
        output_chars,
        prompt_token_count: infer.usage.as_ref().map(|u| u.prompt_token_count),
        candidates_token_count: infer.usage.as_ref().map(|u| u.candidates_token_count),
        total_token_count: infer.usage.as_ref().map(|u| u.total_token_count),
        source: trace_context.source,
        success: true,
        error: None,
    };

    Ok(OllamaInferenceResult {
        model: Some(infer.model),
        response: infer.text,
        usage: infer.usage,
        timing,
    })
}

pub async fn test_ollama(ollama_host: &str, model_override: Option<&str>) -> Result<String> {
    let result = send_to_ollama_with_result(
        ollama_host,
        "You are a helpful assistant running locally via Ollama.",
        "Hello, how are you?",
        false,
        "",
        model_override,
        None,
        InferenceTraceContext {
            source: "settings.test_ollama".to_string(),
            ..InferenceTraceContext::default()
        },
    )
    .await?;
    Ok(result.response)
}
