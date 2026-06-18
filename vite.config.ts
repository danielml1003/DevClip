import { defineConfig } from "vite";

// DevClip frontend build config.
// The fixed port lets Tauri's dev server (`devUrl`) attach reliably.
export default defineConfig({
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
