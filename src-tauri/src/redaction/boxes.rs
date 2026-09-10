//! What the two detectors find, as geometry (§7.10, DET-01, PREV-02, CAP-06).
//!
//! [`detect`] takes the OCR lines of a capture and answers the boxes that have to be burned
//! into it: **locked** ones for a certain match, which the user may not lift (DET-01), and
//! **flagged** ones for a suspected match, which are drawn by default and cost one click to
//! lift (PREV-02). [`RedactionPlan`] is everything the user then does to that answer —
//! unlock, add a box by hand, crop — and it is also where the handoff's exemption list
//! lives, so that every pass over the same capture asks the same question.
//!
//! # A box is a line
//!
//! An OCR [`TextBlock`] is one **line**: that is the one granularity Vision,
//! `Windows.Media.Ocr` and the bundled engine all report faithfully (§7.9), and a fallback
//! must not change what this module sees. Nothing in a block says where inside the line a
//! character sits, so a box that covered only the matched span would have to be guessed
//! from the character count — and a proportional guess is wrong by several glyphs on any
//! line that mixes `W` with `l`. Guessing short is a leaked secret (R-06); guessing long is
//! the whole line anyway. **So a box covers the block it came from.** The cost is real and
//! visible: the label beside a key is blacked out with it. The user can crop, and for a
//! suspected box can unlock.
//!
//! # Two passes for the certain level
//!
//! §7.10 asks for the certain patterns "over the concatenated text of each block and over
//! each block alone". Both are here and they answer different questions: per block catches
//! the ordinary case, and the concatenation catches a key an engine split across two lines
//! — every block the match touches is locked. A pattern that matches inside one block is
//! found by the first pass, so the second only ever adds boxes.
//!
//! # Coordinates
//!
//! Everything here is in the coordinates of the **original** capture, at full resolution
//! (CAP-05, CAP-06). The downscale, the rescale and the 2-pixel expansion happen in
//! [`super::burn`], after the resize, and nothing in this file knows about them.

use std::collections::BTreeSet;

use crate::capture::Rect;
use crate::ocr::TextBlock;

use super::certain::{scan_text as scan_certain, CertainSecretKind};
use super::suspected::{scan_text as scan_suspected, Exemptions, SuspectedReason};

/// What the second certain pass puts between two blocks (§7.10).
///
/// Two of them, because an engine that broke a secret in half did it in one of two ways and
/// one join cannot undo both: a key cut inside a token (`sk_live_51H` / `abcdef`) is put
/// back together by the empty string, and a header cut at a word boundary
/// (`-----BEGIN RSA` / `PRIVATE KEY-----`) by the space the engine dropped with the line
/// break. Scanning twice costs two passes over a screen's worth of text and catches both.
const JOINS: [&str; 2] = ["", " "];

/// The length of the separator [`Detection::text`] is built with.
const LINE_BREAK: usize = 1;

/// How many lines one certain match of the second pass may lock.
///
/// An engine that broke a secret in half produced two lines. Three means the pattern is
/// running through the page rather than reassembling a value.
const MAX_BLOCKS_ACROSS: usize = 2;

/// How sure the app is that a box hides a secret, and therefore who may lift it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxLevel {
    /// A public certain pattern matched. The user may not unlock it (DET-01).
    Locked,
    /// A suspected-level heuristic fired. One click lifts it (PREV-02).
    Flagged,
}

impl BoxLevel {
    /// The name the view and the log use.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::Flagged => "flagged",
        }
    }

    /// Whether the user is allowed to lift it.
    #[must_use]
    pub fn is_unlockable(self) -> bool {
        matches!(self, Self::Flagged)
    }
}

/// Why a box was drawn, in as much detail as may cross into the webview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxCause {
    /// A certain pattern of the given family (§4.6).
    Certain(CertainSecretKind),
    /// A suspected-level rule (§7.10).
    Suspected(SuspectedReason),
    /// The user drew it (PREV-02).
    Added,
}

