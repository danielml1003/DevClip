<div align="center">

# DevClip

**A searchable memory for developer snippets — not a clipboard manager.**

Fast, keyboard-first, local-first. The clipboard is just how data gets in.

</div>

---

Developers constantly have tiny, valuable, throwaway bits of text — SSH/docker/
kubectl commands, SQL queries, API requests, URLs, JWTs, regexes, build
commands, one-off migrations, AI prompts. They're too valuable to lose, too
temporary to document, too small for a file. They scatter across Slack,
bookmarks, random `.txt` files and shell history, and finding them later is
miserable.

DevClip gives them one home and makes them **instantly findable**:

```
Alt+Space  →  type "mig"  →  ↵  →  pasted into whatever you were doing
```

It feels like the VS Code command palette / Raycast / Alfred, but for *your*
commands.

## Features

- ⌨️ **Keyboard-first command palette** — global hotkey, autofocused search,
  arrow keys, Enter to paste. The mouse is optional.
- 🔎 **High-quality fuzzy search** over both **name and content**, with
  word-boundary / camelCase awareness and live highlighting.
  - `mig` → **Pro**duction **mig**ration command
  - `docker compose` → every compose command
  - `prod` → items whose **title** contains "prod" (titles rank higher)
- ⚡ **Smart ranking** — relevance blended with how recently and how often you
  use each snippet, so your muscle-memory commands float to the top.
- 📋 **Clipboard history** as the on-ramp — copy normally, then **explicitly**
  promote the good stuff to a permanent, named snippet (`Ctrl+S`).
- 💾 **Local-first** — a single SQLite file. No cloud or account required.
- 🔗 **Optional LAN sync** — off by default; when enabled, sync snippets and
  clipboard history directly with another machine on your network. No server,
  no account.
- 🪶 **Fast & tiny** — a native OS webview (Tauri), not Electron.

## Download & install

**[Download the latest release](https://github.com/danielml1003/DevClip/releases/latest)** and choose the installer for your platform:

- **Windows** — `.exe` installer (or `.msi`)
- **macOS** — universal `.dmg` (Apple Silicon and Intel)

Release assets are public, permanent downloads and require no GitHub account.

<details>
<summary>Development builds (per-push CI artifacts)</summary>

Every push to `devclip-app` publishes installers as workflow artifacts under
[Actions → Build DevClip](https://github.com/danielml1003/DevClip/actions/workflows/build.yml).
Open the most recent successful run and download `devclip-windows-installers` or
`devclip-macos-installer`. Artifacts require a GitHub sign-in and are retained
for roughly 90 days; releases have neither limitation.

</details>

### First launch

These builds are not code-signed, so each operating system shows a one-time
warning. (Removing it requires code signing — an Apple Developer account for
macOS and a certificate for Windows — which is not configured for these builds.)

**Windows.** SmartScreen displays *"Windows protected your PC."* Select
**More info → Run anyway** and complete the installer. Launch DevClip and press
**Alt + Space**.

**macOS.** Gatekeeper blocks unsigned applications. Open the `.dmg`, drag
**DevClip** into **Applications**, then right-click the app and choose **Open**
(or run `xattr -cr /Applications/DevClip.app`). Grant Accessibility permission
under **System Settings → Privacy & Security → Accessibility** so the global
hotkey and paste work. The hotkey on macOS is **Option + Space**.

## How it works

DevClip opens centered on the window you're working in (on Windows; at the
mouse cursor on macOS and Linux) and defaults to the **Clipboard** view.

### Save
1. Copy text normally (`Ctrl+C`).
2. Open DevClip (`Alt+Space`) — you're on the **Clipboard** view.
3. Select the entry, press `Ctrl+S`. The name is pre-filled from the content.
4. `Enter` — it's now permanently searchable.

### Retrieve
1. `Alt+Space` — search is already focused.
2. Type any fragment. From the default view this searches **both** your
   clipboard history and your saved snippets. Switch to the **Snippets** tab
   (`Tab`) to search snippets only.
3. `↑/↓` to pick, `Enter` to **paste it into the app you were just in**.

| Key | Action |
|-----|--------|
| `Alt+Space` | Show / hide DevClip (global hotkey) |
| type | Fuzzy search (clipboard + snippets on the default view) |
| `↑` / `↓` | Move selection |
| `Enter` | Paste selected into the previously focused app |
| `Tab` | Toggle **Clipboard** ↔ **Snippets** |
| `Ctrl+S` | Save selected clipboard entry (or the live clipboard) as a snippet |
| `Ctrl+Delete` | Delete the selected snippet |
| `Ctrl+,` | Open Settings (storage path, clipboard history limit) |
| `Esc` | Clear the query, then hide |

### Settings (`Ctrl+,`)
An in-app panel to view/change the **data storage file path** and the
**clipboard history limit**. Changing the path switches to the database at the
new location (existing data is copied over if nothing is there yet).

### Sync (optional)
LAN sync is **off by default**. A status chip at the bottom-right shows whether
it's on: left-click to toggle it (enabling shows a one-time risk warning), or
right-click to open the sync panel — name this device, scan for other DevClip
machines on your network, and sync snippets and clipboard history directly, with
no server or account. While sync is on, any DevClip on the network can connect
and receive your data, so enable it only on trusted networks.

## Architecture

A pure-Rust, fully-tested core; a thin Tauri shell; a vanilla-TS UI.

```
frontend (TS/Vite)  ──IPC──▶  src-tauri (shell)  ──▶  devclip-core (logic + SQLite)
```

- **`crates/devclip-core`** — model, fuzzy matcher, ranking, SQLite store, and
  the sync merge logic. No GUI deps; **37 unit tests**. This is where search
  quality lives.
- **`src-tauri`** — global hotkey, clipboard polling, paste injection, LAN sync
  (discovery + transfer), IPC.
- **`src/`** — the command-palette UI (also runs in a plain browser against an
  in-memory mock for fast iteration).

See [`docs/DESIGN.md`](docs/DESIGN.md) for the full design: architecture, UI
flow, UX trade-offs, stack rationale, and the phased plan.

## Tech stack

Tauri v2 · Rust · `rusqlite` (bundled SQLite) · TypeScript + Vite (no UI
framework) · `enigo` (synthetic paste). Chosen for **fast startup** and a tiny
footprint — see DESIGN §4.

## Development

Prerequisites: **Node 18+** and the **Rust** toolchain.

```bash
# 1. Frontend deps
npm install

# 2. Run the fully-tested core logic (no GUI needed)
cargo test --workspace

# 3. Develop the UI in a browser (mock backend, sample data)
npm run dev            # http://localhost:1420

# 4. Run the real desktop app
npm run tauri dev
```

### Linux build dependencies

The Tauri shell needs the system webview + GTK libs:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

(macOS and Windows need only their standard toolchains.)

### Build a release binary

```bash
npm run tauri build
```

### Regenerate icons

The icon set is checked in. To regenerate from a source image:

```bash
npm run tauri icon path/to/icon.png
```

## Status & roadmap

DevClip is ready for daily use and ships installers for **Windows and macOS**
via GitHub Releases. Shipped so far: Clipboard-first unified search with
recency/frequency ranking, an in-app settings panel, reliable Windows paste
(focus-restore + `SendInput`), active-window positioning on Windows, and
**optional LAN sync** (off by default) for snippets and clipboard history.

Planned next: a configurable hotkey, snippet variables (`{{date}}`,
`{{clipboard}}`, typed prompts), clipboard-entry expiration, AI-assisted naming
and semantic search, code-signed installers, and team snippet sharing. The
core/shell split and versioned schema leave room for these without a rewrite.

## License

MIT
