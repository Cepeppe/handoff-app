# Contextual Handoff System — Requirements Specification

| | |
|---|---|
| Status | Draft 0.2 |
| Date | 2026-09-07 |
| Sources | `IDEA.md` (product vision), `DESIGN-TREE.md` (decision record), owner decisions of 2026-09-07 (§15) |
| Audience | Engineers building the server, the overlay app and the agent adapters; anyone reviewing scope before implementation |

> **Published copy.** This is the requirements document the implementation was written against,
> published with the source on 2026-09-17. `IDEA.md` and `DESIGN-TREE.md`, which it cites, are the
> maintainer's working notes and are not published. The licensing statements reflect the decision of
> 2026-09-17 to publish the app under the MIT licence as well (NFR-17). Decisions taken during the
> implementation that depart from this document are listed in
> [implementation-decisions.md](implementation-decisions.md).

`IDEA.md` states the product at vision level; `DESIGN-TREE.md` records every decision with its alternatives and reasoning. As of 2026-09-07 the two are aligned. Should they ever differ, this document follows `DESIGN-TREE.md`.

Requirement keywords follow RFC 2119: **MUST**, **MUST NOT**, **SHOULD**, **MAY**. Each requirement has a stable identifier (`AREA-nn`) for traceability. Items whose decision is explicitly deferred are collected in §14.

---

## 1. Purpose and scope

### 1.1 Problem

AI coding agents can write code and prepare configuration, but they regularly hit a step that a person must perform in another environment: creating an API key, registering an OAuth app, changing DNS, adjusting IAM, using a web dashboard, changing an OS or application setting, or confirming a sensitive operation.

At that moment the agent already knows what must be done, which values to use and what result to expect, but the user is in a different window. Today the loop is manual: browser, problem, screenshot tool, chat, paste, explain, answer, browser again. And when the user says "done", nothing checks that it is true; the error surfaces many steps later.

The loss is not the manual action itself. It is the loss of continuity between **what the agent knows**, **what the user is seeing** and **what actually happened**.

### 1.2 Product promise

**When work passes from the AI to the user, context must not be lost; when it returns to the AI, the result must be verified.**

The system introduces a **contextual handoff**: a small, public, versioned contract (the *handoff spec*) through which an agent hands the user exactly what is needed for a human step, and receives back a verifiable outcome. A local overlay app is where the user reads the spec and answers.

If features must be sacrificed, the order is fixed: the screenshot goes first, verification second, **step-by-step guidance never** (DESIGN-TREE 0.1).

### 1.3 In scope: the steps that stay human

The system does **not** automate the step. It covers steps that a person must perform by policy, security or responsibility, and steps the user chooses to perform in person:

- credentials: API keys, OAuth apps, tokens;
- authentication: login, 2FA, CAPTCHA;
- billing and plan changes;
- permissions and IAM;
- DNS and domains;
- confirmations of production or irreversible operations;
- OS or installed-application settings the agent cannot or should not touch;
- any step the user prefers to do by hand (for example a system setting they cannot find).

### 1.4 Out of scope

- Steps an agent can safely execute alone and the user does not want to perform in person.
- Clicking, typing or otherwise acting on the user's behalf in external interfaces.
- Continuous screen observation.
- Calling any language model from the app or the server.
- Runbook sharing, import, export or a public library (§10).
- Agents running on remote machines (they get text mode, unsupported; §12.4).
- Licensing enforcement, Linux support (deferred; §14).

### 1.5 Target users and platforms

- **Primary user:** independent developers using Claude Code. The product is personal: one person installs it on their own machine.
- **Launch platforms:** macOS and Windows. Linux later, if viable and not too costly.
- **First agent:** Claude Code. Committed follow-ups, in order: Codex, Cursor, GitHub Copilot, OpenCode (§12).

---

## 2. Definitions

| Term | Meaning |
|---|---|
| **Agent** | An AI coding agent that speaks MCP (Model Context Protocol), e.g. Claude Code. |
| **Session** | One running agent process with the handoff MCP server loaded, tied to a project folder. |
| **Handoff** | One unit of human work handed from agent to user, identified by a `handoff_id`, with a lifecycle from open to a final state. |
| **Handoff spec** | The JSON document produced by the agent that describes a handoff (§4). |
| **Outcome** | The structured result returned to the agent when a handoff ends or needs the agent (§5.3). |
| **Tool contract** | The MCP tools through which an agent opens, continues, resumes, verifies and searches handoffs (§5). |
| **Server** | The open-source MCP server (`handoff-mcp`) loaded by the agent (§6). |
| **App / overlay** | The desktop application, Baton, that shows handoffs to the user (§7–§9). |
| **Adapter** | The per-agent piece that maps agent capabilities and installation details onto the common system (§12). |
| **Round** | One pass through a handoff's steps. A failed verification opens a new round ("correction 1 of 2") on the same handoff. |
| **Runbook** | A reusable recipe saved from a completed handoff (§10). |
| **Text mode** | Degraded operation when the app is not reachable: the handoff happens in the chat (§6.5). |
| **Certain / suspected secret** | Two detection levels for sensitive strings (§9.3). |

---

## 3. System overview

### 3.1 Components

```
┌─────────────┐   MCP (stdio)   ┌──────────────────┐  local socket / pipe  ┌─────────────────┐
│   Agent     │ ◄─────────────► │  handoff-mcp     │ ◄───────────────────► │  Overlay app    │
│ (Claude Code│   tool calls    │  server (MIT,    │   internal protocol   │  (MIT, Tauri)   │
│  etc.)      │                 │  TypeScript)     │                       │                 │
└─────────────┘                 └──────────────────┘                       └─────────────────┘
       │                                 │                                          │
       │  Stop / SubagentStop hooks ────►│ `handoff-mcp hook stop`                  │ OCR, redaction,
       │                                 │                                          │ log, runbooks,
       │                                 └── validation, capability table,          │ capture, UI
       │                                     heartbeat, resume, text mode,
       │                                     certain-secret detector,
       │                                     runbook search
```

Flow: `Agent → Handoff → User → Verification → Agent`.

### 3.2 Licences and public promises

