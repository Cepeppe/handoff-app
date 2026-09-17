/**
 * What a UI scenario is, and the page it drives (T-055, §11.4).
 *
 * A scenario is one function. It is handed a running application with its WebDriver session
 * and its automation channel, a way to register fake server sessions, and a place to record
 * what it measured; it plays the user through the real window and throws at the first thing
 * that is not as it should be. Everything around it — the temporary folders, the drivers, the
 * application, the retry, the screenshots — belongs to `main.ts`.
 *
 * # Two gestures WebDriver cannot make, and what stands in for them
 *
 * WebDriver drives the page. It cannot reach the tray icon, press a
 * global shortcut, or move the operating system's focus to another window. Each of those ends
 * in the Rust side emitting one event to the window — `ui://show-view` with the view the tray
 * item or the shortcut asked for (`ui_bridge::show_view`), and `ui://window-focus` with the
 * focus Tauri reported (`ui_bridge::on_window_event`) — and everything the user then sees is
 * the window's reaction to that event. So [`Page.showView`], [`Page.blur`] and [`Page.focus`]
 * emit exactly those two events, through the event plugin the window's capability already
 * grants, and every assertion after them is about the real window. What stays untested here
 * is the Rust half of each: one `emit` call, whose event names a Rust test keeps equal to the
 * frontend's (`ui_bridge` `event_names_match_the_frontend`).
 */
import { execFileSync } from 'node:child_process';

import type { RunningApp, UiWorkspace } from './app.ts';
import type { FakeServer, SessionOptions } from './fake-server.ts';
import { until } from './wait.ts';
import type { ElementRef, Point, Session } from './webdriver.ts';

/** What a scenario is given. */
export interface UiContext {
  /** The window. */
  readonly page: Page;
  /** The application: its automation channel, its workspace, its log. */
  readonly app: RunningApp;
  /** Registers a fake server session; the harness closes it when the attempt ends. */
  session(options?: Partial<SessionOptions>): Promise<FakeServer>;
  /** What was measured, for the report. */
  readonly facts: Record<string, unknown>;
  /** One line on stderr, so a long scenario says where it is. */
  say(what: string): void;
}

/** One scenario of the suite. */
export interface UiScenario {
  /** A short, hyphenated name: `step-view`. */
  readonly id: string;
  /** The requirements it exercises. */
  readonly covers: string;
  /** One line, for the report. */
  readonly title: string;
  /** Whether the application starts as one that has been through onboarding. Default `true`. */
  readonly onboarded?: boolean;
  /** Runs before the application starts, to plant files in the attempt's workspace. */
  prepare?(workspace: UiWorkspace): void;
  /** Plays the user, and throws at the first thing that is wrong. */
  run(context: UiContext): Promise<void>;
}

/** Fails the scenario with `what` unless `condition` holds; `seen` goes into the message. */
export function expect(condition: boolean, what: string, seen?: unknown): void {
  if (!condition) {
    throw new Error(seen === undefined ? what : `${what} (saw ${JSON.stringify(seen)})`);
  }
}

/** A step of a spec: its text, and the values it uses. */
export type StepSpec = string | { readonly text: string; readonly values?: readonly string[] };

/**
 * A spec the handoff schema accepts (§4.2), with what the scenarios do not care about filled in.
 *
 * `values` is always present, even empty: the schema requires it, and a spec without it closes
 * the connection with nothing more than a log line (`docs/dev/smoke.md`).
 */
export function spec(
  goal: string,
  steps: readonly StepSpec[],
  values: Record<string, string | readonly string[]> = {},
): Record<string, unknown> {
  return {
    spec_version: 1,
    goal,
    where: 'The dashboard of the payment provider',
    why_human: 'Only the account owner can sign in to the dashboard.',
    values,
    steps: steps.map((step) =>
      typeof step === 'string'
        ? { text: step }
        : { text: step.text, ...(step.values === undefined ? {} : { values: step.values }) },
    ),
    lang: 'en',
  };
}

/**
 * What the Windows clipboard holds, read the way a paste reads it.
 *
 * The copy buttons write through the Rust side (`copy_value`), which is the point: DET-04 is
 * about what reaches the clipboard, not about what the button looks like.
 *
 * `windowsHide` is load-bearing. The suite's Node has no console of its own when a tool or a
 * runner starts it, so a console program it starts gets a new console window — and that
 * window takes the focus off the panel, which collapses on exactly that (WIN-03). Every read
 * of the clipboard used to be a chance for the next click to find the bar instead of the
 * button it was aimed at.
 */
export function clipboard(): string {
  return execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', 'Get-Clipboard -Raw'], {
    encoding: 'utf8',
    windowsHide: true,
  }).replace(/\r?\n$/u, '');
}

