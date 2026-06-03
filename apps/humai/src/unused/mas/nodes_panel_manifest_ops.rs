use std::path::PathBuf;

use eframe::egui;

use crate::agents::Arpsci;
use crate::run::manifest::{
    APP_NAME, GraphSnapshot, MANIFEST_VERSION, ManifestNode, RunContext, RunManifest,
    RunRuntimeSettings, canonical_graph_signature, derive_experiment_id, export_manifest_to,
    new_run_id, now_rfc3339_utc, read_manifest, runs_root, write_manifest,
};

use super::model::{NodeData, NodePayload};
use super::state::AgentRecord;

fn json_opt_usize(config: &serde_json::Value, key: &str) -> Option<usize> {
    config.get(key).and_then(|v| {
        if v.is_null() {
            None
        } else {
            v.as_u64().map(|u| u as usize)
        }
    })
}

pub(crate) fn sync_evaluator_researcher_activity(agents: &mut [AgentRecord]) {
    for record in agents.iter_mut() {
        match &mut record.data.payload {
            NodePayload::Evaluator(evaluator) => {
                evaluator.active = evaluator.evaluate_all_workers || evaluator.worker_node.is_some();
            }
            NodePayload::Researcher(researcher) => {
                researcher.active = researcher.worker_node.is_some();
            }
            _ => {}
        }
    }
}

impl Arpsci {
    fn selected_model_option(&self) -> Option<String> {
        if self.selected_ollama_model.trim().is_empty() {
            None
        } else {
            Some(self.selected_ollama_model.clone())
        }
    }

    fn capture_runtime_settings(&self) -> RunRuntimeSettings {
        RunRuntimeSettings {
            selected_model: self.selected_model_option(),
            http_endpoint: self.http_endpoint.clone(),
            ollama_host: self.ollama_host.clone(),
            history_size: self.conversation_history_size,
            read_only_replay: self.read_only_replay_mode,
            air_gap_enabled: self.air_gap_enabled,
            allow_local_ollama: self.allow_local_ollama,
            metrics: self.metrics_config.clone(),
        }
    }

    fn capture_graph_snapshot(&self) -> GraphSnapshot {
        let mut nodes: Vec<ManifestNode> = self
            .nodes_panel
            .agents
            .iter()
            .map(|rec| {
                let node = &rec.data;
                let (kind, config) = match &node.payload {
                    NodePayload::Manager(m) => (
                        "manager".to_string(),
                        serde_json::json!({
                            "name": m.name,
                            "global_id": m.global_id,
                        }),
                    ),
                    NodePayload::Worker(w) => (
                        "worker".to_string(),
                        serde_json::json!({
                            "name": w.name,
                            "global_id": w.global_id,
                            "instruction_mode": w.instruction_mode,
                            "instruction": w.instruction,
                            "analysis_mode": w.analysis_mode,
                            "conversation_topic": w.conversation_topic,
                            "conversation_topic_source": w.conversation_topic_source,
                            "manager_node": w.manager_node,
                            "topic_node": w.topic_node,
                            "partner_worker": w.partner_worker,
                        }),
                    ),
                    NodePayload::Evaluator(e) => (
                        "evaluator".to_string(),
                        serde_json::json!({
                            "name": e.name,
                            "global_id": e.global_id,
                            "analysis_mode": e.analysis_mode,
                            "instruction": e.instruction,
                            "limit_token": e.limit_token,
                            "num_predict": e.num_predict,
                            "active": e.active,
                            "evaluate_all_workers": e.evaluate_all_workers,
                            "manager_node": e.manager_node,
                            "worker_node": e.worker_node,
                        }),
                    ),
                    NodePayload::Researcher(r) => (
                        "researcher".to_string(),
                        serde_json::json!({
                            "name": r.name,
                            "global_id": r.global_id,
                            "topic_mode": r.topic_mode,
                            "instruction": r.instruction,
                            "limit_token": r.limit_token,
                            "num_predict": r.num_predict,
                            "active": r.active,
                            "manager_node": r.manager_node,
                            "worker_node": r.worker_node,
                        }),
                    ),
                    NodePayload::Topic(t) => (
                        "topic".to_string(),
                        serde_json::json!({
                            "name": t.name,
                            "global_id": t.global_id,
                            "analysis_mode": t.analysis_mode,
                            "topic": t.topic,
                        }),
                    ),
                };
                ManifestNode {
                    node_id: rec.id,
                    kind,
                    label: node.label.clone(),
                    pos_x: rec.position.x,
                    pos_y: rec.position.y,
                    open: rec.open,
                    config,
                }
            })
            .collect();
        nodes.sort_by_key(|n| n.node_id);

        GraphSnapshot { nodes }
    }

