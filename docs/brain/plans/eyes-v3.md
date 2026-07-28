<!-- brain:projection kind=plan slug=eyes-v3 — GENERATED, READ-ONLY. Edit via: brain artifact edit twin/self plan eyes-v3 --file <md> -->

# Eyes v3 — the application understanding environment

## Context

Eyes v2 shipped (ADR-024): six surfaces, one server-side voice, judgments as
sentences, content you can read, an aggregated map, sub-250 ms warm responses.
It solved "the monster spoke the graph's vocabulary and hid the graph's
product."

Two things now change the target.

**1. A UX/UI briefing** defines Eyes as "an application understanding,
collaboration, governance, and audit environment" with seven sections (Now,
Work, Features, Artifacts, Evidence, History, MRI), a universal inspector,
evidence-backed claims everywhere, and an immersive living-graph view. Its §24
independently forbids exactly the failure modes v2 removed — no admin
dashboard, no graph database browser, no unsupported health scores, no evidence
hidden behind badges — so the briefing and ADR-024 agree on principle. What it
adds is *reach*: three surfaces v2 does not have (Work, Evidence, MRI) and a
far deeper treatment of Features, Artifacts and History.

**2. An explicit content demand**: all tests, test results, test runs,
Playwright tests including screenshots, and TTS narrations. Today Eyes shows a
test *summary* (last run, failing, flaky, uncovered) and no media at all, while
the graph holds 132 `test_case` entities, 5 `test_run`s, 5 `Evidence` objects,
six generated screenshots and two narrated videos.

The outcome: Eyes becomes the single place a developer sees the real current
state of an application — what is claimed, what proves it, what an agent did,
and what it looks like — without ever writing to the graph.

---

## The honesty ledger (established by inspecting the live store, not the docs)

This governs every design decision below. **Nothing in the right-hand column
may be invented.**

| The briefing wants | The graph actually has |
|---|---|
| Feature status with evidence | `features::evaluate` — but each DoD slot is only *a count of live relation targets > 0*. `tested_by` does **not** check that the test passed. |
| "18 passing tests" as proof | Run-level `Evidence` only: 5 objects, all `Behavioral`, all attached to the **repo** node. No per-case, no per-file evidence. |
| Test detail | `test_case` = `derive(["test", prefix, name])`; `result` is observed **only on transition** (that is the flake signal). No duration, no error text, no stack, no retry, no suite. |
| Which cases were in run #3 | Not answerable — only `test_run -failed-> test_case` edges exist. |
| Playwright screenshots | No artifact ingest at all. Playwright is recognised only by `content.contains("@playwright/test")`. Playwright is used *by* brain (capture.mjs) but never *ingested from*. |
| Screenshots as artifacts | The six PNGs are plain `source_file` entities with `generated` / `expected_b3` / `rendered_from`. **`assets::add` has exactly one caller — the CLI.** Zero assets exist, so `assets::stale` can never fire for them. |
| Video playback | Eyes has **no HTTP Range support**. `<video controls>` is already rendered; Safari will refuse it and Chrome cannot seek. |
| Actor — "who did this" | **Nothing.** No field on Intent, Receipt, Observation or any entity. `Observation.source` holds mechanism names (`twin`, `govern`, `testrun`, `agent`). Git author is fetched and deliberately discarded. |
| Approvals | **Definitively none.** The 7 change statuses go `proposed → applied` with nothing between. `VerificationLevel::Authorization` is never constructed. |
| Capability scope | `Object::Capability` has a `scope: BTreeMap` and is **never constructed anywhere**. The only check is `caps.iter().any(\|c\| c == "fs")`. |
| "Reproduce this action" | No CLI invocation is ever recorded; `Intent.arg_hash` is a digest, deliberately not a reference. A change's command can be *reconstructed* from its labels — and must be labelled as reconstructed. |
| Sessions | A watermark (`consolidated_until`) plus one summary string. No id, no owner, no membership. |

**Rule carried forward from ADR-024, sharpened:** absence is silence, and
anything *reconstructed* rather than recorded says so on screen.

---

## Locked decisions (user)

1. **MRI: the full 3D living graph** — WebGL, every entity, atmospheric.
2. **Work: ingest real agent sessions** from Claude Code and Codex transcripts.
3. **Narration: the existing tour *and* narrate-any-screen.**
4. **Eyes stays strictly read-only.** Governance remains behind the capability
   boundary; every actionable item offers the exact command to copy.

---

## Phase 1 — Graph foundations (`brain-observe`, `brain-cli`)

Eyes cannot show what the graph does not hold. Three gaps close first.

### 1a. Playwright ingest with artifacts — `crates/brain-observe/src/testing.rs`

