# brain

An agent-native semantic substrate: one content-addressed graph that holds
code, specs, capabilities, intents, receipts, entities and observations — with
no files above the semantic line.

**Status: scaffold.** The kernel compiles, is tested, and exercises the full
governed loop end to end, but everything here is deliberately minimal.

## The idea in three sentences

Software is not stored as files; it lives as immutable, content-addressed
nodes in a graph, and "the codebase" is a chain of namespace objects mapping
names to node hashes. Programs are terms of a tiny calculus in which the only
gate to the outside world is a capability-checked foreign call wrapped in a
durable intent (before) and a receipt (after), so a crash mid-effect leaves an
explicit *indeterminate* state that is reconciled, never blindly retried. The
same graph can also *twin* external file-based software as entities and
observations, sharing one identity scheme so twinned things can later gain
native implementations without re-modeling.

## Quickstart

```bash
cargo test                 # 23 tests, including crash recovery
cargo run -p brain-cli -- init
cargo run -p brain-cli -- demo            # author -> store -> bind -> run
cargo run -p brain-cli -- ingest . --prefix twin/self   # the brain twins itself
cargo run -p brain-cli -- status
cargo run -p brain-cli -- names
cargo run -p brain-cli -- refs demo/answer            # reverse edges
cargo run -p brain-cli -- deps <b3:hash>              # forward edges
cargo run -p brain-cli -- observations twin/self/README.md
cargo run -p brain-cli -- task check tasks/t01-increment.json tasks/solutions/increment.json
cargo run -p brain-cli -- recover         # marks pending intents indeterminate
```

The demo stores two programs in the graph, runs the pure one (42), shows the
effectful one being denied without `--cap io` and succeeding with it, and
leaves the intent/receipt trail in the store for inspection.

## Layout

| Crate | Role |
|---|---|
| `brain-core` | Identity (`NodeId`, `StableId`), canonical encoding, the object model and core calculus. The constitutional layer: everything else is replaceable. |
| `brain-store` | Content-addressed store, namespace lineage ("version control"), event log, durable intent log. |
| `brain-runtime` | Fuel-metered interpreter; capabilities checked before effects; effects only through the intent/receipt boundary. |
| `brain-observe` | Reflective mode: ingest external source trees as entities + time-bound observations. |
| `brain-index` | The system-of-query seam: derived, disposable indexes rebuilt by replaying the event log. `MemIndex` is the reference backend; embedded graph engines can implement the same trait. |
| `brain-cli` | Projection instrument; holds no state of its own. |

## Invariants the scaffold already enforces

1. Semantically identical objects have identical content identity
   (canonical encoding; floats rejected; tested).
2. Objects are immutable; names rebind via new namespace objects with lineage
   — the codebase cannot be broken in place.
3. The term language has no ambient authority: external effects require a
   declared capability and pass through the effect boundary or not at all.
4. Intent is durably recorded *before* a consequential effect; a crash leaves
   *indeterminate*, and recovery marks — it never re-executes.
5. Simulation posture exists: a boundary that refuses all external effects,
   even when capabilities are granted.
6. Reflective-mode facts are observations — sourced, timestamped, expiring
   into staleness — never eternal truths.

See `docs/architecture.md` for the design, `docs/calculus.md` for the term
language, `docs/schema/term.schema.json` for the shape agents author against,
`docs/authoring.md` for the Stage 1 experiment protocol (with the task corpus
in `tasks/`), and `docs/roadmap.md` for where this is going.
