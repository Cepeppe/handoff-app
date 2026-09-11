/**
 * The ten scenarios of §11.5 this suite runs, in the order they are worth reading (T-043).
 *
 * The one row of the table that is elsewhere stays there: **E2E-8** (the app stopped → text
 * mode) is the server's and it runs there,
 * `handoff-mcp/test/canary/scenarios/e2e-08-text-mode.ts` (`TASKS.md` §0.4 item 3) — its
 * second half, "no database row", has no app to have a row in.
 *
 * **E2E-3** joined them with T-049: the capture pipeline it drives — OCR, both detectors,
 * the burn-in — did not exist when the harness was written, and the automation channel
 * answered "arrives with T-049" until it did.
 */
import type { Scenario } from '../scenario.ts';
import { verifiedScenario } from './e2e-01-verified.ts';
import { questionScenario } from './e2e-02-question.ts';
import { screenshotScenario } from './e2e-03-screenshot.ts';
import { deferScenario } from './e2e-04-defer.ts';
import { parkedScenario } from './e2e-05-parked.ts';
import { correctionScenario } from './e2e-06-correction.ts';
import { heartbeatScenario } from './e2e-07-heartbeat.ts';
import { requestScenario } from './e2e-09-request.ts';
import { requestByClipboardScenario } from './e2e-09-request-clipboard.ts';
import { notVerifiedScenario } from './e2e-10-not-verified.ts';
import { secondSessionScenario } from './e2e-11-second-session.ts';

/** Every scenario, in the order `pnpm e2e` runs them. */
export const SCENARIOS: readonly Scenario[] = [
  verifiedScenario,
  questionScenario,
  screenshotScenario,
  deferScenario,
  parkedScenario,
  correctionScenario,
  heartbeatScenario,
  requestScenario,
  notVerifiedScenario,
  secondSessionScenario,
];

/**
 * The Codex subset (T-067; §13 M7 asks each adapter for "an E2E subset"), in the order
 * `pnpm e2e -- --agent codex` runs it: E2E-1, 2, 4 and 7 exactly as Claude Code runs them, and
 * E2E-9 in the one shape an agent with no end-of-turn hook allows, by the clipboard.
 *
 * Four of them are the flows every agent goes through — verified, a question, a deferral and
 * its resume, the heartbeat — and the fifth is where the missing hook shows: the delivery of a
 * user's request. E2E-5 and E2E-10 are about what the Stop hook says, which a Codex session
 * never hears.
 */
export const CODEX_SCENARIOS: readonly Scenario[] = [
  verifiedScenario,
  questionScenario,
  deferScenario,
  heartbeatScenario,
  requestByClipboardScenario,
];

/**
 * The OpenCode subset (T-074): the Codex one, for the same reasons. OpenCode has no end-of-turn
 * hook either (its row says `stop_hook: false`), so the four flows every agent goes through run
 * as they are and E2E-9 runs by the clipboard; E2E-5 and E2E-10 are about what the Stop hook
 * says, which an OpenCode session never hears.
 */
export const OPENCODE_SCENARIOS: readonly Scenario[] = CODEX_SCENARIOS;
