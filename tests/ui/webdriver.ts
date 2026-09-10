/**
 * The WebDriver client of the UI suite (T-055, TECHNICAL-DESIGN §11.4).
 *
 * `tauri-driver` speaks the W3C WebDriver protocol over HTTP and hands every command on to
 * `msedgedriver`, which drives the WebView2 the application draws in. The protocol is a
 * handful of JSON endpoints, and this file is the ones the scenarios use, over `fetch`: no
 * WebdriverIO, no Selenium, nothing added to `package.json`. A client library would be a few
 * hundred packages to keep current for the same dozen requests, and the automation channel of
 * the e2e harness beside this suite made the same choice for its own wire.
 *
 * Two things here are deliberate:
 *
 * - **No implicit wait.** The session is created with `implicit: 0`, so a lookup answers
 *   "there is none" at once. Waiting is the scenario's business and it waits on a condition
 *   it names (`wait.ts`), never on a timeout the driver hides inside a lookup.
 * - **Elements are found by a script in the page** (`scenario.ts`). A CSS selector alone
 *   cannot say "the button whose text is Done", and the protocol's own text strategies cover
 *   anchors only; one `querySelectorAll` and a text filter cover every case, and the answer is
 *   a real element reference the driver can click.
 */

/** The key under which the protocol carries an element reference (W3C WebDriver §12.1). */
export const ELEMENT_KEY = 'element-6066-11e4-a52e-4f735466cecf';

/** One element of the page, as the driver names it. */
export interface ElementRef {
  readonly [ELEMENT_KEY]: string;
}

/**
 * The two keys the scenarios press (W3C WebDriver §17.4.2, the normalised key values).
 *
 * Built from their code points rather than written as literals: both are in the Unicode
 * private-use area, and a literal of either is an invisible character in the source.
 */
export const KEY = {
  enter: String.fromCharCode(0xe007),
  escape: String.fromCharCode(0xe00c),
} as const;

/** How long one command may take. A new session starts the application, so it gets more. */
const COMMAND_TIMEOUT_MS = 60_000;
const NEW_SESSION_TIMEOUT_MS = 120_000;

/** An error the driver answered with: `error` is the protocol's code, `status` the HTTP one. */
export class WebDriverError extends Error {
  readonly error: string;
  readonly status: number;

  constructor(status: number, error: string, message: string) {
    super(`${error}: ${message}`);
    this.name = 'WebDriverError';
    this.status = status;
    this.error = error;
  }
}

