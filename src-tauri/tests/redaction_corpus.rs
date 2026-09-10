//! The corpus gates of §11.7: detector metrics, redaction geometry, glyph leak (T-048).
//!
//! Four questions, asked of the committed corpus of `tests/fixtures/screenshots/`:
//!
//! 1. **Do the detectors say what the corpus says?** `metrics_over_the_corpus` runs the
//!    real [`detect`] over the ground-truth lines of `labels.json` and reports precision and
//!    recall per level. The thresholds of §11.7 are the gate; the suspected false-positive
//!    rate is printed, because R-07 is a number to watch and not a line to cross.
//! 2. **Does the burn cover the glyphs?** `the_burn_covers_every_ink_pixel_of_a_redacted_line`
//!    maps every dark pixel of a redacted line onto the reduced image and requires it to be
//!    inside a filled rectangle. This one is arithmetic: it says the same thing on every
//!    machine and it is the assertion R-06 is really about.
//! 3. **Would we notice the defect CAP-06 names?** `burning_before_the_resize_is_caught`
//!    performs the wrong order deliberately and requires question 2 to fail on it. A gate
//!    nobody has watched fail is not a gate.
//! 4. **Can an engine still read the secret?** `ocr_of_a_redacted_capture_reads_no_secret`
//!    is the glyph-leak test the task asks for, and it carries its own positive control:
//!    for each planted key it first OCRs the **unredacted** band and records whether the
//!    certain detector finds the key there. Without that, an engine that reads nothing at
//!    all would pass the test by failing at its job.
//!
//! The metrics are printed, so CI runs this binary once with `--nocapture`
//! (`cargo test --test redaction_corpus metrics -- --nocapture`).

#[path = "corpus/mod.rs"]
mod corpus;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use handoff_app_lib::capture::Rect;
use handoff_app_lib::ocr::TextBlock;
use handoff_app_lib::redaction::boxes::{detect, BoxCause, BoxLevel, Detection, RedactionPlan};
use handoff_app_lib::redaction::burn::{
    burn, fill_black, reduced_size, rescale_box, scale_for, BOX_EXPAND_PX,
};
use handoff_app_lib::redaction::certain::scan_text as scan_certain;
use handoff_app_lib::redaction::suspected::Exemptions;
use image::RgbaImage;

use corpus::{Expect, ImageLabels, Labels};

/// §11.7: the certain detector must find every planted key.
const CERTAIN_RECALL: f64 = 1.0;

/// §11.7: and must almost never fire on a line that is not one.
const CERTAIN_PRECISION: f64 = 0.999;

/// §11.7: the suspected detector may miss one in ten.
const SUSPECTED_RECALL: f64 = 0.9;

/// How much of a planted key an engine has to read for the glyph-leak test to mean anything.
///
/// Measured on this machine rather than chosen: the value in the Completion note of T-048
/// is what the two engines actually managed. A floor below what was measured would let the
/// test go quiet the day OCR stops working; a floor at the measurement would make it a
/// flake on a slower or different recogniser.
const OCR_CONTROL_FLOOR: f64 = 0.20;

/// The name [`handoff_app_lib::ocr::engines`] gives the bundled engine.
const BUNDLED_ENGINE: &str = "ocrs";

/// How many planted keys are read when the **bundled** engine is the one answering.
///
/// It is the engine of a machine with no OCR language pack, and today also of the macOS leg
/// (`VisionOcr` is T-059's stub). Reading all twenty-three keys with it takes five minutes
/// in a debug build, which on a runner billed at ten times the minute is a cost out of all
/// proportion to what the extra images say: the assertion is the same one over and over,
/// and the geometry tests above cover every image on every machine. Where the operating
/// system's own engine answers — Windows, and macOS once T-059 lands — every key is read.
const BUNDLED_ENGINE_KEYS: usize = 6;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn labels() -> Labels {
    let path = root().join(corpus::DIRECTORY).join(corpus::LABELS);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).expect("labels.json parses")
}

fn image_of(entry: &ImageLabels) -> RgbaImage {
    let path = root().join(corpus::DIRECTORY).join(&entry.file);
    image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgba8()
}

