# Relation's edge label is a field named `predicate`, not `kind`

Status: accepted

## Context

`Object` serializes with `#[serde(tag = "kind")]`: the enum variant name
occupies the `kind` key in every object's canonical JSON. The first draft of
`Relation` also named its edge-label field `kind` (file `contains` symbol,
file `imports` file), which collides with the tag — serde produces two `kind`
keys and canonicalization becomes ambiguous, which is fatal in a system where
identity *is* the canonical bytes (crates/brain-core/src/canonical.rs).

## Decision

The edge label lives in a field named `predicate`
(crates/brain-core/src/object.rs). The subject–predicate–object reading is
also the honest one: a Relation is a typed triple, and `predicate` names the
role precisely where `kind` would have overloaded one word for two meanings.

## Consequences

- Query APIs still say "kind" at the trait boundary
  (`relations_from(from, kind)` in crates/brain-index/src/lib.rs) — the
  string vocabulary (`contains`, `imports`, `mentions`, `concerns`,
  `supersedes`, `recorded_in`) is shared either way.
- Anything constructing Relations by hand must remember the field is
  `predicate`; the serde tag owns `kind` forever.
