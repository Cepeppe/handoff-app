# Crash reports

Baton sends nothing about you or your use of it to anyone — no telemetry, no usage
statistics, no automatic crash reports, not even ones you would have to opt into.

## If Baton crashes

It writes a text file on this computer, in `%APPDATA%\Baton\crashes\`, named after the moment
of the crash in UTC: `2026-09-10T08-14-05Z.txt`. The file holds:

- Baton's version, and the version and architecture of Windows;
- the error and where in Baton's code it happened, with the call stack;
- the last 50 lines of Baton's own log, **with identifiers only**: handoff ids, times, sizes,
  counts and status codes. Step texts, notes, values and secrets never reach this log, so they
  cannot end up in a crash file.

## The next time Baton starts

It tells you: *Baton crashed the last time it ran. The report is on this machine and nothing
has been sent anywhere: open the folder if you want to send it by hand.* **Open the folder**
opens `%APPDATA%\Baton\crashes\`; **Not now** closes the notice.

If you want to report the crash, read the file first — it is plain text — and attach it to
your message yourself.

## Removing them

The files are yours: delete them whenever you like. Uninstalling Baton with *Delete the
application data* ticked removes the folder (see [Install on Windows](install-windows.md)).
