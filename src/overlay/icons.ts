/**
 * The drawings behind {@link module:overlay/Icon} — the one icon set of the window.
 *
 * Everything is inline SVG on a 16×16 grid, because the webview CSP is `default-src 'self'`
 * and because a font or a sprite sheet would be a dependency for twenty-nine shapes. Each
 * icon is a list of primitives rather than a string of markup: the component turns them into
 * elements, so a shape can never carry a `style`, a colour or a `fill` of its own. Colour is
 * always `currentColor`, which is what lets one drawing sit in a muted control, on an accent
 * surface and in a red menu item without three copies of it.
 *
 * The data is here and not inside the component for a reason that has nothing to do with
 * taste: the "no hard-coded text" lint of `__tests__/hard-coded-text.ts` reads `.svelte`
 * files and calls any literal with a space and three letters a sentence. A path such as
 * `M2.5 8h11 | M5.5 2.75 8 5.25l2.5-2.5` is exactly that shape and nothing else, so it lives
 * in a TypeScript module the lint does not walk instead of teaching the lint to ignore
 * strings that look like prose.
 */

/** One primitive of a drawing. `path` covers all of them; the other two only read better. */
export type IconShape =
  | { readonly kind: 'path'; readonly d: string }
  | { readonly kind: 'circle'; readonly cx: number; readonly cy: number; readonly r: number }
  | {
      readonly kind: 'rect';
      readonly x: number;
      readonly y: number;
      readonly width: number;
      readonly height: number;
      readonly rx: number;
    };

/** One icon: what to draw, how thick, and whether the shapes are filled instead of stroked. */
export interface IconDefinition {
  readonly shapes: readonly IconShape[];
  /** The stroke width on the 16×16 grid. 1.5 unless the drawing needs otherwise. */
  readonly width?: number;
  /** `more` is three dots, which are areas and not lines. */
  readonly filled?: true;
}

const path = (d: string): IconShape => ({ kind: 'path', d });
const circle = (cx: number, cy: number, r: number): IconShape => ({ kind: 'circle', cx, cy, r });
const rect = (x: number, y: number, width: number, height: number, rx: number): IconShape => ({
  kind: 'rect',
  x,
  y,
  width,
  height,
  rx,
});

/**
 * Every icon the window draws, by name.
 *
 * The three window controls come first (WIN-03, WIN-04 and the expanded view), then the
 * actions of the step, then the marks the settings nav uses.
 */
