//! The runbook writer (§7.12, RUN-01, RUN-02, RUN-04, RUN-05, RUN-09, DD-17, DD-18).
//!
//! The one thing in this application that turns a person's work into a file another agent
//! will read. It is triggered by the store when a handoff reaches a final state, and it does
//! the three steps of §7.12 in order: build the executed sequence (§4.5.1), substitute the
//! placeholders (§4.5.2), infer the origin by applying the matching rule of §4.5.3 to the
//! runbooks already on disk (**DD-18**: the origin is inferred and never carried in the
//! spec, which SPEC-06 forbids).
//!
//! # The situation table of §7.12
//!
//! | Situation | What happens here |
//! |---|---|
//! | No matching runbook | a new file, `trust` from the final state, `runs: 1` |
//! | Match, executed sequence identical | `last_verified_at`, `runs += 1`, `trust` raised |
//! | Match, sequence differs, ≥ 2 rounds | a [`RunbookProposal`]; **the file is not touched** |
//! | Match, sequence differs, one round | a new file: another way to reach the same goal |
//! | `failed`, a match exists | `last_run_failed_at`; never a deletion (RUN-09) |
//!
//! Two of the five final states write nothing at all: RUN-01 forbids a runbook from a
//! `not_verified` or an `abandoned` handoff, and there is nothing to mark for either.
//!
//! # What can stop a write, and why none of it is an error the store hears about
//!
//! A runbook that cannot be written must not undo a handoff that is finished (PRIN-10), so
//! every failure below is logged and swallowed:
//!
//! - **the last defence.** The certain detector runs over the finished document and a match
//!   aborts the write (§4.5.2). After [`super::placeholders`] it must find nothing; if it
//!   does, this file is the reason a value would have leaked and the runbook is not worth
//!   the leak.
//! - **the schema.** The document is validated against the vendored
//!   `handoff-runbook.v1.schema.json` before it is written, which is what keeps the promise
//!   that a runbook always converts back into a spec that validates (`> Note from T-005`):
//!   substitution can lengthen a text past `maxLength`, and several rounds can push the
//!   executed sequence past fifty steps.
//! - **nothing was executed.** A handoff nobody confirmed a step in has no recipe in it.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::format::outcome::RunbookTrust;
use crate::format::runbook::{Runbook, RunbookOrigin, RunbookStep};
use crate::format::schema::{self, Document, Problem};
use crate::format::spec::HandoffSpec;
use crate::ids::new_runbook_id;
use crate::log::{Db, Timestamp};
use crate::redaction::certain::scan_text;
use crate::runbooks::matching::{self, RunbookQuery, StoredRunbook};
use crate::store::handoff::{FinalState, Handoff};
use crate::store::runbook_sink::RunbookSink;

use super::sequence::{self, ExecutedStep};
use super::{placeholders, RunbookProposal};

/// What `origin.app` says. The repository's name, as §4.5's example prints it: the file is a
/// public artifact and `handoff-app` is what a reader of it can look up, while `Baton` is the
/// product name the user sees (T-001 D2).
pub const ORIGIN_APP: &str = "handoff-app";

/// The most steps a runbook may hold (`handoff-runbook.v1.schema.json`).
const MAX_STEPS: usize = 50;

/// The longest a slug of the file name may be (DD-17).
const MAX_SLUG_LENGTH: usize = 60;

/// The slug of a `where` or a `goal` that normalises to nothing at all.
const EMPTY_SLUG: &str = "runbook";

/// Why a runbook was not written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// The handoff never received a spec, so there is nothing to record.
    #[error("the handoff has no spec")]
    NoSpec,
    /// No step was ever confirmed: a handoff with no recipe in it.
    #[error("no step of this handoff was confirmed")]
    NoSteps,
    /// The executed sequence is longer than a runbook may be.
    #[error("the executed sequence is {0} steps, and a runbook holds at most {MAX_STEPS}")]
    TooManySteps(usize),
    /// The finished document does not satisfy the vendored schema.
    #[error("the runbook does not validate: {0}")]
    Invalid(String),
    /// The certain detector matched the finished document (§4.5.2, the last defence).
    #[error("a certain secret survived into the runbook; the file was not written")]
    SecretFound,
    /// The diary could not be read.
    #[error("the handoff's exchanges could not be read: {0}")]
    Diary(#[from] crate::store::Refusal),
    /// The file could not be written, or the folder could not be read.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The document could not be serialised.
    #[error("{0}")]
    Serialize(#[from] serde_json::Error),
}

/// What one finalised handoff did to the runbook folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// Nothing: the final state writes no runbook and marks none.
    Nothing,
    /// A new file, at this path.
    Created(PathBuf),
    /// An existing file refreshed in place (§7.12, row 2).
    Refreshed(PathBuf),
    /// An existing file marked as having failed (RUN-09).
    MarkedFailed(PathBuf),
    /// A rewrite the user has to accept (§7.12, row 3). Nothing was written.
    Proposed(Box<RunbookProposal>),
}

impl Written {
    /// Which row of §7.12 this was, in one word, for a log line.
    ///
    /// The whole value is never logged: [`Written::Proposed`] carries the proposed document,
    /// and a debug line that printed it would put a runbook's every step into the log for a
    /// file nobody has written yet.
    #[must_use]
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Nothing => "nothing",
            Self::Created(_) => "created",
            Self::Refreshed(_) => "refreshed",
            Self::MarkedFailed(_) => "marked as failed",
            Self::Proposed(_) => "proposed",
        }
    }
}

/// The writer of `~/.handoff/runbooks/`.
///
/// It holds the folder rather than asking [`crate::paths`] on every call, so that a test can
/// point it at a directory of its own and the production instance is built once, after the
/// environment is settled (`lib.rs`).
#[derive(Debug, Clone)]
pub struct RunbookWriter {
    dir: PathBuf,
}

