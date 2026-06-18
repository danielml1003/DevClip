import { api } from "./api";
import type { ClipboardEntry, Mode, SearchResult } from "./types";
import "./styles.css";

// ----- Element handles -------------------------------------------------------

const $ = <T extends HTMLElement>(sel: string): T => {
  const el = document.querySelector<T>(sel);
  if (!el) throw new Error(`missing element: ${sel}`);
  return el;
};

const searchInput = $<HTMLInputElement>("#search");
const resultsEl = $<HTMLUListElement>("#results");
const emptyEl = $<HTMLDivElement>("#empty");
const footerEl = $<HTMLDivElement>("#footer");
const tabsEl = $<HTMLDivElement>("#mode-tabs");

const saveOverlay = $<HTMLDivElement>("#save-overlay");
const saveName = $<HTMLInputElement>("#save-name");
const saveContent = $<HTMLTextAreaElement>("#save-content");
const saveConfirm = $<HTMLButtonElement>("#save-confirm");

// ----- State -----------------------------------------------------------------

interface State {
  mode: Mode;
  selected: number;
  snippetResults: SearchResult[];
  clipboardResults: ClipboardEntry[];
  saving: boolean;
}

const state: State = {
  mode: "snippets",
  selected: 0,
  snippetResults: [],
  clipboardResults: [],
  saving: false,
};

// ----- Helpers ---------------------------------------------------------------

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
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

/** Smart default name from content: a trimmed, single-line preview. */
function suggestName(content: string): string {
  return firstLine(content, 60);
}

function currentCount(): number {
  return state.mode === "snippets"
    ? state.snippetResults.length
    : state.clipboardResults.length;
}

function clampSelection(): void {
  const n = currentCount();
  if (n === 0) state.selected = 0;
  else state.selected = Math.max(0, Math.min(state.selected, n - 1));
}

// ----- Rendering -------------------------------------------------------------

function render(): void {
  // Tabs
  tabsEl.querySelectorAll<HTMLButtonElement>(".tab").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.mode === state.mode);
  });

  clampSelection();
  resultsEl.innerHTML = "";

  const items =
    state.mode === "snippets"
      ? state.snippetResults.map(renderSnippetRow)
      : state.clipboardResults.map(renderClipboardRow);

  if (items.length === 0) {
    emptyEl.classList.remove("hidden");
    emptyEl.innerHTML =
      state.mode === "snippets"
        ? searchInput.value
          ? `No snippets match <strong>${escapeHtml(searchInput.value)}</strong>.<br/><span class="dim">Press <kbd>Tab</kbd> to search clipboard history, or copy something and <kbd>Ctrl</kbd>+<kbd>S</kbd> to save it.</span>`
          : `No snippets yet.<br/><span class="dim">Copy text, switch to <kbd>Clipboard</kbd> with <kbd>Tab</kbd>, and press <kbd>Ctrl</kbd>+<kbd>S</kbd> to save your first snippet.</span>`
        : `Clipboard history is empty.`;
  } else {
    emptyEl.classList.add("hidden");
    items.forEach((li, i) => {
      li.classList.toggle("selected", i === state.selected);
      li.addEventListener("mousemove", () => {
        if (state.selected !== i) {
          state.selected = i;
          render();
        }
      });
      li.addEventListener("click", () => {
        state.selected = i;
        activate();
      });
      resultsEl.appendChild(li);
    });
    scrollSelectedIntoView();
  }

  renderFooter();
}

function renderSnippetRow(r: SearchResult): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "row";
  li.setAttribute("role", "option");
  const uses = r.snippet.useCount > 0 ? `${r.snippet.useCount}×` : "";
  li.innerHTML = `
    <div class="row-main">
      <div class="row-title">${highlight(r.snippet.name, r.nameIndices)}</div>
      <div class="row-sub">${highlight(firstLine(r.snippet.content), r.contentIndices)}</div>
    </div>
    <div class="row-meta">${uses}</div>`;
  return li;
}

