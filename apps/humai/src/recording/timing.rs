use std::time::{Duration, SystemTime, UNIX_EPOCH};

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const EVENT_TYPE_INFERENCE_TIMING: &str = "inference_timing";
pub const EVENT_TYPE_TURN_TIMING: &str = "turn_timing";

#[derive(Clone, Debug, Default)]
pub struct InferenceTraceContext {
    pub source: String,
    pub experiment_id: Option<String>,
    pub run_id: Option<String>,
    pub node_global_id: Option<String>,
    pub turn_index: Option<u32>,
}

pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn now_rfc3339_utc() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339()
}

pub fn resolve_from_optional_rfc3339(raw: Option<&str>) -> String {
    if let Some(s) = raw {
        let t = s.trim();
        if !t.is_empty() && OffsetDateTime::parse(t, &Rfc3339).is_ok() {
            return t.to_string();
        }
    }
    now_rfc3339()
}

pub fn duration_us(d: Duration) -> u128 {
    d.as_micros()
}

pub fn duration_ms_ceil(d: Duration) -> u128 {
    let us = d.as_micros();
    if us == 0 {
        0
    } else {
        us.div_ceil(1000)
    }
}
