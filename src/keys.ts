/**
 * How a key combination is written on the screen it is shown on.
 *
 * Two places need it: the hint under a sheet's textarea ("Ctrl+Enter sends · Esc cancels")
 * and the keycap beside **New request** in the expanded view, which shows the accelerator
 * the system actually accepted (OPEN-03, FM-18).
 *
 * The names here are **not** catalogue texts, and that is deliberate. `Ctrl`, `Alt`, `Shift`
 * and `⌘` are the labels printed on the keyboard in front of the user: they are the same in
 * English and in Italian, they do not change when the UI language does, and translating them
 * would produce a keycap that names no key. They belong with `Control+Alt+H` — the wire
 * spelling Tauri uses — and that is why this is a module of symbols rather than a locale
 * entry. Everything around them is a `t()` key, `sheet.hint` included.
 */

/** Whether the machine is a Mac, which is the only thing the labels below depend on. */
function isMac(): boolean {
  if (typeof navigator === 'undefined') {
    return false;
  }
  const platform =
    (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform ??
    navigator.platform ??
    '';
  return /mac|iphone|ipad/i.test(platform);
}

/**
 * The modifier that sends a sheet: `Ctrl` on Windows, `⌘` on macOS.
 *
 * `TextSheet` accepts either one (`ctrlKey || metaKey`), so this only decides which of the
 * two the hint names — the one the user is holding.
 */
export function sendModifier(): string {
  return isMac() ? '⌘' : 'Ctrl';
}

/** How one part of an accelerator is printed on a keycap. */
function capOf(part: string): string {
  switch (part) {
    case 'Control':
    case 'CommandOrControl':
    case 'CmdOrCtrl':
      return isMac() ? '⌘' : 'Ctrl';
    case 'Command':
    case 'Cmd':
    case 'Super':
    case 'Meta':
      return isMac() ? '⌘' : 'Win';
    case 'Option':
      return isMac() ? '⌥' : 'Alt';
    default:
      return part;
  }
}

/**
 * An accelerator as the keycaps that spell it: `Control+Alt+H` → `['Ctrl', 'Alt', 'H']`.
 *
 * `null` for a machine with no shortcut registered, where there is no combination to print.
 */
export function keycaps(accelerator: string | null): string[] {
  if (accelerator === null) {
    return [];
  }
  return accelerator
    .split('+')
    .map((part) => part.trim())
    .filter((part) => part.length > 0)
    .map(capOf);
}
