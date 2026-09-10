/**
 * Reading the log while the app holds it, and the invariant of §11.2 (T-043).
 *
 * `node:sqlite` opens a WAL database read-only while another process is writing it, which
 * is the only way to see the rows without stopping the app (the T-032 handoff entry). Every
 * scenario reads two things through here: the rows it asserts on, and the **whole database
 * as text** for the log-invariant check.
 *
 * That check is the "Log invariants" row of §11.2: *after every scenario, no table row
 * contains a value from the spec's `values` or any fixture secret*. It is the one assertion
 * in this suite that is about what the product must **never** do, so it runs after every
 * scenario rather than in the one that seemed relevant — LOG-02 is a property of the log and
 * not of a flow.
 *
 * What it deliberately does not cover is `sends.text_as_sent` and `events.payload_json`:
 * LOG-03 requires the send text to be what actually left, after redaction, because it is the
 * answer to "what did the agent see". The Rust suite `tests/log_invariants.rs` says the same
 * and this is the same rule read from the other side.
 */
import { DatabaseSync } from 'node:sqlite';
import { join } from 'node:path';

import type { Workspace } from './app.ts';

/** The database file, under the run's `HANDOFF_APP_DATA_DIR`. */
export function databaseFile(workspace: Workspace): string {
  return join(workspace.appData, 'handoff.sqlite');
}

/** One handoff row, as the scenarios read it. */
export interface HandoffRow {
  readonly id: string;
  readonly state: string;
  readonly final_state: string | null;
  readonly session_ref: string | null;
  readonly delivered_at: string | null;
  readonly closed_at: string | null;
  readonly request_text: string | null;
  readonly resumed_from_json: string | null;
}

/** A read-only connection to a live database. */
export class Log {
  private readonly db: DatabaseSync;

  constructor(workspace: Workspace) {
    this.db = new DatabaseSync(databaseFile(workspace), { readOnly: true });
  }

  /** Every row of a query, as plain objects. */
  rows<T>(sql: string): T[] {
    return this.db.prepare(sql).all() as T[];
  }

  /** The handoffs, oldest first. */
  handoffs(): HandoffRow[] {
    return this.rows<HandoffRow>(
      'SELECT id, state, final_state, session_ref, delivered_at, closed_at, request_text, ' +
        'resumed_from_json FROM handoffs ORDER BY created_at, id',
    );
  }

  /** One handoff, or nothing. */
  handoff(id: string): HandoffRow | undefined {
    return this.handoffs().find((row) => row.id === id);
  }

  /** The rows the Stop hook wrote (SRV-12: at most one per handoff per session). */
  hookBlocks(): { session_ref: string; item_key: string; at: string }[] {
    return this.rows('SELECT session_ref, item_key, at FROM hook_blocks ORDER BY at');
  }

  /** The rounds of a handoff (VER-09). */
  rounds(handoffId: string): { no: number }[] {
    return this.rows(
      `SELECT no FROM rounds WHERE handoff_id = '${escape(handoffId)}' ORDER BY no`,
    );
  }

  /** The sessions that registered, in registration order. */
  sessions(): { session_ref: string; connected: number; client_name: string | null }[] {
    return this.rows(
      'SELECT session_ref, connected, client_name FROM sessions ORDER BY first_seen, session_ref',
    );
  }

  /** The request queue (§7.7). */
  requests(): {
    id: string;
    text: string;
    linked_handoff_id: string | null;
    about_handoff_id: string | null;
    delivered_via: string | null;
  }[] {
    return this.rows(
      'SELECT id, text, linked_handoff_id, about_handoff_id, delivered_via FROM user_requests ' +
        'ORDER BY created_at, id',
    );
  }

