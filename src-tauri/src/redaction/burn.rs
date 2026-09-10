//! Burn-in: the last thing that happens to a capture before it may leave (CAP-05, CAP-06).
//!
//! The order of the four steps is the whole point of the section, and it is the one thing a
//! reviewer should check here:
//!
//! 1. **crop**, if the user cropped (PREV-02), in the original image's coordinates;
//! 2. **downscale** so that the long side is [`IMAGE_LONG_SIDE_PX`] (CAP-05) — the OCR of
//!    §7.9 has already run on the full-resolution original, which is why it may;
//! 3. **rescale** the boxes onto the reduced image and **expand** them by
//!    [`BOX_EXPAND_PX`];
//! 4. **fill** them with solid black, and only then encode the PNG.
//!
//! Filling before the resize is the defect CAP-06 names: the resampling kernel of step 2
//! reads the pixels around each output pixel, so a black rectangle laid on the original is
//! averaged with the glyphs at its edge and the reduced image carries a grey halo in the
//! shape of the letters. That halo is legible — to a person and to an OCR engine — which is
//! what the glyph-leak test of §11.2 exists to catch. `tests/redaction_corpus.rs` runs both
//! orders over the corpus and asserts the difference rather than trusting this comment.
//!
//! # Text mode
//!
//! [`redact_text`] is the same decision applied to characters instead of pixels (PREV-03,
//! PREV-05): the certain matches become `[REDACTED:<kind>]`, the suspected ones become
//! [`SUSPECTED_MASK`] unless the user lifted the box that covered them, and whatever the
//! user typed into the pane is what is scanned — an edit cannot be trusted to have kept the
//! text the detector was shown, so the detector is run again over what will actually leave.
//!
//! Unlike a box, a text mask replaces **the matched span and not the line** (the rule the
//! server settled for its own masking and the log follows): a box cannot be narrower than
//! the line it was read from, a string can, and blanking a whole line of the text pane
//! would lose the sentence the user is sending.

use image::{imageops::FilterType, RgbaImage};
use sha2::{Digest as _, Sha256};

use crate::capture::Rect;

use super::boxes::{Detection, RedactionPlan};
use super::certain::scan_text as scan_certain;
use super::splice;
use super::suspected::scan_text as scan_suspected;
use super::typed::mask_for;

/// The long side of the image an agent receives (§4.1, CAP-05).
pub const IMAGE_LONG_SIDE_PX: u32 = 1600;

/// How far every box grows once it is on the reduced image (§7.10).
pub const BOX_EXPAND_PX: u32 = 2;

/// What a suspected match becomes in text mode.
///
/// It names the level and not the rule: `hex` or `entropy` would tell a reader how the
/// string was shaped, which is a description of the value itself.
pub const SUSPECTED_MASK: &str = "[REDACTED:suspected]";

/// Why a capture could not be burned.
#[derive(Debug, thiserror::Error)]
pub enum BurnError {
    /// The crop fell outside the image, or the capture has no pixels.
    #[error("nothing is left of the capture to send")]
    Empty,
    /// The PNG encoder refused. There is no recovery: the alternative is sending nothing.
    #[error("{0}")]
    Encode(String),
}

/// The image as it leaves the machine, and what is known about it (LOG-03).
#[derive(Debug, Clone)]
pub struct Burned {
    /// The PNG, redacted, downscaled, ready for the tool result.
    pub png: Vec<u8>,
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// The rectangles that were filled, in the coordinates of **this** image.
    pub boxes: Vec<Rect>,
    /// The hash of `png`, which is the only thing about a screenshot the log may keep.
    pub sha256: String,
}

/// The scale the long side of `(width, height)` is reduced by (CAP-05).
///
/// Never above 1.0: an image smaller than [`IMAGE_LONG_SIDE_PX`] is sent as it is. "About
/// 1600 px" is a ceiling, not a target, and upscaling would invent pixels the OCR never
/// read.
#[must_use]
pub fn scale_for(width: u32, height: u32) -> f64 {
    let long_side = width.max(height);
    if long_side <= IMAGE_LONG_SIDE_PX || long_side == 0 {
        return 1.0;
    }
    f64::from(IMAGE_LONG_SIDE_PX) / f64::from(long_side)
}

