# Kilo Code

Baton works with Kilo Code as well as with Claude Code, Codex, OpenCode, Cursor and GitHub
Copilot, on both of Kilo Code's surfaces: the Kilo CLI (`kilo`) and the Kilo Code extension for
VS Code, which run the same program. A handoff from either runs the same way: the panel opens,
you work through the steps one at a time, and Kilo hears back what happened. Kilo Code has
**base** support where Claude Code has *full*, and this page says what that changes for you.

## Registering it

Baton finds Kilo Code when `kilo` is on your `PATH`, when its settings folder
`%USERPROFILE%\.config\kilo` exists, or when the Kilo Code extension is installed in VS Code. It
offers to register itself there on the first launch, or later from Settings → Agents →
**Register**, and the consent screen shows **one** change: the MCP server entry in `kilo.json`,
which the CLI and the extension both read.
[The consent screen](../consent-screen.md#the-change-for-kilo-code) shows exactly what is
written and why.

Baton writes `%USERPROFILE%\.config\kilo\kilo.json`, or the same file under `XDG_CONFIG_HOME` if
you have set that variable, which is where Kilo itself looks. The extension keeps settings of
its own in `kilo.jsonc` in the same folder; Baton leaves that file alone and puts its entry in
`kilo.json` beside it: Kilo reads both. Kilo reads its settings when a session starts — the
extension when the Kilo panel first opens in a window — so restart the sessions that were
already running, and reload the VS Code windows where Kilo was open.

Kilo tidies the settings file when it reads it: it adds a `"$schema"` line at the top and
indents with two spaces. That is Kilo's doing, and it changes nothing Baton wrote.

**A file with comments is not edited.** Kilo allows comments in `kilo.json`, and rewriting such a
file would lose them. If yours has any, Baton says so and registers nothing; take the comments
out, or write the entry by hand as the consent screen shows it.

## What works as with Claude Code

- **The whole handoff**: the steps, **Ask**, **Note**, **Skip**, **Defer**, **Abandon**, the
  verification, and the runbook written after a verified handoff.
- **Screenshots as pictures**, if your model reads pictures. Kilo hands a picture from Baton to
  the model you chose, whichever provider it comes from. A model that reads only text gets every
  word of the answer and not the picture; with one of those — Kilo's free automatic model among
  them — prefer **Send text** in the preview.
- **A long wait.** Baton gives Kilo's calls to its own server 30 minutes, and with Kilo that
  matters more than with most: without it, Kilo gives up on a call after one minute. If a
  handoff takes longer than 30 minutes, nothing is lost: a minute before the limit the agent is
  told the handoff is still in progress, and picks it up again.
- **No question before each call.** Kilo runs Baton's tools without asking you, in the CLI and in
  VS Code alike, so the entry grants nothing beyond itself.
- **Your sessions in the panel.** A tab opened by Kilo says so: *Kilo Code*, then the name of the
  project folder — for the extension, the folder open in that VS Code window. Two VS Code windows
  are two sessions.

## What is different: nothing reminds the agent

Claude Code runs a hook at the end of every turn, and Baton's hook reminds the agent, once, of
anything that is waiting for it. **Kilo Code has no hook of that kind** — what it has are
plugins, which run inside Kilo — so with Kilo nothing asks at the end of a turn. In practice:

- **A step you defer** comes back to the agent with the instruction to return to it before it
  ends its turn, and to keep the handoff's id in its notes, because nothing will remind it. A
  careful agent does. If it does not, the handoff waits in the panel, and **Resume** there makes
  it active again and copies a sentence for you to paste into Kilo.
- **A request you open** with the shortcut reaches Kilo only through the clipboard: Baton copies
  the sentence and brings forward the window the session runs in — the terminal of the CLI, or
  the VS Code window of the extension — and you paste it into Kilo. A request opened while no
  Kilo session is running waits, and is copied again when the first session starts.
- **An answer the agent has not collected** waits in Baton until the agent calls again.

This is what Kilo allows today rather than a setting of Baton's.

## Checking the registration

```text
kilo mcp list
```

lists the servers Kilo reads, with `handoff` among them once Baton is registered.
`kilo debug config` prints the whole entry: Baton's server as the `command`, `HANDOFF_AGENT` set
to `kilo-code`, and `"timeout": 1800000`.

## One handoff in VS Code, by hand

The extension cannot be driven by a script, so this is how to see Kilo Code work in VS Code,
in about five minutes:

1. In Baton, open Settings → Agents, press **Register** beside Kilo Code, and accept the one
   change.
2. In VS Code, reload every window where Kilo was open (Command Palette → *Developer: Reload
   Window*).
3. Open a project folder, open the Kilo panel, and start a task that needs a person — for
   example: *Ask me, through Baton, to create a webhook endpoint on the Stripe dashboard, then
   verify it.*
4. The panel opens a tab that reads **Kilo Code · the name of that folder**. Do the steps, press
   **Done**, and let the agent verify: the tab ends *verified*.
5. Press `Ctrl+Alt+H`, type a request and press Enter: the VS Code window comes forward, and the
   sentence is on the clipboard. Paste it into the Kilo chat: the agent answers it with a
   handoff, which takes over the tab of the request.
6. Back in Settings → Agents, press **Remove** beside Kilo Code, then compare `kilo.json` with
   the `kilo.json.handoff-backup-…` copy beside it: only the `"handoff"` entry is gone.

## One project instead of this user

In Settings → Agents → **Where** → **One project**, Baton writes the same entry into `kilo.json`
at the top of the folder you choose. Kilo reads it, on top of your own settings, when it works
in that folder — the CLI started there, or a VS Code window with that folder open — and asks no
question about trusting it first.

## Taking it back

Settings → Agents → **Remove** deletes the `"handoff"` entry from `kilo.json`, recognised by the
path of Baton's server in its `command`, and nothing else: your other servers and settings stay
as they were, in the same order, and `kilo.jsonc` is never touched. If Baton's entry was the
only one under `"mcp"`, the emptied key goes too. As for every change, a copy of the file is
saved beside it first.