function renderClipboardRow(e: ClipboardEntry): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "row";
  li.setAttribute("role", "option");
  li.innerHTML = `
    <div class="row-main">
      <div class="row-title mono">${escapeHtml(firstLine(e.content))}</div>
      <div class="row-sub dim">${timeAgo(e.createdAt)}</div>
    </div>
    <div class="row-meta">⌘S to save</div>`;
  return li;
}

function renderFooter(): void {
  footerEl.innerHTML =
    state.mode === "snippets"
      ? `<span><kbd>↵</kbd> paste</span><span><kbd>Tab</kbd> clipboard</span><span><kbd>Ctrl</kbd>+<kbd>S</kbd> save clipboard</span><span><kbd>Ctrl</kbd>+<kbd>⌫</kbd> delete</span><span><kbd>Esc</kbd> close</span>`
      : `<span><kbd>↵</kbd> paste</span><span><kbd>Ctrl</kbd>+<kbd>S</kbd> save as snippet</span><span><kbd>Tab</kbd> snippets</span><span><kbd>Esc</kbd> close</span>`;
}

function scrollSelectedIntoView(): void {
  const el = resultsEl.children[state.selected] as HTMLElement | undefined;
  el?.scrollIntoView({ block: "nearest" });
}

function timeAgo(unix: number): string {
  const s = Math.max(0, Math.floor(Date.now() / 1000) - unix);
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

// ----- Data --------------------------------------------------------------

async function runSearch(): Promise<void> {
  const query = searchInput.value;
  if (state.mode === "snippets") {
    state.snippetResults = await api.search(query);
  } else {
    const all = await api.listClipboard();
    const q = query.trim().toLowerCase();
    state.clipboardResults = q
      ? all.filter((e) => e.content.toLowerCase().includes(q))
      : all;
  }
  state.selected = 0;
  render();
}

// ----- Actions ---------------------------------------------------------------

async function activate(): Promise<void> {
  if (state.mode === "snippets") {
    const r = state.snippetResults[state.selected];
    if (!r) return;
    await api.paste(r.snippet.content, r.snippet.id);
  } else {
    const e = state.clipboardResults[state.selected];
    if (!e) return;
    await api.paste(e.content);
  }
}

async function deleteSelected(): Promise<void> {
  if (state.mode !== "snippets") return;
  const r = state.snippetResults[state.selected];
  if (!r) return;
  await api.deleteSnippet(r.snippet.id);
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
  state.mode = "snippets";
  searchInput.value = "";
  await runSearch();
}

/** Decide what to save: selected clipboard entry, else the live OS clipboard. */
async function startSaveFlow(): Promise<void> {
  if (state.mode === "clipboard") {
    const e = state.clipboardResults[state.selected];
    if (e) {
      openSave(e.content);
      return;
    }
  }
  const current = await api.currentClipboard();
  if (current.trim()) openSave(current);
  else openSave("");
}

function setMode(mode: Mode): void {
  if (state.mode === mode) return;
  state.mode = mode;
  state.selected = 0;
  void runSearch();
}

// ----- Keyboard --------------------------------------------------------------

function onKeyDown(ev: KeyboardEvent): void {
  // Save dialog has its own handling.
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
      state.selected = Math.min(state.selected + 1, currentCount() - 1);
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
      setMode(state.mode === "snippets" ? "clipboard" : "snippets");
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

// ----- Wiring ----------------------------------------------------------------

function init(): void {
  searchInput.addEventListener("input", () => void runSearch());
  document.addEventListener("keydown", onKeyDown);

  tabsEl.querySelectorAll<HTMLButtonElement>(".tab").forEach((btn) => {
    btn.addEventListener("click", () => setMode(btn.dataset.mode as Mode));
  });

  saveConfirm.addEventListener("click", () => void confirmSave());

  // When the backend reveals the window, reset to a clean search state.
  void api.onShow(() => {
    state.mode = "snippets";
    searchInput.value = "";
    searchInput.focus();
    void runSearch();
  });

  // Keep focus in the search box (keyboard-first).
  window.addEventListener("focus", () => {
    if (!state.saving) searchInput.focus();
  });

  searchInput.focus();
  void runSearch();
}

init();
