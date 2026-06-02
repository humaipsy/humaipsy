use crate::agents::conversation_sidecars::{
    ConversationSidecarConfig, ResearchExecutionPolicy, ResearchMessageGrounding,
    run_evaluator_sidecars_for_message, run_researchers_before_worker_turn,
};
use crate::agents::dialogue::{DialogueSessionState, PromptAssembler, PromptBuildInput};
use crate::app_state::AppState;
use crate::metrics::{InferenceTraceContext, TurnTimingEvent, TurnTracker};
use crate::run::manifest::now_rfc3339_utc;
use crate::run::event_ledger::EventLedger;
use crate::run::manifest::RunContext;
use crate::ollama::OllamaStopEpoch;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
struct ConversationPerfConfig {
    quiet_logs: bool,
    async_http: bool,
    compact_ledger: bool,
}

impl ConversationPerfConfig {
    fn load() -> Self {
        Self {
            // Keep timings stable and minimize overhead: fixed fast profile.
            quiet_logs: false,
            async_http: true,
            compact_ledger: true,
        }
    }
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

fn chat_stream_enabled() -> bool {
    parse_bool_env("ARPSCI_CHAT_STREAM_ENABLED", true)
}

fn conversation_http_stream_enabled() -> bool {
    parse_bool_env("ARPSCI_CONVERSATION_HTTP_STREAM_ENABLED", false)
}

#[derive(Clone)]
pub struct ConversationParticipant {
    pub id: usize,
    pub name: String,
    pub instruction: String,
    pub topic: String,
    pub topic_source: String,
    pub manager_name: String,
    pub global_id: String,
}

// ─── Conversation loop entry point ────────────────────────────────────────

/// `message_event_source_id` namespaces evaluator/researcher event keys (e.g. conversation loop id)
/// so parallel loops never share duplicate `TURN:n` prefixes.
pub async fn start_conversation_loop(
    message_event_source_id: usize,
    ollama_stop_epoch: Option<OllamaStopEpoch>,
    sidecars: Arc<ConversationSidecarConfig>,
    participants: Vec<ConversationParticipant>,
    ollama_host: String,
    endpoint: String,
    active_flag: Arc<Mutex<bool>>,
    last_message_in_chat: Arc<Mutex<Option<String>>>,
    message_events: Arc<Mutex<Vec<String>>>,
    selected_model: Option<String>,
    history_size: usize,
    run_context: Option<RunContext>,
    run_generation: u64,
    run_generation_counter: Arc<AtomicU64>,
    loops_remaining_in_run: Arc<AtomicUsize>,
    conversation_graph_running: Arc<AtomicBool>,
    ledger: Option<Arc<EventLedger>>,
    app_state: Arc<AppState>,
    chat_turn_tx: Option<std::sync::mpsc::Sender<crate::agents::AgentChatEvent>>,
    chat_room_id: Option<String>,
) {
    if participants.is_empty() {
        return;
    }

    let perf = ConversationPerfConfig::load();
    let enable_chat_stream = chat_stream_enabled();
    let enable_http_stream = conversation_http_stream_enabled();
    let mut turn = 0;
    let mut current_speaker_idx = 0usize;
    let participant_ids = participants
        .iter()
        .map(|p| p.id.to_string())
        .collect::<Vec<_>>()
        .join("-");
    let session_id = format!("group-{message_event_source_id}-{participant_ids}");
    let mut session = DialogueSessionState::new(session_id.clone(), history_size.max(1));
    let mut turn_tracker = TurnTracker::default();
    let (background_research_tx, mut background_research_rx) =
        tokio::sync::mpsc::unbounded_channel::<(usize, String)>();
    let mut background_research_cache: HashMap<usize, String> = HashMap::new();
    let mut manager_names: Vec<String> = participants
        .iter()
        .map(|p| p.manager_name.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    manager_names.sort();
    let conversation_manager_name = manager_names.join(" + ");
    let topics_summary = participants
        .iter()
        .map(|p| format!("{}: \"{}\"", p.name, p.topic))
        .collect::<Vec<_>>()
        .join(" | ");
    let topics_readable = participants
        .iter()
        .map(|p| format!("- {}: {}", p.name, p.topic))
        .collect::<Vec<_>>()
        .join("\n");
    let roster = participants
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(" -> ");

    let start_message = format!(
        "Conversation started\nSession: {}\nManager: {}\nOrder: {}\nTopics:\n{}",
        session_id, conversation_manager_name, roster, topics_readable
    );
    if !perf.quiet_logs {
        println!("\n{}", start_message);
    }

    if let Some(ref l) = ledger {
        let _ = l.append_with_hashes(
            "dialogue.start",
            None,
            selected_model.clone(),
            "",
            &start_message,
            serde_json::json!({ "topics_summary": topics_summary }),
        );
    }

    if enable_http_stream && perf.async_http {
        let endpoint = endpoint.clone();
        let conversation_manager_name = conversation_manager_name.clone();
        let topics_summary = topics_summary.clone();
        let start_message = start_message.clone();
        let run_context = run_context.clone();
        let ledger = ledger.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::web::send_conversation_message(
                &endpoint,
                0,
                &conversation_manager_name,
                0,
                "System",
                &topics_summary,
                &start_message,
                run_context.as_ref(),
                ledger.as_ref(),
            )
            .await
            {
                eprintln!("[HTTP] Failed to send conversation start message: {}", e);
            }
        });
    } else if enable_http_stream
        && let Err(e) = crate::web::send_conversation_message(
        &endpoint,
        0,
        &conversation_manager_name,
        0,
        "System",
        &topics_summary,
        &start_message,
        run_context.as_ref(),
        ledger.as_ref(),
    )
    .await
    {
        eprintln!("[HTTP] Failed to send conversation start message: {}", e);
    }

