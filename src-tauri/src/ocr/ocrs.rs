//! The bundled fallback engine (§7.9, OCR-03, FM-16, A-26): `ocrs`, pure Rust, English.
//!
//! It is what answers when the operating system's own recogniser cannot: on Windows that
//! is a machine with no OCR language pack (A-14), which is the ordinary state of an install
//! that has never had a second keyboard language. OCR-01 makes the input of the detector
//! compulsory, so "the OS engine said no" cannot be allowed to mean "no detection"; this
//! file is the answer FM-16 promises.
//!
//! # Why this engine and not Tesseract
//!
//! OCR-03 named Tesseract until 2026-09-09 and the owner replaced it with `ocrs` on that
//! date, from a survey recorded in `DEVIATIONS.md`. The short reason is that no published
//! crate links Tesseract statically from a vendored source: the four that build it download
//! Leptonica and Tesseract from the network at build time through `reqwest` — which
//! `deny.toml` bans outright — the `-sys` crates want a system install, and the one crate
//! that really does link statically has no Windows target. `ocrs` is pure Rust, needs no
//! C++ toolchain, downloads nothing, and cross-compiles for every target this application
//! has; Tesseract as a separate signed helper stays the recorded alternative, taken only if
//! the measured quality here is not enough for a release.
//!
//! # The models are resources, not bytes in the binary
//!
//! The two `.rten` files are 12 MB and they are shipped as Tauri resources, listed under
//! `bundle.resources` in `tauri.conf.json` and committed unmodified under
//! `src-tauri/models/ocrs/` with the CC-BY-SA 4.0 attribution their licence asks for.
//! `include_bytes!` would have made them part of the executable, which costs the same disk
//! and loses the one property that matters here: an auditor can see, in the installed
//! folder, exactly which weights the application reads.
//!
//! Reading them is deferred to the first capture and done once ([`OcrsOcr::loaded`]):
//! parsing 12 MB of weights takes longer than recognising a screen, so an engine that
//! reloaded them per capture would spend most of its budget on the file.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use image::RgbaImage;
use ocrs::{ImageSource, OcrEngine as OcrsEngine, OcrEngineParams};

use super::{OcrEngine, OcrError, TextBlock};
use crate::capture::Rect;

/// The directory the two models live in, relative to the resource root and to the
/// repository alike — `bundle.resources` maps one onto the other.
const MODELS_DIR: &str = "models/ocrs";

/// The text detection model: where the words are.
const DETECTION_MODEL: &str = "text-detection.rten";

/// The text recognition model: what each line says.
const RECOGNITION_MODEL: &str = "text-recognition.rten";

/// How far above the executable the development lookup is allowed to walk.
///
/// Five is what reaches `src-tauri/` from the deepest place cargo puts a binary of this
/// crate (`src-tauri/target/<triple>/<profile>/deps/`). Walking further would let an
/// unrelated `models/ocrs` directory somewhere above an installation answer for the one
/// the bundle ships.
const DEVELOPMENT_LOOKUP_DEPTH: usize = 5;

/// `ocrs`, as an engine of §7.9.
pub struct OcrsOcr {
    /// Where [`DETECTION_MODEL`] and [`RECOGNITION_MODEL`] are read from.
    models: PathBuf,
    /// The engine, built on the first recognition and kept, or the reason there is none.
    /// A failure is remembered too: a machine whose models are missing must not pay for
    /// finding that out again on every capture.
    loaded: OnceLock<Result<OcrsEngine, String>>,
}

impl OcrsOcr {
    /// The engine over the models this build ships, shared by every caller.
    ///
    /// One instance per process, because the weights it ends up holding are the expensive
    /// part and they are immutable: [`super::engines`] is called once per capture.
    pub fn bundled() -> Arc<Self> {
        static BUNDLED: OnceLock<Arc<OcrsOcr>> = OnceLock::new();
        Arc::clone(BUNDLED.get_or_init(|| Arc::new(Self::at(models_dir()))))
    }

    /// The engine over the models in `dir`, which is what the tests drive.
    #[must_use]
    pub fn at(dir: PathBuf) -> Self {
        Self {
            models: dir,
            loaded: OnceLock::new(),
        }
    }