    pub(crate) fn build_run_manifest(
        &self,
        experiment_id_override: Option<String>,
        read_only_replay: bool,
    ) -> anyhow::Result<RunManifest> {
        let mut runtime = self.capture_runtime_settings();
        runtime.read_only_replay = read_only_replay;
        let graph = self.capture_graph_snapshot();
        let graph_signature = canonical_graph_signature(&runtime, &graph)?;
        let experiment_id =
            experiment_id_override.unwrap_or_else(|| derive_experiment_id(&graph_signature));
        let run_id = new_run_id();

        Ok(RunManifest {
            manifest_version: MANIFEST_VERSION.to_string(),
            app_name: APP_NAME.to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: now_rfc3339_utc(),
            experiment_id,
            run_id,
            graph_signature,
            runtime,
            graph,
        })
    }

    pub(crate) fn persist_active_manifest(
        &mut self,
        manifest: RunManifest,
    ) -> anyhow::Result<PathBuf> {
        let path = write_manifest(&runs_root(), &manifest)?;
        self.current_run_context = Some(RunContext {
            manifest_version: manifest.manifest_version.clone(),
            experiment_id: manifest.experiment_id.clone(),
            run_id: manifest.run_id.clone(),
        });
        self.current_manifest = Some(manifest);
        Ok(path)
    }

    fn clear_graph(&mut self) {
        self.nodes_panel.agents.clear();
        self.nodes_panel.next_agent_id = 0;
    }