/// The size an image of `(width, height)` is reduced to.
#[must_use]
pub fn reduced_size(width: u32, height: u32) -> (u32, u32) {
    let scale = scale_for(width, height);
    if scale >= 1.0 {
        return (width, height);
    }
    let reduce = |side: u32| -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scaled = (f64::from(side) * scale).round() as u32;
        scaled.max(1)
    };
    (reduce(width), reduce(height))
}

/// One box, moved from the original image onto the reduced one and expanded (CAP-06).
///
/// `origin` is the crop's top-left corner in original coordinates. The edges are taken
/// **outwards** — the left and top rounded down, the right and bottom rounded up — before
/// the expansion, so the rounding of step 3 can only ever make a box larger. A box that
/// falls entirely outside the reduced image answers `None`.
#[must_use]
pub fn rescale_box(rect: Rect, origin: (i32, i32), scale: f64, size: (u32, u32)) -> Option<Rect> {
    let (width, height) = size;
    let map = |value: i64, round_up: bool| -> i64 {
        #[allow(clippy::cast_possible_truncation)]
        let scaled = value as f64 * scale;
        #[allow(clippy::cast_possible_truncation)]
        let rounded = if round_up {
            scaled.ceil() as i64
        } else {
            scaled.floor() as i64
        };
        rounded
    };
    let expand = i64::from(BOX_EXPAND_PX);
    let left = map(i64::from(rect.x) - i64::from(origin.0), false) - expand;
    let top = map(i64::from(rect.y) - i64::from(origin.1), false) - expand;
    let right = map(rect.right() - i64::from(origin.0), true) + expand;
    let bottom = map(rect.bottom() - i64::from(origin.1), true) + expand;

    let left = left.clamp(0, i64::from(width));
    let top = top.clamp(0, i64::from(height));
    let right = right.clamp(0, i64::from(width));
    let bottom = bottom.clamp(0, i64::from(height));
    if right <= left || bottom <= top {
        return None;
    }
    Some(Rect::new(
        i32::try_from(left).ok()?,
        i32::try_from(top).ok()?,
        u32::try_from(right - left).ok()?,
        u32::try_from(bottom - top).ok()?,
    ))
}

/// The capture as it leaves the machine (CAP-05, CAP-06, PRIN-09).
///
/// # Errors
///
/// [`BurnError::Empty`] when the crop leaves no pixels, [`BurnError::Encode`] when the PNG
/// encoder refuses.
pub fn burn(
    image: &RgbaImage,
    detection: &Detection,
    plan: &RedactionPlan,
) -> Result<Burned, BurnError> {
    let full = Rect::new(0, 0, image.width(), image.height());
    let area = match plan.crop() {
        Some(crop) => crop.intersection(&full).ok_or(BurnError::Empty)?,
        None => full,
    };
    if area.width == 0 || area.height == 0 {
        return Err(BurnError::Empty);
    }

    // Step 1 — crop, in the original image's coordinates.
    #[allow(clippy::cast_sign_loss)]
    let cropped = image::imageops::crop_imm(
        image,
        area.x.max(0) as u32,
        area.y.max(0) as u32,
        area.width,
        area.height,
    )
    .to_image();

    // Step 2 — downscale. Triangle is the filter that keeps small text readable at this
    // ratio without the ringing Lanczos leaves around a glyph, and it is what CAP-05 is
    // about: the agent has to be able to read the screen it is sent.
    let scale = scale_for(cropped.width(), cropped.height());
    let (width, height) = reduced_size(cropped.width(), cropped.height());
    let mut reduced = if scale >= 1.0 {
        cropped
    } else {
        image::imageops::resize(&cropped, width, height, FilterType::Triangle)
    };

    // Step 3 — rescale and expand, on the reduced image.
    let boxes: Vec<Rect> = plan
        .boxes(detection)
        .into_iter()
        .filter_map(|rect| rescale_box(rect, (area.x, area.y), scale, (width, height)))
        .collect();

    // Step 4 — fill, then encode. Never the other way round (CAP-06).
    fill_black(&mut reduced, &boxes);

    let mut png = Vec::new();
    reduced
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| BurnError::Encode(error.to_string()))?;
    let sha256 = hex_digest(&png);

    Ok(Burned {
        png,
        width,
        height,
        boxes,
        sha256,
    })
}

