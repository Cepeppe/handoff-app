//! Language resolution on the Rust side (APP-02, §7.16).
//!
//! The UI language is the setting when the user chose one, else the system language when
//! it is `en` or `it`, else English. The strings themselves live in the frontend
//! (`src/locales/{en,it}.json`); what belongs here is the resolution and the few texts the
//! Rust side produces on its own — the tray menu, the notifications and the crash notice.
// TASK: T-028 (resolution and the first keys), T-041 (settings and the i18n audit)
