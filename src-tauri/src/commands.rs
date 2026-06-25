//! Tauri IPC commands and the DTOs exchanged with the frontend.
//!
//! DTOs live here (rather than in `devclip-core`) so the core crate stays free
//! of serde/Tauri dependencies and remains trivially testable.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use devclip_core::{now_unix, ResultKind, ScoredSnippet, Snippet, Store, UnifiedResult};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::settings::Settings;

/// Shared application state.
pub struct AppState {
    pub store: Mutex<Store>,
    pub settings: Mutex<Settings>,
    /// Where `settings.json` lives.
    pub config_path: PathBuf,
    /// The default database path (used by the settings "reset" affordance).
    pub default_db_path: PathBuf,
    /// The last value the tool itself wrote to the clipboard (for a paste),
    /// with a timestamp. The clipboard monitor uses this to recognise its own
    /// output and avoid re-recording it as a new "copy" (the paste loop).
    pub last_self_write: Mutex<Option<(String, Instant)>>,
    /// Handle of the window that was focused before the palette appeared, so we
    /// can restore focus to it and paste reliably. Windows only.
    #[cfg(windows)]
    pub prev_hwnd: std::sync::atomic::AtomicIsize,
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

/// A row in the unified (default Clipboard view) search results.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedResultDto {
    /// "snippet" | "clipboard"
    pub kind: String,
    pub id: i64,
    pub title: String,
    pub content: String,
    pub score: i64,
    pub title_indices: Vec<usize>,
    pub content_indices: Vec<usize>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub use_count: i64,
}

impl From<UnifiedResult> for UnifiedResultDto {
    fn from(r: UnifiedResult) -> Self {
        UnifiedResultDto {
            kind: match r.kind {
                ResultKind::Snippet => "snippet",
                ResultKind::Clipboard => "clipboard",
            }
            .to_string(),
            id: r.id,
            title: r.title,
            content: r.content,
            score: r.score,
            title_indices: r.title_indices,
            content_indices: r.content_indices,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
            use_count: r.use_count,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub db_path: String,
    pub clipboard_cap: usize,
    pub default_db_path: String,
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn settings_dto(state: &AppState, settings: &Settings) -> SettingsDto {
    SettingsDto {
        db_path: settings.db_path.to_string_lossy().into_owned(),
        clipboard_cap: settings.clipboard_cap,
        default_db_path: state.default_db_path.to_string_lossy().into_owned(),
    }
}

/// Search snippets ONLY (the Snippets view). Does not touch clipboard history.
#[tauri::command]
pub fn search(state: State<AppState>, query: String) -> Result<Vec<SearchResultDto>, String> {
    let store = state.store.lock().map_err(err)?;
    let results = store.search(&query, now_unix()).map_err(err)?;
    Ok(results.into_iter().map(Into::into).collect())
}

/// Unified search across BOTH snippets and clipboard history (the default
/// Clipboard view).
#[tauri::command]
pub fn search_all(state: State<AppState>, query: String) -> Result<Vec<UnifiedResultDto>, String> {
    let cap = state.settings.lock().map_err(err)?.clipboard_cap;
    let store = state.store.lock().map_err(err)?;
    let results = store
        .search_unified(&query, now_unix(), cap)
        .map_err(err)?;
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
    let cap = state.settings.lock().map_err(err)?.clipboard_cap;
    let store = state.store.lock().map_err(err)?;
    let entries = store.clipboard_history(cap).map_err(err)?;
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

// ----- Settings ---------------------------------------------------------------

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Result<SettingsDto, String> {
    let settings = state.settings.lock().map_err(err)?;
    Ok(settings_dto(&state, &settings))
}

#[tauri::command]
pub fn set_settings(
    state: State<AppState>,
    db_path: String,
    clipboard_cap: usize,
) -> Result<SettingsDto, String> {
    let cap = clipboard_cap.clamp(1, 100_000);
    let new_path = PathBuf::from(db_path.trim());
    if new_path.as_os_str().is_empty() {
        return Err("Storage path cannot be empty".into());
    }

    // Lock order is always settings -> store (see search_all/list_clipboard).
    let mut settings = state.settings.lock().map_err(err)?;
    let mut store = state.store.lock().map_err(err)?;

    if new_path != settings.db_path {
        if let Some(parent) = new_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create folder for that path: {e}"))?;
        }
        // Carry existing data to the new location if nothing is there yet.
        let _ = store.checkpoint();
        if !new_path.exists() {
            std::fs::copy(&settings.db_path, &new_path).ok();
        }
        let new_store = Store::open(&new_path)
            .map_err(|e| format!("Could not open a database at that path: {e}"))?;
        *store = new_store;
        settings.db_path = new_path;
    }

    settings.clipboard_cap = cap;
    settings.save(&state.config_path).map_err(err)?;

    Ok(settings_dto(&state, &settings))
}

// Settings is rendered as an in-app panel in the main window (not a separate
// OS window): a second WebviewWindow that loads the SPA index.html renders
// blank in production builds — a known Tauri v2 limitation
// (tauri-apps/tauri#14177).

// ----- LAN sync ----------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInfoDto {
    pub device_id: String,
    pub device_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownDeviceDto {
    pub device_id: String,
    pub name: String,
    pub addr: String,
    pub last_synced_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResultDto {
    pub snippets_added: usize,
    pub snippets_updated: usize,
    pub clips_added: usize,
}

/// This machine's sync identity and friendly name.
#[tauri::command]
pub fn sync_info(state: State<AppState>) -> Result<SyncInfoDto, String> {
    let s = state.settings.lock().map_err(err)?;
    Ok(SyncInfoDto {
        device_id: s.device_id.clone(),
        device_name: s.device_name.clone(),
    })
}

/// Rename this machine (the name peers see). Persists immediately.
#[tauri::command]
pub fn set_device_name(state: State<AppState>, name: String) -> Result<SyncInfoDto, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Device name cannot be empty".into());
    }
    let mut s = state.settings.lock().map_err(err)?;
    s.device_name = name.to_string();
    s.save(&state.config_path).map_err(err)?;
    Ok(SyncInfoDto {
        device_id: s.device_id.clone(),
        device_name: s.device_name.clone(),
    })
}

/// Scan the local network for other DevClip instances (~1.2s). Runs on the
/// blocking pool so the UI stays responsive while it waits for replies.
#[tauri::command]
pub async fn discover_peers(app: AppHandle) -> Result<Vec<crate::sync::Peer>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::sync::discover(&app, 1200))
        .await
        .map_err(err)?
}

