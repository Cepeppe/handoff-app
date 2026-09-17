# Contributing to Baton

Thank you for taking the time. Baton has a single maintainer and is built from a written design
([`docs/design/`](docs/design/README.md)), so a few rules keep contributions and the planned work
out of each other's way.

## Before you start

- **Small fixes go straight to a pull request**: a typo, a broken link, a wrong sentence in the
  documentation, or a bug fix together with a test that fails without it.
- **Anything larger starts with an issue**: a new feature, a change of behaviour, a new
  dependency, or a change to what Baton sends, stores or shows. Describe the problem before the
  solution and wait for an answer before writing the code: the change may conflict with the
  design or with work already planned ([`docs/design/tasks.md`](docs/design/tasks.md)).
- **A vulnerability is never a public issue.** Report it privately, as
  [`SECURITY.md`](SECURITY.md) explains.
- **The handoff formats, the tool contract and the MCP server** live in
  [`handoff-mcp`](https://github.com/Cepeppe/handoff-mcp): changes to them start there.

## Reporting a bug

Say what you did, what you expected and what happened, with the versions of Baton, of Windows and
of the agent you used. [`docs/troubleshooting.md`](docs/troubleshooting.md) covers the common
cases. Never attach a real credential, or a screenshot or a log that shows one: use a synthetic
value.

## Setting up

Baton is built and tested on Windows; macOS is deferred (implementation decision 7). The
prerequisites, the first run and the checks are in the [Development](README.md#development)
section of the README. This repository is all you need: the server comes from its pinned release.

## Pull requests

- **One change per pull request**, with its tests.
- **Run the checks** of the README before opening it; CI runs them again on every pull request
  that changes code. After adding or bumping a Rust dependency, run `pnpm notices`.
- **The end-to-end suite** runs by hand against a real agent ([`docs/dev/e2e.md`](docs/dev/e2e.md)).
  When the change touches the channel, the store, the state machine, the hook decision, the
  capture pipeline or the tool contract, say in the pull request whether you ran it.
- **The user documentation is in English and in Italian** (`docs/` and `docs/it/`), and the tests
  check that every page exists in both. Change both; if you do not write Italian, say so and the
  maintainer will translate.
- **Add a line to `CHANGELOG.md`**, under `[Unreleased]`, for a change a user would notice.
- **Commit messages** follow the history: `type(scope): summary`, with `feat`, `fix`, `docs`,
  `test`, `refactor`, `ci` or `chore`, a lowercase summary, and a body that says why.
- **Tests use synthetic secrets only.** A key that has ever been valid anywhere does not belong in
  the repository, not even once revoked.

Comments and documents cite the design as `§5.8`, `SPEC-05` or `T-054`;
[`docs/design/README.md`](docs/design/README.md) explains those citations. A contribution does not
have to add any.

## Licence

Baton is released under the [MIT licence](LICENSE). By opening a pull request you agree that your
contribution is released under the same licence, and you confirm that you have the right to
contribute it.