Everything is released under the MIT licence. What differs from part to part is the promise: the formats and the tool contract are public and versioned, the socket protocol is internal. *Amended 2026-09-17: until then the rule was "closed today, openable tomorrow", and the overlay app was proprietary.*

| Part | License | Notes |
|---|---|---|
| Handoff spec format (with JSON Schema, docs, versioning) | MIT | Public promise 1 |
| Outcome format | MIT | Public promise 2 |
| Tool contract | MIT | Public promise 3 |
| Runbook file format | MIT | Readable by agents without the app |
| MCP server (reference implementation, protocol adapters, hook subcommand) | MIT | Also published as an npm package |
| Server ↔ app socket protocol | Internal | Declared "internal and subject to change"; publication reconsidered after format v1 is stable |
| Overlay app (guidance, capture, OCR, redaction, preview, log, runbook creation/update) | MIT | Distributed as a binary (signing: NFR-08, NFR-09) |

- **ARCH-01** Spec, outcome and tool contract MUST be published, documented and versioned.
- **ARCH-02** Server and app MUST live in two separate repositories from day one.
- **ARCH-03** The server MUST NOT contain app logic. Everything that depends only on protocol or agent facts (validation, capabilities, heartbeat, resume, text mode, certain-secret detection, runbook search, the hook subcommand) belongs to the server; everything that depends on the UI or on trust-sensitive local data (suspected-secret detection, screenshot redaction, OCR, log, runbook creation) belongs to the app.
- **ARCH-04** The server MUST be usable on its own, without the app, as a text-mode handoff protocol in any MCP client.

### 3.3 Design principles (normative)

- **PRIN-01** A handoff can be opened by the agent or by the user; every screenshot is decided only by the user.
- **PRIN-02** The screenshot is optional. The overlay exists first to read and to write.
- **PRIN-03** Secrets never pass through the overlay or the agent.
- **PRIN-04** The system never observes the screen continuously.
- **PRIN-05** The app never calls a model and makes no network calls, with the single exception of the update check (§11.4).
- **PRIN-06** The system never clicks or types on the user's behalf.
- **PRIN-07** Guidance is one step at a time, always saying how many remain. No percentage progress bar.
- **PRIN-08** A handoff is verified by the agent when possible, and declared "confirmed by user" when not. A handoff is never "verified" on trust.
- **PRIN-09** Nothing leaves the machine without redaction and a mandatory preview.
- **PRIN-10** The worst case must be degraded, never broken: without hooks the instructions in the tool result hold; without a raised timeout, resume holds; without image support, text holds.
- **PRIN-11** Only documented agent behaviours may be relied on (MCP blocking calls, images in tool results, the timeout variable, hooks with blocking semantics).

---

## 4. Handoff spec format

### 4.1 Fields

- **SPEC-01** A handoff spec is a JSON object with the following fields.

| Field | Type | Required | Content |
|---|---|---|---|
| `spec_version` | integer | yes | Format version. Currently `1`. |
| `goal` | string | yes | What must be achieved. |
| `where` | string | yes | Where to act: a web service, an application or an OS settings panel, with page or section. Not tied to the browser (e.g. "Stripe Dashboard → Developers → Webhooks", "System Settings → Privacy & Security → Screen Recording", "Docker Desktop → Settings → Resources"). |
| `url` | string | no | Starting point openable with one click (web page, settings panel or application deep link). |
| `why_human` | string | yes | Why a person performs this step. |
| `values` | object (name → value) | yes (may be empty) | Values to use, taken from the project. Each is copyable with one click. A value may be a string or an array of strings. |
| `secrets` | object (variable name → destination file) | no | Values the user will copy **from** the external dashboard into a project file. They never pass through the overlay or the agent. |
| `steps` | array of step objects | yes, non-empty | Ordered steps, shown one at a time. |
| `verify` | string | no | The check the agent performs itself after "done". If absent, the user's confirmation stands. |
| `lang` | string | no | Language of the texts. Used only as a hint for OCR and runbooks; steps are shown as written. |

- **SPEC-02** A step is an object `{ text, url?, values?, warning? }`. `text` is required; `url` is a per-step destination used only when that step moves elsewhere; `values` is a list of keys into the top-level `values`; `warning` is a short caution shown prominently (e.g. irreversible action).
- **SPEC-03** Steps MUST be objects from version 1. Strings with placeholders are not accepted.
- **SPEC-04** The step list is static: the agent produces it at open time and does not recompute it on the fly. Changes during a handoff happen through replacement steps on the same handoff (§5.1), never through a new handoff for the same goal.
- **SPEC-05** `verify` MUST NOT read the values listed in `secrets`. It may check only their presence or their effect (the variable exists; a test event arrives with a valid signature because the project code reads the secret itself). This is a format rule and is repeated in the tool description. It is a norm, not an enforcement: if an agent reads the file anyway, that is the agent's behaviour.

### 4.2 Field exclusions (deliberate)

- **SPEC-06** The following MUST NOT be added to version 1, because every extra field is a public promise: estimated time (invented estimate), priority (one user), reference images (they age, weigh, become a screenshot library), a documentation-link field (agents invent URLs; an `https` URL in step text is clickable anyway), the originating runbook id (internal to the app), `on_failure` (the agent decides), per-step `expected_result` (belongs in the text), `category` (the app infers it).

### 4.3 URLs

- **SPEC-07** Allowed URL schemes, in `url` fields and for auto-linking inside step text, are a closed list: `http`, `https`, `ms-settings:`, `x-apple.systempreferences:`. Anything else is displayed as plain text, not clickable.
- **SPEC-08** The list MUST NOT be widened at runtime or via confirmation dialogs ("ok" habits). It changes only with a format version.

### 4.4 Validation and versioning

- **SPEC-09** The server MUST publish a JSON Schema for the spec and validate every incoming spec against it.
- **SPEC-10** A malformed spec MUST be rejected with a readable error that names the field and the correction. The app never receives a broken spec.
- **SPEC-11** The server accepts its own `spec_version` and all previous ones. A higher version is rejected with an "update the server" error.
- **SPEC-12** The validator MUST reject any spec whose step text contains a residual `{{…}}` placeholder. Placeholders exist only in the runbook format (§10.4).
- **SPEC-13** The certain-secret detector (§9.3) runs in the server on incoming `values` and step texts. A match does not reject the spec (the agent would loop) and does not pass through: the value is treated as a secret (§9.4).

