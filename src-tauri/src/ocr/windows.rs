//! The Windows OCR engine: `Windows.Media.Ocr` (§7.9, OCR-02, A-14).
//!
//! It is the operating system's own recogniser, offline and local (NFR-02): the language
//! models are the ones Windows already installed for the keyboard and the display language,
//! and no part of a capture leaves the process.
//!
//! # Availability is a language question (A-14, FM-16)
//!
//! `Windows.Media.Ocr` can only recognise a language whose **OCR language pack** is present,
//! and a Windows install that has never had a second language has exactly one. So
//! [`WindowsOcr::available`] is not "is this Windows" — it is `TryCreateFromLanguage` for
//! the spec's `lang` (OCR-05), then `TryCreateFromUserProfileLanguages` for whatever the
//! user does have, and `false` when neither answers. That `false` is FM-16, and it is the
//! whole reason §7.9 bundles a second engine.
//!
//! # The apartment, and why this must not run on the main thread
//!
//! `RecognizeAsync` answers a WinRT operation and [`block_on`] waits for it. Tauri's main
//! thread is a single-threaded apartment running the window's message loop, so waiting
//! there would stop the overlay and could deadlock the completion. [`super::run_with`] puts
//! every attempt on a blocking thread, which is implicitly multi-threaded, and that is where
//! the WinRT activation this file performs is safe. Nothing here may be called from a
//! `#[tauri::command]` body directly.

use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use image::RgbaImage;
use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine as WinRtOcr;
use windows::Storage::Streams::DataWriter;

use super::{OcrEngine, OcrError, TextBlock};
use crate::capture::Rect;

/// `Windows.Media.Ocr`, as an engine of §7.9.
#[derive(Debug, Clone, Copy)]
pub struct WindowsOcr;

impl OcrEngine for WindowsOcr {
    fn name(&self) -> &str {
        // The vocabulary of `screenshot.ocr_engine`, beside `vision` and `ocrs`.
        "windows"
    }

    fn available(&self, lang_hint: Option<&str>) -> bool {
        recognizer(lang_hint).is_some()
    }

    fn recognize(
        &self,
        image: &RgbaImage,
        lang_hint: Option<&str>,
    ) -> Result<Vec<TextBlock>, OcrError> {
        let engine = recognizer(lang_hint).ok_or_else(|| {
            OcrError::Engine("no OCR language pack is installed on this machine".to_owned())
        })?;
        let bitmap = software_bitmap(image)?;
        let operation = engine.RecognizeAsync(&bitmap).map_err(win)?;
        let result = block_on(operation).map_err(win)?;

        let mut blocks = Vec::new();
        for line in result.Lines().map_err(win)?.into_iter() {
            let text = line.Text().map_err(win)?.to_string_lossy();
            if text.trim().is_empty() {
                continue;
            }
            let Some(bbox) = line_bounds(&line, image.width(), image.height())? else {
                // A line the engine placed nowhere cannot be redacted, and a box that is
                // not drawn is worse than a line that is not reported (PRIN-09).
                continue;
            };
            blocks.push(TextBlock {
                text,
                bbox,
                // `Windows.Media.Ocr` reports no per-line score. See `TextBlock::confidence`.
                confidence: 1.0,
            });
        }
        Ok(blocks)
    }
}

/// An engine for `lang_hint`, else one for the languages of the user's profile.
///
/// Both `Try…` calls answer with a null interface when no pack matches, which windows-rs
/// surfaces as an error; there is nothing to distinguish there, so either way the answer is
/// "not this language".
fn recognizer(lang_hint: Option<&str>) -> Option<WinRtOcr> {
    if let Some(tag) = lang_hint.map(str::trim).filter(|tag| !tag.is_empty()) {
        let asked = Language::CreateLanguage(&HSTRING::from(tag))
            .and_then(|language| WinRtOcr::TryCreateFromLanguage(&language));
        if let Ok(engine) = asked {
            return Some(engine);
        }
    }
    WinRtOcr::TryCreateFromUserProfileLanguages().ok()
}

