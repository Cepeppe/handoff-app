//! The mandatory preview, and the only path a screenshot has out of this machine
//! (§7.10, PREV-01..05, CTX-01, LOG-03, PRIN-09).
//!
//! [`super::capture`] produces pixels and stops there. This is everything between them and
//! the agent, in the order it happens:
//!
//! 1. [`analyze_capture`] — the OCR of §7.9 on the full-resolution original, then
//!    [`crate::redaction::boxes::detect`] over what it read, with the handoff's exemption
//!    list (DET-03). It is `async` and every engine attempt is already on a blocking
//!    thread, which is what makes OCR-04's "analyzing" state possible: the preview is on
//!    screen with the image the instant the capture ends, and the two send buttons enable
//!    when this answers.
//! 2. [`edit_preview`] — unlock, add a box, crop, undo a crop (PREV-02). Each answers the
//!    whole drawing again, so the window never has to keep a second copy of the plan.
//! 3. [`preview_text`] — what the text pane will actually send, run over what is in the
//!    pane at this instant (PREV-03). It exists so the redaction is something the user
//!    **sees** before pressing the button, which is what PREV-01 is about.
//! 4. [`send_screenshot`] — the burn-in of CAP-06 (image) or [`redact_text`] (text), the
//!    `ScreenshotPayload` the store takes, and the pixels dropped on the way out.
//!
//! # What crosses into the webview, and what does not
//!
//! The boxes cross as geometry and a *family name* — never the matched text (R-19) — and
//! the OCR text crosses whole, because the pane is editable and the user is looking at the
//! screen it was read from anyway. The **image** crosses once, as the raw bytes
//! [`super::capture::capture_preview`] answers, and the redaction is drawn over it by the
//! window: a box is a rectangle the user can lift, so a pre-burned picture would have to be
//! re-encoded on every click.
//!
//! That is why the burn happens **here** and not in the window: what leaves the machine is
//! produced from the plan on this side, and a webview that had been tampered with can lift
//! a flagged box (which is the user's, DET-01) and can never lift a locked one
//! ([`crate::redaction::boxes::RedactionPlan::boxes`] enforces it).
//!
//! # A capture nobody could read
//!
//! `select_and_run` answers `NoEngine` only for an installation whose OCR resources are
//! missing (T-047). There is then no text to send and nothing was found to
//! hide, so the preview says so and leaves the user the two tools that do not need an
//! engine: crop, and a box drawn by hand (PREV-02). Refusing to send at all would be the
//! broken failure PRIN-10 forbids; sending silently would be the one PRIN-09 forbids.

use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager as _};

use crate::capture::Rect;
use crate::format::outcome::ScreenshotMode;
use crate::format::patterns::patterns_version;
use crate::log::Timestamp;
use crate::ocr;
use crate::redaction::boxes::{detect, BoxLevel, Detection, RedactionPlan};
use crate::redaction::burn::{burn, redact_text, Burned, IMAGE_LONG_SIDE_PX};
use crate::redaction::suspected::Exemptions;
use crate::redaction::typed::redact;
use crate::store::{ScreenshotPayload, UserAction};

use super::commands::{Core, CoreState};
use super::events::NoticeKind;
use super::Ui;

/// What one capture was found to contain, and what the user has done to it since.
///
/// It is dropped when the capture is (a discard, another capture, a send), so nothing here
/// outlives the preview it belongs to (PRIN-04).
struct Analysis {
    /// The handoff the screenshot will be sent on. A preview belongs to one tab: the
    /// exemption list of DET-03 is that tab's, and so is the step the outcome names.
    handoff_id: String,
    /// The capture, at full resolution.
    image: Arc<RgbaImage>,
    /// The boxes, in the capture's own coordinates.
    detection: Detection,
    /// Unlock, add, crop — everything the user did (PREV-02).
    plan: RedactionPlan,
    /// The engine that answered, `None` when none did.
    engine: Option<String>,
    /// Why the capture could not be read, when it could not (FM-16, OCR-01).
    unread: Option<String>,
}

