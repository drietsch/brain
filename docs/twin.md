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

## Decisions and plans — the *why* documents

The twin stores not just what the software *is* but why it is that way:
architecture decision records (ADRs) and the plans agents produce are
first-class entities (`decision`, `plan`) with sourced observations
(`content`, `title`, `status`) and typed relations linking them to the code
they concern.

Two capture paths:

- **Auto-detection during refresh.** Markdown in conventional paths is
  captured automatically: `docs/adr/`, `decisions/`, or `adr-*.md` filenames
  become decisions; `plans/` directories become plans. The file entity stays;
  the decision/plan entity is the semantic thing (`doc -recorded_in-> file`),
  so a document's identity survives file moves.
- **Explicit add for files outside the repo** — the Claude Code workflow:

```bash
brain plan add ~/.claude/plans/my-feature.md --prefix twin/app
brain adr  add decision.md --prefix twin/app --status accepted

brain adr  list twin/app        # [status] slug: title (age, mentions)
brain plan list twin/app
brain adr  show twin/app adr-001-storage   # full text + status timeline + mentions
```

What the graph gives you for free:

- **Status timeline.** An ADR whose `Status:` line changes
  (proposed → accepted → superseded) gets a new `status` observation each
  time — the decision's lifecycle is queryable history, never an overwrite.
- **Mentions-scan.** Every twinned file path appearing in a document's text
  becomes a `doc -mentions-> file` relation, so "which decisions cover this
  file?" is a reverse-relation query — and insights tags churn hotspots that
  have documented rationale with `[decided]`.
- **Supersession.** A `Supersedes:` line becomes a `supersedes` relation
  between decisions.
- **Replication.** Decisions and plans travel with `brain pull`, timelines
  and all, like every other graph object.

The intended loop: after a Claude Code plan is approved, `brain plan add`
it; after a significant decision, write an ADR into `docs/adr/` — the next
refresh captures it automatically.

## Continuous insights

`brain twin insights <prefix>` synthesizes the twin into a picture of the
software — built for watching what agents build:

- **Churn**: most-edited files since twinning (content versions observed) —
  where agent activity concentrates.
- **Hubs**: most-imported files — where a change has the widest blast radius.
- **Largest**: most symbols declared — complexity concentrations.
- **External deps**: unresolved imports tallied by use.
- **Recent notes**: the memory agents left behind, newest first.
- **Decisions and plans**: the newest ADRs (with status) and plans; churn
  entries covered by a decision are tagged `[decided]`.
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
