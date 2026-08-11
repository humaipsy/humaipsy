// UI state and rendering for the overview chat panel.
// This type owns local chat state and forwards committed messages to persistence/hooks.
// The pure data types (ChatMessage, Room, MessageSource, ...) live in
// humai_core::chat_types so they're shared with the SQLite Store without pulling
// egui into the engine crate.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use egui::Ui;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use humai_core::chat_types::{ChatMessage, Room, UiInbox};

pub use humai_core::chat_types::MessageSource;

static RENAME_ROOM_MODAL_OPEN: OnceLock<Mutex<bool>> = OnceLock::new();
static RENAME_ROOM_INPUT: OnceLock<Mutex<String>> = OnceLock::new();
static RENAME_ROOM_TARGET_ID: OnceLock<Mutex<Option<String>>> = OnceLock::new();

pub struct ChatExample {
    pub messages: Vec<ChatMessage>,
    pub message_timestamps: Vec<String>,
    pub inbox: UiInbox<ChatMessage>,
    pub input_text: String,
    pub waiting_for_response: Arc<std::sync::Mutex<bool>>,
    pub main_input_enabled: bool,

    pub rooms: Vec<Room>,
    pub selected_room: Option<String>,
    pub room_mode_human_agent: bool,
}

impl Default for ChatExample {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatExample {
    pub fn set_rooms(&mut self, rooms: Vec<Room>) {
        self.rooms = rooms;
    }

    pub fn add_room(&mut self, name: String, store: &humai_core::store::Store) {
        if let Ok(id) = store.create_conversation() {
            let _ = store.rename_conversation(&id, &name);
            self.rooms.push(Room::new(id.clone(), name));
            self.selected_room = Some(id);
        }
    }

