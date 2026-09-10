# Troubleshooting

## First: `doctor`

The MCP server that ships with Baton can check the whole chain. In PowerShell:

```powershell
& "$env:LOCALAPPDATA\Baton\handoff-mcp.exe" doctor
```

It prints what the server resolved (its version, the agent, the tool timeout), whether the
channel token can be read, whether Baton answers on its channel, and whether the runbook
folder can be read. It ends with `doctor: nothing to repair`, or with one `problem:` line per
thing to fix. The token itself is never printed, so the report is safe to share. Run outside
an agent, it shows the agent as `unknown`: that is normal.

The server's own documentation explains every line of the report
([`doctor`](https://github.com/Cepeppe/handoff-mcp/blob/main/docs/install-without-app.md)).

## Symptoms

### Handoffs appear in the chat instead of in the panel

That is text mode: the server could not reach Baton when the agent opened the handoff.

- Baton is not running: start it. A running session finds it within about 30 seconds, and the
  next handoff uses the panel.
- Baton is running: run `doctor` and read its `problem:` lines.

### The agent says it cannot reach Baton, or that the channel refused it

The token the server and Baton share does not match, for example after restoring a backup of
`%USERPROFILE%\.handoff\`. In Settings → Agents, press **Repair the token**, then restart the
agent session (or reconnect the server, below).

### The agent says Baton must be updated

The server and Baton speak different versions of their channel: the agent is starting a server
that did not come with this Baton. In Settings → Agents, **Repair** points the agent back at
the server that ships with Baton.

### Claude Code has no handoff tools

- Settings → Agents should say **Registered**. If not, **Register** or **Repair**.
- Restart the Claude Code session: it reads its settings when it starts.
- In Claude Code, `/mcp` lists the servers; `handoff` should be connected.

### The server stopped in the middle of a session

Claude Code shows the `handoff` server as failed. The handoff itself is safe in Baton. In
Claude Code, run `/mcp` and reconnect `handoff`; the agent then picks the handoff up where it
was.

### *Baton has moved. Repair the registration so your agents can find it again.*

Baton is now somewhere other than the path written in your agents' settings. Settings →
Agents → **Repair** shows the change and rewrites it. Until then, handoffs happen in the chat.

### The shortcut does nothing

Another program holds `Ctrl+Alt+H`. Choose another combination in Settings → General →
Shortcut, or use **New request** in the tray menu.

### The terminal does not come to the front after a request

Some terminals, VS Code's among them, do not let Baton find or raise the right window. Paste
the request from the clipboard yourself; if you forget, the agent is reminded at the end of
its turn.

### Screenshot text is badly read

Windows reads text only in languages whose optical character recognition feature is
installed; otherwise Baton's bundled engine reads it, and it reads English best. Add the
language in Windows Settings → Time & language → Language & region, with its optional
features.

### *The capture could not be read, so nothing was hidden automatically.*

No text-recognition engine could run, which means the bundled models are missing from the
installation. Reinstall Baton. Meanwhile, hide what you need by hand before sending.

### *Session detached; the outcome will be delivered on the next resume*

The agent's session ended while the handoff was open. Keep going if you like; what you do is
delivered to the next session that resumes the handoff. Ask any agent session to resume it by
its id (`hf_…`, **Copy the id** on an orphaned tab).

### *Which session is this?*

Two sessions work in the same folder and Baton could not tell which one a hook came from.
Pick the one you are working in.

### The SmartScreen warning when installing

See [Install on Windows](install-windows.md).

## What to share when asking for help

- the output of `doctor` — it never contains the token;
- the crash file, if there is one (see [Crash reports](crash-reports.md)) — read it first;
- an export of the log only if needed, and after reading it: it contains the handoffs
  themselves (see [Log and export](log-and-export.md)).