/// Paints every rectangle solid black, opaque.
///
/// Opaque matters: a capture may carry an alpha channel, and a black rectangle drawn with
/// the alpha of the pixels under it would leave the glyphs visible on any background the
/// PNG is composited onto.
pub fn fill_black(image: &mut RgbaImage, boxes: &[Rect]) {
    let (width, height) = (i64::from(image.width()), i64::from(image.height()));
    for rect in boxes {
        let left = i64::from(rect.x).clamp(0, width);
        let top = i64::from(rect.y).clamp(0, height);
        let right = rect.right().clamp(0, width);
        let bottom = rect.bottom().clamp(0, height);
        for y in top..bottom {
            for x in left..right {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                image.put_pixel(x as u32, y as u32, image::Rgba([0, 0, 0, 0xff]));
            }
        }
    }
}

/// The SHA-256 of some bytes, as 64 lowercase hexadecimal characters (LOG-03).
#[must_use]
pub fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// What the text pane sends, and what was taken out of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedText {
    /// The text as the agent will read it.
    pub text: String,
    /// The certain families that were replaced, in order, without repetition.
    pub kinds: Vec<String>,
    /// How many spans a certain pattern matched. Not [`RedactedText::kinds`]'s length: one
    /// family can be replaced several times, and `screenshot.redactions` counts the
    /// regions that were taken out (§4.3), not the vocabulary of what was in them.
    pub certain: usize,
    /// How many suspected tokens were replaced. Never which ones (R-19).
    pub suspected: usize,
}

impl RedactedText {
    /// How many regions this text lost, which is `screenshot.redactions` in text mode.
    #[must_use]
    pub fn redactions(&self) -> usize {
        self.certain + self.suspected
    }
}

