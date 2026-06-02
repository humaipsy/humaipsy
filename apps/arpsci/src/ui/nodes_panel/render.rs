use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;


use eframe::egui;
use egui_phosphor::regular;

use crate::agents::Arpsci;
use crate::agents::GlobalChatMode;
use crate::agents::nodes_panel::{
    sync_evaluator_researcher_activity, AgentNodeKind, AgentRecord, NodePayload,
    PanelTab, WORKSPACE_MODULE_ENABLED,
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
                                self.nodes_panel.active_tab == PanelTab::Python,
                                "Python",
                            )
                            .clicked()
                        {
                            self.nodes_panel.active_tab = PanelTab::Python;
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

                    // Statics hoisted here so both Overview and Workspace branches can access them.
                    use crate::ui::overview_chat::chat::{ChatExample, ChatMessage};
                    use std::sync::{Mutex, OnceLock};
                    static CHAT: OnceLock<Mutex<ChatExample>> = OnceLock::new();
                    static ACTIVE_ROOM_ID: OnceLock<Mutex<Option<String>>> = OnceLock::new();

                    // Keep polling UI frames while conversation loops are active so chat turn
                    // events appear incrementally rather than in bursts on the next user input.
                    if self
                        .conversation_graph_running
                        .load(Ordering::Relaxed)
                    {
                        ui.ctx().request_repaint_after(Duration::from_millis(33));
                    }

                    if self.nodes_panel.active_tab == PanelTab::Overview {
                        use crate::ui::overview_chat::chat::Room;
                        use crate::ui::overview_chat::incoming::MessageSource;
                        use crate::ui::overview_chat::store::Store;
                        use std::path::PathBuf;
                        // Static path for demonstration; in real app, make this configurable
                        let db_path = PathBuf::from("metrics/overview_chat.sqlite");
                        let store = match Store::open(db_path) {
                            Ok(s) => s,
                            Err(err) => {
                                ui.colored_label(
                                    ui.visuals().error_fg_color,
                                    format!("Overview chat store error: {err}"),
                                );
                                return;
                            }
                        };

                        // List rooms (conversations) with stable UI ordering.
                        // We intentionally preserve row positions across updates so activity writes
                        // do not reshuffle rooms to the top.
                        let convs = store.list_conversations(50).unwrap_or_default();
                        static ROOM_ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
                        let room_order = ROOM_ORDER.get_or_init(|| Mutex::new(Vec::new()));

                        let mut rooms_by_id: std::collections::HashMap<String, Room> = convs
                            .iter()
                            .map(|c| {
                                (
                                    c.id.clone(),
                                    Room::new(c.id.clone(), format!("Room {}", &c.id[..8])),
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
                            // Always have at least one room
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

                        // Static ChatExample for demonstration; in real app, store in ui_state
                        let chat = CHAT.get_or_init(|| Mutex::new(ChatExample::new()));
                        let active_room_id = ACTIVE_ROOM_ID.get_or_init(|| Mutex::new(None));
                        let mut chat = chat.lock().unwrap();
                        let graph_running = self
                            .conversation_graph_running
                            .load(Ordering::Acquire);
                        let human_to_agent_mode =
                            matches!(self.chat_mode, GlobalChatMode::HumanToAgent);
                        chat.room_mode_human_agent = human_to_agent_mode;
                        chat.set_main_input_enabled(human_to_agent_mode);
                        for item in chat.inbox.drain() {
                            let ts = ChatExample::display_time_for_message(&item);
                            chat.message_timestamps.push(ts);
                            chat.messages.push(item);
                        }

                        // Drain agent turns forwarded from running conversation loops.
                        if let Some(rx) = &self.chat_turn_rx {
                            while let Ok(event) = rx.try_recv() {
                                let msg = ChatMessage {
                                    content: event.content.clone(),
                                    from: Some(event.from.clone()),
                                    correlation: None,
                                    source: MessageSource::Api,
                                    api_auto_respond: false,
                                    assistant_generation: None,
                                };
                                let ts = ChatExample::display_time_for_message(&msg);
                                if let Err(e) = store.append_message(&event.room_id, &msg, &ts) {
                                    eprintln!("[Chat] Failed to persist agent turn: {e}");
                                }
                                if chat.selected_room.as_deref() == Some(event.room_id.as_str()) {
                                    chat.message_timestamps.push(ts);
                                    chat.messages.push(msg);
                                }
                            }
                        }
                        chat.set_rooms(rooms.clone());
                        if chat.selected_room.is_none() {
                            chat.selected_room = Some(rooms[0].id.clone());
                        }

                        // Initial hydration for the first selected room in this session.
                        {
                            let mut active = active_room_id.lock().unwrap();
                            let selected_id = chat.selected_room.clone();
                            if active.is_none()
                                && let Some(selected_id) = selected_id
                            {
                                let (msgs, ts) = store
                                    .load_messages(&selected_id)
                                    .unwrap_or((vec![], vec![]));
                                chat.hydrate(msgs, ts);
                                *active = Some(selected_id);
                            }
                        }

                        ui.horizontal(|ui| {
                            ui.label("Chat Input:");
                            ui.add_enabled_ui(!graph_running, |ui| {
                                ui.selectable_value(
                                    &mut self.chat_mode,
                                    GlobalChatMode::HumanToAgent,
                                    "Enabled",
                                );
                                ui.selectable_value(
                                    &mut self.chat_mode,
                                    GlobalChatMode::AgentToAgent,
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

                        let split_height = ui.available_height() - 20.0; // Account for room selector and spacing
                        egui::Frame::default()
                            .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
                            .inner_margin(egui::Margin::same(6))
                            .show(ui, |ui| {
                            ui.set_min_height(split_height);
                            ui.set_height(split_height);
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                            let left_rect_width = 140.0;
                            let row_height = ui.available_height();
                            let right_rect_width =
                                (ui.available_width() - left_rect_width - ui.spacing().item_spacing.x)
                                    .max(0.0);

                            // THE WIDGETS AND SO ON SHOULD BE INSIDE THIS LEFT RECT

                            // Left bar: chat room sidebar
                            ui.allocate_ui_with_layout(
                                egui::vec2(left_rect_width, row_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    egui::Frame::default()
                                        .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
                                        .inner_margin(egui::Margin::same(0))
                                        .show(ui, |ui| {
                                            ui.set_min_height(row_height);
                                            ui.set_height(row_height);
                                            chat.sidebar_ui(ui);
                                        });
                                },
                            );

                            // Persist current room view, then hydrate selected room, only when room changes.
                            {
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
                                                    .unwrap_or_else(|| {
                                                        ChatExample::display_time_for_message(msg)
                                                    });
                                                let _ = store.append_message(&prev_id, msg, &ts);
                                            }
                                        }
                                    }

                                    if let Some(next_id) = &next_room_id {
                                        let (msgs, ts) = store
                                            .load_messages(next_id)
                                            .unwrap_or((vec![], vec![]));
                                        chat.hydrate(msgs, ts);
                                    }

                                    let mut active = active_room_id.lock().unwrap();
                                    *active = next_room_id;
                                }
                            }

                            
                            // Right: message bubbles area
                            ui.allocate_ui_with_layout(

                                egui::vec2(right_rect_width, row_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                ui.set_min_height(row_height);
                                ui.set_height(row_height);
                                let total_width = ui.available_width();
                                let chat_column_width = (total_width * 0.56).clamp(240.0, total_width);
                                let side_column_width = (total_width * 0.45).clamp(240.0, total_width);

                                ui.horizontal(|ui| {
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(chat_column_width, row_height),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.set_min_height(row_height);
                                            ui.set_height(row_height);
                                            egui::Frame::default()
                                                .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
                                                .inner_margin(egui::Margin::same(0))
                                                .show(ui, |ui| {
                                                    ui.set_min_height(row_height - 60.0); // Account for input row
                                                    ui.set_height(row_height - 60.0); // Account for input row
                                                    ui.set_width((ui.available_width() - 40.0).max(0.0)); // Account for additions on left
                                                    egui::Frame::new()
                                                        .inner_margin(egui::Margin::same(4))
                                                        .show(ui, |ui| {
                                                            let input_row_height = 28.0;
                                                            let bottom_gap = 6.0;
                                                            let messages_height =
                                                                (ui.available_height() - input_row_height - bottom_gap)
                                                                    .max(0.0);
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
                                                                                            // Fixed left column for timestamps.
                                                                                            ui.allocate_ui_with_layout(
                                                                                                egui::vec2(time_col_width, 0.0),
                                                                                                egui::Layout::top_down(egui::Align::Min),
                                                                                                |ui| {
                                                                                                    egui::Frame::new()
                                                                                                        .fill(ui.visuals().widgets.inactive.bg_fill)
                                                                                                        .stroke(egui::Stroke::new(0.5, ui.visuals().widgets.noninteractive.bg_stroke.color))
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

                                                                                            let msg_width = (ui.available_width() - ui.spacing().item_spacing.x)
                                                                                                .max(0.0);
                                                                                            ui.allocate_ui_with_layout(
                                                                                                egui::vec2(msg_width, 0.0),
                                                                                                egui::Layout::top_down(egui::Align::Min),
                                                                                                |ui| {
                                                                                                    // Message bubble — wraps to available width.
                                                                                                    egui::Frame::new()
                                                                                                        .fill(ui.visuals().extreme_bg_color)
                                                                                                        .stroke(egui::Stroke::new(0.5, ui.visuals().widgets.noninteractive.bg_stroke.color))
                                                                                                        .corner_radius(egui::CornerRadius::same(2))
                                                                                                        .inner_margin(egui::Margin::symmetric(8, 4))
                                                                                                        .show(ui, |ui| {
                                                                                                            ui.set_max_width(ui.available_width());
                                                                                                            ui.add(egui::Label::new(&msg.content).wrap());
                                                                                                        });
                                                                                                },
                                                                                            );
                                                                                        },
                                                                                    );
                                                                                    if i + 1 < len {
                                                                                        ui.add_space(4.0);
                                                                                    }
                                                                                }
                                                                            });
                                                                        });
                                                                },
                                                            );
                                                        });
                                                });

                                            ui.add_space(6.0);

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
                                            ui.add_enabled_ui(human_to_agent_mode, |ui| {
                                                ui.horizontal(|ui| {
                                                    let response = ui.add(
                                                        egui::TextEdit::singleline(&mut chat.input_text)
                                                            .hint_text("Type a message")
                                                            .desired_width(300.0),
                                                    );
                                                    let send_clicked = ui
                                                        .add(egui::Button::new("Send"))
                                                        .clicked()
                                                        || (response.lost_focus()
                                                            && ui.input(|i| i.key_pressed(egui::Key::Enter)));

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

                                                        let tx = chat.inbox.sender();
                                                        let waiting = chat.waiting_for_response().clone();
                                                        let selected_room = chat.selected_room.clone();
                                                        let store_path = std::path::PathBuf::from("metrics/overview_chat.sqlite");
                                                        let model = self.selected_ollama_model.trim().to_string();
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
                                                            self.rt_handle.spawn(async move {
                                                                let request = vec![crate::ui::overview_chat::ollama::OllamaMessage {
                                                                    role: "user".to_string(),
                                                                    content: message_text,
                                                                }];
                                                                let out = crate::ui::overview_chat::ollama::chat(
                                                                    &model,
                                                                    &request,
                                                                    crate::ui::overview_chat::ollama::OllamaChatOptions::default(),
                                                                )
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
                                                                        content: format!("Ollama error: {e}"),
                                                                        from: Some("System".to_string()),
                                                                        correlation: None,
                                                                        source: MessageSource::System,
                                                                        api_auto_respond: false,
                                                                        assistant_generation: None,
                                                                    },
                                                                };
                                                                if let Some(room_id) = selected_room
                                                                    && let Ok(store) = crate::ui::overview_chat::store::Store::open(store_path)
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
                                            });
                                        },
                                    );

                                    ui.allocate_ui_with_layout(
                                        egui::vec2(side_column_width, row_height),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.set_min_height(row_height);
                                            ui.set_height(row_height);
                                            egui::Frame::default()
                                                .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
                                                .inner_margin(egui::Margin::same(8))
                                                .show(ui, |ui| {
                                                    ui.set_min_height(row_height - 16.0);
                                                    ui.vertical(|ui| {
                                                        ui.label(egui::RichText::new("Right Bar").strong().size(16.0));
                                                        ui.separator();
                                                        ui.label(format!(
                                                            "Selected room: {}",
                                                            chat.selected_room
                                                                .as_deref()
                                                                .unwrap_or("(none)")
                                                        ));
                                                        ui.label(format!(
                                                            "Chat input mode: {}",
                                                            if human_to_agent_mode { "Enabled" } else { "Disabled" }
                                                        ));
                                                        ui.label(format!(
                                                            "Messages in view: {}",
                                                            chat.messages.len()
                                                        ));
                                                        ui.add_space(8.0);
                                                        ui.label(
                                                            egui::RichText::new(
                                                                "Reserved for the main agent / MAS controls.",
                                                            )
                                                            .small()
                                                            .weak(),
                                                        );
                                                    });
                                                });
                                        },
                                    );
                                });

                                },
                            );
                        });
                        });
                        return;
                    }
                    if self.nodes_panel.active_tab == PanelTab::Ollama {
                        let ctx = ui.ctx().clone();
                        self.render_ollama_settings_widgets(ui, &ctx, ui_state);
                        return;
                    }
                    if self.nodes_panel.active_tab == PanelTab::Python {
                        self.render_python_panel(ui, ui_state);
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
                                self.chat_active_room_id = CHAT
                                    .get()
                                    .and_then(|c| c.lock().ok())
                                    .and_then(|c| c.selected_room.clone());
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
