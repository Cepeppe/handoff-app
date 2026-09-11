# handoff-app

Baton is the desktop overlay application of the contextual handoff system: it shows the
work a coding agent hands over to the person at the machine, guides it one step at a
time, and sends back a structured outcome, keeping screenshots, redaction, the local log
and the runbooks on the machine. It consumes `handoff-mcp` via a pinned release artifact
(`server.lock.json`, verified and unpacked into `vendor/`), never from source. This
repository is proprietary; see `LICENSE`. Status: work in progress, nothing is stable
yet.

The user documentation — installing, the consent screen, the overlay, screenshots, runbooks,
the log, how to verify what Baton sends, troubleshooting and the third-party notices — is in
[`docs/`](docs/index.md), in English and in Italian (`docs/it/`).

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

`cargo test` includes the security suite of §11.7 (`src-tauri/tests/security/`). Run it
alone with `cargo test --test security -- --nocapture` to see its numbers; it writes the
machine-readable `src-tauri/target/security-report.json`, which the `security` job of CI keeps
as an artifact. [`docs/dev/testing.md`](docs/dev/testing.md) says what every suite proves and
what is checked by hand.

and, from the repository root:

```sh
pnpm test           # frontend unit and component tests (vitest, jsdom)
pnpm check          # svelte-check over the components and the TypeScript
pnpm build          # svelte-check + the production frontend bundle
pnpm tauri build --debug
pnpm check:links    # every relative link of the Markdown resolves
node scripts/third-party-notices.mjs --check   # the crate list of the notices is current
```

