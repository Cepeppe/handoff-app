# The consent screen

Baton works by adding a few lines to your agent's settings: the MCP server the agent starts
and, for Claude Code, a hook that runs when the agent finishes a turn. It never writes them
without showing you first. This page explains every line.

## When you see it

- On the first launch, at the step **Your agents**.
- In Settings → Agents, when you press **Register** or **Repair** for an agent.

For each agent Baton found, the screen lists the changes it would make. Every line has a
**Show** control that opens the exact text: the file, where in the file, and what is written.
A line that is already as it should be says **Already in order**; one that would be written
says **Will change**. Nothing is written until you press **Accept and register**. **Not now**
writes nothing, and you can register later from Settings → Agents.

## The changes for Claude Code

There are three, shown on two lines because the two hooks run the same command.

### 1. The MCP server entry, in `%USERPROFILE%\.claude.json`

> The MCP server entry “handoff” in `C:\Users\you\.claude.json`, running
> `C:\Users\you\AppData\Local\Baton\handoff-mcp.exe` with a 30-minute tool timeout

Inside `"mcpServers"`, Baton adds:

```json
"handoff": {
  "type": "stdio",
  "command": "C:\\Users\\you\\AppData\\Local\\Baton\\handoff-mcp.exe",
  "args": [],
  "env": {
    "HANDOFF_AGENT": "claude-code",
    "HANDOFF_TOOL_TIMEOUT_MS": "1800000"
  },
  "timeout": 1800000
}
```

- `command` is the MCP server that ships with Baton. Claude Code starts it for every session;
  it is how the agent opens a handoff and hears back from you.
- `HANDOFF_AGENT` tells the server which agent it runs in, so it knows what that agent can do
  (for example, whether it can read an image).
- `timeout` and `HANDOFF_TOOL_TIMEOUT_MS` are the tool timeout, explained below.

### 2. Two hooks (Stop and SubagentStop), in `%USERPROFILE%\.claude\settings.json`

> Two hooks (Stop and SubagentStop) in `C:\Users\you\.claude\settings.json`, same command

Inside `"hooks"`, Baton adds one entry to the `"Stop"` list and the same entry to the
`"SubagentStop"` list:

```json
{
  "matcher": "",
  "hooks": [
    {
      "type": "command",
      "command": "\"C:\\Users\\you\\AppData\\Local\\Baton\\handoff-mcp.exe\" hook stop",
      "timeout": 5
    }
  ]
}
```

When the agent is about to end its turn, the hook asks Baton — in about two seconds at most —
whether something is waiting for this session: a handoff you deferred, a request you typed,
an outcome it has not collected. If so, it reminds the agent once. If Baton does not answer,
or anything is unclear, the hook says nothing and the agent stops as usual. Hooks you already
had stay where they are; Baton's entry is added beside them.

## The change for Codex

There is one, because Codex runs no hook at the end of a turn; [Codex CLI](agents/codex.md)
says what that changes.

### The MCP server entry, in `%USERPROFILE%\.codex\config.toml`

> The MCP server entry “handoff” in `config.toml`, running
> `C:\Users\you\AppData\Local\Baton\handoff-mcp.exe` with a 30-minute tool timeout; its tools
> are approved in advance, so Codex does not ask before each call

Baton adds this section to the file, or creates the file with it:

```toml
[mcp_servers.handoff]
command = 'C:\Users\you\AppData\Local\Baton\handoff-mcp.exe'
args = []
env = { HANDOFF_AGENT = "codex", HANDOFF_TOOL_TIMEOUT_MS = "1800000" }
default_tools_approval_mode = "approve"
tool_timeout_sec = 1800
```

- `command`, `HANDOFF_AGENT` and `HANDOFF_TOOL_TIMEOUT_MS` do what they do for Claude Code,
  above.
- `default_tools_approval_mode = "approve"` lets Codex run **Baton's** tools without asking you
  before each call. Without it Codex stops to ask every time, and a session started with
  `codex exec` refuses them outright. It applies to this one server: every other server and
  every command keeps the approval rules you set.
- `tool_timeout_sec` is the same 30 minutes as Claude Code's `timeout`, in seconds, which is
  what Codex counts in.

## The timeout

A handoff can take minutes of your time, and the agent waits for it. Claude Code gives every
tool call a time limit, so Baton raises it **for its own server only**, to 30 minutes, with
the `timeout` field of that one entry. If a handoff takes longer, nothing is lost: the agent
is told the handoff is still in progress and picks it up again.

The global variable `MCP_TOOL_TIMEOUT`, which would change the limit of **every** MCP server
in Claude Code, is never written. If you set it yourself, Baton leaves it exactly as it is,
on install and on uninstall.

## What else happens when you accept

- Baton creates `%USERPROFILE%\.handoff\channel.token` if it does not exist: the secret the
  server and Baton use to recognise each other. Only your Windows user can read it.
- Before changing a file, Baton saves a copy of it beside it, named
  `<file>.handoff-backup-<date and time>`.
- Everything else in the file stays: every key and value you had, in the same order and with
  the same indentation. Blank lines between entries are not kept. In Codex's `config.toml`
  comments and blank lines are kept as well, and so is the way each value is written.
- Claude Code and Codex read their settings when a session starts, so restart the sessions
  that were already running.

## Where: this user or one project

By default Baton registers for your Windows user, so every Claude Code session sees it. In
Settings → Agents → **Where** you can choose **One project** instead and pick a folder: Baton
then writes `.mcp.json` and `.claude\settings.json` inside that folder, with the same
content, and only sessions working in that folder see it. For Codex it writes
`.codex\config.toml` there, which Codex reads only for a project you have marked as trusted in
Codex; Baton never marks one for you.

## Taking it back

In Settings → Agents, **Remove** deletes exactly Baton's lines: the `"handoff"` server entry
and the two hook entries, recognised by the path of Baton's server in their command. Nothing
else is touched — your other servers and hooks stay, and `MCP_TOOL_TIMEOUT` is never Baton's
to restore. If Baton's entry was the only one in `"mcpServers"` or `"hooks"`, the emptied key
is removed too. For Codex, **Remove** deletes the `[mcp_servers.handoff]` section, recognised
the same way, and nothing else in `config.toml`. The backup copies stay where they are.

## The status of each agent

Settings → Agents shows one of:

| Status | Meaning |
|---|---|
| **Registered** | all of Baton's lines are there and point at this installation |
| **Partly registered** | some are missing; **Repair** shows what it would add |
| **Registered at another location** | the lines point at a server somewhere else, for example after Baton was moved; **Repair** rewrites them |
| **Not registered** | the agent is installed and Baton is not in its settings |
| **Not on this machine** | the agent was not found |

**Repair** goes through this same screen: it shows the change and waits for you.

## The other question onboarding asks

Onboarding also asks whether Baton should start when you log in (*Start Baton when I log in*,
ticked). That writes one login entry for your user, named `Baton`, which starts Baton hidden
in the tray. You can switch it off in Settings → General.