### 4.5 Example

```json
{
  "spec_version": 1,
  "goal": "Register the Stripe webhook for payment events",
  "where": "Stripe Dashboard → Developers → Webhooks",
  "url": "https://dashboard.stripe.com/webhooks",
  "why_human": "Requires access to the production Stripe account.",
  "values": {
    "endpoint_url": "https://api.myapp.example/webhooks/stripe",
    "events": ["checkout.session.completed", "invoice.paid"]
  },
  "secrets": {
    "STRIPE_WEBHOOK_SECRET": ".env"
  },
  "steps": [
    { "text": "Click Add endpoint and paste the endpoint URL.", "values": ["endpoint_url"] },
    { "text": "Select the events checkout.session.completed and invoice.paid.", "values": ["events"] },
    { "text": "Save and copy the signing secret." },
    { "text": "Paste it into .env as STRIPE_WEBHOOK_SECRET." }
  ],
  "verify": "Check that STRIPE_WEBHOOK_SECRET exists in .env without reading its value, then send a test event from the dashboard and verify it reaches /webhooks/stripe with a valid signature.",
  "lang": "en"
}
```

---

## 5. Tool contract

Tool and package names are final and concept-bound, not brand-bound: `handoff_to_user`, `handoff_verify`, `handoff_runbooks`, executable and repository `handoff-mcp`, field `spec_version`. The npm package is published as `baton-handoff-mcp` (`handoff-mcp` was already taken on the registry; owner decision T-001 D1, 2026-09-07). The product name (Baton) may change without touching the format.

> **Naming note.** `DESIGN-TREE.md` uses Italian working names for several result and parameter fields (e.g. `stato`, `passo_corrente`, `testo_utente`, `passi_saltati`, `già_consegnato`, `ignora_runbook`, `passi_sostitutivi`, `risposta`, `in_corso`). The English names used in this document are **final**: `status`, `current_step`, `user_text`, `skipped_steps`, `already_delivered`, `ignore_runbook`, `replacement_steps`, `reply`, `in_progress`.
>
> The tool descriptions, the validation error messages and every text the server returns to an agent are in **English only**, regardless of the spec's `lang`: they are read by agents, not by users.

### 5.1 `handoff_to_user` — open, continue, resume

- **TOOL-01** `handoff_to_user` is a single **blocking** tool that accepts one of three input shapes:
  1. **Open:** a new spec, optionally with `request_id` (links to a user-opened request, §7.2) and `ignore_runbook` (skips the runbook match, §10.5).
  2. **Continue:** `{ handoff_id, reply, replacement_steps? }` — the agent's answer to a question or screenshot on the current step; `replacement_steps` replace the remaining steps.
  3. **Resume:** `{ resume: handoff_id }` — re-attach to an existing handoff after an interrupted call, from any session of the same installation.
- **TOOL-02** Control fields (`handoff_id`, `resume`, `request_id`, `reply`, `replacement_steps`, `ignore_runbook`) live in the tool contract, **outside** the spec.
- **TOOL-03** The call returns only at the **end of the handoff** or at the **first event that needs the agent**: a question ("Ask"), a screenshot, a deferral, an abandonment. Intermediate "step done" confirmations, notes and skips are handled by the app and delivered together in the final outcome.
- **TOOL-04** Continuing a handoff keeps the same `handoff_id`, the same tab, the same step state and counter. The agent's reply appears on the current step. A new handoff exists only for a different goal.
- **TOOL-05** The call MUST stay open for as long as the user needs, subject to the heartbeat rule below.
- **TOOL-06 (heartbeat)** Shortly before the agent's tool timeout, the server returns `status: in_progress` with `handoff_id` and an explicit instruction to call `handoff_to_user` with `resume`. This applies to every agent, including Claude Code; the raised timeout (§11.1) only lowers the frequency to one call per 30 minutes, and only for long handoffs. One call every N minutes is acceptable; one per click is not.
- **TOOL-06a (heartbeat margin)** The heartbeat fires **60 seconds before** the timeout recorded for that client in the capability table. For clients not in the table, the table's `unknown` row applies: heartbeat after **50 seconds**, because the default timeout of MCP clients we do not know is often 60 seconds. The value rises as soon as the agent enters the table with its real timeout.
- **TOOL-07 (idempotent resume)** Resuming a concluded handoff returns the identical final outcome with `already_delivered: true`.
- **TOOL-08** Resume is allowed from **any session of the same installation**, not only the same session or folder: the handoff belongs to the user. On re-attach the tab shows "resumed from: agent · project". If the original call is still open, it receives the outcome `transferred_to_other_session`.
- **TOOL-09** The tool description MUST instruct the agent to: call `handoff_runbooks` before writing a spec; never read values listed in `secrets`; call `resume` after reconnecting the server if a call errored; cite still-pending handoffs in its final summary when told to.

### 5.2 `handoff_verify` — report a verification

- **TOOL-10** `handoff_verify` accepts `{ handoff_id, verify: { ok, detail } }` where `ok` is `true`, `false` or `null`. `null` means "I could not verify, because …" and `detail` says why. What was executed goes in `detail`.
- **TOOL-11** A handoff moves to **verified** or **failed** only through this call (§8). If `verify` was present in the spec and no report arrives, the handoff ends as **not verified**.

### 5.3 Outcome format

- **TOOL-12** The outcome is part of the public format, with its own schema. It is what any agent receives and what the log and the runbooks preserve.
- **TOOL-13** The outcome MUST include at least: `status` (§8.1), `handoff_id`, `current_step` (index and text), `user_text` (the question or note that caused the return, if any), optional `screenshot` (image or extracted text, per the user's choice and the session's capabilities), `skipped_steps`, per-step notes, the list of values that were treated as secrets at ingress (§9.4), a flag/notice when the app was not running (text mode), `already_delivered` when applicable, and the runbook match when one is returned instead of opening (§10.5).
- **TOOL-14** A deferred outcome MUST include the `handoff_id` and an explicit instruction to call `resume` before the agent concludes its turn (§7.5).