/// Sync with a peer at "ip:port"; returns what changed on this machine. Runs on
/// the blocking pool — a slow or unreachable peer must not freeze the webview.
#[tauri::command]
pub async fn sync_with_peer(app: AppHandle, addr: String) -> Result<SyncResultDto, String> {
    let stats = tauri::async_runtime::spawn_blocking(move || crate::sync::sync_with(&app, &addr))
        .await
        .map_err(err)??;
    Ok(SyncResultDto {
        snippets_added: stats.snippets_added,
        snippets_updated: stats.snippets_updated,
        clips_added: stats.clips_added,
    })
}

/// Devices we've synced with before (for one-tap re-sync), newest first.
#[tauri::command]
pub fn list_known_devices(state: State<AppState>) -> Result<Vec<KnownDeviceDto>, String> {
    let s = state.settings.lock().map_err(err)?;
    let mut devices: Vec<KnownDeviceDto> = s
        .known_devices
        .iter()
        .map(|d| KnownDeviceDto {
            device_id: d.device_id.clone(),
            name: d.name.clone(),
            addr: d.addr.clone(),
            last_synced_at: d.last_synced_at,
        })
        .collect();
    devices.sort_by(|a, b| b.last_synced_at.cmp(&a.last_synced_at));
    Ok(devices)
}

/// Forget a previously-synced device.
#[tauri::command]
pub fn forget_device(state: State<AppState>, device_id: String) -> Result<(), String> {
    let mut s = state.settings.lock().map_err(err)?;
    s.known_devices.retain(|d| d.device_id != device_id);
    s.save(&state.config_path).map_err(err)
}

// ----- Paste ------------------------------------------------------------------

