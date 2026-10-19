use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use egui_phosphor::regular;

use humaipsy_core::agent::timing_to_inference_snapshot;
use crate::app_model::{GlobalChatMode, HumaiPsy};
use humaipsy_core::metrics::{
    display_csv_path, ExperimentDesignSnapshot, MetricsConfig, SessionMetadata,
    TrialMetadata,
};
use humaipsy_core::recording::experiment_csv::{
    self, begin_session, begin_trial, end_experiment, end_trial, record_ai_transcript,
    record_experiment_configured, record_final_answer, record_human_transcript,
};
use humaipsy_core::recording::timing::InferenceTraceContext;
use humaipsy_core::sync_ext::MutexExt;

use humaipsy_core::chat_types::{ChatMessage, MessageSource, Room};
use humaipsy_core::store::Store;

use super::chat::ChatExample;

/// Chat SQLite connections, keyed by path and opened at most once per path —
/// mirrors `InferenceEngine::from_host`'s cache in `humaipsy_core::ollama::engine`.
/// `render_overview_panel` used to call `Store::open` fresh on every UI frame
/// (a new connection, `PRAGMA` setup, and schema check every ~16ms while the
/// Overview tab is visible); this cache is what fixes that. This is a genuine
/// cross-cutting cache keyed by path (not process-started-once global state),
/// so unlike the rest of this file's former statics it correctly stays a
/// `OnceLock` — see AGENTS.md's "Known state-management debt" section.
static STORE_CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Store>>>> = OnceLock::new();

fn shared_store(db_path: &Path) -> Result<Arc<Store>, humaipsy_core::store::StoreError> {
    let cache = STORE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock_recover();
    if let Some(store) = guard.get(db_path) {
        return Ok(store.clone());
    }
    let store = Arc::new(Store::open(db_path)?);
    guard.insert(db_path.to_path_buf(), store.clone());
    Ok(store)
}

/// The chat transcript database lives alongside the experiment CSV output,
/// under the same operator-configurable `MetricsConfig` output directory,
/// instead of a separate hardcoded `"metrics/chat.sqlite"` path.
fn chat_db_path(app: &HumaiPsy) -> PathBuf {
    app.metrics_config.output_path().join("chat.sqlite")
}

// The pure counterbalancing/trial-sequencing math lives in
// humaipsy_core::experiment (parameterized, unit-tested there). This file keeps
// thin wrappers with the same names/signatures the rest of panel.rs already
// calls, so those call sites don't need to change, while the wrappers now read
// the *actual* configured condition/trial counts from the design passed in
// instead of the old hardcoded 2x2 constants — this is what makes the Design
// tab's "Conditions"/"Trials" controls actually take effect.
use humaipsy_core::experiment::TrialDescriptor;