Add `parse_playwright_json` beside the existing `parse_cargo` / `parse_junit`
(auto-detected in `parse_report` by a leading `{` plus a `"suites"` key). The
Playwright JSON reporter carries everything JUnit throws away:

- per-case `status`, `duration`, `retry`, `error.message`, `spec.file`, `line`
- **`attachments[{name, path, contentType}]`** — screenshots, video, trace

Widen `RunReport.cases` from `(String, CaseStatus)` to a `Case` struct
(`name, status, duration_ms, error, file, line, attachments`); `parse_cargo`
and `parse_junit` fill only the first two, so nothing regresses.

In `record_run`, additionally write (all guarded, source `testrun`):

- `duration_ms`, `error` (first line, truncated), `retries` on the `test_case`
- `defined_in` from `spec.file` — this works where cargo's `::` split cannot
- `test_run -skipped-> test_case` alongside the existing `failed` edge
- **`test_run -includes-> test_case` only for cases that failed, were skipped,
  or flipped** — a deliberate bound. 132 edges per import is real store growth,
  and "what is the state of every test" is already answered by the case
  timeline. State the bound in the ADR rather than letting the surface imply
  completeness.
- For each attachment, reuse **`assets::add`** (`crates/brain-observe/src/assets.rs:45`)
  with `owner` = the test case, `subtype` from the existing `infer_subtype`, and
  `depicts` = the file under test where resolvable. Attachments under
  `test-results/` are already ingestible (`png`/`webm` are in
  `INGEST_EXTENSIONS`; `test-results` is not in `SKIP_DIRS`).

Also record `total_duration_ms` on the `test_run`.

### 1b. Generated media becomes declared assets — `crates/brain-cli/src/docsgen.rs`

`record_projection` marks files and stops. It should also declare each media
output through `assets::add`, giving the screenshots the staleness story
`assets.rs` exists to provide:

- `owner` = the tour document; `subtype` from `infer_subtype`
- `depicts` seeded from `rendered_from` — the command already names the query
  whose output the screenshot shows (`brain twin tests twin/self` → the repo and
  test surface). This is what makes `assets::stale` fire when the depicted thing
  changes.
- record the `duration_ms` that `duration_secs()` already computes and discards
- `rendered_from` for `tour.webm`, `tour-narrated.webm` and `narration.txt` too,
  not only the PNGs

Three defects to fix while here:

- **`mux` failure is currently fatal** and aborts before `brain.1` and
  `record_projection` run, leaving the tree writable and the graph unmarked.
  Make it degrade like every other media step.
- `capture.mjs` has no error handling at all and ignores its `prefix` argument.
- `tidy`'s `misplaced-artifact` check would flag every generated asset, because
  the `asset` template's `home` is `docs/assets/**`. Skip `generated=true`
  assets there, consistent with attention, churn and untyped-document, which
  already do.

### 1c. Agent sessions — new `crates/brain-observe/src/sessions.rs`

The first actor the graph has ever had.

- Entity kind `agent_session`, sid `derive(["session", prefix, session_id])`
- Labels: `prefix`, `session_id`, `agent` (`claude` | `codex`), `cwd`
- Observations: `objective` (first user message, truncated), `started_at`,
  `ended_at`, `turns`, `tools` (counts), `model`, `files_touched`, `outcome`
- Relations: `session -touched-> file` (edits and writes only — reads are noise
  at this volume), `session -produced-> artifact` for plans and ADRs authored
  during it, `session -concerns-> repo`

Sources, both confirmed present on this machine:

- Claude Code: `~/.claude/projects/<slug>/<uuid>.jsonl` — `user` / `assistant` /
  tool-use records carrying tool `name` and `cwd`
- Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, discoverable cheaply
  through `~/.codex/session_index.jsonl`

CLI: `brain sessions import [--agent claude|codex] [--since <t>] [--prefix p]`
and `brain sessions list <prefix>`. Idempotent by `(session_id, line_count)`,
guarded writes like every other ingest, stream-parsed (the live transcript here
is 11.9 MB).

**Privacy is a design constraint, not a footnote:** prompt and response bodies
never enter the graph. Only the truncated objective, counts, tool names, file
paths and timings. Asserted by a test, stated in the ADR.

---

## Phase 2 — Eyes server (`crates/brain-eyes`)

Follow the established pattern exactly: every view is a pure function of the
graph version, computed once, cached in `Loaded` via `OnceLock`, freshness by a
`stat` on `events.jsonl` (`state.rs`).

**HTTP Range support in `http.rs`** — the single blocking defect for media.
Parse `Range: bytes=a-b`, answer `206` with `Content-Range` and
`Accept-Ranges: bytes`, keeping `nosniff`, `sandbox` and `Content-Disposition`.
Without it neither the tour nor any Playwright video plays.

