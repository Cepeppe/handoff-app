/**
 * E2E-3 — a screenshot with a fake Stripe key (§11.5, F-04, F-12, PREV-01..05, LOG-03).
 *
 * The user interrupts the guidance with a picture of what they are looking at. The whole
 * pipeline runs for real: the capture (over a fixture image, `capture::fake`), the OCR of
 * §7.9, both detectors of §7.10, the boxes, the burn-in of CAP-06, the outcome, the image
 * block of §4.3 and the `sends` row of §7.11. §11.5 asks for four things and this asserts
 * all four:
 *
 * 1. **the image block is present in the agent's tool result** — A-07, and the only place
 *    it can be seen is the transcript;
 * 2. **the agent describes the visible non-secret text** — the model half, so it is
 *    classified as one: a sampling that answers vaguely is an alert, not a gate;
 * 3. **the key region is black: an OCR of the sent PNG finds no certain pattern** — done
 *    inside the app, on the bytes that really left, because the OCR engines are there and
 *    not here. `certainBefore` is its control and is reported as a note: on a machine whose
 *    engine cannot read the planted key even before the burn, the check is vacuous and says
 *    so rather than passing quietly (the rule §11.7 already took for the glyph-leak pass);
 * 4. **the log has the hash and no pixels** — LOG-03, and the hash is compared with a
 *    digest of the base64 the *agent* was handed, which is what ties the two ends together.
 *
 * The fixture is a page of the committed corpus (§11.7): a Stripe webhook screen carrying
 * `whsec_…`, drawn by a program, with no real screen and no issued credential in it.
 */
import { createHash } from 'node:crypto';

import { callsTo, startAgent, statuses, type AgentRun } from '../agent.ts';
import type { HandoffState } from '../automation.ts';
import { check, note, type Assertion } from '../classify.ts';
import { Log } from '../db.ts';
import { screenshotFixture, screenshotLabels } from '../paths.ts';
import { handoffOf, openPrompt, plantedSpec, REPORT_LINE, walkTheSteps } from '../specs.ts';
import { serverRegistered, wellFormed, type Scenario } from '../scenario.ts';

/** The corpus page the user "sees": a Stripe webhook screen with a fake signing secret. */
export const FIXTURE = 'certain-05-stripe-webhook-secret.png';

/** What the user types beside the picture; it becomes `user_text` (RESP-04, CTX-01). */
export const COMMENT = 'This is the page I am on. Which of these do I need?';

/** Words of the fixture that are **not** secret, one of which the agent should name. */
const VISIBLE_WORDS = ['webhook', 'endpoint', 'signing', 'payment', 'event', 'stripe'];

/** The limits of A-20, asserted on the PNG that actually left. */
const MAX_BYTES = 5 * 1024 * 1024;
const MAX_SIDE = 8000;