  /**
   * The `sends` rows of a handoff: everything that left towards the agent (LOG-03).
   *
   * The one table with a rule stronger than "record it" — an image is a hash, its
   * dimensions and the boxes that were burned over it, and never pixels. E2E-3 reads it to
   * compare the hash with the bytes the transcript shows the agent was handed.
   */
  sends(handoffId: string): {
    kind: string;
    text_as_sent: string | null;
    image_sha256: string | null;
    image_w: number | null;
    image_h: number | null;
    redaction_boxes_json: string | null;
    ocr_engine: string | null;
    patterns_version: string | null;
  }[] {
    return this.rows(
      'SELECT kind, text_as_sent, image_sha256, image_w, image_h, redaction_boxes_json, ' +
        `ocr_engine, patterns_version FROM sends WHERE handoff_id = '${escape(handoffId)}' ` +
        'ORDER BY id',
    );
  }

  /** The names of every table the migrations created. */
  tables(): string[] {
    return this.rows<{ name: string }>(
      "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    ).map((row) => row.name);
  }

  /**
   * Every text the database holds, as `table.column#row → text`.
   *
   * The tables and the columns are read from the file rather than from a list kept here, so
   * a column added by a later migration is covered the day it exists — the same rule
   * `log::test_support::dump_all_text` follows on the Rust side. An INTEGER column read as
   * text would be an error in `rusqlite`; here everything is coerced, which is what a
   * "does this appear anywhere" question wants.
   */
  dump(): { where: string; text: string }[] {
    const found: { where: string; text: string }[] = [];
    for (const table of this.tables()) {
      const columns = this.rows<{ name: string }>(`PRAGMA table_info(${table})`).map(
        (row) => row.name,
      );
      if (columns.length === 0) continue;
      const rows = this.rows<Record<string, unknown>>(`SELECT * FROM ${table}`);
      rows.forEach((row, index) => {
        for (const column of columns) {
          const value = row[column];
          if (value === null || value === undefined) continue;
          found.push({ where: `${table}.${column}#${String(index)}`, text: String(value) });
        }
      });
    }
    return found;
  }

  close(): void {
    this.db.close();
  }
}

/** Where a forbidden string was found, if anywhere. */
export interface Leak {
  /** The value that should not have been there, abbreviated. */
  readonly needle: string;
  /** `table.column#row`. */
  readonly where: string;
}

/**
 * The log-invariant check of §11.2: none of `forbidden` may appear anywhere in the database.
 *
 * `forbidden` is every value of the scenario's spec (`values`, and the secrets it planted),
 * which the scenario knows because it wrote the spec into the prompt. A short or common
 * string would make this check meaningless, so [`sentinel`] is what a scenario plants and
 * this refuses anything under twelve characters rather than reporting a false clean.
 */
export function leaks(dump: readonly { where: string; text: string }[], forbidden: readonly string[]): Leak[] {
  const found: Leak[] = [];
  for (const needle of forbidden) {
    if (needle.length < 12) {
      throw new Error(
        `the log-invariant check needs a value long enough to be unique; "${needle}" is not`,
      );
    }
    for (const entry of dump) {
      if (entry.text.includes(needle)) {
        found.push({ needle: `${needle.slice(0, 8)}…`, where: entry.where });
      }
    }
  }
  return found;
}

/** A unique, long value a scenario can plant in a spec and then look for in the log. */
export function sentinel(scenario: string, what: string): string {
  return `E2E-${scenario}-${what}-${Math.random().toString(36).slice(2, 10)}`.toUpperCase();
}

/**
 * A value every build of the certain-secret pattern file matches as an `api_key`.
 *
 * `stripe_secret_key` is `[sr]k_(?:live|test)_[0-9A-Za-z]{16,}` (§4.6), so this is masked at
 * ingress (DET-04, §5.5) and must not reach a single row — which is a stronger statement
 * than the one about an ordinary value, and the one LOG-02 is really about.
 */
export function fixtureSecret(tag: string): string {
  const filler = tag.replace(/[^0-9A-Za-z]/gu, '').padEnd(16, 'x').slice(0, 16);
  return `sk_live_${filler}${Math.random().toString(36).slice(2, 8)}`;
}

/** SQLite string escaping, for the two queries that take an id. */
function escape(text: string): string {
  return text.replace(/'/gu, "''");
}
