# Contextual Handoff System — Technical Design

| | |
|---|---|
| Status | Draft 1.0 |
| Date | 2026-09-07 |
| Sources | `REQUIREMENTS.md` (Draft 0.2, **authoritative**), `DESIGN-TREE.md` (decision record, read for intent), `IDEA.md` (product framing) |
| Audience | Engineers implementing the MCP server, the overlay app and the agent adapters |
| Language | English. Product-facing strings quoted here are the English defaults; Italian is a translation (APP-02). |

> **Published copy.** This is the technical design the implementation was written against, published
> with the source on 2026-09-17. `DESIGN-TREE.md` and `IDEA.md`, which it cites, are the maintainer's
> working notes and are not published. The repository layout and licensing sections (§3.1, §3.3,
> §3.7) reflect the decision of 2026-09-17 to publish both repositories under the MIT licence.
> Decisions taken during the implementation that depart from this document are listed in
> [implementation-decisions.md](implementation-decisions.md), and code comments cite them as
> "implementation decision N".

---

## 1. Introduction

### 1.1 Purpose

This document states **how** the system fixed by `REQUIREMENTS.md` is built: the two repositories and the boundary between them, the public formats and the tool contract, the internal server ↔ app protocol, the internal architecture of the app, the behaviour in every flow and failure condition, the testing strategy, the implementation order and the risks. A developer implements from it without re-deriving decisions. Requirement text is not repeated; requirements are cited by identifier.

### 1.2 Sources and precedence

`REQUIREMENTS.md` is authoritative. `DESIGN-TREE.md` explains why each requirement exists and is used to understand intent, never to reopen decisions. `IDEA.md` gives the product framing. Where the sources differ, this design follows `REQUIREMENTS.md` and records the difference in §15. `REQUIREMENTS.md` names `DESIGN-TREE.md` as its own tie-breaker; no case was found in which following `REQUIREMENTS.md` contradicts a decision recorded in `DESIGN-TREE.md`.

### 1.3 Conventions

| Marker | Meaning | Where indexed |
|---|---|---|
| `AREA-nn` | A requirement in `REQUIREMENTS.md`. Every design element cites the requirements it satisfies. | Appendix A (traceability) |
| **DD-nn** | A design decision, with the alternatives considered and why they were rejected. | Appendix C |
| **A-nn** | An assumption, with the method that verifies it. | Appendix B |
| **F-nn** | A flow (sequence diagram). | §9 |
| **FM-nn** | A failure mode. | §10 |
| **R-nn** | A risk. | §14 |
| **OI-nn** | An open issue or a recorded contradiction. | §15 |

Identifiers, constants, file names and paths given in this document are normative for the implementation unless marked *example*. JSON examples are illustrative but schema-valid. Pseudocode and interface signatures describe behaviour; they are not application code.

Terms: **agent** is the MCP client process (Claude Code at launch); **server** is `handoff-mcp`; **app** is the overlay application; **channel** is the internal server ↔ app socket protocol (§6); **call** is one blocking invocation of `handoff_to_user`. The full glossary is §16.

### 1.4 Scope of this design version

Launch scope is Claude Code on macOS, then Windows, then the adapters in the committed order Codex → Cursor → GitHub Copilot → OpenCode (ADPT-06). The design covers all of these; the implementation plan (§13) orders them. Items deferred by the owner in `REQUIREMENTS.md` §14 (license verification, Linux, Windows code signing) are left untouched; the remaining §14 items are closed in §12.

---

## 2. Architecture overview

### 2.1 Components and trust boundaries

```mermaid
flowchart LR
  subgraph AGENT["Agent process (Claude Code), the user's trust domain"]
    A[Agent]
    H["hook subprocess<br/>handoff-mcp hook stop"]
  end
  subgraph SRV["handoff-mcp (MIT), child of the agent"]
    S[MCP server<br/>stdio]
  end
  subgraph APP["Overlay app (MIT, Tauri), single long-lived process"]
    L[Channel listener]
    ST[Handoff store<br/>state machine]
    UI[Overlay UI]
    CAP[Capture, OCR, redaction]
    LOG[(SQLite log)]
    RBW[Runbook writer]
    NET[Egress point<br/>update check only]
  end
  subgraph FS["User folder ~/.handoff (shared contract)"]
    TOK[channel.token]
    SOCK[app.sock or named pipe]
    RB[runbooks/*.json]
  end
  A <-- "MCP tool calls (blocking)" --> S
  A -. "spawns at Stop / SubagentStop" .-> H
  S -- "channel: JSON-RPC over NDJSON" --> L
  H -- "channel, role hook, at most 2 s" --> L
  S -. reads .-> TOK
  L -. reads .-> TOK
  L -. listens .-> SOCK
  S -. reads .-> RB
  RBW -. writes .-> RB
  L --> ST --> UI
  UI --> CAP
  ST --> LOG
  ST --> RBW
  NET -- "one fixed domain, version number only" --> INTERNET((Update endpoint))
```

Trust boundaries (SRV-08, NFR-01, NFR-05): everything runs as the same OS user. The channel token separates our processes from other users of the machine and from accidental connections; it does not defend against a malicious process already running as the user. The only network egress is the update check, through one module (§7.13).

### 2.2 Process model and lifetimes

| Process | Started by | Lifetime | Ends when | Requirements |
|---|---|---|---|---|
| Agent (Claude Code) | User | Session | User quits | — |
| Server `handoff-mcp` | Agent, at session start, as a stdio MCP server (A-01) | Session | Agent exits; never restarted by the app | SRV-03, SRV-20, SRV-24 |
| App | User, or autostart at login | Indefinite; single instance enforced | User quits from the tray menu | APP-01, WIN-04, WIN-06 |
| Hook `handoff-mcp hook stop` | Agent, at every Stop / SubagentStop | ≤ 2 s | Exits neutrally or with a block decision | SRV-10, SRV-11, ADPT-08 |
| Capture selection overlay | App, on "Select region" | Seconds | Region chosen or cancelled | CAP-01..03 |

The server is a child of the agent and connects **to** the app; the app never launches, restarts or supervises servers (SRV-02, SRV-03, SRV-05). If the app is not running when a session starts, the server keeps retrying the socket in the background with backoff (§5.3) so that a session started before the app still registers when the app appears; until then every call degrades to text mode (SRV-14).

### 2.3 Where state lives

| State | Owner | Storage | Survives | Requirement |
|---|---|---|---|---|
| Handoff state (steps, index, notes, skips, deferrals, rounds, undelivered events, final state) | App | SQLite, write-through on every transition | App restart, interrupted calls, dead servers, detached sessions | NFR-12, SRV-21..23 |
| In-flight call table (which channel connection waits on which handoff) | Server | Memory | Nothing; rebuilt by `resume` | TOOL-07, TOOL-08 |
| Session registry (connections, PID chains, cwd, bound `session_id`) | App | Memory + SQLite `sessions` | Restart (as history) | SRV-17..20 |
| Capability table | Server | JSON compiled into the binary | — | ADPT-02, ADPT-03 |
| Runbooks | App writes, server reads | `~/.handoff/runbooks/*.json` | Uninstall | RUN-03, RUN-10 |
| Channel token | Installer (app) writes, both read | `~/.handoff/channel.token`, user-only permissions | Reinstall (regenerated only if missing) | SRV-07, INST-07 |
| Log | App | SQLite in the app data directory | Until the user deletes | LOG-01..05 |
| Settings, window positions, hook-block counters, user request queue | App | SQLite (same database) | Restart | WIN-02, SRV-12, OPEN-06 |

### 2.4 Function allocation across the two repositories (ARCH-03)

| Function | Repository | Module (§) | Why here |
|---|---|---|---|
| Spec / outcome / runbook JSON Schemas, validation, error messages | `handoff-mcp` | §4, §5.4 | Public promise, protocol fact (ARCH-01) |
| Certain-secret patterns (public) and ingress check | `handoff-mcp` | §4.6, §5.5 | Protocol fact; must hold without the app (DET-02, SPEC-13) |
| Capability table, heartbeat, resume, text mode | `handoff-mcp` | §5.6, §5.7, §5.9 | Agent facts that must hold without the app (ADPT-03, ARCH-04) |
| Runbook read, match, conversion to draft spec | `handoff-mcp` | §4.5, §5.10 | RUN-10 |
| Hook subcommand | `handoff-mcp` | §5.11 | SRV-10, public mechanism |
| Channel client, session registration, ancestor chain | `handoff-mcp` | §5.8, §6 | SRV-17, SRV-19 |
| Channel listener, session registry, handoff store and state machine | `handoff-app` | §7.3–7.5 | UI-bound state (NFR-12) |
| Overlay UI, guidance, tabs, request sheet, settings, onboarding | `handoff-app` | §7.6, §7.16 | Product |
| Capture, OCR, suspected-secret detector, redaction, preview | `handoff-app` | §7.8–7.10 | Trust-sensitive local data (ARCH-03) |
| Log, runbook creation/update, network page, crash files | `handoff-app` | §7.11–7.14 | Trust-sensitive local data |
| Installation adapters (config files, hooks, timeout variable, token) | `handoff-app` | §7.15 | INST-08 |
| Certain-secret patterns **applied to screenshots** | `handoff-app`, consuming the public pattern file of the server at build time | §7.10 | The patterns are open (DET-02); the screenshot never reaches the server before redaction, so the app must apply them itself. The app reuses the server's file; the server never depends on the app (ARCH-03). |

### 2.5 Principal design decisions

The decisions that shape everything else, each detailed where it applies:

| DD | Decision | Section |
|---|---|---|
| DD-01 | Two repositories `handoff-mcp/` and `handoff-app/`; the workspace root holds documents and dev-only scripts, nothing needed at runtime | §3.1 |
| DD-02 | Cross-cutting artifacts (schemas, patterns, channel definition, fixtures) are owned by `handoff-mcp` and consumed by the app from the published release artifact of the same version it bundles | §3.4 |
| DD-03 | The app consumes the server as a build-time dependency on a published, checksummed release artifact, pinned in a lock file; never a source import | §3.5 |
| DD-04 | The fixed launcher path is the standalone server executable itself inside the app installation; no intermediate launcher process | §3.5 |
| DD-05 | Server: TypeScript on Node.js, bundled and compiled to a Single Executable Application per platform | §5.1 |
| DD-06 | Channel: JSON-RPC 2.0 over newline-delimited JSON on the local socket, authenticated by the installation token in the first message | §6.1 |
| DD-07 | `handoff_to_user` input is one flat object whose shape (open / continue / resume) the server infers, with exclusive-field validation | §4.7 |
| DD-08 | Every outcome carries `final`, `status` and an English `instruction` sentence, so the degraded modes of PRIN-10 always have an instruction to fall back on | §4.3 |
| DD-09 | Agent identity comes first from an environment variable written by the installer into the MCP entry, then from the MCP handshake, then `unknown` | §5.6 |
| DD-10 | The request window (OPEN-04) is a mode of the single overlay window, not a second window | §7.6 |
| DD-11 | Handoff state is persisted write-through in SQLite; the server holds no durable state | §7.4 |
| DD-12 | Interrupting events that find no attached call are queued per handoff and delivered one per call, oldest first | §7.4 |

---

## 3. Workspace and repositories

### 3.1 Workspace root (DD-01)

```
<workspace>/                     a folder with the two checkouts side by side; not a repository
├── handoff-mcp/                 git repository 1 (MIT)
└── handoff-app/                 git repository 2 (MIT)
    ├── docs/design/             these design documents
    └── scripts/workspace/       dev-only scripts that operate on both checkouts
        ├── bootstrap.sh|ps1     clone handoff-mcp beside handoff-app, report missing toolchains
        ├── dev-link.sh|ps1      point the app at a locally built server for development (fills vendor/ in
        │                        handoff-app from ../handoff-mcp/dist, bypassing the lock file; refuses to run in CI)
        └── e2e.sh|ps1           run the cross-repository end-to-end suite (§11.5) against local builds of both
```

Rules:

1. There are exactly two repositories (ARCH-02). The design documents and the workspace scripts live in `handoff-app`, the repository that consumes the other; they are documentation and conveniences, never dependencies. *Amended 2026-09-17: until then they lived in the workspace folder and were not published.*
2. **Nothing outside a repository is required at runtime or at build time by it.** Each repository builds, tests and releases from a clean checkout of itself alone; each CI pipeline runs that way, which is the proof. The workspace scripts are conveniences for a developer who has both checkouts; `dev-link` is forbidden in CI so that the pinned artifact (§3.5) is the only path a release build can take.
3. No code in `handoff-mcp` references `handoff-app` (its documentation names the app); `handoff-app` references `handoff-mcp` only through the release artifact (§3.5).

*Alternatives rejected:* a parent repository with two submodules (couples the histories and the release cycles of the two); a single monorepo (violates ARCH-02).

### 3.2 Repository `handoff-mcp/` (MIT)

```
handoff-mcp/
├── LICENSE                          MIT (NFR-17)
├── README.md                        what the server is; text mode; "usable alone" (ARCH-04)
├── package.json                     npm package "handoff-mcp" (SRV-26); bin: handoff-mcp
├── schemas/                         PUBLIC FORMAT: the three promises + runbook (ARCH-01)
│   ├── handoff-spec.v1.schema.json
│   ├── handoff-outcome.v1.schema.json
│   ├── handoff-runbook.v1.schema.json
│   └── tool-contract.v1.md          tool names, input schemas, descriptions, error catalogue (§4.7)
├── patterns/
│   └── certain-secrets.v1.json      public certain-secret patterns (DET-02) + stop-word lists (RUN-07a)
├── protocol/
│   └── channel/                     INTERNAL server ↔ app protocol definition (§6)
│       ├── README.md                "internal, subject to change without notice" (SRV-06, NFR-16)
│       ├── channel.v1.schema.json   message schemas (JSON-RPC methods, params, results)
│       └── protocol_version         a single integer, currently 1
├── fixtures/                        SHARED TEST FIXTURES (§3.4, §11.2)
│   ├── specs/{valid,invalid}/*.json
│   ├── outcomes/*.json
│   ├── runbooks/{valid,invalid}/*.json
│   ├── secrets/{positive,negative}.txt
│   ├── matching/*.json              where/goal pairs with the expected match result
│   └── channel/*.jsonl              golden message sequences per flow (§9)
├── src/
│   ├── main.ts                      CLI entry: serve (default) | hook stop | validate | runbooks | doctor
│   ├── format/                      schema loading, validation, semantic rules, error rendering (§5.4)
│   ├── secrets/                     certain detector (§5.5)
│   ├── runbooks/                    reader, normaliser, matcher, runbook → draft spec (§5.10)
│   ├── mcp/                         tool registration, descriptions, input parsing, outcome rendering (§5.2)
│   ├── calls/                       in-flight call table, heartbeat timers (§5.7)
│   ├── channel/                     socket client, framing, hello/auth, reconnect (§5.8, §6)
│   ├── adapters/                    capability table + per-agent identity code (§5.6)
│   ├── textmode/                    spec → text rendering (§5.9)
│   ├── hook/                        `hook stop` subcommand (§5.11)
│   └── platform/                    socket path, named pipe name, token file, ancestor chain (§5.8)
├── test/
│   ├── unit/
│   ├── contract/                    runs fixtures/ against validator, matcher, channel codec
│   ├── fake-app/                    an open channel listener used for integration tests (§11.3)
│   └── canary/                      Claude Code canaries (§11.5)
├── docs/                            published documentation (NFR-16): format, tool contract, runbook format,
│                                    text-mode limitations (SRV-15), channel threat model (SRV-08), internal notice
├── build/                           esbuild bundle + SEA configuration per platform (§5.1)
└── .github/workflows/               ci.yml, release.yml (§3.5), canary.yml (§11.5)
```

What belongs here and why: everything that is a protocol or agent fact (ARCH-03) and everything a user of the server **without** the app needs (ARCH-04). The channel definition is here because the server must be buildable and testable from this repository alone; being in a public repository does not make the channel a public promise. What is withheld is the stability promise (SRV-06), and `protocol/channel/README.md` says so.

### 3.3 Repository `handoff-app/` (MIT)

```
handoff-app/
├── LICENSE                          MIT (NFR-17)
├── README.md
├── server.lock.json                 pinned handoff-mcp release: version + sha256 per asset (§3.5)
├── scripts/fetch-server.*           downloads, verifies and unpacks the pinned artifact into vendor/ (§3.5)
├── vendor/handoff-mcp/              output of fetch-server, git-ignored:
│   ├── bin/<platform>/handoff-mcp[.exe]
│   └── format/ (schemas, patterns, protocol, fixtures)
├── src-tauri/                       Rust core (Tauri 2)
│   ├── tauri.conf.json              bundle: externalBin = vendor server binary; resources = models/ocrs, patterns
│   ├── capabilities/                Tauri permission capabilities per window
│   ├── src/
│   │   ├── main.rs                  startup sequence (§7.2)
│   │   ├── channel/                 listener, auth, JSON-RPC dispatch, connection registry (§7.3)
│   │   ├── sessions/                session registry, ancestor-chain resolution, session_id binding (§7.5)
│   │   ├── store/                   handoff store, state machine, persistence, undelivered-event queue (§7.4)
│   │   ├── hook/                    stop-hook decision logic (§7.5)
│   │   ├── requests/                user-opened request queue, clipboard text, terminal focus (§7.7)
│   │   ├── capture/                 monitor capture, region selection overlay, FakeCapture (e2e) (§7.8)
│   │   ├── ocr/                     OcrEngine trait; vision.rs, windows.rs, ocrs.rs (§7.9)
│   │   ├── redaction/               certain (from vendor patterns) + suspected detectors, boxes, burn-in (§7.10)
│   │   ├── log/                     SQLite schema, migrations, queries, export (§7.11)
│   │   ├── runbooks/                writer, placeholder substitution, update proposals (§7.12)
│   │   ├── net/egress.rs            THE single network point + update check (§7.13)
│   │   ├── install/                 adapters: claude_code.rs, codex.rs, cursor.rs, copilot.rs, opencode.rs (§7.15)
│   │   ├── license.rs               isolated entry point, returns Full in v1 (LIC-02)
│   │   ├── crash.rs                 panic hook → local crash files (TEL-02)
│   │   ├── i18n.rs                  en / it (APP-02)
│   │   └── ui_bridge/               Tauri commands and events consumed by the frontend
│   └── Cargo.toml                   cargo-deny: reqwest allowed only in net::egress (§7.13)
├── src/                             frontend (TypeScript, Svelte, Vite): overlay, request sheet, settings, onboarding
│   └── locales/{en,it}.json
├── installer/                       macOS notarization scripts; NSIS hooks for the Windows server-binary swap (§3.5)
├── docs/                            user documentation: install, consent screen, firewall test (NET-02), SmartScreen note (NFR-09);
│                                    design/ holds these design documents
├── scripts/workspace/               bootstrap, dev-link, e2e: conveniences for both checkouts side by side (§3.1)
├── tests/
│   ├── fixtures/screenshots/        OCR / redaction corpus (app-only; large, trust-sensitive)
│   ├── contract/                    runs vendor/format/fixtures against the Rust codec and validators (§11.2)
│   ├── fake-server/                 a channel client that impersonates handoff-mcp for app integration tests (§11.3)
│   └── e2e/                         cross-repository suite driver (§11.5)
└── .github/workflows/               ci.yml (clean checkout, fetch-server, build, test), release.yml
```

`ci.yml` runs its Windows jobs only for a push that changes code: a documentation-only push runs a classifier job and the documentation checks on Linux instead, and the run still exists and concludes `success` (implementation decision 11, T-079).

What belongs here: everything that touches the UI or trust-sensitive local data (ARCH-03), plus installation adapters (INST-08), plus the packaging that turns the two into one signed binary (NFR-06, NFR-08).

### 3.4 Cross-cutting artifacts and their owners (DD-02)

| Artifact | Owner (source of truth) | Consumers | How consumed |
|---|---|---|---|
| Spec / outcome / runbook JSON Schemas | `handoff-mcp/schemas/` | Server (runtime validation), app (rendering, runbook writing, contract tests), agents and third parties (docs) | App: from the pinned release artifact; third parties: from the npm package and the docs site |
| Tool contract document | `handoff-mcp/schemas/tool-contract.v1.md` | Server (descriptions are generated from it at build time), docs | Same |
| Certain-secret patterns, stop-word lists | `handoff-mcp/patterns/` | Server (ingress check, matching), app (screenshot redaction, runbook redaction) | App: pinned artifact, embedded as a resource; the app never edits them |
| Channel protocol definition | `handoff-mcp/protocol/channel/` | Server (client), app (listener), fake-app and fake-server test doubles | Both sides verify their codecs against `channel.v1.schema.json`; `protocol_version` is compared at hello |
| Shared test fixtures | `handoff-mcp/fixtures/` | Both test suites | App CI fetches the pinned artifact and runs the same fixtures through its Rust implementations |
| Screenshot corpus | `handoff-app/tests/fixtures/screenshots/` | App only | Large and trust-sensitive; the server never handles pixels |
| Capability table | `handoff-mcp/src/adapters/capabilities.json` | Server (runtime), app (read-only, via hello) | The app receives the resolved row per session in `hello` (§6.3) |

Rule: a cross-cutting artifact changes only in `handoff-mcp`, is released, and reaches the app by bumping `server.lock.json`. The app never carries a divergent copy.

*Alternatives rejected:* defining the channel in the app repository (the server could not be built or tested from its own repository; against the spirit of ARCH-04); a third "shared" repository (ARCH-02 fixes two; a third adds release coordination for no gain); duplicating the pattern file in both repositories (guaranteed drift between what the server treats as secret at ingress and what the app redacts).

### 3.5 How the app consumes the server (DD-03, DD-04)

**Release artifact.** Every tagged release of `handoff-mcp` publishes, on the GitHub release and on the npm registry where applicable:

| Asset | Content |
|---|---|
| `handoff-mcp-<ver>-darwin-arm64`, `handoff-mcp-<ver>-darwin-x64`, `handoff-mcp-<ver>-win32-x64.exe` | Standalone executables (SRV-24) |
| `handoff-mcp-<ver>-format.tar.gz` | `schemas/`, `patterns/`, `protocol/`, `fixtures/`, `docs/` |
| `SHA256SUMS`, `SHA256SUMS.minisig` | Checksums and a detached signature (minisign key held by the owner) |
| npm `handoff-mcp@<ver>` | JavaScript package for users of the server alone (SRV-26); contains `schemas/`, `patterns/`, `docs/` |

**Pinning.** `handoff-app/server.lock.json`:

```json
{
  "version": "1.0.3",
  "assets": {
    "darwin-arm64": { "sha256": "…" },
    "darwin-x64":   { "sha256": "…" },
    "win32-x64":    { "sha256": "…" },
    "format":       { "sha256": "…" }
  },
  "protocol_version": 1
}
```

