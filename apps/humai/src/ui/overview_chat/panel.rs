use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use egui_phosphor::regular;

use crate::app_model::{GlobalChatMode, Humai};

use super::chat::{ChatExample, ChatMessage, Room};
use super::incoming::MessageSource;
use super::store::Store;

static CHAT: OnceLock<Mutex<ChatExample>> = OnceLock::new();
static ACTIVE_ROOM_ID: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static ROOM_ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
pub(crate) static HUMAI_EXPDESIGN: OnceLock<Mutex<HumaiExpDesign>> = OnceLock::new();
static EXPERIMENT_SESSION: OnceLock<Mutex<ExperimentSessionState>> = OnceLock::new();
static EXP_RUN_COUNTER: AtomicU64 = AtomicU64::new(0);

static GLOBAL_INPUT_MODAL: OnceLock<Mutex<InputModalState>> = OnceLock::new();

const EXP_CONDITION_NAMES: [&str; 2] = ["COOP", "ARGUM"];
const EXP_TRIALS_PER_CONDITION: usize = 2;

#[derive(Clone, Debug)]
struct ExperimentSessionState {
    configured: bool,
    running: bool,
    experiment_id: Option<String>,
    run_id: Option<String>,
    trial_index: usize,
    trial_memory_start_idx: usize,
    trial_timeout_popup_open: bool,
    finished_popup_open: bool,
}

