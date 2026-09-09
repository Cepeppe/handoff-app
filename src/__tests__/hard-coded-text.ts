/**
 * The scanner behind the "no hard-coded strings" lint of `locales.test.ts` (APP-02).
 *
 * `src/locales/{en,it}.json` are the only place a user-visible text is written, on both
 * sides of the application. That rule is easy to state and easy to break by accident — one
 * `<p>Loading…</p>`, one `aria-label="Close"` — and a key-parity test cannot see any of it,
 * because a text nobody put in the catalogue has no key to be missing.
 *
 * So this walks the components themselves and reports three shapes, which are the three
 * ways a sentence reaches a person from a `.svelte` file:
 *
 * 1. **A text node** in the markup. Everything drawn is an expression: `{t('…')}`, the
 *    user's own words, the agent's. Bare letters between two tags are a text nobody can
 *    translate.
 * 2. **A literal attribute** whose name is not in {@link SYMBOL_ATTRIBUTES}. That list is
 *    the "ids and symbols" allowance: `class`, `type`, `role`, `data-*` and their kind carry
 *    identifiers, never prose, and their values are the same in every language. Anything
 *    else — `aria-label`, `title`, `placeholder`, `alt` — is read out or shown.
 * 3. **A sentence in the script**, assigned to something the markup later draws. A literal
 *    with a space and three letters in it is prose; anything shorter is indistinguishable
 *    from an identifier (`'Control+Alt+H'`, `'not_registered'`), and the two rules above are
 *    what cover a one-word text once it is drawn.
 *
 * It is a hand-written walk rather than a set of regular expressions because a Svelte file
 * is not line-shaped: `{#each xs as x (x.id)}`, `onclick={() => (open = !open)}` and
 * `class:done={x}` all put braces, quotes and `>` where a regular expression would stop.
 * Comments — the HTML kind, the line kind and the block kind — are removed first, because a
 * rule that fires on the prose in a doc comment is a rule people switch off.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

/** One thing this scanner refuses, with the text that triggered it. */
export interface HardCodedText {
  /** Which of the three shapes it is, for the failure message. */
  what: 'a text node' | 'an attribute' | 'a sentence in the script';
  /** The offending text, trimmed. */
  text: string;
}

/**
 * Attribute names whose values are identifiers or symbols, never prose.
 *
 * Everything not on this list has to be an expression when it carries letters. The list is
 * deliberately a list of *names* and not a list of allowed values: adding a value to an
 * allowlist is how a lint stops meaning anything.
 */
export const SYMBOL_ATTRIBUTES: ReadonlySet<string> = new Set([
  'class',
  'id',
  'for',
  'name',
  'type',
  'role',
  'href',
  'src',
  'rel',
  'target',
  'lang',
  'dir',
  'slot',
  'value',
  'rows',
  'cols',
  'maxlength',
  'minlength',
  'inputmode',
  'enterkeyhint',
  'autocomplete',
  'autocapitalize',
  'spellcheck',
  'tabindex',
  'contenteditable',
  'draggable',
  'hidden',
  'disabled',
  'checked',
  'readonly',
  'required',
  'multiple',
  'open',
  'aria-current',
  'aria-expanded',
  'aria-hidden',
  'aria-live',
  'aria-atomic',
  'aria-checked',
  'aria-disabled',
  'aria-selected',
  'aria-pressed',
  'aria-controls',
  'aria-labelledby',
  'aria-describedby',
]);

/**
 * Whether an attribute's value is an identifier or a symbol rather than something read.
 *
 * Beside the list: `data-*` is a marker a test or a stylesheet reads, a Svelte directive
 * (`class:`, `bind:`, `use:`) names a binding, and an `on*` handler is code. None of the
 * three carries a language.
 */
function isSymbolAttribute(name: string): boolean {
  return (
    SYMBOL_ATTRIBUTES.has(name) ||
    name.startsWith('data-') ||
    name.startsWith('on') ||
    name.includes(':')
  );
}

/** The escape character, built rather than written: a literal one is fragile in tooling. */
const ESCAPE = String.fromCharCode(92);