export const screenshotScenario: Scenario = {
  id: 'e2e-03-screenshot',
  covers: 'E2E-3',
  title: 'a screenshot reaches the agent as an image with the secret burned out of it',

  async run({ workspace, app, forbidden, facts, say }) {
    const planted = plantedSpec('03');
    const key = plantedKey();
    // The fixture's own key must not reach the log either: it is on the screen the user
    // sent, so it is in the OCR text, and LOG-03 keeps neither the pixels nor the text of
    // an image send.
    forbidden.push(...planted.forbidden, key);

    const prompt = [
      openPrompt(planted.spec),
      'When the call returns with status "screenshot", the user has sent you a picture of the screen they are on.',
      'Look at it and answer by calling mcp__handoff__handoff_to_user again with',
      '{"handoff_id": "<the handoff_id>", "reply": "<one sentence naming what you can see on that screen>"}.',
      'Name what is written on it. Then keep waiting: the call blocks again.',
      'Repeat until the status you get back has "final": true.',
      REPORT_LINE,
    ].join(' ');

    say('starting the agent');
    const agent = startAgent(workspace, { prompt, maxTurns: 12 });

    const handoff = await app.theHandoff();
    const id = handoff.tab.id;
    say(`the agent opened ${id}; sending a screenshot from step 1`);

    await app.waitFor(`${id} is being guided`, (seen) => handoffOf(seen, id)?.state === 'active');
    const sent = await app.screenshotFixture(id, {
      path: screenshotFixture(FIXTURE),
      mode: 'image',
      comment: COMMENT,
    });
    facts['sent'] = sent;

    say('waiting for the agent to answer what it saw');
    const answered = await app.waitFor(
      'the reply reaches the step',
      (seen) => repliesOf(handoffOf(seen, id)).length > 0,
      180_000,
    );
    const replies = repliesOf(handoffOf(answered, id));
    facts['replies'] = replies;

    await walkTheSteps(app, id, say);
    await app.waitFor(
      `${id} reached a final state`,
      (seen) => handoffOf(seen, id)?.closedAt != null,
      120_000,
    );
    const run = await agent;
    facts['statuses'] = statuses(run);

    const log = new Log(workspace);
    const sends = log.sends(id);
    log.close();
    const row = sends.find((send) => send.kind === 'screenshot_image');
    facts['send'] = row;

    const outcome = screenshotOutcomeOf(run);
    const image = imageOf(run);
    const digest =
      image === undefined
        ? undefined
        : createHash('sha256').update(Buffer.from(image.data, 'base64')).digest('hex');
    facts['imageBytes'] = image === undefined ? null : Buffer.from(image.data, 'base64').length;

    return [
      wellFormed(run, 'E2E-3'),
      serverRegistered(run, 'E2E-3'),
      check(
        'E2E-3',
        'the blocking call comes back with a screenshot outcome that says an image is attached (§4.3)',
        'protocol',
        outcome !== undefined &&
          shotOf(outcome)?.['mode'] === 'image' &&
          shotOf(outcome)?.['image_attached'] === true,
        `statuses: ${JSON.stringify(statuses(run))}; screenshot: ${JSON.stringify(
          outcome === undefined ? null : shotOf(outcome),
        )}`,
      ),
      check(
        'E2E-3',
        'the outcome carries the comment and the handoff context the agent needs (CTX-01)',
        'protocol',
        outcome?.['user_text'] === COMMENT && contextOf(outcome) !== undefined,
        `user_text: ${JSON.stringify(outcome?.['user_text'])}; context keys: ${JSON.stringify(
          Object.keys(contextOf(outcome) ?? {}),
        )}`,
      ),
      check(
        'E2E-3',
        'an image block reaches the agent beside the outcome (A-07, §4.3)',
        'protocol',
        image !== undefined && image.mediaType === 'image/png' && image.data.length > 0,
        `images per tool result: ${JSON.stringify(run.toolResults.map((result) => result.images.length))}`,
      ),
      check(
        'E2E-3',
        'the bytes the agent was handed are the bytes the app burned (LOG-03)',
        'protocol',
        digest !== undefined && row?.image_sha256 === digest,
        `sends.image_sha256: ${JSON.stringify(row?.image_sha256)}; digest of the image block: ${JSON.stringify(digest)}`,
      ),
      check(
        'E2E-3',
        'an OCR of the sent PNG finds no certain pattern: the key region is black (R-06)',
        'protocol',
        Array.isArray(sent.certainAfter) && sent.certainAfter.length === 0,
        `before the burn: ${JSON.stringify(sent.certainBefore)}; after it: ${JSON.stringify(sent.certainAfter)}; engine: ${JSON.stringify(sent.ocrEngine)}`,
      ),
      note(
        'E2E-3',
        "the planted key was legible to this machine's engine before the burn, so the check above means something",
        sent.certainBefore.length > 0,
        `${FIXTURE} carries ${key.slice(0, 6)}…; the detectors found ${JSON.stringify(sent.certainBefore)} in the capture`,
      ),
      check(
        'E2E-3',
        'the log holds the hash, the size and the boxes, and no pixels (LOG-03)',
        'protocol',
        row !== undefined &&
          typeof row.image_sha256 === 'string' &&
          /^[0-9a-f]{64}$/u.test(row.image_sha256) &&
          row.image_w !== null &&
          row.image_h !== null &&
          row.redaction_boxes_json !== null &&
          row.ocr_engine !== null &&
          row.patterns_version !== null &&
          row.text_as_sent === COMMENT,
        `sends row: ${JSON.stringify(row)}`,
      ),
      check(
        'E2E-3',
        'the PNG is inside the limits a model result has to fit (A-20)',
        'protocol',
        sent.bytes !== null &&
          sent.bytes <= MAX_BYTES &&
          Math.max(sent.width, sent.height) <= MAX_SIDE,
        `${JSON.stringify(sent.bytes)} bytes, ${String(sent.width)} × ${String(sent.height)} px`,
      ),
      check(
        'E2E-3',
        'the agent describes the non-secret text of the screen it was sent',
        'model',
        replies.some((reply) =>
          VISIBLE_WORDS.some((word) => reply.toLowerCase().includes(word)),
        ),
        `replies: ${JSON.stringify(replies)}`,
      ),
      check(
        'E2E-3',
        'the agent answers on the same step, without opening a round (VER-09)',
        'model',
        callsTo(run, 'handoff_to_user').some(
          (use) => typeof use.input['reply'] === 'string' && use.input['handoff_id'] === id,
        ),
        `calls: ${JSON.stringify(callsTo(run, 'handoff_to_user').map((use) => Object.keys(use.input)))}`,
      ),
    ] satisfies Assertion[];
  },
};

/** The fake Stripe secret the fixture page carries, read from the corpus's own labels. */
function plantedKey(): string {
  const page = screenshotLabels().images.find((image) => image.file === FIXTURE);
  const line = page?.lines.find((entry) => entry.expect.level === 'certain');
  const found = /whsec_[0-9A-Za-z]+/u.exec(line?.text ?? '');
  if (found === null) {
    throw new Error(`${FIXTURE} no longer carries the planted key E2E-3 is written around`);
  }
  return found[0];
}

/** The replies drawn on the current step. */
function repliesOf(handoff: HandoffState | undefined): string[] {
  return (handoff?.step?.replies ?? []).map((reply) => reply.text);
}

/** The `screenshot` outcome the agent was handed, if any. */
function screenshotOutcomeOf(run: AgentRun): Record<string, unknown> | undefined {
  return run.toolResults
    .map((result) => result.outcome)
    .find((outcome) => outcome?.['status'] === 'screenshot');
}

/** The `screenshot` block of an outcome. */
function shotOf(outcome: Record<string, unknown>): Record<string, unknown> | undefined {
  const shot = outcome['screenshot'];
  return typeof shot === 'object' && shot !== null ? (shot as Record<string, unknown>) : undefined;
}

/** The `context` block of an outcome (CTX-01). */
function contextOf(
  outcome: Record<string, unknown> | undefined,
): Record<string, unknown> | undefined {
  const context = outcome?.['context'];
  return typeof context === 'object' && context !== null
    ? (context as Record<string, unknown>)
    : undefined;
}

/** The first image block of the run, which is the screenshot the user sent. */
function imageOf(run: AgentRun): { mediaType: string; data: string } | undefined {
  for (const result of run.toolResults) {
    const [image] = result.images;
    if (image !== undefined) return image;
  }
  return undefined;
}
