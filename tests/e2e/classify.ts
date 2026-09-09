/**
 * Protocol failure or model behaviour (T-043, TECHNICAL-DESIGN §11.5).
 *
 * §11.5 asks for "a classifier that separates protocol failures (a gate) from
 * model-behaviour failures (an alert)". The distinction is not about severity, it is a
 * question about *what was observed*, and it has one operational consequence: a model
 * failure is retried once, a protocol failure never is. Retrying a protocol failure buys a
 * second identical red, and a flaky retry can hide a real regression behind a lucky run.
 *
 * - **protocol** — the shape of the run is wrong: the app did not start, the server did not
 *   register, a row the app writes without the model's help is missing or says the wrong
 *   thing, a state the store reached is not the one §8.1 draws. Nothing a different sampling
 *   of the model would change.
 * - **model** — the run was well formed and the model did not do what the prompt asked: it
 *   never called the tool, called it with something the schema rejects, or ran out of turns.
 *
 * The same rule as `handoff-mcp/test/canary/classify.ts`, written again rather than imported:
 * nothing in `handoff-app` may reach into the other repository (§3.1 rule 3).
 *
 * A third kind exists here that the server's canary does not need: **pending**. An assertion
 * whose subject is a later task ships written but reports `pending` and does not decide the
 * verdict — the T-009 precedent. Deleting it would lose the assertion; failing on it would
 * make the suite red for work nobody has done yet. No scenario needs it at the moment: the
 * two that did were E2E-1's and E2E-6's runbook checks, and T-044's writer turned them into
 * ordinary ones.
 */

/** What a failing assertion looked at. */
export type FailureKind = 'protocol' | 'model';

/** One thing a scenario checked, and what it found. */
export interface Assertion {
  /** The scenario id this belongs to, `E2E-1`, `LOG`, `harness`, … */
  readonly id: string;
  /** What was checked, in one line, in the present tense. */
  readonly what: string;
  readonly ok: boolean;
  /** Which half of §11.5 a failure here belongs to. Meaningless when `ok`. */
  readonly kind: FailureKind;
  /** Reported, never decisive: an observation the design does not depend on. */
  readonly informational?: boolean;
  /** Waiting for a later task. Reported as `pending`, never decisive. */
  readonly pendingTask?: string;
  /** What was actually seen. Never a spec value. */
  readonly detail?: string;
}

/** The verdict over one run of one scenario. */
export type RunVerdict = 'passed' | 'protocol' | 'model';

/** Whether an assertion may decide the verdict. */
function decisive(assertion: Assertion): boolean {
  return (
    !assertion.ok && assertion.informational !== true && assertion.pendingTask === undefined
  );
}

/**
 * `passed` when nothing decisive failed, `protocol` when at least one protocol assertion
 * failed, `model` when the only failures were model ones.
 */
export function classify(assertions: readonly Assertion[]): RunVerdict {
  const failed = assertions.filter(decisive);
  if (failed.length === 0) return 'passed';
  return failed.some((assertion) => assertion.kind === 'protocol') ? 'protocol' : 'model';
}

/** Whether §11.5's single retry applies: model failures only, and only once. */
export function shouldRetry(verdict: RunVerdict, attempt: number): boolean {
  return verdict === 'model' && attempt === 1;
}

/** The failures of a run, most useful first: protocol before model. */
export function failures(assertions: readonly Assertion[]): Assertion[] {
  const failed = assertions.filter(decisive);
  return [
    ...failed.filter((assertion) => assertion.kind === 'protocol'),
    ...failed.filter((assertion) => assertion.kind === 'model'),
  ];
}

/** Every assertion that did not pass, including the ones that decide nothing. */
export function reported(assertions: readonly Assertion[]): Assertion[] {
  return assertions.filter((assertion) => !assertion.ok);
}

/** Builds a passing or failing assertion in one call, so a scenario reads as a list. */
export function check(
  id: string,
  what: string,
  kind: FailureKind,
  ok: boolean,
  detail?: string,
): Assertion {
  return { id, what, kind, ok, ...(detail === undefined ? {} : { detail }) };
}

/** The same, for an observation that is recorded but never decides a verdict. */
export function note(id: string, what: string, ok: boolean, detail?: string): Assertion {
  return {
    id,
    what,
    kind: 'protocol',
    ok,
    informational: true,
    ...(detail === undefined ? {} : { detail }),
  };
}

/**
 * An assertion whose subject is a later task: written now, reported `pending`, decisive
 * never.
 *
 * Unused while every scenario's subject exists (T-044 closed the last two). It stays because
 * the next scenario written ahead of its task needs it — E2E-3 waits for T-049 — and because
 * the report's own vocabulary is what makes "written but not yet decisive" sayable.
 */
export function pending(id: string, what: string, task: string, ok: boolean, detail?: string): Assertion {
  return {
    id,
    what,
    kind: 'protocol',
    ok,
    pendingTask: task,
    ...(detail === undefined ? {} : { detail }),
  };
}

/** How an assertion is labelled in the report. */
export function label(assertion: Assertion): string {
  if (assertion.pendingTask !== undefined) return `pending ${assertion.pendingTask}`;
  if (assertion.informational === true) return 'note';
  return assertion.kind;
}
