/**
 * A server session the UI suite registers by itself (T-055, §11.3, §11.4).
 *
 * §11.4 drives the frontend "against `fake-server`", and the `fake-server` of T-034 is a
 * module of the Rust integration suite (`src-tauri/tests/fake-server/`): it lives inside
 * `cargo test` and cannot be handed to a WebDriver scenario running in Node. This is the same
 * double on the same wire, cut down to what a scenario needs from a session: register with the
 * capability row the scenario chooses (images or not, §5.6), open a handoff with a spec,
 * answer the application's pings so the connection stays up, and keep what the application
 * sends back, so a scenario can assert that a button really reached the agent.
 *
 * It is written from `protocol/channel/README.md` of the pinned format and from the working
 * example in `docs/dev/smoke.md`: the same `hello`, the same `handoff.open`, and the same
 * three shapes the schema refuses without a word (a `call_id` of the wrong shape, a spec
 * without `values`, a field the spec schema does not name).
 */
import { randomInt } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { connect as netConnect, type Socket } from 'node:net';
import { join } from 'node:path';

import { until } from './wait.ts';

/** How long one request may wait for its answer. */
const REQUEST_TIMEOUT_MS = 15_000;

/** The alphabet of an id's tail (§4.1): Crockford's base 32, lower-case. */
const ID_ALPHABET = '0123456789abcdefghjkmnpqrstvwxyz';

/**
 * The process the session says it is.
 *
 * The System process: it owns no window and has no parent. The application completes a
 * session's ancestor chain from this pid on Windows (DD-22), and the request sheet then tries
 * to bring the session's terminal forward (OPEN-05) — with the harness's own pid that chain
 * climbs to whatever terminal started the suite, raises it, and takes the focus off the panel
 * the scenario is driving (WIN-03 collapses on exactly that). With this one there is nothing
 * to raise, and the notification of FM-21 is what the application falls back to.
 */
const SESSION_PID = 4;

/** What a scenario chooses about the session it registers. */
export interface SessionOptions {
  /** `images_in_results` of the capability row (§5.6, PREV-04, FM-05). */
  readonly imagesInResults: boolean;
  /** The folder the session works in; its name labels the session in the window. */
  readonly projectDir: string;
}

/** A notification the application sent, as it arrived. */
export interface Notification {
  readonly method: string;
  readonly params: Record<string, unknown>;
}

/** What `handoff.event` carries: the outcome, and the burned PNG beside it in image mode. */
export interface HandoffEvent {
  readonly outcome: Record<string, unknown>;
  readonly image?: string;
}

/** A fresh id with a prefix, in the shape of §4.1. */
function idWith(prefix: string): string {
  let tail = '';
  for (let index = 0; index < 8; index += 1) tail += ID_ALPHABET[randomInt(ID_ALPHABET.length)];
  return `${prefix}${tail}`;
}

/** One connection attempt to a named pipe. */
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

/** A registered session on the product channel. */
export class FakeServer {
  /** The `ses_` reference the application gave this session in its `hello` answer. */
  sessionRef = '';

  private readonly socket: Socket;
  private buffer = '';
  private nextId = 1;
  private readonly pending = new Map<number, { resolve: (value: unknown) => void; reject: (cause: Error) => void }>();
  private readonly received: Notification[] = [];
  private closed: Error | undefined;

  private constructor(socket: Socket) {
    this.socket = socket;
    socket.setEncoding('utf8');
    socket.on('data', (chunk: string) => this.onData(chunk));
    socket.on('error', (cause) => this.onClose(cause));
    socket.on('close', () => this.onClose(new Error('the channel closed')));
  }

