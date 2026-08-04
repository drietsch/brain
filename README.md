# brain

An agent-native semantic substrate: one content-addressed graph that is the
authoritative semantic layer above a codebase — decisions, plans, features,
symbols, test evidence, observations — while the code itself lives where it
always has: in files, under git. Everything semantic is authored through the
governed `brain` CLI and persisted only in the graph; the filesystem carries
read-only projections of it, for reference, never for editing. Programs can
also live natively in the graph as content-addressed terms; that mode runs
end to end and remains the founding experiment.

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

The graph *twins* file-based software: code stays on the filesystem under
git, and the graph holds the semantic layer above it — symbols, relations,
decisions, plans, test evidence — as sourced, time-bound observations rather
than eternal truths. The same graph can also hold programs natively, as
immutable terms of a tiny calculus whose only gate to the outside world is a
capability-checked foreign call wrapped in a durable intent (before) and a
receipt (after), so a crash mid-effect leaves an explicit *indeterminate*
state that is reconciled, never blindly retried. One identity scheme spans
both modes, so migration is a gradient — describe, observe, govern, absorb —
and a twinned thing that later gains a native implementation is the same
node, no re-modeling.

## The ambition

That the graph always reflects the actual state of the application it
supports: every feature, every capability, every test, every plan, every
coding session — present, current, and interconnected. Freshness is worked
for, not assumed: hooks refresh the twin on every commit, observations carry
sources and timestamps, staleness is surfaced (`brain twin stale`) and drift
is repaired under governance (`brain tidy`). And because it is one graph, the
connections are queryable — which tests cover which feature, which decision
shaped which file, which session touched what — with `brain eyes` as the
visualization layer on top: the state of the application, rendered for
people, with judgments, evidence and media. For developers, eyes doubles as
observability over the agentic work itself — sessions, plans, protocols,
in-flight intents — so everybody stays constantly up to date and the
orchestrating developer knows exactly what happened, what is going on now,
and what comes next.

## Where things live

- **Code** — on the filesystem, under git, as ever. The twin observes it:
  symbols, imports, churn, blast radius. The one CLI over all of it
  enters at `crates/brain-cli/src/main.rs`.
- **Deliverables** — plans, ADRs, features, task lists, test protocols,
  agent sessions, notes — are persisted only in the graph. Agents (Claude
  Code, Codex, ...) author and modify them exclusively through the `brain`
  CLI (`brain artifact new|edit`, `brain plan add`, `brain testrun import`,
  `brain note`, ...), never by writing documents into the repository.
- **Projections** — the graph renders deliverables to files (e.g. under
  `docs/brain/`) so people and tools can read them in place. They are
  read-only by contract: no mutations through the file layer — a change goes
  through the CLI or it doesn't happen, and the opt-in pre-commit gate
  refuses hand-edited projections.

## One contract for many agents

Deliverables are authored against templates that live in the graph itself:
plan templates, ADR templates, briefing/concept templates, UX/UI design
templates, test templates — one standardized registry (`brain template set`,
scaffolding via `brain deliverable new`), so agents from different vendors
(Claude Code, Codex, ...) produce the same shapes and can pick up each
other's work. The same registry projects the guardrails agents read (`brain
instructions generate`). And the templates are not static: they evolve on
evidence — `brain template fitness` scores contract versions by conformance,
outcomes and verdicts, so past learnings reshape what the next agent is
asked to produce.

## Quickstart

```bash
cargo test                 # 178 tests, including crash recovery
cargo run -p brain -- init
cargo run -p brain -- demo            # author -> store -> bind -> run
cargo run -p brain -- twin refresh . --prefix twin/self   # the brain twins itself
cargo run -p brain -- hook install --tests   # every commit: refresh + run tests + import protocol
cargo run -p brain -- man --install    # then: man brain — projected from the same registry as --help
cargo run -p brain -- twin symbols twin/self/crates/brain-core/src/object.rs
cargo run -p brain -- twin rdeps twin/self/crates/brain-observe/src/symbols.rs --transitive  # blast radius
cargo run -p brain -- twin at twin/self 2h        # the twin as it was (also takes a git hash)
cargo run -p brain -- twin backfill . --prefix twin/self  # brownfield: replay git history into the twin
cargo run -p brain -- bench index                 # cortex vs cold replay, answers verified
cargo run -p brain -- note twin/self/README.md "docs entry point"
cargo run -p brain -- plan add ~/.claude/plans/feature.md --prefix twin/self  # Claude Code plans
cargo run -p brain -- adr list twin/self       # decisions (auto-captured from docs/adr/)
cargo run -p brain -- skill list twin/self     # agent skills (SKILL.md, auto-captured)
cargo run -p brain -- agentcfg list twin/self  # CLAUDE.md/AGENTS.md/.cursorrules/settings
cargo run -p brain -- deliverable new adr --title "Use X"  # scaffold from graph template
cargo run -p brain -- template set runbook --applies-to runbook --capture "docs/runbooks/*.md" \
  --fields "title=heading, service=line" --requires "title,service"  # teach a new kind, no code
