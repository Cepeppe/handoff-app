/**
 * The harness end of the automation channel (T-043, DD-33, TECHNICAL-DESIGN §11.5).
 *
 * NDJSON JSON-RPC over the second endpoint the `--features e2e` build opens: one `auth`
 * carrying the token of `<HANDOFF_HOME>/e2e.token`, then `state`, `act`, `open_request`,
 * `settings` and `quit`. `src-tauri/src/e2e/api.rs` is the other side and the table in its
 * module documentation is the contract.
 *
 * Two things here are not obvious and both cost a run to learn:
 *
 * 1. **A named-pipe client has to retry.** The accept loop creates the next pipe instance
 *    only after taking the previous one, so a client arriving in that window is refused with
 *    `ENOENT` or `EBUSY` rather than queued (the T-031 handoff entry). [`connect`] retries;
 *    it is also what waits out the app's startup, which is why the timeout is generous.
 * 2. **`state` is a poll, not a subscription.** Nothing pushes: a scenario waits for a
 *    condition with [`Automation.waitFor`], which re-reads and gives up with the last state
 *    it saw, so a timeout says what the app thought rather than only that it timed out.
 */
import { readFileSync } from 'node:fs';
import { connect as netConnect, type Socket } from 'node:net';
import { join } from 'node:path';

/** How long [`connect`] keeps trying before it gives up on the app. */
export const CONNECT_TIMEOUT_MS = 60_000;

/** How long one method may take. Every method is a local read or a store command. */
export const CALL_TIMEOUT_MS = 30_000;

/**
 * One handoff, as `state` reports it: the whole `HandoffView` the window is given, plus the
 * two counters the view drops (§11.5 asks about both).
 *
 * `camelCase`, because it is the window's own object: `ui_bridge::view` is what serialises
 * it and the automation channel does not rename what it forwards.
 */
export interface HandoffState {
  readonly tab: {
    readonly id: string;
    readonly label: string;
    readonly state: string;
    readonly uiState: string;
    readonly group: string;
    readonly orphan: boolean;
  };
  /** The wire name of §8.1: `active`, `deferred`, `verified`, … */
  readonly state: string;
  /** The row of §8.4. */
  readonly uiState: string;
  readonly banner: { readonly key: string; readonly arg: string | null } | null;
  readonly goal: string | null;
  readonly step: {
    readonly counter: { readonly index: number; readonly total: number; readonly round: number };
    readonly text: string;
    readonly confirmed: boolean;
    readonly skipped: boolean;
    readonly last: boolean;
    readonly notes: readonly { readonly step: number; readonly text: string }[];
    readonly questions: readonly { readonly step: number; readonly text: string }[];
    readonly replies: readonly { readonly step: number; readonly text: string }[];
  } | null;
  readonly pending: { readonly kind: string } | null;
  readonly history: readonly unknown[];
  readonly verify: string | null;
  readonly verifyResult: {
    readonly state: string;
    readonly detail: string | null;
  } | null;
  /**
   * Why a `not_verified` handoff is not verified, when no report says it (VER-06).
   *
   * A catalogue key, `null` on every other state and on a `not_verified` the agent reported
   * itself — there `verifyResult` carries the agent's own detail.
   */
  readonly notVerifiedReason: string | null;
  readonly callAttached: boolean;
  readonly undelivered: number;
  readonly requestText: string | null;
  readonly linkedRequest: { readonly id: string; readonly text: string | null } | null;
  readonly resumedFrom: { readonly agent: string; readonly project: string } | null;
  readonly createdAt: string;
  readonly closedAt: string | null;
  /** The current round (VER-09). */
  readonly round: number;
  /** 0, 1 or 2 (RESP-05, RESP-06). */
  readonly deferralCount: number;
  readonly [field: string]: unknown;
}

/** One session of the registry, as `state` reports it. */
export interface SessionState {
  readonly sessionRef: string;
  readonly agentId: string | null;
  readonly clientName: string | null;
  readonly connected: boolean;
  readonly cwd: string;
  readonly projectDir: string | null;
  readonly claudeSessionId: string | null;
  readonly firstSeen: string;
  readonly lastSeen: string;
}

/** One entry of the request queue, as `state` reports it. */
export interface RequestState {
  readonly id: string;
  readonly text: string;
  readonly sessionRef: string | null;
  readonly deliveredVia: string | null;
  readonly linkedHandoffId: string | null;
  readonly aboutHandoffId: string | null;
  readonly createdAt: string;
}

