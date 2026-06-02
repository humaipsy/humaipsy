use std::path::PathBuf;
use std::sync::atomic::Ordering;

use eframe::egui;
use egui_phosphor::regular;

use crate::agents::Arpsci;
use crate::agents::nodes_panel::{
    sync_evaluator_researcher_activity, AgentNodeKind, AgentRecord, NodePayload, PanelTab,
    WORKSPACE_MODULE_ENABLED,
};
use crate::ui::ArpsciUiState;

#[derive(Default)]
pub(super) struct BasicNodeViewer;

impl BasicNodeViewer {
    pub(super) fn show_body(&mut self, id: usize, ui: &mut egui::Ui, agents: &mut [AgentRecord]) {
        super::body::show_node_body(id, ui, agents);
    }
}

impl Arpsci {
    pub(crate) fn render_nodes_panel(&mut self, ui: &mut egui::Ui, ui_state: &mut ArpsciUiState) {
        if !WORKSPACE_MODULE_ENABLED && self.nodes_panel.active_tab == PanelTab::Agents {
            self.nodes_panel.active_tab = PanelTab::Overview;
        }

        let panel_border_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let nodes_panel = egui::Frame::default()
            .fill(ui.visuals().panel_fill)
            .stroke(egui::Stroke::new(1.0, panel_border_color))
            .corner_radius(4.0)
            .inner_margin(egui::Margin::same(6));

        let panel_height = ui.available_height().max(120.0);
        let mut viewer = BasicNodeViewer;

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), panel_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                nodes_panel.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(
                                self.nodes_panel.active_tab == PanelTab::Overview,
                                "Overview",
                            )
                            .clicked()
                        {
                            self.nodes_panel.active_tab = PanelTab::Overview;
                        }
                        if WORKSPACE_MODULE_ENABLED
                            && ui
                                .selectable_label(
                                    self.nodes_panel.active_tab == PanelTab::Agents,
                                    "Workspace",
                                )
                                .clicked()
                        {
                            self.nodes_panel.active_tab = PanelTab::Agents;
                        }
                        if ui
                            .selectable_label(
                                self.nodes_panel.active_tab == PanelTab::Ollama,
                                "Ollama",
                            )
                            .clicked()
                        {
                            self.nodes_panel.active_tab = PanelTab::Ollama;
                        }
                        if ui
                            .selectable_label(
                                self.nodes_panel.active_tab == PanelTab::Settings,
                                "Settings",
                            )
                            .clicked()
                        {
                            self.nodes_panel.active_tab = PanelTab::Settings;
                        }
                        if self.air_gap_enabled {
                            ui.add_space(8.0);
                            let mut badge = "Air-gap mode: outbound HTTP disabled";
                            if !self.allow_local_ollama {
                                badge = "Air-gap mode: outbound HTTP + Ollama disabled";
                            }
                            ui.label(egui::RichText::new(badge).small().strong().color(
                                ui.visuals().warn_fg_color,
                            ))
                            .on_hover_text(
                                "Policy is enforced at runtime: non-loopback HTTP is blocked and logged.",
                            );
                        }
                    });
                    ui.separator();

                    if self.nodes_panel.active_tab == PanelTab::Overview {
                        crate::ui::overview_chat::render_overview_panel(self, ui);
                        return;
                    }
                    if self.nodes_panel.active_tab == PanelTab::Ollama {
                        let ctx = ui.ctx().clone();
                        self.render_ollama_settings_widgets(ui, &ctx, ui_state);
                        return;
                    }
                    if self.nodes_panel.active_tab == PanelTab::Settings {
                        self.render_reproducibility_settings_widgets(ui);
                        return;
                    }

                    if !WORKSPACE_MODULE_ENABLED {
                        self.render_reproducibility_settings_widgets(ui);
                        return;
                    }

                    ui.label(egui::RichText::new("Workspace").strong().size(16.0));
                    ui.separator();

                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!self.read_only_replay_mode, egui::Button::new("Add"))
                            .clicked()
                        {
                            self.nodes_panel.add_agent(self.nodes_panel.selected_add_kind);
                        }

                        egui::ComboBox::from_id_salt("add_agent_kind")
                            .selected_text(self.nodes_panel.selected_add_kind.label())
                            .show_ui(ui, |ui| {
                                for kind in [
                                    AgentNodeKind::Manager,
                                    AgentNodeKind::Worker,
                                    AgentNodeKind::Evaluator,
                                    AgentNodeKind::Researcher,
                                ] {
                                    ui.selectable_value(
                                        &mut self.nodes_panel.selected_add_kind,
                                        kind,
                                        kind.label(),
                                    );
                                }
                            });

                        if ui.button("Load").clicked() {
                            let path = PathBuf::from(ui_state.agents_workspace_path.trim());
                            match self.load_agents_workspace_from_path(path) {
                                Ok(message) => ui_state.manifest_status_message = message,
                                Err(e) => {
                                    ui_state.manifest_status_message =
                                        format!("Load workspace failed: {e}");
                                }
                            }
                        }
                        if ui.button("Save").clicked() {
                            let path = PathBuf::from(ui_state.agents_workspace_path.trim());
                            match self.save_agents_workspace_to_path(path) {
                                Ok(message) => ui_state.manifest_status_message = message,
                                Err(e) => {
                                    ui_state.manifest_status_message =
                                        format!("Save workspace failed: {e}");
                                }
                            }
                        }

                        let (start_stop_label, start_stop_hover) =
                            if self
                                .conversation_graph_running
                                .load(Ordering::Acquire)
                            {
                                (
                                    "Stop",
                                    "Stop conversation loops and agent streaming to the chat endpoint.",
                                )
                            } else {
                                (
                                    "Start",
                                    "Save run manifest and start conversation loops for workers with a topic (paired in node order; optional HTTP streaming).",
                                )
                            };
                        if ui
                            .add_enabled(
                                !self.read_only_replay_mode,
                                egui::Button::new(start_stop_label),
                            )
                            .on_hover_text(start_stop_hover)
                            .clicked()
                        {
                            if self
                                .conversation_graph_running
                                .load(Ordering::Acquire)
                            {
                                self.stop_graph();
                            } else {
                                // Snapshot the currently selected chat room so agent turns
                                // are routed to the room that was open when Start was pressed.
                                self.chat_active_room_id =
                                    crate::ui::overview_chat::panel::selected_room_id();
                                ui_state.manifest_status_message = self.run_graph();
                            }
                        }
                    });
                    if !ui_state.manifest_status_message.trim().is_empty() {
                        ui.label(
                            egui::RichText::new(ui_state.manifest_status_message.clone()).small(),
                        );
                    }
                    ui.separator();

                    let mut node_ids: Vec<usize> =
                        self.nodes_panel.agents.iter().map(|a| a.id).collect();
                    node_ids.sort_unstable();

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let mut row_remove: Vec<usize> = Vec::new();
                        for node_id in node_ids {
                            let Some(rec) = self
                                .nodes_panel
                                .agents
                                .iter()
                                .find(|a| a.id == node_id)
                            else {
                                continue;
                            };
                            let node = &rec.data;
                            let global_id = match &node.payload {
                                NodePayload::Manager(m) => m.global_id.clone(),
                                NodePayload::Worker(w) => w.global_id.clone(),
                                NodePayload::Evaluator(e) => e.global_id.clone(),
                                NodePayload::Researcher(r) => r.global_id.clone(),
                                NodePayload::Topic(t) => t.global_id.clone(),
                            };
                            let row_label = node.label.clone();

                            ui.set_width(ui.available_width());
                            let row_state_id = ui.make_persistent_id(("agent_row", node_id));
                            egui::Frame::default()
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    ui.visuals().widgets.noninteractive.bg_stroke.color,
                                ))
                                .corner_radius(4.0)
                                .inner_margin(egui::Margin::same(6))
                                .show(ui, |ui| {
                                    let header_close = egui::collapsing_header::CollapsingState::load_with_default_open(
                                        ui.ctx(),
                                        row_state_id,
                                        true,
                                    )
                                    .show_header(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 6.0;
                                            ui.label(&row_label);
                                            ui.label("•");
                                            ui.label(global_id);
                                            ui.label("•");
                                            if !self.read_only_replay_mode {
                                                let x_btn = egui::Button::new(
                                                    egui::RichText::new(regular::X)
                                                        .line_height(Some(ui.text_style_height(&egui::TextStyle::Body))),
                                                )
                                                .frame(false)
                                                .min_size(egui::Vec2::ZERO)
                                                .small();
                                                if ui
                                                    .add(x_btn)
                                                    .on_hover_text("Remove")
                                                    .clicked()
                                                {
                                                    row_remove.push(node_id);
                                                }
                                            }
                                        });
                                    });
                                    let _ = header_close.body(|ui| {
                                        ui.add_enabled_ui(!self.read_only_replay_mode, |ui| {
                                            viewer.show_body(node_id, ui, &mut self.nodes_panel.agents);
                                        });
                                    });
                                });
                            ui.add_space(4.0);
                        }
                        for id in row_remove {
                            self.nodes_panel.remove_agent(id);
                        }
                    });
                });

                let ctx = ui.ctx().clone();
                sync_evaluator_researcher_activity(&mut self.nodes_panel.agents);
                let mut pending_events = {
                    let mut q = self.conversation_message_events.lock().unwrap();
                    std::mem::take(&mut *q)
                };
                if !self
                    .conversation_graph_running
                    .load(Ordering::Acquire)
                {
                    // Backward-compatible fallback if queue is empty but latest still has a message.
                    if pending_events.is_empty()
                        && let Some(last_msg) = self.last_message_in_chat.lock().unwrap().clone()
                    {
                        pending_events.push(last_msg);
                    }
                    // Queue new events for active Evaluator nodes.
                    for rec in &self.nodes_panel.agents {
                        let id = rec.id;
                        if let NodePayload::Evaluator(e) = &rec.data.payload {
                            if !e.active {
                                continue;
                            }
                            for raw in &pending_events {
                                let (event_key, message) = if let Some((key, body)) =
                                    raw.split_once("::MSG::")
                                {
                                    (key.to_string(), body.to_string())
                                } else {
                                    ("LEGACY".to_string(), raw.clone())
                                };
                                let last_eval = self
                                    .last_evaluated_message_by_evaluator
                                    .lock()
                                    .unwrap()
                                    .get(&id)
                                    .cloned();
                                if event_key.is_empty() || last_eval.as_ref() == Some(&event_key) {
                                    continue;
                                }
                                self.last_evaluated_message_by_evaluator
                                    .lock()
                                    .unwrap()
                                    .insert(id, event_key.clone());
                                self.evaluator_event_queues
                                    .lock()
                                    .unwrap()
                                    .entry(id)
                                    .or_default()
                                    .push_back((event_key, message));
                            }
                        }
                    }

                    // Queue new events for active Researcher nodes.
                    for rec in &self.nodes_panel.agents {
                        let id = rec.id;
                        if let NodePayload::Researcher(r) = &rec.data.payload {
                            if !r.active {
                                continue;
                            }
                            for raw in &pending_events {
                                let (event_key, message) = if let Some((key, body)) =
                                    raw.split_once("::MSG::")
                                {
                                    (key.to_string(), body.to_string())
                                } else {
                                    ("LEGACY".to_string(), raw.clone())
                                };
                                let last_research = self
                                    .last_researched_message_by_researcher
                                    .lock()
                                    .unwrap()
                                    .get(&id)
                                    .cloned();
                                if event_key.is_empty()
                                    || last_research.as_ref() == Some(&event_key)
                                {
                                    continue;
                                }
                                self.last_researched_message_by_researcher
                                    .lock()
                                    .unwrap()
                                    .insert(id, event_key.clone());
                                self.researcher_event_queues
                                    .lock()
                                    .unwrap()
                                    .entry(id)
                                    .or_default()
                                    .push_back((event_key, message));
                            }
                        }
                    }

                    // Start at most one Evaluator inference per node (strict sequential order).
                    for rec in &self.nodes_panel.agents {
                        let id = rec.id;
                        let NodePayload::Evaluator(e) = &rec.data.payload else {
                            continue;
                        };
                        if !e.active {
                            continue;
                        }
                        if self.evaluator_inflight_nodes.lock().unwrap().contains(&id) {
                            continue;
                        }
                        let next_msg = self
                            .evaluator_event_queues
                            .lock()
                            .unwrap()
                            .entry(id)
                            .or_default()
                            .pop_front();
                        let Some((_, message)) = next_msg else {
                            continue;
                        };

                        self.evaluator_inflight_nodes.lock().unwrap().insert(id);
                        let inflight = self.evaluator_inflight_nodes.clone();
                        let node_key = id;
                        let instruction = e.instruction.clone();
                        let analysis_mode = e.analysis_mode.clone();
                        let limit_token = e.limit_token;
                        let num_predict = e.num_predict.clone();
                        let endpoint = self.http_endpoint.clone();
                        let ollama_host = self.ollama_host.clone();
                        let run_context = self.current_run_context.clone();
                        let ctx = ctx.clone();
                        let handle = self.rt_handle.clone();
                        let selected_model = if self.selected_ollama_model.trim().is_empty() {
                            None
                        } else {
                            Some(self.selected_ollama_model.clone())
                        };
                        let epoch_arc = self.ollama_run_epoch.clone();
                        let epoch_caught = self.ollama_run_epoch.load(Ordering::SeqCst);
                        let ledger = self.event_ledger.clone();
                        let app_state = self.app_state.clone();
                        let eval_global_id = e.global_id.clone();
                        handle.spawn(async move {
                            let ollama_in = format!("{instruction}\n{message}");
                            match crate::ollama::send_to_ollama(
                                ollama_host.as_str(),
                                &instruction,
                                &message,
                                limit_token,
                                &num_predict,
                                selected_model.as_deref(),
                                Some((epoch_arc, epoch_caught)),
                                app_state,
                                crate::metrics::InferenceTraceContext {
                                    source: "ui.sidecar.evaluator".to_string(),
                                    experiment_id: run_context
                                        .as_ref()
                                        .map(|r| r.experiment_id.clone()),
                                    run_id: run_context.as_ref().map(|r| r.run_id.clone()),
                                    node_global_id: Some(eval_global_id.clone()),
                                    turn_index: None,
                                },
                            )
                            .await
                            {
                                Ok(response) => {
                                    if let Some(ref l) = ledger {
                                        let _ = l.append_with_hashes(
                                            "sidecar.evaluator",
                                            Some(eval_global_id.clone()),
                                            selected_model.clone(),
                                            &ollama_in,
                                            &response,
                                            serde_json::json!({ "analysis_mode": analysis_mode }),
                                        );
                                    }
                                    let response_lower = response.to_lowercase();
                                    let sentiment = match analysis_mode.as_str() {
                                        "Topic Extraction" => "topic",
                                        "Decision Analysis" => "decision",
                                        "Sentiment Classification" => {
                                            if response_lower.contains("positive")
                                                || response_lower.contains("happy")
                                                || response_lower.contains("negative")
                                                || response_lower.contains("sad")
                                                || response_lower.contains("angry")
                                                || response_lower.contains("frustrated")
                                                || response_lower.contains("neutral")
                                            {
                                                "sentiment"
                                            } else {
                                                "unknown"
                                            }
                                        }
                                        _ => {
                                            if response_lower.contains("happy") {
                                                "happy"
                                            } else if response_lower.contains("sad") {
                                                "sad"
                                            } else {
                                                "analysis"
                                            }
                                        }
                                    };
                                    if let Err(e) = crate::web::send_evaluator_result(
                                        &endpoint,
                                        "Agent Evaluator",
                                        sentiment,
                                        &response,
                                        run_context.as_ref(),
                                        ledger.as_ref(),
                                    )
                                    .await
                                    {
                                        eprintln!("[Evaluator] Failed to send to ams-chat: {e}");
                                    }
                                }
                                Err(e) => {
                                    if e.to_string() != crate::ollama::OLLAMA_STOPPED_MSG
                                    {
                                        if let Some(ref l) = ledger {
                                            let _ = l.append_with_hashes(
                                                "sidecar.evaluator",
                                                Some(eval_global_id.clone()),
                                                selected_model.clone(),
                                                &ollama_in,
                                                "",
                                                serde_json::json!({
                                                    "analysis_mode": analysis_mode,
                                                    "stage": "ollama",
                                                    "error": e.to_string(),
                                                }),
                                            );
                                        }
                                        eprintln!("[Evaluator] Ollama error: {e}");
                                    }
                                }
                            }
                            inflight.lock().unwrap().remove(&node_key);
                            ctx.request_repaint();
                        });
                    }

                    // Start at most one Researcher inference per node (strict sequential order).
                    for rec in &self.nodes_panel.agents {
                        let id = rec.id;
                        let NodePayload::Researcher(r) = &rec.data.payload else {
                            continue;
                        };
                        if !r.active {
                            continue;
                        }
                        if self.researcher_inflight_nodes.lock().unwrap().contains(&id) {
                            continue;
                        }
                        let next_msg = self
                            .researcher_event_queues
                            .lock()
                            .unwrap()
                            .entry(id)
                            .or_default()
                            .pop_front();
                        let Some((_, message)) = next_msg else {
                            continue;
                        };

                        self.researcher_inflight_nodes.lock().unwrap().insert(id);
                        let inflight = self.researcher_inflight_nodes.clone();
                        let node_key = id;
                        let topic = if r.topic_mode.trim().is_empty() {
                            "Articles".to_string()
                        } else {
                            r.topic_mode.clone()
                        };
                        let instruction = format!(
                            "{}\n\nUsing the latest chat message, suggest 3 {} references related to what was said. Keep it concise with bullet points: title and one-line why it matches.",
                            r.instruction,
                            topic.to_lowercase()
                        );
                        let limit_token = r.limit_token;
                        let num_predict = r.num_predict.clone();
                        let endpoint = self.http_endpoint.clone();
                        let ollama_host = self.ollama_host.clone();
                        let run_context = self.current_run_context.clone();
                        let ctx = ctx.clone();
                        let handle = self.rt_handle.clone();
                        let selected_model = if self.selected_ollama_model.trim().is_empty() {
                            None
                        } else {
                            Some(self.selected_ollama_model.clone())
                        };
                        let epoch_arc = self.ollama_run_epoch.clone();
                        let epoch_caught = self.ollama_run_epoch.load(Ordering::SeqCst);
                        let ledger = self.event_ledger.clone();
                        let app_state = self.app_state.clone();
                        let res_global_id = r.global_id.clone();
                        handle.spawn(async move {
                            let ollama_in = format!("{instruction}\n{message}");
                            match crate::ollama::send_to_ollama(
                                ollama_host.as_str(),
                                &instruction,
                                &message,
                                limit_token,
                                &num_predict,
                                selected_model.as_deref(),
                                Some((epoch_arc, epoch_caught)),
                                app_state,
                                crate::metrics::InferenceTraceContext {
                                    source: "ui.sidecar.researcher".to_string(),
                                    experiment_id: run_context
                                        .as_ref()
                                        .map(|r| r.experiment_id.clone()),
                                    run_id: run_context.as_ref().map(|r| r.run_id.clone()),
                                    node_global_id: Some(res_global_id.clone()),
                                    turn_index: None,
                                },
                            )
                            .await
                            {
                                Ok(response) => {
                                    if let Some(ref l) = ledger {
                                        let _ = l.append_with_hashes(
                                            "sidecar.researcher",
                                            Some(res_global_id.clone()),
                                            selected_model.clone(),
                                            &ollama_in,
                                            &response,
                                            serde_json::json!({ "topic": topic }),
                                        );
                                    }
                                    if let Err(e) = crate::web::send_researcher_result(
                                        &endpoint,
                                        "Agent Researcher",
                                        &topic,
                                        &response,
                                        run_context.as_ref(),
                                        ledger.as_ref(),
                                    )
                                    .await
                                    {
                                        eprintln!("[Researcher] Failed to send to ams-chat: {e}");
                                    }
                                }
                                Err(e) => {
                                    if e.to_string()
                                        != crate::ollama::OLLAMA_STOPPED_MSG
                                    {
                                        if let Some(ref l) = ledger {
                                            let _ = l.append_with_hashes(
                                                "sidecar.researcher",
                                                Some(res_global_id.clone()),
                                                selected_model.clone(),
                                                &ollama_in,
                                                "",
                                                serde_json::json!({
                                                    "topic": topic,
                                                    "stage": "ollama",
                                                    "error": e.to_string(),
                                                }),
                                            );
                                        }
                                        eprintln!("[Researcher] Ollama error: {e}");
                                    }
                                }
                            }
                            inflight.lock().unwrap().remove(&node_key);
                            ctx.request_repaint();
                        });
                    }
                }
            },
        );
    }
}