  /**
   * Connects and registers, as a server does at session start (§5.3, F-01).
   *
   * A Windows named-pipe client has to retry: the listener creates the next pipe instance
   * only after taking the previous one, so a peer arriving in between is refused rather than
   * queued.
   */
  static async connect(endpoint: string, home: string, options: SessionOptions): Promise<FakeServer> {
    const token = readFileSync(join(home, 'channel.token'), 'utf8').trim();
    const socket = await until('the product channel accepts a connection', async () => {
      try {
        return await connectOnce(endpoint);
      } catch {
        return null;
      }
    });
    const server = new FakeServer(socket);
    const answer = (await server.request('hello', {
      protocol_version: 1,
      token,
      role: 'server',
      server_version: '0.2.0',
      identity: {
        pid: SESSION_PID,
        ppid: 0,
        ancestors: [],
        cwd: options.projectDir,
        project_dir: options.projectDir,
      },
      agent_id: 'claude-code',
      client: { name: 'claude-code', version: '2.1.266' },
      capability_row: {
        agent_id: 'claude-code',
        display_name: 'Claude Code',
        support: 'full',
        images_in_results: options.imagesInResults,
        stop_hook: true,
        tool_timeout_ms: 1_800_000,
      },
    })) as { session_ref: string };
    server.sessionRef = answer.session_ref;
    return server;
  }

  /**
   * Opens a handoff with `spec` and answers its id (§6.3 `handoff.open`).
   *
   * The call stays attached for as long as this connection lives, which is what makes the tab
   * one that is being guided rather than one waiting for the agent's next resume.
   */
  async open(spec: Record<string, unknown>, secretTreated: readonly { location: string; kind: string }[] = []): Promise<string> {
    const answer = (await this.request('handoff.open', {
      call_id: idWith('call_'),
      spec,
      secret_treated: secretTreated,
      request_id: null,
    })) as { handoff_id: string };
    return answer.handoff_id;
  }

  /** Every notification the application has sent so far. */
  notifications(): readonly Notification[] {
    return this.received;
  }

  /** Waits for a `handoff.event` whose outcome has `status`, and answers it. */
  async event(status: string, timeoutMs = 20_000): Promise<HandoffEvent> {
    return until(
      `an outcome with status ${status} reaches the agent`,
      () =>
        (this.received.find(
          (message) =>
            message.method === 'handoff.event' &&
            (message.params['outcome'] as { status?: unknown } | undefined)?.status === status,
        )?.params as HandoffEvent | undefined) ?? null,
      timeoutMs,
    );
  }

  /** Drops the connection, which is how a session ends when its agent leaves (§8.3). */
  close(): void {
    this.socket.destroy();
  }

  private request(method: string, params: Record<string, unknown>): Promise<unknown> {
    if (this.closed !== undefined) return Promise.reject(this.closed);
    const id = this.nextId++;
    const answer = new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} was not answered within ${String(REQUEST_TIMEOUT_MS)} ms`));
      }, REQUEST_TIMEOUT_MS);
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
    this.write({ jsonrpc: '2.0', id, method, params });
    return answer;
  }

  private write(message: Record<string, unknown>): void {
    this.socket.write(`${JSON.stringify(message)}\n`);
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
      method?: string;
      params?: Record<string, unknown>;
      result?: unknown;
      error?: { code: number; message: string };
    };
    try {
      message = JSON.parse(line) as typeof message;
    } catch {
      return;
    }
    if (message.method === 'ping' && message.id !== undefined) {
      // The application closes a peer that stops answering (§6.2); a session that went
      // silent in the middle of a scenario would turn into a detached tab.
      this.write({ jsonrpc: '2.0', id: message.id, result: {} });
      return;
    }
    if (message.method !== undefined) {
      this.received.push({ method: message.method, params: message.params ?? {} });
      return;
    }
    const waiting = typeof message.id === 'number' ? this.pending.get(message.id) : undefined;
    if (waiting === undefined || typeof message.id !== 'number') return;
    this.pending.delete(message.id);
    if (message.error !== undefined) {
      waiting.reject(new Error(`${message.error.message} (code ${String(message.error.code)})`));
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
