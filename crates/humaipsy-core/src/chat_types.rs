//! Pure chat/message data types shared by the SQLite [`crate::store::Store`] and the
//! UI layer. No rendering dependency lives here — anything that needs `egui::Ui`
//! belongs in the `humaipsy` UI crate instead.

use std::sync::mpsc::{self, Receiver, Sender};

// Ported from openchat/src/incoming/source.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageSource {
    Human,
    Api,
    #[default]
    System,
}

impl MessageSource {
    pub fn as_db(self) -> &'static str {
        // Stable lowercase labels preserve compatibility across app versions.
        match self {
            MessageSource::Human => "human",
            MessageSource::Api => "api",
            MessageSource::System => "system",
        }
    }

    pub fn from_db(s: &str) -> Option<Self> {
        match s {
            "human" => Some(Self::Human),
            "api" => Some(Self::Api),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Room {
    pub id: String,
    pub name: String,
}

impl Room {
    pub fn new(id: String, name: String) -> Self {
        Room { id, name }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageCorrelation {
    pub conversation_id: String,
    pub event_id: String,
    pub request_id: String,
    pub timestamp_rfc3339: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AssistantGeneration {
    pub model: String,
    pub num_predict: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub content: String,
    pub from: Option<String>,
    pub correlation: Option<MessageCorrelation>,
    pub source: MessageSource,
    pub api_auto_respond: bool,
    pub assistant_generation: Option<AssistantGeneration>,
}

/// Non-blocking mpsc-backed inbox so a background Tokio task can hand a completed
/// message back to the UI thread without blocking egui's frame loop.
pub struct UiInbox<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
}

impl<T> Default for UiInbox<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> UiInbox<T> {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }

    pub fn sender(&self) -> Sender<T> {
        self.tx.clone()
    }

    pub fn drain(&self) -> Vec<T> {
        // Non-blocking drain keeps egui's frame loop responsive.
        let mut out = Vec::new();
        while let Ok(item) = self.rx.try_recv() {
            out.push(item);
        }
        out
    }
}
