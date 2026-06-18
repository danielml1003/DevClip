# DevClip — Design

> A searchable memory for developer snippets. **Not** a clipboard manager — the
> clipboard is just the fastest way data enters the system.

This document covers the six deliverables from the product spec: architecture,
UI flow, UX risks, technology stack, the phased plan, and how the code maps to
all of it.

---

## 1. Architecture

DevClip is split into three layers, with a hard rule: **all the logic worth
trusting lives in a pure-Rust core that has no GUI dependencies and is fully
unit-tested.** The desktop shell is deliberately thin.

```
┌──────────────────────────────────────────────────────────┐
│  Frontend (TypeScript + Vite, vanilla — no framework)      │
│  • command-palette UI, keyboard nav, highlighting          │
│  • runs in the OS webview; also runs in a plain browser     │
│    against an in-memory mock (great DX)                     │
└───────────────▲───────────────────────┬──────────────────┘
                │ Tauri IPC (invoke)     │ events ("devclip://show")
┌───────────────┴───────────────────────▼──────────────────┐
│  src-tauri  (thin Rust shell)                              │
│  • global hotkey (Alt+Space) → show/hide palette           │
│  • clipboard polling → history (capped, deduped)           │
│  • paste-into-focused-app (clipboard + synthetic Ctrl/⌘+V) │
│  • IPC commands; DTOs (serde) ↔ core types                 │
└───────────────────────────┬──────────────────────────────┘
                            │ plain function calls
┌───────────────────────────▼──────────────────────────────┐
│  devclip-core  (pure Rust, no Tauri/webkit, 23 unit tests) │
│  • model     — Snippet, ClipboardEntry, ScoredSnippet      │
│  • fuzzy     — hand-written scoring matcher                 │
│  • search    — ranking: fuzzy × field-weight + recency/freq │
│  • store     — SQLite (rusqlite, bundled) + migrations      │
└──────────────────────────────────────────────────────────┘
```

**Why a separate core crate?** Search quality and storage correctness are the
product. Keeping them in a crate with zero GUI dependencies means they compile
and test anywhere (including headless CI), independent of the platform webview.
The fuzzy/ranking/storage behaviour is locked down by unit tests; the Tauri
layer is just plumbing.

### Data model (as specified)

| Snippet field  | Type            | Notes                                  |
|----------------|-----------------|----------------------------------------|
| `id`           | INTEGER PK      | autoincrement                          |
| `name`         | TEXT            | friendly name                          |
| `content`      | TEXT            | the actual snippet                     |
| `created_at`   | INTEGER (unix)  | set on insert                          |
| `last_used_at` | INTEGER \| NULL | stamped on paste — drives recency      |
| `use_count`    | INTEGER         | incremented on paste — drives freq     |

Clipboard history (`clipboard_history`) is separate and transient: capped at
100 entries, consecutive-duplicate and empty captures skipped. It is **never**
the permanent store (Principle 4).

The schema is versioned with `PRAGMA user_version`, so future features
(variables, expiration, sync metadata) migrate cleanly.

### Search & ranking (the heart)

`fuzzy.rs` implements a hand-written subsequence matcher (so we fully control
"search quality"):

- a query matches if its characters appear **in order** (subsequence);
- **exact substring** is the strongest signal (fast-path);
- **word-boundary** and **camelCase** hits are rewarded — `mig` strongly hits
  the *migration* word in "Production migration command";
- **contiguous runs** are rewarded; leading and internal **gaps** are penalised;
- matching is case-insensitive and returns matched indices for **highlighting**.

`search.rs` blends per-snippet relevance:

```
score = Σ over query terms ( max(name_fuzzy × 1.6, content_fuzzy × 1.0) )
      + ln(1 + use_count) × 12                 // frequency
      + 40 × 0.5^(age / 7 days)                // recency, half-life 7d
```

- Multi-word queries are **ANDed** ("docker compose" → both terms must match).
- **Name matches outrank content matches** (so "prod" prefers titles).
- **Empty query** → everything ordered by recency+frequency: the "what was I
  just doing" view, perfect for the popup's resting state.

For a personal local-first tool (hundreds–thousands of rows) we load all
snippets and rank in memory on every keystroke — comfortably sub-millisecond.

---

## 2. UI flow

