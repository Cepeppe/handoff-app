-- 0002_resume_requests: the handoff a queued request asks an agent to come back to
-- (TECHNICAL-DESIGN §7.7, FM-31, RESP-07).
--
-- The queue of §7.7 carries two things that reach an agent the same way — the clipboard
-- fast path (OPEN-05) and the Stop hook (OPEN-06) — and mean opposite things to it:
--
--   * a **request**, which asks for a spec and whose id the new handoff takes over
--     (DD-13, OPEN-08);
--   * a **resume**, which asks the agent to pick a handoff the user resumed from the
--     overlay back up (FM-31). It opens nothing.
--
-- One nullable column tells them apart, and it is the handoff itself: a queue entry that
-- names a handoff is a resume, one that names none is a request. That is what keeps
-- OPEN-08 from linking a brand-new handoff to a resume entry, and it is what lets the
-- entry be closed when a call finally attaches to that handoff — `linked_handoff_id`
-- then records, for both kinds alike, the handoff that answered it.
--
-- `ON DELETE CASCADE`: a resume entry is about one handoff and means nothing without it.
-- `linked_handoff_id` is `ON DELETE SET NULL` for the opposite reason — a request the
-- user wrote is their own text and outlives whatever became of it.
ALTER TABLE user_requests ADD COLUMN about_handoff_id TEXT
    REFERENCES handoffs (id) ON DELETE CASCADE;

CREATE INDEX idx_user_requests_about ON user_requests (about_handoff_id);
