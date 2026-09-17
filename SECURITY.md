# Security policy

Baton exists to keep secrets on the machine: a screenshot is redacted before anything can be
sent, nothing leaves without the preview, and the application's own code makes no network
connection ([`docs/verify-trust.md`](docs/verify-trust.md)). A way around any of that is a
vulnerability even when nothing crashes: a secret that reaches an agent unmasked, a redaction
that can be lifted where it should not, a connection nobody asked for, or a local process that
talks to the channel without its token.

## Reporting a vulnerability

Please report it privately, with **Report a vulnerability** on the Security tab of this
repository, and not in a public issue. Say what you did, what you expected and what happened.
A spec, a screenshot or a test that reproduces it helps most; if it involves a secret, use a
synthetic one, never a real credential.

## Supported versions

Nothing is stable yet: fixes go into the next release, and earlier releases are not patched.
