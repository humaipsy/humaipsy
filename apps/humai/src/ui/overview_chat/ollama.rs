use serde::{Deserialize, Serialize};

pub const OLLAMA_URL: &str = "http://127.0.0.1:11434";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OllamaMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OllamaChatOptions {
    pub num_predict: Option<i32>,
    pub temperature: Option<f64>,
    pub seed: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    options: OllamaChatOptions,
}

#[derive(Clone, Debug, Deserialize)]
struct OllamaChatResponse {
    message: Option<OllamaMessage>,
    response: Option<String>,
}

pub async fn chat(
    model: &str,
    messages: &[OllamaMessage],
    options: OllamaChatOptions,
) -> Result<String, String> {
    let req = OllamaChatRequest {
        model: model.to_string(),
        messages: messages.to_vec(),
        stream: false,
        options,
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/chat", OLLAMA_URL))
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("ollama request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("ollama returned {status}: {body}"));
    }

    let parsed: OllamaChatResponse = resp
        .json()
        .await
        .map_err(|e| format!("ollama response parse failed: {e}"))?;

    if let Some(msg) = parsed.message {
        return Ok(msg.content);
    }
    if let Some(text) = parsed.response {
        return Ok(text);
    }

    Err("ollama response missing message content".to_string())
}
