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

## [1.7.1] - 2026-09-17

Baton is open source, under the MIT licence, and a certain secret inside an array value is masked
again. Bundles `handoff-mcp` 1.7.1.

### Security

- A certain secret inside an **array** value is masked again. The server reports an array one
  item at a time (`values.events[0]`), and the overlay and the outcome compared the location with
  `values.<name>` alone, so the value chips showed such an item unmasked and `context.step_values`
  of a question or a failure sent it to the agent in full. The value is now secret-treated
  whenever any of its items matched, one rule for the overlay, the outcome and the runbook
  writer (`SecretTreated::is_in_value`).
- `rustls` 0.23.45, for [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285):
  TLS 1.3 handshake messages were accepted across a change of encryption level.

### Changed

- Baton is open source, under the MIT licence: `LICENSE`, the installer's licence page, the
  manifests and the notices say so.
- `scripts/fetch-server.mjs` needs no token, `handoff-mcp` being public. It still sends
  `GH_TOKEN`, `GITHUB_TOKEN` or the one of `gh auth token` when there is one, to stay clear of
  the anonymous rate limit, and a token the API refuses is dropped rather than fatal; CI passes
  the run's own `github.token` instead of a cross-repository secret.
- The workflows pin every action to a commit, and Dependabot keeps the pins, the npm packages
  and the crates current once a month.
- Bundles `handoff-mcp` 1.7.1, whose releases publish the licence texts of Node.js and of the
  npm packages beside each executable; the server itself is unchanged.

### Added

- `docs/design/`: the requirements and the technical design both repositories were built from,
  the decisions taken while implementing them (cited in comments as "implementation decision
  N") and the task list that comments and commit messages cite as `T-054`.
- `scripts/workspace/`: `bootstrap`, `dev-link` and `e2e` for a developer with both repositories
  checked out side by side, in this repository rather than in a folder around it.
- `SECURITY.md`, and a secret-scanning configuration that leaves out the synthetic keys of the
  test corpus.
- `CONTRIBUTING.md`: which changes go straight to a pull request and which start with an issue,
  the checks a pull request runs, and the licence of a contribution.

### Fixed

- The VS Code goldens of the Copilot project installation are committed: `.gitignore` left them
  out, so in CI and in any clone the test compared the `.github` file alone.

## [1.7.0] - 2026-09-12

Kilo Code joins Claude Code, Codex, OpenCode, Cursor and GitHub Copilot, in the Kilo CLI and in
its VS Code extension. Bundles `handoff-mcp` 1.7.0.

### Added

- The Kilo Code installation adapter: one change, shown on the consent screen and removable from
  Settings → Agents, for both of Kilo Code's surfaces — the MCP server entry in
  `~/.config/kilo/kilo.json` (or under `XDG_CONFIG_HOME`), with a 30-minute tool timeout, which
  the Kilo CLI and the `kilo serve` of the VS Code extension both read. The extension's own
  `kilo.jsonc` is never touched, a `kilo.json` with comments in it is refused rather than
  rewritten, and a project installation writes `kilo.json` in the project. Baton finds Kilo
  Code by its CLI, its settings folder or the extension's folder in VS Code. Kilo Code has base
  support, and `docs/agents/kilo-code.md` says what that changes — nothing reminds the agent at
  the end of a turn, and your requests reach it by the clipboard — and walks through one
  handoff in VS Code by hand.
- A Kilo Code subset of the end-to-end suite, run with `scripts\e2e.ps1 -Agent kilo-code` on
  free models of the Kilo Gateway: E2E-1, 2, 4, 7, and 9 by the clipboard, against the Kilo
  CLI, after a preflight in which the real Kilo reads the entry the installer writes.

### Changed

- Bundles `handoff-mcp` 1.7.0, whose capability table knows Kilo Code.

## [1.6.0] - 2026-09-11

GitHub Copilot joins Claude Code, Codex, OpenCode and Cursor, in VS Code and in the Copilot CLI.

### Added

- The GitHub Copilot installation adapter: two changes, shown on the consent screen and
  removable together from Settings → Agents, with every other server of each file kept in its
  order. The MCP server entry for the Copilot CLI, in `~/.copilot/mcp-config.json` (or where
  `COPILOT_HOME` points), with a 30-minute tool timeout and every tool of the server; and the MCP
  server entry for VS Code, whose chat is Copilot's, in VS Code's user `mcp.json`. A project
  installation writes `.github\mcp.json` and `.vscode\mcp.json` in the project, leaving the
  `.mcp.json` Claude Code reads alone. A file with comments in it is refused rather than
  rewritten. Copilot has base support, and `docs/agents/copilot.md` says what that changes:
  nothing reminds the agent at the end of a turn, your requests reach it by the clipboard, and
  the Copilot CLI's print mode needs `--allow-tool=handoff` to call Baton's tools.
- A session VS Code starts is shown under the folder of its window, one session per window,
  which VS Code gives to the server only as the roots of its MCP client.
- A GitHub Copilot subset of the end-to-end suite, run by hand with
  `scripts\e2e.ps1 -Agent copilot`: the session of VS Code, measured by launching a VS Code of
  its own with no chat request, and five scenarios against the Copilot CLI.

### Changed

- A Stop hook is matched only to the nearest session above it in the process tree, and to none
  when that session's agent runs no hook of Baton's: the Copilot CLI runs a project's Claude Code
  hooks as its own, and such a hook can no longer be taken for one of a Claude Code session
  further up.

Bundles `handoff-mcp` 1.6.0.

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
