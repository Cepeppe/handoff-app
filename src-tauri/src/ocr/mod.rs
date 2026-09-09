//! OCR (§7.9, OCR-01..05, DD-30).
//!
//! One [`OcrEngine`] trait, one implementation per platform behind it, and the selection
//! rule of §7.9: the operating system's engine when it is available, the bundled one
//! otherwise, and an engine that errors or outstays [`OCR_ENGINE_TIMEOUT_MS`] falls through
//! to the next. Everything is local and offline — no OCR text ever leaves the machine
//! (NFR-02) — and the engine that answered is recorded, because the outcome and the log
//! both carry it (`screenshot.ocr_engine`).
//!
//! # Why the trait, and why the fallback
//!
//! OCR-01 makes this path compulsory: OCR runs on **every** capture, in image mode as much
//! as in text mode, because the secret detection of §7.10 is built on what it reads. So a
//! platform engine that is missing a language pack cannot be allowed to mean "no
//! detection" — FM-16 is exactly that case, and its answer is the bundled engine. The trait
//! is what keeps the runtime risk of a native engine local: a failure inside one is one
//! `Err`, and the caller above never learns which engine produced the blocks it got.
//!
//! # The granularity of a block
//!
//! A [`TextBlock`] is **one line** of recognised text. That is the one granularity all
//! three engines of §7.9 produce faithfully — Vision returns line observations, `OcrLine`
//! is the unit `Windows.Media.Ocr` reports, and Tesseract can be asked for it — and the
//! parity matters more than the precision: a fallback that changed the shape of the result
//! would change what §7.10 detects, and FM-16 promises lower accuracy, not a different
//! pipeline. Word boxes are recoverable from the pixels; a line the engine never separated
//! is not.
//!
//! # What runs where
//!
//! [`select_and_run`] is `async` and does no work of its own: each attempt goes to a
//! blocking thread (§7.9) so the preview can already be on screen in its "analyzing" state
//! (OCR-04) while a native engine holds a thread for several seconds. On Windows that is
//! also load-bearing for a second reason, written down in [`windows`].
//!
//! # The bundled engine is not here yet
//!
//! OCR-03 bundles Tesseract as the fallback and this module has no `tesseract.rs`: no
//! published crate builds it from source without a system install, and the ways round that
//! are decisions about what the product ships rather than implementation choices. The
//! survey, the four options and their costs are in `DEVIATIONS.md` and in the T-047 entry
//! of `TASKS.md`; until one is chosen, [`engines`] returns the platform engine alone and
//! FM-16 has no answer on a machine whose language pack is missing.
// TASK: T-047 — the bundled fallback engine of OCR-03.

#[cfg(target_os = "macos")]
pub mod vision;
#[cfg(target_os = "windows")]
pub mod windows;

use std::sync::Arc;
use std::time::Duration;

use image::RgbaImage;

use crate::capture::Rect;

/// The budget one engine attempt gets before the next one is tried (§4.1, §7.9).
pub const OCR_ENGINE_TIMEOUT_MS: u64 = 10_000;

/// One line of recognised text, in the coordinates of the image it was read from (§7.9).
#[derive(Debug, Clone, PartialEq)]
pub struct TextBlock {
    /// The line, as the engine read it.
    pub text: String,
    /// Where it is, in pixels of the full-resolution capture (CAP-05, CAP-06).
    pub bbox: Rect,
    /// How sure the engine is, from 0.0 to 1.0. `1.0` from an engine that reports none:
    /// a redactor that treated "no score" as "low score" would drop boxes it must draw.
    pub confidence: f32,
}

/// Why an OCR pass produced nothing.
#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    /// One engine's own failure, in its own words. Never carries recognised text: what is
    /// on the user's screen must not reach a log line (LOG-03, R-19).
    #[error("{0}")]
    Engine(String),
    /// Every engine of this build was unavailable, failed or ran out of its budget. The
    /// string is one reason per engine, in the order they were tried, for the log.
    #[error("no OCR engine could read the capture ({0})")]
    NoEngine(String),
}

