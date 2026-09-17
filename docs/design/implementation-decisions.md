# Implementation decisions

Decisions taken while implementing the design, where they depart from
[REQUIREMENTS.md](REQUIREMENTS.md) or [TECHNICAL-DESIGN.md](TECHNICAL-DESIGN.md). Code comments cite
them by number ("implementation decision 7"), and the numbers do not change.

1. **Windows first.** TECHNICAL-DESIGN §13 builds the app on macOS first (M2) and on Windows later
   (M6). Development happens on Windows 11, so every cross-platform module is written with both
   `cfg(target_os)` branches, verified on Windows and compiled and unit-tested on macOS in CI. The
   dependency order of §13 is unchanged; only the platform each piece is first exercised on changes.
   Decision 7 defers macOS altogether for now.
2. **npm package name.** `handoff-mcp` was already taken on the npm registry, so the package is
   published as `baton-handoff-mcp`. The repository, the `handoff-mcp` executable, its subcommands,
   the `~/.handoff/` folder and the tool names keep the names of the design.
3. **Where the canaries run.** TECHNICAL-DESIGN §11.5 puts `canary.yml`, which runs E2E-1..11, in
   `handoff-mcp`. All but one of those scenarios need the app, so `handoff-mcp` runs the assumption
   canaries and the text-mode scenario (E2E-8), and `handoff-app` runs the end-to-end suite.
4. **Test isolation of the channel endpoint.** The design derives the Windows pipe name from the user
   name alone (DD-26). So that an end-to-end instance of the app can run next to the one in use, when
   `HANDOFF_HOME` is set (tests only, §5.12) both peers derive the pipe suffix from
   `sha256(lowercase(USERDOMAIN\USERNAME) + "|" + HANDOFF_HOME)`; without it the rule of the design
   applies unchanged. The app also honours `HANDOFF_APP_DATA_DIR`, for tests only, as its private data
   directory.
5. **Small modules built earlier than their milestone.** `license.rs`, `crash.rs`, the
   `secrets-write` feature stub (§12.2) and the egress lints were created with the app skeleton,
   because they touch `main()`; the e2e automation channel (`--features e2e`, DD-33) was built as soon
   as the UI existed, so that the exit criteria of M2 could be run.
6. **A lock-file bump is an ordinary commit.** The design bumps `server.lock.json` through a pull
   request that runs the app CI. With a single maintainer the bump is a commit on `main`, and a green
   app CI is still what accepts it.
7. **macOS is deferred.** Without a Mac and an Apple Developer account, the macOS platform work
   (T-058 – T-061) waits: the macOS CI jobs of both repositories run on manual dispatch only, and the
   `cfg(target_os = "macos")` branches written so far have to keep compiling there. Suspended until
   then: the signed and notarized macOS build of NFR-08 and the two-platform launch of REQUIREMENTS
   §1.5.
8. **The update check, npm publishing and Windows code signing are deferred.** The app contains the
   egress module and its lints, but nothing calls it: the app makes **zero** network connections, and
   the firewall test of NET-02 expects no domain at all until the update check exists (T-050, T-078).
   The npm package waits (`PUBLISH_NPM` is `false`, T-024), and the Windows build is unsigned (T-076,
   T-077). Until 2026-09-17 this decision also kept both repositories private.
9. **Canaries and end-to-end runs with a real agent are started by hand.** No CI secret is provisioned
   for a real agent (T-024, T-056): `canary.yml` in `handoff-mcp` and `e2e.yml` in `handoff-app` run
   on manual dispatch and skip without a key. NFR-15 is met by running the canaries and `pnpm e2e` after an
   agent update and recording the result in `handoff-mcp/docs/agent-facts.md`.
10. **Adapter order.** ADPT-06 commits to Codex, Cursor, GitHub Copilot, then OpenCode. The adapters
    were built in the order Codex, OpenCode, Cursor, GitHub Copilot, then Kilo Code (decision 13).
11. **Documentation-only pushes skip the Windows jobs of `handoff-app`.** A classifier job lets the
    `windows`, `security` and `ui` jobs skip on a push that changes only documentation, while a Linux
    job still checks the pages (T-079).
12. **A release stays a draft until it is published by hand.** Both release workflows create a draft
    release from the tag; publishing it is a separate decision.
13. **Kilo Code is the fifth agent**, on both of its surfaces: the Kilo CLI and its VS Code extension
    (T-080, T-081). The editor session identity written for Cursor (T-069, T-070) serves GitHub
    Copilot and Kilo Code as well.
14. **Versions keep ascending after 1.4.0.** OpenCode shipped as 1.4.0 in both repositories; Cursor,
    GitHub Copilot and Kilo Code shipped as 1.5.0, 1.6.0 and 1.7.0. 1.2.0 and 1.3.0 were never
    released, because a lower number would be a downgrade for the in-place server update (FM-24).
15. **Open source.** On 2026-09-17 both repositories were published under the MIT licence (NFR-17
    had the app proprietary), together with these design documents and the workspace scripts, which
    moved into `handoff-app` (TECHNICAL-DESIGN §3.1).
