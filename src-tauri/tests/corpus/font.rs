//! A stroke font, drawn in Rust, so that the corpus is the same image on every machine.
//!
//! `ocr::paint` renders with GDI and is the right renderer for the OCR tests: it draws with
//! a font a real recogniser was trained on. It is also Windows-only and it depends on which
//! version of Segoe UI the machine has, so it can be neither committed against nor run on
//! the macOS leg. The corpus of T-048 needs the opposite properties — the same pixels
//! everywhere, no system font, no third-party asset — because the images are committed and
//! a test regenerates them and compares.
//!
//! So the glyphs are strokes, written down here. Each one is a set of polylines in a grid
//! where the cap height is [`CAP`] units, the baseline is at `y = CAP`, the x-height top at
//! `y = X_HEIGHT` and descenders reach `y = DESCENDER`. They are stamped with a round pen,
//! which is what gives an antialiased edge with nothing but `sqrt` — no transcendental, so
//! the arithmetic is bit-identical on x86-64 and on aarch64.
//!
//! It is a plain sans-serif and it is meant to be read: the corpus exists to be OCRed, and
//! `tests/redaction_corpus.rs` prints how much of it the engine of the machine actually
//! read rather than assuming.

use image::{Rgba, RgbaImage};

/// The cap height, in glyph units.
pub const CAP: f64 = 100.0;

/// The top of a lowercase letter without an ascender.
pub const X_HEIGHT: f64 = 42.0;

/// How far below the baseline a descender reaches.
pub const DESCENDER: f64 = 130.0;

/// The gap between two glyphs, in glyph units.
const LETTER_SPACING: f64 = 14.0;

/// The width of a space, in glyph units.
const SPACE_WIDTH: f64 = 30.0;

/// The pen's radius as a fraction of the cap height.
const PEN: f64 = 0.045;

/// One glyph: its advance in glyph units, and its polylines.
struct Glyph {
    advance: f64,
    strokes: &'static str,
}

/// The polylines of one glyph, parsed from its compact spelling.
///
/// `"0,100 30,0 60,100;11,68 49,68"` is two polylines: the two diagonals of an `A` and its
/// crossbar. Points are `x,y` in glyph units, points are separated by spaces and polylines
/// by semicolons.
fn strokes_of(spelling: &str) -> Vec<Vec<(f64, f64)>> {
    spelling
        .split(';')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            part.split_whitespace()
                .map(|point| {
                    let (x, y) = point.split_once(',').expect("a point is `x,y`");
                    (
                        x.parse::<f64>().expect("an x"),
                        y.parse::<f64>().expect("a y"),
                    )
                })
                .collect()
        })
        .collect()
}

