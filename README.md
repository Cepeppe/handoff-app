# handoff-app

Baton is the desktop overlay application of the contextual handoff system: it shows the
work a coding agent hands over to the person at the machine, guides it one step at a
time, and sends back a structured outcome, keeping screenshots, redaction, the local log
and the runbooks on the machine. It consumes `handoff-mcp` via a pinned release artifact
(`server.lock.json`, verified and unpacked into `vendor/`), never from source. This
repository is proprietary; see `LICENSE`. Status: work in progress, nothing is stable
yet.

## Development

### Prerequisites

| Tool | Notes |
|---|---|
| Rust stable, `x86_64-pc-windows-msvc` | with `clippy` and `rustfmt` (`rustup component add clippy rustfmt`) |
| Visual Studio 2022 Build Tools | the **Desktop development with C++** workload and the Windows 11 SDK |
| CMake and LLVM | `bindgen` needs them; if it cannot find `libclang`, set `LIBCLANG_PATH` to the LLVM `bin` folder |
| Node 22 or newer, pnpm 11 | |
| WebView2 runtime | present on an up-to-date Windows 11 |
| `cargo-deny` | `cargo install cargo-deny --locked`, for the dependency policy check |

The Tauri CLI comes with `pnpm install` (`@tauri-apps/cli`), so `pnpm tauri …` needs
nothing else; `cargo install tauri-cli --version "^2"` gives the same commands as
`cargo tauri …`.

### First run

```sh
pnpm install
node scripts/fetch-server.mjs      # downloads and verifies the pinned server
pnpm tauri dev
```

`fetch-server` needs a token that can read the private `handoff-mcp` repository; it takes
one from `HANDOFF_MCP_READ_TOKEN`, from `GH_TOKEN`, or from `gh auth token`. To work
against a local build of the server instead, run `scripts/dev-link` from the workspace
root: it fills the same layout from `../handoff-mcp` and marks the version `dev-<sha>`,
which the build accepts everywhere except in CI.

Every `tauri` build first runs `node scripts/fetch-server.mjs --check`. It refuses to
build when `vendor/` is missing or holds a version other than the one `server.lock.json`
pins, so a packaged build can only ever contain the pinned server.

### Checks

The four commands CI runs, from `src-tauri/`:

```sh
cargo fmt --check
cargo lint          # alias for: clippy --all-targets --all-features -- -D warnings
cargo test
cargo deny check
```

and, from the repository root, `pnpm build` (frontend) and `pnpm tauri build --debug`.

### Layout

```
src/                 frontend (TypeScript, Vite; Svelte arrives with the first views)
src-tauri/src/       Rust core, one module per area:
                       channel  sessions  store  hook  requests  capture  ocr
                       redaction  log  runbooks  net/egress  install
                       license  crash  i18n  ui_bridge  paths
src-tauri/binaries/  the pinned server, named for Tauri (git-ignored)
vendor/handoff-mcp/  the unpacked release artifact: binary and format files (git-ignored)
scripts/             fetch-server and the pinning documentation
```

Two rules the layout depends on, both explained at the top of `src-tauri/src/lib.rs`:

- **the core never depends on `tauri::AppHandle`.** Side effects — clipboard,
  notification, focus, opener, capture, the socket — go behind traits, implemented over
  Tauri in `ui_bridge` and over fakes in tests, so `cargo test` runs the whole core
  without a webview.
- **only `net::egress` may open a network connection.** `clippy.toml` disallows the HTTP
  and TCP types everywhere, `deny.toml` refuses the HTTP crates as dependencies, and the
  webview CSP is `default-src 'self'` with no `connect-src`. The application makes zero
  network connections today.

### Cargo features

| Feature | State |
|---|---|
| `e2e` | the automation channel that lets the e2e suite play the user; empty for now, never enabled in a release build |
| `fake-capture` | a capture backend returning a fixture image; empty for now |
| `secrets-write` | reserved extension point for writing `.env`-style files; empty, and off in every v1 build |