/// All Overview-tab experiment/chat orchestration state, owned by `HumaiPsyApp`
/// via `HumaiPsyUiState::overview_chat` and threaded through render calls
/// explicitly. This used to be a set of process-global `OnceLock<Mutex<T>>`
/// statics — see ARCHITECTURE.md's former "Known Limitations" entry. Every
/// field here is only ever touched on the UI thread (values are cloned out
/// *before* being handed to a spawned Tokio task, never read back through this
/// struct from inside one), so plain fields with no `Mutex` wrapping are
/// correct — unlike `OllamaUiState`'s fields, which are genuinely written from
/// spawned tasks and correctly stay `Arc<Mutex<T>>`.
#[derive(Default)]
pub(crate) struct OverviewChatUiState {
    pub(crate) chat: ChatExample,
    pub(crate) active_room_id: Option<String>,
    pub(crate) room_order: Vec<String>,
    pub(crate) exp_design: HumaiPsyExpDesign,
    pub(crate) session: ExperimentSessionState,
    pub(crate) exp_run_counter: u64,
    pub(crate) input_modal: InputModalState,
    pub(crate) finished_popup_status: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ExperimentSessionState {
    configured: bool,
    running: bool,
    experiment_id: Option<String>,
    run_id: Option<String>,
    trial_index: usize,
    trial_memory_start_idx: usize,
    trial_timeout_popup_open: bool,
    awaiting_final_answer: bool,
    finished_popup_open: bool,
    experiment_complete: bool,
}

fn total_experiment_trials(design: &HumaiPsyExpDesign) -> usize {
    humaipsy_core::experiment::total_experiment_trials(
        humaipsy_core::experiment::clamp_condition_count(design.conditions),
        humaipsy_core::experiment::clamp_trials_per_condition(design.trials),
    )
}

fn trial_descriptor(design: &HumaiPsyExpDesign, index: usize) -> TrialDescriptor {
    humaipsy_core::experiment::trial_descriptor(
        index,
        &design.participant_id,
        humaipsy_core::experiment::clamp_condition_count(design.conditions),
    )
}

fn current_trial_descriptor_from_state(
    design: &HumaiPsyExpDesign,
    state: &ExperimentSessionState,
) -> TrialDescriptor {
    let capped_index = state
        .trial_index
        .min(total_experiment_trials(design).saturating_sub(1));
    trial_descriptor(design, capped_index)
}

fn experiment_status_label(design: &HumaiPsyExpDesign, state: &ExperimentSessionState) -> String {
    if !state.configured {
        return "Experiment".to_string();
    }

    if state.awaiting_final_answer {
        return "Experiment, Final Question".to_string();
    }

    if state.experiment_complete {
        return "Experiment Complete".to_string();
    }

    let desc = current_trial_descriptor_from_state(design, state);
    format!(
        "Experiment, Cond {}, Trial {}",
        desc.condition_index + 1,
        desc.trial_in_condition + 1
    )
}

/// Refreshes `app.web_status`, the snapshot the local read-only web dashboard
/// (`humaipsy-web`) polls. Called at the trial-lifecycle points that already
/// mutate `state.session` (session start, each trial advance/timeout,
/// finalization) — not on every frame, since nothing here changes between
/// those points.
fn update_web_status(app: &HumaiPsy, state: &OverviewChatUiState) {
    let (condition_label, trial_in_condition, total_trials) = if state.session.configured
        && !state.session.awaiting_final_answer
        && !state.session.experiment_complete
    {
        let desc = current_trial_descriptor_from_state(&state.exp_design, &state.session);
        let condition = humaipsy_core::experiment::condition_name(desc.condition_index);
        (
            humaipsy_core::experiment::condition_label(condition).to_string(),
            desc.trial_in_condition + 1,
            total_experiment_trials(&state.exp_design),
        )
    } else {
        (String::new(), 0, 0)
    };

    let snapshot = humaipsy_web::WebStatusSnapshot {
        design_name: state.exp_design.name.clone(),
        participant_id: state.exp_design.participant_id.clone(),
        condition_label,
        trial_in_condition,
        total_trials,
        running: state.session.running,
        timer_display: humaipsy_core::timer::main_timer_display(),
        // humaipsy-web's get_status route overwrites this from the live vault
        // lock flag on every request — this value is never actually read.
        vault_unlocked: false,
    };

    *app.web_status.lock_recover() = snapshot;
}

fn condition_instruction_for_state(
    design: &HumaiPsyExpDesign,
    state: &ExperimentSessionState,
    coop_prompt: &str,
    argum_prompt: &str,
) -> String {
    let desc = current_trial_descriptor_from_state(design, state);
    let condition = humaipsy_core::experiment::condition_name(desc.condition_index);

    match condition {
        "ARGUM" => argum_prompt.to_string(),
        _ => coop_prompt.to_string(),
    }
}

fn design_snapshot(design: &HumaiPsyExpDesign) -> ExperimentDesignSnapshot {
    ExperimentDesignSnapshot {
        name: design.name.clone(),
        csv_name: design.csv_name.clone(),
        participant_id: design.participant_id.clone(),
        groups: design.groups,
        conditions: design.conditions,
        trials: design.trials,
        trial_seconds: design.trial_seconds,
        topic: design.topic.clone(),
    }
}

fn trial_metadata_from_descriptor(desc: TrialDescriptor) -> TrialMetadata {
    let condition = humaipsy_core::experiment::condition_name(desc.condition_index);
    TrialMetadata {
        condition_index: desc.condition_index,
        condition_label: humaipsy_core::experiment::condition_label(condition).to_string(),
        trial_in_condition: desc.trial_in_condition,
        block: desc.trial_in_condition + 1,
    }
}

fn session_metadata_from_state(state: &ExperimentSessionState) -> Option<SessionMetadata> {
    Some(SessionMetadata {
        experiment_id: state.experiment_id.clone()?,
        run_id: state.run_id.clone()?,
        trial_index: state.trial_index,
    })
}

fn inference_trace_context(session: &ExperimentSessionState, source: &str) -> InferenceTraceContext {
    InferenceTraceContext {
        source: source.to_string(),
        experiment_id: session.experiment_id.clone(),
        run_id: session.run_id.clone(),
        turn_index: Some(experiment_csv::current_turn_index() as u32),
        node_global_id: None,
    }
}

fn assistant_prompt_for_current_condition(
    app: &HumaiPsy,
    design: &HumaiPsyExpDesign,
    session: &ExperimentSessionState,
) -> String {
    format!(
        "{}\n\n{}",
        app.assistant_prompt,
        condition_instruction_for_state(
            design,
            session,
            &app.coop_agent_prompt,
            &app.argum_agent_prompt,
        )
    )
}

fn advance_to_next_trial(
    app: &HumaiPsy,
    store: &Store,
    state: &mut OverviewChatUiState,
    timed_out: bool,
) {
    let was_running = humaipsy_core::timer::main_timer_is_running();

    if !state.session.configured {
        append_message_to_chat_and_store(
            &mut state.chat,
            store,
            ChatMessage {
                content: "Submit experiment settings before advancing trials.".to_string(),
                from: Some("System".to_string()),
                correlation: None,
                source: MessageSource::System,
                api_auto_respond: false,
                assistant_generation: None,
            },
        );
        return;
    }

    if state.session.awaiting_final_answer || state.session.experiment_complete {
        return;
    }

    if state.session.running {
        end_trial(timed_out);
    }

    let next_index = state.session.trial_index + 1;
    if next_index >= total_experiment_trials(&state.exp_design) {
        state.session.running = false;
        state.session.trial_timeout_popup_open = false;
        humaipsy_core::timer::pause_main_timer();

        let final_question = state.exp_design.question.trim().to_string();

        if final_question.is_empty() {
            finalize_experiment(app, state, store);
        } else {
            state.session.awaiting_final_answer = true;
            append_message_to_chat_and_store(
                &mut state.chat,
                store,
                ChatMessage {
                    content: final_question,
                    from: Some("System".to_string()),
                    correlation: None,
                    source: MessageSource::System,
                    api_auto_respond: false,
                    assistant_generation: None,
                },
            );
        }

        update_web_status(app, state);
        return;
    }

    state.session.trial_index = next_index;
    state.session.trial_memory_start_idx = state.chat.messages.len();
    state.session.trial_timeout_popup_open = false;
    let desc = current_trial_descriptor_from_state(&state.exp_design, &state.session);
    let condition = humaipsy_core::experiment::condition_name(desc.condition_index);
    let session_meta = session_metadata_from_state(&state.session);
    update_web_status(app, state);

    if let Some(session_meta) = session_meta {
        begin_trial(session_meta, trial_metadata_from_descriptor(desc));
    }

    append_message_to_chat_and_store(
        &mut state.chat,
        store,
        ChatMessage {
            content: format!(
                "Moved to condition {} ({}), trial {}.",
                desc.condition_index + 1,
                condition,
                desc.trial_in_condition + 1
            ),
            from: Some("System".to_string()),
            correlation: None,
            source: MessageSource::System,
            api_auto_respond: false,
            assistant_generation: None,
        },
    );

    if was_running {
        humaipsy_core::timer::stop_and_reset_main_timer();
        humaipsy_core::timer::start_main_timer();
    }

    let topic = state.exp_design.topic.trim().to_string();
    let participant_id = state.exp_design.participant_id.trim().to_string();
    let model = app.selected_ollama_model.trim().to_string();

    if topic.is_empty() || participant_id.is_empty() || model.is_empty() {
        append_message_to_chat_and_store(
            &mut state.chat,
            store,
            ChatMessage {
                content: "Cannot start next trial question: missing topic, participant ID, or model."
                    .to_string(),
                from: Some("System".to_string()),
                correlation: None,
                source: MessageSource::System,
                api_auto_respond: false,
                assistant_generation: None,
            },
        );
        return;
    }

    spawn_ollama_opening_question(
        app,
        &state.chat,
        &state.exp_design,
        &state.session,
        topic,
        participant_id,
        model,
    );
}

fn handle_trial_timeout(app: &HumaiPsy, store: &Store, state: &mut OverviewChatUiState) {
    if !humaipsy_core::timer::main_timer_is_running() {
        return;
    }

    let trial_seconds = state.exp_design.trial_seconds.max(1) as u64;

    if humaipsy_core::timer::main_timer_elapsed_seconds() < trial_seconds {
        return;
    }

    if !state.session.configured || !state.session.running || state.session.awaiting_final_answer {
        return;
    }

    state.session.trial_timeout_popup_open = true;
    let desc = current_trial_descriptor_from_state(&state.exp_design, &state.session);

    let condition = humaipsy_core::experiment::condition_name(desc.condition_index);
    append_message_to_chat_and_store(
        &mut state.chat,
        store,
        ChatMessage {
            content: format!(
                "Trial ended: condition {} ({}), trial {} reached {} seconds. Advancing to next trial.",
                desc.condition_index + 1,
                condition,
                desc.trial_in_condition + 1,
                trial_seconds
            ),
            from: Some("System".to_string()),
            correlation: None,
            source: MessageSource::System,
            api_auto_respond: false,
            assistant_generation: None,
        },
    );

    advance_to_next_trial(app, store, state, true);
}

fn render_trial_timeout_popup(ctx: &egui::Context, state: &mut OverviewChatUiState) {
    let mut open = state.session.trial_timeout_popup_open;

    if !open {
        return;
    }

    let mut close_clicked = false;
    egui::Window::new("Trial Time Limit Reached")
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .fixed_size(egui::vec2(360.0, 140.0))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                ui.label("Current trial ended by time limit.");
                ui.label("Advancing to the next trial.");
                ui.add_space(12.0);
                if ui.button("OK").clicked() {
                    close_clicked = true;
                }
            });
        });

    if close_clicked {
        open = false;
    }

    state.session.trial_timeout_popup_open = open;
}