/// The capture as the `Bgra8` bitmap `RecognizeAsync` takes.
///
/// Two conversions happen here and both are deliberate. The channel order is swapped
/// because WinRT has no RGBA bitmap; and the alpha channel is forced opaque, because
/// `CreateCopyFromBuffer` declares the pixels **premultiplied** and the composite of §7.8
/// leaves fully transparent pixels wherever no monitor covers the region — which, read as
/// premultiplied, is exactly the black those gaps already look like. Making it explicit
/// keeps the engine from ever being handed a pixel whose colour depends on how the reader
/// interprets its alpha.
fn software_bitmap(image: &RgbaImage) -> Result<SoftwareBitmap, OcrError> {
    let max = WinRtOcr::MaxImageDimension().map_err(win)?;
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(OcrError::Engine("the capture has no pixels".to_owned()));
    }
    if width > max || height > max {
        return Err(OcrError::Engine(format!(
            "the capture is {width}×{height}, past the {max} px this engine accepts"
        )));
    }

    let mut bgra = Vec::with_capacity(image.as_raw().len());
    for pixel in image.pixels() {
        let [red, green, blue, _] = pixel.0;
        bgra.extend_from_slice(&[blue, green, red, 0xff]);
    }

    let writer = DataWriter::new().map_err(win)?;
    writer.WriteBytes(&bgra).map_err(win)?;
    let buffer = writer.DetachBuffer().map_err(win)?;
    let (width, height) = (
        i32::try_from(width).map_err(|_| OcrError::Engine("the capture is too wide".to_owned()))?,
        i32::try_from(height)
            .map_err(|_| OcrError::Engine("the capture is too tall".to_owned()))?,
    );
    SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, width, height)
        .map_err(win)
}

/// The rectangle covering every word of `line`, in whole pixels of the source image.
///
/// The engine reports each word in floating-point pixels of the bitmap it was given, which
/// is the capture itself, so no scaling is involved — only rounding, and it rounds
/// **outwards**: a box one pixel too large hides a pixel that did not need hiding, a box one
/// pixel too small leaves a sliver of a secret on screen (CAP-06, PRIN-09).
fn line_bounds(
    line: &windows::Media::Ocr::OcrLine,
    width: u32,
    height: u32,
) -> Result<Option<Rect>, OcrError> {
    let mut bounds: Option<(f32, f32, f32, f32)> = None;
    for word in line.Words().map_err(win)?.into_iter() {
        let rect = word.BoundingRect().map_err(win)?;
        if !(rect.Width > 0.0 && rect.Height > 0.0) {
            continue;
        }
        let (left, top, right, bottom) =
            (rect.X, rect.Y, rect.X + rect.Width, rect.Y + rect.Height);
        bounds = Some(match bounds {
            None => (left, top, right, bottom),
            Some((l, t, r, b)) => (l.min(left), t.min(top), r.max(right), b.max(bottom)),
        });
    }
    let Some((left, top, right, bottom)) = bounds else {
        return Ok(None);
    };

    let clamp = |value: f32, limit: u32| value.max(0.0).min(limit as f32);
    let left = clamp(left.floor(), width) as u32;
    let top = clamp(top.floor(), height) as u32;
    let right = clamp(right.ceil(), width) as u32;
    let bottom = clamp(bottom.ceil(), height) as u32;
    if right <= left || bottom <= top {
        return Ok(None);
    }
    Ok(Some(Rect::new(
        i32::try_from(left).unwrap_or(i32::MAX),
        i32::try_from(top).unwrap_or(i32::MAX),
        right - left,
        bottom - top,
    )))
}

/// A WinRT failure, as the message the fallback rule logs.
fn win(error: windows::core::Error) -> OcrError {
    OcrError::Engine(error.message())
}

