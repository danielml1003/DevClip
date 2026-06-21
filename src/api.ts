// Thin API layer over Tauri IPC.
//
// When running inside the Tauri webview we call real Rust commands. When the
// page is opened in a plain browser (`npm run dev` without Tauri) we fall back
// to an in-memory mock seeded with sample data, so the UI can be developed and
// demoed standalone.

import type {
  AppSettings,
  ClipboardEntry,
  Mode,
  SearchResult,
  Snippet,
  UnifiedResult,
} from "./types";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
type ListenFn = (event: string, handler: () => void) => Promise<() => void>;

let invoke: InvokeFn = async () => {
  throw new Error("Tauri invoke unavailable");
};
let listenImpl: ListenFn = async () => async () => {};
let hideWindow: () => Promise<void> = async () => {};
let currentLabel = "main";

if (isTauri) {
  // Dynamic imports so the mock build doesn't require the packages at runtime.
  const core = await import("@tauri-apps/api/core");
  const event = await import("@tauri-apps/api/event");
  const win = await import("@tauri-apps/api/window");
  invoke = core.invoke as InvokeFn;
  listenImpl = ((name, handler) =>
    event.listen(name, () => handler())) as ListenFn;
  const current = win.getCurrentWindow();
  currentLabel = current.label;
  hideWindow = async () => {
    await current.hide();
  };
}

/** Which window this script is running in ("main" or "settings"). */
export function windowLabel(): string {
  return currentLabel;
}

// ----- Public API ------------------------------------------------------------

export const api = {
  /** Search snippets ONLY (the Snippets view). */
  search(query: string): Promise<SearchResult[]> {
    return isTauri ? invoke("search", { query }) : mock.search(query);
  },

  /** Unified search across snippets AND clipboard (the default view). */
  searchAll(query: string): Promise<UnifiedResult[]> {
    return isTauri ? invoke("search_all", { query }) : mock.searchAll(query);
  },

  saveSnippet(name: string, content: string): Promise<Snippet> {
    return isTauri
      ? invoke("save_snippet", { name, content })
      : mock.saveSnippet(name, content);
  },

  deleteSnippet(id: number): Promise<void> {
    return isTauri ? invoke("delete_snippet", { id }) : mock.deleteSnippet(id);
  },

  listClipboard(): Promise<ClipboardEntry[]> {
    return isTauri ? invoke("list_clipboard") : mock.listClipboard();
  },

  /** Read the *current* OS clipboard (for the Save flow). */
  currentClipboard(): Promise<string> {
    return isTauri ? invoke("current_clipboard") : mock.currentClipboard();
  },

  /**
   * Paste `content` into the previously focused application. When `snippetId`
   * is provided the backend also records the use (for ranking).
   */
  paste(content: string, snippetId?: number): Promise<void> {
    return isTauri
      ? invoke("paste", { content, snippetId: snippetId ?? null })
      : mock.paste(content, snippetId);
  },

  getSettings(): Promise<AppSettings> {
    return isTauri ? invoke("get_settings") : mock.getSettings();
  },

  setSettings(dbPath: string, clipboardCap: number): Promise<AppSettings> {
    return isTauri
      ? invoke("set_settings", { dbPath, clipboardCap })
      : mock.setSettings(dbPath, clipboardCap);
  },

  openSettings(): Promise<void> {
    return isTauri ? invoke("open_settings") : mock.openSettings();
  },

  closeSettings(): Promise<void> {
    return isTauri ? invoke("close_settings") : mock.closeSettings();
  },

  hide(): Promise<void> {
    return isTauri ? hideWindow() : mock.hide();
  },

  /** Subscribe to the backend's "window was shown" event. Returns unlisten. */
  onShow(handler: () => void): Promise<() => void> {
    return isTauri ? listenImpl("devclip://show", handler) : mock.onShow(handler);
  },
};

export { isTauri };
export type { Mode };

// ----- Mock backend (browser dev only) --------------------------------------