/// What the preview remembers between the capture and the send.
#[derive(Default)]
pub struct State {
    current: Mutex<Option<Analysis>>,
    /// The bytes of the last **Send image**, for the automation channel of §11.5.
    ///
    /// E2E-3 asserts on the sent PNG itself — "OCR of the sent PNG finds no certain
    /// pattern" — and the harness has no OCR engine of its own, so the check is performed
    /// in the app, on the bytes that really left. Compiled out of every build that is not
    /// `--features e2e`, where nothing keeps a screenshot past the send (LOG-03, PRIN-04).
    #[cfg(feature = "e2e")]
    sent: Mutex<Option<Vec<u8>>>,
}

impl State {
    /// Drops whatever the preview was holding.
    pub(super) fn forget(&self) {
        *self.current.lock().expect("the preview mutex is poisoned") = None;
    }
}

/// One box the preview draws (§7.10, DET-01, PREV-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewBox {
    /// Stable for the life of one analysis; an edit names it.
    pub id: usize,
    /// Left edge, in the capture's own pixels.
    pub x: i32,
    /// Top edge, in the capture's own pixels.
    pub y: i32,
    /// Width, in the capture's own pixels.
    pub width: u32,
    /// Height, in the capture's own pixels.
    pub height: u32,
    /// `locked` for a certain match, `flagged` for a suspected one (DET-01).
    pub level: &'static str,
    /// The family or the rule that drew it, never the matched text (R-19).
    pub cause: &'static str,
    /// Whether the user has lifted it. Always false for a locked box.
    pub unlocked: bool,
}

/// The boxes and the crop, as the window draws them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewDraw {
    /// Every box the detectors found, plus the ones the user drew.
    pub boxes: Vec<PreviewBox>,
    /// The rectangle the user cropped to, in the capture's own pixels.
    pub crop: Option<Rect>,
    /// How many regions would be burned if **Send image** were pressed now.
    pub redactions: usize,
}

/// Everything the preview needs once the detectors have answered (OCR-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewAnalysis {
    /// The tab this preview belongs to.
    pub handoff_id: String,
    /// The capture's width in pixels.
    pub width: u32,
    /// The capture's height in pixels.
    pub height: u32,
    /// The boxes and the crop.
    #[serde(flatten)]
    pub draw: PreviewDraw,
    /// The recognised text, as the editable pane starts (PREV-03).
    pub text: String,
    /// The engine that read it (§7.9), `null` when none did.
    pub ocr_engine: Option<String>,
    /// Why nothing could be read, when nothing could (FM-16).
    pub unread: Option<String>,
    /// Whether **Send image** may be offered at all (PREV-04, FM-05).
    pub images_in_results: bool,
    /// Whether the capture is large enough to be worth sending as text (FM-30, PREV-05).
    pub large: bool,
}

/// What the text pane will send, and what the two detectors took out of it (§7.10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewText {
    /// The text as the agent will read it.
    pub text: String,
    /// The certain families that were replaced, without repetition. Never the text (R-19).
    pub kinds: Vec<String>,
    /// How many suspected tokens were replaced.
    pub suspected: usize,
}

/// One edit of the preview (PREV-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PreviewEdit {
    /// Lift a flagged box. A locked one is refused here and again at the burn (DET-01).
    Unlock { id: usize },
    /// Put a lifted box back.
    Relock { id: usize },
    /// A box the user drew, in the capture's own pixels.
    AddBox { rect: Rect },
    /// Keep only this rectangle of the capture.
    Crop { rect: Rect },
    /// Undo the crop.
    Uncrop,
}

/// What one **Send image** or **Send text** produced (LOG-03).
///
/// It carries no pixels and no recognised text: what the agent was given is in the outcome
/// and in the `sends` row, and this is what the window shows the user afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentScreenshot {
    /// Image or text (PREV-04).
    pub mode: ScreenshotMode,
    /// The width of what was sent.
    pub width: u32,
    /// The height of what was sent.
    pub height: u32,
    /// How many regions were taken out before it left (PRIN-09).
    pub redactions: u32,
    /// How many bytes the PNG weighs, `null` in text mode (A-20, FM-30).
    pub bytes: Option<usize>,
}

