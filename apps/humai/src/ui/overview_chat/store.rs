#[path = "store_schema.rs"]
mod schema;

use std::io;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::recording::audit;
use super::chat::{AssistantGeneration, ChatMessage, MessageCorrelation};
use super::incoming::MessageSource;

pub use schema::USER_VERSION;

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "{e}"),
            StoreError::Sqlite(e) => write!(f, "{e}"),
            StoreError::Json(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Json(e)
    }
}

#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub id: String,
    pub name: String,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        // Ensure metrics directory exists when opening the embedded sqlite file.
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        // Always run idempotent schema bootstrap so older DBs get missing columns.
        schema::create_tables(&conn)?;
        let ver: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        if ver < USER_VERSION {
            conn.execute_batch(&format!("PRAGMA user_version = {USER_VERSION};"))?;
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn create_conversation(&self) -> Result<String, StoreError> {
        let id = audit::new_id();
        let now = audit::now_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"INSERT INTO conversations (id, name, created_at, updated_at)
               VALUES (?1, '', ?2, ?3)"#,
            params![&id, &now, &now],
        )?;
        Ok(id)
    }

    pub fn delete_conversation(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_conversations(&self, limit: usize) -> Result<Vec<ConversationSummary>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name FROM conversations ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name) = row?;
            out.push(ConversationSummary { id, name });
        }
        Ok(out)
    }

    pub fn rename_conversation(&self, id: &str, new_name: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE conversations SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, new_name, audit::now_rfc3339()],
        )?;
        Ok(())
    }

    pub fn append_message(&self, conversation_id: &str, msg: &ChatMessage, display_ts: &str) -> Result<(), StoreError> {
        // Keep optional correlation payload as JSON to preserve request/event linkage.
        let correlation_json = msg
            .correlation
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"INSERT INTO messages (
                conversation_id, content, from_label, source, api_auto_respond,
                display_timestamp, correlation_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                conversation_id,
                &msg.content,
                msg.from.as_deref(),
                msg.source.as_db(),
                msg.api_auto_respond as i32,
                display_ts,
                correlation_json,
            ],
        )?;
        let mid = conn.last_insert_rowid();

        if let Some(ref g) = msg.assistant_generation {
            conn.execute(
                "INSERT INTO generations (message_id, model, num_predict) VALUES (?1, ?2, ?3)",
                params![mid, &g.model, g.num_predict],
            )?;
        }

        let now = audit::now_rfc3339();
        conn.execute(
            "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
            params![&now, conversation_id],
        )?;
        Ok(())
    }

    pub fn delete_messages_for_conversation(&self, conversation_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM messages WHERE conversation_id = ?1",
            params![conversation_id],
        )?;
        touch_conversation_conn(&conn, conversation_id)?;
        Ok(())
    }

    pub fn load_messages(
        &self,
        conversation_id: &str,
    ) -> Result<(Vec<ChatMessage>, Vec<String>), StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r#"SELECT m.id, m.content, m.from_label, m.source, m.api_auto_respond,
                      m.display_timestamp, m.correlation_json,
                      g.model, g.num_predict
               FROM messages m
               LEFT JOIN generations g ON g.message_id = m.id
               WHERE m.conversation_id = ?1
               ORDER BY m.id ASC"#,
        )?;

        let rows = stmt.query_map(params![conversation_id], |r| {
            let correlation: Option<MessageCorrelation> = match r.get::<_, Option<String>>(6)? {
                Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                })?),
                None => None,
            };
            let source = MessageSource::from_db(&r.get::<_, String>(3)?).unwrap_or(MessageSource::System);
            let api_ar = r.get::<_, i32>(4)? != 0;
            let display_ts: String = r.get(5)?;

            let assistant_generation =
                if let Some(model) = r.get::<_, Option<String>>(7)? {
                    Some(AssistantGeneration {
                        model,
                        num_predict: r.get(8)?,
                    })
                } else {
                    None
                };

            Ok((
                ChatMessage {
                    content: r.get(1)?,
                    from: r.get(2)?,
                    correlation,
                    source,
                    api_auto_respond: api_ar,
                    assistant_generation,
                },
                display_ts,
            ))
        })?;

        let mut messages = Vec::new();
        let mut timestamps = Vec::new();
        for row in rows {
            let (m, ts) = row?;
            messages.push(m);
            timestamps.push(ts);
        }
        Ok((messages, timestamps))
    }
}

fn touch_conversation_conn(conn: &Connection, id: &str) -> Result<(), StoreError> {
    // Touch parent conversation so list sorting reflects message-level updates/deletions.
    let now = audit::now_rfc3339();
    conn.execute(
        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
        params![&now, id],
    )?;
    Ok(())
}