/** The first element matching a selector, with a given trimmed text if one is asked for. */
const QUERY = `
  const [css, text] = arguments;
  for (const element of document.querySelectorAll(css)) {
    if (text === null || element.textContent.trim() === text) return element;
  }
  return null;
`;

/** Every element matching a selector. */
const QUERY_ALL = 'return [...document.querySelectorAll(arguments[0])];';

/** One Tauri event, emitted from the page as the Rust side emits it. */
const EMIT = `
  const [event, payload, done] = arguments;
  window.__TAURI_INTERNALS__.invoke('plugin:event|emit', { event, payload })
    .then(() => done(null), (error) => done(String(error)));
`;

/** The window, as the scenarios read and press it. */
export class Page {
  readonly session: Session;

  constructor(session: Session) {
    this.session = session;
  }

  /** Runs a script in the page. */
  execute<T>(script: string, args: readonly unknown[] = []): Promise<T> {
    return this.session.execute<T>(script, args);
  }

  /** The element, or `null` when there is none right now. */
  query(css: string, text?: string): Promise<ElementRef | null> {
    return this.session.execute<ElementRef | null>(QUERY, [css, text ?? null]);
  }

  /**
   * Whether a panel that collapsed on its own is opened again while a lookup waits (WIN-03).
   *
   * The panel collapses whenever the operating system takes the focus off it, and on a desktop
   * something always can: a capture hides the panel and shows it again, and here a copy to the
   * clipboard was followed by a lost focus once in about five runs. A scenario about the step
   * view is not about that, so the page does what a person does — clicks the bar — and counts
   * it in `reopened`, which the report carries, so a guard that is doing work shows. The
   * collapse scenario, which is about exactly that, switches it off.
   */
  keepPanelOpen = true;

  /** How many times the page opened a panel that had collapsed on its own. */
  reopened = 0;

  /** Clicks the collapsed bar, when the panel is collapsed and the page is keeping it open. */
  private async reopenIfCollapsed(): Promise<void> {
    if (!this.keepPanelOpen) return;
    const bar = await this.query('.collapsed-line');
    if (bar === null) return;
    this.reopened += 1;
    await this.session.click(bar);
  }

  /** Waits for the element and answers it. */
  find(css: string, text?: string, timeoutMs?: number): Promise<ElementRef> {
    return until(
      `${describe(css, text)} is on screen`,
      async () => {
        const found = await this.query(css, text);
        if (found === null) await this.reopenIfCollapsed();
        return found;
      },
      timeoutMs,
    );
  }

  /** Waits for an element whose trimmed text is exactly `text`. */
  findText(css: string, text: string, timeoutMs?: number): Promise<ElementRef> {
    return this.find(css, text, timeoutMs);
  }

  /** Waits until no element matches. */
  async gone(css: string, text?: string, timeoutMs?: number): Promise<void> {
    await until(`${describe(css, text)} is gone`, async () => (await this.query(css, text)) === null, timeoutMs);
  }

  /** Waits for the element and clicks it. */
  async click(css: string, text?: string): Promise<void> {
    await this.session.click(await this.find(css, text));
  }

  /** Clicks the `index`-th element matching `css`, once there are that many. */
  async clickNth(css: string, index: number): Promise<void> {
    const element = await until(`${css} number ${String(index + 1)} is on screen`, async () => {
      const all = await this.session.execute<ElementRef[]>(QUERY_ALL, [css]);
      return all[index] ?? null;
    });
    await this.session.click(element);
  }

  /** Types into the element, which takes the focus first. */
  async type(css: string, text: string): Promise<void> {
    await this.session.type(await this.find(css), text);
  }

  /** Presses one key in the element (`KEY.enter`, `KEY.escape`). */
  async press(css: string, key: string): Promise<void> {
    await this.session.type(await this.find(css), key);
  }

  /** The trimmed text of the first matching element, once there is one. */
  async text(css: string): Promise<string> {
    const element = await this.find(css);
    return this.session.execute<string>('return arguments[0].textContent.trim();', [element]);
  }

  /** The trimmed texts of every matching element, in document order. */
  texts(css: string): Promise<string[]> {
    return this.session.execute<string[]>(
      'return [...document.querySelectorAll(arguments[0])].map((element) => element.textContent.trim());',
      [css],
    );
  }

  /** How many elements match right now. */
  count(css: string): Promise<number> {
    return this.session.execute<number>('return document.querySelectorAll(arguments[0]).length;', [css]);
  }

  /** An attribute of every matching element, in document order. */
  attributes(css: string, name: string): Promise<(string | null)[]> {
    return this.session.execute<(string | null)[]>(
      'return [...document.querySelectorAll(arguments[0])].map((element) => element.getAttribute(arguments[1]));',
      [css, name],
    );
  }

