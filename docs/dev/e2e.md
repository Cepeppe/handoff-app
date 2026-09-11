# The end-to-end suite: Baton, a real agent, and nobody at the keyboard

`pnpm e2e` runs the scenarios of the design's §11.5 against a **real Claude Code** and a
**real build of the app**, with the person at the machine played by an automation channel
that only exists in an `--features e2e` build. `pnpm e2e -- --agent codex` runs a subset of
them against a **real Codex CLI** instead ([The Codex subset](#the-codex-subset), T-067),
`--agent opencode` the same subset against a **real OpenCode**
([The OpenCode subset](#the-opencode-subset), T-074), `--agent cursor` the same subset
against the **real Cursor Agent CLI**, after one scenario that launches **Cursor's editor**
([The Cursor subset](#the-cursor-subset), T-070), and `--agent copilot` the same subset against
the **real GitHub Copilot CLI**, after one scenario that launches **VS Code**
([The GitHub Copilot subset](#the-github-copilot-subset), T-072).

It is the only check that exercises the whole system at once. `cargo test` drives the core
with a fake server on one side and a fake window on the other; the frontend suite draws
components into jsdom; the server's own suites drive an in-memory MCP client. All of them can
be green while the thing a user would actually do is broken — and twice already they were:
`npx baton-handoff-mcp` never ran the server for eight tasks (T-025), and a hook command
without quotes died on every turn (T-039).

Run it when you change the channel, the store, the state machine, the hook decision, the tool
contract or the installation adapter — and after every Claude Code update, together with the
canary of `handoff-mcp` (`docs/agent-facts.md` there). The suite is run **by hand**: no CI
secret is provisioned for a real agent (`TASKS.md` §0.4 item 9), and `e2e.yml` exists so that
turning it on later is a secret rather than a task.

- [What it runs](#what-it-runs)
- [The Codex subset](#the-codex-subset)
- [The OpenCode subset](#the-opencode-subset)
- [The Cursor subset](#the-cursor-subset)
- [The GitHub Copilot subset](#the-github-copilot-subset)
- [Running it](#running-it)
- [How a scenario works](#how-a-scenario-works)
- [The automation channel](#the-automation-channel)
- [It is not in a shipped build](#it-is-not-in-a-shipped-build)
- [Reading a failure](#reading-a-failure)
- [Traps](#traps)

## What it runs

| Scenario | What it proves |
|---|---|
| `e2e-01-verified` | open → confirm each step → done → `handoff_verify ok:true` → `verified` in the log |
| `e2e-02-question` | Ask from the overlay comes back as `status: question`; the reply lands on the step; one round |
| `e2e-03-screenshot` | a capture of a fixture page reaches the agent as an image block, with the fake Stripe key burned out of it and only its hash in the log |
| `e2e-04-defer` | a deferral comes back as `deferred`; the agent's resume takes the tab back to `active` |
| `e2e-05-parked` | two deferrals park it; the Stop hook stops the agent **once** and names it |
| `e2e-06-correction` | `ok:false` → `replacement_steps` → a second round → `verified` |
| `e2e-07-heartbeat` | a slow user gets `in_progress` at the 50 s floor of §5.6, and the resume re-attaches |
| `e2e-09-request` | a request the user types is delivered by the hook and **adopted** by the handoff |
| `e2e-10-not-verified` | a verification nobody reports times out into `not_verified`; the hook blocks once |
| `e2e-11-second-session` | a second `claude -p` resumes what the first left; a second resume says `already_delivered` |

After **every** scenario, whatever it was about, the log-invariant check of §11.2 runs: the
whole database is dumped column by column and searched for the fixture secrets the scenario
planted. LOG-02 is a property of the log and not of a flow.

One row of the §11.5 table is not here: **E2E-8** (the app stopped → text mode) is the
server's and runs there, in `handoff-mcp`'s canary — its second half, "no database row", has
no app to have a row in.

**E2E-3 borrows its fixture from the corpus of §11.7**, `src-tauri/tests/fixtures/screenshots/`:
a Stripe webhook page drawn by a program, with a fake signing secret on it. No real screen
and no issued credential is ever committed. It also asks the app one question the harness
cannot answer itself — *what does an OCR of the sent PNG still read?* — because the OCR
engines are in the app; the answer comes back with the `screenshot_fixture` action, together
with the families the detectors found in the same capture **before** the burn, which is the
control. On a machine whose engine cannot read the planted key even unredacted, that control
is reported as a failing **note** and the check above it is vacuous rather than green — the
same rule §11.7 takes for the glyph-leak pass.

## The Codex subset

`pnpm e2e -- --agent codex` runs five scenarios against the real Codex CLI (`tests/e2e/codex.ts`),
the subset §13 M7 asks of every adapter:

| Scenario | What it proves for Codex |
|---|---|
| `e2e-01-verified` | the happy path and the verification, exactly as for Claude Code |
| `e2e-02-question` | Ask comes back as `status: question`, and Codex's reply lands on the step |
| `e2e-04-defer` | a deferral comes back as `deferred` with the **no-hook** instruction ("Nothing will remind you"), and Codex's own resume takes the tab back to `active` |
| `e2e-07-heartbeat` | `in_progress` at the 50 s floor, and the resume re-attaches |
| `e2e-09-request-clipboard` | a request typed with no session running reaches Codex through the clipboard — the harness reads the clipboard back and starts Codex with it, as Ctrl+V would — and the handoff adopts its id; no hook row exists |

E2E-9 has its own shape because Codex runs no end-of-turn hook (the `codex` row of
`handoff-mcp` says `stop_hook: false`): the clipboard is the whole of the delivery, and the
person who pastes is the whole of the transport. E2E-5 and E2E-10 are about what the Stop hook
says, which a Codex session never hears. Every Codex scenario also checks that the session
registered as Codex and that its tab is labelled *Codex CLI* — the name the capability row
carries in `hello`, which the overlay never keeps a list of.

Before the scenarios, a **preflight** hands the golden `config.toml` of
`src-tauri/tests/fixtures/install/codex-empty/out/` — byte for byte what the installer writes,
which `tests/install_golden.rs` pins — to the real `codex mcp get` in a throw-away
`CODEX_HOME`, and checks the command, the arguments, both variables, `tool_timeout_sec` and
the approval mode. The scenarios cannot prove that themselves: they must stay off the user's
own configuration, so they declare the server with `-c` overrides holding the same values.

Every `codex exec` runs with the isolation the Codex canary of `handoff-mcp` measured:
`--ignore-user-config` (it keeps the login), `apps` and `plugins` disabled with the browser,
computer-use, image-generation and sub-agent features, `--ephemeral`, and the `read-only`
sandbox. The report is `tests/e2e/results/last-run-codex.json`, beside the Claude Code one.

## The OpenCode subset

`pnpm e2e -- --agent opencode` runs the same five scenarios against the real OpenCode
(`tests/e2e/opencode.ts`). OpenCode has no end-of-turn hook either — the `opencode` row of
`handoff-mcp` says `stop_hook: false` — so the reasons are the Codex subset's: E2E-4 expects the
**no-hook** instruction, E2E-9 runs by the clipboard, and every scenario checks that the session
registered as OpenCode (`clientInfo` `opencode`) and that its tab is labelled *OpenCode*.

Before the scenarios, a **preflight** puts the golden `opencode.json` of
`src-tauri/tests/fixtures/install/opencode-empty/out/` in a throw-away configuration folder and
asks the real `opencode debug config` what it read: a local server, Baton's path alone as its
command, both variables and `"timeout": 1800000`. No model is involved.

Every `opencode run` runs with the isolation the OpenCode canary of `handoff-mcp` measured: our
server declared inline in `OPENCODE_CONFIG_CONTENT`, `XDG_CONFIG_HOME` pointed at an empty
folder of the run so the user's own servers, plugins and permissions never load (the login is
kept), project configuration and Claude Code's files switched off, `PWD` set to the run's
project, and the session each run leaves in OpenCode's history deleted afterwards. The model is
a free OpenRouter model unless `HANDOFF_E2E_OPENCODE_MODEL` names another, so a run costs
nothing. The report is `tests/e2e/results/last-run-opencode.json`.

## The Cursor subset

`pnpm e2e -- --agent cursor` runs six scenarios: one against **Cursor's editor**, then the five
of the Codex subset against the real **Cursor Agent CLI** (`tests/e2e/cursor.ts`). No hook of
Cursor's reaches ours either — the `cursor` row of `handoff-mcp` says `stop_hook: false` — so
the reasons are the Codex subset's.

| Scenario | What it proves for Cursor |
|---|---|
| `cursor-editor-session` | a session Cursor's editor starts registers as the editor (`cursor-vscode`), keyed at the editor (`ancestor_chain:editor`) with the launched editor in its chain, named after the window's folder (*Cursor · baton-cursor-window*) while it runs in the home folder; and the window's title names that folder, which is how a request finds the window |
| `e2e-01`, `02`, `04`, `07`, `09-request-clipboard` | as for Codex, through `agent -p`; each also checks that the session registered as Cursor (`clientInfo` `Cursor`) and that its tab says *Cursor* |

**The editor scenario** launches an editor of the harness's own on a project folder named
`baton-cursor-window`, with a fresh `--user-data-dir` and extensions folder, and with
`USERPROFILE` and `HOME` pointed at a folder of the run whose `.cursor\mcp.json` holds the
installer's entry and the run's `HANDOFF_HOME`. It is a separate instance from the owner's
Cursor, it opens a window for the seconds it takes, and it is closed with everything it
started. It spends no agent request: the editor starts its servers as the window opens, before
any chat — and the editor's chat cannot be driven from a script anyway. It is the recipe of the
Cursor canary of `handoff-mcp` (T-069).

**E2E-7 injects nothing** for Cursor. No entry can raise Cursor's limit and the installer writes
no `HANDOFF_TOOL_TIMEOUT_MS`, so the runner does not either: the row's 60 000 ms, which is the
Agent CLI's own cut, puts the heartbeat at the same 50 s floor, and the report records 60 000 as
the timeout the run had.

Before the scenarios, a **preflight** puts the golden `mcp.json` of
`src-tauri/tests/fixtures/install/cursor-empty/out/` in a throw-away project and asks the real
`agent mcp list` what it read there, with the home folder pointed at an empty one so that the
golden file is the only one it can read. The answer is `handoff: not loaded (needs approval)`:
a server of a project file waits for an approval the preflight never gives. No model is involved
and no request is spent.

Every `agent -p` runs with what the Cursor canary of `handoff-mcp` measured: our server declared
in the run's project, `.cursor\mcp.json`, and approved for that run with `--approve-mcps`; the one
permission rule `Mcp(handoff:*)` in the project's `.cursor\cli.json`, because print mode refuses a
tool that is not read-only, and never `--force`; `--trust`; the model `auto` unless
`HANDOFF_E2E_CURSOR_MODEL` names another. Each run leaves a folder under `~/.cursor/projects` and
a conversation under `~/.cursor/chats`, which the runner deletes; the stream-json transcript is
written to `agent-cursor-<session id>.jsonl` in the run's root first. The report is
`tests/e2e/results/last-run-cursor.json`.

**Each CLI scenario spends one request** of the Cursor account, which the owner keeps on the Free
plan: the subset is run by hand, and rarely — after a Cursor update, or when the adapter changes.
`e2e.yml` has no `cursor` choice for the same reason.

## The GitHub Copilot subset

`pnpm e2e -- --agent copilot` runs six scenarios: one against **VS Code**, whose chat is
Copilot's, then the five of the Codex subset against the real **GitHub Copilot CLI**
(`tests/e2e/copilot.ts`). Both surfaces run hooks, but neither answers ours the way the row's
`stop_hook` would promise — the `copilot` row of `handoff-mcp` says `stop_hook: false` — so the
reasons are the Codex subset's.

| Scenario | What it proves for Copilot |
|---|---|
| `copilot-editor-session` | a session VS Code starts registers as VS Code (`Visual Studio Code`) under the Copilot row, keyed at the editor (`ancestor_chain:editor`) with the launched VS Code in its chain, named after the window's folder (*GitHub Copilot · baton-copilot-window*) — which VS Code gives only as the roots of its MCP client — while it runs in the home folder; and the window's title names that folder, which is how a request finds the window |
| `e2e-01`, `02`, `04`, `07`, `09-request-clipboard` | as for Codex, through `copilot -p`; each also checks that the session registered as Copilot (`clientInfo` `copilot-cli`) and that its tab says *GitHub Copilot* |

**The editor scenario** launches a VS Code of the harness's own on a project folder named
`baton-copilot-window`, with a fresh `--user-data-dir` whose `User\mcp.json` holds the golden
entry of `src-tauri/tests/fixtures/install/copilot-empty/out/` — the pinned server as its command
and the run's `HANDOFF_HOME` beside it — a fresh extensions folder, and `USERPROFILE` and `HOME`
pointed at a folder of the run. VS Code starts no server when a window opens, so a two-file
extension of the harness, loaded in development mode, runs VS Code's own
`workbench.mcp.startServer` once the window is up, with the argument the "Start" link of
`mcp.json` passes, which skips the trust prompt. It spends nothing, it opens a window for the
seconds it takes, and it is closed with everything it started. A development-mode window's title
starts with `[Extension Development Host]`, which a user's window never shows: the title check
reads it without that prefix. It is the recipe of the Copilot canary of `handoff-mcp` (T-072).

**E2E-7 injects its timeout** as it does for Codex: the CLI honours the entry's `timeout` and
cancels a call past it, so the runner writes the scenario's 90 000 ms as both `timeout` and
`HANDOFF_TOOL_TIMEOUT_MS`, and the heartbeat falls at the 50 s floor.

Before the scenarios, a **preflight** puts the golden `mcp-config.json` of the same folder in a
throw-away `COPILOT_HOME`, with an empty home folder beside it, and asks the real
`copilot mcp get handoff --json` what it read: a `local` server, Baton's path alone as its
command, both variables, every tool and `"timeout": 1800000`. No model is involved. VS Code has no
such command; the editor scenario, which hands VS Code the golden entry, is its answer.

Every `copilot -p` runs with what the Copilot canary of `handoff-mcp` measured: the whole Copilot
folder moved into the run with `COPILOT_HOME` — our server in its `mcp-config.json`, the run's
project in its `trustedFolders`, and whatever the run writes, sessions and logs included, goes
with the run (the login is gh's, and is kept); `USERPROFILE` and `HOME` moved too;
`--allow-tool=handoff`, which allows our tools and nothing else, and `--deny-tool` for the shell
and every file write, never `--allow-all-tools`; the built-in GitHub server off, no custom
instructions, no question to the user, no update, no export; the model `auto` unless
`HANDOFF_E2E_COPILOT_MODEL` names another. Under `-p` the CLI does not wait for its servers
before the first model call, and the pinned server's first start on a machine is slow while it
is scanned (20 s the first time, 135 ms after, measured), so the runner starts it once with
`--version` before every run. The transcript is written to `agent-copilot-<session id>.jsonl` in
the run's root. The report is `tests/e2e/results/last-run-copilot.json`.

**Each CLI scenario spends from the Copilot account**, which the owner keeps on the Free plan: the
subset is run by hand, and rarely — after a Copilot or VS Code update, or when the adapter
changes. `e2e.yml` has no `copilot` choice for the same reason.

## Running it

From the workspace root, which builds everything first:

```powershell
scripts\e2e.ps1                    # all ten
scripts\e2e.ps1 e2e-01-verified    # one
scripts\e2e.ps1 -Agent codex       # the Codex subset
scripts\e2e.ps1 -Agent opencode    # the OpenCode subset
scripts\e2e.ps1 -Agent cursor      # the Cursor subset
scripts\e2e.ps1 -Agent copilot     # the GitHub Copilot subset
scripts\e2e.ps1 -DevLink           # against a local build of handoff-mcp
scripts\e2e.ps1 -SkipBuild         # reuse what is already built
```

Or, with the build already done, from `handoff-app`:

```bash
pnpm e2e
pnpm e2e -- e2e-05-parked
pnpm e2e -- --agent codex
pnpm e2e -- --agent opencode
pnpm e2e -- --agent cursor
pnpm e2e -- --agent copilot
pnpm e2e -- --list
```

Four things must exist, and `missingPrerequisites()` names the two it can check:

1. `src-tauri/target/release/handoff-app.exe`, built **with `--features e2e`**;
2. `src-tauri/binaries/handoff-mcp-<triple>.exe`, from `node scripts/fetch-server.mjs`;
3. `dist/`, from `pnpm build` — a release binary loads it, a debug one looks for a Vite dev
   server on port 1420 and comes up empty (the T-040 handoff entry), which is why the suite
   uses the release profile;
4. `claude` on `PATH`, logged in — or `codex` or `opencode`, logged in, for their subsets,
   Cursor's `cursor-agent`, signed in with `agent login`, and Cursor's editor, for the Cursor
   subset, or `copilot`, signed in, and VS Code, for the GitHub Copilot subset.

For the Cursor subset one thing must **not** exist: a `handoff` server in `~/.cursor/mcp.json`.
The Agent CLI reads that file into every run and has no switch to leave it out, so Baton
registered there would load beside the run's server, under the same name, and could open a
scenario's handoff in the Baton you use every day. The suite refuses to start the subset then
(exit 2); remove Baton's registration for Cursor for the length of the run.

Environment: `HANDOFF_E2E_MODEL` pins Claude Code's model (default `sonnet`),
`HANDOFF_E2E_CODEX_MODEL` Codex's (default `gpt-5.6-luna`, at low reasoning effort),
`HANDOFF_E2E_OPENCODE_MODEL` OpenCode's (default `openrouter/thinkingmachines/inkling-small:free`),
`HANDOFF_E2E_CURSOR_MODEL` Cursor's (default `auto`), `HANDOFF_E2E_CURSOR` another Cursor Agent
launcher and `HANDOFF_E2E_CURSOR_EDITOR` another editor executable,
`HANDOFF_E2E_COPILOT_MODEL` Copilot's (default `auto`), `HANDOFF_E2E_COPILOT` another Copilot CLI
and `HANDOFF_E2E_VSCODE` another VS Code executable, `HANDOFF_E2E_KEEP=1` keeps
each run's temporary root, `HANDOFF_E2E_SERVER` points the MCP entry at another server binary,
`HANDOFF_E2E_RUST_LOG` changes what the app logs.

A whole run is about four minutes and a few cents. The report is
`tests/e2e/results/last-run.json` (git-ignored; `last-run-codex.json`, `last-run-opencode.json`,
`last-run-cursor.json` and `last-run-copilot.json` for the subsets):
verdicts, every assertion, the measured facts, and the **transcript ids** — with which the agent's own transcript can be read at
`~/.claude/projects/<slug>/<session-id>.jsonl`, the one place a hook error is written down.

## How a scenario works

Every scenario is the same six moves:

1. a temporary root with its own `HANDOFF_HOME`, `HANDOFF_APP_DATA_DIR` and project folder;
2. the app is started `--hidden` and the harness waits for its automation channel;
3. `.claude.json` and `.claude/settings.json` are written into the project, pointing at the
   **pinned** server binary, with the hook command quoted;
4. `claude -p` is started with a prescriptive prompt carrying the spec verbatim — it is
   **not** awaited: its tool call blocks while the harness plays the user;
5. the harness polls `state()`, presses buttons through `act()`, and waits for what the
   scenario is about;
6. the agent is awaited, the database is read, the assertions are produced.

Nothing is mocked anywhere in that list. The one thing that is not real is the person.

Each assertion declares whether a failure would be a **protocol** failure (the shape of the
run is wrong: a state the store should not be in, a row that is missing) or a **model**
failure (the run was well formed and the model did not do what the prompt asked). Only a
model failure is retried, once, exactly as §11.5 asks. A third kind, **pending**, is an
assertion whose subject is a later task: written and reported, never decisive. No scenario
uses it at the moment — E2E-1 and E2E-6 both ask about the runbook file, and the writer
exists now, so both read what is in it.

## The automation channel

`src-tauri/src/e2e/` — a second local endpoint beside the product channel:

| | |
|---|---|
| Windows | `\\.\pipe\handoff-e2e-<h>`, `h` being the digest the product channel uses |
| elsewhere | `<HANDOFF_HOME>/e2e.sock`, with the `sun_path` fallback and its pointer file |
| token | `<HANDOFF_HOME>/e2e.token`, 64 hex characters, owner-only, compared in constant time |
| published at | `<HANDOFF_HOME>/e2e.endpoint`, written after the bind — the "app is up" signal |
| framing | NDJSON, one JSON-RPC object per line, as the product channel frames its traffic |

Six methods: `auth` (first, or everything else is refused), `state`, `act`, `open_request`,
`settings`, `quit`. `act` takes one action name that is not a button of the window,
`screenshot_fixture`: the button it stands for is four presses — pick the fixture that
replaces the screen, take the capture, lift the flagged boxes the user would lift, send — so
its payload is a JSON object (`path`, `mode`, `scale`, `unlock`, `comment`, `text`) and its
answer is the summary of what left. Each of them is a thin adapter over `ui_bridge::commands` — the same
functions the window calls, with the same managed state — so a scenario that passes has
exercised the path a person exercises. `state` answers with the **view** the window is given,
where secret-treated values are masked: the true values of a spec never cross this socket
either (DET-04), or the log-invariant check would be reading through a leak.

`settings` carries one key that is not in the settings table: `e2e.verifying_timeout_ms`
shortens the verification window of VER-06 for the process, which is what lets E2E-10 watch
thirty minutes pass in twenty seconds. It is never persisted.

## It is not in a shipped build

§11.5: *"It does not exist in release builds (feature flag off; CI asserts the symbol is
absent from release binaries), because a hidden control channel in a trust-sensitive app must
not ship."*

The whole module is behind `--features e2e`, and the two names the endpoint cannot exist
without — `handoff-e2e` and `e2e.sock` — appear in `src/e2e/endpoint.rs` and nowhere else.
`scripts/check-no-automation.mjs` greps a built binary for them, as UTF-8 and as UTF-16:

```bash
node scripts/check-no-automation.mjs src-tauri/target/release/handoff-app.exe            # must be absent
node scripts/check-no-automation.mjs --present src-tauri/target/debug/handoff-app.exe    # must be there
```

`ci.yml` runs both on every push to `main` that changes code: the first on the debug bundle
it has just built
without the feature, which the `cfg` gate makes as clean as a release binary, the second on a
debug build made with the feature. The second is the positive control and it is not
decoration: a grep for a string nobody writes passes for ever, including on the day the grep
itself breaks.

`ci.yml` builds no release binary, so the release binary meets the check in the release
workflow (`.github/workflows/release.yml`), and both halves run there too: the first on the
`src-tauri/target/release/handoff-app.exe` the bundler has just packed into the setup, before
anything else is built over it, the second on a release build made afterwards with
`pnpm tauri build --features e2e --no-bundle`, which overwrites that same path. Either one
failing fails the release.

## Reading a failure

The report names the assertion, the kind, and what was actually seen. Then:

- **`HANDOFF_E2E_KEEP=1`** keeps the temporary root. In it: `app.log` (the app's own
  `tracing` output at `debug`), `agent-<session-id>.jsonl` (the `stream-json` transcript, one
  per agent run), `appdata/handoff.sqlite`, and the project with the two configuration files
  the run used.
- **A hook that misbehaved says nothing in the transcript**, only `stop-hook-error … ctrl+o
  to see`. The message is in the agent's own transcript,
  `~/.claude/projects/<slug>/<session-id>.jsonl`, as an attachment of type
  `hook_non_blocking_error` carrying `command`, `stderr`, `exitCode` and `durationMs`.
- **A timeout carries the last state it saw**, not just the fact that it timed out —
  `waitFor` prints the handoffs, their states and their steps, so "it never reached
  `verified`" and "there was never a handoff" are distinguishable.

## Traps

Each of these cost a run.

- **Smart App Control blocks a freshly built binary**, and from Node it is not `os error
  4551` but `spawn UNKNOWN` (`errno -4094`), which says nothing. `startApp` retries. For the
  build itself, the remedy that works here is to delete the **blocked file** and let cargo
  relink it — including a build script: `rm src-tauri/target/release/build/<crate>-*/build-script-build.exe`
  cleared a block that had survived six identical re-runs and a deleted directory.
- **An MCP tool is denied unless it is named in `--allowedTools`.** Without it the run ends
  with "Claude requested permissions to use `mcp__handoff__handoff_to_user`" and nothing was
  called.
- **`--strict-mcp-config` is mandatory**, or the account's own claude.ai connectors load into
  the child session.
- **Do not tell the model "call no other tool".** It reaches an MCP tool through its own tool
  search first, so forbidding everything else makes the wanted tool unreachable.
- **Never redirect `HOME` or `USERPROFILE` for `claude`.** The credentials are there; a
  redirected home lands the child on an unauthenticated profile. This suite redirects only
  `HANDOFF_HOME` and `HANDOFF_APP_DATA_DIR`, and never runs the installation adapter.
- **The heartbeat floor is 50 s**, `max(tool timeout − 60 s, 50 s)`. A scenario that needs a
  blocked call to come back on its own sets `HANDOFF_TOOL_TIMEOUT_MS` accordingly and waits
  past 50 s, whatever arithmetic the timeout alone suggests.
- **A file gate holds the agent still** between two steps of a scenario: a `Bash` command that
  sleeps, with `alsoAllow: ['Bash']`. E2E-9 needs it, because a turn that ended before the
  request existed would have nothing to deliver.
- **An unref'd timer empties the event loop.** A scenario spends most of its life waiting for
  a model; with the polling timer unref'd, Node exits with "unsettled top-level await" and no
  other explanation.
- **A plain `codex exec` reaches the owner's accounts.** Apps and plugins are on by default
  and a `-c mcp_servers.…` override merges with the user's own servers, so the isolation flags
  of `tests/e2e/codex.ts` are never optional (the T-066 handoff entry).
- **Without `default_tools_approval_mode = "approve"`, `codex exec` refuses our tools** with
  "MCP tool call requires approval, but approval policy is never", which reads like a model
  that never called them.
- **Codex keeps no transcript of an `--ephemeral` run.** The `--json` stream is written to
  `agent-codex-<thread id>.jsonl` in the run's root, and `HANDOFF_E2E_KEEP=1` is the only way
  to read it afterwards. There is no `--max-turns`: the harness timeout is the bound.
- **The clipboard is read, not composed, in `e2e-09-request-clipboard`**: the sentence the
  harness pastes is the one the app rendered, in the app's language. A person copying
  something else during the run would change what the agent is given.
- **OpenCode starts its servers in `PWD`, not in its own working directory.** Started from a
  shell, it would start ours in the checkout; `tests/e2e/opencode.ts` sets `PWD` to the run's
  project (measured by the OpenCode canary of `handoff-mcp`).
- **An OpenCode session outlives `opencode run`.** The runner writes the `--format json`
  stream to `agent-opencode-<session id>.jsonl` in the run's root and then deletes the session
  from OpenCode's history; `HANDOFF_E2E_KEEP=1` keeps the file.
- **A free model is a shared one.** A busy one answers "temporarily rate-limited upstream"
  before any tool is called, and the scenario fails on its first assertion. Run it again, or
  name another model with `HANDOFF_E2E_OPENCODE_MODEL`.
- **Cursor's print mode refuses our tools without a permission rule**, and the refusal reads as
  if a person had said no: `User rejected MCP: handoff-handoff_to_user`, after about two
  minutes. The run's `.cursor\cli.json` carries `Mcp(handoff:*)`; `--force` would also allow
  every shell command and is never passed (T-068, T-069).
- **Start the Cursor CLI through its `node.exe`, not through `agent.cmd`.** The shim goes
  through `cmd.exe` and PowerShell, whose quoting mangles the JSON a prompt carries;
  `tests/e2e/cursor.ts` finds the newest `versions\<date>-<commit>\node.exe` and `index.js`
  beside the shim, as the CLI's own launcher does.
- **Every `agent -p` leaves state behind** — a folder under `~/.cursor/projects` and a
  conversation under `~/.cursor/chats` — and has no ephemeral mode. The runner lists both
  folders before a run and deletes only what the run added.
- **The editor scenario opens a real window** for the seconds it takes to register. It is a
  separate instance with a user-data folder of its own, so the owner's Cursor is never touched,
  and it is closed with `taskkill /T /F` together with the server it started.