/// Paste `content` into the previously focused application.
///
/// Copy to clipboard → hide our window → restore focus to the previous app and
/// synthesize the platform paste shortcut. On Windows we explicitly restore the
/// foreground window and inject a real Ctrl+V via `SendInput`, which is both
/// instant and reliable in apps like Discord. When a `snippet_id` is supplied
/// the use is recorded for ranking.
#[tauri::command]
pub fn paste(
    app: AppHandle,
    state: State<AppState>,
    content: String,
    snippet_id: Option<i64>,
) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;

    // Record what we're about to put on the clipboard so the monitor can
    // recognise its own output and not re-add it to history (the paste loop).
    if let Ok(mut sw) = state.last_self_write.lock() {
        *sw = Some((content.clone(), Instant::now()));
    }
    app.clipboard().write_text(content).map_err(err)?;

    if let Some(id) = snippet_id {
        let store = state.store.lock().map_err(err)?;
        store.record_use(id, now_unix()).map_err(err)?;
    }

    #[cfg(windows)]
    let prev = state.prev_hwnd.load(std::sync::atomic::Ordering::SeqCst);

    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }

    #[cfg(windows)]
    std::thread::spawn(move || winpaste::paste_into(prev));

    #[cfg(not(windows))]
    std::thread::spawn(|| {
        // Give the OS a moment to return focus to the previous app, then paste.
        std::thread::sleep(std::time::Duration::from_millis(80));
        send_paste_keystroke();
    });

    Ok(())
}

/// Synthesize Cmd+V (macOS) / Ctrl+V (Linux) into whatever is now focused.
#[cfg(not(windows))]
fn send_paste_keystroke() {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings as EnigoSettings};

    let mut enigo = match Enigo::new(&EnigoSettings::default()) {
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

/// Windows focus-restore + paste. Capturing the previous foreground window and
/// injecting a real Ctrl+V keydown is what fixes pasting into Discord and makes
/// it feel instant.
#[cfg(windows)]
pub(crate) mod winpaste {
    use std::sync::atomic::Ordering;
    use std::thread::sleep;
    use std::time::Duration;

    use windows::Win32::Foundation::{BOOL, HWND};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_CONTROL,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };

    use super::AppState;

    /// Virtual-key code for the 'V' key.
    const VK_V: u16 = 0x56;

    /// Record the currently focused (foreground) window so we can paste back
    /// into it later. Called when the palette is summoned.
    pub fn capture_foreground(state: &AppState) {
        let hwnd = unsafe { GetForegroundWindow() };
        state.prev_hwnd.store(hwnd.0 as isize, Ordering::SeqCst);
    }

    /// Restore focus to `prev_hwnd` (if any) and inject Ctrl+V.
    pub fn paste_into(prev_hwnd: isize) {
        unsafe {
            if prev_hwnd != 0 {
                let hwnd = HWND(prev_hwnd as *mut core::ffi::c_void);
                force_foreground(hwnd);
                // Spin briefly until focus actually lands (fast, but reliable).
                for _ in 0..50 {
                    if GetForegroundWindow() == hwnd {
                        break;
                    }
                    sleep(Duration::from_millis(4));
                }
            } else {
                sleep(Duration::from_millis(30));
            }
            send_ctrl_v();
        }
    }

    /// Reliably bring `hwnd` to the foreground, working around the Win32
    /// foreground lock by briefly attaching our input queue to the current
    /// foreground and target threads. Without this, the *first* paste after the
    /// palette appears is frequently refused by Windows and only the *second*
    /// one succeeds.
    unsafe fn force_foreground(hwnd: HWND) {
        let cur = GetCurrentThreadId();
        let fg = GetForegroundWindow();
        let fg_thread = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let target_thread = GetWindowThreadProcessId(hwnd, None);

        let attach_fg = fg_thread != 0 && fg_thread != cur;
        let attach_tgt = target_thread != 0 && target_thread != cur && target_thread != fg_thread;

        if attach_fg {
            let _ = AttachThreadInput(cur, fg_thread, BOOL(1));
        }
        if attach_tgt {
            let _ = AttachThreadInput(cur, target_thread, BOOL(1));
        }

        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);

        if attach_tgt {
            let _ = AttachThreadInput(cur, target_thread, BOOL(0));
        }
        if attach_fg {
            let _ = AttachThreadInput(cur, fg_thread, BOOL(0));
        }
    }

    unsafe fn key_input(vk: u16, key_up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: if key_up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    unsafe fn send_ctrl_v() {
        let inputs = [
            key_input(VK_CONTROL.0, false),
            key_input(VK_V, false),
            key_input(VK_V, true),
            key_input(VK_CONTROL.0, true),
        ];
        SendInput(&inputs, core::mem::size_of::<INPUT>() as i32);
    }
}
