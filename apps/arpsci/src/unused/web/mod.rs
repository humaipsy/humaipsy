use crate::run::event_ledger::EventLedger;
use crate::run::manifest::RunContext;
use anyhow::{Result, anyhow};
use rocket::figment::Figment;
use rocket::response::content::RawHtml;
use rocket::serde::json::Json;
use rocket::{Build, Config, Rocket, routes};
use reqwest::Url;
use serde::Serialize;
use serde::{Deserialize, Serialize as DeriveSerialize};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::runtime::Handle;

static OUTGOING_HTTP_LOG: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static HTTP_POLICY: OnceLock<RwLock<HttpPolicy>> = OnceLock::new();

fn outgoing_http_log() -> &'static Mutex<Vec<String>> {
    OUTGOING_HTTP_LOG.get_or_init(|| Mutex::new(Vec::new()))
}

fn trim_line(input: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for c in input.chars().take(max_chars) {
        out.push(c);
    }
    if input.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn push_outgoing_http_log_line(line: String) {
    let mut log = outgoing_http_log().lock().unwrap();
    log.push(line);
    if log.len() > 400 {
        let keep_from = log.len() - 400;
        log.drain(0..keep_from);
    }
}

pub fn get_outgoing_http_log_lines() -> Vec<String> {
    outgoing_http_log().lock().unwrap().clone()
}

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

#[derive(Clone, Debug)]
struct WebConfig {
    enabled: bool,
    address: String,
    port: u16,
}

impl WebConfig {
    fn from_env() -> Self {
        let enabled = parse_bool_env("ARPSCI_WEB_ENABLED", false);
        let address = std::env::var("ARPSCI_WEB_ADDRESS").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = std::env::var("ARPSCI_WEB_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(8000);

        Self {
            enabled,
            address,
            port,
        }
    }
}

pub fn outbound_webhooks_enabled() -> bool {
    parse_bool_env("ARPSCI_WEBHOOKS_ENABLED", false)
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
            air_gap_enabled: parse_bool_env("ARPSCI_AIR_GAP", false),
            allow_local_ollama: parse_bool_env("ARPSCI_ALLOW_LOCAL_OLLAMA", true),
        }
    }
}

pub fn set_policy(policy: HttpPolicy) {
    let cell = HTTP_POLICY.get_or_init(|| RwLock::new(policy));
    if let Ok(mut guard) = cell.write() {
        *guard = policy;
    }
}

