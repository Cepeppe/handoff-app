-- 0001_init: the log and the state it shares a transaction with (TECHNICAL-DESIGN §7.11,
-- DD-31, LOG-01..05, NET-01, SRV-12).
--
-- One database holds both the log and the handoff state, so a transition and its log row
-- commit together or not at all (DD-31). Every table of §7.11 is created here; migrations
-- are forward-only from this file on, and a column added later is added by a new file,
-- never by editing this one — a database created by an earlier release must reach the
-- current schema by replaying the same steps.
--
-- Conventions, all of them load-bearing:
--
--   * Every instant is TEXT in canonical RFC 3339 UTC with milliseconds
--     (`2026-09-08T12:34:56.789Z`), written through `log::time::Timestamp`. That form sorts
--     lexicographically in the same order it sorts chronologically, which is what the age
--     comparisons of the orphan rule (SRV-23) and of the session purge (§8.3) rely on.
--   * Every `*_json` column is a JSON document as text. The log does not look inside them,
--     with the single exception of `handoffs.spec_json`, which it writes masked (LOG-02).
--   * The CHECK constraints spell out the vocabularies §8.1 and §7.11 fix. They are here
--     because this schema is hard to change once databases exist on users' machines: a
--     typo in a state name has to fail at the write, not three releases later.

CREATE TABLE sessions (
    -- `ses_` + 8 characters (§4.1). One row per registration; a reconnection is a new
    -- session and the old row stays as history (§8.3).
    session_ref       TEXT PRIMARY KEY,
    agent_id          TEXT,
    client_name       TEXT,
    client_version    TEXT,
    -- The completed ancestor chain (SRV-17, DD-22), as a JSON array of PIDs.
    pid_chain_json    TEXT NOT NULL,
    cwd               TEXT,
    project_dir       TEXT,
    -- Bound once a hook's chain intersects this session's PIDs (§7.5).
    claude_session_id TEXT,
    -- §7.5 keeps `connected` on the Session record; on disk it is what separates a live
    -- registration from the history the purge rule of §8.3 may remove.
    connected         INTEGER NOT NULL DEFAULT 1 CHECK (connected IN (0, 1)),
    first_seen        TEXT NOT NULL,
    last_seen         TEXT NOT NULL
);

CREATE TABLE handoffs (
    -- `hf_` + 10 characters (§4.1). A user-opened request keeps its id when the spec
    -- arrives, so one row lives from "waiting for spec" to its final state (DD-13).
    id                TEXT PRIMARY KEY,
    created_at        TEXT NOT NULL,
    closed_at         TEXT,
    -- The opener (§7.4). Not in the abridged column list of §7.11, and required by it: the
    -- session purge of §8.3 removes a session only when no handoff points at it, and
    -- RESTRICT is what makes that a promise of the database rather than of one query.
    session_ref       TEXT REFERENCES sessions (session_ref) ON DELETE RESTRICT,
    agent_id          TEXT,
    client_name       TEXT,
    project_dir       TEXT,
    -- The text of the user request this handoff grew from, when it grew from one (OPEN-04).
    request_text      TEXT,
    state             TEXT NOT NULL CHECK (state IN (
                          'awaiting_spec', 'active', 'deferred', 'parked',
                          'awaiting_verification', 'verified', 'failed',
                          'not_verified', 'confirmed_by_user', 'abandoned')),
    -- Set when a final state is reached, and equal to `state` from then on (§8.1).
    final_state       TEXT CHECK (final_state IN (
                          'verified', 'failed', 'not_verified',
                          'confirmed_by_user', 'abandoned')),
    -- The spec **with secret-treated values masked** (LOG-02, DET-04). Null while the
    -- handoff is still waiting for one.
    spec_json         TEXT,
    -- The store's own serialised state (§7.4). Opaque here.
    state_json        TEXT NOT NULL,
    -- When the final outcome reached an agent. Null and old is what makes an orphan
    -- (SRV-23).
    delivered_at      TEXT,
    resumed_from_json TEXT,
    lang              TEXT
);

