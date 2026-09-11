# Cursor

Baton works with Cursor as well as with Claude Code, Codex and OpenCode: in Cursor's editor and
in its Agent CLI, `agent`. A handoff from Cursor runs the same way: the panel opens, you work
through the steps one at a time, and Cursor hears back what happened. Cursor has **base**
support where Claude Code has *full*, and this page says what that changes for you.

## Registering it

Baton finds Cursor when its settings folder `%USERPROFILE%\.cursor` exists — the editor and the
Agent CLI both create it the first time they run — or when `cursor` or `cursor-agent` is on
your `PATH`. It offers to register itself there on the first launch, or later from Settings →
Agents → **Register**, and the consent screen shows **one** change: the MCP server entry in
`mcp.json`. [The consent screen](../consent-screen.md#the-change-for-cursor) shows exactly what
is written and why.

Baton writes `%USERPROFILE%\.cursor\mcp.json`, the file the editor and the Agent CLI both read,
so one registration covers the two. The editor starts the servers of that file when a window
opens: close and reopen the Cursor windows that were already open, and restart the CLI sessions
that were already running.

**A file with comments is not edited.** Rewriting such a file would lose them. If yours has
any, Baton says so and registers nothing; take the comments out, or write the entry by hand as
the consent screen shows it.

## What works as with Claude Code

- **The whole handoff**: the steps, **Ask**, **Note**, **Skip**, **Defer**, **Abandon**, the
  verification, and the runbook written after a verified handoff.
- **Screenshots as pictures.** The Agent CLI hands a picture from Baton to the model. The
  editor's chat was not measured; if a picture does not seem to arrive there, prefer **Send
  text** in the preview.
- **Your sessions in the panel, one per window.** A tab opened from Cursor's editor says
  *Cursor*, then the name of the window's folder — the first one, in a workspace with several.
  Every chat of a window shares that one session, and two windows are two sessions. A session
  of the Agent CLI names the folder it was started in.
- **Your requests reach the right window.** When you open a request with the shortcut, Baton
  copies it and brings forward the Cursor window whose title names that session's folder.

## What is different

### Nothing reminds the agent

Claude Code runs a hook at the end of every turn, and Baton's hook reminds the agent, once, of
anything that is waiting for it. Cursor has hooks of its own, but **none of them reaches
Baton**: what Cursor hands a hook is not what Baton's hook reads, so Baton registers none. If
Baton is also registered for Claude Code, Cursor may run that hook at the end of its own turns;
it stays silent there, and never stops a Cursor turn. In practice:

- **A step you defer** comes back to the agent with the instruction to return to it before it
  ends its turn, and to keep the handoff's id in its notes, because nothing will remind it. If
  it does not, the handoff waits in the panel, and **Resume** there makes it active again and
  copies a sentence for you to paste into Cursor.
- **A request you open** reaches Cursor only through the clipboard: Baton copies the sentence
  and brings the window forward, and you paste it into a chat. A request opened while no Cursor
  session is running waits, and is copied again when the first session starts.
- **An answer the agent has not collected** waits in Baton until the agent calls again.

### A call lasts a minute

Cursor gives a tool call a limit that no entry can raise: the Agent CLI cuts a call after sixty
seconds, and the editor waits longer. So while you work through a handoff, Baton's server tells
the agent after fifty seconds that the handoff is still in progress, and the agent picks it up
again at once. Nothing is lost, and there is nothing to set.

### Cursor asks before a call

Baton's entry grants nothing beyond itself. The editor asks you before it runs one of Baton's
tools, as it does for any MCP server. The Agent CLI's print mode, `agent -p`, has nobody to ask
and refuses such a tool; to let it run Baton's, add `Mcp(handoff:*)` to the `allow` list of
`permissions` in `.cursor\cli.json` in the project (or in `%USERPROFILE%\.cursor\cli-config.json`,
for every project). In a project that has no `cli.json` yet, the whole file is:

```json
{ "permissions": { "allow": ["Mcp(handoff:*)"] } }
```

It allows Baton's tools and nothing else. Baton does not write it for you.

## Checking the registration

```text
agent mcp list
```

lists the servers the Agent CLI reads in the folder you run it from, with `handoff` among them
once Baton is registered. Cursor's settings list them for the editor.

## One project instead of this user

In Settings → Agents → **Where** → **One project**, Baton writes the same entry into
`.cursor\mcp.json` inside the folder you choose. Cursor loads a server of a project file only
once you have approved it: the editor asks, and the Agent CLI refuses it until you run
`agent mcp enable handoff` in that folder. Baton never approves it for you.

## Taking it back

Settings → Agents → **Remove** deletes the `"handoff"` entry from `mcp.json`, recognised by the
path of Baton's server in its `command`, and nothing else: your other servers stay as they
were, in the same order. If Baton's entry was the only one under `"mcpServers"`, the emptied
key goes too. As for every change, a copy of the file is saved beside it first.