fn finalize_experiment(app: &HumaiPsy, state: &mut OverviewChatUiState, store: &Store) {
    if state.session.experiment_complete {
        return;
    }
    state.session.awaiting_final_answer = false;
    state.session.experiment_complete = true;
    state.session.finished_popup_open = true;

    end_experiment();
    update_web_status(app, state);
    append_message_to_chat_and_store(
        &mut state.chat,
        store,
        ChatMessage {
            content: "Experiment Finished".to_string(),
            from: Some("System".to_string()),
            correlation: None,
            source: MessageSource::System,
            api_auto_respond: false,
            assistant_generation: None,
        },
    );
}

fn submit_final_answer(app: &HumaiPsy, state: &mut OverviewChatUiState, store: &Store, answer: &str) {
    if !state.session.awaiting_final_answer || state.session.experiment_complete {
        return;
    }
    state.session.awaiting_final_answer = false;

    record_final_answer(answer);
    finalize_experiment(app, state, store);
}

fn copy_experiment_csv_to_desktop() -> Result<PathBuf, String> {
    let source = experiment_csv::current_experiment_csv_path()
        .ok_or_else(|| "No experiment CSV file found.".to_string())?;
    // Windows sets USERPROFILE (not HOME); fall back to it so the Desktop
    // copy works the same way it does on Linux/macOS.
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Could not determine home directory (HOME/USERPROFILE not set).".to_string())?;
    let desktop_dir = PathBuf::from(home).join("Desktop");
    std::fs::create_dir_all(&desktop_dir)
        .map_err(|err| format!("Failed to create Desktop folder: {err}"))?;
    let file_name = source
        .file_name()
        .ok_or_else(|| "Invalid experiment CSV path.".to_string())?;
    let destination = desktop_dir.join(file_name);
    std::fs::copy(&source, &destination)
        .map_err(|err| format!("Failed to copy CSV to Desktop: {err}"))?;
    Ok(destination)
}

fn render_experiment_finished_popup(ctx: &egui::Context, state: &mut OverviewChatUiState) {
    let mut open = state.session.finished_popup_open;

    if !open {
        return;
    }

    // Safety net: ensure timer is not running while experiment is finished.
    humaipsy_core::timer::pause_main_timer();

    let mut close_clicked = false;
    let mut download_clicked = false;

    egui::Window::new("Experiment Finished")
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .fixed_size(egui::vec2(380.0, 180.0))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                ui.label("All trials are complete.");
                ui.label("Save your experiment CSV to Desktop if needed.");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Save to Desktop").clicked() {
                        download_clicked = true;
                    }
                    if ui.button("Close").clicked() {
                        close_clicked = true;
                    }
                });
                if !state.finished_popup_status.is_empty() {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(state.finished_popup_status.as_str()).small().weak());
                }
            });
        });

    if download_clicked {
        state.finished_popup_status = match copy_experiment_csv_to_desktop() {
            Ok(path) => format!("Saved to {}", path.display()),
            Err(err) => err,
        };
    }

    if close_clicked {
        open = false;
        state.finished_popup_status.clear();
    }

    state.session.finished_popup_open = open;
}

fn experiment_hash_hex(input: &str) -> String {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn derive_experiment_id_local(signature: &str) -> String {
    format!("exp_{}", &experiment_hash_hex(signature)[..12])
}

fn new_run_id_local(counter: &mut u64) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let count = *counter;
    *counter += 1;
    format!("run_{}_{:04}", millis, count)
}

fn append_message_to_chat_and_store(
    chat: &mut ChatExample,
    store: &Store,
    message: ChatMessage,
) {
    let ts = ChatExample::display_time_for_message(&message);
    chat.messages.push(message.clone());
    chat.message_timestamps.push(ts.clone());
    if let Some(room_id) = &chat.selected_room {
        let _ = store.append_message(room_id, &message, &ts);
    }
}

fn agent_memory_from_chat(
    chat: &ChatExample,
    start_index: usize,
) -> Vec<humaipsy_core::agent::AgentMemoryMessage> {
    chat.messages
        .iter()
        .skip(start_index.min(chat.messages.len()))
        .filter_map(|msg| {
            let role = match msg.source {
                MessageSource::Human => humaipsy_core::agent::AgentRole::User,
                MessageSource::Api => humaipsy_core::agent::AgentRole::Assistant,
                _ => return None,
            };

            let content = msg.content.trim();
            if content.is_empty() {
                return None;
            }

            Some(humaipsy_core::agent::AgentMemoryMessage {
                role,
                content: content.to_string(),
            })
        })
        .collect()
}

