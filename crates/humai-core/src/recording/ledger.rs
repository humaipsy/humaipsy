//! Append-only JSONL event ledger per run (`events.jsonl`) for offline audit and reproducibility.
//!
//! Paired with `manifest.json` under `runs/<experiment_id>/<run_id>/`. See GitHub issue #2.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::recording::timing::now_rfc3339_utc;
use crate::sync_ext::MutexExt;

pub const EVENTS_FILE: &str = "events.jsonl";
pub const SUMMARY_FILE: &str = "summary.json";
pub const BUNDLE_VERSION: &str = "1";

/// Stable SHA-256 hex digest of UTF-8 text (empty string if `data` is empty).
pub fn sha256_hex(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn file_sha256_hex(path: &Path) -> Result<String> {
    let raw = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&raw);
    Ok(hex_lower(&hasher.finalize()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub experiment_id: String,
    pub run_id: String,
    pub event_id: u64,
    pub event_type: String,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_global_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub input_hash: String,
    pub output_hash: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunSummary {
    pub bundle_version: String,
    pub experiment_id: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events_sha256: Option<String>,
    pub event_counts: HashMap<String, u64>,
    pub total_events: u64,
    pub transport_http_ok: u64,
    pub transport_http_failed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_timestamp: Option<String>,
}

struct EventLedgerState {
    writer: BufWriter<File>,
    counts: HashMap<String, u64>,
    transport_ok: u64,
    transport_failed: u64,
    first_timestamp: Option<String>,
    last_timestamp: Option<String>,
}

/// Thread-safe append-only ledger for a single run directory.
pub struct EventLedger {
    run_dir: PathBuf,
    experiment_id: String,
    run_id: String,
    seq: AtomicU64,
    /// Ensures `system.run_stopped` + summary are written at most once per run.
    finalized: AtomicBool,
    state: Mutex<EventLedgerState>,
}

impl EventLedger {
    /// Opens or creates `events.jsonl` in `run_dir` and starts a fresh sequence (new process run).
    pub fn open(run_dir: PathBuf, experiment_id: String, run_id: String) -> Result<Self> {
        fs::create_dir_all(&run_dir)
            .with_context(|| format!("create run dir {}", run_dir.display()))?;
        let path = run_dir.join(EVENTS_FILE);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open ledger {}", path.display()))?;
        Ok(Self {
            run_dir,
            experiment_id,
            run_id,
            seq: AtomicU64::new(0),
            finalized: AtomicBool::new(false),
            state: Mutex::new(EventLedgerState {
                writer: BufWriter::new(file),
                counts: HashMap::new(),
                transport_ok: 0,
                transport_failed: 0,
                first_timestamp: None,
                last_timestamp: None,
            }),
        })
    }

    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    fn next_id(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn touch_timestamps(state: &mut EventLedgerState, ts: &str) {
        if state.first_timestamp.is_none() {
            state.first_timestamp = Some(ts.to_string());
        }
        state.last_timestamp = Some(ts.to_string());
    }

    /// Append a fully specified envelope (hashes precomputed).
    pub fn append_envelope(
        &self,
        event_type: &str,
        node_global_id: Option<String>,
        model: Option<String>,
        input_hash: String,
        output_hash: String,
        payload: serde_json::Value,
    ) -> Result<u64> {
        let event_id = self.next_id();
        let timestamp = now_rfc3339_utc();
        let envelope = EventEnvelope {
            experiment_id: self.experiment_id.clone(),
            run_id: self.run_id.clone(),
            event_id,
            event_type: event_type.to_string(),
            timestamp: timestamp.clone(),
            node_global_id,
            model,
            input_hash,
            output_hash,
            payload,
        };
        let line = serde_json::to_string(&envelope).context("serialize ledger event")?;
        let mut g = self.state.lock_recover();
        writeln!(g.writer, "{}", line).context("write ledger line")?;
        g.writer.flush().ok();
        *g.counts.entry(event_type.to_string()).or_insert(0) += 1;
        Self::touch_timestamps(&mut g, &timestamp);
        Ok(event_id)
    }

    /// Convenience: hash `input` and `output` strings for the envelope.
    pub fn append_with_hashes(
        &self,
        event_type: &str,
        node_global_id: Option<String>,
        model: Option<String>,
        input: &str,
        output: &str,
        payload: serde_json::Value,
    ) -> Result<u64> {
        self.append_envelope(
            event_type,
            node_global_id,
            model,
            sha256_hex(input),
            sha256_hex(output),
            payload,
        )
    }

    pub fn append_transport_http(
        &self,
        kind: &str,
        input_body: &str,
        response_summary: &str,
        http_status: Option<u16>,
        err: Option<&str>,
    ) -> Result<u64> {
        let ok = err.is_none() && http_status.map(|s| s < 400).unwrap_or(false);
        let mut g = self.state.lock_recover();
        if ok {
            g.transport_ok += 1;
        } else {
            g.transport_failed += 1;
        }
        drop(g);

        let payload = serde_json::json!({
            "kind": kind,
            "http_status": http_status,
            "outcome": if ok { "ok" } else { "failed" },
            "error": err,
        });
        self.append_envelope(
            "transport.http",
            None,
            None,
            sha256_hex(input_body),
            sha256_hex(response_summary),
            payload,
        )
    }

    pub fn append_system_run_started(&self, manifest_path: &Path) -> Result<u64> {
        let payload = serde_json::json!({
            "manifest_path": manifest_path.display().to_string(),
        });
        self.append_with_hashes("system.run_started", None, None, "", "", payload)
    }

    pub fn append_system_run_stopped(&self, reason: &str) -> Result<u64> {
        let payload = serde_json::json!({ "reason": reason });
        self.append_with_hashes("system.run_stopped", None, None, "", reason, payload)
    }

    /// Writes `system.run_stopped`, `summary.json`, and flushes — no-op if already finalized.
    pub fn try_finalize_run_stopped(&self, reason: &str) -> Result<()> {
        if self
            .finalized
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(());
        }
        self.append_system_run_stopped(reason)?;
        self.write_summary()?;
        self.flush()?;
        Ok(())
    }

    /// Writes `summary.json` next to `events.jsonl` and returns the written path.
    pub fn write_summary(&self) -> Result<PathBuf> {
        let manifest_file = self.run_dir.join("manifest.json");
        let events_file = self.run_dir.join(EVENTS_FILE);

        let manifest_sha256 = if manifest_file.is_file() {
            Some(file_sha256_hex(&manifest_file)?)
        } else {
            None
        };
        let events_sha256 = if events_file.is_file() {
            Some(file_sha256_hex(&events_file)?)
        } else {
            None
        };

        let g = self.state.lock_recover();
        let total_events: u64 = g.counts.values().sum();
        let summary = RunSummary {
            bundle_version: BUNDLE_VERSION.to_string(),
            experiment_id: self.experiment_id.clone(),
            run_id: self.run_id.clone(),
            manifest_sha256,
            events_sha256,
            event_counts: g.counts.clone(),
            total_events,
            transport_http_ok: g.transport_ok,
            transport_http_failed: g.transport_failed,
            first_timestamp: g.first_timestamp.clone(),
            last_timestamp: g.last_timestamp.clone(),
        };
        drop(g);

        let path = self.run_dir.join(SUMMARY_FILE);
        let json = serde_json::to_string_pretty(&summary).context("serialize summary")?;
        fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }

    pub fn flush(&self) -> Result<()> {
        let mut g = self.state.lock_recover();
        g.writer.flush().context("flush ledger")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_empty_string() {
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