### 5.4 `handoff_runbooks` — search

- **TOOL-15** `handoff_runbooks(where, goal)` returns matching runbooks with their last-verified date and a draft spec derived from each (§10.5).

---

## 6. MCP server

### 6.1 Responsibilities

- **SRV-01** The server implements: spec validation (§4.4), the capability table (§12.2), heartbeat (TOOL-06), resume (TOOL-07/08), text mode (§6.5), the certain-secret detector with **public** patterns (§9.3), runbook read and search (§10.5), and the hook subcommand (§6.4).
- **SRV-02** The server MUST NOT launch the app. An agent tool that starts processes is a trust problem.
- **SRV-03** The server MUST NOT be restarted by the app: it is a child of the agent, not of the app.

### 6.2 Transport to the app

- **SRV-04** Server and app communicate over a local socket: a Unix domain socket on macOS, a named pipe on Windows. No TCP port, no access from other hosts, OS permissions for free.
- **SRV-05** The **app listens**; servers connect. There is no separate daemon: the app is the single long-lived process.
- **SRV-06** The socket protocol is internal and may change without notice (ARCH table).

### 6.3 Channel security

- **SRV-07** Authentication uses a **per-installation token** stored in a file with user-only permissions, generated by the installer and read by both app and server. Per-session tokens via environment variables are not used (they add nothing and leak into config files).
- **SRV-08 (threat model, declared)** The token protects against other users of the same machine and against accidental connections. It does not protect against a malicious process already running as the same user; that is the operating system's boundary.
- **SRV-09** The app does not verify server identity beyond the token: the server is open by design, and the user controls what is registered in their agent.

### 6.4 Hook subcommand

- **SRV-10** The Stop and SubagentStop hooks run the open subcommand `handoff-mcp hook stop`.
- **SRV-11** If the app does not answer within about two seconds, the hook MUST exit neutrally (no block). The hook never blocks on uncertainty, and exits immediately with a neutral result if the app is not running.
- **SRV-11a** The hook connects to the app socket at every end of turn of every session with the server loaded. No marker file or other pre-check guard: a local connection costs milliseconds and the hook exits neutrally when there is nothing queued.
- **SRV-12** The hook blocks the agent's stop **at most once per handoff per session** (counter kept in the app, plus the agent's `stop_hook_active` guard) when there is a deferred handoff or an unreported verification (§7.5, §8.3). The block reason is written as an instruction the agent can act on.
- **SRV-13** The hook also delivers the queue of user-opened requests at the end of a turn (§7.2).

### 6.5 Text mode (app not running)

- **SRV-14** If the app socket is not reachable, the tool returns the validated spec **as text** in the result, with the instruction to present it to the user in chat and collect the answer there. The result states that the app is not active.
- **SRV-15** In text mode there is no log and no "verified" state: the agent composes the outcome from the chat. The server documentation MUST state this.
- **SRV-16** Text mode is also what remote agents get (Claude Code web, Codex cloud, Copilot on GitHub): the socket is not found and the server degrades with no extra code. The documentation says "works in text mode, not supported".

### 6.6 Session identity

