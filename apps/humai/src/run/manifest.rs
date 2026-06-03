use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MANIFEST_VERSION: &str = "2.0.0";
pub const APP_NAME: &str = "humai";

static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunContext {
    pub manifest_version: String,
    pub experiment_id: String,
    pub run_id: String,
}

fn default_ollama_host() -> String {
    "http://127.0.0.1:11434".to_string()
}

fn default_air_gap_enabled() -> bool {
    false
}

fn default_allow_local_ollama() -> bool {
    true
}

fn default_metrics() -> crate::metrics::MetricsConfig {
    crate::metrics::MetricsConfig::default()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRuntimeSettings {
    pub selected_model: Option<String>,
    pub http_endpoint: String,
    #[serde(default = "default_ollama_host")]
    pub ollama_host: String,
    pub history_size: usize,
    pub read_only_replay: bool,
    #[serde(default = "default_air_gap_enabled")]
    pub air_gap_enabled: bool,
    #[serde(default = "default_allow_local_ollama")]
    pub allow_local_ollama: bool,
    #[serde(default = "default_metrics")]
    pub metrics: crate::metrics::MetricsConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestNode {
    pub node_id: usize,
    pub kind: String,
    pub label: String,
    pub pos_x: f32,
    pub pos_y: f32,
    pub open: bool,
    pub config: serde_json::Value,
}

/// Workspace snapshot: one record per agent row. Links (`manager_node`, `worker_node`, …) live in each node `config`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphSnapshot {
    pub nodes: Vec<ManifestNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunManifest {
    pub manifest_version: String,
    pub app_name: String,
    pub app_version: String,
    pub created_at: String,
    pub experiment_id: String,
    pub run_id: String,
    pub graph_signature: String,
    pub runtime: RunRuntimeSettings,
    pub graph: GraphSnapshot,
}

pub fn now_rfc3339_utc() -> String {
    crate::recording::timing::now_rfc3339_utc()
}

pub fn hash_hex(input: &str) -> String {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn derive_experiment_id(signature: &str) -> String {
    format!("exp_{}", &hash_hex(signature)[..12])
}

pub fn new_run_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let count = RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("run_{}_{:04}", millis, count)
}

pub fn canonical_graph_signature(
    runtime: &RunRuntimeSettings,
    graph: &GraphSnapshot,
) -> Result<String> {
    let canonical_payload = serde_json::json!({
        "manifest_version": MANIFEST_VERSION,
        "runtime": runtime,
        "graph": graph,
    });
    let canonical_json = serde_json::to_string(&canonical_payload)
        .context("failed to serialize canonical graph signature")?;
    Ok(hash_hex(&canonical_json))
}

pub fn runs_root() -> PathBuf {
    PathBuf::from("runs")
}

pub fn run_dir(base_dir: &Path, experiment_id: &str, run_id: &str) -> PathBuf {
    base_dir.join(experiment_id).join(run_id)
}

pub fn write_manifest(base_dir: &Path, manifest: &RunManifest) -> Result<PathBuf> {
    let dir = run_dir(base_dir, &manifest.experiment_id, &manifest.run_id);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create run dir: {}", dir.display()))?;
    let path = dir.join("manifest.json");
    let json =
        serde_json::to_string_pretty(manifest).context("failed to serialize run manifest")?;
    fs::write(&path, json)
        .with_context(|| format!("failed to write manifest: {}", path.display()))?;
    Ok(path)
}

pub fn export_manifest_to(manifest: &RunManifest, target_path: &Path) -> Result<()> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create export parent dir: {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(manifest)
        .context("failed to serialize manifest for export")?;
    fs::write(target_path, json)
        .with_context(|| format!("failed to export manifest to {}", target_path.display()))?;
    Ok(())
}

pub fn read_manifest(path: &Path) -> Result<RunManifest> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest from {}", path.display()))?;
    let manifest: RunManifest = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse manifest from {}", path.display()))?;
    if manifest.manifest_version != MANIFEST_VERSION {
        return Err(anyhow!(
            "unsupported manifest version '{}', expected '{}'",
            manifest.manifest_version,
            MANIFEST_VERSION
        ));
    }
    Ok(manifest)
}
