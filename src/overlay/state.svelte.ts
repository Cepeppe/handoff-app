/**
 * What the overlay knows between two repaints (§7.6, MULTI-01..04).
 *
 * The core is the single owner of handoff state, so nothing here is a copy of it: this holds
 * the last answer of `listHandoffs` / `getHandoffView` and the three things only the window
 * can know.
 *
 * - **Which tab is selected.** Switching tabs returns no tool call (MULTI-01) and never
 *   touches the agent, so it is a variable here and nothing else.
 * - **The badge.** "Unseen events" (§7.6) is a fact about which tab the user is *looking
 *   at*, which the core has no way to know: a change to a tab that is not the selected one
 *   raises its badge, selecting it clears it. It lives for as long as the window does, which
 *   is what "unseen" means.
 * - **The reveal.** A secret-treated value is shown for ten seconds and then hidden again
 *   (DET-04). The value is fetched when the user asks and dropped when the timer fires; it
 *   is never part of a repaint and never stored.
 *
 * Focus (MULTI-03): the first handoff that arrives while none is active selects itself and
 * asks for the front; later ones only raise a badge. The rule is here because it is about
 * what the user is looking at, which the core has no way to know — it reports that a tab
 * changed and this module decides whether that is worth interrupting anyone for.
 */
import { bridge } from '../bridge';
import type { HandoffView, Notice, TabView } from '../model';

/** How long a revealed value stays visible (DET-04). */
export const REVEAL_MS = 10_000;

let tabs = $state<TabView[]>([]);
let selected = $state<string | null>(null);
let current = $state<HandoffView | null>(null);
let badges = $state<Record<string, number>>({});
let notice = $state<Notice | null>(null);
let revealed = $state<Record<string, string[]>>({});

/**
 * Whether the strip has been read at least once.
 *
 * The first read is the window opening on what a previous run left behind; a handoff
 * *arriving* is every read after it. Only the second kind may ask for the front (MULTI-03).
 */
let loaded = false;

const revealTimers = new Map<string, ReturnType<typeof setTimeout>>();

/** The tab strip, oldest first. */
export function allTabs(): TabView[] {
  return tabs;
}

/** The id of the selected tab, if there is one. */
export function selectedId(): string | null {
  return selected;
}

/** The selected tab, whole. */
export function currentView(): HandoffView | null {
  return current;
}

/** How many changes happened on `id` while the user was looking elsewhere. */
export function badge(id: string): number {
  return badges[id] ?? 0;
}

/** The sentence to show, if any. */
export function currentNotice(): Notice | null {
  return notice;
}

/** Shows a sentence to the user until the next one. */
export function showNotice(next: Notice | null): void {
  notice = next;
}

/** The items of `key` while its ten seconds last, or `null` when it is hidden. */
export function revealedValue(key: string): string[] | null {
  return revealed[key] ?? null;
}

/** Re-reads the tab strip, keeping the selection when it still exists. */
export async function refreshTabs(): Promise<void> {
  const next = await bridge().listHandoffs();
  const arriving = next.filter((tab) => !tabs.some((known) => known.id === tab.id));
  const first = !loaded;
  loaded = true;
  tabs = next;

  if (selected !== null && !next.some((tab) => tab.id === selected)) {
    selected = null;
    current = null;
  }
  // MULTI-03: the first handoff opened while none is active is the one the user is taken
  // to; anything arriving beside an active tab only raises a badge, and the badge is raised
  // by `handoffChanged` when the core says that tab moved.
  if (selected === null && next.length > 0) {
    const arrived = arriving[0] ?? next[0];
    await select(arrived.id);
    // The first read of the strip is the window opening on what a previous run left behind,
    // not a handoff arriving: restoring the tabs must not raise the window at every launch.
    // What the window does on a launch is decided with the rest of the startup sequence.
    // TASK: T-042 — the startup sequence decides whether a restored tab shows the window.
    if (!first && arrived.uiState !== 'final') {
      await bridge().showWindow();
    }
  }
}

/** Re-reads the selected tab. */
export async function refreshCurrent(): Promise<void> {
  if (selected === null) {
    current = null;
    return;
  }
  current = await bridge().getHandoffView(selected);
}

/** Switches to `id`, clears its badge and forgets whatever was revealed on the old tab. */
export async function select(id: string): Promise<void> {
  if (selected !== id) {
    hideEverything();
  }
  selected = id;
  const { [id]: _seen, ...rest } = badges;
  badges = rest;
  await refreshCurrent();
}

/** The core says `id` changed: re-read it, or raise its badge when it is not on screen. */
export async function handoffChanged(id: string): Promise<void> {
  await refreshTabs();
  if (selected === id) {
    await refreshCurrent();
    return;
  }
  badges = { ...badges, [id]: (badges[id] ?? 0) + 1 };
}

/**
 * Reveals a value for ten seconds (DET-04).
 *
 * A second press before the timer fires hides it again, which is the only way a user has to
 * take a shoulder-surfer off a screen they did not mean to show.
 */
export async function reveal(id: string, key: string): Promise<void> {
  const slot = `${id}/${key}`;
  if (revealed[slot] !== undefined) {
    hide(slot);
    return;
  }
  const items = await bridge().revealValue(id, key);
  if (items.length === 0) {
    return;
  }
  revealed = { ...revealed, [slot]: items };
  revealTimers.set(
    slot,
    setTimeout(() => hide(slot), REVEAL_MS),
  );
}

/** Hides a revealed value, now. */
export function hide(slot: string): void {
  const timer = revealTimers.get(slot);
  if (timer !== undefined) {
    clearTimeout(timer);
    revealTimers.delete(slot);
  }
  const { [slot]: _gone, ...rest } = revealed;
  revealed = rest;
}

/** Hides everything that is revealed: a tab switch, and the teardown of the view. */
export function hideEverything(): void {
  for (const timer of revealTimers.values()) {
    clearTimeout(timer);
  }
  revealTimers.clear();
  revealed = {};
}

/** Back to an empty overlay. Tests call it between cases; nothing else does. */
export function resetOverlay(): void {
  hideEverything();
  loaded = false;
  tabs = [];
  selected = null;
  current = null;
  badges = {};
  notice = null;
}
