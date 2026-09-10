# Install Baton on Windows

Baton is built and tested on Windows 11, 64-bit. It installs for your Windows user only and
needs no administrator rights.

## What you need

- **The setup program.** A release is called `Baton-<version>-win32-x64-setup.exe`; a setup
  you build yourself from the sources is called `Baton_<version>_x64-setup.exe`.
- **Microsoft Edge WebView2**, the part of Windows that draws web-based windows. Windows 11
  includes it. If it is missing, the setup downloads Microsoft's installer for it from
  `go.microsoft.com` — the only time the setup itself uses the network.
- **A supported agent**: Claude Code. Baton looks for it on the first launch.

## Check the file

If the release comes with a `SHA256SUMS` file, compare the setup with it before you run it.
In PowerShell, in the folder where you saved the setup:

```powershell
Get-FileHash .\Baton-<version>-win32-x64-setup.exe -Algorithm SHA256
```

The hash printed must be the one written beside the file name in `SHA256SUMS`. If it is not,
do not run the setup.

## The SmartScreen warning

This build of Baton is not signed with a code-signing certificate yet, so Windows cannot tell
who published it. When you open a setup downloaded from the internet, Microsoft Defender
SmartScreen stops it with a blue window:

> **Windows protected your PC**
>
> Microsoft Defender SmartScreen prevented an unrecognized app from starting. Running this
> app might put your PC at risk.

To install Baton anyway:

1. Click **More info**. The window now shows the file name and **Publisher: Unknown
   publisher**.
2. Check that the file name is the setup you downloaded, from the place you expected, and
   that its hash matched (above).
3. Click **Run anyway**.

The warning is about the missing signature, not about anything the setup does. A later build
will be signed, and the warning will go away with it.

There are two cases where the window offers no **Run anyway**:

- **Smart App Control is on** (Windows Security → App & browser control → Smart App Control).
  It blocks unsigned programs outright and has no exception for a single program, so Baton
  cannot be installed on that PC until it is signed. Do not turn Smart App Control off for one
  program: Windows only turns it back on by resetting the PC.
- **Your organisation manages the PC** and does not allow unsigned programs.

A setup you built on the same computer usually shows no SmartScreen warning at all, because
the file was not downloaded.

## Install

Run the setup and follow it. It installs for your user only — no administrator prompt — into
`%LOCALAPPDATA%\Baton\`:

| File | What it is |
|---|---|
| `handoff-app.exe` | Baton itself: the panel, the tray icon, the local log |
| `handoff-mcp.exe` | the MCP server your agent starts; Baton writes its path into the agent's settings |
| `patterns\` | the public list of secret patterns Baton hides automatically |
| `models\ocrs\` | the bundled text-recognition models, with their licence |

It adds a Start menu shortcut, and a desktop shortcut if you ask for one. Nothing runs as a
service and nothing is installed for other users.

## First launch

Baton opens a short welcome:

1. **Welcome to Baton.**
2. **Your agents**: the agents Baton found, and the exact changes it would make to their
   settings. Nothing is written until you press **Accept and register**.
   [The consent screen](consent-screen.md) explains every line.
3. **Start with your session**: the box *Start Baton when I log in* is ticked. With it,
   Baton starts hidden in the tray when you log in. Untick it if you prefer to start Baton
   yourself; while it is not running, a handoff happens in the agent's chat instead.
4. **Your shortcut**: `Ctrl+Alt+H` tells Baton what you are about to do, from anywhere (see
   [Using the overlay](using-the-overlay.md)).
5. **Ready.** From now on Baton lives in the tray.

Then restart your agent sessions: Claude Code reads its settings when a session starts.

## Where Baton keeps things

| Folder | What is in it | Removed by the uninstaller |
|---|---|---|
| `%LOCALAPPDATA%\Baton\` | the program | always |
| `%APPDATA%\Baton\` | the log (`handoff.sqlite`) and the crash files (`crashes\`) | only if you tick *Delete the application data* |
| `%APPDATA%\com.cepeppe.baton\`, `%LOCALAPPDATA%\com.cepeppe.baton\` | the data of Baton's window: WebView2's cache and storage | only if you tick *Delete the application data* |
| `%USERPROFILE%\.handoff\` | the channel token and your runbooks, shared with the MCP server | never |
| `%USERPROFILE%\.claude.json`, `%USERPROFILE%\.claude\settings.json` | your agent's settings, where Baton added its lines with your consent | never: use **Remove** first (below) |

## Update

This build does not check for updates (Settings → Updates says so). To update, run the setup
of the newer version over the installed one. Your agents' settings do not change: they point
at `%LOCALAPPDATA%\Baton\handoff-mcp.exe`, and that path stays the same. Agent sessions that
were already running keep the server they started with until you restart them.

## Uninstall

1. **First take Baton out of your agents.** In Baton, open Settings → Agents and press
   **Remove** for each registered agent. That deletes exactly the lines Baton added and
   nothing else (see [The consent screen](consent-screen.md)).
2. Quit Baton from its tray icon: **Quit**.
3. Uninstall it from Windows Settings → Apps → Installed apps → Baton → **Uninstall**.

The uninstaller removes the program, the shortcuts and the login entry. It asks whether to
**Delete the application data**, unticked: tick it to remove everything Baton wrote itself —
`%APPDATA%\Baton\` (the log and the crash files) and the two `com.cepeppe.baton` folders. It
never removes `%USERPROFILE%\.handoff\`, because your runbooks live there and the MCP server
uses that folder too, and it never edits your agents' settings.

### If you uninstalled without pressing Remove

Your agent's settings still point at a server that is gone: Claude Code reports that the
`handoff` MCP server failed to start, and a hook fails at the end of every turn. Take Baton's
lines out by hand:

- in `%USERPROFILE%\.claude.json`, delete the `"handoff"` entry inside `"mcpServers"`;
- in `%USERPROFILE%\.claude\settings.json`, inside `"hooks"` → `"Stop"` and
  `"SubagentStop"`, delete the entries whose `command` ends with `handoff-mcp.exe" hook stop`.
  Leave every other entry as it is.

Check that each file is still valid JSON when you are done. Every time Baton changed one of
these files it first saved a copy beside it, named `<file>.handoff-backup-<date and time>`;
restoring one also undoes any change made to the file since.