/// Runs the OCR and both detectors over the capture that is waiting (§7.9, §7.10, OCR-04).
///
/// Called by the preview as soon as it is on screen. Answering twice for the same capture
/// costs nothing: the second call finds the analysis already there and re-draws it, which
/// is what a webview that reloaded needs.
///
/// # Errors
///
/// When there is no capture to analyse, or when the tab it is for has gone.
#[tauri::command]
pub async fn analyze_capture(
    app: AppHandle,
    handoff_id: String,
) -> Result<PreviewAnalysis, String> {
    let Some(held) = app.state::<Ui>().capture.held() else {
        return Err("there is no capture to analyse".to_owned());
    };
    let core = core(&app);
    let images_in_results = images_in_results(core.as_ref(), &handoff_id).await;
    let large = held.image.width().max(held.image.height()) > IMAGE_LONG_SIDE_PX;

    // Answering twice for the same capture must not re-run the OCR, and must not lose the
    // boxes the user has already lifted: a webview that reloaded asks again.
    {
        let ui = app.state::<Ui>();
        let current = ui
            .preview
            .current
            .lock()
            .expect("the preview mutex is poisoned");
        if let Some(analysis) = current.as_ref() {
            if analysis.handoff_id == handoff_id && Arc::ptr_eq(&analysis.image, &held.image) {
                return Ok(describe(analysis, images_in_results, large));
            }
        }
    }

    let (exemptions, lang) = match core.as_ref() {
        Some(core) => (
            core.store.exemptions(handoff_id.clone()).await,
            core.store
                .snapshot(handoff_id.clone(), Timestamp::now())
                .await
                .and_then(|snapshot| snapshot.lang),
        ),
        None => (Exemptions::none(), None),
    };

    let read = ocr::select_and_run(Arc::clone(&held.image), lang).await;
    let (recognition, unread) = match read {
        Ok(recognition) => (Some(recognition), None),
        Err(error) => {
            tracing::warn!(error = %error, "a capture could not be read");
            (None, Some(error.to_string()))
        }
    };
    let blocks = recognition
        .as_ref()
        .map_or_else(Vec::new, |read| read.blocks.clone());
    let detection = detect(&blocks, &exemptions);

    let analysis = Analysis {
        handoff_id,
        image: Arc::clone(&held.image),
        detection,
        plan: RedactionPlan::new(exemptions),
        engine: recognition.map(|it| it.engine),
        unread,
    };
    let answer = describe(&analysis, images_in_results, large);
    *app.state::<Ui>()
        .preview
        .current
        .lock()
        .expect("the preview mutex is poisoned") = Some(analysis);
    Ok(answer)
}

/// Applies one edit and answers the whole drawing again (PREV-02).
///
/// # Errors
///
/// When there is no analysis to edit, or when the edit names a box that is not there or is
/// not the user's to lift (DET-01).
#[tauri::command]
pub fn edit_preview(app: AppHandle, edit: PreviewEdit) -> Result<PreviewDraw, String> {
    let ui = app.state::<Ui>();
    let mut current = ui
        .preview
        .current
        .lock()
        .expect("the preview mutex is poisoned");
    let analysis = current
        .as_mut()
        .ok_or_else(|| "there is no preview to edit".to_owned())?;
    match edit {
        PreviewEdit::Unlock { id } => {
            if !analysis.plan.unlock(&analysis.detection, id) {
                return Err(format!("box {id} is not the user's to lift"));
            }
        }
        PreviewEdit::Relock { id } => analysis.plan.relock(&analysis.detection, id),
        PreviewEdit::AddBox { rect } => analysis.plan.add_box(rect),
        PreviewEdit::Crop { rect } => analysis.plan.crop_to(rect),
        PreviewEdit::Uncrop => analysis.plan.uncrop(),
    }
    Ok(draw(analysis))
}

