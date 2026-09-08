import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

// The Tauri webview is the only consumer of this build, so the dev server is pinned to a
// port the Rust side names in tauri.conf.json (`devUrl`) and `strictPort` makes a clash
// fail loudly instead of moving the app to a port the webview will not load.
export default defineConfig({
  plugins: [svelte()],
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
    // the bundler from emitting syntax the older of the two would refuse.
    target: 'chrome105',
  },
  // Under vitest the components must resolve to Svelte's client build, not to its server
  // one: without this a rendered component produces a string and never a DOM node.
  resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined,
  test: {
    // The components are written for a webview, so they are tested in a DOM. Nothing here
    // touches Tauri: `bridge()` answers with the no-op implementation outside a webview,
    // and a test that wants to observe a call installs its own fake with `setBridge`.
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    restoreMocks: true,
  },
});
