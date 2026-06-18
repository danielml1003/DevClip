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

export type Mode = "snippets" | "clipboard";