fn spawn_ollama_opening_question(
    app: &HumaiPsy,
    chat: &ChatExample,
    design: &HumaiPsyExpDesign,
    session: &ExperimentSessionState,
    topic: String,
    participant_id: String,
    model: String,
) {
    let tx = chat.inbox.sender();
    let waiting = chat.waiting_for_response().clone();
    let selected_room = chat.selected_room.clone();
    let store_path = chat_db_path(app);
    let ollama_host = app.ollama_host.clone();
    let assistant_prompt = assistant_prompt_for_current_condition(app, design, session);
    let history_size = app.conversation_history_size;
    let trace_context =
        inference_trace_context(session, "overview_chat.humai_agent.opening_question");

    if let Ok(mut flag) = waiting.lock() {
        *flag = true;
    }

    app.rt_handle.spawn(async move {
        let provider = std::sync::Arc::new(humaipsy_core::provider::ollama_provider::OllamaProvider::new(
            ollama_host,
        ));
        let agent = humaipsy_core::agent::HumaiPsyAgent::new(provider, model, assistant_prompt)
            .with_history_size(history_size);

        let out = agent
            .generate_opening_question(&participant_id, &topic, trace_context)
            .await;

        let reply = match out {
            Ok(agent_reply) => {
                record_ai_transcript(
                    &agent_reply.text,
                    timing_to_inference_snapshot(&agent_reply.timing, agent_reply.model),
                );
                ChatMessage {
                    content: agent_reply.text.trim_end().to_string(),
                    from: Some("Assistant".to_string()),
                    correlation: None,
                    source: MessageSource::Api,
                    api_auto_respond: false,
                    assistant_generation: None,
                }
            }
            Err(e) => ChatMessage {
                content: format!("Ollama error while generating opening question: {e}"),
                from: Some("System".to_string()),
                correlation: None,
                source: MessageSource::System,
                api_auto_respond: false,
                assistant_generation: None,
            },
        };

        if let Some(room_id) = selected_room
            && let Ok(store) = shared_store(&store_path)
        {
            let ts = ChatExample::display_time_for_message(&reply);
            let _ = store.append_message(&room_id, &reply, &ts);
        }
        let _ = tx.send(reply);
        if let Ok(mut flag) = waiting.lock() {
            *flag = false;
        }
    });
}

fn start_experiment_from_play(app: &HumaiPsy, store: &Store, state: &mut OverviewChatUiState) {
    if state.session.running || state.session.awaiting_final_answer {
        return;
    }

    if state.exp_design.participant_id.trim().is_empty() {
        append_message_to_chat_and_store(
            &mut state.chat,
            store,
            ChatMessage {
                content: "Submit experiment settings and provide a Participant ID before starting."
                    .to_string(),
                from: Some("System".to_string()),
                correlation: None,
                source: MessageSource::System,
                api_auto_respond: false,
                assistant_generation: None,
            },
        );
        return;
    }

    if state.exp_design.topic.trim().is_empty() {
        append_message_to_chat_and_store(
            &mut state.chat,
            store,
            ChatMessage {
                content: "Please set a topic before starting the experiment.".to_string(),
                from: Some("System".to_string()),
                correlation: None,
                source: MessageSource::System,
                api_auto_respond: false,
                assistant_generation: None,
            },
        );
        return;
    }

    let model = app.selected_ollama_model.trim().to_string();
    if model.is_empty() {
        append_message_to_chat_and_store(
            &mut state.chat,
            store,
            ChatMessage {
                content: "Please select a global Ollama model in Settings before starting."
                    .to_string(),
                from: Some("System".to_string()),
                correlation: None,
                source: MessageSource::System,
                api_auto_respond: false,
                assistant_generation: None,
            },
        );
        return;
    }

    humaipsy_core::timer::start_main_timer();

    let topic = state.exp_design.topic.trim().to_string();
    let participant_id = state.exp_design.participant_id.trim().to_string();
    let experiment_id = derive_experiment_id_local(&format!(
        "{}|{}|{}",
        state.exp_design.name.trim(),
        participant_id,
        topic
    ));
    let run_id = new_run_id_local(&mut state.exp_run_counter);

    state.session.running = true;
    state.session.configured = true;
    state.session.experiment_id = Some(experiment_id.clone());
    state.session.run_id = Some(run_id.clone());
    state.session.trial_index = 0;
    state.session.trial_memory_start_idx = state.chat.messages.len();
    state.session.trial_timeout_popup_open = false;
    state.session.awaiting_final_answer = false;
    state.session.finished_popup_open = false;
    state.session.experiment_complete = false;

    let design_snapshot = design_snapshot(&state.exp_design);
    let trial_desc = current_trial_descriptor_from_state(&state.exp_design, &state.session);
    begin_session(
        design_snapshot,
        SessionMetadata {
            experiment_id,
            run_id,
            trial_index: 0,
        },
        trial_metadata_from_descriptor(trial_desc),
    );

    update_web_status(app, state);

    append_message_to_chat_and_store(
        &mut state.chat,
        store,
        ChatMessage {
            content: "Experiment Started".to_string(),
            from: Some("System".to_string()),
            correlation: None,
            source: MessageSource::System,
            api_auto_respond: false,
            assistant_generation: None,
        },
    );

    spawn_ollama_opening_question(
        app,
        &state.chat,
        &state.exp_design,
        &state.session,
        topic,
        participant_id,
        model,
    );
}

#[derive(Clone, Debug)]
pub(crate) struct HumaiPsyExpDesign {
    pub name: String,
    pub csv_name: String,
    pub participant_id: String,
    pub groups: u32,
    pub conditions: u32,
    pub trials: u32,
    pub trial_seconds: u32,
    pub topic: String,
    pub question: String,
}

impl Default for HumaiPsyExpDesign {
    fn default() -> Self {
        Self {
            name: "Exp001".to_string(),
            csv_name: "Exp001.csv".to_string(),
            participant_id: String::new(),
            groups: 1,
            conditions: 2,
            trials: 2,
            trial_seconds: 120,
            topic: "Mental Health".to_string(),
            question: "Are you happy?".to_string(),
        }
    }
}

impl HumaiPsyExpDesign {
    fn sync_csv_name_from_name(&mut self) {
        self.csv_name = format!("{}.csv", self.name.trim());
    }
}

#[derive(Clone, Debug)]
enum ModalAction {
    SetParticipantId,
}

#[derive(Clone, Debug)]
pub(crate) struct InputModalState {
    open: bool,
    title: String,
    label: String,
    hint: String,
    confirm_label: String,
    input: String,
    action: Option<ModalAction>,
}

impl Default for InputModalState {
    fn default() -> Self {
        Self {
            open: false,
            title: "Input".to_string(),
            label: "Value".to_string(),
            hint: "Type value".to_string(),
            confirm_label: "OK".to_string(),
            input: String::new(),
            action: None,
        }
    }
}

