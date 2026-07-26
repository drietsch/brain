# Roadmap

The founding assumption — programs live natively in the graph, no files above
the semantic line — is treated as a bet whose riskiest test comes first.

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

De Bruijn canonicalization (alpha-equivalent hashing), semantic diff between
namespace steps, `(spec, implementation, evidence)` triples linked and
queried, evidence-keyed test caching (a test result is a fact about a hash,
forever), embedding annotations beside hashes for similarity retrieval.

## Stage 4 — Replication and distribution

Content-addressed sync between stores (how code moves; replaces deployment),
signed partitions, receipts that survive reconciliation after disconnection.

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