    /// The two model files, in the order [`load`] wants them.
    fn model_files(&self) -> [PathBuf; 2] {
        [
            self.models.join(DETECTION_MODEL),
            self.models.join(RECOGNITION_MODEL),
        ]
    }

    /// The loaded engine, or why there is none.
    fn engine(&self) -> Result<&OcrsEngine, OcrError> {
        self.loaded
            .get_or_init(|| load(&self.model_files()))
            .as_ref()
            .map_err(|why| OcrError::Engine(why.clone()))
    }
}

impl OcrEngine for OcrsOcr {
    fn name(&self) -> &str {
        // The vocabulary of `screenshot.ocr_engine`, beside `windows` and `vision`.
        "ocrs"
    }

    fn available(&self, _lang_hint: Option<&str>) -> bool {
        // Deliberately not a question about the language. This engine ships the English
        // models alone (OCR-03) and it is the last one in the list: answering `false` for
        // an Italian spec would leave FM-16 with nothing at all, where what FM-16 promises
        // is lower accuracy on non-English text. What can make it unavailable is a build
        // or an installation whose resources are missing, which is what this checks — and
        // it checks it without reading 12 MB, because §7.9 asks this before every capture.
        self.model_files().iter().all(|file| file.is_file())
    }

    fn recognize(
        &self,
        image: &RgbaImage,
        _lang_hint: Option<&str>,
    ) -> Result<Vec<TextBlock>, OcrError> {
        let (width, height) = image.dimensions();
        if width == 0 || height == 0 {
            return Err(OcrError::Engine("the capture has no pixels".to_owned()));
        }
        let engine = self.engine()?;
        let source = ImageSource::from_bytes(image.as_raw(), (width, height))
            .map_err(|error| OcrError::Engine(format!("the capture is unreadable: {error}")))?;
        let input = engine
            .prepare_input(source)
            .map_err(|error| OcrError::Engine(error.to_string()))?;
        let words = engine
            .detect_words(&input)
            .map_err(|error| OcrError::Engine(error.to_string()))?;
        let lines = engine.find_text_lines(&input, &words);
        let recognized = engine
            .recognize_text(&input, &lines)
            .map_err(|error| OcrError::Engine(error.to_string()))?;

        let mut blocks = Vec::new();
        for line in recognized.into_iter().flatten() {
            let text = line.to_string();
            if text.trim().is_empty() {
                continue;
            }
            let Some(bbox) = bounds(&line, width, height) else {
                // A line placed nowhere cannot be redacted, and a box that is not drawn is
                // worse than a line that is not reported (PRIN-09). Same rule as the
                // Windows engine's.
                continue;
            };
            blocks.push(TextBlock {
                text,
                bbox,
                // `ocrs` reports no per-line score. See `TextBlock::confidence`.
                confidence: 1.0,
            });
        }
        Ok(blocks)
    }
}

/// The engine over the two model files, or the reason it could not be built.
///
/// Every failure is one sentence naming the file, because the only person who can act on it
/// is looking at an installation with a resource missing. The models never carry anything
/// of the user's, so naming their path leaks nothing (LOG-03).
fn load(files: &[PathBuf; 2]) -> Result<OcrsEngine, String> {
    let [detection, recognition] = files;
    let detection_model = rten::Model::load_file(detection)
        .map_err(|error| format!("{} could not be read: {error}", detection.display()))?;
    let recognition_model = rten::Model::load_file(recognition)
        .map_err(|error| format!("{} could not be read: {error}", recognition.display()))?;
    OcrsEngine::new(OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })
    .map_err(|error| format!("the bundled OCR models were refused: {error}"))
}

/// The line's bounding box in the pixels of the image it was read from.
///
/// `ocrs` places characters, and a character can land a pixel or two outside the image it
/// came from; the box is clamped so that the redaction of §7.10 never draws outside the
/// capture. `None` when nothing of the line is inside, which cannot happen for a line the
/// detector found and is a defence rather than a case.
fn bounds(line: &ocrs::TextLine, width: u32, height: u32) -> Option<Rect> {
    use ocrs::TextItem as _;

    let rect = line.bounding_rect();
    let left = rect.left().clamp(0, i32::try_from(width).ok()?);
    let top = rect.top().clamp(0, i32::try_from(height).ok()?);
    let right = rect.right().clamp(left, i32::try_from(width).ok()?);
    let bottom = rect.bottom().clamp(top, i32::try_from(height).ok()?);
    let size = (
        u32::try_from(right - left).ok()?,
        u32::try_from(bottom - top).ok()?,
    );
    (size.0 > 0 && size.1 > 0).then(|| Rect::new(left, top, size.0, size.1))
}

