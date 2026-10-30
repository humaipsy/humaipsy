use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::Deserialize;

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagModel>,
}

#[derive(Deserialize)]
struct OllamaTagModel {
    name: String,
}

#[derive(Deserialize)]
struct OllamaVersionResponse {
    version: String,
}

/// Result of the metadata-only health check behind Settings' "Ping Ollama".
#[derive(Clone, Debug)]
pub(crate) struct PingResult {
    /// `None` when the daemon predates `GET /api/version` (or answers it with
    /// something unparseable); the ping still succeeds because `/api/tags` is
    /// the stable fallback.
    pub version: Option<String>,
    pub model_count: usize,
    pub latency_ms: u128,
}

pub(crate) fn normalize_ollama_host(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "http://127.0.0.1:11434".to_string();
    }
    let s = s.trim_end_matches('/').to_string();
    if s.contains("://") {
        s
    } else {
        format!("http://{}", s)
    }
}

/// A ping must never load a model: it is two small metadata GETs
/// (`/api/version` + `/api/tags`), not a chat completion. The old
/// implementation called `/api/chat`, so a cold 20+ GB model had to be loaded
/// into memory before the "ping" could answer — that is the delay users saw,
/// and it was never an Ollama-version problem.
pub(crate) async fn ping(ollama_host: &str) -> Result<PingResult> {
    let started = std::time::Instant::now();
    let host = normalize_ollama_host(ollama_host);
    // Both requests are expected to answer in milliseconds on a local daemon;
    // a short timeout keeps an unreachable host from stalling the Settings panel.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap_or_default();

    // Run them together so a healthy ping costs one round-trip, not two.
    let (version, model_count) = futures_util::future::join(
        fetch_version(&client, &host),
        fetch_model_count(&client, &host),
    )
    .await;

    let model_count = model_count
        .map_err(|e| anyhow!("could not reach Ollama at {host}: {e}"))?;

    Ok(PingResult {
        // An older daemon without /api/version still gets a successful ping
        // through /api/tags, so the version is optional rather than fatal.
        version: version.ok(),
        model_count,
        latency_ms: started.elapsed().as_millis(),
    })
}

async fn fetch_version(client: &reqwest::Client, host: &str) -> Result<String> {
    let url = format!("{host}/api/version");
    crate::ollama::guard_ollama_request(&url)?;
    let response = client.get(url).send().await?.error_for_status()?;
    let parsed = response.json::<OllamaVersionResponse>().await?;
    Ok(parsed.version)
}

async fn fetch_model_count(client: &reqwest::Client, host: &str) -> Result<usize> {
    let url = std::env::var("OLLAMA_TAGS_URL")
        .unwrap_or_else(|_| format!("{host}/api/tags"));
    crate::ollama::guard_ollama_request(&url)?;
    let response = client.get(url).send().await?.error_for_status()?;
    let tags = response.json::<OllamaTagsResponse>().await?;
    Ok(tags.models.len())
}

pub(crate) async fn fetch_models(ollama_host: &str) -> Result<Vec<String>> {
    let url = std::env::var("OLLAMA_TAGS_URL")
        .unwrap_or_else(|_| format!("{}/api/tags", normalize_ollama_host(ollama_host)));
    crate::ollama::guard_ollama_request(&url)?;
    // Listing models is a quick, non-streaming request — bound it tightly so
    // an unreachable/hung host doesn't leave the Settings panel stuck loading.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default();
    let response = client.get(url).send().await?.error_for_status()?;
    let tags = response.json::<OllamaTagsResponse>().await?;
    let mut models: Vec<String> = tags.models.into_iter().map(|m| m.name).collect();
    models.sort();
    models.dedup();
    Ok(models)
}
