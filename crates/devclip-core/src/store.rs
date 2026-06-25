//! SQLite-backed persistence for snippets and (transient) clipboard history.
//!
//! Local-first: a single SQLite file, no server, no account. The schema is
//! versioned via `PRAGMA user_version` so future features (variables,
//! expiration, sync metadata) can migrate cleanly.

use crate::model::{ClipboardEntry, ScoredSnippet, Snippet, UnifiedResult};
use crate::search;
use rusqlite::{params, Connection, OptionalExtension};

/// Current schema version. Bump and add a migration arm in [`Store::migrate`]
/// whenever the schema changes.
const SCHEMA_VERSION: i64 = 2;

/// Default cap on retained clipboard history entries. History is a convenience,
/// not the product (Principle 4: explicit promotion keeps the DB clean).
pub const DEFAULT_CLIPBOARD_CAP: usize = 100;

pub type Result<T> = std::result::Result<T, rusqlite::Error>;

/// What a LAN sync merge changed locally. Returned so the UI can report
/// "added 3 snippets, 5 clips" after a sync.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MergeStats {
    pub snippets_added: usize,
    pub snippets_updated: usize,
    pub clips_added: usize,
}

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

        if version < 2 {
            // LAN sync needs a stable cross-machine identity and an edit
            // timestamp for last-write-wins. The auto-increment `id` collides
            // between machines, so it can't be that identity.
            self.conn.execute_batch(
                r#"
                ALTER TABLE snippets ADD COLUMN sync_id    TEXT;
                ALTER TABLE snippets ADD COLUMN updated_at INTEGER;
                "#,
            )?;

            // Backfill existing rows: give each a UUID and treat its creation
            // time as its last edit. Done in Rust because SQLite can't mint UUIDs.
            let stale: Vec<(i64, i64)> = {
                let mut stmt = self
                    .conn
                    .prepare("SELECT id, created_at FROM snippets WHERE sync_id IS NULL")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<Result<_>>()?
            };
            for (id, created_at) in stale {
                self.conn.execute(
                    "UPDATE snippets SET sync_id = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, crate::new_sync_id(), created_at],
                )?;
            }

            self.conn.execute_batch(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_snippets_sync_id
                     ON snippets(sync_id);",
            )?;
        }

        // Future migrations: `if version < 3 { ... }`, etc.

        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    // ----- Snippets -------------------------------------------------------

    /// Insert a new snippet and return it with its assigned id.
    pub fn insert_snippet(&self, name: &str, content: &str, now: i64) -> Result<Snippet> {
        let sync_id = crate::new_sync_id();
        self.conn.execute(
            "INSERT INTO snippets (sync_id, name, content, created_at, updated_at, last_used_at, use_count)
             VALUES (?1, ?2, ?3, ?4, ?4, NULL, 0)",
            params![sync_id, name, content, now],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(Snippet {
            id,
            sync_id,
            name: name.to_string(),
            content: content.to_string(),
            created_at: now,
            updated_at: now,
            last_used_at: None,
            use_count: 0,
        })
    }

    /// Update a snippet's name and/or content. Bumps `updated_at` so the edit
    /// wins on the next sync.
    pub fn update_snippet(&self, id: i64, name: &str, content: &str, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE snippets SET name = ?2, content = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, name, content, now],
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
                "SELECT id, sync_id, name, content, created_at, updated_at, last_used_at, use_count
                 FROM snippets WHERE id = ?1",
                params![id],
                row_to_snippet,
            )
            .optional()
    }

    /// Look a snippet up by its stable cross-machine identity. Used by sync to
    /// decide insert-vs-update for an incoming snippet.
    pub fn get_snippet_by_sync_id(&self, sync_id: &str) -> Result<Option<Snippet>> {
        self.conn
            .query_row(
                "SELECT id, sync_id, name, content, created_at, updated_at, last_used_at, use_count
                 FROM snippets WHERE sync_id = ?1",
                params![sync_id],
                row_to_snippet,
            )
            .optional()
    }

    /// Load all snippets. For a local-first personal tool this comfortably fits
    /// in memory (thousands of rows), and in-memory fuzzy search per keystroke
    /// stays well under a frame.
    pub fn all_snippets(&self) -> Result<Vec<Snippet>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, sync_id, name, content, created_at, updated_at, last_used_at, use_count
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

    /// Unified search across snippets AND clipboard history (the default view).
    /// `clip_limit` bounds how many recent clipboard entries are considered.
    pub fn search_unified(
        &self,
        query: &str,
        now: i64,
        clip_limit: usize,
    ) -> Result<Vec<UnifiedResult>> {
        let snippets = self.all_snippets()?;
        let clips = self.clipboard_history(clip_limit)?;
        Ok(search::search_unified(&snippets, &clips, query, now))
    }

    /// Flush the write-ahead log into the main database file. Used before
    /// copying the database to a new location (e.g. when the user changes the
    /// storage path in settings).
    pub fn checkpoint(&self) -> Result<()> {
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
    }

    pub fn count_snippets(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM snippets", [], |r| r.get(0))
    }

    // ----- Sync merge -----------------------------------------------------

    /// Merge a batch of snippets received from another machine.
    ///
    /// For each incoming snippet, matched by stable `sync_id`:
    /// * **unknown** → inserted as-is (a new snippet to this machine);
    /// * **known, incoming edited more recently** → name/content/`updated_at`
    ///   overwrite the local copy (last-write-wins);
    /// * **known, local same-or-newer** → local name/content are kept.
    ///
    /// In every "known" case usage stats are merged monotonically — the higher
    /// `use_count` and the later `last_used_at` win — so using a snippet on one
    /// machine is never lost and never clobbers a content edit from another.
    pub fn merge_snippets(&self, incoming: &[Snippet]) -> Result<(usize, usize)> {
        let mut added = 0usize;
        let mut updated = 0usize;
        for s in incoming {
            match self.get_snippet_by_sync_id(&s.sync_id)? {
                None => {
                    self.conn.execute(
                        "INSERT INTO snippets
                            (sync_id, name, content, created_at, updated_at, last_used_at, use_count)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            s.sync_id,
                            s.name,
                            s.content,
                            s.created_at,
                            s.updated_at,
                            s.last_used_at,
                            s.use_count
                        ],
                    )?;
                    added += 1;
                }
                Some(local) => {
                    // Stats merge monotonically regardless of who wins the edit.
                    let new_use = local.use_count.max(s.use_count);
                    let new_last = local.last_used_at.max(s.last_used_at);
                    if s.updated_at > local.updated_at {
                        self.conn.execute(
                            "UPDATE snippets
                             SET name = ?2, content = ?3, updated_at = ?4,
                                 last_used_at = ?5, use_count = ?6
                             WHERE id = ?1",
                            params![local.id, s.name, s.content, s.updated_at, new_last, new_use],
                        )?;
                        updated += 1;
                    } else if new_use != local.use_count || new_last != local.last_used_at {
                        // Same/older edit, but newer usage stats to fold in.
                        self.conn.execute(
                            "UPDATE snippets SET last_used_at = ?2, use_count = ?3 WHERE id = ?1",
                            params![local.id, new_last, new_use],
                        )?;
                    }
                }
            }
        }
        Ok((added, updated))
    }

    /// Merge clipboard entries received from another machine. Clipboard text is
    /// its own identity (there's no cross-machine id), so an incoming entry is
    /// added only if its exact content isn't already present. History is then
    /// trimmed to `cap`, keeping the most recent by capture time.
    pub fn merge_clipboard(&self, incoming: &[ClipboardEntry], cap: usize) -> Result<usize> {
        let mut added = 0usize;
        for e in incoming {
            if e.content.trim().is_empty() {
                continue;
            }
            let exists: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM clipboard_history WHERE content = ?1)",
                params![e.content],
                |r| r.get(0),
            )?;
            if exists {
                continue;
            }
            self.conn.execute(
                "INSERT INTO clipboard_history (content, created_at) VALUES (?1, ?2)",
                params![e.content, e.created_at],
            )?;
            added += 1;
        }
        // Trim to cap by capture time (merged entries may be older than local
        // ones, so order by created_at rather than insertion id).
        self.conn.execute(
            "DELETE FROM clipboard_history
             WHERE id NOT IN (
                 SELECT id FROM clipboard_history
                 ORDER BY created_at DESC, id DESC LIMIT ?1
             )",
            params![cap as i64],
        )?;
        Ok(added)
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
        sync_id: row.get(1)?,
        name: row.get(2)?,
        content: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        last_used_at: row.get(6)?,
        use_count: row.get(7)?,
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

    // ----- Sync merge -----------------------------------------------------

    #[test]
    fn inserted_snippet_has_sync_id_and_updated_at() {
        let store = Store::open_in_memory().unwrap();
        let s = store.insert_snippet("a", "b", 100).unwrap();
        assert!(!s.sync_id.is_empty(), "sync_id assigned on insert");
        assert_eq!(s.updated_at, 100, "updated_at stamped on insert");
    }

    #[test]
    fn update_snippet_bumps_updated_at() {
        let store = Store::open_in_memory().unwrap();
        let s = store.insert_snippet("a", "b", 100).unwrap();
        store.update_snippet(s.id, "a2", "b2", 200).unwrap();
        let got = store.get_snippet(s.id).unwrap().unwrap();
        assert_eq!(got.updated_at, 200);
        assert_eq!(got.sync_id, s.sync_id, "sync_id is stable across edits");
    }

    /// A snippet that exists only on the remote machine is inserted locally,
    /// keeping its remote sync_id so it stays the same logical snippet.
    #[test]
    fn merge_inserts_unknown_snippet() {
        let store = Store::open_in_memory().unwrap();
        let incoming = vec![Snippet {
            id: 999, // remote local-id is irrelevant
            sync_id: "remote-uuid".into(),
            name: "Remote".into(),
            content: "value".into(),
            created_at: 10,
            updated_at: 10,
            last_used_at: None,
            use_count: 0,
        }];
        let (added, updated) = store.merge_snippets(&incoming).unwrap();
        assert_eq!((added, updated), (1, 0));
        let got = store.get_snippet_by_sync_id("remote-uuid").unwrap().unwrap();
        assert_eq!(got.name, "Remote");
    }

    /// Last-write-wins: a remote edit newer than the local copy overwrites it.
    #[test]
    fn merge_newer_remote_edit_wins() {
        let store = Store::open_in_memory().unwrap();
        let local = store.insert_snippet("Title", "old", 100).unwrap();
        let incoming = vec![Snippet {
            sync_id: local.sync_id.clone(),
            name: "Title".into(),
            content: "new from other machine".into(),
            updated_at: 200, // newer than local's 100
            ..local.clone()
        }];
        store.merge_snippets(&incoming).unwrap();
        let got = store.get_snippet(local.id).unwrap().unwrap();
        assert_eq!(got.content, "new from other machine");
    }

    /// An older remote edit must NOT clobber a newer local edit.
    #[test]
    fn merge_older_remote_edit_is_ignored() {
        let store = Store::open_in_memory().unwrap();
        let local = store.insert_snippet("Title", "local-current", 300).unwrap();
        let incoming = vec![Snippet {
            sync_id: local.sync_id.clone(),
            content: "stale remote".into(),
            updated_at: 100, // older than local's 300
            ..local.clone()
        }];
        let (added, updated) = store.merge_snippets(&incoming).unwrap();
        assert_eq!((added, updated), (0, 0));
        let got = store.get_snippet(local.id).unwrap().unwrap();
        assert_eq!(got.content, "local-current");
    }

    /// Usage stats merge monotonically even when the local edit is newer, so
    /// using a snippet on another machine is never lost.
    #[test]
    fn merge_folds_in_higher_usage_stats() {
        let store = Store::open_in_memory().unwrap();
        let local = store.insert_snippet("Title", "body", 300).unwrap();
        let incoming = vec![Snippet {
            sync_id: local.sync_id.clone(),
            updated_at: 100, // older edit, so content is NOT taken
            use_count: 7,
            last_used_at: Some(250),
            ..local.clone()
        }];
        store.merge_snippets(&incoming).unwrap();
        let got = store.get_snippet(local.id).unwrap().unwrap();
        assert_eq!(got.use_count, 7, "higher use_count folded in");
        assert_eq!(got.last_used_at, Some(250));
    }

    #[test]
    fn merge_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let local = store.insert_snippet("Title", "body", 100).unwrap();
        let incoming = vec![store.get_snippet(local.id).unwrap().unwrap()];
        store.merge_snippets(&incoming).unwrap();
        let again = store.merge_snippets(&incoming).unwrap();
        assert_eq!(again, (0, 0), "re-merging the same data changes nothing");
        assert_eq!(store.count_snippets().unwrap(), 1);
    }

    #[test]
    fn merge_clipboard_adds_only_new_content() {
        let store = Store::open_in_memory().unwrap();
        store.add_clipboard("already here", 1, 100).unwrap();
        let incoming = vec![
            ClipboardEntry { id: 0, content: "already here".into(), created_at: 5 },
            ClipboardEntry { id: 0, content: "brand new".into(), created_at: 6 },
            ClipboardEntry { id: 0, content: "  ".into(), created_at: 7 }, // empty, skipped
        ];
        let added = store.merge_clipboard(&incoming, 100).unwrap();
        assert_eq!(added, 1, "only the genuinely new entry is added");
        let hist = store.clipboard_history(100).unwrap();
        assert_eq!(hist.len(), 2);
    }

    #[test]
    fn merge_clipboard_trims_to_cap_by_recency() {
        let store = Store::open_in_memory().unwrap();
        store.add_clipboard("local newest", 100, 100).unwrap();
        let incoming = vec![
            ClipboardEntry { id: 0, content: "remote oldest".into(), created_at: 1 },
            ClipboardEntry { id: 0, content: "remote middle".into(), created_at: 50 },
        ];
        store.merge_clipboard(&incoming, 2).unwrap();
        let kept: std::collections::HashSet<String> = store
            .clipboard_history(100)
            .unwrap()
            .into_iter()
            .map(|e| e.content)
            .collect();
        assert_eq!(kept.len(), 2, "trimmed to cap");
        assert!(kept.contains("local newest"));
        assert!(kept.contains("remote middle"));
        assert!(!kept.contains("remote oldest"), "oldest by time is dropped");
    }

    #[test]
    fn migration_backfills_sync_id_for_old_rows() {
        // Simulate a v1 database, then reopen to trigger the v2 migration.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devclip.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE snippets (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL, content TEXT NOT NULL,
                    created_at INTEGER NOT NULL, last_used_at INTEGER,
                    use_count INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE clipboard_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    content TEXT NOT NULL, created_at INTEGER NOT NULL
                );
                INSERT INTO snippets (name, content, created_at) VALUES ('legacy', 'cmd', 42);
                "#,
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 1i64).unwrap();
        }
        let store = Store::open(&path).unwrap();
        let all = store.all_snippets().unwrap();
        assert_eq!(all.len(), 1);
        assert!(!all[0].sync_id.is_empty(), "old row got a sync_id");
        assert_eq!(all[0].updated_at, 42, "updated_at backfilled from created_at");
    }
}
