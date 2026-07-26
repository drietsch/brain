# Alpha-normalization happens at the store boundary

Status: accepted

## Context

Two programs that differ only in bound-variable names mean the same thing.
If they hash differently, the graph stores duplicate nodes, evidence caching
misses re-authored solutions, and "identity before names" is a slogan rather
than a property. Normalizing inside every producer (authoring tools, task
checkers, sync) would scatter the invariant across the codebase.

## Decision

`Store::put` canonicalizes every object on the way in
(crates/brain-store/src/lib.rs calls `brain_core::object::canonicalize`),
which alpha-normalizes Code terms: binders are rewritten to de Bruijn-level
names (`_0`, `_1`, ...) in crates/brain-core/src/object.rs. No object enters
the graph un-normalized, so alpha-equivalent programs share one NodeId and
stored bytes always re-hash to their id.

## Consequences

- Evidence keyed by code hash transfers across encodings and variable
  names: a solution re-authored in compact notation hits the check cache
  (crates/brain-cli/src/tasks.rs).
- Objects written before this rule predate the current canonical form; sync
  detects them with a distinct `CanonEpoch` error rather than misreporting
  corruption (crates/brain-store/src/sync.rs).
- The original surface names are projection-layer concerns; the graph never
  sees them.