impl RunbookWriter {
    /// A writer over `dir`.
    #[must_use]
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The writer of the contract folder, `~/.handoff/runbooks/` (RUN-03a).
    #[must_use]
    pub fn in_handoff_home() -> Self {
        Self::at(crate::paths::runbooks_dir())
    }

    /// Where it writes.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Rewrites the file a proposal names, which is what accepting it means (RUN-09).
    ///
    /// # Errors
    ///
    /// [`WriteError`] when the document does not validate, the detector matches it, or the
    /// file cannot be written. Nothing is written in the first two cases.
    pub fn accept(&self, proposal: &RunbookProposal) -> Result<PathBuf, WriteError> {
        let path = PathBuf::from(&proposal.path);
        self.write_file(&path, &proposal.runbook)?;
        Ok(path)
    }

    /// Every runbook the folder holds, with the file it came from.
    ///
    /// A file that cannot be read or does not parse is skipped with a warning and never
    /// fails the read, which is the app's half of FM-19: a broken file in the folder must
    /// not stop the runbook of the handoff that has just finished from being written.
    #[must_use]
    pub fn stored(&self) -> Vec<StoredRunbook> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            // No folder yet is the ordinary case on a fresh installation.
            return Vec::new();
        };
        let mut found: Vec<StoredRunbook> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            match fs::read_to_string(&path)
                .map_err(|error| error.to_string())
                .and_then(|text| {
                    serde_json::from_str::<Runbook>(&text).map_err(|error| error.to_string())
                }) {
                Ok(runbook) => found.push(StoredRunbook { path, runbook }),
                Err(error) => tracing::warn!(
                    file = %path.display(),
                    error = %error,
                    "a runbook file could not be read and was skipped"
                ),
            }
        }
        // `read_dir` has no order of its own; the matching rule breaks its own ties by id,
        // but a caller that lists the folder deserves the same answer twice.
        found.sort_by(|left, right| left.path.cmp(&right.path));
        found
    }

    /// What §7.12 does for `handoff`, having just reached `final_state`.
    ///
    /// # Errors
    ///
    /// [`WriteError`], which the [`RunbookSink`] implementation logs and swallows.
    pub fn run(
        &self,
        db: &Db,
        handoff: &Handoff,
        final_state: FinalState,
    ) -> Result<Written, WriteError> {
        let spec = handoff.spec.as_ref().ok_or(WriteError::NoSpec)?;
        // The instant the handoff closed, not the wall clock: `last_verified_at` is a fact
        // about the run, and the store already decided when the run ended. The fallback is
        // unreachable — a final state always sets `closed_at` — and is written as one rather
        // than as a panic, because a corrupted `state_json` must not take the app down.
        let now = handoff.closed_at.clone().unwrap_or_else(Timestamp::now);
        let stored = self.stored();
        let existing = best_match(&stored, spec);

        if final_state == FinalState::Failed {
            // RUN-09: the mark is written the moment the run fails. Whether a correction
            // follows is not knowable here, and if one does and succeeds it comes back
            // through this function as `verified` and refreshes the same file. The mark is
            // a fact about a run that did fail, and RUN-09 never removes it.
            let Some(existing) = existing else {
                return Ok(Written::Nothing);
            };
            let mut marked = existing.runbook.clone();
            marked.last_run_failed_at = Some(now.as_str().to_owned());
            marked.updated_at = now.as_str().to_owned();
            self.write_file(&existing.path, &marked)?;
            return Ok(Written::MarkedFailed(existing.path.clone()));
        }

        if !final_state.writes_a_runbook() {
            // RUN-01: never from a `not_verified` or an `abandoned` handoff.
            return Ok(Written::Nothing);
        }

        let executed = sequence::executed(handoff, &crate::store::exchanges_of(db, &handoff.id)?);
        if executed.is_empty() {
            return Err(WriteError::NoSteps);
        }
        if executed.len() > MAX_STEPS {
            return Err(WriteError::TooManySteps(executed.len()));
        }
        let trust = trust_of(final_state);
        let candidate = self.compose(spec, handoff, &executed, trust, &now);

        let Some(existing) = existing else {
            let path = self.dir.join(file_name(spec, &candidate.id));
            self.write_file(&path, &candidate)?;
            return Ok(Written::Created(path));
        };

        if same_recipe(&existing.runbook.steps, &candidate.steps) {
            // Row 2: the recipe on disk is the one that was executed, so only the freshness
            // changes. The annotations already on the file stay as they are: a runbook is a
            // recipe, and folding every run's notes into it would turn it into the diary
            // RUN-02 keeps out of it.
            let mut refreshed = existing.runbook.clone();
            refreshed.last_verified_at = now.as_str().to_owned();
            refreshed.updated_at = now.as_str().to_owned();
            refreshed.runs = refreshed.runs.saturating_add(1);
            refreshed.trust = raised(refreshed.trust, trust);
            refreshed.origin = origin();
            self.write_file(&existing.path, &refreshed)?;
            return Ok(Written::Refreshed(existing.path.clone()));
        }

        if handoff.rounds.len() >= 2 {
            // Row 3: a correction happened, so the difference is a correction of the recipe
            // and not a second way of reaching the goal. The user decides (RUN-09), and
            // until they do the new sequence lives in `state_json`.
            let proposal = RunbookProposal {
                runbook_id: existing.runbook.id.clone(),
                path: existing.path.to_string_lossy().into_owned(),
                goal: existing.runbook.goal.clone(),
                runbook: folded_into(&existing.runbook, candidate, &now, trust),
            };
            // It is not written, but it is checked as if it were: a proposal the user
            // accepts must not be the first moment a defect is found.
            check(&proposal.runbook)?;
            return Ok(Written::Proposed(Box::new(proposal)));
        }

        // Row 4: same place, same goal, a different way of getting there.
        let path = self.dir.join(file_name(spec, &candidate.id));
        self.write_file(&path, &candidate)?;
        Ok(Written::Created(path))
    }

    /// The runbook this handoff would create, as a brand-new file.
    fn compose(
        &self,
        spec: &HandoffSpec,
        handoff: &Handoff,
        executed: &[ExecutedStep],
        trust: RunbookTrust,
        now: &Timestamp,
    ) -> Runbook {
        let filled = placeholders::apply(spec, executed, &|name| secret_treated(handoff, name));
        Runbook {
            runbook_version: 1,
            id: new_runbook_id(),
            // These three are copied and not substituted (§4.5.2 names only the step texts,
            // the warnings and the `verify`), but they are scanned at ingress like every
            // other text of §5.5, so they are masked: see `placeholders::mask`.
            r#where: placeholders::mask(&spec.r#where),
            goal: placeholders::mask(&spec.goal),
            why_human: placeholders::mask(&spec.why_human),
            url: spec.url.clone(),
            lang: spec.lang.clone(),
            values: filled.values,
            secrets: spec.secrets.clone().unwrap_or_default(),
            steps: filled.steps,
            verify: filled.verify,
            trust,
            last_verified_at: now.as_str().to_owned(),
            last_run_failed_at: None,
            runs: 1,
            created_at: now.as_str().to_owned(),
            updated_at: now.as_str().to_owned(),
            origin: origin(),
        }
    }

    /// Validates `runbook`, re-runs the detector over it and writes it atomically.
    fn write_file(&self, path: &Path, runbook: &Runbook) -> Result<(), WriteError> {
        let text = check(runbook)?;
        fs::create_dir_all(&self.dir)?;
        // A temporary file beside the target, so the rename stays on one volume, and a name
        // nothing else can be using: two writes of the same runbook in the same millisecond
        // would otherwise overwrite each other's half-written file.
        let temporary = self.dir.join(format!(
            ".{}.{}.tmp",
            path.file_name().map_or_else(
                || runbook.id.clone(),
                |name| name.to_string_lossy().into_owned()
            ),
            new_runbook_id()
        ));
        fs::write(&temporary, text.as_bytes())?;
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(WriteError::Io(error));
        }
        Ok(())
    }
}