const QUOTES: ReadonlySet<string> = new Set(['"', "'", '`']);

/**
 * The characters after which a `/` opens a regular expression rather than dividing.
 *
 * The usual heuristic, and enough here: what it protects is the comment stripper, which
 * would otherwise read the inside of `/[^'"]/` as an unterminated string.
 */
const VALUE_POSITION: ReadonlySet<string> = new Set([
  '(',
  ',',
  '=',
  ':',
  '[',
  '!',
  '&',
  '|',
  '?',
  '{',
  '}',
  ';',
  '+',
  '-',
  '*',
  '%',
  '<',
  '>',
  '~',
  '^',
]);

/** Where a quoted run ends, escapes honoured. Points past the closing quote. */
function endOfQuoted(text: string, at: number): number {
  const quote = text[at];
  let i = at + 1;
  while (i < text.length) {
    if (text[i] === ESCAPE) {
      i += 2;
      continue;
    }
    if (text[i] === quote) {
      return i + 1;
    }
    i += 1;
  }
  return text.length;
}

/** Where a `{…}` expression ends, quotes and nesting honoured. Points past the `}`. */
function endOfExpression(text: string, at: number): number {
  let depth = 0;
  let i = at;
  while (i < text.length) {
    const ch = text[i];
    if (QUOTES.has(ch)) {
      i = endOfQuoted(text, i);
      continue;
    }
    if (ch === '{') {
      depth += 1;
      i += 1;
      continue;
    }
    if (ch === '}') {
      depth -= 1;
      i += 1;
      if (depth === 0) {
        return i;
      }
      continue;
    }
    i += 1;
  }
  return text.length;
}

/** One attribute of a tag: its name, and its value when the value is a literal. */
interface Attribute {
  name: string;
  literal: string | null;
}

/** The attributes of the tag starting at `at`, and where the tag ends. */
function readTag(text: string, at: number): { end: number; attributes: Attribute[] } {
  const attributes: Attribute[] = [];
  let i = at + 1;
  if (text[i] === '/') {
    i += 1;
  }
  while (i < text.length && /[^\s/>]/.test(text[i] ?? '>')) {
    i += 1;
  }
  while (i < text.length) {
    while (i < text.length && /\s/.test(text[i] ?? 'x')) {
      i += 1;
    }
    if (i >= text.length || text[i] === '>') {
      return { end: i + 1, attributes };
    }
    if (text[i] === '/' && text[i + 1] === '>') {
      return { end: i + 2, attributes };
    }
    if (text[i] === '{') {
      // A spread or a shorthand: `{...rest}`, `{disabled}`. Nothing literal in it.
      i = endOfExpression(text, i);
      continue;
    }
    const nameStart = i;
    while (i < text.length && /[^\s=/>]/.test(text[i] ?? '>')) {
      i += 1;
    }
    const name = text.slice(nameStart, i);
    if (name === '') {
      // Nothing consumed: advance so the walk always makes progress.
      i += 1;
      continue;
    }
    while (i < text.length && /\s/.test(text[i] ?? 'x')) {
      i += 1;
    }
    if (text[i] !== '=') {
      attributes.push({ name, literal: null });
      continue;
    }
    i += 1;
    while (i < text.length && /\s/.test(text[i] ?? 'x')) {
      i += 1;
    }
    const value = text[i] ?? '>';
    if (QUOTES.has(value)) {
      const end = endOfQuoted(text, i);
      attributes.push({ name, literal: text.slice(i + 1, end - 1) });
      i = end;
    } else if (value === '{') {
      attributes.push({ name, literal: null });
      i = endOfExpression(text, i);
    } else {
      const start = i;
      while (i < text.length && /[^\s>]/.test(text[i] ?? '>')) {
        i += 1;
      }
      attributes.push({ name, literal: text.slice(start, i) });
    }
  }
  return { end: i, attributes };
}

/** The `<script>` bodies of a component, joined. */
function scriptsOf(source: string): string {
  return [...source.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/gi)]
    .map((match) => match[1] ?? '')
    .join('\n');
}

