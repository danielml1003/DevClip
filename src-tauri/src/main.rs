// Prevent an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod settings;
mod sync;
#[cfg(windows)]
mod winpos;

use std::sync::Mutex;
use std::time::Duration;

use commands::AppState;
use devclip_core::{now_unix, Store, DEFAULT_CLIPBOARD_CAP};
use settings::Settings;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};
#[cfg(not(windows))]
use tauri::PhysicalSize;
use tauri_plugin_global_shortcut::{
    Builder as ShortcutBuilder, Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};

/// How often the background thread samples the OS clipboard.
const CLIPBOARD_POLL: Duration = Duration::from_millis(700);
/// How long a tool-written clipboard value is treated as "our own output" and
/// suppressed from history. Generous enough to survive the async OS clipboard
/// event, short enough not to swallow a genuine later re-copy of the same text.
const SELF_WRITE_TTL: Duration = Duration::from_secs(5);

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            ShortcutBuilder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_window(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            // Resolve the default locations and load persisted settings.
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&config_dir)?;
            let config_path = config_dir.join("settings.json");
            let default_db_path = data_dir.join("devclip.db");

            let (settings, changed) = Settings::load_or_default(&config_path, &data_dir);
            // Persist a first-run device id / name so it stays stable.
            if changed {
                let _ = settings.save(&config_path);
            }
            if let Some(parent) = settings.db_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let store = Store::open(&settings.db_path)
                .map_err(|e| format!("failed to open database: {e}"))?;

            app.manage(AppState {
                store: Mutex::new(store),
                settings: Mutex::new(settings),
                config_path,
                default_db_path,
                last_self_write: Mutex::new(None),
                #[cfg(windows)]
                prev_hwnd: std::sync::atomic::AtomicIsize::new(0),
            });

            // Default global hotkey: Alt+Space (chosen to avoid clobbering
            // Windows' native Win+V history popup).
            let shortcut = Shortcut::new(Some(Modifiers::ALT), Code::Space);
            app.global_shortcut().register(shortcut)?;

            start_clipboard_monitor(app.handle().clone());
            // LAN sync discovery responder + sync server (best-effort; never
            // fatal if a port is busy).
            sync::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Raycast-style: dismiss the *palette* when it loses focus. The
            // settings window must NOT auto-hide, so guard on the label.
            if window.label() == "main" {
                if let tauri::WindowEvent::Focused(false) = event {
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::search_all,
            commands::save_snippet,
            commands::delete_snippet,
            commands::list_clipboard,
            commands::current_clipboard,
            commands::paste,
            commands::get_settings,
            commands::set_settings,
            commands::sync_info,
            commands::set_device_name,
            commands::discover_peers,
            commands::sync_with_peer,
            commands::list_known_devices,
            commands::forget_device,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DevClip");
}

/// Show the palette if hidden, hide it if visible.
fn toggle_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        // Everything that inspects the *currently* focused app must happen
        // before we show/focus our own window.
        let pos = position_window(app, &win);
        let _ = win.show();
        let _ = win.set_focus();
        // Re-assert the position after showing: on Windows, set_position before
        // the first show is sometimes overridden by the OS's default placement,
        // which can land the window on the wrong monitor.
        #[cfg(windows)]
        if let Some(p) = pos {
            let _ = win.set_position(p);
        }
        #[cfg(not(windows))]
        let _ = pos;
        // Tell the frontend to reset to a clean, focused search state.
        let _ = app.emit("devclip://show", ());
    }
}