impl BoxCause {
    /// The family or rule name. Never the matched text (R-19).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Certain(kind) => kind.as_str(),
            Self::Suspected(reason) => reason.as_str(),
            Self::Added => "added",
        }
    }
}

/// One rectangle to burn, in the coordinates of the original capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionBox {
    /// Stable within one [`Detection`]: it is what a plan names when the user unlocks.
    pub id: usize,
    /// Where it is, in the original image's pixels.
    pub rect: Rect,
    /// Locked or flagged.
    pub level: BoxLevel,
    /// What put it there.
    pub cause: BoxCause,
    /// The OCR block it came from, for the tests and for the log's count.
    pub block: usize,
}

/// What the two detectors made of one capture.
#[derive(Debug, Clone, Default)]
pub struct Detection {
    /// The boxes, ordered by block and, inside a block, locked before flagged.
    pub boxes: Vec<RedactionBox>,
    /// The lines the engine read, joined by `\n` in the order it reported them.
    ///
    /// This is what the text pane of the preview starts from (PREV-03) and what
    /// [`super::burn::redact_text`] is asked about. It is the recognised text of the user's
    /// own screen: it stays in memory and is never logged (LOG-03).
    pub text: String,
    /// Per flagged box, the tokens it covers, so that unlocking one can restore exactly
    /// those words in the text pane and nothing else.
    flagged_tokens: Vec<(usize, Vec<String>)>,
}

impl Detection {
    /// The box with this id.
    #[must_use]
    pub fn box_with(&self, id: usize) -> Option<&RedactionBox> {
        self.boxes.iter().find(|drawn| drawn.id == id)
    }

    /// How many boxes are locked, which is what the outcome's `redactions` counts.
    #[must_use]
    pub fn locked(&self) -> usize {
        self.boxes
            .iter()
            .filter(|drawn| drawn.level == BoxLevel::Locked)
            .count()
    }

    /// How many boxes are flagged.
    #[must_use]
    pub fn flagged(&self) -> usize {
        self.boxes
            .iter()
            .filter(|drawn| drawn.level == BoxLevel::Flagged)
            .count()
    }

    /// The suspected tokens one flagged box covers.
    fn tokens_of(&self, id: usize) -> &[String] {
        self.flagged_tokens
            .iter()
            .find(|(box_id, _)| *box_id == id)
            .map_or(&[], |(_, tokens)| tokens.as_slice())
    }
}

/// The boxes of one capture (§7.10).
///
/// `blocks` are the OCR lines in the order the engine reported them, in the coordinates of
/// the image they were read from. `exempt` is the handoff's exemption list (DET-03).
#[must_use]
pub fn detect(blocks: &[TextBlock], exempt: &Exemptions) -> Detection {
    let joined = blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // Which blocks a certain pattern touches, and under which family. The per-block pass
    // first, then the concatenation for a match an engine split across two lines.
    let mut certain_of: Vec<Option<CertainSecretKind>> = vec![None; blocks.len()];
    for (index, block) in blocks.iter().enumerate() {
        if let Some(hit) = scan_certain(&block.text).first() {
            certain_of[index] = Some(hit.kind);
        }
    }
    let alone = certain_of.clone();
    for separator in JOINS {
        let across = blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join(separator);
        for hit in scan_certain(&across) {
            let touched = blocks_touched(blocks, separator.len(), hit.start, hit.end);
            let Some(first) = touched.first().copied() else {
                continue;
            };
            if alone[first].is_some() || touched.len() > MAX_BLOCKS_ACROSS {
                // Two ways this pass says nothing new, and both matter. A match whose first
                // line already matched on its own has only run on into what follows —
                // every tail quantifier of §4.6 is greedy and the join removed the line
                // break that used to stop it, so `sk_live_…` on one line swallows the two
                // rows under it and locks them. And a match reaching further than two lines
                // is not a secret an engine broke in half; it is a pattern eating the page.
                continue;
            }
            for index in touched {
                certain_of[index].get_or_insert(hit.kind);
            }
        }
    }

    // The suspected pass runs on the joined text, because "the line below" of §7.10 is a
    // line break and the blocks are the lines.
    let mut suspected_of: Vec<Vec<(SuspectedReason, String)>> = vec![Vec::new(); blocks.len()];
    for hit in scan_suspected(&joined, exempt) {
        let token = joined[hit.start..hit.end].to_owned();
        for index in blocks_touched(blocks, LINE_BREAK, hit.start, hit.end) {
            suspected_of[index].push((hit.reason, token.clone()));
        }
    }

    let mut detection = Detection {
        text: joined,
        ..Detection::default()
    };
    let mut next_id = 0_usize;
    for (index, block) in blocks.iter().enumerate() {
        if let Some(kind) = certain_of[index] {
            detection.boxes.push(RedactionBox {
                id: next_id,
                rect: block.bbox,
                level: BoxLevel::Locked,
                cause: BoxCause::Certain(kind),
                block: index,
            });
            next_id += 1;
            // A line already locked is not flagged as well: one box, and the stricter of
            // the two levels. Its suspected tokens are dropped with it, so unlocking can
            // never restore a word the certain detector matched.
            continue;
        }
        let Some(reason) = suspected_of[index].first().map(|(reason, _)| *reason) else {
            continue;
        };
        let id = next_id;
        next_id += 1;
        detection.boxes.push(RedactionBox {
            id,
            rect: block.bbox,
            level: BoxLevel::Flagged,
            cause: BoxCause::Suspected(reason),
            block: index,
        });
        detection.flagged_tokens.push((
            id,
            suspected_of[index]
                .iter()
                .map(|(_, token)| token.clone())
                .collect(),
        ));
    }
    detection
}

