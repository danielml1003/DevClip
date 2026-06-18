import { api } from "./api";
import type { ClipboardEntry, ItemKind, Mode, SearchResult, UnifiedResult } from "./types";
import "./styles.css";

// ----- Element handles -------------------------------------------------------

const $ = <T extends HTMLElement>(sel: string): T => {
  const el = document.querySelector<T>(sel);
  if (!el) throw new Error(`missing element: ${sel}`);
  return el;
};

// ----- Row model -------------------------------------------------------------

/** A normalised, render-ready row (snippet or clipboard entry). */
interface Row {
  kind: ItemKind;
  id: number;
  /** Full content to paste. */
  content: string;
  title: string;
  titleIndices: number[];
  subtitle: string;
  subtitleIndices: number[];
  meta: string;
}

// ----- Shared helpers --------------------------------------------------------

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** Render `text` with the characters at `indices` wrapped in <mark>. */
function highlight(text: string, indices: number[]): string {
  if (indices.length === 0) return escapeHtml(text);
  const set = new Set(indices);
  let out = "";
  let run = "";
  let inMark = false;
  const chars = Array.from(text);
  for (let i = 0; i < chars.length; i++) {
    const hit = set.has(i);
    if (hit !== inMark) {
      out += inMark ? `<mark>${escapeHtml(run)}</mark>` : escapeHtml(run);
      run = "";
      inMark = hit;
    }
    run += chars[i];
  }
  out += inMark ? `<mark>${escapeHtml(run)}</mark>` : escapeHtml(run);
  return out;
}

function firstLine(s: string, max = 120): string {
  const line = s.split("\n")[0].trim();
  return line.length > max ? line.slice(0, max) + "…" : line;
}

function suggestName(content: string): string {
  return firstLine(content, 60);
}

