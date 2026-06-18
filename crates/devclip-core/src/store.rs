//! SQLite-backed persistence for snippets and (transient) clipboard history.
//!
//! Local-first: a single SQLite file, no server, no account. The schema is
//! versioned via `PRAGMA user_version` so future features (variables,
//! expiration, sync metadata) can migrate cleanly.

use crate::model::{ClipboardEntry, ScoredSnippet, Snippet};
use crate::search;
use rusqlite::{params, Connection, OptionalExtension};

/// Current schema version. Bump and add a migration arm in [`Store::migrate`]
/// whenever the schema changes.
const SCHEMA_VERSION: i64 = 1;

/// Default cap on retained clipboard history entries. History is a convenience,
/// not the product (Principle 4: explicit promotion keeps the DB clean).
pub const DEFAULT_CLIPBOARD_CAP: usize = 100;

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) a DevClip database at `path`.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::from_conn(conn)
    }

    /// Open an in-memory database (used by tests).
    pub fn open_in_memory() -> Result<Self> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<Self> {
        // Pragmas tuned for a snappy local desktop app.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 =
            self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?;

        if version < 1 {
            self.conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS snippets (
                    id           INTEGER PRIMARY KEY AUTOINCREMENT,
                    name         TEXT    NOT NULL,
                    content      TEXT    NOT NULL,
                    created_at   INTEGER NOT NULL,
                    last_used_at INTEGER,
                    use_count    INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS clipboard_history (
                    id         INTEGER PRIMARY KEY AUTOINCREMENT,
                    content    TEXT    NOT NULL,
                    created_at INTEGER NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_snippets_last_used
                    ON snippets(last_used_at DESC);
                CREATE INDEX IF NOT EXISTS idx_clipboard_created
                    ON clipboard_history(created_at DESC);
                "#,
            )?;
        }

        // Future migrations: `if version < 2 { ... }`, etc.

        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    // ----- Snippets -------------------------------------------------------

    /// Insert a new snippet and return it with its assigned id.
    pub fn insert_snippet(&self, name: &str, content: &str, now: i64) -> Result<Snippet> {
        self.conn.execute(
            "INSERT INTO snippets (name, content, created_at, last_used_at, use_count)
             VALUES (?1, ?2, ?3, NULL, 0)",
            params![name, content, now],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(Snippet {
            id,
            name: name.to_string(),
            content: content.to_string(),
            created_at: now,
            last_used_at: None,
            use_count: 0,
        })
    }

    /// Update a snippet's name and/or content.
    pub fn update_snippet(&self, id: i64, name: &str, content: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE snippets SET name = ?2, content = ?3 WHERE id = ?1",
            params![id, name, content],
        )?;
        Ok(())
    }

    pub fn delete_snippet(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM snippets WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_snippet(&self, id: i64) -> Result<Option<Snippet>> {
        self.conn
            .query_row(
                "SELECT id, name, content, created_at, last_used_at, use_count
                 FROM snippets WHERE id = ?1",
                params![id],
                row_to_snippet,
            )
            .optional()
    }

    /// Load all snippets. For a local-first personal tool this comfortably fits
    /// in memory (thousands of rows), and in-memory fuzzy search per keystroke
    /// stays well under a frame.
    pub fn all_snippets(&self) -> Result<Vec<Snippet>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, content, created_at, last_used_at, use_count
             FROM snippets",
        )?;
        let rows = stmt.query_map([], row_to_snippet)?;
        rows.collect()
    }

    /// Record that a snippet was used: increments `use_count` and stamps
    /// `last_used_at`. This drives the recency/frequency ranking.
    pub fn record_use(&self, id: i64, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE snippets
             SET use_count = use_count + 1, last_used_at = ?2
             WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    /// Fuzzy-search snippets as of `now`. Convenience wrapper that loads all
    /// snippets and ranks them via [`search::search`].
    pub fn search(&self, query: &str, now: i64) -> Result<Vec<ScoredSnippet>> {
        let snippets = self.all_snippets()?;
        Ok(search::search(&snippets, query, now))
    }

    pub fn count_snippets(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM snippets", [], |r| r.get(0))
    }

    // ----- Clipboard history ---------------------------------------------

    /// Record a clipboard capture. Skips empty content and consecutive
    /// duplicates, then trims history to `cap` most-recent entries.
    /// Returns the inserted entry, or `None` if it was skipped.
    pub fn add_clipboard(
        &self,
        content: &str,
        now: i64,
        cap: usize,
    ) -> Result<Option<ClipboardEntry>> {
        if content.trim().is_empty() {
            return Ok(None);
        }
        // Skip if identical to the most recent entry.
        let latest: Option<String> = self
            .conn
            .query_row(
                "SELECT content FROM clipboard_history ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if latest.as_deref() == Some(content) {
            return Ok(None);
        }

        self.conn.execute(
            "INSERT INTO clipboard_history (content, created_at) VALUES (?1, ?2)",
            params![content, now],
        )?;
        let id = self.conn.last_insert_rowid();

        // Trim to cap.
        self.conn.execute(
            "DELETE FROM clipboard_history
             WHERE id NOT IN (
                 SELECT id FROM clipboard_history ORDER BY id DESC LIMIT ?1
             )",
            params![cap as i64],
        )?;

        Ok(Some(ClipboardEntry {
            id,
            content: content.to_string(),
            created_at: now,
        }))
    }

    /// Most-recent clipboard entries, newest first.
    pub fn clipboard_history(&self, limit: usize) -> Result<Vec<ClipboardEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content, created_at
             FROM clipboard_history
             ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(ClipboardEntry {
                id: row.get(0)?,
                content: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn clear_clipboard(&self) -> Result<()> {
        self.conn.execute("DELETE FROM clipboard_history", [])?;
        Ok(())
    }
}

fn row_to_snippet(row: &rusqlite::Row) -> Result<Snippet> {
    Ok(Snippet {
        id: row.get(0)?,
        name: row.get(1)?,
        content: row.get(2)?,
        created_at: row.get(3)?,
        last_used_at: row.get(4)?,
        use_count: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let s = store
            .insert_snippet("Prod migrate", "docker compose exec api migrate", 100)
            .unwrap();
        assert!(s.id > 0);
        let got = store.get_snippet(s.id).unwrap().unwrap();
        assert_eq!(got, s);
        assert_eq!(store.count_snippets().unwrap(), 1);
    }

    #[test]
    fn record_use_increments_and_stamps() {
        let store = Store::open_in_memory().unwrap();
        let s = store.insert_snippet("a", "b", 100).unwrap();
        store.record_use(s.id, 250).unwrap();
        store.record_use(s.id, 300).unwrap();
        let got = store.get_snippet(s.id).unwrap().unwrap();
        assert_eq!(got.use_count, 2);
        assert_eq!(got.last_used_at, Some(300));
    }

    #[test]
    fn delete_removes_snippet() {
        let store = Store::open_in_memory().unwrap();
        let s = store.insert_snippet("a", "b", 1).unwrap();
        store.delete_snippet(s.id).unwrap();
        assert!(store.get_snippet(s.id).unwrap().is_none());
    }

    #[test]
    fn search_through_store() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_snippet("Production migration command", "manage.py migrate", 1)
            .unwrap();
        store.insert_snippet("Regex", "^a$", 1).unwrap();
        let results = store.search("mig", 1000).unwrap();
        assert_eq!(results[0].snippet.name, "Production migration command");
    }

    #[test]
    fn clipboard_dedupes_consecutive_and_skips_empty() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.add_clipboard("hello", 1, 10).unwrap().is_some());
        // consecutive duplicate -> skipped
        assert!(store.add_clipboard("hello", 2, 10).unwrap().is_none());
        // empty -> skipped
        assert!(store.add_clipboard("   ", 3, 10).unwrap().is_none());
        assert!(store.add_clipboard("world", 4, 10).unwrap().is_some());
        let hist = store.clipboard_history(10).unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].content, "world"); // newest first
    }

    #[test]
    fn clipboard_trims_to_cap() {
        let store = Store::open_in_memory().unwrap();
        for i in 0..20 {
            store
                .add_clipboard(&format!("entry-{i}"), i, 5)
                .unwrap();
        }
        let hist = store.clipboard_history(100).unwrap();
        assert_eq!(hist.len(), 5, "history trimmed to cap");
        assert_eq!(hist[0].content, "entry-19");
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devclip.db");
        {
            let store = Store::open(&path).unwrap();
            store.insert_snippet("kept", "value", 1).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.count_snippets().unwrap(), 1);
        let all = store.all_snippets().unwrap();
        assert_eq!(all[0].name, "kept");
    }
}
