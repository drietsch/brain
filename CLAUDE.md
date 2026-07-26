# Working on brain

An agent-native semantic substrate: the graph is authoritative, files exist
only below the semantic line. Read `README.md` first, then
`docs/architecture.md`; the twin (reflective mode) is documented in
`docs/twin.md`.

## Build and test

- `cargo test` — the whole workspace; every crate has its own suite.
- `cargo build -p brain-cli` before running `target/debug/brain` — `cargo
  test` does NOT rebuild the CLI binary, and a stale binary has caused
  confusing demos twice.
- The local store lives in `.brain/` (gitignored). `BRAIN_STORE=<dir>`
  points the CLI at another store.

## Orientation via the twin (instead of re-reading files)

```bash
target/debug/brain twin refresh . --prefix twin/self
target/debug/brain twin insights twin/self     # churn, hubs, decisions, plans
target/debug/brain notes twin/self             # what previous sessions learned
```

## Invariants to never break

- Identity is canonical bytes: no floats in objects, sorted keys, and
  `Store::put` alpha-normalizes Code terms (see
  docs/adr/adr-002-alpha-normalization-at-store-boundary.md).
- `Object` serde tag owns the `kind` key; Relation's edge label is the
  `predicate` field (see docs/adr/adr-001-relation-predicate-field.md).
- Objects are immutable; names rebind through namespace lineage. Twin facts
  are observations — new nodes, never overwrites.

## Conventions

- After an approved plan: `brain plan add <plan.md> --prefix twin/self`.
- After a significant decision: write an ADR into `docs/adr/` — the next
  refresh captures it.
- After a test run: `cargo test 2>&1 | brain testrun import - --prefix
  twin/self` — protocols belong in the graph.
- Before finishing: `brain twin stale twin/self` (fix rotted docs) and
  `scripts/docsgen/generate.sh . twin/self` (docs/generated/ is a
  projection — regenerate, never edit).
- Leave `brain note` breadcrumbs for the next session.