CREATE INDEX idx_handoffs_state ON handoffs (state);
CREATE INDEX idx_handoffs_session ON handoffs (session_ref);

CREATE TABLE rounds (
    handoff_id         TEXT NOT NULL REFERENCES handoffs (id) ON DELETE CASCADE,
    -- 1-based; a replacement opens round n+1 (§7.4).
    no                 INTEGER NOT NULL,
    steps_json         TEXT NOT NULL,
    started_at         TEXT NOT NULL,
    ended_at           TEXT,
    verify_ok          INTEGER CHECK (verify_ok IS NULL OR verify_ok IN (0, 1)),
    verify_detail      TEXT,
    verify_reported_at TEXT,
    -- A report accepted after `not_verified`, within 7 days (VER-10, DD-16).
    verify_late        INTEGER NOT NULL DEFAULT 0 CHECK (verify_late IN (0, 1)),
    PRIMARY KEY (handoff_id, no)
);

CREATE TABLE events (
    id           INTEGER PRIMARY KEY,
    handoff_id   TEXT NOT NULL REFERENCES handoffs (id) ON DELETE CASCADE,
    round        INTEGER,
    at           TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN (
                     'confirm', 'note', 'skip', 'ask', 'screenshot', 'defer', 'abandon',
                     'reply', 'replace', 'resume', 'attach', 'detach', 'state')),
    -- 1-based index into the round's steps, for the kinds that happen on a step.
    step_index   INTEGER,
    payload_json TEXT
);

CREATE INDEX idx_events_handoff ON events (handoff_id, id);

CREATE TABLE sends (
    id                   INTEGER PRIMARY KEY,
    handoff_id           TEXT NOT NULL REFERENCES handoffs (id) ON DELETE CASCADE,
    at                   TEXT NOT NULL,
    kind                 TEXT NOT NULL CHECK (kind IN (
                             'question', 'screenshot_text', 'screenshot_image',
                             'defer', 'abandon')),
    -- The full text exactly as it left, after redaction (LOG-03).
    text_as_sent         TEXT,
    -- **Never pixels** (LOG-03): a hash, the dimensions and the boxes. The length check is
    -- the cheap guard that says so — 64 hex characters cannot be an encoded image.
    image_sha256         TEXT CHECK (image_sha256 IS NULL OR length(image_sha256) = 64),
    image_w              INTEGER,
    image_h              INTEGER,
    redaction_boxes_json TEXT,
    ocr_engine           TEXT,
    patterns_version     TEXT
);

CREATE INDEX idx_sends_handoff ON sends (handoff_id, id);

CREATE TABLE user_requests (
    -- The shape of the handoff it becomes (§4.1, DD-13).
    id                TEXT PRIMARY KEY,
    -- Null while the request waits for the first session to register (OPEN-04a). No
    -- foreign key on purpose: this is a note about where the request went, and it must
    -- survive the purge of the session it names.
    session_ref       TEXT,
    text              TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    -- The two delivery paths of OPEN-05 and OPEN-06; null until one of them ran.
    delivered_via     TEXT CHECK (delivered_via IS NULL
                                  OR delivered_via IN ('clipboard', 'stop_hook')),
    linked_handoff_id TEXT REFERENCES handoffs (id) ON DELETE SET NULL
);

CREATE INDEX idx_user_requests_session ON user_requests (session_ref);

CREATE TABLE hook_blocks (
    -- "At most once per handoff per session" (SRV-12): the primary key *is* the rule.
    session_ref TEXT NOT NULL REFERENCES sessions (session_ref) ON DELETE CASCADE,
    item_key    TEXT NOT NULL,
    at          TEXT NOT NULL,
    PRIMARY KEY (session_ref, item_key)
);

CREATE TABLE network_events (
    id         INTEGER PRIMARY KEY,
    at         TEXT NOT NULL,
    domain     TEXT NOT NULL,
    bytes_sent INTEGER NOT NULL,
    purpose    TEXT NOT NULL
);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    -- A JSON document, so the typed accessors read back what they wrote.
    value TEXT NOT NULL
);
