# The artifact-kind registry: built-ins are pre-taught defaults

Status: accepted

## Context

Every agent session produced artifacts in slightly different shapes, and
the brain's own knowledge of artifact kinds was split three ways:
hardcoded path conventions (docs.rs, agents.rs), graph-taught capture
rules (ADR-008), and prose conventions nobody enforced. The most-read
documents — README.md, docs/architecture.md, the runbooks — belonged to
no kind at all: no entity, no mentions, no staleness. ADR-008's promise
("built-in detectors are merely pre-taught defaults") was aspirational.

## Decision

One merged registry (crates/brain-observe/src/kinds.rs): compiled
defaults from `templates::DEFAULTS` form the base layer; every graph
template entity overlays its observed properties per property, graph
winning. A kind's record — schema (`requires`), scaffold, capture globs,
field extraction, `placement`, `home`, `project_to`, `enforce`, `rot`,
`parser`, `links`, `extensions`, `contract_b3` — is its template entity,
extended, never a parallel structure. The shipped defaults now include
`doc` (README.md, docs/*.md), `runbook`, `task-list`, `capability-matrix`,
`asset`, and `prototype`, so a store that never re-seeded still knows
them, and `brain template set` overrides any property at runtime.

Capture routes by `parser`: decision/plan and agent documents keep their
richer code parsers (identity preserved by construction — sids derive
from kind + prefix + slug exactly as before); everything else goes
through the fields DSL, most-specific glob winning. Ingestion is
rule-gated: a kind's `extensions` reach only where its globs reach, and
repo-level `ingest_extensions` (via `brain twin config`) is the explicit
everywhere-opt-in — both size-capped.

Every judged artifact records `template_b3`, the contract version that
judged it (fitness's data feed, ADR-022). Seeding follows the upgrade
rule: a property is rewritten when its latest value was itself seeded and
the shipped default changed; a store-local edit is never superseded.

## Consequences

- The narrative docs finally rot visibly: captured, mentions-scanned,
  warn-severity by default.
- Teaching a kind is still two observations; the taught kind now carries
  placement, enforcement, and rot policy too.
- The registry is the single source for capture, tidy, instructions
  generation, and the authoring gate — one definition, every consumer.
