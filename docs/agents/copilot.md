# GitHub Copilot

Baton works with GitHub Copilot as well as with Claude Code, Codex, Cursor and OpenCode: in VS
Code's chat, where Copilot runs, and in the Copilot CLI, `copilot`. A handoff from Copilot runs
the same way: the panel opens, you work through the steps one at a time, and Copilot hears back
what happened. Copilot has **base** support where Claude Code has *full*, and this page says what
that changes for you.

## Registering it

Baton finds Copilot when the Copilot CLI's folder `%USERPROFILE%\.copilot` or VS Code's settings
folder `%APPDATA%\Code\User` exists, or when `copilot` or `code` is on your `PATH`. It offers to
register itself on the first launch, or later from Settings → Agents → **Register**, and the
consent screen shows **two** changes, one for each place Copilot runs:

- the MCP server entry for the Copilot CLI, in `%USERPROFILE%\.copilot\mcp-config.json`;
- the MCP server entry for VS Code, in `%APPDATA%\Code\User\mcp.json`.

[The consent screen](../consent-screen.md#the-changes-for-github-copilot) shows exactly what is
written and why. If you have set `COPILOT_HOME`, the CLI keeps its file in that folder instead,
and that is where Baton writes it.

VS Code starts Baton's server the first time a chat needs it, and asks you whether you trust it
the first time it starts: say yes. The Copilot CLI reads its file when a session starts: restart
the sessions that were already running.

**A file with comments is not edited.** Rewriting such a file would lose them. If either file has
any, Baton says so and registers nothing; take the comments out, or write the entries by hand as
the consent screen shows them.

## What works as with Claude Code

- **The whole handoff**: the steps, **Ask**, **Note**, **Skip**, **Defer**, **Abandon**, the
  verification, and the runbook written after a verified handoff.
- **Screenshots as pictures.** The Copilot CLI hands a picture from Baton to the model. VS Code's
  chat was not measured; if a picture does not seem to arrive there, prefer **Send text** in the
  preview.
- **Your sessions in the panel.** A tab opened from VS Code says *GitHub Copilot*, then the name
  of the window's folder — the first one, in a workspace with several. Every chat of a window
  shares that one session, and two windows are two sessions. A session of the Copilot CLI names
  the folder it was started in.
- **Your requests reach the right window.** When you open a request with the shortcut, Baton
  copies it and brings forward the VS Code window whose title names that session's folder.

## What is different

### Nothing reminds the agent

Claude Code runs a hook at the end of every turn, and Baton's hook reminds the agent, once, of
anything that is waiting for it. Copilot has hooks of its own in both places, but **neither
answers Baton's hook in a way the agent acts on**, so Baton registers none. The Copilot CLI also
runs the Claude Code hooks of a project's `.claude\settings.json` as its own: if Baton is
registered for Claude Code in that project, a Copilot turn there reaches Baton, and Baton answers
it with nothing rather than with another agent's reminders. In practice:

- **A step you defer** comes back to the agent with the instruction to return to it before it
  ends its turn, and to keep the handoff's id in its notes, because nothing will remind it. If it
  does not, the handoff waits in the panel, and **Resume** there makes it active again and copies
  a sentence for you to paste into Copilot.
- **A request you open** reaches Copilot only through the clipboard: Baton copies the sentence
  and brings the window forward, and you paste it into a chat. A request opened while no Copilot
  session is running waits, and is copied again when the first session starts.
- **An answer the agent has not collected** waits in Baton until the agent calls again.

### How long a call may last

The Copilot CLI's entry raises its limit, for Baton's server alone, to 30 minutes, as for Claude
Code. VS Code's entry has no such setting; there, Baton's server tells the agent after fifty
seconds that the handoff is still in progress, and the agent picks it up again at once. Nothing
is lost either way, and there is nothing to set.

### Copilot asks before a call

Baton's entries grant nothing beyond themselves. VS Code asks you before a chat runs one of
Baton's tools, as it does for any MCP server, and so does the Copilot CLI in an interactive
session. In print mode, `copilot -p`, nobody is there to ask; allow Baton's tools on its command
line with `--allow-tool=handoff`, which allows them and nothing else. Baton does not grant it for
you.

## Checking the registration

```text
copilot mcp list
```

lists the servers the Copilot CLI reads, with `handoff` among them once Baton is registered, and
`copilot mcp get handoff` shows its entry. In VS Code, the command **MCP: List Servers** shows
`handoff` among the servers of your user settings.

## One project instead of this user

In Settings → Agents → **Where** → **One project**, Baton writes the CLI's entry into
`.github\mcp.json` and VS Code's into `.vscode\mcp.json` inside the folder you choose — not into
`.mcp.json`, which is Claude Code's. The Copilot CLI reads a project's files only in a folder you
have told it to trust, and VS Code asks you to trust a server of a workspace file before its first
start. Baton never trusts either for you.

## Taking it back

Settings → Agents → **Remove** deletes Baton's `"handoff"` entry from each file, recognised by the
path of Baton's server in its `command`, and nothing else: your other servers stay as they were,
in the same order. If Baton's entry was the only one under `"mcpServers"` or `"servers"`, the
emptied key goes too. As for every change, a copy of each file is saved beside it first.