CI runs all of them on a push that changes code. A push that changes only documentation runs
the link check and the documentation tests alone, and its Windows jobs show as skipped:
[`docs/dev/testing.md`](docs/dev/testing.md#what-runs-on-which-push) says what counts as
documentation.

After adding or bumping a Rust dependency, `pnpm notices` rewrites the crate list of
`docs/third-party-notices.md` and `docs/it/third-party-notices.md`.

The WebDriver suite drives the real window — the step view, collapse, the request sheet, the
preview, the settings, the consent screen — and runs in its own CI job, `ui`. By hand, on
Windows, about a minute:

```sh
pnpm tauri build --debug --no-bundle --features e2e
pnpm test:ui -- --setup   # once: tauri-driver, and the msedgedriver of this machine's WebView2
pnpm test:ui
```

[`docs/dev/ui.md`](docs/dev/ui.md) is the harness, the three gestures it cannot make and what
stands in for them, and its traps.

And, when the change touches the channel, the store, the state machine, the hook decision,
the capture pipeline or the tool contract, the end-to-end suite — ten scenarios against a
**real** Claude Code and a real build of the app, about four minutes, and a subset of five
against a real Codex CLI, a real OpenCode, the real Cursor Agent CLI or the real GitHub Copilot
CLI, with one scenario more for Cursor's editor or for VS Code. It is run by hand, not in CI:

```sh
scripts\e2e.ps1                   # from the workspace root: builds everything, then runs pnpm e2e
scripts\e2e.ps1 -Agent codex      # the Codex subset
scripts\e2e.ps1 -Agent opencode   # the OpenCode subset
scripts\e2e.ps1 -Agent cursor     # the Cursor subset
scripts\e2e.ps1 -Agent copilot    # the GitHub Copilot subset
```

`docs/dev/e2e.md` is the harness, the traps and how to read a failure. The automation channel
it drives is compiled only with `--features e2e`; `scripts/check-no-automation.mjs` proves a
build without the feature carries none of it, on every push in CI that changes code and on
the binary the release workflow publishes.

### Releasing

A release is a tag. Bump the version in `src-tauri/Cargo.toml` and `package.json` (cargo
rewrites `src-tauri/Cargo.lock` on the next build), write its `## [<version>] - <date>`
section in `CHANGELOG.md`, commit, and push the tag `v<version>`.
`.github/workflows/release.yml` then:

- refuses a tag that is not that version, or a version with no changelog section, before
  building anything (`node scripts/release-version.mjs <tag>` and
  `node scripts/changelog-section.mjs <version>` are the same two checks, by hand);
- builds the unsigned per-user setup from the pinned server, with the hooks of
  `installer/windows/hooks.nsh` inside it — the in-place server update of FM-24;
- runs the automation-channel gate on the binary it packed, and its positive control;
- runs the security suite on the tagged commit;
- leaves a **draft** release carrying `Baton-<version>-win32-x64-setup.exe`,
  `Baton-<version>-security-report.json` and `SHA256SUMS`, with the changelog section as its
  notes.

Publishing the draft is done by hand. Code signing is deferred, so the setup meets
SmartScreen: `docs/install-windows.md` is what a user reads about it.

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
installer/windows/   the NSIS hooks Tauri's Windows installer runs (FM-24), and their test driver
scripts/             fetch-server and the pinning documentation, the link check, the notices,
                       the two release checks
CHANGELOG.md         one section per release; the release workflow reads its notes from it
docs/                the user documentation (English, Italian in it/); dev/ the test harnesses
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
  webview CSP is `default-src 'self'` with no `connect-src`. The application's own code
  makes zero network connections today. The WebView2 runtime that draws the window is
  another program with connections of its own: every window starts it with the switches of
  `ui_bridge::WEBVIEW2_BROWSER_ARGS`, and `docs/verify-trust.md` says what remains.

### OCR

Every capture is read locally before anything can be sent, because the secret detection is
built on the text (`OCR-01`). `src-tauri/src/ocr/` holds one `OcrEngine` trait, one engine
per file behind it, and the selection rule: the operating system's engine when it is
available, the bundled one after it, and an engine that errors or takes longer than ten
seconds falls through to the next. Nothing leaves the machine — no engine here is a service.

| Engine | State |
|---|---|
| `windows` | `Windows.Media.Ocr`, the platform's own recogniser. Available only for a language whose **OCR language pack** is installed; a machine without one falls through, which is what the bundled engine is for. |
| `vision` | macOS `VNRecognizeTextRequest`. A stub answering "unavailable" until T-059; macOS is deferred. |
| `ocrs` | The bundled fallback (`OCR-03`): the pure-Rust `ocrs` engine, English models only. It answers whenever the engine above cannot, which on Windows is a machine with no OCR language pack. |

The two `ocrs` models are committed unmodified under `src-tauri/models/ocrs/` and shipped
as Tauri resources (`bundle.resources`), with the CC-BY-SA 4.0 attribution their licence
asks for in `src-tauri/models/ocrs/LICENSE`. Tesseract was the engine `OCR-03` named until
2026-09-09: no published crate links it statically from a vendored source, and the ones
that build it from source download it at build time with `reqwest`, which `deny.toml`
refuses. `ocrs` and `rten` are pure Rust, so there is **no build prerequisite** for OCR
beyond the ones above and nothing is fetched during a build.

At run time the engine looks for the models beside the executable, which is where Tauri
puts a resource on Windows (`../Resources` inside the `.app` on macOS). A build that was
not bundled — `cargo tauri dev`, `cargo test`, `cargo build` — has no such copy, so the
lookup then walks up from the executable to the one committed in `src-tauri/`.

### Detection and redaction

Everything that reaches an agent from a capture goes through `src-tauri/src/redaction/`:

| Module | What it decides |
|---|---|
| `certain` | the public patterns of the pinned server artifact, over a spec at ingress and over a capture |
| `suspected` | the app's own heuristics: long hexadecimal, base64-looking, high-entropy, and a value beside a `key`/`secret`/`token`/`password` label |
| `boxes` | what they found, as rectangles in the original image, plus the plan the preview edits |
| `burn` | crop, downscale to 1600 px, rescale, expand by 2 px, fill black, encode |
| `typed` | the same two detectors over what the user writes in a sheet |

Two rules decide who may lift what, and they come straight from `DET-01`. A **certain**
match is redacted automatically and the user cannot put it back: the patterns are
precision-first by policy, so a false positive there is a defect of the pattern file. A
**suspected** match is a heuristic and the user decides — in the preview by unlocking the
box in one click, in a sheet by editing their own sentence, where the words are marked and
sent as they were written.

A redaction box covers a **whole OCR line**, because a line is the one unit every engine
reports faithfully and nothing in it says where inside the line a character sits. And the
burn happens **after** the downscale, never before: filling first leaves the resampling
kernel a grey halo in the shape of the letters, which is exactly what `CAP-06` forbids.

### The preview is the only way out

`src-tauri/src/ui_bridge/preview.rs` is the one path a capture has to an agent, and the
window draws what it answers. There is no "send without preview" (`PREV-01`): the picture
appears the instant the capture ends, the OCR and both detectors run behind it while the two
send buttons are disabled (`OCR-04`), and the boxes drawn over the image are the rectangles
the burn will fill.

The burn happens on **this** side and never in the webview, which is the one part of the
application that must not be believed about what may leave the machine: the window sends an
edit — unlock, add a box, crop — and gets the drawing back, and a plan that had been
tampered with still cannot lift a locked box. **Send image** and **Send text** sit side by
side with no default, and the image button is not drawn at all for a session whose
capability row says the agent cannot read one (`PREV-04`, `FM-05`).

What is recorded is the `sends` row of `LOG-03`: the text exactly as it left, or an image's
hash, size and boxes. Never pixels — there is no column to put them in.

### The screenshot corpus

`src-tauri/tests/fixtures/screenshots/` is a synthetic corpus: 46 dashboard-looking pages
drawn by a program, with a fake key of every family, the four suspected shapes, and the
negatives a heuristic is most likely to get wrong. **No real screenshot and no issued
credential is in this repository.** `labels.json` says what every line is, written by hand
in `src-tauri/tests/corpus/mod.rs` and never by running a detector: a corpus derived from
the detector would agree with it whatever it did.

```bash
cargo test --test gen_corpus                          # the committed images still match
HANDOFF_WRITE_CORPUS=1 cargo test --test gen_corpus   # redraw them after changing a page
cargo test --test security metrics -- --nocapture     # the numbers of §11.7
```

The pages are drawn with a stroke font written in `src-tauri/tests/corpus/font.rs` rather
than with a system font, so the same source produces the same pixels on every machine and
the images can be committed and checked back. `tests/security/redaction.rs` then asks four
things of them: whether the detectors say what the corpus says, whether the burn covers
every ink pixel of a redacted line, whether burning in the wrong order would be noticed,
and whether an OCR engine can still read a planted key out of the redacted image.

### Cargo features

| Feature | State |
|---|---|
| `e2e` | the automation channel that lets the e2e suite play the user; empty for now, never enabled in a release build |
| `fake-capture` | a capture backend returning a fixture image; empty for now |
| `secrets-write` | reserved extension point for writing `.env`-style files; empty, and off in every v1 build |
