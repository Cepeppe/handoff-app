//! OCR (§7.9, OCR-01..05, DD-30).
//!
//! One `OcrEngine` trait with three implementations — Vision on macOS, `Windows.Media.Ocr`
//! on Windows, bundled Tesseract as the fallback — and the selection rule that picks one
//! and falls back when it is unavailable.
// TASK: T-047
