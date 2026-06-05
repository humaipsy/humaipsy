use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub use crate::recording::timing::InferenceTraceContext;

fn default_metrics_file() -> String {
    "metrics/timings.jsonl".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetricsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_metrics_file")]
    pub metrics_file: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            metrics_file: default_metrics_file(),
        }
    }
}

impl MetricsConfig {
    pub fn from_env() -> Self {
        let metrics_file = std::env::var("HUMAI_METRICS_FILE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(default_metrics_file);
        Self {
            // Metrics capture is globally enabled by default; users opt out from Settings.
            enabled: true,
            metrics_file,
        }
    }

    pub fn metrics_path(&self) -> PathBuf {
        PathBuf::from(self.metrics_file.trim())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InferenceTimingEvent {
    pub event_type: String,
    pub timestamp: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_global_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub t_start: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_first_token: Option<String>,
    pub t_end: String,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttft_us: Option<u128>,
    pub input_chars: usize,
    pub output_chars: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_token_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates_token_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_token_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnTimingEvent {
    pub event_type: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub loop_key_node_id: usize,
    pub turn_index: usize,
    pub speaker_id: usize,
    pub speaker_name: String,
    pub receiver_id: usize,
    pub receiver_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_us: Option<u128>,
}

pub trait MetricsSink: Send + Sync {
    fn record_inference(&self, event: InferenceTimingEvent);
    fn record_turn(&self, event: TurnTimingEvent);
}

#[derive(Default)]
pub struct NoopMetricsSink;

impl MetricsSink for NoopMetricsSink {
    fn record_inference(&self, _event: InferenceTimingEvent) {}
    fn record_turn(&self, _event: TurnTimingEvent) {}
}

pub struct FileMetricsSink {
    writer: Mutex<BufWriter<File>>,
}

impl FileMetricsSink {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create metrics dir {}", parent.display()))?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("open metrics sink at {}", path.display()))?;
        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
        })
    }

    fn emit_json_line<T: Serialize>(&self, event: &T) -> Result<()> {
        let line = serde_json::to_string(event).context("serialize metrics event")?;
        let mut guard = self.writer.lock().unwrap();
        writeln!(guard, "{line}").context("write metrics line")?;
        guard.flush().ok();
        Ok(())
    }
}

impl MetricsSink for FileMetricsSink {
    fn record_inference(&self, event: InferenceTimingEvent) {
        if let Err(e) = self.emit_json_line(&event) {
            eprintln!("[Metrics] failed to write inference event: {e}");
        }
    }

    fn record_turn(&self, event: TurnTimingEvent) {
        if let Err(e) = self.emit_json_line(&event) {
            eprintln!("[Metrics] failed to write turn event: {e}");
        }
    }
}

pub fn build_metrics_sink(config: &MetricsConfig) -> Arc<dyn MetricsSink> {
    if !config.enabled {
        return Arc::new(NoopMetricsSink);
    }
    let path = config.metrics_path();
    match FileMetricsSink::new(path.as_path()) {
        Ok(sink) => Arc::new(sink),
        Err(e) => {
            eprintln!("[Metrics] disabled because sink init failed: {e}");
            Arc::new(NoopMetricsSink)
        }
    }
}

pub struct TurnTracker {
    turn_index: usize,
    last_turn_start: Option<Instant>,
}

impl Default for TurnTracker {
    fn default() -> Self {
        Self::new(1)
    }
}

impl TurnTracker {
    pub fn new(first_turn_index: usize) -> Self {
        Self {
            turn_index: first_turn_index,
            last_turn_start: None,
        }
    }

    pub fn current_turn_index(&self) -> usize {
        self.turn_index
    }

    pub fn current_gap_us(&self) -> Option<u128> {
        self.last_turn_start.map(|t| t.elapsed().as_micros())
    }

    pub fn current_gap_ms(&self) -> Option<u128> {
        self.current_gap_us().map(|us| if us == 0 { 0 } else { us.div_ceil(1000) })
    }

    pub fn mark_turn_started(&mut self) {
        self.last_turn_start = Some(Instant::now());
    }

    pub fn mark_turn_completed(&mut self) {
        self.turn_index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FileMetricsSink, InferenceTimingEvent, MetricsConfig, MetricsSink, TurnTracker,
    };
    use crate::recording::timing::EVENT_TYPE_INFERENCE_TIMING;
    use std::fs;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn metrics_config_default_is_enabled() {
        let cfg = MetricsConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.metrics_file, "metrics/timings.jsonl");
    }

    #[test]
    fn file_sink_writes_jsonl() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("timings.jsonl");
        let sink = FileMetricsSink::new(path.as_path()).expect("sink");

        sink.record_inference(InferenceTimingEvent {
            event_type: EVENT_TYPE_INFERENCE_TIMING.to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            source: "test".to_string(),
            experiment_id: Some("exp_abc".to_string()),
            run_id: Some("run_123".to_string()),
            node_global_id: Some("node-1".to_string()),
            model: Some("glm-4.7-flash:latest".to_string()),
            success: true,
            error: None,
            t_start: "2026-01-01T00:00:00Z".to_string(),
            t_first_token: Some("2026-01-01T00:00:00Z".to_string()),
            t_end: "2026-01-01T00:00:01Z".to_string(),
            duration_ms: 1000,
            ttft_ms: Some(250),
            ttft_us: Some(250_000),
            input_chars: 12,
            output_chars: 24,
            prompt_token_count: Some(10),
            candidates_token_count: Some(20),
            total_token_count: Some(30),
            turn_index: None,
            prompt: None,
        });

        let raw = fs::read_to_string(path).expect("read metrics file");
        assert_eq!(raw.lines().count(), 1);
        let v: serde_json::Value = serde_json::from_str(raw.trim()).expect("valid json line");
        assert_eq!(v["event_type"], EVENT_TYPE_INFERENCE_TIMING);
        assert_eq!(v["source"], "test");
    }

    #[test]
    fn turn_tracker_defaults_to_turn_one_and_no_gap() {
        let tracker = TurnTracker::default();
        assert_eq!(tracker.current_turn_index(), 1);
        assert_eq!(tracker.current_gap_ms(), None);
    }

    #[test]
    fn turn_tracker_advances_and_reports_gap() {
        let mut tracker = TurnTracker::new(3);
        assert_eq!(tracker.current_turn_index(), 3);
        assert_eq!(tracker.current_gap_ms(), None);

        tracker.mark_turn_started();
        tracker.mark_turn_completed();
        assert_eq!(tracker.current_turn_index(), 4);

        thread::sleep(Duration::from_millis(1));
        let gap = tracker.current_gap_ms();
        assert!(gap.is_some());
    }
}