impl Default for RunbookWriter {
    fn default() -> Self {
        Self::in_handoff_home()
    }
}

impl RunbookSink for RunbookWriter {
    fn on_finalised(
        &self,
        db: &Db,
        handoff: &Handoff,
        final_state: FinalState,
    ) -> Option<RunbookProposal> {
        match self.run(db, handoff, final_state) {
            Ok(Written::Proposed(proposal)) => Some(*proposal),
            Ok(written) => {
                tracing::debug!(
                    handoff_id = %handoff.id,
                    written = written.describe(),
                    "the runbook folder is up to date"
                );
                None
            }
            Err(error) => {
                // Never the spec, never a step, never a value (R-19): the handoff's id and
                // the reason are what a person needs and all they may be told.
                tracing::warn!(
                    handoff_id = %handoff.id,
                    error = %error,
                    "no runbook was written for this handoff"
                );
                None
            }
        }
    }

    fn accept(&self, proposal: &RunbookProposal) -> Result<(), String> {
        // Named rather than `self.accept(...)`: the inherent method and this one differ
        // only in what they answer, and the call would silently pick either.
        Self::accept(self, proposal)
            .map(|path| {
                tracing::info!(
                    runbook_id = %proposal.runbook_id,
                    file = %path.display(),
                    "a runbook update proposal was accepted"
                );
            })
            .map_err(|error| error.to_string())
    }
}

/// Whether the ingress detector matched anything inside the value `name` (DET-04, §4.5.2).
///
/// [`Handoff::is_secret_value`] answers this for a single-valued entry, whose location the
/// server reports as exactly `values.<name>`. An array's items are reported one index at a
/// time (`values.<name>[0]`, §4.7.5) and that helper's equality does not see them, while
/// §4.5.2 calls the **value** secret-treated however many of its items matched — its
/// description must say nothing about it either way. Hence the prefix, which covers both
/// shapes; nothing here changes what the helper answers for its own callers.
fn secret_treated(handoff: &Handoff, name: &str) -> bool {
    let exact = format!("values.{name}");
    let indexed = format!("{exact}[");
    handoff
        .secret_treated
        .iter()
        .any(|treated| treated.location == exact || treated.location.starts_with(&indexed))
}

/// `handoff-app` and its version, as every file it writes records them.
fn origin() -> RunbookOrigin {
    RunbookOrigin {
        app: ORIGIN_APP.to_owned(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

/// The runbook the spec's `where` and `goal` came from, if the folder holds one (DD-18).
fn best_match<'a>(stored: &'a [StoredRunbook], spec: &HandoffSpec) -> Option<&'a StoredRunbook> {
    let query = RunbookQuery {
        where_: spec.r#where.clone(),
        goal: spec.goal.clone(),
        lang: spec.lang.clone(),
    };
    matching::match_runbooks(stored, &query)
        .first()
        .map(|matched| matched.stored)
}

/// The trust a final state gives a runbook (RUN-01).
fn trust_of(final_state: FinalState) -> RunbookTrust {
    match final_state {
        FinalState::Verified => RunbookTrust::Verified,
        _ => RunbookTrust::ConfirmedByUser,
    }
}

/// Trust never goes down: a file an agent once verified stays verified even when the next
/// run of it was only confirmed by the user (§7.12, "raise `trust` to `verified`").
fn raised(current: RunbookTrust, reached: RunbookTrust) -> RunbookTrust {
    if current == RunbookTrust::Verified || reached == RunbookTrust::Verified {
        RunbookTrust::Verified
    } else {
        RunbookTrust::ConfirmedByUser
    }
}

/// Whether two step lists are the same **recipe**: same texts, same links, same
/// placeholders, same warnings. The annotations are what happened on a run and differ
/// between two runs of the same recipe by definition.
fn same_recipe(left: &[RunbookStep], right: &[RunbookStep]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(one, other)| {
            one.text == other.text
                && one.url == other.url
                && one.values == other.values
                && one.warning == other.warning
        })
}

