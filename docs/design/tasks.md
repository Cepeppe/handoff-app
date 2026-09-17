# Tasks

The implementation was carried out as a numbered sequence of tasks, in the order below. A task id
in a code comment or in a commit message, such as `T-054`, is one of these. The list records the
order of the work and what is left; it is not a promise of dates.

| Task | Repository | Title | Status |
|---|---|---|---|
| T-001 | — | Owner decisions that shape names, formats and process | Done |
| T-002 | — | Developer machine prerequisites (Windows 11) | Done |
| T-003 | both | Create the two private GitHub repositories, local clones, initial commits, root scripts | Done |
| T-004 | handoff-mcp | handoff-mcp: TypeScript project scaffold, CLI skeleton, CI | Done |
| T-005 | handoff-mcp | Public JSON Schemas v1 (spec, outcome, runbook), fixtures, schema contract tests | Done |
| T-006 | handoff-mcp | Certain-secret pattern file, stop-words, secret fixtures, id generator, tests | Done |
| T-007 | handoff-mcp | Internal channel protocol definition, golden message fixtures, contract test | Done |
| T-008 | handoff-mcp | Tool contract document and build-time generator of descriptions and instruction texts | Done |
| T-009 | handoff-mcp | Single Executable Application build (spike A-12) and per-platform build scripts | Done |
| T-010 | — | Release credentials: minisign key, npm token, cross-repo read token, Actions budget | Done |
| T-011 | handoff-mcp | Release pipeline of handoff-mcp and the first tag v0.1.0 | Done |
| T-012 | handoff-app | handoff-app: `server.lock.json`, `fetch-server`, vendor layout, root `dev-link` | Done |
| T-013 | handoff-mcp | Validation pipeline, semantic rules S1–S6, error catalogue, `validate` CLI | Done |
| T-014 | handoff-mcp | Certain detector at ingress (`secret_treated`) and text-mode renderer | Done |
| T-015 | handoff-mcp | Capability table, agent identity resolution, heartbeat arithmetic, configuration, logging | Done |
| T-016 | handoff-mcp | Runbook reader, matcher, converter to draft spec, `runbooks search` CLI | Done |
| T-017 | handoff-mcp | MCP tool registration, input shapes, outcome rendering, text-mode-only server | Done |
| T-018 | handoff-mcp | Platform paths, token, ancestor chain, channel client with reconnection | Done |
| T-019 | handoff-mcp | `fake-app`: scripted channel listener (test double) | Done |
| T-020 | handoff-mcp | Blocking calls: in-flight table, heartbeat, resume, transfer, full pipeline, integration tests | Done |
| T-021 | handoff-mcp | `hook stop` subcommand, `doctor`, CLI polish | Done |
| T-022 | handoff-mcp | Published documentation and README of handoff-mcp | Done |
| T-023 | handoff-mcp | Canary harness with the real Claude Code: server-alone scenarios, assumption canaries, measured facts | Done |
| T-024 | — | CI secrets for canaries; enable npm publishing | Deferred |
| T-025 | both | Release v0.2.0 of handoff-mcp and bump the app lock file | Done |
| T-026 | — | Timeout strategy decision (OI-02) from the measured facts | Done |
| T-027 | handoff-app | Tauri 2 skeleton: Rust crate layout, configuration, features, env overrides, lints, CI | Done |
| T-028 | handoff-app | Frontend skeleton: Svelte, i18n en/it, view switching, tray, window basics | Done |
| T-029 | handoff-app | Rust format layer: types, schema validation, certain detector, matcher, ids, contract tests | Done |
| T-030 | handoff-app | SQLite persistence: schema, migrations, repositories, delete/export, invariants helper | Done |
| T-031 | handoff-app | Channel listener: named pipe with DACL, Unix socket, token file, auth, framing, ping | Done |
| T-032 | handoff-app | Session registry, ancestor-chain completion, session_id binding | Done |
| T-033 | handoff-app | Handoff store, state machine, outcome builder, undelivered queue, timers | Done |
| T-034 | handoff-app | Channel ↔ store dispatch, `fake-server` double, app-side integration tests F-01..F-11 | Done |
| T-035 | handoff-app | Stop-hook decision logic and user-request queue core | Done |
| T-036 | handoff-app | UI bridge, view model, tab strip, step view, action sheets | Done |
| T-037 | handoff-app | Secondary views, banners, collapsed bar, window and tray behaviour | Done |
| T-038 | handoff-app | User-opened requests: global shortcut, request sheet, clipboard, terminal focus, linking UI | Done |
| T-039 | handoff-app | Claude Code installation adapter (core, golden-file tests) | Done |
| T-040 | handoff-app | Onboarding, consent screen, Agents settings page, scan and repair | Done |
| T-041 | handoff-app | General settings, autostart, language, i18n audit | Done |
| T-042 | handoff-app | Startup sequence integration, restore on launch, smoke test with the real Claude Code | Done |
| T-043 | handoff-app | e2e automation channel, cross-repo e2e driver, scenarios E2E-1/2/4/5/6/7/9/10/11 | Done |
| T-044 | handoff-app | Runbook writer, update proposals, failed marks, server round-trip | Done |
| T-045 | handoff-app | Settings: Log page and Runbooks page; verification UI polish; log invariants in suites | Done |
| T-046 | handoff-app | Capture backend, region selection overlays, Screenshot button flow, `FakeCapture` | Done |
| T-047 | handoff-app | OCR engines: Windows OCR, bundled `ocrs` fallback, selection and fallback | Done |
| T-048 | handoff-app | Suspected detector, redaction geometry and burn-in, synthetic corpus, metrics and glyph-leak tests | Done |
| T-049 | handoff-app | Preview UI, send image/text, `sends` log, E2E-3 | Done |
| T-050 | — | Update endpoint hosting and download page | Deferred |
| T-051 | handoff-app | Single egress module, lint enforcement, Network page, zero-egress test (update check deferred to T-078) | Done |
| T-052 | handoff-app | User documentation: install, consent, firewall test, SmartScreen, trust, third-party notices | Done |
| T-053 | handoff-app | Security test suite (§11.7) | Done |
| T-054 | handoff-app | Windows installer (NSIS, in-place server update) and app release pipeline (Windows) | Needs review |
| T-055 | handoff-app | WebDriver UI tests (`tauri-driver`) | Done |
| T-079 | handoff-app | CI: documentation-only pushes skip the Windows jobs | Done |
| T-056 | handoff-app | App canary / release watch (E2E-1..11 on new Claude Code versions) | Deferred |
| T-058 | — | Apple Developer Program, certificates, notarization credentials, a Mac for testing | Deferred |
| T-059 | handoff-app | macOS platform code: Vision OCR, permission flow, focus, window level, autostart, relocation | Deferred |
| T-060 | both | macOS signing and notarization pipeline, DMG, update manifest | Deferred |
| T-061 | — | macOS manual matrix, firewall test with Little Snitch, clean-Mac install | Deferred |
| T-063 | — | Launch decisions: EULA, branding, website, repository visibility | Deferred |
| T-064 | both | Launch release 1.0.0, docs publication, public-repository checks | Deferred |
| T-065 | — | Codex CLI installation and account | Done |
| T-066 | handoff-mcp | Codex adapter: capability row, canary, degraded-path validation | Done |
| T-067 | handoff-app | Codex adapter: installation adapter, consent, e2e subset, docs | Done |
| T-068 | — | Cursor installation and account | Done |
| T-069 | handoff-mcp | Cursor adapter: editor session identity, capability row, canary | Done |
| T-070 | handoff-app | Cursor adapter: installation adapter, editor focus, e2e subset, docs | Done |
| T-071 | — | GitHub Copilot access | Done |
| T-072 | both | GitHub Copilot adapter (server + app) | Done |
| T-073 | — | OpenCode installation | Done |
| T-074 | both | OpenCode adapter (server + app) | Done |
| T-080 | — | Kilo Code installation and access (CLI and VS Code extension) | Done |
| T-081 | both | Kilo Code adapter (server + app): CLI and VS Code surfaces, release 1.7.0 | Done |
| T-057 | — | Windows manual matrix, firewall test, second-user test, SmartScreen check | Planned |
| T-062 | both | Failure-mode coverage audit, requirements traceability, test matrix | Planned |
| T-075 | both | Post-adapter release and traceability refresh | Planned |
| T-076 | — | Windows code-signing certificate | Deferred |
| T-077 | handoff-app | Windows signing step in the release pipeline | Deferred |
| T-078 | handoff-app | Update check, Updates settings page, release manifest | Deferred |