  /** An attribute of the element with that text, once it is there. */
  async attributeOf(css: string, text: string | undefined, name: string): Promise<string | null> {
    const element = await this.find(css, text);
    return this.session.execute<string | null>('return arguments[0].getAttribute(arguments[1]);', [element, name]);
  }

  /** A property of the first matching element (`value`, `disabled`, …). */
  async property<T>(css: string, name: string): Promise<T> {
    const element = await this.find(css);
    return this.session.execute<T>('return arguments[0][arguments[1]];', [element, name]);
  }

  /** Whether the element with that text is present and can be pressed. */
  async enabled(css: string, text: string): Promise<boolean> {
    const element = await this.query(css, text);
    if (element === null) return false;
    return this.session.execute<boolean>('return !arguments[0].disabled;', [element]);
  }

  /** The `value` of every `<option>` (or any element) matching `css`. */
  values(css: string): Promise<string[]> {
    return this.session.execute<string[]>(
      'return [...document.querySelectorAll(arguments[0])].map((element) => element.value);',
      [css],
    );
  }

  /** The id of the element that has the focus, or `null`. */
  activeId(): Promise<string | null> {
    return this.session.execute<string | null>('return document.activeElement?.id || null;');
  }

  /** The text a person reads on the whole window. */
  bodyText(): Promise<string> {
    return this.session.execute<string>('return document.body.innerText;');
  }

  /** The view the window is showing (§7.6): `overlay`, `request`, `settings`, … */
  view(): Promise<string | null> {
    return this.session.execute<string | null>(
      'return document.querySelector("[data-view]")?.getAttribute("data-view") ?? null;',
    );
  }

  /**
   * Waits until the window shows `name`.
   *
   * A collapsed panel shows no view at all — the bar replaces the whole window — so a panel
   * that collapsed on its own is opened again here as in `find`.
   */
  async untilView(name: string, timeoutMs?: number): Promise<void> {
    await until(
      `the window shows the ${name} view`,
      async () => {
        if ((await this.view()) === name) return true;
        await this.reopenIfCollapsed();
        return false;
      },
      timeoutMs,
    );
  }

  /** The window's inner size, in CSS pixels. */
  size(): Promise<{ width: number; height: number }> {
    return this.session.execute<{ width: number; height: number }>(
      'return { width: window.innerWidth, height: window.innerHeight };',
    );
  }

  /** What the tray's menu item or the global shortcut does to the window (`ui://show-view`). */
  async showView(name: string): Promise<void> {
    await this.emit('ui://show-view', name);
    await this.untilView(name);
  }

  /** What clicking another window does to the panel (WIN-03): Tauri reports the focus lost. */
  async blur(): Promise<void> {
    await this.emit('ui://window-focus', false);
  }

  /** What clicking back on the panel does (WIN-03): Tauri reports the focus gained. */
  async focus(): Promise<void> {
    await this.emit('ui://window-focus', true);
  }

  /**
   * Presses the mouse at `from` and releases it at `to`, both given as fractions of the
   * element's box (`{ x: 0.1, y: 0.8 }` is near its bottom-left corner).
   */
  async drag(css: string, from: Point, to: Point): Promise<void> {
    const element = await this.find(css);
    await this.session.execute('arguments[0].scrollIntoView({ block: "nearest" });', [element]);
    const box = await this.session.rect(element);
    const offset = (point: Point): Point => ({
      x: (point.x - 0.5) * box.width,
      y: (point.y - 0.5) * box.height,
    });
    await this.session.drag(element, offset(from), offset(to));
  }

  /** Presses the mouse at `from` (a fraction of the box) and releases it `pixels` further on. */
  async dragPixels(css: string, from: Point, pixels: Point): Promise<void> {
    const element = await this.find(css);
    await this.session.execute('arguments[0].scrollIntoView({ block: "nearest" });', [element]);
    const box = await this.session.rect(element);
    const start: Point = { x: (from.x - 0.5) * box.width, y: (from.y - 0.5) * box.height };
    await this.session.drag(element, start, { x: start.x + pixels.x, y: start.y + pixels.y }, 1);
  }

  /** One Tauri event, emitted from the page (see the module documentation for why). */
  private async emit(event: string, payload: unknown): Promise<void> {
    const refused = await this.session.executeAsync<string | null>(EMIT, [event, payload]);
    if (refused !== null) {
      throw new Error(`the window could not emit ${event}: ${refused}`);
    }
  }
}

/** `css`, or `css` with the text it must have, for a message. */
function describe(css: string, text?: string): string {
  return text === undefined ? css : `${css} "${text}"`;
}
