# Codex CLI

Baton works with the Codex CLI as well as with Claude Code. A handoff from Codex runs the same
way: the panel opens, you work through the steps one at a time, and Codex hears back what
happened. Codex has **base** support where Claude Code has *full*, and this page says what
that changes for you.

## Registering it

Baton finds Codex when `codex` is on your `PATH`, or when its settings folder
`%USERPROFILE%\.codex` exists. It offers to register itself there on the first launch, or
later from Settings → Agents → **Register**, and the consent screen shows **one** change: the
MCP server entry in Codex's `config.toml`. [The consent screen](../consent-screen.md#the-change-for-codex)
shows exactly what is written and why, including the approval Codex is given for Baton's
tools.

If you have set `CODEX_HOME`, Codex keeps its settings there instead, and Baton writes there
too. Codex reads its settings when a session starts, so restart the sessions that were already
running.

## What works as with Claude Code

- **The whole handoff**: the steps, **Ask**, **Note**, **Skip**, **Defer**, **Abandon**, the
  verification, and the runbook written after a verified handoff.
- **Screenshots as pictures.** Codex hands a picture from Baton to its model, so the preview
  offers **Send image** as well as **Send text**.
- **A long wait.** Baton gives Codex's calls to its own server 30 minutes. If a handoff takes
  longer, nothing is lost: a minute before the limit the agent is told the handoff is still in
  progress, and picks it up again.
- **Your sessions in the panel.** A tab opened by Codex says so: *Codex CLI*, then the name of
  the project folder.

## What is different: nothing reminds the agent

Claude Code runs a hook at the end of every turn, and Baton's hook reminds the agent, once, of
anything that is waiting for it. **Codex runs no such hook**, so with Codex nothing asks at the
end of a turn. In practice:

- **A step you defer** comes back to the agent with the instruction to return to it before it
  ends its turn, and to keep the handoff's id in its notes, because nothing will remind it. A
  careful agent does. If it does not, the handoff waits in the panel, and **Resume** there
  makes it active again and copies a sentence for you to paste into Codex.
- **A request you open** with the shortcut reaches Codex only through the clipboard: Baton
  copies the sentence and brings the terminal forward, and you paste it. A request opened while
  no Codex session is running waits, and is copied again when the first session starts.
- **An answer the agent has not collected** waits in Baton until the agent calls again.

This is what Codex allows today rather than a setting of Baton's. If a later version of Codex
runs hooks for sessions like these, its support can become full.

## Checking the registration

```text
codex mcp get handoff
```

prints the entry Codex reads. It names Baton's server as the command and shows
`tool_timeout_sec: 1800` and `default_tools_approval_mode: approve`.

## One project instead of this user

In Settings → Agents → **Where** → **One project**, Baton writes the same entry into
`.codex\config.toml` inside the folder you choose. **Codex reads that file only for a project
you have marked as trusted in Codex**; until then the entry is ignored. Baton never marks a
project as trusted for you.

## Taking it back

Settings → Agents → **Remove** deletes the `[mcp_servers.handoff]` section from Codex's
`config.toml`, recognised by the path of Baton's server in its `command`, and nothing else:
comments, other servers and profiles stay exactly as they were. As for every change, a copy of
the file is saved beside it first.