    /// Rebuilds graph and runtime fields from a manifest (stops graph, clears agents).
    fn apply_manifest_graph_and_runtime(&mut self, manifest: &RunManifest) -> anyhow::Result<()> {
        self.stop_graph();
        self.clear_graph();

        let mut nodes_sorted = manifest.graph.nodes.clone();
        nodes_sorted.sort_by_key(|n| n.node_id);

        for node in nodes_sorted {
            let kind = node.kind.as_str();
            let pos = egui::pos2(node.pos_x, node.pos_y);
            let mut node_data = match kind {
                "manager" => NodeData::new_manager(),
                "worker" => NodeData::new_worker(),
                "evaluator" => NodeData::new_evaluator(),
                "researcher" => NodeData::new_researcher(),
                "topic" => NodeData::new_topic(),
                _ => continue,
            };
            node_data.label = node.label.clone();
            match (&mut node_data.payload, kind) {
                (NodePayload::Manager(m), "manager") => {
                    m.name = node.config["name"].as_str().unwrap_or(&m.name).to_string();
                    m.global_id = node.config["global_id"]
                        .as_str()
                        .unwrap_or(&m.global_id)
                        .to_string();
                }
                (NodePayload::Worker(w), "worker") => {
                    w.name = node.config["name"].as_str().unwrap_or(&w.name).to_string();
                    w.global_id = node.config["global_id"]
                        .as_str()
                        .unwrap_or(&w.global_id)
                        .to_string();
                    w.instruction_mode = node.config["instruction_mode"]
                        .as_str()
                        .unwrap_or(&w.instruction_mode)
                        .to_string();
                    w.instruction = node.config["instruction"]
                        .as_str()
                        .unwrap_or(&w.instruction)
                        .to_string();
                    w.analysis_mode = node.config["analysis_mode"]
                        .as_str()
                        .unwrap_or(&w.analysis_mode)
                        .to_string();
                    w.conversation_topic = node.config["conversation_topic"]
                        .as_str()
                        .unwrap_or(&w.conversation_topic)
                        .to_string();
                    w.conversation_topic_source = node.config["conversation_topic_source"]
                        .as_str()
                        .unwrap_or(&w.conversation_topic_source)
                        .to_string();
                    w.manager_node = json_opt_usize(&node.config, "manager_node");
                    w.topic_node = json_opt_usize(&node.config, "topic_node");
                    w.partner_worker = json_opt_usize(&node.config, "partner_worker");
                }
                (NodePayload::Evaluator(e), "evaluator") => {
                    e.name = node.config["name"].as_str().unwrap_or(&e.name).to_string();
                    e.global_id = node.config["global_id"]
                        .as_str()
                        .unwrap_or(&e.global_id)
                        .to_string();
                    e.analysis_mode = node.config["analysis_mode"]
                        .as_str()
                        .unwrap_or(&e.analysis_mode)
                        .to_string();
                    e.instruction = node.config["instruction"]
                        .as_str()
                        .unwrap_or(&e.instruction)
                        .to_string();
                    e.limit_token = node.config["limit_token"]
                        .as_bool()
                        .unwrap_or(e.limit_token);
                    e.num_predict = node.config["num_predict"]
                        .as_str()
                        .unwrap_or(&e.num_predict)
                        .to_string();
                    e.active = node.config["active"].as_bool().unwrap_or(e.active);
                    e.evaluate_all_workers = node.config["evaluate_all_workers"]
                        .as_bool()
                        .unwrap_or(e.evaluate_all_workers);
                    e.manager_node = json_opt_usize(&node.config, "manager_node");
                    e.worker_node = json_opt_usize(&node.config, "worker_node");
                }
                (NodePayload::Researcher(r), "researcher") => {
                    r.name = node.config["name"].as_str().unwrap_or(&r.name).to_string();
                    r.global_id = node.config["global_id"]
                        .as_str()
                        .unwrap_or(&r.global_id)
                        .to_string();
                    r.topic_mode = node.config["topic_mode"]
                        .as_str()
                        .unwrap_or(&r.topic_mode)
                        .to_string();
                    r.instruction = node.config["instruction"]
                        .as_str()
                        .unwrap_or(&r.instruction)
                        .to_string();
                    r.limit_token = node.config["limit_token"]
                        .as_bool()
                        .unwrap_or(r.limit_token);
                    r.num_predict = node.config["num_predict"]
                        .as_str()
                        .unwrap_or(&r.num_predict)
                        .to_string();
                    r.active = node.config["active"].as_bool().unwrap_or(r.active);
                    r.manager_node = json_opt_usize(&node.config, "manager_node");
                    r.worker_node = json_opt_usize(&node.config, "worker_node");
                }
                (NodePayload::Topic(t), "topic") => {
                    t.name = node.config["name"].as_str().unwrap_or(&t.name).to_string();
                    t.global_id = node.config["global_id"]
                        .as_str()
                        .unwrap_or(&t.global_id)
                        .to_string();
                    t.analysis_mode = node.config["analysis_mode"]
                        .as_str()
                        .unwrap_or(&t.analysis_mode)
                        .to_string();
                    t.topic = node.config["topic"]
                        .as_str()
                        .unwrap_or(&t.topic)
                        .to_string();
                }
                _ => {}
            }

            self.nodes_panel
                .insert_agent_with_id(node.node_id, pos, node.open, node_data);
        }

        for r in self.nodes_panel.agents.iter_mut() {
            if let NodePayload::Evaluator(e) = &mut r.data.payload {
                if e.evaluate_all_workers {
                    e.worker_node = None;
                }
            }
        }
        sync_evaluator_researcher_activity(&mut self.nodes_panel.agents);

        self.selected_ollama_model = manifest.runtime.selected_model.clone().unwrap_or_default();
        self.http_endpoint = manifest.runtime.http_endpoint.clone();
        self.ollama_host = manifest.runtime.ollama_host.clone();
        self.conversation_history_size = manifest.runtime.history_size;
        self.air_gap_enabled = manifest.runtime.air_gap_enabled;
        self.allow_local_ollama = manifest.runtime.allow_local_ollama;
        self.sync_http_policy();
        self.refresh_metrics_sink();

        Ok(())
    }

    pub(crate) fn save_agents_workspace_to_path(
        &mut self,
        path: PathBuf,
    ) -> anyhow::Result<String> {
        let manifest = self.build_run_manifest(None, false)?;
        export_manifest_to(&manifest, &path)?;
        self.current_manifest = Some(manifest);
        Ok(format!("Saved workspace: {}", path.display()))
    }

    pub(crate) fn load_agents_workspace_from_path(
        &mut self,
        path: PathBuf,
    ) -> anyhow::Result<String> {
        let manifest = read_manifest(&path)?;
        self.apply_manifest_graph_and_runtime(&manifest)?;
        self.read_only_replay_mode = false;
        self.current_run_context = None;
        self.current_manifest = Some(manifest);
        Ok(format!("Loaded workspace: {}", path.display()))
    }
}