/// Place the palette before showing it; returns the position it set (if any).
///
/// On Windows: remember the focused app (for paste-back) and open centered on
/// the active app's monitor. On other platforms: open at the mouse cursor.
fn position_window(app: &AppHandle, win: &WebviewWindow) -> Option<PhysicalPosition<i32>> {
    #[cfg(windows)]
    {
        if let Some(state) = app.try_state::<AppState>() {
            commands::winpaste::capture_foreground(state.inner());
        }
        // Logical window size from tauri.conf.json; winpos scales it to the
        // active monitor's DPI internally.
        match winpos::active_screen_origin((720, 480)) {
            // Safety net: only trust the point if it lands on a real monitor in
            // Tauri's own coordinate space (the space set_position uses).
            Some((x, y)) if point_on_a_monitor(win, x, y) => {
                let p = PhysicalPosition::new(x, y);
                let _ = win.set_position(p);
                Some(p)
            }
            _ => {
                let _ = win.center();
                None
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = app; // unused off Windows
        position_at_cursor(win)
    }
}

/// True if `(x, y)` (physical px) falls within any connected monitor, per
/// Tauri's monitor list — the authoritative coordinate space for `set_position`.
#[cfg(windows)]
fn point_on_a_monitor(win: &WebviewWindow, x: i32, y: i32) -> bool {
    win.available_monitors()
        .map(|mons| {
            mons.iter().any(|m| {
                let p = m.position();
                let s = m.size();
                x >= p.x && x < p.x + s.width as i32 && y >= p.y && y < p.y + s.height as i32
            })
        })
        .unwrap_or(false)
}

/// Position the palette near the mouse cursor, clipped to the bounds of the
/// monitor the cursor is on (so it always appears on the active screen).
/// Used on macOS/Linux; Windows anchors to the text caret instead.
#[cfg(not(windows))]
fn position_at_cursor(win: &WebviewWindow) -> Option<PhysicalPosition<i32>> {
    let Ok(cursor) = win.cursor_position() else {
        let _ = win.center();
        return None;
    };

    // Find the monitor under the cursor; fall back to current/primary.
    let monitor = win
        .available_monitors()
        .ok()
        .and_then(|mons| {
            mons.into_iter().find(|m| {
                let p = m.position();
                let s = m.size();
                let cx = cursor.x as i32;
                let cy = cursor.y as i32;
                cx >= p.x
                    && cx < p.x + s.width as i32
                    && cy >= p.y
                    && cy < p.y + s.height as i32
            })
        })
        .or_else(|| win.current_monitor().ok().flatten())
        .or_else(|| win.primary_monitor().ok().flatten());

    let Some(monitor) = monitor else {
        let _ = win.center();
        return None;
    };

    let mpos = monitor.position();
    let msize = monitor.size();
    let wsize = win
        .outer_size()
        .unwrap_or(PhysicalSize::new(720, 480));

    // Appear just below-right of the cursor/caret, then clip to the monitor.
    let mut x = cursor.x as i32 + 8;
    let mut y = cursor.y as i32 + 16;

    let max_x = mpos.x + msize.width as i32 - wsize.width as i32;
    let max_y = mpos.y + msize.height as i32 - wsize.height as i32;
    x = x.min(max_x).max(mpos.x);
    y = y.min(max_y).max(mpos.y);

    let p = PhysicalPosition::new(x, y);
    let _ = win.set_position(p);
    Some(p)
}

/// Poll the OS clipboard and append changes to the (capped) history.
fn start_clipboard_monitor(app: AppHandle) {
    use tauri_plugin_clipboard_manager::ClipboardExt;

    std::thread::spawn(move || {
        let mut last = String::new();
        loop {
            std::thread::sleep(CLIPBOARD_POLL);
            let Ok(text) = app.clipboard().read_text() else {
                continue;
            };
            if text == last || text.trim().is_empty() {
                continue;
            }
            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };

            // Echo suppression: if this value is one the tool just wrote (for a
            // paste), it's our own output, not a user copy — adopt it as the
            // baseline but don't record it (prevents the paste-to-top loop).
            let is_echo = if let Ok(mut sw) = state.last_self_write.lock() {
                match sw.as_ref() {
                    Some((val, t)) if *val == text && t.elapsed() < SELF_WRITE_TTL => {
                        *sw = None;
                        true
                    }
                    Some((_, t)) if t.elapsed() >= SELF_WRITE_TTL => {
                        *sw = None;
                        false
                    }
                    _ => false,
                }
            } else {
                false
            };

            last = text.clone();
            if is_echo {
                continue;
            }

            let cap = state
                .settings
                .lock()
                .map(|s| s.clipboard_cap)
                .unwrap_or(DEFAULT_CLIPBOARD_CAP);
            // Named binding (declared after `state`) so the guard drops first.
            let store = state.store.lock();
            if let Ok(store) = store {
                let _ = store.add_clipboard(&text, now_unix(), cap);
            }
        }
    });
}
