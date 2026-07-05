import { api } from "./api";
import type {
  ClipboardEntry,
  ItemKind,
  KnownDevice,
  Mode,
  Peer,
  SearchResult,
  SyncResult,
  SyncStatus,
  UnifiedResult,
} from "./types";
import "./styles.css";

/** Build identifier injected at build time (see vite.config.ts). */
declare const __DEVCLIP_BUILD__: string;

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
  const footerHints = $<HTMLSpanElement>("#footer-hints");
  const footerBuild = $<HTMLSpanElement>("#footer-build");
  const tabsEl = $<HTMLDivElement>("#mode-tabs");

  const saveOverlay = $<HTMLDivElement>("#save-overlay");
  const saveName = $<HTMLInputElement>("#save-name");
  const saveContent = $<HTMLTextAreaElement>("#save-content");
  const saveConfirm = $<HTMLButtonElement>("#save-confirm");

  const settingsOverlay = $<HTMLDivElement>("#settings-overlay");
  const setPath = $<HTMLInputElement>("#set-path");
  const setCap = $<HTMLInputElement>("#set-cap");
  const setStatus = $<HTMLDivElement>("#set-status");
  const setDefaultPath = $<HTMLElement>("#set-default-path");
  const setSaveBtn = $<HTMLButtonElement>("#set-save");
  const setResetBtn = $<HTMLButtonElement>("#set-reset");
  const setCancelBtn = $<HTMLButtonElement>("#set-cancel");
  const setBuild = $<HTMLElement>("#set-build");

  // Sync chip (footer) + sync panel + enable warning.
  const syncChip = $<HTMLButtonElement>("#sync-chip");
  const syncChipLabel = $<HTMLSpanElement>("#sync-chip-label");
  const syncOverlay = $<HTMLDivElement>("#sync-overlay");
  const syncStateLabel = $<HTMLElement>("#sync-state-label");
  const syncToggleBtn = $<HTMLButtonElement>("#sync-toggle");
  const syncCloseBtn = $<HTMLButtonElement>("#sync-close");
  const syncControls = $<HTMLDivElement>("#sync-controls");
  const syncOffHint = $<HTMLParagraphElement>("#sync-off-hint");
  const syncWarnOverlay = $<HTMLDivElement>("#sync-warn-overlay");
  const syncWarnConfirm = $<HTMLButtonElement>("#sync-warn-confirm");
  const syncWarnCancel = $<HTMLButtonElement>("#sync-warn-cancel");

  const setDeviceName = $<HTMLInputElement>("#set-device-name");
  const syncScanBtn = $<HTMLButtonElement>("#sync-scan");
  const syncStatus = $<HTMLSpanElement>("#sync-status");
  const syncPeersEl = $<HTMLUListElement>("#sync-peers");
  const syncKnownWrap = $<HTMLDivElement>("#sync-known-wrap");
  const syncKnownEl = $<HTMLUListElement>("#sync-known");

  interface State {
    mode: Mode;
    selected: number;
    rows: Row[];
    /** Whether the current list mixes snippets and clipboard (show badges). */
    unified: boolean;
    saving: boolean;
    settingsOpen: boolean;
    syncPanelOpen: boolean;
    syncWarnOpen: boolean;
    /** The user's persisted sync on/off choice. */
    syncEnabled: boolean;
    /** Whether the listener sockets are actually bound right now. */
    syncRunning: boolean;
  }

  const state: State = {
    mode: "clipboard", // Clipboard is the default view.
    selected: 0,
    rows: [],
    unified: false,
    saving: false,
    settingsOpen: false,
    syncPanelOpen: false,
    syncWarnOpen: false,
    syncEnabled: false,
    syncRunning: false,
  };

  let defaultDbPath = "";

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
    const hints =
      state.mode === "clipboard"
        ? `<span><kbd>↵</kbd> paste</span><span><kbd>Tab</kbd> snippets</span><span><kbd>Ctrl</kbd>+<kbd>S</kbd> save</span><span><kbd>Ctrl</kbd>+<kbd>,</kbd> settings</span><span><kbd>Esc</kbd> close</span>`
        : `<span><kbd>↵</kbd> paste</span><span><kbd>Tab</kbd> clipboard</span><span><kbd>Ctrl</kbd>+<kbd>S</kbd> save clipboard</span><span><kbd>Ctrl</kbd>+<kbd>⌫</kbd> delete</span><span><kbd>Ctrl</kbd>+<kbd>,</kbd> settings</span><span><kbd>Esc</kbd> close</span>`;
    // Only the hint/build spans are rewritten; the sync chip is a sibling that
    // keeps its listeners.
    footerHints.innerHTML = hints;
    footerBuild.textContent = `build ${__DEVCLIP_BUILD__}`;
  }

  /** Reflect the sync on/off state in the footer chip. */
  function renderSyncChip(): void {
    const on = state.syncEnabled;
    syncChip.classList.toggle("on", on);
    syncChip.classList.toggle("off", !on);
    syncChipLabel.textContent = on ? "Sync on" : "Sync off";
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

  // ----- Settings panel ------------------------------------------------------

  async function openSettings(): Promise<void> {
    state.settingsOpen = true;
    setStatus.textContent = "";
    setStatus.className = "set-status";
    settingsOverlay.classList.remove("hidden");
    try {
      const s = await api.getSettings();
      setPath.value = s.dbPath;
      setCap.value = String(s.clipboardCap);
      defaultDbPath = s.defaultDbPath;
      setDefaultPath.textContent = s.defaultDbPath;
    } catch (e) {
      setStatus.textContent = String(e);
      setStatus.className = "set-status err";
    }
    setPath.focus();
    setPath.select();
  }

  function closeSettings(): void {
    state.settingsOpen = false;
    settingsOverlay.classList.add("hidden");
    searchInput.focus();
  }

  async function saveSettings(): Promise<void> {
    const cap = Math.max(1, Math.min(100000, parseInt(setCap.value, 10) || 0));
    try {
      const updated = await api.setSettings(setPath.value.trim(), cap);
      setPath.value = updated.dbPath;
      setCap.value = String(updated.clipboardCap);
      setStatus.textContent = "Saved ✓  Settings applied.";
      setStatus.className = "set-status ok";
    } catch (e) {
      setStatus.textContent = String(e);
      setStatus.className = "set-status err";
    }
  }

  // ----- LAN sync ------------------------------------------------------------

  function syncMsg(text: string, kind: "ok" | "err" | "" = ""): void {
    syncStatus.textContent = text;
    syncStatus.className = "set-status" + (kind ? ` ${kind}` : "");
  }

  function applyStatus(st: SyncStatus): void {
    state.syncEnabled = st.enabled;
    state.syncRunning = st.running;
    renderSyncChip();
    if (state.syncPanelOpen) renderSyncPanel();
  }

  /** Fetch the current sync state and reflect it in the chip (called at start). */
  async function loadSyncStatus(): Promise<void> {
    try {
      applyStatus(await api.syncStatus());
    } catch {
      // Non-fatal; chip stays "off".
    }
  }

  /** Turn sync on/off, surfacing any bind/firewall error in the panel. */
  async function applySyncEnabled(enabled: boolean): Promise<void> {
    try {
      applyStatus(await api.setSyncEnabled(enabled));
      if (state.syncPanelOpen && enabled) syncMsg("Sync is on.", "ok");
    } catch (e) {
      // Enabling can fail (port busy / firewall denied). Reflect reality and
      // show why — opening the panel if it isn't already visible.
      state.syncEnabled = false;
      state.syncRunning = false;
      renderSyncChip();
      if (!state.syncPanelOpen) openSyncPanel();
      renderSyncPanel();
      syncMsg(String(e), "err");
    }
  }

  // ----- Enable warning ------------------------------------------------------

  function openSyncWarn(): void {
    state.syncWarnOpen = true;
    syncWarnOverlay.classList.remove("hidden");
    syncWarnConfirm.focus();
  }

  function closeSyncWarn(): void {
    state.syncWarnOpen = false;
    syncWarnOverlay.classList.add("hidden");
  }

  async function confirmSyncWarn(): Promise<void> {
    closeSyncWarn();
    await applySyncEnabled(true);
  }

  // ----- Sync panel ----------------------------------------------------------

  /** Left-click the chip: enable (with warning) when off, disable when on. */
  function onChipToggle(): void {
    if (state.syncEnabled) {
      void applySyncEnabled(false);
    } else {
      openSyncWarn();
    }
  }

  function renderSyncPanel(): void {
    const on = state.syncEnabled;
    syncStateLabel.textContent = on ? "On" : "Off";
    syncStateLabel.className = on ? "on" : "off";
    syncToggleBtn.textContent = on ? "Disable" : "Enable";
    syncToggleBtn.classList.toggle("danger", !on);
    syncControls.classList.toggle("hidden", !on);
    syncOffHint.classList.toggle("hidden", on);
  }

  async function openSyncPanel(): Promise<void> {
    state.syncPanelOpen = true;
    syncPeersEl.innerHTML = "";
    syncMsg("");
    syncOverlay.classList.remove("hidden");
    try {
      const st = await api.syncStatus();
      state.syncEnabled = st.enabled;
      state.syncRunning = st.running;
      setDeviceName.value = st.deviceName;
    } catch {
      // Non-fatal.
    }
    renderSyncChip();
    renderSyncPanel();
    await renderKnownDevices();
  }

  function closeSyncPanel(): void {
    state.syncPanelOpen = false;
    syncOverlay.classList.add("hidden");
    searchInput.focus();
  }

  function syncResultText(r: SyncResult): string {
    const parts: string[] = [];
    if (r.snippetsAdded) parts.push(`${r.snippetsAdded} snippet${r.snippetsAdded === 1 ? "" : "s"} added`);
    if (r.snippetsUpdated) parts.push(`${r.snippetsUpdated} updated`);
    if (r.clipsAdded) parts.push(`${r.clipsAdded} clip${r.clipsAdded === 1 ? "" : "s"} added`);
    return parts.length ? `Synced ✓  ${parts.join(", ")}.` : "Synced ✓  Already up to date.";
  }

  async function syncWith(addr: string, label: string): Promise<void> {
    syncMsg(`Syncing with ${label}…`);
    try {
      const r = await api.syncWithPeer(addr);
      syncMsg(syncResultText(r), "ok");
      await renderKnownDevices();
      // Local data changed — refresh the list behind the overlay.
      await runSearch();
    } catch (e) {
      syncMsg(String(e), "err");
    }
  }

  async function scanForPeers(): Promise<void> {
    syncScanBtn.disabled = true;
    syncScanBtn.textContent = "Scanning…";
    syncPeersEl.innerHTML = "";
    syncMsg("Looking for DevClip on your network…");
    try {
      const peers = await api.discoverPeers();
      renderPeers(peers);
      syncMsg(
        peers.length === 0
          ? "No devices found. Open DevClip on the other machine and make sure both are on the same network."
          : `Found ${peers.length} device${peers.length === 1 ? "" : "s"}.`,
        peers.length === 0 ? "" : "ok",
      );
    } catch (e) {
      syncMsg(String(e), "err");
    } finally {
      syncScanBtn.disabled = false;
      syncScanBtn.textContent = "Scan for devices";
    }
  }

  function syncRow(name: string, sub: string): { li: HTMLLIElement; info: HTMLDivElement } {
    const li = document.createElement("li");
    li.className = "sync-row";
    const info = document.createElement("div");
    info.className = "sync-info";
    info.innerHTML = `<span class="sync-name">${escapeHtml(name)}</span><span class="sync-addr">${escapeHtml(sub)}</span>`;
    return { li, info };
  }

  function renderPeers(peers: Peer[]): void {
    syncPeersEl.innerHTML = "";
    for (const p of peers) {
      const { li, info } = syncRow(p.deviceName, p.addr);
      const btn = document.createElement("button");
      btn.className = "primary sync-go";
      btn.textContent = "Sync";
      btn.addEventListener("click", () => void syncWith(p.addr, p.deviceName));
      li.append(info, btn);
      syncPeersEl.appendChild(li);
    }
  }

  async function renderKnownDevices(): Promise<void> {
    let devices: KnownDevice[] = [];
    try {
      devices = await api.listKnownDevices();
    } catch {
      devices = [];
    }
    syncKnownEl.innerHTML = "";
    if (devices.length === 0) {
      syncKnownWrap.classList.add("hidden");
      return;
    }
    syncKnownWrap.classList.remove("hidden");
    for (const d of devices) {
      const { li, info } = syncRow(d.name, `last synced ${timeAgo(d.lastSyncedAt)} · ${d.addr}`);
      const go = document.createElement("button");
      go.className = "ghost sync-go";
      go.textContent = "Sync";
      go.addEventListener("click", () => void syncWith(d.addr, d.name));
      const forget = document.createElement("button");
      forget.className = "ghost sync-forget";
      forget.textContent = "Forget";
      forget.addEventListener("click", async () => {
        await api.forgetDevice(d.deviceId);
        await renderKnownDevices();
      });
      li.append(info, go, forget);
      syncKnownEl.appendChild(li);
    }
  }

  async function saveDeviceName(): Promise<void> {
    const name = setDeviceName.value.trim();
    if (!name) return;
    try {
      const info = await api.setDeviceName(name);
      setDeviceName.value = info.deviceName;
    } catch (e) {
      syncMsg(String(e), "err");
    }
  }

  // ----- Keyboard ------------------------------------------------------------

  function onKeyDown(ev: KeyboardEvent): void {
    if (state.syncWarnOpen) {
      if (ev.key === "Escape") {
        ev.preventDefault();
        closeSyncWarn();
      } else if (ev.key === "Enter") {
        ev.preventDefault();
        void confirmSyncWarn();
      }
      return;
    }

    if (state.syncPanelOpen) {
      if (ev.key === "Escape") {
        ev.preventDefault();
        closeSyncPanel();
      } else if (ev.key === "Enter" && ev.target === setDeviceName) {
        ev.preventDefault();
        void saveDeviceName();
      }
      return;
    }

    if (state.settingsOpen) {
      if (ev.key === "Escape") {
        ev.preventDefault();
        closeSettings();
      } else if (ev.key === "Enter") {
        // Enter saves the field you're in; on buttons, let them click normally.
        if (ev.target === setDeviceName) {
          ev.preventDefault();
          void saveDeviceName();
        } else if (ev.target === setPath || ev.target === setCap) {
          ev.preventDefault();
          void saveSettings();
        }
      }
      return;
    }

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
          void openSettings();
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

  setSaveBtn.addEventListener("click", () => void saveSettings());
  setCancelBtn.addEventListener("click", () => closeSettings());
  setResetBtn.addEventListener("click", () => {
    setPath.value = defaultDbPath;
  });
  setBuild.textContent = __DEVCLIP_BUILD__;

  syncScanBtn.addEventListener("click", () => void scanForPeers());
  setDeviceName.addEventListener("change", () => void saveDeviceName());

  // Sync chip: left-click toggles (with warning when enabling), right-click
  // opens the details panel. The title attribute is the hover tooltip.
  syncChip.addEventListener("click", () => onChipToggle());
  syncChip.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    void openSyncPanel();
  });
  syncToggleBtn.addEventListener("click", () => onChipToggle());
  syncCloseBtn.addEventListener("click", () => closeSyncPanel());
  syncWarnConfirm.addEventListener("click", () => void confirmSyncWarn());
  syncWarnCancel.addEventListener("click", () => closeSyncWarn());

  // When the backend reveals the window, reset to the default (clipboard) view.
  void api.onShow(() => {
    if (state.syncWarnOpen) closeSyncWarn();
    if (state.syncPanelOpen) closeSyncPanel();
    if (state.settingsOpen) closeSettings();
    state.mode = "clipboard";
    searchInput.value = "";
    searchInput.focus();
    void runSearch();
    void loadSyncStatus();
  });

  window.addEventListener("focus", () => {
    if (!state.saving && !state.settingsOpen && !state.syncPanelOpen && !state.syncWarnOpen) {
      searchInput.focus();
    }
  });

  searchInput.focus();
  renderSyncChip();
  void loadSyncStatus();
  void runSearch();
}

// ----- Entry point -----------------------------------------------------------

initPalette();
