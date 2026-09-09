/**
 * Turning a step's text into text and links, safely (GUIDE-03, §7.6).
 *
 * The step text is written by an agent, so it is untrusted input rendered inside our own
 * window: it is never handed to `{@html}`. This function does not produce markup at all — it
 * cuts the text into segments and the component renders each of them, so Svelte escapes
 * every character of the agent's words and an anchor exists only where this function put
 * one.
 *
 * Only `https://` is auto-linked. The other three schemes of SPEC-07 are reachable through
 * the step's own `url` — a declared field the schema validated — and finding
 * `ms-settings:display` inside a sentence would mean guessing where the word ends, which is
 * a guess a clickable target should not be built on. An `http://` link inside a text is left
 * as text for the same reason the design lists it last: it is not what a dashboard hands a
 * user in 2026, and a downgrade offered as a button is a downgrade the window suggested.
 */

/** One piece of a rendered step text. */
export interface Segment {
  /** `text` is printed as it is; `link` becomes an anchor that calls `openUrl`. */
  kind: 'text' | 'link';
  value: string;
}

/**
 * A run of `https://` followed by anything that is not whitespace or a character that
 * could end an anchor. Angle brackets and quotes are excluded so a URL cannot swallow the
 * markup around it in any renderer that ever sees this string.
 */
const HTTPS = /https:\/\/[^\s<>"'`]+/g;

/**
 * Characters that end a sentence rather than a URL.
 *
 * `https://example.test/page.` is a link and a full stop, not a link to `page.`; the same
 * for a URL closing a parenthetical. A closing bracket is only trimmed when the URL carries
 * no matching opening one, so `https://en.wikipedia.org/wiki/Foo_(bar)` survives.
 */
const TRAILING = new Set(['.', ',', ';', ':', '!', '?', "'", '"', '”', '’', '»']);

/** The link `candidate` really is, once the punctuation around it is given back. */
function trim(candidate: string): string {
  let end = candidate.length;
  while (end > 0) {
    const last = candidate[end - 1] ?? '';
    if (TRAILING.has(last)) {
      end -= 1;
      continue;
    }
    if (last === ')' && !unbalanced(candidate.slice(0, end), '(', ')')) {
      end -= 1;
      continue;
    }
    if (last === ']' && !unbalanced(candidate.slice(0, end), '[', ']')) {
      end -= 1;
      continue;
    }
    break;
  }
  return candidate.slice(0, end);
}

/** Whether `text` opens more brackets than it closes, so the last one belongs to the URL. */
function unbalanced(text: string, open: string, close: string): boolean {
  let depth = 0;
  for (const character of text) {
    if (character === open) {
      depth += 1;
    } else if (character === close) {
      depth -= 1;
    }
  }
  return depth >= 0;
}

/**
 * Cuts `text` into the segments a step view renders.
 *
 * Always returns at least one segment for a non-empty text, and the concatenation of every
 * `value` is exactly `text`: nothing of what the agent wrote is dropped, moved or rewritten.
 */
export function linkify(text: string): Segment[] {
  const segments: Segment[] = [];
  let cursor = 0;

  // A fresh regex state per call: the literal carries `g`, and a shared `lastIndex` would
  // make the second call over the same text find nothing.
  HTTPS.lastIndex = 0;
  for (let match = HTTPS.exec(text); match !== null; match = HTTPS.exec(text)) {
    const href = trim(match[0]);
    if (href.length <= 'https://'.length) {
      // `https://` with nothing after it is not a link; leave it in the text.
      continue;
    }
    if (match.index > cursor) {
      segments.push({ kind: 'text', value: text.slice(cursor, match.index) });
    }
    segments.push({ kind: 'link', value: href });
    cursor = match.index + href.length;
    HTTPS.lastIndex = cursor;
  }

  if (cursor < text.length) {
    segments.push({ kind: 'text', value: text.slice(cursor) });
  }
  return segments;
}