pub fn current_policy() -> HttpPolicy {
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

fn log_blocked_http_event(
    attempted_url: &str,
    component: &str,
    reason: &str,
    ledger: Option<&Arc<EventLedger>>,
) {
    if let Some(l) = ledger {
        let _ = l.append_with_hashes(
            "transport.http_blocked",
            None,
            None,
            attempted_url,
            reason,
            serde_json::json!({
                "attempted_url": attempted_url,
                "component": component,
                "reason": reason,
            }),
        );
    }
}

pub fn guard_http_request(
    attempted_url: &str,
    component: &str,
    ledger: Option<&Arc<EventLedger>>,
) -> Result<()> {
    let policy = current_policy();
    if !policy.air_gap_enabled {
        return Ok(());
    }

    let parsed = parse_url_like(attempted_url)?;
    let host = parsed.host_str().unwrap_or_default().to_string();

    if is_loopback_host(&host) {
        return Ok(());
    }

    let reason = "AirGapPolicy";
    log_blocked_http_event(attempted_url, component, reason, ledger);
    Err(anyhow!(
        "air-gap mode blocked outbound HTTP to '{}' (component: {})",
        host,
        component
    ))
}

pub fn guard_ollama_request(ollama_url: &str) -> Result<()> {
    let policy = current_policy();
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

#[derive(DeriveSerialize, Deserialize, Debug)]
pub struct ConversationMessage {
    pub sender_id: usize,
    pub sender_name: String,
    pub receiver_id: usize,
    pub receiver_name: String,
    pub topic: String,
    pub message: String,
    pub timestamp: String,
    pub experiment_id: Option<String>,
    pub run_id: Option<String>,
    pub manifest_version: Option<String>,
}

pub async fn send_conversation_message(
    endpoint: &str,
    sender_id: usize,
    sender_name: &str,
    receiver_id: usize,
    receiver_name: &str,
    topic: &str,
    message: &str,
    run_context: Option<&RunContext>,
    ledger: Option<&Arc<EventLedger>>,
) -> Result<(), anyhow::Error> {
    if !outbound_webhooks_enabled() {
        return Ok(());
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let timestamp_str = chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp as i64, 0)
        .unwrap()
        .to_rfc3339();

    let payload = ConversationMessage {
        sender_id,
        sender_name: sender_name.to_string(),
        receiver_id,
        receiver_name: receiver_name.to_string(),
        topic: topic.to_string(),
        message: message.to_string(),
        timestamp: timestamp_str,
        experiment_id: run_context.map(|c| c.experiment_id.clone()),
        run_id: run_context.map(|c| c.run_id.clone()),
        manifest_version: run_context.map(|c| c.manifest_version.clone()),
    };

    let payload_json = serde_json::to_string(&payload)?;

    if let Err(e) = guard_http_request(endpoint, "conversation", ledger) {
        push_outgoing_http_log_line(format!(
            "BLOCKED {} | conversation | {} -> {} | {}",
            endpoint,
            sender_name,
            receiver_name,
            trim_line(message, 90),
        ));
        return Err(e);
    }

    push_outgoing_http_log_line(format!(
        "POST {} | conversation | {} -> {} | {}",
        endpoint,
        sender_name,
        receiver_name,
        trim_line(message, 90),
    ));

    let client = reqwest::Client::new();
    let response = match client.post(endpoint).json(&payload).send().await {
        Ok(r) => r,
        Err(e) => {
            if let Some(l) = ledger {
                let _ = l.append_transport_http(
                    "conversation",
                    &payload_json,
                    &e.to_string(),
                    None,
                    Some(&e.to_string()),
                );
            }
            return Err(anyhow!(e));
        }
    };
    let status = response.status();
    let code = status.as_u16();
    let body_text = response.text().await.unwrap_or_default();

    if let Some(l) = ledger {
        let err = if status.is_success() {
            None
        } else {
            Some(format!("HTTP {}", code))
        };
        let _ = l.append_transport_http(
            "conversation",
            &payload_json,
            &body_text,
            Some(code),
            err.as_deref(),
        );
    }

    if !status.is_success() {
        anyhow::bail!("conversation HTTP {}: {}", code, trim_line(&body_text, 200));
    }

    Ok(())
}

#[derive(DeriveSerialize, Deserialize, Debug)]
pub struct EvaluatorResult {
    pub evaluator_name: String,
    pub sentiment: String,
    pub message: String,
    pub timestamp: String,
    pub experiment_id: Option<String>,
    pub run_id: Option<String>,
    pub manifest_version: Option<String>,
}

pub async fn send_evaluator_result(
    endpoint: &str,
    evaluator_name: &str,
    sentiment: &str,
    message: &str,
    run_context: Option<&RunContext>,
    ledger: Option<&Arc<EventLedger>>,
) -> Result<(), anyhow::Error> {
    if !outbound_webhooks_enabled() {
        return Ok(());
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let timestamp_str = chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp as i64, 0)
        .unwrap()
        .to_rfc3339();
    let payload = EvaluatorResult {
        evaluator_name: evaluator_name.to_string(),
        sentiment: sentiment.to_string(),
        message: message.to_string(),
        timestamp: timestamp_str,
        experiment_id: run_context.map(|c| c.experiment_id.clone()),
        run_id: run_context.map(|c| c.run_id.clone()),
        manifest_version: run_context.map(|c| c.manifest_version.clone()),
    };
    let payload_json = serde_json::to_string(&payload)?;

    if let Err(e) = guard_http_request(endpoint, "evaluator", ledger) {
        push_outgoing_http_log_line(format!(
            "BLOCKED {} | evaluator {} [{}] | {}",
            endpoint,
            evaluator_name,
            sentiment,
            trim_line(message, 90),
        ));
        return Err(e);
    }

    push_outgoing_http_log_line(format!(
        "POST {} | evaluator {} [{}] | {}",
        endpoint,
        evaluator_name,
        sentiment,
        trim_line(message, 90),
    ));
    let client = reqwest::Client::new();
    let response = match client.post(endpoint).json(&payload).send().await {
        Ok(r) => r,
        Err(e) => {
            if let Some(l) = ledger {
                let _ = l.append_transport_http(
                    "evaluator",
                    &payload_json,
                    &e.to_string(),
                    None,
                    Some(&e.to_string()),
                );
            }
            return Err(anyhow!(e));
        }
    };
    let status = response.status();
    let code = status.as_u16();
    let body_text = response.text().await.unwrap_or_default();

    if let Some(l) = ledger {
        let err = if status.is_success() {
            None
        } else {
            Some(format!("HTTP {}", code))
        };
        let _ = l.append_transport_http(
            "evaluator",
            &payload_json,
            &body_text,
            Some(code),
            err.as_deref(),
        );
    }

    if !status.is_success() {
        anyhow::bail!("evaluator HTTP {}: {}", code, trim_line(&body_text, 200));
    }
    Ok(())
}

pub async fn send_researcher_result(
    endpoint: &str,
    researcher_name: &str,
    topic: &str,
    message: &str,
    run_context: Option<&RunContext>,
    ledger: Option<&Arc<EventLedger>>,
) -> Result<(), anyhow::Error> {
    if !outbound_webhooks_enabled() {
        return Ok(());
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let timestamp_str = chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp as i64, 0)
        .unwrap()
        .to_rfc3339();
    let payload = EvaluatorResult {
        evaluator_name: researcher_name.to_string(),
        sentiment: format!("references:{}", topic.to_lowercase()),
        message: message.to_string(),
        timestamp: timestamp_str,
        experiment_id: run_context.map(|c| c.experiment_id.clone()),
        run_id: run_context.map(|c| c.run_id.clone()),
        manifest_version: run_context.map(|c| c.manifest_version.clone()),
    };
    let payload_json = serde_json::to_string(&payload)?;

    if let Err(e) = guard_http_request(endpoint, "researcher", ledger) {
        push_outgoing_http_log_line(format!(
            "BLOCKED {} | researcher {} [{}] | {}",
            endpoint,
            researcher_name,
            topic,
            trim_line(message, 90),
        ));
        return Err(e);
    }

    push_outgoing_http_log_line(format!(
        "POST {} | researcher {} [{}] | {}",
        endpoint,
        researcher_name,
        topic,
        trim_line(message, 90),
    ));
    let client = reqwest::Client::new();
    let response = match client.post(endpoint).json(&payload).send().await {
        Ok(r) => r,
        Err(e) => {
            if let Some(l) = ledger {
                let _ = l.append_transport_http(
                    "researcher",
                    &payload_json,
                    &e.to_string(),
                    None,
                    Some(&e.to_string()),
                );
            }
            return Err(anyhow!(e));
        }
    };
    let status = response.status();
    let code = status.as_u16();
    let body_text = response.text().await.unwrap_or_default();

    if let Some(l) = ledger {
        let err = if status.is_success() {
            None
        } else {
            Some(format!("HTTP {}", code))
        };
        let _ = l.append_transport_http(
            "researcher",
            &payload_json,
            &body_text,
            Some(code),
            err.as_deref(),
        );
    }

    if !status.is_success() {
        anyhow::bail!("researcher HTTP {}: {}", code, trim_line(&body_text, 200));
    }
    Ok(())
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

#[derive(Serialize)]
struct CapabilitiesResponse {
    service: &'static str,
    api_version: &'static str,
    endpoints: Vec<&'static str>,
}

#[derive(Deserialize)]
struct BridgePingRequest {
    message: Option<String>,
}

#[derive(Serialize)]
struct BridgePingResponse {
    status: &'static str,
    service: &'static str,
    echoed_message: String,
}

#[derive(Serialize)]
struct OutgoingHttpLogResponse {
    total: usize,
    entries: Vec<OutgoingHttpLogEntry>,
}

#[derive(Serialize)]
struct OutgoingHttpLogEntry {
    index: usize,
    raw: String,
    action: String,
    endpoint: String,
    component: String,
    sender: Option<String>,
    receiver: Option<String>,
    message: String,
    blocked: bool,
}

fn parse_outgoing_http_log_line(index: usize, raw: String) -> OutgoingHttpLogEntry {
    let parts: Vec<&str> = raw.split(" | ").collect();

    let (action, endpoint) = if let Some(first) = parts.first() {
        let mut it = first.splitn(2, ' ');
        (
            it.next().unwrap_or_default().to_string(),
            it.next().unwrap_or_default().to_string(),
        )
    } else {
        (String::new(), String::new())
    };

    let component = parts.get(1).copied().unwrap_or_default().to_string();
    let mut sender = None;
    let mut receiver = None;
    let message = if parts.len() >= 4 {
        let participants = parts[2];
        if let Some((s, r)) = participants.split_once(" -> ") {
            sender = Some(s.trim().to_string());
            receiver = Some(r.trim().to_string());
        }
        parts[3..].join(" | ")
    } else if parts.len() >= 3 {
        parts[2..].join(" | ")
    } else {
        String::new()
    };

    let blocked = action.eq_ignore_ascii_case("BLOCKED");

    OutgoingHttpLogEntry {
        index,
        raw,
        action,
        endpoint,
        component,
        sender,
        receiver,
        message,
        blocked,
    }
}

fn build_outgoing_http_log_response() -> OutgoingHttpLogResponse {
    let lines = get_outgoing_http_log_lines();
    let total = lines.len();
    let entries = lines
        .into_iter()
        .enumerate()
        .map(|(index, raw)| parse_outgoing_http_log_line(index, raw))
        .collect();

    OutgoingHttpLogResponse { total, entries }
}

#[rocket::get("/health")]
fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "arpsci",
    })
}