/** What `state` answers with. */
export interface AppState {
  readonly handoffs: readonly HandoffState[];
  readonly sessions: readonly SessionState[];
  readonly requests: readonly RequestState[];
}

/** A method the app refused, with the code of `e2e::api::codes`. */
export class AutomationError extends Error {
  readonly code: number;

  constructor(code: number, message: string) {
    super(`${message} (code ${String(code)})`);
    this.name = 'AutomationError';
    this.code = code;
  }
}

/** `<HANDOFF_HOME>/e2e.endpoint`, which the app writes once it is bound. */
export const ENDPOINT_FILE = 'e2e.endpoint';

/**
 * The endpoint the app of this `HANDOFF_HOME` is listening on.
 *
 * Read from the file the app publishes, never re-derived. The product channel is the
 * opposite case by design — the server computes the same digest from the same environment
 * and that is what proves the derivation (§5.8) — but both ends of *this* channel are the
 * same commit, so a second SHA-256 here would buy nothing and could be wrong.
 */
export function endpointOf(home: string): string {
  return readFileSync(join(home, ENDPOINT_FILE), 'utf8').trim();
}

/** The token the app wrote for this run. */
export function tokenOf(home: string): string {
  return readFileSync(join(home, 'e2e.token'), 'utf8').trim();
}

