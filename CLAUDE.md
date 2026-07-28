# Working on brain

An agent-native semantic substrate: the graph is authoritative, files exist
only below the semantic line. Read `README.md` first, then
`docs/architecture.md`; the twin (reflective mode) is documented in
`docs/twin.md`.

## Build and test

- `cargo test` — the whole workspace; every crate has its own suite.
- `cargo build -p brain` before running `target/debug/brain` — `cargo
  test` does NOT rebuild the CLI binary, and a stale binary has caused
  confusing demos twice.
- The local store lives in `.brain/` (gitignored). `BRAIN_STORE=<dir>`
  points the CLI at another store.

## Orientation via the twin (instead of re-reading files)

```bash
target/debug/brain twin refresh . --prefix twin/self
target/debug/brain wake twin/self              # the whole present: last sleep, delta, attention, stale, in-flight
target/debug/brain attend twin/self            # what matters now, ranked
target/debug/brain twin insights twin/self     # churn, hubs, decisions, plans, last sleep
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

- Git hooks are installed (`brain hook install`): every commit/push
  refreshes the twin automatically and prints stale-doc warnings — read
  them.
- After an approved plan: `brain plan add <plan.md> --prefix twin/self`.
- After a significant decision: write an ADR into `docs/adr/` — the next
  refresh captures it.
- After a test run: `cargo test 2>&1 | brain testrun import - --prefix
  twin/self` — protocols belong in the graph.
- Before finishing: `brain twin stale twin/self` (fix `[warn]` rot; a doc
  that is still accurate gets `brain adr|plan|artifact ack` instead of a
  touch), `brain plan done <prefix> <slug>` for finished plans, and
  `brain docs generate` (docs/generated/ is a
  projection — regenerate, never edit).
- Leave `brain note` breadcrumbs, then `brain sleep twin/self` — the next
  session wakes to the consolidated summary instead of raw history.

<!-- brain:begin instructions — generated from the kind registry by `brain instructions generate`; edit rules with `brain template set`, never here -->

## Brain guardrails

Orient with `brain wake twin/self` before working; consolidate with `brain sleep twin/self` before finishing.

Artifact kinds (where truth lives, how to author):

| kind | placement | lives at | author via | requires | enforcement |
|---|---|---|---|---|---|
| asset | file_first | docs/assets/** | write the file; the twin captures it |  | advisory |
| capability_matrix | projection | docs/brain/capability-matrix/{slug}.md | rendered query — never authored |  | advisory |
| decision | file_first | docs/adr/*.md | write the file; the twin captures it | title, status | advisory |
| doc | file_first | README.md, docs/*.md | write the file; the twin captures it | title | advisory |
| feature | graph_first | in the graph only | `brain artifact new twin/self feature <slug>` | implemented_by, tested_by, decided_by, documented_in | advisory |
| plan | graph_first | docs/brain/plans/{slug}.md | `brain artifact new twin/self plan <slug>` | title | advisory |
| prototype | file_first | prototypes/** | write the file; the twin captures it | title | advisory |
| runbook | file_first | docs/runbooks/** | write the file; the twin captures it | title | advisory |
| skill | file_first | in the graph only | write the file; the twin captures it | name, description | advisory |
| stage | graph_first | in the graph only | `brain artifact new twin/self stage <slug>` | title | advisory |
| task_list | graph_first | docs/brain/task-lists/{slug}.md | `brain artifact new twin/self task_list <slug>` | title | advisory |

Rules:

- Files under `docs/brain/` are **read-only projections** of the graph. Edit through `brain artifact edit twin/self <kind> <slug>`, never the file.
- Finished plans: `brain plan done twin/self <slug>`. A doc reviewed and still accurate: `brain adr|plan|artifact ack` (resets its staleness clock).
- A wrong or outdated link: `brain relation retract <from> <predicate> <to>`.
- Binary assets (screenshots, HTML templates): `brain asset add <file> --prefix twin/self --for <kind>/<slug> --depicts <path>` — declared links are their staleness story.
- Enforced kinds refuse nonconforming writes (exit 3) and, with the pre-commit gate, block commits; the error names the fix.

<!-- brain:end instructions -->
