//! Core data model for DevClip.

/// A permanently saved, searchable developer snippet.
///
/// This is the data model suggested by the product spec. Fields are public for
/// easy (de)serialisation in the Tauri layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub id: i64,
    pub name: String,
    pub content: String,
    /// Unix seconds.
    pub created_at: i64,
    /// Unix seconds; `None` until the snippet is used for the first time.
    pub last_used_at: Option<i64>,
    pub use_count: i64,
}

/// A transient clipboard capture. Clipboard history is allowed (Principle 4)
/// but is never the permanent store — entries are promoted into [`Snippet`]s
/// explicitly by the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardEntry {
    pub id: i64,
    pub content: String,
    /// Unix seconds.
    pub created_at: i64,
}

/// A snippet together with its search score and the matched character indices
/// (for highlighting in the UI).
#[derive(Debug, Clone)]
pub struct ScoredSnippet {
    pub snippet: Snippet,
    pub score: i64,
    /// Matched char indices within `snippet.name` (may be empty).
    pub name_indices: Vec<usize>,
    /// Matched char indices within `snippet.content` (may be empty).
    pub content_indices: Vec<usize>,
}
