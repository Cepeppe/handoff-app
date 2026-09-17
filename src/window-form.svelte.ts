/**
 * Which shape the window has while it is open: the narrow panel, or the expanded view.
 *
 * WIN-02 fixes the panel's width, and it stays fixed: nothing here widens the window by
 * itself, by dragging an edge or by remembering anything. What this adds is one control the
 * user presses — **Expand** — and the answer to "what did they press last".
 *
 * **It is in memory and never on disk.** A preference written to the settings table would
 * decide the shape of the first window of the next launch, which is a decision about a
 * handoff nobody has seen yet; the panel of WIN-02 is the right default every morning. The
 * form therefore lives exactly as long as the process, like the badge counts of
 * `overlay/state.svelte.ts` and for the same reason.
 *
 * It is apart from `overlay/collapse.svelte.ts` because the two answer different questions.
 * `collapsed` is *the window got out of the way* (WIN-03) and is driven by the focus;
 * `form` is *the user asked for more room* and is driven by a button. They compose: the bar
 * is always 360 wide, and a click on the bar comes back to whichever form was in force.
 */

/** The two shapes the open window can have. The collapsed bar is neither: it replaces both. */
export type WindowForm = 'panel' | 'expanded';

let current = $state<WindowForm>('panel');

/** The form in force. Reactive: reading it inside a component subscribes to it. */
export function form(): WindowForm {
  return current;
}

/** The expanded view of §7.6: the handoff list on the left, one column of content. */
export function expandForm(): void {
  current = 'expanded';
}

/** Back to the narrow panel of WIN-02. */
export function restoreForm(): void {
  current = 'panel';
}

/** What the third window control does, and what a double-click on the header does. */
export function toggleForm(): void {
  current = current === 'expanded' ? 'panel' : 'expanded';
}

/** Back to the panel with nothing remembered. Tests call it between cases; nothing else does. */
export function resetForm(): void {
  current = 'panel';
}