```
                 ┌──────── Alt+Space (global) ────────┐
                 ▼                                     │
   ╔═══════════════════════════════════════╗          │
   ║ ⌘  Search your snippets…   [Snippets|Clipboard] ║  ← autofocused
   ╟───────────────────────────────────────╢          │
   ║ ▸ Production migration command          ║  ← fuzzy results, highlighted
   ║   docker compose exec api … migrate     ║          │
   ║   Local docker compose up               ║          │
   ╟───────────────────────────────────────╢          │
   ║ ↵ paste · Tab clipboard · ⌃S save · Esc║          │
   ╚═══════════════════════════════════════╝          │
        │ ↑/↓ select   ↵ paste into prev app ──────────┘ (window hides,
        │ Tab → Clipboard history                         Ctrl/⌘+V injected)
        │ Ctrl+S on a clipboard entry → Save dialog (name prefilled) → ↵
        │ Esc → clear query, then hide
```

- **Retrieve:** hotkey → type → ↑/↓ → **Enter pastes into the app you were just
  in**. Mouse optional.
- **Save:** copy normally → hotkey → `Tab` to Clipboard → `Ctrl+S` → name is
  pre-filled from the content's first line → `Enter`. Minimal friction.
- Dismisses on blur (Raycast-style) and on `Esc`.

---

## 3. UX risks & how they're handled

| Risk | Mitigation |
|------|------------|
| **Win+V collides** with Windows' native clipboard history | Default hotkey is **Alt+Space**; hotkey is centralised in one place and designed to be user-configurable (V2). |
| **Paste reliability** — needs focus to return to the prior app + a synthetic keystroke; macOS requires Accessibility permission | Copy-to-clipboard happens first and always succeeds, so the fallback is "it's on your clipboard, press ⌘/Ctrl+V". Window hides before injecting so focus returns. |
| **Clipboard polling & sensitive data** (password managers) | Poll lightly (700 ms), skip empty/duplicate captures, **cap history at 100**, and require **explicit** promotion to a permanent snippet (Principle 4). History can be cleared. |
| **Global shortcut already taken** | Registration is isolated; surfacing failures + rebind UI is the V2 follow-up. |
| **Multiline / huge content** | Rows show a single-line preview; full content is preserved and editable in the Save dialog. |
| **Accidental loss** | Delete is an explicit `Ctrl+Delete` on a selected snippet (not automatic). |
| **First-run emptiness** | Empty state explains the copy → Tab → Ctrl+S flow. |

---

## 4. Technology stack (and why)

| Concern | Choice | Why |
|--------|--------|-----|
| Shell | **Tauri v2** | Native OS webview (no bundled Chromium) → tens-of-ms cold start and a few-MB binary. Directly serves the "fast startup / Raycast feel" goal; Electron would fight it. |
| Core logic | **Rust** | Fast fuzzy search, safe SQLite, native global-shortcut/clipboard/keystroke access. |
| UI | **Vanilla TypeScript + Vite** | The UI is one command-palette surface; a framework adds bundle + parse cost for no benefit. Bundle is ~4.5 KB gzipped → instant first paint. |
| Storage | **SQLite via `rusqlite` (bundled)** | Local-first, single file, zero external dependency, no server/account. |
| Search | **Custom Rust matcher** | Full control over ranking quality; no opaque dependency. |
| Paste | **`enigo`** | Cross-platform synthetic keystrokes. |

Trade-off considered: a faster ship would be Electron + Fuse.js, but it
contradicts the explicit *fast startup* and *Raycast-like* goals. Tauri is the
right call for a tool that must feel instant.

---

## 5. Phased implementation plan

| Phase | Scope | Status |
|------:|-------|--------|
| **P0** | Workspace scaffold, core crate, SQLite schema + migrations | ✅ done, tested |
| **P1** | Fuzzy matcher + blended ranking (recency/frequency) | ✅ done, tested |
| **P2** | Tauri shell: window, global hotkey, show/hide, IPC | ✅ done, builds |
| **P3** | Command-palette UI: search, keyboard nav, highlighting | ✅ done, builds |
| **P4** | Clipboard monitoring + history + Save (promote) flow | ✅ done |
| **P5** | Paste-into-focused-app + use tracking | ✅ done |
| **P6** | Polish: configurable hotkey, settings, tray, packaging/signing | ⏳ V1.x |

### Explicitly out of scope for V1 (designed-for, not built)

Snippet variables, expiration dates, cross-machine sync, AI naming/search,
team sharing. The schema's `meta`/versioning and the core/shell split leave
room for all of these without a rewrite.

---

## 6. Verification

- `cargo test --workspace` → **23 passing** unit tests covering the fuzzy
  matcher, ranking, and SQLite store (including the spec's headline cases:
  `mig` → "Production migration command", `docker compose`, `prod` prefers
  titles).
- `npm run build` → TypeScript (strict) + Vite production build, clean.
- `cargo build -p devclip-tauri` → the desktop binary links (requires the
  platform webview libs; see README).
