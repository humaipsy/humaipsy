use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use eframe::egui;

use crate::agents::{Arpsci, GlobalChatMode};

use super::chat::{ChatExample, ChatMessage, Room};
use super::incoming::MessageSource;
use super::store::Store;

static CHAT: OnceLock<Mutex<ChatExample>> = OnceLock::new();
static ACTIVE_ROOM_ID: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static ROOM_ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
pub(crate) static EXPERIMENT_NAME: OnceLock<Mutex<String>> = OnceLock::new();
pub(crate) static EXPERIMENT_CSV_NAME: OnceLock<Mutex<String>> = OnceLock::new();
pub(crate) static FINAL_PARTICIPANT_QUESTION: OnceLock<Mutex<String>> = OnceLock::new();

pub(crate) fn selected_room_id() -> Option<String> {
    CHAT.get()
        .and_then(|c| c.lock().ok())
        .and_then(|c| c.selected_room.clone())
}

pub(crate) fn render_overview_panel(app: &mut Arpsci, ui: &mut egui::Ui) {
    let db_path = PathBuf::from("metrics/overview_chat.sqlite");
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
    let human_to_agent_mode = matches!(app.chat_mode, GlobalChatMode::HumanToAgent);
    chat.room_mode_human_agent = human_to_agent_mode;
    chat.set_main_input_enabled(human_to_agent_mode);

    for item in chat.inbox.drain() {
        let ts = ChatExample::display_time_for_message(&item);
        chat.message_timestamps.push(ts);
        chat.messages.push(item);
    }
    chat.set_rooms(rooms.clone());
    if chat.selected_room.is_none() {
        chat.selected_room = rooms.first().map(|room| room.id.clone());
    }

    if graph_running {
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }

    ui.horizontal(|ui| {
        ui.label("Chat Input:");
        ui.add_enabled_ui(!graph_running, |ui| {
            ui.selectable_value(
                &mut app.chat_mode,
                GlobalChatMode::HumanToAgent,
                "Enabled",
            );
            ui.selectable_value(
                &mut app.chat_mode,
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
    app: &mut Arpsci,
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
    app: &mut Arpsci,
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
            egui::Frame::new()
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    let input_row_height = 28.0;
                    let bottom_gap = 6.0;
                    // Reserve space for the separator and input footer so the message list
                    // cannot push the panel past its bottom edge.
                    let footer_height = input_row_height + bottom_gap + 18.0;
                    let messages_height = (ui.available_height() - footer_height).max(0.0);
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
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut chat.input_text)
                            .hint_text("Type a message")
                            .desired_width((ui.available_width() - 72.0).max(220.0)),
                    );
                    let send_clicked = ui.add(egui::Button::new("Send")).clicked()
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

                        let tx = chat.inbox.sender();
                        let waiting = chat.waiting_for_response().clone();
                        let selected_room = chat.selected_room.clone();
                        let store_path = PathBuf::from("metrics/overview_chat.sqlite");
                        let model = app.selected_ollama_model.trim().to_string();
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
                let experiment_name =
                    EXPERIMENT_NAME.get_or_init(|| Mutex::new("Exp001".to_string()));
                let experiment_csv_name =
                    EXPERIMENT_CSV_NAME.get_or_init(|| Mutex::new("Exp001.csv".to_string()));
                let final_participant_question = FINAL_PARTICIPANT_QUESTION
                    .get_or_init(|| Mutex::new(String::new()));

                ui.label(egui::RichText::new("Experimental Control Panel").strong().size(16.0));
                ui.separator();
                ui.label(format!(
                    "Selected room: {}",
                    chat.selected_room.as_deref().unwrap_or("(none)")
                ));
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
                let left_width = (total_width * 0.3).max(0.0);
                let right_width = (total_width * 0.61).max(0.0);

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

                                        let csv_label = experiment_csv_name
                                            .lock()
                                            .map(|v| v.clone())
                                            .unwrap_or_else(|_| "Exp001.csv".to_string());
                                        ui.horizontal(|ui| {
                                            ui.label("CSV:");
                                            ui.label(csv_label);
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Exp. Design");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Groups: 1");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Conditions: 2");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Blocks: 2");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Trials: 2");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Trial (s): 3000");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Timer: Yes");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Question: Yes");
                                        });

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
                                            if let Ok(mut name) = experiment_name.lock() {
                                                let response = ui.add(
                                                    egui::TextEdit::singleline(&mut *name)
                                                        .desired_width(140.0),
                                                );
                                                if response.changed() {
                                                    let csv_name = format!("{}.csv", name.trim());
                                                    if let Ok(mut csv) = experiment_csv_name.lock() {
                                                        *csv = csv_name;
                                                    }
                                                }
                                            }
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Between Subjects (type group name)");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            let _ = ui.button("Groups");
                                            let _ = ui.button("Group 1");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Within Subjects (Condition)");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            let _ = ui.button("Conditions");
                                            let _ = ui.button("Coop");
                                            let _ = ui.button("Argum");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Number of Condition Blocks");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            let _ = ui.button("No Trials");
                                            let _ = ui.button("2");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Number of Trials per Condition");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            let _ = ui.button("No Trials");
                                            let _ = ui.button("2");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Max Duration of Trial");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            let _ = ui.button("Seconds");
                                            let _ = ui.button("180");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Final Question to Participant​");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            if let Ok(mut question) = final_participant_question.lock() {
                                                ui.add(
                                                    egui::TextEdit::singleline(&mut *question)
                                                        .hint_text("Type final question")
                                                        .desired_width((right_width - 24.0).max(120.0)),
                                                );
                                            }
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Timer​");
                                        });

                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            let _ = ui.button("ON");
                                        });

                                        ui.add_space(50.0);
                                        ui.horizontal(|ui| {
                                            let _ = ui.button("Reset");
                                            let _ = ui.button("Submit");
                                            let _ = ui.button("Download");
                                            let _ = ui.button("Upload");
                                        });

                                        
                                    });
                            },
                        );
                    },
                );
                
            });
        });
}
