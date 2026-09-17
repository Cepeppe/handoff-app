/**
 * The specs the scenarios hand to the agent, and the way the user is played (T-043, §11.5).
 *
 * Two rules run through the file.
 *
 * **The spec is written here, not by the model.** §11.5's scenarios are about the system —
 * the server, the channel, the store, the overlay, the hook — and a run in which the model
 * drafted its own spec would fail whenever the drafting was poor and tell nobody anything
 * about the system. The prompt carries the spec verbatim and says so.
 *
 * **Every spec plants a fixture secret**, in a value and in a step's text, because the
 * log-invariant check of §11.2 runs after every scenario and needs something to look for.
 * `sk_live_…` is what `stripe_secret_key` matches (§4.6), so the certain detector masks it
 * at ingress (§5.5) and no row of the log may hold it afterwards (LOG-02, DET-04).
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { CLAUDE_CODE, type AgentRunner } from './agent.ts';
import type { Automation, HandoffState } from './automation.ts';
import { fixtureSecret, sentinel } from './db.ts';
import { REPO_ROOT } from './paths.ts';

/** A spec and the values the log must never hold afterwards. */
export interface PlantedSpec {
  /** The spec, exactly as the prompt will carry it. */
  readonly spec: Record<string, unknown>;
  /** What the log-invariant check looks for (§11.2). */
  readonly forbidden: string[];
  /** How many steps the user has to walk. */
  readonly steps: number;
}

/**
 * The spec every scenario opens with: two steps, one plain value and one certain secret.
 *
 * It reads as ordinary work — enabling a maintenance banner in an admin console — because a
 * prompt that reads as a test invites the model to treat it as one.
 */
export function plantedSpec(
  scenario: string,
  options: { readonly verify?: string; readonly goal?: string } = {},
): PlantedSpec {
  const secret = fixtureSecret(scenario);
  const inText = fixtureSecret(`${scenario}step`);
  const banner = sentinel(scenario, 'banner');
  return {
    spec: {
      spec_version: 1,
      goal: options.goal ?? 'Enable the maintenance banner in the admin console',
      where: 'Admin console → Settings → Announcements',
      why_human: 'Only an account administrator can publish an announcement.',
      values: { banner_text: banner, api_key: secret },
      steps: [
        { text: 'Open Settings and then Announcements.' },
        {
          text: `Paste the banner text into the message field, authenticating with ${inText} if asked.`,
          values: ['banner_text'],
        },
      ],
      ...(options.verify === undefined ? {} : { verify: options.verify }),
    },
    // The plain `banner_text` is deliberately **not** forbidden: §7.11 stores `spec_json`
    // with the secret-treated values replaced by placeholders, and the Log page of §7.11 is
    // what the rest of the spec is stored for. What LOG-02 and DET-04 forbid is the value
    // the certain detector matched, and that is what this list holds, departing on purpose
    // from the wording of §11.2.
    forbidden: [secret, inText],
    steps: 2,
  };
}

/** The verification sentence a scenario with a `verify` uses. */
export const VERIFY_TEXT = 'The banner is listed as published in the announcements table.';

/**
 * The phrase only the no-hook variant of a `deferred` or `parked` instruction carries
 * (§4.7.4, the tool contract's `no_stop_hook` rows): the server picks the variant from the
 * `stop_hook` of the session's capability row.
 */
export const NO_HOOK_PHRASE = 'Nothing will remind you';

/**
 * A text of the English catalogue, `src/locales/en.json`.
 *
 * `state()` answers with the view the window draws, and the view carries catalogue keys; this
 * is how a scenario reads the sentence a person would be shown for one of them.
 */
export function catalogueText(key: string): string | undefined {
  const catalogue = JSON.parse(
    readFileSync(join(REPO_ROOT, 'src', 'locales', 'en.json'), 'utf8'),
  ) as Record<string, string>;
  return catalogue[key];
}

/** The prompt preamble every scenario shares: call the tool once, with this exact spec. */
export function openPrompt(
  spec: Record<string, unknown>,
  agent: Pick<AgentRunner, 'tool'> = CLAUDE_CODE,
): string {
  return [
    'You are the coding agent in an automated end-to-end test of a handoff system.',
    `Call the tool ${agent.tool('handoff_to_user')} exactly once with this exact argument,`,
    'copied verbatim and not modified in any way:',
    JSON.stringify({ spec }),
    'The call blocks while a person works through the steps; that is expected and you must wait for it.',
  ].join(' ');
}

/** The last line every prompt asks for, which is what a run is read back through. */
export const REPORT_LINE =
  'When you have nothing left to do, print exactly one line and nothing else, in this form: ' +
  'STATUS=<the status of the last outcome you received> ID=<the handoff_id you saw>.';

/** Walks the user through the steps of the current round: confirm each, done on the last. */
export async function walkTheSteps(
  app: Automation,
  handoffId: string,
  say: (what: string) => void,
): Promise<void> {
  for (;;) {
    const state = await app.waitFor(
      `${handoffId} is being guided`,
      (seen) => stepOf(seen, handoffId) !== undefined,
      120_000,
    );
    const step = stepOf(state, handoffId);
    if (step === undefined) return;
    if (step.index < step.total) {
      say(`confirming step ${String(step.index)} of ${String(step.total)}`);
      await app.act(handoffId, 'confirm');
    } else {
      say(`done on step ${String(step.index)} of ${String(step.total)}`);
      await app.act(handoffId, 'done');
      return;
    }
  }
}

/** The step the user is on, when the handoff is showing one. */
export function stepOf(
  state: { readonly handoffs: readonly HandoffState[] },
  handoffId: string,
): { index: number; total: number } | undefined {
  const handoff = handoffOf(state, handoffId);
  if (handoff?.step === undefined || handoff.step === null) return undefined;
  if (handoff.state !== 'active') return undefined;
  return { index: handoff.step.counter.index, total: handoff.step.counter.total };
}

/** One handoff of a state, by id. */
export function handoffOf(
  state: { readonly handoffs: readonly HandoffState[] },
  handoffId: string,
): HandoffState | undefined {
  return state.handoffs.find((handoff) => handoff.tab.id === handoffId);
}

/** The state of one handoff, by id, or `"absent"`. */
export function stateOf(
  state: { readonly handoffs: readonly HandoffState[] },
  handoffId: string,
): string {
  return handoffOf(state, handoffId)?.state ?? 'absent';
}