    if enable_chat_stream && let (Some(tx), Some(room_id)) = (&chat_turn_tx, &chat_room_id) {
        let _ = tx.send(crate::agents::AgentChatEvent {
            from: conversation_manager_name.clone(),
            content: format!("{}: {}", conversation_manager_name, start_message),
            room_id: room_id.clone(),
        });
    }

    loop {
        let active = {
            let flag = active_flag.lock().unwrap();
            *flag
        };

        if !active {
            if !perf.quiet_logs {
                println!("\n[Conversation stopped by user]");
            }
            break;
        }

        while let Ok((worker_id, refs)) = background_research_rx.try_recv() {
            if !refs.trim().is_empty() {
                background_research_cache.insert(worker_id, refs);
            }
        }

        let sender = &participants[current_speaker_idx];
        let receiver = &participants[(current_speaker_idx + 1) % participants.len()];
        let sender_id = sender.id;
        let sender_name = sender.name.clone();
        let sender_instruction = sender.instruction.clone();
        let sender_topic = sender.topic.clone();
        let sender_topic_source = sender.topic_source.clone();
        let receiver_id = receiver.id;
        let receiver_name = receiver.name.clone();
        let receiver_topic = receiver.topic.clone();
        let effective_topic = if sender_topic_source == "Follow Partner" {
            receiver_topic.clone()
        } else {
            sender_topic.clone()
        };
        let manager_name = sender.manager_name.clone();

        // Pre-turn: ground on the tied worker's last line when it exists; else partner line (first turn).
        let research_grounding = session
            .last_message_from_agent(sender_id)
            .map(|t| (t, ResearchMessageGrounding::TiedWorkerLastMessage))
            .or_else(|| {
                session
                    .last_message_from_agent(receiver_id)
                    .map(|p| (p, ResearchMessageGrounding::PartnerFallbackFirstTurn))
            });

        let research_injection = match sidecars.scheduling.research {
            ResearchExecutionPolicy::Off => String::new(),
            ResearchExecutionPolicy::Inline => {
                if let Some((line, grounding)) = research_grounding {
                    match run_researchers_before_worker_turn(
                        sidecars.as_ref(),
                        sender_id,
                        sender_name.as_str(),
                        line,
                        grounding,
                        ollama_host.as_str(),
                        endpoint.as_str(),
                        run_context.as_ref(),
                        selected_model.as_deref(),
                        ollama_stop_epoch.clone(),
                        false,
                        ledger.as_ref(),
                        app_state.clone(),
                    )
                    .await
                    {
                        Ok(s) => s,
                        Err(()) => break,
                    }
                } else {
                    String::new()
                }
            }
            ResearchExecutionPolicy::Background => {
                if let Some((line, grounding)) = research_grounding {
                    let bg_sidecars = sidecars.clone();
                    let bg_sender = sender_name.clone();
                    let bg_line = line.to_string();
                    let bg_host = ollama_host.clone();
                    let bg_endpoint = endpoint.clone();
                    let bg_context = run_context.clone();
                    let bg_selected_model = selected_model.clone();
                    let bg_ollama_epoch = ollama_stop_epoch.clone();
                    let bg_ledger = ledger.clone();
                    let bg_app_state = app_state.clone();
                    let tx = background_research_tx.clone();
                    tokio::spawn(async move {
                        if let Ok(research) = run_researchers_before_worker_turn(
                            bg_sidecars.as_ref(),
                            sender_id,
                            bg_sender.as_str(),
                            &bg_line,
                            grounding,
                            bg_host.as_str(),
                            bg_endpoint.as_str(),
                            bg_context.as_ref(),
                            bg_selected_model.as_deref(),
                            bg_ollama_epoch,
                            false,
                            bg_ledger.as_ref(),
                            bg_app_state,
                        )
                        .await
                        {
                            let _ = tx.send((sender_id, research));
                        }
                    });
                }
                background_research_cache.remove(&sender_id).unwrap_or_default()
            }
        };

        let memory_block = session.memory_block(&receiver_name, &effective_topic);
        let assembled_prompt = PromptAssembler::assemble(PromptBuildInput {
            base_instruction: &sender_instruction,
            manager_name: &manager_name,
            turn_index: turn,
            sender_name: &sender_name,
            receiver_name: &receiver_name,
            topic: &effective_topic,
            memory_block: &memory_block,
            sidecar_augmentation: &research_injection,
        });

        let gap_us = turn_tracker.current_gap_us();
        let gap_ms = turn_tracker.current_gap_ms();
        turn_tracker.mark_turn_started();

        app_state.metrics_sink().record_turn(TurnTimingEvent {
            event_type: "turn_timing".to_string(),
            timestamp: now_rfc3339_utc(),
            experiment_id: run_context.as_ref().map(|r| r.experiment_id.clone()),
            run_id: run_context.as_ref().map(|r| r.run_id.clone()),
            loop_key_node_id: message_event_source_id,
            turn_index: turn_tracker.current_turn_index(),
            speaker_id: sender_id,
            speaker_name: sender_name.clone(),
            receiver_id,
            receiver_name: receiver_name.clone(),
            gap_ms,
            gap_us,
        });

        let turn_message = assembled_prompt.turn_directive.clone();
        if !perf.quiet_logs {
            println!("{}", turn_message);
        }

        if enable_chat_stream && let (Some(tx), Some(room_id)) = (&chat_turn_tx, &chat_room_id) {
            let _ = tx.send(crate::agents::AgentChatEvent {
                from: manager_name.clone(),
                content: format!("{}: {}", manager_name, turn_message),
                room_id: room_id.clone(),
            });
        }

        if enable_http_stream && perf.async_http {
            let endpoint_clone = endpoint.clone();
            let topic_clone = effective_topic.clone();
            let turn_message_clone = turn_message.clone();
            let run_context_for_turn = run_context.clone();
            let ledger_turn = ledger.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::web::send_conversation_message(
                    &endpoint_clone,
                    0,
                    "Agent Manager",
                    0,
                    "System",
                    &topic_clone,
                    &turn_message_clone,
                    run_context_for_turn.as_ref(),
                    ledger_turn.as_ref(),
                )
                .await
                {
                    eprintln!("[HTTP] Failed to send turn message: {}", e);
                }
            });
        }

        let dialogue_input = if perf.compact_ledger {
            String::new()
        } else {
            format!(
                "{}\n---\n{}",
                assembled_prompt.system_instruction, assembled_prompt.user_prompt
            )
        };
        let sender_gid = sender.global_id.clone();
        match crate::ollama::send_to_ollama_with_result(
            ollama_host.as_str(),
            &assembled_prompt.system_instruction,
            &assembled_prompt.user_prompt,
            false,
            "",
            selected_model.as_deref(),
            ollama_stop_epoch.clone(),
            app_state.clone(),
            InferenceTraceContext {
                source: "dialogue.turn".to_string(),
                experiment_id: run_context.as_ref().map(|r| r.experiment_id.clone()),
                run_id: run_context.as_ref().map(|r| r.run_id.clone()),
                node_global_id: Some(sender_gid.clone()),
                turn_index: Some(turn as u32),
            },
        )
        .await
        {
            Ok(inference) => {
                let response = inference.response;
                if let Some(ref l) = ledger {
                    let _ = l.append_with_hashes(
                        "dialogue.turn",
                        Some(sender_gid),
                        selected_model.clone(),
                        &dialogue_input,
                        &response,
                        serde_json::json!({
                            "turn": turn,
                            "receiver_name": receiver_name,
                            "prompt_chars": assembled_prompt.system_instruction.len() + assembled_prompt.user_prompt.len(),
                        }),
                    );
                }
                session.record_turn(
                    sender_id,
                    sender_name.clone(),
                    response.clone(),
                    inference.usage.as_ref(),
                );
                let event = format!(
                    "SRC{}:TURN:{}::MSG::{}",
                    message_event_source_id, turn, response
                );
                *last_message_in_chat.lock().unwrap() = Some(event.clone());
                message_events.lock().unwrap().push(event);
                if !perf.quiet_logs {
                    println!("\n[{}]: {}", sender_name, response);
                    println!();
                }

                // Forward the completed turn to the overview chat room.
                if enable_chat_stream && let (Some(tx), Some(room_id)) = (&chat_turn_tx, &chat_room_id) {
                    let _ = tx.send(crate::agents::AgentChatEvent {
                        from: sender_name.clone(),
                        content: format!("{}: {}", sender_name, response),
                        room_id: room_id.clone(),
                    });
                }

                let message_for_chat = if research_injection.is_empty() {
                    response.clone()
                } else {
                    format!(
                        "{}\n\n---\nResearch (used for this turn)\n{}",
                        response, research_injection
                    )
                };

                if enable_http_stream && perf.async_http {
                    let endpoint = endpoint.clone();
                    let sender_name_http = sender_name.clone();
                    let receiver_name_http = receiver_name.clone();
                    let effective_topic_http = effective_topic.clone();
                    let message_for_chat_http = message_for_chat.clone();
                    let run_context_http = run_context.clone();
                    let ledger_http = ledger.clone();
                    tokio::spawn(async move {
                        if let Err(e) = crate::web::send_conversation_message(
                            &endpoint,
                            sender_id,
                            &sender_name_http,
                            receiver_id,
                            &receiver_name_http,
                            &effective_topic_http,
                            &message_for_chat_http,
                            run_context_http.as_ref(),
                            ledger_http.as_ref(),
                        )
                        .await
                        {
                            eprintln!("[HTTP] Failed to send message: {}", e);
                        }
                    });
                } else if enable_http_stream
                    && let Err(e) = crate::web::send_conversation_message(
                    &endpoint,
                    sender_id,
                    &sender_name,
                    receiver_id,
                    &receiver_name,
                    &effective_topic,
                    &message_for_chat,
                    run_context.as_ref(),
                    ledger.as_ref(),
                )
                .await
                {
                    eprintln!("[HTTP] Failed to send message: {}", e);
                }

                let evaluator_outputs = if sidecars.scheduling.should_run_evaluators(turn) {
                    match run_evaluator_sidecars_for_message(
                        sidecars.as_ref(),
                        &response,
                        ollama_host.as_str(),
                        &endpoint,
                        run_context.as_ref(),
                        selected_model.as_deref(),
                        ollama_stop_epoch.clone(),
                        true,
                        ledger.as_ref(),
                        app_state.clone(),
                    )
                    .await
                    {
                        Ok(outputs) => outputs,
                        Err(()) => break,
                    }
                } else {
                    Vec::new()
                };

                if enable_chat_stream && let (Some(tx), Some(room_id)) = (&chat_turn_tx, &chat_room_id) {
                    for ev_out in evaluator_outputs {
                        let _ = tx.send(crate::agents::AgentChatEvent {
                            from: "Agent Evaluator".to_string(),
                            content: format!("Agent Evaluator: {}", ev_out),
                            room_id: room_id.clone(),
                        });
                    }
                }

                turn_tracker.mark_turn_completed();

                current_speaker_idx = (current_speaker_idx + 1) % participants.len();
                turn += 1;
            }
            Err(e) => {
                if e.to_string() == crate::ollama::OLLAMA_STOPPED_MSG {
                    break;
                }
                if let Some(ref l) = ledger {
                    let _ = l.append_with_hashes(
                        "dialogue.ollama_error",
                        None,
                        selected_model.clone(),
                        &dialogue_input,
                        "",
                        serde_json::json!({ "error": e.to_string(), "turn": turn }),
                    );
                }
                eprintln!("[Error] Ollama error in conversation loop: {}", e);
                break;
            }
        }

        if turn > 50 {
            if !perf.quiet_logs {
                println!("\n[Conversation reached safety limit of 50 turns]");
            }
            break;
        }
    }

    let end_message = format!(
        "Conversation Ended: {}\nTotal turns: {}",
        participants
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(" -> "),
        turn
    );
    if !perf.quiet_logs {
        println!("\n{}", end_message);
    }

    if let Some(ref l) = ledger {
        let _ = l.append_with_hashes(
            "dialogue.end",
            None,
            selected_model.clone(),
            "",
            &end_message,
            serde_json::json!({ "total_turns": turn }),
        );
    }

    if enable_http_stream && perf.async_http {
        let endpoint = endpoint.clone();
        let conversation_manager_name = conversation_manager_name.clone();
        let topics_summary = topics_summary.clone();
        let end_message_http = end_message.clone();
        let run_context = run_context.clone();
        let ledger = ledger.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::web::send_conversation_message(
                &endpoint,
                0,
                &conversation_manager_name,
                0,
                "System",
                &topics_summary,
                &end_message_http,
                run_context.as_ref(),
                ledger.as_ref(),
            )
            .await
            {
                eprintln!("[HTTP] Failed to send conversation end message: {}", e);
            }
        });
    } else if enable_http_stream
        && let Err(e) = crate::web::send_conversation_message(
        &endpoint,
        0,
        &conversation_manager_name,
        0,
        "System",
        &topics_summary,
        &end_message,
        run_context.as_ref(),
        ledger.as_ref(),
    )
    .await
    {
        eprintln!("[HTTP] Failed to send conversation end message: {}", e);
    }

    if enable_chat_stream && let (Some(tx), Some(room_id)) = (&chat_turn_tx, &chat_room_id) {
        let _ = tx.send(crate::agents::AgentChatEvent {
            from: conversation_manager_name.clone(),
            content: format!("{}: {}", conversation_manager_name, end_message),
            room_id: room_id.clone(),
        });
    }

    let prev_remaining = loops_remaining_in_run.fetch_sub(1, Ordering::SeqCst);
    if prev_remaining == 1 && run_generation_counter.load(Ordering::SeqCst) == run_generation {
        conversation_graph_running.store(false, Ordering::Release);
        if let Some(ref l) = ledger {
            let _ = l.try_finalize_run_stopped("conversation_loops_finished");
        }
    }
}
