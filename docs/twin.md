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
  unchanged. After an extractor upgrade, `brain twin refresh --full`
  reprocesses every file as if changed — still guarded, so unchanged
  facts write nothing.
- **Repo-level facts:** the prefix itself binds to a `repo` entity carrying
  `git_commit` / `git_branch` observations (skipped outside git repos).
- **Continuous by design:** `brain hook install` wires refresh into git —
  every commit and push triggers the brain (post-commit / pre-push, with
  stale-doc and failing-test warnings at the moment of change; `--docs`
  regenerates projections on push; `--tests` runs the repo's test command
  post-commit and imports the protocol automatically). Hooks are
  fail-open: a sense organ, never a gate. `brain watch` covers the
  between-commits interval.

## Brownfield adoption: backfill the past

A twin normally accrues history from the day it starts observing. For
existing repos, `brain twin backfill <dir>` replays git history into the
graph first — every commit's changed files become `content_b3`
observations stamped with the **commit's time**, deletions become
`present=false`, resurrections restore presence, and every commit lands
as a `git_commit` observation on the repo entity. So a brownfield repo
arrives with:

- **churn that reflects its real life**, not just post-adoption edits;
- **`brain twin at <any-old-commit>`** working across all history;
- **co-change association** for every commit ever made (one commit = one
  timestamp = one batch).

Deliberate limits: file-level facts only (no historical symbol/import
reconstruction — the current refresh covers the present), blobs over 4 MB
skipped, facts sourced `"backfill"`. Idempotent by construction:
identical historical facts are content-addressed no-ops, so re-running
writes zero objects. The intended brownfield minute-one:

```bash
brain init
brain twin backfill . --prefix twin/app     # the past
brain twin refresh  . --prefix twin/app     # the present (structure, docs, tests)
brain hook install --tests                  # the future
brain attend twin/app                       # where to look first
```

## Languages and precision (honest limits)

Symbol/import extraction is **line-based and best-effort** — orientation, not
compiler-grade analysis:

| Language | Symbols | Imports |
|---|---|---|
| Rust | fn, struct, enum, trait, mod | `use` paths; `crate::foo` and cross-crate `foo_bar::baz` resolve to files across sibling `src/` trees (crate-root fallback for item imports) |
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

## Skills and agent configuration — the *how it is built* layer

Agentically-built software carries the files that configure the agents that
build it. The twin captures them as `skill` and `agent_config` entities with
`content`, `name`, `agent`, `role`, and `description` observations — so a
changed CLAUDE.md or skill is a timeline of versions, and the operating
setup replicates with the graph like everything else.

Auto-detected conventions during refresh:

| Path | Entity | agent / role |
|---|---|---|
| `**/<name>/SKILL.md` | skill | claude / skill (frontmatter `name:`/`description:` parsed) |
| `CLAUDE.md` (any depth) | agent_config | claude / instructions |
| `AGENTS.md` | agent_config | generic / instructions |
| `GEMINI.md` | agent_config | gemini / instructions |
| `.claude/agents/*.md` | agent_config | claude / subagent |
| `.claude/commands/*.md` | agent_config | claude / command |
| `.claude/settings(.local).json` | agent_config | claude / settings |
| `.mcp.json` | agent_config | claude / mcp |
| `.cursorrules`, `.cursor/rules/*.mdc` | agent_config | cursor / rules |
| `.github/copilot-instructions.md` | agent_config | copilot / instructions |
| `.codex/*` | agent_config | codex / settings |

Nested instruction files keep distinct identities (the slug is the path), so
`crates/core/CLAUDE.md` never collides with the root `CLAUDE.md`.

Explicit add for user-level configuration outside the repo:

```bash
brain skill add ~/.claude/skills/deploy/SKILL.md --prefix twin/app
brain agentcfg add ~/.claude/CLAUDE.md --prefix twin/app --agent claude
brain skill list twin/app          # [agent] slug (role) — description
brain agentcfg show twin/app claude.md
```

Mentions-scanning applies here too: a skill whose text names twinned files
gets `mentions` relations to them, so "which skills touch this file?" is a
reverse-relation query.

## Templates, the definition of done, and the feature registry

The deliverable contract itself lives in the graph. `brain init` (or `brain
template seed`) writes `template` entities under `brain/templates/` — each
with a `content` scaffold, machine-checkable `requires` fields, and the
entity kind it `applies_to`. Because templates are graph objects:

