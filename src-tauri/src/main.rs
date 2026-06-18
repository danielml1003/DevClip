// Prevent an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod settings;

use std::sync::Mutex;
use std::time::Duration;

use commands::AppState;
use devclip_core::{now_unix, Store, DEFAULT_CLIPBOARD_CAP};
use settings::Settings;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use tauri_plugin_global_shortcut::{
    Builder as ShortcutBuilder, Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};

/// How often the background thread samples the OS clipboard.
const CLIPBOARD_POLL: Duration = Duration::from_millis(700);

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

            let settings = Settings::load_or_default(&config_path, &data_dir);
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
                #[cfg(windows)]
                prev_hwnd: std::sync::atomic::AtomicIsize::new(0),
            });

            // Default global hotkey: Alt+Space (chosen to avoid clobbering
            // Windows' native Win+V history popup).
            let shortcut = Shortcut::new(Some(Modifiers::ALT), Code::Space);
            app.global_shortcut().register(shortcut)?;

            start_clipboard_monitor(app.handle().clone());
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
            commands::open_settings,
            commands::close_settings,
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
        // Remember the app that's focused right now so we can paste back into
        // it (Windows), then position the palette at the cursor.
        #[cfg(windows)]
        if let Some(state) = app.try_state::<AppState>() {
            commands::winpaste::capture_foreground(state.inner());
        }
        position_at_cursor(&win);
        let _ = win.show();
        let _ = win.set_focus();
        // Tell the frontend to reset to a clean, focused search state.
        let _ = app.emit("devclip://show", ());
    }
}

/// Position the palette near the mouse cursor, clipped to the bounds of the
/// monitor the cursor is on (so it always appears on the active screen).
fn position_at_cursor(win: &WebviewWindow) {
    let Ok(cursor) = win.cursor_position() else {
        let _ = win.center();
        return;
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
        return;
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

    let _ = win.set_position(PhysicalPosition::new(x, y));
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
            last = text.clone();
            if let Some(state) = app.try_state::<AppState>() {
                let cap = state
                    .settings
                    .lock()
                    .map(|s| s.clipboard_cap)
                    .unwrap_or(DEFAULT_CLIPBOARD_CAP);
                if let Ok(store) = state.store.lock() {
                    let _ = store.add_clipboard(&text, now_unix(), cap);
                }
            }
        }
    });
}