`fetch-server` downloads the assets for the target platform, verifies the checksums and the signature, refuses to proceed on any mismatch, and unpacks into `vendor/handoff-mcp/`. The Tauri build fails if `vendor/` is absent or its recorded version differs from the lock file. The `protocol_version` in the lock must equal the one compiled into the listener of the app; the build fails otherwise (compatibility is settled at build time, never at run time, §3.6).

**Placement and the fixed launcher path (SRV-25, SRV-19).** The server binary is bundled as a Tauri external binary so it is signed and notarized with the app (NFR-08). The path registered in the agent configuration is the binary itself:

| Platform | Registered path (example; the installer writes the actual location) | Parent of the server process |
|---|---|---|
| macOS | `/Applications/Baton.app/Contents/MacOS/handoff-mcp` | The agent (Claude Code) |
| Windows | `%LOCALAPPDATA%\Baton\handoff-mcp.exe` | The agent; a native executable, never `.cmd` (SRV-19) |

Updates replace the file in place, so the path in the agent configuration never changes (SRV-25, UPD-02). Two platform details:

- **Windows in-place update while sessions are running.** An executable in use cannot be overwritten but can be renamed. The NSIS installer hook renames `handoff-mcp.exe` to `handoff-mcp.<oldver>.old.exe` before copying the new file; running sessions keep their old binary; the app deletes `*.old.exe` at launch once they are no longer locked (FM-24).
- **macOS relocation.** If the user moves the bundle, the registered path breaks. At every launch the app compares its own bundle path with the registered one and, if they differ, offers a one-click repair that rewrites the MCP entry and hook commands (FM-23). Onboarding asks the user to move the app to `/Applications` first, which also avoids Gatekeeper path translocation.

Never a source import: the app repository contains no TypeScript from the server, no git submodule, no path dependency. The only coupling is the artifact and its version.

*Alternatives rejected:* **(B)** a copy of the server managed by the app under `~/.handoff/bin/`: brand- and location-independent path, but duplicates a ~90 MB binary and moves the Windows locked-file problem into the app; **(C)** a tiny native launcher at a fixed path that finds the current app and starts the bundled server: stable path, but on Windows it is an intermediate process (no `exec`), needs a job object to die with the agent, a pointer file to find the app, and versioned server directories; more moving parts than the repair flow of the chosen option. SRV-19 already mandates registering the full ancestor chain, so (C) remains available later without protocol changes.

### 3.6 Versions and compatibility

| Version | Carried in | Bumped when | Compatibility rule |
|---|---|---|---|
| `spec_version` (integer, 1) | Every spec | A field is added, removed or changes meaning | Server accepts own and all previous; higher → rejected "update the server" (SPEC-11) |
| `outcome_version` (integer, 1) | Every outcome | Same | Producer is the server; agents read what they get; a bump is announced in the tool description |
| `runbook_version` (integer, 1) | Every runbook file | Same | Server reads own and previous; app writes the current |
| Channel `protocol_version` (integer, 1) | `hello` (§6.3) | Any message change | Exact equality required; mismatch → app answers `protocol_unsupported`, server falls back to text mode with an "update" instruction (FM-11) |
| `patterns_version` | `certain-secrets.v1.json` | Pattern set changes | Informational; recorded in the log with each send |
| `handoff-mcp` package version (semver) | npm, release tag, `hello.server_version` | Every release | Pinned by the app lock file |
| App version (semver) | About page, update check | Every release | Bundles exactly one server version |

Format versions and package versions are independent: a server 1.4.0 may still speak `spec_version` 1.

### 3.7 Licensing per repository (NFR-17)

Both repositories are MIT: `handoff-mcp` including schemas, patterns, the channel definition and fixtures, and `handoff-app` (amended 2026-09-17; it had a proprietary binary licence until then). The bundled server keeps its own notice in the third-party notices of the app, beside the other components the app ships. Nothing in the app repository is modified server code; the app links to nothing from the server at the source level.

---

## 4. Public formats and tool contract

The three public promises (ARCH-01) are the handoff spec (§4.2), the outcome (§4.3) and the tool contract (§4.7). The runbook format (§4.5) and the certain-secret patterns (§4.6) are public as well (RUN-03, DET-02). Everything in this section is written into `handoff-mcp/schemas/` and `handoff-mcp/patterns/` and published (NFR-16). All texts the server returns to an agent are English (REQUIREMENTS §5 naming note).

### 4.1 Identifiers and constants

| Identifier | Format | Assigned by | Notes |
|---|---|---|---|
| `handoff_id` | `hf_` + 10 characters of lowercase Crockford base32 (`0-9 a-h j k m n p-t v-z`), 50 random bits; regex `^hf_[0-9a-hjkmnp-tv-z]{10}$` | App (or server in text mode: none) | Also the id of a user-opened request: when a spec arrives with `request_id`, the handoff **takes that id**, so one tab keeps one id from "waiting for spec" to its final state (**DD-13**). The certain-pattern for Hugging Face tokens requires `hf_` + 34 alphanumerics, so ids never match a secret pattern; a contract test asserts that no generated id matches any certain pattern. |
| `request_id` | Same as `handoff_id` | App | Appears in the clipboard text (OPEN-05) |
| `session_ref` | `ses_` + 8 characters, same alphabet | App, per registration | Internal to the channel; never shown to agents |
| `call_id` | `call_` + 8 characters | Server, per blocking call | Internal to the channel |
| Runbook `id` | `rb_` + 10 characters | App | Part of the runbook file name |

*Alternative rejected for `handoff_id`:* UUIDs (long to quote in a chat, no visual prefix; the prefix lets an agent recognise an id in free text).

Normative constants:

