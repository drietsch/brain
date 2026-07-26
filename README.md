# brain

An agent-native semantic substrate: one content-addressed graph that holds
code, specs, capabilities, intents, receipts, entities and observations — with
no files above the semantic line.

**Status: scaffold.** The kernel compiles, is tested, and exercises the full
governed loop end to end, but everything here is deliberately minimal.

## Install (one command)

```bash
curl -fsSL https://raw.githubusercontent.com/drietsch/brain/main/install.sh | sh
```

or, with a Rust toolchain already present:

```bash
cargo install --locked --git https://github.com/drietsch/brain brain
```

Either way you get one monolithic `brain` binary — store, twin, templates,
features, test protocols, and the docs pipeline included (the docs media
steps use node/playwright, python3, and ffmpeg when present, and skip
gracefully when not).

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
cargo test                 # 67 tests, including crash recovery
cargo run -p brain -- init
cargo run -p brain -- demo            # author -> store -> bind -> run
cargo run -p brain -- twin refresh . --prefix twin/self   # the brain twins itself
cargo run -p brain -- twin symbols twin/self/crates/brain-core/src/object.rs
cargo run -p brain -- twin rdeps twin/self/crates/brain-observe/src/symbols.rs
cargo run -p brain -- note twin/self/README.md "docs entry point"
cargo run -p brain -- plan add ~/.claude/plans/feature.md --prefix twin/self  # Claude Code plans
cargo run -p brain -- adr list twin/self       # decisions (auto-captured from docs/adr/)
cargo run -p brain -- skill list twin/self     # agent skills (SKILL.md, auto-captured)
cargo run -p brain -- agentcfg list twin/self  # CLAUDE.md/AGENTS.md/.cursorrules/settings
cargo run -p brain -- deliverable new adr --title "Use X"  # scaffold from graph template
cargo run -p brain -- feature matrix twin/self # definition-of-done as a rendered query
cargo test 2>&1 | cargo run -p brain -- testrun import - --prefix twin/self  # protocol -> graph
cargo run -p brain -- twin tests twin/self     # frameworks, covers-relations, failing
cargo run -p brain -- twin stale twin/self     # docs invalidated by later file changes
brain docs generate            # regenerate docs: md + screenshots + narrated screencast
cargo run -p brain -- twin insights twin/self   # churn, hubs, growth, notes, decisions
cargo run -p brain -- watch . --prefix twin/self --interval 60   # continuous loop, built in
cargo run -p brain -- status
cargo run -p brain -- names
cargo run -p brain -- refs demo/answer            # reverse edges
cargo run -p brain -- deps <b3:hash>              # forward edges
cargo run -p brain -- observations twin/self/README.md
cargo run -p brain -- task check tasks/t01-increment.json tasks/solutions/increment.json
cargo run -p brain -- notation tasks/solutions/abs.json   # project to compact notation
cargo run -p brain -- recover         # marks pending intents indeterminate

# Replication: code moves as content-addressed sync, with its evidence
BRAIN_STORE=/tmp/brain2 cargo run -p brain -- init
BRAIN_STORE=/tmp/brain2 cargo run -p brain -- pull .brain
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
| `brain-observe` | Reflective mode — the twin: drift-aware observation of external software with symbols, import relations, agent notes, decisions/plans (ADRs), skills and agent configuration. See `docs/twin.md`. |
| `brain-index` | The system-of-query seam: derived, disposable indexes rebuilt by replaying the event log. `MemIndex` is the reference backend; embedded graph engines can implement the same trait. |
| `brain` (in `crates/brain-cli`) | The monolithic binary: projection instrument plus the embedded docs pipeline; holds no state of its own. |

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
7. A test result is a fact about a hash: the checker skips evaluation when
   passing evidence already attests the (code hash, task content) pair —
   and alpha-normalization means a re-authored solution in any encoding,
   with any variable names, hits that cache if it means the same program.

See `docs/architecture.md` for the design, `docs/calculus.md` for the term
language, `docs/schema/term.schema.json` for the shape agents author against,
`docs/authoring.md` for the Stage 1 experiment protocol (with the task corpus
in `tasks/`), and `docs/roadmap.md` for where this is going.
