# Evidence settles applied changes

Status: accepted

## Context

Applied-but-unverified governed changes accumulated: four `brain tidy`
moves sat five days in "the file was written; not verified yet" while
the graph already held every fact `brain change verify` would have
established — the twin had observed the written state on every refresh,
and green test protocols had landed repeatedly since the apply. The
cockpit dutifully asked a person to run a command whose outcome the
machine could derive from its own records.

A run-button in eyes was designed and rejected first: it would have made
the person faster at doing the machine's bookkeeping instead of removing
the bookkeeping. The distinction that emerged: commands split into
*judgments* (approve this write, this document still holds, this plan is
finished — irreducibly someone's call) and *mechanical follow-through*
(verify once the evidence exists). Only the first family belongs on a
person's desk.

## Decision

`govern::reconcile_applied` closes the reflex arc, called from every
`twin refresh` (and therefore from every commit through the hooks):

- The judgment mirrors `verify` exactly: the newest recorded test run
  after the apply decides, and is linked as `verified_by`. Green stamps
  `verified`; red stamps `broken`, which the coherence pass already
  surfaces loudly. A run of nothing vouches for nothing.
- One guard `verify` never needed: a content change settles itself only
  while its file still carries exactly what the change wrote. A target
  someone hand-edited afterwards stays a person's call. A move has no
  single hash to hold, so run evidence alone decides — as it would
  under `verify`.
- Reflex stamps carry source `"reflex"`: the audit trail says the
  machine decided, and through which run. Nothing is re-executed.

In the same spirit, `brain sessions import` joined the post-commit hook,
so the workforce picture (live, collision, stall signals) is at most one
commit stale without anyone importing by hand.

## Consequences

- "Needs you" holds judgments only; the mechanical family drains itself.
- A red suite after an apply turns the change `broken` automatically —
  louder than the old limbo, which is the honest direction.
- Implemented in `crates/brain-observe/src/govern.rs`, wired in
  `crates/brain-observe/src/twin.rs`; the hook side in
  `crates/brain-cli/src/hooks.rs`.
