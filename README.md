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

and, from the repository root:

```sh
pnpm test           # frontend unit and component tests (vitest, jsdom)
pnpm check          # svelte-check over the components and the TypeScript
pnpm build          # svelte-check + the production frontend bundle
pnpm tauri build --debug
```

And, when the change touches the channel, the store, the state machine, the hook decision or
the tool contract, the end-to-end suite — nine scenarios against a **real** Claude Code and a
real build of the app, about four minutes. It is run by hand, not in CI:

```sh
scripts\e2e.ps1     # from the workspace root: builds everything, then runs pnpm e2e
```

`docs/dev/e2e.md` is the harness, the traps and how to read a failure. The automation channel
it drives is compiled only with `--features e2e`; `scripts/check-no-automation.mjs` proves a
build without the feature carries none of it, on every push in CI and on the binary the
release workflow publishes.

### Layout

```
src/                 frontend (Svelte 5, TypeScript, Vite):
                       App.svelte      the one window, switching between the views
                       views/          one placeholder component per view of the design
                       bridge.ts       the typed, mockable `invoke` / `listen` seam
                       i18n.ts         language resolution and the text lookup
                       locales/        en.json and it.json, the product's only texts
                       styles.css      the only stylesheet (the CSP forbids injected ones)
                       __tests__/      vitest + @testing-library/svelte
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
- **the user-visible texts live in one place.** `src/locales/{en,it}.json` are the whole
  catalogue: the frontend imports them and the Rust side compiles the same two files in
  (`src-tauri/src/i18n.rs`) for the texts it owns, so the tray menu and the window are
  never translated twice and one key-parity test covers both.
- **only `net::egress` may open a network connection.** `clippy.toml` disallows the HTTP
  and TCP types everywhere, `deny.toml` refuses the HTTP crates as dependencies, and the
  webview CSP is `default-src 'self'` with no `connect-src`. The application makes zero
  network connections today.

### OCR

Every capture is read locally before anything can be sent, because the secret detection is
built on the text (`OCR-01`). `src-tauri/src/ocr/` holds one `OcrEngine` trait, the engine
of the operating system behind it, and the selection rule: the OS engine when it is
available, the bundled one after it, and an engine that errors or takes longer than ten
seconds falls through to the next. Nothing leaves the machine — no engine here is a service.

| Engine | State |
|---|---|
| `windows` | `Windows.Media.Ocr`, the platform's own recogniser. Available only for a language whose **OCR language pack** is installed; a machine without one falls through, which is what the bundled engine is for. |
| `vision` | macOS `VNRecognizeTextRequest`. A stub answering "unavailable" until T-059; macOS is deferred. |
| bundled fallback | **Not chosen yet.** No published crate links Tesseract statically from a vendored source: the four that build it from source download it at build time with `reqwest`, which `deny.toml` refuses; the two `-sys` crates need a system install; and the one prebuilt crate has no Windows target. Until the owner decides, a Windows machine with no OCR language pack has no OCR at all. |

There is therefore **no build prerequisite** for OCR beyond the ones above, and no
`tessdata/` resource in `tauri.conf.json` — Tauri refuses a resource glob that matches
nothing, so the entry arrives with the file it points at.

### Cargo features

| Feature | State |
|---|---|
| `e2e` | the automation channel that lets the e2e suite play the user; empty for now, never enabled in a release build |
| `fake-capture` | a capture backend returning a fixture image; empty for now |
| `secrets-write` | reserved extension point for writing `.env`-style files; empty, and off in every v1 build |