- they **version through observations** and evolve per store (a local edit
  to `requires` wins over the shipped default and survives re-seeding);
- they **replicate** with `brain pull` — the team's working contract
  travels with the software;
- agents ask the graph, not a wiki: `brain template list`,
  `brain deliverable new adr --title "..."` emits the scaffold.

**Conformance is recorded, never enforced.** During refresh every captured
decision/plan/skill is checked against its template's required fields; the
result is a `conforms` observation (with `missing` when false) plus a
`conforms_to` relation. Violations surface in insights as "nonconforming
docs" — the reflective mode stays descriptive; enforcement belongs to the
governed mode.

**Definition of done = the `feature` template's `requires`.** Its fields
are relation predicates, not text checks: `implemented_by`, `tested_by`,
`decided_by`, `documented_in` by default. A feature is an explicit
declaration linked into the graph, and done-ness is a query:

```bash
brain feature add  twin/app checkout --title "Checkout flow" --status building
brain feature link twin/app checkout implemented_by src/checkout.rs
brain feature link twin/app checkout decided_by adr-007-payments
brain done twin/app checkout          # ✓/✗ per predicate; records the outcome
brain feature matrix twin/app         # the registry as a rendered query
```

Re-running `feature add` with a new `--status` updates it (guarded, so it
is also the status-change mechanism); `done` flips are observations — a
feature that regresses out of done-ness is a recorded event, not a silent
cell change in a spreadsheet.

## Tests and test protocols

Tests are graph citizens on both axes — what test code *exists*, and what
happened when it *ran*:

**Static (zero-config, at refresh).** Twinned files are classified by
framework — Rust `#[test]`, Playwright/Jest specs (`.test.` / `.spec.` /
`@playwright/test`), pytest files, PHPUnit classes — as `test_framework`,
`tests_declared`, and `file_role=test` observations. A test file gets
`covers` relations to the twinned files it imports, so "which tests cover
this file?" is `relations_to(file, covers)` — and insights surface
**untested hubs**: heavily-imported files with no declared tests and no
covering spec, the concentrated-risk list.

