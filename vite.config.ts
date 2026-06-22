import { defineConfig } from "vite";

// A human-readable build identifier, shown in the UI so we can confirm exactly
// which build is running. CI sets DEVCLIP_BUILD (run number + short SHA).
const build = process.env.DEVCLIP_BUILD || "dev";

// DevClip frontend build config.
// The fixed port lets Tauri's dev server (`devUrl`) attach reliably.
export default defineConfig({
  define: {
    __DEVCLIP_BUILD__: JSON.stringify(build),
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    // Tauri ships a modern webview (WebView2 / recent WebKit), so we can target
    // esnext — this also enables top-level await used to lazy-load Tauri APIs.
    target: "esnext",
    // Smaller, faster-to-parse bundle => snappier first paint.
    minify: "esbuild",
    sourcemap: false,
  },
});