const mock = (() => {
  let nextId = 100;
  let snippets: Snippet[] = [
    s(1, "Production migration command", "docker compose exec api python manage.py migrate", 6, -3600),
    s(2, "Local docker compose up", "docker compose up -d", 12, -120),
    s(3, "Prod SSH", "ssh deploy@prod.example.com", 3, -86400),
    s(4, "Tail k8s logs", "kubectl logs -f deploy/api -n production", 2, -7200),
    s(5, "Find big files", "find . -type f -size +100M -exec ls -lh {} \\;", 1, -200000),
    s(6, "JWT decode (jq)", "cut -d. -f2 <<< \"$JWT\" | base64 -d | jq .", 0, null),
    s(7, "Reset local DB", "dropdb app_dev && createdb app_dev && npm run migrate", 4, -50000),
  ];
  let clipboard: ClipboardEntry[] = [
    { id: 90, content: "git rebase -i HEAD~3", createdAt: now() - 30 },
    { id: 89, content: "SELECT * FROM users WHERE last_login > now() - interval '7 days';", createdAt: now() - 300 },
    { id: 88, content: "https://github.com/danielml1003/devclip", createdAt: now() - 900 },
    { id: 87, content: "docker compose restart api", createdAt: now() - 1500 },
  ];
  let settings: AppSettings = {
    dbPath: "C:\\Users\\you\\AppData\\Roaming\\dev.devclip.app\\devclip.db",
    clipboardCap: 100,
    defaultDbPath: "C:\\Users\\you\\AppData\\Roaming\\dev.devclip.app\\devclip.db",
  };

  function s(
    id: number,
    name: string,
    content: string,
    useCount: number,
    lastUsedDelta: number | null,
  ): Snippet {
    return {
      id,
      name,
      content,
      createdAt: now() - 1_000_000,
      lastUsedAt: lastUsedDelta === null ? null : now() + lastUsedDelta,
      useCount,
    };
  }

  function now(): number {
    return Math.floor(Date.now() / 1000);
  }

  // A small fuzzy matcher mirroring the Rust heuristics closely enough for dev.
  function fuzzy(query: string, text: string): { score: number; indices: number[] } | null {
    if (!query) return { score: 0, indices: [] };
    const q = query.toLowerCase();
    const t = text.toLowerCase();
    const sub = t.indexOf(q);
    if (sub >= 0) {
      const indices = Array.from({ length: q.length }, (_, i) => sub + i);
      let score = 40 + q.length * 16 + (q.length - 1) * 18;
      score += sub === 0 ? 54 : isBoundary(text, sub) ? 30 : 0;
      return { score, indices };
    }
    const indices: number[] = [];
    let ti = 0;
    for (const qc of q) {
      let found = -1;
      while (ti < t.length) {
        if (t[ti] === qc) {
          found = ti;
          ti++;
          break;
        }
        ti++;
      }
      if (found < 0) return null;
      indices.push(found);
    }
    let score = -Math.min(indices[0], 30) * 3;
    let prev = -2;
    for (const idx of indices) {
      score += 16;
      if (idx === 0) score += 54;
      else if (isBoundary(text, idx)) score += 30;
      if (idx === prev + 1) score += 18;
      prev = idx;
    }
    return { score, indices };
  }

  function isBoundary(text: string, i: number): boolean {
    if (i === 0) return true;
    const prev = text[i - 1];
    const cur = text[i];
    if (!/[a-z0-9]/i.test(prev)) return true;
    if (cur === cur.toUpperCase() && prev === prev.toLowerCase() && /[a-z]/i.test(cur)) return true;
    return false;
  }

  function usageBoost(snip: Snippet): number {
    const freq = Math.log1p(Math.max(0, snip.useCount)) * 12;
    const recency = snip.lastUsedAt
      ? 40 * Math.pow(0.5, Math.max(0, now() - snip.lastUsedAt) / (7 * 24 * 3600))
      : 0;
    return freq + recency;
  }

  function scoreSnippet(query: string, snip: Snippet): SearchResult | null {
    const terms = query.split(/\s+/).filter(Boolean);
    if (terms.length === 0) {
      return { snippet: snip, score: Math.round(usageBoost(snip)), nameIndices: [], contentIndices: [] };
    }
    let total = 0;
    let nameIdx: number[] = [];
    let contentIdx: number[] = [];
    for (const term of terms) {
      const nm = fuzzy(term, snip.name);
      const cm = fuzzy(term, snip.content);
      if (!nm && !cm) return null;
      total += Math.max(nm ? nm.score * 1.6 : -Infinity, cm ? cm.score * 1.0 : -Infinity);
      if (nm) nameIdx = nameIdx.concat(nm.indices);
      if (cm) contentIdx = contentIdx.concat(cm.indices);
    }
    total += usageBoost(snip);
    return {
      snippet: snip,
      score: Math.round(total),
      nameIndices: dedupeSorted(nameIdx),
      contentIndices: dedupeSorted(contentIdx),
    };
  }

  return {
    async search(query: string): Promise<SearchResult[]> {
      const out = snippets
        .map((snip) => scoreSnippet(query, snip))
        .filter((r): r is SearchResult => r !== null);
      out.sort((a, b) => b.score - a.score || (b.snippet.lastUsedAt ?? 0) - (a.snippet.lastUsedAt ?? 0));
      return out;
    },
    async searchAll(query: string): Promise<UnifiedResult[]> {
      if (!query.trim()) return [];
      const out: UnifiedResult[] = [];
      const snippetContents = new Set<string>();
      for (const snip of snippets) {
        const r = scoreSnippet(query, snip);
        if (!r) continue;
        snippetContents.add(snip.content);
        out.push({
          kind: "snippet",
          id: snip.id,
          title: snip.name,
          content: snip.content,
          score: r.score,
          titleIndices: r.nameIndices,
          contentIndices: r.contentIndices,
          createdAt: snip.createdAt,
          lastUsedAt: snip.lastUsedAt,
          useCount: snip.useCount,
        });
      }
      const terms = query.split(/\s+/).filter(Boolean);
      for (const e of clipboard) {
        if (snippetContents.has(e.content)) continue;
        let total = 0;
        let idx: number[] = [];
        let ok = true;
        for (const term of terms) {
          const m = fuzzy(term, e.content);
          if (!m) {
            ok = false;
            break;
          }
          total += m.score;
          idx = idx.concat(m.indices);
        }
        if (!ok) continue;
        total += 30 * Math.pow(0.5, Math.max(0, now() - e.createdAt) / 86400);
        const indices = dedupeSorted(idx);
        out.push({
          kind: "clipboard",
          id: e.id,
          title: e.content,
          content: e.content,
          score: Math.round(total),
          titleIndices: indices,
          contentIndices: indices,
          createdAt: e.createdAt,
          lastUsedAt: null,
          useCount: 0,
        });
      }
      out.sort((a, b) => b.score - a.score || b.createdAt - a.createdAt);
      return out;
    },
    async saveSnippet(name: string, content: string): Promise<Snippet> {
      const snip = s(nextId++, name, content, 0, null);
      snippets = [snip, ...snippets];
      return snip;
    },
    async deleteSnippet(id: number): Promise<void> {
      snippets = snippets.filter((x) => x.id !== id);
    },
    async listClipboard(): Promise<ClipboardEntry[]> {
      return clipboard;
    },
    async currentClipboard(): Promise<string> {
      return clipboard[0]?.content ?? "";
    },
    async paste(content: string, snippetId?: number): Promise<void> {
      // eslint-disable-next-line no-console
      console.info("[mock] paste", { snippetId, content });
    },
    async getSettings(): Promise<AppSettings> {
      return { ...settings };
    },
    async setSettings(dbPath: string, clipboardCap: number): Promise<AppSettings> {
      settings = { ...settings, dbPath, clipboardCap: Math.max(1, Math.min(100000, clipboardCap)) };
      return { ...settings };
    },
    async openSettings(): Promise<void> {
      location.hash = "settings";
      location.reload();
    },
    async closeSettings(): Promise<void> {
      location.hash = "";
      location.reload();
    },
    async hide(): Promise<void> {
      // eslint-disable-next-line no-console
      console.info("[mock] hide window");
    },
    async onShow(_handler: () => void): Promise<() => void> {
      return () => {};
    },
  };

  function dedupeSorted(xs: number[]): number[] {
    return Array.from(new Set(xs)).sort((a, b) => a - b);
  }
})();
