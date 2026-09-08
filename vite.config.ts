import { defineConfig } from 'vite';

// The Tauri webview is the only consumer of this build, so the dev server is pinned to a
// port the Rust side names in tauri.conf.json (`devUrl`) and `strictPort` makes a clash
// fail loudly instead of moving the app to a port the webview will not load.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // `cargo tauri dev` rebuilds the Rust side itself; watching it here would restart Vite
    // on every incremental compile.
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // WebView2 on Windows and WKWebView on macOS both sit far above this; it only keeps
    // esbuild from emitting syntax the older of the two would refuse.
    target: 'chrome105',
  },
});
