# Testing: what each suite proves

This page is the map of the automated suites of this repository: what each one proves, how
to run it, and what stays a manual check. The commands CI runs are in the
[README](../../README.md#checks); the end-to-end harness has a page of its own,
[e2e.md](e2e.md), and so do the WebDriver suite, [ui.md](ui.md), and driving the built app by
hand, [smoke.md](smoke.md).

- [The suites](#the-suites)
- [What runs on which push](#what-runs-on-which-push)
- [The security suite](#the-security-suite)
- [The report](#the-report)
- [What the security suite leaves to others](#what-the-security-suite-leaves-to-others)
- [Manual complements](#manual-complements)

## The suites

Every Rust suite runs from `src-tauri/`, after `node scripts/fetch-server.mjs` has filled
`vendor/`: the crate embeds the pinned format and does not compile without it.

| Suite | Command | What it proves |
|---|---|---|
| Core unit tests | `cargo test --lib` | Every module of `src-tauri/src/` against its own rules, beside the code: the state machine, the hook decision, the detectors, the masking of the log, the endpoint digest, the crash-file filter. |
| Contract | `cargo test --test contract` | The fixtures of the pinned `handoff-mcp` release against the Rust implementations: schemas, secret patterns, channel codec, runbook matching. One fixture set, two implementations. |
| Channel listener | `cargo test --test channel_listener` | The listener over a real pipe or socket, driven by a client that speaks only the wire: framing, the handshake and its refusals, every reply checked against the channel schema. |
| Integration flows | `cargo test --test integration_flows` | The flows of the design's §9 with no window: listener, registry, store and dispatch against `fake-server`, compared with the vendored channel goldens, and the log-invariant check after every flow. |
| Store properties | `cargo test --test store_properties` | Random sequences of user and agent actions never break the invariants of the state machine. |
| Installation golden files | `cargo test --test install_golden` | The adapters over whole synthetic home folders, compared byte for byte: apply, apply twice, uninstall. |
| Runbook round trip | `cargo test --test runbook_roundtrip` | A runbook the app writes is found by the real `handoff-mcp` (RUN-10). |
| Egress boundary | `cargo test --test egress_boundary` | No shipped line names a network type outside `net::egress`, the CSP has no `connect-src`, and nothing calls the egress point in this build. |
| Timers | `cargo test --test timers` | Every timer of the shipped code is declared with the requirement it serves (WIN-06). |
| Corpus generator | `cargo test --test gen_corpus` | The committed screenshot corpus is exactly what its source draws. |
| Automation channel | `cargo test --features e2e --test e2e_channel` | The transport of the e2e channel, which exists only with the feature; CI runs it in a step of its own. |
| **Security** | `cargo test --test security -- --nocapture` | What the design's §11.7 asks for: [below](#the-security-suite). |
| Frontend | `pnpm test` (repository root) | The components and the TypeScript in jsdom, the locale parity, the user documentation and its links. |
| Documentation | `pnpm check:links` and `pnpm test src/__tests__/docs.test.ts` (repository root) | Every relative link and `#anchor` of every Markdown file resolves; `docs/` holds the user pages in both languages and both indexes link them; the pages say what the product promises (the firewall test, the process-tree watch, the WebView2 switches, the `ocrs` attribution, the crate notices). Part of `pnpm test`, and a job of its own in CI, `docs`, which runs on every push: [below](#what-runs-on-which-push). |
| CI classifier | `pnpm test src/__tests__/ci.test.ts` | The script of the `changes` job of `ci.yml`, run under bash with `git` and `gh` faked: which pushes may leave the Windows jobs out, and the jobs that wait for its answer. |
| UI (WebDriver) | `pnpm test:ui` (repository root) | The real window of a debug build with `--features e2e`, driven through `tauri-driver` and `msedgedriver` against fake server sessions and the fixture capture backend: the step view, collapse and expand, the request sheet, the preview, the settings pages, the consent screen. Its own job in CI; [ui.md](ui.md) is the harness. |
| Installer hooks | `pnpm test src/__tests__/installer-hooks.test.ts` | `installer/windows/hooks.nsh` compiled with the real `makensis` and run against folders of its own, with a live process standing in for an agent's server: the in-place rename of FM-24, the uninstaller's cleanup, the application-data box, the login marker. Part of `pnpm test`, where it is skipped with a warning until the first `pnpm tauri build` has downloaded `makensis`; the `windows` job of CI runs it again after its bundle, and there a missing `makensis` fails. |
| Release pipeline | `pnpm test src/__tests__/release.test.ts` | The two checks the release workflow starts with (the tag is the version, the changelog has its section), through their command line, and the decisions written in `release.yml`: the gate on the binary the setup was packed from, the report attached, only the drafting job allowed to write. |
| End to end | `scripts\e2e.ps1` (workspace root) | Ten scenarios with a real Claude Code and a real build of the app; with `-Agent codex`, `-Agent opencode`, `-Agent cursor` or `-Agent copilot`, five of them against a real Codex CLI, OpenCode, Cursor Agent CLI or GitHub Copilot CLI, after a preflight in which the agent reads the file the installer writes, and for Cursor and Copilot one more that launches Cursor's editor or VS Code and reads the session it registers; run by hand and not in CI: [e2e.md](e2e.md). |

## What runs on which push

`ci.yml` asks one question first, in a job of its own on Linux: did this push change anything
but documentation? The job is `changes`, and `windows`, `security` and `ui` wait for its
answer.

| Event | Jobs |
|---|---|
| Push or pull request that changes documentation only | `changes` and `docs`; `windows`, `security` and `ui` show as *skipped*, and the run concludes `success` |
| Push or pull request that changes anything else | `changes`, `docs`, `windows`, `security` and `ui` |
| `workflow_dispatch` | `changes` and `macos`; `windows`, `security` and `ui` only when dispatched with `-f windows=true` |
| Tag `v*` | `release.yml` alone: `ci.yml` runs on `main` and on pull requests |

Documentation is `docs/**`, `README.md`, `CHANGELOG.md` and `LICENSE*` at the root, with one
exception: the two notices pages, `docs/third-party-notices.md` and
`docs/it/third-party-notices.md`, are generated from `Cargo.lock` and checked by a step of the
`windows` job, so a change to either counts as code. Whatever `changes` cannot decide counts as
code as well: a first push, a force push, a diff it cannot read or that lists nothing, and a
push on top of a commit whose own run did not succeed. That last rule is there because a newer
push cancels the run of the one before it: a documentation push must not turn `main` green over
code that no Windows job passed. A file moved into or out of `docs/` counts under both of its
paths. The allow-list is written in the workflow, where it acts, and `src/__tests__/ci.test.ts`
runs that script.

The job prints its answer and the first reason it met on one line of its log:
`code=false: documentation only, on top of a commit that passed`, or for example
`code=true: .github/workflows/ci.yml is not documentation`. To read it before the whole run
has ended, ask for the job's own log, `gh api repos/{owner}/{repo}/actions/jobs/<job id>/logs`;
`gh run view --log` answers only once every job is done.

The `docs` job runs on every push and pull request, whatever the answer: the link check, and
the two suites that read a file on the allow-list, `docs.test.ts` and `release.test.ts` (which
reads `CHANGELOG.md`). A suite that starts reading one of those files belongs in that job's
command, and `ci.test.ts` fails until it is there.

## The security suite

`src-tauri/tests/security/` gathers the security tests of §11.7 into one target. A check
that could pass by measuring nothing carries a control that has to be seen working first,
and the last column says what it is.

| Check | Module | What it proves | Its control |
|---|---|---|---|
| Token mismatch | `token.rs` | A `hello` with the wrong token is answered `auth_failed`, the connection closes, the attempt is logged, and neither token is in the log, whole or as its first or last eight characters (SRV-07, SRV-08). The log is the production subscriber, in a child process. | The refusal line has to be in the output that is searched. |
| Pipe DACL (Windows) | `endpoint.rs` | The pipe and the token file carry a protected DACL with one allow ACE, for this user's SID, with full access, read back from the objects and not from the string the app handed Windows (A-16). | A pipe created with the default DACL has to fail the same check. |
| Socket mode (macOS) | `endpoint.rs` | The socket is `0600`, `~/.handoff/` is `0700`, the token file `0600`. | None needed: it compares the modes of the real files. |
| Corpus gates | `redaction.rs` | Over the 46 synthetic screenshots: certain recall 1.0 and precision at least 0.999, suspected recall at least 0.9. The suspected false-positive rate is reported, not bounded (R-07). | The labels are written by hand, never by running a detector. |
| Burn geometry | `redaction.rs` | Every ink pixel of every redacted line is black in the image that leaves. | Burning before the resize, the defect CAP-06 names, has to be caught. |
| Glyph leak | `redaction.rs` | An OCR engine of the machine reads no certain secret in a redacted band: every planted key with the platform's own engine, a sample of six with the bundled `ocrs`, and none, said out loud, where no engine answers inside its own time budget. | The unredacted band has to read as the key first. |
| Log never holds a value | `log_invariants.rs` | No value the certain detector matched reaches any column of the database (LOG-02, DET-04). | The values have to be masked in place, not dropped. |
| Crash files | `crash_files.rs` | A crash file written after real traffic, with a planted secret in every field of the spec, carries identifiers only, and the tracing output carries no value either (R-19). The panic is real, in a child process, with the production panic hook. | A value logged in a field the crash ring drops has to reach the tracing output and not the file. |

The values searched for are one list, `forbidden.rs`: a planted certain secret per field of
a spec and per free-text place a flow can reach, plus every line of the pinned format's
`fixtures/secrets/positive.txt`. `tests/integration_flows.rs` reads the same list after every
flow. The e2e suite applies the same rule to the live database after every scenario
(`tests/e2e/db.ts`), with secrets it generates per run, because they travel through a
model's prompt.

The token and crash-file checks run a second copy of the test binary with only themselves
selected, because the subscriber and the panic hook they are about are process-wide: in the
shared test process they would collect every other test's lines and panics. The child prints
a last line the parent waits for, so a mistyped test name fails instead of passing empty.

Run one check alone with a filter, for example `cargo test --test security token --
--nocapture`.

## The report

Every check records a section of `src-tauri/target/security-report.json` as it finishes:
its status (`passed`, `failed`, or `not_performed` for a glyph-leak pass no engine of the
machine could run), what it measured and, where it has one, its threshold. A file left by an
earlier run is replaced rather than merged, so a missing section is a check that did not
finish. The corpus section is the one §11.7 asks to be kept per release:
`certain.precision`, `certain.recall`, `suspected.recall` and
`suspected.false_positive_rate`, with the clean lines it flagged.

In CI the `security` job runs the suite on Windows on every push that changes code
([above](#what-runs-on-which-push)), prints the report and keeps
it as the artifact `security-report-windows`. The macOS leg is dispatch-only while macOS is
deferred, and its `macos` job runs the suite as a step and keeps `security-report-macos`.
The release workflow runs the suite once more on the tagged commit and attaches the report to
the draft release as `Baton-<version>-security-report.json`, so every release carries its own
numbers.

## What the security suite leaves to others

- **The automation channel is absent from a shipped build** (DD-33). The check reads a
  built binary, so it runs where one exists: `scripts/check-no-automation.mjs` in the
  `windows` job of CI, with its positive control, and again on the binary the release
  workflow publishes. [e2e.md](e2e.md#it-is-not-in-a-shipped-build) has the details.
- **Zero egress.** `tests/egress_boundary.rs` holds the boundary in the sources, and the e2e
  suite asserts that `network_events` is empty after every scenario. The firewall half of
  §11.7 is a manual check, below; the variant with the update check enabled, which expects
  exactly one row, arrives with the update check itself (T-078).
- **The bytes an agent was actually sent.** E2E-3 asks the running app what an OCR of the
  sent PNG still reads, with what the detectors found before the burn as its control: the
  end-to-end answer to the question the glyph-leak test answers over the corpus.
- **The log after a real run.** The same rule as `log_invariants.rs`, over the database an
  e2e scenario left behind: [e2e.md](e2e.md).

## Manual complements

Two promises need something a test runner does not have.

- **The second-user test.** The DACL check proves the pipe and the token file name this user
  alone; whether Windows then keeps a second account out needs a second account. It is a
  step of the Windows manual matrix (T-057), and of the macOS one (T-061, deferred with
  macOS).
- **The firewall test.** Blocking Baton's executables in the system firewall and checking
  that nothing fails is what [verify-trust.md](../verify-trust.md) tells a user to do, and
  what the manual matrices run: T-057 on Windows, T-061 with Little Snitch on macOS.
