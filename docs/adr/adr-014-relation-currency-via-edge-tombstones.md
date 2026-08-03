# Relation currency via edge tombstones

Status: accepted

## Context

Relations were add-only: a deleted symbol kept its `contains` edge, a removed
import kept inflating hub counts and `rdeps` blast radius, and a doc
rewritten to drop a file kept the `mentions` edge that made it eternally
stale. Structure-shaped queries reported history as if it were the present,
and the error grew monotonically with every refactor. Objects are immutable,
so the fix cannot be deletion.

## Decision

An edge's currency is an observation timeline on a derived edge sid:
`StableId::derive(["edge", from, predicate, to])`
(crates/brain-index/src/lib.rs). Retraction writes `active=false`;
re-observation writes `active=true`; **absence of any `active` observation
means live**, so stores that predate tombstones need no migration. Writes
are transition-guarded — retracting a dead edge writes nothing.

Refresh sweeps retractions automatically (crates/brain-observe/src/twin.rs):
structure a pass no longer observes (`contains`/`imports`/`covers`),
mentions whose paths a re-recorded doc no longer names, `recorded_in` of a
former location, and every outgoing edge of a deleted file — including
files deleted before tombstones existed, which is the self-healing path.
A mention of a deleted file the text still names stays live: that mismatch
belongs to coherence, not currency. `brain relation retract` is the
agent-facing escape hatch; `relation list --all` shows retracted history.

Every reader — insights, attention, features::evaluate, assoc,
`cortex::reach`, the CLI's relation views — consumes live edges only.

Rejected: epoch-stamping every edge per refresh (writes one observation per
edge per run, destroying "refresh twice writes zero objects") and deriving
current relations from file content at query time (content is not stored,
only its hash).

## Consequences

- Hub counts, blast radius, `symbols_total`, and doc staleness track what
  the code is, not what it ever was.
- Relation objects are never destroyed: bi-temporal reads
  (`edge_active_at`) answer "did this edge hold at T?".
- Renames leave a `renamed_to` edge (same-run delete + add of identical
  bytes; backfill maps git's R status), so a moved file's identity trail
  survives where before it just leaked.
- The retraction sweeps live in `crates/brain-observe/src/twin.rs`; `edge_active_at` in `crates/brain-index/src/lib.rs`.
