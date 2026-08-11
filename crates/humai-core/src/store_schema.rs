pub const USER_VERSION: i32 = 1;

pub fn create_tables(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS conversations (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            selected_model TEXT NOT NULL DEFAULT '',
            chat_token_limit INTEGER NOT NULL DEFAULT 70,
            chat_token_limit_enabled INTEGER NOT NULL DEFAULT 0,
            ollama_token_limit INTEGER NOT NULL DEFAULT 70,
            ollama_token_limit_enabled INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
            content TEXT NOT NULL,
            from_label TEXT,
            source TEXT NOT NULL,
            api_auto_respond INTEGER NOT NULL DEFAULT 0,
            display_timestamp TEXT NOT NULL,
            correlation_json TEXT
        );

        CREATE TABLE IF NOT EXISTS generations (
            message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
            model TEXT NOT NULL,
            num_predict INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_id);
        "#,
    )?;

    // Existing user databases may have an earlier conversations schema.
    // Backfill columns expected by current code so runtime inserts/updates do not fail.
    ensure_conversations_columns(conn)?;

    Ok(())
}

fn ensure_conversations_columns(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    ensure_column(conn, "conversations", "name", "TEXT NOT NULL DEFAULT ''")?;
    ensure_column(
        conn,
        "conversations",
        "selected_model",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "conversations",
        "chat_token_limit",
        "INTEGER NOT NULL DEFAULT 70",
    )?;
    ensure_column(
        conn,
        "conversations",
        "chat_token_limit_enabled",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        conn,
        "conversations",
        "ollama_token_limit",
        "INTEGER NOT NULL DEFAULT 70",
    )?;
    ensure_column(
        conn,
        "conversations",
        "ollama_token_limit_enabled",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    Ok(())
}

fn ensure_column(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    column_def: &str,
) -> rusqlite::Result<()> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&pragma)?;
    let mut rows = stmt.query([])?;
    let mut exists = false;

    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            exists = true;
            break;
        }
    }

    if !exists {
        let alter = format!("ALTER TABLE {table} ADD COLUMN {column} {column_def}");
        conn.execute(&alter, [])?;
    }

    Ok(())
}
