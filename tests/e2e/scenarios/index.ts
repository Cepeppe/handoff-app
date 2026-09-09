/**
 * The nine scenarios of §11.5 this task runs, in the order they are worth reading (T-043).
 *
 * The other two rows of the table are elsewhere and stay there:
 *
 * - **E2E-3** (screenshot with a fixture image) waits for the capture pipeline. The
 *   automation channel already takes the action name and answers "arrives with T-049", so
 *   the scenario is one file away the day the pipeline exists.
 * - **E2E-8** (the app stopped → text mode) is the server's, and it runs there:
 *   `handoff-mcp/test/canary/scenarios/e2e-08-text-mode.ts` (`TASKS.md` §0.4 item 3). Its
 *   second half — "no database row" — has no app to have a row in.
 */
import type { Scenario } from '../scenario.ts';
import { verifiedScenario } from './e2e-01-verified.ts';
import { questionScenario } from './e2e-02-question.ts';
import { deferScenario } from './e2e-04-defer.ts';
import { parkedScenario } from './e2e-05-parked.ts';
import { correctionScenario } from './e2e-06-correction.ts';
import { heartbeatScenario } from './e2e-07-heartbeat.ts';
import { requestScenario } from './e2e-09-request.ts';
import { notVerifiedScenario } from './e2e-10-not-verified.ts';
import { secondSessionScenario } from './e2e-11-second-session.ts';

/** Every scenario, in the order `pnpm e2e` runs them. */
export const SCENARIOS: readonly Scenario[] = [
  verifiedScenario,
  questionScenario,
  deferScenario,
  parkedScenario,
  correctionScenario,
  heartbeatScenario,
  requestScenario,
  notVerifiedScenario,
  secondSessionScenario,
];
