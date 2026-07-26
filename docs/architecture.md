# Architecture

## Position

This project builds a **governed agent execution fabric** whose founding
assumption is that programs live natively in a graph (no files above the
semantic line). It treats the richer "living cognitive substrate" as a
possible evolution of this fabric, to be earned through evidence, not assumed.

Two modes share one schema and one identity scheme:

- **Native mode** — software *is* the graph: `Code` objects (terms of the core
  calculus), executed by the interpreter.
- **Reflective mode (the twin)** — the graph *describes* external software:
  `Entity` + `Observation` nodes ingested by observers. Observations are
  time-bound and sourced; stale ≠ false.

Shared identity is the migration path: a twinned entity that later gains a
native implementation is the *same node* acquiring a new edge. Migration is a
gradient — describe → observe → govern → absorb — not an event.

## The six founding components

1. **Core calculus** (`brain-core::object::Term`) — 12 operations, no ambient
   authority, typed holes, references by content hash.
2. **Canonical encoding + CAS** (`brain-core::canonical`, `brain-store`) —
   identical meaning ⇒ identical bytes ⇒ identical `NodeId`. The store
   persists exactly the canonical bytes and verifies them on read.
3. **Namespace layer** (`brain-store`) — the graph as a codebase: name→hash
   maps as immutable `Namespace` objects chained by `parent`. Version control,
   branching and history are this chain; nothing is ever edited in place.
4. **Effect boundary** (`brain-store::intents`, `brain-runtime::EffectPort`) —
   durable intent before, receipt after, indeterminate on crash, reconcile
   before retry.
5. **Observer frame** (`brain-observe`) — reflective-mode ingestion as a
   continuous sense organ.
6. **Replication** — not yet built. Content-addressed sync between stores is
   how code moves; it replaces deployment.

## Authority model

- Possibility is never permission: the existence of a foreign symbol does not
  allow calling it. Effectful symbols declare a required capability; the
  evaluation context carries the granted set; the check happens before the
  boundary is touched.
- The interpreter is the only path from terms to effects, and
  `EffectPort` is the only path from the interpreter to the world. A
  `DenyEffects` port implements the simulation posture: capability or not,
  a simulation branch cannot reach production reality.
- Verification levels (`VerificationLevel` on `Evidence`) are recorded data.
  The intended rule — not yet enforced — is that authority is capped by the
  strongest verification level actually available for a node: unverified code
  runs sandbox-only; behavioral evidence unlocks scoped effects; formal
  evidence is required for irreversible ones.

## Crash safety

The ordering contract is the whole design:

```
put(Intent) → intents.begin(id)  [fsync]  →  effect runs  →  put(Receipt) → confirm/fail
```

A crash between `begin` and `confirm` leaves the intent Pending; `recover()`
relabels it Indeterminate and returns it for reconciliation. Recovery marks —
it never re-executes. Retrying an externally ambiguous, non-idempotent action
is forbidden by protocol, not by convention.

## What is deliberately absent (and why)

- **Type checking** — types are opaque strings on `Spec` for now; the calculus
  is designed so a checker can be added without changing object identity.
- **Alpha-equivalence in hashing** — `\x.x` and `\y.y` hash differently.
  Known limitation; fixing it means canonicalizing binders (de Bruijn) in the
  encoding, which should happen before any large corpus of code accumulates.
- **Replication, branch worlds, learning lineage, agent-to-agent transfer** —
  roadmap items; see `docs/roadmap.md`.
- **A files export** — intentionally not built into the core loop. Projection
  to files may exist for legacy interop, but the moment internal tooling
  depends on exported files, files are authoritative again through the back
  door.
