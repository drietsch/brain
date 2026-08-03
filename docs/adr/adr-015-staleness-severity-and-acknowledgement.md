# Staleness carries severity and can be acknowledged

Status: accepted

## Context

The staleness signal had saturated to noise: ten of twelve ADRs plus
CLAUDE.md flagged simultaneously, because every doc kind rotted identically
(binary, forever, no way out but re-touching the file) and finished plans
rotted loudest of all. A signal that flags everything discriminates nothing,
and agents learned to ignore it.

## Decision

Three changes, all query-time (crates/brain-observe/src/twin.rs):

1. **Only active documents rot.** Lifecycle (ADR-013) gates the check: a
   done plan or superseded ADR is a record, and records do not rot.
2. **Severity is a per-kind rot policy, stored as graph data**: a `rot`
   observation on the kind's template (`none | info | warn`, set via
   `brain template set <slug> --rot`), with code defaults — decisions and
   plans are records once written (`info`); skills, agent configuration,
   and taught kinds describe the present (`warn`). Hooks and wake nag only
   on warn; info stays visible without shouting.
3. **Acknowledgement resets the clock without touching the file**:
   `brain adr|plan|artifact ack` writes a `reviewed` observation whose
   timestamp becomes the doc's effective time
   (`max(content, reviewed)`). "I checked it against current code and it
   is still accurate" is now a recordable fact, deliberately unguarded —
   re-acknowledging is the point.

Only live `mentions` count (ADR-014), so a doc rewritten to drop a file
stops being invalidated by it.

## Consequences

- `brain twin stale` output is short enough to act on: warn first, info
  labeled, with the ack verbs named in the footer.
- Weighting "specifies a file" vs "names it in passing" is deferred; the
  ack path covers the gap at the cost of one explicit judgment.
- A kind can opt out entirely (`--rot none`) — right for changelog-like
  artifacts whose whole nature is being a snapshot.
- The severity split is read where the brain orients: `crates/brain-observe/src/wake.rs` reports warn-level rot on every wake.
