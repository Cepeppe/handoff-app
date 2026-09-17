//! The macOS OCR engine: `VNRecognizeTextRequest`, accurate level (§7.9, A-13).
//!
//! It is a stub. macOS is deferred (implementation decision 7) and the platform work — the
//! Vision request, its `recognitionLanguages` from the spec's `lang` (OCR-05), the
//! normalised bounding boxes mapped back to image pixels — is T-059. What is here is the
//! shape, so that the selection rule of §7.9 has the same list on both platforms and the
//! macOS CI leg compiles the branch it will one day fill (implementation decision 7).
//!
//! [`VisionOcr::available`] answers `false`, which is the honest answer for a build that
//! cannot recognise anything: [`super::run_with`] then falls through without ever handing
//! it an image, exactly as it does for a Windows machine with no language pack (FM-16).
// TASK: T-059 — the Vision implementation.

use image::RgbaImage;

use super::{OcrEngine, OcrError, TextBlock};

/// Vision, once T-059 has written it.
#[derive(Debug, Clone, Copy)]
pub struct VisionOcr;

impl OcrEngine for VisionOcr {
    fn name(&self) -> &str {
        // The value the published outcome fixtures already carry
        // (`fixtures/outcomes/screenshot.json`), so it is fixed rather than chosen here.
        "vision"
    }

    fn available(&self, _lang_hint: Option<&str>) -> bool {
        false
    }

    fn recognize(
        &self,
        _image: &RgbaImage,
        _lang_hint: Option<&str>,
    ) -> Result<Vec<TextBlock>, OcrError> {
        Err(OcrError::Engine(
            "the Vision engine is not implemented yet".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_never_selected_while_it_cannot_read_anything() {
        assert_eq!(VisionOcr.name(), "vision");
        assert!(!VisionOcr.available(None));
        assert!(!VisionOcr.available(Some("en")));
    }
}
