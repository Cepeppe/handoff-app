//! Screen capture and the region selection overlay (§7.8, CAP-01..06, PRIN-04).
//!
//! Every capture is decided by the user, one at a time; nothing observes the screen
//! continuously (PRIN-04). The backend is a trait so that the unit tests and the e2e suite
//! can substitute a fixture image (`--features fake-capture`, `--features e2e`).
// TASK: T-046