/// The glyph for a character, or `None` when the font has none.
#[allow(clippy::too_many_lines)]
fn glyph(character: char) -> Option<Glyph> {
    let (advance, strokes) = match character {
        'A' => (60.0, "0,100 30,0 60,100;11,68 49,68"),
        'B' => (58.0, "0,0 0,100;0,0 40,0 55,15 55,33 40,48 0,48;0,48 44,48 58,64 58,84 44,100 0,100"),
        'C' => (60.0, "58,20 44,5 26,2 10,14 2,38 2,62 10,86 26,98 44,95 58,80"),
        'D' => (60.0, "0,0 0,100;0,0 34,0 54,18 60,50 54,82 34,100 0,100"),
        'E' => (56.0, "56,0 0,0 0,100 56,100;0,48 44,48"),
        'F' => (56.0, "56,0 0,0 0,100;0,48 44,48"),
        'G' => (62.0, "58,20 44,5 26,2 10,14 2,38 2,62 10,86 26,98 46,94 58,80 58,55 34,55"),
        'H' => (60.0, "0,0 0,100;60,0 60,100;0,50 60,50"),
        // Serifs, because a bare bar is the same shape as `l` and every engine tried
        // here read `AKIA…` as `AKlA…` until they were added.
        'I' => (32.0, "5,0 27,0;16,0 16,100;5,100 27,100"),
        'J' => (48.0, "46,0 46,76 38,95 20,100 6,92 2,78"),
        'K' => (58.0, "0,0 0,100;56,0 6,56;20,42 58,100"),
        'L' => (52.0, "0,0 0,100 52,100"),
        'M' => (72.0, "0,100 0,0 36,58 72,0 72,100"),
        'N' => (62.0, "0,100 0,0 62,100 62,0"),
        'O' => (62.0, "31,0 13,10 3,34 3,66 13,90 31,100 49,90 59,66 59,34 49,10 31,0"),
        'P' => (56.0, "0,100 0,0 40,0 56,14 56,36 40,52 0,52"),
        'Q' => (62.0, "31,0 13,10 3,34 3,66 13,90 31,100 49,90 59,66 59,34 49,10 31,0;38,74 62,110"),
        'R' => (58.0, "0,100 0,0 40,0 56,14 56,34 40,50 0,50;30,50 58,100"),
        'S' => (56.0, "54,18 42,4 22,2 8,14 8,32 20,44 42,54 54,66 54,84 40,97 20,98 4,84"),
        'T' => (56.0, "0,0 56,0;28,0 28,100"),
        'U' => (60.0, "0,0 0,72 10,94 30,100 50,94 60,72 60,0"),
        'V' => (60.0, "0,0 30,100 60,0"),
        'W' => (86.0, "0,0 18,100 43,26 68,100 86,0"),
        'X' => (58.0, "0,0 58,100;58,0 0,100"),
        'Y' => (58.0, "0,0 29,54 58,0;29,54 29,100"),
        'Z' => (56.0, "0,0 56,0 0,100 56,100"),

        'a' => (50.0, "50,58 36,44 18,48 8,62 8,84 18,98 36,100 50,86;50,44 50,100"),
        'b' => (50.0, "0,0 0,100;0,72 8,52 26,44 44,54 50,72 44,90 26,100 8,92 0,72"),
        'c' => (48.0, "48,58 34,44 16,48 6,62 6,82 16,96 34,100 48,86"),
        'd' => (50.0, "50,0 50,100;50,72 42,52 24,44 6,54 0,72 6,90 24,100 42,92 50,72"),
        'e' => (48.0, "4,74 48,74 48,62 38,47 20,45 8,56 4,74 10,92 26,100 44,96"),
        'f' => (34.0, "34,0 22,2 16,14 16,100;2,44 34,44"),
        'g' => (50.0, "50,44 50,116 40,128 20,130 6,120;50,72 42,52 24,44 6,54 0,72 6,90 24,100 42,92 50,72"),
        'h' => (48.0, "0,0 0,100;0,64 14,48 34,44 48,56 48,100"),
        'i' => (16.0, "8,100 8,44;8,18 8,26"),
        'j' => (20.0, "12,44 12,116 6,128 0,128;12,18 12,26"),
        'k' => (46.0, "0,0 0,100;44,44 8,76;18,66 46,100"),
        'l' => (16.0, "8,0 8,100"),
        'm' => (80.0, "0,100 0,44;0,62 12,46 28,44 38,54 38,100;38,62 52,46 68,44 80,54 80,100"),
        'n' => (48.0, "0,100 0,44;0,62 14,46 34,44 48,56 48,100"),
        'o' => (50.0, "25,44 9,52 3,70 9,90 25,100 41,92 47,72 41,52 25,44"),
        'p' => (50.0, "0,44 0,130;0,72 8,52 26,44 44,54 50,72 44,90 26,100 8,92 0,72"),
        'q' => (50.0, "50,44 50,130;50,72 42,52 24,44 6,54 0,72 6,90 24,100 42,92 50,72"),
        'r' => (36.0, "0,100 0,44;0,64 12,48 30,44 36,46"),
        's' => (44.0, "44,54 30,44 14,46 6,56 10,68 26,74 40,80 44,90 34,99 16,100 4,90"),
        't' => (36.0, "12,16 12,84 18,98 30,100 36,94;0,44 32,44"),
        'u' => (48.0, "0,44 0,84 10,98 28,100 44,90 48,72;48,44 48,100"),
        'v' => (48.0, "0,44 24,100 48,44"),
        'w' => (70.0, "0,44 14,100 35,54 56,100 70,44"),
        'x' => (46.0, "0,44 46,100;46,44 0,100"),
        'y' => (48.0, "0,44 26,100;48,44 18,130"),
        'z' => (44.0, "0,44 44,44 0,100 44,100"),

        // Narrower than `O`, which is the only thing that separates them in a sans font
        // without reaching for a slashed zero.
        '0' => (46.0, "23,0 10,10 3,32 3,68 10,90 23,100 36,90 43,68 43,32 36,10 23,0"),
        '1' => (40.0, "6,20 26,2 26,100;8,100 44,100"),
        '2' => (56.0, "4,22 18,4 38,4 52,18 52,38 4,100 54,100"),
        '3' => (56.0, "4,16 20,2 42,4 52,18 46,38 26,48;26,48 48,54 56,72 48,92 28,100 8,94 0,80"),
        '4' => (58.0, "42,100 42,0 2,72 58,72"),
        '5' => (56.0, "52,4 12,4 8,44 26,38 44,42 56,58 54,82 38,98 16,98 4,86"),
        '6' => (56.0, "50,10 34,2 16,10 6,34 4,66 12,90 30,100 46,94 54,78 50,60 34,52 16,56 6,70"),
        '7' => (54.0, "0,4 54,4 22,100"),
        '8' => (56.0, "28,48 12,40 6,24 16,8 34,6 48,16 48,34 32,46 14,54 4,72 12,92 30,100 48,94 54,76 44,56 28,48"),
        '9' => (56.0, "8,90 24,98 40,90 52,66 54,34 46,10 28,0 12,6 4,22 8,40 24,48 42,44 52,30"),

        ' ' => (SPACE_WIDTH, ""),
        '.' => (16.0, "6,94 6,100"),
        ',' => (16.0, "8,92 8,100 0,116"),
        ':' => (16.0, "6,54 6,60;6,94 6,100"),
        ';' => (16.0, "8,54 8,60;8,92 8,100 0,116"),
        '-' => (40.0, "2,70 38,70"),
        // Two strokes, so the underscore survives the resize as a mark rather than as a
        // faint line an engine reports as a space.
        '_' => (52.0, "0,112 52,112;0,115 52,115"),
        '/' => (44.0, "0,110 44,-6"),
        '\\' => (44.0, "0,-6 44,110"),
        '+' => (48.0, "4,70 44,70;24,50 24,90"),
        '=' => (48.0, "4,58 44,58;4,82 44,82"),
        '*' => (40.0, "20,4 20,44;3,14 37,34;37,14 3,34"),
        '(' => (30.0, "26,-8 10,24 6,50 10,80 26,110"),
        ')' => (30.0, "4,-8 20,24 24,50 20,80 4,110"),
        '[' => (30.0, "26,-8 8,-8 8,110 26,110"),
        ']' => (30.0, "4,-8 22,-8 22,110 4,110"),
        '{' => (30.0, "26,-8 14,4 14,44 4,51 14,58 14,98 26,110"),
        '}' => (30.0, "4,-8 16,4 16,44 26,51 16,58 16,98 4,110"),
        '!' => (16.0, "8,0 8,68;8,94 8,100"),
        '?' => (48.0, "2,20 14,4 34,4 46,18 44,38 24,52 24,68;24,94 24,100"),
        '\'' => (14.0, "6,0 6,26"),
        '"' => (28.0, "6,0 6,26;22,0 22,26"),
        '@' => (76.0, "50,60 42,52 32,54 28,66 32,78 42,80 50,72 50,50;50,72 56,82 66,74 68,50 58,30 36,24 16,32 6,52 6,78 18,98 40,104 58,98"),
        '#' => (52.0, "16,30 6,100;40,30 30,100;2,52 46,52;0,78 44,78"),
        '%' => (64.0, "56,4 8,100;6,10 6,26 18,32 28,24 28,8 18,2 6,10;38,76 38,92 50,98 60,90 60,74 50,68 38,76"),
        '&' => (62.0, "62,100 18,44 10,28 16,10 32,6 44,16 42,32 8,60 4,78 14,96 34,100 52,86"),
        '|' => (16.0, "8,-8 8,110"),
        '<' => (44.0, "40,40 6,70 40,100"),
        '>' => (44.0, "4,40 38,70 4,100"),
        '~' => (48.0, "2,74 14,64 26,74 38,64"),
        _ => return None,
    };
    Some(Glyph { advance, strokes })
}