- **SRV-17** The session key is the **PID of the agent process** (the server's parent). The hook, spawned by the same agent process, walks its own ancestor chain and sends the list; the app intersects it with registered PIDs and from then on binds the agent `session_id` to the registration.
- **SRV-18** The working directory is only a fallback key. If still ambiguous, the app asks the user which tab.
- **SRV-19** On Windows the fixed-path launcher MUST be a native executable, not a `.cmd`, otherwise the server's parent is `cmd.exe`. **In addition**, the server always registers its full ancestor chain, on every platform: it costs nothing and is already required for Cursor and Copilot, where the parent process is the editor rather than the agent.
- **SRV-20** Each session with the server loaded registers with the app **at session start**, not at the first tool call.

### 6.7 Failure modes

- **SRV-21 (server dead, session alive)** The app marks the tab "server disconnected" and keeps the handoff. The agent gets an error on its next call; the tool description tells it to call `resume` after reconnecting the server (e.g. `/mcp` or a new session). Re-attachment happens on a new connection with the same session key or on a resume from another session.
- **SRV-22 (client-side cancellation, e.g. Ctrl+C)** The overlay is **never closed by an agent-side event**. The tab shows a banner "session detached, the outcome will be delivered on the next resume", steps stay navigable, "done" keeps working and the outcome is preserved. The banner disappears when the session re-registers.
- **SRV-23 (orphan outcome)** An outcome is kept until consumed or closed manually by the user. After 7 days it is shown as "orphan" in the overlay's list; the log keeps it regardless (data only). From the list the user can view it, close it by hand, or copy its `handoff_id` to resume it in a new session. The app never closes an orphan on its own.

### 6.8 Distribution

- **SRV-24** The server is written in TypeScript (most mature MCP SDK) and shipped as a standalone executable: no Node.js required on the user's machine.
- **SRV-25** The app bundles a copy of the server behind a **fixed-path launcher**; the installer writes that path into the agent configuration, so updates come with the app and the agent config never changes.
- **SRV-26** An npm package exists for users of the server alone.

---

## 7. Handoff lifecycle in the overlay

### 7.1 Opening by the agent

- **OPEN-01** When the agent calls `handoff_to_user` with a valid spec and the app is reachable, the overlay shows the handoff with the spec ready.
- **OPEN-02** The overlay always shows which session the handoff belongs to: agent name and project folder.

### 7.2 Opening by the user

- **OPEN-03** A global shortcut opens a small request window. Defaults: `⌃⌥H` on macOS, `Ctrl+Alt+H` on Windows. If the shortcut is taken, the app detects it at startup and asks once to choose another; it never steals it silently.
- **OPEN-04** The request window has a session selector on top (pre-selected when only one session is active) and a field "What are you about to do?". Enter sends, Esc cancels. The tab appears immediately in state "waiting for spec".
- **OPEN-04a** If no session is registered, the request window still opens and shows the notice "no active session". The request is queued and delivered to the first session that registers, through the same paths as OPEN-05 and OPEN-06.
- **OPEN-05 (delivery, launch version)** The request is always queued in the app **and** placed on the clipboard as a short text in the user's language, with the request id, e.g.:

  ```
  [Handoff hf_123] The user opened a request: "I'm about to create the API key on Stripe". Produce the spec and call handoff_to_user with request_id=hf_123.
  ```

  The app brings the agent's terminal window to the front on a best-effort basis; the user presses Enter. If the window cannot be found, the app notifies "request copied: paste it into session X".
- **OPEN-06 (delivery, safety net)** The queue is also delivered at the end of the agent's turn through the Stop hook, for when the agent is busy. The clipboard is the fast path, not the only one.
- **OPEN-07** Rejected delivery methods: writing directly into the terminal (fragile); a `CLAUDE.md` rule with an "are there handoffs?" polling tool each turn.
- **OPEN-08** If the agent omits `request_id`, the app links the first new handoff of that session to the oldest open request of that session. Two open user requests in the same session may mismatch; accepted as a rare case.
- **OPEN-09** In both opening paths, the spec is produced by the agent with the project context, never by the app.

### 7.3 Multiple sessions and handoffs

- **MULTI-01** There is exactly **one overlay window**, with one **tab per handoff** (agent + project). The user switches freely between tabs; switching returns no tool call.
- **MULTI-02** Each tool call is bound to its `handoff_id`; tab states are independent.
- **MULTI-03** A handoff arriving while another is active puts a badge on its tab and does not steal focus. Only the first opening brings the overlay to the front.
- **MULTI-04** Screenshot and action buttons act on the active tab. Never two windows.

### 7.4 Guidance

- **GUIDE-01** The overlay shows **one step at a time**, with the counter ("2 of 4") and a button to advance.
- **GUIDE-02** Every value referenced by the step is copyable with one click. Array values are copyable as a whole and per item.
- **GUIDE-03** If the spec or the step has a `url`, it opens with one click (allowed schemes only, SPEC-07). `https` URLs inside step text are auto-linked with the same list.
- **GUIDE-04** A step `warning` is shown prominently on that step.
- **GUIDE-05** There is no percentage progress bar and no estimated time.
- **GUIDE-06** Steps are shown in the language the agent wrote them; the UI language is separate (§11.3).

### 7.5 User responses

- **RESP-01** From a step the user can: **Confirm** (done, go to next), **Note** (annotate the step), **Ask** (question or error description to the agent), **Defer**, **Skip**, **Abandon**, and **Screenshot** (only when needed).
- **RESP-02** "Note" and "Ask" are **distinct buttons**. Note annotates locally; Ask interrupts.
- **RESP-03** **Local, non-interrupting** actions: Confirm, Note, Skip. The app records them and reports them all together in the final outcome (`skipped_steps`, notes per step).
- **RESP-04** **Interrupting** actions (the tool call returns): Ask, Screenshot (it exists to be seen), Defer, Abandon.
- **RESP-05 (defer)** The deferred outcome includes the id and an explicit instruction to call `resume` before concluding. The agent parks the step, proceeds with work that does not depend on it, and re-proposes it at the end. The app keeps the queue.
- **RESP-06 (defer, safety net)** If a handoff is still deferred when the agent tries to stop, the Stop hook blocks once with the reason (SRV-12).
- **RESP-07 (second deferral)** On a second deferral the agent stops; the handoff stays in the app's queue with a visible badge; the tool result tells the agent to cite it in its final summary. The user resumes it from the overlay whenever they want.
- **RESP-08** "Abandon" is always available next to "Defer". Abandon closes the handoff without a positive outcome and the agent is informed.
- **RESP-09** "Done" on the last step moves the tab to **verifying** (§8) or, when `verify` is absent, to **confirmed by user**.

---

## 8. Verification and final states

### 8.1 States

- **VER-01** Every concluded handoff has exactly one final state:

| State | Meaning |
|---|---|
| **verified** | The agent reported a positive result via `handoff_verify` (`ok: true`). |
| **failed** | The agent reported a negative result (`ok: false`). |
| **confirmed by user** | `verify` is absent by choice, because the step cannot be verified by the agent (a plan change, a consent given in a dashboard). |
| **not verified** | `verify` is present, but the agent never reported the result, or reported `ok: null`. |
| **abandoned** | The user abandoned the handoff. |

- **VER-02** A handoff is **never** marked verified on trust.
- **VER-03** A handoff with `ok: null` ("could not verify because …") is recorded with the detail; it is better than an invented result.

### 8.2 What the user sees

- **VER-04** On "done", the tab moves to **verifying** and displays the `verify` text, so the user knows what the agent should be checking.
- **VER-05** When the report arrives, the tab shows state and detail, labelled **"declared by agent"**. It is a declaration, not a proof, and the log preserves it as such.

### 8.3 Missing report

- **VER-06** "verifying" becomes **not verified** when the session's server disconnects or after **30 minutes** without a report, the same value as the raised tool timeout (INST-03): after that, the agent's call has expired anyway.
- **VER-07** The Stop hook blocks once for unreported verifications as well (SRV-12): they are the main path through which "verified on trust" would sneak back in.

### 8.4 Failed verification

- **VER-08** On a failed verification the agent re-opens the **same handoff** (same `handoff_id`, same tab) with replacement steps that start from the actual error, not a generic repeat of the instruction.
- **VER-09** The tab shows the round counter ("correction 1 of 2") and keeps the history visible but collapsed.
- **VER-10** In the log this is one handoff with several rounds, linked by the id.

---

## 9. Screenshot, OCR, redaction, preview

### 9.1 Capture

- **CAP-01** Clicking the screenshot button never starts a capture by itself. Two options appear: **Full screen** and **Select region**; the user chooses every time. The last choice is highlighted but does not fire.
- **CAP-02** Full screen captures the monitor where the cursor is. Region selection spans all monitors.
- **CAP-03** The overlay always hides itself during capture.
- **CAP-04** On macOS, the screen-recording permission is requested during first-launch onboarding, with an explanation, never mid-handoff (it requires an app restart).
- **CAP-05** OCR runs on the original at full resolution. The image sent to the agent is downscaled to about 1600 px on the long side, PNG (JPEG ruins small text).
- **CAP-06** Redaction boxes are computed in original-image coordinates and rescaled to the reduced image; the redaction is burned in **after** resizing so that no halo leaks glyphs.

### 9.2 OCR

- **OCR-01** OCR always runs on every capture, in both image and text mode, because secret detection depends on it.
- **OCR-02** OCR is local, using the OS engine: Vision on macOS, `Windows.Media.Ocr` on Windows.
- **OCR-03** A bundled engine, `ocrs`, is the fallback (e.g. missing Windows language pack), always local and offline. Only the English models ship, unmodified and with their CC-BY-SA 4.0 attribution; other languages are a future decision. The engine is pure Rust, so it is also the basis for future Linux support. Tesseract as a separate signed helper is the recorded alternative, taken only if the bundled engine's measured quality is not enough for a release.
- **OCR-04** OCR cost is absorbed asynchronously: the preview appears immediately with an "analyzing" state and the send buttons enable when detection finishes.
- **OCR-05** The spec's `lang` is a hint to the OCR engine.

### 9.3 Secret detection

- **DET-01** Two levels, applied to screenshots (via OCR) and to typed text:
  - **certain** (known patterns: key prefixes, tokens, private-key blocks): redacted automatically and shown redacted in the preview;
  - **suspected** (long random strings; fields labelled key, secret, token, password): highlighted with the warning "may contain a secret"; the user decides.
- **DET-02** The **certain** detector lives in the server, with **public** patterns. The **suspected** detector lives in the app.
- **DET-03** Only values that passed the certain check at ingress (§9.4) are exempt from suspected-level redaction in screenshots.

### 9.4 Spec values that look like secrets

- **DET-04** When the certain detector matches a value or a step text at ingress: the spec is **not rejected** and the value does **not** pass through. The overlay shows it masked with a local "show"; the copy button copies the true value; log and runbook keep a placeholder; the outcome warns the agent that the value was treated as a secret.

### 9.5 Mandatory preview

- **PREV-01** Everything sent to the agent passes through a preview. There is no "send without preview".
- **PREV-02** In the preview the user can: unlock a redacted box (a false positive costs one click, not lost information), add a box by hand, crop.
- **PREV-03** In text mode the extracted text is editable: the user sends what the user sends.
- **PREV-04** Two send buttons, **Send image** and **Send text**, both visible, no default. If the session does not support images (capability table), only **Send text** is shown.
- **PREV-05** Text-only mode keeps the image on the machine; the agent receives only the locally extracted text. Intended for users who cannot send images, models without vision, or to save calls.

### 9.6 Context travels with the screenshot

- **CTX-01** Every screenshot or text is delivered to the agent **together with the handoff context** (task, current step, expected result, relevant values), so the agent interprets what it sees knowing what the user is trying to achieve.

### 9.7 Secrets from the dashboard

- **SEC-01** The overlay never receives secret values. The spec says where to paste them; the user does it in the indicated file.
- **SEC-02 (v1)** For each entry in `secrets`, the overlay shows the variable name and destination file and offers **Open file**, which opens the file with the operating system's default application for it.
- **SEC-03 (v2, planned)** One click takes the clipboard and writes the line into the file indicated by `secrets`, showing only the path and variable name, never the value. Explicit opt-in, limited to environment-variable files. This gives the app file-writing capability; the trust story must be built first.

---

## 10. Runbooks

- **RUN-01** A runbook is created **automatically** after every **verified** handoff, and after every **confirmed by user** handoff with a lower trust label. Never from **not verified** or **failed** handoffs. The user can delete any runbook.
- **RUN-02** Content is the sequence actually executed: original steps minus skipped plus replacement steps, in real order, with notes and errors as annotations on the step where they emerged. A recipe, not a diary: the diary is the log.
- **RUN-03** Runbooks are **files in a user folder**, in a **public format** like spec and outcome, so agents can read them without the app, they survive uninstallation and the user can see them. Storing them in a project folder is not in scope now.
- **RUN-03a (location)** The folder is `~/.handoff/runbooks/` on every platform (`%USERPROFILE%\.handoff\runbooks\` on Windows). The path is cited in the tool description.
- **RUN-03b (format)** Runbook files are **JSON**, in the same style as the spec, one file per runbook, with a published JSON Schema. Conversion from runbook to draft spec is therefore a field mapping.
- **RUN-04** Runbooks store **only value names**, never real values. Each name carries a short description: the sentence of the step where the value appears, with the placeholder in place of the value ("paste {{endpoint_url}} into the Endpoint URL field"). If a value appears in no step, only the name remains. The agent fills values from the current project.
- **RUN-05** Placeholders `{{name}}` exist **only in the runbook format**. The app produces them by searching for the literal value in the step text (longest values first, to avoid partial substitutions). The server, when converting a runbook to a draft spec, moves them into `values` as names to fill, and the validator rejects any spec with a residual placeholder (SPEC-12).
- **RUN-06 (discovery, main path)** The tool description instructs the agent to call `handoff_runbooks(where, goal)` before writing a spec.
- **RUN-07 (discovery, safety net)** On validating a **new** spec, the server searches for related runbooks by reading the files. If it finds any, it returns them **without opening** the handoff, already converted into a draft spec with placeholders, with the message "a runbook exists: start from here?". The agent's second call passes `ignore_runbook` so the search is not repeated. The extra turn is paid only when the agent skipped the main path.
- **RUN-07a (matching rule)** A runbook matches when its `where` equals the spec's `where` after normalisation (case, whitespace, arrow and separator characters) **and** the two `goal` texts share words beyond stop-words. The rule is deterministic and explainable; no fuzzy similarity, no model. `handoff_runbooks(where, goal)` uses the same rule.
- **RUN-08** Every runbook carries the date of its last verified execution, returned to the agent so it can weigh freshness.
- **RUN-09** If a handoff born from a runbook fails and is then corrected successfully, the app proposes to update the runbook with the corrected sequence: one click, the user decides. If it fails without correction, the runbook is marked "last run failed" with the date, never deleted automatically.
- **RUN-10** Reading and searching runbooks is in the server (open). Creating, updating and redacting them is in the app (closed).
- **RUN-11** No export, import or public library. The public file format makes a runbook copyable as a file, and that is enough.
- **RUN-12** The spec's `lang` is a hint for the runbook.

---

## 11. Overlay application

### 11.1 Installation and agent registration

- **INST-01** On first launch the app scans the machine for supported agents and, with the user's consent, registers itself in their configurations, **showing each exact modification first** (file, lines) with a "show" control. The user never configures MCP by hand.
- **INST-02** For Claude Code the consent screen lists three modifications: the MCP server entry (which carries the per-server tool timeout of INST-03), the Stop hook and the SubagentStop hook (the two hooks presented as one line "two hooks (Stop and SubagentStop), same command"). *Amended 2026-09-08 (T-026, OI-02): the former fourth modification, the global tool-timeout variable, is no longer written.*
- **INST-03** The tool timeout of our server is raised to a contained value (**30 minutes**, not an hour) through the per-server `timeout` field of the MCP entry, which affects only our server, needs no separate consent, and disappears with the entry on uninstall. The global variable `MCP_TOOL_TIMEOUT`, which applies to all MCP servers of Claude Code, is **not** written by the installer, and an existing value is never touched, on install or on uninstall. *Amended 2026-09-08 (T-026, OI-02): until then this requirement wrote the global variable, with the consent sentence "applies to all MCP servers of Claude Code" and restoration on uninstall.*
- **INST-04** Existing user configuration is never replaced. An existing Stop hook is kept and ours is added alongside. On uninstall, only our lines are removed.
- **INST-05** The scan runs at every launch (it is cheap), with a discreet notice **once per newly found agent**, plus a "find agents" action in settings.
- **INST-06** Registration scope defaults to user. Project scope is offered in settings, not in onboarding.
- **INST-07** The installer generates the per-installation token file (SRV-07) and writes the fixed launcher path into the agent configuration (SRV-25).
- **INST-08** The installation adapter (config files, variables, hook registration) is in the app, keyed by the same agent id as the protocol adapter in the server (§12.2).

### 11.2 Window behaviour

- **WIN-01** Always on top (non-negotiable).
- **WIN-02** A narrow panel with fixed width and content-driven height, draggable, position remembered per monitor.
- **WIN-03** When the user clicks elsewhere, the panel collapses to a bar showing the current step and three buttons: **Done**, **Ask**, **Screenshot**. It re-expands on click. The other actions (Note, Skip, Defer, Abandon) are available only in the expanded panel.
- **WIN-04** Closing the window hides it to the system tray / menu bar; quitting is only from the tray icon menu.
- **WIN-05** The tray icon is always present; a badge appears only when there are active handoffs.
- **WIN-06** At rest the app is an icon and a listening socket.

### 11.3 Startup and language

- **APP-01** Autostart is on by default, proposed in the consent screen with a pre-checked box and the sentence "stays in the background doing nothing until an agent asks for a handoff". It can be disabled in settings. Off means text mode for the first handoff of every day.
- **APP-02** UI language: English by default, Italian available (non-negotiable). The UI follows the system language if it is one of the two, otherwise English; changeable in settings.

### 11.4 Updates

- **UPD-01** No automatic updates. On launch the app checks for a new version with a **single request to a fixed address** that sends **only the version number**. The check can be disabled in settings.
- **UPD-02** If a new version exists, the app shows a notice with the changes, a button that opens the download page, and "remind me later". The user downloads and reinstalls; the agent configuration does not change because it points to the same path.

### 11.5 Local log

- **LOG-01** The log is a local SQLite database.
- **LOG-02** Per handoff: the spec with placeholders in place of secret-treated values, the outcome, timestamps, session (agent, folder), verification state, rounds.
- **LOG-03** Per send: the full text exactly as it left, after redaction; for images only dimensions, hash and redacted boxes. **Never pixels.**
- **LOG-04** Deletion of a single entry and of everything. Export to JSON: it is the user's data.
- **LOG-05** No automatic retention limit: entries are kept until the user deletes them.

### 11.6 Network transparency

- **NET-01** A **Network** page in settings lists every outbound connection since launch (date, domain, bytes sent), fed by the **single point in the code** through which the app can reach the network.
- **NET-02** The documentation describes a reproducible test with the system firewall or a tool such as Little Snitch that shows exactly one domain, that of the version check. The page is a self-declaration; the firewall test is the verification. Both are required.

### 11.7 Telemetry and crashes

- **TEL-01** No telemetry, ever.
- **TEL-02** No crash reports are sent, not even opt-in, in v1. Crash reports are written locally; the user can open the folder and send one by hand.

### 11.8 Licensing hooks

- **LIC-01** No license verification in v1: the app starts and works in full.
- **LIC-02** The code MUST keep an isolated entry point where a local license check can be added later without touching the rest.

---

## 12. Agent adapters and support levels

### 12.1 Support levels

- **ADPT-01** Each agent has a declared support level:
  - **full**: long wait (configurable timeout), images in tool results, end-of-turn hook;
  - **base**: heartbeat `in_progress` plus instructions in the tool result only;
  - **unsupported**: only an agent that cannot make a blocking call with our result.

### 12.2 What an adapter is

- **ADPT-02** A **static capability table** keyed by client name and version from the MCP handshake (timeout, images, hooks, heartbeat), plus a piece of code per agent only where needed (session identity inside editors).
- **ADPT-03** The table is read by the **server**, because it concerns protocol and agent facts that must hold for users of the server without the app. The app asks the server "what does this session support?" and adapts only the UI (e.g. hides "Send image").
- **ADPT-04** An adapter is responsible for: registering the server in the agent's configuration at install time (app side); recognising session and project to show them in the overlay; switching to text-only automatically if the agent does not accept images from tools; handling the long wait in the way that agent allows; delivering user-opened requests in the way that agent supports. The rest of the system does not notice the difference.
- **ADPT-05** An "empty" adapter is acceptable and equals **base** support.

### 12.3 Roadmap (commitment, not example)

- **ADPT-06** At launch: Claude Code only. Then, in this order, each entering the capability table and the automated test as it ships:
  1. **Codex** — a CLI, so the parent-PID key works unchanged; speaks MCP; configurable timeout; no Stop hook, which forces early validation of the degraded path (instructions in result, heartbeat).
  2. **Cursor** — solves session identity inside an editor; the solution then covers Copilot.
  3. **GitHub Copilot**.
  4. **OpenCode** — the easy case.

### 12.4 Non-local agents and sub-agents

- **ADPT-07** Remote agents are out of scope but need no extra code: the server finds no socket and degrades to text mode (SRV-16).
- **ADPT-08** `SubagentStop` is registered with the same command as `Stop`; the app treats both alike (same session key, same tab). Without it, a handoff deferred by a sub-agent would have no safety net.

---

## 13. Non-functional requirements

### 13.1 Privacy and security

- **NFR-01** The app communicates only with the local MCP server over the local socket. The only network call is the update check (UPD-01).
- **NFR-02** The app never calls a model and never needs the user's API keys; screenshot interpretation is done by the agent with its own model.
- **NFR-03** No process observes the screen between handoffs. Capture is explicit, user-initiated, every time.
- **NFR-04** Nothing leaves the machine without redaction and preview (PREV-01).
- **NFR-05** Trust in the app rests on three user-verifiable facts, none of which requires reading its source: local-only communication (Network page + firewall test), mandatory preview, and the local log of everything sent.

### 13.2 Technology

- **NFR-06** App: **Tauri**, with native pieces in Rust (region capture, system OCR bindings via Vision and the `windows` crate, socket/named pipe, global shortcut plugin, signing/notarization pipeline). Electron rejected (150 MB of Chromium is the wrong signal for an app that asks for trust); two native apps rejected (double code and bugs for a simple panel).
- **NFR-07** Server: TypeScript, standalone executable (SRV-24).

### 13.3 Code signing and distribution

- **NFR-08** macOS builds MUST be signed and notarized at launch (Apple Developer Program); an unsigned Mac app is effectively unusable.
- **NFR-09** Windows MAY ship unsigned at launch with a clear instruction for the SmartScreen warning, and be signed later with a code-signing certificate.

### 13.4 Reliability and degradation

- **NFR-10** Every agent-dependent behaviour sits in the adapter behind the capability table; the worst case is degraded, never broken (PRIN-10).
- **NFR-11** The Stop hook MUST return within about two seconds and exit neutrally on any doubt (SRV-11).
- **NFR-12** Handoff state lives in the app and survives interrupted tool calls, detached sessions and dead servers (SRV-21–23).

### 13.5 Performance

- **NFR-13** The preview appears immediately after capture; OCR and detection run asynchronously (OCR-04).
- **NFR-14** At rest the app consumes negligible resources (an icon and a listening socket).

### 13.6 Testing and documentation

- **NFR-15** An automated test MUST run a complete handoff in a non-interactive Claude Code session for every Claude Code release, and later for each supported agent.
- **NFR-16** Published documentation MUST include: the JSON Schema for spec and outcome, the tool contract, the runbook format, the text-mode limitations (SRV-15), the channel threat model (SRV-08), the firewall test (NET-02), and the "internal, subject to change" notice on the socket protocol.

### 13.7 Licensing

- **NFR-17** MIT for everything: the formats, the server and the app. *Amended 2026-09-17: the app had a proprietary binary licence until then.*

---

## 14. Deferred and open items

| Item | Status | Reference |
|---|---|---|
| License verification | Not in v1; isolated entry point only | 7.2 |
| Publication of the socket protocol | Reconsidered after format v1 is stable | 7.4 |
| One-click write of secrets into env files | v2, opt-in | 2.4 |
| Runbooks in the project folder | Not now | 6.2 |
| Linux support | Future, if viable (bundled `ocrs` path) | Product vision |
| Windows code signing | May follow launch | Product vision |
| Two open user requests in the same session without `request_id` | Accepted as rare mismatch | 1.5.b |

---

## 15. Decisions taken on this document (2026-09-07)

Owner decisions that closed the points left open in draft 0.1. Each is now a requirement above; the question, the alternatives and the reasoning are recorded in `DESIGN-TREE.md` branch 9 (9.1–9.15, same numbering as this table).

| # | Point | Decision | Requirement |
|---|---|---|---|
| 1 | Timeout of "verifying" | 30 minutes, aligned with the raised tool timeout | VER-06 |
| 2 | Runbook folder | `~/.handoff/runbooks/` on every platform | RUN-03a |
| 3 | Runbook file format | JSON, same style as the spec, with JSON Schema | RUN-03b |
| 4 | English field names | Adopted as final | §5 naming note |
| 5 | Collapsed-bar buttons | Done, Ask, Screenshot | WIN-03 |
| 6 | Runbook matching rule | Normalised `where` equality plus word overlap on `goal` | RUN-07a |
| 7 | Shortcut with no session | Window opens, request queued for the first session that registers | OPEN-04a |
| 8 | Windows launcher | Native executable **and** full ancestor chain always registered (needed anyway for Cursor and Copilot) | SRV-19 |
| 9 | Array values | Copyable whole and per item | GUIDE-02 |
| 10 | Heartbeat margin | 60 s before the known timeout; `unknown` row of the capability table at 50 s, raised when the agent enters the table | TOOL-06a |
| 11 | Log retention | Forever, manual deletion only | LOG-05 |
| 12 | "Open file" for secrets | OS default application | SEC-02 |
| 13 | Orphan outcomes | View, close by hand, or copy the id to resume; never auto-closed | SRV-23 |
| 14 | Language of tool texts | English only | §5 naming note |
| 15 | Hook connection guard | None; connect every turn, exit neutrally | SRV-11a |

**Added 2026-09-07 (T-001, owner decisions).** npm package name `baton-handoff-mcp`, executable and repository unchanged (§5); product name **Baton**, bundle identifier `com.cepeppe.baton`; update-check domain left open until the update check is reactivated (§11.4).

**Added 2026-09-08 (T-026, OI-02, owner decision).** The tool timeout is raised through the per-server `timeout` field of the MCP entry only; the global `MCP_TOOL_TIMEOUT` is no longer written, and the consent screen lists three modifications (INST-02, INST-03).

**Added 2026-09-17 (owner decision).** Both repositories are published under the MIT licence (§3.2, NFR-17).
