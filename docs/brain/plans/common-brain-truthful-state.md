<!-- brain:projection kind=plan slug=common-brain-truthful-state — GENERATED, READ-ONLY. Edit via: brain artifact edit twin/self plan common-brain-truthful-state --file <md> -->

# Common Brain: Truthful State, Typed Artifacts, Placement & Learning

## Context

Fully agentic coding (Claude Code, OpenAI Codex, and comparable CLI agents — the *only*
consumers) produces artifacts that rot fast: plans, ADRs, design guides, task lists, test
protocols, screenshots, capability matrices. They pile up, each agent formats them slightly
differently, and knowing the *real* current state becomes impossible. The brain repo
(`/Users/drietsch/brain`) exists to solve exactly this, and deep exploration confirmed the
bones are right but load-bearing pieces are missing:

- `supersedes` relations are **written but read by nothing** — superseded ADRs list as live.
- Plans **have no lifecycle by construction** (`docs.rs` discards plan status); finished
  plans stay stale-flagged forever.
- Relations are **never retracted** — deleted symbols/imports/mentions inflate hubs, blast
  radius, `symbols_total`, and staleness forever.
- Staleness is binary (`any mentioned file newer than doc`) and **saturated to noise**:
  10 of 12 ADRs + CLAUDE.md flagged simultaneously in the shipped tour.md.