function timeAgo(unix: number): string {
  const s = Math.max(0, Math.floor(Date.now() / 1000) - unix);
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

// =============================================================================
// Palette
// =============================================================================

function initPalette(): void {
  const searchInput = $<HTMLInputElement>("#search");
  const resultsEl = $<HTMLUListElement>("#results");
  const emptyEl = $<HTMLDivElement>("#empty");
  const footerEl = $<HTMLDivElement>("#footer");
  const tabsEl = $<HTMLDivElement>("#mode-tabs");

  const saveOverlay = $<HTMLDivElement>("#save-overlay");
  const saveName = $<HTMLInputElement>("#save-name");
  const saveContent = $<HTMLTextAreaElement>("#save-content");
  const saveConfirm = $<HTMLButtonElement>("#save-confirm");

  interface State {
    mode: Mode;
    selected: number;
    rows: Row[];
    /** Whether the current list mixes snippets and clipboard (show badges). */
    unified: boolean;
    saving: boolean;
  }

  const state: State = {
    mode: "clipboard", // Clipboard is the default view.
    selected: 0,
    rows: [],
    unified: false,
    saving: false,
  };

  // ----- Row builders --------------------------------------------------------

  function snippetRow(r: SearchResult): Row {
    return {
      kind: "snippet",
      id: r.snippet.id,
      content: r.snippet.content,
      title: r.snippet.name,
      titleIndices: r.nameIndices,
      subtitle: firstLine(r.snippet.content),
      subtitleIndices: r.contentIndices,
      meta: r.snippet.useCount > 0 ? `${r.snippet.useCount}×` : "",
    };
  }

  function clipboardRow(e: ClipboardEntry): Row {
    return {
      kind: "clipboard",
      id: e.id,
      content: e.content,
      title: e.content,
      titleIndices: [],
      subtitle: timeAgo(e.createdAt),
      subtitleIndices: [],
      meta: "⌘S",
    };
  }

  function unifiedRow(r: UnifiedResult): Row {
    if (r.kind === "snippet") {
      return {
        kind: "snippet",
        id: r.id,
        content: r.content,
        title: r.title,
        titleIndices: r.titleIndices,
        subtitle: firstLine(r.content),
        subtitleIndices: r.contentIndices,
        meta: r.useCount > 0 ? `${r.useCount}×` : "",
      };
    }
    return {
      kind: "clipboard",
      id: r.id,
      content: r.content,
      title: r.title,
      titleIndices: r.titleIndices,
      subtitle: timeAgo(r.createdAt),
      subtitleIndices: [],
      meta: "",
    };
  }

  // ----- Data ----------------------------------------------------------------

  async function runSearch(): Promise<void> {
    const query = searchInput.value;
    const q = query.trim();

    if (state.mode === "snippets") {
      // Snippets view searches snippets ONLY (never the clipboard).
      state.unified = false;
      const results = await api.search(query);
      state.rows = results.map(snippetRow);
    } else if (q === "") {
      // Default Clipboard view, no query → plain clipboard history.
      state.unified = false;
      const all = await api.listClipboard();
      state.rows = all.map(clipboardRow);
    } else {
      // Default view with a query → search BOTH clipboard and snippets.
      state.unified = true;
      const results = await api.searchAll(query);
      state.rows = results.map(unifiedRow);
    }

    state.selected = 0;
    render();
  }

  // ----- Rendering -----------------------------------------------------------

  function render(): void {
    tabsEl.querySelectorAll<HTMLButtonElement>(".tab").forEach((btn) => {
      btn.classList.toggle("active", btn.dataset.mode === state.mode);
    });

    searchInput.placeholder =
      state.mode === "clipboard" ? "Search clipboard and snippets…" : "Search snippets…";

    const n = state.rows.length;
    state.selected = n === 0 ? 0 : Math.max(0, Math.min(state.selected, n - 1));

    resultsEl.innerHTML = "";

    if (n === 0) {
      emptyEl.classList.remove("hidden");
      emptyEl.innerHTML = emptyMessage();
    } else {
      emptyEl.classList.add("hidden");
      state.rows.forEach((row, i) => {
        const li = renderRow(row);
        li.classList.toggle("selected", i === state.selected);
        li.addEventListener("mousemove", () => {
          if (state.selected !== i) {
            state.selected = i;
            render();
          }
        });
        li.addEventListener("click", () => {
          state.selected = i;
          void activate();
        });
        resultsEl.appendChild(li);
      });
      const sel = resultsEl.children[state.selected] as HTMLElement | undefined;
      sel?.scrollIntoView({ block: "nearest" });
    }

    renderFooter();
  }

  function renderRow(row: Row): HTMLLIElement {
    const li = document.createElement("li");
    li.className = "row";
    li.setAttribute("role", "option");
    const badge = state.unified
      ? `<span class="badge ${row.kind}">${row.kind === "snippet" ? "snippet" : "clip"}</span>`
      : "";
    const titleClass = row.kind === "clipboard" ? "row-title mono" : "row-title";
    li.innerHTML = `
      <div class="row-main">
        <div class="${titleClass}">${badge}${highlight(firstLine(row.title), row.titleIndices)}</div>
        <div class="row-sub${row.kind === "clipboard" ? " dim" : ""}">${highlight(row.subtitle, row.subtitleIndices)}</div>
      </div>
      <div class="row-meta">${escapeHtml(row.meta)}</div>`;
    return li;
  }

  function emptyMessage(): string {
    if (state.mode === "snippets") {
      return searchInput.value
        ? `No snippets match <strong>${escapeHtml(searchInput.value)}</strong>.`
        : `No snippets yet.<br/><span class="dim">Copy text, then press <kbd>Ctrl</kbd>+<kbd>S</kbd> to save your first snippet.</span>`;
    }
    return searchInput.value
      ? `Nothing in your clipboard or snippets matches <strong>${escapeHtml(searchInput.value)}</strong>.`
      : `Clipboard history is empty.<br/><span class="dim">Copy something and it'll appear here. Press <kbd>Ctrl</kbd>+<kbd>S</kbd> to save it as a snippet.</span>`;
  }

  function renderFooter(): void {
    footerEl.innerHTML =
      state.mode === "clipboard"
        ? `<span><kbd>↵</kbd> paste</span><span><kbd>Tab</kbd> snippets</span><span><kbd>Ctrl</kbd>+<kbd>S</kbd> save</span><span><kbd>Ctrl</kbd>+<kbd>,</kbd> settings</span><span><kbd>Esc</kbd> close</span>`
        : `<span><kbd>↵</kbd> paste</span><span><kbd>Tab</kbd> clipboard</span><span><kbd>Ctrl</kbd>+<kbd>S</kbd> save clipboard</span><span><kbd>Ctrl</kbd>+<kbd>⌫</kbd> delete</span><span><kbd>Ctrl</kbd>+<kbd>,</kbd> settings</span><span><kbd>Esc</kbd> close</span>`;
  }

  // ----- Actions -------------------------------------------------------------

  async function activate(): Promise<void> {
    const row = state.rows[state.selected];
    if (!row) return;
    await api.paste(row.content, row.kind === "snippet" ? row.id : undefined);
  }

  async function deleteSelected(): Promise<void> {
    const row = state.rows[state.selected];
    if (!row || row.kind !== "snippet") return;
    await api.deleteSnippet(row.id);
    await runSearch();
  }

  function openSave(content: string): void {
    state.saving = true;
    saveContent.value = content;
    saveName.value = suggestName(content);
    saveOverlay.classList.remove("hidden");
    saveName.focus();
    saveName.select();
  }

  function closeSave(): void {
    state.saving = false;
    saveOverlay.classList.add("hidden");
    searchInput.focus();
  }

  async function confirmSave(): Promise<void> {
    const name = saveName.value.trim();
    const content = saveContent.value;
    if (!name || !content.trim()) {
      saveName.focus();
      return;
    }
    await api.saveSnippet(name, content);
    closeSave();
    searchInput.value = "";
    await runSearch();
  }

  /** Save the selected clipboard row, else the live OS clipboard. */
  async function startSaveFlow(): Promise<void> {
    const row = state.rows[state.selected];
    if (row && row.kind === "clipboard") {
      openSave(row.content);
      return;
    }
    const current = await api.currentClipboard();
    openSave(current.trim() ? current : "");
  }

  function setMode(mode: Mode): void {
    if (state.mode === mode) return;
    state.mode = mode;
    state.selected = 0;
    void runSearch();
  }

  // ----- Keyboard ------------------------------------------------------------

  function onKeyDown(ev: KeyboardEvent): void {
    if (state.saving) {
      if (ev.key === "Escape") {
        ev.preventDefault();
        closeSave();
      } else if (ev.key === "Enter" && (ev.target === saveName || ev.metaKey || ev.ctrlKey)) {
        ev.preventDefault();
        void confirmSave();
      }
      return;
    }

    switch (ev.key) {
      case "ArrowDown":
        ev.preventDefault();
        state.selected = Math.min(state.selected + 1, state.rows.length - 1);
        render();
        break;
      case "ArrowUp":
        ev.preventDefault();
        state.selected = Math.max(state.selected - 1, 0);
        render();
        break;
      case "Enter":
        ev.preventDefault();
        void activate();
        break;
      case "Tab":
        ev.preventDefault();
        setMode(state.mode === "clipboard" ? "snippets" : "clipboard");
        break;
      case "Escape":
        ev.preventDefault();
        if (searchInput.value) {
          searchInput.value = "";
          void runSearch();
        } else {
          void api.hide();
        }
        break;
      case "s":
        if (ev.ctrlKey || ev.metaKey) {
          ev.preventDefault();
          void startSaveFlow();
        }
        break;
      case ",":
        if (ev.ctrlKey || ev.metaKey) {
          ev.preventDefault();
          void api.openSettings();
        }
        break;
      case "Backspace":
      case "Delete":
        if (ev.ctrlKey || ev.metaKey) {
          ev.preventDefault();
          void deleteSelected();
        }
        break;
      default:
        break;
    }
  }

  // ----- Wiring --------------------------------------------------------------

  searchInput.addEventListener("input", () => void runSearch());
  document.addEventListener("keydown", onKeyDown);

  tabsEl.querySelectorAll<HTMLButtonElement>(".tab").forEach((btn) => {
    btn.addEventListener("click", () => setMode(btn.dataset.mode as Mode));
  });

  saveConfirm.addEventListener("click", () => void confirmSave());

  // When the backend reveals the window, reset to the default (clipboard) view.
  void api.onShow(() => {
    state.mode = "clipboard";
    searchInput.value = "";
    searchInput.focus();
    void runSearch();
  });

  window.addEventListener("focus", () => {
    if (!state.saving) searchInput.focus();
  });

  searchInput.focus();
  void runSearch();
}

// =============================================================================
// Settings window
// =============================================================================

async function initSettings(): Promise<void> {
  document.getElementById("app")?.classList.add("hidden");
  const page = $<HTMLDivElement>("#settings-page");
  page.classList.remove("hidden");

  const pathInput = $<HTMLInputElement>("#set-path");
  const capInput = $<HTMLInputElement>("#set-cap");
  const status = $<HTMLDivElement>("#set-status");
  const defaultPathEl = $<HTMLElement>("#set-default-path");

  const initial = await api.getSettings();
  pathInput.value = initial.dbPath;
  capInput.value = String(initial.clipboardCap);
  defaultPathEl.textContent = initial.defaultDbPath;

  function showStatus(msg: string, ok: boolean): void {
    status.textContent = msg;
    status.className = `set-status ${ok ? "ok" : "err"}`;
  }

  async function save(): Promise<void> {
    const cap = Math.max(1, Math.min(100000, parseInt(capInput.value, 10) || initial.clipboardCap));
    try {
      const updated = await api.setSettings(pathInput.value.trim(), cap);
      pathInput.value = updated.dbPath;
      capInput.value = String(updated.clipboardCap);
      showStatus("Saved ✓  Settings applied.", true);
    } catch (e) {
      showStatus(String(e), false);
    }
  }

  $<HTMLButtonElement>("#set-save").addEventListener("click", () => void save());
  $<HTMLButtonElement>("#set-reset").addEventListener("click", () => {
    pathInput.value = initial.defaultDbPath;
  });
  $<HTMLButtonElement>("#set-cancel").addEventListener("click", () => void api.closeSettings());

  document.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape") {
      ev.preventDefault();
      void api.closeSettings();
    } else if (ev.key === "Enter") {
      ev.preventDefault();
      void save();
    }
  });
}

// ----- Entry point -----------------------------------------------------------

const route = location.hash.replace(/^#\/?/, "");
if (route === "settings") {
  void initSettings();
} else {
  initPalette();
}
