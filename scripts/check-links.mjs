#!/usr/bin/env node
/**
 * Checks that every relative link in the repository's Markdown resolves.
 *
 * The user documentation of `docs/` (NET-02, NFR-09, NFR-16) is a set of pages in two
 * languages that link to each other and to files of the repository. A renamed page or a
 * mistyped anchor turns one of those links into a dead end nobody notices until a reader hits
 * it, so `pnpm check:links` runs in CI and fails the build instead.
 *
 * Copied from `handoff-mcp/build/check-links.mjs` (T-022) rather than rewritten: the two
 * repositories publish documentation under the same rules, and only the folders to ignore
 * differ (`target/` is Cargo's build output here).
 *
 * What it checks, for every `.md` file outside the ignored folders:
 *   - inline links and images, reference definitions and autolinks;
 *   - the target exists on disk, resolved relative to the file that links to it;
 *   - when the target is a Markdown file, the `#anchor` matches one of its headings.
 * Links inside fenced code blocks and inline code spans are examples, not links, and are
 * skipped. External links (any scheme, and protocol-relative ones) are not fetched: this
 * check is about the repository's own consistency, and a network call in CI would make it
 * flaky.
 *
 * Usage: `node scripts/check-links.mjs [root]`. Exits 0 when everything resolves, 1 with one
 * line per problem otherwise.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Folders that hold no documentation of ours, or hold generated copies of it. */
const IGNORED_DIRS = new Set(['node_modules', 'dist', 'coverage', '.git', 'vendor', 'target']);

/** Anything with a scheme, or protocol-relative, is somebody else's to keep alive. */
const HAS_SCHEME = /^[a-z][a-z0-9+.-]*:/i;

/** Lists every Markdown file under `dir`, depth first, in a stable order. */
function markdownFiles(dir) {
  const found = [];
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) =>
    a.name < b.name ? -1 : 1,
  )) {
    if (entry.isDirectory()) {
      if (!IGNORED_DIRS.has(entry.name)) found.push(...markdownFiles(join(dir, entry.name)));
    } else if (entry.isFile() && entry.name.endsWith('.md')) {
      found.push(join(dir, entry.name));
    }
  }
  return found;
}

/**
 * Blanks out fenced code blocks, keeping the line count intact so that a problem still
 * reports the line it is on. A link or a heading inside a fence is an example.
 */