/// The indices of the blocks a span of a joined text falls in.
///
/// `separator` is the length in bytes of what was put between two blocks, so the offsets
/// are known exactly; a span that ends on a separator belongs to the block before it.
fn blocks_touched(blocks: &[TextBlock], separator: usize, start: usize, end: usize) -> Vec<usize> {
    let mut touched = Vec::new();
    let mut offset = 0_usize;
    for (index, block) in blocks.iter().enumerate() {
        let block_end = offset + block.text.len();
        if start < block_end && offset < end {
            touched.push(index);
        }
        offset = block_end + separator;
    }
    touched
}

/// Everything that decides what is redacted for one capture: the handoff's exemptions, and
/// what the user did in the preview (PREV-02, PREV-03).
///
/// It is deliberately one object rather than three arguments. `burn` and `redact_text` are
/// asked the same question about the same capture and must not be able to answer it
/// differently, and the exemption list is a fact about the handoff that outlives every edit.
#[derive(Debug, Clone)]
pub struct RedactionPlan {
    exemptions: Exemptions,
    crop: Option<Rect>,
    unlocked: BTreeSet<usize>,
    restored: BTreeSet<String>,
    added: Vec<Rect>,
}

impl RedactionPlan {
    /// A plan over a handoff's exemption list, with nothing edited yet.
    #[must_use]
    pub fn new(exemptions: Exemptions) -> Self {
        Self {
            exemptions,
            crop: None,
            unlocked: BTreeSet::new(),
            restored: BTreeSet::new(),
            added: Vec::new(),
        }
    }

    /// The exemption list this capture is detected against (DET-03).
    #[must_use]
    pub fn exemptions(&self) -> &Exemptions {
        &self.exemptions
    }

    /// Lifts a flagged box (PREV-02).
    ///
    /// Answers `false` for a box that is locked or that this detection does not have — a
    /// certain match is not the user's to unlock (DET-01), and the refusal is here rather
    /// than in the window because this is the side that decides what is burned.
    pub fn unlock(&mut self, detection: &Detection, id: usize) -> bool {
        let Some(drawn) = detection.box_with(id) else {
            return false;
        };
        if !drawn.level.is_unlockable() {
            return false;
        }
        self.unlocked.insert(id);
        for token in detection.tokens_of(id) {
            self.restored.insert(token.clone());
        }
        true
    }