/// What one OCR pass produced, and which engine produced it.
#[derive(Debug, Clone)]
pub struct Recognition {
    /// The engine's [`OcrEngine::name`]. It travels to the agent as
    /// `screenshot.ocr_engine` and is stored with the send (§7.9, §7.11).
    pub engine: String,
    /// The lines, in the order the engine reported them.
    pub blocks: Vec<TextBlock>,
}

/// One way of turning pixels into lines of text (§7.9).
///
/// Implementations are stateless and shared across threads: [`select_and_run`] hands one to
/// a blocking thread and may abandon it when its budget runs out, so nothing here may hold
/// a resource whose owner has to be told.
pub trait OcrEngine: Send + Sync {
    /// The name recorded in the outcome and the log: `windows`, `vision`, `tesseract`.
    ///
    /// It is the vocabulary the published outcome fixtures already use
    /// (`fixtures/outcomes/screenshot.json` carries `"ocr_engine": "vision"`), so it is a
    /// value an agent may see and not an internal label.
    fn name(&self) -> &str;

    /// Whether this engine can read `lang_hint` — or anything at all, when there is none.
    ///
    /// It must be cheap and must not recognise anything: it is asked before every capture.
    fn available(&self, lang_hint: Option<&str>) -> bool;

    /// The lines of `image`, at its own resolution (CAP-05).
    ///
    /// `lang_hint` is the spec's `lang` (OCR-05), a BCP-47 tag or `None`. An engine that
    /// cannot honour it recognises in whatever language it has rather than refusing.
    ///
    /// # Errors
    ///
    /// [`OcrError::Engine`] with the platform's message. The caller falls through to the
    /// next engine, so a failure here is never fatal by itself.
    fn recognize(
        &self,
        image: &RgbaImage,
        lang_hint: Option<&str>,
    ) -> Result<Vec<TextBlock>, OcrError>;
}

/// The engines of this platform, in the order §7.9 tries them: the operating system's
/// first, the bundled one after it.
///
/// The bundled one is missing; see the module documentation.
#[must_use]
pub fn engines() -> Vec<Arc<dyn OcrEngine>> {
    #[cfg(target_os = "windows")]
    let os_engine: Option<Arc<dyn OcrEngine>> = Some(Arc::new(windows::WindowsOcr));
    #[cfg(target_os = "macos")]
    let os_engine: Option<Arc<dyn OcrEngine>> = Some(Arc::new(vision::VisionOcr));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let os_engine: Option<Arc<dyn OcrEngine>> = None;
    // The bundled engine of OCR-03 goes after this one, when there is one to put here.
    os_engine.into_iter().collect()
}

/// Read `image` with the first engine of this platform that answers (§7.9).
///
/// # Errors
///
/// [`OcrError::NoEngine`] when every engine was unavailable, refused or outstayed its
/// budget, with one reason each. There is no partial answer: a caller that got `Err` has to
/// treat the capture as unread, which is what FM-16 degrades to when nothing is bundled.
pub async fn select_and_run(
    image: Arc<RgbaImage>,
    lang_hint: Option<String>,
) -> Result<Recognition, OcrError> {
    run_with(
        engines(),
        image,
        lang_hint,
        Duration::from_millis(OCR_ENGINE_TIMEOUT_MS),
    )
    .await
}

