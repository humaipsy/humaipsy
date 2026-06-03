use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::agents::Arpsci;
use crate::run::event_ledger::EventLedger;
use crate::run::manifest::runs_root;
use super::manifest_ops::sync_evaluator_researcher_activity;
use super::model::NodePayload;
use super::play_plan::{
    PlayConversationGroupJson, PlayWorkerInPlayJson, build_conversation_sidecar_from_agents,
    collect_run_play_plan_from_agents,
};

impl Arpsci {
    fn should_log_play_plan() -> bool {
        std::env::var("ARPSCI_LOG_PLAY_PLAN")
            .ok()
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    }

    fn conversation_group_size() -> usize {
        std::env::var("ARPSCI_CONVERSATION_GROUP_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2)
            .max(1)
    }

    pub(crate) fn stop_graph(&mut self) {
        self.ollama_run_epoch.fetch_add(1, Ordering::SeqCst);
        for (_, flag, _) in &self.conversation_loop_handles {
            *flag.lock().unwrap() = false;
        }
        self.conversation_loop_handles.clear();
        self.conversation_graph_running
            .store(false, Ordering::Release);
        self.chat_mode = crate::agents::GlobalChatMode::HumanToAgent;
        *self.last_message_in_chat.lock().unwrap() = None;
        self.conversation_message_events.lock().unwrap().clear();
        self.evaluator_event_queues.lock().unwrap().clear();
        self.researcher_event_queues.lock().unwrap().clear();
        self.last_evaluated_message_by_evaluator
            .lock()
            .unwrap()
            .clear();
        self.last_researched_message_by_researcher
            .lock()
            .unwrap()
            .clear();
    }

    /// Returns a UI-facing status message for the most recent run action.
    pub(crate) fn run_graph(&mut self) -> String {
        // Bulletproof behavior: re-run means stop existing graph processes first.
        self.stop_graph();
        // Mark as running immediately so UI can switch Start -> Stop while setup occurs.
        self.conversation_graph_running
            .store(true, Ordering::Release);
        self.chat_mode = crate::agents::GlobalChatMode::AgentToAgent;
        // Create a fresh agent→chat channel for this run.
        let (chat_tx, chat_rx) = std::sync::mpsc::channel::<crate::agents::AgentChatEvent>();
        self.chat_turn_tx = Some(chat_tx);
        self.chat_turn_rx = Some(chat_rx);
        self.last_evaluated_message_by_evaluator
            .lock()
            .unwrap()
            .clear();
        self.last_researched_message_by_researcher
            .lock()
            .unwrap()
            .clear();
        self.evaluator_event_queues.lock().unwrap().clear();
        self.researcher_event_queues.lock().unwrap().clear();
        self.evaluator_inflight_nodes.lock().unwrap().clear();
        self.researcher_inflight_nodes.lock().unwrap().clear();

        let experiment_id_override = if self.read_only_replay_mode {
            self.current_manifest
                .as_ref()
                .map(|m| m.experiment_id.clone())
        } else {
            None
        };
        let manifest =
            match self.build_run_manifest(experiment_id_override, self.read_only_replay_mode) {
                Ok(m) => m,
                Err(e) => {
                    self.conversation_graph_running
                        .store(false, Ordering::Release);
                    eprintln!("[Run Graph] Manifest build failed: {e}");
                    return format!("Manifest build failed: {e}");
                }
            };
        let manifest_path = match self.persist_active_manifest(manifest) {
            Ok(p) => p,
            Err(e) => {
                self.conversation_graph_running
                    .store(false, Ordering::Release);
                eprintln!("[Run Graph] Manifest save failed: {e}");
                return format!("Manifest save failed: {e}");
            }
        };
        let mut status_message = format!("Manifest saved: {}", manifest_path.display());

        if let Some(ctx) = self.current_run_context.as_ref() {
            let run_dir = manifest_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| runs_root().join(&ctx.experiment_id).join(&ctx.run_id));
            match EventLedger::open(run_dir, ctx.experiment_id.clone(), ctx.run_id.clone()) {
                Ok(ledger) => {
                    let arc = Arc::new(ledger);
                    if let Err(e) = arc.append_system_run_started(&manifest_path) {
                        status_message = format!("Ledger start failed: {e}");
                        eprintln!("[Run Graph] Ledger start failed: {e}");
                    } else {
                        self.event_ledger = Some(arc);
                    }
                }
                Err(e) => {
                    status_message = format!("Ledger open failed: {e}");
                    eprintln!("[Run Graph] Ledger open failed: {e}");
                }
            }
        }

        sync_evaluator_researcher_activity(&mut self.nodes_panel.agents);
        let sidecar_policy = crate::agents::conversation_sidecars::SidecarSchedulingPolicy::from_env();
        let sidecars = std::sync::Arc::new(build_conversation_sidecar_from_agents(
            &self.nodes_panel.agents,
            sidecar_policy,
        ));

        // Workers with a non-empty topic, stable order for pairing (row order by id).
        struct EligibleWorker {
            id: usize,
            name: String,
            instruction: String,
            topic: String,
            topic_source: String,
            manager_name: String,
            partner_worker: Option<usize>,
            global_id: String,
        }

        let mut eligible: Vec<EligibleWorker> = self
            .nodes_panel
            .agents
            .iter()
            .filter_map(|r| {
                if let NodePayload::Worker(w) = &r.data.payload {
                    if !w.conversation_topic.trim().is_empty() {
                        let manager_name = w
                            .manager_node
                            .and_then(|manager_id| {
                                self.nodes_panel
                                    .agents
                                    .iter()
                                    .find(|m| m.id == manager_id)
                                    .and_then(|m| {
                                        if let NodePayload::Manager(md) = &m.data.payload {
                                            Some(md.name.clone())
                                        } else {
                                            None
                                        }
                                    })
                            })
                            .unwrap_or_else(|| "Agent Manager".to_string());
                        return Some(EligibleWorker {
                            id: r.id,
                            name: w.name.clone(),
                            instruction: w.instruction.clone(),
                            topic: w.conversation_topic.clone(),
                            topic_source: w.conversation_topic_source.clone(),
                            manager_name,
                            partner_worker: w.partner_worker,
                            global_id: w.global_id.clone(),
                        });
                    }
                }
                None
            })
            .collect();
        eligible.sort_by_key(|w| w.id);
        let eligible_by_id: std::collections::HashMap<usize, usize> = eligible
            .iter()
            .enumerate()
            .map(|(idx, w)| (w.id, idx))
            .collect();

        if eligible.is_empty() {
            self.conversation_graph_running
                .store(false, Ordering::Release);
            if Self::should_log_play_plan() {
                let play_plan = collect_run_play_plan_from_agents(&self.nodes_panel.agents, vec![]);
                match serde_json::to_string_pretty(&play_plan) {
                    Ok(json) => println!("[Run Graph] play plan:\n{}", json),
                    Err(e) => eprintln!("[Run Graph] failed to serialize play plan: {e}"),
                }
            }
            if let Some(ref l) = self.event_ledger {
                let _ = l.try_finalize_run_stopped("no_eligible_conversation_workers");
            }
            return status_message;
        }

        let group_size = Self::conversation_group_size();
        let mut planned_groups: Vec<Vec<usize>> = Vec::new();
        if group_size == 2 {
            // Preserve explicit partner behavior when running classic pairs.
            let mut used_workers: std::collections::HashSet<usize> = std::collections::HashSet::new();
            for a in &eligible {
                if used_workers.contains(&a.id) {
                    continue;
                }
                if let Some(partner_id) = a.partner_worker
                    && partner_id != a.id
                    && !used_workers.contains(&partner_id)
                    && eligible_by_id.contains_key(&partner_id)
                {
                    used_workers.insert(a.id);
                    used_workers.insert(partner_id);
                    planned_groups.push(vec![a.id, partner_id]);
                }
            }
            let remaining: Vec<usize> = eligible
                .iter()
                .filter(|w| !used_workers.contains(&w.id))
                .map(|w| w.id)
                .collect();
            let mut i = 0;
            while i < remaining.len() {
                if i + 1 < remaining.len() {
                    planned_groups.push(vec![remaining[i], remaining[i + 1]]);
                    i += 2;
                } else {
                    planned_groups.push(vec![remaining[i]]);
                    i += 1;
                }
            }
        } else {
            // For 3+ participants, group workers by sorted row order.
            let ordered: Vec<usize> = eligible.iter().map(|w| w.id).collect();
            let mut i = 0;
            while i < ordered.len() {
                let end = (i + group_size).min(ordered.len());
                planned_groups.push(ordered[i..end].to_vec());
                i = end;
            }
        }

        self.conversation_run_generation
            .fetch_add(1, Ordering::SeqCst);
        let run_generation = self.conversation_run_generation.load(Ordering::SeqCst);
        let loops_remaining = Arc::new(AtomicUsize::new(planned_groups.len()));
        let gen_counter = self.conversation_run_generation.clone();
        let graph_running_flag = self.conversation_graph_running.clone();

        let mut conversations_plan = Vec::new();
        for group in planned_groups {
            let participants: Vec<crate::agents::agent_conversation_loop::ConversationParticipant> =
                group
                    .iter()
                    .filter_map(|worker_id| eligible_by_id.get(worker_id).map(|idx| &eligible[*idx]))
                    .map(|w| crate::agents::agent_conversation_loop::ConversationParticipant {
                        id: w.id,
                        name: w.name.clone(),
                        instruction: w.instruction.clone(),
                        topic: w.topic.clone(),
                        topic_source: w.topic_source.clone(),
                        manager_name: w.manager_name.clone(),
                        global_id: w.global_id.clone(),
                    })
                    .collect();

            if participants.is_empty() {
                continue;
            }

            let loop_key = participants[0].id;
            conversations_plan.push(PlayConversationGroupJson {
                loop_key_node_id: loop_key,
                workers: participants
                    .iter()
                    .map(|p| PlayWorkerInPlayJson {
                        node_id: p.id,
                        name: p.name.clone(),
                        global_id: p.global_id.clone(),
                        conversation_topic: p.topic.clone(),
                        conversation_topic_source: p.topic_source.clone(),
                    })
                    .collect(),
            });

            self.start_conversation_from_node_worker_resolved(
                sidecars.clone(),
                run_generation,
                gen_counter.clone(),
                loops_remaining.clone(),
                graph_running_flag.clone(),
                loop_key,
                participants,
            );
        }

        if Self::should_log_play_plan() {
            let play_plan =
                collect_run_play_plan_from_agents(&self.nodes_panel.agents, conversations_plan);
            match serde_json::to_string_pretty(&play_plan) {
                Ok(json) => println!("[Run Graph] play plan:\n{}", json),
                Err(e) => eprintln!("[Run Graph] failed to serialize play plan: {e}"),
            }
        }

        status_message
    }