pub(crate) fn render_overview_panel(
    app: &mut HumaiPsy,
    ui: &mut egui::Ui,
    ui_state: &mut crate::ui::HumaiPsyUiState,
) {
    let db_path = chat_db_path(app);
    let store = match shared_store(&db_path) {
        Ok(store) => store,
        Err(err) => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Overview chat store error: {err}"),
            );
            return;
        }
    };

    let convs = store.list_conversations(50).unwrap_or_default();

    let mut rooms_by_id: HashMap<String, Room> = convs
        .iter()
        .map(|c| {
            let label = if c.name.trim().is_empty() {
                format!("Room {}", &c.id[..8])
            } else {
                c.name.clone()
            };
            (
                c.id.clone(),
                Room::new(c.id.clone(), label),
            )
        })
        .collect();

    let state = &mut ui_state.overview_chat;

    state.room_order.retain(|id| rooms_by_id.contains_key(id));
    for c in &convs {
        if !state.room_order.iter().any(|id| id == &c.id) {
            state.room_order.push(c.id.clone());
        }
    }

    let mut rooms: Vec<Room> = state
        .room_order
        .iter()
        .filter_map(|id| rooms_by_id.remove(id))
        .collect();
    if rooms.is_empty() {
        let id = match store.create_conversation() {
            Ok(id) => id,
            Err(err) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Failed to create conversation: {err}"),
                );
                return;
            }
        };
        rooms.push(Room::new(id.clone(), format!("Room {}", &id[..8])));
        state.room_order.push(id);
    }

    let graph_running = app.conversation_graph_running.load(Ordering::Acquire);
    let human_to_agent_mode = matches!(app.chat_mode, GlobalChatMode::Enabled);
    state.chat.room_mode_human_agent = human_to_agent_mode;
    state.chat.set_main_input_enabled(human_to_agent_mode);

    for item in state.chat.inbox.drain() {
        let ts = ChatExample::display_time_for_message(&item);
        state.chat.message_timestamps.push(ts);
        state.chat.messages.push(item);
    }

    handle_trial_timeout(app, &store, state);

    state.chat.set_rooms(rooms.clone());
    if state.chat.selected_room.is_none() {
        state.chat.selected_room = rooms.first().map(|room| room.id.clone());
    }

    if graph_running || humaipsy_core::timer::main_timer_is_running() {
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }

    ui.horizontal(|ui| {
        ui.label("Chat Input:");
        ui.add_enabled_ui(!graph_running, |ui| {
            ui.selectable_value(
                &mut app.chat_mode,
                GlobalChatMode::Enabled,
                "Enabled",
            );
            ui.selectable_value(
                &mut app.chat_mode,
                GlobalChatMode::Disabled,
                "Disabled",
            );
        });
        if graph_running {
            ui.label(
                egui::RichText::new("(locked while graph is running)")
                    .small()
                    .weak(),
            );
        }
    });
    ui.add_space(4.0);

    // persist_and_hydrate_selected_room only reads/writes chat.selected_room and
    // chat.messages (set up above, before this point) — it doesn't need to run
    // "between" the sidebar and chat panes, just once per frame before either
    // renders, so it's pulled out of the tile tree entirely rather than trying
    // to make it happen at a specific point in egui_tiles' render order.
    persist_and_hydrate_selected_room(&store, &rooms, state);

    let split_height = ui.available_height() - 20.0;
    egui::Frame::default()
        .stroke(egui::Stroke::new(
            1.0_f32,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_min_height(split_height);
            ui.set_height(split_height);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                let left_rect_width = 200.0;
                let row_height = ui.available_height();
                let right_rect_width =
                    (ui.available_width() - left_rect_width - ui.spacing().item_spacing.x)
                        .max(0.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(left_rect_width, row_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::Frame::default()
                            .stroke(egui::Stroke::new(
                                1.0_f32,
                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                            ))
                            .inner_margin(egui::Margin::same(0))
                            .show(ui, |ui| {
                                ui.set_min_height(row_height);
                                ui.set_height(row_height);
                                state
                                    .chat
                                    .sidebar_ui(ui, &store, &state.exp_design.participant_id);
                            });
                    },
                );

                ui.allocate_ui_with_layout(
                    egui::vec2(right_rect_width, row_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_height(row_height);
                        ui.set_height(row_height);
                        render_chat_columns(app, ui, &store, state, human_to_agent_mode);
                    },
                );
            });
        });
}

fn persist_and_hydrate_selected_room(
    store: &Store,
    rooms: &[Room],
    state: &mut OverviewChatUiState,
) {
    let previous_room_id = state.active_room_id.clone();
    let next_room_id = state.chat.selected_room.clone();
    if previous_room_id != next_room_id {
        if let Some(prev_id) = previous_room_id {
            let prev_still_exists = rooms.iter().any(|r| r.id == prev_id);
            if prev_still_exists {
                let _ = store.delete_messages_for_conversation(&prev_id);
                for (idx, msg) in state.chat.messages.iter().enumerate() {
                    let ts = state
                        .chat
                        .message_timestamps
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| ChatExample::display_time_for_message(msg));
                    let _ = store.append_message(&prev_id, msg, &ts);
                }
            }
        }

        if let Some(next_id) = &next_room_id {
            let (msgs, ts) = store.load_messages(next_id).unwrap_or((vec![], vec![]));
            state.chat.hydrate(msgs, ts);
        }

        state.active_room_id = next_room_id;
    }
}

fn render_chat_columns(
    app: &mut HumaiPsy,
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut OverviewChatUiState,
    human_to_agent_mode: bool,
) {
    let row_height = ui.available_height();
    render_chat_rectangle(app, ui, store, state, human_to_agent_mode, row_height);
}

pub(crate) fn render_design_panel(
    app: &mut HumaiPsy,
    ui: &mut egui::Ui,
    ui_state: &mut crate::ui::HumaiPsyUiState,
) {
    let human_to_agent_mode = matches!(app.chat_mode, GlobalChatMode::Enabled);
    let metrics_config = app.metrics_config.clone();
    render_right_bar(
        ui,
        app,
        &mut ui_state.overview_chat,
        human_to_agent_mode,
        &metrics_config,
    );
}

pub(crate) fn render_experiment_modals(ctx: &egui::Context, state: &mut OverviewChatUiState) {
    render_participant_id_modal(ctx, state);
    render_trial_timeout_popup(ctx, state);
    render_experiment_finished_popup(ctx, state);
}

