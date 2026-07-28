<!-- brain:projection kind=plan slug=eyes-concept-parity — GENERATED, READ-ONLY. Edit via: brain artifact edit twin/self plan eyes-concept-parity --file <md> -->

# Eyes concept parity: graph-backed application cockpit

## Outcome

Eyes becomes the human application cockpit expressed by
`design-draft/Eyes - Prototype.dc.html` and
`design-draft/Eyes - MRI.dc.html`: the same information architecture,
visual language, interaction model, and depth, backed only by current graph
state.

The prototype is the product reference, not loose inspiration. Where a
prototype feature has no truthful graph representation yet, Eyes shows an
explicit empty or unavailable state and the graph model is built before the
feature is presented as live.

## Product contract

- Preserve the prototype's two complementary modes:
  - **Eyes** is the calm application cockpit for Now, Work, Features,
    Artifacts, Evidence, and History.
  - **MRI** is the dense graph instrument for projection, comparison,
    reachability, activity, and replay.
- Match the concept's shell, density, hierarchy, type system, palette,
  navigation, cards, tables, inspectors, and full-screen MRI composition.
- Keep namespace HEAD and event cursor visible and propagate them through
  every projection.
- Deep links, filters, selection, and snapshot state live in the URL.
- Never manufacture demo entities, counts, approvals, sessions, features, or
  evidence in production.
- Keep Eyes read-only until a separate accepted ADR defines authorization,
  capability scope, intent/receipt handling, CSRF protection, and audit
  behavior for mutations.
- Build projections in Rust and send purpose-built DTOs; the browser never
  reconstructs domain truth from raw graph objects.

## Current baseline and gaps

The current implementation provides four read-only endpoints
(`/api/cursor`, `/api/overview`, `/api/graph`, `/api/entity`) and two usable
surfaces (Now and Explore). It does not yet implement the concept's Work,
Features, Artifacts, Evidence, History, feature dossier, artifact detail,
governed-action flow, global command search, or MRI controls.

Graph data already supports entities, live relations, observations,
attention, source structure, decisions, documents, plans, templates, tests,
staleness, snapshots, reachability, and event history. The current graph does
not expose registered features, actors, sessions, approvals, or governed
actions as usable first-class Eyes entities.

## Slice 0 — Reference contract and UI foundation

1. Capture the default, detail, empty, loading, error, and responsive states
   of every prototype surface in a checked-in reference matrix.
2. Extract the prototype's design tokens and reusable patterns: application
   shell, left navigation, command bar, status chips, tables, cards,
   inspector, timeline, modal, and MRI chrome.
3. Replace the current Now/Explore visual structure with the prototype shell.
   Do not create a third visual direction.
4. Add client routing and URL state for view, search, filters, selected entity,
   projection, snapshot, and comparison target.
5. Establish screenshot baselines at desktop and narrow widths, plus keyboard
   focus and reduced-motion checks.

Acceptance:

- The shell is recognizably the prototype before feature depth is added.
- Existing live overview and entity data renders inside the new shell.
- Loading and empty states do not shift the application geometry.

## Slice 1 — Now and global search parity

1. Rebuild Now around the prototype hierarchy:
   application pulse, attention banners, active work, recent changes, and
   application health.
2. Add a server-side global search projection across entities, artifacts,
   relations, observations, and stable IDs.
3. Implement the command/search overlay with keyboard navigation and deep
   links to the relevant surface and inspector.
4. Derive health indicators from evidence and staleness rather than generic
   repository counts.
5. Show active-work and approval panels only when their domain data exists;
   otherwise explain what has not yet been modeled.

Acceptance:

- Now answers “what needs a human now?”, “what changed?”, and “is the
  application healthy?” without exposing raw `brain wake` output.
- Every headline and count links to the underlying graph-backed view.

## Slice 2 — MRI as the primary graph instrument

1. Replace Explore with the full-screen MRI composition from the concept.
2. Add projection selection for live activity, features, evidence,
   capabilities, artifacts, architecture, change, risk and attention,
   governance, and sessions. Disable projections whose required entities do
   not exist, with a precise explanation.
3. Implement graph/path search, entity-kind lenses, fade/hide/isolate display
   modes, selection, and the universal inspector.
4. Add Cortex-backed reachability and blast-radius queries.
5. Add snapshot selection and compare mode using namespace history.
6. Add an activity window and event lanes sourced from graph timestamps and
   event history.
