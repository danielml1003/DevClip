//! Persisted application settings.
//!
//! Stored as a small JSON file in the OS config directory. The database path
//! cannot live in the database itself (chicken-and-egg), so it lives here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A device we've synced with before, so the user can re-sync with one tap
/// without re-scanning the network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownDevice {
    /// Stable identity of the peer (its `Settings::device_id`).
    pub device_id: String,
    /// Friendly name shown in the UI.
    pub name: String,
    /// Last address its TCP sync server was reachable at ("ip:port"). Tried
    /// first on a one-tap re-sync; if it fails the user can re-scan.
    pub addr: String,
    /// Unix seconds of the last successful sync with this device.
    pub last_synced_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Absolute path to the SQLite database file.
    pub db_path: PathBuf,
    /// Maximum number of clipboard-history entries to retain.
    pub clipboard_cap: usize,
    /// This machine's stable identity for LAN sync (a UUID). Generated once.
    #[serde(default)]
    pub device_id: String,
    /// Friendly name other machines show for this one (defaults to hostname).
    #[serde(default)]
    pub device_name: String,
    /// Devices previously synced with, for one-tap re-sync.
    #[serde(default)]
    pub known_devices: Vec<KnownDevice>,
    /// Whether LAN sync is enabled. OFF by default so a fresh install never
    /// binds a network socket (and never triggers the OS firewall prompt) until
    /// the user explicitly opts in.
    #[serde(default)]
    pub sync_enabled: bool,
}

/// Best-effort hostname for a friendly default device name.
fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "DevClip device".to_string())
}

impl Settings {
    pub fn defaults(data_dir: &Path) -> Self {
        Settings {
            db_path: data_dir.join("devclip.db"),
            clipboard_cap: devclip_core::DEFAULT_CLIPBOARD_CAP,
            device_id: devclip_core::new_sync_id(),
            device_name: default_device_name(),
            known_devices: Vec::new(),
            sync_enabled: false,
        }
    }

    /// Load settings from `config_path`, falling back to sensible defaults if
    /// the file is missing or unreadable. Returns `(settings, changed)` where
    /// `changed` is true if a missing field (e.g. a first-run device id) was
    /// backfilled and the caller should persist the result.
    pub fn load_or_default(config_path: &Path, data_dir: &Path) -> (Self, bool) {
        let loaded = std::fs::read_to_string(config_path)
            .ok()
            .and_then(|s| serde_json::from_str::<Settings>(&s).ok());
        let mut changed = loaded.is_none();
        let mut settings = loaded.unwrap_or_else(|| Settings::defaults(data_dir));

        // Guard against a corrupt/zero cap.
        if settings.clipboard_cap == 0 {
            settings.clipboard_cap = devclip_core::DEFAULT_CLIPBOARD_CAP;
        }
        if settings.db_path.as_os_str().is_empty() {
            settings.db_path = data_dir.join("devclip.db");
        }
        // Backfill sync identity for databases created before LAN sync existed.
        if settings.device_id.trim().is_empty() {
            settings.device_id = devclip_core::new_sync_id();
            changed = true;
        }
        if settings.device_name.trim().is_empty() {
            settings.device_name = default_device_name();
            changed = true;
        }
        (settings, changed)
    }

    /// Record (or refresh) a device we just synced with.
    pub fn remember_device(&mut self, device_id: &str, name: &str, addr: &str, now: i64) {
        if let Some(d) = self
            .known_devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
        {
            d.name = name.to_string();
            d.addr = addr.to_string();
            d.last_synced_at = now;
        } else {
            self.known_devices.push(KnownDevice {
                device_id: device_id.to_string(),
                name: name.to_string(),
                addr: addr.to_string(),
                last_synced_at: now,
            });
        }
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
