# Design documents

The documents Baton and [`handoff-mcp`](https://github.com/Cepeppe/handoff-mcp) were built from.
Comments in the code of both repositories cite them throughout; this page says how to read those
citations.

| Document | What it is | Cited as |
|---|---|---|
| [REQUIREMENTS.md](REQUIREMENTS.md) | What the system must do | `SPEC-05`, `DET-04`, `OCR-03`, `NFR-17`: an area and a number |
| [TECHNICAL-DESIGN.md](TECHNICAL-DESIGN.md) | How it is built | a section, `§7.10`; `DD-nn` design decisions, `A-nn` assumptions, `F-nn` flows, `FM-nn` failure modes, `R-nn` risks, `OI-nn` open issues, `E2E-n` scenarios |
| [implementation-decisions.md](implementation-decisions.md) | The decisions taken while building it that depart from the two documents above | "implementation decision 7" |
| [tasks.md](tasks.md) | The numbered sequence of tasks the implementation was carried out in | `T-054`, in comments and in commit messages |

A bare section number in a comment of either repository (`§3.5`) is a section of
TECHNICAL-DESIGN.md unless the comment names another document.

The two documents are published as they were written for the implementation, dated 2026-09-07
and amended since, with two editorial changes: their licensing statements reflect the decision of
2026-09-17 to publish both repositories under the MIT licence, and the product vision and the
decision record they cite (`IDEA.md`, `DESIGN-TREE.md`) are working notes of the maintainer that
are not published.
