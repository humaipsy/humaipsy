use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagModel>,
}

#[derive(Deserialize)]
struct OllamaTagModel {
    name: String,
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
