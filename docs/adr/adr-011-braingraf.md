# braingraf: our own persistent graph-query engine

Status: accepted

## Context

Evaluating minigraf (an embedded bi-temporal Datalog database) confirmed
two things at once: an embedded graph engine belongs at the Index seam —
and the record layer must not move. Records stay content-addressed,
integrity-checked, replicated by set union; minigraf's 4 KB fact cap
alone disqualifies it there, and its youth (single maintainer) makes it a
risky dependency anywhere. We take its lessons and implement our own.

## Decision

`crates/braingraf`: learn from minigraf, then simplify with one
observation — **brain already has a WAL** (the store's event log), so
braingraf needs no write path. It is:

- a **checkpoint** (`.brain/index.graf`, JSON snapshot of MemIndex state
  plus an event-log cursor) written temp+rename;
- **delta replay** on open: only events after the cursor are folded in —
  warm start is O(new events), measured 15x faster than cold replay on
  this repo's 3,780-object store;
- **recursive traversal** (`Graf::reach`): BFS over relation edges, both
  directions, cycle-safe — transitive imports and true blast radius
  (`brain twin rdeps --transitive`), which a flat index cannot express;
- **bi-temporal reads** in the twin (`latest_at_before`, `files_at`,
  `brain twin at <prefix> <when>` where `<when>` may be a git commit
  hash resolved through the repo's observation timeline).

Disposability is the contract: corrupt, missing, or version-mismatched
checkpoints mean a silent cold rebuild, never an error. The file is local
and never replicates — truth travels as objects; indexes are grown where
they are needed. `Graf` derefs to `MemIndex`, so the reference backend
remains the semantics; `BRAIN_INDEX=mem` forces reference behavior, and
`brain bench index` verifies identical answers before reporting timings —
the earn-adoption gate, kept honest.

## Consequences

- CLI startup stops scaling with graph size; the event log stays the
  single write path.
- Blast-radius and as-of queries become one-liners; deeper Datalog-style
  rules can grow inside braingraf later without touching the store.
- Known limit inherited from the extractors: cross-crate Rust imports
  resolve to module entities, not files, so transitive walks are
  strongest within a crate.
