/**
 * Whether the panel is collapsed to the one-line bar (WIN-03, R-10).
 *
 * The rule is one sentence — "when the user clicks elsewhere, the panel collapses; it
 * re-expands on click" — and everything here exists because that sentence has two halves
 * that do not always arrive.
 *
 * - **The blur.** The Rust side reports the window losing and gaining the focus
 *   (`EVENT_WINDOW_FOCUS`), which is the same gesture said the other way round: clicking
 *   elsewhere is losing the focus, and clicking back on the bar is gaining it.
 * - **The fallback.** R-10 records that always-on-top and blur detection are unreliable on
 *   at least one platform, and gives the mitigation: collapse also on a three-second timer
 *   after the last interaction. It is **off by default** and lives behind a setting,
 *   because a timer that collapses a panel someone is reading is worse than a blur event
 *   that never comes.
 *
 * It lives in a module rather than in a component because the collapse replaces the whole
 * window (`App.svelte`) while what the bar shows comes from the overlay, and because the
 * event that drives it arrives from outside the component tree.
 */
import { bridge } from '../bridge';

let collapsed = $state(false);
let fallbackEnabled = $state(false);
let fallbackMs = $state(3_000);

let idleTimer: ReturnType<typeof setTimeout> | null = null;

/** Whether the panel is showing the one-line bar. */
export function isCollapsed(): boolean {
  return collapsed;
}

/** Whether the R-10 timer is armed at all. */
export function fallbackIsOn(): boolean {
  return fallbackEnabled;
}

/** How long the R-10 timer waits, in milliseconds. */
export function fallbackDelay(): number {
  return fallbackMs;
}

/** Shrinks the panel to the bar. */
export function collapse(): void {
  stopIdleTimer();
  collapsed = true;
}

/** Brings the whole panel back, and re-arms the fallback if it is on. */
export function expand(): void {
  collapsed = false;
  noteInteraction();
}

/**
 * The window gained or lost the focus (WIN-03).
 *
 * Losing it collapses; gaining it expands, which is what "re-expands on click" means for a
 * window that has to be clicked to be focused.
 */
export function focusChanged(focused: boolean): void {
  if (focused) {
    expand();
  } else {
    collapse();
  }
}

/**
 * The user did something in the window: restart the R-10 timer.
 *
 * A no-op while the fallback is off, which is the ordinary case — nothing arms a timer
 * nobody asked for.
 */
export function noteInteraction(): void {
  stopIdleTimer();
  if (!fallbackEnabled || collapsed) {
    return;
  }
  idleTimer = setTimeout(collapse, fallbackMs);
}

/** Reads the window settings (§7.16) and arms the fallback if the user switched it on. */
export async function loadWindowSettings(): Promise<void> {
  const settings = await bridge().windowSettings();
  fallbackEnabled = settings.collapseFallback;
  fallbackMs = settings.collapseFallbackMs;
  noteInteraction();
}

/** Stops the timer without changing anything else; the teardown of the window. */
export function stopIdleTimer(): void {
  if (idleTimer !== null) {
    clearTimeout(idleTimer);
    idleTimer = null;
  }
}

/** Back to an expanded panel with no timer. Tests call it between cases; nothing else does. */
export function resetCollapse(): void {
  stopIdleTimer();
  collapsed = false;
  fallbackEnabled = false;
  fallbackMs = 3_000;
}
