# Runbooks

A runbook is a recipe: the steps that worked the last time a piece of human work was done, so
that the next time the agent can start from them instead of from nothing.

## When Baton writes one

After every handoff that ends **Verified**, and after every one that ends **Confirmed by you**
(with a lower trust label). Never after one that failed, was not verified, or was abandoned.

The recipe is what you actually did: the steps you confirmed, in order, including the
corrected steps of a later round, without the ones you skipped. Your notes, and the errors
that led to a correction, are kept as remarks on the step they belong to.

## Where they are

In `%USERPROFILE%\.handoff\runbooks\`, one JSON file per runbook, in a public format
documented with the open MCP server
([runbook format](https://github.com/Cepeppe/handoff-mcp/blob/main/docs/runbook-format.md)).
You can open them, read them and copy them. They survive uninstalling Baton.

## No values, only their names

A runbook never contains the values of a handoff — no URLs of your project, no keys. Where a
step used a value, the runbook keeps its name as a placeholder, `{{endpoint_url}}`, with the
sentence of the step it appeared in as its description. A secret found anywhere else in the
text is replaced by a mask such as `[treated as secret: api_key]`.

## How agents use them

Before writing a new handoff, the agent asks the MCP server for runbooks about the same place
and the same goal. The server also checks by itself when a new handoff arrives: if a runbook
matches, it offers it to the agent as a draft to start from, and the agent fills in the values
from your project. A runbook matches when the place is the same and the two goals share
meaningful words — a plain rule, no guessing. The agent also sees when the runbook was last
verified, to judge how fresh it is.

## When a runbook's handoff fails

A handoff that started from a runbook and fails marks the runbook *Last run failed* with the
date. Baton never deletes a runbook by itself.

If the agent then corrects the steps and the correction works, Baton asks: *Update runbook …
with the corrected sequence?* — **Update it** replaces the steps, **Keep both** leaves the
runbook as it was.

## Settings → Runbooks

The list shows every runbook with its runs, its steps, when it was last verified, when a run
last failed, and its trust: **Verified** or **Confirmed by you**.

- **Open the folder** opens `%USERPROFILE%\.handoff\runbooks\`.
- **Delete** moves a runbook to the Recycle Bin, after asking.

There is no export, import or shared library: a runbook is a file, and copying the file is
enough.
