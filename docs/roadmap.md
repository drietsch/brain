# Roadmap

The founding assumption — programs live natively in the graph, no files above
the semantic line — is treated as a bet whose riskiest test comes first.

**Pivot (2026-07-26): the twin is the first deliverable.** Reflective mode —
the graph as a persistent semantic model of existing software — ships before
further native-mode capability. Twin v1 (structure queries, drift &
staleness, agent notes, continuous refresh; `docs/twin.md`) is done; its
natural growth path is deeper extraction (tree-sitter), the govern step
(routing changes to twinned software through intents/receipts), and
embedding-based similarity over twin entities.

## Stage 0 — Kernel object model ✅ (this scaffold)

Canonical encoding, content addressing, the object model, namespace lineage,
the durable intent log with indeterminate outcomes, the fuel-metered
capability-checked interpreter, reflective-mode ingestion, self-twin.

## Stage 1 — The authoring experiment (make-or-break, run early)

Can current models reliably author and *edit* non-trivial programs in the
term schema via constrained output? Harness: N tasks of increasing size;
measure emission validity, edit locality (typed graph edits vs. whole-program
regeneration), and hole-refinement success. If this fails, the calculus gets
redesigned while it is still cheap. Everything downstream is gated here.

## Stage 2 — Governed effects worth having

Real foreign symbols (HTTP, process, file-projection for legacy interop),
capability objects with scopes actually consulted at the boundary,
reconciliation workflows for indeterminate intents, and the
verification-level → authority-ceiling rule enforced rather than recorded.

## Stage 3 — The graph as a codebase in earnest

~~De Bruijn canonicalization (alpha-equivalent hashing)~~ (done: store-boundary
alpha-normalization), ~~compact authoring notation~~ (done: 10-20x denser,
provably projection-level), semantic diff between namespace steps,
`(spec, implementation, evidence)` triples linked and queried (evidence
queries exist: `brain evidence <name>`), ~~evidence-keyed test caching~~
(done: the checker consults the graph and skips evaluation when passing
evidence attests the (code hash, task content) pair; failures always re-run),
embedding annotations beside hashes for similarity retrieval. The `brain-index` trait seam exists; this stage
benchmarks embedded graph-database backends (OverGraph, Graph_D) against
`MemIndex` on real twin data and adopts one only if it wins.

**Asked and answered once already (2026-07, prompted by GrafeoDB).** The
measurements pointed away from the index: warm index open was 10 ms while
the query after it took 1.92 s, because the `Index` trait returns
`NodeId`s and the cost was turning those into objects — repeatedly, and
one file at a time. Caching parsed objects, memoising the put feed and
packing object bytes took a commit from 10.9 s to 0.14 s and `brain wake`
from 2.9 s to 0.05 s. No engine was adopted and the seam is untouched.
Two things would make the question live again: a graph too large to hold
in memory, or queries that are genuinely declarative and traversal-heavy
rather than "latest value for this (subject, property)".

## Stage 4 — Replication and distribution

~~Content-addressed sync between stores~~ (done: `brain pull|push`, set-union
objects with epoch-checked ingest, conflicts preserved as `sync-conflict/`
bindings, evidence travels and re-checks arrive cached). Remaining: signed
partitions, transport beyond the local filesystem, receipts that survive
reconciliation after disconnection.

## Stage 5 — The tournament

Same tasks, same governance, competing representations: native terms vs.
foreign-wrapped conventional code. Measured on success, cost, defect rate,
recovery quality, maintainability over repeated edits, governance
enforceability. Re-run per model generation. The foreign→native migration
rate of real workloads is the standing scoreboard for the founding bet.

## The crossover metric

The scaffold's own logic lives in Rust files (the bootstrap irony is
unavoidable). The milestone that matters: the first day a piece of the
system's *own* behavior — a reconciliation procedure, a maintenance task —
runs as nodes in its own graph rather than code in its repo, observed by its
own twin. From then on, the migration is measurable from inside.