    /// Puts a lifted box back.
    pub fn relock(&mut self, detection: &Detection, id: usize) {
        self.unlocked.remove(&id);
        let still_out: BTreeSet<String> = self
            .unlocked
            .iter()
            .flat_map(|other| detection.tokens_of(*other).iter().cloned())
            .collect();
        self.restored = still_out;
    }

    /// Whether the user lifted this box.
    #[must_use]
    pub fn is_unlocked(&self, id: usize) -> bool {
        self.unlocked.contains(&id)
    }

    /// Adds a box the user drew, in the original image's coordinates (PREV-02).
    pub fn add_box(&mut self, rect: Rect) {
        self.added.push(rect);
    }

    /// The boxes the user drew.
    #[must_use]
    pub fn added(&self) -> &[Rect] {
        &self.added
    }

    /// Crops the capture, in the original image's coordinates (PREV-02).
    pub fn crop_to(&mut self, rect: Rect) {
        self.crop = Some(rect);
    }

    /// Undoes a crop.
    pub fn uncrop(&mut self) {
        self.crop = None;
    }

    /// The crop, if the user set one.
    #[must_use]
    pub fn crop(&self) -> Option<Rect> {
        self.crop
    }

    /// Whether a suspected token was restored by an unlock, and may therefore stay in the
    /// text the agent reads.
    #[must_use]
    pub fn restores(&self, token: &str) -> bool {
        self.restored.contains(token)
    }

