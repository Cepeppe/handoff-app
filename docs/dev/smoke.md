# The smoke run: Baton against the real Claude Code

Everything below runs on a developer machine, by hand, against a **real agent**. It is the
one check the automated suites cannot make: `cargo test` exercises the core with fakes on
both sides of the channel, and a green suite says nothing about whether an agent that has
never heard of us can register the server, open a handoff and read the outcome back.

Run it when you change the startup sequence, the channel, the store, the installation
adapter or the tool contract — and after every Claude Code update, together with the
canary of `handoff-mcp` (`docs/agent-facts.md` there).

Budget about forty minutes the first time.

> Most of what is below is automated now: `pnpm e2e` runs nine of the §11.5 scenarios in four
> minutes, driving the same paths through the automation channel of an `--features e2e` build
> ([`e2e.md`](e2e.md)). Run that first. What stays here and cannot be automated is the part
> that needs eyes and a mouse: the onboarding walk, the consent screen, the restart with a
> live handoff, and the two launch notices.

- [What it proves](#what-it-proves)
- [The harness](#the-harness)
- [Setting it up](#setting-it-up)
- [The three runs](#the-three-runs)
- [Restart with a live handoff](#restart-with-a-live-handoff)
- [The two launch checks](#the-two-launch-checks)
- [Cleaning up](#cleaning-up)
- [Traps](#traps)

## What it proves

| # | Check | Expected |
|---|---|---|
| 1 | A handoff with no `verify`, every step confirmed | outcome `confirmed_by_user`, `final: true` |
| 2 | **Ask** from the overlay, answered by the agent | outcome `question` with the text, then `confirmed_by_user` |
| 3 | A handoff with `verify`, reported with `handoff_verify ok:true` | outcome `awaiting_verification`, then `verified` |
| 4 | The app is killed and started again while a handoff is live | the tab is restored with the "session detached" banner, which clears when the agent resumes |
| 5 | The launch checks | onboarding on a first launch, the crash notice after a panic, the superseded binaries deleted |

## The harness

Everything lives under one folder, `%TEMP%\baton-t042\`, so that **nothing touches the
installation you use every day**: not `~/.handoff`, not the app's database, not
`~/.claude.json`, not the login items. Three redirections do it, and all three must be
spelled identically everywhere — the Windows pipe name is a digest of the `HANDOFF_HOME`
*string* and not of the folder it resolves to (`TASKS.md` §0.4 item 4):

| Variable | What it moves |
|---|---|
| `HANDOFF_HOME` | `~/.handoff`: the token, the socket or pipe, the runbooks |
| `HANDOFF_APP_DATA_DIR` | the database, the settings, `crashes/` |
| `USERPROFILE` | where the installation adapter writes `~/.claude.json` and `~/.claude/settings.json` |

The scripts are eight small files. Write them once into `%TEMP%\baton-t042\` and keep them;
the paths below are literal, so change the two `C:\Users\<you>` roots and nothing else.

### `app.ps1` — the overlay under test

```powershell
$env:HANDOFF_HOME = 'C:\Users\<you>\AppData\Local\Temp\baton-t042\home'
$env:HANDOFF_APP_DATA_DIR = 'C:\Users\<you>\AppData\Local\Temp\baton-t042\appdata'
$env:USERPROFILE = 'C:\Users\<you>\AppData\Local\Temp\baton-t042\claude-home'
$env:HOME = $env:USERPROFILE
# rustup and cargo find their own homes through the user's profile, so the redirection above
# hides the toolchain from them ("rustup could not choose a version of cargo"). Point them
# back: the redirection is for the application under test, not for the build that makes it.
$env:RUSTUP_HOME = 'C:\Users\<you>\.rustup'
$env:CARGO_HOME = 'C:\Users\<you>\.cargo'
$env:RUST_LOG = 'handoff_app_lib=debug'
Set-Location 'C:\Users\<you>\...\handoff-app'
& cargo tauri dev 2>&1 | Tee-Object -FilePath "$env:TEMP\baton-t042\app.log"
```

### `show.ps1` — bring the panel forward

A second launch of the single instance is cheaper than the tray flyout. It has to carry the
same environment, or it starts a *second* instance on a different pipe instead of reaching
the one under test.

```powershell
$env:HANDOFF_HOME = 'C:\Users\<you>\AppData\Local\Temp\baton-t042\home'
$env:HANDOFF_APP_DATA_DIR = 'C:\Users\<you>\AppData\Local\Temp\baton-t042\appdata'
$env:USERPROFILE = 'C:\Users\<you>\AppData\Local\Temp\baton-t042\claude-home'
& 'C:\Users\<you>\...\handoff-app\src-tauri\target\debug\handoff-app.exe'
```

### `run-claude.ps1` — one agent run

```powershell
param([Parameter(Mandatory = $true)][string]$Prompt,
      [Parameter(Mandatory = $true)][string]$Out,
      [string]$McpConfig = '',
      [string]$AlsoAllow = '')

$root = 'C:\Users\<you>\AppData\Local\Temp\baton-t042'
$env:HANDOFF_HOME = "$root\home"
$env:HANDOFF_AGENT = 'claude-code'
Set-Location "$root\project"

# PowerShell has no `<` redirection: an empty string on the pipeline is what stops
# `claude -p` waiting three seconds for stdin.
'' | & claude -p (Get-Content -Raw $Prompt) `
  --mcp-config $(if ($McpConfig -ne '') { $McpConfig } else { "$root\claude-home\.claude.json" }) `
  --strict-mcp-config `
  --settings "$root\claude-home\.claude\settings.json" `
  --output-format stream-json --verbose `
  --model sonnet `
  --allowedTools "mcp__handoff__handoff_to_user,mcp__handoff__handoff_verify,mcp__handoff__handoff_runbooks$AlsoAllow" *>&1 |
  Tee-Object -FilePath $Out
```

Five things on that command line, every one of them load-bearing — see [Traps](#traps).

### `db.mjs` — read the live database

`node:sqlite` reads a WAL database while the app holds it open, which is the only way to see
the state without a UI.

```js
import { DatabaseSync } from 'node:sqlite';
const db = new DatabaseSync(
  'C:/Users/<you>/AppData/Local/Temp/baton-t042/appdata/handoff.sqlite',
  { readOnly: true },
);
const rows = (sql) => db.prepare(sql).all();
const show = (what, sql) => console.log(what, JSON.stringify(rows(sql), null, 1));
show('handoffs ', 'select id, state, session_ref, delivered_at from handoffs');
show('sessions ', 'select session_ref, client_name, connected, first_seen from sessions');
show('settings ', 'select key, value from settings');
```

### `drive.ps1`, `click.ps1`, `click2.ps1`, `expand-shot.ps1` — driving the window

The webview publishes **no UI Automation tree**, so there is no way to find a button by
name: every press is a click at coordinates read off a screenshot. It works because the
screenshot and the click are both taken by DPI-*unaware* PowerShell, so the two coordinate
spaces are the same virtualised one.

- `drive.ps1 -Action screenshot -Text <path>` saves the virtual screen; `-Action type -Text
  <keys>` sends keystrokes; `-Action foreground` prints the foreground window;
  `-Action clipboard` prints the clipboard.
- `click.ps1 -X <x> -Y <y>` is `SetCursorPos` plus `mouse_event` down/up.
- `click2.ps1 -X1 -Y1 -X2 -Y2` does **two** clicks in one process. The panel collapses
  whenever it loses focus (WIN-03), so the first click is the one that gives it focus and
  expands it and the second is the button; split across two commands, the focus drops in
  between and the second click lands on the collapsed bar again.
- `expand-shot.ps1 -X -Y -Out <path>` clicks and then photographs, in one process, for the
  same reason: a click in one command and a screenshot in the next photographs the bar.

## Setting it up

1. **The server the app will register.** `install::fixed_path` registers the server *next to
   the application executable*. Under `cargo tauri dev` that is `src-tauri/target/debug/`,
   where Tauri does not put the sidecar, so copy one in first:

   ```bash
   cp src-tauri/binaries/handoff-mcp-x86_64-pc-windows-msvc.exe src-tauri/target/debug/
   ```

   Either spelling is found — `handoff-mcp.exe` first, then the target-triple one.

   The vendored server is the **pinned release** (`server.lock.json`), which is what will
   ship. `scripts/dev-link` in the workspace root replaces it with a local build of
   `handoff-mcp`; use it only when the change under test is in the server.

2. **The folders.**

   ```bash
   mkdir -p "$TEMP/baton-t042"/{home,appdata,claude-home,project}
   ```

3. **Start the app**: `pwsh -NoProfile -File app.ps1`. The first build takes a minute. Watch
   `app.log` for the startup sequence:

   ```text
   INFO handoff_app_lib: starting entitlement=full
   INFO handoff_app_lib: the channel is listening endpoint=\\.\pipe\handoff-<digest>
   DEBUG handoff_app_lib: update check status=disabled
   INFO handoff_app_lib::ui_bridge::shortcut: the global shortcut is registered accelerator="Control+Alt+H"
   ```

4. **Onboarding runs by itself** on a fresh data directory. Click through it: *Avanti* →
   **Registra** → read the consent screen (it must say **three** modifications on two rows)
   → **Accetta e registra** → *Avanti* → **untick "Avvia Baton quando accedo"** → *Avanti* →
   *Avanti* → **Fine**.

   > Untick the autostart box. The login items live in the real `HKCU\…\Run`, which no
   > environment variable redirects, so leaving it ticked puts a development build in your
   > own startup.

5. **Check what it wrote**, under the temporary home and nowhere else:

   ```bash
   cat "$TEMP/baton-t042/claude-home/.claude.json"
   cat "$TEMP/baton-t042/claude-home/.claude/settings.json"
   ```

   `.claude.json` must carry `"timeout": 1800000` and `HANDOFF_TOOL_TIMEOUT_MS`, and
   **no** `MCP_TOOL_TIMEOUT`; `settings.json` must carry the Stop and SubagentStop hooks
   with the server path **in quotes**.

## The three runs

Each run is a prompt file and one `run-claude.ps1` invocation. Write the prompts so the
agent prints one line at the end — `STATUS=… FINAL=… ID=…` — which is what you assert on,
and keep them prescriptive: the spec belongs in the prompt, not in the model's judgement.

```bash
pwsh -NoProfile -File run-claude.ps1 -Prompt "$TEMP\baton-t042\prompt-1.txt" \
                                     -Out    "$TEMP\baton-t042\run1.jsonl"
```

The `init` line of the transcript is the first assertion, before the model does anything:

```json
{"type":"system","subtype":"init","mcp_servers":[{"name":"handoff","status":"connected"}],
 "tools":[…,"mcp__handoff__handoff_runbooks","mcp__handoff__handoff_to_user","mcp__handoff__handoff_verify"]}
```

**Run 1 — confirmed by user.** A spec with two steps and no `verify`. Press **Fatto** on
each step. Expect `STATUS=confirmed_by_user FINAL=true`.

**Run 2 — ask and reply.** The same shape. On step 1 press **Chiedi**, type a question,
press **Invia**. The agent receives `status: "question"` with `user_text` and the step's
context, and answers with the continue shape; the overlay then shows both halves
("Hai chiesto: …" / "L'agente ha risposto: …"). Confirm the steps. Expect
`confirmed_by_user`.

**Run 3 — verified.** A spec that carries `verify`, and a prompt telling the agent to report
`handoff_verify` with `ok: true` (there is nothing to check in a smoke run — say so in the
`detail`). Confirm both steps: the tab goes to *Verifying* and the agent's report closes it.
Expect `STATUS=verified FINAL=true`.

Reading a transcript:

```bash
node -e "
const fs=require('fs');
for (const line of fs.readFileSync(process.argv[1],'utf8').split(/\r?\n/)) {
  if(!line.trim()) continue; let m; try{m=JSON.parse(line);}catch{continue;}
  if (m.type==='assistant') for (const c of m.message.content ?? [])
    if (c.type==='tool_use') console.log('TOOL:', c.name, JSON.stringify(c.input).slice(0,200));
  if (m.type==='user') for (const c of m.message?.content ?? [])
    if (c.type==='tool_result') console.log('RESULT:', JSON.stringify(c.content).slice(0,500));
  if (m.type==='result') console.log('FINAL:', m.result);
}" run1.jsonl
```

## Restart with a live handoff

This is the check §7.2 exists for: state lives in the app and survives it (NFR-12, FM-13).

The difficulty is timing, not behaviour. The server reconnects **about five seconds** after
the app is back (§5.3's backoff), and the agent's blocked call is freed by the heartbeat,
which is `max(tool timeout − 60 s, 50 s)` — half an hour with what the installer writes. Two
adjustments make the window observable:

1. **Shorten the heartbeat.** Copy `.claude.json` to `mcp-short-heartbeat.json` with
   `HANDOFF_TOOL_TIMEOUT_MS` set to `"110000"`, and pass it with `-McpConfig`. Anything at
   or below 110 000 gives the floor of 50 s. Leave `"timeout"` alone — that is Claude Code's
   own cut and must stay longer than the call.
2. **Gate the resume on a file**, so the agent waits where you want it to. Put this in the
   prompt and allow `Bash` (`-AlsoAllow ",Bash"`):

   ```text
   Whatever came back - status "in_progress", or an error saying the server is
   disconnected - run this Bash command and read its output:
     sleep 10; ls /c/Users/<you>/AppData/Local/Temp/baton-t042/go.txt 2>&1
   If the output says "No such file", run the same command again. Repeat until the output
   is the path itself, for at most 30 attempts. Once the gate file exists, call
   handoff_to_user in the resume shape with resume set to the handoff id.
   ```

Then:

1. Start the run and wait for the handoff row to appear (`db.mjs`).
2. **Kill** the app rather than quitting it: `Stop-Process -Name handoff-app -Force`. A kill
   leaves a `sessions` row flagged connected, which is exactly what the next start has to
   repair.
3. Start it again and bring the panel forward (`app.ps1`, then `show.ps1`). The log must say:

   ```text
   INFO handoff_app_lib::sessions::registry: sessions left connected by a previous run were closed count=1
   INFO handoff_app_lib::sessions::registry: a session registered session_ref="ses_…" agent_id="claude-code"
   ```

4. Select the restored tab. It carries its goal, its step and its buttons, and the banner

   > **Sessione staccata; l'esito arriverà al prossimo resume** (SRV-22)

5. Open the gate (`echo x > go.txt`). The agent resumes, the call attaches, and **the banner
   goes**: the tab is being guided again. Confirm the steps to close the run.

The banner is keyed on "no agent is on this handoff right now", not on the opening session:
a reconnection is a **new** session with a new `session_ref` (§8.3), so the session that
opened a restored handoff never comes back under that name and a banner keyed on it alone
would be permanent.

## The two launch checks

Both are cheap to fake and worth doing once per release.

**The crash notice (§7.14).** Drop a file into the crash folder and restart:

```bash
mkdir -p "$TEMP/baton-t042/appdata/crashes"
echo "planted" > "$TEMP/baton-t042/appdata/crashes/2026-09-09T06-45-00Z.txt"
```

The log says `INFO handoff_app_lib::ui_bridge::crash: the previous run ended in a crash` and
the overlay draws the notice with **Apri la cartella** and **Non ora**. Restart once more:
it must **not** come back — the report is recorded in the `crash_seen` setting, which
`db.mjs` shows.

**The superseded binaries (FM-24).** Drop a file beside the application executable and
restart:

```bash
printf 'x' > src-tauri/target/debug/handoff-mcp.9.9.9.old.exe
```

The log says `INFO handoff_app_lib: superseded server binaries were deleted count=1`, the
file is gone, and `handoff-mcp.exe` beside it is untouched.

## A live handoff without an agent

The three runs above need Claude Code, which is the point of a smoke run. A walk through
something the *user* does — the capture flow of §7.8, an action sheet, a banner — needs a
guided handoff on screen and nothing else, and starting an agent for that is minutes of
tokens for a tab. Any process that speaks the channel can open one: it is `hello` with the
token, then `handoff.open` with a spec.

```js
// open-handoff.mjs <pipe name from the app log>
import net from 'node:net';
import fs from 'node:fs';
const token = fs.readFileSync(process.env.TEMP + '/baton-t042/home/channel.token', 'utf8').trim();
const socket = net.connect({ path: process.argv[2] });
let buffer = '', nextId = 1;
const pending = new Map();
// The framing of §6.1, built rather than written: a line feed in a code fence of this file
// is one backslash away from being a real newline, and three tasks have lost one that way.
const NDJSON = String.fromCharCode(10);
const send = (method, params) => new Promise((resolve, reject) => {
  const id = nextId++;
  pending.set(id, { resolve, reject });
  socket.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + NDJSON);
});
socket.on('data', (chunk) => {
  buffer += chunk;
  for (let at; (at = buffer.indexOf(NDJSON)) !== -1; ) {
    const line = buffer.slice(0, at); buffer = buffer.slice(at + 1);
    if (!line.trim()) continue;
    const m = JSON.parse(line);
    if (m.method === 'ping') socket.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result: {} }) + NDJSON);
    else if (m.id !== undefined && !m.method) { pending.get(m.id)?.resolve(m.result); pending.delete(m.id); }
  }
});
socket.on('connect', async () => {
  await send('hello', {
    protocol_version: 1, token, role: 'server', server_version: '0.2.0',
    identity: { pid: process.pid, ppid: 1, ancestors: [{ pid: 1, name: 'pwsh' }], cwd: '.', project_dir: '.' },
    agent_id: 'claude-code', client: { name: 'claude-code', version: '2.1.266' },
    capability_row: { agent_id: 'claude-code', display_name: 'Claude Code', support: 'full',
                      images_in_results: true, stop_hook: true, tool_timeout_ms: 1800000 },
  });
  console.log(await send('handoff.open', {
    call_id: 'call_2q7m8r1t',
    spec: { spec_version: 1, goal: 'Read the numbers on the screen', where: 'The terminal',
            why_human: 'Only a person can look at the screen.', values: {},
            steps: [{ text: 'Look at the screen.' }, { text: 'Confirm when you are done.' }], lang: 'en' },
    secret_treated: [], request_id: null,
  }));
});
```

Keep the process alive: it answers the app's pings, and the handoff has a call attached for
as long as it is connected — which is what makes the tab *Guiding* rather than "the agent
will pick it up at its next resume".

Three things the channel schema refuses, each of which closes the connection with
`the channel peer sent a message the schema refuses` and nothing else:

- a `call_id` that is not `call_` plus **eight** characters of `[0-9a-hjkmnp-tv-z]`;
- a spec without `values` — it is required even when it is empty (§4.2);
- any field the spec schema does not name: it is closed (`additionalProperties: false`).

The pipe name is in the app's own log (`the channel is listening endpoint=\\.\pipe\handoff-…`);
deriving it again is a second implementation of §5.8 and a way to be wrong.

## Cleaning up

```powershell
Stop-Process -Name handoff-app -Force
Get-NetTCPConnection -LocalPort 1420 | ForEach-Object { Stop-Process -Id $_.OwningProcess -Force }
```

Then check three things:

- `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` has no `Baton` value;
- port 1420 is free — a `pnpm dev` left running makes the next `cargo tauri dev` fail with
  a message that does not mention it (`strictPort`);
- `git status` in `handoff-app` is clean.

`%TEMP%\baton-t042\` can stay: it is the harness, and rebuilding it is the slow part.

## Traps

Each of these cost a run the first time.

- **An MCP tool is denied unless it is named in `--allowedTools`.** Without it the run ends
  with "Claude requested permissions to use `mcp__handoff__handoff_to_user`, but you haven't
  granted it yet" and nothing was called.
- **`--strict-mcp-config` is mandatory.** Without it the account's own claude.ai connectors
  load into the child session.
- **`--output-format stream-json` needs `--verbose`.** Claude Code refuses it otherwise, and
  `--output-format json` prints only the final object, with no transcript to read.
- **Do not tell the model "call no other tool".** An MCP tool is not in its context
  directly: it reaches it through its own tool search first, so forbidding everything else
  makes the wanted tool unreachable.
- **Never redirect `HOME` for `claude` itself.** The credentials are there; a redirected
  home lands the child on an unauthenticated profile. `--mcp-config` and `--settings` are
  how a produced configuration is used without touching the real one.
- **Smart App Control blocks a freshly linked binary** with `os error 4551` and the app
  never starts. Run the identical command again — the second attempt usually passes; if it
  does not, delete `src-tauri/target/debug/handoff-app.exe` and let cargo relink it.
  Evidence: `Get-WinEvent -FilterHashtable @{LogName='Microsoft-Windows-CodeIntegrity/Operational'; Id=3077}`.
- **A second launch logs a channel error before it hands over.** `run()` starts the listener
  before the single-instance plugin can pass the arguments on, so `show.ps1` prints
  `the channel could not start … (os error 5)` and exits. That is the second instance
  failing to bind a pipe the first one owns, which is what it should do; the running
  instance is untouched.
- **The panel collapses on focus loss** (WIN-03), so a click and a screenshot in two
  commands photograph two different things. Use `click2.ps1` and `expand-shot.ps1`.
- **A window position is remembered per monitor** (WIN-02), so the panel does not always
  come back where you last saw it. Take a full screenshot before the first click of a walk.
- **Pace a synthetic drag over the selection overlay** (§7.8). A burst of `SetCursorPos`
  calls 50 ms apart produced one crop that did not match the rectangle asked for, and a
  paced one — 200 ms between the press, the moves and the release — was exact every time.
  The overlay draws the size in the pixels the image will have, from the same numbers the
  crop is made of, so photograph the rectangle **before** releasing and read the label: it
  is the only place the drag can be checked while it is still cancellable.
