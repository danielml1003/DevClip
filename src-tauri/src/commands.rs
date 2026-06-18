//! Tauri IPC commands and the DTOs exchanged with the frontend.
//!
//! DTOs live here (rather than in `devclip-core`) so the core crate stays free
//! of serde/Tauri dependencies and remains trivially testable.

use std::sync::Mutex;

use devclip_core::{now_unix, ScoredSnippet, Snippet, Store, DEFAULT_CLIPBOARD_CAP};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

/// Shared application state: the SQLite-backed store behind a mutex.
pub struct AppState {
    pub store: Mutex<Store>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetDto {
    pub id: i64,
    pub name: String,
    pub content: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub use_count: i64,
}

impl From<Snippet> for SnippetDto {
    fn from(s: Snippet) -> Self {
        SnippetDto {
            id: s.id,
            name: s.name,
            content: s.content,
            created_at: s.created_at,
            last_used_at: s.last_used_at,
            use_count: s.use_count,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultDto {
    pub snippet: SnippetDto,
    pub score: i64,
    pub name_indices: Vec<usize>,
    pub content_indices: Vec<usize>,
}

impl From<ScoredSnippet> for SearchResultDto {
    fn from(r: ScoredSnippet) -> Self {
        SearchResultDto {
            snippet: r.snippet.into(),
            score: r.score,
            name_indices: r.name_indices,
            content_indices: r.content_indices,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardEntryDto {
    pub id: i64,
    pub content: String,
    pub created_at: i64,
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub fn search(state: State<AppState>, query: String) -> Result<Vec<SearchResultDto>, String> {
    let store = state.store.lock().map_err(err)?;
    let results = store.search(&query, now_unix()).map_err(err)?;
    Ok(results.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub fn save_snippet(
    state: State<AppState>,
    name: String,
    content: String,
) -> Result<SnippetDto, String> {
    let name = name.trim();
    if name.is_empty() || content.trim().is_empty() {
        return Err("Name and content are required".into());
    }
    let store = state.store.lock().map_err(err)?;
    let snippet = store.insert_snippet(name, &content, now_unix()).map_err(err)?;
    Ok(snippet.into())
}

#[tauri::command]
pub fn delete_snippet(state: State<AppState>, id: i64) -> Result<(), String> {
    let store = state.store.lock().map_err(err)?;
    store.delete_snippet(id).map_err(err)
}

#[tauri::command]
pub fn list_clipboard(state: State<AppState>) -> Result<Vec<ClipboardEntryDto>, String> {
    let store = state.store.lock().map_err(err)?;
    let entries = store.clipboard_history(DEFAULT_CLIPBOARD_CAP).map_err(err)?;
    Ok(entries
        .into_iter()
        .map(|e| ClipboardEntryDto {
            id: e.id,
            content: e.content,
            created_at: e.created_at,
        })
        .collect())
}

/// Read the current OS clipboard (used by the Save flow).
#[tauri::command]
pub fn current_clipboard(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().read_text().map_err(err)
}

/// Paste `content` into the previously focused application.
///
/// Flow: copy to clipboard → hide our window (so focus returns to the prior
/// app) → after a short delay, synthesize the platform paste shortcut. When a
/// `snippet_id` is supplied we also record the use for ranking.
#[tauri::command]
pub fn paste(
    app: AppHandle,
    state: State<AppState>,
    content: String,
    snippet_id: Option<i64>,
) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;

    app.clipboard().write_text(content).map_err(err)?;

    if let Some(id) = snippet_id {
        let store = state.store.lock().map_err(err)?;
        store.record_use(id, now_unix()).map_err(err)?;
    }

    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }

    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(140));
        send_paste_keystroke();
    });

    Ok(())
}

/// Synthesize Ctrl+V (Cmd+V on macOS) into whatever is now focused.
fn send_paste_keystroke() {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};

    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(_) => return,
    };

    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    let _ = enigo.key(modifier, Direction::Press);
    let _ = enigo.key(Key::Unicode('v'), Direction::Click);
    let _ = enigo.key(modifier, Direction::Release);
}
