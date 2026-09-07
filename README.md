# handoff-app

Baton is the desktop overlay application of the contextual handoff system: it shows the
work a coding agent hands over to the person at the machine, guides it one step at a
time, and sends back a structured outcome, keeping screenshots, redaction, the local log
and the runbooks on the machine. It consumes `handoff-mcp` via a pinned release artifact
(`server.lock.json`, verified and unpacked into `vendor/`), never from source. This
repository is proprietary; see `LICENSE`. Status: work in progress, nothing is stable
yet.