impl Default for ExperimentSessionState {
    fn default() -> Self {
        Self {
            configured: false,
            running: false,
            experiment_id: None,
            run_id: None,
            trial_index: 0,
            trial_memory_start_idx: 0,
            trial_timeout_popup_open: false,
            finished_popup_open: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct TrialDescriptor {
    condition_index: usize,
    trial_in_condition: usize,
}

fn total_experiment_trials() -> usize {
    // 2 conditions x 2 trials per condition = 4 trial windows.
    EXP_CONDITION_NAMES.len() * EXP_TRIALS_PER_CONDITION
}

fn trial_descriptor(index: usize) -> TrialDescriptor {
    // Sequence by blocks: [COOP, ARGUM] x 2 blocks.
    // This gives trial order: COOP-1, ARGUM-1, COOP-2, ARGUM-2.
    let condition_index = index % EXP_CONDITION_NAMES.len();
    let trial_in_condition = index / EXP_CONDITION_NAMES.len();
    TrialDescriptor {
        condition_index,
        trial_in_condition,
    }
}

fn current_trial_descriptor_from_state(state: &ExperimentSessionState) -> TrialDescriptor {
    let capped_index = state.trial_index.min(total_experiment_trials().saturating_sub(1));
    trial_descriptor(capped_index)
}

fn experiment_status_label(state: &ExperimentSessionState) -> String {
    if !state.configured {
        return "Experiment".to_string();
    }

    let desc = current_trial_descriptor_from_state(state);
    format!(
        "Experiment, Cond {}, Trial {}",
        desc.condition_index + 1,
        desc.trial_in_condition + 1
    )
}

fn condition_instruction_for_state(state: &ExperimentSessionState) -> &'static str {
    let desc = current_trial_descriptor_from_state(state);
    let condition = EXP_CONDITION_NAMES
        .get(desc.condition_index)
        .copied()
        .unwrap_or("COOP");

    match condition {
        "ARGUM" => {
            "Experimental condition ARGUM: be argumentative with the human participant. You must challenge assumptions, present counterpoints, and disagree when appropriate. Do NOT use a supportive or reassuring therapeutic style."
        }
        _ => {
            "Experimental condition COOP: be cooperative with the human participant. Be supportive, collaborative, and help them progress constructively while staying safe and on-topic."
        }
    }
}

fn assistant_prompt_for_current_condition(base_prompt: &str) -> String {
    let state = EXPERIMENT_SESSION
        .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
        .lock()
        .map(|state| state.clone())
        .unwrap_or_default();

    format!(
        "{}\n\n{}",
        base_prompt,
        condition_instruction_for_state(&state)
    )
}

fn advance_to_next_trial(app: &Humai, store: &Store, chat: &mut ChatExample) {
    let was_running = crate::timer::main_timer_is_running();
    let mut session = match EXPERIMENT_SESSION
        .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
        .lock()
    {
        Ok(guard) => guard,
        Err(_) => return,
    };

    if !session.configured {
        append_message_to_chat_and_store(
            chat,
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

    let next_index = session.trial_index + 1;
    if next_index >= total_experiment_trials() {
        session.running = false;
        session.trial_timeout_popup_open = false;
        session.finished_popup_open = true;
        drop(session);
        crate::timer::pause_main_timer();
        append_message_to_chat_and_store(
            chat,
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
        return;
    }

    session.trial_index = next_index;
    session.trial_memory_start_idx = chat.messages.len();
    session.trial_timeout_popup_open = false;
    let desc = current_trial_descriptor_from_state(&session);
    let condition = EXP_CONDITION_NAMES
        .get(desc.condition_index)
        .copied()
        .unwrap_or("COOP");
    drop(session);

    append_message_to_chat_and_store(
        chat,
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
        crate::timer::stop_and_reset_main_timer();
        crate::timer::start_main_timer();
    }

    let design = HUMAI_EXPDESIGN
        .get_or_init(|| Mutex::new(HumaiExpDesign::default()))
        .lock()
        .map(|design| design.clone())
        .unwrap_or_default();

    let topic = design.topic.trim().to_string();
    let participant_id = design.participant_id.trim().to_string();
    let model = app.selected_ollama_model.trim().to_string();

    if topic.is_empty() || participant_id.is_empty() || model.is_empty() {
        append_message_to_chat_and_store(
            chat,
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

    spawn_ollama_opening_question(app, chat, topic, participant_id, model);
}

fn handle_trial_timeout(app: &Humai, store: &Store, chat: &mut ChatExample) {
    if !crate::timer::main_timer_is_running() {
        return;
    }

    let trial_seconds = HUMAI_EXPDESIGN
        .get_or_init(|| Mutex::new(HumaiExpDesign::default()))
        .lock()
        .map(|design| design.trial_seconds.max(1) as u64)
        .unwrap_or(120);

    if crate::timer::main_timer_elapsed_seconds() < trial_seconds {
        return;
    }

    let desc = {
        let mut session = match EXPERIMENT_SESSION
            .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
            .lock()
        {
            Ok(guard) => guard,
            Err(_) => return,
        };

        if !session.configured || !session.running {
            return;
        }

        session.trial_timeout_popup_open = true;
        current_trial_descriptor_from_state(&session)
    };

    let condition = EXP_CONDITION_NAMES
        .get(desc.condition_index)
        .copied()
        .unwrap_or("COOP");
    append_message_to_chat_and_store(
        chat,
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

    advance_to_next_trial(app, store, chat);
}

fn render_trial_timeout_popup(ctx: &egui::Context) {
    let mut open = EXPERIMENT_SESSION
        .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
        .lock()
        .map(|state| state.trial_timeout_popup_open)
        .unwrap_or(false);

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

    if let Ok(mut state) = EXPERIMENT_SESSION
        .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
        .lock()
    {
        state.trial_timeout_popup_open = open;
    }
}

fn render_experiment_finished_popup(ctx: &egui::Context) {
    let mut open = EXPERIMENT_SESSION
        .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
        .lock()
        .map(|state| state.finished_popup_open)
        .unwrap_or(false);

    if !open {
        return;
    }

    // Safety net: ensure timer is not running while experiment is finished.
    crate::timer::pause_main_timer();

    let mut close_clicked = false;
    egui::Window::new("Experiment Finished")
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .fixed_size(egui::vec2(320.0, 120.0))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                ui.label("Experiment Finished");
                ui.add_space(12.0);
                if ui.button("OK").clicked() {
                    close_clicked = true;
                }
            });
        });

    if close_clicked {
        open = false;
    }

    if let Ok(mut state) = EXPERIMENT_SESSION
        .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
        .lock()
    {
        state.finished_popup_open = open;
    }
}

fn experiment_hash_hex(input: &str) -> String {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn derive_experiment_id_local(signature: &str) -> String {
    format!("exp_{}", &experiment_hash_hex(signature)[..12])
}

fn new_run_id_local() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let count = EXP_RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("run_{}_{:04}", millis, count)
}

fn set_experiment_running(is_running: bool) {
    if let Ok(mut session) = EXPERIMENT_SESSION
        .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
        .lock()
    {
        session.running = is_running;
    }
}

fn mark_experiment_configured() {
    if let Ok(mut session) = EXPERIMENT_SESSION
        .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
        .lock()
    {
        session.configured = true;
    }
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

fn current_trial_memory_start_index() -> usize {
    EXPERIMENT_SESSION
        .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
        .lock()
        .map(|state| state.trial_memory_start_idx)
        .unwrap_or(0)
}

fn agent_memory_from_chat(
    chat: &ChatExample,
    start_index: usize,
) -> Vec<crate::agent::AgentMemoryMessage> {
    chat.messages
        .iter()
        .skip(start_index.min(chat.messages.len()))
        .filter_map(|msg| {
            let role = match msg.source {
                MessageSource::Human => crate::agent::AgentRole::User,
                MessageSource::Api => crate::agent::AgentRole::Assistant,
                _ => return None,
            };

            let content = msg.content.trim();
            if content.is_empty() {
                return None;
            }

            Some(crate::agent::AgentMemoryMessage {
                role,
                content: content.to_string(),
            })
        })
        .collect()
}

fn spawn_ollama_opening_question(
    app: &Humai,
    chat: &ChatExample,
    topic: String,
    participant_id: String,
    model: String,
) {
    let tx = chat.inbox.sender();
    let waiting = chat.waiting_for_response().clone();
    let selected_room = chat.selected_room.clone();
    let store_path = PathBuf::from("metrics/chat.sqlite");
    let ollama_host = app.ollama_host.clone();
    let assistant_prompt = assistant_prompt_for_current_condition(&app.assistant_prompt);
    let app_state = app.app_state.clone();
    let history_size = app.conversation_history_size;

    if let Ok(mut flag) = waiting.lock() {
        *flag = true;
    }

    app.rt_handle.spawn(async move {
        let agent = crate::agent::HumaiAgent::new(ollama_host, model, assistant_prompt, app_state)
            .with_history_size(history_size);

        let out = agent
            .generate_opening_question(&participant_id, &topic)
            .await;

        let reply = match out {
            Ok(content) => ChatMessage {
                content: content.trim_end().to_string(),
                from: Some("Assistant".to_string()),
                correlation: None,
                source: MessageSource::Api,
                api_auto_respond: false,
                assistant_generation: None,
            },
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
            && let Ok(store) = super::store::Store::open(store_path)
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

fn start_experiment_from_play(app: &Humai, store: &Store, chat: &mut ChatExample) {
    let design = HUMAI_EXPDESIGN
        .get_or_init(|| Mutex::new(HumaiExpDesign::default()))
        .lock()
        .map(|design| design.clone())
        .unwrap_or_default();

    let mut session = EXPERIMENT_SESSION
        .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
        .lock()
        .ok();

    if let Some(state) = session.as_deref_mut()
        && state.running
    {
        return;
    }

    if design.participant_id.trim().is_empty() {
        append_message_to_chat_and_store(
            chat,
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

    if design.topic.trim().is_empty() {
        append_message_to_chat_and_store(
            chat,
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
            chat,
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

    crate::timer::start_main_timer();

    let topic = design.topic.trim().to_string();
    let participant_id = design.participant_id.trim().to_string();
    let experiment_id = derive_experiment_id_local(&format!(
        "{}|{}|{}",
        design.name.trim(),
        participant_id,
        topic
    ));
    let run_id = new_run_id_local();

    if let Some(state) = session.as_deref_mut() {
        state.running = true;
        state.configured = true;
        state.experiment_id = Some(experiment_id);
        state.run_id = Some(run_id);
        state.trial_index = 0;
        state.trial_memory_start_idx = chat.messages.len();
        state.trial_timeout_popup_open = false;
        state.finished_popup_open = false;
    }

    // Release the lock before creating prompts that also read experiment state.
    drop(session);

    append_message_to_chat_and_store(
        chat,
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

    spawn_ollama_opening_question(app, chat, topic, participant_id, model);
}

#[derive(Clone, Debug)]
pub(crate) struct HumaiExpDesign {
    pub name: String,
    pub csv_name: String,
    pub participant_id: String,
    pub groups: u32,
    pub conditions: u32,
    pub blocks: u32,
    pub trials: u32,
    pub trial_seconds: u32,
    pub topic: String,
    pub question: String,
}

impl Default for HumaiExpDesign {
    fn default() -> Self {
        Self {
            name: "Exp001".to_string(),
            csv_name: "Exp001.csv".to_string(),
            participant_id: String::new(),
            groups: 1,
            conditions: 2,
            blocks: 2,
            trials: 2,
            trial_seconds: 120,
            topic: "Mental Health".to_string(),
            question: "Are you happy?".to_string(),
        }
    }
}

impl HumaiExpDesign {
    fn sync_csv_name_from_name(&mut self) {
        self.csv_name = format!("{}.csv", self.name.trim());
    }
}

#[derive(Clone, Debug)]
enum ModalAction {
    SetParticipantId,
}

#[derive(Clone, Debug)]
struct InputModalState {
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

pub(crate) fn render_overview_panel(app: &mut Humai, ui: &mut egui::Ui) {
    let db_path = PathBuf::from("metrics/chat.sqlite");
    let store = match Store::open(db_path) {
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
    let room_order = ROOM_ORDER.get_or_init(|| Mutex::new(Vec::new()));

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

    let mut order = room_order.lock().unwrap();
    order.retain(|id| rooms_by_id.contains_key(id));
    for c in &convs {
        if !order.iter().any(|id| id == &c.id) {
            order.push(c.id.clone());
        }
    }

    let mut rooms: Vec<Room> = order
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
        order.push(id);
    }

    let chat_cell = CHAT.get_or_init(|| Mutex::new(ChatExample::new()));
    let active_room_id = ACTIVE_ROOM_ID.get_or_init(|| Mutex::new(None));
    let mut chat = chat_cell.lock().unwrap();

    let graph_running = app.conversation_graph_running.load(Ordering::Acquire);
    let human_to_agent_mode = matches!(app.chat_mode, GlobalChatMode::Enabled);
    chat.room_mode_human_agent = human_to_agent_mode;
    chat.set_main_input_enabled(human_to_agent_mode);

    for item in chat.inbox.drain() {
        let ts = ChatExample::display_time_for_message(&item);
        chat.message_timestamps.push(ts);
        chat.messages.push(item);
    }

    handle_trial_timeout(app, &store, &mut chat);

    chat.set_rooms(rooms.clone());
    if chat.selected_room.is_none() {
        chat.selected_room = rooms.first().map(|room| room.id.clone());
    }

    if graph_running || crate::timer::main_timer_is_running() {
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

    let split_height = ui.available_height() - 20.0;
    egui::Frame::default()
        .stroke(egui::Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_min_height(split_height);
            ui.set_height(split_height);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                let left_rect_width = 140.0;
                let row_height = ui.available_height();
                let right_rect_width =
                    (ui.available_width() - left_rect_width - ui.spacing().item_spacing.x).max(0.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(left_rect_width, row_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::Frame::default()
                            .stroke(egui::Stroke::new(
                                1.0,
                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                            ))
                            .inner_margin(egui::Margin::same(0))
                            .show(ui, |ui| {
                                ui.set_min_height(row_height);
                                ui.set_height(row_height);
                                chat.sidebar_ui(ui);
                            });
                    },
                );

                persist_and_hydrate_selected_room(&store, &rooms, &mut chat, active_room_id);

                ui.allocate_ui_with_layout(
                    egui::vec2(right_rect_width, row_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_height(row_height);
                        ui.set_height(row_height);
                        render_chat_columns(app, ui, &store, &mut chat, human_to_agent_mode);
                    },
                );
            });
        });
}

fn persist_and_hydrate_selected_room(
    store: &Store,
    rooms: &[Room],
    chat: &mut ChatExample,
    active_room_id: &Mutex<Option<String>>,
) {
    let previous_room_id = {
        let active = active_room_id.lock().unwrap();
        active.clone()
    };
    let next_room_id = chat.selected_room.clone();
    if previous_room_id != next_room_id {
        if let Some(prev_id) = previous_room_id {
            let prev_still_exists = rooms.iter().any(|r| r.id == prev_id);
            if prev_still_exists {
                let _ = store.delete_messages_for_conversation(&prev_id);
                for (idx, msg) in chat.messages.iter().enumerate() {
                    let ts = chat
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
            chat.hydrate(msgs, ts);
        }

        let mut active = active_room_id.lock().unwrap();
        *active = next_room_id;
    }
}

fn render_chat_columns(
    app: &mut Humai,
    ui: &mut egui::Ui,
    store: &Store,
    chat: &mut ChatExample,
    human_to_agent_mode: bool,
) {
    let total_width = ui.available_width();
    let chat_column_width = (total_width * 0.52).clamp(240.0, total_width);
    let side_column_width = (total_width * 0.49).clamp(240.0, total_width);
    let row_height = ui.available_height();

    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(chat_column_width, row_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                render_chat_rectangle(app, ui, store, chat, human_to_agent_mode, row_height);
            },
        );

        ui.allocate_ui_with_layout(
            egui::vec2(side_column_width, row_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                render_right_bar(ui, chat, human_to_agent_mode, row_height);
            },
        );
    });
}

fn render_chat_rectangle(
    app: &mut Humai,
    ui: &mut egui::Ui,
    store: &Store,
    chat: &mut ChatExample,
    human_to_agent_mode: bool,
    row_height: f32,
) {
    ui.set_min_height(row_height);
    ui.set_height(row_height);
    egui::Frame::default()
        .stroke(egui::Stroke::new(
            1.0,
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
            let bar_stroke = egui::Stroke::new(1.0, ui.visuals().strong_text_color());

            egui::Frame::new()
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    egui::Frame::new()
                        .fill(bar_fill)
                        .stroke(bar_stroke)
                        .inner_margin(egui::Margin::symmetric(8, 0))
                        .show(ui, |ui| {
                            let status_label = EXPERIMENT_SESSION
                                .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
                                .lock()
                                .map(|state| experiment_status_label(&state))
                                .unwrap_or_else(|_| "Experiment".to_string());

                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), top_row_height),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(egui::RichText::new(status_label).strong());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let next_btn = egui::Button::new(">>").small();
                                        if ui
                                            .add(next_btn)
                                            .on_hover_text("Move to next trial")
                                            .clicked()
                                        {
                                            advance_to_next_trial(app, store, chat);
                                        }

                                        let stop_btn = egui::Button::new(
                                            egui::RichText::new(regular::X)
                                                .line_height(Some(ui.text_style_height(&egui::TextStyle::Body))),
                                        )
                                        .small();
                                        if ui
                                            .add(stop_btn)
                                            .on_hover_text("Stop timer and reset to 00:00:00")
                                            .clicked()
                                        {
                                            crate::timer::stop_and_reset_main_timer();
                                            set_experiment_running(false);
                                            if let Ok(mut state) = EXPERIMENT_SESSION
                                                .get_or_init(|| Mutex::new(ExperimentSessionState::default()))
                                                .lock()
                                            {
                                                state.trial_index = 0;
                                                state.trial_memory_start_idx = 0;
                                                state.trial_timeout_popup_open = false;
                                                state.finished_popup_open = false;
                                            }
                                        }

                                        let pause_btn = egui::Button::new(
                                            egui::RichText::new(regular::PAUSE)
                                                .line_height(Some(ui.text_style_height(&egui::TextStyle::Body))),
                                        )
                                        .small();
                                        if ui
                                            .add_enabled(crate::timer::main_timer_is_running(), pause_btn)
                                            .on_hover_text("Pause timer")
                                            .clicked()
                                        {
                                            crate::timer::pause_main_timer();
                                        }

                                        let play_btn = egui::Button::new(
                                            egui::RichText::new(regular::PLAY)
                                                .line_height(Some(ui.text_style_height(&egui::TextStyle::Body))),
                                        )
                                        .small();
                                        if ui
                                            .add_enabled(!crate::timer::main_timer_is_running(), play_btn)
                                            .on_hover_text("Start timer")
                                            .clicked()
                                        {
                                            start_experiment_from_play(app, store, chat);
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
                                        let len = chat.messages.len();
                                        let time_col_width = 82.0;
                                        for (i, msg) in chat.messages.iter().enumerate() {
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
                                                                    0.5,
                                                                    ui.visuals().widgets.noninteractive.bg_stroke.color,
                                                                ))
                                                                .corner_radius(egui::CornerRadius::same(2))
                                                                .inner_margin(egui::Margin::symmetric(6, 2))
                                                                .show(ui, |ui| {
                                                                    let ts = chat
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
                                                                    0.5,
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
                                        if *chat.waiting_for_response().lock().unwrap() {
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

            

            


            ui.add_enabled_ui(human_to_agent_mode, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let input_width = (ui.available_width() - 72.0).max(220.0);
                    let response = ui.add_sized(
                        egui::vec2(input_width, input_row_height),
                        egui::TextEdit::singleline(&mut chat.input_text)
                            .hint_text("Type a message"),
                    );
                    let send_clicked = ui
                        .add_sized(
                            egui::vec2(52.0, input_row_height),
                            egui::Button::new("Send"),
                        )
                        .clicked()
                        || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));

                    if send_clicked && !chat.input_text.trim().is_empty() {
                        let message_text = chat.input_text.trim().to_string();
                        chat.input_text.clear();

                        let user_message = ChatMessage {
                            content: message_text.clone(),
                            from: Some("Human".to_string()),
                            correlation: None,
                            source: MessageSource::Human,
                            api_auto_respond: false,
                            assistant_generation: None,
                        };
                        let ts = ChatExample::current_timestamp_string();
                        chat.messages.push(user_message.clone());
                        chat.message_timestamps.push(ts.clone());

                        if let Some(room_id) = &chat.selected_room {
                            let _ = store.append_message(room_id, &user_message, &ts);
                        }

                        let trial_memory_start_index = current_trial_memory_start_index();
                        let memory_snapshot =
                            agent_memory_from_chat(chat, trial_memory_start_index);

                        let tx = chat.inbox.sender();
                        let waiting = chat.waiting_for_response().clone();
                        let selected_room = chat.selected_room.clone();
                        let store_path = PathBuf::from("metrics/chat.sqlite");
                        let model = app.selected_ollama_model.trim().to_string();
                        let ollama_host = app.ollama_host.clone();
                        let assistant_prompt = assistant_prompt_for_current_condition(&app.assistant_prompt);
                        let app_state = app.app_state.clone();
                        let history_size = app.conversation_history_size;
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
                                let agent = crate::agent::HumaiAgent::new(
                                    ollama_host,
                                    model,
                                    assistant_prompt,
                                    app_state,
                                )
                                .with_history_size(history_size);
                                let out = agent.reply_from_memory(&memory_snapshot).await;
                                let reply = match out {
                                    Ok(content) => ChatMessage {
                                        content: content.trim_end().to_string(),
                                        from: Some("Assistant".to_string()),
                                        correlation: None,
                                        source: MessageSource::Api,
                                        api_auto_respond: false,
                                        assistant_generation: None,
                                    },
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
                                    && let Ok(store) = super::store::Store::open(store_path)
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
                });
                ui.add_space(8.0);
            });
        });
}

fn render_right_bar(
    ui: &mut egui::Ui,
    chat: &ChatExample,
    human_to_agent_mode: bool,
    row_height: f32,
) {
    ui.set_min_height(row_height);
    ui.set_height(row_height);
    egui::Frame::default()
        .stroke(egui::Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .inner_margin(egui::Margin {
            left: 6,
            right: 6,
            top: 0,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_min_height(row_height);
            ui.set_height(row_height);
            ui.add_space(-1.0);
            ui.vertical(|ui| {
                let exp_design = HUMAI_EXPDESIGN.get_or_init(|| Mutex::new(HumaiExpDesign::default()));
                let exp_snapshot = exp_design
                    .lock()
                    .map(|v| v.clone())
                    .unwrap_or_default();
                let selected_room_id = chat.selected_room.as_deref().unwrap_or("(none)");
                let selected_room_name = chat
                    .selected_room
                    .as_ref()
                    .and_then(|id| {
                        chat.rooms
                            .iter()
                            .find(|room| room.id == *id)
                            .map(|room| room.name.as_str())
                    })
                    .unwrap_or("(none)");

                ui.label(egui::RichText::new("Experimental Control Panel").strong().size(16.0));
                ui.separator();
                ui.label(format!("Selected room ID: {selected_room_id}"));
                ui.label(format!("Selected room name: {selected_room_name}"));
                ui.label(format!(
                    "Chat input mode: {}",
                    if human_to_agent_mode { "Enabled" } else { "Disabled" }
                ));
                ui.label(format!("Messages in view: {}", chat.messages.len()));
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                let columns_height = (ui.available_height() - 20.0).max(0.0);

                let total_width = ui.available_width();
                let left_width = 120.0;
                let right_width = 250.0;

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
                                        1.0,
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

                                        left_row(ui, "Exp. Design".to_string());
                                        ui.add_space(4.0);

                                        left_row(ui, format!("CSV: {}", exp_snapshot.csv_name));
                                        ui.add_space(4.0);
                                        
                                        left_row(ui, format!("Groups: {}", exp_snapshot.groups));
                                        ui.add_space(4.0);
                                        left_row(ui, format!("Conditions: {}", exp_snapshot.conditions));
                                        ui.add_space(4.0);
                                        left_row(ui, format!("Blocks: {}", exp_snapshot.blocks));
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
                            egui::vec2(right_width, columns_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_width(right_width);
                                ui.set_max_width(right_width);
                                egui::Frame::default()
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                                    ))
                                    .inner_margin(egui::Margin::same(6))
                                    .show(ui, |ui| {
                                        ui.set_min_width(right_width);
                                        ui.set_max_width(right_width);
                                        ui.set_min_height(columns_height);
                                        ui.set_height(columns_height);

                                        

                                        ui.horizontal(|ui| {
                                            ui.label("Name:");
                                            if let Ok(mut design) = exp_design.lock() {
                                                let response = ui.add(
                                                    egui::TextEdit::singleline(&mut design.name)
                                                        .desired_width(140.0),
                                                );
                                                if response.changed() {
                                                    design.sync_csv_name_from_name();
                                                }
                                            }
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Between Subjects (Group)");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Groups:");
                                            if let Ok(mut design) = exp_design.lock() {
                                                ui.add(
                                                    egui::DragValue::new(&mut design.groups)
                                                        .range(1..=32)
                                                        .speed(0.05),
                                                );
                                            }
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Within Subjects (Condition)");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Coop x Argum");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Number of Condition Blocks");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Blocks:");
                                            if let Ok(mut design) = exp_design.lock() {
                                                ui.add(
                                                    egui::DragValue::new(&mut design.blocks)
                                                        .range(1..=32)
                                                        .speed(0.05),
                                                );
                                            }
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Number of Trials per Condition");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Trials:");
                                            if let Ok(mut design) = exp_design.lock() {
                                                ui.add(
                                                    egui::DragValue::new(&mut design.trials)
                                                        .range(1..=32)
                                                        .speed(0.05),
                                                );
                                            }
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Max Duration of Trial");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Seconds:");
                                            if let Ok(mut design) = exp_design.lock() {
                                                ui.add(
                                                    egui::DragValue::new(&mut design.trial_seconds)
                                                        .range(1..=3600)
                                                        .speed(1.0),
                                                );
                                            }
                                        });

                                        

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Topic:​");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            if let Ok(mut design) = exp_design.lock() {
                                                ui.add(
                                                    egui::TextEdit::singleline(&mut design.topic)
                                                        .desired_width((right_width - 24.0).max(120.0)),
                                                );
                                            }
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Final Question to Participant​");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            if let Ok(mut design) = exp_design.lock() {
                                                ui.add(
                                                    egui::TextEdit::singleline(&mut design.question)
                                                        .hint_text("Type final question")
                                                        .desired_width((right_width - 24.0).max(120.0)),
                                                );
                                            }
                                        });

                                        ui.add_space(8.0);
                                        ui.separator();
                                        ui.add_space(8.0);

                                        
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "groups={} | blocks={} | trials={} | seconds={}",
                                                exp_snapshot.groups,
                                                exp_snapshot.blocks,
                                                exp_snapshot.trials,
                                                exp_snapshot.trial_seconds,
                                            ))
                                            .small()
                                            .weak(),
                                        );
                                        
                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            if ui.button("Submit").clicked() {
                                                open_participant_id_modal();
                                            }
                                        });
                                        
                                    });
                            },
                        );
                    },
                );
                
            });
        });
    render_participant_id_modal(ui.ctx());
    render_trial_timeout_popup(ui.ctx());
    render_experiment_finished_popup(ui.ctx());
}