/// What the text pane would send right now (PREV-03, §7.10).
///
/// # Errors
///
/// When there is no analysis, so there is no plan to read the unlocked boxes from.
#[tauri::command]
pub fn preview_text(app: AppHandle, text: String) -> Result<PreviewText, String> {
    let ui = app.state::<Ui>();
    let current = ui
        .preview
        .current
        .lock()
        .expect("the preview mutex is poisoned");
    let analysis = current
        .as_ref()
        .ok_or_else(|| "there is no preview to read".to_owned())?;
    let redacted = redact_text(&text, &analysis.plan);
    Ok(PreviewText {
        text: redacted.text,
        kinds: redacted.kinds,
        suspected: redacted.suspected,
    })
}

/// **Send image** or **Send text** (PREV-01, PREV-04, CAP-06, LOG-03).
///
/// `text` is what is in the pane, used in text mode alone; `comment` is the optional line
/// beside the picture, which becomes `user_text` in the outcome. Both are redacted here as
/// well as in the window: what the user sees before the button is what leaves, and what
/// leaves is decided on this side whatever called the command.
///
/// # Errors
///
/// When there is no capture to send, when the burn or the encoder refused, or when the
/// store refused the action (§7.4).
#[tauri::command]
pub async fn send_screenshot(
    app: AppHandle,
    handoff_id: String,
    mode: ScreenshotMode,
    text: Option<String>,
    comment: Option<String>,
) -> Result<SentScreenshot, String> {
    let Some(core) = core(&app) else {
        return Err("the store is not running".to_owned());
    };
    let (image, detection, plan, engine) = {
        let ui = app.state::<Ui>();
        let current = ui
            .preview
            .current
            .lock()
            .expect("the preview mutex is poisoned");
        let analysis = current
            .as_ref()
            .ok_or_else(|| "there is no capture to send".to_owned())?;
        if analysis.handoff_id != handoff_id {
            return Err("this capture belongs to another handoff".to_owned());
        }
        (
            Arc::clone(&analysis.image),
            analysis.detection.clone(),
            analysis.plan.clone(),
            analysis.engine.clone(),
        )
    };

    let exemptions = core.store.exemptions(handoff_id.clone()).await;
    // The comment is typed text: the certain level replaces, the suspected level marks and
    // is sent as written, exactly as in the three sheets.
    let comment = comment
        .map(|written| redact(&written, &exemptions).text)
        .filter(|written| !written.trim().is_empty());

    let (payload, bytes) = match mode {
        ScreenshotMode::Image => {
            let burned = tokio::task::spawn_blocking(move || burn(&image, &detection, &plan))
                .await
                .map_err(|joined| joined.to_string())?
                .map_err(|error| error.to_string())?;
            let bytes = burned.png.len();
            #[cfg(feature = "e2e")]
            {
                *app.state::<Ui>()
                    .preview
                    .sent
                    .lock()
                    .expect("the preview mutex is poisoned") = Some(burned.png.clone());
            }
            (image_payload(&burned, engine, comment), Some(bytes))
        }
        ScreenshotMode::Text => {
            let pane = text.unwrap_or_default();
            let redacted = redact_text(&pane, &plan);
            let redactions = redacted.redactions();
            let area = plan
                .crop()
                .unwrap_or_else(|| Rect::new(0, 0, image.width(), image.height()));
            let payload = ScreenshotPayload {
                mode: ScreenshotMode::Text,
                text: Some(redacted.text),
                image_base64: None,
                image_sha256: None,
                // No image left, so these are the capture's — the screen the text was read
                // from, which is what §4.3's own example prints for a text-mode screenshot.
                width: area.width,
                height: area.height,
                redactions: u32::try_from(redactions).unwrap_or(u32::MAX),
                redaction_boxes_json: None,
                ocr_engine: engine,
                patterns_version: Some(patterns_version().to_string()),
                comment,
            };
            (payload, None)
        }
    };

    let sent = SentScreenshot {
        mode: payload.mode,
        width: payload.width,
        height: payload.height,
        redactions: payload.redactions,
        bytes,
    };

    let result = core
        .store
        .user(
            handoff_id,
            UserAction::Screenshot(Box::new(payload)),
            Timestamp::now(),
        )
        .await;
    // Whatever the store answered, the capture is over: the user pressed a send button and
    // there is nothing left to look at (PRIN-04).
    discard(&app);
    match result {
        Ok(()) => Ok(sent),
        Err(refusal) => {
            super::notifier(&app).notice(NoticeKind::Warning, super::NOTICE_ACTION_FAILED);
            Err(refusal.to_string())
        }
    }
}