/// Whether every character of `text` has a glyph.
pub fn covers(text: &str) -> bool {
    text.chars().all(|character| glyph(character).is_some())
}

/// The width of `text`, in pixels, at the given cap height.
pub fn width_of(text: &str, size: f64) -> f64 {
    let scale = size / CAP;
    let mut advance = 0.0;
    for character in text.chars() {
        let Some(item) = glyph(character) else {
            continue;
        };
        advance += item.advance + LETTER_SPACING;
    }
    (advance - LETTER_SPACING).max(0.0) * scale
}

/// Draws `text` with its **baseline** at `(x, y)` in pixels, and answers the width drawn.
///
/// The pen is a disc, so a stroke is antialiased by the distance from the pixel's centre to
/// the polyline: `coverage = clamp(radius + 0.5 - distance, 0, 1)`. Nothing here calls a
/// transcendental function, so the same source produces the same bytes on both platforms of
/// §1.5.
pub fn draw(image: &mut RgbaImage, text: &str, x: f64, y: f64, size: f64, ink: Rgba<u8>) -> f64 {
    let scale = size / CAP;
    let radius = size * PEN;
    let mut pen_x = x;
    for character in text.chars() {
        let Some(item) = glyph(character) else {
            continue;
        };
        for stroke in strokes_of(item.strokes) {
            let points: Vec<(f64, f64)> = stroke
                .iter()
                .map(|(gx, gy)| (pen_x + gx * scale, y + (gy - CAP) * scale))
                .collect();
            stamp(image, &points, radius, ink);
        }
        pen_x += (item.advance + LETTER_SPACING) * scale;
    }
    (pen_x - x - LETTER_SPACING * scale).max(0.0)
}

