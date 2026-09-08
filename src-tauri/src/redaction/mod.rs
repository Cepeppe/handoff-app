//! Detection, redaction geometry and burn-in (§7.10, DET-01..04, PREV-01..05).
//!
//! Two detectors over the OCR result: the **certain** patterns, compiled from the pattern
//! file of the pinned server artifact (never a copy of it, §3.4), and the **suspected**
//! heuristics that belong to the app alone. What they find becomes geometry, the geometry
//! is burned into the pixels before anything leaves the machine, and the preview the user
//! must confirm shows the burned image (PRIN-09).
//!
//! [`certain`] is here. The suspected detector, the geometry and the burn-in are not.
// TASK: T-048 — suspected detector, redaction geometry, burn-in, the synthetic corpus.

pub mod certain;