/// Where the two `.rten` files are at run time.
///
/// `bundle.resources` maps `models/ocrs/*` onto `models/ocrs/` inside the bundle, and
/// Tauri's resource directory is the executable's own directory on Windows and
/// `../Resources` inside the `.app` on macOS (`tauri_utils::platform::resource_dir`). This
/// reproduces those two, rather than asking Tauri, because §7.9's engines are core code and
/// the core takes no `AppHandle` (`lib.rs`).
///
/// Neither shape exists in a development build: `cargo tauri dev`, `cargo test` and a plain
/// `cargo build` all leave the executable under `target/`, where Tauri copies no resources
/// — the same hole the sidecar has (T-042). So the lookup then walks up from the executable
/// until it meets a `models/ocrs` directory, which from `target/<profile>/[deps/]` is the
/// copy committed in `src-tauri/`.
fn models_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let exe_dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();
    let in_app_bundle = exe_dir.join("..").join("Resources").join(MODELS_DIR);
    if in_app_bundle.is_dir() {
        return in_app_bundle;
    }
    exe_dir
        .ancestors()
        .take(DEVELOPMENT_LOOKUP_DEPTH)
        .map(|ancestor| ancestor.join(MODELS_DIR))
        .find(|candidate| candidate.is_dir())
        // Nothing found: name the place the bundle puts them, so that `available()` reports
        // a missing resource rather than an absent path nobody recognises.
        .unwrap_or_else(|| exe_dir.join(MODELS_DIR))
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    fn blank(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]))
    }

    #[test]
    fn the_bundled_models_are_where_the_engine_looks_for_them() {
        // The one assertion that fails when the two resources stop being committed, or when
        // `models_dir` stops finding them from a test binary.
        let engine = OcrsOcr::bundled();
        assert!(
            engine.available(None),
            "the bundled models were not found at {}",
            engine.models.display()
        );
        for file in engine.model_files() {
            assert!(file.is_file(), "{} is missing", file.display());
        }
    }

    #[test]
    fn a_build_whose_models_are_missing_is_unavailable_rather_than_broken() {
        let engine = OcrsOcr::at(PathBuf::from("no-such-directory"));
        assert!(!engine.available(None));
        let error = engine
            .recognize(&blank(32, 32), None)
            .expect_err("there are no models to read");
        assert!(
            error.to_string().contains(DETECTION_MODEL),
            "the reason names the file that is missing: {error}"
        );
    }

    #[test]
    fn the_reason_a_load_failed_is_remembered_rather_than_recomputed() {
        let engine = OcrsOcr::at(PathBuf::from("no-such-directory"));
        let first = engine
            .recognize(&blank(8, 8), None)
            .unwrap_err()
            .to_string();
        let second = engine
            .recognize(&blank(8, 8), None)
            .unwrap_err()
            .to_string();
        assert_eq!(first, second);
    }

    #[test]
    fn an_image_with_no_pixels_is_refused_before_the_models_are_read() {
        // Refused by this engine rather than by `ocrs`, which panics on a zero-length
        // dimension deep inside the tensor library.
        let engine = OcrsOcr::at(PathBuf::from("no-such-directory"));
        let error = engine
            .recognize(&RgbaImage::new(0, 0), None)
            .expect_err("no pixels");
        assert!(error.to_string().contains("no pixels"), "{error}");
    }

    #[test]
    fn the_language_hint_does_not_make_the_last_engine_unavailable() {
        // FM-16 promises lower accuracy on non-English text, not the absence of OCR: the
        // English models answer for an Italian spec too, because there is nothing after
        // them to fall through to.
        let engine = OcrsOcr::bundled();
        assert!(engine.available(Some("it-IT")));
        assert!(engine.available(Some("")));
    }

    #[test]
    fn a_blank_image_produces_no_blocks() {
        let engine = OcrsOcr::bundled();
        let blocks = engine
            .recognize(&blank(320, 120), None)
            .expect("the engine ran");
        assert!(
            blocks.is_empty(),
            "text was read out of a blank image: {blocks:?}"
        );
    }

    /// The tests that need real glyphs on real pixels.
    ///
    /// They are Windows-only because the painter is: it is GDI, and it is the shape T-046
    /// settled on — a hand-drawn bitmap font would test our own glyphs instead of the
    /// engine, and a committed PNG would be a blob nobody can regenerate. A cross-platform
    /// renderer is T-048's corpus generator; until it exists the macOS leg of CI compiles
    /// this file, loads the models and runs everything above, and the accuracy of the
    /// engine is asserted here, on the platform whose missing language pack is the reason
    /// the engine exists at all.
    // TASK: T-048 — reuse the corpus generator here, and drop the `cfg`.
    #[cfg(windows)]
    mod on_real_glyphs {
        use super::*;
        use crate::ocr::paint;

        /// A fake key, in the shape the certain patterns of §4.6 match, so that the
        /// assertion is about the string the detector of T-048 will have to see. It lives
        /// inside this module and not beside `blank` because everything outside it is
        /// compiled on the macOS leg too, where an unused constant is `dead_code` and
        /// `-D warnings` turns that into a failed build.
        const FAKE_KEY: &str = "sk-live-4Kd93jfMz0";

        #[test]
        fn the_bundled_engine_reads_a_fake_key_off_a_rendered_line() {
            let image = paint::text(560, 90, 30, &[FAKE_KEY]);
            let engine = OcrsOcr::bundled();
            let blocks = engine.recognize(&image, None).expect("the engine ran");

            let read = blocks
                .iter()
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                read.to_lowercase().replace(' ', "").contains("sk-live"),
                "the rendered key was not read back; the engine returned {read:?}"
            );
        }

        #[test]
        fn every_line_is_reported_once_and_placed_over_its_own_ink() {
            let image = paint::text(560, 170, 30, &["Open the billing page", FAKE_KEY]);
            let engine = OcrsOcr::bundled();
            let blocks = engine.recognize(&image, None).expect("the engine ran");

            assert_eq!(
                blocks.len(),
                2,
                "two rendered lines, {} blocks: {blocks:?}",
                blocks.len()
            );
            for block in &blocks {
                assert!(
                    i64::from(block.bbox.x) >= 0
                        && block.bbox.right() <= i64::from(image.width())
                        && i64::from(block.bbox.y) >= 0
                        && block.bbox.bottom() <= i64::from(image.height()),
                    "a box left the image: {block:?}"
                );
                assert!(
                    block.confidence > 0.0,
                    "a redactor thresholding on the score would drop this box: {block:?}"
                );
            }
            let (first, second) = (&blocks[0], &blocks[1]);
            assert!(
                first.bbox.bottom() <= i64::from(second.bbox.y),
                "the two lines overlap vertically: {first:?} {second:?}"
            );
            // The same three questions the Windows engine's placement test asks, because
            // the boxes are what gets filled with black before anything is sent (CAP-06)
            // and a fallback whose geometry is weaker is a fallback that leaks: no box
            // leaves the image (above), **every dark pixel is inside one**, and the boxes
            // do not simply cover the picture. A box off by an offset, off by a scale
            // factor, or given the whole frame to be safe, fails one of the three.
            let covered = |x: u32, y: u32| {
                blocks.iter().any(|block| {
                    block.bbox.contains(crate::capture::Point::new(
                        i32::try_from(x).expect("a sane column"),
                        i32::try_from(y).expect("a sane row"),
                    ))
                })
            };
            let ink: Vec<(u32, u32)> = image
                .enumerate_pixels()
                .filter(|(_, _, pixel)| pixel.0[0] < 100)
                .map(|(x, y, _)| (x, y))
                .collect();
            assert!(ink.len() > 200, "the painter drew almost nothing");
            let escaped = ink.iter().filter(|(x, y)| !covered(*x, *y)).count();
            assert_eq!(escaped, 0, "{escaped} dark pixels fall outside every block");

            let area: u32 = blocks
                .iter()
                .map(|block| block.bbox.width * block.bbox.height)
                .sum();
            assert!(
                area * 2 < image.width() * image.height(),
                "the blocks cover {area} of {} pixels, which is not a redaction any more",
                image.width() * image.height()
            );
        }
    }
}
