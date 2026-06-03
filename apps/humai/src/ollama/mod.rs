use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use crate::app_state::AppState;
use crate::metrics::{InferenceTimingEvent, InferenceTraceContext};
use crate::recording::timing::{duration_ms_ceil, duration_us, now_rfc3339_utc, EVENT_TYPE_INFERENCE_TIMING};

mod client;
mod engine;

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
    let context_window = std::env::var("ARPSCI_OLLAMA_CONTEXT_WINDOW")
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