#[rocket::get("/capabilities")]
fn capabilities() -> Json<CapabilitiesResponse> {
    Json(CapabilitiesResponse {
        service: "arpsci",
        api_version: "v1",
        endpoints: vec!["/api/health", "/api/capabilities", "/api/bridge/ping"],
    })
}

#[rocket::post("/bridge/ping", format = "json", data = "<payload>")]
fn bridge_ping(payload: Json<BridgePingRequest>) -> Json<BridgePingResponse> {
    Json(BridgePingResponse {
        status: "ok",
        service: "arpsci",
        echoed_message: payload
            .message
            .clone()
            .unwrap_or_else(|| "ping".to_string()),
    })
}

#[rocket::get("/outgoing-http-log")]
fn outgoing_http_log_route() -> Json<OutgoingHttpLogResponse> {
        Json(build_outgoing_http_log_response())
}

#[rocket::get("/outgoing-http-log/live")]
fn outgoing_http_log_live_route() -> RawHtml<String> {
        RawHtml(
                r#"<!doctype html>
<html lang="en">
<head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Outgoing HTTP Log</title>
    <style>
        :root {
            --bg: #0f172a;
            --panel: #111827;
            --text: #e5e7eb;
            --muted: #9ca3af;
            --ok: #34d399;
            --blocked: #f87171;
            --border: #374151;
        }
        body {
            margin: 0;
            font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
            background: radial-gradient(circle at top, #1f2937, var(--bg));
            color: var(--text);
        }
        .wrap {
            max-width: 1000px;
            margin: 0 auto;
            padding: 20px;
        }
        .meta {
            color: var(--muted);
            margin-bottom: 12px;
        }
        details {
            background: var(--panel);
            border: 1px solid var(--border);
            border-radius: 8px;
            margin: 8px 0;
            padding: 8px;
        }
        summary {
            cursor: pointer;
            list-style: none;
            display: flex;
            gap: 12px;
            align-items: center;
            flex-wrap: wrap;
        }
        .badge {
            padding: 2px 6px;
            border-radius: 999px;
            border: 1px solid var(--border);
            font-size: 12px;
        }
        .badge.ok { color: var(--ok); }
        .badge.blocked { color: var(--blocked); }
        pre {
            margin: 8px 0 0;
            white-space: pre-wrap;
            word-break: break-word;
            color: var(--muted);
        }
    </style>
</head>
<body>
    <div class="wrap">
        <h1>Outgoing HTTP Log</h1>
        <div class="meta" id="meta">Loading...</div>
        <div id="entries"></div>
    </div>
    <script>
        const meta = document.getElementById('meta');
        const entriesEl = document.getElementById('entries');

        function entrySummary(entry) {
            const who = entry.sender && entry.receiver
                ? `${entry.sender} -> ${entry.receiver}`
                : entry.component;
            return `#${entry.index} ${entry.action} ${who}`;
        }

        function render(data) {
            const now = new Date().toLocaleTimeString();
            meta.textContent = `Entries: ${data.total} | Auto-updates every 1s | Last update: ${now}`;

            const html = data.entries
                .slice()
                .reverse()
                .map((entry) => {
                    const badgeClass = entry.blocked ? 'blocked' : 'ok';
                    const payload = JSON.stringify(entry, null, 2);
                    return `
                        <details>
                            <summary>
                                <span class="badge ${badgeClass}">${entry.action}</span>
                                <span>${entrySummary(entry)}</span>
                            </summary>
                            <pre>${payload}</pre>
                        </details>
                    `;
                })
                .join('');

            entriesEl.innerHTML = html;
        }

        async function refresh() {
            try {
                const res = await fetch('/api/outgoing-http-log', { cache: 'no-store' });
                if (!res.ok) throw new Error(`HTTP ${res.status}`);
                const data = await res.json();
                render(data);
            } catch (err) {
                meta.textContent = `Failed to load: ${err}`;
            }
        }

        refresh();
        setInterval(refresh, 1000);
    </script>
</body>
</html>"#
                        .to_string(),
        )
}

fn build_rocket(config: &WebConfig) -> Rocket<Build> {
    let figment: Figment = Config::figment()
        .merge(("address", config.address.clone()))
        .merge(("port", config.port));

    rocket::custom(figment).mount(
        "/api",
        routes![
            health,
            capabilities,
            bridge_ping,
            outgoing_http_log_route,
            outgoing_http_log_live_route
        ],
    )
}

pub fn start_embedded_server_if_enabled(rt_handle: &Handle) -> bool {
    let config = WebConfig::from_env();
    if !config.enabled {
        return false;
    }

    rt_handle.spawn(async move {
        if let Err(err) = build_rocket(&config).launch().await {
            eprintln!("Embedded Rocket server failed: {err}");
        }
    });

    true
}

#[cfg(test)]
mod tests {
    use super::{parse_bool_env, parse_outgoing_http_log_line};

    #[test]
    fn parse_bool_env_honors_default_when_unset() {
        let key = "ARPSCI_TEST_WEB_BOOL_MISSING";
        unsafe { std::env::remove_var(key) };
        assert!(!parse_bool_env(key, false));
        assert!(parse_bool_env(key, true));
    }

    #[test]
    fn parse_bool_env_supports_truthy_values() {
        let key = "ARPSCI_TEST_WEB_BOOL_TRUE";
        unsafe { std::env::set_var(key, "yes") };
        assert!(parse_bool_env(key, false));
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn parse_bool_env_supports_falsey_values() {
        let key = "ARPSCI_TEST_WEB_BOOL_FALSE";
        unsafe { std::env::set_var(key, "off") };
        assert!(!parse_bool_env(key, true));
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn parse_outgoing_http_conversation_line() {
        let entry = parse_outgoing_http_log_line(
            3,
            "POST http://localhost:3000/ | conversation | Agent A -> Agent B | hello".to_string(),
        );
        assert_eq!(entry.index, 3);
        assert_eq!(entry.action, "POST");
        assert_eq!(entry.endpoint, "http://localhost:3000/");
        assert_eq!(entry.component, "conversation");
        assert_eq!(entry.sender.as_deref(), Some("Agent A"));
        assert_eq!(entry.receiver.as_deref(), Some("Agent B"));
        assert_eq!(entry.message, "hello");
        assert!(!entry.blocked);
    }

    #[test]
    fn parse_outgoing_http_blocked_line() {
        let entry = parse_outgoing_http_log_line(
            1,
            "BLOCKED http://example.com | evaluator Agent Evaluator [topic] | denied"
                .to_string(),
        );
        assert_eq!(entry.action, "BLOCKED");
        assert!(entry.blocked);
        assert_eq!(entry.component, "evaluator Agent Evaluator [topic]");
        assert_eq!(entry.message, "denied");
    }
}