New endpoints, each backed by existing workspace functions:

| Endpoint | Built from |
|---|---|
| `/api/tests` (rewritten) | `testing::runs`, `failing_cases`, per-case `result` timelines, `defined_in` / `covers`, attachment assets |
| `/api/evidence` | `Object::Evidence` via `index.evidence_for`, `features::evaluate`, `coherence::check`, `projection::drift`, `assets::stale` |
| `/api/work` | `agent_session` entities, `govern` change statuses, active plans |
| `/api/features` | `features::evaluate`, enriched: resolve each DoD target's *current* state, so a `tested_by` target that is a test resolves to its result |
| `/api/mri` | new `query/mri.rs` — see Phase 4 |
| `/api/ask` | deterministic intent grammar over `query::find` — see Phase 3 |
| `/api/media` | asset previews, chapters, drift state, `rendered_from` provenance |

`say.rs` gains vocabulary for sessions, evidence levels, attachments,
Playwright outcomes, and recorded-versus-reconstructed labelling. The jargon
deny-list test extends to every new surface — it has already caught one real
leak and must keep doing so.

---

## Phase 3 — Operational surfaces

Navigation becomes **Now · Work · Features · Tests · Artifacts · Evidence ·
History · MRI**, plus ⌘K. That is the briefing's seven with **Tests** promoted
to its own surface, because it is both the explicit ask and the densest real
data in the store.

A **universal inspector** (§15) is built once and used by every surface:
Summary, State, Why, Relationships, Evidence, Current work, History, Actions
(commands to copy), Raw semantic data under disclosure. It opens without
navigating away.

- **Now** — headline; ranked attention with reason, age, affected entities and
  next action; **active work cards** (real sessions, at last); evidence-backed
  health indicators that each drill into their surface; a small live MRI
  viewport as the "application pulse".
- **Work** — sessions as the primary content, with objective, focus, activity,
  outputs and what they touched; governed changes with their stage strip;
  active plans. By-actor and by-feature groupings.
- **Features** — the dense table (§8.2), then the dossier: definition of done as
  evidence cards where each slot resolves to its *actual* current state, a local
  relationship map, and every claim drillable.
- **Tests** — runs (totals, evidence object, what failed, what flipped, which
  change it verified); every case with its full result history and defining
  file; test files by framework with declared counts and coverage; Playwright
  cases showing their **failure screenshot, video and trace inline**; filters
  for framework, result and flaky-only.
- **Artifacts** — the v2 shelves plus filters, and media as first-class:
  screenshots with preview, producing command, capture time and drift; the
  **narrated tour** as a chaptered player, each chapter pairing its narration
  sentence with its screenshot and the command that produced it; documents with
  their staleness explanation.
- **Evidence** — claim ↔ proof in two columns, where the claim is never visually
  stronger than its proof. The categories of §10.2, plus the section this system
  can uniquely produce: **unsupported claims** — a feature whose `tested_by`
  target has no passing result, a projection whose bytes drifted, an asset whose
  depicted file moved on. Evidence chains render as vertical traces: claim ←
  verification ← protocol ← result ← hash ← snapshot.
- **History** — episodes, plus **action detail** for governed changes: inputs
  (before and after content are both in the graph), capability, the Intent
  object, the effect, the Receipt, the verifying run. Reproduction offers the
  reconstructed command, explicitly labelled as reconstructed. **Replay is
  real**, not simulated: `twin::latest_at` and `latest_at_before` read the graph
  as it was at any past millisecond.
- **Ask (⌘K)** — structured search plus a small deterministic grammar for the
  briefing's question forms ("why is X stale", "features missing documentation",
  "what changed since…"). Every answer carries its entities, its evidence and
  the lens to open. **No language model, no invention** — an unmatched question
  says so and degrades to search.

Design system per §16: adaptive light/dark for operational surfaces (§24 — not
everything dark), always-dark MRI, IBM Plex as today, the glyph set as shared
SVG symbols, motion honouring `prefers-reduced-motion`. Accessibility per §17:
every MRI fact is also reachable as a list.

---

## Phase 4 — MRI: a stable 3D anatomy

