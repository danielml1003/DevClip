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

/// Discriminates the source of a [`UnifiedResult`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultKind {
    Snippet,
    Clipboard,
}

/// A single ranked row in the default ("Clipboard") view, which searches across
/// BOTH snippets and clipboard history. Snippet and clipboard entries are
/// normalised into a common shape so the UI can render and rank them together.
#[derive(Debug, Clone)]
pub struct UnifiedResult {
    pub kind: ResultKind,
    pub id: i64,
    /// Display title: the snippet name, or the clipboard entry's content.
    pub title: String,
    /// Full content that gets pasted.
    pub content: String,
    pub score: i64,
    /// Matched char indices within `title` (for highlighting).
    pub title_indices: Vec<usize>,
    /// Matched char indices within `content` (for highlighting the preview).
    pub content_indices: Vec<usize>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub use_count: i64,
}