/// Wait, on this thread, for a WinRT operation to finish.
///
/// `windows` 0.62 dropped the blocking `get()` of earlier versions: an operation is now a
/// `IntoFuture`, meant to be awaited. This module cannot await — [`OcrEngine::recognize`] is
/// synchronous by §7.9, and it is called on a blocking thread precisely so that it may wait
/// — so the thread parks until the completion handler wakes it. It is twenty lines instead
/// of a runtime because the alternatives are worse: a nested tokio runtime inside a
/// `spawn_blocking` thread, or polling `Status()` with a sleep, which would be the only
/// timer in this crate that waits for nothing in particular.
///
/// There is no deadline here on purpose. The bound is [`super::OCR_ENGINE_TIMEOUT_MS`],
/// applied by the caller around the whole attempt, which is the only place that can also
/// move on to the next engine.
fn block_on<F: IntoFuture>(operation: F) -> F::Output {
    /// Wakes the thread that is parked inside `block_on`.
    struct Unpark(std::thread::Thread);

    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let mut future = std::pin::pin!(operation.into_future());
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            // `park` may return without anyone unparking, which is why this is a loop and
            // not one wait.
            Poll::Pending => std::thread::park(),
        }
    }
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;
    use crate::ocr::paint;

    /// Whether this machine has any OCR language pack at all.
    ///
    /// The tests that need a recogniser skip themselves without one rather than failing:
    /// A-14 says the packs are what decides, this suite runs on a developer machine and on
    /// a hosted runner, and a red test would be reporting the machine, not the code. The
    /// selection rule that covers the missing-pack case is tested with stubs in the parent
    /// module, on every machine.
    fn has_a_recognizer() -> bool {
        WindowsOcr.available(None)
    }

    #[test]
    fn the_painted_text_is_black_on_white() {
        // A control on the harness itself: an OCR assertion over a blank image would pass
        // for the wrong reason for ever.
        let image = paint::text(600, 120, 40, &["Baton"]);
        assert_eq!(image.dimensions(), (600, 120));
        assert_eq!(*image.get_pixel(599, 119), Rgba([255, 255, 255, 255]));
        let dark = image
            .pixels()
            .filter(|pixel| pixel.0[0] < 128 && pixel.0[1] < 128 && pixel.0[2] < 128)
            .count();
        assert!(
            dark > 200,
            "the font drew almost nothing: {dark} dark pixels"
        );
    }

    #[test]
    fn it_reads_a_dashboard_line_with_a_key_in_it() {
        if !has_a_recognizer() {
            eprintln!("skipped: no OCR language pack on this machine (A-14, FM-16)");
            return;
        }
        // The shape §11.7's corpus is about: a label and a key-looking value, which is what
        // the certain detector of T-048 has to find in the blocks this returns.
        let image = paint::text(
            900,
            220,
            44,
            &["Secret key", "sk test 4eC39HqLyjWDarjtT1zdp7dc"],
        );
        let blocks = WindowsOcr
            .recognize(&image, Some("en"))
            .expect("the engine reads the image");

        let read: Vec<&str> = blocks.iter().map(|block| block.text.as_str()).collect();
        let joined = read.join("\n");
        assert!(
            joined.contains("Secret key"),
            "the label was not read: {read:?}"
        );
        assert!(
            joined.contains("4eC39HqLyjWDarjtT1zdp7dc"),
            "the key was not read: {read:?}"
        );
        assert_eq!(blocks.len(), 2, "one block per line: {read:?}");
    }

    #[test]
    fn every_block_is_placed_inside_the_image_it_came_from() {
        if !has_a_recognizer() {
            eprintln!("skipped: no OCR language pack on this machine (A-14, FM-16)");
            return;
        }
        // The boxes are what gets filled with black before anything is sent (CAP-06), so
        // this asserts what a redaction needs and not merely that the numbers are sane: no
        // box leaves the image, **every dark pixel is inside one**, and they do not simply
        // cover the picture. A box off by an offset, off by a scale factor, or given the
        // whole frame to be safe, fails one of the three.
        let image = paint::text(900, 220, 44, &["Secret key", "sk test 4eC39Hq"]);
        let blocks = WindowsOcr
            .recognize(&image, None)
            .expect("the engine reads the image");
        assert!(!blocks.is_empty());
        for block in &blocks {
            assert!(block.bbox.x >= 0 && block.bbox.y >= 0, "{block:?}");
            assert!(block.bbox.width > 0 && block.bbox.height > 0, "{block:?}");
            let right = block.bbox.x as i64 + i64::from(block.bbox.width);
            let bottom = block.bbox.y as i64 + i64::from(block.bbox.height);
            assert!(right <= i64::from(image.width()), "{block:?}");
            assert!(bottom <= i64::from(image.height()), "{block:?}");
            assert!((0.0..=1.0).contains(&block.confidence), "{block:?}");
        }

        let covers = |x: u32, y: u32| {
            blocks.iter().any(|block| {
                let (left, top) = (block.bbox.x as i64, block.bbox.y as i64);
                let (x, y) = (i64::from(x), i64::from(y));
                x >= left
                    && y >= top
                    && x < left + i64::from(block.bbox.width)
                    && y < top + i64::from(block.bbox.height)
            })
        };
        let escaped = image
            .enumerate_pixels()
            .filter(|(x, y, pixel)| pixel.0[0] < 100 && !covers(*x, *y))
            .count();
        assert_eq!(escaped, 0, "{escaped} dark pixels fall outside every block");

        let covered: u32 = blocks
            .iter()
            .map(|block| block.bbox.width * block.bbox.height)
            .sum();
        assert!(
            covered * 2 < image.width() * image.height(),
            "the blocks cover {covered} of {} pixels, which is not a redaction any more",
            image.width() * image.height()
        );

        // The second line is drawn below the first, so the blocks come back in reading
        // order — which is what the "same line or the line below" rule of §7.10 needs.
        assert!(blocks[0].bbox.y < blocks[1].bbox.y);
    }

    #[test]
    fn a_language_nobody_has_falls_back_to_the_profiles_own() {
        // OCR-05 makes `lang` a hint, not a demand: a spec written in a language with no
        // pack must still be read, in whatever the machine does have.
        assert_eq!(
            WindowsOcr.available(Some("zz")),
            has_a_recognizer(),
            "an unknown tag must not make the engine disappear"
        );
        assert_eq!(WindowsOcr.available(Some("")), has_a_recognizer());
    }

    #[test]
    fn an_empty_capture_is_refused_before_it_reaches_the_engine() {
        let error = software_bitmap(&RgbaImage::new(0, 0)).expect_err("no pixels");
        assert!(error.to_string().contains("no pixels"));
    }

    #[test]
    fn the_engine_is_named_as_the_outcome_names_it() {
        assert_eq!(WindowsOcr.name(), "windows");
    }
}