fn render_chat_rectangle(
    app: &mut HumaiPsy,
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut OverviewChatUiState,
    human_to_agent_mode: bool,
    row_height: f32,
) {
    ui.set_min_height(row_height);
    ui.set_height(row_height);
    egui::Frame::default()
        .stroke(egui::Stroke::new(
            1.0_f32,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .inner_margin(egui::Margin::same(0))
        .show(ui, |ui| {
            ui.set_min_height(row_height);
            ui.set_height(row_height);
            ui.set_width((ui.available_width() - 40.0).max(0.0));
            let normal_widget_height = ui.spacing().interact_size.y;
            let input_row_height = (normal_widget_height - 2.0).max(20.0);
            let top_row_height = normal_widget_height;
            let top_gap = 6.0;
            let bottom_gap = 6.0;
            let bar_fill = ui.visuals().faint_bg_color;
            let bar_stroke = egui::Stroke::new(1.0_f32, ui.visuals().strong_text_color());

            egui::Frame::new()
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    egui::Frame::new()
                        .fill(bar_fill)
                        .stroke(bar_stroke)
                        .inner_margin(egui::Margin::symmetric(8, 0))
                        .show(ui, |ui| {
                            let status_label =
                                experiment_status_label(&state.exp_design, &state.session);

                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), top_row_height),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(egui::RichText::new(status_label).strong());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let trial_navigation_enabled = !state.session.awaiting_final_answer
                                            && !state.session.experiment_complete;
                                        let next_btn = egui::Button::new(">>").small();
                                        if ui
                                            .add_enabled(trial_navigation_enabled, next_btn)
                                            .on_hover_text("Move to next trial")
                                            .clicked()
                                        {
                                            advance_to_next_trial(app, store, state, false);
                                        }

                                        let stop_btn = egui::Button::new(
                                            egui::RichText::new(regular::X)
                                                .line_height(Some(ui.text_style_height(&egui::TextStyle::Body))),
                                        )
                                        .small();
                                        if ui
                                            .add_enabled(trial_navigation_enabled, stop_btn)
                                            .on_hover_text("Stop timer and reset to 00:00:00")
                                            .clicked()
                                        {
                                            humaipsy_core::timer::stop_and_reset_main_timer();
                                            end_trial(false);
                                            end_experiment();
                                            state.session.running = false;
                                            state.session.trial_index = 0;
                                            state.session.trial_memory_start_idx = 0;
                                            state.session.trial_timeout_popup_open = false;
                                            state.session.awaiting_final_answer = false;
                                            state.session.finished_popup_open = false;
                                            state.session.experiment_complete = false;
                                        }

                                        let pause_btn = egui::Button::new(
                                            egui::RichText::new(regular::PAUSE)
                                                .line_height(Some(ui.text_style_height(&egui::TextStyle::Body))),
                                        )
                                        .small();
                                        if ui
                                            .add_enabled(
                                                trial_navigation_enabled
                                                    && humaipsy_core::timer::main_timer_is_running(),
                                                pause_btn,
                                            )
                                            .on_hover_text("Pause timer")
                                            .clicked()
                                        {
                                            humaipsy_core::timer::pause_main_timer();
                                        }

                                        let play_btn = egui::Button::new(
                                            egui::RichText::new(regular::PLAY)
                                                .line_height(Some(ui.text_style_height(&egui::TextStyle::Body))),
                                        )
                                        .small();
                                        if ui
                                            .add_enabled(
                                                !state.session.awaiting_final_answer
                                                    && !humaipsy_core::timer::main_timer_is_running(),
                                                play_btn,
                                            )
                                            .on_hover_text("Start timer")
                                            .clicked()
                                        {
                                            start_experiment_from_play(app, store, state);
                                        }
                                    });
                                },
                            );
                        });

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(top_gap);

                    // Reserve space for the separator and input footer so the message list
                    // cannot push the panel past its bottom edge. Also reserve space for
                    // the fixed top action bar.
                    let footer_height = input_row_height + bottom_gap;
                    let top_bar_height = top_row_height + top_gap + 8.0;
                    let messages_height = (ui.available_height() - footer_height - top_bar_height).max(0.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), messages_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_height(messages_height);
                            egui::ScrollArea::vertical()
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.set_width(ui.available_width());
                                        let len = state.chat.messages.len();
                                        let time_col_width = 82.0;
                                        for (i, msg) in state.chat.messages.iter().enumerate() {
                                            ui.allocate_ui_with_layout(
                                                egui::vec2(ui.available_width(), 0.0),
                                                egui::Layout::left_to_right(egui::Align::Min),
                                                |ui| {
                                                    ui.allocate_ui_with_layout(
                                                        egui::vec2(time_col_width, 0.0),
                                                        egui::Layout::top_down(egui::Align::Min),
                                                        |ui| {
                                                            egui::Frame::new()
                                                                .fill(ui.visuals().widgets.inactive.bg_fill)
                                                                .stroke(egui::Stroke::new(
                                                                    0.5_f32,
                                                                    ui.visuals().widgets.noninteractive.bg_stroke.color,
                                                                ))
                                                                .corner_radius(egui::CornerRadius::same(2))
                                                                .inner_margin(egui::Margin::symmetric(6, 2))
                                                                .show(ui, |ui| {
                                                                    let ts = state
                                                                        .chat
                                                                        .message_timestamps
                                                                        .get(i)
                                                                        .cloned()
                                                                        .unwrap_or_else(|| "--:--:--".to_string());
                                                                    ui.label(ts);
                                                                });
                                                        },
                                                    );

                                                    let msg_width =
                                                        ui.available_width().max(0.0);
                                                    ui.allocate_ui_with_layout(
                                                        egui::vec2(msg_width, 0.0),
                                                        egui::Layout::right_to_left(egui::Align::Min),
                                                        |ui| {
                                                            egui::Frame::new()
                                                                .fill(ui.visuals().extreme_bg_color)
                                                                .stroke(egui::Stroke::new(
                                                                    0.5_f32,
                                                                    ui.visuals().widgets.noninteractive.bg_stroke.color,
                                                                ))
                                                                .corner_radius(egui::CornerRadius::same(2))
                                                                .inner_margin(egui::Margin::symmetric(8, 4))
                                                                .show(ui, |ui| {
                                                                    ui.set_max_width(ui.available_width());
                                                                    ui.with_layout(
                                                                        egui::Layout::left_to_right(egui::Align::Min),
                                                                        |ui| {
                                                                            ui.add(egui::Label::new(&msg.content).wrap());
                                                                        },
                                                                    );
                                                                });
                                                        },
                                                    );
                                                },
                                            );
                                            if i + 1 < len {
                                                ui.add_space(4.0);
                                            }
                                        }
                                        if *state.chat.waiting_for_response().lock_recover() {
                                            ui.horizontal(|ui| {
                                                ui.spinner();
                                                ui.label(
                                                    egui::RichText::new("Waiting for assistant response...")
                                                        .small()
                                                        .weak(),
                                                );
                                            });
                                        }
                                    });
                                });
                        },
                    );
                });

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            ui.add_enabled_ui(
                (human_to_agent_mode && !state.session.experiment_complete)
                    || state.session.awaiting_final_answer,
                |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let input_width = (ui.available_width() - 72.0).max(220.0);
                    let hint = if state.session.awaiting_final_answer {
                        "Answer the final question"
                    } else {
                        "Type a message"
                    };
                    let response = ui.add_sized(
                        egui::vec2(input_width, input_row_height),
                        egui::TextEdit::singleline(&mut state.chat.input_text).hint_text(hint),
                    );
                    let send_clicked = ui
                        .add_sized(
                            egui::vec2(52.0, input_row_height),
                            egui::Button::new("Send"),
                        )
                        .clicked()
                        || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));

                    if send_clicked && !state.chat.input_text.trim().is_empty() {
                        let message_text = state.chat.input_text.trim().to_string();
                        state.chat.input_text.clear();

                        let user_message = ChatMessage {
                            content: message_text.clone(),
                            from: Some("Human".to_string()),
                            correlation: None,
                            source: MessageSource::Human,
                            api_auto_respond: false,
                            assistant_generation: None,
                        };
                        let ts = ChatExample::current_timestamp_string();
                        state.chat.messages.push(user_message.clone());
                        state.chat.message_timestamps.push(ts.clone());

                        if let Some(room_id) = &state.chat.selected_room {
                            let _ = store.append_message(room_id, &user_message, &ts);
                        }

                        if state.session.awaiting_final_answer {
                            submit_final_answer(app, state, store, &message_text);
                        } else {
                        record_human_transcript(&message_text);

                        let trial_memory_start_index = state.session.trial_memory_start_idx;
                        let memory_snapshot =
                            agent_memory_from_chat(&state.chat, trial_memory_start_index);

                        let tx = state.chat.inbox.sender();
                        let waiting = state.chat.waiting_for_response().clone();
                        let selected_room = state.chat.selected_room.clone();
                        let store_path = chat_db_path(app);
                        let model = app.selected_ollama_model.trim().to_string();
                        let ollama_host = app.ollama_host.clone();
                        let assistant_prompt =
                            assistant_prompt_for_current_condition(app, &state.exp_design, &state.session);
                        let history_size = app.conversation_history_size;
                        let trace_context =
                            inference_trace_context(&state.session, "overview_chat.humai_agent.reply");
                        if model.is_empty() {
                            let _ = tx.send(ChatMessage {
                                content: "Please select a global Ollama model in Settings.".to_string(),
                                from: Some("System".to_string()),
                                correlation: None,
                                source: MessageSource::System,
                                api_auto_respond: false,
                                assistant_generation: None,
                            });
                        } else {
                            if let Ok(mut flag) = waiting.lock() {
                                *flag = true;
                            }
                            app.rt_handle.spawn(async move {
                                let provider = std::sync::Arc::new(
                                    humaipsy_core::provider::ollama_provider::OllamaProvider::new(
                                        ollama_host,
                                    ),
                                );
                                let agent = humaipsy_core::agent::HumaiPsyAgent::new(
                                    provider,
                                    model,
                                    assistant_prompt,
                                )
                                .with_history_size(history_size);
                                let out = agent
                                    .reply_from_memory(&memory_snapshot, trace_context)
                                    .await;
                                let reply = match out {
                                    Ok(agent_reply) => {
                                        record_ai_transcript(
                                            &agent_reply.text,
                                            timing_to_inference_snapshot(
                                                &agent_reply.timing,
                                                agent_reply.model,
                                            ),
                                        );
                                        ChatMessage {
                                            content: agent_reply.text.trim_end().to_string(),
                                            from: Some("Assistant".to_string()),
                                            correlation: None,
                                            source: MessageSource::Api,
                                            api_auto_respond: false,
                                            assistant_generation: None,
                                        }
                                    }
                                    Err(e) => ChatMessage {
                                        content: format!("Ollama error: {e}"),
                                        from: Some("System".to_string()),
                                        correlation: None,
                                        source: MessageSource::System,
                                        api_auto_respond: false,
                                        assistant_generation: None,
                                    },
                                };
                                if let Some(room_id) = selected_room
                                    && let Ok(store) = shared_store(&store_path)
                                {
                                    let ts = ChatExample::display_time_for_message(&reply);
                                    let _ = store.append_message(&room_id, &reply, &ts);
                                }
                                let _ = tx.send(reply);
                                if let Ok(mut flag) = waiting.lock() {
                                    *flag = false;
                                }
                            });
                        }
                        }
                    }
                });
                ui.add_space(8.0);
            });
        });
}