export const ICONS = {
  /** Minimize to tray (WIN-04): one line, the universal mark for "put it away". */
  minimize: { shapes: [path('M4 8.5h8')] },
  /** Shrink to the bar of WIN-03: two arrows folding onto one line. */
  collapse: {
    shapes: [path('M2.5 8h11'), path('M5.5 2.75 8 5.25l2.5-2.5'), path('M5.5 13.25 8 10.75l2.5 2.5')],
  },
  /** The same arrows the other way round: open the panel from the bar. */
  unfold: {
    shapes: [path('M2.5 8h11'), path('M5.5 5.25 8 2.75l2.5 2.5'), path('M5.5 10.75 8 13.25l2.5-2.5')],
  },
  /** Widen the window to the expanded view. */
  expand: {
    shapes: [path('M9.5 2.5h4v4'), path('M13.5 2.5 9 7'), path('M6.5 13.5h-4v-4'), path('M2.5 13.5 7 9')],
  },
  /** Back to the narrow panel. */
  restore: {
    shapes: [path('M13.5 2.5 9.5 6.5'), path('M9.5 3.5v3h3'), path('M2.5 13.5l4-4'), path('M3.5 9.5h3v3')],
  },
  /** The one button that advances a step (GUIDE-01, RESP-09). */
  check: { shapes: [path('M3.5 8.5 6.5 11.5 12.5 5')], width: 2 },
  /** Ask: a speech bubble, because it interrupts the agent (RESP-02). */
  ask: { shapes: [path('M13.5 9.5a1.5 1.5 0 0 1-1.5 1.5H6.5L3 13.5V4a1.5 1.5 0 0 1 1.5-1.5H12A1.5 1.5 0 0 1 13.5 4z')] },
  /** Screenshot (CAP-01). */
  camera: {
    shapes: [
      path('M2 5.5A1.5 1.5 0 0 1 3.5 4h1.75l1-1.5h3.5l1 1.5h1.75A1.5 1.5 0 0 1 14 5.5V12a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 12z'),
      circle(8, 8.5, 2.25),
    ],
  },
  /** Note: a pencil, because it annotates the step locally (RESP-03). */
  note: { shapes: [path('M10.75 2.75 13.25 5.25 6 12.5H3.5V10z'), path('M9 4.5 11.5 7')] },
  /** The three dots of the More menu. */
  more: { shapes: [circle(3.5, 8, 1.2), circle(8, 8, 1.2), circle(12.5, 8, 1.2)], filled: true },
  skip: { shapes: [path('M4 3.5 10 8l-6 4.5z'), path('M12.5 3.5v9')] },
  defer: { shapes: [circle(8, 8, 5.75), path('M8 5v3.25l2.25 1.5')] },
  abandon: { shapes: [circle(8, 8, 5.75), path('M6 6l4 4M10 6l-4 4')] },
  /** Open the step's own address, which leaves the window (GUIDE-03, SPEC-07). */
  external: {
    shapes: [
      path('M9.5 2.5h4v4'),
      path('M13.5 2.5 8 8'),
      path('M11.5 9.5v2.5a1.5 1.5 0 0 1-1.5 1.5H4A1.5 1.5 0 0 1 2.5 12V6A1.5 1.5 0 0 1 4 4.5h2.5'),
    ],
  },
  /** The step warning of GUIDE-04. */
  warning: {
    shapes: [
      path('M7.13 2.5a1 1 0 0 1 1.74 0l5.4 9.5a1 1 0 0 1-.87 1.5H2.6a1 1 0 0 1-.87-1.5z'),
      path('M8 6.25v3'),
      path('M8 11.25h.01'),
    ],
  },
  copy: {
    shapes: [rect(5.5, 5.5, 8, 8, 1.5), path('M10.5 5.5V4A1.5 1.5 0 0 0 9 2.5H4A1.5 1.5 0 0 0 2.5 4v5A1.5 1.5 0 0 0 4 10.5h1.5')],
  },
  /** Show a masked value for ten seconds (DET-04). */
  eye: { shapes: [path('M1.75 8S4 3.75 8 3.75 14.25 8 14.25 8 12 12.25 8 12.25 1.75 8 1.75 8z'), circle(8, 8, 2)] },
  /** A secret the window never receives (SEC-01, SEC-02). */
  lock: { shapes: [rect(3, 7, 10, 6.5, 1.5), path('M5.5 7V5a2.5 2.5 0 0 1 5 0v2')] },
  /** A certain redaction, already applied to what will be sent (DET-01). */
  shield: { shapes: [path('M8 1.75 13 3.75V7.5c0 3.1-2.1 5.6-5 6.75-2.9-1.15-5-3.65-5-6.75V3.75z'), path('M5.75 8 7.25 9.5 10.25 6.5')] },
  send: { shapes: [path('M14 2 7.5 8.5'), path('M14 2 9.75 14l-2.25-5.5L2 6.25z')] },
  plus: { shapes: [path('M8 3.25v9.5M3.25 8h9.5')] },
  chevron: { shapes: [path('M4.5 6.25 8 9.75l3.5-3.5')] },
  back: { shapes: [path('M12.5 8h-9'), path('M7 4.5 3.5 8 7 11.5')] },
  settings: { shapes: [path('M2.5 5h6.5M12 5h1.5M2.5 11h1.5M7 11h6.5'), circle(10.5, 5, 1.5), circle(5.5, 11, 1.5)] },
  agents: { shapes: [rect(1.75, 2.75, 12.5, 10.5, 1.5), path('M4.5 6 6.5 8l-2 2'), path('M8.5 10.5h3')] },
  network: {
    shapes: [
      circle(8, 8, 5.75),
      path('M2.25 8h11.5'),
      path('M8 2.25c1.6 1.6 2.4 3.5 2.4 5.75S9.6 12.15 8 13.75C6.4 12.15 5.6 10.25 5.6 8S6.4 3.85 8 2.25z'),
    ],
  },
  log: { shapes: [path('M2.75 8a5.25 5.25 0 1 0 1.55-3.7'), path('M2.75 2.75V5.5h2.75'), path('M8 5.25V8l2 1.25')] },
  runbooks: { shapes: [path('M3 12.5V3.5A1.5 1.5 0 0 1 4.5 2H13v9.5H4.5A1.5 1.5 0 0 0 3 13a1 1 0 0 0 1 1h9')] },
  updates: { shapes: [path('M13.25 8A5.25 5.25 0 1 1 11.7 4.3'), path('M13.25 2.25V5H10.5')] },
} as const satisfies Record<string, IconDefinition>;

/** The name of an icon. A name that is not here is a compile error at the call site. */
export type IconName = keyof typeof ICONS;
