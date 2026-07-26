# The Twin — Reflective Mode

The twin is the graph acting as a **persistent, queryable semantic model of
existing software**. Native mode holds programs the substrate executes;
reflective mode holds *descriptions* of software that lives elsewhere — as
entities, time-bound observations, and typed relations. Both share one
identity scheme, so a twinned thing can later gain a native implementation
without re-modeling: migration is a gradient (describe → observe → govern →
absorb), not an event.

Why it matters, in one sentence each:

- **Persistent agent memory.** An agent orients in a codebase by querying the
  twin instead of re-reading files every session.
- **Typed structure beats RAG.** Symbols, imports, and reverse-dependencies
  are queries over relations, not statistical retrieval over text chunks.
- **A CMDB that doesn't rot.** Every fact is a sourced, timestamped
  observation that expires into staleness — never an eternal truth. Deletion
  is itself an observation (`present=false`), never an erasure.

## Data model

| Piece | Object | Example |
|---|---|---|
| A thing | `Entity` (stable id) | repo, `source_file`, `symbol`, `module` |
| A fact about it | `Observation` | `content_b3`, `language`, `line`, `present`, `git_commit` |
| Structure between things | `Relation` (from, predicate, to) | file `contains` symbol; file `imports` file/module |
| Agent memory | `Observation` with `property: "note"` | anything worth remembering |

Stable ids are derived deterministically (`file:<path>`, `symbol:<path>:<kind>:<name>`,
`module:<import>`), so re-observation is idempotent and two stores twin the
same software to the same identities.

## CLI walkthrough

```bash
brain twin refresh <dir> --prefix twin/app   # observe; record only what drifted
brain twin status  <dir> --prefix twin/app   # same comparison, read-only
brain twin files   twin/app                  # files with language/symbols/freshness
brain twin symbols twin/app/src/main.rs      # what a file declares (with lines)
brain twin imports twin/app/src/main.rs      # what it depends on
brain twin rdeps   twin/app/src/util.rs      # who depends on it
brain twin search  util                      # find entities by name

brain note  twin/app/src/main.rs "entry point; config loading lives here"
brain notes twin/app/src/main.rs             # durable, chronological

brain observations twin/app/src/main.rs      # full observation timeline
```

`brain ingest` is an alias for `twin refresh`.

## The refresh contract

- **Only drift is recorded.** Unchanged files write nothing; an immediately
  repeated refresh writes zero objects. Changed and new files get fresh
  observations, symbols, and relations; vanished files get `present=false`;
  returned files get `present=true`.
- **Backfill:** a file twinned before structure extraction existed (no
  `language` observation) gets its structure on the next refresh even if
  unchanged.
- **Repo-level facts:** the prefix itself binds to a `repo` entity carrying
  `git_commit` / `git_branch` observations (skipped outside git repos).
- **Continuous by design:** run it from cron, a git post-commit hook, or a
  session-start hook — the observer is a sense organ, not an importer.

## Languages and precision (honest limits)

Symbol/import extraction is **line-based and best-effort** — orientation, not
compiler-grade analysis:

| Language | Symbols | Imports |
|---|---|---|
| Rust | fn, struct, enum, trait, mod | `use` paths; `crate::foo` resolves to `src/foo.rs` when present |
| PHP | class, interface, trait, function, namespace | `use X\Y;` |
| Python | def, class | `import X`, `from X import` |
| JS/TS | function, class (incl. exported) | `import ... from`, bare imports, `require()` — relative paths resolve to files |
| other | file-level only | — |

Known imprecision: same-named symbols of the same kind in one file collapse
to one entity; arrow-function bindings and macro-generated items are missed;
unresolved imports become `module` entities rather than file links. Replacing
extractors with tree-sitter is a drop-in upgrade path (same entities and
relations, better extraction).

## Notes as agent memory

`brain note` attaches a durable observation to any entity. Notes survive
restarts, travel with replication (`brain pull` carries them between stores,
like all graph objects), and are ordered by the event log — the intended use
is agents recording what they learned (*"this module is the entry point"*,
*"tests here are flaky because of X"*) so the next session starts oriented
instead of from zero.

## Continuous insights

`brain twin insights <prefix>` synthesizes the twin into a picture of the
software — built for watching what agents build:

- **Churn**: most-edited files since twinning (content versions observed) —
  where agent activity concentrates.
- **Hubs**: most-imported files — where a change has the widest blast radius.
- **Largest**: most symbols declared — complexity concentrations.
- **External deps**: unresolved imports tallied by use.
- **Recent notes**: the memory agents left behind, newest first.
- **Growth series**: files/symbols/relations over time. Every refresh that
  changes the totals records one complete series point on the repo entity —
  so trends are graph objects: they persist, replicate with `brain pull`,
  and are queryable like everything else (`brain observations <prefix>`).

For true continuity, run the loop beside your agent sessions:

```bash
scripts/twin_watch.sh . twin/self 60   # refresh + insights every 60s
```

or wire `brain twin refresh` into a git post-commit hook / session-start
hook and read `brain twin insights` whenever you want the picture.

## Relation to the founding architecture

The twin is the "describe → observe" half of the adoption gradient in
`docs/architecture.md`. The govern step (routing changes to the external
software through intents/receipts) and the absorb step (twinned entities
gaining native implementations) reuse this same data model — shared identity
is what makes migration inward a matter of adding edges, not re-modeling.
