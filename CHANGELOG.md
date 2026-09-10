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

Nothing has been released yet. The first release is the application as it stands, for
Windows.

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
- The Windows installer: a per-user setup into `%LOCALAPPDATA%\Baton\` that updates the
  bundled server in place while agent sessions are still running it, and an uninstaller that
  keeps the runbooks and removes Baton's own data only when asked.
- The release workflow: the unsigned setup, `SHA256SUMS` and the report of the security
  suite, as a draft release.
- The user documentation, in English and Italian.

Bundles `handoff-mcp` 0.2.0.