**Layout is computed in Rust, once per graph version, and cached** — the browser
never runs a force simulation. This is what makes §3.5 ("stable anatomy, moving
activity") and §24 ("no permanently moving force graph") achievable at the same
time, and it is the exact opposite of v1's golden-angle spiral that ignored
edges entirely.

- `query/mri.rs`: deterministic force-directed layout with cluster attraction,
  seeded from stable ids, over the §12.3 clusters — features, implementation (by
  module), tests, evidence, decisions, documentation, artifacts, sessions,
  governance, history. ~1,535 nodes and ~1,771 edges, a few hundred iterations,
  cached in `OnceLock` like every other derived view.
- WebGL2 renderer written directly (no CDN, no vendored library): instanced
  billboards against a runtime-generated glyph atlas implementing §16.4's shape
  language, one draw call for edges, orbit camera, restrained glow and depth
  haze.
- **Semantic zoom is level-of-detail, never truncation** — the distinction that
  matters. v1 shipped ~1,300 nodes and silently dropped 75% in the browser.
  Here, far shows clusters and features, medium shows modules and
  neighbourhoods, near shows individual files, symbols, tests and receipts.
  Nothing is discarded; detail resolves as you approach, and the count on screen
  is always stated.
- Motion is reserved for meaning (§12.4): recent changes pulse, failing
  verification pulses sharply, an active session holds a halo, indeterminate
  effects pulse slowly and irregularly.
- Interaction per §12.5. Left rail for lens and filters; the right rail is the
  same universal inspector; the bottom is a timeline scrubber that replays
  **real** past graph states through `twin::latest_at`.
- Blast radius is a first-class lens, backed by `Cortex::reach` — the same
  traversal behind `brain twin rdeps --transitive`.

---

## Phase 5 — Narration

- **The tour**, as a durable artifact: the narrated webm plays (Range now
  works), chapters come from the seven graph-computed narration lines, each
  chapter pairs its sentence with its screenshot and the command that produced
  it, and the tour carries its own freshness — it is a projection, so it can be
  visibly out of date, which is the artifact-rot story told about itself.
- **Narrate any screen**: a "brief me" control speaks the current view's
  `say.rs` sentences through the browser's built-in speech synthesis, with the
  spoken sentence highlighted. No file written, no Python dependency, no network
  call — which keeps Eyes read-only and offline. `tts.py` stays what it is: the
  generator of *durable* audio in the docs pipeline.

---

## Phase 6 — Documents and dogfooding

- **ADR-025 — Agent sessions are first-class.** What a session is, what is
  deliberately not recorded (prompt bodies), idempotence, and why the graph
  needed an actor before Work could exist.
- **ADR-026 — MRI: a stable anatomy with real motion.** This **amends ADR-024's
  clause 3**, which forbade whole-graph drawing. That rule was written against a
  spiral that ignored edges and truncated silently; a server-computed stable
  layout with level-of-detail is a different object, and the ADR must say so
  plainly rather than quietly contradicting itself.
- **ADR-027 — Evidence you can see.** Playwright artifact ingest, generated
  media as declared assets, Range serving, and the recorded-versus-reconstructed
  rule.
- Rewrite `docs/eyes.md` around the eight surfaces and the honesty ledger.
- Then dogfood it, as the last phase did: `brain artifact new` for the plan,
  refresh the twin, import the protocol, `brain docs generate`,
  `brain instructions generate`, `brain tidy`, resolve or acknowledge any warn
  staleness, `brain sleep`, commit and push.

---

## Verification

**Tests** (extending `crates/brain-eyes/src/tests.rs` and the observe crates):

- Playwright JSON parsing: statuses, retries, error text, `spec.file` linkage,
  and attachments becoming assets owned by the case
- Session ingest: a re-import changes nothing (`count_objects()` equal), and
  **no prompt or response body appears anywhere in the store**
- Range requests: `206`, correct `Content-Range`, open-ended and out-of-range
  forms, and a full request still returning `200`
- MRI layout: identical coordinates for an identical graph version; every node
  present in the payload (no truncation); bounded element count per LOD level
- Evidence chains resolve claim → proof for a verified change
- The jargon deny-list gate extended across all eight surfaces
- Read-only gate: `count_objects()` unchanged across a crawl of every endpoint

**Live checks:**

- `brain eyes --prefix twin/self`, then walk all eight surfaces and screenshot
- The narrated tour plays *and seeks* — this fails today
- A real Claude Code session appears in Work with its objective and the files it
  touched
- Performance: every operational endpoint under 250 ms warm; `/api/mri` under
  400 ms cold and cached thereafter

**The honesty check, which matters most:** every number on screen traces to a
graph record, every reconstructed value is labelled as reconstructed, and
nothing the graph does not model produces a panel.

---

## Deliberate non-goals

- No writes from Eyes — no approvals, no apply, no mutation endpoints.
- No language model in Ask; a deterministic grammar that admits when it does not
  understand.
- No invented actors on historical records: intents and receipts written before
  session ingest existed have no actor, and will say so rather than guess.
- No per-case evidence objects fabricated from run-level evidence.