/** The string literals of a script, with comments and regular expressions left out. */
function literalsOf(script: string): string[] {
  const literals: string[] = [];
  let significant = '';
  let i = 0;
  while (i < script.length) {
    const ch = script[i] as string;
    if (ch === "'" || ch === '"') {
      const end = endOfQuoted(script, i);
      literals.push(script.slice(i + 1, end - 1));
      significant = ch;
      i = end;
      continue;
    }
    if (ch === '`') {
      // A template literal is skipped rather than read: what it holds is nearly always an
      // interpolation, and its own text is caught by the two markup rules once drawn.
      i = endOfQuoted(script, i);
      significant = ch;
      continue;
    }
    if (ch === '/' && script[i + 1] === '/') {
      while (i < script.length && script[i] !== '\n') {
        i += 1;
      }
      continue;
    }
    if (ch === '/' && script[i + 1] === '*') {
      const end = script.indexOf('*/', i + 2);
      i = end === -1 ? script.length : end + 2;
      continue;
    }
    if (ch === '/' && (significant === '' || VALUE_POSITION.has(significant))) {
      i = endOfRegex(script, i);
      significant = '/';
      continue;
    }
    if (!/\s/.test(ch)) {
      significant = ch;
    }
    i += 1;
  }
  return literals;
}

/** Where a regular-expression literal ends, character classes and escapes honoured. */
function endOfRegex(script: string, at: number): number {
  let inClass = false;
  let i = at + 1;
  while (i < script.length) {
    const ch = script[i];
    if (ch === ESCAPE) {
      i += 2;
      continue;
    }
    if (ch === '\n') {
      // Not a regular expression after all; treat the `/` as one character.
      return at + 1;
    }
    if (ch === '[') {
      inClass = true;
    } else if (ch === ']') {
      inClass = false;
    } else if (ch === '/' && !inClass) {
      return i + 1;
    }
    i += 1;
  }
  return script.length;
}

/** Two or more letters in a row: what makes a run of characters a word rather than a symbol. */
const WORD = /\p{L}{2,}/u;

/** A space and three letters: what makes a string literal prose rather than an identifier. */
function isSentence(literal: string): boolean {
  return literal.includes(' ') && (literal.match(/\p{L}/gu)?.length ?? 0) >= 3;
}

/** Every hard-coded user-visible text of one component. */
export function hardCodedText(source: string): HardCodedText[] {
  const markup = source
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<script[\s\S]*?<\/script>/gi, '')
    .replace(/<style[\s\S]*?<\/style>/gi, '');

  const found: HardCodedText[] = [];
  let text = '';
  let i = 0;
  while (i < markup.length) {
    const ch = markup[i] as string;
    if (ch === '<') {
      if (WORD.test(text)) {
        found.push({ what: 'a text node', text: text.trim() });
      }
      text = '';
      const tag = readTag(markup, i);
      for (const attribute of tag.attributes) {
        if (
          attribute.literal !== null &&
          !isSymbolAttribute(attribute.name) &&
          WORD.test(attribute.literal)
        ) {
          found.push({ what: 'an attribute', text: `${attribute.name}="${attribute.literal}"` });
        }
      }
      i = tag.end;
      continue;
    }
    if (ch === '{') {
      i = endOfExpression(markup, i);
      continue;
    }
    text += ch;
    i += 1;
  }
  if (WORD.test(text)) {
    found.push({ what: 'a text node', text: text.trim() });
  }

  for (const literal of literalsOf(scriptsOf(source))) {
    if (isSentence(literal)) {
      found.push({ what: 'a sentence in the script', text: literal });
    }
  }

  return found;
}

/** Every `.svelte` file under `dir`, tests excluded: they name what they check. */
export function svelteFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      return entry === '__tests__' ? [] : svelteFiles(path);
    }
    return entry.endsWith('.svelte') ? [path] : [];
  });
}

/** `hardCodedText` over every component, each offender named with its file. */
export function hardCodedTextIn(dir: string): string[] {
  return svelteFiles(dir).flatMap((path) =>
    hardCodedText(readFileSync(path, 'utf8')).map(
      (offender) => `${path}: ${offender.what}, ${JSON.stringify(offender.text)}`,
    ),
  );
}