    /// Keys the async loop by `loop_key_node_id` (first worker in each pair). Conversation output nodes were removed; pairing is automatic from eligible workers.
    fn start_conversation_from_node_worker_resolved(
        &mut self,
        sidecars: Arc<crate::agents::conversation_sidecars::ConversationSidecarConfig>,
        run_generation: u64,
        run_generation_counter: Arc<AtomicU64>,
        loops_remaining_in_run: Arc<AtomicUsize>,
        conversation_graph_running_flag: Arc<AtomicBool>,
        loop_key_node_id: usize,
        participants: Vec<crate::agents::agent_conversation_loop::ConversationParticipant>,
    ) {
        let active_flag = Arc::new(Mutex::new(true));
        let flag_clone = active_flag.clone();
        let endpoint = self.http_endpoint.clone();
        let ollama_host = self.ollama_host.clone();
        let last_msg = self.last_message_in_chat.clone();
        let message_events = self.conversation_message_events.clone();
        let selected_model = if self.selected_ollama_model.trim().is_empty() {
            None
        } else {
            Some(self.selected_ollama_model.clone())
        };
        let history_size = self.conversation_history_size;
        let handle = self.rt_handle.clone();
        let run_context = self.current_run_context.clone();
        let message_event_source_id = loop_key_node_id;
        let ollama_epoch = self.ollama_run_epoch.clone();
        let ollama_caught = self.ollama_run_epoch.load(Ordering::SeqCst);
        let ollama_stop_epoch = Some((ollama_epoch, ollama_caught));
        let ledger = self.event_ledger.clone();
        let app_state = self.app_state.clone();
        let chat_tx = self.chat_turn_tx.clone();
        let chat_room_id = self.chat_active_room_id.clone();

        let loop_handle = handle.spawn(async move {
            crate::agents::agent_conversation_loop::start_conversation_loop(
                message_event_source_id,
                ollama_stop_epoch,
                sidecars,
                participants,
                ollama_host,
                endpoint,
                flag_clone,
                last_msg,
                message_events,
                selected_model,
                history_size,
                run_context,
                run_generation,
                run_generation_counter,
                loops_remaining_in_run,
                conversation_graph_running_flag,
                ledger,
                app_state,
                chat_tx,
                chat_room_id,
            )
            .await;
        });

        self.conversation_loop_handles
            .push((loop_key_node_id, active_flag, loop_handle));
    }
}
