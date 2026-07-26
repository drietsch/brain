# Governed mode: changes to twinned software go through the effect boundary

Status: accepted

## Context

The adoption gradient is describe → observe → govern → absorb. The twin
covered the first two; the intent/receipt machinery (crates/brain-store/
src/intents.rs) had guarded native-mode effects since the first commit
but never mediated changes to the external software brain observes. An
agent editing files directly leaves no reason, no before-state, no
crash-safe record — exactly the gap governed mode exists to close.

## Decision

`brain change` (crates/brain-observe/src/govern.rs) — the motor system:

- **propose**: a pure graph write. The change entity records target,
  reason, full before/after content and hashes, and `changes` /
  `concerns` relations. The working tree is untouched.
- **apply**: the effect boundary, in order — durable Intent BEFORE the
  write (fsynced to intents.jsonl), the write (temp + rename), then the
  Receipt. Refused without `--cap fs`: no ambient authority, identical
  to runtime effects. Failure produces a failed receipt and status, never
  a torn file.
- **verify**: runs the repo's graph-configured test command, imports the
  protocol, links it `verified_by`, grades the change verified/broken.
- **revert**: a governed write of the recorded before-state (or governed
  removal when the change created the file).
- **crash honesty**: a crash between intent and receipt leaves the intent
  pending; `brain recover` marks it indeterminate and reconciles the
  change's status. Marked, never retried — reconciliation is a deliberate
  act.

## Consequences

- Every governed mutation carries its why (reason), its what (before and
  after, content-addressed), its authority (capability), its receipt, and
  its evidence (linked protocol) — the full provenance a file edit
  normally destroys.
- The twin sees applied changes as ordinary drift on the next refresh, so
  observation and governance compose rather than compete.
- Enforcement is now possible but still opt-in: nothing forces changes
  through the boundary; agents that want provenance use it. Absorb — the
  final gradient step — remains future work.
