# Tidy acts only through governed changes

Status: accepted

## Context

Artifacts and their files outlive their purpose: projections of concluded
plans, assets of retired features, prototype directories nobody will
revisit, documents no kind claims. A cleaner that silently deletes or
moves files would be a second, unaccountable mutation path — exactly what
governed mode (ADR-010) exists to prevent.

## Decision

`brain tidy` (crates/brain-observe/src/tidy.rs) is an advisory scan
first: hand-edited/stale/missing projections, writable projections,
retired artifacts' files, misplaced artifacts (outside their kind's
`home`), legacy assets (owner concluded), concluded prototypes, untyped
documents (with a ready-to-run teaching command), and stale instruction
blocks. One line per finding, fix named.

`--fix` applies the safe set: chmod re-arming, re-renders from graph
truth (hand-edits rescued into the artifact's timeline first, ADR-019),
instruction regeneration, and archival **moves to docs/attic/ as
governed changes** — `govern::propose_move` + `apply`, so every move is
a `change` entity with intent, receipt, and a revert path
(`brain change revert` renames it back). Moves refuse when the path has
uncommitted git changes and everything content-touching requires
`--cap fs`. The graph entities of archived artifacts stay untouched:
the history remains queryable after the files are gone.

Deletion is never chosen by the scan. `--rm <path>` is the explicit act,
still intent/receipt-logged.

## Consequences

- `brain change list` doubles as the tidy audit log.
- Repos stop accumulating dead artifacts without anyone losing work:
  the attic holds the bytes, the graph holds the meaning.
- The safe/unsafe boundary is legible: re-render and move are safe
  because the graph can always reproduce or revert them; deletion and
  edits to hand-written content never happen implicitly.
- Tidy moves through `crates/brain-observe/src/govern.rs`, over the intent/receipt boundary in `crates/brain-store/src/intents.rs`.
