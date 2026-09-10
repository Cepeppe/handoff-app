//! The synthetic screenshot corpus: generated here, committed, and checked back (T-048).
//!
//! Two jobs in one test binary, because they are the same code read in two directions.
//!
//! - `HANDOFF_WRITE_CORPUS=1 cargo test --test gen_corpus` **writes**
//!   `tests/fixtures/screenshots/`: one PNG per page of [`corpus::pages`] and `labels.json`
//!   beside them. That is how the corpus is regenerated after a page is added or changed.
//! - Without the variable it **checks**: every committed image is regenerated in memory and
//!   compared pixel for pixel, and the labels are compared as data. A renderer that is not
//!   deterministic, or a machine that draws differently, fails here rather than silently
//!   moving the ground truth of `tests/redaction_corpus.rs` out from under it.
//!
//! The comparison is over decoded pixels rather than over the bytes of the file: the PNG
//! encoder is free to compress the same image differently between versions of its crate,
//! and what this test is about is the drawing.
//!
//! Nothing here runs a detector. What the corpus *says* about each line is written by hand
//! in `corpus/mod.rs`; what the detectors make of it is `tests/redaction_corpus.rs`.

#[path = "corpus/mod.rs"]
mod corpus;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use handoff_app_lib::redaction::certain::scan_text as scan_certain;
use handoff_app_lib::redaction::suspected::{
    entropy_bits_per_char, is_mixed, ENTROPY_BITS_PER_CHAR, ENTROPY_MIN_CHARS,
};

/// The task's floor: the corpus is not a corpus below it.
const MIN_IMAGES: usize = 40;

/// The task's ceiling for one image.
const MAX_BYTES: u64 = 300 * 1024;

/// How far a token's entropy must stay from the threshold of §7.10.
///
/// `log2` is the one function in the detector that a platform's maths library rather than
/// IEEE-754 decides, so a corpus token sitting a hair from 3.5 bits could be flagged on one
/// runner and not on the other, and the metrics gate would depend on the machine. The
/// margin is enforced instead of hoped for.
const ENTROPY_MARGIN: f64 = 0.05;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn directory() -> PathBuf {
    root().join(corpus::DIRECTORY)
}

fn writing() -> bool {
    std::env::var("HANDOFF_WRITE_CORPUS").is_ok_and(|value| value == "1")
}

#[test]
fn the_font_can_draw_every_character_the_corpus_uses() {
    for page in corpus::pages() {
        for text in corpus::every_text(&page) {
            assert!(
                corpus::covered(&text),
                "{}: the stroke font has no glyph for something in {text:?}",
                page.name
            );
        }
    }
}

#[test]
fn no_corpus_token_sits_on_the_entropy_threshold() {
    for page in corpus::pages() {
        for text in corpus::every_text(&page) {
            for token in text.split_whitespace() {
                if token.chars().count() < ENTROPY_MIN_CHARS || !is_mixed(token) {
                    continue;
                }
                let entropy = entropy_bits_per_char(token);
                assert!(
                    (entropy - ENTROPY_BITS_PER_CHAR).abs() > ENTROPY_MARGIN,
                    "{}: {token:?} has {entropy:.4} bits per character, within {ENTROPY_MARGIN} \
                     of the threshold; the corpus must not depend on the platform's log2",
                    page.name
                );
            }
        }
    }
}

#[test]
fn every_planted_key_is_a_whole_key_on_its_own_line() {
    // A fake key one character short of its pattern is completed by the second pass of
    // §7.10 out of the line under it, which locks that line too: recall stays at 1.0 and
    // precision quietly drops. Three of the sixteen families were written short here and
    // this is what found them. The corpus's own well-formedness, not the detector's.
    for page in corpus::pages() {
        let (_, labels) = corpus::render(&page);
        for line in &labels.lines {
            let corpus::Expect::Certain { kind } = &line.expect else {
                continue;
            };
            let found = scan_certain(&line.text);
            assert_eq!(
                found.len(),
                1,
                "{}: {:?} is declared a certain {kind} and matches {} times on its own line",
                page.name,
                line.text,
                found.len()
            );
            assert_eq!(
                found[0].kind.as_str(),
                kind,
                "{}: {:?} is declared {kind} and reads as {}",
                page.name,
                line.text,
                found[0].kind
            );
        }
    }
}

#[test]
fn the_corpus_is_generated_and_matches_what_is_committed() {
    let pages = corpus::pages();
    assert!(
        pages.len() >= MIN_IMAGES,
        "the corpus is {} images, the task asks for at least {MIN_IMAGES}",
        pages.len()
    );
    let names: BTreeSet<&str> = pages.iter().map(|page| page.name).collect();
    assert_eq!(names.len(), pages.len(), "two pages share a name");

    let directory = directory();
    if writing() {
        std::fs::create_dir_all(&directory).expect("the corpus directory");
    }

    let mut labels = Vec::with_capacity(pages.len());
    for page in &pages {
        let (image, label) = corpus::render(page);
        let path = directory.join(&label.file);
        if writing() {
            image.save(&path).expect("the image is written");
        } else {
            let committed = image::open(&path)
                .unwrap_or_else(|error| {
                    panic!(
                        "{}: {error}. Regenerate with HANDOFF_WRITE_CORPUS=1",
                        path.display()
                    )
                })
                .to_rgba8();
            assert_eq!(
                (committed.width(), committed.height()),
                (image.width(), image.height()),
                "{} is a different size from what the generator draws",
                label.file
            );
            let different = committed
                .pixels()
                .zip(image.pixels())
                .filter(|(left, right)| left != right)
                .count();
            assert_eq!(
                different, 0,
                "{}: {different} pixels differ from the committed image. Either the renderer \
                 is not deterministic on this machine, or the corpus needs regenerating with \
                 HANDOFF_WRITE_CORPUS=1",
                label.file
            );
        }
        labels.push(label);
    }

    let written = corpus::Labels {
        about: corpus::ABOUT.to_owned(),
        images: labels,
    };
    let json = format!(
        "{}\n",
        serde_json::to_string_pretty(&written).expect("the labels serialise")
    );
    let labels_path = directory.join(corpus::LABELS);
    if writing() {
        std::fs::write(&labels_path, json.as_bytes()).expect("the labels are written");
    } else {
        let committed = std::fs::read_to_string(&labels_path).unwrap_or_else(|error| {
            panic!(
                "{}: {error}. Regenerate with HANDOFF_WRITE_CORPUS=1",
                labels_path.display()
            )
        });
        assert_eq!(
            committed.replace("\r\n", "\n"),
            json,
            "labels.json is not what the corpus declares"
        );
    }
}

#[test]
fn every_committed_image_is_small_enough_to_live_in_a_repository() {
    if writing() {
        return;
    }
    for page in corpus::pages() {
        let path = directory().join(format!("{}.png", page.name));
        let size = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
            .len();
        assert!(
            size <= MAX_BYTES,
            "{} is {size} bytes, over the {MAX_BYTES} the task allows",
            page.name
        );
    }
}
