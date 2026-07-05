// Types mirroring the Rust DTOs returned over Tauri IPC (camelCase).

export interface Snippet {
  id: number;
  name: string;
  content: string;
  createdAt: number;
  lastUsedAt: number | null;
  useCount: number;
}

export interface SearchResult {
  snippet: Snippet;
  score: number;
  /** Matched char indices in `snippet.name`, for highlighting. */
  nameIndices: number[];
  /** Matched char indices in `snippet.content`, for highlighting. */
  contentIndices: number[];
}

export interface ClipboardEntry {
  id: number;
  content: string;
  createdAt: number;
}

export type ItemKind = "snippet" | "clipboard";

/** A row in the unified (default Clipboard view) search across both sources. */
export interface UnifiedResult {
  kind: ItemKind;
  id: number;
  title: string;
  content: string;
  score: number;
  titleIndices: number[];
  contentIndices: number[];
  createdAt: number;
  lastUsedAt: number | null;
  useCount: number;
}

export interface AppSettings {
  dbPath: string;
  clipboardCap: number;
  defaultDbPath: string;
}

export type Mode = "clipboard" | "snippets";

// ----- LAN sync ---------------------------------------------------------------

/** This machine's sync identity and current on/off state. */
export interface SyncStatus {
  deviceId: string;
  deviceName: string;
  /** The user's persisted choice. */
  enabled: boolean;
  /** Whether the listener sockets are actually bound right now. */
  running: boolean;
}

/** A DevClip instance found on the local network during a scan. */
export interface Peer {
  deviceId: string;
  deviceName: string;
  /** "ip:port" of the peer's sync server. */
  addr: string;
}

/** A device we've synced with before (for one-tap re-sync). */
export interface KnownDevice {
  deviceId: string;
  name: string;
  addr: string;
  lastSyncedAt: number;
}

/** What a sync changed on this machine. */
export interface SyncResult {
  snippetsAdded: number;
  snippetsUpdated: number;
  clipsAdded: number;
}