7. Add deterministic replay of recorded events; omit the prototype's
   simulation-only controls from production.

Required projections include:

- projection catalog and availability,
- snapshot catalog and snapshot diff,
- path and reachability queries,
- time-windowed activity,
- projection-specific graph DTOs.

Acceptance:

- MRI matches the concept's layout and interaction vocabulary.
- Selecting a projection materially changes the graph semantics, not merely
  node colors.
- Snapshot compare and blast radius are reproducible from the same HEAD and
  cursor.

## Slice 3 — Artifacts, History, and Evidence

### Artifacts

1. Build a searchable, filterable artifact inventory from documents,
   decisions, plans, runbooks, assets, protocols, reports, configuration, and
   receipts.
2. Add artifact detail with Rendered, Source, Relationships, Evidence,
   History, and Audit tabs.
3. Derive origin, freshness, verification, version, and changed-at fields
   server-side.

### History

1. Project the event log into human events with actor/source, timestamp,
   subject, outcome, and snapshot.
2. Group related intent, effect, receipt, verification, and lifecycle events
   into action chains when those relations exist.
3. Support search, actor/source filtering, pagination, and links back to the
   affected entities.

### Evidence

1. Introduce claim DTOs whose support is a visible graph path.
2. Classify claims as verified, partly supported, unsupported, stale, or
   unconfirmed from linked observations and protocols.
3. Add proof-chain inspection and “Trace in MRI”.

Acceptance:

- Users can move from a health statement to its claim, proof path, artifacts,
  versions, and audit history without losing snapshot context.
- Content bytes remain omitted unless an artifact endpoint explicitly and
  safely exposes a supported preview.

## Slice 4 — Feature registry and dossiers

This slice starts with graph modeling, because the current repository reports
zero registered features.

1. Define and capture first-class feature entities and their relationships to
   components, implementation, tests, documents, decisions, artifacts, and
   work.
2. Define completion as a derived contract backed by those relationships and
   observations; status is never hand-set in the UI.
3. Add feature list search, filtering, paging, and completion rollups.
4. Add the feature dossier with definition of done, component tree, evidence,
   active work, documentation freshness, and visual evidence.
5. Add the Features MRI projection only after the same model powers the
   dossier.

Acceptance:

- At least one real repository feature is captured end to end.
- Its status can be explained entirely through linked graph facts.
- List, dossier, Evidence, Artifacts, Now, and MRI agree at one snapshot.

## Slice 5 — Work, sessions, and governed actions

1. Model actors, sessions, work items, actions, approvals, capabilities,
   intents, receipts, and verification as first-class graph concepts.
2. Add a read-only Work board, list, timeline, actor grouping, and feature
   grouping.
3. Add session detail and governed-action history using real action chains.
4. Draft and accept a separate ADR before adding any approval or mutation
   endpoint.
5. Only after that ADR, implement approval review and execution with explicit
   capability scope and durable intent/receipt auditing.

Acceptance:

- Work never infers a running agent from file churn.
- Every approval identifies actor, scope, target, intended effect, snapshot,
  and audit destination.
- Replaying or refreshing the UI cannot repeat an external effect.

## Slice 6 — Production hardening

1. Add projection contract tests for snapshot consistency, relation
   retraction, pagination, filtering, historical reads, and read-only
   guarantees.
2. Add browser flows and visual comparisons for every reference state.
3. Verify keyboard navigation, focus order, contrast, reduced motion, narrow
   layouts, large graphs, and honest error recovery.
4. Add performance budgets and incremental caches keyed by prefix, HEAD,
   cursor, projection, filters, and snapshot.
5. Document the operational model and add a release runbook for Eyes.

## Delivery order

The next implementation milestone is **Slice 0 + Slice 1 + the MRI frame and
one real projection from Slice 2**. This corrects the visual/product direction
immediately while delivering one complete graph-backed interaction. Artifacts,
History, and Evidence follow because the graph already contains much of their
truth. Features and Work follow only after their missing domain models exist.

## Explicitly deferred

- Production mutation endpoints before the governance ADR.
- Fake approvals, sessions, actors, features, or feature completion.
- Prototype-only reset, connection-loss simulation, and simulated time.
- Remote binding and multi-user authentication.