/// The bytes of the last **Send image**, for the automation channel of §11.5 (E2E-3).
#[cfg(feature = "e2e")]
#[must_use]
pub fn last_sent_png(app: &AppHandle) -> Option<Vec<u8>> {
    app.state::<Ui>()
        .preview
        .sent
        .lock()
        .expect("the preview mutex is poisoned")
        .clone()
}

/// Drops the analysis and the pixels it was drawn from.
pub(super) fn discard(app: &AppHandle) {
    let ui = app.state::<Ui>();
    ui.preview.forget();
    ui.capture.forget();
}

/// The `ScreenshotPayload` of an image that has been burned (CAP-06, LOG-03).
fn image_payload(
    burned: &Burned,
    engine: Option<String>,
    comment: Option<String>,
) -> ScreenshotPayload {
    ScreenshotPayload {
        mode: ScreenshotMode::Image,
        text: None,
        image_base64: Some(BASE64.encode(&burned.png)),
        image_sha256: Some(burned.sha256.clone()),
        // The **sent** image's, not the capture's: they are the numbers the hash and the
        // boxes of the same `sends` row are about, and the picture the agent is looking at
        // (LOG-03). In text mode there is no image and they are the capture's instead.
        width: burned.width,
        height: burned.height,
        redactions: u32::try_from(burned.boxes.len()).unwrap_or(u32::MAX),
        redaction_boxes_json: serde_json::to_string(&burned.boxes).ok(),
        ocr_engine: engine,
        patterns_version: Some(patterns_version().to_string()),
        comment,
    }
}

/// The core, when the channel came up.
fn core(app: &AppHandle) -> Option<Core> {
    app.state::<CoreState>().inner().0.clone()
}

/// Whether the session that owns this handoff takes an image block (PREV-04, FM-05).
///
/// The row is the **server's** answer (§5.6, ADPT-03) and the app owns no agent facts of
/// its own. A handoff whose session is not in the registry — one restored from a previous
/// run, one whose agent has gone — answers `true`: the server gates the image block again
/// when it builds the tool result (§4.7.4), so the cost of being wrong here is a button
/// that was offered and not one that leaks a picture to a model that cannot read it.
async fn images_in_results(core: Option<&Core>, handoff_id: &str) -> bool {
    let Some(core) = core else {
        return true;
    };
    let Some(session_ref) = core
        .store
        .snapshot(handoff_id.to_owned(), Timestamp::now())
        .await
        .and_then(|snapshot| snapshot.session_ref)
    else {
        return true;
    };
    let registry = core
        .registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    registry.get(&session_ref).is_none_or(|session| {
        session
            .capability_row
            .as_ref()
            .is_none_or(|row| row.images_in_results)
    })
}

