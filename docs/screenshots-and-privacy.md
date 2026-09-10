# Screenshots and privacy

A screenshot is the one thing Baton sends that you did not type, so it is where Baton is most
careful. Nothing is ever captured unless you press **Screenshot**, and nothing leaves until
you have seen exactly what will.

## Capture

**Screenshot** offers two choices, every time:

- **Full screen** — the monitor under your mouse pointer.
- **Select region** — drag a rectangle, across monitors if you need; `Esc` cancels.

The last choice you used is highlighted, but Baton never picks for you. The panel hides
itself while the screen is captured, so it is not in the picture. Between captures nothing
watches your screen.

## The preview

The picture appears at once, while Baton reads it: *Reading the capture…*. The two send
buttons stay disabled until that is done.

### Reading the text, on this computer

To find secrets, Baton reads the text in the picture — locally, and offline:

- with **Windows' own text recognition** when Windows has it for the language (an *optical
  character recognition* feature comes with a language pack; English is usually there);
- otherwise with **ocrs**, a text-recognition engine bundled with Baton, which reads English
  best.

The preview says which one read it (*Read by windows*, *Read by ocrs*). No text and no picture
is sent anywhere to be read.

### What is hidden

| Box | Why | Can you lift it? |
|---|---|---|
| solid, red | a **certain** secret: text that matches a public pattern of keys, tokens, private keys, webhook secrets and signed tokens | no |
| dashed, amber, *May contain a secret* | a **suspected** secret: a long random-looking string, a long hexadecimal or base64-looking string, or a value beside a word like *key*, *secret*, *token*, *password* | yes: **Show this again**, and **Hide this again** |

A box covers the whole line of text the secret was found in. A value the handoff itself gave
you, which is not a secret, is not flagged.

You can also **Hide an area** by dragging over it, and **Crop** the picture (**Undo the
crop** puts it back).

## Send image or send text

The two buttons sit side by side; neither is the default.

- **Send image** sends the picture. It is reduced so its longer side is at most 1600 pixels,
  and the hidden boxes are painted solid black *after* the reduction, so no trace of the text
  under them survives. The button is not there when the agent cannot read images.
- **Send text** sends the text Baton read, in a pane you can edit. Under the pane you see it
  *as it will be sent*: certain secrets replaced by `[REDACTED:<kind>]`, suspected ones by
  `[REDACTED:suspected]` unless you showed them again.

For a large screen Baton suggests text: *This screen is large. Sending it as text is often
clearer for the agent and cheaper for the session.* A big picture costs the agent's session
more, and some agents cut a large image short.

The optional comment (*Anything to say about it*) is checked like everything you type: a
certain secret is taken out, a suspected one is marked and sent as you wrote it.

If the capture could not be read at all, the preview says so — *nothing was hidden
automatically* — and leaves the decision to you: hide what you need by hand, then send the
picture, or discard it. **Send text** is not offered, because there is no text.

## Where it goes

What you send goes to your agent, through the MCP server running on this computer. Your agent
then sends it to its model provider as part of the conversation, exactly like anything you
type into the agent yourself. Baton itself sends it nowhere else.

The capture is kept in memory only until you send or discard it. Baton never writes the
picture to disk.

## What the log keeps

For every send, the [local log](log-and-export.md) keeps:

- for text, the text **exactly as it was sent**;
- for an image, its size, its SHA-256 hash and the rectangles that were hidden — **never the
  pixels**.

## Secrets in a handoff

Separately from screenshots, the MCP server checks every handoff for text that matches the
same certain patterns before Baton shows it. Such a value is shown masked (`••••••`): **Copy**
copies the real value, **Show** reveals it for ten seconds, and the log and the runbooks keep
only a mask such as `[treated as secret: api_key]`.

Your **notes** are yours: they are kept as you typed them, and a certain secret in one is
replaced when the notes are reported to the agent at the end.
