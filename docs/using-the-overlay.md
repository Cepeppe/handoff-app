# Using the overlay

## The panel

Baton is a narrow panel that stays on top of your other windows. Drag it by its header;
Baton remembers where you put it on each monitor. Closing it hides it to the tray — to quit
Baton, use **Quit** in the tray icon's menu.

The tray icon is always there. Its menu has **Show**, **New request**, **Settings** and
**Quit**. A dot on the icon means handoffs are open, and the tooltip says how many.

When an agent opens the first handoff, the panel comes forward by itself. Later ones add a
tab and a badge (*2 new*) without taking the focus.

## Tabs

Each handoff is a tab, labelled with the agent and the project folder. Handoffs that wait for
you — parked ones, and outcomes nobody collected — sit in a **Waiting** group you can fold.

## A step

A handoff is a list of steps, shown one at a time: *Step 2 of 4*, or *Correction · 1 of 2*
when the agent sent corrected steps after a failed check.

- **The text** of the step, as the agent wrote it, and a warning above it when the agent gave
  one.
- **Values** you will need, each with **Copy** (or **Copy this one** for an item of a list).
- **Open** for the page the step is about. It opens in your browser.
- **Secrets** you will create in this step are listed under *Paste yourself, we never see it*,
  with **Open file** for the file they belong in. Baton never sees them: you paste them there
  yourself.
- **Masked values**: a value the agent passed that looks like a secret is shown as `••••••`.
  **Copy** copies the real value; **Show** reveals it for ten seconds.

## The buttons

| Button | What it does |
|---|---|
| **Done** | the step is done; after the last one, the round is finished and the agent is told |
| **Ask** | asks the agent a question about this step; the reply appears on the same step |
| **Screenshot** | shows the agent what you see, after a mandatory preview (see [Screenshots and privacy](screenshots-and-privacy.md)) |
| **Note** | a note for yourself on this step; the notes are reported to the agent at the end |
| **Skip** | skips the step; the agent is told which steps were skipped |
| **Defer** | you will come back to it later; the agent is told, and resumes when you are ready |
| **Abandon** | stops the handoff; you can say why |

What you type in **Ask**, **Defer** and **Abandon** is shown under *What the agent will read*
before you send it. A secret Baton recognises for certain is taken out; words that only *may*
be a secret are marked, and sent as you wrote them — edit them if they should not go.

A handoff deferred twice is **parked**: it waits in the Waiting group until you press
**Resume**.

## The collapsed bar

When you click outside the panel it shrinks to one line: the current step and **Done**,
**Ask**, **Screenshot**. Click it to open the panel again. If it stays open when it should
not, Settings → General → Panel can also shrink it a few seconds after your last click.

## What the banners mean

| Banner | Meaning |
|---|---|
| Waiting for the agent's spec | you opened a request and the agent has not answered with the steps yet |
| The agent will pick up on its next resume | the agent is not waiting on this handoff right now; what you do is kept and delivered when it comes back |
| Sent to the agent, waiting for the reply | your question or screenshot is with the agent |
| Deferred; the agent will come back | you deferred it |
| Parked; resume when you want | you deferred it twice |
| The agent should now check: … | you finished the steps and the agent is checking the result |
| Session detached; the outcome will be delivered on the next resume | the agent's session ended; whatever you do is delivered to the next session that resumes this handoff |

## How a handoff ends

When you press **Done** on the last step, the agent is told. If the handoff says how to check
the result, the agent checks it and the tab shows what it reported, labelled *declared by
agent*:

- **Verified** — the agent checked and it worked.
- **Failed** — it did not work. The agent can send corrected steps: a new round,
  *Correction 1 of 2*.
- **Not verified** — no report arrived, either in time or before the agent's session ended;
  the tab says which.
- **Confirmed by you** — there was nothing for the agent to check.
- **Abandoned** — you stopped it.

A finished outcome no agent collected for seven days is shown as *Nobody collected this
outcome.*, with **Close it** and **Copy the id**.

## Asking for a handoff yourself

When you are about to do something the agent should guide, press `Ctrl+Alt+H` anywhere (or
**New request** in the tray menu). Baton asks *What are you about to do?* and which session
it is for. `Enter` sends, `Esc` cancels.

Baton then puts a line for the agent on the clipboard and tries to bring the agent's terminal
to the front, so you can paste it. When it cannot — some terminals do not let it — a
notification says *Request copied: paste it into session …*. If you do not paste it, the agent
is reminded at the end of its turn. With no session running, the request waits for the first
one that starts.

When the agent answers, the handoff takes the place of your request. If it was linked to the
wrong request, **Change** lets you pick the right one.

If another program already uses `Ctrl+Alt+H`, Baton leaves it alone and asks you once for
another combination. You can change it at any time in Settings → General → Shortcut.

## Which session is this?

If Baton cannot tell which of two sessions working in the same folder a hook came from, a tab
asks *Which session is this?*. Pick the one you are working in, or *I do not know*.

## When Baton is not running

The agent can still hand work over: the MCP server answers with the handoff as text, and the
agent guides you **in the chat**. That is text mode. Nothing is shown in a panel, nothing is
logged, there is no *verified* state and nothing to resume later — the chat is the record.

Start Baton and the next handoff uses the panel again. A running session finds Baton within
about 30 seconds; you do not need to restart it.

## Language

Baton follows the language of Windows if it is English or Italian, and uses English
otherwise. Settings → General → Language changes it.