/// The boxes and the crop of one analysis.
fn draw(analysis: &Analysis) -> PreviewDraw {
    let boxes = analysis
        .detection
        .boxes
        .iter()
        .map(|drawn| PreviewBox {
            id: drawn.id,
            x: drawn.rect.x,
            y: drawn.rect.y,
            width: drawn.rect.width,
            height: drawn.rect.height,
            level: drawn.level.as_str(),
            cause: drawn.cause.as_str(),
            unlocked: analysis.plan.is_unlocked(drawn.id),
        })
        .chain(
            analysis
                .plan
                .added()
                .iter()
                .enumerate()
                .map(|(index, rect)| PreviewBox {
                    // Added boxes are numbered after the detected ones, so an id is unique
                    // across the drawing; nothing ever names one, since PREV-02 gives the
                    // user no way to lift a box they drew themselves.
                    id: analysis.detection.boxes.len() + index,
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                    level: BoxLevel::Locked.as_str(),
                    cause: "added",
                    unlocked: false,
                }),
        )
        .collect();
    PreviewDraw {
        boxes,
        crop: analysis.plan.crop(),
        redactions: analysis.plan.boxes(&analysis.detection).len(),
    }
}

/// One analysis, as the window is given it.
fn describe(analysis: &Analysis, images_in_results: bool, large: bool) -> PreviewAnalysis {
    PreviewAnalysis {
        handoff_id: analysis.handoff_id.clone(),
        width: analysis.image.width(),
        height: analysis.image.height(),
        draw: draw(analysis),
        text: analysis.detection.text.clone(),
        ocr_engine: analysis.engine.clone(),
        unread: analysis.unread.clone(),
        images_in_results,
        large,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::TextBlock;

    /// The frontend half of the bridge, so the two spellings of the commands stay together.
    const BRIDGE_TS: &str = include_str!("../../../src/bridge.ts");

    const AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

    fn analysis() -> Analysis {
        let blocks = vec![
            TextBlock {
                text: format!("Access key   {AWS_KEY}"),
                bbox: Rect::new(10, 10, 300, 30),
                confidence: 1.0,
            },
            TextBlock {
                text: "Created   9 September 2026".to_owned(),
                bbox: Rect::new(10, 50, 300, 30),
                confidence: 1.0,
            },
            TextBlock {
                text: "Session token   f4c3b2a1908877665544332211aabbcc".to_owned(),
                bbox: Rect::new(10, 90, 300, 30),
                confidence: 1.0,
            },
        ];
        let exemptions = Exemptions::none();
        Analysis {
            handoff_id: "hf_0123456789".to_owned(),
            image: Arc::new(RgbaImage::new(400, 200)),
            detection: detect(&blocks, &exemptions),
            plan: RedactionPlan::new(exemptions),
            engine: Some("windows".to_owned()),
            unread: None,
        }
    }

    #[test]
    fn a_certain_box_is_locked_and_a_suspected_one_is_not() {
        let drawing = draw(&analysis());
        let locked: Vec<&PreviewBox> = drawing
            .boxes
            .iter()
            .filter(|drawn| drawn.level == "locked")
            .collect();
        let flagged: Vec<&PreviewBox> = drawing
            .boxes
            .iter()
            .filter(|drawn| drawn.level == "flagged")
            .collect();
        assert_eq!(locked.len(), 1, "{:#?}", drawing.boxes);
        assert_eq!(locked[0].cause, "api_key");
        assert!(!flagged.is_empty(), "{:#?}", drawing.boxes);
        assert_eq!(drawing.redactions, drawing.boxes.len());
    }

    #[test]
    fn the_user_may_lift_a_flagged_box_and_never_a_locked_one() {
        let mut analysis = analysis();
        let locked = analysis
            .detection
            .boxes
            .iter()
            .find(|drawn| drawn.level == BoxLevel::Locked)
            .expect("a locked box")
            .id;
        let flagged = analysis
            .detection
            .boxes
            .iter()
            .find(|drawn| drawn.level == BoxLevel::Flagged)
            .expect("a flagged box")
            .id;

        assert!(!analysis.plan.unlock(&analysis.detection, locked));
        assert!(analysis.plan.unlock(&analysis.detection, flagged));
        let drawing = draw(&analysis);
        assert_eq!(drawing.redactions, drawing.boxes.len() - 1);
        // DET-01 from the other side: the burn still covers the certain match, whatever the
        // unlock set says.
        assert!(analysis
            .plan
            .boxes(&analysis.detection)
            .contains(&Rect::new(10, 10, 300, 30)));
    }

    #[test]
    fn a_box_the_user_drew_is_numbered_after_the_detected_ones() {
        let mut analysis = analysis();
        let detected = analysis.detection.boxes.len();
        analysis.plan.add_box(Rect::new(0, 0, 20, 20));
        let drawing = draw(&analysis);
        assert_eq!(drawing.boxes.len(), detected + 1);
        let added = drawing.boxes.last().expect("the added box");
        assert_eq!(added.id, detected);
        assert_eq!(added.cause, "added");
        assert_eq!(added.level, "locked");
    }

    #[test]
    fn the_text_pane_reports_what_it_would_send() {
        let analysis = analysis();
        let redacted = redact_text(
            &format!("the key is {AWS_KEY} and nothing else"),
            &analysis.plan,
        );
        assert!(!redacted.text.contains(AWS_KEY));
        assert_eq!(redacted.kinds, vec!["api_key".to_owned()]);
        assert!(redacted.redactions() >= 1);
    }

    #[test]
    fn an_image_payload_carries_the_hash_and_never_the_pixels_twice() {
        let analysis = analysis();
        let burned = burn(&analysis.image, &analysis.detection, &analysis.plan)
            .expect("an empty image still burns");
        let payload = image_payload(&burned, Some("windows".to_owned()), None);
        assert_eq!(
            payload.image_sha256.as_deref(),
            Some(burned.sha256.as_str())
        );
        assert_eq!(payload.width, burned.width);
        assert_eq!(payload.height, burned.height);
        assert_eq!(
            payload.redaction_boxes_json,
            serde_json::to_string(&burned.boxes).ok()
        );
        assert_eq!(
            payload.patterns_version.as_deref(),
            Some(patterns_version().to_string().as_str())
        );
        let encoded = payload.image_base64.expect("an image travels base64");
        assert_eq!(
            BASE64.decode(&encoded).expect("it decodes"),
            burned.png,
            "the bytes on the channel must be the bytes that were burned"
        );
    }

    #[test]
    fn the_edits_are_spelled_the_way_the_window_spells_them() {
        // The tag is part of the payload the webview sends, so a rename here has to break a
        // test rather than a click.
        for (json, expected) in [
            (r#"{"kind":"unlock","id":3}"#, PreviewEdit::Unlock { id: 3 }),
            (r#"{"kind":"relock","id":3}"#, PreviewEdit::Relock { id: 3 }),
            (
                r#"{"kind":"addBox","rect":{"x":1,"y":2,"width":3,"height":4}}"#,
                PreviewEdit::AddBox {
                    rect: Rect::new(1, 2, 3, 4),
                },
            ),
            (
                r#"{"kind":"crop","rect":{"x":1,"y":2,"width":3,"height":4}}"#,
                PreviewEdit::Crop {
                    rect: Rect::new(1, 2, 3, 4),
                },
            ),
            (r#"{"kind":"uncrop"}"#, PreviewEdit::Uncrop),
        ] {
            let parsed: PreviewEdit = serde_json::from_str(json).expect("an edit parses");
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn the_analysis_is_flattened_the_way_the_window_reads_it() {
        let described = describe(&analysis(), false, true);
        let json = serde_json::to_value(&described).expect("an analysis serialises");
        assert_eq!(json["handoffId"], "hf_0123456789");
        assert_eq!(json["ocrEngine"], "windows");
        assert_eq!(json["imagesInResults"], false);
        assert_eq!(json["large"], true);
        assert!(json["boxes"].is_array(), "the draw is flattened into it");
        assert!(json["crop"].is_null());
        assert!(json["unread"].is_null());
    }

    #[test]
    fn the_frontend_calls_the_commands_this_module_registers() {
        for command in [
            "analyze_capture",
            "edit_preview",
            "preview_text",
            "send_screenshot",
        ] {
            assert!(
                BRIDGE_TS.contains(command),
                "src/bridge.ts does not invoke {command}"
            );
        }
    }
}