/// The ground-truth lines of one image, as an OCR engine would have reported them.
fn blocks_of(entry: &ImageLabels) -> Vec<TextBlock> {
    entry
        .lines
        .iter()
        .map(|line| TextBlock {
            text: line.text.clone(),
            bbox: Rect::new(line.x, line.y, line.width, line.height),
            confidence: 1.0,
        })
        .collect()
}

fn exemptions_of(entry: &ImageLabels) -> Exemptions {
    Exemptions::of_values(&entry.exempt)
}

/// What one level scored over the corpus.
#[derive(Default)]
struct Score {
    declared: usize,
    found: usize,
    fired: usize,
    wrong_kind: Vec<String>,
    missed: Vec<String>,
    spurious: Vec<String>,
}

impl Score {
    fn recall(&self) -> f64 {
        if self.declared == 0 {
            return 1.0;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            self.found as f64 / self.declared as f64
        }
    }

    fn precision(&self) -> f64 {
        if self.fired == 0 {
            return 1.0;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            (self.fired - self.spurious.len()) as f64 / self.fired as f64
        }
    }
}

#[test]
fn metrics_over_the_corpus() {
    let labels = labels();
    let mut certain = Score::default();
    let mut suspected = Score::default();
    let mut clean_lines = 0_usize;
    let mut clean_flagged = 0_usize;

    for entry in &labels.images {
        let blocks = blocks_of(entry);
        let detection = detect(&blocks, &exemptions_of(entry));
        for (index, line) in entry.lines.iter().enumerate() {
            let drawn = detection
                .boxes
                .iter()
                .find(|drawn| drawn.block == index)
                .cloned();
            let where_ = format!("{} line {index}", entry.file);
            match &line.expect {
                Expect::Certain { kind } => {
                    certain.declared += 1;
                    match drawn.as_ref().map(|drawn| (drawn.level, drawn.cause)) {
                        Some((BoxLevel::Locked, BoxCause::Certain(found))) => {
                            certain.found += 1;
                            if found.as_str() != kind {
                                certain
                                    .wrong_kind
                                    .push(format!("{where_}: {kind} declared, {found} found"));
                            }
                        }
                        _ => certain.missed.push(format!("{where_}: {:?}", line.text)),
                    }
                }
                Expect::Suspected { reason } => {
                    suspected.declared += 1;
                    match drawn.as_ref().map(|drawn| (drawn.level, drawn.cause)) {
                        Some((BoxLevel::Flagged, BoxCause::Suspected(found))) => {
                            suspected.found += 1;
                            if found.as_str() != reason {
                                suspected
                                    .wrong_kind
                                    .push(format!("{where_}: {reason} declared, {found} found"));
                            }
                        }
                        _ => suspected.missed.push(format!("{where_}: {:?}", line.text)),
                    }
                }
                Expect::Clean => {
                    clean_lines += 1;
                    match drawn.as_ref().map(|drawn| drawn.level) {
                        Some(BoxLevel::Locked) => {
                            certain.spurious.push(format!("{where_}: {:?}", line.text))
                        }
                        Some(BoxLevel::Flagged) => {
                            clean_flagged += 1;
                            suspected
                                .spurious
                                .push(format!("{where_}: {:?}", line.text));
                        }
                        None => {}
                    }
                }
            }
            if let Some(drawn) = drawn {
                match drawn.level {
                    BoxLevel::Locked => certain.fired += 1,
                    BoxLevel::Flagged => suspected.fired += 1,
                }
            }
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let false_positive_rate = if clean_lines == 0 {
        0.0
    } else {
        clean_flagged as f64 / clean_lines as f64
    };

    println!("--- redaction corpus metrics (§11.7) ---");
    println!("images                     {}", labels.images.len());
    println!(
        "lines                      {}",
        labels.images.iter().map(|it| it.lines.len()).sum::<usize>()
    );
    println!(
        "certain    declared {:>3}  found {:>3}  recall {:.4}  precision {:.4}",
        certain.declared,
        certain.found,
        certain.recall(),
        certain.precision()
    );
    println!(
        "suspected  declared {:>3}  found {:>3}  recall {:.4}  precision {:.4}",
        suspected.declared,
        suspected.found,
        suspected.recall(),
        suspected.precision()
    );
    println!(
        "suspected false positives  {clean_flagged}/{clean_lines} clean lines  \
         rate {false_positive_rate:.4}"
    );
    for line in suspected.spurious.iter().take(12) {
        println!("  flagged clean line: {line}");
    }
    println!("--- end of metrics ---");

    assert!(
        certain.missed.is_empty(),
        "the certain detector missed a planted key: {:#?}",
        certain.missed
    );
    assert!(
        certain.wrong_kind.is_empty(),
        "the certain detector named the wrong family: {:#?}",
        certain.wrong_kind
    );
    assert!(
        certain.spurious.is_empty(),
        "the certain detector fired on a clean line: {:#?}",
        certain.spurious
    );
    assert!(
        certain.recall() >= CERTAIN_RECALL,
        "certain recall {:.4} is under {CERTAIN_RECALL}",
        certain.recall()
    );
    assert!(
        certain.precision() >= CERTAIN_PRECISION,
        "certain precision {:.4} is under {CERTAIN_PRECISION}",
        certain.precision()
    );
    assert!(
        suspected.recall() >= SUSPECTED_RECALL,
        "suspected recall {:.4} is under {SUSPECTED_RECALL}; missed {:#?}",
        suspected.recall(),
        suspected.missed
    );
    assert!(
        suspected.wrong_kind.is_empty(),
        "the suspected detector named the wrong rule: {:#?}",
        suspected.wrong_kind
    );
}

#[test]
fn the_exemption_of_det_03_is_what_keeps_a_declared_value_visible() {
    let labels = labels();
    let mut checked = 0_usize;
    for entry in labels.images.iter().filter(|it| !it.exempt.is_empty()) {
        let blocks = blocks_of(entry);
        let without = detect(&blocks, &Exemptions::none());
        let with = detect(&blocks, &exemptions_of(entry));
        // A locked box is never lifted by an exemption (DET-03's second half).
        assert_eq!(
            without.locked(),
            with.locked(),
            "{}: the exemption changed a certain box",
            entry.file
        );
        assert!(
            with.flagged() <= without.flagged(),
            "{}: the exemption added a flagged box",
            entry.file
        );
        checked += 1;
    }
    assert!(checked >= 5, "only {checked} pages exercise the exemption");
}

/// Whether a pixel is dark enough to be a glyph on a light page, or light enough on a dark
/// one. The corpus draws ink at either end of the range and backgrounds in between.
fn is_ink(pixel: &image::Rgba<u8>, dark_page: bool) -> bool {
    let luma =
        u32::from(pixel.0[0]) * 299 + u32::from(pixel.0[1]) * 587 + u32::from(pixel.0[2]) * 114;
    if dark_page {
        luma > 160_000
    } else {
        luma < 110_000
    }
}

/// The reduced image with nothing burned into it, which is the control for both geometry
/// and OCR.
fn reduced_plain(image: &RgbaImage) -> RgbaImage {
    let (width, height) = reduced_size(image.width(), image.height());
    if (width, height) == (image.width(), image.height()) {
        return image.clone();
    }
    image::imageops::resize(image, width, height, image::imageops::FilterType::Triangle)
}

/// Every redacted line of one image, as the corpus declares it.
fn redacted_lines(entry: &ImageLabels, detection: &Detection) -> Vec<Rect> {
    detection
        .boxes
        .iter()
        .map(|drawn| {
            let line = &entry.lines[drawn.block];
            Rect::new(line.x, line.y, line.width, line.height)
        })
        .collect()
}

#[test]
fn the_burn_covers_every_ink_pixel_of_a_redacted_line() {
    let labels = labels();
    let mut covered = 0_usize;
    for entry in &labels.images {
        let image = image_of(entry);
        let blocks = blocks_of(entry);
        let detection = detect(&blocks, &exemptions_of(entry));
        if detection.boxes.is_empty() {
            continue;
        }
        let plan = RedactionPlan::new(exemptions_of(entry));
        let burned = burn(&image, &detection, &plan).expect("the capture burns");
        let reduced = image::load_from_memory(&burned.png)
            .expect("the burned PNG decodes")
            .to_rgba8();
        let scale = scale_for(image.width(), image.height());
        let dark_page = is_ink(image.get_pixel(0, 0), true);

        for line in redacted_lines(entry, &detection) {
            let expected = rescale_box(line, (0, 0), scale, (reduced.width(), reduced.height()))
                .expect("a line of the corpus is inside its own image");
            // Everything the burn promised is black, including the two pixels of margin
            // CAP-06 asks for: that margin is what covers the halo the resize leaves.
            for y in expected.y..i32::try_from(expected.bottom()).expect("in range") {
                for x in expected.x..i32::try_from(expected.right()).expect("in range") {
                    #[allow(clippy::cast_sign_loss)]
                    let pixel = reduced.get_pixel(x as u32, y as u32);
                    assert_eq!(
                        pixel.0,
                        [0, 0, 0, 0xff],
                        "{}: ({x}, {y}) is not black inside the box of {line:?}",
                        entry.file
                    );
                }
            }
            // And no ink of that line survived anywhere in the reduced image.
            let mut leaked = 0_usize;
            for y in line.y..i32::try_from(line.bottom()).expect("in range") {
                for x in line.x..i32::try_from(line.right()).expect("in range") {
                    #[allow(clippy::cast_sign_loss)]
                    if x < 0 || y < 0 || x as u32 >= image.width() || y as u32 >= image.height() {
                        continue;
                    }
                    #[allow(clippy::cast_sign_loss)]
                    if !is_ink(image.get_pixel(x as u32, y as u32), dark_page) {
                        continue;
                    }
                    let (rx, ry) = (
                        (f64::from(x) * scale).floor(),
                        (f64::from(y) * scale).floor(),
                    );
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let (rx, ry) = (rx as u32, ry as u32);
                    if rx >= reduced.width() || ry >= reduced.height() {
                        continue;
                    }
                    if reduced.get_pixel(rx, ry).0 != [0, 0, 0, 0xff] {
                        leaked += 1;
                    }
                }
            }
            assert_eq!(
                leaked, 0,
                "{}: {leaked} ink pixels of {line:?} are still visible after the burn",
                entry.file
            );
            covered += 1;
        }
    }
    assert!(covered >= 20, "only {covered} lines were checked");
}

#[test]
fn burning_before_the_resize_is_caught() {
    // The defect CAP-06 names, performed on purpose: fill the boxes on the original and
    // then resize. The black region ends up a scaled-down box with a blended edge instead
    // of a box with two pixels of margin, so the check above has something to find.
    let labels = labels();
    let entry = labels
        .images
        .iter()
        .find(|it| it.file == "certain-20-large-console.png")
        .expect("the page that is over 1600 pixels wide");
    let image = image_of(entry);
    let blocks = blocks_of(entry);
    let detection = detect(&blocks, &exemptions_of(entry));
    assert!(!detection.boxes.is_empty());

    let mut wrong = image.clone();
    let plan = RedactionPlan::new(exemptions_of(entry));
    let expanded: Vec<Rect> = plan
        .boxes(&detection)
        .into_iter()
        .map(|rect| {
            Rect::new(
                rect.x - i32::try_from(BOX_EXPAND_PX).expect("small"),
                rect.y - i32::try_from(BOX_EXPAND_PX).expect("small"),
                rect.width + 2 * BOX_EXPAND_PX,
                rect.height + 2 * BOX_EXPAND_PX,
            )
        })
        .collect();
    fill_black(&mut wrong, &expanded);
    let wrong = reduced_plain(&wrong);

    let scale = scale_for(image.width(), image.height());
    let mut off_colour = 0_usize;
    for line in redacted_lines(entry, &detection) {
        let expected = rescale_box(line, (0, 0), scale, (wrong.width(), wrong.height()))
            .expect("inside the image");
        for y in expected.y..i32::try_from(expected.bottom()).expect("in range") {
            for x in expected.x..i32::try_from(expected.right()).expect("in range") {
                #[allow(clippy::cast_sign_loss)]
                if wrong.get_pixel(x as u32, y as u32).0 != [0, 0, 0, 0xff] {
                    off_colour += 1;
                }
            }
        }
    }
    assert!(
        off_colour > 0,
        "burning before the resize produced the same image as burning after it, so the \
         geometry check above proves nothing"
    );
    println!("burning before the resize leaves {off_colour} pixels the check would catch");
}

/// The band of the reduced image a line occupies, cropped out for OCR.
fn band(image: &RgbaImage, rect: Rect) -> RgbaImage {
    let margin = 12_i64;
    let left = (i64::from(rect.x) - margin).clamp(0, i64::from(image.width()));
    let top = (i64::from(rect.y) - margin).clamp(0, i64::from(image.height()));
    let right = (rect.right() + margin).clamp(0, i64::from(image.width()));
    let bottom = (rect.bottom() + margin).clamp(0, i64::from(image.height()));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    image::imageops::crop_imm(
        image,
        left as u32,
        top as u32,
        (right - left).max(1) as u32,
        (bottom - top).max(1) as u32,
    )
    .to_image()
}

/// The text an engine of this machine reads out of one band.
fn read(runtime: &tokio::runtime::Runtime, image: &RgbaImage) -> (String, String) {
    let recognised = runtime
        .block_on(handoff_app_lib::ocr::select_and_run(
            Arc::new(image.clone()),
            None,
        ))
        .expect("an OCR engine answered");
    let text = recognised
        .blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (text, recognised.engine)
}

#[test]
fn ocr_of_a_redacted_capture_reads_no_secret() {
    let labels = labels();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let started = Instant::now();
    let mut controls = 0_usize;
    let mut read_back = 0_usize;
    let mut limit = usize::MAX;
    let mut engine = String::new();

    for entry in &labels.images {
        let planted: Vec<usize> = entry
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| matches!(line.expect, Expect::Certain { .. }))
            .map(|(index, _)| index)
            .collect();
        if planted.is_empty() {
            continue;
        }
        let image = image_of(entry);
        let blocks = blocks_of(entry);
        let detection = detect(&blocks, &exemptions_of(entry));
        let plan = RedactionPlan::new(exemptions_of(entry));
        let burned = burn(&image, &detection, &plan).expect("the capture burns");
        let redacted = image::load_from_memory(&burned.png)
            .expect("the burned PNG decodes")
            .to_rgba8();
        let plain = reduced_plain(&image);
        let scale = scale_for(image.width(), image.height());

        for index in planted {
            let line = &entry.lines[index];
            let rect = Rect::new(line.x, line.y, line.width, line.height);
            let Some(reduced_rect) =
                rescale_box(rect, (0, 0), scale, (redacted.width(), redacted.height()))
            else {
                continue;
            };
            if controls >= limit {
                break;
            }
            controls += 1;
            let (control, name) = read(&runtime, &band(&plain, reduced_rect));
            if name == BUNDLED_ENGINE {
                limit = BUNDLED_ENGINE_KEYS;
            }
            engine = name;
            if scan_certain(&control).is_empty() {
                println!("MISS {} line {index}: {control:?}", entry.file);
            } else {
                read_back += 1;
            }
            let (after, _) = read(&runtime, &band(&redacted, reduced_rect));
            let leaked = scan_certain(&after);
            assert!(
                leaked.is_empty(),
                "{} line {index}: the redacted band still reads as a certain secret ({:?})",
                entry.file,
                after
            );
        }
    }

    #[allow(clippy::cast_precision_loss)]
    let rate = read_back as f64 / controls as f64;
    println!(
        "--- glyph leak: {engine} read {read_back} of {controls} planted keys before \
         redaction ({rate:.2}) and none after, in {:?} ---",
        started.elapsed()
    );
    assert!(
        rate >= OCR_CONTROL_FLOOR,
        "only {rate:.2} of the planted keys were readable before redaction, so the assertion \
         that none is readable after it proves nothing"
    );
}