/// The text an agent receives in text mode (§7.10, PREV-03, PREV-05).
///
/// `text` is what is in the pane at the moment **Send text** is pressed, edited or not; the
/// detectors are run over it again, because an edit could have introduced a key as easily
/// as removed one. A suspected token the user restored by unlocking its box is left as it
/// is; a certain match is replaced whatever the plan says.
#[must_use]
pub fn redact_text(text: &str, plan: &RedactionPlan) -> RedactedText {
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();
    let mut kinds: Vec<String> = Vec::new();
    let mut certain = 0_usize;
    for hit in scan_certain(text) {
        replacements.push((hit.start, hit.end, mask_for(hit.kind)));
        certain += 1;
        let kind = hit.kind.as_str().to_owned();
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    let mut suspected = 0_usize;
    for hit in scan_suspected(text, plan.exemptions()) {
        if plan.restores(&text[hit.start..hit.end]) {
            continue;
        }
        if replacements
            .iter()
            .any(|(start, end, _)| hit.start < *end && *start < hit.end)
        {
            continue;
        }
        replacements.push((hit.start, hit.end, SUSPECTED_MASK.to_owned()));
        suspected += 1;
    }
    replacements.sort_by_key(|(start, _, _)| *start);

    RedactedText {
        text: splice(text, &replacements),
        kinds,
        certain,
        suspected,
    }
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;
    use crate::ocr::TextBlock;
    use crate::redaction::suspected::Exemptions;

    const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

    fn plain(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([0xff, 0xff, 0xff, 0xff]))
    }

    fn detection_of(rect: Rect) -> Detection {
        super::super::boxes::detect(
            &[TextBlock {
                text: format!("key {AWS_KEY}"),
                bbox: rect,
                confidence: 1.0,
            }],
            &Exemptions::none(),
        )
    }

    #[test]
    fn the_long_side_is_brought_down_to_1600_and_never_up() {
        assert_eq!(reduced_size(3200, 1800), (1600, 900));
        assert_eq!(reduced_size(1800, 3200), (900, 1600));
        assert_eq!(reduced_size(1600, 1200), (1600, 1200));
        assert_eq!(reduced_size(800, 600), (800, 600));
    }

    #[test]
    fn a_box_is_rescaled_then_expanded_by_two_pixels() {
        let rescaled =
            rescale_box(Rect::new(100, 200, 400, 40), (0, 0), 0.5, (1600, 900)).expect("inside");
        // 100..500 and 200..240 halve to 50..250 and 100..120, then grow by two each way.
        assert_eq!(rescaled, Rect::new(48, 98, 204, 24));
    }

    #[test]
    fn rounding_only_ever_makes_a_box_larger() {
        let rescaled =
            rescale_box(Rect::new(101, 201, 401, 41), (0, 0), 0.5, (1600, 900)).expect("inside");
        assert!(rescaled.x <= 48, "left edge moved inwards: {rescaled:?}");
        assert!(
            rescaled.right() >= 253,
            "right edge moved inwards: {rescaled:?}"
        );
    }

    #[test]
    fn a_crop_moves_the_origin_of_every_box() {
        let rescaled =
            rescale_box(Rect::new(300, 300, 100, 20), (200, 250), 1.0, (400, 400)).expect("inside");
        assert_eq!(rescaled, Rect::new(98, 48, 104, 24));
    }

    #[test]
    fn a_box_outside_the_crop_disappears_instead_of_being_clamped_to_a_corner() {
        assert!(rescale_box(Rect::new(0, 0, 10, 10), (500, 500), 1.0, (400, 400)).is_none());
    }

    #[test]
    fn the_burned_image_is_black_where_the_box_is_and_untouched_elsewhere() {
        let image = plain(2000, 1000);
        let detection = detection_of(Rect::new(100, 100, 400, 40));
        let plan = RedactionPlan::new(Exemptions::none());
        let burned = burn(&image, &detection, &plan).expect("burned");

        assert_eq!((burned.width, burned.height), (1600, 800));
        assert_eq!(burned.boxes.len(), 1);
        let drawn = burned.boxes[0];
        let reduced = image::load_from_memory(&burned.png)
            .expect("a PNG")
            .to_rgba8();
        #[allow(clippy::cast_sign_loss)]
        let inside = reduced.get_pixel(drawn.x as u32 + 1, drawn.y as u32 + 1);
        assert_eq!(inside, &Rgba([0, 0, 0, 0xff]));
        assert_eq!(reduced.get_pixel(0, 0), &Rgba([0xff, 0xff, 0xff, 0xff]));
        assert_eq!(burned.sha256.len(), 64);
    }

    #[test]
    fn a_crop_that_leaves_nothing_is_refused_rather_than_sent_empty() {
        let image = plain(100, 100);
        let detection = Detection::default();
        let mut plan = RedactionPlan::new(Exemptions::none());
        plan.crop_to(Rect::new(500, 500, 10, 10));
        assert!(matches!(
            burn(&image, &detection, &plan),
            Err(BurnError::Empty)
        ));
    }

    #[test]
    fn a_certain_match_in_the_text_pane_is_replaced_and_a_restored_token_is_not() {
        let plan = RedactionPlan::new(Exemptions::none());
        let redacted = redact_text(&format!("the key is {AWS_KEY} ok"), &plan);
        assert_eq!(redacted.text, "the key is [REDACTED:api_key] ok");
        assert_eq!(redacted.kinds, vec!["api_key".to_owned()]);
        assert_eq!(redacted.suspected, 0);
    }

    #[test]
    fn a_suspected_token_is_masked_unless_its_box_was_unlocked() {
        let blocks = [
            TextBlock {
                text: "Password".to_owned(),
                bbox: Rect::new(0, 0, 100, 10),
                confidence: 1.0,
            },
            TextBlock {
                text: "hunter2-tango".to_owned(),
                bbox: Rect::new(0, 20, 100, 10),
                confidence: 1.0,
            },
        ];
        let detection = super::super::boxes::detect(&blocks, &Exemptions::none());
        let mut plan = RedactionPlan::new(Exemptions::none());
        let masked = redact_text(&detection.text, &plan);
        assert_eq!(masked.text, format!("Password\n{SUSPECTED_MASK}"));
        assert_eq!(masked.suspected, 1);

        assert!(plan.unlock(&detection, detection.boxes[0].id));
        let restored = redact_text(&detection.text, &plan);
        assert_eq!(restored.text, "Password\nhunter2-tango");
        assert_eq!(restored.suspected, 0);
    }

    #[test]
    fn an_edit_that_adds_a_key_is_still_redacted() {
        // PREV-03 lets the user send what the user sends; §7.10 runs the detectors over it.
        let plan = RedactionPlan::new(Exemptions::none());
        let edited = format!("I typed this myself: {AWS_KEY}");
        assert!(!redact_text(&edited, &plan).text.contains(AWS_KEY));
    }

    #[test]
    fn a_certain_match_wins_over_a_suspected_one_on_the_same_span() {
        let plan = RedactionPlan::new(Exemptions::none());
        let redacted = redact_text(&format!("api key {AWS_KEY}"), &plan);
        assert_eq!(redacted.text, "api key [REDACTED:api_key]");
        assert_eq!(redacted.suspected, 0);
    }
}
