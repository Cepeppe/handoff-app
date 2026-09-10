# The end-to-end suite: Baton, a real agent, and nobody at the keyboard

`pnpm e2e` runs the scenarios of the design's §11.5 against a **real Claude Code** and a
**real build of the app**, with the person at the machine played by an automation channel
that only exists in an `--features e2e` build.

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

## Running it

From the workspace root, which builds everything first:

```powershell
scripts\e2e.ps1                    # all ten
scripts\e2e.ps1 e2e-01-verified    # one
scripts\e2e.ps1 -DevLink           # against a local build of handoff-mcp
scripts\e2e.ps1 -SkipBuild         # reuse what is already built
```

Or, with the build already done, from `handoff-app`:

```bash
pnpm e2e
pnpm e2e -- e2e-05-parked
pnpm e2e -- --list
```

Four things must exist, and `missingPrerequisites()` names the two it can check:

1. `src-tauri/target/release/handoff-app.exe`, built **with `--features e2e`**;
2. `src-tauri/binaries/handoff-mcp-<triple>.exe`, from `node scripts/fetch-server.mjs`;
3. `dist/`, from `pnpm build` — a release binary loads it, a debug one looks for a Vite dev
   server on port 1420 and comes up empty (the T-040 handoff entry), which is why the suite
   uses the release profile;
4. `claude` on `PATH`, logged in.

Environment: `HANDOFF_E2E_MODEL` pins the model (default `sonnet`), `HANDOFF_E2E_KEEP=1`
keeps each run's temporary root, `HANDOFF_E2E_SERVER` points the MCP entry at another server
binary, `HANDOFF_E2E_RUST_LOG` changes what the app logs.

A whole run is about four minutes and a few cents. The report is
`tests/e2e/results/last-run.json` (git-ignored): verdicts, every assertion, the measured
facts, and the **transcript ids** — with which the agent's own transcript can be read at
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

`ci.yml` runs both on every push to `main`: the first on the debug bundle it has just built
without the feature, which the `cfg` gate makes as clean as a release binary, the second on a
debug build made with the feature. The release workflow runs the first again on the binary it
publishes. The second is the positive control and it is not decoration: a grep for a string
nobody writes passes for ever, including on the day the grep itself breaks.

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
