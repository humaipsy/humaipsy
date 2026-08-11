#[path = "store_schema.rs"]
mod schema;

use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{params, Connection};

use crate::chat_types::{AssistantGeneration, ChatMessage, MessageCorrelation, MessageSource};
use crate::recording::timing::{new_id, now_rfc3339};

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

/// How long a connection will retry against a locked database (e.g. a brief
/// external lock from an AV scanner or a backup tool) before giving up.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        // Ensure metrics directory exists when opening the embedded sqlite file.
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        restrict_to_owner(path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        // WAL: readers don't block writers/each other; NORMAL sync is the
        // recommended, still-durable pairing with WAL (only risks losing the
        // last commit on an OS crash, not on an app crash).
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
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

    /// Locks the connection, recovering it if a previous panic poisoned the mutex
    /// rather than letting that panic cascade into every subsequent DB call.
    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn create_conversation(&self) -> Result<String, StoreError> {
        let id = new_id();
        let now = now_rfc3339();
        let conn = self.conn();
        conn.execute(
            r#"INSERT INTO conversations (id, name, created_at, updated_at)
               VALUES (?1, '', ?2, ?3)"#,
            params![&id, &now, &now],
        )?;
        Ok(id)
    }

    pub fn delete_conversation(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        conn.execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_conversations(&self, limit: usize) -> Result<Vec<ConversationSummary>, StoreError> {
        let conn = self.conn();
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
        let conn = self.conn();
        conn.execute(
            "UPDATE conversations SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, new_name, now_rfc3339()],
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
        let mut conn = self.conn();
        // All three writes must land together, or none should: a crash between
        // them previously could leave a message row without its generation row,
        // or a stale conversation.updated_at.
        let tx = conn.transaction()?;
        tx.execute(
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
        let mid = tx.last_insert_rowid();

        if let Some(ref g) = msg.assistant_generation {
            tx.execute(
                "INSERT INTO generations (message_id, model, num_predict) VALUES (?1, ?2, ?3)",
                params![mid, &g.model, g.num_predict],
            )?;
        }

        let now = now_rfc3339();
        tx.execute(
            "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
            params![&now, conversation_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_messages_for_conversation(&self, conversation_id: &str) -> Result<(), StoreError> {
        let conn = self.conn();
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
        let conn = self.conn();
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

/// Restricts a file to owner-only read/write (`0600`) on Unix, where the
/// default umask otherwise leaves it world-readable (`0644`) — letting any
/// other local user on a shared machine read the full chat transcript
/// history. No-op on Windows, where per-user profile ACLs already isolate this.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn touch_conversation_conn(conn: &Connection, id: &str) -> Result<(), StoreError> {
    // Touch parent conversation so list sorting reflects message-level updates/deletions.
    let now = now_rfc3339();
    conn.execute(
        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
        params![&now, id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open(dir.path().join("chat.sqlite")).expect("open store");
        (dir, store)
    }

    #[test]
    fn open_enables_wal_and_busy_timeout() {
        let (_dir, store) = open_temp_store();
        let conn = store.conn();
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .expect("read journal_mode");
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    }

    #[cfg(unix)]
    #[test]
    fn open_restricts_db_file_permissions_to_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("chat.sqlite");
        let _store = Store::open(&db_path).expect("open store");

        let mode = std::fs::metadata(&db_path).expect("metadata").permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "chat.sqlite should be owner-only read/write, got {:o}",
            mode & 0o777
        );
    }

    #[test]
    fn append_message_persists_message_and_generation_atomically() {
        let (_dir, store) = open_temp_store();
        let conversation_id = store.create_conversation().expect("create conversation");

        let msg = ChatMessage {
            content: "hello".to_string(),
            from: Some("Human".to_string()),
            correlation: None,
            source: MessageSource::Human,
            api_auto_respond: false,
            assistant_generation: Some(AssistantGeneration {
                model: "test-model".to_string(),
                num_predict: Some(64),
            }),
        };
        store
            .append_message(&conversation_id, &msg, "2026-01-01T00:00:00Z")
            .expect("append message");

        let (messages, _timestamps) = store.load_messages(&conversation_id).expect("load messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello");
        let generation = messages[0]
            .assistant_generation
            .as_ref()
            .expect("generation row should be present alongside its message");
        assert_eq!(generation.model, "test-model");
    }

    #[test]
    fn conn_recovers_from_a_poisoned_mutex_instead_of_panicking() {
        let (_dir, store) = open_temp_store();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = store.conn();
            panic!("simulated panic while holding the connection lock");
        }));
        assert!(result.is_err());

        // A naive `.lock().unwrap()` would panic here too, cascading the failure
        // into every subsequent call; `Store::conn` must recover instead.
        assert!(store.create_conversation().is_ok());
    }
}