/// The candidate document as a rewrite of `existing`: the file keeps its identity and its
/// history, the run brings the sequence and the freshness.
fn folded_into(
    existing: &Runbook,
    candidate: Runbook,
    now: &Timestamp,
    trust: RunbookTrust,
) -> Runbook {
    Runbook {
        id: existing.id.clone(),
        created_at: existing.created_at.clone(),
        last_run_failed_at: existing.last_run_failed_at.clone(),
        runs: existing.runs.saturating_add(1),
        trust: raised(existing.trust, trust),
        last_verified_at: now.as_str().to_owned(),
        updated_at: now.as_str().to_owned(),
        ..candidate
    }
}

/// The file name of DD-17: `<where-slug>__<goal-slug>__<id>.json`.
fn file_name(spec: &HandoffSpec, id: &str) -> String {
    format!("{}__{}__{id}.json", slug(&spec.r#where), slug(&spec.goal))
}

/// One slug of the file name (DD-17): the normalised text with every run of
/// non-alphanumerics replaced by `-`, cut to sixty characters.
///
/// [`matching::normalize_where`] is the normalisation of §4.5.3, and it is applied to the
/// `goal` as well: what a file name needs from it is the same folding of case, width and
/// separators, and using one function means a `where` and a `goal` that read the same to the
/// matching rule read the same in the folder.
fn slug(text: &str) -> String {
    let normalised = matching::normalize_where(text);
    let mut slug = String::with_capacity(normalised.len());
    let mut pending = false;
    for character in normalised.chars().take(MAX_SLUG_LENGTH) {
        if character.is_alphanumeric() {
            if pending && !slug.is_empty() {
                slug.push('-');
            }
            pending = false;
            slug.push(character);
        } else {
            pending = true;
        }
    }
    if slug.is_empty() {
        EMPTY_SLUG.to_owned()
    } else {
        slug
    }
}

/// The document as it would be written, having passed the schema and the detector.
///
/// The two checks are together because they answer the same question — may this reach the
/// disk — and because both need the serialised text: the validator reads the JSON value and
/// the detector reads every string of it at once, including the ones a future field might
/// add without anyone remembering this function.
fn check(runbook: &Runbook) -> Result<String, WriteError> {
    let value: Value = serde_json::to_value(runbook)?;
    if let Err(problems) = schema::validate(Document::Runbook, &value) {
        return Err(WriteError::Invalid(describe(&problems)));
    }
    let text = format!("{}\n", serde_json::to_string_pretty(runbook)?);
    if !scan_text(&text).is_empty() {
        return Err(WriteError::SecretFound);
    }
    Ok(text)
}

/// The schema problems as one line: where, and which keyword refused it. Never a value.
fn describe(problems: &[Problem]) -> String {
    problems
        .iter()
        .map(|problem| format!("{} ({})", problem.path, problem.keyword))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;
    use crate::format::outcome::{AnnotationKind, ResumedFrom};
    use crate::format::spec::{HandoffStep, SpecValue};
    use crate::log::sessions;
    use crate::log::testing::{at, session, STRIPE_KEY};
    use crate::runbooks::placeholders::SECRET_DESCRIPTION;
    use crate::store::handoff::{Call, Opener};
    use crate::store::{NoRequests, NoRunbookSink, OpenParams, Store};

    const OPENER: &str = "ses_00000001";

    /// A temporary runbook folder of this test binary's own, removed when it is dropped.
    struct Folder(PathBuf);

    impl Folder {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "handoff-runbooks-{}-{}",
                std::process::id(),
                crate::ids::new_session_ref()
            ));
            fs::create_dir_all(&dir).expect("the temporary folder is created");
            Self(dir)
        }

        fn writer(&self) -> RunbookWriter {
            RunbookWriter::at(&self.0)
        }

        /// The runbook files it holds, by name, sorted.
        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .expect("the folder is readable")
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".json"))
                .collect();
            names.sort();
            names
        }

        /// The one runbook it holds.
        fn only(&self) -> Runbook {
            let stored = self.writer().stored();
            assert_eq!(stored.len(), 1, "exactly one runbook: {:?}", self.names());
            stored[0].runbook.clone()
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn database() -> Db {
        let db = Db::open_in_memory().expect("a database");
        sessions::register(&db, &session(OPENER)).expect("a session");
        db
    }

    fn store(folder: &Folder) -> Store {
        Store::load(database(), Box::new(folder.writer()), Box::new(NoRequests))
            .expect("an empty store")
    }

    fn spec(steps: &[&str], verify: Option<&str>) -> HandoffSpec {
        let mut values = IndexMap::new();
        values.insert(
            "endpoint_url".to_owned(),
            SpecValue::One("https://api.example.test/hook".to_owned()),
        );
        HandoffSpec {
            spec_version: 1,
            goal: "Register the webhook for payment events".to_owned(),
            r#where: "Stripe Dashboard → Developers → Webhooks".to_owned(),
            url: None,
            why_human: "Requires access to the production account.".to_owned(),
            values,
            secrets: None,
            steps: steps
                .iter()
                .map(|text| HandoffStep {
                    text: (*text).to_owned(),
                    url: None,
                    values: None,
                    warning: None,
                })
                .collect(),
            verify: verify.map(ToOwned::to_owned),
            lang: Some("en".to_owned()),
        }
    }

    fn opener() -> Opener {
        Opener {
            session_ref: OPENER.to_owned(),
            agent_id: Some("claude-code".to_owned()),
            client_name: Some("claude-code".to_owned()),
            project_dir: Some("C:/projects/baton".to_owned()),
            label: ResumedFrom {
                agent: "Claude Code".to_owned(),
                project: "baton".to_owned(),
            },
        }
    }

    fn call(call_id: &str) -> Call {
        Call {
            conn_id: 1,
            call_id: call_id.to_owned(),
            session_ref: Some(OPENER.to_owned()),
        }
    }

    fn open(store: &mut Store, spec: HandoffSpec) -> String {
        store
            .open(
                OpenParams {
                    spec,
                    secret_treated: Vec::new(),
                    request_id: None,
                    opener: opener(),
                    call: call("call_00000001"),
                },
                &at("2026-09-09T11:00:00Z"),
            )
            .expect("the open is accepted")
            .handoff_id
    }

    /// Confirms every step of the current round and presses Done.
    fn walk(store: &mut Store, id: &str, steps: usize, moment: &str) {
        for _ in 0..steps {
            store.confirm(id, &at(moment)).expect("a confirmation");
        }
        store.done(id, &at(moment)).expect("done");
    }

    /// The whole happy path: open, walk every step, report a successful verification.
    fn verified(store: &mut Store, spec: HandoffSpec, moment: &str) -> String {
        let steps = spec.steps.len();
        let id = open(store, spec);
        walk(store, &id, steps, moment);
        store
            .verify(&id, Some(true), Some("it fires".to_owned()), &at(moment))
            .expect("the report is accepted");
        id
    }

    #[test]
    fn a_verified_handoff_creates_a_runbook_that_validates() {
        let folder = Folder::new();
        let mut store = store(&folder);
        verified(
            &mut store,
            spec(
                &["Open Developers.", "Paste https://api.example.test/hook."],
                Some("A test event reaches https://api.example.test/hook."),
            ),
            "2026-09-09T11:00:00Z",
        );

        // The file itself, against the vendored schema: what `check` promises before every
        // write is asserted here on what actually reached the disk (§4.5, `> Note from
        // T-005`), because that is the document the server will read.
        let written = fs::read_to_string(folder.0.join(&folder.names()[0])).expect("the file");
        let value: Value = serde_json::from_str(&written).expect("the file is JSON");
        assert_eq!(schema::validate(Document::Runbook, &value), Ok(()));

        let runbook = folder.only();
        assert_eq!(runbook.runbook_version, 1);
        assert_eq!(runbook.trust, RunbookTrust::Verified);
        assert_eq!(runbook.runs, 1);
        assert_eq!(runbook.last_run_failed_at, None);
        assert_eq!(runbook.origin.app, ORIGIN_APP);
        assert_eq!(runbook.origin.app_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(runbook.steps.len(), 2);
        assert_eq!(runbook.steps[1].text, "Paste {{endpoint_url}}.");
        assert_eq!(runbook.steps[1].values, vec!["endpoint_url"]);
        assert_eq!(
            runbook.verify.as_deref(),
            Some("A test event reaches {{endpoint_url}}.")
        );
        assert_eq!(
            runbook.values["endpoint_url"].description.as_deref(),
            Some("Paste {{endpoint_url}}.")
        );
        // The value itself is nowhere in the file (RUN-04).
        let text = fs::read_to_string(folder.0.join(&folder.names()[0])).expect("the file");
        assert!(!text.contains("https://api.example.test/hook"), "{text}");
    }

    #[test]
    fn the_file_name_is_the_two_slugs_and_the_id() {
        let folder = Folder::new();
        let mut store = store(&folder);
        verified(
            &mut store,
            spec(&["Open Developers."], Some("it fires")),
            "2026-09-09T11:00:00Z",
        );
        let runbook = folder.only();
        assert_eq!(
            folder.names(),
            vec![format!(
                "stripe-dashboard-developers-webhooks__register-the-webhook-for-payment-events__{}.json",
                runbook.id
            )]
        );
    }

    #[test]
    fn a_handoff_with_no_verify_is_only_confirmed_by_the_user() {
        let folder = Folder::new();
        let mut store = store(&folder);
        let id = open(&mut store, spec(&["Open Developers."], None));
        walk(&mut store, &id, 1, "2026-09-09T11:00:00Z");
        assert_eq!(folder.only().trust, RunbookTrust::ConfirmedByUser);
    }

    #[test]
    fn a_skipped_step_is_not_in_the_recipe() {
        let folder = Folder::new();
        let mut store = store(&folder);
        let id = open(
            &mut store,
            spec(&["Open Developers.", "Skip me.", "Save."], Some("it fires")),
        );
        let moment = at("2026-09-09T11:00:00Z");
        store.confirm(&id, &moment).expect("step 1");
        store.skip(&id, &moment).expect("step 2");
        store.confirm(&id, &moment).expect("step 3");
        store.done(&id, &moment).expect("done");
        store
            .verify(&id, Some(true), None, &moment)
            .expect("the report");

        let texts: Vec<String> = folder
            .only()
            .steps
            .iter()
            .map(|step| step.text.clone())
            .collect();
        assert_eq!(texts, vec!["Open Developers.", "Save."]);
    }

    #[test]
    fn a_note_becomes_an_annotation_on_its_step() {
        let folder = Folder::new();
        let mut store = store(&folder);
        let id = open(&mut store, spec(&["Open Developers."], Some("it fires")));
        let moment = at("2026-09-09T11:00:00Z");
        store
            .note(&id, "the button is called Add destination now", &moment)
            .expect("a note");
        store.confirm(&id, &moment).expect("step 1");
        store.done(&id, &moment).expect("done");
        store
            .verify(&id, Some(true), None, &moment)
            .expect("the report");

        let runbook = folder.only();
        assert_eq!(runbook.steps[0].annotations.len(), 1);
        assert_eq!(runbook.steps[0].annotations[0].kind, AnnotationKind::Note);
        assert_eq!(
            runbook.steps[0].annotations[0].text,
            "the button is called Add destination now"
        );
        assert_eq!(runbook.steps[0].annotations[0].round, 1);
    }

    #[test]
    fn a_question_and_the_answer_to_it_become_two_annotations_on_the_step() {
        let folder = Folder::new();
        let mut store = store(&folder);
        let id = open(
            &mut store,
            spec(&["Open Developers.", "Save."], Some("it fires")),
        );
        let moment = at("2026-09-09T11:00:00Z");
        store
            .ask(&id, "which of the two tabs?", &moment)
            .expect("a question");
        store
            .continue_handoff(&id, &call("call_00000002"), "the second one", None, &moment)
            .expect("the answer");
        store.confirm(&id, &moment).expect("step 1");
        store.confirm(&id, &moment).expect("step 2");
        store.done(&id, &moment).expect("done");
        store
            .verify(&id, Some(true), None, &moment)
            .expect("the report");

        let runbook = folder.only();
        assert_eq!(
            runbook.steps[0]
                .annotations
                .iter()
                .map(|annotation| (annotation.kind, annotation.text.clone()))
                .collect::<Vec<_>>(),
            vec![
                (
                    AnnotationKind::Question,
                    "which of the two tabs?".to_owned()
                ),
                (AnnotationKind::Reply, "the second one".to_owned()),
            ]
        );
        assert!(runbook.steps[1].annotations.is_empty());
    }

    #[test]
    fn a_sequence_longer_than_a_runbook_may_be_is_refused_and_nothing_is_written() {
        let folder = Folder::new();
        let mut store = store(&folder);
        let steps: Vec<String> = (1..=MAX_STEPS + 1).map(|at| format!("step {at}")).collect();
        let texts: Vec<&str> = steps.iter().map(String::as_str).collect();
        verified(
            &mut store,
            spec(&texts, Some("it fires")),
            "2026-09-09T11:00:00Z",
        );
        assert!(folder.names().is_empty(), "{:?}", folder.names());
    }

    #[test]
    fn a_second_run_of_the_same_recipe_refreshes_the_file_instead_of_adding_one() {
        let folder = Folder::new();
        let mut store = store(&folder);
        verified(
            &mut store,
            spec(&["Open Developers."], Some("it fires")),
            "2026-09-09T11:00:00Z",
        );
        let first = folder.only();

        verified(
            &mut store,
            spec(&["Open Developers."], Some("it fires")),
            "2026-09-09T12:00:00Z",
        );
        let second = folder.only();

        assert_eq!(second.id, first.id, "the same file");
        assert_eq!(second.runs, 2);
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(second.last_verified_at, "2026-09-09T12:00:00.000Z");
        assert_eq!(folder.names().len(), 1);
    }

    #[test]
    fn a_run_only_the_user_confirmed_does_not_lower_the_trust_of_a_verified_runbook() {
        let folder = Folder::new();
        let mut store = store(&folder);
        verified(
            &mut store,
            spec(&["Open Developers."], Some("it fires")),
            "2026-09-09T11:00:00Z",
        );
        assert_eq!(folder.only().trust, RunbookTrust::Verified);

        // The same place, the same goal, the same recipe — and no `verify` this time.
        let id = open(&mut store, spec(&["Open Developers."], None));
        walk(&mut store, &id, 1, "2026-09-09T12:00:00Z");

        let runbook = folder.only();
        assert_eq!(runbook.trust, RunbookTrust::Verified);
        assert_eq!(runbook.runs, 2);
    }

    #[test]
    fn a_different_sequence_with_no_correction_becomes_a_second_file() {
        let folder = Folder::new();
        let mut store = store(&folder);
        verified(
            &mut store,
            spec(&["Open Developers."], Some("it fires")),
            "2026-09-09T11:00:00Z",
        );
        verified(
            &mut store,
            spec(&["Open the API keys page instead."], Some("it fires")),
            "2026-09-09T12:00:00Z",
        );
        assert_eq!(folder.names().len(), 2, "{:?}", folder.names());
    }

    #[test]
    fn a_correction_proposes_the_rewrite_and_leaves_the_file_alone() {
        let folder = Folder::new();
        let mut store = store(&folder);
        verified(
            &mut store,
            spec(&["Open Developers."], Some("it fires")),
            "2026-09-09T11:00:00Z",
        );
        let before = folder.only();

        // A second run of the same runbook that fails, is corrected, and ends verified.
        let id = open(&mut store, spec(&["Open Developers."], Some("it fires")));
        let moment = at("2026-09-09T12:00:00Z");
        store.confirm(&id, &moment).expect("step 1");
        store.done(&id, &moment).expect("done");
        store
            .verify(
                &id,
                Some(false),
                Some("the banner is still a draft".to_owned()),
                &moment,
            )
            .expect("the failure is accepted");
        store
            .continue_handoff(
                &id,
                &call("call_00000002"),
                "start from the draft instead",
                Some(vec![HandoffStep {
                    text: "Open the draft banner and press Publish.".to_owned(),
                    url: None,
                    values: None,
                    warning: None,
                }]),
                &moment,
            )
            .expect("the correction round opens");
        store.confirm(&id, &moment).expect("the new step");
        store.done(&id, &moment).expect("done again");
        store
            .verify(&id, Some(true), None, &moment)
            .expect("the second report");

        // The recipe on disk is untouched: the user has not answered yet (§7.12 row 3). The
        // one thing that did change is the mark the *failed* half of this run wrote, which
        // is row 5 of the same table doing its job.
        let untouched = folder.only();
        assert_eq!(untouched.steps, before.steps);
        assert_eq!(untouched.runs, before.runs);
        assert_eq!(
            untouched.last_run_failed_at.as_deref(),
            Some("2026-09-09T12:00:00.000Z")
        );

        let summary = store
            .snapshot(&id, &moment)
            .expect("the handoff is there")
            .runbook_proposal
            .expect("a proposal reaches the projection");
        assert_eq!(summary.runbook_id, before.id);
        assert_eq!(summary.goal, before.goal);

        let handoff = store.get(&id).expect("the handoff");
        let proposed = handoff
            .runbook_proposal
            .as_ref()
            .expect("the document itself is kept in state_json");
        assert_eq!(proposed.runbook.id, before.id);
        assert_eq!(proposed.runbook.runs, before.runs + 1);
        assert_eq!(
            proposed
                .runbook
                .steps
                .iter()
                .map(|step| step.text.clone())
                .collect::<Vec<_>>(),
            vec![
                "Open Developers.".to_owned(),
                "Open the draft banner and press Publish.".to_owned()
            ]
        );
        // The failure is on the last step of the round it happened in, and the correction on
        // the first step of the round that followed (§4.5.1).
        assert_eq!(
            proposed.runbook.steps[0]
                .annotations
                .iter()
                .map(|annotation| (annotation.kind, annotation.text.clone()))
                .collect::<Vec<_>>(),
            vec![(
                AnnotationKind::Error,
                "the banner is still a draft".to_owned()
            )]
        );
        assert_eq!(
            proposed.runbook.steps[1]
                .annotations
                .iter()
                .map(|annotation| (annotation.kind, annotation.text.clone()))
                .collect::<Vec<_>>(),
            vec![(
                AnnotationKind::Correction,
                "start from the draft instead".to_owned()
            )]
        );

        // Accepting it is one write of the document that was shown, over the same file.
        folder
            .writer()
            .accept(proposed)
            .expect("the proposal is applied");
        assert_eq!(folder.only(), proposed.runbook);
        assert_eq!(folder.names().len(), 1);
    }

    #[test]
    fn the_proposal_survives_a_restart_because_it_lives_in_state_json() {
        let folder = Folder::new();
        let dir = crate::log::testing::tempdir();
        let path = dir.join("handoff.sqlite");
        let moment = at("2026-09-09T12:00:00Z");

        let id = {
            let db = Db::open_at(&path).expect("a database");
            sessions::register(&db, &session(OPENER)).expect("a session");
            let mut store = Store::load(db, Box::new(folder.writer()), Box::new(NoRequests))
                .expect("an empty store");
            verified(
                &mut store,
                spec(&["Open Developers."], Some("it fires")),
                "2026-09-09T11:00:00Z",
            );
            let id = open(&mut store, spec(&["Open Developers."], Some("it fires")));
            store.confirm(&id, &moment).expect("step 1");
            store.done(&id, &moment).expect("done");
            store
                .verify(&id, Some(false), Some("it never fired".to_owned()), &moment)
                .expect("the failure");
            store
                .continue_handoff(
                    &id,
                    &call("call_00000002"),
                    "try the other page",
                    Some(vec![HandoffStep {
                        text: "Open the API keys page.".to_owned(),
                        url: None,
                        values: None,
                        warning: None,
                    }]),
                    &moment,
                )
                .expect("the correction round");
            store.confirm(&id, &moment).expect("the new step");
            store.done(&id, &moment).expect("done again");
            store
                .verify(&id, Some(true), None, &moment)
                .expect("the second report");
            assert!(store
                .get(&id)
                .expect("the handoff")
                .runbook_proposal
                .is_some());
            id
        };

        let db = Db::open_at(&path).expect("the same database");
        let restored = Store::load(db, Box::new(NoRunbookSink), Box::new(NoRequests))
            .expect("the store comes back");
        let proposal = restored
            .get(&id)
            .expect("the handoff came back")
            .runbook_proposal
            .clone()
            .expect("§7.12 keeps the new sequence until it is decided");
        assert_eq!(proposal.runbook.steps.len(), 2);
        crate::log::testing::clean(&dir);
    }

    #[test]
    fn a_failed_handoff_marks_the_matching_runbook_and_writes_no_new_file() {
        let folder = Folder::new();
        let mut store = store(&folder);
        verified(
            &mut store,
            spec(&["Open Developers."], Some("it fires")),
            "2026-09-09T11:00:00Z",
        );

        let id = open(&mut store, spec(&["Open Developers."], Some("it fires")));
        let moment = at("2026-09-09T12:00:00Z");
        store.confirm(&id, &moment).expect("step 1");
        store.done(&id, &moment).expect("done");
        store
            .verify(&id, Some(false), Some("it never fired".to_owned()), &moment)
            .expect("the failure is accepted");

        let runbook = folder.only();
        assert_eq!(
            runbook.last_run_failed_at.as_deref(),
            Some("2026-09-09T12:00:00.000Z")
        );
        assert_eq!(runbook.runs, 1, "a failure folds no run in");
        assert_eq!(runbook.trust, RunbookTrust::Verified, "and lowers nothing");
    }

    #[test]
    fn a_failed_handoff_with_no_matching_runbook_writes_nothing() {
        let folder = Folder::new();
        let mut store = store(&folder);
        let id = open(&mut store, spec(&["Open Developers."], Some("it fires")));
        let moment = at("2026-09-09T11:00:00Z");
        store.confirm(&id, &moment).expect("step 1");
        store.done(&id, &moment).expect("done");
        store
            .verify(&id, Some(false), Some("it never fired".to_owned()), &moment)
            .expect("the failure is accepted");
        assert!(folder.names().is_empty());
    }

    #[test]
    fn an_abandoned_and_a_not_verified_handoff_write_nothing() {
        let folder = Folder::new();
        let mut store = store(&folder);
        let moment = at("2026-09-09T11:00:00Z");

        let abandoned = open(&mut store, spec(&["Open Developers."], Some("it fires")));
        store.confirm(&abandoned, &moment).expect("step 1");
        store
            .abandon(&abandoned, None, &moment)
            .expect("the tab is abandoned");

        let timed_out = open(&mut store, spec(&["Open Developers."], Some("it fires")));
        walk(&mut store, &timed_out, 1, "2026-09-09T11:00:00Z");
        store
            .verifying_timeout(&timed_out, &at("2026-09-09T12:00:00Z"))
            .expect("the window runs out");

        assert!(folder.names().is_empty(), "{:?}", folder.names());
    }

    #[test]
    fn a_handoff_nobody_confirmed_a_step_of_has_no_recipe_and_writes_nothing() {
        let folder = Folder::new();
        let mut store = store(&folder);
        let id = open(&mut store, spec(&["Open Developers.", "Save."], None));
        let moment = at("2026-09-09T11:00:00Z");
        store.skip(&id, &moment).expect("step 1 skipped");
        store.skip(&id, &moment).expect("step 2 skipped");
        store.done(&id, &moment).expect("done");
        assert!(folder.names().is_empty());
    }

    #[test]
    fn a_certain_secret_outside_a_value_is_masked_and_the_runbook_is_still_written() {
        let folder = Folder::new();
        let mut store = store(&folder);
        let mut planted = spec(&["Open Developers."], Some("it fires"));
        planted.steps[0].text = format!("Sign in with {STRIPE_KEY} and open Developers.");
        planted.why_human = format!("The key {STRIPE_KEY} is only on the production account.");
        verified(&mut store, planted, "2026-09-09T11:00:00Z");

        let runbook = folder.only();
        assert_eq!(
            runbook.steps[0].text,
            "Sign in with [treated as secret: api_key] and open Developers."
        );
        assert_eq!(
            runbook.why_human,
            "The key [treated as secret: api_key] is only on the production account."
        );
        let text = fs::read_to_string(folder.0.join(&folder.names()[0])).expect("the file");
        assert!(!text.contains(STRIPE_KEY), "{text}");
    }

    #[test]
    fn an_array_value_one_of_whose_items_is_a_secret_is_a_secret_treated_value() {
        // §4.5.2 describes a secret-treated value by the fixed sentence, and the server
        // reports an array one index at a time (`values.events[0]`, §4.7.5).
        let folder = Folder::new();
        let mut store = store(&folder);
        let mut planted = spec(&["Select the events."], Some("it fires"));
        planted.values.insert(
            "events".to_owned(),
            SpecValue::Many(vec![STRIPE_KEY.to_owned(), "charge.refunded".to_owned()]),
        );
        planted.steps[0].text = format!("Select {STRIPE_KEY} and charge.refunded.");
        // The channel carries the server's own list; `scan_spec` is the same rule applied
        // here, so the fixture is the shape a real `handoff.open` would arrive with.
        let secret_treated: Vec<crate::format::outcome::SecretTreated> =
            crate::redaction::certain::scan_spec(&planted)
                .into_iter()
                .map(|treated| crate::format::outcome::SecretTreated {
                    location: treated.location,
                    kind: treated.kind.to_string(),
                })
                .collect();
        assert!(
            secret_treated
                .iter()
                .any(|treated| treated.location == "values.events[0]"),
            "the fixture reproduces the indexed location: {secret_treated:?}"
        );

        let id = store
            .open(
                OpenParams {
                    spec: planted,
                    secret_treated,
                    request_id: None,
                    opener: opener(),
                    call: call("call_00000001"),
                },
                &at("2026-09-09T11:00:00Z"),
            )
            .expect("the open is accepted")
            .handoff_id;
        walk(&mut store, &id, 1, "2026-09-09T11:00:00Z");
        store
            .verify(&id, Some(true), None, &at("2026-09-09T11:00:00Z"))
            .expect("the report");

        let runbook = folder.only();
        assert_eq!(
            runbook.values["events"].description.as_deref(),
            Some(SECRET_DESCRIPTION)
        );
        assert_eq!(runbook.steps[0].text, "Select {{events}} and {{events}}.");
        let text = fs::read_to_string(folder.0.join(&folder.names()[0])).expect("the file");
        assert!(!text.contains(STRIPE_KEY), "{text}");
    }

    #[test]
    fn a_secret_that_survived_into_the_document_aborts_the_write() {
        // The last defence of §4.5.2, exercised directly: nothing upstream can produce this
        // document any more, which is exactly why the check is tested on its own.
        let folder = Folder::new();
        let mut leaking = runbook_fixture();
        leaking.why_human = format!("Use {STRIPE_KEY} to sign in.");

        let error = folder
            .writer()
            .write_file(&folder.0.join("leak.json"), &leaking)
            .expect_err("the write is refused");
        assert!(matches!(error, WriteError::SecretFound), "{error}");
        assert!(folder.names().is_empty(), "and nothing reached the disk");
    }

    #[test]
    fn a_document_the_schema_refuses_is_not_written() {
        let folder = Folder::new();
        let mut too_long = runbook_fixture();
        too_long.steps[0].text = "x".repeat(2001);

        let error = folder
            .writer()
            .write_file(&folder.0.join("long.json"), &too_long)
            .expect_err("the write is refused");
        assert!(
            matches!(&error, WriteError::Invalid(problems) if problems.contains("steps[0].text")),
            "{error}"
        );
        assert!(folder.names().is_empty());
    }

    #[test]
    fn a_file_that_does_not_parse_is_skipped_and_the_others_are_read() {
        let folder = Folder::new();
        fs::write(folder.0.join("broken.json"), "{ not json").expect("the broken file");
        folder
            .writer()
            .write_file(&folder.0.join("good.json"), &runbook_fixture())
            .expect("the good one");
        let stored = folder.writer().stored();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].runbook.id, runbook_fixture().id);
    }

    #[test]
    fn a_slug_is_the_normalised_text_and_never_empty() {
        assert_eq!(slug("→ / \\ |"), EMPTY_SLUG);
        assert_eq!(
            slug("Stripe Dashboard → Developers"),
            "stripe-dashboard-developers"
        );
        assert_eq!(slug(&"a".repeat(200)).chars().count(), MAX_SLUG_LENGTH);
    }

    fn runbook_fixture() -> Runbook {
        Runbook {
            runbook_version: 1,
            id: "rb_0123456789".to_owned(),
            r#where: "Stripe Dashboard".to_owned(),
            goal: "Register the webhook".to_owned(),
            why_human: "Only a person can log in.".to_owned(),
            url: None,
            lang: Some("en".to_owned()),
            values: IndexMap::new(),
            secrets: IndexMap::new(),
            steps: vec![RunbookStep {
                text: "Open Developers.".to_owned(),
                url: None,
                values: Vec::new(),
                warning: None,
                annotations: Vec::new(),
            }],
            verify: None,
            trust: RunbookTrust::Verified,
            last_verified_at: "2026-09-09T11:00:00.000Z".to_owned(),
            last_run_failed_at: None,
            runs: 1,
            created_at: "2026-09-09T11:00:00.000Z".to_owned(),
            updated_at: "2026-09-09T11:00:00.000Z".to_owned(),
            origin: origin(),
        }
    }
}