fn open_participant_id_modal() {
    let participant_id = HUMAI_EXPDESIGN
        .get_or_init(|| Mutex::new(HumaiExpDesign::default()))
        .lock()
        .map(|design| design.participant_id.clone())
        .unwrap_or_default();

    if let Ok(mut modal) = GLOBAL_INPUT_MODAL
        .get_or_init(|| Mutex::new(InputModalState::default()))
        .lock()
    {
        modal.open = true;
        modal.title = "Participant ID".to_string();
        modal.label = "Participant ID".to_string();
        modal.hint = "Type participant ID".to_string();
        modal.confirm_label = "OK".to_string();
        modal.input = participant_id;
        modal.action = Some(ModalAction::SetParticipantId);
    }
}

fn render_participant_id_modal(ctx: &egui::Context) {
    let (mut is_open, title, label, hint, confirm_label) = {
        let modal_guard = GLOBAL_INPUT_MODAL
            .get_or_init(|| Mutex::new(InputModalState::default()))
            .lock();
        let Ok(modal) = modal_guard else {
            return;
        };

        (
            modal.open,
            modal.title.clone(),
            modal.label.clone(),
            modal.hint.clone(),
            modal.confirm_label.clone(),
        )
    };

    if !is_open {
        return;
    }

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

                if let Ok(mut modal) = GLOBAL_INPUT_MODAL
                    .get_or_init(|| Mutex::new(InputModalState::default()))
                    .lock()
                {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut modal.input)
                            .hint_text(hint.clone())
                            .desired_width((modal_size.x * 0.6).max(220.0)),
                    );

                    ui.add_space(16.0);
                    let ok_clicked = ui.button(confirm_label.clone()).clicked()
                        || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));

                    if ok_clicked {
                        let value = modal.input.trim().to_string();
                        match modal.action.clone() {
                            Some(ModalAction::SetParticipantId) => {
                                if let Ok(mut design) = HUMAI_EXPDESIGN
                                    .get_or_init(|| Mutex::new(HumaiExpDesign::default()))
                                    .lock()
                                {
                                    design.participant_id = value;
                                }
                                mark_experiment_configured();
                            }
                            None => {}
                        }
                        modal.open = false;
                        modal.action = None;
                        close_modal = true;
                    }
                }
            });
        });

    if close_modal {
        is_open = false;
    }

    if let Ok(mut modal) = GLOBAL_INPUT_MODAL
        .get_or_init(|| Mutex::new(InputModalState::default()))
        .lock()
    {
        modal.open = is_open;
        if !is_open {
            modal.action = None;
        }
    }
}
