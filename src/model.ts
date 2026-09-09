/**
 * The view model, as it arrives from the Rust side (§7.6, §8.4).
 *
 * The mirror of `src-tauri/src/ui_bridge/view.rs`, field for field: that module is the one
 * that decides, this one only names what it decided. Three of its choices shape every
 * component that reads these types:
 *
 * - a **key** is a catalogue key, never a sentence. `counter.key`, `banner.key` and the
 *   state labels are looked up with `t()`; the only strings here are the user's own words
 *   and the agent's, which are never translated (GUIDE-06).
 * - a **masked** value chip carries `••••••` and not the value. The true one is fetched on
 *   demand by `copyValue` and `revealValue` (DET-04), so it is never part of a repaint.
 * - `actions` says which buttons exist in this state. The store refuses an action a state
 *   does not have, and §7.4 calls that a defect of the view: a component that draws a
 *   button `actions` says nothing about is that defect.
 */

/** The row of §8.4 a tab is on. */
export type UiState =
  | 'waitingForSpec'
  | 'guiding'
  | 'agentAway'
  | 'questionSent'
  | 'deferred'
  | 'parked'
  | 'verifying'
  | 'final'
  | 'detached';

/** Where a tab sits in the strip: the list, or the collapsible "waiting" group of §7.6. */
export type TabGroup = 'open' | 'waiting';

/** One entry of the tab strip (MULTI-01, OPEN-02). */
export interface TabView {
  id: string;
  /** Agent and project, as the tab is labelled. */
  label: string;
  agent: string | null;
  project: string | null;
  /** The state of §8.1, by its wire name. */
  state: string;
  uiState: UiState;
  group: TabGroup;
  goal: string | null;
  orphan: boolean;
  createdAt: string;
}

/** The counter of GUIDE-01: a key and its numbers, never a rendered sentence. */
export interface CounterView {
  /** `counter.step`, or `counter.correction` in a correction round. */
  key: string;
  index: number;
  total: number;
  round: number;
}

/** A link, with its scheme already judged against SPEC-07. */
export interface LinkView {
  href: string;
  /** False for a URL the design shows as plain text; never render it as an anchor. */
  openable: boolean;
}

/** One value chip (GUIDE-02, DET-04). */
export interface ValueChipView {
  name: string;
  /** Whether the certain detector matched it: `items` then holds the mask, not the value. */
  masked: boolean;
  /** The family, when masked: `api_key`, `token`, … never the pattern id. */
  kind: string | null;
  /** Whether it is an array, which is copyable as a whole and per item. */
  list: boolean;
  items: string[];
}

/** One entry of the `secrets` list (SEC-01, SEC-02): a name and a destination, no value. */
export interface SecretEntryView {
  name: string;
  file: string;
}

/** A note the user wrote on a step. */
export interface StepNoteView {
  step: number;
  text: string;
  at: string;
}

/** An agent's answer, on the step it referred to. */
export interface StepReplyView {
  round: number;
  step: number;
  text: string;
  at: string;
}

/** The step the user is on (GUIDE-01..04). */
export interface StepView {
  counter: CounterView;
  text: string;
  warning: string | null;
  url: LinkView | null;
  values: ValueChipView[];
  confirmed: boolean;
  skipped: boolean;
  notes: StepNoteView[];
  replies: StepReplyView[];
  last: boolean;
}

/** The interruption the agent has not answered yet. */
export interface PendingView {
  kind: 'question' | 'screenshot';
  step: number;
}

/** A verification report (VER-05). */
export interface VerifyResultView {
  ok: boolean | null;
  detail: string | null;
  reportedAt: string;
  late: boolean;
}

/** One closed round, collapsed (VER-09). */
export interface HistoryRoundView {
  no: number;
  steps: string[];
  confirmed: number[];
  skipped: number[];
  notes: StepNoteView[];
  replies: StepReplyView[];
  verify: VerifyResultView | null;
}

/** The banner of §8.4: a catalogue key and the text it quotes, when it quotes one. */
export interface BannerView {
  key: string;
  arg: string | null;
}

/** Which buttons this state offers. */
export interface ActionsView {
  done: boolean;
  ask: boolean;
  note: boolean;
  skip: boolean;
  defer: boolean;
  abandon: boolean;
  /** False until the capture pipeline exists; the button is drawn disabled (T-049). */
  screenshot: boolean;
  resume: boolean;
  closeOrphan: boolean;
}

/** The request a handoff answers, when the link is not the id itself (OPEN-08, FM-20). */
export interface LinkedRequestView {
  id: string;
  text: string | null;
}

/** The opening session, when the current call comes from another one (TOOL-08). */
export interface ResumedFromView {
  agent: string;
  project: string;
}

/** One handoff, whole, as the overlay draws it (§7.6). */
export interface HandoffView {
  tab: TabView;
  state: string;
  uiState: UiState;
  banner: BannerView | null;
  goal: string | null;
  location: string | null;
  url: LinkView | null;
  lang: string | null;
  step: StepView | null;
  secrets: SecretEntryView[];
  notes: StepNoteView[];
  pending: PendingView | null;
  history: HistoryRoundView[];
  verify: string | null;
  verifyResult: VerifyResultView | null;
  actions: ActionsView;
  requestText: string | null;
  linkedRequest: LinkedRequestView | null;
  resumedFrom: ResumedFromView | null;
  callAttached: boolean;
  undelivered: number;
  createdAt: string;
  closedAt: string | null;
}

/** What `act` accepts (RESP-01..09, SRV-23, FM-20). */
export type ActionName =
  | 'confirm'
  | 'note'
  | 'skip'
  | 'ask'
  | 'defer'
  | 'abandon'
  | 'done'
  | 'resume_from_overlay'
  | 'close_orphan'
  | 'relink';

/** What the certain detector made of a typed text (§7.10). */
export interface Redacted {
  /** The text as it would be sent, with every certain match replaced. */
  text: string;
  /** The families that matched, in order, without repetition. Never the matched text. */
  kinds: string[];
}

/** How loudly a notice is shown. */
export type NoticeKind = 'info' | 'warning' | 'error';

/** A sentence the window shows and forgets; the Rust side already translated it. */
export interface Notice {
  kind: NoticeKind;
  text: string;
}
