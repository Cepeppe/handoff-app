# OpenCode

Baton works with OpenCode as well as with Claude Code and Codex. A handoff from OpenCode runs
the same way: the panel opens, you work through the steps one at a time, and OpenCode hears
back what happened. OpenCode has **base** support where Claude Code has *full*, and this page
says what that changes for you.

## Registering it

Baton finds OpenCode when `opencode` is on your `PATH`, or when its settings folder
`%USERPROFILE%\.config\opencode` exists. It offers to register itself there on the first
launch, or later from Settings → Agents → **Register**, and the consent screen shows **one**
change: the MCP server entry in `opencode.json`.
[The consent screen](../consent-screen.md#the-change-for-opencode) shows exactly what is
written and why.

Baton writes `%USERPROFILE%\.config\opencode\opencode.json`, or the same file under
`XDG_CONFIG_HOME` if you have set that variable, which is where OpenCode itself looks. If you
keep your settings in `opencode.jsonc` instead, Baton leaves that file alone and puts its entry
in `opencode.json` beside it: OpenCode reads both. OpenCode reads its settings when a session
starts, so restart the sessions that were already running.

**A file with comments is not edited.** OpenCode allows comments in `opencode.json`, and
rewriting such a file would lose them. If yours has any, Baton says so and registers nothing;
take the comments out, or write the entry by hand as the consent screen shows it.

## What works as with Claude Code

- **The whole handoff**: the steps, **Ask**, **Note**, **Skip**, **Defer**, **Abandon**, the
  verification, and the runbook written after a verified handoff.
- **Screenshots as pictures**, if your model reads pictures. OpenCode hands a picture from Baton
  to the model you chose. A model that reads only text gets every word of the answer and not
  the picture; with one of those, prefer **Send text** in the preview.
- **A long wait.** Baton gives OpenCode's calls to its own server 30 minutes, and with OpenCode
  that matters more than with the others: without it, OpenCode gives up on a call after one
  minute. If a handoff takes longer than 30 minutes, nothing is lost: a minute before the limit
  the agent is told the handoff is still in progress, and picks it up again.
- **No question before each call.** OpenCode runs Baton's tools without asking you, so the entry
  grants nothing beyond itself.
- **Your sessions in the panel.** A tab opened by OpenCode says so: *OpenCode*, then the name of
  the project folder.

## What is different: nothing reminds the agent

Claude Code runs a hook at the end of every turn, and Baton's hook reminds the agent, once, of
anything that is waiting for it. **OpenCode has no hook of that kind** — what it has are
plugins, which run inside OpenCode — so with OpenCode nothing asks at the end of a turn. In
practice:

- **A step you defer** comes back to the agent with the instruction to return to it before it
  ends its turn, and to keep the handoff's id in its notes, because nothing will remind it. A
  careful agent does. If it does not, the handoff waits in the panel, and **Resume** there
  makes it active again and copies a sentence for you to paste into OpenCode.
- **A request you open** with the shortcut reaches OpenCode only through the clipboard: Baton
  copies the sentence and brings the terminal forward, and you paste it. A request opened while
  no OpenCode session is running waits, and is copied again when the first session starts.
- **An answer the agent has not collected** waits in Baton until the agent calls again.

This is what OpenCode allows today rather than a setting of Baton's.

## Checking the registration

```text
opencode mcp list
```

lists the servers OpenCode reads, with `handoff` among them once Baton is registered.
`opencode debug config` prints the whole entry: Baton's server as the `command`,
`HANDOFF_AGENT` set to `opencode`, and `"timeout": 1800000`.

## One project instead of this user

In Settings → Agents → **Where** → **One project**, Baton writes the same entry into
`opencode.json` at the top of the folder you choose. OpenCode reads it, on top of your own
settings, when it is started in that folder or in one below it.

## Taking it back

Settings → Agents → **Remove** deletes the `"handoff"` entry from `opencode.json`, recognised by
the path of Baton's server in its `command`, and nothing else: your other servers and settings
stay as they were, in the same order. If Baton's entry was the only one under `"mcp"`, the
emptied key goes too. As for every change, a copy of the file is saved beside it first.