- Attention has **no recency** (README.md, lifetime churn 41, permanently ranks #1).
- README/architecture.md/twin.md/runbooks have **no staleness story at all** (no capture
  rule matches them; live store has zero template entities — defaults ship no capture globs).
- `generated=true` only *suppresses* churn/attention; hand-edits to projections are hidden,
  not detected. `narration.txt`/`brain.1` aren't even twinned (`.txt`/`.1` not ingestible).
- Insights truncate to TOP=5 and narration reports truncated counts as totals.

### Confirmed decisions (user-approved)

1. **Hybrid per kind**: process artifacts (plans, task lists, matrices) become graph-first
   with files as projections; code + narrative docs stay file-first with real staleness.
   Placement policy per kind is graph data.
2. **Opt-in gate**: advisory by default; per-kind escalation to enforced (refusal with
   actionable fix-it output, exit code 3). Pre-commit gate only via `hook install --gate`.
3. **Truthful state first**: Phase 1 fixes lifecycle/retraction/staleness before new machinery.
4. **Deliverable**: design ADRs (013–022) + phased Rust implementation in this repo.
5. **Projections are read-only** (user hard requirement): chmod 444 + `expected_b3`
   detection + opt-in commit gate + rescue-then-re-render repair. Hand-edits are preserved
   as observations, never silently destroyed.
6. **Agent-neutral CLI-first**: every capability is a deterministic shell command with
   token-efficient output; CLAUDE.md/AGENTS.md guardrail sections become generated
   projections so all agent families get identical rules.

### Invariants (never break)

Objects immutable & content-addressed; observations/relations add-only ("update" = new
observation, `latest()` resolves; retraction = tombstone observation, like `present=false`);
no floats; serde tag owns `kind`, Relation label is `predicate`; salience computed at query
time, never stored; no new `Object` variants — artifact kinds are `entity_kind` strings.
House test norm: **every write path gets a run-twice-writes-zero-objects test**
(`store.count_objects()`).

---

## New vocabulary (unified reference)

### Observation properties

| property | on | values | purpose |
|---|---|---|---|
| `active` | edge sid | `true`/`false` | relation tombstone; absence = live |
| `lifecycle` | doc-ish entities | `active,done,abandoned,retired,superseded` | explicit lifecycle override |
| `lifecycle_why` | same | text | optional reason |
| `reviewed` | doc-ish entities | text/`ok` | staleness ack; its timestamp resets the rot clock |
| `rot` | template entity | `none`/`info`/`warn` | per-kind staleness severity (`none` = exempt). Time-based (`<N>d`) deferred |
| `status` | **plan** entities (newly allowed) | parsed `Status:` line | docs.rs stops discarding it |
| `placement` | template entity | `graph_first`/`file_first`/`projection` | where truth lives |
| `enforce` | template entity | `advisory`(default)/`enforced` | the opt-in gate |
| `links` | template entity | csv predicates | allowed link vocabulary (advisory) |
| `project_to` | template entity | path pattern | projection render target |
| `parser` | template entity | `doc.decision`/`doc.plan`/`agent`/`fields` | capture routing |
| `contract_b3` | template entity | blake3 hex of `requires+"\n---\n"+content` | **template version id** |
| `recommended` | template entity | csv fields | suggested-not-required |
| `template_b3` | artifact entities | blake3 hex | which template version judged this artifact |
| `expected_b3` | file entities | blake3 hex | projection contract hash |
| `hand_edit` | file entities | rescued content | tidy's preservation of overwritten edits |
| `ingest_extensions` | repo entity | csv | additive extension allowlist |
| `instructions_b3` | agent_config file entity | blake3 hex | managed-block drift detection |

### Predicates (new)

- `renamed_to` (old file sid → new file sid) — written by refresh (same-run delete+add with
  identical `content_b3`) and backfill (git reports renames there).
- `projected_to` (artifact entity → file entity).
- `attached_to` (asset entity → owning artifact/template/feature) and
  `depicts` (asset entity → file/feature it captures) — declared at capture, since media
  can't be substring-scanned for mentions.

**Edge sid**: `StableId::derive(["edge", from, predicate, to])` — collides with no existing
derive namespace.

### CLI verbs (all added to `manual::COMMANDS` + match arm in `main.rs`)

```
brain wake <prefix> [--full]
brain relation retract <from> <predicate> <to> | relation list <name> [--all]
brain plan done|abandon|reopen|ack <prefix> <slug> [--why|--note]
brain adr ack <prefix> <slug> [--note]
brain artifact set-lifecycle|ack <prefix> <kind> <slug> ...
brain artifact new|edit <prefix> <kind> <slug> [--title T] [--file f|-]
brain artifact render [dir] [--prefix p] [--kind k] [--check]
brain asset add <file> --for <kind>/<slug> [--depicts <path>...]
brain instructions generate [dir] [--prefix p] [--check]
brain tidy [dir] [--prefix p] [--fix --cap fs] [--rm <path>]
brain template set <slug> ... [--placement|--enforce|--rot|--links|--project-to|--parser]
brain template fitness [slug] | template evolve <slug> [--apply]
brain twin config <prefix> --add-extensions csv
```

Exit codes: 0 ok, 1 error, 2 usage, **3 = deliberate gate refusal** (lets hooks distinguish
"brain broke" from "brain refused"; fail-open for errors, fail-closed only for refusals).

---

## Phase 1 — Truthful current state

Key files: `crates/brain-observe/src/twin.rs` (refresh `run()` 60–418, staleness 594–609,
`relate()` 789–819, `record_entity_doc` 1103–1193), `attention.rs`, `sleep.rs`, `docs.rs`
(plan status at 122–130), `crates/brain-index/src/lib.rs`, `crates/cortex/src/lib.rs`
(`reach`), `crates/brain-cli/src/main.rs` + `manual.rs` + `hooks.rs`.

1. **Edge substrate** (`brain-index`): `edge_sid`, `edge_active`, `edge_active_at` helpers;
   `relate()` writes `active=true` (guarded) when re-observing a retracted edge;
   `cortex::reach` skips dead edges. Absence-means-live ⇒ zero store migration.
2. **Refresh retraction sweeps** (`twin.rs run()`), all guarded (write only on transition):
   changed/`--full` files retract un-reobserved `contains`/`imports`/`covers`; re-recorded
   docs retract dropped `mentions` and stale `recorded_in`; deleted files retract all
   outgoing edges — *including already-deleted files*, which is the self-healing migration.
   Readers switch to live edges: `insights_with` (fixes `symbols_total`, hubs),
   `attention.rs`, `features::evaluate`, CLI `relation_targets`/`symbols|imports|rdeps`,
   `assoc.rs`. Add `brain relation retract|list`.
3. **Lifecycle** (`brain-observe/src/lifecycle.rs`): derivation precedence (query-time,
   never stored unless explicitly set): live incoming `supersedes` → Superseded; explicit
   `lifecycle` obs; mapped `status` (`done|completed|shipped`→Done etc.); all `recorded_in`
   files `present=false` → Retired; else Active. `docs.rs` parses plan `Status:`. All list
   commands + insights + stale + attend filter non-active by default, `--all` shows history,
   tags like `[superseded]` printed. `adr show` prints "superseded by <slug>".
4. **Staleness v2**: skip non-Active docs; only live `mentions` count;
   `effective_time = max(content, reviewed)`; severity from template `rot` (code defaults:
   decision/plan → `info`, skill/agent_config/custom → `warn`). `twin stale` groups
   warn-first; hook nags only on warn. Ack verbs write `reviewed`.
5. **Attention recency**: window = `consolidated_until` (last sleep; 0 ⇒ current behavior).
   Churn score `recent*4 + lifetime.min(10)`; hubs use live edges; doc weights by severity.
   Still deterministic integers, never stored.
6. **Truncation honesty + `brain wake`**: `Insights` returns full lists; rendering truncates
   with "(showing 5 of 12)". Refactor sleep's delta into `delta_since()`; new `wake.rs`
   composes: last session summary + age, delta since sleep, top attention, warn-stale,
   in-flight (active plans, pending/indeterminate changes, unfinished features), coherence
   findings, notes since sleep. Budget ~40 lines.
7. **Coherence** (`coherence.rs`): active docs mentioning deleted files; `test_case` with
   dangling `defined_in`; proposed-never-applied / indeterminate / broken changes; shipped
   features with incomplete DoD (moved from attention). Surfaced in wake + insights.
8. **Renames**: match deleted↔added by `content_b3` in the same run → `renamed_to`;
   mirror in `backfill.rs`; `twin files` prints `[moved to X]`.
9. **Docs**: update `docs/runbooks/release.md` (add `--full` step, delete the
   "finished plan stays flagged" workaround), regenerate `docs/generated/`.

Steps land independently; 3–5 are parallel after 2.

## Phase 2 — Artifact kind registry

**Step 2.0 (zero code, do first):** seed capture rules via existing CLI so README/docs/*.md
and runbooks finally get entities + mentions + staleness:
`brain template set doc --applies-to doc --capture "README.md,docs/*.md" --fields "title=heading" --requires title`
(exclude `docs/generated/**` and `docs/brain/**` in the glob set); same for `runbook` →
`docs/runbooks/**/*.md`.

1. **`kinds.rs` registry** (read side): `KindDef` struct; compiled `DEFAULTS` mirror today's
   hardcoded conventions (adr/plan/skill/agent_config with `parser` routing) + new kinds
   `doc`, `runbook`, `task-list` (graph_first), `capability-matrix` (projection),
   `asset` (file_first, `docs/assets/**` + media globs). `registry() = DEFAULTS ⊔ graph
   observations` (graph wins per property via `latest()`). Identity preservation: registry
   routing derives the same `StableId::derive([kind, prefix, slug])` as the hardcoded path
   → zero identity churn, re-capture is a guarded no-op.
2. **Seed v2 + `template set` flags** (`--placement --enforce --rot --links --project-to
   --parser`); stamp `contract_b3`. Migration = `brain template seed` (optional; overlay
   makes behavior identical without it).
3. **Twin routes capture through the registry** (`twin.rs` 88–182): single lookup replaces
   the doc/agent/rules triple; route by `parser`; hardcoded detectors remain fallback only.
   Stamp `template_b3` beside `conforms` — starts Phase 4 data collection now.
4. **Rule-driven ingest extensions**: `collect_files` takes extras from repo
   `ingest_extensions` obs + extensions in capture globs; additive only, >1 MiB and binary
   (non-media) skipped. Add `txt` and `1` to the compiled const (narration.txt/brain.1
   become twinnable). `brain twin config --add-extensions`.
5. **`resolve_target` for all kinds** (features.rs): try file, every registered kind,
   `test_run`, `change`; warn (never refuse) on predicates outside the kind's `links`.
6. **Assets & prototypes**: `brain asset add <file> --for <kind>/<slug>
   [--depicts <path>]` — typed entity (subtype, `content_b3`, mime) + `attached_to`/
   `depicts` relations. Authored assets (HTML templates, reference designs) are file-first
   under `docs/assets/`; lifecycle derives from owner (owner retired → asset retired) and
   the generic `artifact set-lifecycle|ack` verbs apply to assets too; staleness: `depicts`
   target changed after asset capture ⇒ stale, surfaced in `twin stale`/wake (reuses Phase 1
   machinery — screenshots finally rot *visibly*). Seed a `prototype` kind (file_first,
   capture `prototypes/**/README.md` — the README is the prototype's identity; the entity
   represents the whole directory): prototypes carry lifecycle (active/done/retired) so
   spikes get formally concluded instead of lingering. Binary NEVER enters graph objects;
   a `.brain/blobs/` sidecar is explicitly deferred until needed.

## Phase 3 — Read-only projections, authoring gate, instructions, tidy

Projection home: **`docs/brain/<kind-plural>/<slug>.md`** — git-visible on purpose (agents
grep repo files; PRs show artifact diffs); renders are deterministic so diffs = meaning.

1. **`projection.rs`**: `render()` — first-line marker naming the authoring command
   (`<!-- brain:projection kind=plan slug=X — GENERATED, READ-ONLY. Edit via: brain
   artifact edit ... -->`); `write_projection()` — atomic write, then **chmod 444**
   (fs layer), guarded `generated=true` + `expected_b3` + `projected_to`;
   `reapply_readonly()` — re-arm the bit (git doesn't preserve it) from refresh, tidy, and
   hooks; `drift()` — the authoritative detection layer: `HandEdited`/`Missing`/
   `StaleRender` per file with fix-it command.
2. **`brain artifact new|edit|render`**: authoring INTO the graph, validated against
   `requires` at write time — the gate's enforcement point (`advisory` → warn + record;
   `enforced` → fix-it message + exit 3, nothing written). Renders projection when
   graph_first. Refresh skips doc-capture on files whose `content_b3 == expected_b3`
   (prevents double-capture of projections). `capability-matrix` renders from the live
   feature registry (direct call, no subprocess).
3. **Drift surfaced**: `Insights.drifted_projections` (query-time), post-commit hook line
   with the exact fix commands, refresh re-arms chmod.
4. **Hook events**: add `post-checkout`/`post-merge` (reapply read-only + drift line);
   `pre-commit` installed only with `hook install --gate` — exit 3 iff staged files
   intersect drifted projections or enforced-kind nonconformance; internal errors stay
   fail-open. The only fail-closed path, doubly opt-in.
5. **`brain instructions generate`**: renders one deterministic guardrail block (kind table:
   placement, home, authoring command, required fields, enforcement; projection rules;
   session rhythm) into managed marker blocks in **CLAUDE.md and AGENTS.md** (identical
   content — every agent family gets identical rules). Only the block is replaced; file not
   chmod'd; block drift via `instructions_b3` observation.
6. **docsgen adopts the contract**: tour.md/narration.txt/brain.1 through
   `write_projection`; keep screenshot provenance (record section mapping in the graph
   instead of deleting `.sections.json`).
7. **`brain tidy`** (`tidy.rs`): advisory scan → categories: hand-edited projections
   (fix: rescue content into `hand_edit` observation + note, then re-render — full content,
   not a diff; never silently destroy agent work), stale renders (re-render), orphaned
   projections (governed move to `docs/attic/`), misplaced artifacts (governed move to the
   kind's home; refuse if git-dirty), retired artifacts' files (attic), **legacy assets &
   prototypes** — assets whose owner is retired/done or whose `depicts` target no longer
   exists, and prototypes with lifecycle done/retired: governed move of the file (or whole
   prototype directory) to `docs/attic/`, preserving the graph entity + relations as
   history; deletion only via explicit `--rm` — unknown ingestible files (advise a
   `template set` teaching command), writable projections (chmod).
   `--fix --cap fs` required for content-touching fixes; every move/edit goes through
   `govern::propose/apply` (extend govern with a removal proposal) so tidy actions are
   graph-recorded, auditable, revertible. Deletion only via explicit `--rm <path>`.

## Phase 4 — Template learning

1. **Versions**: template version = `contract_b3` (stamped in 2.2; artifacts record
   `template_b3` since 2.3). Version chain = observation timeline; `supersedes` reserved for
   slug replacement.
2. **`brain template fitness [slug]`** (`fitness.rs`, query-time, never persisted, integer
   math): per version — first-capture conformance rate (earliest `conforms` per artifact via
   `put_history()` order), missing-field frequency (drop/enforce candidates), outcome rates
   (plans done vs abandoned, decisions superseded, features DoD-met, docs currently stale),
   median days-to-rot. Fixed-width table + `verdict:` lines
   ("status missed by 4/5 adr at first capture — consider --enforce").
3. **`brain template evolve <slug> [--apply]`**: deterministic integer-threshold
   suggestions (field missed ≥50% with good outcomes → demote to `recommended`; missed ≤10%
   → suggest `enforced`; kind ≥50% abandoned → scaffold restructure). Prints a draft
   `template set` invocation; `--apply` writes it (bumping `contract_b3`, opening the next
   measurement window). **No auto-mutation** — approval is explicit; old artifacts keep
   their `template_b3`, so fitness compares versions across brain generations.

---

## ADRs to write (`docs/adr/`, captured by the twin)

013 lifecycle-as-derived-judgment · 014 relation-currency-via-edge-tombstones ·
015 staleness-severity-and-acknowledgement · 016 wake-and-the-sleep-window ·
017 artifact-kind-registry (overlay, parser routing, identity-preserving migration) ·
018 placement-policy-and-assets (graph_first/file_first/projection; docs/brain/;
docs/assets/) · 019 read-only-projection-contract (marker, chmod, expected_b3,
rescue-then-re-render) · 020 opt-in-enforcement-gates (exit 3, --gate, the sense-organ
boundary drawn precisely) · 021 tidy-through-governed-changes · 022 template-fitness
(computed never persisted; contract_b3 versions; threshold evolution).

## Verification

- `cargo test` green per landed step; **every writer: double-run writes zero objects**
  (`store.count_objects()` pattern).
- Phase-1 specifics: retract→re-observe→retract = 2 observations; deleting a symbol drops
  `symbols_total` and second refresh writes nothing; `Supersedes:` hides target from
  `adr list` (visible with `--all`); `plan done` exits stale/wake; ack resets clock without
  touching the file; recency test (high-lifetime/zero-recent ranks below recently-edited
  after sleep); wake totals are true totals; `reach` skips dead edges (`answers_match`
  still passes).
- Phase-2/3 specifics: registry overlay precedence; old-store no-identity-churn migration
  test; render determinism (byte-identical); chmod round-trip + reapply after simulated
  checkout; hand-edit → tidy → `hand_edit` observation + restored read-only file;
  pre-commit gate in tempdir git repo (advisory commits, enforced exits 3 listing missing
  fields); governed moves leave confirmed change entities; managed-block replacement
  preserves surrounding text; fitness math on synthetic two-version store.
- End-to-end demo on the live store: `cargo build -p brain`, `brain twin refresh . --prefix
  twin/self --full`, then verify: `adr list` no longer shows the retired pre-rename ADR;
  the finished plan is gone from `twin stale`; `brain wake twin/self` produces a truthful
  sub-40-line orientation; seed step 2.0 makes README/architecture.md staleness-tracked.

## Migration

None required: all mechanisms are add-only and absence-tolerant. Self-healing on refresh;
one `--full` refresh after upgrading retracts vanished edges of unchanged files. Optional
`brain template seed` materializes kind records for replication/editing.
