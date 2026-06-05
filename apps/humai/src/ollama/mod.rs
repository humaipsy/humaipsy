use anyhow::{Result, anyhow};
use reqwest::Url;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::SystemTime;

use crate::app_state::AppState;
use crate::metrics::{InferenceTimingEvent, InferenceTraceContext};
use crate::recording::timing::{duration_ms_ceil, duration_us, now_rfc3339_utc, EVENT_TYPE_INFERENCE_TIMING};

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
pub type OllamaStopEpoch = (Arc<AtomicU64>, u64);

#[derive(Clone, Debug)]
pub struct TokenUsage {
    pub prompt_token_count: u64,
    pub candidates_token_count: u64,
    pub total_token_count: u64,
}

pub struct OllamaInferenceResult {
    pub model: Option<String>,
    pub response: String,
    pub usage: Option<TokenUsage>,
}

fn format_rfc3339(ts: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(ts).to_rfc3339()
}

pub async fn fetch_ollama_models(ollama_host: &str) -> Result<Vec<String>> {
    client::fetch_models(ollama_host).await
}

pub async fn send_to_ollama(
    ollama_host: &str,
    instruction: &str,
    input: &str,
    limit_token: bool,
    num_predict: &str,
    model_override: Option<&str>,
    stop_epoch: Option<OllamaStopEpoch>,
    app_state: Arc<AppState>,
    trace_context: InferenceTraceContext,
) -> Result<String> {
    let result = send_to_ollama_with_result(
        ollama_host,
        instruction,
        input,
        limit_token,
        num_predict,
        model_override,
        stop_epoch,
        app_state,
        trace_context,
    )
    .await?;
    Ok(result.response)
}

pub async fn send_to_ollama_with_result(
    ollama_host: &str,
    instruction: &str,
    input: &str,
    limit_token: bool,
    num_predict: &str,
    model_override: Option<&str>,
    stop_epoch: Option<OllamaStopEpoch>,
    app_state: Arc<AppState>,
    trace_context: InferenceTraceContext,
) -> Result<OllamaInferenceResult> {
    let metrics_sink = app_state.metrics_sink();
    let t_start_wall = SystemTime::now();
    let t_start = std::time::Instant::now();

    if let Some((epoch, caught)) = &stop_epoch {
        if epoch.load(Ordering::SeqCst) != *caught {
            let t_end_wall = SystemTime::now();
            metrics_sink.record_inference(InferenceTimingEvent {
                event_type: EVENT_TYPE_INFERENCE_TIMING.to_string(),
                timestamp: now_rfc3339_utc(),
                source: trace_context.source.clone(),
                experiment_id: trace_context.experiment_id.clone(),
                run_id: trace_context.run_id.clone(),
                node_global_id: trace_context.node_global_id.clone(),
                model: model_override.map(|m| m.to_string()),
                success: false,
                error: Some(OLLAMA_STOPPED_MSG.to_string()),
                t_start: format_rfc3339(t_start_wall),
                t_first_token: None,
                t_end: format_rfc3339(t_end_wall),
                duration_ms: t_start.elapsed().as_millis(),
                ttft_ms: None,
                ttft_us: None,
                input_chars: input.chars().count(),
                output_chars: 0,
                prompt_token_count: None,
                candidates_token_count: None,
                total_token_count: None,
                turn_index: trace_context.turn_index,
                prompt: Some(input.to_string()),
            });
            return Err(anyhow::anyhow!(OLLAMA_STOPPED_MSG));
        }
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
            let t_end_wall = SystemTime::now();
            metrics_sink.record_inference(InferenceTimingEvent {
                event_type: EVENT_TYPE_INFERENCE_TIMING.to_string(),
                timestamp: now_rfc3339_utc(),
                source: trace_context.source.clone(),
                experiment_id: trace_context.experiment_id.clone(),
                run_id: trace_context.run_id.clone(),
                node_global_id: trace_context.node_global_id.clone(),
                model: model_override.map(|m| m.to_string()),
                success: false,
                error: Some(e.to_string()),
                t_start: format_rfc3339(t_start_wall),
                t_first_token: None,
                t_end: format_rfc3339(t_end_wall),
                duration_ms: t_start.elapsed().as_millis(),
                ttft_ms: None,
                ttft_us: None,
                input_chars: input.chars().count(),
                output_chars: 0,
                prompt_token_count: None,
                candidates_token_count: None,
                total_token_count: None,
                turn_index: trace_context.turn_index,
                prompt: Some(input.to_string()),
            });
            return Err(e);
        }
    };

    let t_end_wall = SystemTime::now();
    let first_token_at = infer
        .ttft
        .and_then(|d| t_start_wall.checked_add(d))
        .map(format_rfc3339);
    let ttft_us = infer.ttft.map(duration_us);
    let ttft_ms = infer.ttft.map(duration_ms_ceil);
    let output_chars = infer.text.chars().count();
    metrics_sink.record_inference(InferenceTimingEvent {
        event_type: EVENT_TYPE_INFERENCE_TIMING.to_string(),
        timestamp: now_rfc3339_utc(),
        source: trace_context.source,
        experiment_id: trace_context.experiment_id,
        run_id: trace_context.run_id,
        node_global_id: trace_context.node_global_id,
        model: Some(infer.model.clone()),
        success: true,
        error: None,
        t_start: format_rfc3339(t_start_wall),
        t_first_token: first_token_at,
        t_end: format_rfc3339(t_end_wall),
        duration_ms: t_start.elapsed().as_millis(),
        ttft_ms,
        ttft_us,
        input_chars: input.chars().count(),
        output_chars,
        prompt_token_count: infer.usage.as_ref().map(|u| u.prompt_token_count),
        candidates_token_count: infer.usage.as_ref().map(|u| u.candidates_token_count),
        total_token_count: infer.usage.as_ref().map(|u| u.total_token_count),
        turn_index: trace_context.turn_index,
        prompt: Some(input.to_string()),
    });

    Ok(OllamaInferenceResult {
        model: Some(infer.model),
        response: infer.text,
        usage: infer.usage,
    })
}

pub async fn test_ollama(
    ollama_host: &str,
    model_override: Option<&str>,
    app_state: Arc<AppState>,
) -> Result<String> {
    let result = send_to_ollama_with_result(
        ollama_host,
        "You are a helpful assistant running locally via Ollama.",
        "Hello, how are you?",
        false,
        "",
        model_override,
        None,
        app_state,
        InferenceTraceContext {
            source: "settings.test_ollama".to_string(),
            experiment_id: None,
            run_id: None,
            node_global_id: None,
            turn_index: None,
        },
    )
    .await?;
    Ok(result.response)
}