    /// Renders the Overview sidebar (timer, participant ID, room management).
    /// `store` is the already-open chat database handle owned by the caller —
    /// this used to open its own second connection on every frame; now it
    /// reuses the one the caller already has.
    pub fn sidebar_ui(&mut self, ui: &mut Ui, store: &humai_core::store::Store) {
        let rooms_section_height = ui.available_height();
        let rooms_list_height = 200.0;

        

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), rooms_section_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(4, 0))
                    .show(ui, |ui| {
                        let value_row_width = ui.available_width().max(0.0);

                        ui.label(egui::RichText::new("Main Timer").strong().size(16.0));

                        ui.add_space(6.0);

                        let timer_value = humai_core::timer::main_timer_display();
                        Self::render_full_width_value_row(ui, &timer_value, value_row_width);

                        Self::section_separator(ui);

                        ui.label(egui::RichText::new("Participant ID").strong().size(16.0));

                        ui.add_space(6.0);


                        let participant_id = super::panel::HUMAI_EXPDESIGN
                            .get_or_init(|| {
                                std::sync::Mutex::new(super::panel::HumaiExpDesign::default())
                            })
                            .lock()
                            .map(|design| {
                                let trimmed = design.participant_id.trim();
                                if trimmed.is_empty() {
                                    "NONE".to_string()
                                } else {
                                    trimmed.to_string()
                                }
                            })
                            .unwrap_or_else(|_| "NONE".to_string());
                        Self::render_full_width_value_row(ui, &participant_id, value_row_width);

                        Self::section_separator(ui);

                        ui.label(egui::RichText::new("Room Settings").strong().size(16.0));

                        ui.add_space(6.0);

                        if ui.button("New Room").clicked() {
                            let name = format!("Room {}", self.rooms.len() + 1);
                            self.add_room(name, store);
                        }

                        if ui
                            .add_enabled(
                                self.selected_room.is_some(),
                                egui::Button::new("Clear Room"),
                            )
                            .on_hover_text("Clear messages in the current room")
                            .clicked()
                            && let Some(room_id) = self.selected_room.as_ref()
                        {
                            let _ = store.delete_messages_for_conversation(room_id);
                            self.messages.clear();
                            self.message_timestamps.clear();
                        }

                        if ui
                            .add_enabled(
                                self.selected_room.is_some(),
                                egui::Button::new("Rename Room"),
                            )
                            .on_hover_text("Rename the current room")
                            .clicked()
                        {
                            self.open_rename_room_modal();
                        }

                        Self::section_separator(ui);

                        let mut delete_room_id: Option<String> = None;
                        egui::ScrollArea::vertical()
                            .id_salt("overview_sidebar_rooms_scroll")
                            .max_height(rooms_list_height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for room in &self.rooms {
                                    let selected = self.selected_room.as_ref() == Some(&room.id);
                                    ui.horizontal(|ui| {
                                        let row_height = ui.spacing().interact_size.y;
                                        let x_width = row_height;
                                        let selectable_width =
                                            (ui.available_width() - x_width - ui.spacing().item_spacing.x)
                                                .max(0.0);
                                        let label = egui::Button::new(&room.name).selected(selected);
                                        if ui
                                            .add_sized(egui::vec2(selectable_width, row_height), label)
                                            .clicked()
                                        {
                                            self.selected_room = Some(room.id.clone());
                                        }
                                        if ui
                                            .add_sized(egui::vec2(x_width, row_height), egui::Button::new("x"))
                                            .on_hover_text("Delete room")
                                            .clicked()
                                        {
                                            delete_room_id = Some(room.id.clone());
                                        }
                                    });
                                }
                            });

                        if let Some(id) = delete_room_id {
                            self.rooms.retain(|r| r.id != id);
                            let _ = store.delete_conversation(&id);
                            if self.selected_room.as_deref() == Some(id.as_str()) {
                                self.selected_room = self.rooms.first().map(|r| r.id.clone());
                            }
                        }
                    });
                    ui.separator();
                    ui.spacing_mut().item_spacing.y = 2.0;
            },
        );

        self.render_rename_room_modal(ui.ctx(), store);
    }

    fn section_separator(ui: &mut Ui) {
        ui.separator();
        ui.spacing_mut().item_spacing.y = 2.0;
    }

    fn render_full_width_value_row(ui: &mut Ui, value: &str, width: f32) {
        let height = ui.spacing().interact_size.y;
        egui::Frame::new()
            .fill(ui.visuals().widgets.inactive.bg_fill)
            .stroke(ui.visuals().widgets.inactive.bg_stroke)
            .inner_margin(egui::Margin::symmetric(8, 1))
            .show(ui, |ui| {
                ui.set_height(height);
                ui.set_min_width(width);
                ui.set_max_width(width);
                ui.with_layout(
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(egui::RichText::new(value).size(14.0).strong());
                    },
                );
            });
    }

    fn open_rename_room_modal(&mut self) {
        let Some(target_id) = self.selected_room.clone() else {
            return;
        };

        let initial_name = self
            .rooms
            .iter()
            .find(|room| room.id == target_id)
            .map(|room| room.name.clone())
            .unwrap_or_default();

        if let Ok(mut input) = RENAME_ROOM_INPUT
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
        {
            *input = initial_name;
        }

        if let Ok(mut target) = RENAME_ROOM_TARGET_ID
            .get_or_init(|| Mutex::new(None))
            .lock()
        {
            *target = Some(target_id);
        }

        if let Ok(mut open) = RENAME_ROOM_MODAL_OPEN.get_or_init(|| Mutex::new(false)).lock() {
            *open = true;
        }
    }

    fn render_rename_room_modal(&mut self, ctx: &egui::Context, store: &humai_core::store::Store) {
        let mut is_open = RENAME_ROOM_MODAL_OPEN
            .get_or_init(|| Mutex::new(false))
            .lock()
            .map(|value| *value)
            .unwrap_or(false);

        if !is_open {
            return;
        }

        let mut close_modal = false;
        egui::Window::new("Rename Room")
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .fixed_size(egui::vec2(420.0, 140.0))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut is_open)
            .show(ctx, |ui| {
                ui.label("New room name");
                ui.add_space(8.0);

                if let Ok(mut input) = RENAME_ROOM_INPUT
                    .get_or_init(|| Mutex::new(String::new()))
                    .lock()
                {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut *input)
                            .hint_text("Type room name")
                            .desired_width(360.0),
                    );

                    ui.add_space(10.0);
                    let ok_clicked = ui.button("OK").clicked()
                        || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));

                    if ok_clicked {
                        let new_name = input.trim().to_string();
                        if !new_name.is_empty() {
                            let target_id = RENAME_ROOM_TARGET_ID
                                .get_or_init(|| Mutex::new(None))
                                .lock()
                                .ok()
                                .and_then(|value| value.clone());

                            if let Some(room_id) = target_id {
                                if let Some(room) = self.rooms.iter_mut().find(|room| room.id == room_id)
                                {
                                    room.name = new_name.clone();
                                }
                                let _ = store.rename_conversation(&room_id, &new_name);
                            }
                        }
                        close_modal = true;
                    }
                }
            });

        if close_modal {
            is_open = false;
        }

        if let Ok(mut open) = RENAME_ROOM_MODAL_OPEN.get_or_init(|| Mutex::new(false)).lock() {
            *open = is_open;
        }
    }

    pub fn current_timestamp_string() -> String {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let day_secs = now_secs % 86_400;
        let hours = day_secs / 3_600;
        let minutes = (day_secs % 3_600) / 60;
        let seconds = day_secs % 60;
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }

    pub fn display_time_for_message(msg: &ChatMessage) -> String {
        if let Some(c) = &msg.correlation
            && let Ok(odt) = OffsetDateTime::parse(&c.timestamp_rfc3339, &Rfc3339)
        {
            let t = odt.time();
            return format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second());
        }
        Self::current_timestamp_string()
    }

    pub fn default_welcome() -> (ChatMessage, String) {
        let msg = ChatMessage {
            content: "humai Started".to_string(),
            from: Some("System".to_string()),
            correlation: None,
            source: MessageSource::System,
            api_auto_respond: false,
            assistant_generation: None,
        };
        let ts = Self::display_time_for_message(&msg);
        (msg, ts)
    }

    pub fn new() -> Self {
        let inbox = UiInbox::new();
        let (welcome, ts) = Self::default_welcome();
        ChatExample {
            messages: vec![welcome],
            message_timestamps: vec![ts],
            inbox,
            input_text: String::new(),
            waiting_for_response: Arc::new(std::sync::Mutex::new(false)),
            main_input_enabled: true,
            rooms: Vec::new(),
            selected_room: None,
            room_mode_human_agent: true,
        }
    }

    pub fn hydrate(&mut self, messages: Vec<ChatMessage>, message_timestamps: Vec<String>) {
        self.messages = messages;
        self.message_timestamps = message_timestamps;
    }

    pub fn waiting_for_response(&self) -> &Arc<std::sync::Mutex<bool>> {
        &self.waiting_for_response
    }

    pub fn set_main_input_enabled(&mut self, enabled: bool) {
        self.main_input_enabled = enabled;
    }

}
