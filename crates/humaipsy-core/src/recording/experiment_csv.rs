use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::recording::ledger::EventLedger;
use crate::recording::timing::now_rfc3339_utc;
use crate::timer;

pub const DEFAULT_METRICS_OUTPUT_DIR: &str = "metrics";

const CSV_HEADER: &str = "participant_id,group,condition,block,block_cb,trial_label,trial_time_secs,trial_random,event,timestamp,trial_elapsed_secs,timeout,transcript_ai,transcript_human,final_answer,experiment_id,run_id,turn_index,model,duration_ms,ttft_us,input_chars,output_chars,prompt_tokens,output_tokens,total_tokens,source,success,error";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetricsConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
}

fn default_enabled() -> bool {
    true
}

fn default_output_dir() -> String {
    DEFAULT_METRICS_OUTPUT_DIR.to_string()
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            output_dir: default_output_dir(),
        }
    }
}

impl MetricsConfig {
    pub fn from_env() -> Self {
        let output_dir = crate::env_compat::var("METRICS_DIR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(default_output_dir);
        Self {
            enabled: true,
            output_dir,
        }
    }

    pub fn output_path(&self) -> PathBuf {
        PathBuf::from(self.output_dir.trim())
    }
}

#[derive(Clone, Debug)]
pub struct ExperimentDesignSnapshot {
    pub name: String,
    pub csv_name: String,
    pub participant_id: String,
    pub groups: u32,
    pub conditions: u32,
    pub trials: u32,
    pub trial_seconds: u32,
    pub topic: String,
}

#[derive(Clone, Debug)]
pub struct TrialMetadata {
    pub condition_index: usize,
    pub condition_label: String,
    pub trial_in_condition: usize,
    pub block: usize,
}

#[derive(Clone, Debug)]
pub struct SessionMetadata {
    pub experiment_id: String,
    pub run_id: String,
    pub trial_index: usize,
}

#[derive(Clone, Debug, Default)]
pub struct InferenceSnapshot {
    pub model: Option<String>,
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

#[derive(Default)]
struct RecorderState {
    config: MetricsConfig,
    design: Option<ExperimentDesignSnapshot>,
    session: Option<SessionMetadata>,
    trial: Option<TrialMetadata>,
    turn_index: usize,
    writer: Option<BufWriter<std::fs::File>>,
    active_csv_path: Option<PathBuf>,
    /// Reproducible-audit-trail sibling of the CSV: every row `emit()` writes
    /// to the CSV is also appended here as a JSONL event, keyed by
    /// (experiment_id, run_id) so a new experiment run gets its own ledger.
    ledger: Option<EventLedger>,
    ledger_key: Option<(String, String)>,
}

static RECORDER: OnceLock<Mutex<RecorderState>> = OnceLock::new();

fn recorder() -> &'static Mutex<RecorderState> {
    RECORDER.get_or_init(|| Mutex::new(RecorderState::default()))
}

pub fn update_config(config: MetricsConfig) {
    if let Ok(mut state) = recorder().lock() {
        state.config = config;
    }
}

pub fn configure_design(design: ExperimentDesignSnapshot) {
    if let Ok(mut state) = recorder().lock() {
        state.design = Some(design);
    }
}

/// Reduces `raw` to a single safe path component: strips path separators (`/`
/// and `\`, regardless of host OS, since a design could be authored on one
/// platform and exported on another), `:` (Windows drive-prefix character), and
/// control characters, and trims leading `.` so `..`/`.` can't be used to
/// escape `output_dir`. `design.name`/`csv_name` are free-text UI fields —
/// this is the only place their value becomes a filesystem path, so sanitize
/// here rather than trying to catch every UI entry point.
fn sanitize_file_stem(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == ':' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() {
        "experiment".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn csv_path_for_design(design: &ExperimentDesignSnapshot, output_dir: &Path) -> PathBuf {
    let raw = design.csv_name.trim();
    let raw = if raw.is_empty() { design.name.trim() } else { raw };
    let safe = sanitize_file_stem(raw);
    let file_name = if safe.ends_with(".csv") {
        safe
    } else {
        format!("{safe}.csv")
    };
    output_dir.join(file_name)
}

pub fn display_csv_path(design: &ExperimentDesignSnapshot, config: &MetricsConfig) -> String {
    csv_path_for_design(design, &config.output_path())
        .display()
        .to_string()
}

fn trial_elapsed_secs() -> u64 {
    timer::main_timer_elapsed_seconds()
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn csv_opt_u64(value: Option<u64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn csv_opt_u128(value: Option<u128>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn csv_opt_bool_success(success: bool) -> String {
    success.to_string()
}

struct EventRow {
    participant_id: String,
    group: String,
    condition: String,
    block: String,
    block_cb: String,
    trial_label: String,
    trial_time_secs: String,
    trial_random: String,
    event: String,
    timestamp: String,
    trial_elapsed_secs: String,
    timeout: String,
    transcript_ai: String,
    transcript_human: String,
    final_answer: String,
    experiment_id: String,
    run_id: String,
    turn_index: String,
    model: String,
    duration_ms: String,
    ttft_us: String,
    input_chars: String,
    output_chars: String,
    prompt_tokens: String,
    output_tokens: String,
    total_tokens: String,
    source: String,
    success: String,
    error: String,
}

impl EventRow {
    fn to_csv_line(&self) -> String {
        [
            &self.participant_id,
            &self.group,
            &self.condition,
            &self.block,
            &self.block_cb,
            &self.trial_label,
            &self.trial_time_secs,
            &self.trial_random,
            &self.event,
            &self.timestamp,
            &self.trial_elapsed_secs,
            &self.timeout,
            &self.transcript_ai,
            &self.transcript_human,
            &self.final_answer,
            &self.experiment_id,
            &self.run_id,
            &self.turn_index,
            &self.model,
            &self.duration_ms,
            &self.ttft_us,
            &self.input_chars,
            &self.output_chars,
            &self.prompt_tokens,
            &self.output_tokens,
            &self.total_tokens,
            &self.source,
            &self.success,
            &self.error,
        ]
        .iter()
        .map(|c| csv_cell(c))
        .collect::<Vec<_>>()
        .join(",")
    }
}

fn ensure_writer(state: &mut RecorderState, csv_path: &Path) -> Result<()> {
    if state.active_csv_path.as_deref() == Some(csv_path) && state.writer.is_some() {
        return Ok(());
    }

    if let Some(parent) = csv_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("create metrics dir {}", parent.display()))?;
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(csv_path)
        .with_context(|| format!("open experiment csv {}", csv_path.display()))?;
    let needs_header = fs::metadata(csv_path)
        .map(|m| m.len() == 0)
        .unwrap_or(true);
    let mut writer = BufWriter::new(file);
    if needs_header {
        writeln!(writer, "{CSV_HEADER}").context("write experiment csv header")?;
        writer.flush().ok();
    }

    state.writer = Some(writer);
    state.active_csv_path = Some(csv_path.to_path_buf());
    Ok(())
}

fn write_row(state: &mut RecorderState, row: EventRow) {
    if !state.config.enabled {
        return;
    }
    let Some(design) = state.design.clone() else {
        return;
    };

    let csv_path = csv_path_for_design(&design, &state.config.output_path());
    if let Err(e) = ensure_writer(state, &csv_path) {
        eprintln!("[ExperimentCSV] failed to open sink: {e}");
        return;
    }

    let Some(writer) = state.writer.as_mut() else {
        return;
    };

    let line = row.to_csv_line();
    if let Err(e) = writeln!(writer, "{line}") {
        eprintln!("[ExperimentCSV] failed to write row: {e}");
        return;
    }
    if let Err(e) = writer.flush() {
        eprintln!("[ExperimentCSV] failed to flush row: {e}");
    }
}

fn base_metadata(state: &RecorderState) -> (ExperimentDesignSnapshot, Option<SessionMetadata>, Option<TrialMetadata>) {
    (
        state.design.clone().unwrap_or(ExperimentDesignSnapshot {
            name: String::new(),
            csv_name: String::new(),
            participant_id: String::new(),
            groups: 1,
            conditions: 2,
            trials: 2,
            trial_seconds: 120,
            topic: String::new(),
        }),
        state.session.clone(),
        state.trial.clone(),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_row(
    state: &RecorderState,
    event: &str,
    timeout: Option<bool>,
    transcript_ai: Option<&str>,
    transcript_human: Option<&str>,
    final_answer: Option<&str>,
    turn_index: Option<usize>,
    inference: Option<&InferenceSnapshot>,
) -> EventRow {
    let (design, session, trial) = base_metadata(state);
    let group = format!("Group {}", design.groups.max(1));
    let (condition, block) = if let Some(trial) = &trial {
        (trial.condition_label.clone(), (trial.block).to_string())
    } else {
        (String::new(), String::new())
    };

    EventRow {
        participant_id: design.participant_id.clone(),
        group,
        condition,
        block,
        block_cb: "yes".to_string(),
        trial_label: design.topic.clone(),
        trial_time_secs: design.trial_seconds.to_string(),
        trial_random: "no".to_string(),
        event: event.to_string(),
        timestamp: now_rfc3339_utc(),
        trial_elapsed_secs: trial_elapsed_secs().to_string(),
        timeout: timeout.map(|v| if v { "1" } else { "0" }.to_string())
            .unwrap_or_default(),
        transcript_ai: transcript_ai.unwrap_or("").to_string(),
        transcript_human: transcript_human.unwrap_or("").to_string(),
        final_answer: final_answer.unwrap_or("").to_string(),
        experiment_id: session
            .as_ref()
            .map(|s| s.experiment_id.clone())
            .unwrap_or_default(),
        run_id: session
            .as_ref()
            .map(|s| s.run_id.clone())
            .unwrap_or_default(),
        turn_index: turn_index.map(|v| v.to_string()).unwrap_or_default(),
        model: inference
            .and_then(|i| i.model.clone())
            .unwrap_or_default(),
        duration_ms: inference
            .map(|i| i.duration_ms.to_string())
            .unwrap_or_default(),
        ttft_us: csv_opt_u128(inference.and_then(|i| i.ttft_us)),
        input_chars: inference
            .map(|i| i.input_chars.to_string())
            .unwrap_or_default(),
        output_chars: inference
            .map(|i| i.output_chars.to_string())
            .unwrap_or_default(),
        prompt_tokens: csv_opt_u64(inference.and_then(|i| i.prompt_token_count)),
        output_tokens: csv_opt_u64(inference.and_then(|i| i.candidates_token_count)),
        total_tokens: csv_opt_u64(inference.and_then(|i| i.total_token_count)),
        source: inference.map(|i| i.source.clone()).unwrap_or_default(),
        success: inference
            .map(|i| csv_opt_bool_success(i.success))
            .unwrap_or_default(),
        error: inference
            .and_then(|i| i.error.clone())
            .unwrap_or_default(),
    }
}

/// Opens (or reuses) the event ledger for the current session's
/// (experiment_id, run_id) under `<output_dir>/runs/<experiment_id>/<run_id>/`.
/// No-op if there is no active session yet (e.g. the `ExperimentConfigured`
/// row emitted before a session exists).
fn ensure_ledger(state: &mut RecorderState) {
    let Some(session) = state.session.clone() else {
        return;
    };
    let key = (session.experiment_id.clone(), session.run_id.clone());
    if state.ledger_key.as_ref() == Some(&key) && state.ledger.is_some() {
        return;
    }

    let run_dir = state
        .config
        .output_path()
        .join("runs")
        .join(&session.experiment_id)
        .join(&session.run_id);
    match EventLedger::open(run_dir, session.experiment_id, session.run_id) {
        Ok(ledger) => {
            state.ledger = Some(ledger);
            state.ledger_key = Some(key);
        }
        Err(e) => {
            eprintln!("[ExperimentCSV] failed to open event ledger: {e}");
            state.ledger = None;
            state.ledger_key = None;
        }
    }
}

/// Mirrors one CSV row into the JSONL event ledger, using the CSV's own
/// `event` name as the ledger event type so the two stay trivially
/// cross-referenceable.
fn ledger_append(state: &mut RecorderState, row: &EventRow) {
    ensure_ledger(state);
    let Some(ledger) = state.ledger.as_ref() else {
        return;
    };

    let model = (!row.model.is_empty()).then(|| row.model.clone());
    let input = row.transcript_human.as_str();
    let output = if !row.transcript_ai.is_empty() {
        row.transcript_ai.as_str()
    } else {
        row.final_answer.as_str()
    };
    let payload = serde_json::json!({
        "participant_id": row.participant_id,
        "condition": row.condition,
        "block": row.block,
        "trial_elapsed_secs": row.trial_elapsed_secs,
        "timeout": row.timeout,
        "turn_index": row.turn_index,
        "duration_ms": row.duration_ms,
        "success": row.success,
        "error": row.error,
    });
    if let Err(e) = ledger.append_with_hashes(&row.event, None, model, input, output, payload) {
        eprintln!("[ExperimentCSV] failed to append ledger event: {e}");
    }
}

fn emit(state: &mut RecorderState, row: EventRow) {
    ledger_append(state, &row);
    write_row(state, row);
}

pub fn record_experiment_configured(design: ExperimentDesignSnapshot) {
    if let Ok(mut state) = recorder().lock() {
        state.design = Some(design.clone());
        let row = build_row(&state, "ExperimentConfigured", None, None, None, None, None, None);
        emit(&mut state, row);
    }
}

pub fn begin_session(
    design: ExperimentDesignSnapshot,
    session: SessionMetadata,
    trial: TrialMetadata,
) {
    if let Ok(mut state) = recorder().lock() {
        state.design = Some(design);
        state.session = Some(session.clone());
        state.trial = Some(trial.clone());
        state.turn_index = 0;

        let start_row = build_row(&state, "ExperimentStart", None, None, None, None, None, None);
        emit(&mut state, start_row);

        state.turn_index = 1;
        let trial_row = build_row(&state, "TrialStart", None, None, None, None, None, None);
        emit(&mut state, trial_row);
    }
}

pub fn begin_trial(session: SessionMetadata, trial: TrialMetadata) {
    if let Ok(mut state) = recorder().lock() {
        state.session = Some(session);
        state.trial = Some(trial);
        state.turn_index = 1;
        let row = build_row(&state, "TrialStart", None, None, None, None, None, None);
        emit(&mut state, row);
    }
}

pub fn record_human_transcript(content: &str) {
    if content.trim().is_empty() {
        return;
    }
    if let Ok(mut state) = recorder().lock() {
        let turn = state.turn_index.max(1);
        let row = build_row(
            &state,
            "Transcript",
            None,
            None,
            Some(content),
            None,
            Some(turn),
            None,
        );
        emit(&mut state, row);
        state.turn_index = turn + 1;
    }
}

pub fn record_ai_transcript(content: &str, inference: InferenceSnapshot) {
    if let Ok(mut state) = recorder().lock() {
        let turn = state.turn_index.max(1);
        let row = build_row(
            &state,
            "Transcript",
            None,
            Some(content),
            None,
            None,
            Some(turn),
            Some(&inference),
        );
        emit(&mut state, row);
        state.turn_index = turn + 1;
    }
}

pub fn end_trial(timed_out: bool) {
    if let Ok(mut state) = recorder().lock() {
        let elapsed = trial_elapsed_secs();
        let end_row = build_row(
            &state,
            "TrialEnd",
            Some(timed_out),
            None,
            None,
            None,
            None,
            None,
        );
        emit(&mut state, end_row);

        let total_row = EventRow {
            trial_elapsed_secs: elapsed.to_string(),
            event: "TrialTotal".to_string(),
            timeout: if timed_out { "1" } else { "0" }.to_string(),
            ..build_row(&state, "TrialTotal", Some(timed_out), None, None, None, None, None)
        };
        emit(&mut state, total_row);
    }
}

pub fn end_experiment() {
    if let Ok(mut state) = recorder().lock() {
        if state.session.is_none() {
            return;
        }
        let row = build_row(&state, "ExperimentEnd", None, None, None, None, None, None);
        emit(&mut state, row);
        if let Some(ledger) = state.ledger.take()
            && let Err(e) = ledger.try_finalize_run_stopped("experiment_complete")
        {
            eprintln!("[ExperimentCSV] failed to finalize event ledger: {e}");
        }
        state.ledger_key = None;
        state.session = None;
        state.trial = None;
        state.turn_index = 0;
    }
}

pub fn record_final_answer(content: &str) {
    if content.trim().is_empty() {
        return;
    }
    if let Ok(mut state) = recorder().lock() {
        let row = build_row(
            &state,
            "FinalAnswer",
            None,
            None,
            None,
            Some(content),
            None,
            None,
        );
        emit(&mut state, row);
    }
}

pub fn current_experiment_csv_path() -> Option<PathBuf> {
    let state = recorder().lock().ok()?;
    if let Some(path) = &state.active_csv_path
        && path.is_file()
    {
        return Some(path.clone());
    }
    let design = state.design.as_ref()?;
    let path = csv_path_for_design(design, &state.config.output_path());
    path.is_file().then_some(path)
}

pub fn current_turn_index() -> usize {
    recorder()
        .lock()
        .map(|state| state.turn_index.max(1))
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_design() -> ExperimentDesignSnapshot {
        ExperimentDesignSnapshot {
            name: "Exp001".to_string(),
            csv_name: "Exp001.csv".to_string(),
            participant_id: "P01".to_string(),
            groups: 1,
            conditions: 2,
            trials: 2,
            trial_seconds: 120,
            topic: "Mental Health".to_string(),
        }
    }

    #[test]
    fn metrics_config_defaults() {
        let cfg = MetricsConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.output_dir, DEFAULT_METRICS_OUTPUT_DIR);
    }

    #[test]
    fn csv_path_uses_output_dir() {
        let design = sample_design();
        let path = csv_path_for_design(&design, Path::new("metrics"));
        assert_eq!(path, Path::new("metrics/Exp001.csv"));
    }

    #[test]
    fn csv_path_rejects_parent_dir_traversal() {
        let mut design = sample_design();
        design.csv_name = "../../../../etc/passwd".to_string();
        let path = csv_path_for_design(&design, Path::new("metrics"));
        assert_single_component_under(&path, Path::new("metrics"));
    }

    /// The invariant that actually matters: the sanitized filename is exactly
    /// one path component appended to `output_dir` — no `/`/`\` smuggled a
    /// directory segment through. A literal ".." *inside* a filename (with no
    /// separator around it) is harmless and not worth rejecting.
    fn assert_single_component_under(path: &Path, output_dir: &Path) {
        assert!(
            path.starts_with(output_dir),
            "sanitized path escaped output_dir: {}",
            path.display()
        );
        let extra_components = path.components().count() - output_dir.components().count();
        assert_eq!(
            extra_components, 1,
            "sanitized filename smuggled extra path components: {}",
            path.display()
        );
    }

    #[test]
    fn csv_path_rejects_unix_absolute_path() {
        let mut design = sample_design();
        design.csv_name = "/etc/passwd".to_string();
        let path = csv_path_for_design(&design, Path::new("metrics"));
        assert_single_component_under(&path, Path::new("metrics"));
    }

    #[test]
    fn csv_path_rejects_windows_absolute_path() {
        let mut design = sample_design();
        design.csv_name = r"C:\Windows\System32\config".to_string();
        let path = csv_path_for_design(&design, Path::new("metrics"));
        assert_single_component_under(&path, Path::new("metrics"));
        assert!(!path.to_string_lossy().contains(':'));
    }

    #[test]
    fn csv_path_falls_back_to_default_name_when_sanitized_to_empty() {
        let mut design = sample_design();
        design.csv_name = "...".to_string();
        design.name = String::new();
        let path = csv_path_for_design(&design, Path::new("metrics"));
        assert_eq!(path, Path::new("metrics/experiment.csv"));
    }

    #[test]
    fn recorder_writes_header_and_event_rows() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = MetricsConfig {
            enabled: true,
            output_dir: tmp.path().display().to_string(),
        };
        update_config(config);

        let design = sample_design();
        record_experiment_configured(design.clone());

        begin_session(
            design.clone(),
            SessionMetadata {
                experiment_id: "exp_test".to_string(),
                run_id: "run_test".to_string(),
                trial_index: 0,
            },
            TrialMetadata {
                condition_index: 0,
                condition_label: "Cooperative".to_string(),
                trial_in_condition: 0,
                block: 1,
            },
        );

        record_human_transcript("Hello AI");
        record_ai_transcript(
            "Hello human",
            InferenceSnapshot {
                model: Some("gemma3:1b".to_string()),
                duration_ms: 500,
                ttft_us: Some(20_000),
                input_chars: 10,
                output_chars: 12,
                prompt_token_count: Some(5),
                candidates_token_count: Some(7),
                total_token_count: Some(12),
                source: "test".to_string(),
                success: true,
                error: None,
            },
        );

        end_trial(false);
        end_experiment();

        let raw = fs::read_to_string(tmp.path().join("Exp001.csv")).expect("read csv");
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines[0], CSV_HEADER);
        assert!(lines.len() >= 6);
        assert!(raw.contains("ExperimentConfigured"));
        assert!(raw.contains("TrialStart"));
        assert!(raw.contains("Hello AI"));
        assert!(raw.contains("Hello human"));
        assert!(raw.contains(",500,"));
        assert!(!raw.contains("ttft_ms"));

        // Every CSV row emitted above should also have landed in the JSONL
        // event ledger under runs/<experiment_id>/<run_id>/, and
        // end_experiment() should have finalized it (system.run_stopped +
        // summary.json) — this is the audit-trail sibling to the CSV that
        // used to be dead code (recording::ledger had zero callers).
        let run_dir = tmp.path().join("runs").join("exp_test").join("run_test");
        let events_raw = fs::read_to_string(run_dir.join("events.jsonl")).expect("read ledger events");
        let event_lines: Vec<&str> = events_raw.lines().collect();
        assert!(
            event_lines.len() >= 6,
            "expected at least as many ledger events as CSV rows, got {}",
            event_lines.len()
        );
        assert!(events_raw.contains("\"event_type\":\"ExperimentStart\""));
        assert!(events_raw.contains("\"event_type\":\"TrialStart\""));
        assert!(events_raw.contains("\"event_type\":\"system.run_stopped\""));
        assert!(events_raw.contains("\"experiment_id\":\"exp_test\""));

        let summary_raw = fs::read_to_string(run_dir.join("summary.json")).expect("read ledger summary");
        assert!(summary_raw.contains("\"experiment_id\": \"exp_test\""));
        assert!(summary_raw.contains("\"run_id\": \"run_test\""));
    }

    #[test]
    fn csv_cell_escapes_commas() {
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
    }
}
