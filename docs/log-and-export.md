# Log and export

Baton keeps a record of every handoff on this computer. It is the third of the three things
you can check about Baton for yourself (see [Verify it yourself](verify-trust.md)): not only
what was done, but everything that left the panel.

## Where it is

One SQLite database, `%APPDATA%\Baton\handoff.sqlite`. It is never sent anywhere.

## What it keeps

For each handoff:

- the handoff as the agent wrote it, with every value treated as a secret replaced by a mask
  such as `[treated as secret: api_key]`;
- what the agent was told at the end;
- when it was opened and closed, the agent and the project folder of the session;
- the rounds, what you did on each step, and the verification the agent reported.

For each send — a question, a screenshot, a deferral, an abandonment:

- text **exactly as it left**;
- for a screenshot sent as an image, its size, its SHA-256 hash and the hidden rectangles —
  **never the pixels**.

Nothing is deleted automatically: entries stay until you delete them.

## Settings → Log

The list shows the handoffs that are finished, with when they were opened and closed and how
many rounds they took. A handoff still in progress is in the panel, not here.

**Open** shows one entry: your request if you made one, the handoff *as it was stored*, *what
the agent was told*, the rounds, and *what left this machine*.

## Deleting

- **Delete** removes one entry with its rounds, notes and sends.
- **Delete everything** removes every finished handoff, round, note and send. Your settings
  are kept, and handoffs still in progress are not touched. This cannot be undone.

Both ask for confirmation in place: **Yes, delete**.

## Export

**Export JSON** writes the whole log to a file you choose: one JSON document with every table,
as it is in the database. Masked values stay masked. It is your data, in a form any program
can read.

## Reading the database yourself

Any SQLite tool can open `handoff.sqlite`. Open it read-only while Baton is running, and do
not edit it: Baton relies on what it wrote there.
