# The WebDriver suite: the window, pressed by a program

`pnpm test:ui` drives the **real window** of a real build: `tauri-driver` starts the
application through `msedgedriver`, and each scenario clicks, types and drags in its WebView2
while a fake server session plays the agent on the real channel. It is the design's §11.4, and
the one suite whose subject is the window itself.

The other suites cannot be that, by construction. The component tests draw the Svelte
components into jsdom behind a fake bridge, so the IPC, the events and the layout are stubs.
The e2e suite ([e2e.md](e2e.md)) runs the whole system with a real agent, but it plays the user
through the automation channel and its build never loads the frontend at all. Everything
between the two — a button that calls the wrong command, an event the window never listens
to, a panel that does not resize — is what this suite is for.

- [What it runs](#what-it-runs)
- [Running it](#running-it)
- [How a scenario works](#how-a-scenario-works)
- [Three gestures WebDriver cannot make](#three-gestures-webdriver-cannot-make)
- [The flaky-test guard and the evidence](#the-flaky-test-guard-and-the-evidence)
- [In CI](#in-ci)
- [Traps](#traps)

## What it runs

| Scenario | Requirements | What it proves |
|---|---|---|
| `step-view` | GUIDE-01..03, DET-04, PRIN-07 | the counter and no progress bar; the one `https://` address of a step is a link and an `http://` or `file://` one is not; **Copy** puts a value on the clipboard, and for a secret-treated value it is the true value while the chip shows the mask; the last **Done** ends the round |
| `collapse` | WIN-03, WIN-02 | losing the focus shrinks the panel to the current step and Done, Ask, Screenshot, and the window to the bar; Done works from the bar; a click on the bar, or the focus coming back, restores the panel |
| `request-sheet` | OPEN-04, OPEN-04a | with no session the sheet opens and says so; the caret is in the field without a click; Esc queues nothing; the only session is pre-selected; Enter on an empty field sends nothing; Enter sends and the tab appears waiting for its spec |
| `preview` | PREV-01, PREV-02, PREV-04, CAP-01 | Screenshot offers two choices; both send buttons, neither the default; a flagged box lifted and put back; a box drawn by hand, which offers no control; a drag shorter than four pixels adds nothing; crop and undo; the burned PNG reaches the agent |
| `preview-text-only` | PREV-03, PREV-04, FM-05 | a session whose agent cannot read images is not offered **Send image** at all, and the text is what reaches the agent |
| `settings` | APP-02, LOG-04, WIN-02 | the six pages in the order §7.6 names them; the Log page lists the handoff just closed; the language changes every label at once; the panel is wider while the settings are open |
| `onboarding-consent` | F-13, INST-01, INST-02 | a first launch shows onboarding; the consent screen counts three modifications on two rows, each **Show** opens its diff; nothing is written without Accept |

The code is under `tests/ui/`: `main.ts` runs the scenarios of `scenarios/`, `app.ts` starts
one isolated application, `fake-server.ts` is the session, `webdriver.ts` the client,
`tools.ts` the drivers, and `scenario.ts` the page the scenarios press.

## Running it

It runs on Windows: `tauri-driver` has no driver for the macOS webview, and macOS is deferred
(`TASKS.md` §0.4 item 7). Three things have to exist, and the suite names the one that does not:

1. **The build it drives**, a debug build with the automation channel and the frontend built
   in:

   ```sh
   pnpm tauri build --debug --no-bundle --features e2e
   ```

   `tauri build` and not `cargo build`: Tauri embeds `dist/` only under a feature that
   `tauri build` turns on, and a plain `cargo build` produces a window that looks for the Vite
   dev server and draws an error page. The suite checks the page it lands on and says so.
2. **The two drivers**, fetched once into `tests/ui/.cache/` (git-ignored):

   ```sh
   pnpm test:ui -- --setup
   ```

   `tauri-driver` is pinned (`tests/ui/tools.ts`) and installed with `cargo install --locked
   --root` into that folder, never into `~/.cargo/bin`. `msedgedriver` has to be the build of
   the machine's WebView2 runtime, which updates itself with Windows, so its version is read
   from the registry and the matching driver is downloaded from Microsoft's own host. Run
   `--setup` again after a WebView2 update.
3. **No other Baton running.** The single-instance lock is keyed on the bundle identifier, so
   a launch while another Baton runs would hand itself over to that one. The suite refuses to
   start instead.

Then:

```sh
pnpm test:ui                          # all seven, about a minute
pnpm test:ui -- collapse preview      # only these
pnpm test:ui -- --list
```

The window appears on the desktop, always on top, for each scenario. The copy chips write the
real clipboard, and the request sheet may raise a Windows notification (FM-21): run it when
the desktop is not in use.

Environment: `HANDOFF_UI_APP` points at another build, `HANDOFF_UI_KEEP=1` keeps each attempt's
temporary root, `HANDOFF_UI_RUST_LOG` changes what the application logs, and
`HANDOFF_UI_TAURI_DRIVER` / `HANDOFF_UI_MSEDGEDRIVER` point at drivers of your own.

## How a scenario works

1. A temporary root with its own `HANDOFF_HOME` and `HANDOFF_APP_DATA_DIR`, a `bin/` put first
   on the application's `PATH`, and a `tmp/` that is the drivers' `TEMP`: `msedgedriver`
   gives the WebView2 a profile folder of its own making there (`scoped_dir…\EBWebView`) with
   `--remote-debugging-port=0`, and waits for the runtime to write the port into a
   `DevToolsActivePort` file in it. That file is how the driver finds the window.
2. `tauri-driver` is started with that environment — the application is `msedgedriver`'s child
   and inherits it — and a WebDriver session starts the application.
3. Through the automation channel of the `e2e` build, the **language** is set to English and,
   unless the scenario is about onboarding, **onboarding** is marked as done; the page is
   reloaded so the window reads both the way it does at every launch. The first is why the
   scenarios can look for the words a person reads, on the runner and on an Italian desktop
   alike. The second is what onboarding's own last button writes — pressing it would also
   answer its autostart question, which writes the machine's real login items.
4. The scenario registers the fake sessions it needs (`fake-server.ts`: `hello` with the
   capability row it chooses, `handoff.open` with a spec), and plays the user.
5. The application is quit, the session deleted, the drivers stopped, the root removed.

**Every wait is a condition.** Nothing sleeps for a duration and then looks: a wait names what
it is waiting for, re-reads it until it holds, and gives up after a bound with that name and
the last error it saw (`tests/ui/wait.ts`).

**The preview's screenshot is a fixture.** The e2e build's capture backend returns a page of
the synthetic corpus of §11.7 (`src-tauri/tests/fixtures/screenshots/`) instead of the screen,
chosen through the settings key `e2e.capture_fixture`. No real screen is ever captured.

## Three gestures WebDriver cannot make

WebDriver drives the page. It cannot reach the tray icon, press a global shortcut, or give the
focus to another window — and each of those ends in the Rust side emitting one event to the
window: `ui://show-view` with the view the tray item or the shortcut asked for, and
`ui://window-focus` with the focus Tauri reported. Everything the user then sees is the
window's reaction to that event, so the suite emits exactly those two events from the page,
through the event plugin the window's capability already grants, and asserts on the real
window afterwards. What it leaves untested is the Rust half of each gesture: one `emit` call,
whose event names a Rust test keeps equal to the frontend's.

## The flaky-test guard and the evidence

A scenario that fails is run once more, from a fresh application. A pass on the second
attempt is reported as **flaky** rather than as a pass, so a guard that is doing work shows.
Every failed attempt leaves four files in `tests/ui/results/` (git-ignored): the error, a
screenshot of the window, its DOM, and what the drivers and the application printed. The
report of the whole run is `tests/ui/results/last-run.json`.

Two things keep a broken machine from costing a whole job. A session that has not started
after twenty seconds gets the machine photographed while the driver is still waiting — the
processes of the drivers, of the application and of its WebView2 browser with their command
lines, whether the session's profile has its `DevToolsActivePort` yet, the tail of the
runtime's own `chrome_debug.log`, and any WebView2 or Edge policy — and that goes into the
error of the attempt (`app.ts`, `snapshot`). And a scenario whose every attempt died before
the window existed stops the run: the scenarios after it are reported **not run**, because
they would spend the same minutes learning the same thing.

Exit codes: 0 every scenario passed (flaky ones included), 1 at least one failed twice or was
not run, 2 the suite could not run.

## In CI

The `ui` job of `ci.yml` runs on `windows-latest` on every push that changes code
([testing.md](testing.md#what-runs-on-which-push)), on the runner's desktop session. It builds the application itself (neither binary of the `windows` job is one it can
drive), restores the `windows` job's Rust cache read-only, keeps `tauri-driver` in a cache of
its own, fetches the `msedgedriver` of the runner's WebView2, and uploads `tests/ui/results/`
as the artifact `ui-results` whatever happened.

## Traps

Each of these cost a run while the suite was written.

- **Not every WebView2 build keeps the driver's switches.** `msedgedriver` hands the runtime
  `--remote-debugging-port=0`, `--enable-automation` and the rest through
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, and the application passes switches of its own
  (T-052). Runtime 152 merges the two; the 151 of the CI runner kept the application's and
  dropped the driver's, so no port was opened and every session failed after sixty seconds
  with `DevToolsActivePort file doesn't exist` — the browser process's command line in the
  attempt's snapshot is what showed it. An e2e build now merges them itself
  (`src-tauri/src/e2e/webdriver.rs`); a release build is untouched.
- **Redirecting `USERPROFILE` breaks every session.** It is where the installation adapter
  finds `~/.claude.json`, and the obvious way to keep a scenario off the machine's real one.
  But WebView2 and the Windows components it loads follow it too, and in a fresh profile folder
  `msedgedriver` never finds the WebView2 it started: `session not created: DevToolsActivePort
  file doesn't exist`, after exactly sixty seconds, every time. The suite leaves it alone; the
  onboarding scenario makes Claude Code "found" with a stand-in `claude.cmd` in its `bin/`,
  never presses Accept, and checks the machine's two files are byte for byte what they were.
- **`WEBVIEW2_USER_DATA_FOLDER` is ignored under the drivers.** `msedgedriver` hands the
  runtime a `--user-data-dir` of its own making under `TEMP`, and that wins. Point `TEMP` at a
  folder of your own instead, as the harness does, to know where the profile and its
  `chrome_debug.log` are.
- **The preview's four pixels are the capture's.** The canvas draws a 1300-pixel page at about
  a third of its size, so a "two-pixel" drag on screen is a box of about eight. A drag meant to
  be too short is sized from the scale on screen.
- **The panel collapses whenever the operating system takes the focus off it**, and on a
  desktop something always can: a capture hides the panel and shows it again, and a copy to
  the clipboard was followed by a lost focus here about once in five runs. WIN-03 then does
  what it says, and the bar that replaces the window has no `[data-view]` and none of the
  panel's buttons, so a wait for either never ends. The page clicks the bar, as a person
  would, and counts it (`panelReopened` in the report); the collapse scenario, which is about
  exactly that, switches it off (`Page.keepPanelOpen`).
- **A console program the harness starts can collapse the panel.** The suite's Node has no
  console of its own when a tool or a runner starts it, so every `powershell.exe` or
  `tasklist` it spawns gets a new console window, and that window takes the focus off the
  panel — WIN-03, again. Reading the clipboard is several such spawns in a row, and the next
  click found the bar instead of the Done it was aimed at. Every child is started with
  `windowsHide: true`; anything added to the harness that starts a process needs the same.
- **`hello` is answered before the session is in the registry.** A request sheet opened in that
  moment reads "no active session" and keeps it, because it reads the list when it opens. The
  harness waits until the automation channel reports the session connected.
- **A frontend-only change does not relink the build** (`HANDOFF.md`, T-049). After touching
  `src/` alone, `touch src-tauri/src/lib.rs` before the build, or the window under test is the
  previous one.
- **Smart App Control** refuses a freshly linked executable at random on the development
  machine. The session start is retried; see [smoke.md](smoke.md#traps) for the remedies when
  the build itself is refused.
