# Changelog

All notable changes to Baton, the overlay application of the contextual handoff system, are
documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Baton
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). The version is the one
`src-tauri/Cargo.toml` and `package.json` both carry. A release is a `## [<version>] - <date>`
section here and the tag `v<version>`: the release workflow refuses a tag without its
section, and the section becomes the notes of the draft release it creates. The MCP server
Baton bundles keeps its own changelog in `handoff-mcp`, and which release of it a version of
Baton carries is `server.lock.json`.

## [Unreleased]

## [1.5.0] - 2026-09-11

Cursor joins Claude Code, Codex and OpenCode, in its editor and in its Agent CLI. `1.2.0` and
`1.3.0` stay unreleased for good: a number below `1.4.0` would read as a downgrade to the
in-place update of an installation, so the adapters that follow `1.4.0` take `1.5.0` onward.

### Added

- The Cursor installation adapter: one change to Cursor's `mcp.json`, the MCP server entry,
  shown on the consent screen and removable from Settings → Agents, with every other server of
  the file kept in its order. Cursor's editor and its Agent CLI both read that file, so one
  registration covers the two. The entry carries no timeout, because Cursor reads none, and no
  permission: Cursor asks before it runs a tool, as it does for every server. A file with
  comments in it is refused rather than rewritten. Cursor has base support, and
  `docs/agents/cursor.md` says what that changes: nothing reminds the agent at the end of a
  turn, a call lasts a minute before the agent is told to pick the handoff up again, and your
  requests reach it by the clipboard.
- A session started by Cursor's editor is shown under the folder of its window, one session per
  window, and is told apart from the other windows of the same editor by that folder.
- A Cursor subset of the end-to-end suite, run by hand with `scripts\e2e.ps1 -Agent cursor`: the
  session of Cursor's editor, measured by launching an editor of its own with no agent request,
  and five scenarios against the Agent CLI.

### Changed

- A request you open with the shortcut brings forward the window whose title names the
  session's project folder, when the program in front of the session has several windows: the
  right Cursor or VS Code window rather than the last one used.
- A Stop hook is matched only to a session whose agent runs one, so a Cursor, Codex or OpenCode
  session can no longer be taken for the owner of a Claude Code hook, and the home folder Cursor
  starts its servers in is never the folder a hook is matched by.

Bundles `handoff-mcp` 1.5.0.

## [1.4.0] - 2026-09-11

OpenCode joins Claude Code and Codex. `1.2.0` and `1.3.0` are not released, and will not be:
the adapters that follow take `1.5.0` onward.

### Added

- The OpenCode installation adapter: one change to OpenCode's `opencode.json`, the MCP server
  entry with a 30-minute tool timeout, shown on the consent screen and removable from
  Settings → Agents, with every other server and setting of the file kept in its order. A file
  with comments in it is refused rather than rewritten. OpenCode sessions appear in the panel
  under their own name. OpenCode has base support, and `docs/agents/opencode.md` says what that
  changes: nothing reminds the agent at the end of a turn, and your requests reach it by the
  clipboard.
- An OpenCode subset of the end-to-end suite, run by hand with
  `scripts\e2e.ps1 -Agent opencode`.

Bundles `handoff-mcp` 1.4.0.

## [1.1.0] - 2026-09-11

The first release: the application as it stands, for Windows.

### Added

- The overlay: a panel that stays on top, one tab per handoff, one step at a time with how
  many remain, the answers the agent waits for, the verification that follows and the banner
  that says who is waiting on whom; the tray icon; `Ctrl+Alt+H` to tell the agent what you
  are about to do.
- The channel the bundled `handoff-mcp` server talks to: a named pipe for this Windows user
  alone, the channel token, the session identity taken from the process tree, and the
  decisions of the Stop hook.
- Screenshots that are read before they can leave: local OCR (Windows OCR, and the bundled
  `ocrs` engine where no OCR language pack is installed), certain secrets burned out of the
  image, suspected ones flagged, and a preview before anything is sent.
- Runbooks written after a verified handoff, the local log and its export, and the General,
  Agents, Log, Runbooks and Network settings pages.
- The Claude Code installation adapter with its consent screen, onboarding and the repair
  offer.
- The Codex CLI installation adapter: one change to Codex's `config.toml`, the MCP server
  entry with its tools approved in advance and a 30-minute tool timeout, shown on the consent
  screen and removable from Settings → Agents, with every comment and setting of the file
  kept. Codex sessions appear in the panel under their own name. Codex has base support, and
  `docs/agents/codex.md` says what that changes: nothing reminds the agent at the end of a
  turn, and your requests reach it by the clipboard.
- The Windows installer: a per-user setup into `%LOCALAPPDATA%\Baton\` that updates the
  bundled server in place while agent sessions are still running it, and an uninstaller that
  keeps the runbooks and removes Baton's own data only when asked.
- The release workflow: the unsigned setup, `SHA256SUMS` and the report of the security
  suite, as a draft release.
- The user documentation, in English and Italian.

Bundles `handoff-mcp` 1.1.0.
