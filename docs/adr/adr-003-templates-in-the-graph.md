# Deliverable templates live in the graph

Status: accepted

## Context

What an ADR, a plan, a skill, or a "done" feature must contain was implicit:
hardcoded parsing conventions in crates/brain-observe/src/docs.rs and tribal
knowledge in prompt files. A working contract that lives outside the graph
cannot version, replicate, or be queried — the exact failure mode the
substrate exists to remove.

## Decision

Templates are `template` entities seeded under `brain/templates/`
(crates/brain-observe/src/templates.rs): a `content` scaffold, a `requires`
list of machine-checkable fields, and the entity kind each `applies_to`.
Captured documents get recorded — never enforced — conformance observations
against their template. The definition of done is the `feature` template's
`requires` list, whose fields are relation predicates evaluated as graph
queries (crates/brain-observe/src/features.rs); the feature matrix is a
rendered query, not a spreadsheet.

## Consequences

- The contract replicates with `brain pull` and evolves per store; local
  edits to `requires` win over shipped defaults and survive re-seeding.
- "Done" is checkable graph state; done flips are observations, so a
  feature regressing out of done-ness is a recorded event.
- Reflective mode stays descriptive: nonconforming documents surface in
  insights but are still captured. Enforcement is deferred to the governed
  mode, where brain mediates the change itself.