**Dynamic (protocols).** `brain testrun import` ingests a report — raw
`cargo test` output, JUnit XML (the interchange format Playwright,
pytest, PHPUnit, and Jest all export), or Playwright's own JSON reporter.
Prefer the JSON one for browser tests: it is the only report that names
the screenshots, videos and traces a run produced, and those become
assets owned by the case that produced them. With `brain hook install --tests`,
this happens automatically on every commit: the test command (inferred
from the repo's manifest, or set with `--test-cmd`) is stored as a
`test_command` observation on the repo entity — change it any time
without touching the hooks, and it replicates with the graph:

```bash
cargo test 2>&1 | brain testrun import - --prefix twin/app
npx playwright test --reporter=json | brain testrun import - --prefix twin/app
brain testrun list twin/app
brain twin tests twin/app        # files, frameworks, covers, failing cases
```

What the graph gives you:

- **Content-addressed runs.** A run's identity is the hash of its raw
  report; re-importing the same report writes nothing.
- **Result timelines = flake history.** Each test case is an entity whose
  `result` observations are guarded — only *transitions* (pass→fail,
  fail→pass) are recorded, so a flaky test is literally a case with many
  result observations.
- **Evidence, not just data.** Every run writes a Behavioral-level
  Evidence object on the repo entity (`testrun@<hash>`), tying test
  protocols into the same verification taxonomy native code uses.
- **File linkage.** A spec file named by the reporter — or a JUnit
  classname that is a twinned path, Playwright's convention — produces
  `test_case -defined_in-> file` relations.
- **Evidence you can look at.** Playwright's JSON also carries the failure
  message, the duration, the retry count, and the attachments. Each
  attachment becomes a declared asset `attached_to` its case, so a failing
  browser test in Eyes shows its screenshot rather than just its name.
- **Run membership where it means something.** A run links to the cases
  that failed, were skipped, or changed their mind (`failed`, `skipped`,
  `includes`). Passing cases are deliberately not linked to every run —
  the case's own result timeline already says what it does.
- **Replication.** Runs, timelines, and evidence travel with `brain pull`.

## Teach brain a new artifact type (no code)

The built-in detectors (ADRs, plans, skills, agent config, tests) are just
conveniences. Any *other* artifact family — runbooks, incidents, RFCs,
postmortems — can be taught to the store as **data on its template**:

```bash
brain template set runbook \
  --applies-to runbook \
  --capture "docs/runbooks/*.md" \
  --fields  "title=heading, service=line" \
  --requires "title,service" \
  --title "Operational runbook"
```

Two observations do the work: `capture` (glob patterns — `*` within a
segment, `**` across, `?` one char) says which paths are artifacts of this
kind; `fields` says how to lift properties out of the text with a fixed
extractor vocabulary (`heading`, `line[:Key]`, `frontmatter[:key]`,
`slug`). From the next refresh — including the one your git hook runs —
matching files become entities of the new kind with extracted-field
observations, mentions-links to the code they name, conformance against
`requires`, staleness detection, and replication. Nothing was compiled;
the store taught itself, and `brain pull` teaches every replica.

Built-in detectors keep precedence; rules apply to paths they didn't
claim. Browse any kind, built-in or taught, with:

```bash
brain artifact list twin/app runbook
brain artifact show twin/app runbook deploy   # fields, full text, mentions
```

## Always-up-to-date docs: staleness + projections

Two mechanisms keep documentation honest (see
docs/adr/adr-005-docs-as-projections.md):

**Staleness detection** for hand-written docs. A doc whose mentioned files
gained newer content observations than the doc itself is *possibly stale*:

```bash
brain twin stale twin/app     # doc -> which files changed since it was written
```

Derived at query time, never written; surfaced in insights the moment rot
happens instead of when a reader trips over it.

**Generated docs as projections.** `brain docs generate` regenerates
`docs/generated/` wholesale from live graph queries:

- `tour.md` — insights, feature matrix, decisions, tests, protocols,
  staleness: verbatim query results.
- `img/*.png` — terminal screenshots rendered with Playwright's bundled
  Chromium.
- `tour.webm` — a typed screencast of the same session.
- `narration.txt` + `tour-narrated.webm` — a spoken tour whose sentences
  are computed from the same queries (file counts, pass rates, DoD
  fractions), synthesized to audio and muxed onto the screencast.

TTS backend: **Qwen3-TTS-12Hz-0.6B-Base** when its stack is available
(`pip install torch soundfile transformers`; weights fetch on first use —
see `crates/brain-cli/assets/tts.py`, embedded in the binary), with espeak-ng as the offline fallback, so
the pipeline degrades gracefully instead of failing.

The generated artifacts are themselves twinned (media extensions are
ingested), so "when were the docs last regenerated, and from which commit"
is a graph query like everything else. Run it from the same hooks as
`twin refresh` — docs that regenerate with the twin cannot drift from it.

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
- **Features**: DoD progress per feature; **nonconforming docs**: template
  contract violations.
- **Tests**: test files and declared cases, the last imported run,
  currently-failing cases, and untested hubs.
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

## The functional brain

The name is taken seriously — by function, not by structure (see
docs/adr/adr-009-functional-brain-not-structural.md; simulated neurons are
deliberately rejected: the LLM agent is the neural layer, the graph is the
exact memory it thinks against):

| Organ | Mechanism |
|---|---|
| Senses | the observers: files, symbols, tests, protocols, git |
| Reflexes | git hooks — stimulus → response, fail-open (plus the opt-in pre-commit gate, ADR-020) |
| Long-term memory | the immutable graph and its timelines |
| Learning | the kind registry + capture rules (ADR-017), measured by template fitness (ADR-022) |
| **Orientation** | `brain wake <prefix>` — last sleep, the delta since, attention, warn-stale, in-flight work, coherence |
| **Attention** | `brain attend <prefix>` — one ranked list of what matters now, recency-weighted by the sleep window |
| **Consolidation** | `brain sleep <prefix>` — distill history into durable memory |
| **Association** | `brain related <name>` — what is related, and why |
| **Hygiene** | `brain tidy <prefix>` — drifted projections, retired artifacts, legacy assets; fixes are governed changes (ADR-021) |

```bash
brain wake twin/app            # one command, the whole present (~40 lines)
brain attend twin/app          # recent churn × blast-radius × untested × failing × warn-stale
brain related twin/app/src/checkout.rs   # "changed together 6×", "both mentioned by adr-007"
brain tidy twin-app-dir --prefix twin/app   # what has outlived its purpose, and the fix for each
brain sleep twin/app           # writes session_summary + per-file memory digests
```

Attention is computed at query time and never stored (salience is a
judgment about now), and its churn signal is windowed by the last sleep
(`consolidated_until`, ADR-016): the present dominates, history is
capped. Sleep only ever *adds* — per-file `memory` digests and a repo
`session_summary` that wake reads back — so a long-lived twin orients
from consolidated experience instead of replaying raw history.
Association lives at the Index seam: derived, disposable, rebuildable —
fuzzy recall that can never become a second source of truth.

Currency is first-class (ADR-013/014/015): structure the refresh no
longer observes is retracted (edge tombstones — hubs and blast radius
track reality, not history), superseded and finished artifacts leave
every list (`brain plan done`, lifecycle derivation), staleness carries
per-kind severity and can be acknowledged (`brain adr ack` — reviewed,
still accurate, file untouched). Graph-first kinds render **read-only
projections** under docs/brain/ (ADR-019): marker, chmod, and
`expected_b3` detection keep files views of the graph, never rivals of
it. The intended rhythm: **`wake`, work, `tidy`, `sleep`.**

## Governed mode — the motor system

The gradient's third step: brain doesn't just observe the software, it
can *change* it — through the same intent/receipt boundary native effects
use, with explicit capability and full provenance:

```bash
brain change propose twin/app src/config.rs --from new-config.rs \
  --reason "raise the connection pool limit"
brain change apply  twin/app config.rs-3fa2b1c9 --cap fs   # Intent → write → Receipt
brain change verify twin/app config.rs-3fa2b1c9            # run tests, grade it
brain change revert twin/app config.rs-3fa2b1c9 --cap fs   # governed undo
brain change list twin/app                                 # the ledger
```

- **Propose is pure**: reason, target, full before/after content and
  hashes land in the graph; disk is untouched until apply.
- **Apply is crash-safe**: the Intent is durably logged *before* the
  write, the Receipt after. A crash in between leaves *indeterminate* —
  `brain recover` marks it (never retries); reconciliation is deliberate.
- **No ambient authority**: apply and revert refuse without `--cap fs`,
  exactly as runtime effects refuse without their capability.
- **Verification is evidence**: `verify` runs the repo's stored test
  command, imports the protocol, and links it `verified_by` — a change's
  status timeline reads proposed → applied → verified (or broken).

Observation and governance compose: the next refresh sees an applied
change as ordinary drift, and the change entity's `changes` relation ties
the drift to its reason. See
docs/adr/adr-010-governed-mode.md.

## cortex — the query engine underneath

Every query above runs on **cortex** (`crates/cortex`), brain's own
persistent graph-query engine — learned from minigraf, then simplified by
one observation: brain's event log already is a WAL, so cortex is just
a checkpoint (`.brain/cortex.json`) plus delta-replay from a cursor:

- warm opens are O(new events) — and the checkpoint is derived,
  disposable, and rebuilt silently if corrupt or stale. It is one of two
  such caches: `.brain/objects.pack` does the same for object bytes,
  because reading 2.6 MB as 10,575 files costs 979 ms and as one file
  4 ms. Both are disposable; the loose objects and the event log remain
  the only systems of record;
- recursive traversal powers `--transitive` on imports/rdeps (the true
  blast radius) — `brain twin rdeps <file> --transitive`;
- bi-temporal reads power `brain twin at <prefix> <when>` — the twin as
  it was at an epoch, `30m`/`2h`/`1d` ago, or **at a git commit** (hashes
  resolve through the repo entity's observation timeline);
- `brain bench index` is the standing earn-adoption gate: it verifies
  both backends answer identically over real probes before printing
  timings, and `BRAIN_INDEX=mem` always forces reference behavior.

The `.graf` file never replicates: truth travels as objects with
`brain pull`; each store grows its own index. See
docs/adr/adr-011-cortex.md.

## Relation to the founding architecture

The twin is the "describe → observe" half of the adoption gradient in
`docs/architecture.md`. The govern step (routing changes to the external
software through intents/receipts) and the absorb step (twinned entities
gaining native implementations) reuse this same data model — shared identity
is what makes migration inward a matter of adding edges, not re-modeling.
