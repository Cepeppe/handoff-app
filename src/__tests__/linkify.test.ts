/**
 * Auto-linking a step text, safely (GUIDE-03, §7.6).
 *
 * Two properties matter more than any single case and both are asserted below: the
 * concatenation of the segments is exactly the input — nothing an agent wrote is dropped,
 * moved or rewritten — and no scheme but `https` ever becomes a link.
 */
import { describe, expect, it } from 'vitest';

import { linkify, type Segment } from '../overlay/linkify';

/** The text of every segment, joined: it must always be the input again. */
function joined(segments: Segment[]): string {
  return segments.map((segment) => segment.value).join('');
}

/** Just the links. */
function links(text: string): string[] {
  return linkify(text)
    .filter((segment) => segment.kind === 'link')
    .map((segment) => segment.value);
}

describe('linkify', () => {
  it('leaves a text with no link in one piece', () => {
    const segments = linkify('Open the dashboard and add the endpoint.');
    expect(segments).toEqual([
      { kind: 'text', value: 'Open the dashboard and add the endpoint.' },
    ]);
  });

  it('finds an https link and keeps the words around it', () => {
    const text = 'Go to https://dashboard.example.test/webhooks and press Add.';
    expect(links(text)).toEqual(['https://dashboard.example.test/webhooks']);
    expect(joined(linkify(text))).toBe(text);
  });

  it('links every https url in the text', () => {
    const text = 'First https://a.example.test then https://b.example.test/x';
    expect(links(text)).toEqual(['https://a.example.test', 'https://b.example.test/x']);
  });

  it('links no other scheme, whatever SPEC-07 allows for a declared url', () => {
    // `http`, `ms-settings:` and `x-apple.systempreferences:` are openable as a step's own
    // `url`, which the schema validated. Inside free text there is no way to tell where
    // such a token ends, and a clickable target must not be a guess.
    for (const text of [
      'see http://example.test/page',
      'open ms-settings:display and switch it on',
      'open x-apple.systempreferences:com.apple.preference.security',
      'the file is at file:///etc/passwd',
      'javascript:alert(1) is not a link',
      'data:text/html,<script>alert(1)</script>',
    ]) {
      expect(links(text)).toEqual([]);
      expect(joined(linkify(text))).toBe(text);
    }
  });

  it('gives the sentence its punctuation back', () => {
    expect(links('go to https://example.test/page.')).toEqual(['https://example.test/page']);
    expect(links('either https://a.example.test, or nothing')).toEqual(['https://a.example.test']);
    expect(links('(see https://example.test/page)')).toEqual(['https://example.test/page']);
    expect(links('read https://en.example.test/wiki/Foo_(bar)')).toEqual([
      'https://en.example.test/wiki/Foo_(bar)',
    ]);
  });

  it('never swallows the markup around it', () => {
    // The value is escaped by Svelte wherever it is rendered, and it is never given to
    // `{@html}`; this keeps a URL from reaching for what follows it in any case.
    expect(links('<a href="https://evil.example.test">x</a>')).toEqual([
      'https://evil.example.test',
    ]);
  });

  it('is not a link when there is nothing after the scheme', () => {
    expect(links('the prefix https:// alone')).toEqual([]);
  });

  it('gives the same answer every time it is asked', () => {
    // The pattern is a module-level literal with the `g` flag; a shared `lastIndex` would
    // make the second call over the same text find nothing.
    const text = 'go to https://example.test/page';
    expect(links(text)).toEqual(links(text));
  });

  it('keeps nothing at all for an empty text', () => {
    expect(linkify('')).toEqual([]);
  });
});