/** A connected, authenticated automation channel. */
export class Automation {
  private readonly socket: Socket;
  private buffer = '';
  private nextId = 1;
  private readonly pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (cause: Error) => void }
  >();
  private closed: Error | undefined;

  private constructor(socket: Socket) {
    this.socket = socket;
    socket.setEncoding('utf8');
    socket.on('data', (chunk: string) => this.onData(chunk));
    socket.on('error', (cause) => this.onClose(cause));
    socket.on('close', () => this.onClose(new Error('the automation channel closed')));
  }

  /** Connects and authenticates, retrying while the app is still starting. */
  static async open(home: string, timeoutMs = CONNECT_TIMEOUT_MS): Promise<Automation> {
    const deadline = Date.now() + timeoutMs;
    let last = 'the app never opened the automation channel';
    while (Date.now() < deadline) {
      let token: string;
      try {
        token = tokenOf(home);
      } catch {
        last = `${join(home, 'e2e.token')} does not exist yet`;
        await sleep(200);
        continue;
      }
      try {
        const socket = await connectOnce(endpointOf(home));
        const automation = new Automation(socket);
        await automation.call('auth', { token });
        return automation;
      } catch (cause) {
        last = cause instanceof Error ? cause.message : String(cause);
        await sleep(200);
      }
    }
    throw new Error(`${last} within ${String(timeoutMs)} ms`);
  }

  /** One method call. */
  async call(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
    if (this.closed !== undefined) throw this.closed;
    const id = this.nextId++;
    const line = `${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`;
    const answer = new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} did not answer within ${String(CALL_TIMEOUT_MS)} ms`));
      }, CALL_TIMEOUT_MS);
      this.pending.set(id, {
        resolve: (value) => {
          clearTimeout(timer);
          resolve(value);
        },
        reject: (cause) => {
          clearTimeout(timer);
          reject(cause);
        },
      });
    });
    this.socket.write(line);
    return answer;
  }

  /** The store snapshot, the sessions and the request queue. */
  async state(): Promise<AppState> {
    return (await this.call('state')) as AppState;
  }

  /** One press of one button (RESP-01..09). */
  async act(handoffId: string, action: string, payload?: string): Promise<void> {
    await this.call('act', {
      handoff_id: handoffId,
      action,
      ...(payload === undefined ? {} : { payload }),
    });
  }

  /** The request sheet of §7.7, without the sheet. Answers the `hf_` id it created. */
  async openRequest(text: string, sessionRef?: string): Promise<string> {
    const answer = (await this.call('open_request', {
      text,
      ...(sessionRef === undefined ? {} : { session_ref: sessionRef }),
    })) as { id: string };
    return answer.id;
  }

  /** Reads a setting. */
  async getSetting(key: string): Promise<unknown> {
    return ((await this.call('settings', { op: 'get', key })) as { value: unknown }).value;
  }

  /** Writes a setting, including the injected verification window of E2E-10. */
  async setSetting(key: string, value: unknown): Promise<unknown> {
    return ((await this.call('settings', { op: 'set', key, value })) as { value: unknown }).value;
  }

  /** Asks the app to leave, and waits for the socket to go with it. */
  async quit(): Promise<void> {
    try {
      await this.call('quit');
    } catch {
      // A `quit` that never came back means the app went first, which is the point of it.
    }
    this.socket.destroy();
  }

  /** Drops the connection without asking the app to leave. */
  close(): void {
    this.socket.destroy();
  }

  /**
   * Polls `state` until `predicate` holds, and answers the state that satisfied it.
   *
   * The failure carries `what` and the last state seen, because "the handoff never reached
   * `verified`" and "there was never a handoff at all" are different bugs and a bare timeout
   * cannot tell them apart.
   */
  async waitFor(
    what: string,
    predicate: (state: AppState) => boolean,
    timeoutMs = 120_000,
  ): Promise<AppState> {
    const deadline = Date.now() + timeoutMs;
    let last: AppState = { handoffs: [], sessions: [], requests: [] };
    while (Date.now() < deadline) {
      last = await this.state();
      if (predicate(last)) return last;
      await sleep(250);
    }
    throw new Error(
      `${what} did not happen within ${String(timeoutMs)} ms. Last state: ${summarise(last)}`,
    );
  }

  /** The one handoff of a scenario, once there is exactly one. */
  async theHandoff(timeoutMs = 120_000): Promise<HandoffState> {
    const state = await this.waitFor(
      'a handoff appeared',
      (seen) => seen.handoffs.length > 0,
      timeoutMs,
    );
    const [handoff] = state.handoffs;
    if (handoff === undefined) throw new Error('unreachable: waitFor guaranteed one');
    return handoff;
  }

  private onData(chunk: string): void {
    this.buffer += chunk;
    let at = this.buffer.indexOf('\n');
    while (at !== -1) {
      const line = this.buffer.slice(0, at).trim();
      this.buffer = this.buffer.slice(at + 1);
      if (line !== '') this.onLine(line);
      at = this.buffer.indexOf('\n');
    }
  }

  private onLine(line: string): void {
    let message: {
      id?: unknown;
      result?: unknown;
      error?: { code: number; message: string };
    };
    try {
      message = JSON.parse(line) as typeof message;
    } catch {
      return;
    }
    const id = typeof message.id === 'number' ? message.id : -1;
    const waiting = this.pending.get(id);
    if (waiting === undefined) return;
    this.pending.delete(id);
    if (message.error !== undefined) {
      waiting.reject(new AutomationError(message.error.code, message.error.message));
    } else {
      waiting.resolve(message.result);
    }
  }

  private onClose(cause: Error): void {
    this.closed ??= cause;
    for (const [, waiting] of this.pending) waiting.reject(cause);
    this.pending.clear();
  }
}

/** One connection attempt. */
function connectOnce(endpoint: string): Promise<Socket> {
  return new Promise((resolve, reject) => {
    const socket = netConnect({ path: endpoint });
    const onError = (cause: Error): void => {
      socket.destroy();
      reject(cause);
    };
    socket.once('error', onError);
    socket.once('connect', () => {
      socket.removeListener('error', onError);
      resolve(socket);
    });
  });
}

/** A short, readable digest of a state, for the message of a timeout. */
export function summarise(state: AppState): string {
  return JSON.stringify({
    handoffs: state.handoffs.map((handoff) => ({
      id: handoff.tab.id,
      state: handoff.state,
      ui: handoff.uiState,
      step:
        handoff.step === null
          ? null
          : `${String(handoff.step.counter.index)}/${String(handoff.step.counter.total)}`,
      call: handoff.callAttached,
    })),
    sessions: state.sessions.map((session) => ({
      ref: session.sessionRef,
      connected: session.connected,
    })),
    requests: state.requests.map((request) => ({
      id: request.id,
      linked: request.linkedHandoffId,
      about: request.aboutHandoffId,
    })),
  });
}

/**
 * `setTimeout` as a promise.
 *
 * Deliberately **not** unref'd. A scenario spends most of its life inside one of these,
 * waiting for a model to think; with the timer unref'd, a moment in which nothing else holds
 * the loop — between two polls, while the agent is quiet — empties it, and Node exits with
 * "unsettled top-level await" and no other explanation. Measured on the first real run.
 */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}