/// [`select_and_run`] over a given list and budget, which is what the tests drive.
///
/// Each attempt runs on a blocking thread (§7.9). A thread whose budget runs out is
/// **abandoned, not stopped**: a native OCR call cannot be interrupted, so the honest cost
/// of the 10 s bound is one blocking-pool thread finishing an answer nobody reads. The
/// alternative — waiting for it — is the hang the bound exists to prevent.
///
/// # Errors
///
/// As [`select_and_run`].
pub async fn run_with(
    engines: Vec<Arc<dyn OcrEngine>>,
    image: Arc<RgbaImage>,
    lang_hint: Option<String>,
    budget: Duration,
) -> Result<Recognition, OcrError> {
    if engines.is_empty() {
        return Err(OcrError::NoEngine(
            "no OCR engine is compiled into this build".to_owned(),
        ));
    }
    let mut refused = Vec::new();
    for engine in engines {
        let name = engine.name().to_owned();
        if !engine.available(lang_hint.as_deref()) {
            refused.push(format!("{name}: unavailable"));
            continue;
        }
        let pixels = Arc::clone(&image);
        let hint = lang_hint.clone();
        let attempt =
            tokio::task::spawn_blocking(move || engine.recognize(&pixels, hint.as_deref()));
        match tokio::time::timeout(budget, attempt).await {
            Ok(Ok(Ok(blocks))) => {
                return Ok(Recognition {
                    engine: name,
                    blocks,
                })
            }
            Ok(Ok(Err(error))) => refused.push(format!("{name}: {error}")),
            Ok(Err(joined)) => refused.push(format!("{name}: {joined}")),
            Err(_) => refused.push(format!(
                "{name}: no answer within {} ms",
                budget.as_millis()
            )),
        }
        tracing::warn!(
            engine = %name,
            "an OCR engine did not answer; falling back to the next one"
        );
    }
    Err(OcrError::NoEngine(refused.join("; ")))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use image::Rgba;

    use super::*;

    /// An engine whose whole behaviour is written down at the call site, and which counts
    /// the times it was asked to recognise anything.
    struct Stub {
        name: &'static str,
        available: bool,
        answer: Answer,
        calls: Arc<AtomicUsize>,
    }

    enum Answer {
        Lines(&'static str),
        Fails(&'static str),
        /// Sleeps past any budget a test sets, standing in for a native engine that hangs.
        Hangs,
    }

    impl Stub {
        /// The engine and the counter of the times it was asked to recognise anything.
        fn wired(
            name: &'static str,
            available: bool,
            answer: Answer,
        ) -> (Arc<dyn OcrEngine>, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            let engine = Arc::new(Self {
                name,
                available,
                answer,
                calls: Arc::clone(&calls),
            });
            (engine, calls)
        }
    }

    impl OcrEngine for Stub {
        fn name(&self) -> &str {
            self.name
        }

        fn available(&self, _lang_hint: Option<&str>) -> bool {
            self.available
        }

        fn recognize(
            &self,
            _image: &RgbaImage,
            _lang_hint: Option<&str>,
        ) -> Result<Vec<TextBlock>, OcrError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.answer {
                Answer::Lines(text) => Ok(vec![TextBlock {
                    text: text.to_owned(),
                    bbox: Rect::new(0, 0, 10, 4),
                    confidence: 1.0,
                }]),
                Answer::Fails(why) => Err(OcrError::Engine(why.to_owned())),
                Answer::Hangs => {
                    std::thread::sleep(Duration::from_millis(5_000));
                    Ok(Vec::new())
                }
            }
        }
    }

    fn an_image() -> Arc<RgbaImage> {
        Arc::new(RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 255])))
    }

    #[tokio::test]
    async fn the_first_available_engine_answers_and_is_named() {
        let (os, os_calls) = Stub::wired("windows", true, Answer::Lines("sk_live_notarealkey"));
        let (bundled, bundled_calls) = Stub::wired("tesseract", true, Answer::Lines("never asked"));
        let recognition = run_with(
            vec![os, bundled],
            an_image(),
            None,
            Duration::from_millis(500),
        )
        .await
        .expect("the first engine answers");

        assert_eq!(recognition.engine, "windows");
        assert_eq!(recognition.blocks.len(), 1);
        assert_eq!(recognition.blocks[0].text, "sk_live_notarealkey");
        assert_eq!(os_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            bundled_calls.load(Ordering::SeqCst),
            0,
            "the fallback is not run when the first engine answered"
        );
    }

    #[tokio::test]
    async fn an_unavailable_os_engine_falls_through_to_the_bundled_one() {
        // FM-16 as the user meets it: no language pack, so the OS engine is not even asked.
        let (os, os_calls) = Stub::wired("windows", false, Answer::Lines("never asked"));
        let (bundled, _) = Stub::wired("tesseract", true, Answer::Lines("read by the fallback"));
        let recognition = run_with(
            vec![os, bundled],
            an_image(),
            Some("it".to_owned()),
            Duration::from_millis(500),
        )
        .await
        .expect("the fallback answers");

        assert_eq!(recognition.engine, "tesseract");
        assert_eq!(recognition.blocks[0].text, "read by the fallback");
        assert_eq!(
            os_calls.load(Ordering::SeqCst),
            0,
            "an unavailable engine is never handed an image"
        );
    }

    #[tokio::test]
    async fn an_engine_that_errors_falls_through_to_the_next() {
        let (os, _) = Stub::wired("windows", true, Answer::Fails("the API refused"));
        let (bundled, _) = Stub::wired("tesseract", true, Answer::Lines("read by the fallback"));
        let recognition = run_with(
            vec![os, bundled],
            an_image(),
            None,
            Duration::from_millis(500),
        )
        .await
        .expect("the fallback answers");

        assert_eq!(recognition.engine, "tesseract");
    }

    #[tokio::test]
    async fn an_engine_that_outstays_its_budget_falls_through_to_the_next() {
        // The 10 s of §4.1, shortened to a tenth of a second so the suite does not wait.
        // The abandoned thread is still sleeping when this test ends, deliberately: the
        // point is that the caller does not.
        let (os, _) = Stub::wired("windows", true, Answer::Hangs);
        let (bundled, _) = Stub::wired("tesseract", true, Answer::Lines("read by the fallback"));
        let started = std::time::Instant::now();
        let recognition = run_with(
            vec![os, bundled],
            an_image(),
            None,
            Duration::from_millis(100),
        )
        .await
        .expect("the fallback answers");

        assert_eq!(recognition.engine, "tesseract");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "the caller waited for the hung engine instead of moving on: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn every_engine_failing_reports_each_reason_in_order() {
        let (os, _) = Stub::wired("windows", false, Answer::Lines("never asked"));
        let (bundled, _) = Stub::wired("tesseract", true, Answer::Fails("no traineddata"));
        let error = run_with(
            vec![os, bundled],
            an_image(),
            None,
            Duration::from_millis(500),
        )
        .await
        .expect_err("nothing could read it");

        let said = error.to_string();
        assert!(said.contains("windows: unavailable"), "{said}");
        assert!(said.contains("tesseract: no traineddata"), "{said}");
        assert!(
            said.find("windows").unwrap() < said.find("tesseract").unwrap(),
            "the reasons are in the order the engines were tried: {said}"
        );
    }

    #[tokio::test]
    async fn a_build_with_no_engine_says_so_rather_than_answering_nothing() {
        // Not reachable on either supported platform, and the one answer that must not be
        // an empty `Ok`: OCR-01 makes the detector's input compulsory, so "no engine" and
        // "no text on the screen" have to be different answers.
        let error = run_with(Vec::new(), an_image(), None, Duration::from_millis(500))
            .await
            .expect_err("there is no engine");
        assert!(error.to_string().contains("compiled into this build"));
    }

    #[test]
    fn this_platform_offers_the_operating_systems_engine_first() {
        let list = engines();
        if cfg!(any(target_os = "windows", target_os = "macos")) {
            let first = list.first().expect("a platform engine");
            assert_eq!(
                first.name(),
                if cfg!(target_os = "windows") {
                    "windows"
                } else {
                    "vision"
                }
            );
        }
        // The bundled engine of OCR-03 is not in the list yet (T-047); when it lands this
        // assertion is what says the OS engine still comes first.
        assert!(list.len() <= 2, "an engine appeared that nobody declared");
    }
}