function withoutFences(source) {
  let fence = null;
  return source
    .split('\n')
    .map((line) => {
      const marker = /^\s{0,3}(```+|~~~+)/.exec(line);
      if (fence === null) {
        if (!marker) return line;
        fence = marker[1];
        return '';
      }
      if (marker && marker[1].startsWith(fence[0].repeat(fence.length))) fence = null;
      return '';
    })
    .join('\n');
}

/**
 * The same, plus inline code spans: `[x](y)` inside backticks is prose about a link, not a
 * link. Headings keep their code spans, because their slug does.
 */
function withoutCode(source) {
  return withoutFences(source)
    .split('\n')
    .map((line) => line.replace(/`[^`]*`/g, (span) => ' '.repeat(span.length)))
    .join('\n');
}

/**
 * Every link of one file: `[text](target)`, `![alt](target)`, `[label]: target` and
 * `<relative/target>`, each with the 1-based line it sits on.
 */
function linksOf(source) {
  const text = withoutCode(source);
  const found = [];
  const add = (index, target) => {
    const line = text.slice(0, index).split('\n').length;
    found.push({ target: target.trim(), line });
  };

  // [text](target "title") and ![alt](target); the target may be wrapped in <>.
  const inline = /!?\[(?:[^\]\\]|\\.)*\]\(\s*(<[^>]*>|[^\s()]*)(?:\s+(?:"[^"]*"|'[^']*'))?\s*\)/g;
  for (let m = inline.exec(text); m; m = inline.exec(text)) add(m.index, m[1]);

  // [label]: target "title", at the start of a line.
  const definition = /^[ \t]{0,3}\[(?:[^\]\\]|\\.)+\]:[ \t]*(<[^>]*>|\S+)/gm;
  for (let m = definition.exec(text); m; m = definition.exec(text)) add(m.index, m[1]);

  // <target> autolinks, but only relative ones: <https://…> and <a@b> are not ours.
  const autolink = /<([^\s<>]+\.md(?:#[^\s<>]*)?)>/g;
  for (let m = autolink.exec(text); m; m = autolink.exec(text)) add(m.index, m[1]);

  return found;
}

/**
 * GitHub's heading slug: the rendered text, lowercased, with punctuation dropped and runs
 * of whitespace turned into hyphens; a repeated slug gets `-1`, `-2` and so on.
 */
function anchorsOf(source) {
  const text = withoutFences(source);
  const seen = new Map();
  const anchors = new Set();
  for (const line of text.split('\n')) {
    const heading = /^ {0,3}(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (!heading) continue;
    const slug = slugify(
      heading[2]
        .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1') // a link in a heading slugs its text
        .replace(/[*~]/g, ''), // emphasis markers; `_` stays, it is part of identifiers
    );
    if (slug === '') continue;
    const count = seen.get(slug) ?? 0;
    seen.set(slug, count + 1);
    anchors.add(count === 0 ? slug : `${slug}-${count}`);
  }
  return anchors;
}

function slugify(heading) {
  return heading
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\p{M}\s_-]/gu, '')
    .trim()
    .replace(/\s+/g, '-');
}

/** Checks one repository and returns one message per problem, empty when all is well. */
export function checkLinks(root) {
  const problems = [];
  const files = markdownFiles(root);
  const anchorCache = new Map();
  let checked = 0;

  const anchorsFor = (path) => {
    if (!anchorCache.has(path)) anchorCache.set(path, anchorsOf(readFileSync(path, 'utf8')));
    return anchorCache.get(path);
  };

  for (const file of files) {
    const where = relative(root, file).split(sep).join('/');
    const report = (line, message) => problems.push(`${where}:${line}: ${message}`);

    for (const { target, line } of linksOf(readFileSync(file, 'utf8'))) {
      const raw = target.startsWith('<') && target.endsWith('>') ? target.slice(1, -1) : target;
      if (raw === '' || HAS_SCHEME.test(raw) || raw.startsWith('//')) continue;
      checked += 1;

      if (raw.startsWith('/')) {
        report(line, `absolute path "${raw}"; link relative to this file instead`);
        continue;
      }

      const hash = raw.indexOf('#');
      const pathPart = hash === -1 ? raw : raw.slice(0, hash);
      const anchor = hash === -1 ? '' : decodeURIComponent(raw.slice(hash + 1));
      const targetPath =
        pathPart === '' ? file : resolve(dirname(file), decodeURIComponent(pathPart));

      if (relative(root, targetPath).startsWith('..')) {
        report(line, `"${raw}" points outside the repository`);
        continue;
      }

      let stats;
      try {
        stats = statSync(targetPath);
      } catch {
        report(line, `"${raw}" does not exist`);
        continue;
      }

      if (anchor === '') continue;
      if (stats.isDirectory()) {
        report(line, `"${raw}" has an anchor but points at a folder`);
        continue;
      }
      if (!targetPath.endsWith('.md')) continue; // an anchor into a non-Markdown file
      if (!anchorsFor(targetPath).has(anchor)) {
        report(
          line,
          `"${raw}" has no matching heading in ${pathPart === '' ? 'this file' : pathPart}`,
        );
      }
    }
  }

  return { problems, files: files.length, checked };
}

const invokedDirectly =
  process.argv[1] !== undefined &&
  resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url));

if (invokedDirectly) {
  const root = resolve(process.argv[2] ?? fileURLToPath(new URL('..', import.meta.url)));
  const { problems, files, checked } = checkLinks(root);
  for (const problem of problems) console.error(problem);
  if (problems.length > 0) {
    console.error(`\ncheck-links: ${problems.length} broken link(s)`);
    process.exit(1);
  }
  console.log(`check-links: ${checked} relative link(s) in ${files} markdown file(s) all resolve`);
}
