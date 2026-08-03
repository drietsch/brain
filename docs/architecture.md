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
5. **Observer frame** (`brain-observe`) — reflective mode, the project's
   first deliverable: drift-aware `twin refresh`/`status`, per-language
   symbol and import extraction into `Relation` edges, agent notes as
   durable observations, and deletion-as-observation. See `docs/twin.md`.
6. **Replication** (`brain-store::sync`) — content-addressed sync between
   stores; how code moves, replacing deployment. Objects are a conflict-free
   set union (immutable + content-addressed; every ingest re-hashed, with a
   distinct canonicalization-epoch error when a source predates the current
   canonical form). Namespace conflicts are preserved as explicit structure:
   the destination's binding is kept and the source's target lands under
   `sync-conflict/<name>` — never silently overwritten. Operational state
   (the intent log) stays local; receipts and evidence travel as objects, so
   a program verified in one store arrives in another with its evidence —
   where the checker's cache then recognizes it as already-verified work.
   CLI: `brain pull|push <store-root>`; `BRAIN_STORE` selects the local store.

## The index seam: system of record vs. system of query

The CAS and event log are the only systems of record. Everything that makes
them queryable — reverse edges, subject lookups, eventually similarity search
— is a *derived* structure behind the `brain-index::Index` trait: disposable,
rebuilt by replaying `Store::put_history()`, and never a second source of
truth. Backends must be idempotent under replay. `MemIndex` is the naive
reference implementation and the baseline any embedded graph engine
(candidates: OverGraph, Graph_D) must beat on real workloads before it earns
adoption; because nothing authoritative lives in an index, backend risk is
contained to performance, never correctness. Edge semantics are defined once,
in `object_edges` — every backend shares them.

The same bargain is struck one level down, for bytes rather than answers.
`.brain/objects.pack` holds every object's canonical bytes in one file,
because reading the graph as 10,575 loose files costs 979 ms and as one
file 4 ms. It is derived, disposable, never replicated, and verified on
read exactly as a loose object is — the loose objects stay the record.
Two caches, one rule: **nothing authoritative lives in either.**

That rule is also the answer to "would an embedded graph database make
this faster?", asked and measured in 2026-07. It would not have: the
`Index` trait hands back `NodeId`s, so a faster backend answers *which
nodes* sooner while the cost sat entirely in what happens next — turning
ids into objects, and doing it again on the next call. Caching parsed
objects, memoising the put feed and packing the bytes took a commit from
10.9 s to 0.14 s and `brain wake` from 2.9 s to 0.05 s, with no engine
adopted. The seam stays open; the question is worth re-asking when the
graph no longer fits in memory, which at 2.6 MB is a long way off.

The quality series added to every refresh in 2026-07 (tests, features,
document drift, uncorroborated claims — the last needs a spine build)
was measured against that budget before landing: ~10 ms on a warm index,
refresh 0.42 s → 0.43 s, so the spine runs on every refresh rather than
behind a change-gate.

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
- **Who is recorded, but grants nothing.** `agent_session` entities
  (ADR-025) are the only record of a principal: which coding agent ran,
  what it was asked to do, and which files it edited. They are history, not
  authority — an `Intent` still carries no requester, `Object::Capability`
  is still never constructed, and the only check that exists is membership
  of the capability set passed on the command line. Nothing in the graph
  can approve anything.

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
- ~~**Alpha-equivalence in hashing**~~ — fixed: `Store::put` alpha-normalizes
  Code terms (binders renamed to de Bruijn-level names), so alpha-equivalent
  programs deduplicate to one node and stored bytes always re-hash to their
  id. Verified end to end: the same program authored in JSON with one
  parameter name and in compact notation with another lands on a single node.
- **Replication, branch worlds, learning lineage, agent-to-agent transfer** —
  roadmap items; see `docs/roadmap.md`.
- **A files export** — intentionally not built into the core loop. Projection
  to files may exist for legacy interop, but the moment internal tooling
  depends on exported files, files are authoritative again through the back
  door.
