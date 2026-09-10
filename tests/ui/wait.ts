/**
 * Waiting on a condition: the only way the UI suite waits (T-055).
 *
 * A WebDriver suite goes flaky in one way more often than in all the others put together: it
 * waits a fixed time for something that usually takes less, and one day it takes more. So
 * nothing here sleeps for a duration and then looks. Every wait names the condition it is
 * for, re-reads it until it holds, and gives up after a bound with that name and the last
 * error it saw — a timeout then says *what* never happened, which is the first thing a reader
 * of a red run needs.
 *
 * The re-read interval is the one timer of the suite, and it is not a wait for anything: a
 * condition that already holds is answered on the first read.
 */

/** How often a condition is re-read. */
export const POLL_MS = 100;

/** The bound a wait gets unless it asks for another. */
export const DEFAULT_WAIT_MS = 10_000;

/** What a probe may answer: a value to hand back, or one of the three ways of saying "not yet". */
type Answer<T> = T | null | undefined | false;

/**
 * Re-reads `probe` until it answers something other than `null`, `undefined` or `false`, and
 * answers that.
 *
 * A probe that throws counts as "not yet": the page is allowed to be between two states, and
 * an element replaced while it was being read is the ordinary case of a view that repaints.
 */
export async function until<T>(
  what: string,
  probe: () => Promise<Answer<T>> | Answer<T>,
  timeoutMs = DEFAULT_WAIT_MS,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  let lastError: unknown;
  for (;;) {
    try {
      const value = await probe();
      if (value !== null && value !== undefined && value !== false) {
        return value;
      }
      lastError = undefined;
    } catch (cause) {
      lastError = cause;
    }
    if (Date.now() >= deadline) {
      const why = lastError instanceof Error ? ` (the last read failed: ${lastError.message})` : '';
      throw new Error(`${what}: not within ${String(timeoutMs)} ms${why}`);
    }
    await nextRead();
  }
}

/**
 * The pause between two reads of the same condition.
 *
 * Deliberately not unref'd: a scenario spends most of its life inside one of these, and an
 * unref'd timer lets Node leave with "unsettled top-level await" the moment nothing else holds
 * the loop — the e2e harness learnt that on its first real run (`tests/e2e/automation.ts`).
 */
function nextRead(): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, POLL_MS);
  });
}