fn render_right_bar(
    ui: &mut egui::Ui,
    app: &mut HumaiPsy,
    state: &mut OverviewChatUiState,
    human_to_agent_mode: bool,
    metrics_config: &MetricsConfig,
) {
    egui::Frame::default()
        .stroke(egui::Stroke::new(
            1.0_f32,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                let exp_snapshot = state.exp_design.clone();
                let selected_room_id = state.chat.selected_room.as_deref().unwrap_or("(none)");
                let selected_room_name = state
                    .chat
                    .selected_room
                    .as_ref()
                    .and_then(|id| {
                        state
                            .chat
                            .rooms
                            .iter()
                            .find(|room| room.id == *id)
                            .map(|room| room.name.as_str())
                    })
                    .unwrap_or("(none)");

                ui.label(egui::RichText::new("Experimental Design Control Panel").strong().size(16.0));
                ui.separator();
                ui.label(format!("Selected room ID: {selected_room_id}"));
                ui.label(format!("Selected room name: {selected_room_name}"));
                ui.label(format!(
                    "Chat input mode: {}",
                    if human_to_agent_mode { "Enabled" } else { "Disabled" }
                ));
                ui.label(format!("Messages in view: {}", state.chat.messages.len()));
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                let columns_height = 300.0;

                let total_width = ui.available_width();
                let left_width = 200.0;
                let middle_width = 300.0;
                let agent_width = 360.0;

                ui.allocate_ui_with_layout(
                    egui::vec2(total_width, columns_height),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(left_width, columns_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_width(left_width);
                                ui.set_max_width(left_width);
                                egui::Frame::default()
                                    .stroke(egui::Stroke::new(
                                        1.0_f32,
                                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                                    ))
                                    .inner_margin(egui::Margin::same(6))
                                    .show(ui, |ui| {
                                        ui.set_min_width(left_width);
                                        ui.set_max_width(left_width);
                                        ui.set_min_height(columns_height);
                                        ui.set_height(columns_height);
                                        let label_wrap_width = (left_width - 16.0).max(80.0);
                                        let left_row = |ui: &mut egui::Ui, text: String| {
                                            ui.allocate_ui_with_layout(
                                                egui::vec2(label_wrap_width, 0.0),
                                                egui::Layout::left_to_right(egui::Align::Min),
                                                |ui| {
                                                    ui.add(egui::Label::new(text).wrap());
                                                },
                                            );
                                        };

                                        left_row(ui, format!(
                                            "CSV: {}",
                                            display_csv_path(
                                                &design_snapshot(&exp_snapshot),
                                                metrics_config,
                                            )
                                        ));
                                        ui.add_space(4.0);

                                        left_row(ui, format!("Groups: {}", exp_snapshot.groups));
                                        ui.add_space(4.0);
                                        left_row(ui, format!("Conditions: {}", exp_snapshot.conditions));
                                        ui.add_space(4.0);
                                        left_row(ui, format!("Trials: {}", exp_snapshot.trials));
                                        ui.add_space(4.0);
                                        left_row(ui, format!("Trial (s): {}", exp_snapshot.trial_seconds));
                                        ui.add_space(4.0);
                                        left_row(ui, format!("Topic: {}", exp_snapshot.topic));
                                        ui.add_space(4.0);
                                        left_row(ui, format!("Question: {}", exp_snapshot.question));
                                    });
                            },
                        );

                        ui.allocate_ui_with_layout(
                            egui::vec2(middle_width, columns_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_width(middle_width);
                                ui.set_max_width(middle_width);
                                egui::Frame::default()
                                    .stroke(egui::Stroke::new(
                                        1.0_f32,
                                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                                    ))
                                    .inner_margin(egui::Margin::same(6))
                                    .show(ui, |ui| {
                                        ui.set_min_width(middle_width);
                                        ui.set_max_width(middle_width);
                                        ui.set_min_height(columns_height);
                                        ui.set_height(columns_height);

                                        ui.horizontal(|ui| {
                                            ui.label("Name:");
                                            let response = ui.add(
                                                egui::TextEdit::singleline(&mut state.exp_design.name)
                                                    .desired_width(140.0),
                                            );
                                            if response.changed() {
                                                state.exp_design.sync_csv_name_from_name();
                                            }
                                        });

                                        ui.add_space(4.0);

                                        ui.horizontal(|ui| {
                                            ui.label("Groups:");
                                            ui.add(
                                                egui::DragValue::new(&mut state.exp_design.groups)
                                                    .range(1..=32)
                                                    .speed(0.05),
                                            );
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Conditions:");
                                            ui.add(
                                                egui::DragValue::new(&mut state.exp_design.conditions)
                                                    .range(1..=humaipsy_core::experiment::MAX_CONDITIONS as u32)
                                                    .speed(0.05),
                                            )
                                            .on_hover_text(
                                                "HumaiPsy currently supports up to 2 named conditions \
                                                 (Cooperative and Argumentative), each with its own \
                                                 system prompt. 1 condition runs Cooperative only, no \
                                                 counterbalancing.",
                                            );
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Trials per condition:");
                                            ui.add(
                                                egui::DragValue::new(&mut state.exp_design.trials)
                                                    .range(1..=32)
                                                    .speed(0.05),
                                            );
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Seconds:");
                                            ui.add(
                                                egui::DragValue::new(&mut state.exp_design.trial_seconds)
                                                    .range(1..=3600)
                                                    .speed(1.0),
                                            );
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Topic:");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::TextEdit::singleline(&mut state.exp_design.topic)
                                                    .desired_width((middle_width - 24.0).max(120.0)),
                                            );
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Final Question to Participant");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::TextEdit::singleline(&mut state.exp_design.question)
                                                    .hint_text("Type final question")
                                                    .desired_width((middle_width - 24.0).max(120.0)),
                                            );
                                        });

                                        ui.add_space(8.0);
                                        ui.separator();
                                        ui.add_space(8.0);

                                        ui.label(
                                            egui::RichText::new(format!(
                                                "groups={} | conditions={} | trials={} | seconds={}",
                                                exp_snapshot.groups,
                                                exp_snapshot.conditions,
                                                exp_snapshot.trials,
                                                exp_snapshot.trial_seconds,
                                            ))
                                            .small()
                                            .weak(),
                                        );

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            if ui.button("Submit").clicked() {
                                                open_participant_id_modal(state);
                                            }
                                        });
                                    });
                            },
                        );

                        ui.allocate_ui_with_layout(
                            egui::vec2(agent_width, columns_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_width(agent_width);
                                ui.set_max_width(agent_width);
                                ui.set_min_height(columns_height);
                                ui.set_height(columns_height);
                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.set_min_width(agent_width);
                                    ui.set_max_width(agent_width);
                                    ui.set_min_height(columns_height);
                                    ui.set_height(columns_height);
                                    egui::ScrollArea::vertical()
                                        .max_height(columns_height)
                                        .show(ui, |ui| {
                                    ui.label(egui::RichText::new("Experimental Design Agent").strong().size(16.0));
                                    ui.separator();
                                    ui.label(
                                        egui::RichText::new(
                                            "Define the conditions appended to the global agent.",
                                        )
                                        .small()
                                        .weak(),
                                    );
                                    ui.add_space(6.0);

                                    ui.label("Cooperative prompt:");
                                    ui.add(
                                        egui::TextEdit::multiline(&mut app.coop_agent_prompt)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(4),
                                    );

                                    ui.add_space(8.0);
                                    ui.label("Argumentative prompt:");
                                    ui.add(
                                        egui::TextEdit::multiline(&mut app.argum_agent_prompt)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(4),
                                    );
                                    });
                                });
                            },
                        );
                    },
                );
            });
        });
}

