// Prevent an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::sync::Mutex;
use std::time::Duration;

use commands::AppState;
use devclip_core::{now_unix, Store, DEFAULT_CLIPBOARD_CAP};
use tauri::{AppHandle, Emitter, Manager};
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
            // Open the local SQLite database in the OS app-data directory.
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = Store::open(dir.join("devclip.db"))
                .map_err(|e| format!("failed to open database: {e}"))?;
            app.manage(AppState {
                store: Mutex::new(store),
            });

            // Default global hotkey: Alt+Space (configurable in a future version;
            // chosen to avoid clobbering Windows' native Win+V history popup).
            let shortcut = Shortcut::new(Some(Modifiers::ALT), Code::Space);
            app.global_shortcut().register(shortcut)?;

            start_clipboard_monitor(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            // Raycast-style: dismiss the palette when it loses focus.
            if let tauri::WindowEvent::Focused(false) = event {
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::save_snippet,
            commands::delete_snippet,
            commands::list_clipboard,
            commands::current_clipboard,
            commands::paste,
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
        let _ = win.center();
        let _ = win.show();
        let _ = win.set_focus();
        // Tell the frontend to reset to a clean, focused search state.
        let _ = app.emit("devclip://show", ());
    }
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
                if let Ok(store) = state.store.lock() {
                    let _ = store.add_clipboard(&text, now_unix(), DEFAULT_CLIPBOARD_CAP);
                }
            }
        }
    });
}
