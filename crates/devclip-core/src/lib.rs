//! `devclip-core` — the pure-Rust heart of DevClip.
//!
//! This crate contains the data model, SQLite persistence and fuzzy search /
//! ranking. It has no GUI, Tauri or webkit dependencies, which means the most
//! important behaviour (search quality and storage correctness) is fully
//! unit-tested and portable to any platform, including headless CI.
//!
//! The Tauri application in `src-tauri` is a thin shell over this crate.

pub mod fuzzy;
pub mod model;
pub mod search;
pub mod store;

pub use model::{ClipboardEntry, ScoredSnippet, Snippet};
pub use search::search;
pub use store::{Store, DEFAULT_CLIPBOARD_CAP};

/// Current unix time in seconds. Convenience for callers; core functions take
/// an explicit `now` so they stay deterministic and testable.
pub fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