/** One protocol request, answering the `value` of the reply. */
async function request(
  base: string,
  method: 'GET' | 'POST' | 'DELETE',
  path: string,
  body?: unknown,
  timeoutMs = COMMAND_TIMEOUT_MS,
): Promise<unknown> {
  const response = await fetch(`${base}${path}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json; charset=utf-8' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(timeoutMs),
  });
  const text = await response.text();
  let parsed: { value?: unknown };
  try {
    parsed = JSON.parse(text) as { value?: unknown };
  } catch {
    throw new WebDriverError(response.status, 'unreadable answer', text.slice(0, 300));
  }
  if (!response.ok) {
    const value = (parsed.value ?? {}) as { error?: string; message?: string };
    throw new WebDriverError(
      response.status,
      value.error ?? 'unknown error',
      (value.message ?? text).slice(0, 600),
    );
  }
  return parsed.value;
}

/** A point, in CSS pixels. */
export interface Point {
  readonly x: number;
  readonly y: number;
}

/** One WebDriver session: one running application. */
export class Session {
  /** Where the driver listens, `http://127.0.0.1:<port>`. */
  readonly base: string;
  /** The session id the driver gave. */
  readonly id: string;
  /** What the driver answered when the session was created. */
  readonly capabilities: Record<string, unknown>;

  private constructor(base: string, id: string, capabilities: Record<string, unknown>) {
    this.base = base;
    this.id = id;
    this.capabilities = capabilities;
  }

  /** Starts the application the capabilities name, and answers once its page has loaded. */
  static async create(base: string, capabilities: Record<string, unknown>): Promise<Session> {
    const value = (await request(
      base,
      'POST',
      '/session',
      { capabilities: { alwaysMatch: capabilities } },
      NEW_SESSION_TIMEOUT_MS,
    )) as { sessionId: string; capabilities: Record<string, unknown> };
    const session = new Session(base, value.sessionId, value.capabilities);
    await session.call('POST', '/timeouts', { implicit: 0, script: 30_000 });
    return session;
  }

  private call(method: 'GET' | 'POST' | 'DELETE', path: string, body?: unknown): Promise<unknown> {
    return request(this.base, method, `/session/${this.id}${path}`, body);
  }

  /** Ends the session, which closes the application. */
  async delete(): Promise<void> {
    await request(this.base, 'DELETE', `/session/${this.id}`);
  }

  /** Runs `script` in the page; `arguments` are `args`, and an element reference is a DOM node. */
  async execute<T>(script: string, args: readonly unknown[] = []): Promise<T> {
    return (await this.call('POST', '/execute/sync', { script, args })) as T;
  }

  /** Runs `script` in the page and waits for it to call its last argument. */
  async executeAsync<T>(script: string, args: readonly unknown[] = []): Promise<T> {
    return (await this.call('POST', '/execute/async', { script, args })) as T;
  }

  /** A click at the element's centre, scrolled into view first (W3C Element Click). */
  async click(element: ElementRef): Promise<void> {
    await this.call('POST', `/element/${element[ELEMENT_KEY]}/click`, {});
  }

  /** Types `text` into the element, which takes the focus first (W3C Element Send Keys). */
  async type(element: ElementRef, text: string): Promise<void> {
    await this.call('POST', `/element/${element[ELEMENT_KEY]}/value`, { text });
  }

  /** The element's rectangle in the page, in CSS pixels. */
  async rect(element: ElementRef): Promise<{ x: number; y: number; width: number; height: number }> {
    return (await this.call('GET', `/element/${element[ELEMENT_KEY]}/rect`)) as {
      x: number;
      y: number;
      width: number;
      height: number;
    };
  }

  /**
   * Presses the left button at `from`, moves to `to` in a few steps, and releases it.
   *
   * Both points are offsets from the element's centre, which is what the protocol measures a
   * pointer move against when its origin is an element. The intermediate moves are there
   * because the preview reads the rectangle from `pointermove` events, as a hand would give
   * it, and not from the two ends alone.
   */
  async drag(element: ElementRef, from: Point, to: Point, steps = 4): Promise<void> {
    const moves: unknown[] = [];
    for (let step = 1; step <= steps; step += 1) {
      moves.push({
        type: 'pointerMove',
        duration: 40,
        origin: element,
        x: Math.round(from.x + ((to.x - from.x) * step) / steps),
        y: Math.round(from.y + ((to.y - from.y) * step) / steps),
      });
    }
    await this.call('POST', '/actions', {
      actions: [
        {
          type: 'pointer',
          id: 'mouse',
          parameters: { pointerType: 'mouse' },
          actions: [
            { type: 'pointerMove', duration: 0, origin: element, x: Math.round(from.x), y: Math.round(from.y) },
            { type: 'pointerDown', button: 0 },
            ...moves,
            { type: 'pointerUp', button: 0 },
          ],
        },
      ],
    });
    await this.call('DELETE', '/actions');
  }

  /** The window as the webview draws it, as PNG bytes. */
  async screenshot(): Promise<Buffer> {
    return Buffer.from((await this.call('GET', '/screenshot')) as string, 'base64');
  }

  /** The DOM as it is now, serialised. */
  async source(): Promise<string> {
    return (await this.call('GET', '/source')) as string;
  }

  /** The page's address. */
  async url(): Promise<string> {
    return (await this.call('GET', '/url')) as string;
  }

  /** Reloads the page, which mounts the window again exactly as a launch does. */
  async refresh(): Promise<void> {
    await this.call('POST', '/refresh', {});
  }
}
