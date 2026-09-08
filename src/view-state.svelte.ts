/**
 * Which view the overlay window is showing (§7.6).
 *
 * One window, one current view: the request sheet, the settings and the onboarding are
 * modes of this window (DD-10), so switching is a state change here and never a second
 * window. The state is a rune in a module rather than a component variable because the
 * tray menu changes it from outside the component tree.
 */
import { DEFAULT_VIEW, type ViewName } from './views';

let current = $state<ViewName>(DEFAULT_VIEW);

/** The view being shown. Reactive: reading it inside a component subscribes to it. */
export function view(): ViewName {
  return current;
}

/** Switches the window to `next`. */
export function showView(next: ViewName): void {
  current = next;
}

/** Back to the view the window opens on. */
export function resetView(): void {
  current = DEFAULT_VIEW;
}