/// Paints one polyline with a round pen.
fn stamp(image: &mut RgbaImage, points: &[(f64, f64)], radius: f64, ink: Rgba<u8>) {
    if points.is_empty() {
        return;
    }
    let reach = radius + 1.0;
    let (mut left, mut top, mut right, mut bottom) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (px, py) in points {
        left = left.min(px - reach);
        top = top.min(py - reach);
        right = right.max(px + reach);
        bottom = bottom.max(py + reach);
    }
    let (width, height) = (f64::from(image.width()), f64::from(image.height()));
    let first_x = left.max(0.0).floor();
    let first_y = top.max(0.0).floor();
    let last_x = right.min(width - 1.0).ceil();
    let last_y = bottom.min(height - 1.0).ceil();
    if last_x < first_x || last_y < first_y {
        return;
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    for row in (first_y as u32)..=(last_y as u32) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        for column in (first_x as u32)..=(last_x as u32) {
            if row >= image.height() || column >= image.width() {
                continue;
            }
            let centre = (f64::from(column) + 0.5, f64::from(row) + 0.5);
            let distance = if points.len() == 1 {
                length(centre, points[0])
            } else {
                points
                    .windows(2)
                    .map(|pair| distance_to_segment(centre, pair[0], pair[1]))
                    .fold(f64::MAX, f64::min)
            };
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let pixel = image.get_pixel_mut(column, row);
            for channel in 0..3 {
                let under = f64::from(pixel.0[channel]);
                let over = f64::from(ink.0[channel]);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    pixel.0[channel] = (under + (over - under) * coverage).round() as u8;
                }
            }
            pixel.0[3] = 0xff;
        }
    }
}

/// The distance between two points.
fn length(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    (dx * dx + dy * dy).sqrt()
}

/// The distance from a point to a segment.
fn distance_to_segment(point: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let squared = vx * vx + vy * vy;
    if squared <= 0.0 {
        return length(point, a);
    }
    let t = (((point.0 - a.0) * vx + (point.1 - a.1) * vy) / squared).clamp(0.0, 1.0);
    length(point, (a.0 + t * vx, a.1 + t * vy))
}