fn open_participant_id_modal(state: &mut OverviewChatUiState) {
    state.input_modal.open = true;
    state.input_modal.title = "Participant ID".to_string();
    state.input_modal.label = "Participant ID".to_string();
    state.input_modal.hint = "Type participant ID".to_string();
    state.input_modal.confirm_label = "OK".to_string();
    state.input_modal.input = state.exp_design.participant_id.clone();
    state.input_modal.action = Some(ModalAction::SetParticipantId);
}

fn render_participant_id_modal(ctx: &egui::Context, state: &mut OverviewChatUiState) {
    if !state.input_modal.open {
        return;
    }

    let title = state.input_modal.title.clone();
    let label = state.input_modal.label.clone();
    let hint = state.input_modal.hint.clone();
    let confirm_label = state.input_modal.confirm_label.clone();
    let mut is_open = true;

    let rect = ctx.content_rect();
    let modal_size = egui::vec2((rect.width() * 0.6).max(320.0), (rect.height() * 0.6).max(220.0));
    let mut close_modal = false;

    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .fixed_size(modal_size)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut is_open)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                ui.label(label.clone());
                ui.add_space(8.0);

                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.input_modal.input)
                        .hint_text(hint.clone())
                        .desired_width((modal_size.x * 0.6).max(220.0)),
                );

                ui.add_space(16.0);
                let ok_clicked = ui.button(confirm_label.clone()).clicked()
                    || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));

                if ok_clicked {
                    let value = state.input_modal.input.trim().to_string();
                    match state.input_modal.action.clone() {
                        Some(ModalAction::SetParticipantId) => {
                            state.exp_design.participant_id = value;
                            record_experiment_configured(design_snapshot(&state.exp_design));
                            state.session.configured = true;
                        }
                        None => {}
                    }
                    state.input_modal.open = false;
                    state.input_modal.action = None;
                    close_modal = true;
                }
            });
        });

    if close_modal {
        is_open = false;
    }

    state.input_modal.open = is_open;
    if !is_open {
        state.input_modal.action = None;
    }
}