| Constant | Value | Source |
|---|---|---|
| `HEARTBEAT_MARGIN_MS` | 60 000 before the known client timeout | TOOL-06a |
| `UNKNOWN_CLIENT_HEARTBEAT_MS` | 50 000 after call start | TOOL-06a |
| `RAISED_TOOL_TIMEOUT_MS` | 1 800 000 (30 min) written as the per-server `timeout` field of the MCP entry; `MCP_TOOL_TIMEOUT` is not written (T-026, OI-02) | INST-03 |
| `VERIFYING_TIMEOUT_MS` | 1 800 000 | VER-06 |
| `ORPHAN_AGE_MS` | 7 days | SRV-23 |
| `HOOK_CONNECT_TIMEOUT_MS` / `HOOK_TOTAL_BUDGET_MS` / hard exit | 500 / 1 800 / 1 950 | SRV-11, NFR-11 |
| Hook `timeout` written in the hooks configuration | 5 s (so a stuck hook cannot hold the agent) | SRV-11 |
| Server reconnect backoff | 1, 2, 5, 10, then 30 s forever | §5.3 |
| `CHANNEL_MAX_MESSAGE_BYTES` | 16 MiB | §6.1 |
| `IMAGE_LONG_SIDE_PX` | 1600 | CAP-05 |
| `OCR_ENGINE_TIMEOUT_MS` | 10 000 per engine attempt | §7.9 |
| Token | 32 random bytes, stored as 64 lowercase hex characters | SRV-07 |
| `RUNBOOK_MATCH_MAX_RESULTS` | 5 | §4.5 |
| Shared user folder | `~/.handoff/` (`%USERPROFILE%\.handoff\`) containing `runbooks/`, `channel.token`, `app.sock` (macOS) | RUN-03a, SRV-07, SRV-04 |
| Named pipe name (Windows) | `\\.\pipe\handoff-<h>`, `h` = first 16 hex digits of `sha256(lowercase(USERDOMAIN\USERNAME))` | SRV-04, §5.8, §6.2 |

### 4.2 Handoff spec v1 (`handoff-spec.v1.schema.json`)

JSON Schema draft 2020-12. `additionalProperties: false` at every object level except `values` and `secrets` (whose keys are user-defined). Fields per SPEC-01 and SPEC-02, with the constraints the schema enforces (**DD-14**):

| Path | Type | Constraints |
|---|---|---|
| `spec_version` | integer | `const 1` in the v1 schema; the version check runs before schema validation (§5.4) |
| `goal` | string | 1–300 characters after trimming |
| `where` | string | 1–300 |
| `url` | string | optional; `^(https?://|ms-settings:|x-apple\.systempreferences:)` (SPEC-07); ≤ 2048 |
| `why_human` | string | 1–1000 |
| `values` | object | required, may be `{}`; ≤ 50 keys; key pattern `^[A-Za-z_][A-Za-z0-9_.-]{0,63}$`; each value a string (≤ 4096) or an array of 1–100 strings (each ≤ 4096) |
| `secrets` | object | optional; ≤ 20 keys; key 1–128 characters (the variable name); value 1–1024 characters (destination file, relative to the project or absolute) |
| `steps` | array of step | 1–50 items |
| `steps[].text` | string | 1–2000 |
| `steps[].url` | string | optional; same scheme rule as `url` |
| `steps[].values` | array of string | optional; 1–20 keys, each must exist in `values` (semantic rule S3) |
| `steps[].warning` | string | optional; 1–300 |
| `verify` | string | optional; 1–4000 |
| `lang` | string | optional; BCP-47 shape `^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$` |

Rationale for the limits: the overlay is a narrow panel; unbounded strings break its layout and make error messages unreadable. Limits are generous for real handoffs and are documented next to the schema. *Alternative rejected:* no limits (a 200-step spec or a 50 KB step text would render as a broken panel). Rationale for `additionalProperties: false`: an agent that mistypes `warnings` for `warning` must learn it from the error, not have the field silently dropped (SPEC-06, SPEC-10). *Alternative rejected:* ignore unknown fields (silent loss).

Semantic rules applied after schema validation (all produce readable errors, §4.7.5):

| Rule | Check | Fix text returned |
|---|---|---|
| S1 | `spec_version` > 1 | "This server supports spec_version ≤ 1. Update the server or lower spec_version." (SPEC-11) |
| S2 | Control fields (`handoff_id`, `resume`, `request_id`, `reply`, `replacement_steps`, `ignore_runbook`) found inside `spec` | "Control fields belong outside `spec`, at the top level of the tool input." (TOOL-02) |
| S3 | Every `steps[i].values[j]` is a key of `values` | "Unknown value key `X` in steps[i].values; declare it in `values` or remove it. Known keys: …" |
| S4 | No `{{…}}` in `goal`, `where`, `why_human`, `verify`, `steps[].text`, `steps[].warning`, or in any value | "Placeholder `{{name}}` found in steps[i].text. Placeholders exist only in runbooks; replace it with the real value or move it to `values`." (SPEC-12) |
| S5 | `url` schemes | "Scheme `X` is not allowed. Allowed: http, https, ms-settings:, x-apple.systempreferences:. Show other links as plain text in the step." (SPEC-07, SPEC-08) |
| S6 | Strings are non-empty after trimming | "Field X is empty." |
| S7 | Certain-secret scan of `values` and text fields | Never an error: produces `secret_treated` (SPEC-13, DET-04, §5.5) |

A second example, showing an OS settings target and a per-step `url`:

```json
{
  "spec_version": 1,
  "goal": "Allow screen recording for the terminal so the agent's capture tool works",
  "where": "System Settings → Privacy & Security → Screen Recording",
  "url": "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
  "why_human": "macOS lets only the user grant this permission.",
  "values": { "app_name": "iTerm" },
  "steps": [
    { "text": "Unlock the panel and enable the toggle next to the terminal application.", "values": ["app_name"] },
    { "text": "Quit and reopen the terminal; the permission applies at next launch.",
      "warning": "Open sessions in the terminal will be closed." }
  ],
  "verify": "Run `screencapture -x /tmp/probe.png` from the terminal and check that the file is not black.",
  "lang": "en"
}
```

### 4.3 Outcome v1 (`handoff-outcome.v1.schema.json`)

The outcome is the JSON object returned by `handoff_to_user` and `handoff_verify`. It is also what the log stores (LOG-02) and what the runbook writer reads (TOOL-12). **DD-08:** every outcome carries `final`, `status` and `instruction`; the instruction is the fallback that holds when hooks, raised timeouts or images do not (PRIN-10). **DD-15:** statuses are one enumeration covering final states (VER-01) and the reasons a call returns (TOOL-03), so an agent branches on one field.

| `status` | `final` | Returned when | Instruction (gist; exact text in `tool-contract.v1.md`) |
|---|---|---|---|
| `in_progress` | no | Heartbeat (TOOL-06) | "The user is still working. Call handoff_to_user with resume=`<id>` now to keep waiting." |
| `question` | no | User pressed Ask (RESP-04) | "Answer on the current step: call handoff_to_user with handoff_id and reply; add replacement_steps only if the remaining steps must change." |
| `screenshot` | no | User sent a screenshot (image or text) | Same as `question`; "the image/text shows what the user sees at this step" |
| `deferred` | no | First deferral (RESP-05) | "Park this step, continue work that does not depend on it, then call handoff_to_user with resume=`<id>` before you conclude." (TOOL-14) |
| `parked` | no | Second deferral (RESP-07) | "Do not resume now. The handoff stays in the overlay; mention `<id>` as pending in your final summary." |
| `awaiting_verification` | no | "Done" on the last step with `verify` present (RESP-09) | "Perform the verification below with your own tools, then call handoff_verify with ok true/false/null and detail. Never read values listed in secrets." |
| `confirmed_by_user` | yes | "Done" on the last step, no `verify` | "The handoff is complete and recorded as confirmed by the user." |
| `verified` | yes | `handoff_verify` with `ok: true` | "Recorded as verified; a runbook was saved." |
| `failed` | yes | `handoff_verify` with `ok: false` | "If you can correct it, call handoff_to_user with handoff_id and replacement_steps that start from the actual error; otherwise tell the user." (VER-08) |
| `not_verified` | yes | `ok: null`, timeout or disconnect (VER-06) | "Recorded as not verified. If you can still verify, call handoff_verify; a late report is accepted." |
| `abandoned` | yes | User pressed Abandon (RESP-08) | "The user abandoned this handoff. Do not retry the same steps; ask the user how to proceed." |
| `transferred_to_other_session` | no | Another session resumed the handoff (TOOL-08) | "Another session took over `<id>`. Do nothing further with it." |
| `runbook_match` | no | New spec matched runbooks (RUN-07) | "A runbook exists: start from here? Fill values_to_fill and call handoff_to_user again with the completed spec and ignore_runbook=true." |
| `text_mode` | no | App unreachable (SRV-14) | "The overlay app is not running. Present the spec below to the user in chat, walk them through the steps, and collect the result in chat. No log or verified state exists in this mode." |

Fields (TOOL-13; every field is present, with `null` or `[]` when not applicable, so agents never branch on absence):

| Field | Type | Content |
|---|---|---|
| `outcome_version` | integer | 1 |
| `handoff_id` | string or null | null only for `runbook_match` and `text_mode` |
| `status`, `final`, `instruction` | see above | |
| `round` | integer ≥ 1 | Current round (VER-10) |
| `current_step` | object or null | `{ index (1-based, matches the "2 of 4" counter), total, text }` |
| `user_text` | string or null | The question, the comment attached to a screenshot, or the reason typed with Defer/Abandon |
| `screenshot` | object or null | `{ mode: "image" or "text", text, image_attached, width, height, redactions, ocr_engine }`; `text` is the OCR text as edited and sent (mode text) or null; `image_attached` says whether an MCP image block accompanies the outcome |
| `context` | object or null | For `question` and `screenshot`: `{ goal, where, step: { index, total, text, url, warning }, step_values }` where `step_values` maps the step's value names to their values, with `"[treated as secret]"` for secret-treated ones (CTX-01) |
| `skipped_steps` | array of integer | 1-based indices, current round (RESP-03) |
| `notes` | array | `{ step, text, at }` (RESP-03) |
| `secret_treated` | array | `{ location, kind }`, e.g. `{ "location": "values.api_key", "kind": "stripe_secret_key" }` (DET-04) |
| `verify` | object or null | `{ ok, detail, reported_at, late }` once reported (VER-05) |
| `deferral_count` | integer | 0, 1 or 2 |
| `resumed_from` | object or null | `{ agent, project }` of the opening session when the current call comes from a different one (TOOL-08) |
| `app_reachable` | boolean | false only in `text_mode` (TOOL-13) |
| `already_delivered` | boolean | true when a resume returns a final outcome already delivered (TOOL-07) |
| `runbooks` | array | Only for `runbook_match`: `{ id, path, where, goal, trust, last_verified_at, last_run_failed_at, runs, matched_words, draft_spec, values_to_fill, annotations }` |
| `spec_text` | string or null | Only for `text_mode`: the rendered spec (§5.9) |

MCP mapping: `content[0]` is a text block with the outcome JSON; `content[1]` is an image block (`image/png`, base64) when `screenshot.mode == "image"` and the session's capability row has `images_in_results: true`; `structuredContent` carries the same object and the tool declares `outputSchema` (MCP 2025-06-18) so clients that support it get typed output while older clients read the text block. Outcomes are never `isError`; only the errors of §4.7.5 are.

Example, screenshot in text mode with a comment:

```json
{
  "outcome_version": 1, "handoff_id": "hf_7k3m9p2q4r", "status": "screenshot", "final": false,
  "instruction": "The user sent what they see at step 2 as extracted text. Answer on this step: call handoff_to_user with handoff_id and reply; add replacement_steps only if the remaining steps must change.",
  "round": 1,
  "current_step": { "index": 2, "total": 4, "text": "Select the events checkout.session.completed and invoice.paid." },
  "user_text": "I only see 'checkout.session.async_payment_succeeded'",
  "screenshot": { "mode": "text", "text": "Select events to listen to\n[search] checkout\ncheckout.session.async_payment_failed\ncheckout.session.async_payment_succeeded\n…", "image_attached": false, "width": 2880, "height": 1800, "redactions": 0, "ocr_engine": "vision" },
  "context": { "goal": "Register the Stripe webhook for payment events", "where": "Stripe Dashboard → Developers → Webhooks",
               "step": { "index": 2, "total": 4, "text": "Select the events checkout.session.completed and invoice.paid.", "url": null, "warning": null },
               "step_values": { "events": ["checkout.session.completed", "invoice.paid"] } },
  "skipped_steps": [], "notes": [{ "step": 1, "text": "Button is called 'Add destination' now", "at": "2026-09-07T10:12:03Z" }],
  "secret_treated": [], "verify": null, "deferral_count": 0, "resumed_from": null,
  "app_reachable": true, "already_delivered": false, "runbooks": [], "spec_text": null
}
```

### 4.4 Verification report

`handoff_verify` (TOOL-10) carries `{ handoff_id, verify: { ok: true | false | null, detail: string (1–4000) } }`. Effects (VER-01, VER-03, VER-11):

| `ok` | Handoff state after | Returned outcome | Side effects |
|---|---|---|---|
| `true` | `verified` | `verified`, final | Runbook created or refreshed (RUN-01, §7.12) |
| `false` | `failed` | `failed`, final but continuable | Matching runbook marked "last run failed" if no correction follows (RUN-09) |
| `null` | `not_verified` | `not_verified`, final | Detail recorded (VER-03) |

**DD-16 (late reports).** A report that arrives after the handoff became `not_verified` by timeout or disconnect (VER-06) is accepted while the handoff is younger than `ORPHAN_AGE_MS`, re-finalises the handoff and is logged with `late: true`. Rationale: the Stop hook exists precisely to make the agent report after the fact (VER-07); refusing the report would discard an honest declaration and push agents towards silence. The state is still set only through the tool (VER-02). *Alternative rejected:* treating `not_verified` as terminal (loses honest late reports; no benefit).

A report for a handoff whose spec had no `verify` is rejected with `NO_VERIFY_IN_SPEC` (§4.7.5): the format says such a handoff is confirmed by the user, and an agent that wants verification must put `verify` in the spec.

### 4.5 Runbook v1 (`handoff-runbook.v1.schema.json`)

One JSON file per runbook in `~/.handoff/runbooks/` (RUN-03a, RUN-03b). **DD-17:** file name `<where-slug>__<goal-slug>__<id>.json`, slugs being the normalised `where` and `goal` (§4.5.3) truncated to 60 characters with non-alphanumerics replaced by `-`; readable in a file browser (RUN-03) and unique through the id. Written atomically (temporary file + rename).

| Field | Type | Content |
|---|---|---|
| `runbook_version` | integer | 1 |
| `id` | string | `rb_…` |
| `where`, `goal`, `why_human` | string | Copied from the spec of the last verified round |
| `url`, `lang` | string or null | Copied |
| `values` | object | name → `{ "description": string or null }`; the description is the text of the first step whose text contained the value literally, with `{{name}}` in its place; null when the value appeared in no step (RUN-04) |
| `secrets` | object | variable name → destination file, as in the spec (names only; never values) |
| `steps` | array | The sequence actually executed (RUN-02): `{ text (with placeholders), url, values, warning, annotations }` where `annotations` is an array of `{ kind: "note" or "question" or "reply" or "error" or "correction", text, round }` |
| `verify` | string or null | With placeholders where values appeared |
| `trust` | `"verified"` or `"confirmed_by_user"` | RUN-01 |
| `last_verified_at` | date-time | RUN-08 |
| `last_run_failed_at` | date-time or null | RUN-09 |
| `runs` | integer | Number of verified or confirmed executions folded into this file |
| `created_at`, `updated_at` | date-time | |
| `origin` | object | `{ "app": "handoff-app", "app_version": "…" }` |

#### 4.5.1 Sequence actually executed (RUN-02)

For each round in order: the round's steps minus the ones skipped in that round, minus the steps that a later round replaced before they were confirmed; replacement steps appear where they were executed. Notes become `note` annotations on their step; each Ask becomes a `question` annotation and the agent's reply a `reply` annotation; a failed verification detail becomes an `error` annotation on the last step of that round; the first step of the following round carries a `correction` annotation. Steps are the recipe; the log is the diary.

#### 4.5.2 Placeholders (RUN-04, RUN-05)

The app substitutes values in `steps[].text`, `steps[].warning` and `verify` with `{{name}}`: values sorted by length descending, exact literal match, each array item substituted with `{{name}}` as well. Secret-treated values (DET-04) are never written; their name gets `description: "[treated as secret at ingress]"` and any literal occurrence is replaced by `{{name}}` before writing. The certain detector runs once more over the finished file as a last defence; a match aborts the write and logs a warning.

#### 4.5.3 Matching rule (RUN-07a), used by `handoff_runbooks` and by the safety net

```
normalize_where(s):  NFKC → lowercase → replace every run of whitespace or of the characters
                     → > » / \ | – — - : , ; . with one space → trim
tokens(goal, lang):  NFKC → lowercase → split on non-alphanumerics → drop tokens shorter than 3
                     → drop stop-words (list for `lang` if shipped, else union of en and it)
matches(rb, where, goal, lang):
    normalize_where(rb.where) == normalize_where(where)
    and |tokens(rb.goal, lang) ∩ tokens(goal, lang)| ≥ 1
ranking: shared-token count desc, then last_verified_at desc; at most RUNBOOK_MATCH_MAX_RESULTS
```

Stop-word lists (en, it) live in `patterns/certain-secrets.v1.json` under `stop_words`, so both sides use one list. The rule is deterministic and explainable; `matched_words` is returned so the agent can see why.

#### 4.5.4 Conversion to a draft spec (RUN-05, RUN-07)

**DD-19:** `{{name}}` in step texts, warnings and `verify` becomes `[name]`; every name found is added to that step's `values`; `draft_spec.values` maps each name to `""`; `values_to_fill` maps each name to its description. The draft is intentionally invalid until the agent fills the values (empty strings fail S6), which prevents an agent from opening a handoff with placeholders or blanks. Annotations are returned alongside (`runbooks[].annotations`) but not inside the draft. *Alternative rejected:* leaving `{{name}}` in the draft text (would be rejected by S4 and forces the agent to edit every step by hand).

Example runbook:

```json
{
  "runbook_version": 1, "id": "rb_2b9x4d7fkq",
  "where": "Stripe Dashboard → Developers → Webhooks",
  "goal": "Register the Stripe webhook for payment events",
  "why_human": "Requires access to the production Stripe account.",
  "url": "https://dashboard.stripe.com/webhooks", "lang": "en",
  "values": {
    "endpoint_url": { "description": "Click Add destination and paste {{endpoint_url}} as the endpoint URL." },
    "events": { "description": "Select the events {{events}}." }
  },
  "secrets": { "STRIPE_WEBHOOK_SECRET": ".env" },
  "steps": [
    { "text": "Click Add destination and paste {{endpoint_url}} as the endpoint URL.", "url": null, "values": ["endpoint_url"], "warning": null,
      "annotations": [{ "kind": "note", "text": "Button is called 'Add destination' now", "round": 1 }] },
    { "text": "Select the events {{events}}.", "url": null, "values": ["events"], "warning": null, "annotations": [] },
    { "text": "Save and copy the signing secret.", "url": null, "values": [], "warning": null, "annotations": [] },
    { "text": "Paste it into .env as STRIPE_WEBHOOK_SECRET.", "url": null, "values": [], "warning": null, "annotations": [] }
  ],
  "verify": "Check that STRIPE_WEBHOOK_SECRET exists in .env without reading its value, then send a test event and verify it reaches /webhooks/stripe with a valid signature.",
  "trust": "verified", "last_verified_at": "2026-09-07T10:31:44Z", "last_run_failed_at": null, "runs": 1,
  "created_at": "2026-09-07T10:31:44Z", "updated_at": "2026-09-07T10:31:44Z",
  "origin": { "app": "handoff-app", "app_version": "1.0.0" }
}
```

### 4.6 Certain-secret patterns (`patterns/certain-secrets.v1.json`)

**DD-20:** one public JSON file, versioned, consumed unchanged by server and app. Regexes are written in the common subset of JavaScript `RegExp` (flag `u`) and the Rust `regex` crate: no look-behind, no back-references, no possessive quantifiers; a contract test compiles every pattern on both sides and runs the embedded `match` / `no_match` examples plus `fixtures/secrets/`.

```json
{
  "patterns_version": 1,
  "patterns": [
    { "id": "private_key_block", "kind": "private_key",
      "regex": "-----BEGIN (?:[A-Z ]+ )?PRIVATE KEY-----", "description": "PEM private key header",
      "tests": { "match": ["-----BEGIN RSA PRIVATE KEY-----"], "no_match": ["-----BEGIN CERTIFICATE-----"] } },
    { "id": "stripe_secret_key", "kind": "api_key", "regex": "\\b[sr]k_(?:live|test)_[0-9A-Za-z]{16,}\\b", "description": "Stripe secret or restricted key" }
  ],
  "stop_words": { "en": ["the", "and", "for", "with", "into", "from", "that", "this"], "it": ["il", "lo", "la", "gli", "per", "con", "del", "della", "che", "una", "uno"] }
}
```

Policy: **certain means precision first.** A pattern enters the file only with a documented prefix or structure whose false-positive rate on ordinary text is negligible; anything entropy- or label-based belongs to the suspected detector in the app (DET-01). Initial families:

| id | Family | Shape |
|---|---|---|
| `private_key_block` | PEM private keys | header line |
| `aws_access_key_id` | AWS | `(AKIA|ASIA)[0-9A-Z]{16}` |
| `stripe_secret_key`, `stripe_webhook_secret` | Stripe | `[sr]k_(live|test)_…`, `whsec_[0-9A-Za-z]{20,}` |
| `github_token` | GitHub | `gh[pousr]_[0-9A-Za-z]{36,}`, `github_pat_[0-9A-Za-z_]{80,}` |
| `slack_token`, `slack_webhook_url` | Slack | `xox[abprs]-[0-9A-Za-z-]{10,}`, `https://hooks\.slack\.com/services/T[0-9A-Z]+/B[0-9A-Z]+/[0-9A-Za-z]+` |
| `google_api_key` | Google | `AIza[0-9A-Za-z_-]{35}` |
| `anthropic_api_key` | Anthropic | `sk-ant-[0-9A-Za-z_-]{20,}` |
| `openai_api_key` | OpenAI | `sk-(?:proj-)?[0-9A-Za-z_-]{20,}` (ordered after `anthropic_api_key`) |
| `gitlab_pat` | GitLab | `glpat-[0-9A-Za-z_-]{20}` |
| `npm_token` | npm | `npm_[0-9A-Za-z]{36}` |
| `sendgrid_api_key` | SendGrid | `SG\.[0-9A-Za-z_-]{22}\.[0-9A-Za-z_-]{43}` |
| `huggingface_token` | Hugging Face | `hf_[0-9A-Za-z]{34}` |
| `digitalocean_token` | DigitalOcean | `dop_v1_[0-9a-f]{64}` |
| `jwt` | JSON Web Token | `eyJ[0-9A-Za-z_-]{10,}\.eyJ[0-9A-Za-z_-]{10,}\.[0-9A-Za-z_-]{10,}` |

`kind` is what the outcome reports in `secret_treated` and what the log stores; the matched text itself is never stored anywhere.

### 4.7 Tool contract (`tool-contract.v1.md`)

Three tools (REQUIREMENTS §5). Input schemas below are the ones registered with MCP; the description texts are normative and generated into the server from this document at build time so the two cannot drift.

#### 4.7.1 `handoff_to_user`

**DD-07:** one flat input object; the server infers the shape from which fields are present and rejects ambiguous combinations. *Alternative rejected:* a top-level `oneOf` of three objects (correct, but several agent runtimes flatten or mishandle `oneOf` in tool schemas; a flat object with a clear description is the compatible choice; the three shapes are still spelled out in the description).

| Field | Type | Shape | Meaning |
|---|---|---|---|
| `spec` | object (handoff spec) | **open** | A new spec (§4.2). Nested, so control fields are structurally outside it (TOOL-02). |
| `request_id` | string | open | Links to a user-opened request (OPEN-05); the handoff takes this id |
| `ignore_runbook` | boolean | open | Skip the runbook safety net (RUN-07) |
| `handoff_id` | string | **continue** | With `reply` |
| `reply` | string, 1–4000 | continue | The answer shown on the current step (TOOL-04) |
| `replacement_steps` | array of step, 1–50 | continue | Replaces the remaining steps (SPEC-04); validated like `steps`, including S3 against the handoff's `values`, S4, S5 |
| `resume` | string | **resume** | Re-attach to an existing handoff (TOOL-01) |

Shape inference: exactly one of `spec`, `reply` (with `handoff_id`), `resume` must be present; anything else is `SHAPE_AMBIGUOUS` with a fix text listing the three shapes. `continue` on a handoff that has no pending question or screenshot is `HANDOFF_NOT_WAITING` unless `replacement_steps` is present (then it is a correction round, VER-08, allowed in states `failed`, `active`, `deferred`). `resume` on a final handoff returns the final outcome with `already_delivered: true` (TOOL-07). `resume` on a handoff whose call is attached elsewhere detaches that call with `transferred_to_other_session` (TOOL-08).

Blocking: the call returns only on the events of TOOL-03, at the heartbeat (TOOL-06), on transfer, or on error. Annotations: `readOnlyHint: false`, `openWorldHint: false`, `title: "Hand a step off to the user"`.

Description (normative text, TOOL-09, SPEC-05, RUN-06, SRV-21, RESP-07, SRV-15):

> Hands a human step to the user through the local overlay and waits until the user finishes it or needs you. Use it when a step must be done by a person: credentials, OAuth apps, DNS, IAM, billing, confirmations, OS or application settings.
> Before writing a spec, call `handoff_runbooks(where, goal)` and start from a matching runbook if one exists.
> Three ways to call it. (1) Open: `{ "spec": {...}, "request_id"?: "hf_…", "ignore_runbook"?: true }`, where `spec` follows handoff-spec v1: `spec_version` 1, `goal`, `where`, optional `url` (http, https, ms-settings:, x-apple.systempreferences: only), `why_human`, `values` (name → string or list; every value the user must type or paste, taken from the project), optional `secrets` (variable name → destination file, for values the user copies from the dashboard; never their values), `steps` (objects `{ text, url?, values?, warning? }`, shown one at a time), optional `verify`, optional `lang`. (2) Continue: `{ "handoff_id", "reply", "replacement_steps"? }` to answer a question or a screenshot on the current step; `replacement_steps` replaces the remaining steps. (3) Resume: `{ "resume": "hf_…" }` to re-attach after an interrupted call, from any session; a resume of a finished handoff returns its outcome again with `already_delivered: true`.
> The call blocks while the user works and returns an outcome JSON with `status`, `final` and `instruction`. Follow `instruction`. `in_progress` means call again with `resume` at once. `deferred` means continue other work and call `resume` before you finish your turn. `parked` means the user will resume it; mention the id in your final summary. `awaiting_verification` means run the verification yourself and report it with `handoff_verify`.
> Rules. Never read the values listed in `secrets`; verify only their presence or their effect. Do not open a second handoff for the same goal; continue the same one. If a call fails because the server was disconnected, reconnect the server (for example `/mcp reconnect handoff`) and call `resume` with the same id. If the overlay app is not running the result is `status: text_mode`: present the spec in chat, guide the user step by step and collect the result in chat; in that mode nothing is logged and no verified state exists.
> Errors come back as `{ "error": { "code", "message", "problems": [{ "path", "problem", "fix" }] } }`; fix the named field and call again.

#### 4.7.2 `handoff_verify`

Input `{ handoff_id: string, verify: { ok: boolean or null, detail: string 1–4000 } }`; returns an outcome (§4.4). Annotations: `readOnlyHint: false`, `title: "Report the verification of a handoff"`.

Description (normative, TOOL-10, VER-02, VER-03, SPEC-05):

> Reports the result of the verification you performed after the user finished a handoff. `ok: true` if the check passed, `false` if it failed, `null` if you could not verify; `detail` says exactly what you ran and what you observed, or why you could not verify. Never invent a result: `null` with an honest reason is recorded as "not verified", which is better than a false pass. Never read values listed in the spec's `secrets`; check presence or effect only. On `false` you may correct the handoff by calling `handoff_to_user` with the same `handoff_id` and `replacement_steps` that start from the actual error. A late report for a handoff that timed out is accepted.

#### 4.7.3 `handoff_runbooks`

Input `{ where: string 1–300, goal: string 1–300, lang?: string }`; returns `{ runbooks: [ … ] }` with the fields of §4.3 `runbooks[]` (TOOL-15). Annotations: `readOnlyHint: true`, `title: "Search saved runbooks"`.

Description (normative, RUN-06, RUN-08, RUN-03a):

> Searches the user's saved runbooks in `~/.handoff/runbooks/` for a previous verified execution of the same kind of step: same `where` (normalised) and shared words in `goal`. Call it before writing a handoff spec. Each result carries `trust` (verified or confirmed_by_user), `last_verified_at` so you can weigh freshness, `last_run_failed_at`, the executed steps with their annotations, a `draft_spec` and `values_to_fill`. Fill the values from the current project and pass the completed spec to `handoff_to_user` with `ignore_runbook: true`.

#### 4.7.4 Capability-dependent behaviour

The server adapts the result to the session's capability row (§5.6): image blocks only when `images_in_results` is true (PREV-04 mirrors this in the UI); heartbeat timing from `tool_timeout_ms`; the instruction texts for `deferred` and `parked` mention the Stop hook only when `stop_hook` is true; otherwise they say the agent must remember by itself (PRIN-10).

#### 4.7.5 Error catalogue

Errors are MCP tool results with `isError: true` whose text is the JSON `{ "error": { "code", "message", "problems": [...] } }`.

| Code | When | Fix text |
|---|---|---|
| `SPEC_INVALID` | Schema or semantic rule S2–S6 fails | Per problem, as in §4.2 |
| `SPEC_VERSION_UNSUPPORTED` | S1 | "Update the server or lower spec_version." |
| `SHAPE_AMBIGUOUS` | Not exactly one of the three shapes | Lists the three shapes |
| `HANDOFF_NOT_FOUND` | Unknown `handoff_id` / `resume` id | "Check the id; ids look like hf_xxxxxxxxxx. The overlay lists open and orphan handoffs." |
| `HANDOFF_NOT_WAITING` | `reply` without a pending question and without `replacement_steps` | "The user has not asked anything; wait for the outcome or send replacement_steps." |
| `HANDOFF_FINAL` | `replacement_steps` on `abandoned`, `verified` or `confirmed_by_user` | "This handoff is closed; open a new one only for a different goal." |
| `NO_VERIFY_IN_SPEC` | `handoff_verify` on a spec without `verify` | "This handoff is confirmed by the user; include `verify` in the spec if you want to verify." |
| `APP_DISCONNECTED` | A non-blocking request (continue, resume, `handoff_verify`) finds the app unreachable; blocking calls instead wait and re-attach (§5.3) | "The overlay app is not reachable right now. Retry in a few seconds with the same handoff_id; if it stays unreachable, the app is not running." (SRV-21) |
| `CHANNEL_AUTH_FAILED` | Token mismatch | "The token file ~/.handoff/channel.token does not match the app. Reinstall or repair from the app settings." |
| `PROTOCOL_MISMATCH` | Channel versions differ | "Server and app versions do not match. Update the app (it bundles the matching server)." |
| `RUNBOOKS_UNREADABLE` | Folder unreadable | "Check permissions on ~/.handoff/runbooks." |
| `INTERNAL` | Unexpected | Message and a request to retry |

Errors never include spec values (they could be secret-treated); they cite paths and expected shapes only.

---

## 5. Server design (`handoff-mcp`)

### 5.1 Runtime and build (DD-05, DD-21)

TypeScript (strict) on Node.js 22 LTS with `@modelcontextprotocol/sdk` (stdio transport) and `ajv` (JSON Schema 2020-12). The source is bundled by esbuild into one CommonJS file and compiled into a **Node Single Executable Application** per platform (SRV-24): `darwin-arm64`, `darwin-x64`, `win32-x64`. No native addons: Unix sockets and Windows named pipes are covered by Node's `net` module; process enumeration uses one `ps` spawn on macOS and none on Windows (§5.8). The npm package (SRV-26) ships the same bundle with a `bin` entry for users who have Node.

*Alternatives rejected:* `bun build --compile` (a second runtime to trust and to notarize; named-pipe support on Windows not verified); Deno compile (the MCP SDK runs under npm compatibility, adding risk for no gain); `pkg` (archived). Verification of the SEA route, including signing with the hardened runtime and the JIT entitlements Node needs, is spike A-12 and is the first task of milestone M1 (§13).

### 5.2 Module map and the tool-call pipeline

```mermaid
flowchart TB
  MCP[mcp: tools, descriptions, input parsing] --> FMT[format: schema + semantic validation]
  MCP --> SEC[secrets: certain detector]
  MCP --> RB[runbooks: read, match, convert]
  MCP --> CALLS[calls: in-flight table, heartbeat]
  CALLS --> CH[channel: client, framing, reconnect]
  MCP --> TM[textmode: spec to text]
  MCP --> AD[adapters: capability row]
  CH --> PF[platform: paths, token, ancestors]
  HK[hook: stop subcommand] --> CH
  HK --> PF
```

Pipeline of a `handoff_to_user` call:

1. Parse input; infer shape (§4.7.1) or return `SHAPE_AMBIGUOUS`.
2. **Open:** version check (S1), schema validation, semantic rules S2–S6; on failure return `SPEC_INVALID` with all problems at once (SPEC-10). Run the certain detector (S7) and build `secret_treated`. Unless `ignore_runbook`, run the runbook safety net (§5.10); on a match return `runbook_match` without opening (RUN-07).
3. **Continue:** validate `replacement_steps` against the schema and against the handoff's `values` (the server asks the app for the value names in `handoff.continue`'s result if they are unknown locally; the app performs the same check and answers with an error if a key is unknown).
4. If the channel is not connected: **open** → `text_mode` (§5.9); **continue** / **resume** → `APP_DISCONNECTED` with the retry instruction (the handoff state is in the app, so nothing can be done without it).
5. Send `handoff.open` / `handoff.continue` / `handoff.resume` and register the call in the in-flight table (§5.7); await the outcome; render it (§4.3), attaching the image block when allowed (§4.7.4).

`handoff_verify` and `handoff_runbooks` are non-blocking: the first forwards to the app (`handoff.verify`) and returns the resulting outcome, the second reads files locally and never touches the channel (RUN-10).

### 5.3 Startup, registration and reconnection

```
serve():
  cfg  = read env: HANDOFF_AGENT, HANDOFF_TOOL_TIMEOUT_MS, MCP_TOOL_TIMEOUT, HANDOFF_HOME, CLAUDE_PROJECT_DIR
  proj = CLAUDE_PROJECT_DIR ?? process.cwd()                     # A-24
  start MCP stdio transport; register the three tools
  on initialize: remember clientInfo {name, version}, protocolVersion
  row = resolve_capability_row(cfg, clientInfo)                   # §5.6
  connect_channel(row) with backoff 1,2,5,10,30,30… s, forever   # SRV-20, FM-02
  on stdin EOF (agent exited): send session.bye best effort; exit 0
```

Registration happens in `hello` (§6.3) as soon as the socket connects, at session start rather than at the first tool call (SRV-20). If the app is not running, the server retries forever at 30 s intervals (**DD-23**); a local connect attempt costs microseconds, so a session that started before the app still appears in the overlay within 30 s of the app launching (FM-02). Until then, calls degrade to text mode (SRV-14).

Reconnection after the app disappears mid-session (crash, quit): the same backoff. Blocking calls that were attached to the lost connection stay pending in the server; when the connection returns the server re-issues `handoff.resume` for each of them, so the user sees no difference beyond a "server disconnected" banner in the app's tab while the app was down (FM-13). If the heartbeat deadline arrives first, the call returns `in_progress` as usual (§5.7).

### 5.4 Validation pipeline (SPEC-09..12)

Order: JSON shape → `spec_version` (S1, before the schema, so a future version gets "update" rather than a list of unknown fields) → schema (`ajv`, `allErrors: true`) → semantic rules S2–S6 → certain detector (S7, never an error). All problems are collected and returned together with `path`, `problem` and `fix` (§4.7.5); `ajv` errors are translated to the same shape (for example `additionalProperties` → "Unknown field `X` at `steps[1]`. Allowed fields: text, url, values, warning."). The validator is a pure function with no I/O and is the same code that `handoff-mcp validate <file>` runs, so agents' authors can test specs offline.

### 5.5 Certain detector at ingress (SPEC-13, DET-04)

Runs over every string in `values` (array items included) and over `goal`, `where`, `why_human`, `verify`, `steps[].text`, `steps[].warning`. Each match yields `{ location, kind }`; the matched text is never logged. The spec sent to the app is unchanged (the copy button must copy the true value, DET-04) but the `handoff.open` request carries `secret_treated`, and the app masks accordingly (§7.6). The outcome returns the same list (TOOL-13). Text mode masks those values in the rendered text (§5.9).

### 5.6 Agent identity and the capability table (DD-09, ADPT-02, ADPT-03)

Identity resolution, in order:

1. `HANDOFF_AGENT` environment variable, written by the installer into the `env` block of our MCP server entry (a documented mechanism, A-02). This is authoritative because the installer knows which agent's configuration it wrote.
2. `clientInfo.name` from the MCP `initialize` handshake matched against `match.client_names` of each row. The values Claude Code and the other agents send are not documented (A-08); they are recorded empirically per agent version by the canary suite (§11.5) and only ever used as a fallback for servers installed without our installer (npm users).
3. The `unknown` row.

Tool-timeout resolution for the heartbeat (TOOL-06, TOOL-06a): `HANDOFF_TOOL_TIMEOUT_MS` (written by the installer to mirror what it configured) → for `claude-code`, `MCP_TOOL_TIMEOUT` inherited from the settings `env` block (documented as applying to subprocesses, A-03) → the row's `tool_timeout_ms_default` → for `unknown`, heartbeat at 50 s. Heartbeat time = timeout − `HEARTBEAT_MARGIN_MS`, never below 50 s.

`capabilities.json` (excerpt; `status: planned` rows exist so the adapters enter the table in the committed order, ADPT-06):

```json
{
  "rows": [
    { "agent_id": "claude-code", "display_name": "Claude Code", "status": "supported", "support": "full",
      "match": { "env": "claude-code", "client_names": [] },
      "tool_timeout_ms_default": null, "per_server_timeout_field": "timeout",
      "images_in_results": true, "stop_hook": true, "subagent_stop_hook": true,
      "session_identity": "parent_pid", "user_request_delivery": ["clipboard_focus", "stop_hook"],
      "cancellation_notifications": true },
    { "agent_id": "codex", "display_name": "Codex CLI", "status": "planned", "support": "base",
      "match": { "env": "codex", "client_names": [] },
      "tool_timeout_ms_default": null, "images_in_results": null, "stop_hook": false, "subagent_stop_hook": false,
      "session_identity": "parent_pid", "user_request_delivery": ["clipboard_focus"] },
    { "agent_id": "cursor",   "status": "planned", "support": "base", "session_identity": "ancestor_chain:editor" },
    { "agent_id": "copilot",  "status": "planned", "support": "base", "session_identity": "ancestor_chain:editor" },
    { "agent_id": "opencode", "status": "planned", "support": "base", "session_identity": "parent_pid" },
    { "agent_id": "unknown",  "display_name": "MCP client", "status": "supported", "support": "base",
      "match": {}, "tool_timeout_ms_default": null, "heartbeat_after_ms": 50000,
      "images_in_results": false, "stop_hook": false, "subagent_stop_hook": false,
      "session_identity": "parent_pid", "user_request_delivery": ["clipboard_focus"] }
  ]
}
```

`null` in a planned row means "to be measured when the adapter ships"; the server treats `null` as the `unknown` value for that field. `tool_timeout_ms_default` for Claude Code is filled from the documentation at the time of the release (A-03 records the currently documented default); the installer normally makes it moot by writing the timeout. The resolved row travels to the app in `hello` so the app adapts its UI (hide "Send image" when `images_in_results` is false, PREV-04) without owning agent facts (ADPT-03).

Per-agent code exists only where the table cannot express a behaviour: session identity inside editors (Cursor, Copilot), where the process parent is the editor and the chain must be matched at the editor level (ADPT-02, ADPT-06).

### 5.7 Calls, blocking, heartbeat, resume and transfer (TOOL-03..08)

```
in_flight: Map<call_id, { handoff_id, started_at, deadline, resolve }>

wait_for_outcome(handoff_id, call_id, row):
  deadline = now + heartbeat_after(row)
  timer = at deadline:
      notify app handoff.detach_call { handoff_id, call_id, reason: "heartbeat" }      # DD-24
      resolve(outcome in_progress with instruction "call resume now")
  on channel notification handoff.event { call_id, outcome }: clear timer; resolve(outcome)
  on MCP notifications/cancelled for this request (A-09):
      notify app handoff.detach_call { reason: "cancelled" }; forget the call (nothing to resolve)
  on channel loss: keep the entry; re-attach with handoff.resume after reconnection (§5.3)
```

**DD-24:** the server tells the app when a call detaches and why, so the tab can show "waiting for the agent to resume" instead of pretending the agent is listening; the correctness of the system does not depend on this notification (the app queues events regardless, DD-12).

Resume: `handoff.resume { handoff_id, call_id, session_ref }` returns a snapshot `{ state, outcome? }`: if the handoff is final, `outcome` is the final outcome and the server returns it with `already_delivered: true` (TOOL-07); if an undelivered event is queued, `outcome` is that event and the server returns it immediately; otherwise the call attaches and waits. The app detaches any previously attached call of that handoff by sending it `handoff.event` with `transferred_to_other_session` (TOOL-08), and records `resumed_from` when the sessions differ. Resume works from any session of the installation because the app, not the session, owns the handoff (TOOL-08).

### 5.8 Channel client, session identity and the ancestor chain (SRV-17..20, DD-22)

**Endpoints (DD-26).** macOS: `~/.handoff/app.sock`; if the resulting path exceeds the `sun_path` limit (104 bytes), the server reads the pointer file `~/.handoff/app.sock.path` that the app writes in that case (FM-12). Windows: `\\.\pipe\handoff-<h>` where `h` is the first 16 hex digits of `sha256(lowercase(USERDOMAIN + "\" + USERNAME))`, computed identically by app and server from their environment; both run as the same user, and the suffix prevents two users' apps from colliding on the machine-global pipe namespace (FM-12).

**Token.** Read from `~/.handoff/channel.token` at every connection attempt (so a regenerated token is picked up without restart). On POSIX, a mode looser than `0600` produces a stderr warning; the connection proceeds because the property protected is the app's, not the server's (SRV-08).

**Identity payload** in `hello`: `pid`, `ppid`, `ancestors` (best effort), `cwd`, `project_dir`, `agent_id`, `client {name, version}`, `capability row`, `server_version`, `protocol_version`, `token`, `role: "server"`. The hook sends the same identity fields plus the hook input (`session_id`, `hook_event_name`, `stop_hook_active`, `agent_id`/`agent_type` for SubagentStop) with `role: "hook"`.

**Ancestor chain (DD-22).** The sender walks its chain where that is cheap: on macOS one `ps -axo pid=,ppid=,comm=` spawn (≈ 20 ms, capped at 200 ms in the hook) parsed into a table; on Windows nothing is spawned (PowerShell or WMI costs hundreds of milliseconds against the hook's budget) and only `pid`/`ppid` are sent. The app, which has a native cross-platform process table (`sysinfo` crate), **always** resolves the full chain of the connected peer itself while the peer is alive, and uses the union. SRV-17 and SRV-19 are satisfied in substance (the app always ends up with the full chain of both the server and the hook) with the cheapest reliable mechanism per platform. *Alternative rejected:* the TypeScript side always producing the full chain (on Windows it needs `Get-CimInstance` or WMI, too slow and being deprecated; a native addon would break SRV-24's single-executable simplicity).

**Project folder.** `CLAUDE_PROJECT_DIR` when set, else the process working directory; the app shows it in the tab (OPEN-02) and uses it as the fallback key (SRV-18).

### 5.9 Text mode (SRV-14..16, ARCH-04)

Rendered as Markdown text inside the `text_mode` outcome's `spec_text`:

```
# Handoff (text mode): <goal>
Where: <where>  [<url>]
Why a person: <why_human>
Values (from the project):
  - endpoint_url: https://api.myapp.example/webhooks/stripe
  - api_key: [treated as secret: stripe_secret_key]
Steps:
  1. <text>   (values: endpoint_url)   [url]   WARNING: <warning>
  2. …
After the steps, the user copies these values into project files (never paste them in chat):
  - STRIPE_WEBHOOK_SECRET → .env
Verification you must perform afterwards: <verify>
```

The outcome's `instruction` says the app is not running, that the agent must guide the user in chat and collect the result there, and that no log and no verified state exist in this mode (SRV-15). `handoff_id` is null: nothing can be resumed because no state exists anywhere. Remote agents get exactly this path with no extra code (SRV-16, ADPT-07).

### 5.10 Runbook reader, matcher and converter (RUN-06..08, RUN-10)

`read_all()`: list `~/.handoff/runbooks/*.json`, parse, validate against the runbook schema (own version and previous), skip invalid files with a stderr warning naming the file (never fail a tool call because of one bad file, FM-19), cache by path + mtime. `match(where, goal, lang)` applies §4.5.3; `to_draft(rb)` applies §4.5.4. A missing folder is an empty result. An unreadable folder is `RUNBOOKS_UNREADABLE` for `handoff_runbooks`, and silently skipped by the safety net so a runbook problem can never block opening a handoff.

### 5.11 Hook subcommand `handoff-mcp hook stop` (SRV-10..13, ADPT-08)

Input: the hook JSON on stdin (`session_id`, `transcript_path`, `cwd`, `hook_event_name`, `stop_hook_active`, and for SubagentStop `agent_id`, `agent_type`; documented, A-05). Budget: connect within 500 ms, total 1 800 ms, hard exit at 1 950 ms; the hooks configuration also sets `timeout: 5` seconds so a hung hook can never hold the agent (NFR-11).

```
hook_stop(input):
  if input.stop_hook_active: exit_neutral()                       # loop guard, SRV-12
  identity = { pid, ppid, ancestors (macOS only, ≤ 200 ms), cwd }
  conn = connect(socket, timeout 500 ms) or exit_neutral()       # app not running, SRV-11
  send hello { role: "hook", token, identity, hook: input }
  reply = await hook.stop result within remaining budget or exit_neutral()
  if reply.block: print JSON {"decision":"block","reason": reply.reason}; exit 0
  else exit_neutral()
exit_neutral(): print nothing; exit 0
```

The decision itself is taken by the app (§7.5), which owns the queues and the once-per-item counters; the hook is a transport. JSON output is used rather than exit code 2 because both are documented and JSON carries the reason unambiguously (A-05). The hook never blocks on uncertainty: any error, timeout or missing app is neutral (SRV-11, SRV-11a).

### 5.12 CLI, configuration and logging

| Command | Purpose |
|---|---|
| `handoff-mcp` | Serve MCP over stdio (default) |
| `handoff-mcp hook stop` | §5.11 |
| `handoff-mcp validate <spec.json>` | Offline validation with the same errors the tool returns |
| `handoff-mcp runbooks search --where … --goal …` | Offline matching, for users of the server alone |
| `handoff-mcp doctor` | Prints resolved agent id, capability row, token file status, socket reachability, runbook folder status; the first thing support asks for |

Environment: `HANDOFF_AGENT`, `HANDOFF_TOOL_TIMEOUT_MS` (installer-written), `HANDOFF_HOME` (overrides `~/.handoff`, tests only), `HANDOFF_MCP_LOG` (`error` default, `debug` for diagnostics). Variable names deliberately avoid the substrings `TOKEN`, `SECRET`, `PASSWORD`, `KEY`, `AUTH`, which Claude Code strips from the environment of servers declared in project scope (A-23). Logging goes to stderr only, never to files, and never includes spec values or texts: ids, codes, sizes and timings only.

---

## 6. Internal channel protocol (server ↔ app)

Declared internal and subject to change without notice (SRV-06); defined in `handoff-mcp/protocol/channel/` (§3.4) and versioned by a single integer (§3.6).

### 6.1 Transport and framing (DD-06)

Local stream socket (SRV-04): Unix domain socket on macOS, named pipe on Windows. Messages are **JSON-RPC 2.0** objects, one per line, UTF-8, newline-delimited (NDJSON). Maximum message size `CHANNEL_MAX_MESSAGE_BYTES` (16 MiB); a longer line closes the connection. Binary payloads (PNG) travel base64-encoded inside JSON. *Alternatives rejected:* length-prefixed binary frames (more efficient for images, but images are rare and ≤ 3 MB; NDJSON is debuggable with a text tool and trivially implemented on both sides); HTTP on localhost (a TCP port, reachable by any local process without the token handshake, and a port to fight over, SRV-04); a file queue (no push, no liveness).

### 6.2 Connection lifecycle and authentication

```mermaid
stateDiagram-v2
  [*] --> Connected: socket accepted
  Connected --> Authenticated: hello with valid token and protocol_version (within 2 s)
  Connected --> [*]: timeout, bad token, bad version (error then close)
  Authenticated --> Registered: role server, session_ref assigned
  Authenticated --> HookServed: role hook, hook.stop answered, then close
  Registered --> [*]: EOF, session.bye, app.shutdown, or protocol violation
  HookServed --> [*]
```

- The app listens; the socket file is created with mode `0600` and any stale file is removed at startup after a failed liveness connect (FM-12). On Windows the pipe is created with a DACL granting access to the current user only (A-16).
- The first message must be `hello` within 2 s; otherwise the app closes the connection. `token` is compared in constant time. Failures are answered with JSON-RPC errors (`-32001 auth_failed`, `-32002 protocol_unsupported`) and the connection closes; the app logs the attempt (no token material) and delays further accepts from a failing peer by 1 s, which is enough against accidental connections (SRV-08).
- The app does not verify the server beyond the token (SRV-09). Any server presenting the token and the right version is served, including an npm-installed one.

### 6.3 Methods and notifications

Requests carry `id`; notifications do not. `→` server to app, `←` app to server.

| Direction | Method | Kind | Params | Result |
|---|---|---|---|---|
| → | `hello` | request | `protocol_version, token, role ("server"/"hook"), server_version, identity {pid, ppid, ancestors[], cwd, project_dir}, agent_id, client {name, version}, capability_row`, and for hooks `hook {session_id, hook_event_name, stop_hook_active, agent_id?, agent_type?}` | `{ app_version, protocol_version, session_ref }` |
| → | `handoff.open` | request | `call_id, spec, secret_treated[], request_id?` | `{ handoff_id, resumed_from: null }`; then the app waits for an event |
| → | `handoff.continue` | request | `call_id, handoff_id, reply, replacement_steps?` | `{ ok: true }` or error `unknown_value_key {keys}`, `not_waiting`, `final` |
| → | `handoff.resume` | request | `call_id, handoff_id` | `{ state, outcome?, resumed_from? }` (§5.7) |
| → | `handoff.verify` | request | `handoff_id, verify {ok, detail}` | `{ outcome }` (final) or error `no_verify_in_spec`, `not_found` |
| → | `handoff.detach_call` | notification | `handoff_id, call_id, reason ("heartbeat"/"cancelled")` | — |
| → | `hook.stop` | request (role hook, after hello) | none beyond hello | `{ block: bool, reason?: string }` |
| → | `session.bye` | notification | none | — |
| ← | `handoff.event` | notification | `call_id, handoff_id, outcome` | — (delivers the outcome to the waiting call; the server resolves the call) |
| ← | `app.shutdown` | notification | `reason` | — (the server marks the channel down and starts the backoff) |
| ↔ | `ping` | request | none | `{}` (every 30 s of silence; two missed → connection considered dead) |

The app validates every incoming message against `channel.v1.schema.json`; violations close the connection. The server tolerates unknown fields in results (forward compatibility within a version is not promised, but this keeps a patch release from breaking a stale server needlessly).

### 6.4 Examples

```json
{"jsonrpc":"2.0","id":1,"method":"hello","params":{"protocol_version":1,"token":"3f9c…e1","role":"server","server_version":"1.0.3",
 "identity":{"pid":48211,"ppid":48190,"ancestors":[{"pid":48190,"name":"node"},{"pid":9120,"name":"zsh"},{"pid":9100,"name":"iTerm2"}],
 "cwd":"/Users/g/dev/shop","project_dir":"/Users/g/dev/shop"},
 "agent_id":"claude-code","client":{"name":"claude-code","version":"2.1.211"},
 "capability_row":{"agent_id":"claude-code","support":"full","images_in_results":true,"stop_hook":true,"tool_timeout_ms":1800000}}}
{"jsonrpc":"2.0","id":1,"result":{"app_version":"1.0.0","protocol_version":1,"session_ref":"ses_4m7q2t9x"}}
{"jsonrpc":"2.0","id":2,"method":"handoff.open","params":{"call_id":"call_2q7m8r1t","spec":{"spec_version":1,"goal":"…"},"secret_treated":[],"request_id":null}}
{"jsonrpc":"2.0","id":2,"result":{"handoff_id":"hf_7k3m9p2q4r","resumed_from":null}}
{"jsonrpc":"2.0","method":"handoff.event","params":{"call_id":"call_2q7m8r1t","handoff_id":"hf_7k3m9p2q4r","outcome":{"outcome_version":1,"status":"awaiting_verification","final":false,"…":"…"}}}
{"jsonrpc":"2.0","id":7,"method":"hello","params":{"protocol_version":1,"token":"3f9c…e1","role":"hook","identity":{"pid":50310,"ppid":48190,"ancestors":[],"cwd":"/Users/g/dev/shop"},
 "hook":{"session_id":"0b1e…","hook_event_name":"Stop","stop_hook_active":false}}}
{"jsonrpc":"2.0","id":8,"method":"hook.stop","params":{}}
{"jsonrpc":"2.0","id":8,"result":{"block":true,"reason":"Handoff hf_7k3m9p2q4r (Register the Stripe webhook…) is awaiting your verification report: perform its verify and call handoff_verify."}}
```

### 6.5 Versioning

`protocol_version` must be equal on both sides. On mismatch the app answers `protocol_unsupported` with its own version and closes; the server logs it, stops retrying for that version (it retries every 5 minutes in case the app is updated), and serves every call in text mode with the `PROTOCOL_MISMATCH` instruction "update the app; it bundles the matching server" (FM-11). The app never accepts older protocol versions: both binaries ship from one release (§3.5), so mismatches only arise from npm-installed servers or a stale app, and text mode is the correct degraded state for both.

### 6.6 Security and limits

Token authentication (SRV-07), OS permissions on the socket (SRV-04), message size cap, schema validation of every message, no dynamic evaluation of anything received, request timeout of 10 s for non-blocking requests (blocking waits are notifications and have no timeout), and a threat model published verbatim from SRV-08 in `protocol/channel/README.md` and `docs/`. Screenshot pixels cross the channel only from app to server inside an outcome, after preview (PREV-01), and the server keeps them in memory only for the duration of the tool result.

---

## 7. Overlay application design (`handoff-app`)

### 7.1 Stack (NFR-06, DD-27)

Tauri 2 with a Rust core and a web frontend. Rust: `tokio` (async runtime, Unix sockets and named pipes), `rusqlite` (bundled SQLite), `serde`/`serde_json`, `regex`, `sysinfo` (process table), `xcap` (monitor capture), `image` (resize, PNG), `objc2` + `objc2-vision` (macOS OCR), `windows` crate (`Windows.Media.Ocr`, named-pipe DACL), `ocrs` with `rten` (the bundled pure-Rust OCR engine) with its `.rten` models as bundled resources, Tauri plugins `global-shortcut`, `autostart`, `clipboard-manager`, `opener`, `notification`, `single-instance`, `window-state`. Frontend: TypeScript + Svelte + Vite, one Svelte app rendering the overlay, the request sheet, onboarding and settings as routes.

*Alternatives rejected:* Electron and two native apps (NFR-06 records why); React/Vue for the frontend (heavier bundles for a panel with a dozen views; the choice is minor and reversible); a separate Rust GUI toolkit (no reuse of web layout for i18n text flow).

### 7.2 Startup sequence and file locations (DD-28)

```
main():
  single-instance lock (a second launch focuses the running app and exits)
  crash::install_panic_hook()                                     # TEL-02
  license::check() -> Entitlement::Full                            # LIC-01, LIC-02 (isolated call site)
  open SQLite (app data dir), run migrations                       # §7.11
  ensure ~/.handoff/{runbooks/} exists; ensure channel.token exists (create 0600 if missing)   # INST-07
  channel::listen() on the socket / pipe                           # §7.3
  store::load_active_handoffs()                                    # tabs restored, connections marked "disconnected" until servers reconnect (FM-13)
  tray::show(); shortcut::register_or_ask()                        # WIN-05, OPEN-03
  install::scan_agents()  -> onboarding on first launch, discreet notice for new agents later   # INST-01, INST-05
  install::check_registered_paths() -> offer repair if the bundle moved (FM-23)
  updater::check_if_enabled()                                      # UPD-01, through net::egress only
  cleanup: delete vendor *.old.exe no longer locked (Windows, FM-24)
```

| Location | Content | Why here |
|---|---|---|
| `~/.handoff/` | `runbooks/`, `channel.token`, `app.sock` (+ `app.sock.path` when needed) | The contract folder shared with the server and readable by agents; survives uninstall (RUN-03) |
| App data dir (`~/Library/Application Support/Baton/`, `%APPDATA%\Baton\`) | `handoff.sqlite` (log, state, settings), `crashes/`, `update-cache.json` | App-private data; the server never reads it |

*Alternative rejected:* everything under `~/.handoff/` (mixes the contract folder that agents may browse with a database that must never be edited by hand).

### 7.3 Channel listener

One tokio task accepts connections; each connection gets a reader task (NDJSON → JSON-RPC, schema-validated), a writer task and a `Peer` record `{ conn_id, role, session_ref?, identity, capability_row, authenticated_at }`. `hello` handling: timeout 2 s, token compare, protocol check, ancestor-chain completion from the process table (DD-22), then `sessions::register` (role server) or `hook::decide` (role hook). Requests are dispatched to the store on a single actor task (all state mutations are serialised, which makes the state machine simple to reason about and test); results go back on the same connection; `handoff.event` notifications are addressed by `conn_id` recorded at attach time. A connection loss marks the session `disconnected`, detaches its calls (their handoffs keep running), and fires the UI banner (SRV-21, SRV-22).

### 7.4 Handoff store and state machine (DD-11, DD-12, NFR-12)

The store is the single owner of handoff state. Every transition is applied by the actor, persisted write-through into `handoffs.state_json` and `events`, and then published to the UI. State per handoff:

```
Handoff {
  id, created_at, session_ref (opener), request_text?, spec, secret_treated[],
  rounds: [ { no, steps[], started_at, ended_at?, verify? } ],
  cursor: { round, step_index },          # 1-based index into the current round's steps
  confirmed[], skipped[], notes[],        # per round
  deferral_count,                         # 0..2
  pending_question?: { kind: question|screenshot, step, at },   # set while an interrupting event awaits a reply
  undelivered: VecDeque<Outcome>,         # DD-12: events produced with no attached call
  attached_call?: { conn_id, call_id, session_ref },
  state: enum (§8.1), final_outcome?, delivered_at?, verifying_since?
}
```

Rules:

- **Interrupting actions** (Ask, Screenshot, Defer, Abandon, Done-on-last-step) build an outcome. If a call is attached, it is delivered to that call and the call detaches; otherwise it is pushed to `undelivered` (DD-12). `resume` pops the oldest undelivered outcome first (§5.7), so nothing is lost and order is preserved. *Alternative rejected:* merging queued events into one outcome (lossy; a question and a later screenshot answer different things).
- **Local actions** (Confirm, Note, Skip) mutate state and are reported inside the next outcome (RESP-03).
- **Replacement steps** (continue with `replacement_steps`) close the current round's remaining steps and open round `n+1` whose steps are the replacements; the counter restarts ("Correction · 1 of 2", VER-09).
- **Done on the last step** → `awaiting_verification` when the spec has `verify` (RESP-09, VER-04), with `verifying_since` set and a 30-minute timer (VER-06); otherwise `confirmed_by_user`, final.
- **Session disconnect while `awaiting_verification`** → `not_verified` at once (VER-06); a later report is accepted as late (DD-16).
- **Finalisation** writes `final_outcome`, and for `verified` / `confirmed_by_user` triggers the runbook writer (§7.12).
- **Orphan flag** is computed, not stored: `final_outcome` present, not delivered, older than 7 days (SRV-23).

### 7.5 Session registry, identity binding and hook decisions

`sessions::register(peer)` creates `Session { session_ref, agent_id, client, pid_chain, cwd, project_dir, connected: true, claude_session_id: None, first_seen, last_seen }`. When a hook connects, the app intersects the hook's completed ancestor chain with the PIDs of registered sessions (SRV-17); the unique match binds `session_id` to that session for the rest of its life. Fallback: `cwd` equality (SRV-18); if still ambiguous (two sessions in the same folder with no PID intersection) the hook is answered neutrally and the tab shows "which session is this?" with a picker the next time the user interacts (SRV-18, FM-22).

Hook decision (`hook::decide`, SRV-12, SRV-13, RESP-06, VER-07; **DD-25**):

```
decide(hook_peer):
  if hook.stop_hook_active: return neutral
  session = bind(hook_peer) or return neutral
  items = []
  for h in handoffs where h.session_ref == session or h.attached_call.session_ref == session:
      if h.state in {deferred, parked} and no attached call:            items.push(Deferred(h))
      if h.state == awaiting_verification and no attached call:          items.push(Unreported(h))
      if h.state == active and h.undelivered non-empty and no call:      items.push(PendingEvent(h))   # extension, see below
  for r in user_requests where r.session in {session, unassigned}:      items.push(Request(r))
  items = items.filter(i => not blocked_before(session, i.key))         # once per item per session
  if items.empty: return neutral
  record blocked(session, item.key) for each; return block(reason = render(items, max 3, English))
```

The third condition (an active handoff with an undelivered question or screenshot and no listening call) is a design extension of SRV-12: it is the same safety net applied to the case where a heartbeat returned and the agent forgot to resume; the once-per-handoff-per-session cap is unchanged. Requests delivered by the hook are marked `delivered_via: stop_hook` and assigned to the session (OPEN-06, OPEN-04a).

### 7.6 UI composition (DD-10)

One overlay window, always on top (WIN-01), fixed width 360 px, content-driven height, draggable, position remembered per monitor (WIN-02). Views inside it:

| View | Content | Requirements |
|---|---|---|
| Tab strip | One tab per handoff: agent name, project folder name, badge for unseen events; orphan and parked handoffs in a collapsible "waiting" group | OPEN-02, MULTI-01..03, RESP-07, SRV-23 |
| Step view | "Step 2 of 4" (or "Correction · 1 of 2"), `text`, `warning` banner, value chips (copy whole / per item, GUIDE-02), Open button for the step or spec `url` (allowed schemes only; other URLs are plain text), auto-linked `https` in text, `secrets` list with **Open file** (SEC-02), buttons Done / Ask / Screenshot / Note / Skip / Defer / Abandon; a **Show** toggle on masked values (DET-04) | GUIDE-01..06, RESP-01..02, RESP-08, SEC-01 |
| Waiting-for-spec | Request text, session, "the agent has not answered yet", Abandon; a "Copy request again" button | OPEN-04, OPEN-05 |
| Question pending | The question or screenshot summary, "waiting for the agent"; the reply appears here when it arrives, on the step it referred to (TOOL-04) | RESP-04 |
| Verifying | The `verify` text, "the agent should now check:"; then state + detail labelled **"declared by agent"** | VER-04, VER-05 |
| History | Previous rounds collapsed, notes, questions, replies | VER-09 |
| Collapsed bar | Current step text (one line), Done / Ask / Screenshot; appears on window blur, re-expands on click | WIN-03 |
| Request sheet | Session selector (pre-selected when one) + "What are you about to do?"; Enter sends, Esc cancels; "no active session" notice when none | OPEN-03, OPEN-04, OPEN-04a |
| Preview | Image with boxes (locked red = certain, dashed amber = suspected with "may contain a secret"), unlock / add box / crop, editable text pane, **Send image** / **Send text** (image hidden when unsupported), optional comment field | PREV-01..05, DET-01 |
| Settings | General (language, autostart, shortcut), Agents (found, registered, repair, uninstall, scope), Network (NET-01), Log (LOG-04), Runbooks (open folder, delete, pending update proposals), Updates (check on/off, check now) | APP-01, APP-02, INST-05, INST-06, UPD-01, LOG-04, NET-01 |
| Onboarding | Welcome → agents & consent (INST-01, INST-02) with **Show** diffs → autostart checkbox (APP-01) → macOS screen recording explanation (CAP-04) → shortcut check (OPEN-03) → done | INST-01..03, APP-01, CAP-04 |

The request sheet and the settings are modes/routes of the same window rather than separate windows (DD-10): the "never two windows" rule (MULTI-04) then holds structurally, focus and always-on-top are managed once, and the tab strip stays visible behind the sheet. Settings that need more room open the window in a wider layout temporarily. *Alternative rejected:* a separate request window (a second always-on-top window to position and focus; OPEN-04 calls it a window only descriptively). Region-selection overlays (§7.8) are capture tools, not handoff windows (**DD-29**).

Focus rules: the first handoff opened while none is active brings the window to the front (MULTI-03); later ones add a badge; the window collapses on blur (WIN-03); Close hides to tray, Quit only from the tray menu (WIN-04); the tray badge reflects active handoffs (WIN-05).

Masked values (DET-04): a secret-treated value renders as `••••••` with **Show** (local reveal for 10 s) and **Copy** (copies the true value); log and runbook get the placeholder; the outcome already told the agent (§5.5).

### 7.7 User-opened requests (OPEN-03..08)

```
on shortcut:
  show window in request mode; selector = registered sessions (pre-select if one; notice if none)
  on Enter(text, session?):
     id = new hf_ id; tab "waiting for spec"
     queue UserRequest { id, text, session (may be unassigned), created_at }
     clipboard = render_request_text(user_language, id, text)              # OPEN-05
     focus_terminal(session.pid_chain) best effort                           # OPEN-05
     if not focused: notify "Request copied: paste it into session <agent · project>"
```

Clipboard text (English): `[Handoff hf_7k3m9p2q4r] The user opened a request: "I'm about to create the API key on Stripe". Produce the spec and call handoff_to_user with request_id=hf_7k3m9p2q4r.` (Italian when the UI language is Italian; the id and the tool name are invariant.)

Linking: a `handoff.open` with `request_id` adopts that id and the tab moves from waiting to active; without `request_id`, the first new handoff of that session links to the oldest open request of the same session (OPEN-08), and the tab shows "linked to request: …" with a **Change** control so a mismatch (FM-20) costs one click. Requests with no session are delivered to the first session that registers: the app puts the text on the clipboard again with a notification, and the Stop hook delivers it at that session's next end of turn (OPEN-04a, OPEN-06).

Terminal focus (best effort, A-18): macOS walks the session's ancestor chain to the first process that owns windows (`NSRunningApplication`) and activates it; Windows enumerates top-level windows whose owning PID is in the chain (Windows Terminal hosts the shell as a child, so the chain contains it) and calls `SetForegroundWindow`. Failure is silent apart from the notification (OPEN-05).

### 7.8 Capture (CAP-01..06, PRIN-04)

`capture::Backend` trait with `list_monitors()`, `capture_monitor(id)`, `capture_all()`; implementations `XcapBackend` (production) and `FakeCapture` (e2e builds only, returns fixture images). Flow: the Screenshot button shows two choices, **Full screen** and **Select region**, the last choice highlighted but never fired (CAP-01). The overlay hides itself (CAP-03). Full screen captures the monitor under the cursor (CAP-02). Region selection creates one transparent click-through-off overlay per monitor (DD-29), the user drags a rectangle, the overlays close and the region is cropped from the composite of all monitors (CAP-02). Nothing runs between captures (PRIN-04, NFR-03). macOS screen-recording permission is requested in onboarding with an explanation and a deep link to the settings pane (CAP-04); if missing at capture time the preview shows the explanation and a button to the pane, never a mid-handoff prompt (FM-17).

### 7.9 OCR (OCR-01..05, DD-30)

```rust
trait OcrEngine { fn name(&self) -> &str; fn available(&self, lang_hint: Option<&str>) -> bool;
                  fn recognize(&self, img: &RgbaImage, lang_hint: Option<&str>) -> Result<Vec<TextBlock>>; }
struct TextBlock { text: String, bbox: Rect, confidence: f32 }
```

Engines: `VisionOcr` (macOS, `VNRecognizeTextRequest`, accurate level, `recognitionLanguages` from `lang`), `WindowsOcr` (`Windows.Media.Ocr.OcrEngine::TryCreateFromLanguage(lang)` then `TryCreateFromUserProfileLanguages()`), `OcrsOcr` (the bundled pure-Rust `ocrs` engine, English models only, OCR-03). Selection: OS engine if `available()`, else the bundled engine; an engine that errors or exceeds `OCR_ENGINE_TIMEOUT_MS` falls back to the next; all run on the full-resolution original (CAP-05) in a blocking thread while the preview already shows the image with an "analyzing" state and disabled send buttons (OCR-04). The engine used is recorded in the outcome and the log. Text is never sent off the machine by OCR: everything is local and offline (NFR-02).

### 7.10 Detection, redaction and preview (DET-01..04, PREV-01..05, CAP-06)

Inputs: OCR blocks, the spec's non-secret values (exemption list, DET-03), the vendored certain patterns (§4.6). Certain matches (patterns over the concatenated text of each block and over each block alone) produce **locked boxes**; suspected matches produce **flagged boxes**: tokens ≥ 20 characters with Shannon entropy > 3.5 bits/char and mixed character classes, hex strings ≥ 32, base64-looking strings ≥ 24, and any token adjacent (same line or the line below) to a label matching `key|secret|token|password|passwd|pwd|bearer|api` — unless the token equals an exempt spec value. Typed text (Ask, comments, edited OCR text) runs through the same detectors before send.

Preview (PREV-01..04): the user can unlock a box, add one, crop, edit the text pane; two send buttons, **Send image** and **Send text**, no default; **Send image** hidden when the capability row has `images_in_results: false` (PREV-04). Redaction burn-in (CAP-06): boxes are computed in original coordinates, the image is downscaled so the long side is 1600 px (CAP-05), boxes are rescaled and expanded by 2 px, filled with solid black **after** the resize, and the PNG is encoded. Text mode sends the edited text with certain matches replaced by `[REDACTED:<kind>]` and unlocked boxes' text restored. What left is logged (§7.11); pixels never are.

### 7.11 Log (LOG-01..05, NET-01)

One SQLite database, WAL mode, foreign keys on (**DD-31**: state and log share the database, so a transition and its log row commit atomically).

| Table | Columns (abridged) | Requirement |
|---|---|---|
| `handoffs` | `id PK, created_at, closed_at, agent_id, client_name, project_dir, request_text, state, final_state, spec_json (secret-treated values replaced by placeholders), state_json, delivered_at, resumed_from_json, lang` | LOG-02 |
| `rounds` | `handoff_id, no, steps_json, started_at, ended_at, verify_ok, verify_detail, verify_reported_at, verify_late` | LOG-02, VER-10 |
| `events` | `id, handoff_id, round, at, kind (confirm/note/skip/ask/screenshot/defer/abandon/reply/replace/resume/attach/detach/state), step_index, payload_json` | LOG-02 |
| `sends` | `id, handoff_id, at, kind (question/screenshot_text/screenshot_image/defer/abandon), text_as_sent, image_sha256, image_w, image_h, redaction_boxes_json, ocr_engine, patterns_version` | LOG-03 |
| `sessions` | `session_ref, agent_id, client_name, client_version, pid_chain_json, cwd, project_dir, claude_session_id, first_seen, last_seen` | SRV-17..20 |
| `user_requests` | `id, session_ref NULL, text, created_at, delivered_via, linked_handoff_id` | OPEN-05..08 |
| `hook_blocks` | `session_ref, item_key, at` | SRV-12 |
| `network_events` | `at, domain, bytes_sent, purpose` | NET-01 |
| `settings` | `key, value` | — |

Deletion of one handoff cascades to its rounds, events and sends; "delete everything" truncates all tables except `settings` (LOG-04). Export writes the same tables as one JSON document to a user-chosen path (LOG-04). No retention job exists (LOG-05). Pixels are never stored: `image_sha256`, dimensions and boxes only (LOG-03).

### 7.12 Runbook writer (RUN-01, RUN-02, RUN-04, RUN-05, RUN-09, RUN-10, DD-18)

Triggered by the transition to `verified` or `confirmed_by_user`. Steps: build the executed sequence (§4.5.1); substitute placeholders (§4.5.2); determine origin by applying the matching rule (§4.5.3) to existing runbooks (**DD-18**: origin is inferred, not carried in the spec, which SPEC-06 forbids, nor in a new control field); then:

| Situation | Action |
|---|---|
| No matching runbook | Create a new file, `trust` from the final state, `runs: 1` |
| Match, executed sequence identical | Refresh `last_verified_at`, `runs += 1`, raise `trust` to `verified` if applicable |
| Match, sequence differs, and this handoff had ≥ 2 rounds (a correction happened) | Show the proposal "Update runbook `<name>` with the corrected sequence?" with one click (RUN-09); until decided, the new sequence is kept in `state_json` |
| Match, sequence differs, no correction | Create a new file (a different way to reach the same goal) |
| Handoff `failed` and no correction followed, match exists | Set `last_run_failed_at` (RUN-09); never delete |

Files are written atomically; the certain detector runs on the final text (§4.5.2). Deletion from the Runbooks settings page moves the file to the OS trash.

### 7.13 Network egress and update check (NET-01, NET-02, UPD-01, UPD-02, PRIN-05, DD-32)

`net::egress::get(url) -> Result<Response>` is the **only** function in the codebase that opens a network connection. It records `{ at, domain, bytes_sent, purpose }` in `network_events` before sending. Enforcement: `reqwest` is a dependency only of the `net` module; `cargo-deny` and a `clippy` `disallowed_types` rule fail the build if `reqwest`, `hyper`, `std::net::TcpStream` or `tokio::net::TcpStream` appear outside `net/egress.rs`; the frontend's CSP is `default-src 'self'` with no `connect-src`, so the webview cannot reach the network either. The Network settings page lists `network_events` (NET-01). The documentation describes the firewall test (NET-02): block all outbound traffic for the app with the system firewall or Little Snitch, run a full handoff with screenshot, observe that nothing is blocked; then enable the update check and observe exactly one connection to the update domain.

Update check (UPD-01): once per launch, if enabled: `GET https://<fixed update domain>/v1/check?app=<version>&os=<darwin|win32>` — only the version number and the OS name, no identifiers, no cookies; response `{ latest, notes, download_url }`; cached 24 h. If newer: a notice with the notes, **Download** (opens the page in the browser), **Remind me later** (UPD-02). No automatic download or install; the agent configuration is untouched because the path is fixed (SRV-25).

### 7.14 Crash files and licence entry point (TEL-01, TEL-02, LIC-01, LIC-02)

A panic hook writes `crashes/<timestamp>.txt` (version, OS, backtrace, last 50 log lines with ids only) to the app data dir; the next launch shows "the app crashed last time; open the folder to send the report by hand". Nothing is uploaded, ever; no telemetry code exists (TEL-01). `license::check()` is called once in `main` and returns `Entitlement::Full`; every feature gate in v1 is a no-op reading that value, so a future local check changes one module (LIC-02).

### 7.15 Installation adapters (INST-01..08, SRV-07, SRV-25)

```rust
trait InstallAdapter {
  fn agent_id(&self) -> &str;
  fn detect(&self) -> Detection;                    // found / not found, config paths, agent version if cheap
  fn plan(&self, scope: Scope) -> Vec<Modification>; // exact file + exact JSON/text change, with a rendered diff
  fn apply(&self, plan: &[Modification]) -> Result<()>;   // backup, write, verify
  fn verify(&self) -> Registration;                 // registered / partial / path mismatch
  fn uninstall(&self) -> Result<()>;                // remove only our lines (INST-04)
}
```

Claude Code adapter (`claude_code.rs`), user scope by default (INST-06), three modifications shown on the consent screen (INST-02, as amended on 2026-09-08 by T-026):

| # | File | Change | Notes |
|---|---|---|---|
| 1 | `~/.claude.json` → `mcpServers.handoff` | `{ "type": "stdio", "command": "<fixed path>", "args": [], "env": { "HANDOFF_AGENT": "claude-code", "HANDOFF_TOOL_TIMEOUT_MS": "1800000" }, "timeout": 1800000 }` | The per-server `timeout` field is always written (A-04 verified 2026-09-08; T-026, Option B); it affects only our server and is the only timeout the installer configures. `HANDOFF_TOOL_TIMEOUT_MS` mirrors it. `env` names avoid the stripped substrings (A-23). |
| 2 | `~/.claude/settings.json` → `hooks.Stop[]` | `{ "matcher": "", "hooks": [ { "type": "command", "command": "<fixed path> hook stop", "timeout": 5 } ] }` appended | Existing entries untouched (INST-04) |
| 2 (same line) | `~/.claude/settings.json` → `hooks.SubagentStop[]` | Same command | Presented as one line "two hooks (Stop and SubagentStop), same command" with **Show** (INST-02, ADPT-08) |
| — | `~/.claude/settings.json` → `env.MCP_TOOL_TIMEOUT` | *not written* | **Dropped on 2026-09-08 (T-026, OI-02, Option B).** The global variable applies to every MCP server of Claude Code and the default is already longer than 30 minutes, so the installer neither writes it nor touches an existing value, on install or on uninstall (INST-03). The row stays so that the former fourth consent line is not reintroduced. |

Project scope (INST-06) writes `.mcp.json` and `.claude/settings.json` in the chosen folder instead. `apply` writes a backup `<file>.handoff-backup-<timestamp>`, edits the JSON preserving unrelated keys, and re-reads to verify. Our entries are recognisable by the fixed path in `command`, so `uninstall` removes exactly them; `MCP_TOOL_TIMEOUT` is never ours to restore (INST-03, INST-04). `detect` runs at every launch (INST-05); a newly found agent produces one discreet notice. The token file is created by the first `apply` (INST-07) or at startup if missing. Adapters for Codex, Cursor, Copilot and OpenCode implement the same trait with their own files and the same `HANDOFF_AGENT` value, keyed by the agent id shared with the server's table (INST-08).

### 7.16 Window behaviour, tray, shortcut, autostart, language

- Always on top; fixed width; height from content; draggable by the header; position stored per monitor identifier; collapse on blur to the bar with Done / Ask / Screenshot (WIN-01..03).
- Tray icon always present; badge only with active handoffs; menu: Show, New request, Settings, Quit (WIN-04, WIN-05).
- Global shortcut default `⌃⌥H` / `Ctrl+Alt+H`; if registration fails at startup the app asks once for another combination and never steals a taken one (OPEN-03, FM-18).
- Autostart on by default, proposed in onboarding with the pre-checked box and the sentence from APP-01; toggle in settings.
- UI language from the system if `en` or `it`, else English; changeable (APP-02). Step texts are shown as written (GUIDE-06).
- At rest: tray icon, listening socket, no timers except the 30-minute verifying timers of open handoffs (WIN-06, NFR-14).

---

## 8. State machines

### 8.1 Handoff lifecycle

```mermaid
stateDiagram-v2
  [*] --> awaiting_spec: user request (shortcut)
  [*] --> active: handoff.open (agent)
  awaiting_spec --> active: handoff.open with request_id (or linked)
  awaiting_spec --> abandoned: user cancels
  active --> active: confirm / note / skip / navigate
  active --> active: Ask or Screenshot → outcome delivered or queued, reply arrives (continue)
  active --> deferred: Defer (count = 1)
  deferred --> active: agent resumes
  deferred --> parked: Defer again (count = 2)
  parked --> active: user resumes from the overlay (agent re-engaged via request queue / hook)
  active --> active: continue with replacement_steps → new round
  active --> awaiting_verification: Done on last step, verify present
  active --> confirmed_by_user: Done on last step, no verify
  awaiting_verification --> verified: handoff_verify ok=true
  awaiting_verification --> failed: handoff_verify ok=false
  awaiting_verification --> not_verified: ok=null, or 30 min, or session disconnect
  not_verified --> verified: late report ok=true (within 7 days)
  not_verified --> failed: late report ok=false
  failed --> active: continue with replacement_steps (round n+1)
  active --> abandoned: Abandon
  deferred --> abandoned: Abandon
  parked --> abandoned: Abandon
  awaiting_verification --> abandoned: Abandon
  verified --> [*]
  confirmed_by_user --> [*]
  abandoned --> [*]
```

Notes: `failed`, `not_verified`, `verified`, `confirmed_by_user` and `abandoned` are the final states of VER-01; `failed` (and `not_verified`) can be left only by an agent action on the same id (VER-08, DD-16). Connection flags (attached call, session connected, server disconnected) are orthogonal to this machine and drive banners only (SRV-21, SRV-22). Orphan is a computed flag on final states (SRV-23).

### 8.2 Blocking call (server side)

```mermaid
stateDiagram-v2
  [*] --> Validating
  Validating --> Error: shape / schema / semantic failure
  Validating --> RunbookMatch: new spec, matches, no ignore_runbook
  Validating --> TextMode: channel down (open)
  Validating --> Attached: open / continue / resume accepted by the app
  Attached --> Returned: handoff.event (question, screenshot, deferred, parked, abandoned, awaiting_verification, confirmed_by_user, transferred)
  Attached --> Returned: heartbeat deadline → in_progress (detach_call sent)
  Attached --> Forgotten: MCP cancelled → detach_call sent
  Attached --> Reattaching: channel lost
  Reattaching --> Attached: channel back, handoff.resume
  Reattaching --> Returned: heartbeat deadline → in_progress
  Returned --> [*]
  Error --> [*]
  RunbookMatch --> [*]
  TextMode --> [*]
  Forgotten --> [*]
```

### 8.3 Session connection (app side)

```mermaid
stateDiagram-v2
  [*] --> Connected: hello accepted (server role)
  Connected --> Bound: hook chain intersects PIDs → session_id bound
  Connected --> Disconnected: EOF / bye / ping failure
  Bound --> Disconnected: EOF / bye / ping failure
  Disconnected --> Connected: new hello with the same agent PID (server restarted by a new session in the same process is impossible, a new session is a new Session, the old one is kept for history)
  Disconnected --> [*]: after 7 days without handoffs
```

A `Disconnected` session's handoffs stay in their own state; their tabs show "server disconnected" (SRV-21) and any session may `resume` them (TOOL-08).

### 8.4 Tab UI states

| UI state | Handoff state | Banner / label |
|---|---|---|
| Waiting for spec | `awaiting_spec` | "Waiting for the agent's spec" |
| Guiding | `active`, call attached | none |
| Guiding, agent away | `active`, no call attached, nothing queued | "The agent will pick up on its next resume" (after `detach_call`) |
| Question sent | `active`, `pending_question` | "Sent to the agent, waiting for the reply" |
| Deferred / Parked | `deferred` / `parked` | "Deferred; the agent will come back" / "Parked; resume when you want" |
| Verifying | `awaiting_verification` | "The agent should now check: <verify>" |
| Final | any final | State label, detail "declared by agent" when a report exists; "orphan" after 7 days undelivered |
| Detached | any non-final, session disconnected | "Session detached; the outcome will be delivered on the next resume" (SRV-22) |

---

## 9. Flows

Participants: **U** user, **A** agent (Claude Code), **S** server, **P** app, **H** hook subprocess. Each flow names the requirements it realises; the failure branches are in §10.

### F-01 Session start and registration (SRV-20, SRV-05, FM-02)

```mermaid
sequenceDiagram
  participant U as User
  participant A as Agent
  participant S as Server
  participant P as App
  U->>A: start a session in a project folder
  A->>S: spawn stdio server, initialize(clientInfo)
  S->>S: resolve capability row (HANDOFF_AGENT, clientInfo, unknown)
  S->>P: connect socket, hello{token, identity, agent_id, row}
  alt app running
    P->>P: verify token and protocol, complete ancestor chain, register session
    P-->>S: welcome{session_ref}
    P->>P: if unassigned user requests exist: clipboard + notification for this session
  else app not running
    S->>S: retry with backoff 1, 2, 5, 10, 30 s… forever, calls degrade to text mode meanwhile
  end
```

### F-02 Agent-opened handoff, happy path to verified (OPEN-01, GUIDE-01, RESP-03, RESP-09, VER-04, VER-05, RUN-01)

```mermaid
sequenceDiagram
  participant U as User
  participant A as Agent
  participant S as Server
  participant P as App
  A->>S: handoff_runbooks(where, goal) → none
  A->>S: handoff_to_user{spec}
  S->>S: validate, certain scan, runbook safety net (none)
  S->>P: handoff.open{call_id, spec, secret_treated}
  P->>P: create handoff hf_x, tab, bring window to front if it is the first active
  P-->>S: {handoff_id}
  loop steps 1..n-1
    U->>P: Confirm / Note / Skip (local, no call return)
  end
  U->>P: Done on step n
  P->>P: awaiting_verification, show verify text, start 30-min timer
  P->>S: handoff.event{awaiting_verification}
  S-->>A: outcome awaiting_verification (instruction: verify, then handoff_verify)
  A->>A: performs the verification with its own tools
  A->>S: handoff_verify{hf_x, ok:true, detail}
  S->>P: handoff.verify
  P->>P: verified, log, runbook written, tab shows detail "declared by agent"
  P-->>S: {outcome verified}
  S-->>A: outcome verified (final)
```

### F-03 Runbook safety net (RUN-06, RUN-07, RUN-07a)

```mermaid
sequenceDiagram
  participant A as Agent
  participant S as Server
  participant P as App
  A->>S: handoff_to_user{spec}  (handoff_runbooks was skipped)
  S->>S: validate, read ~/.handoff/runbooks, match where+goal
  S-->>A: runbook_match{runbooks[{draft_spec, values_to_fill, last_verified_at, annotations}]}
  A->>A: fill values from the project
  A->>S: handoff_to_user{spec', ignore_runbook:true}
  S->>P: handoff.open
```

### F-04 Ask and screenshot round trip (RESP-04, CTX-01, TOOL-04, PREV-01)

```mermaid
sequenceDiagram
  participant U as User
  participant A as Agent
  participant S as Server
  participant P as App
  U->>P: Ask "the field is not there" (or Screenshot → Full/Region → capture → OCR → preview → Send text/image)
  P->>P: pending_question on step i, log the send (text as sent, image hash)
  alt call attached
    P->>S: handoff.event{question|screenshot, context}
    S-->>A: outcome (+ image block when allowed)
  else no call attached
    P->>P: queue outcome (delivered at next resume)
  end
  A->>S: handoff_to_user{handoff_id, reply, replacement_steps?}
  S->>P: handoff.continue
  P->>P: reply shown on step i, replacement → new round "Correction · 1 of m"
  P-->>S: ok
  S->>S: call attached again, waiting
```

### F-05 Defer, resume, second deferral, resume from the overlay (RESP-05..07, TOOL-14, SRV-12)

```mermaid
sequenceDiagram
  participant U as User
  participant A as Agent
  participant S as Server
  participant P as App
  U->>P: Defer (count 1)
  P->>S: handoff.event{deferred}
  S-->>A: outcome deferred (instruction: resume before concluding)
  A->>A: other work
  A->>S: handoff_to_user{resume: hf_x}
  S->>P: handoff.resume → attached
  U->>P: Defer again (count 2)
  P->>S: handoff.event{parked}
  S-->>A: outcome parked (instruction: cite hf_x in the summary, do not resume)
  Note over P: tab badge, handoff in the "waiting" group
  U->>P: Resume from the overlay → active, works through the steps, Done
  P->>P: awaiting_verification with no attached call → outcome queued, resume request queued for the session
  Note over A,P: next end of turn of that session: hook blocks once with "resume hf_x", the agent resumes and receives awaiting_verification
```

### F-06 Heartbeat and resume (TOOL-05, TOOL-06, TOOL-06a, TOOL-07)

```mermaid
sequenceDiagram
  participant A as Agent
  participant S as Server
  participant P as App
  A->>S: handoff_to_user{spec}
  S->>P: handoff.open
  Note over S: deadline = tool timeout − 60 s (50 s for unknown clients)
  S->>P: handoff.detach_call{heartbeat}
  S-->>A: in_progress (instruction: call resume now)
  A->>S: handoff_to_user{resume: hf_x}
  S->>P: handoff.resume{new call_id}
  P-->>S: {state: active}  (or a queued outcome, or a final outcome with already_delivered)
  Note over S: attached again, new deadline
```

### F-07 User-opened request (OPEN-03..06, OPEN-08, OPEN-09)

```mermaid
sequenceDiagram
  participant U as User
  participant A as Agent
  participant S as Server
  participant P as App
  participant H as Hook
  U->>P: global shortcut
  P->>U: request sheet (session selector, or "no active session")
  U->>P: "I'm about to create the API key on Stripe" + Enter
  P->>P: id hf_x, tab "waiting for spec", queue request, clipboard text with hf_x
  P->>A: bring terminal to front (best effort)
  U->>A: paste + Enter
  A->>S: handoff_to_user{spec, request_id: hf_x}
  S->>P: handoff.open{request_id}
  P->>P: tab hf_x becomes active (same id)
  opt agent was busy or the user did not paste
    A->>H: Stop hook at end of turn
    H->>P: hello(role hook) + hook.stop
    P-->>H: block, reason = the request text with hf_x
    H-->>A: {"decision":"block","reason":…}
    A->>S: handoff_to_user{spec, request_id: hf_x}
  end
```

### F-08 Failed verification and correction round (VER-08..10, RUN-09)

```mermaid
sequenceDiagram
  participant U as User
  participant A as Agent
  participant S as Server
  participant P as App
  A->>S: handoff_verify{ok:false, detail:"test event rejected: invalid signature"}
  S->>P: handoff.verify
  P->>P: failed (round 1 closed with error annotation)
  P-->>S: {outcome failed}
  S-->>A: failed (instruction: replacement_steps from the actual error)
  A->>S: handoff_to_user{handoff_id, reply, replacement_steps:[…]}
  S->>P: handoff.continue → round 2 "Correction · 1 of 2", history collapsed
  U->>P: steps… Done
  P->>S: handoff.event{awaiting_verification}
  S-->>A: outcome
  A->>S: handoff_verify{ok:true}
  P->>P: verified, matching runbook found with a different sequence → proposal "update runbook?"
```

### F-09 Text mode (SRV-14..16, ARCH-04)

```mermaid
sequenceDiagram
  participant U as User
  participant A as Agent
  participant S as Server
  A->>S: handoff_to_user{spec}
  S->>S: validate, channel down
  S-->>A: text_mode{spec_text, instruction, handoff_id:null, app_reachable:false}
  A->>U: presents the steps in chat, collects answers, composes the outcome itself
  Note over S: nothing logged, no verified state (documented)
```

### F-10 Stop hook evaluation (SRV-10..13, RESP-06, VER-07, OPEN-06, ADPT-08)

```mermaid
sequenceDiagram
  participant A as Agent
  participant H as Hook
  participant P as App
  A->>H: spawn "handoff-mcp hook stop", JSON on stdin
  H->>H: stop_hook_active true → exit 0, no output
  H->>P: connect (≤ 500 ms), hello{role hook, identity, hook input}
  P->>P: bind session (chain ∩ registered PIDs, cwd fallback, ambiguous → neutral)
  P->>P: items = deferred/parked, unreported verification, undelivered events, user requests, drop items already blocked for this session
  alt items present
    P-->>H: {block:true, reason}
    H-->>A: stdout {"decision":"block","reason":"…"}, exit 0
    A->>A: continues, following the reason
  else nothing, app down, timeout, error
    H-->>A: exit 0, no output (neutral)
  end
```

Decision table (each row is a test in §11):

| `stop_hook_active` | Session bound | Items after once-per-item filter | Result |
|---|---|---|---|
| true | any | any | neutral |
| false | no (no PID intersection, ambiguous cwd) | — | neutral; UI asks "which session?" later |
| false | yes | none | neutral |
| false | yes | ≥ 1 | block with reason (up to 3 items, English) |
| — | app unreachable / > budget | — | neutral |

### F-11 Detach, disconnect, transfer (SRV-21, SRV-22, TOOL-08, VER-06, DD-16)

```mermaid
sequenceDiagram
  participant U as User
  participant A1 as Agent (session 1)
  participant S1 as Server 1
  participant P as App
  participant A2 as Agent (session 2)
  participant S2 as Server 2
  Note over A1: user presses Ctrl+C during the call
  A1->>S1: notifications/cancelled (documented as sent, not relied on)
  S1->>P: handoff.detach_call{cancelled}
  P->>P: banner "the agent will pick up on its next resume", steps navigable, Done works
  U->>P: Done → awaiting_verification, outcome queued
  Note over A1: session 1 exits
  S1--xP: EOF
  P->>P: session 1 disconnected, tab "server disconnected", awaiting_verification → not_verified
  U->>A2: new session, "resume handoff hf_x"
  A2->>S2: handoff_to_user{resume: hf_x}
  S2->>P: handoff.resume
  P->>P: resumed_from = session 1, any still-attached call gets transferred_to_other_session
  P-->>S2: outcome not_verified (instruction: a late report is accepted)
  S2-->>A2: not_verified
  A2->>S2: handoff_verify{ok:true} → verified, late:true
```

### F-12 Screenshot pipeline (CAP-01..06, OCR-01..05, DET-01..03, PREV-01..05, LOG-03)

```mermaid
flowchart TD
  B[Screenshot button] --> C{Full screen or Select region<br/>chosen every time}
  C -->|Full| H1[hide overlay] --> M[capture monitor under cursor]
  C -->|Region| H2[hide overlay] --> R[transparent overlays on all monitors → drag → crop]
  M --> PV[preview appears at once: state analyzing, send disabled]
  R --> PV
  PV --> OCR[OCR on full-resolution original: OS engine, else the bundled engine, 10 s per engine]
  OCR --> DET[certain patterns → locked boxes<br/>suspected heuristics → flagged boxes<br/>exempt: spec values that passed ingress]
  DET --> ED[user: unlock, add box, crop, edit text, comment]
  ED -->|Send image, if allowed| IMG[downscale to 1600 px → rescale boxes +2 px → burn black → PNG]
  ED -->|Send text| TXT[edited text with certain matches replaced by REDACTED tags]
  IMG --> OUT[outcome screenshot + context → server → agent]
  TXT --> OUT
  OUT --> LOG[log: text as sent or image hash, size, boxes, engine, never pixels]
```

### F-13 Install, consent, uninstall, repair (INST-01..08, APP-01, CAP-04, FM-23)

```mermaid
sequenceDiagram
  participant U as User
  participant P as App
  participant FS as Agent config files
  P->>P: first launch: scan for agents
  P->>U: consent screen: 3 modifications for Claude Code (T-026), each with Show (exact lines), autostart box pre-checked
  U->>P: accept
  P->>FS: backup files, write MCP entry (fixed path, env, per-server timeout), append Stop and SubagentStop hooks
  P->>P: create ~/.handoff/channel.token (0600) if missing, verify by re-reading
  P->>U: macOS: screen recording explanation + button to the settings pane, shortcut check
  Note over P: every later launch: scan again (notice once per new agent), verify registered path == own path, else offer repair
  U->>P: Uninstall (settings)
  P->>FS: remove only our MCP entry and hook lines (MCP_TOOL_TIMEOUT is never touched)
```

### F-14 Update check (UPD-01, UPD-02, NET-01)

```mermaid
sequenceDiagram
  participant P as App
  participant E as net::egress
  participant D as Update endpoint
  P->>P: launch, setting "check for updates" on? cache older than 24 h?
  P->>E: get(check URL with app version and OS)
  E->>E: record network_events{at, domain, bytes_sent, purpose:"update-check"}
  E->>D: GET
  D-->>E: {latest, notes, download_url}
  E-->>P: response
  P->>P: newer? → notice with notes, Download (opens browser), Remind me later
```

---

## 10. Failure-mode matrix (PRIN-10, NFR-10, NFR-12)

Each row states what still works and how the system degrades; "broken" never appears in the last column.

| FM | Condition | Detected by | Behaviour | Still works | Degraded to | Req. |
|---|---|---|---|---|---|---|
| FM-01 | App not running at call time | Server: connect fails | `text_mode` outcome with rendered spec and instruction | Guidance in chat; agent-composed outcome | No overlay, no log, no verified state | SRV-14, SRV-15, ARCH-04 |
| FM-02 | App started after the session | Server retry loop | Registration within 30 s of app start | Everything from then on | Earlier calls were text mode | SRV-20 |
| FM-03 | Hooks not installed or disabled | Nothing to detect | Instructions in outcomes carry the resume/verify duties; `deferred`/`parked` texts omit the hook mention | All flows | No safety net for forgotten resumes; user can copy a resume request from the overlay | PRIN-10, SRV-12 |
| FM-04 | Raised timeout not applied (consent declined, overridden) | Server reads env / table | Heartbeat at the row default − 60 s or 50 s | Long handoffs via `in_progress` + resume | More frequent resume turns | TOOL-06, TOOL-06a |
| FM-05 | Agent does not accept images | Capability row | Only **Send text** shown; outcome text-only | Screenshot as OCR text | No pixels to the agent | PREV-04, PREV-05, ADPT-04 |
| FM-06 | Heartbeat returned, agent never resumes | App: no attached call | Events queue; hook blocks once (DD-25); tab shows "agent will pick up on resume" | User completes steps; Done queues outcome | Agent-side latency until resume | TOOL-14, SRV-12 |
| FM-07 | Client-side cancellation (Ctrl+C) during a call | MCP cancelled notification (if sent) or heartbeat | `detach_call`; banner; state kept | Steps, Done, notes | Outcome delivered at next resume | SRV-22 |
| FM-08 | Agent exits mid-handoff (server dies) | Socket EOF | Session disconnected; tab banner; handoff kept; `awaiting_verification` → `not_verified` | Resume from any new session; late verify | Verification state pessimistic until report | SRV-21, VER-06, DD-16 |
| FM-09 | Agent alive, server dead (killed) | Agent error on next call; app EOF | Tool description: reconnect (`/mcp reconnect`) and `resume` | Handoff state | One extra turn | SRV-21 |
| FM-10 | Token missing or mismatch | App auth check | `auth_failed`; server logs; text mode with `CHANNEL_AUTH_FAILED` fix text; app settings show "repair token" | Text mode | No overlay until repaired | SRV-07 |
| FM-11 | Channel protocol version mismatch | `hello` | `protocol_unsupported`; text mode with "update the app" | Text mode | No overlay until updated | SRV-06, §3.6 |
| FM-12 | Socket path too long; stale socket file; pipe name collision | App at listen; server at connect | Pointer file `app.sock.path`; stale file removed after liveness check; per-user pipe suffix | Channel | None | SRV-04 |
| FM-13 | App crashes or is restarted mid-handoff | Server: channel lost; app: startup load | State reloaded from SQLite; servers reconnect and re-attach pending calls | Handoffs continue | Banner while down; crash file written locally | NFR-12, TEL-02 |
| FM-14 | Malformed spec / unsupported version | Validator | `SPEC_INVALID` / `SPEC_VERSION_UNSUPPORTED` with per-field fixes; nothing reaches the app | Agent corrects and retries | One extra turn | SPEC-10, SPEC-11 |
| FM-15 | Secret-like value or text in the spec | Certain detector at ingress | Not rejected; masked in overlay with Show; placeholder in log/runbook; `secret_treated` in outcome; excluded from screenshot exemptions | Copy button copies the true value | Value hidden by default | SPEC-13, DET-03, DET-04 |
| FM-16 | OS OCR unavailable (language pack, API error, timeout) | Engine `available()` / error / 10 s | Fallback to the bundled `ocrs` engine (English) | Detection and text mode | Lower accuracy on non-English text | OCR-03 |
| FM-17 | macOS screen-recording permission missing | Capture returns denied / black | Preview shows explanation and a button to the settings pane; no mid-handoff prompt | Ask, Note, guidance | No screenshot until granted and app restarted | CAP-04 |
| FM-18 | Global shortcut taken | Registration error at startup | Ask once for another combination; tray menu "New request" always available | User requests via tray | Shortcut unavailable until changed | OPEN-03 |
| FM-19 | Runbook folder unreadable / malformed files | Reader | Safety net skipped silently; `handoff_runbooks` returns `RUNBOOKS_UNREADABLE`; invalid files skipped with stderr warning | Opening handoffs | No runbook suggestions | RUN-07, RUN-10 |
| FM-20 | Two open user requests, spec without `request_id` | App linking rule | Linked to the oldest; tab shows "linked to request: … Change" | Handoff proceeds | One click to relink if wrong | OPEN-08, §12.4 |
| FM-21 | Terminal window not found / clipboard unavailable | Focus API failure | Notification "request copied: paste it into session X"; hook delivers at end of turn | Delivery via hook | Slower fast path | OPEN-05, OPEN-06 |
| FM-22 | Ambiguous session identity (same cwd, no PID intersection) | App bind step | Hook neutral; UI asks which tab when the user next interacts | Handoffs | Safety net delayed for that session | SRV-18 |
| FM-23 | App bundle moved (macOS) | Path check at launch | Repair offer rewrites MCP entry and hook commands | After repair, all | Text mode until repaired (agent spawns nothing at the old path) | SRV-25 |
| FM-24 | Windows update while servers run | Installer | Old binary renamed, new copied; running sessions keep the old one; cleanup at next app launch | Update completes | Old sessions run the old server until restarted | SRV-25 |
| FM-25 | Resume while another call is attached | App attach logic | Old call receives `transferred_to_other_session`; new call attaches; `resumed_from` set | Continuity across sessions | None | TOOL-08 |
| FM-26 | Verify report after `not_verified` | Store | Accepted within 7 days; `late: true`; state re-finalised | Honest late verification | None | VER-06, DD-16 |
| FM-27 | Final outcome never consumed | Age check in UI | Shown as orphan after 7 days: view, close by hand, copy id | Resume from a new session | None | SRV-23 |
| FM-28 | Disk full / SQLite write failure | Store write error | Transition refused with a visible error; the UI keeps the in-memory state and retries; no silent loss | Reading, guidance | Actions blocked until space is freed | NFR-12 |
| FM-29 | Update endpoint unreachable / check disabled | Egress error | Silent (log entry only); no notice | Everything | No update notice | UPD-01 |
| FM-30 | Image result too large for the client (`MAX_MCP_OUTPUT_TOKENS`) | Not detectable by the server | Preview offers Send text; docs recommend text for large screens; image long side capped at 1600 px | Text mode | Agent may receive a truncated image result | PREV-05, A-07 |
| FM-31 | Parked handoff resumed by the user, agent gone | Store | Steps work locally; Done queues; a resume request is queued and copied; hook delivers at the session's next turn | Guidance | Verification waits for the agent | RESP-07 |
| FM-32 | `reply` without a pending question | Server/app check | `HANDOFF_NOT_WAITING` unless `replacement_steps` present | Correction rounds | One extra turn | TOOL-04 |
| FM-33 | Hook budget exceeded (slow app) | Hook timers | Neutral exit at 1 950 ms; hooks config `timeout: 5` as a backstop | Agent stops normally | Safety net missed once | SRV-11, NFR-11 |
| FM-34 | Session detached while `awaiting_spec` (user request pending) | Session disconnect | Request re-queued as unassigned; delivered to the next session | Request survives | Delay | OPEN-04a |

---

## 11. Testing strategy (NFR-15, NFR-10, PRIN-11)

### 11.1 Principles

1. **Deterministic gates, non-deterministic canaries (DD-34).** Everything that can be tested without a language model is a merge gate: schemas, detectors, matching, the channel, the state machine, the hook decision, installation edits. The real-agent suite (§11.5) is a canary that runs on every Claude Code release and on a schedule; it alerts, and a failure is triaged before it gates a release.
2. **One fixture set, two implementations.** `handoff-mcp/fixtures/` is run by both repositories (§3.4); a fixture cannot pass in one and fail in the other.
3. **Every fact borrowed from an agent has a test** (PRIN-11): the capability table rows and the assumptions of Appendix B map one-to-one to canary assertions (§11.5).
4. **Every row of §10 has a test**, at the lowest level that can reproduce it.

### 11.2 Unit and contract tests

| Suite | Repository | Covers |
|---|---|---|
| Schema validation | both | `fixtures/specs/{valid,invalid}`, every semantic rule S1–S6 with the exact error `path`/`fix`, size limits, `additionalProperties` |
| Certain detector | both | `fixtures/secrets/positive` (recall 1.0) and `negative` (zero matches); every pattern compiles in JS and Rust; no generated id matches any pattern |
| Runbook matching and conversion | both | `fixtures/matching`, normalisation of `where` (arrows, case, separators), stop-words, `{{name}}` → `[name]`, `values_to_fill` |
| Channel codec | both | Every message in `fixtures/channel/*.jsonl` validates against `channel.v1.schema.json`; round-trips through each codec |
| Outcome rendering | server | Every status has `final`, `instruction`, all fields present; image block only with `images_in_results` |
| Heartbeat arithmetic | server | Timeout sources precedence; margins; 50 s floor |
| Text mode rendering | server | Masking of secret-treated values; snapshot tests |
| Ancestor chain | both | Parsing of `ps` output; app-side completion from a synthetic process table |
| Hook subcommand | server | Budget timers with a slow fake app; neutral on every error path; JSON output shape |
| Store state machine | app | Property-based: random sequences of user/agent actions never violate invariants (at most one attached call; undelivered FIFO preserved; final states reached only through the tool; `verified` never without a report; counters reset per round) |
| Hook decision | app | The decision table of F-10; once-per-item per session; `stop_hook_active` guard |
| Redaction geometry | app | Boxes rescaled correctly; burn-in after resize; OCR of the redacted output finds no certain pattern (glyph-leak test) |
| Suspected detector | app | Entropy thresholds, labels, exemption of spec values |
| Placeholder substitution | app | Longest-first; array items; secret-treated never written; detector re-run on output |
| Install adapters | app | Golden-file tests on synthetic `~/.claude.json` and `settings.json`: apply, apply twice (idempotent), uninstall leaves unrelated content byte-identical, existing hooks preserved, an existing `MCP_TOOL_TIMEOUT` never touched (T-026) |
| Log invariants | app | After every scenario, no table row contains a value from the spec's `values` or any fixture secret (grep on a database dump) |

### 11.3 Integration with test doubles

- `handoff-mcp/test/fake-app`: an open implementation of the channel listener that replays `fixtures/channel` scripts and records what the server sends. Server integration tests drive the MCP side with the SDK's in-memory client and exercise F-01..F-11 end to end through a real socket, including reconnection with backoff, `detach_call`, transfer and text mode.
- `handoff-app/tests/fake-server`: a channel client that plays the server's part against the real app (headless Tauri build), exercising the store, the hook decision and the UI bridge with the same scripts. Both doubles are validated against the same golden sequences, so the fakes cannot drift from the real peers.

### 11.4 App UI and platform tests

`tauri-driver` (WebDriver) runs the frontend against `fake-server` for the step view, collapse/expand, the request sheet, preview interactions (unlock, add box, crop, send buttons visibility), settings pages and onboarding. `FakeCapture` returns fixture screenshots so the OCR → detection → preview path is testable on CI without a display. Real-hardware checks (manual matrix, §11.6) cover what CI cannot: actual capture, OS OCR engines, permissions, always-on-top, terminal focus.

### 11.5 End-to-end with a real agent (NFR-15) and release watch

**Harness.** The app is built with `--features e2e`, which compiles in an **automation channel** (**DD-33**): a second local socket, token-protected, offering `act(handoff_id, action)` and `state()` so the harness can play the user (confirm, note, skip, ask, defer, abandon, screenshot with a fixture image, send text or image, done). It does not exist in release builds (feature flag off; CI asserts the symbol is absent from release binaries), because a hidden control channel in a trust-sensitive app must not ship. Claude Code runs non-interactively: `claude -p "<prescriptive prompt>" --mcp-config <tmp> --strict-mcp-config --allowedTools "mcp__handoff__*" --output-format json --max-turns 12`, with hooks in a temporary project `.claude/settings.json` (documented to run in `-p`, A-06).

**Scenarios** (each with assertions on the transcript JSON, the app database and the runbook folder):

| Scenario | Asserts |
|---|---|
| E2E-1 open → confirm all → done → verify true | `verified` in log; runbook file valid; outcome fields |
| E2E-2 ask → reply → done | `question` outcome; reply visible in state; one round |
| E2E-3 screenshot image (fixture with a fake Stripe key) | Image block present in the agent's tool result; agent describes the visible non-secret text; the key region is black (OCR of the sent PNG finds no pattern); log has hash, no pixels |
| E2E-4 defer → agent resumes → done | `deferred` then `awaiting_verification`; deferral_count 1 |
| E2E-5 defer twice → agent stops → hook blocks once → agent resumes | `parked`; exactly one `hook_blocks` row; transcript shows the block reason |
| E2E-6 verify false → replacement steps → verify true | Two rounds; runbook update proposal state |
| E2E-7 heartbeat with a 90 s timeout injected via `HANDOFF_TOOL_TIMEOUT_MS` | `in_progress` returned at 30 s; resume attaches; final outcome once |
| E2E-8 app stopped → text mode | `text_mode` outcome; no database row |
| E2E-9 user request via automation channel → clipboard text → hook delivery | Handoff adopts the request id |
| E2E-10 unverified: agent told to finish without reporting | `not_verified` after the injected 20 s verifying timeout; hook blocked once |
| E2E-11 resume from a second `claude -p` session | `resumed_from` set; `already_delivered` on a second resume |

**Assumption canaries** (PRIN-11): each Appendix B assumption that concerns Claude Code has a dedicated assertion: server registered before the first tool call (A-01); `HANDOFF_AGENT` visible to the server (A-02); `MCP_TOOL_TIMEOUT` from settings visible and honoured (A-03: a test tool sleeping past the configured value is cancelled); per-server `timeout` honoured (A-04); hook JSON block continues the run and `stop_hook_active` is present (A-05); hooks run in `-p` (A-06); image blocks reach the model (A-07); `clientInfo.name` recorded and diffed against the table (A-08); cancellation notification observed on interrupt (A-09, informational); ancestor chain of the hook contains the agent PID (A-11).

**Release watch** (`canary.yml`): a scheduled job checks the npm registry for a new `@anthropic-ai/claude-code` version (there is no documented changelog feed, A-22), installs it on macOS and Windows runners, runs E2E-1..11 and the assumption canaries, and opens an issue with the version and the failing assertions. Model non-determinism is handled with prescriptive prompts, `--max-turns`, one retry, and a classifier that separates protocol failures (a gate) from model-behaviour failures (an alert).

### 11.6 Manual and platform matrix

| Check | macOS | Windows |
|---|---|---|
| Full-screen capture on the cursor's monitor; region across two monitors | ✓ | ✓ |
| Screen-recording permission flow (denied → explanation → granted → restart) | ✓ | n/a |
| OS OCR on English and Italian dashboards; bundled `ocrs` fallback (Windows language pack removed) | Vision | Windows.Media.Ocr |
| Always on top over full-screen browsers; collapse on blur; position per monitor | ✓ | ✓ |
| Tray icon, badge, Quit only from tray | ✓ | ✓ |
| Shortcut conflict prompt | ✓ | ✓ |
| Terminal focus: Terminal.app, iTerm2, VS Code terminal | ✓ | Windows Terminal, conhost, VS Code |
| Second OS user cannot connect to the socket / pipe | ✓ | ✓ |
| Firewall test as documented (NET-02): zero connections, then exactly one domain | ✓ | ✓ |
| Unsigned-launch instructions (SmartScreen) | n/a | ✓ |
| Signed and notarized bundle opens from a fresh download | ✓ | later |

### 11.7 Security tests

Token mismatch rejected and logged without token material; socket mode `0600`; pipe DACL; certain corpus precision ≥ 0.999 on the negative corpus and recall 1.0 on the positive; suspected detector recall ≥ 0.9 on a labelled screenshot corpus with the false-positive rate reported per release; glyph-leak test on every redaction fixture; log-never-contains-values test after every scenario; zero-egress test: the e2e suite runs with the system firewall blocking the app and asserts that no action fails and `network_events` is empty, then with the update check enabled asserts exactly one row.

### 11.8 Test matrix

| Feature | Unit | Contract | Integration | E2E | Manual |
|---|---|---|---|---|---|
| Spec validation and errors | ✓ | ✓ | ✓ | ✓ (E2E-1) | |
| Certain / suspected detection | ✓ | ✓ | ✓ | ✓ (E2E-3) | ✓ |
| Runbook match / convert / write / update proposal | ✓ | ✓ | ✓ | ✓ (E2E-1, 6) | |
| Channel, auth, reconnection, transfer | ✓ | ✓ | ✓ | ✓ (E2E-11) | |
| Heartbeat and resume | ✓ | | ✓ | ✓ (E2E-7) | |
| Store state machine and undelivered queue | ✓ | | ✓ | ✓ | |
| Hook decision and delivery | ✓ | | ✓ | ✓ (E2E-5, 9, 10) | |
| Text mode | ✓ | | ✓ | ✓ (E2E-8) | |
| Capture, OCR, preview, redaction | ✓ (geometry) | | ✓ (FakeCapture) | ✓ (fixture) | ✓ |
| Installation adapters | ✓ | | | ✓ (temp config) | ✓ |
| Network transparency | ✓ (lint) | | | ✓ (zero egress) | ✓ (firewall) |
| Window, tray, shortcut, autostart, i18n | ✓ (i18n keys) | | ✓ (webdriver) | | ✓ |

---

## 12. Closure of the design-level items in `REQUIREMENTS.md` §14

Owner-deferred items are untouched: license verification (`license::check()`, LIC-02), Linux (the bundled `ocrs` engine and the Unix-socket channel are the future basis; nothing in the design is macOS/Windows-only except the OS OCR engines, the capture backend and the installer), Windows code signing (the unsigned path and the SmartScreen documentation are designed, NFR-09).

### 12.1 Publication of the socket protocol → closed: stays internal in v1, publishable by a documentation act

Decision: the channel remains internal (SRV-06). Its definition is versioned and machine-readable in the open repository (§3.2, §6) with the "internal, subject to change" notice; both peers validate against it; the protocol version is compared at hello. Publication later requires only a stability promise and a docs page; no code changes. Rationale: the definition must exist somewhere the open server can be built from; declaring it internal costs nothing now and preserves the freedom to change it after the format is stable (DESIGN-TREE 7.4). *Alternative rejected:* an undocumented, code-only protocol (untestable across two repositories).

### 12.2 One-click write of secrets into env files (v2) → closed for v1: no file-writing path exists, extension point reserved

Decision: in v1 the app writes files only in the app data directory, in `~/.handoff/` (runbooks, token, socket), as backups of agent configuration files during install/uninstall (§7.15), and to a path the user picks in an export dialog. No module can write into a project file. The v2 feature is reserved as a `secrets::writer` module behind a Cargo feature `secrets-write` (off in v1 builds) and a settings opt-in, limited to `.env`-style files, showing only path and variable name (SEC-03). Rationale: the trust story (Network page, preview, log) must exist before the app gains a project-writing capability (DESIGN-TREE 2.4). v1 keeps **Open file** (SEC-02).

### 12.3 Runbooks in the project folder → closed for v1: single root, list-shaped API

Decision: the reader (§5.10) takes a list of roots and is configured with exactly one, `~/.handoff/runbooks/`. Adding `<project>/.handoff/runbooks/` later is a configuration and documentation change with no format change. Rationale: RUN-03 and DESIGN-TREE 6.2 exclude project folders now; the list-shaped API costs nothing and avoids a later refactor.

### 12.4 Two open user requests in one session without `request_id` → closed: accepted mismatch, one-click correction

Decision: the linking rule stays as OPEN-08 states (oldest open request of the session); the tab shows "linked to request: …" with **Change** (§7.7, FM-20); the clipboard text always carries the id so the fast path normally avoids the case. No protocol or format change. Rationale: the mismatch is rare and reversible; a heavier disambiguation (asking the agent, or refusing to link) would cost a turn in every case to save a click in a rare one.

---

## 13. Implementation plan

Dependency order, aligned with the launch scope (Claude Code, macOS, then Windows, then adapters in the committed order, ADPT-06). Each milestone ends with its gates green (§11) and its requirements traceable.

| M | Scope | Depends on | Exit criteria | Requirements |
|---|---|---|---|---|
| **M0 Foundations** | Two repositories; `schemas/`, `patterns/`, `protocol/channel/`, `fixtures/` v1; CI skeletons proving clean-checkout builds; release pipeline of `handoff-mcp` (artifacts, checksums, signature); `server.lock.json` and `fetch-server`; SEA spike (A-12) | — | Both CIs green from clean checkouts; a tagged `handoff-mcp` 0.1.0 artifact is fetched and verified by the app build | ARCH-01..03, SRV-24..26, NFR-16 (skeleton) |
| **M1 Server usable alone** | Validator and errors; certain detector; text mode; tool contract with descriptions; capability table (claude-code, unknown); heartbeat/resume/in-flight table; runbook reader, matcher, converter; `hook stop`, `validate`, `doctor`; `fake-app`; npm package; docs (format, tool contract, text mode, threat model) | M0 | `npx baton-handoff-mcp` in Claude Code runs a full handoff in text mode; all §11.2 server suites green; integration F-01..F-11 against `fake-app` | ARCH-04, SPEC-*, TOOL-*, SRV-01..20, RUN-05..08, RUN-10, ADPT-01..03, ADPT-05, ADPT-07 |
| **M2 App core on macOS** | Tauri skeleton; channel listener and auth; session registry and chain completion; store, state machine, persistence, undelivered queue; overlay UI (tabs, step view, collapsed bar, actions, masked values, Open file); user requests (shortcut, sheet, clipboard, terminal focus); hook decision; Claude Code install adapter and consent screen; token; tray; autostart; i18n en/it; `fake-server` | M1 | E2E-1, 2, 4, 5, 7, 9, 11 pass on macOS with the real Claude Code; property tests green; onboarding registers Claude Code with the three modifications (T-026) | OPEN-*, MULTI-*, GUIDE-*, RESP-*, SRV-21..23, INST-*, WIN-*, APP-*, SEC-01, SEC-02, ADPT-04, ADPT-08 |
| **M3 Verification, log, runbooks** | Verifying view and timers; `handoff_verify` path; late reports; rounds and history; SQLite log complete, delete, export; runbook writer, update proposals, failed marks; orphan list | M2 | E2E-1, 6, 10 pass; log-never-contains-values test; runbook fixtures round-trip server ↔ app | VER-*, LOG-*, RUN-01..04, RUN-09, RUN-11, RUN-12 |
| **M4 Screenshot pipeline (macOS)** | Capture backend, region overlays, permission flow; Vision OCR; bundled `ocrs` fallback; suspected detector; certain patterns from vendor; preview; redaction burn-in; send image/text; `FakeCapture` | M2 | E2E-3 passes; glyph-leak and corpus tests green; manual matrix rows for capture and OCR | CAP-*, OCR-*, DET-*, PREV-*, CTX-01, PRIN-09 |
| **M5 Trust and release (macOS)** | Egress module and lint; Network page; update check; crash files; license entry point; docs: firewall test, consent, install; signing and notarization; release watch job | M3, M4 | Zero-egress test; firewall test documented and performed; notarized DMG opens on a clean Mac; `canary.yml` running | NET-*, UPD-*, TEL-*, LIC-*, NFR-01..05, NFR-08, NFR-15 |
| **M6 Windows** | Named pipe with DACL; native server exe at the fixed path; NSIS rename hook; `Windows.Media.Ocr`; capture; autostart; shortcut; terminal focus; SmartScreen documentation; e2e on a Windows runner | M5 | E2E suite on Windows; manual matrix Windows column; second-user pipe test | SRV-04, SRV-19, OCR-02, NFR-09, ADPT-06 (platform) |
| **M7 Adapters** | Codex (base: validates the degraded path first, capability row, install adapter, canary), then Cursor (editor session identity, chain matching at editor level), then GitHub Copilot (reuses Cursor's identity code), then OpenCode | M6 | Each adapter: capability row measured, install adapter with consent screen, E2E subset on both platforms, documentation of its support level | ADPT-06, ADPT-02 (per-agent code), INST-08 |

Cross-cutting from M0 onward: every merge runs the fixture suites of both repositories; every `handoff-mcp` release bumps the app lock file through a pull request that runs the app CI.

---

## 14. Risk register

| R | Risk | L | I | Mitigation | Early indicator |
|---|---|---|---|---|---|
| R-01 | Claude Code changes hook semantics or fields (Stop/SubagentStop, `stop_hook_active`) | M | H | Hook is a transport; degraded mode without hooks is designed (FM-03); release watch (§11.5) | Canary A-05 fails |
| R-02 | `MCP_TOOL_TIMEOUT` semantics or default change; per-server `timeout` absent | M | M | Heartbeat + resume hold without any timeout (FM-04); timeout sources are layered (§5.6) | Canary A-03/A-04 fails |
| R-03 | Documented default tool timeout is already long, making INST-03 counter-productive | M | M | OI-02 raised to the owner before M2; the installer's consent text can be adjusted without design change | A-03 verification |
| R-04 | Node SEA binaries fail notarization or crash under the hardened runtime | M | H | Spike in M0 (A-12); fallback: ship Node runtime + script inside the bundle behind the same fixed path (still no user-installed Node) | Spike result |
| R-05 | Image results truncated or dropped by `MAX_MCP_OUTPUT_TOKENS` | M | M | 1600 px cap; text mode always offered; docs; measure in E2E-3 | Canary A-07 |
| R-06 | Vision / Windows OCR misses small dashboard text, weakening certain-level redaction | M | H | Full-resolution OCR; suspected heuristics; mandatory preview with add-box; corpus-based precision/recall gates | Corpus metrics |
| R-07 | False positives of the suspected detector annoy users into unlocking everything | M | M | Exemption of spec values; thresholds tuned on the corpus; unlock is one click; certain set kept precise | Unlock rate in e2e logs |
| R-08 | Agents ignore `resume` / `handoff_verify` instructions → many `not_verified` | M | M | Hook safety net; explicit instructions in every outcome; canary tracks not-verified rate | E2E-10 rate |
| R-09 | Named pipe DACL or single-instance behaviour differs across Windows versions | L | M | Manual second-user test in M6; fall back to token-only with a warning if DACL creation fails | M6 tests |
| R-10 | Tauri always-on-top or blur detection unreliable on one platform | M | M | Collapse also on a 3 s timer after last interaction as a fallback; manual matrix | Manual matrix |
| R-11 | Terminal focus heuristics fail for common terminals | H | L | Best effort by requirement; notification + hook delivery cover it | Manual matrix |
| R-12 | Ancestor-chain intersection empty in editor-hosted agents (Cursor, Copilot) | H | M | Planned per-agent identity code (M7); cwd fallback and "which tab?" prompt exist from M2 | M7 canaries |
| R-13 | Runbook matching too loose or too strict | M | L | Deterministic rule with `matched_words` shown; fixtures; the agent still decides | Fixture review |
| R-14 | Windows unsigned build deters users (SmartScreen) | H | M | Clear documentation (NFR-09); signing after launch (owner item) | Support requests |
| R-15 | Windows in-place server update fails under lock despite rename | L | M | NSIS hook tested in M6 with a running session; fallback instruction to close sessions | M6 tests |
| R-16 | Bundle size (≈ 90 MB server + app) perceived negatively | L | L | Documented; single copy; not Electron | — |
| R-17 | Undocumented `clientInfo` makes npm-only installs land on the `unknown` row (50 s heartbeats) | M | L | Installer env var is the primary identity; docs tell npm users to set `HANDOFF_AGENT` | `doctor` output |
| R-18 | Single developer bandwidth across two repositories and two platforms | H | H | Strict milestone order; server-alone value shipped at M1; adapters after launch | Milestone slips |
| R-19 | Secret leakage through a path not covered (logs, crash files, runbook text) | L | H | Log-never-contains-values test; detector re-run on runbooks; crash files carry ids only; stderr never logs values | Test failures |

---

## 15. Open issues and recorded contradictions

Every item follows `REQUIREMENTS.md`; the list exists so the owner can align the sources or confirm a design refinement.

| OI | Item | Where | Resolution in this design |
|---|---|---|---|
| OI-01 | `IDEA.md` counts four final states; `REQUIREMENTS.md` VER-01 has five (adds **abandoned**) | IDEA "Verifica" vs VER-01 | Five states (§8.1). IDEA could list abandoned explicitly. |
| OI-02 | INST-03 raises `MCP_TOOL_TIMEOUT` to 30 minutes on the premise that the default is short. The current Claude Code documentation (checked 2026-09-07, A-03) describes the default as very long (≈ 28 h) and documents a per-server `timeout` field in the MCP entry (A-04). If both hold, writing 30 minutes globally **lowers** every server's timeout, and the per-server field satisfies TOOL-05 with no global side effect | INST-02, INST-03, TOOL-06 vs documentation | Implemented as written: the consent screen and the 30-minute global value stay (INST-03), and the per-server field is written too (§7.15). **Recommendation to the owner:** after A-03/A-04 are verified, consider replacing the global variable with the per-server field and removing the fourth consent line. Design impact: one adapter row and one consent line. **Closed 2026-09-08 (T-026): Option B.** A-03 and A-04 verified by the canary against Claude Code 2.1.263 (both fields honoured, in milliseconds; the default is only bounded from below, > 70 s, documented ≈ 28 h). The installer writes the per-server `timeout` field only; the global variable and its consent line are dropped. Amended: INST-02, INST-03, §4.1, §7.15, F-13, §11.2. |
| OI-03 | OPEN-04 speaks of a "request window"; MULTI-04 says "never two windows" | REQUIREMENTS internal | DD-10: the request sheet is a mode of the single window. Wording could say "request sheet". |
| OI-04 | CAP-02 region selection across all monitors needs transient full-screen overlays | CAP-02 vs MULTI-04 | DD-29: capture tools are not handoff windows; documented. |
| OI-05 | DET-02 locates the certain detector in the server, but screenshots are redacted only in the app (DET-01) | DET-01 vs DET-02 | The app applies the server's public pattern file (§2.4, §7.10); the detector's definition and the ingress check remain in the server. No contradiction with ARCH-03 (the app reuses the server's file, never the reverse). |
| OI-06 | SRV-17 says the hook walks its own chain and sends the list; on Windows that is too slow for the 2 s budget | SRV-17, SRV-11 | DD-22: sender best effort, app completes the chain from its process table; the app always ends with the full chain of both peers. |
| OI-07 | SRV-22 says the "session detached" banner disappears when the session re-registers; after Ctrl+C the session does not re-register (the server survives), a call re-attaches | SRV-22 | §8.4 defines two banners: call detached (clears on re-attach) and server disconnected (clears on re-registration). |
| OI-08 | `REQUIREMENTS.md` declares `DESIGN-TREE.md` as its tie-breaker; the design brief declares `REQUIREMENTS.md` authoritative | Header of REQUIREMENTS | No conflicting case found; the design follows `REQUIREMENTS.md`. |
| OI-09 | The `hf_` id prefix (OPEN-05 example) coincides with the Hugging Face token prefix | OPEN-05, DET-01 | DD-13: ids are 10 characters, tokens 34; contract test guards it. |
| OI-10 | The hook blocks also for an active handoff with an undelivered event and no attached call | SRV-12 lists two cases | DD-25 extension, once-per-handoff cap kept. Confirmed by the owner on 2026-09-07 (T-001 D4). |
| OI-11 | A verification report arriving after `not_verified` re-finalises the handoff | VER-01, VER-06 | DD-16, within 7 days, logged as late. Confirmed by the owner on 2026-09-07 (T-001 D4). |
| OI-12 | The update-check address is "fixed" but not named | UPD-01 | Placeholder `<fixed update domain>`; the owner names it before M5; the firewall test documentation must cite it. Owner decision 2026-09-07 (T-001 D3): left blank; named in T-078 when the update check is reactivated. |
| OI-13 | The product name is provisional ("Handoff" in paths and bundle name) while the fixed launcher path embeds it | SRV-25, REQUIREMENTS §5 | Repair flow covers renames (FM-23); the name should be final before the first public release to avoid a repair on every installation. Closed 2026-09-07 (T-001 D2): display name **Baton**, bundle identifier `com.cepeppe.baton`; every "Handoff" in paths and bundle names reads "Baton". |
| OI-14 | Whether Claude Code sets `CLAUDE_PROJECT_DIR` for MCP servers is unverified | SRV-18, OPEN-02 | A-24; cwd fallback. |
| OI-15 | `IDEA.md` says that the agent replaces the remaining steps through the same handoff (aligned), but also that a runbook is produced from every verified handoff, without mentioning **failed**; RUN-01 adds "never from failed" | IDEA vs RUN-01 | RUN-01 followed (§7.12). Editorial only. |

---

## 16. Glossary

| Term | Meaning |
|---|---|
| Agent | The MCP client process that loads the server (Claude Code at launch; Codex, Cursor, Copilot, OpenCode later) |
| App / overlay | The Tauri desktop application, Baton (`handoff-app`) |
| Attached call | The blocking `handoff_to_user` invocation currently waiting on a handoff; at most one per handoff |
| Call | One blocking invocation of `handoff_to_user`, identified internally by `call_id` |
| Canary | A test against a real agent, run on every agent release; alerts rather than gates |
| Capability row | The entry of the capability table resolved for a session (support level, timeout, images, hooks) |
| Certain / suspected secret | Two detection levels: public high-precision patterns (server-owned) vs heuristics (app-owned) |
| Channel | The internal server ↔ app protocol: JSON-RPC 2.0 over NDJSON on a Unix socket or named pipe |
| Detach | A call stops waiting (heartbeat, cancellation, transfer) while the handoff continues |
| Draft spec | A spec derived from a runbook with `[name]` markers and empty values for the agent to fill |
| Fixed launcher path | The stable path of the bundled server executable written into the agent configuration |
| Handoff | One unit of human work, `hf_…`, from open to a final state, possibly across several rounds |
| Heartbeat | The `in_progress` outcome returned shortly before the agent's tool timeout |
| Hook | The `handoff-mcp hook stop` subprocess run by the agent at Stop / SubagentStop |
| Orphan | A final outcome not consumed for 7 days |
| Outcome | The JSON object returned to the agent and preserved by the log |
| Parked | A handoff deferred twice; it waits in the overlay for the user |
| Placeholder | `{{name}}`, allowed only in runbook files |
| Round | One pass through a step list; replacement steps open a new round |
| Runbook | A JSON recipe saved from a verified or user-confirmed handoff, in `~/.handoff/runbooks/` |
| Secret-treated value | A spec value or text matched by the certain detector at ingress; masked, placeholdered, reported |
| Session | One agent process with the server loaded, tied to a project folder; `ses_…` in the channel |
| Text mode | Degraded operation with the app unreachable: the spec is returned as text and the handoff happens in chat |
| Undelivered event | An interrupting outcome produced while no call was attached; delivered at the next resume |

---

## Appendix A. Requirements traceability

Every identifier of `REQUIREMENTS.md` and the design elements that satisfy it.

### Architecture and principles

| Req. | Design elements |
|---|---|
| ARCH-01 | §3.2 `schemas/`, §3.6 versions, §4.2–4.7 |
| ARCH-02 | §3.1–3.3 (DD-01) |
| ARCH-03 | §2.4 allocation table, §3.2, §3.3, §3.4 (DD-02) |
| ARCH-04 | §5.9 text mode, §5.12 CLI, §3.2 README, M1 (§13) |
| PRIN-01 | §7.7 user requests, §7.8 capture (user-initiated only), F-07 |
| PRIN-02 | §7.6 step view (Ask/Note first-class; Screenshot optional), §7.8 |
| PRIN-03 | §4.2 `secrets`, §7.6 secrets list + Open file, §5.5, §12.2 |
| PRIN-04 | §7.8 (no process between captures), NFR-03 row in §11.7 |
| PRIN-05 | §7.13 single egress (DD-32), §7.9 local OCR |
| PRIN-06 | §7.6 (opener only opens allowed URLs), §7.7 (clipboard and focus only) |
| PRIN-07 | §7.6 counter, no progress bar; §8.4 |
| PRIN-08 | §4.4, §8.1, VER rows |
| PRIN-09 | §7.10 preview and redaction, F-12 |
| PRIN-10 | §10 matrix, DD-08 instructions, §5.6, §4.7.4 |
| PRIN-11 | Appendix B, §5.6 identity order, §11.5 canaries |

### Spec format

| Req. | Design elements |
|---|---|
| SPEC-01 | §4.2 field table |
| SPEC-02 | §4.2 step fields |
| SPEC-03 | §4.2 (`steps` items are objects; strings rejected by schema) |
| SPEC-04 | §4.7.1 `replacement_steps`, §7.4 rounds |
| SPEC-05 | §4.7.1 and §4.7.2 descriptions ("never read values listed in secrets") |
| SPEC-06 | §4.2 `additionalProperties: false` (DD-14), DD-18 (no runbook id in the spec) |
| SPEC-07 | §4.2 S5, §7.6 Open button and auto-link |
| SPEC-08 | §4.2 S5 (list fixed per format version; no dialogs) |
| SPEC-09 | §3.2 `schemas/`, §5.4 |
| SPEC-10 | §4.7.5 error catalogue, §5.4 |
| SPEC-11 | §4.2 S1 |
| SPEC-12 | §4.2 S4, §4.5.4 |
| SPEC-13 | §5.5, §4.2 S7 |

### Tool contract

| Req. | Design elements |
|---|---|
| TOOL-01 | §4.7.1 three shapes (DD-07) |
| TOOL-02 | §4.7.1 nested `spec`, S2 |
| TOOL-03 | §4.3 statuses, §7.4 interrupting vs local actions |
| TOOL-04 | §7.4, §7.6 question pending view |
| TOOL-05 | §5.7 |
| TOOL-06 | §5.7 heartbeat, §4.3 `in_progress` |
| TOOL-06a | §4.1 constants, §5.6 timeout resolution |
| TOOL-07 | §5.7 resume snapshot, `already_delivered` |
| TOOL-08 | §5.7, §7.4 transfer, FM-25 |
| TOOL-09 | §4.7.1 description text |
| TOOL-10 | §4.4, §4.7.2 |
| TOOL-11 | §4.4 table, §8.1 |
| TOOL-12 | §4.3, §3.2 outcome schema |
| TOOL-13 | §4.3 field table |
| TOOL-14 | §4.3 `deferred` instruction |
| TOOL-15 | §4.7.3 |

### Server

| Req. | Design elements |
|---|---|
| SRV-01 | §5 (all), §5.2 |
| SRV-02 | §2.2 (server never launches the app) |
| SRV-03 | §2.2, §5.3 (agent owns the server) |
| SRV-04 | §6.1, §5.8 endpoints (DD-26) |
| SRV-05 | §7.3 listener, §5.3 client |
| SRV-06 | §3.2 `protocol/channel/README.md`, §6, §12.1 |
| SRV-07 | §5.8 token, §6.2, §7.2 creation |
| SRV-08 | §2.1, §6.6, docs in §3.2 |
| SRV-09 | §6.2 |
| SRV-10 | §5.11, §7.15 hook lines |
| SRV-11 | §5.11 budget, §4.1 constants |
| SRV-11a | §5.11 (no guard; connect every turn) |
| SRV-12 | §7.5 decision (DD-25), `hook_blocks` table |
| SRV-13 | §7.5 requests in the reason, §7.7 |
| SRV-14 | §5.9, FM-01 |
| SRV-15 | §5.9 instruction, docs |
| SRV-16 | §5.9 (remote agents) |
| SRV-17 | §5.8 identity payload, §7.5 binding (DD-22) |
| SRV-18 | §7.5 cwd fallback and picker, FM-22 |
| SRV-19 | §3.5 native executable (DD-04), §5.8 chain always registered |
| SRV-20 | §5.3 hello at connect |
| SRV-21 | §5.3, §7.3, FM-08, FM-09, §4.7.1 description |
| SRV-22 | §8.4 banners, FM-07 |
| SRV-23 | §7.4 orphan flag, §7.6 waiting group, FM-27 |
| SRV-24 | §5.1 (DD-05, DD-21) |
| SRV-25 | §3.5 (DD-04), FM-23, FM-24 |
| SRV-26 | §3.5 npm asset, §5.1 |

### Opening, sessions, guidance, responses

| Req. | Design elements |
|---|---|
| OPEN-01 | F-02, §7.6 |
| OPEN-02 | §7.6 tab strip, §5.8 project folder |
| OPEN-03 | §7.16 shortcut, FM-18 |
| OPEN-04 | §7.6 request sheet (DD-10), §7.7 |
| OPEN-04a | §7.7 unassigned requests, FM-34 |
| OPEN-05 | §7.7 clipboard text and focus, FM-21 |
| OPEN-06 | §7.5 hook delivery |
| OPEN-07 | §7.7 (neither rejected method is used) |
| OPEN-08 | §7.7 linking, FM-20, §12.4 |
| OPEN-09 | §7.7 (the app never composes a spec) |
| MULTI-01 | §7.6 one window, tabs |
| MULTI-02 | §7.4 per-handoff state, §5.7 call binding |
| MULTI-03 | §7.6 focus rules |
| MULTI-04 | §7.6 (DD-10), DD-29 |
| GUIDE-01 | §7.6 step view |
| GUIDE-02 | §7.6 value chips (whole and per item) |
| GUIDE-03 | §7.6 Open button, auto-link, S5 |
| GUIDE-04 | §7.6 warning banner |
| GUIDE-05 | §7.6, §8.4 |
| GUIDE-06 | §7.16 language |
| RESP-01 | §7.6 buttons |
| RESP-02 | §7.6 distinct Note and Ask |
| RESP-03 | §7.4 local actions, §4.3 `skipped_steps`, `notes` |
| RESP-04 | §7.4 interrupting actions |
| RESP-05 | §4.3 `deferred`, F-05 |
| RESP-06 | §7.5 |
| RESP-07 | §4.3 `parked`, §7.6 waiting group, FM-31 |
| RESP-08 | §7.6 Abandon always present, §4.3 `abandoned` |
| RESP-09 | §7.4 Done rule |

### Verification

| Req. | Design elements |
|---|---|
| VER-01 | §8.1, §4.3 statuses |
| VER-02 | §4.4 (state only via the tool), property test in §11.2 |
| VER-03 | §4.4 `ok: null` row |
| VER-04 | §7.6 verifying view |
| VER-05 | §7.6 "declared by agent", §7.11 `rounds` |
| VER-06 | §7.4 timer and disconnect rule |
| VER-07 | §7.5 decision |
| VER-08 | §4.7.1 continue with `replacement_steps`, F-08 |
| VER-09 | §7.6 counter and history |
| VER-10 | §7.11 `rounds` |

### Screenshot, OCR, detection, preview, secrets

| Req. | Design elements |
|---|---|
| CAP-01 | §7.8 two choices, F-12 |
| CAP-02 | §7.8 |
| CAP-03 | §7.8 |
| CAP-04 | §7.6 onboarding, FM-17 |
| CAP-05 | §7.9, §7.10 |
| CAP-06 | §7.10 burn-in, §11.2 glyph-leak test |
| OCR-01 | §7.9 (always runs) |
| OCR-02 | §7.9 engines |
| OCR-03 | §7.9 bundled `ocrs`, FM-16 |
| OCR-04 | §7.9 async, §7.6 preview state |
| OCR-05 | §7.9 `lang` hint |
| DET-01 | §7.10, §4.6 |
| DET-02 | §4.6 (DD-20), §2.4 |
| DET-03 | §7.10 exemption list |
| DET-04 | §5.5, §7.6 masked values, FM-15 |
| PREV-01 | §7.10, F-12 |
| PREV-02 | §7.10 |
| PREV-03 | §7.10 editable text |
| PREV-04 | §7.10, §4.7.4 |
| PREV-05 | §7.10 text mode send |
| CTX-01 | §4.3 `context` |
| SEC-01 | §7.6 (never receives values), §4.2 |
| SEC-02 | §7.6 Open file |
| SEC-03 | §12.2 |

### Runbooks

| Req. | Design elements |
|---|---|
| RUN-01 | §7.12 trigger |
| RUN-02 | §4.5.1 |
| RUN-03 | §4.5 files, §7.2 folder |
| RUN-03a | §4.1 folder, §5.10 |
| RUN-03b | §4.5 schema (DD-17) |
| RUN-04 | §4.5 `values` descriptions, §4.5.2 |
| RUN-05 | §4.5.2, §4.5.4 |
| RUN-06 | §4.7.1 and §4.7.3 descriptions |
| RUN-07 | §5.2 step 2, F-03 |
| RUN-07a | §4.5.3 |
| RUN-08 | §4.5 `last_verified_at`, §4.7.3 |
| RUN-09 | §7.12 table |
| RUN-10 | §5.10 vs §7.12 |
| RUN-11 | §7.12 (no export/import functions; files only) |
| RUN-12 | §4.5.3 stop-word list by `lang` |

### Overlay application

| Req. | Design elements |
|---|---|
| INST-01 | §7.15, §7.6 onboarding, F-13 |
| INST-02 | §7.15 table |
| INST-03 | §7.15 row 3, OI-02 |
| INST-04 | §7.15 (append, recognisable entries, restore rule) |
| INST-05 | §7.2 scan, §7.15 detect |
| INST-06 | §7.15 project scope |
| INST-07 | §7.2, §7.15 token creation |
| INST-08 | §7.15 trait keyed by agent id |
| WIN-01 | §7.6, §7.16 |
| WIN-02 | §7.6 |
| WIN-03 | §7.6 collapsed bar |
| WIN-04 | §7.16 tray |
| WIN-05 | §7.16 badge |
| WIN-06 | §7.16 at rest |
| APP-01 | §7.16 autostart, onboarding |
| APP-02 | §7.16 language |
| UPD-01 | §7.13 |
| UPD-02 | §7.13 notice |
| LOG-01 | §7.11 |
| LOG-02 | §7.11 `handoffs`, `rounds`, `events` |
| LOG-03 | §7.11 `sends` |
| LOG-04 | §7.11 deletion and export |
| LOG-05 | §7.11 (no retention job) |
| NET-01 | §7.13 egress + Network page |
| NET-02 | §7.13 firewall test, §11.6 |
| TEL-01 | §7.14 |
| TEL-02 | §7.14 crash files |
| LIC-01 | §7.2, §7.14 |
| LIC-02 | §7.14 `license::check()` |

### Adapters and non-functional

| Req. | Design elements |
|---|---|
| ADPT-01 | §5.6 `support` field, §4.7.4 |
| ADPT-02 | §5.6 table + per-agent identity code |
| ADPT-03 | §5.6 (server reads; app receives the row in hello) |
| ADPT-04 | §7.15 (install), §5.8 (identity), §4.7.4 (text-only), §5.7 (long wait), §7.7 (request delivery) |
| ADPT-05 | §5.6 `unknown` row = base |
| ADPT-06 | §5.6 planned rows, §13 M7 |
| ADPT-07 | §5.9 |
| ADPT-08 | §7.15 hooks, §7.5 same decision |
| NFR-01 | §7.13 (DD-32), §11.7 zero-egress |
| NFR-02 | §7.9 local OCR, §7.13 |
| NFR-03 | §7.8 |
| NFR-04 | §7.10 |
| NFR-05 | §7.13 Network page + docs, §7.10 preview, §7.11 log |
| NFR-06 | §7.1 |
| NFR-07 | §5.1 |
| NFR-08 | §3.5 externalBin signing, M5 |
| NFR-09 | §3.3 docs, M6 |
| NFR-10 | §5.6, §10 |
| NFR-11 | §5.11, FM-33 |
| NFR-12 | §7.4 (DD-11), FM-08, FM-13 |
| NFR-13 | §7.9 async OCR |
| NFR-14 | §7.16 at rest |
| NFR-15 | §11.5 |
| NFR-16 | §3.2 `docs/`, §3.3 `docs/`, §6, §7.13 |
| NFR-17 | §3.7 |

---

## Appendix B. Assumptions register

Each assumption names what the design relies on, its documentation status as checked on 2026-09-07 against the official Claude Code documentation, and the verification method. Canary tests (§11.5) re-verify the Claude Code items on every release.

| A | Assumption | Status | Verification | Design if false |
|---|---|---|---|---|
| A-01 | Claude Code starts stdio MCP servers at session start, so registration at session start (SRV-20) is possible | Documented behaviour for `-p` (servers connected before the first turn); interactive assumed equal | Canary: registration timestamp precedes the first tool call | Registration at first call; user requests before that are delivered by hook only |
| A-02 | The `env` block of an MCP server entry is passed to the server process | Documented | Canary: server echoes `HANDOFF_AGENT` in `doctor` | Identity from `clientInfo`, then `unknown` |
| A-03 | `MCP_TOOL_TIMEOUT` is in milliseconds, settable in the settings `env` block, applied to every session and inherited by subprocesses; the documented default when unset is very long (≈ 28 h) | Documented; verified 2026-09-08 (canary, Claude Code 2.1.263): milliseconds, honoured; the default is only bounded from below (> 70 s) | Canary: a test tool sleeping past the configured value is cancelled; server reads the variable | Heartbeat from the table default or 50 s; OI-02 |
| A-04 | A per-server `timeout` field exists in the MCP entry and bounds our server's tool calls | Verified 2026-09-08 (canary, Claude Code 2.1.263): exists, honoured, milliseconds; basis of the T-026 decision | Canary as A-03 with the field instead of the variable | Global variable only (INST-03) |
| A-05 | Stop/SubagentStop hooks receive `session_id`, `cwd`, `hook_event_name`, `stop_hook_active` (plus `agent_id`, `agent_type` for SubagentStop); JSON stdout `{"decision":"block","reason"}` or exit 2 blocks; user and project hooks merge | Documented | Canary E2E-5, E2E-10; golden hook input fixtures updated per release | FM-03 degraded mode |
| A-06 | `claude -p` runs hooks and loads MCP servers from `--mcp-config` (and project settings) | Documented | Canary harness itself | Interactive PTY harness for hook scenarios |
| A-07 | Image content blocks in MCP tool results reach the model; results are bounded by `MAX_MCP_OUTPUT_TOKENS` (default 25 000 tokens) | Documented | Canary E2E-3: the model describes the fixture image; measure the token cost of a 1600 px PNG | Lower `IMAGE_LONG_SIDE_PX`; text mode |
| A-08 | The value of `clientInfo.name` sent by Claude Code | Not documented | Canary records it per version and diffs against the table | DD-09: installer env var is primary |
| A-09 | Claude Code sends MCP cancellation notifications when a tool call is interrupted | Documented (timing not detailed) | Canary observes the notification on interrupt | Not relied on: heartbeat detaches the call (§5.7) |
| A-10 | `/mcp reconnect <server>` reconnects a disconnected server in a session | Documented | Manual | Instruction text says "reconnect the server or start a new session" |
| A-11 | Hook subprocesses are spawned by the Claude Code process, so their ancestor chain contains the agent PID | Not documented | Canary: chain of the hook ∩ registered PIDs is non-empty | cwd fallback (SRV-18), picker |
| A-12 | Node SEA binaries can be signed with the hardened runtime and notarized (with JIT entitlements) and run from inside the bundle | To verify | M0 spike on a clean Mac | Bundle the Node runtime and script behind the same fixed path (R-04) |
| A-13 | Vision `VNRecognizeTextRequest` reads dashboard text at 1x and 2x well enough for certain-pattern detection | To verify | Corpus metrics gate (§11.7) | The bundled `ocrs` engine on the same image as a second pass |
| A-14 | `Windows.Media.Ocr` availability depends on installed language packs and is detectable | Documented by Microsoft | VM test with packs removed | Bundled `ocrs` fallback (OCR-03) |
| A-15 | Tauri 2 supports always-on-top and delivers blur events reliably on both platforms | To verify | Manual matrix | Timer-based collapse fallback (R-10) |
| A-16 | A named pipe can be created with a DACL restricting access to the current user | Documented Win32 behaviour | Second-user test | Token-only with a warning |
| A-17 | `~/.handoff/app.sock` fits the `sun_path` limit for typical home paths | Verifiable by arithmetic | Unit test on long paths; pointer file fallback (FM-12) | Pointer file |
| A-18 | The terminal window hosting the agent can be found from the ancestor chain for common terminals | To verify | Manual matrix | Notification + hook delivery (OPEN-05) |
| A-19 | Global shortcut registration failure is reported by the plugin | To verify | Unit test with a pre-registered shortcut | Ask the user to test the shortcut in onboarding |
| A-20 | Anthropic API image limits (5 MB, 8000 px) are satisfied by a 1600 px PNG | Documented API limits | Size assertion in E2E-3 | Reduce size or compress |
| A-21 | The settings `env` block and hooks from user and project settings are merged, so adding ours does not replace the user's | Documented | Golden-file tests, canary | Warn in the consent screen |
| A-22 | New Claude Code versions are detectable via the npm registry (`@anthropic-ai/claude-code` dist-tags); no changelog feed is documented | Registry is public | `canary.yml` | Manual trigger |
| A-23 | Claude Code strips environment variables whose names contain TOKEN, SECRET, PASSWORD, KEY or AUTH from servers declared in project scope | Documented | Unit test of variable names; canary with project scope | Names already avoid those substrings |
| A-24 | `CLAUDE_PROJECT_DIR` is set for MCP server processes, or the working directory equals the project folder | Documented for hooks; unclear for servers | Canary compares against the session's cwd | Show cwd; cwd fallback (SRV-18) |
| A-25 | `xcap` captures all monitors on both platforms with correct scale factors | To verify | Manual matrix, unit test of coordinate mapping | Platform capture APIs directly |
| A-26 | `ocrs` and its two `.rten` models run inside a signed bundle with no native dependency | To verify | M4 build | Tesseract as a separate signed helper, only if the measured quality of `ocrs` is not enough for a release |

---

## Appendix C. Design decisions index

| DD | Decision | Section |
|---|---|---|
| DD-01 | Two repositories; root holds documents and dev-only scripts | §3.1 |
| DD-02 | Cross-cutting artifacts owned by `handoff-mcp`, consumed via the release artifact | §3.4 |
| DD-03 | App consumes the server as a pinned, verified release artifact; never a source import | §3.5 |
| DD-04 | The fixed launcher path is the server executable itself; no intermediate process | §3.5 |
| DD-05 | Server in TypeScript on Node.js, compiled to a Single Executable Application | §5.1 |
| DD-06 | Channel = JSON-RPC 2.0 over NDJSON on the local socket, token in `hello` | §6.1 |
| DD-07 | Flat `handoff_to_user` input with shape inference and exclusive-field validation | §4.7.1 |
| DD-08 | Every outcome carries `final`, `status`, `instruction` | §4.3 |
| DD-09 | Agent identity: installer env var → `clientInfo` → `unknown` | §5.6 |
| DD-10 | Request sheet is a mode of the single overlay window | §7.6 |
| DD-11 | Handoff state persisted write-through in SQLite; server holds no durable state | §7.4 |
| DD-12 | Undelivered interrupting events are queued per handoff and delivered one per call | §7.4 |
| DD-13 | `hf_` + 10 base32 ids, shared between user requests and handoffs | §4.1 |
| DD-14 | `additionalProperties: false` and explicit size limits in the spec schema | §4.2 |
| DD-15 | One `status` enumeration for final states and return reasons | §4.3 |
| DD-16 | Late verification reports accepted within 7 days, logged as late | §4.4 |
| DD-17 | Runbook file naming from normalised `where`/`goal` plus id | §4.5 |
| DD-18 | Runbook origin inferred by the matching rule; no new field | §7.12 |
| DD-19 | Draft spec conversion: `{{name}}` → `[name]`, empty values, `values_to_fill` | §4.5.4 |
| DD-20 | One public pattern file in a JS/Rust-common regex subset; precision first | §4.6 |
| DD-21 | Node SEA over Bun/Deno/pkg | §5.1 |
| DD-22 | Ancestor chain: sender best effort, app completes from its process table | §5.8 |
| DD-23 | Server reconnects forever with bounded backoff; calls degrade to text mode meanwhile | §5.3 |
| DD-24 | Server notifies the app when a call detaches and why | §5.7 |
| DD-25 | Hook blocks for deferred/parked, unreported verification, undelivered events, user requests; once per item per session | §7.5 |
| DD-26 | Socket at `~/.handoff/app.sock` with pointer-file fallback; per-user named pipe suffix | §5.8, §6.2 |
| DD-27 | Svelte frontend | §7.1 |
| DD-28 | Contract folder `~/.handoff/` vs app-private data directory | §7.2 |
| DD-29 | Region-selection overlays are capture tools, not handoff windows | §7.6, §7.8 |
| DD-30 | OCR engine selection: OS engine, else the bundled `ocrs` engine; per-engine timeout | §7.9 |
| DD-31 | One SQLite database for state, log and settings | §7.11 |
| DD-32 | Single egress function enforced by dependency lints and CSP | §7.13 |
| DD-33 | e2e automation channel compiled only into e2e builds | §11.5 |
| DD-34 | Deterministic gates vs real-agent canaries | §11.1 |