    /// The rectangles to burn, in the coordinates of the **original** image (CAP-06).
    ///
    /// Locked boxes are here whatever the unlock set says: a plan that had been tampered
    /// with — a webview is the one part of this application that must never be believed —
    /// still cannot lift one.
    #[must_use]
    pub fn boxes(&self, detection: &Detection) -> Vec<Rect> {
        detection
            .boxes
            .iter()
            .filter(|drawn| match drawn.level {
                BoxLevel::Locked => true,
                BoxLevel::Flagged => !self.unlocked.contains(&drawn.id),
            })
            .map(|drawn| drawn.rect)
            .chain(self.added.iter().copied())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key of the AWS shape, which the pattern file calls an `api_key`.
    const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

    fn line(text: &str, y: i32) -> TextBlock {
        TextBlock {
            text: text.to_owned(),
            bbox: Rect::new(10, y, 400, 18),
            confidence: 1.0,
        }
    }

    #[test]
    fn a_certain_match_locks_the_line_it_is_on_and_no_other() {
        let blocks = vec![
            line("Stripe dashboard", 10),
            line(&format!("Access key {AWS_KEY}"), 40),
            line("Press Reveal to see it", 70),
        ];
        let detection = detect(&blocks, &Exemptions::none());
        assert_eq!(detection.boxes.len(), 1);
        assert_eq!(detection.boxes[0].level, BoxLevel::Locked);
        assert_eq!(detection.boxes[0].block, 1);
        assert_eq!(detection.boxes[0].rect, blocks[1].bbox);
        assert_eq!(
            detection.boxes[0].cause,
            BoxCause::Certain(CertainSecretKind::ApiKey)
        );
    }

    #[test]
    fn a_key_the_engine_split_across_two_lines_locks_both() {
        // The second pass of §7.10: neither half matches on its own.
        let blocks = vec![
            line("-----BEGIN RSA", 10),
            line(" PRIVATE KEY-----", 40),
            line("nothing here", 70),
        ];
        assert!(scan_certain(&blocks[0].text).is_empty());
        let detection = detect(&blocks, &Exemptions::none());
        let locked: Vec<usize> = detection
            .boxes
            .iter()
            .filter(|drawn| drawn.level == BoxLevel::Locked)
            .map(|drawn| drawn.block)
            .collect();
        assert_eq!(locked, vec![0, 1]);
    }

    #[test]
    fn a_greedy_pattern_does_not_run_from_its_line_into_the_next_ones() {
        // Found by the corpus: every tail quantifier of §4.6 is greedy, and joining the
        // lines removes the break that stopped it. Three rows were locked for one key.
        let blocks = vec![
            line("Secret key  sk_live_51H8xQ2eZvKYlo2C0Sd8h4kL", 10),
            line("Last used  3 hours ago", 40),
            line("Mode  Live", 70),
        ];
        let detection = detect(&blocks, &Exemptions::none());
        assert_eq!(
            detection
                .boxes
                .iter()
                .filter(|drawn| drawn.level == BoxLevel::Locked)
                .map(|drawn| drawn.block)
                .collect::<Vec<_>>(),
            vec![0]
        );
    }

    #[test]
    fn a_suspected_match_flags_its_line_and_can_be_unlocked() {
        let blocks = vec![
            line("Webhook signing secret", 10),
            line("whsec-alpha-bravo-9931", 40),
        ];
        let detection = detect(&blocks, &Exemptions::none());
        assert_eq!(detection.flagged(), 1);
        let id = detection.boxes[0].id;
        assert_eq!(detection.boxes[0].level, BoxLevel::Flagged);

        let mut plan = RedactionPlan::new(Exemptions::none());
        assert_eq!(plan.boxes(&detection).len(), 1);
        assert!(plan.unlock(&detection, id));
        assert!(plan.boxes(&detection).is_empty());
        assert!(plan.restores("whsec-alpha-bravo-9931"));
        plan.relock(&detection, id);
        assert_eq!(plan.boxes(&detection).len(), 1);
        assert!(!plan.restores("whsec-alpha-bravo-9931"));
    }

    #[test]
    fn a_locked_box_is_refused_an_unlock_and_burned_anyway() {
        let blocks = vec![line(&format!("key {AWS_KEY}"), 10)];
        let detection = detect(&blocks, &Exemptions::none());
        let id = detection.boxes[0].id;
        let mut plan = RedactionPlan::new(Exemptions::none());
        assert!(!plan.unlock(&detection, id), "a certain box was unlocked");
        assert_eq!(plan.boxes(&detection).len(), 1);
    }

    #[test]
    fn an_unlock_set_that_names_a_locked_box_still_burns_it() {
        // The webview is the one part of the application that is never believed: a plan
        // that arrived with a locked id in it must not lift the box.
        let blocks = vec![line(&format!("key {AWS_KEY}"), 10)];
        let detection = detect(&blocks, &Exemptions::none());
        let mut plan = RedactionPlan::new(Exemptions::none());
        plan.unlocked.insert(detection.boxes[0].id);
        assert_eq!(plan.boxes(&detection).len(), 1);
    }

    #[test]
    fn a_line_that_is_both_certain_and_suspected_is_locked_once() {
        let blocks = vec![line(&format!("api_key {AWS_KEY}"), 10)];
        let detection = detect(&blocks, &Exemptions::none());
        assert_eq!(detection.boxes.len(), 1);
        assert_eq!(detection.boxes[0].level, BoxLevel::Locked);
        assert!(detection.tokens_of(detection.boxes[0].id).is_empty());
    }

    #[test]
    fn an_exempt_spec_value_draws_no_box() {
        let blocks = vec![
            line("Endpoint id", 10),
            line("we_1P9xTz2eZvKYlo2C0Sd8h4kL", 40),
        ];
        assert_eq!(detect(&blocks, &Exemptions::none()).boxes.len(), 1);
        let exempt = Exemptions::of_values(["we_1P9xTz2eZvKYlo2C0Sd8h4kL"]);
        assert!(detect(&blocks, &exempt).boxes.is_empty());
    }

    #[test]
    fn a_box_the_user_drew_is_added_to_the_ones_that_were_found() {
        let detection = detect(&[line("nothing here", 10)], &Exemptions::none());
        let mut plan = RedactionPlan::new(Exemptions::none());
        plan.add_box(Rect::new(4, 5, 20, 30));
        assert_eq!(plan.boxes(&detection), vec![Rect::new(4, 5, 20, 30)]);
    }

    #[test]
    fn the_recognised_text_is_the_lines_in_order() {
        let detection = detect(
            &[line("first", 10), line("second", 40)],
            &Exemptions::none(),
        );
        assert_eq!(detection.text, "first\nsecond");
    }
}
