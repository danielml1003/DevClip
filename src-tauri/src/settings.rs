//! Persisted application settings.
//!
//! Stored as a small JSON file in the OS config directory. The database path
//! cannot live in the database itself (chicken-and-egg), so it lives here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Absolute path to the SQLite database file.
    pub db_path: PathBuf,
    /// Maximum number of clipboard-history entries to retain.
    pub clipboard_cap: usize,
}

impl Settings {
    pub fn defaults(data_dir: &Path) -> Self {
        Settings {
            db_path: data_dir.join("devclip.db"),
            clipboard_cap: devclip_core::DEFAULT_CLIPBOARD_CAP,
        }
    }

    /// Load settings from `config_path`, falling back to sensible defaults if
    /// the file is missing or unreadable.
    pub fn load_or_default(config_path: &Path, data_dir: &Path) -> Self {
        let mut settings = std::fs::read_to_string(config_path)
            .ok()
            .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
            .unwrap_or_else(|| Settings::defaults(data_dir));

        // Guard against a corrupt/zero cap.
        if settings.clipboard_cap == 0 {
            settings.clipboard_cap = devclip_core::DEFAULT_CLIPBOARD_CAP;
        }
        if settings.db_path.as_os_str().is_empty() {
            settings.db_path = data_dir.join("devclip.db");
        }
        settings
    }

    pub fn save(&self, config_path: &Path) -> std::io::Result<()> {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .unwrap_or_else(|_| "{}".to_string());
        std::fs::write(config_path, json)
    }
}
