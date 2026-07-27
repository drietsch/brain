# Template fitness: contracts are measured, evolution is approved

Status: accepted

## Context

Templates were set once and never questioned. Whether a contract actually
worked — did agents produce conforming artifacts on the first try? did
plans written against it get finished or abandoned? — was invisible, so
the definitions could not improve across brain generations.

## Decision

A template version is its `contract_b3` — the hash of `requires` plus
scaffold, restamped whenever either changes — and every judged artifact
records the `template_b3` that judged it (ADR-017). Fitness
(crates/brain-observe/src/fitness.rs) is then a pure query, integer
arithmetic, never persisted (ADR-009): per version, first-capture
conformance (the *earliest* `conforms` observation — did the agent get it
right before anyone corrected it?), missing-field frequency, artifact
outcomes from lifecycle (done, abandoned, superseded, active), and
current staleness. Deterministic thresholds turn the numbers into
verdicts: a field missed in half of first captures is a demotion
candidate; one almost always present suggests `--enforce enforced`; a
kind whose artifacts are mostly abandoned has a scope problem.

`brain template evolve <slug>` renders the proposal — demote
chronically-missed fields from `requires` to `recommended` — and only
`--apply` writes it, bumping `contract_b3` and opening the next
measurement window. **No auto-mutation**: the brain learns, a person or
agent decides. Old artifacts keep the version that judged them, so
versions stay comparable forever.

## Consequences

- "Does this template work?" is one command with an evidence-backed
  answer, and the evidence accumulates from day one (`template_b3` is
  stamped at every capture).
- Contract changes are themselves observable history: fitness compares
  the before and after of every evolution.
- The learning loop closes the last gap in the artifact story:
  placement (ADR-018) says where, the projection contract (ADR-019) says
  how, gates (ADR-020) say how strictly, tidy (ADR-021) says until when
  — and fitness says whether any of it is working.
