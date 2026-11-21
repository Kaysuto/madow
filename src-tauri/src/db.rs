//! The local database: one SQLite file holding everything Madow knows.

use rusqlite::Connection;
use std::path::Path;

/// Schema steps, applied in order. Never edit a step that has shipped: append a new one.
const MIGRATIONS: &[&str] = &[
    // 1: items (the one shape every source is translated into), full-text search, settings.
    "
    CREATE TABLE items (
        id INTEGER PRIMARY KEY,
        source TEXT NOT NULL,
        external_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        title TEXT NOT NULL,
        body TEXT NOT NULL DEFAULT '',
        author TEXT,
        url TEXT,
        state TEXT NOT NULL DEFAULT '',
        triage TEXT NOT NULL DEFAULT 'inbox',
        occurred_at INTEGER NOT NULL,
        ends_at INTEGER,
        meta TEXT NOT NULL DEFAULT '{}',
        synced_at INTEGER NOT NULL,
        UNIQUE (source, external_id)
    );
    CREATE INDEX items_by_time ON items (occurred_at DESC);

    CREATE VIRTUAL TABLE items_fts USING fts5(
        title, body, author,
        content = 'items', content_rowid = 'id',
        tokenize = 'unicode61 remove_diacritics 2'
    );
    CREATE TRIGGER items_fts_insert AFTER INSERT ON items BEGIN
        INSERT INTO items_fts (rowid, title, body, author)
        VALUES (new.id, new.title, new.body, new.author);
    END;
    CREATE TRIGGER items_fts_delete AFTER DELETE ON items BEGIN
        INSERT INTO items_fts (items_fts, rowid, title, body, author)
        VALUES ('delete', old.id, old.title, old.body, old.author);
    END;
    CREATE TRIGGER items_fts_update AFTER UPDATE ON items BEGIN
        INSERT INTO items_fts (items_fts, rowid, title, body, author)
        VALUES ('delete', old.id, old.title, old.body, old.author);
        INSERT INTO items_fts (rowid, title, body, author)
        VALUES (new.id, new.title, new.body, new.author);
    END;

    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) WITHOUT ROWID;
    ",
    // 2: the label a model gave an item when sorting it. Local, like the triage.
    "ALTER TABLE items ADD COLUMN label TEXT;",
    // 3: finished focus sessions. `item_id` is not a foreign key: a sync may remove the item,
    // the time spent on it stays.
    "
    CREATE TABLE focus (
        id INTEGER PRIMARY KEY,
        item_id INTEGER,
        title TEXT NOT NULL,
        started_at INTEGER NOT NULL,
        ended_at INTEGER NOT NULL,
        seconds INTEGER NOT NULL
    );
    CREATE INDEX focus_by_end ON focus (ended_at);
    ",
    // 4: conversations with the assistant. `messages` is a JSON array of {role, content}.
    // The single conversation that used to live in the `chat` setting becomes the first one,
    // and stays the one that is open.
    "
    CREATE TABLE conversations (
        id INTEGER PRIMARY KEY,
        title TEXT NOT NULL,
        messages TEXT NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX conversations_by_time ON conversations (updated_at DESC);

    INSERT INTO conversations (title, messages, updated_at)
    SELECT substr(json_extract(value, '$[0].content'), 1, 80), value, strftime('%s', 'now')
    FROM settings WHERE key = 'chat' AND json_array_length(value) > 0;
    INSERT INTO settings (key, value) SELECT 'chat.current', id FROM conversations;
    DELETE FROM settings WHERE key = 'chat';
    ",
    // 5: the journal of what the user does with what arrives, read to suggest rules. It keeps
    // the kind and the author of the item, not the item: a sync may remove it.
    "
    CREATE TABLE usage (
        id INTEGER PRIMARY KEY,
        at INTEGER NOT NULL,
        triage TEXT NOT NULL,
        kind TEXT NOT NULL,
        author TEXT NOT NULL
    );
    CREATE INDEX usage_by_time ON usage (at);
    ",
];

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrate(&conn)?;
    Ok(conn)
}

#[cfg(test)]
pub fn open_in_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory database");
    migrate(&conn).expect("migrations");
    conn
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let done: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, step) in MIGRATIONS.iter().enumerate().skip(done as usize) {
        conn.execute_batch(&format!(
            "BEGIN; {step} PRAGMA user_version = {}; COMMIT;",
            index + 1
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_run_once() {
        let conn = open_in_memory();
        migrate(&conn).expect("second run is a no-op");
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());
    }

    #[test]
    fn the_conversation_kept_as_a_setting_becomes_the_first_of_the_history() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&MIGRATIONS[..3].concat()).unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('chat', ?1)",
            [r#"[{"role":"user","content":"When am I free?"},{"role":"assistant","content":"At 3."}]"#],
        )
        .unwrap();

        migrate(&conn).unwrap();
        let (id, title): (i64, String) = conn
            .query_row("SELECT id, title FROM conversations", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(title, "When am I free?");
        let settings: Vec<(String, String)> = conn
            .prepare("SELECT key, value FROM settings")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(settings, [("chat.current".to_owned(), id.to_string())]);
    }

    #[test]
    fn an_empty_conversation_is_not_carried_over() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&MIGRATIONS[..3].concat()).unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('chat', '[]')",
            [],
        )
        .unwrap();
        migrate(&conn).unwrap();
        let rows: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM conversations) + (SELECT COUNT(*) FROM settings)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0);
    }
}