cargo run -p brain -- feature add twin/self core --part-of auth  # a feature has testable parts
cargo run -p brain -- feature tree twin/self    # parts, with readiness rolled up from the leaves
cargo run -p brain -- feature matrix twin/self # definition-of-done as a rendered query
cargo test 2>&1 | cargo run -p brain -- testrun import - --prefix twin/self  # protocol -> graph
npx playwright test --reporter=json | brain testrun import - --prefix twin/self  # + screenshots, videos, traces
cargo run -p brain -- twin tests twin/self     # frameworks, covers-relations, failing
cargo run -p brain -- sessions import . --prefix twin/self  # which agent worked here, and on what
cargo run -p brain -- sessions list twin/self  # objectives, models, blast radius
cargo run -p brain -- twin stale twin/self     # docs invalidated by later file changes
brain docs generate            # regenerate docs: md + screenshots + narrated screencast
cargo run -p brain -- twin insights twin/self   # churn, hubs, growth, notes, decisions
cargo run -p brain -- wake twin/self            # orientation: last sleep, the delta since, attention, stale, in-flight
cargo run -p brain -- attend twin/self          # attention: what matters now, ranked with reasons
cargo run -p brain -- spine twin/self           # what each feature reaches, what nothing claims, what nothing corroborates
cargo run -p brain -- related twin/self/crates/brain-observe/src/twin/reads.rs  # association, with why
cargo run -p brain -- before twin/self/crates/brain-core/src/object.rs  # pre-edit briefing: write access, blast radius, tests, docs, churn, notes
cargo run -p brain -- next twin/self            # the future leg: ranked work queue — failing, unsettled, rotting, unfinished
cargo run -p brain -- find twin/self "effect boundary"   # where is the thing that does X — symbols, docs, notes, ranked by centrality
cargo run -p brain -- can-i docs/brain/plans/sprint.md   # the gate as a question: exit 0 = write the file, exit 3 = the graph owns it
cargo run -p brain -- wake twin/self --json     # orientation queries speak JSON too — same data, machine shape
cargo run -p brain -- note twin/self/src/ui.rs "tried X, failed because Y" --kind dead-end  # negative knowledge, queryable
cargo run -p brain -- sessions annotate twin/self <id> --outcome shipped  # did the session's work survive?
cargo run -p brain -- sleep twin/self           # consolidation: distill the session into memory
cargo run -p brain -- plan done twin/self <slug>   # lifecycle: finished plans stop rotting and leave the lists
cargo run -p brain -- adr ack twin/self <slug>     # reviewed, still accurate — staleness clock reset, file untouched
cargo run -p brain -- artifact new twin/self plan sprint --title "Sprint"  # graph-first: renders a READ-ONLY projection
cargo run -p brain -- asset add docs/assets/flow.svg --prefix twin/self --for plan/sprint --depicts src/ui.rs
cargo run -p brain -- instructions generate     # one guardrail block into CLAUDE.md + AGENTS.md, from the registry
cargo run -p brain -- eyes --prefix twin/self   # the visual layer for people: judgments, evidence, media, the anatomy
cargo run -p brain -- tidy . --prefix twin/self # drifted projections, retired files, legacy assets — fixes are governed
cargo run -p brain -- template fitness          # which contract versions work: conformance, outcomes, verdicts
cargo run -p brain -- hook install --tests --gate  # opt-in pre-commit: refuse hand-edited projections + contract violations
cargo run -p brain -- change propose twin/self <path> --from <file> --reason "why"  # governed mode
cargo run -p brain -- change apply twin/self <slug> --cap fs   # Intent -> write -> Receipt -> verify
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
| `brain-index` | The system-of-query seam: derived, disposable indexes rebuilt by replaying the event log. `MemIndex` is the reference backend. |
| `brain-cortex` | Our own persistent graph-query engine on that seam: checkpoint + event-log delta-replay (O(new events) warm opens), recursive traversal (`--transitive`), bi-temporal reads (`twin at`). Disposable by contract. |
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
